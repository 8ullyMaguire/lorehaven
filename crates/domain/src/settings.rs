// M47: User Configuration — resolution hierarchy and namespace rules (spec §46).
//
// Settings resolve: context override → pseud → account → instance.
// Per-domain tables (never JSONB blobs). Unknown keys rejected.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::AppError;

/// Where a setting value comes from (provenance label).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingSource {
    Context,
    Pseud,
    Account,
    Instance,
}

/// One resolved setting key with its provenance.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedSetting {
    pub key: String,
    pub value: serde_json::Value,
    pub source: SettingSource,
}

/// Content filter types (spec §46.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentFilterType {
    Tag,
    Fandom,
    Warning,
}

impl ContentFilterType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tag => "tag",
            Self::Fandom => "fandom",
            Self::Warning => "warning",
        }
    }
}

impl std::str::FromStr for ContentFilterType {
    type Err = crate::AppError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "tag" => Ok(Self::Tag),
            "fandom" => Ok(Self::Fandom),
            "warning" => Ok(Self::Warning),
            _ => Err(AppError::Validation {
                message: format!("unknown content filter type: {s}"),
                field_errors: Default::default(),
            }),
        }
    }
}

/// Validated key-value for a settings write.
#[derive(Debug, Deserialize)]
pub struct SettingWrite {
    pub key: String,
    pub value: serde_json::Value,
}

/// Notification channel options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationChannel {
    InApp,
    Email,
    Push,
    None,
}

impl NotificationChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InApp => "in_app",
            Self::Email => "email",
            Self::Push => "push",
            Self::None => "none",
        }
    }
}

impl std::str::FromStr for NotificationChannel {
    type Err = crate::AppError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "in_app" => Ok(Self::InApp),
            "email" => Ok(Self::Email),
            "push" => Ok(Self::Push),
            "none" => Ok(Self::None),
            _ => Err(AppError::Validation {
                message: format!("unknown notification channel: {s}"),
                field_errors: Default::default(),
            }),
        }
    }
}

/// Namespace registry — every recognized setting key belongs to a namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingNamespace {
    Privacy,
    Content,
    ContentFilter,
    Search,
    Notifications,
    Reader,
    Appearance,
    Discovery,
}

impl SettingNamespace {
    /// Whether this namespace supports pseud-level overrides.
    pub fn supports_pseud(self) -> bool {
        matches!(
            self,
            Self::Privacy
                | Self::ContentFilter
                | Self::Search
                | Self::Notifications
                | Self::Reader
                | Self::Appearance
                | Self::Discovery
        )
    }

    /// Whether this namespace supports account-level overrides.
    pub fn supports_account(self) -> bool {
        true
    }
}

impl std::str::FromStr for SettingNamespace {
    type Err = crate::AppError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "privacy" => Ok(Self::Privacy),
            "content" => Ok(Self::Content),
            "content_filter" | "content-filters" => Ok(Self::ContentFilter),
            "search" => Ok(Self::Search),
            "notifications" => Ok(Self::Notifications),
            "reader" => Ok(Self::Reader),
            "appearance" => Ok(Self::Appearance),
            "discovery" => Ok(Self::Discovery),
            _ => Err(AppError::Validation {
                message: format!("unknown settings namespace: {s}"),
                field_errors: Default::default(),
            }),
        }
    }
}

/// Resolved settings export document (spec §46.6).
#[derive(Debug, Serialize)]
pub struct SettingsExport {
    pub version: u32,
    pub namespaces: HashMap<String, Vec<ResolvedSetting>>,
}

pub const SETTINGS_EXPORT_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> ContentFilterType {
        s.parse().unwrap()
    }

    #[test]
    fn content_filter_type_roundtrip() {
        assert_eq!(parse("tag"), ContentFilterType::Tag);
        assert_eq!(parse("fandom"), ContentFilterType::Fandom);
        assert_eq!(parse("warning"), ContentFilterType::Warning);
        assert_eq!(ContentFilterType::Tag.as_str(), "tag");
    }

    #[test]
    fn unknown_filter_rejected() {
        assert!("banana".parse::<ContentFilterType>().is_err());
    }

    #[test]
    fn channel_roundtrip() {
        let c: NotificationChannel = "email".parse().unwrap();
        assert_eq!(c, NotificationChannel::Email);
        assert_eq!(c.as_str(), "email");
    }

    #[test]
    fn namespace_pseud_matrix() {
        assert!(SettingNamespace::Search.supports_pseud());
        assert!(SettingNamespace::Privacy.supports_pseud());
        assert!(SettingNamespace::Content.supports_account());
    }

    #[test]
    fn unknown_namespace_rejected() {
        assert!("telepathy".parse::<SettingNamespace>().is_err());
    }
}
