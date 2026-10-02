//! Pre-read report: per-dimension verdicts aggregated into something an author acts on.
//!
//! Step 2 of gap C (`docs/plans/gap-c-ai-pre-read-scoring.md`). Step 1 is
//! [`crate::ai`], which defines the verdicts; this turns a bag of them into a per-work
//! report, and — more importantly — defines what happens when some are missing.
//!
//! **The central rule: a partial report is not a report.** A provider that answers two of
//! five dimensions has not produced a score with two of five numbers; it has produced a
//! different, weaker thing, and presenting it as the same thing is how a reader or author
//! ends up trusting a number that was computed from a fraction of the evidence. So a
//! dimension with no verdict is either [`PreReadStatus::Incomplete`] or explicitly
//! excluded by the operator — never silently skipped and never averaged over what
//! happens to be present.
//!
//! **No aggregate score crosses this boundary.** §32.6 forbids displaying composite
//! quality scores publicly, and §0.3 forbids any ranking signal moving on payment. The
//! strongest form of both is that there is no number here that could be mistaken for a
//! single quality judgement — [`PreReadReport`] holds the dimensions separately and
//! offers only per-dimension ranks and flags. Adding a `composite()` method later would
//! be the violation, so there is deliberately no way to get one.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ai::{AiAbstain, AiOutcome, AiTask, PreReadVerdict};

/// Why a dimension is not part of the report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum DimensionStatus {
    /// The provider answered.
    Scored,
    /// The provider was not asked — the operator configured this dimension out.
    NotConfigured,
    /// Asked for, and abstained.
    Abstained(AiAbstain),
}

impl DimensionStatus {
    /// Whether this dimension has a number.
    pub fn has_score(&self) -> bool {
        matches!(self, DimensionStatus::Scored)
    }
}

/// One dimension's result, as the author sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DimensionOutcome {
    pub dimension: String,
    pub score: f64,
    pub note: String,
}

/// The report for one work.
///
/// Deliberately **not** `Serialize` in a form that reaches a reader-facing endpoint.
/// `Serialize` exists here so the worker can persist a report and the author can fetch
/// it; what it must never reach is the public work page. That boundary is enforced by
/// having no conversion into any public work type — see the module docs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreReadReport {
    pub work_id: String,
    /// Per-dimension outcomes, keyed by the operator's dimension name. A `BTreeMap`
    /// rather than a `HashMap` so the report has a stable order for display and for
    /// comparison — two reports differing only in iteration order are the same report.
    pub dimensions: BTreeMap<String, DimensionOutcome>,
    /// Configured-but-unanswered dimensions, and why.
    pub missing: BTreeMap<String, DimensionStatus>,
}

