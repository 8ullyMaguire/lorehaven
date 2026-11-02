//! §60 — finisher signals: update cadence and completion history, positive framing only.
//!
//! The gaps review (C5) asks for two numbers an author can be proud of and one rule the archive
//! must keep: *"positive framing only; never label non-finishers"*.
//!
//! That rule is why [`History`] has **no `abandoned` field**. The count exists inside
//! [`history`] to compute `concluded`, and stops there: a public payload with a field named
//! `abandoned` is a label, and a label is the thing the sentence forbids. What the surface says is
//! "finished 9 of 10 concluded works" — the review's own example — which is a statement about
//! finishing rather than a mark against the one that stopped.
//!
//! Cadence is the harder half, because a cadence can be **unmeasurable** and saying so is part of
//! being honest: an import writes every revision on the day it ran, so a gap between revisions is
//! the import's rhythm and not the author's. [`cadence`] answers [`Cadence::Unmeasured`] with a
//! reason in that case rather than inventing a number.

/// How often this work (or this author's works) actually gained material.
///
/// Serialised as `{"measurable": true, "median_days": …, "samples": …}` or
/// `{"measurable": false, "reason": "…"}` — one object shape with a real boolean, hand-written
/// because `#[serde(tag)]` renames the tag's *values* into strings and a client would then have to
/// know that `"false"` is false.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cadence {
    Measurable {
        /// Median gap in whole days between the days a revision landed.
        median_days: i64,
        /// How many gaps the median was taken over — a one-gap median is a fact about two days.
        samples: usize,
    },
    Unmeasured {
        /// Why this cannot be answered here. Always a sentence a reader can act on.
        reason: String,
    },
}

impl serde::Serialize for Cadence {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        match self {
            Cadence::Measurable {
                median_days,
                samples,
            } => {
                let mut state = serializer.serialize_struct("Cadence", 3)?;
                state.serialize_field("measurable", &true)?;
                state.serialize_field("median_days", median_days)?;
                state.serialize_field("samples", samples)?;
                state.end()
            }
            Cadence::Unmeasured { reason } => {
                let mut state = serializer.serialize_struct("Cadence", 2)?;
                state.serialize_field("measurable", &false)?;
                state.serialize_field("reason", reason)?;
                state.end()
            }
        }
    }
}

impl Cadence {
    pub fn is_measurable(&self) -> bool {
        matches!(self, Cadence::Measurable { .. })
    }
}

/// Completion history: what was finished, out of what has been concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct History {
    /// Works whose `completion` is `complete`.
    pub finished: i64,
    /// Works whose `completion` is `complete` or `abandoned` — the denominator of "9 of 10".
    pub concluded: i64,
}

/// §60: completion history from the two completion states that are conclusions.
///
/// `in_progress` is neither finished nor concluded: a work being written is not a verdict, and
/// counting it would make every active author look like they finish a small fraction of their work.
pub fn history(finished: i64, abandoned: i64) -> History {
    History {
        finished: finished.max(0),
        concluded: finished.max(0) + abandoned.max(0),
    }
}

/// §60: the sentence the surface shows. Never a label, always a count.
pub fn summary(h: &History) -> String {
    if h.concluded == 0 {
        return "no concluded works yet".to_string();
    }
    format!(
        "finished {} of {} concluded works",
        h.finished, h.concluded
    )
}

/// Words that turn a count into a mark against somebody. [`summary`]'s output must contain none of
/// them, and so must anything else this surface grows.
pub const NEGATIVE_WORDS: [&str; 6] = [
    "abandoned",
    "unfinished",
    "incomplete",
    "failed",
    "stalled",
    "gave up",
];

/// §60's framing check as a function, so a test can point at the rule instead of a comment.
pub fn is_positive(text: &str) -> bool {
    let lower = text.to_lowercase();
    !NEGATIVE_WORDS.iter().any(|word| lower.contains(word))
}

/// §60: the median gap between the days on which revisions landed.
///
/// `days` are day ordinals (days since the epoch), one per distinct day, in any order. One day —
/// or none — is not a cadence: a median of no gaps would be a number about nothing, which is exactly
/// the import's shape on this instance (127,474 revisions across 2 days).
pub fn cadence(days: &[i64]) -> Cadence {
    let mut sorted: Vec<i64> = days.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    if sorted.len() < 2 {
        return Cadence::Unmeasured {
            reason: if sorted.is_empty() {
                "no revisions, so there is no cadence to measure".to_string()
            } else {
                "every revision landed on one day, which is an import rather than a writing \
                 history"
                    .to_string()
            },
        };
    }
    let mut gaps: Vec<i64> = sorted.windows(2).map(|w| (w[1] - w[0]).max(0)).collect();
    gaps.sort_unstable();
    let samples = gaps.len();
    let median_days = if samples % 2 == 0 {
        (gaps[samples / 2 - 1] + gaps[samples / 2]) / 2
    } else {
        gaps[samples / 2]
    };
    Cadence::Measurable {
        median_days,
        samples,
    }
}

