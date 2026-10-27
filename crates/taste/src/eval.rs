//! Ranking metrics — how "we match kindred" becomes a number (spec §8.3, R-6).
//!
//! The claim this module exists to replace is *"our recommender is as good as kindred's"*,
//! which nobody can check. The replacement is two ranked lists over the same readers and the
//! same judgements, scored the same way: kindred's ranking is the oracle, ours is the
//! candidate, and the difference is a number a reviewer can argue with.
//!
//! # What is measured, and why exactly these three
//!
//! - **nDCG@10 at graded relevance.** The first page is what a reader actually sees, and the
//!   graded form is the only one that distinguishes "the work they loved, at position 3" from
//!   "a work they tolerated, at position 3".
//! - **recall@50.** nDCG@10 can be perfect while the engine's 50-item candidate set holds
//!   almost nothing a reader wants. Recall is what detects that: it is the measure of whether
//!   the *pool* is right, as distinct from the order.
//! - **overlap@10.** The shadow comparison asks a narrower question than either: do the two
//!   rankings agree at all? A high nDCG with a low overlap means we are not reproducing
//!   kindred, we are getting lucky on the judged subset — which is precisely the difference
//!   between "matches" and "scores similarly on this test set".
//!
//! # The traps this module refuses to fall into
//!
//! - **A reader with no judgements is not a perfect score.** Their recall is 0/0, and
//!   counting that as 1.0 would inflate the mean in exactly the run that judged nothing.
//!   They return `None` and [`ParityReport`] counts how many were dropped, so the mean is
//!   always reported beside the denominator that produced it.
//! - **`k` larger than the list is not an error.** The denominator stays the number of
//!   relevant works, not `min(k, len)` — clamping the denominator would let a ten-item list
//!   score 1.0 on recall@50.
//! - **Ties in the ranking are not broken by luck.** Order is the ranking's order; this
//!   module never sorts, because re-sorting a ranking would measure this module's sort rather
//!   than the engine's ranking.
//! - **Duplicate positions do not count twice.** A ranking that repeats a work gets credit
//!   once, because the second appearance occupies a slot the engine wasted — counting it as a
//!   second hit would reward the waste.

use std::collections::HashSet;

/// One judgement about one work, in the form the corpus stores it.
///
/// `grade` is graded relevance: 0 is "not relevant" (judged and rejected), 1 "relevant", and
/// higher grades are stronger. A judge that only ever says yes/no uses 1, and nDCG then
/// reduces to the binary form without any change here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judgement {
    /// The work the judgement is about.
    pub work_id: String,
    /// How relevant it is: 0 for judged-and-rejected, 1+ for relevant.
    pub grade: u32,
}

/// `nDCG@k` over a ranking and a set of graded judgements.
///
/// Gain is `2^grade − 1` and the discount is `log2(position + 2)`, the standard formulation:
/// the first position is `log2(2) = 1`, so the top hit carries its full gain and each step
/// down is worth less.
///
/// Returns `None` when no judgement in `judged` is relevant: a reader who judged nothing is
/// not a reader who was served perfectly, and the difference matters in the mean.
#[must_use]
pub fn ndcg_at(ranking: &[String], judged: &[Judgement], k: usize) -> Option<f64> {
    let ideal_gains: Vec<f64> = {
        let mut gains: Vec<f64> = judged
            .iter()
            .map(|j| gain(j.grade))
            .filter(|g| *g > 0.0)
            .collect();
        gains.sort_by(|a, b| b.total_cmp(a));
        gains
    };
    if ideal_gains.is_empty() {
        return None;
    }

    let grades: std::collections::HashMap<&str, u32> = judged
        .iter()
        .map(|j| (j.work_id.as_str(), j.grade))
        .collect();

    let mut seen: HashSet<&str> = HashSet::new();
    let mut dcg = 0.0;
    for (position, work) in ranking.iter().take(k).enumerate() {
        if !seen.insert(work.as_str()) {
            // A repeat occupies a slot and earns nothing: see the module docs.
            continue;
        }
        let grade = grades.get(work.as_str()).copied().unwrap_or(0);
        dcg += gain(grade) / discount(position);
    }

    let mut idcg = 0.0;
    for (position, ideal) in ideal_gains.iter().take(k).enumerate() {
        idcg += ideal / discount(position);
    }
    if idcg == 0.0 {
        return None;
    }
    Some(dcg / idcg)
}