impl PreReadReport {
    /// The dimensions that were scored, best first.
    ///
    /// Sorted by score descending with the dimension name breaking ties, so two runs
    /// over the same verdicts produce byte-identical reports. Without the tiebreak a
    /// `BTreeMap` iteration plus an unstable sort can produce two different reports for
    /// the same data, which makes a diff between two runs meaningless.
    pub fn ranked(&self) -> Vec<&DimensionOutcome> {
        let mut out: Vec<&DimensionOutcome> = self.dimensions.values().collect();
        out.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.dimension.cmp(&b.dimension))
        });
        out
    }

    /// Whether every configured dimension was answered.
    ///
    /// This is the flag a caller should branch on before showing anything. An incomplete
    /// report is still useful — two scores out of five is real information — but it is
    /// not the same object as a complete one, and the UI says so.
    pub fn is_complete(&self) -> bool {
        self.missing.is_empty()
    }

    /// Dimensions scoring at or below `threshold`, worst first.
    ///
    /// This is the report's actual purpose: an author wants to know which dimensions to
    /// *change*, and "0.3 on length" only matters next to a threshold. Exposed as a
    /// method taking the threshold rather than a stored constant because §20.10.4's
    /// weights are operator configuration published on `/api/v1/meta`, so the number
    /// belongs to the operator, not here.
    pub fn below(&self, threshold: f64) -> Vec<&DimensionOutcome> {
        let mut out: Vec<&DimensionOutcome> = self
            .dimensions
            .values()
            .filter(|d| d.score <= threshold)
            .collect();
        // Worst first, because the question is "what do I fix".
        out.sort_by(|a, b| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.dimension.cmp(&b.dimension))
        });
        out
    }

    /// The task every dimension in a report comes from.
    ///
    /// A report mixing tasks would mean some output came from a provider the author had
    /// not consented to for this work, so the caller passes the expected task and a
    /// mismatch is refused rather than merged.
    pub fn from_verdicts(
        work_id: &str,
        configured: &[String],
        verdicts: Vec<PreReadVerdict>,
    ) -> AiOutcome<Self> {
        if verdicts.is_empty() {
            return Err(AiAbstain::InvalidOutput(
                "no verdicts for any configured dimension".to_string(),
            ));
        }
        let mut dimensions = BTreeMap::new();
        let mut missing: BTreeMap<String, DimensionStatus> = BTreeMap::new();

        for dimension in configured {
            missing.insert(dimension.clone(), DimensionStatus::NotConfigured);
        }
        for verdict in verdicts {
            if verdict.task != AiTask::PreReadScoring {
                return Err(AiAbstain::InvalidOutput(format!(
                    "verdict for {:?} came from {:?}, not PreReadScoring",
                    verdict.dimension, verdict.task
                )));
            }
            let name = verdict.dimension.clone();
            dimensions.insert(
                name.clone(),
                DimensionOutcome {
                    dimension: name,
                    score: verdict.score,
                    note: verdict.note,
                },
            );
            missing.remove(&verdict.dimension);
        }
        // A verdict for a dimension the operator never configured is not silently
        // dropped: it means the provider answered a question nobody asked, which is the
        // shape of a prompt-injection result. Recorded so the caller can see it.
        for dimension in dimensions.keys() {
            if !configured.iter().any(|c| c == dimension) {
                missing.insert(
                    dimension.clone(),
                    DimensionStatus::Abstained(AiAbstain::InvalidOutput(format!(
                        "{dimension:?} is not a configured dimension"
                    ))),
                );
            }
        }
        Ok(PreReadReport {
            work_id: work_id.to_string(),
            dimensions,
            missing,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::PreReadVerdict;

    fn verdict(dimension: &str, score: f64) -> PreReadVerdict {
        PreReadVerdict::new(dimension, score, "note").expect("in range")
    }

    fn configured() -> Vec<String> {
        ["length", "tone", "tags"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn a_missing_dimension_is_recorded_not_averaged_away() {
        let report = PreReadReport::from_verdicts(
            "w1",
            &configured(),
            vec![verdict("length", 0.3), verdict("tone", 0.9)],
        )
        .expect("a partial report is still a report");

        assert_eq!(report.dimensions.len(), 2, "only what was answered");
        assert!(!report.is_complete(), "one dimension never came back");
        assert_eq!(
            report.missing.get("tags"),
            Some(&DimensionStatus::NotConfigured),
            "an unanswered configured dimension is visible"
        );
    }

    #[test]
    fn a_verdict_for_an_unconfigured_dimension_is_recorded_not_dropped() {
        // The shape of a prompt-injection result: the provider answered a question nobody
        // asked. Silently dropping it would hide the only evidence that it happened.
        let report = PreReadReport::from_verdicts(
            "w1",
            &configured(),
            vec![
                verdict("length", 0.3),
                verdict("tone", 0.9),
                verdict("tags", 0.5),
                verdict("system_prompt", 1.0),
            ],
        )
        .expect("a report with an extra dimension is still constructible");

        assert!(
            report.dimensions.contains_key("system_prompt"),
            "the dimension is present, so the report cannot pretend it was never said"
        );
        assert!(
            matches!(
                report.missing.get("system_prompt"),
                Some(DimensionStatus::Abstained(AiAbstain::InvalidOutput(_)))
            ),
            "and it is recorded as not a configured dimension"
        );
    }

    #[test]
    fn no_verdicts_at_all_is_an_abstention_not_an_empty_report() {
        // An empty report and "the provider declined" are different facts, and collapsing
        // them is how a caller ends up rendering an empty scorecard as a zero.
        let outcome: AiOutcome<PreReadReport> =
            PreReadReport::from_verdicts("w1", &configured(), vec![]);
        assert!(matches!(outcome, Err(AiAbstain::InvalidOutput(_))));
    }

    #[test]
    fn a_verdict_from_another_task_is_refused() {
        // Merging output from a task the author did not consent to for this work would
        // be a consent violation dressed as a convenience.
        let mut other = verdict("length", 0.3);
        other.task = AiTask::Translation;
        let outcome = PreReadReport::from_verdicts("w1", &configured(), vec![other]);
        assert!(matches!(outcome, Err(AiAbstain::InvalidOutput(_))));
    }

    #[test]
    fn ranking_is_stable_across_identical_inputs() {
        let report = PreReadReport::from_verdicts(
            "w1",
            &configured(),
            vec![
                verdict("length", 0.5),
                verdict("tone", 0.5),
                verdict("tags", 0.5),
            ],
        )
        .expect("report");
        // Every score ties, so only the tiebreak decides -- and it must decide the same
        // way every time, or a diff between two runs means nothing.
        let order: Vec<&str> = report
            .ranked()
            .iter()
            .map(|d| d.dimension.as_str())
            .collect();
        assert_eq!(order, vec!["length", "tags", "tone"]);
    }

    #[test]
    fn below_returns_the_worst_first_and_includes_the_boundary() {
        let report = PreReadReport::from_verdicts(
            "w1",
            &configured(),
            vec![
                verdict("length", 0.3),
                verdict("tone", 0.9),
                verdict("tags", 0.5),
            ],
        )
        .expect("report");
        let flagged: Vec<&str> = report
            .below(0.5)
            .iter()
            .map(|d| d.dimension.as_str())
            .collect();
        // `<=` not `<`: a dimension exactly at the threshold is the one that decides
        // whether the author changes anything, so excluding it would hide the answer.
        assert_eq!(flagged, vec!["length", "tags"]);
    }

    #[test]
    fn an_empty_report_ranks_and_flags_without_panicking() {
        // `ranked` and `below` are called on every fetch, including the empty case, so
        // they must not assume a non-empty map.
        let report = PreReadReport {
            work_id: "w1".to_string(),
            dimensions: BTreeMap::new(),
            missing: configured()
                .into_iter()
                .map(|d| (d, DimensionStatus::NotConfigured))
                .collect(),
        };
        assert!(report.ranked().is_empty());
        assert!(report.below(1.0).is_empty());
        assert!(!report.is_complete());
    }
}
