//! Structured DNF (did-not-finish) reasons for abandoned works (spec M45-21).

use serde::{Deserialize, Serialize};

/// A structured reason a reader stopped reading a work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DnfReason {
    /// The reader's taste changed or the work was not for them.
    NotMyTaste,
    /// The work contains content the reader finds triggering.
    Triggering,
    /// The pacing felt too slow to continue.
    SlowPacing,
    /// The author appears to have abandoned the work.
    AbandonedByAuthor,
    /// The reader dropped it for another reason not covered here.
    DroppedOther,
    /// The reader chose a free-text reason.
    Other,
}

impl DnfReason {
    /// The canonical string form, for APIs and SQLite columns.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotMyTaste => "not_my_taste",
            Self::Triggering => "triggering",
            Self::SlowPacing => "slow_pacing",
            Self::AbandonedByAuthor => "abandoned_by_author",
            Self::DroppedOther => "dropped_other",
            Self::Other => "other",
        }
    }

    /// A human-readable label for the UI.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::NotMyTaste => "Not my taste",
            Self::Triggering => "Triggering content",
            Self::SlowPacing => "Too slow",
            Self::AbandonedByAuthor => "Author abandoned it",
            Self::DroppedOther => "Dropped for another reason",
            Self::Other => "Other",
        }
    }
}

impl std::fmt::Display for DnfReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for DnfReason {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "not_my_taste" => Ok(Self::NotMyTaste),
            "triggering" => Ok(Self::Triggering),
            "slow_pacing" => Ok(Self::SlowPacing),
            "abandoned_by_author" => Ok(Self::AbandonedByAuthor),
            "dropped_other" => Ok(Self::DroppedOther),
            "other" => Ok(Self::Other),
            other => Err(format!("unknown DNF reason: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_reasons() -> [DnfReason; 6] {
        [
            DnfReason::NotMyTaste,
            DnfReason::Triggering,
            DnfReason::SlowPacing,
            DnfReason::AbandonedByAuthor,
            DnfReason::DroppedOther,
            DnfReason::Other,
        ]
    }

    #[test]
    fn each_reason_roundtrips_through_its_canonical_string() {
        for reason in all_reasons() {
            let parsed: DnfReason = reason.as_str().parse().expect("parses");
            assert_eq!(parsed, reason);
        }
    }

    #[test]
    fn unknown_reason_strings_are_rejected() {
        assert!("not_a_reason".parse::<DnfReason>().is_err());
    }

    #[test]
    fn labels_are_human_readable() {
        for reason in all_reasons() {
            let label = reason.label();
            assert!(!label.is_empty());
            assert!(
                label.chars().any(|c| c.is_ascii_lowercase()),
                "labels should be sentence case"
            );
        }
    }

    #[test]
    fn display_matches_canonical_string() {
        for reason in all_reasons() {
            assert_eq!(reason.to_string(), reason.as_str());
        }
    }
}
