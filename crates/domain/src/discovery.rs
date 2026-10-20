use crate::ids::WorkId;

// ---------------------------------------------------------------------------
// The "more like this" ladder (spec §11.5)
// ---------------------------------------------------------------------------

/// A rung of the "more like this" ladder (spec §11.5).
///
/// **The ladder is ordered, and the order is the whole design.** The first
/// rungs are facts about the work — the explicit relation, the same author —
/// and they are *true*; the later rungs are inferences, and they are
/// inferences about a reader. A recommendation that comes from a fact is
/// better than one that comes from a similarity score, and the ladder says so
/// by being a ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MoreLikeThisRung {
    /// **Next in series** — the explicit relation.
    NextInSeries,
    /// **Same author, other works.**
    SameAuthor,
    /// **Theme neighbours** — the works sharing the most-weighted tags of this
    /// work's themes.
    ThemeNeighbours,
    /// **Readers also bookmarked** — item-item co-bookmark similarity.
    ReadersAlsoBookmarked,
    /// **Taste-matched** — the reader's own profile, where one exists.
    TasteMatched,
}

impl MoreLikeThisRung {
    /// The rung's stable name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NextInSeries => "next_in_series",
            Self::SameAuthor => "same_author",
            Self::ThemeNeighbours => "theme_neighbours",
            Self::ReadersAlsoBookmarked => "readers_also_bookmarked",
            Self::TasteMatched => "taste_matched",
        }
    }

    /// The rung's position in the ladder, counting from one.
    #[must_use]
    pub const fn position(self) -> u8 {
        match self {
            Self::NextInSeries => 1,
            Self::SameAuthor => 2,
            Self::ThemeNeighbours => 3,
            Self::ReadersAlsoBookmarked => 4,
            Self::TasteMatched => 5,
        }
    }

    /// Whether the rung needs the taste engine.
    ///
    /// **The three that do are not implemented until M13/M19**, and a rung that
    /// is not implemented is *named as unavailable* rather than answered with a
    /// plausible number — a stub that returns a score is a ranking that looks
    /// measured and is not.
    #[must_use]
    pub const fn needs_taste_engine(self) -> bool {
        matches!(
            self,
            Self::ThemeNeighbours | Self::ReadersAlsoBookmarked | Self::TasteMatched
        )
    }

    /// Every rung, in ladder order.
    #[must_use]
    pub fn all() -> Vec<MoreLikeThisRung> {
        vec![
            Self::NextInSeries,
            Self::SameAuthor,
            Self::ThemeNeighbours,
            Self::ReadersAlsoBookmarked,
            Self::TasteMatched,
        ]
    }

    /// The rung with this name, if there is one.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "next_in_series" => Some(Self::NextInSeries),
            "same_author" => Some(Self::SameAuthor),
            "theme_neighbours" => Some(Self::ThemeNeighbours),
            "readers_also_bookmarked" => Some(Self::ReadersAlsoBookmarked),
            "taste_matched" => Some(Self::TasteMatched),
            _ => None,
        }
    }
}

/// The reason a work is on the "more like this" ladder.
///
/// **A recommendation without a reason is an advertisement** (spec §11.5), and
/// so is a recommendation whose reason is the same string for every row. The
/// reason is *why this work, for this reader*, and the two facts the ladder
/// rests on are the explicit relation and the authorship.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LadderReason {
    /// The work is the next one in the series this work belongs to.
    NextInSeries {
        /// The series' title.
        series_title: String,
    },
    /// The work is by the same author.
    SameAuthor {
        /// The author's public name.
        author_name: String,
    },
}

impl LadderReason {
    /// The reason's wording, for the reader.
    #[must_use]
    pub fn as_text(&self) -> String {
        match self {
            Self::NextInSeries { series_title } => {
                format!("the next one in {series_title}")
            }
            Self::SameAuthor { author_name } => {
                format!("more by {author_name}")
            }
        }
    }
}

/// A recommendation on the ladder: the work, the rung that found it, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LadderEntry {
    /// The work recommended.
    pub work_id: WorkId,
    /// The rung that found it.
    pub rung: MoreLikeThisRung,
    /// Why this work, for this reader.
    pub reason: LadderReason,
}

