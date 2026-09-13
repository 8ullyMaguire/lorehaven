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
}