/// `recall@k`: the share of the relevant works that appear in the first `k` positions.
///
/// Returns `None` when nothing judged was relevant, for the same reason as [`ndcg_at`].
#[must_use]
pub fn recall_at(ranking: &[String], judged: &[Judgement], k: usize) -> Option<f64> {
    let relevant: HashSet<&str> = judged
        .iter()
        .filter(|j| j.grade > 0)
        .map(|j| j.work_id.as_str())
        .collect();
    if relevant.is_empty() {
        return None;
    }

    let mut found: HashSet<&str> = HashSet::new();
    for work in ranking.iter().take(k) {
        if relevant.contains(work.as_str()) {
            found.insert(work.as_str());
        }
    }
    Some(found.len() as f64 / relevant.len() as f64)
}

/// `overlap@k`: the share of the first `k` positions the two rankings agree on.
///
/// This is the shadow comparison's question — "is this the same recommendation, or a
/// different one that happens to score well?" — and it is deliberately not rank-weighted: two
/// rankings holding the same works in a different order are the same recommendation for this
/// purpose, and the difference in order is what nDCG is for.
///
/// Both lists are truncated to `k` before comparing, and the denominator is `k` rather than
/// `min(len_a, len_b)`: two five-item lists agreeing completely is `overlap@10 = 0.5`, because
/// five slots out of ten were served and they matched. Using the shorter length would report
/// 1.0 for a run that served half a page.
#[must_use]
pub fn overlap_at(left: &[String], right: &[String], k: usize) -> f64 {
    if k == 0 {
        return 0.0;
    }
    let left_set: HashSet<&str> = left.iter().take(k).map(String::as_str).collect();
    let right_set: HashSet<&str> = right.iter().take(k).map(String::as_str).collect();
    left_set.intersection(&right_set).count() as f64 / k as f64
}

/// `2^grade − 1`. Grade 0 contributes nothing, which is why a rejected work cannot pad a list.
fn gain(grade: u32) -> f64 {
    if grade == 0 {
        return 0.0;
    }
    2f64.powi(i32::try_from(grade).unwrap_or(i32::MAX)) - 1.0
}

/// `log2(position + 2)`, so position 0 is undiscounted.
fn discount(position: usize) -> f64 {
    ((position + 2) as f64).log2()
}

/// One reader's judgements and the two rankings over them.
#[derive(Debug, Clone)]
pub struct ReaderCase {
    /// Stable identifier, so a disagreement can be named rather than counted.
    pub reader: String,
    /// What the judge said about this reader's works.
    pub judged: Vec<Judgement>,
    /// The candidate engine's ranking.
    pub candidate: Vec<String>,
    /// The oracle's ranking — kindred's, when this is the parity harness.
    pub oracle: Vec<String>,
}

/// One reader's scores, and what a missing score means.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReaderScore {
    /// `nDCG@10`, or `None` when the reader has no relevant judgements.
    pub ndcg_at_10: Option<f64>,
    /// `recall@50`, or `None` when the reader has no relevant judgements.
    pub recall_at_50: Option<f64>,
    /// Whether the two rankings agree, position for position.
    pub overlap_at_10: f64,
}

impl ReaderScore {
    /// Score one reader against the oracle.
    #[must_use]
    pub fn of(case: &ReaderCase) -> Self {
        Self {
            ndcg_at_10: ndcg_at(&case.candidate, &case.judged, 10),
            recall_at_50: recall_at(&case.candidate, &case.judged, 50),
            overlap_at_10: overlap_at(&case.candidate, &case.oracle, 10),
        }
    }
}

/// The whole run: candidate means, oracle means, and the denominators that produced them.
#[derive(Debug, Clone, PartialEq)]
pub struct ParityReport {
    /// Readers in the run, including those with no judgements.
    pub readers: usize,
    /// Readers whose judgements were sufficient to score.
    pub scored: usize,
    /// Mean `nDCG@10` of the candidate ranking, over the scored readers.
    pub candidate_ndcg_at_10: Option<f64>,
    /// Mean `nDCG@10` of the oracle ranking, over the same readers.
    pub oracle_ndcg_at_10: Option<f64>,
    /// Mean `recall@50` of the candidate ranking.
    pub candidate_recall_at_50: Option<f64>,
    /// Mean `recall@50` of the oracle ranking.
    pub oracle_recall_at_50: Option<f64>,
    /// Mean overlap of the two rankings.
    pub overlap_at_10: f64,
}

/// Mean of the values that exist, or `None` when none do.
fn mean(values: impl Iterator<Item = f64>) -> Option<f64> {
    let collected: Vec<f64> = values.collect();
    if collected.is_empty() {
        return None;
    }
    Some(collected.iter().sum::<f64>() / collected.len() as f64)
}

