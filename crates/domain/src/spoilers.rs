/// Spoiler, warning, and readability types (spec §35.4).
use std::fmt;

/// A content warning type (shares vocabulary with §15.16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WarningType {
    Violence,
    SexualContent,
    SelfHarm,
    Spoilers,
    Custom,
}

impl WarningType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Violence => "violence",
            Self::SexualContent => "sexual_content",
            Self::SelfHarm => "self_harm",
            Self::Spoilers => "spoilers",
            Self::Custom => "custom",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "violence" => Some(Self::Violence),
            "sexual_content" => Some(Self::SexualContent),
            "self_harm" => Some(Self::SelfHarm),
            "spoilers" => Some(Self::Spoilers),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }

    pub const ALL: [Self; 5] = [
        Self::Violence,
        Self::SexualContent,
        Self::SelfHarm,
        Self::Spoilers,
        Self::Custom,
    ];
}

impl fmt::Display for WarningType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A reader's preferred action for a warning type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningAction {
    Blur,
    Show,
}

impl WarningAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Blur => "blur",
            Self::Show => "show",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "blur" => Some(Self::Blur),
            "show" => Some(Self::Show),
            _ => None,
        }
    }
}

/// Severity of a content warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningSeverity {
    Light,
    Heavy,
}

impl WarningSeverity {
    pub fn as_i64(&self) -> i64 {
        match self {
            Self::Light => 1,
            Self::Heavy => 2,
        }
    }

    pub fn from_i64(n: i64) -> Option<Self> {
        match n {
            1 => Some(Self::Light),
            2 => Some(Self::Heavy),
            _ => None,
        }
    }
}

/// Whether a post's content should be hidden from a reader based on spoiler
/// scope and the reader's progress.
pub struct SpoilerCheck {
    /// True if the post is a spoiler for this reader.
    pub is_spoiler: bool,
    /// The reason: "topic_scope", "content_warning", or "none".
    pub reason: &'static str,
}

impl SpoilerCheck {
    pub fn none() -> Self {
        Self {
            is_spoiler: false,
            reason: "none",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_display() {
        assert_eq!(WarningType::Spoilers.to_string(), "spoilers");
        assert_eq!(WarningAction::Blur.to_string(), "blur");
    }

    #[test]
    fn test_round_trip() {
        for wt in WarningType::ALL {
            let s = wt.as_str();
            let parsed = WarningType::from_str(s).unwrap();
            assert_eq!(parsed, wt);
        }
    }

    #[test]
    fn test_severity() {
        assert_eq!(WarningSeverity::Light.as_i64(), 1);
        assert_eq!(WarningSeverity::Heavy.as_i64(), 2);
        assert_eq!(WarningSeverity::from_i64(1), Some(WarningSeverity::Light));
        assert_eq!(WarningSeverity::from_i64(2), Some(WarningSeverity::Heavy));
        assert_eq!(WarningSeverity::from_i64(3), None);
    }
}
