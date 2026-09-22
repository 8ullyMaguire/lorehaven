/// Shared browse ordering vocabulary (spec §43).
///
/// One `Sort` enum serves every browse surface from `/discover` to the directory.
/// A surface that does not currently take `?sort=` keeps behaving as before;
/// a surface that does, parses the value through [`Sort::parse`] and refuses
/// anything outside the set with a named error.
use serde::{Deserialize, Serialize};

/// The ordering a reader has asked for (spec §43.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Sort {
    /// The §16 engine blend and the reader's recipe; a permutation, never a filter.
    ForYou,
    /// Publication or update event order, from the §4.3 event log.
    New,
    /// Last publication event.
    Updated,
    /// §15.10's quality signals: completion rate, positive-feedback ratio,
    /// recency, update velocity, bibliography quality.
    Top,
    /// Time-windowed with §16.14's decay.
    Trending,
    /// Literal query relevance; search only; never taste-steered (§15.10).
    BestMatch,
    /// Alphabetical; people, tags, collections.
    Az,
}

impl Sort {
    /// All accepted values, for error messages and validation.
    pub const ALL: &'static [Self] = &[
        Self::ForYou,
        Self::New,
        Self::Updated,
        Self::Top,
        Self::Trending,
        Self::BestMatch,
        Self::Az,
    ];

    /// Parse from a query string, returning `None` for an unknown value.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "for-you" => Some(Self::ForYou),
            "new" => Some(Self::New),
            "updated" => Some(Self::Updated),
            "top" => Some(Self::Top),
            "trending" => Some(Self::Trending),
            "best-match" => Some(Self::BestMatch),
            "az" => Some(Self::Az),
            _ => None,
        }
    }

    /// The stored form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ForYou => "for-you",
            Self::New => "new",
            Self::Updated => "updated",
            Self::Top => "top",
            Self::Trending => "trending",
            Self::BestMatch => "best-match",
            Self::Az => "az",
        }
    }

    /// The set of values this surface accepts, as a comma-separated list
    /// for error messages.
    #[must_use]
    pub fn accepted_set() -> String {
        Self::ALL
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Whether this sort is taste-steered (i.e. influenced by the reader's profile).
    #[must_use]
    pub const fn is_taste_steered(self) -> bool {
        matches!(self, Self::ForYou | Self::Trending)
    }

    /// Whether this sort is exact (i.e. must not be reordered by any profile).
    #[must_use]
    pub const fn is_exact(self) -> bool {
        matches!(
            self,
            Self::New | Self::Updated | Self::Top | Self::BestMatch | Self::Az
        )
    }
}

impl Default for Sort {
    fn default() -> Self {
        Self::New
    }
}

impl std::str::FromStr for Sort {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
            .ok_or_else(|| format!("unknown sort `{s}`; accepted: {}", Self::accepted_set()))
    }
}

impl std::fmt::Display for Sort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_all_values() {
        for sort in Sort::ALL {
            assert_eq!(Sort::parse(sort.as_str()), Some(*sort));
        }
    }

    #[test]
    fn parse_unknown_returns_none() {
        assert!(Sort::parse("random").is_none());
        assert!(Sort::parse("").is_none());
        assert!(Sort::parse("for_you").is_none());
    }

    #[test]
    fn from_str_names_accepted_set() {
        let s: Result<Sort, String> = "bogus".parse();
        assert!(s.is_err());
        let msg = s.unwrap_err();
        assert!(msg.contains("for-you"), "error should list for-you: {msg}");
        assert!(msg.contains("new"), "error should list new: {msg}");
        assert!(msg.contains("az"), "error should list az: {msg}");
    }

    #[test]
    fn taste_steered_flag() {
        assert!(Sort::ForYou.is_taste_steered());
        assert!(Sort::Trending.is_taste_steered());
        assert!(!Sort::New.is_taste_steered());
        assert!(!Sort::Top.is_taste_steered());
        assert!(!Sort::BestMatch.is_taste_steered());
    }

    #[test]
    fn exact_flag() {
        assert!(Sort::New.is_exact());
        assert!(Sort::Updated.is_exact());
        assert!(Sort::Top.is_exact());
        assert!(Sort::BestMatch.is_exact());
        assert!(Sort::Az.is_exact());
        assert!(!Sort::ForYou.is_exact());
    }

    #[test]
    fn display_round_trip() {
        for sort in Sort::ALL {
            let s = sort.to_string();
            let parsed: Sort = s.parse().unwrap();
            assert_eq!(*sort, parsed);
        }
    }

    #[test]
    fn serde_round_trip() {
        for sort in Sort::ALL {
            let json = serde_json::to_string(sort).unwrap();
            let parsed: Sort = serde_json::from_str(&json).unwrap();
            assert_eq!(*sort, parsed);
        }
    }
}
