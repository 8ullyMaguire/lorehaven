use crate::ids::WorkId;

/// A recommendation candidate with its score and reason.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub work_id: WorkId,
    pub score: i64,
    pub reason: String,
    /// Taste signal: how strongly this work aligns with the instance taste profile.
    pub taste_signal: f64,
    /// Diversity class: 0.0 = aligned with admin taste, 1.0 = maximally distant.
    pub diversity_class: f64,
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
                    taste_signal: 0.0,
                    diversity_class: 0.0,
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

/// Apply theme gravity to candidates (spec §0.4.6).
///
/// In `thematic` mode, `nudge_bp` provides a uniform topic bonus to works
/// matching the instance's public topics. In `adaptive` mode, the nudge is
/// derived per-work from the current drift vector (computed separately). In
/// `generic` mode, no nudge is applied.
///
/// `work_nudge_bp` returns the additional gravity in basis points for a given
/// work. When no topic matches, it returns 0.
pub fn apply_theme_gravity(
    candidates: Vec<Candidate>,
    work_nudge_bp: &dyn Fn(&WorkId) -> i64,
) -> Vec<Candidate> {
    let mut results: Vec<Candidate> = candidates
        .into_iter()
        .map(|mut c| {
            let nudge = work_nudge_bp(&c.work_id);
            if nudge != 0 {
                // Multiplicative nudge: 1000 bp = 1.1x, 2500 bp = 1.25x.
                let multiplier = 1.0 + (nudge as f64 / 10_000.0);
                c.score = (c.score as f64 * multiplier) as i64;
            }
            c
        })
        .collect();
    results.sort_by_key(|c| -c.score);
    results
}

