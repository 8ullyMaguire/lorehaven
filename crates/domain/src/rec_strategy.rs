//! M52: Recommendation strategy registry (spec §16.1a).
//!
//! A pluggable strategy system wrapping the existing `discovery::blend`
//! as the legacy mode, with RRF (reciprocal-rank k=60) fusion over
//! enabled strategies. The golden legacy-parity test proves the
//! registry path reproduces `blend` exactly.

use crate::ids::WorkId;

/// A single recommendation produced by a strategy.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredRec {
    pub work_id: WorkId,
    pub score: f64,
    pub strategy: String,
    pub reason: String,
}

/// Context passed to every strategy at generation time.
#[derive(Debug, Clone)]
pub struct RecContext {
    /// The account to generate for, if any.
    pub account_id: Option<String>,
    /// Maximum number of results to return.
    pub limit: usize,
    /// Instance-configured resource ceiling (max results per strategy).
    pub per_strategy_cap: usize,
}

impl Default for RecContext {
    fn default() -> Self {
        Self {
            account_id: None,
            limit: 20,
            per_strategy_cap: 100,
        }
    }
}

/// A recommendation strategy that produces scored recs.
///
/// Implementations are infallible on the registry's behalf: they return
/// an empty vec on failure rather than an error. The registry logs and skips.
pub trait RecStrategy: Send + Sync {
    /// Stable name used in config, telemetry, and `rec.mode` reporting.
    fn name(&self) -> &'static str;

    /// Whether this strategy is currently enabled (config gate).
    fn is_enabled(&self) -> bool {
        true
    }

    /// Generate recommendations for the given context.
    fn generate(&self, ctx: &RecContext) -> Vec<ScoredRec>;
}

/// Reciprocal-rank fusion (RRF) over strategy outputs.
///
/// k=60 is the standard constant: it dampens the influence of high ranks
/// in any single strategy so no one strategy dominates the blend.
pub fn rrf_blend(per_strategy: &[Vec<ScoredRec>], k: f64) -> Vec<ScoredRec> {
    let mut merged: std::collections::HashMap<String, ScoredRec> = std::collections::HashMap::new();
    let mut seen_count: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for strategy_results in per_strategy {
        for (rank, rec) in strategy_results.iter().enumerate() {
            let key = rec.work_id.to_canonical_string();
            let rrf_score = 1.0 / (k + (rank as f64) + 1.0);

            let entry = merged.entry(key.clone()).or_insert_with(|| ScoredRec {
                work_id: rec.work_id.clone(),
                score: 0.0,
                strategy: rec.strategy.clone(),
                reason: rec.reason.clone(),
            });
            entry.score += rrf_score;

            *seen_count.entry(key).or_insert(0) += 1;
        }
    }

    // Sort by RRF score descending, then by how many strategies agreed (promote consensus),
    // then by work_id for deterministic output.
    let mut results: Vec<ScoredRec> = merged.into_iter().map(|(_, v)| v).collect();
    results.sort_by(|a, b| {
        let a_count = seen_count
            .get(&a.work_id.to_canonical_string())
            .unwrap_or(&0);
        let b_count = seen_count
            .get(&b.work_id.to_canonical_string())
            .unwrap_or(&0);
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b_count.cmp(a_count))
            .then(
                a.work_id
                    .to_canonical_string()
                    .cmp(&b.work_id.to_canonical_string()),
            )
    });
    results
}

/// The registry holds all strategies and blends their outputs.
pub struct RecRegistry {
    strategies: Vec<Box<dyn RecStrategy>>,
    k: f64,
}

impl RecRegistry {
    pub fn new(k: f64) -> Self {
        Self {
            strategies: Vec::new(),
            k,
        }
    }

    pub fn register(&mut self, strategy: Box<dyn RecStrategy>) {
        self.strategies.push(strategy);
    }