impl ParityReport {
    /// Score every reader and average.
    #[must_use]
    pub fn build(cases: &[ReaderCase]) -> Self {
        let mut ndcg_candidate = Vec::new();
        let mut ndcg_oracle = Vec::new();
        let mut recall_candidate = Vec::new();
        let mut recall_oracle = Vec::new();
        let mut overlaps = Vec::new();

        for case in cases {
            let score = ReaderScore::of(case);
            if let Some(value) = score.ndcg_at_10 {
                ndcg_candidate.push(value);
            }
            if let Some(value) = ndcg_at(&case.oracle, &case.judged, 10) {
                ndcg_oracle.push(value);
            }
            if let Some(value) = score.recall_at_50 {
                recall_candidate.push(value);
            }
            if let Some(value) = recall_at(&case.oracle, &case.judged, 50) {
                recall_oracle.push(value);
            }
            overlaps.push(score.overlap_at_10);
        }

        Self {
            readers: cases.len(),
            scored: ndcg_candidate.len(),
            candidate_ndcg_at_10: mean(ndcg_candidate.into_iter()),
            oracle_ndcg_at_10: mean(ndcg_oracle.into_iter()),
            candidate_recall_at_50: mean(recall_candidate.into_iter()),
            oracle_recall_at_50: mean(recall_oracle.into_iter()),
            overlap_at_10: mean(overlaps.into_iter()).unwrap_or(0.0),
        }
    }

