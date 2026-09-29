//! What a model's number is allowed to do.
//!
//! This module is the reason `lorehaven-decisions` is a crate rather than a
//! `reqwest` call at a call site. Transporting a probability is easy;
//! deciding what a probability may change is the whole design, and a caller
//! that thresholds directly has reimplemented this policy with whatever
//! defaults it happened to have.
//!
//! # The asymmetry
//!
//! A calibrated posterior here may **narrow an acceptance to a hold, and may
//! do nothing else.** It cannot reject, and it cannot accept.
//!
//! Neither of those is caution about the model. Both are about what the model
//! is looking at:
//!
//! * **Rejection is structural.** Spec §11.14 reserves `rejected` for
//!   "certainly not a work: an empty or placeholder title or author". An
//!   empty string is not a judgement call, and no amount of text evidence
//!   argues against it. A model that outvoted an empty author would be
//!   outvoting arithmetic.
//! * **Acceptance is the deterministic path's to grant**, on evidence the
//!   model never saw: the fetched metadata, the word count, the adapter's
//!   own source-specific evidence. The model reads prose. It cannot see the
//!   chapter list.
//!
//! So the model is a *veto on optimism*, and the ceiling on its influence is a
//! hold — a state §11.14 already defines as "held for a person rather than
//! deleted". That is what makes this safe to point at an instance holding
//! other people's work: the worst a bad model can do is send a work to a human.

use std::fmt;

/// A §11.14 quality call, in the form this module reconciles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QualityCall {
    /// A real work: real title, real author, non-zero length.
    Accepted,
    /// Certainly not a work. Carries the reason, because §11.14 requires a
    /// rejection to name itself.
    Rejected {
        /// Why it was refused.
        reason: String,
    },
    /// No confident call could be made. Held for a person.
    Held {
        /// Why it could not be decided.
        reason: String,
    },
}

impl QualityCall {
    /// The short name, for the audit record.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected { .. } => "rejected",
            Self::Held { .. } => "held",
        }
    }
}

impl fmt::Display for QualityCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Accepted => f.write_str("accepted"),
            Self::Rejected { reason } => write!(f, "rejected: {reason}"),
            Self::Held { reason } => write!(f, "held: {reason}"),
        }
    }
}

/// Reconcile a deterministic classification with a calibrated posterior on
/// "is this a work?".
///
/// `posterior` is `None` when no model was consulted: the provider is
/// deterministic, the model was unreachable, the body was never fetched, or
/// the answer was refused as nonsense. All four mean the same thing here, and
/// all four mean the deterministic answer stands unchanged. Distinguishing them
/// is the caller's business for the audit record, and it must not change this
/// function's answer — a path that behaves differently when a model is
/// configured is a path whose behaviour nobody can reason about.
///
/// `accept_threshold` is the posterior at or above which an acceptance is kept.
/// It defaults high (0.90) because §11.14's posture is *reject the obvious
/// junk, hold the rest*: a model that is usually right should still send the
/// unusual case to a person rather than admit it.
///
/// The threshold is compared with `<`, so a posterior exactly equal to the
/// threshold is accepted. A threshold is a floor on confidence, and a value
/// that meets the floor meets it.
#[must_use]
pub fn reconcile(
    deterministic: QualityCall,
    posterior: Option<f64>,
    accept_threshold: f64,
) -> QualityCall {
    // The model's answer is validated before it reaches here — see
    // `Answer::check_probabilities`, which refuses NaN and out-of-range values
    // rather than clamping them. This arm therefore does not re-check, because
    // a second check that silently treated NaN as "no opinion" would make the
    // same nonsense input produce a *permissive* answer here, which is exactly
    // what the client refuses to let it do upstream.
    match deterministic {
        // A rejection stands, whatever the model thinks. High confidence that
        // this IS a work must not overturn an empty author: the model reads
        // prose and the reason is arithmetic.
        QualityCall::Rejected { reason } => QualityCall::Rejected { reason },
        // §11.14 makes a zero word count a *rule* rather than a probability:
        // "A zero word count is held, not rejected... A real work with no
        // counted words exists." A model that says the work is certainly real
        // must not convert that hold into an acceptance, because the hold is
        // the spec's answer for a case the model cannot see.
        QualityCall::Held { reason } => QualityCall::Held { reason },
        QualityCall::Accepted => match posterior {
            None => QualityCall::Accepted,
            Some(p) if p < accept_threshold => QualityCall::Held {
                reason: format!(
                    "the decision model put this at {p:.3}, below the {accept_threshold:.3} \
                     needed to accept without a person; held rather than refused, because a \
                     low score is not evidence that the work is not real"
                ),
            },
            Some(_) => QualityCall::Accepted,
        },
    }
}

