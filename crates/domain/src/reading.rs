//! Reading position, progress resolution and reading-time estimates (spec §9).
//!
//! Spec §9.3 is the rule that shapes this module: "When devices disagree,
//! present a choice rather than always taking the furthest position." That is a
//! *decision about what to show a reader*, so it belongs here, in a pure
//! function, rather than in whichever handler happens to load the rows first —
//! a second handler that took the furthest position silently would be exactly
//! the kind of "subtly different check" the project's history warns about.
//!
//! Everything here is pure: no database, no clock, no I/O. Callers load facts
//! and hand them in.

use crate::ids::RevisionId;

/// A stored position within a subject.
///
/// `fraction` is permille (0..=1000) rather than a fraction, because the crate
/// binds only `String`/`i64` to the database and a `REAL` column would need a
/// second bind path. A thousandths precision is enough for "where was I" and is
/// what the `position_permille` column stores.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReadingPosition {
    /// The revision the reader was looking at, when one was recorded.
    pub revision: Option<RevisionId>,
    /// A stable paragraph anchor, when one was recorded.
    pub anchor: Option<String>,
    /// Position in the content, in permille (0..=1000).
    pub fraction: u16,
    /// A per-device identifier, when one was recorded.
    pub device: Option<String>,
}

impl ReadingPosition {
    /// Whether two positions point at the same revision.
    ///
    /// Anchors are revision-specific: a position that was recorded against an
    /// older revision cannot be honoured against a newer one, even if the
    /// fraction is the same, because the text in between may have changed.
    #[must_use]
    pub fn same_revision(&self, other: &Self) -> bool {
        self.revision.is_some() && self.revision == other.revision
    }
}

/// What to do when a reader has more than one stored position for a subject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgressResolution {
    /// One position, or several that agree: use it.
    UseStored(ReadingPosition),
    /// Several that disagree: the reader must choose.
    AskTheReader {
        /// The device's own stored position, if any.
        mine: Option<ReadingPosition>,
        /// The other device's position, if any.
        other: Option<ReadingPosition>,
    },
    /// Nothing stored at all.
    NoPosition,
}

/// Resolve a reader's stored positions into a single decision.
///
/// Spec §9.3: one position uses it; several that agree use the (shared)
/// value; several that disagree ask the reader.
pub fn resolve_progress(rows: &[ReadingPosition]) -> ProgressResolution {
    match rows {
        [] => ProgressResolution::NoPosition,
        [single] => ProgressResolution::UseStored(single.clone()),
        other => {
            let first = &other[0];
            if other
                .iter()
                .all(|row| row.same_revision(first) && row.fraction == first.fraction)
            {
                ProgressResolution::UseStored(first.clone())
            } else {
                // The reader's own device is identified by the most recent
                // device id present; if there is none, fall back to the first.
                let mine = other
                    .iter()
                    .find(|row| row.device.is_some())
                    .cloned()
                    .or_else(|| Some(first.clone()));
                let other_position = other
                    .iter()
                    .find(|row| !row.same_revision(first) || row.fraction != first.fraction)
                    .cloned();
                ProgressResolution::AskTheReader {
                    mine,
                    other: other_position,
                }
            }
        }
    }
}

/// Whether a stored position still points at content the reader saw.
///
/// A position recorded against a revision that no longer resolves is not
/// reliable: the anchor or fraction would point at text that has since been
/// rewritten (ADR 0002's append-only revisions make this the common case).
#[must_use]
pub fn position_is_reliable(position: &ReadingPosition, current_revision: RevisionId) -> bool {
    position
        .revision
        .is_some_and(|revision| revision == current_revision)
}

/// An estimate of how long a text takes to read.
///
/// The figure is an *assumption*, not a measurement: 200 words per minute,
/// which is the commonly cited average for silent English prose reading. It is
/// stated here, next to the constant, so it can be found and revisited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadingTime {
    /// Whole minutes; `0` for an empty text.
    pub minutes: u32,
}

