// M47: User Configuration — resolution hierarchy and namespace rules (spec §46).
//
// Settings resolve: context override → pseud → account → instance.
// Per-domain tables (never JSONB blobs). Unknown keys rejected.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::AppError;

/// A recognized setting key: its namespace, default value (as a JSON literal),
/// and a human-readable summary.
#[derive(Debug, Clone)]
pub struct SettingKeyDef {
    pub key: &'static str,
    pub namespace: SettingNamespace,
    pub default_json: &'static str,
    pub summary: &'static str,
}

/// The canonical registry of recognized setting keys (spec §46.3).
pub const SETTING_KEYS: &[SettingKeyDef] = &[
    SettingKeyDef {
        key: "search.default_sort",
        namespace: SettingNamespace::Search,
        default_json: r#""updated""#,
        summary: "Default ordering for search results",
    },
    SettingKeyDef {
        key: "search.default_scope",
        namespace: SettingNamespace::Search,
        default_json: r#""works""#,
        summary: "Which entity type the search targets by default",
    },
    SettingKeyDef {
        key: "search.results_per_page",
        namespace: SettingNamespace::Search,
        default_json: "25",
        summary: "How many results to return per search page",
    },
    SettingKeyDef {
        key: "reader.font_family",
        namespace: SettingNamespace::Reader,
        default_json: r#""system""#,
        summary: "Font used in the reading view",
    },
    SettingKeyDef {
        key: "reader.font_size",
        namespace: SettingNamespace::Reader,
        default_json: r#""medium""#,
        summary: "Base font size in the reading view",
    },
    SettingKeyDef {
        key: "reader.line_height",
        namespace: SettingNamespace::Reader,
        default_json: r#""comfortable""#,
        summary: "Spacing between lines in the reading view",
    },
    SettingKeyDef {
        key: "reader.theme",
        namespace: SettingNamespace::Reader,
        default_json: r#""auto""#,
        summary: "Color theme in the reading view",
    },
    SettingKeyDef {
        key: "appearance.theme",
        namespace: SettingNamespace::Appearance,
        default_json: r#""system""#,
        summary: "Site-wide color theme",
    },
    SettingKeyDef {
        key: "appearance.compact_density",
        namespace: SettingNamespace::Appearance,
        default_json: "false",
        summary: "Reduce spacing and element size across the site",
    },
    SettingKeyDef {
        key: "discovery.recs_personalized",
        namespace: SettingNamespace::Discovery,
        default_json: "true",
        summary: "Use reading history to personalize recommendations",
    },
    SettingKeyDef {
        key: "discovery.recs_blend",
        namespace: SettingNamespace::Discovery,
        default_json: r#""balanced""#,
        summary: "Mix of familiar versus exploratory recommendations",
    },
];

/// Look up a recognized key definition.
pub fn key_def(key: &str) -> Option<&'static SettingKeyDef> {
    SETTING_KEYS.iter().find(|def| def.key == key)
}

/// The instance-level default for a recognized key, parsed from its JSON literal.
pub fn default_value(key: &str) -> Option<serde_json::Value> {
    key_def(key)
        .map(|def| serde_json::from_str(def.default_json).expect("invalid default_json in registry"))
}

/// The namespace a key belongs to.
pub fn namespace_of(key: &str) -> Option<SettingNamespace> {
    key_def(key).map(|def| def.namespace)
}

/// Where a setting value comes from.
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
    pub summary: String,
}

/// Resolve a setting value through the hierarchy: context → pseud → account → instance.
pub fn resolve_setting(
    key: &str,
    context: Option<&serde_json::Value>,
    pseud: Option<&serde_json::Value>,
    account: Option<&serde_json::Value>,
) -> Result<ResolvedSetting, AppError> {
    let def = key_def(key).ok_or_else(|| AppError::Validation {
        message: format!("unknown setting key: {key}"),
        field_errors: Default::default(),
    })?;

    if let Some(value) = context {
        return Ok(ResolvedSetting {
            key: key.to_owned(),
            value: value.clone(),
            source: SettingSource::Context,
            summary: def.summary.to_owned(),
        });
    }
    if let Some(value) = pseud {
        return Ok(ResolvedSetting {
            key: key.to_owned(),
            value: value.clone(),
            source: SettingSource::Pseud,
            summary: def.summary.to_owned(),
        });
    }
    if let Some(value) = account {
        return Ok(ResolvedSetting {
            key: key.to_owned(),
            value: value.clone(),
            source: SettingSource::Account,
            summary: def.summary.to_owned(),
        });
    }
    let default = default_value(key).expect("registry default parses");
    Ok(ResolvedSetting {
        key: key.to_owned(),
        value: default,
        source: SettingSource::Instance,
        summary: def.summary.to_owned(),
    })
}

/// Resolved settings export document (spec §46.6).
#[derive(Debug, Serialize)]
pub struct SettingsExport {
    pub version: u32,
    pub exported_at: String,
    pub namespaces: HashMap<String, Vec<ResolvedSetting>>,
}

pub const SETTINGS_EXPORT_VERSION: u32 = 1;

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

/// Namespace registry.
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

    #[test]
    fn registry_has_no_duplicate_keys() {
        let mut seen = std::collections::HashSet::new();
        for def in SETTING_KEYS {
            assert!(seen.insert(def.key), "duplicate key: {}", def.key);
        }
    }

    #[test]
    fn every_registry_key_resolves_to_instance_default() {
        for def in SETTING_KEYS {
            let resolved = resolve_setting(def.key, None, None, None).unwrap();
            assert_eq!(resolved.source, SettingSource::Instance);
            let expected: serde_json::Value = serde_json::from_str(def.default_json).unwrap();
            assert_eq!(resolved.value, expected);
        }
    }

    #[test]
    fn resolve_honors_precedence_chain() {
        let r = resolve_setting("reader.font_family", None, None, None).unwrap();
        assert_eq!(r.source, SettingSource::Instance);

        let pseud = serde_json::json!("pseud-value");
        let account = serde_json::json!("account-value");
        let r = resolve_setting("reader.font_family", None, Some(&pseud), Some(&account)).unwrap();
        assert_eq!(r.source, SettingSource::Pseud);

        let context = serde_json::json!("context-value");
        let pseud = serde_json::json!("pseud-value");
        let account = serde_json::json!("account-value");
        let r =
            resolve_setting("reader.font_family", Some(&context), Some(&pseud), Some(&account))
                .unwrap();
        assert_eq!(r.source, SettingSource::Context);
        assert_eq!(r.value, "context-value");
    }

    #[test]
    fn resolve_rejects_unknown_key() {
        assert!(resolve_setting("telepathy.mode", None, None, None).is_err());
    }
}
