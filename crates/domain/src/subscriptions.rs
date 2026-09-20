//! Content subscriptions and saved-search alerts (spec §23.3, §14.2) — pure
//! rules, no I/O.
//!
//! Skeleton: signatures are the contract; the implementing agent fills the
//! bodies marked [`Rules::todo`].

/// What a reader may subscribe to (§23.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject {
    Work,
    Series,
    Collection,
    Fandom,
    Author,
}

impl Subject {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "work" => Some(Self::Work),
            "series" => Some(Self::Series),
            "collection" => Some(Self::Collection),
            "fandom" => Some(Self::Fandom),
            "author" => Some(Self::Author),
            _ => None,
        }
    }
}

pub struct Rules;

impl Rules {
    /// A subscription never exposes the subscriber: the author sees a count,
    /// never a list (§23.3). Exists so notification code has a rule to call.
    pub fn subscriber_list_visible() -> bool {
        false
    }

    /// Alerts run the saved query with the reader's own permissions at run
    /// time, so a match that has become ineligible is not reported (§14.2).
    /// The skeleton pins the decision shape; the db layer enforces it.
    pub fn alert_match_eligible(viewer_account: &str, match_owner_account: &str) -> bool {
        !viewer_account.is_empty() && !match_owner_account.is_empty()
    }

    /// Alerts are bounded in frequency — daily by default (§14.2).
    pub fn alert_due(frequency: &str, last_run_epoch: i64, now_epoch: i64) -> bool {
        let period: i64 = match frequency {
            "daily" => 86_400,
            "weekly" => 604_800,
            _ => 86_400,
        };
        now_epoch - last_run_epoch >= period
    }

    /// Nobody, including the author of a matched work, is told a view
    /// matched (§14.2). A rule for callers, not a comment.
    pub fn match_activity_disclosed() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscriber_lists_are_never_visible() {
        assert!(!Rules::subscriber_list_visible());
    }

    #[test]
    fn alerts_respect_their_frequency_bound() {
        assert!(!Rules::alert_due("daily", 0, 86_399));
        assert!(Rules::alert_due("daily", 0, 86_400));
        assert!(Rules::alert_due("weekly", 0, 604_800));
    }

    #[test]
    fn matching_is_never_disclosed_to_authors() {
        assert!(!Rules::match_activity_disclosed());
    }

    #[test]
    fn subjects_parse() {
        assert_eq!(Subject::parse("fandom"), Some(Subject::Fandom));
        assert_eq!(Subject::parse("podcast"), None);
    }
}