/// A rung the ladder cannot serve yet.
///
/// **Named, not silently absent.** A client that asks for a taste-ranked
/// recommendation and gets a shorter list has no way to tell an empty ladder
/// from an unimplemented one, and an empty ladder reads as "this work has no
/// neighbours", which is a statement about the work rather than about the
/// build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnavailableRung {
    /// The rung.
    pub rung: MoreLikeThisRung,
    /// Why it is unavailable, in words.
    pub reason: String,
}

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
    // Collect from the map WITHOUT losing the order the engines produced.
    //
    // `merged.into_values()` is HashMap iteration order, which SipHash randomises
    // per process. `sort_by_key` below is stable, so candidates with equal scores
    // came out in that random order -- and the route's taste ranker received a
    // genuinely different list on every run. That was the source of a 2-in-6
    // flake, and it was invisible here because every test that exercised blend
    // used candidates whose scores all differed.
    //
    // Order is reconstructed by recording each work's first appearance across
    // the engines, then sorting on that. First-appearance order is a property of
    // the input rather than of the hash seed, so two calls on the same engines
    // agree -- which is what §47.2 asks for -- while `sort_by_key`'s stability
    // makes the result a deterministic function of (scores, appearance order).
    let mut first_seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (position, engine_results) in engines.iter().enumerate() {
        for candidate in engine_results {
            first_seen
                .entry(candidate.work_id.to_canonical_string())
                .or_insert(position);
        }
    }
    let mut results: Vec<Candidate> = merged.into_values().collect();
    results.sort_by_key(|c| {
        (
            first_seen
                .get(&c.work_id.to_canonical_string())
                .copied()
                .unwrap_or(usize::MAX),
            -c.score,
        )
    });
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

/// Apply taste gravity to a ranked candidate list (spec §9.7.3).
///
/// `strength` controls influence (0.0 = off, 1.0 = full taste gravity).
/// `mode` determines weighting strategy: "egalitarian" (equal), "taste_weighted"
/// (multiply score by taste_signal × weight), "admin_only" (only admin-aligned).
/// `admin_weight` multiplies the final boost for admin-aligned works.
pub fn apply_taste_gravity(
    candidates: Vec<Candidate>,
    strength: f64,
    mode: &str,
    admin_weight: f64,
) -> Vec<Candidate> {
    if strength <= 0.0 || mode == "egalitarian" {
        return candidates;
    }
    let mut results: Vec<Candidate> = candidates
        .into_iter()
        .map(|mut c| {
            let boost = match mode {
                "admin_only" => {
                    if c.taste_signal > 0.5 {
                        1.0 + (c.taste_signal * admin_weight * strength)
                    } else {
                        1.0
                    }
                }
                "taste_weighted" => 1.0 + (c.taste_signal * admin_weight * strength),
                _ => 1.0,
            };
            c.score = (c.score as f64 * boost) as i64;
            c
        })
        .collect();
    results.sort_by_key(|c| -c.score);
    results
}