/// Words per minute used by [`estimate_reading_time`].
///
/// A named constant rather than a literal, so the assumption lives in one place
/// and is documented as such (spec §3.2 requires documented interpretation for
/// statistical estimates).
pub const WORDS_PER_MINUTE: u32 = 200;

/// Estimate the reading time of a word count.
///
/// Empty text is `0 minutes`; anything else is rounded up to one minute and
/// then rounded to whole minutes above that, so a one-word story is not
/// reported as "instant".
#[must_use]
pub fn estimate_reading_time(word_count: u32) -> ReadingTime {
    if word_count == 0 {
        return ReadingTime { minutes: 0 };
    }
    let minutes = (word_count + WORDS_PER_MINUTE - 1) / WORDS_PER_MINUTE;
    ReadingTime {
        minutes: minutes.max(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position(revision: &str, fraction: u16) -> ReadingPosition {
        ReadingPosition {
            revision: Some(revision.parse().unwrap()),
            anchor: None,
            fraction,
            device: Some("device-a".to_owned()),
        }
    }

    #[test]
    fn a_single_position_is_used_directly() {
        let only = position("00000000-0000-0000-0000-000000000011", 500);
        assert_eq!(
            resolve_progress(&[only.clone()]),
            ProgressResolution::UseStored(only)
        );
    }

    #[test]
    fn several_agreeing_positions_are_used_directly() {
        let a = position("00000000-0000-0000-0000-000000000011", 500);
        let b = ReadingPosition {
            revision: a.revision,
            anchor: None,
            fraction: 500,
            device: Some("device-b".to_owned()),
        };
        assert_eq!(
            resolve_progress(&[a.clone(), b]),
            ProgressResolution::UseStored(a)
        );
    }

    #[test]
    fn disagreeing_positions_ask_the_reader() {
        let mine = position("00000000-0000-0000-0000-000000000011", 100);
        let theirs = ReadingPosition {
            revision: mine.revision,
            anchor: None,
            fraction: 900,
            device: Some("device-b".to_owned()),
        };
        assert_eq!(
            resolve_progress(&[mine.clone(), theirs.clone()]),
            ProgressResolution::AskTheReader {
                mine: Some(mine),
                other: Some(theirs),
            }
        );
    }

    #[test]
    fn positions_on_different_revisions_disagree() {
        let mine = position("00000000-0000-0000-0000-000000000011", 500);
        let theirs = position("00000000-0000-0000-0000-000000000022", 500);
        assert_eq!(
            resolve_progress(&[mine.clone(), theirs.clone()]),
            ProgressResolution::AskTheReader {
                mine: Some(mine),
                other: Some(theirs),
            }
        );
    }

    #[test]
    fn no_positions_is_a_choice_with_no_options() {
        assert_eq!(resolve_progress(&[]), ProgressResolution::NoPosition);
    }

    #[test]
    fn a_stale_position_is_not_reliable() {
        let current: RevisionId = "00000000-0000-0000-0000-000000000022".parse().unwrap();
        let position = ReadingPosition {
            revision: Some("00000000-0000-0000-0000-000000000011".parse().unwrap()),
            anchor: None,
            fraction: 500,
            device: None,
        };
        assert!(!position_is_reliable(&position, current));

        let current_position = ReadingPosition {
            revision: Some(current),
            anchor: None,
            fraction: 500,
            device: None,
        };
        assert!(position_is_reliable(&current_position, current));
    }

    #[test]
    fn reading_time_rounds_up_to_one_minute_and_then_to_whole_minutes() {
        assert_eq!(estimate_reading_time(0).minutes, 0);
        assert_eq!(estimate_reading_time(1).minutes, 1);
        assert_eq!(estimate_reading_time(199).minutes, 1);
        assert_eq!(estimate_reading_time(200).minutes, 1);
        assert_eq!(estimate_reading_time(1000).minutes, 5);
    }

    #[test]
    fn reading_time_uses_the_documented_words_per_minute() {
        assert_eq!(WORDS_PER_MINUTE, 200);
    }
}