    /// The lines a harness prints, so every number arrives with its denominator.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let show = |value: Option<f64>| match value {
            Some(v) => format!("{v:.4}"),
            None => "n/a".to_string(),
        };
        let delta = |candidate: Option<f64>, oracle: Option<f64>| match (candidate, oracle) {
            (Some(c), Some(o)) => format!("{:+.4}", c - o),
            _ => "n/a".to_string(),
        };
        vec![
            format!(
                "readers: {} ({} scored, {} had no relevant judgements)",
                self.readers,
                self.scored,
                self.readers.saturating_sub(self.scored)
            ),
            format!(
                "nDCG@10   candidate {}  oracle {}  delta {}",
                show(self.candidate_ndcg_at_10),
                show(self.oracle_ndcg_at_10),
                delta(self.candidate_ndcg_at_10, self.oracle_ndcg_at_10)
            ),
            format!(
                "recall@50 candidate {}  oracle {}  delta {}",
                show(self.candidate_recall_at_50),
                show(self.oracle_recall_at_50),
                delta(self.candidate_recall_at_50, self.oracle_recall_at_50)
            ),
            format!("overlap@10 {}", show(Some(self.overlap_at_10))),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn judged(pairs: &[(&str, u32)]) -> Vec<Judgement> {
        pairs
            .iter()
            .map(|(id, grade)| Judgement {
                work_id: (*id).to_string(),
                grade: *grade,
            })
            .collect()
    }

    fn ranking(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_string()).collect()
    }

    #[test]
    fn a_perfect_ranking_scores_one() {
        let judged = judged(&[("a", 3), ("b", 2), ("c", 1)]);
        let ranked = ranking(&["a", "b", "c"]);
        assert!((ndcg_at(&ranked, &judged, 10).unwrap() - 1.0).abs() < 1e-12);
        assert!((recall_at(&ranked, &judged, 50).unwrap() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn a_reversed_ranking_scores_below_one_but_keeps_full_recall() {
        let judged = judged(&[("a", 3), ("b", 1)]);
        let ranked = ranking(&["z", "y", "b", "a"]);
        let ndcg = ndcg_at(&ranked, &judged, 10).unwrap();
        assert!(ndcg < 1.0 && ndcg > 0.0, "nDCG = {ndcg}");
        // Order hurt the score; the pool did not. That is exactly the distinction the two
        // metrics are for.
        assert!((recall_at(&ranked, &judged, 50).unwrap() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn ndcg_matches_a_hand_computation() {
        // One relevant work (grade 1) found at position 1 (second slot).
        // DCG  = 1 / log2(3)                       = 0.6309297535714574
        // IDCG = 1 / log2(2)                       = 1.0
        let judged = judged(&[("a", 1)]);
        let ranked = ranking(&["x", "a"]);
        let ndcg = ndcg_at(&ranked, &judged, 10).unwrap();
        assert!(
            (ndcg - 1.0 / 3f64.log2()).abs() < 1e-12,
            "nDCG = {ndcg}, expected {}",
            1.0 / 3f64.log2()
        );
    }

    #[test]
    fn graded_relevance_beats_binary_at_the_same_position() {
        // The same position, the same hit count, different grades: the graded form must
        // separate them, because "the work they loved" and "a work they tolerated" are not
        // the same recommendation.
        let loved = judged(&[("a", 3), ("b", 1)]);
        let tolerated = judged(&[("a", 1), ("b", 3)]);
        let ranked = ranking(&["b", "a"]);
        let s1 = ndcg_at(&ranked, &loved, 10).unwrap();
        let s2 = ndcg_at(&ranked, &tolerated, 10).unwrap();
        assert!(s1 < s2, "graded nDCG is not reflecting grade: {s1} vs {s2}");
    }

    #[test]
    fn a_reader_with_no_relevant_judgements_is_not_a_perfect_score() {
        let judged = judged(&[("a", 0), ("b", 0)]);
        let ranked = ranking(&["a", "b"]);
        assert_eq!(ndcg_at(&ranked, &judged, 10), None);
        assert_eq!(recall_at(&ranked, &judged, 50), None);
        assert_eq!(ndcg_at(&ranked, &[], 10), None);

        let report = ParityReport::build(&[ReaderCase {
            reader: "nobody-judged-this".into(),
            judged,
            candidate: ranked.clone(),
            oracle: ranked,
        }]);
        assert_eq!(report.readers, 1);
        assert_eq!(report.scored, 0, "an unjudged reader must not be scored");
        assert_eq!(report.candidate_ndcg_at_10, None);
    }

    #[test]
    fn the_recall_denominator_is_not_clamped_to_the_list_length() {
        // Ten relevant works, a ranking of four: recall@50 is 0.4, not 1.0.
        let judged: Vec<Judgement> = (0..10)
            .map(|i| Judgement {
                work_id: format!("w{i}"),
                grade: 1,
            })
            .collect();
        let ranked = ranking(&["w0", "w1", "w2", "w3"]);
        let recall = recall_at(&ranked, &judged, 50).unwrap();
        assert!((recall - 0.4).abs() < 1e-12, "recall = {recall}");
    }

    #[test]
    fn a_repeated_work_earns_one_slot_of_credit() {
        let judged = judged(&[("a", 1), ("b", 1)]);
        let once = ranking(&["a", "b"]);
        let repeated = ranking(&["a", "a", "b"]);
        let clean = ndcg_at(&once, &judged, 10).unwrap();
        let wasteful = ndcg_at(&repeated, &judged, 10).unwrap();
        assert!(
            wasteful < clean,
            "repeating a work must cost the ranking: {wasteful} vs {clean}"
        );
        assert_eq!(
            recall_at(&repeated, &judged, 50),
            recall_at(&once, &judged, 50),
            "a repeat must not be counted twice in recall"
        );
    }

    #[test]
    fn overlap_is_measured_against_k_and_not_the_shorter_list() {
        let short = ranking(&["a", "b", "c"]);
        let same = ranking(&["a", "b", "c"]);
        // Three of ten slots served, all matching: 0.3, not 1.0.
        assert!((overlap_at(&short, &same, 10) - 0.3).abs() < 1e-12);
        assert!((overlap_at(&short, &same, 3) - 1.0).abs() < 1e-12);
        assert_eq!(overlap_at(&short, &same, 0), 0.0);
    }

    #[test]
    fn a_report_carries_the_denominator_that_produced_it() {
        let cases = vec![
            ReaderCase {
                reader: "judged".into(),
                judged: judged(&[("a", 2)]),
                candidate: ranking(&["a"]),
                oracle: ranking(&["a"]),
            },
            ReaderCase {
                reader: "unjudged".into(),
                judged: vec![],
                candidate: ranking(&["z"]),
                oracle: ranking(&["z"]),
            },
        ];
        let report = ParityReport::build(&cases);
        assert_eq!(report.readers, 2);
        assert_eq!(report.scored, 1);
        // The one scored reader is perfect, and that is the mean — the unjudged reader must
        // not drag it toward zero or push it above one.
        assert_eq!(report.candidate_ndcg_at_10, Some(1.0));
        assert_eq!(report.oracle_ndcg_at_10, Some(1.0));
        assert!((report.overlap_at_10 - 0.1).abs() < 1e-12);
        let lines = report.lines();
        assert!(lines[0].contains("2 readers") || lines[0].contains("readers: 2"));
        assert!(lines[0].contains("1 scored"), "{}", lines[0]);
    }
}
