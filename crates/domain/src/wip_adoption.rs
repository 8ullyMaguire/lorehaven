//! §56 — WIP adoption, closure notes and lineage. The rules, without a database.
//!
//! Three clauses, and two of them are prohibitions, which is the hard part: an implementation
//! that adds a prohibition correctly is mostly an implementation that does not add the thing.
//!
//! - **§56.1 ownership never moves.** `works.owner_pseud_id` is not written by any function
//!   here, and there is no route that can write it. The adoption is recorded on the offer row
//!   and the adopter appears as a contributor.
//! - **§56.2 a closure note is optional.** Nothing in this module makes a note a precondition
//!   of anything: `next_revision(None)` is 1, and a work with no note is a `None` the caller
//!   asked for, not a refusal.
//! - **§56.3 lineage is derived.** [`Contributor`] carries `first_revision_at`, and
//!   [`order_contributors`] is the one place that decides the order, so the rule cannot be
//!   changed by editing an `ORDER BY` someone copied.

/// Where an offer stands. `Withdrawn` and `Adopted` both keep the row: an offer is evidence of
/// a past act, not a toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferStatus {
    Open,
    Withdrawn,
    Adopted,
}

impl OfferStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            OfferStatus::Open => "open",
            OfferStatus::Withdrawn => "withdrawn",
            OfferStatus::Adopted => "adopted",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "open" => Some(OfferStatus::Open),
            "withdrawn" => Some(OfferStatus::Withdrawn),
            "adopted" => Some(OfferStatus::Adopted),
            _ => None,
        }
    }
}

/// Why an offer could not be opened. Failure-only on purpose: a `Result<Offer, OpenRefusal>`
/// where the error enum also had an `Opened` variant would carry a state the success arm already
/// represents, and every `match` on it would need an `unreachable!()` arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenRefusal {
    /// The caller does not own the work. §56.1's first sentence: the offer is the author's.
    NotOwner,
    /// §56.1: only a work marked `abandoned` may be offered. `hiatus` is an author who means
    /// to come back, and `complete` has nothing to continue.
    NotAbandoned,
    /// One open offer per work; a second would be two answers to one question.
    AlreadyOpen,
}

/// Why an adoption could not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdoptionRefusal {
    NotOpen,
    /// The work stopped being adoptable between the offer and the adoption.
    NotAbandoned,
    /// §56.1: adoption is a publishing act and is gated like one. Both numbers are reported so
    /// the refusal can name the bar rather than say "forbidden".
    BelowBar { bar: i64, caller: i64 },
}

/// Why a withdrawal could not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WithdrawalRefusal {
    NotTheAuthor,
    NotOpen,
}

/// §56.1: adoption requires the trust level that publishing requires.
///
/// No such level exists in the schema — `trust_levels` is `(account, level, computed_at, basis)`
/// with no seed and no named capabilities, and this tree has no publish route to borrow one
/// from. So the bar is a configuration value (`[adoption] min_trust_level`) rather than a
/// constant here, and this function exists to make the comparison one line that a test can
/// point at.
pub fn may_adopt(min_trust_level: i64, caller_trust: i64) -> bool {
    caller_trust >= min_trust_level
}

/// §56.3: the reader-facing contributor list is ordered by **first contribution**.
///
/// Ascending, deliberately: "most active first" is how an adopter of chapter nine outranks the
/// person who wrote chapter one. Ties fall back to the pseud id so two contributors who arrived
/// in the same second still order the same way on every instance.
pub fn order_contributors(mut list: Vec<Contributor>) -> Vec<Contributor> {
    list.sort_by(|a, b| {
        a.first_revision_at
            .cmp(&b.first_revision_at)
            .then_with(|| a.pseud_id.cmp(&b.pseud_id))
    });
    list
}

/// §56.2: the next revision number for a closure note. No previous note is revision 1, not 0
/// and not an error — a work may conclude with no note at all.
pub fn next_revision(current: Option<i64>) -> i64 {
    current.map(|n| n + 1).unwrap_or(1)
}

/// One person who has written part of a work, derived from revisions (§56.3).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Contributor {
    pub pseud_id: String,
    pub handle: String,
    /// The earliest revision by this pseud — the ordering key.
    pub first_revision_at: String,
    pub revisions: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(pseud: &str, at: &str, revisions: i64) -> Contributor {
        Contributor {
            pseud_id: pseud.to_string(),
            handle: pseud.to_string(),
            first_revision_at: at.to_string(),
            revisions,
        }
    }

    #[test]
    fn the_trust_bar_is_inclusive() {
        assert!(may_adopt(2, 2), "exactly at the bar may adopt");
        assert!(may_adopt(2, 5));
        assert!(!may_adopt(2, 1));
        assert!(may_adopt(0, 0), "a bar of zero lets everyone in");
    }

    #[test]
    fn the_adopter_of_chapter_nine_does_not_leapfrog_chapter_one() {
        let original = c("original", "2026-01-01T00:00:00Z", 9);
        let adopter = c("adopter", "2026-09-01T00:00:00Z", 12);
        let ordered = order_contributors(vec![adopter, original]);
        assert_eq!(
            ordered[0].pseud_id, "original",
            "first contribution orders the list, not volume"
        );
        assert_eq!(ordered[1].pseud_id, "adopter");
    }

    #[test]
    fn contributors_who_arrived_together_order_by_pseud() {
        let a = c("bbb", "2026-01-01T00:00:00Z", 1);
        let b = c("aaa", "2026-01-01T00:00:00Z", 1);
        let ordered = order_contributors(vec![a, b]);
        assert_eq!(ordered[0].pseud_id, "aaa");
        assert_eq!(ordered[1].pseud_id, "bbb");
    }

    #[test]
    fn a_missing_closure_note_is_revision_one_not_an_error() {
        assert_eq!(next_revision(None), 1);
        assert_eq!(next_revision(Some(1)), 2);
        assert_eq!(next_revision(Some(7)), 8);
    }

    #[test]
    fn the_status_vocabulary_round_trips_and_keeps_its_check_constraint_honest() {
        for status in [
            OfferStatus::Open,
            OfferStatus::Withdrawn,
            OfferStatus::Adopted,
        ] {
            assert_eq!(OfferStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(OfferStatus::parse("pending"), None);
    }
}
