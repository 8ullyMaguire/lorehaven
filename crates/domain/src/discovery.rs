use crate::ids::WorkId;

/// A recommendation candidate with its score and reason.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub work_id: WorkId,
    pub score: i64,
    pub reason: String,
}

/// Blend candidates from multiple engines deterministically.
pub fn blend(engines: &[Vec<Candidate>]) -> Vec<Candidate> {
    let mut merged: std::collections::HashMap<String, Candidate> = std::collections::HashMap::new();
    for engine_results in engines {
        for candidate in engine_results {
            let entry = merged
                .entry(candidate.work_id.to_canonical_string())
                .or_insert_with(|| Candidate {
                    work_id: candidate.work_id,
                    score: 0,
                    reason: candidate.reason.clone(),
                });
            entry.score += candidate.score;
        }
    }
    let mut results: Vec<Candidate> = merged.into_values().collect();
    results.sort_by_key(|c| -c.score);
    results
}

/// Apply operator affinity multipliers to candidates, silently.
///
/// affinity_bp is in base points (range -5000..=10000). The multiplier is
/// computed as 1 + affinity_bp / 10000, so:
/// affinity_bp = 10000 gives 2.0x (double score)
/// affinity_bp = 5000 gives 1.5x
/// affinity_bp = 0 gives 1.0x (no change)
/// affinity_bp = -5000 gives 0.5x
///
/// The reason field is NEVER modified; influenced vs uninfluenced responses
/// must differ only in ordering, never in field presence or renaming
/// (spec section 16.3, section 20 silent rule).
pub fn apply_affinity_ranking(
    candidates: Vec<Candidate>,
    affinities: &std::collections::HashMap<String, i64>,
) -> Vec<Candidate> {
    let mut results: Vec<Candidate> = candidates
        .into_iter()
        .map(|mut c| {
            if let Some(&affinity_bp) = affinities.get(&c.work_id.to_canonical_string()) {
                let multiplier = 1.0 + (affinity_bp as f64 / 10_000.0);
                c.score = (c.score as f64 * multiplier) as i64;
            }
            c
        })
        .collect();
    results.sort_by_key(|c| -c.score);
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_returns_empty_with_no_engines() {
        let engines: Vec<Vec<Candidate>> = vec![];
        let result = blend(&engines);
        assert!(result.is_empty());
    }

    #[test]
    fn blend_sums_scores_across_engines() {
        let w1 = WorkId::new();
        let w2 = WorkId::new();
        let engines = vec![
            vec![
                Candidate {
                    work_id: w1,
                    score: 10,
                    reason: "engine1".into(),
                },
                Candidate {
                    work_id: w2,
                    score: 5,
                    reason: "engine1".into(),
                },
            ],
            vec![Candidate {
                work_id: w1,
                score: 20,
                reason: "engine2".into(),
            }],
        ];
        let result = blend(&engines);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].work_id, w1);
        assert_eq!(result[0].score, 30);
        assert_eq!(result[1].work_id, w2);
        assert_eq!(result[1].score, 5);
    }

    #[test]
    fn apply_affinity_ranking_boosts_work_with_positive_affinity() {
        let w1 = WorkId::new();
        let w2 = WorkId::new();
        let candidates = vec![
            Candidate {
                work_id: w1,
                score: 10,
                reason: "base".into(),
            },
            Candidate {
                work_id: w2,
                score: 20,
                reason: "base".into(),
            },
        ];
        let mut affinities = std::collections::HashMap::new();
        affinities.insert(w1.to_canonical_string(), 10_000); // 2.0x
        let result = apply_affinity_ranking(candidates, &affinities);
        assert_eq!(result[0].work_id, w1); // boosted
        assert_eq!(result[0].score, 20); // 10 * 2.0
        assert_eq!(result[1].work_id, w2);
        assert_eq!(result[1].score, 20); // unchanged
    }

    #[test]
    fn apply_affinity_ranking_reason_is_never_modified() {
        let w = WorkId::new();
        let candidates = vec![Candidate {
            work_id: w,
            score: 10,
            reason: "engine-tag".into(),
        }];
        let mut affinities = std::collections::HashMap::new();
        affinities.insert(w.to_canonical_string(), 10_000);
        let result = apply_affinity_ranking(candidates, &affinities);
        assert_eq!(result[0].reason, "engine-tag");
    }

    #[test]
    fn apply_affinity_ranking_negative_affinity_reduces_score() {
        let w = WorkId::new();
        let candidates = vec![Candidate {
            work_id: w,
            score: 10,
            reason: "base".into(),
        }];
        let mut affinities = std::collections::HashMap::new();
        affinities.insert(w.to_canonical_string(), -5_000); // 0.5x
        let result = apply_affinity_ranking(candidates, &affinities);
        assert_eq!(result[0].score, 5); // 10 * 0.5
    }
}