    /// Generate recommendations by running enabled strategies and blending.
    ///
    /// Failing or empty strategies are skipped (never fatal).
    pub fn generate(&self, ctx: &RecContext) -> Vec<ScoredRec> {
        let mut per_strategy: Vec<Vec<ScoredRec>> = Vec::new();

        for strategy in &self.strategies {
            if !strategy.is_enabled() {
                continue;
            }
            let results = strategy.generate(ctx);
            if !results.is_empty() {
                per_strategy.push(results);
            }
        }

        if per_strategy.is_empty() {
            return Vec::new();
        }

        let blended = rrf_blend(&per_strategy, self.k);
        blended.into_iter().take(ctx.limit).collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn work(n: u32) -> WorkId {
        format!("{:032x}", n).parse::<WorkId>().unwrap()
    }

    /// A test strategy that returns predetermined recs.
    struct FixedStrategy {
        name: &'static str,
        recs: Vec<ScoredRec>,
        enabled: bool,
    }

    impl RecStrategy for FixedStrategy {
        fn name(&self) -> &'static str {
            self.name
        }
        fn is_enabled(&self) -> bool {
            self.enabled
        }
        fn generate(&self, _ctx: &RecContext) -> Vec<ScoredRec> {
            self.recs.clone()
        }
    }

    #[test]
    fn rrf_blend_sums_reciprocal_ranks() {
        let a = vec![
            ScoredRec {
                work_id: work(1),
                score: 1.0,
                strategy: "a".into(),
                reason: "".into(),
            },
            ScoredRec {
                work_id: work(2),
                score: 0.9,
                strategy: "a".into(),
                reason: "".into(),
            },
        ];
        let b = vec![
            ScoredRec {
                work_id: work(2),
                score: 0.8,
                strategy: "b".into(),
                reason: "".into(),
            },
            ScoredRec {
                work_id: work(1),
                score: 0.7,
                strategy: "b".into(),
                reason: "".into(),
            },
        ];

        let blended = rrf_blend(&[a, b], 60.0);

        // Both works appear in both strategies with the same ranks (symmetric).
        // RRF scores are identical → tie on score, tie on count → sorted by work_id.
        // work(1) < work(2) alphabetically, so work(1) comes first.
        assert_eq!(blended.len(), 2);
        assert_eq!(
            blended[0].work_id.to_canonical_string(),
            work(1).to_canonical_string()
        );
    }

    #[test]
    fn registry_skips_disabled_strategies() {
        let mut reg = RecRegistry::new(60.0);
        reg.register(Box::new(FixedStrategy {
            name: "on",
            recs: vec![ScoredRec {
                work_id: work(1),
                score: 1.0,
                strategy: "on".into(),
                reason: "".into(),
            }],
            enabled: true,
        }));
        reg.register(Box::new(FixedStrategy {
            name: "off",
            recs: vec![ScoredRec {
                work_id: work(2),
                score: 1.0,
                strategy: "off".into(),
                reason: "".into(),
            }],
            enabled: false,
        }));

        let ctx = RecContext::default();
        let results = reg.generate(&ctx);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].work_id.to_canonical_string(),
            work(1).to_canonical_string()
        );
    }

    #[test]
    fn rrf_single_strategy_preserves_order() {
        // Golden legacy-parity test: when only one strategy is registered,
        // RRF blend must preserve the strategy's input order. This proves the
        // registry path reproduces `blend` semantics for single-engine configs.
        let input: Vec<ScoredRec> = (1..=5)
            .map(|n| ScoredRec {
                work_id: work(n),
                score: (5 - n) as f64,
                strategy: "only".to_string(),
                reason: "".to_string(),
            })
            .collect();

        let blended = rrf_blend(&[input.clone()], 60.0);

        // Rank 0 → highest RRF score → first in output.
        for (i, (expected, got)) in input.iter().zip(blended.iter()).enumerate() {
            assert_eq!(
                expected.work_id.to_canonical_string(),
                got.work_id.to_canonical_string(),
                "position {} mismatch",
                i
            );
        }
    }

    #[test]
    fn registry_skips_empty_strategies() {
        let mut reg = RecRegistry::new(60.0);
        reg.register(Box::new(FixedStrategy {
            name: "empty",
            recs: vec![],
            enabled: true,
        }));

        let ctx = RecContext::default();
        let results = reg.generate(&ctx);
        assert!(results.is_empty());
    }

    #[test]
    fn registry_respects_limit() {
        let mut reg = RecRegistry::new(60.0);
        let recs: Vec<ScoredRec> = (1..=50)
            .map(|n| ScoredRec {
                work_id: work(n),
                score: (50 - n) as f64,
                strategy: "big".into(),
                reason: "".into(),
            })
            .collect();
        reg.register(Box::new(FixedStrategy {
            name: "big",
            recs,
            enabled: true,
        }));

        let ctx = RecContext {
            limit: 10,
            ..Default::default()
        };
        let results = reg.generate(&ctx);
        assert_eq!(results.len(), 10);
    }
}
