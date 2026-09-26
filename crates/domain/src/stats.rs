//! M19 — Statistics domain: k-anonymity, aggregation definitions, gap reporting.
//!
//! # Moved
//!
//! The floor now lives in [`crate::analytics`], which has two of them: 5 for
//! aggregates about the viewer, 10 for aggregates about anyone else (§36.12).
//! This module keeps the M19 name and entry points so existing callers still
//! compile, but it no longer owns the constant or the logic.
//!
//! # A bug this consolidation fixed
//!
//! The old `apply_k_anonymity` returned `(String, i64)` — a coarsened *key*
//! alongside the **true count**. A caller that respected the key and ignored
//! the count was safe; a caller that logged or returned the tuple leaked the
//! exact small number it had just hidden. Returning the pair made the safe
//! path and the leaking path look equally reasonable.
//!
//! The count returned now is the *coarsened* one, so the small case is zero
//! rather than the truth, and [`report`] gives callers a form that cannot be
//! mistaken for a number at all.

use crate::analytics::{coarsen, floor_for, Coarsened, Reported, Subject, K_SELF};

/// Minimum count threshold for per-entity statistics (k-anonymity).
///
/// M19's original single floor, kept for callers that have not yet been split
/// by subject. New code should use [`K_SELF`] or [`K_OTHERS`] and say which it
/// means: a floor of 5 about other people is a disclosure.
pub const K_ANON_FLOOR: i64 = K_SELF;

/// Apply k-anonymity floor: counts below the floor are merged into "other".
pub fn apply_k_anonymity(key: &str, count: i64) -> (String, i64) {
    match coarsen(count, K_ANON_FLOOR) {
        Coarsened::Exact(n) => (key.to_string(), n),
        Coarsened::Below(_) => ("other".to_string(), 0),
    }
}

/// The wire form of a count, with the floor for `subject` applied.
pub fn report(count: i64, subject: Subject) -> Reported {
    match coarsen(count, floor_for(subject)) {
        Coarsened::Exact(n) => Reported::Exact(n),
        Coarsened::Below(f) => Reported::BelowFloor { fewer_than: f },
    }
}

/// Check whether a statistic can be computed (has sufficient data).
pub fn stat_available(count: i64) -> bool {
    count >= K_ANON_FLOOR
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analytics::K_OTHERS;

    #[test]
    fn k_anonymity_merges_small_counts() {
        let (key, count) = apply_k_anonymity("work-123", 3);
        assert_eq!(key, "other");
        // The old function returned 3 here, which is the leak this replaced.
        assert_eq!(count, 0);
    }

    #[test]
    fn k_anonymity_keeps_large_counts() {
        let (key, count) = apply_k_anonymity("work-123", 10);
        assert_eq!(key, "work-123");
        assert_eq!(count, 10);
    }

    #[test]
    fn stat_available_at_floor() {
        assert!(stat_available(K_ANON_FLOOR));
        assert!(!stat_available(K_ANON_FLOOR - 1));
    }

    #[test]
    fn the_two_floors_are_not_interchangeable() {
        // A floor of 5 about other people is a disclosure; a floor of 10 about
        // your own re-reads hides ordinary reading. Both are exported so that
        // a caller has to choose rather than inherit.
        assert_eq!(K_SELF, 5);
        assert_eq!(K_OTHERS, 10);
        assert_eq!(floor_for(Subject::Self_), K_SELF);
        assert_eq!(floor_for(Subject::Other), K_OTHERS);
    }

    #[test]
    fn reporting_about_other_people_uses_the_stricter_floor() {
        // The same 7 readers: a number in your own dashboard, and a suppression
        // in someone else's.
        assert_eq!(
            report(7, Subject::Other),
            Reported::BelowFloor { fewer_than: 10 }
        );
        assert_eq!(report(7, Subject::Self_), Reported::Exact(7));
    }

    #[test]
    fn a_suppressed_count_is_not_a_number_anywhere_in_its_wire_form() {
        // Serialising must not produce a bare 7 that a client renders as "7".
        let json = serde_json::to_string(&report(7, Subject::Other)).unwrap();
        assert!(json.contains("fewer_than"), "{json}");
        assert!(!json.contains("7,"), "the true count survived: {json}");

        let exact = serde_json::to_string(&report(7, Subject::Self_)).unwrap();
        assert_eq!(exact, "7");
    }
}