/// Apply per-fandom caps and exploration slots to a ranked candidate list.
///
/// - `per_fandom_cap`: the maximum number of works from the same fandom to
///   keep in the final output.
/// - `exploration_rate`: fraction of the total `limit` reserved for exploration.
///   Those slots are filled with works whose fandom is NOT among the reader's
///   known fandoms. If there are not enough exploration candidates, the slots
///   fall back to remaining capped results transparently.
///
/// Returns at most `limit` work IDs. The input `ranked` is assumed to already
/// be in the final score order. `fandoms_of` returns the fandom tags for a
/// work as canonical strings.
pub fn apply_diversity(
    ranked: Vec<WorkId>,
    known_fandoms: &std::collections::HashSet<String>,
    per_fandom_cap: usize,
    exploration_rate: f64,
    limit: usize,
    fandoms_of: &dyn Fn(&WorkId) -> Vec<String>,
) -> Vec<WorkId> {
    let cap = if per_fandom_cap == 0 {
        usize::MAX
    } else {
        per_fandom_cap
    };
    let exploration_slots = if exploration_rate > 0.0 {
        (exploration_rate * limit as f64).round() as usize
    } else {
        0
    };
    let keep_main = limit.saturating_sub(exploration_slots);

    let mut fandom_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut kept: Vec<WorkId> = Vec::new();
    let mut exploration_pool: Vec<WorkId> = Vec::new();

    for work in ranked {
        let fandoms = fandoms_of(&work);
        let is_known = fandoms.iter().any(|f| known_fandoms.contains(f));
        if is_known {
            let mut can_add = true;
            for f in &fandoms {
                let count = fandom_counts.get(f).copied().unwrap_or(0);
                if count >= cap {
                    can_add = false;
                    break;
                }
            }
            if can_add {
                for f in &fandoms {
                    *fandom_counts.entry(f.clone()).or_insert(0) += 1;
                }
                if kept.len() < keep_main {
                    kept.push(work);
                } else {
                    exploration_pool.push(work);
                }
            } else if exploration_pool.len() < exploration_slots {
                exploration_pool.push(work);
            }
        } else {
            // New fandom — goes to exploration pool if there's budget,
            // otherwise falls through to the main keep list when no
            // exploration slots are configured (pass-through mode).
            if exploration_slots > 0 {
                if exploration_pool.len() < exploration_slots {
                    exploration_pool.push(work);
                }
            } else if kept.len() < keep_main {
                kept.push(work);
            } else {
                exploration_pool.push(work);
            }
        }
    }

    let mut result = kept;
    result.extend(exploration_pool);
    result.truncate(limit);
    result
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
                    taste_signal: 0.0,
                    diversity_class: 0.0,
                },
                Candidate {
                    work_id: w2,
                    score: 5,
                    reason: "engine1".into(),
                    taste_signal: 0.0,
                    diversity_class: 0.0,
                },
            ],
            vec![Candidate {
                work_id: w1,
                score: 20,
                reason: "engine2".into(),
                taste_signal: 0.0,
                diversity_class: 0.0,
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
                    taste_signal: 0.0,
                    diversity_class: 0.0,
            },
            Candidate {
                work_id: w2,
                score: 20,
                reason: "base".into(),
                    taste_signal: 0.0,
                    diversity_class: 0.0,
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
                    taste_signal: 0.0,
                    diversity_class: 0.0,
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
                    taste_signal: 0.0,
                    diversity_class: 0.0,
        }];
        let mut affinities = std::collections::HashMap::new();
        affinities.insert(w.to_canonical_string(), -5_000); // 0.5x
        let result = apply_affinity_ranking(candidates, &affinities);
        assert_eq!(result[0].score, 5); // 10 * 0.5
    }

    #[test]
    fn apply_diversity_caps_per_fandom() {
        // Three works from the same fandom; cap = 2 → only first 2 kept.
        let w1 = WorkId::new();
        let w2 = WorkId::new();
        let w3 = WorkId::new();
        let fandom = "fandom:x".to_string();
        let fandoms_of = |_w: &WorkId| vec![fandom.clone()];
        let known: std::collections::HashSet<String> = [fandom.clone()].into_iter().collect();
        let result = apply_diversity(vec![w1, w2, w3], &known, 2, 0.0, 10, &fandoms_of);
        assert_eq!(result.len(), 2);
        assert_eq!(result, vec![w1, w2]);
    }

    #[test]
    fn apply_diversity_exploration_does_not_reorder_existing() {
        // With per_fandom_cap = 0 (unlimited) and 0% exploration, the list
        // passes through unchanged.
        let w1 = WorkId::new();
        let w2 = WorkId::new();
        let known: std::collections::HashSet<String> = std::collections::HashSet::new();
        let fandoms_of = |_w: &WorkId| Vec::new();
        let result = apply_diversity(vec![w1, w2], &known, 0, 0.0, 10, &fandoms_of);
        assert_eq!(result, vec![w1, w2]);
    }

    #[test]
    fn apply_theme_gravity_uniform_nudge() {
        let w1 = WorkId::new();
        let w2 = WorkId::new();
        let candidates = vec![
            Candidate { work_id: w1, score: 10, reason: "base".into(), taste_signal: 0.0, diversity_class: 0.0 },
            Candidate { work_id: w2, score: 10, reason: "base".into(), taste_signal: 0.0, diversity_class: 0.0 },
        ];
        let nudge_bp = 1000; // 1.1x
        let work_nudge = |_w: &WorkId| nudge_bp;
        let result = apply_theme_gravity(candidates, &work_nudge);
        assert_eq!(result[0].score, 11); // 10 * 1.1
        assert_eq!(result[1].score, 11);
    }

    #[test]
    fn apply_theme_gravity_selective_nudge_orders_correctly() {
        let w1 = WorkId::new();
        let w2 = WorkId::new();
        let candidates = vec![
            Candidate { work_id: w1, score: 10, reason: "base".into(), taste_signal: 0.0, diversity_class: 0.0 },
            Candidate { work_id: w2, score: 8, reason: "base".into(), taste_signal: 0.0, diversity_class: 0.0 },
        ];
        // w1 gets a 1.25x nudge (2500 bp); w2 gets nothing.
        let work_nudge = |w: &WorkId| {
            if w.to_canonical_string() == w1.to_canonical_string() { 2500 } else { 0 }
        };
        let result = apply_theme_gravity(candidates, &work_nudge);
        assert_eq!(result[0].work_id, w1); // 12 > 8 after nudge
        assert_eq!(result[0].score, 12);
        assert_eq!(result[1].score, 8);
    }

    #[test]
    fn apply_theme_gravity_zero_nudge_preserves_order() {
        let w1 = WorkId::new();
        let w2 = WorkId::new();
        let candidates = vec![
            Candidate { work_id: w1, score: 10, reason: "base".into(), taste_signal: 0.0, diversity_class: 0.0 },
            Candidate { work_id: w2, score: 5, reason: "base".into(), taste_signal: 0.0, diversity_class: 0.0 },
        ];
        let work_nudge = |_w: &WorkId| 0;
        let result = apply_theme_gravity(candidates, &work_nudge);
        assert_eq!(result[0].score, 10);
        assert_eq!(result[1].score, 5);
    }

    #[test]
    fn apply_theme_gravity_empty_candidates() {
        let candidates: Vec<Candidate> = vec![];
        let work_nudge = |_w: &WorkId| 1000;
        let result = apply_theme_gravity(candidates, &work_nudge);
        assert!(result.is_empty());
    }
}