/// §60: an author's cadence, taken as the **median of the works' own cadences**.
///
/// Pooling raw revision days across works would measure the gap between two different works as if
/// it were an update interval; a work that started a year after another would drag the number.
/// Each work gets its own median gap, and the author's answer is the median of those — with the
/// sample count telling the reader how many works it rests on.
pub fn cadence_across(per_work_medians: &[i64]) -> Cadence {
    if per_work_medians.is_empty() {
        return Cadence::Unmeasured {
            reason: "no work has revisions on two different days yet, so there is no cadence to \
                     measure"
                .to_string(),
        };
    }
    let mut sorted: Vec<i64> = per_work_medians.to_vec();
    sorted.sort_unstable();
    let samples = sorted.len();
    let median_days = if samples % 2 == 0 {
        (sorted[samples / 2 - 1] + sorted[samples / 2]) / 2
    } else {
        sorted[samples / 2]
    };
    Cadence::Measurable {
        median_days,
        samples,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_progress_is_neither_finished_nor_concluded() {
        let h = history(3, 1);
        assert_eq!(h.finished, 3);
        assert_eq!(h.concluded, 4, "concluded is complete + abandoned");
        // A work still being written contributes to neither number.
        assert_eq!(summary(&h), "finished 3 of 4 concluded works");
    }

    #[test]
    fn an_author_with_nothing_concluded_gets_a_sentence_not_a_zero_ratio() {
        let h = history(0, 0);
        assert_eq!(summary(&h), "no concluded works yet");
        assert!(is_positive(&summary(&h)));
    }

    #[test]
    fn negative_counts_cannot_produce_a_negative_ratio() {
        let h = history(-5, -2);
        assert_eq!(h.finished, 0);
        assert_eq!(h.concluded, 0);
    }

    #[test]
    fn the_summary_never_carries_a_negative_word() {
        for (finished, abandoned) in [(0, 0), (9, 1), (1, 9), (0, 5), (5, 0)] {
            let text = summary(&history(finished, abandoned));
            assert!(is_positive(&text), "framing rule broken by {text:?}");
        }
        assert!(!is_positive("this work was abandoned"));
        assert!(!is_positive("Stalled since May"));
    }

    #[test]
    fn cadence_is_the_median_gap_between_days() {
        // Odd number of gaps: 3 gaps (2, 14, 30) -> median 14.
        assert_eq!(
            cadence(&[0, 2, 16, 46]),
            Cadence::Measurable {
                median_days: 14,
                samples: 3
            }
        );
        // Even number: 4 gaps (1, 2, 3, 4) -> (2 + 3) / 2 = 2.
        assert_eq!(
            cadence(&[0, 1, 3, 6, 10]),
            Cadence::Measurable {
                median_days: 2,
                samples: 4
            }
        );
    }

    #[test]
    fn unsorted_and_repeated_days_measure_the_same_cadence() {
        let a = cadence(&[10, 0, 4]);
        let b = cadence(&[10, 10, 4, 0, 4]);
        assert_eq!(a, b, "order and duplicates must not change the answer");
        // Days {0, 4, 10} -> gaps {4, 6} -> median (4 + 6) / 2 = 5.
        assert_eq!(a, Cadence::Measurable { median_days: 5, samples: 2 });
    }

    #[test]
    fn an_author_cadence_medians_the_works_rather_than_pooling_their_days() {
        // Two works: one updated every 7 days, one every 30. The median of {7, 30} is 18 due to
        // integer division, and the answer must NOT be the median of the four pooled gaps.
        assert_eq!(
            cadence_across(&[7, 30]),
            Cadence::Measurable {
                median_days: 18,
                samples: 2
            }
        );
        assert_eq!(
            cadence_across(&[5, 5, 90]),
            Cadence::Measurable {
                median_days: 5,
                samples: 3
            }
        );
        match cadence_across(&[]) {
            Cadence::Unmeasured { reason } => assert!(reason.contains("two different days")),
            other => panic!("nothing to measure: {other:?}"),
        }
    }

    #[test]
    fn a_single_day_is_unmeasured_and_says_why() {
        let single = cadence(&[7]);
        match single {
            Cadence::Unmeasured { reason } => {
                assert!(reason.contains("one day"), "the reason must say what is wrong: {reason:?}");
                assert!(is_positive(&reason), "even a refusal is phrased positively");
            }
            other => panic!("one day is not a cadence: {other:?}"),
        }
        assert!(!cadence(&[]).is_measurable());
    }
}