/// The default posterior at or above which an acceptance is kept.
///
/// Deliberately high. §11.14's default posture is "reject the obvious junk,
/// hold the rest", and a model asked to grade a work it is usually right about
/// should still send the unusual case to a person. An operator who has read
/// the audit trail can lower it; the audit trail exists so that lowering it is
/// an informed choice rather than a guess.
pub const DEFAULT_ACCEPT_THRESHOLD: f64 = 0.90;

#[cfg(test)]
mod tests {
    use super::{reconcile, QualityCall};

    /// Sweep the posterior across the whole range and report the outcomes.
    ///
    /// A helper rather than eight near-identical tests, because the property
    /// that matters is not the value at 0.5 — it is that the outcome changes
    /// **once** and in the right direction. Eight table cells can all pass while
    /// the function is non-monotone, and a non-monotone threshold means a
    /// work is accepted at 0.3 and held at 0.6.
    fn outcomes(deterministic: QualityCall, threshold: f64) -> Vec<&'static str> {
        (0..=1000)
            .map(|i| {
                let p = f64::from(i) / 1000.0;
                reconcile(deterministic.clone(), Some(p), threshold).as_str()
            })
            .collect()
    }

    fn transitions<'a>(outcomes: &'a [&'static str]) -> Vec<(usize, &'a str, &'a str)> {
        let mut changes = Vec::new();
        for (i, pair) in outcomes.windows(2).enumerate() {
            if pair[0] != pair[1] {
                changes.push((i, pair[0], pair[1]));
            }
        }
        changes
    }

    // --- the two cells a model must never touch -----------------------------

    #[test]
    fn a_rejection_stands_even_at_maximum_confidence_that_it_is_a_work() {
        // The anti-regret cell, and the one most likely to be written wrong.
        // A model that says "this is certainly a work" has seen prose; the
        // rejection is an empty author, which no prose can supply.
        let call = reconcile(
            QualityCall::Rejected {
                reason: "empty author".to_owned(),
            },
            Some(0.999),
            0.90,
        );
        assert_eq!(
            call,
            QualityCall::Rejected {
                reason: "empty author".to_owned()
            },
            "a model's confidence must not outvote an empty author"
        );
    }

    #[test]
    fn a_rejection_stands_even_when_the_model_is_asked_nothing() {
        // Same asymmetry, the other direction: no model opinion changes
        // nothing either.
        let call = reconcile(
            QualityCall::Rejected {
                reason: "empty or placeholder title".to_owned(),
            },
            None,
            0.90,
        );
        assert!(matches!(call, QualityCall::Rejected { .. }));
    }

    #[test]
    fn a_held_is_never_promoted_to_accepted_however_certain_the_model_is() {
        // §11.14: "A zero word count is held, not rejected. A real work with
        // no counted words exists, and a rule that called it junk would lose
        // it without trace." The hold IS the spec's answer, so promoting it
        // would be the model overriding the spec rather than adding to it.
        let call = reconcile(
            QualityCall::Held {
                reason: "no word count: cannot determine if content is real".to_owned(),
            },
            Some(1.0),
            0.90,
        );
        assert_eq!(
            call,
            QualityCall::Held {
                reason: "no word count: cannot determine if content is real".to_owned()
            }
        );
    }

    // --- the one direction the model may move -------------------------------

    #[test]
    fn an_acceptance_below_the_threshold_becomes_a_hold() {
        let call = reconcile(QualityCall::Accepted, Some(0.42), 0.90);
        match call {
            QualityCall::Held { reason } => {
                assert!(
                    reason.contains("0.420"),
                    "the reason names the number, so an operator can see what was \
                     measured: {reason}"
                );
                assert!(
                    reason.contains("0.900"),
                    "and the threshold it was measured against: {reason}"
                );
            }
            other => panic!("expected a hold, got {other}"),
        }
    }

    #[test]
    fn an_acceptance_at_or_above_the_threshold_is_kept() {
        assert_eq!(
            reconcile(QualityCall::Accepted, Some(0.90), 0.90),
            QualityCall::Accepted,
            "a value that meets the floor meets it"
        );
        assert_eq!(
            reconcile(QualityCall::Accepted, Some(1.0), 0.90),
            QualityCall::Accepted
        );
    }

    #[test]
    fn an_acceptance_with_no_model_opinion_is_kept() {
        // The deployed state of every instance that has not opted in, and the
        // degradation path when a model goes down. It must be indistinguishable
        // from "the model agreed".
        assert_eq!(
            reconcile(QualityCall::Accepted, None, 0.90),
            QualityCall::Accepted
        );
    }

    // --- the property the table cells cannot see ----------------------------

    #[test]
    fn sweeping_the_posterior_crosses_over_exactly_once_and_the_right_way() {
        let swept = outcomes(QualityCall::Accepted, 0.90);
        let changes = transitions(&swept);
        assert_eq!(
            changes.len(),
            1,
            "a threshold that is crossed more than once accepts a work at one \
             confidence and holds it at a higher one: {changes:?}"
        );
        assert_eq!(
            (changes[0].1, changes[0].2),
            ("held", "accepted"),
            "the only transition must be held -> accepted, as confidence rises: {changes:?}"
        );
    }

    #[test]
    fn the_sweep_starts_held_and_ends_accepted() {
        let swept = outcomes(QualityCall::Accepted, 0.90);
        assert_eq!(swept.first(), Some(&"held"), "p = 0.0 is not a work");
        assert_eq!(swept.last(), Some(&"accepted"), "p = 1.0 is");
    }

    #[test]
    fn a_threshold_the_operator_lowered_moves_the_crossover_not_the_direction() {
        // An operator who has read the audit can lower the threshold; what
        // they cannot do is invert it.
        let strict = outcomes(QualityCall::Accepted, 0.90);
        let lenient = outcomes(QualityCall::Accepted, 0.50);
        let first_lenient = lenient
            .iter()
            .position(|o| *o == "accepted")
            .expect("0.50 is reachable in a sweep to 1.0");
        let first_strict = strict
            .iter()
            .position(|o| *o == "accepted")
            .expect("0.90 is reachable in a sweep to 1.0");
        assert_eq!(
            first_lenient, 500,
            "at a 0.50 threshold, 0.500 is the first acceptance"
        );
        assert_eq!(first_strict, 900);
        assert!(
            first_lenient < first_strict,
            "lowering the threshold admits more, never fewer"
        );
    }

    #[test]
    fn a_swept_rejection_never_transitions_at_all() {
        let swept = outcomes(
            QualityCall::Rejected {
                reason: "empty author".to_owned(),
            },
            0.90,
        );
        assert!(
            transitions(&swept).is_empty(),
            "a rejection must not depend on the model's opinion at any confidence"
        );
        assert!(swept.iter().all(|o| *o == "rejected"));
    }

    #[test]
    fn a_swept_held_never_transitions_at_all() {
        let swept = outcomes(
            QualityCall::Held {
                reason: "no word count".to_owned(),
            },
            0.90,
        );
        assert!(transitions(&swept).is_empty());
        assert!(swept.iter().all(|o| *o == "held"));
    }

    #[test]
    fn a_zero_threshold_accepts_everything_including_p_zero() {
        // A degenerate but legal configuration. The model's ceiling is still a
        // hold and its floor is still "no effect", so a threshold of zero
        // means the model never holds — it does not mean the model can reject.
        let swept = outcomes(QualityCall::Accepted, 0.0);
        assert!(
            swept.iter().all(|o| *o == "accepted"),
            "a zero threshold disables the model's veto, not its other limits"
        );
    }

    #[test]
    fn the_reason_a_model_held_says_held_rather_than_refused() {
        // The distinction §11.14 is about: a rejection is "a reason shown to
        // whoever asked", a hold "waits for a decision". A model's doubt is
        // doubt, and calling it a refusal would tell a reader their work is not
        // a work.
        let call = reconcile(QualityCall::Accepted, Some(0.01), 0.90);
        let reason = match call {
            QualityCall::Held { reason } => reason,
            other => panic!("expected a hold, got {other}"),
        };
        assert!(reason.contains("held rather than refused"), "{reason}");
        assert!(!reason.to_lowercase().contains("not a work"), "{reason}");
    }
}