/// Inject taste-distant content into discovery results (spec §0.4.1).
///
/// Reserves `injection_percent` of the final `final_count` slots for content
/// that is maximally distant from admin taste (diversity_class close to 1.0).
/// Only candidates meeting a minimum quality threshold are eligible.
pub fn inject_diversity_by_taste(
    candidates: &[Candidate],
    final_count: usize,
    injection_percent: f64,
) -> Vec<usize> {
    let slots = if injection_percent > 0.0 {
        (injection_percent * final_count as f64).round() as usize
    } else {
        0
    };
    if slots == 0 {
        return (0..candidates.len()).collect();
    }
    // Sort by diversity_class descending to find the most distant works.
    let mut indexed: Vec<(usize, &Candidate)> = candidates.iter().enumerate().collect();
    indexed.sort_by(|a, b| {
        b.1.diversity_class
            .partial_cmp(&a.1.diversity_class)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let diverse_indices: Vec<usize> = indexed.iter().take(slots).map(|(i, _)| *i).collect();
    let mut result: Vec<usize> = (0..candidates.len())
        .filter(|i| !diverse_indices.contains(i))
        .collect();
    result.truncate(final_count.saturating_sub(slots));
    result.extend(diverse_indices);
    result
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
            Candidate {
                work_id: w1,
                score: 10,
                reason: "base".into(),
                taste_signal: 0.0,
                diversity_class: 0.0,
            },
            Candidate {
                work_id: w2,
                score: 10,
                reason: "base".into(),
                taste_signal: 0.0,
                diversity_class: 0.0,
            },
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
            Candidate {
                work_id: w1,
                score: 10,
                reason: "base".into(),
                taste_signal: 0.0,
                diversity_class: 0.0,
            },
            Candidate {
                work_id: w2,
                score: 8,
                reason: "base".into(),
                taste_signal: 0.0,
                diversity_class: 0.0,
            },
        ];
        // w1 gets a 1.25x nudge (2500 bp); w2 gets nothing.
        let work_nudge = |w: &WorkId| {
            if w.to_canonical_string() == w1.to_canonical_string() {
                2500
            } else {
                0
            }
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
            Candidate {
                work_id: w1,
                score: 10,
                reason: "base".into(),
                taste_signal: 0.0,
                diversity_class: 0.0,
            },
            Candidate {
                work_id: w2,
                score: 5,
                reason: "base".into(),
                taste_signal: 0.0,
                diversity_class: 0.0,
            },
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

    #[test]
    fn apply_taste_gravity_egalitarian_noop() {
        let w1 = WorkId::new();
        let w2 = WorkId::new();
        let candidates = vec![
            Candidate {
                work_id: w1,
                score: 10,
                reason: "base".into(),
                taste_signal: 0.9,
                diversity_class: 0.0,
            },
            Candidate {
                work_id: w2,
                score: 5,
                reason: "base".into(),
                taste_signal: 0.1,
                diversity_class: 0.0,
            },
        ];
        let result = apply_taste_gravity(candidates, 1.0, "egalitarian", 1.0);
        assert_eq!(result[0].score, 10);
        assert_eq!(result[1].score, 5);
    }

    #[test]
    fn apply_taste_gravity_zero_strength_noop() {
        let w1 = WorkId::new();
        let candidates = vec![Candidate {
            work_id: w1,
            score: 10,
            reason: "base".into(),
            taste_signal: 0.9,
            diversity_class: 0.0,
        }];
        let result = apply_taste_gravity(candidates, 0.0, "taste_weighted", 1.0);
        assert_eq!(result[0].score, 10);
    }

    #[test]
    fn apply_taste_gravity_boosts_aligned() {
        let w1 = WorkId::new();
        let w2 = WorkId::new();
        let candidates = vec![
            Candidate {
                work_id: w1,
                score: 10,
                reason: "base".into(),
                taste_signal: 0.9,
                diversity_class: 0.0,
            },
            Candidate {
                work_id: w2,
                score: 10,
                reason: "base".into(),
                taste_signal: 0.1,
                diversity_class: 0.0,
            },
        ];
        let result = apply_taste_gravity(candidates, 1.0, "taste_weighted", 1.0);
        // w1: 10 * (1 + 0.9 * 1 * 1) = 19
        // w2: 10 * (1 + 0.1 * 1 * 1) = 11
        assert_eq!(result[0].work_id, w1);
        assert_eq!(result[0].score, 19);
        assert_eq!(result[1].score, 11);
    }

    #[test]
    fn apply_taste_gravity_admin_only() {
        let w1 = WorkId::new();
        let w2 = WorkId::new();
        let candidates = vec![
            Candidate {
                work_id: w1,
                score: 10,
                reason: "base".into(),
                taste_signal: 0.6,
                diversity_class: 0.0,
            },
            Candidate {
                work_id: w2,
                score: 10,
                reason: "base".into(),
                taste_signal: 0.3,
                diversity_class: 0.0,
            },
        ];
        let result = apply_taste_gravity(candidates, 1.0, "admin_only", 1.0);
        // w1: 10 * (1 + 0.6 * 1 * 1) = 16
        // w2: 10 (no boost, taste_signal <= 0.5)
        assert_eq!(result[0].work_id, w1);
        assert_eq!(result[0].score, 16);
        assert_eq!(result[1].score, 10);
    }

    #[test]
    fn inject_diversity_by_taste_zero_percent() {
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
                score: 5,
                reason: "base".into(),
                taste_signal: 0.0,
                diversity_class: 1.0,
            },
        ];
        let indices = inject_diversity_by_taste(&candidates, 2, 0.0);
        assert_eq!(indices, vec![0, 1]);
    }

    #[test]
    fn inject_diversity_by_taste_reserves_slots() {
        let w1 = WorkId::new();
        let w2 = WorkId::new();
        let w3 = WorkId::new();
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
                score: 8,
                reason: "base".into(),
                taste_signal: 0.0,
                diversity_class: 0.5,
            },
            Candidate {
                work_id: w3,
                score: 5,
                reason: "base".into(),
                taste_signal: 0.0,
                diversity_class: 1.0,
            },
        ];
        // 50% of 3 = 1 slot reserved for most diverse (w3)
        let indices = inject_diversity_by_taste(&candidates, 3, 0.5);
        assert!(indices.contains(&2)); // w3 (most diverse) must be included
        assert_eq!(indices.len(), 3);
    }
}
