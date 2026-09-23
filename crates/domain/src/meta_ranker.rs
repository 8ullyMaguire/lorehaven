use rand::Rng;
use serde::{Deserialize, Serialize};

/// A recommendation strategy that can fill discovery slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyKind {
    /// Direct match against admin taste profile dimensions.
    TasteGravity,
    /// What high-resonance users bookmarked/rated/completed.
    ResonanceWeightedCollaborative,
    /// What users with similar reading history liked (unweighted by resonance).
    PureCollaborative,
    /// Raw kudos + bookmarks + reads, time-decayed.
    Popularity,
    /// Newest works, filtered by minimum quality.
    Recency,
    /// Finished works weighted higher, WIPs deprioritized.
    CompletionBoosted,
    /// Tags with highest admin engagement over 90 days.
    DynamicTagGravity,
    /// Works most-bookmarked by current Vanguard users.
    VanguardConsensus,
    /// Works matching instance topics, weighted by topic bonus.
    TopicAlignment,
    /// Works adjacent to but slightly outside admin taste.
    TasteProbe,
    /// Works farthest from admin taste that meet quality floor.
    Diversity,
    /// Recently completed or resurrected works.
    Lifecycle,
    /// Works linked by shared media references — cross-work discovery via faceclaims/moodboards/playlists.
    MediaReferenceCollaborative,
}

impl StrategyKind {
    /// All built-in strategies in the default evaluation order.
    pub fn all() -> Vec<StrategyKind> {
        vec![
            StrategyKind::TasteGravity,
            StrategyKind::ResonanceWeightedCollaborative,
            StrategyKind::PureCollaborative,
            StrategyKind::Popularity,
            StrategyKind::Recency,
            StrategyKind::CompletionBoosted,
            StrategyKind::DynamicTagGravity,
            StrategyKind::VanguardConsensus,
            StrategyKind::TopicAlignment,
            StrategyKind::TasteProbe,
            StrategyKind::Diversity,
            StrategyKind::Lifecycle,
            StrategyKind::MediaReferenceCollaborative,
        ]
    }

    /// Short identifier for logging and persistence.
    pub fn id(&self) -> &'static str {
        match self {
            StrategyKind::TasteGravity => "taste_gravity",
            StrategyKind::ResonanceWeightedCollaborative => "resonance_collab",
            StrategyKind::PureCollaborative => "collab",
            StrategyKind::Popularity => "popularity",
            StrategyKind::Recency => "recency",
            StrategyKind::CompletionBoosted => "completion",
            StrategyKind::DynamicTagGravity => "tag_gravity",
            StrategyKind::VanguardConsensus => "vanguard",
            StrategyKind::TopicAlignment => "topic",
            StrategyKind::TasteProbe => "probe",
            StrategyKind::Diversity => "diversity",
            StrategyKind::Lifecycle => "lifecycle",
            StrategyKind::MediaReferenceCollaborative => "media_ref_collab",
        }
    }
}

/// A Beta distribution over a strategy's success rate.
/// Used by Thompson Sampling to balance exploration and exploitation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BetaDistribution {
    /// Number of observed successes + prior.
    pub alpha: f64,
    /// Number of observed failures + prior.
    pub beta: f64,
}

impl BetaDistribution {
    /// Create a new Beta distribution with a uniform prior (Beta(1,1)).
    pub fn new() -> Self {
        BetaDistribution {
            alpha: 1.0,
            beta: 1.0,
        }
    }

    /// Create with an informative prior.
    pub fn with_prior(alpha: f64, beta: f64) -> Self {
        BetaDistribution {
            alpha: alpha.max(0.01),
            beta: beta.max(0.01),
        }
    }

    /// Sample from this Beta distribution using the provided RNG.
    /// Returns a value in [0, 1].
    pub fn sample<R: Rng>(&self, rng: &mut R) -> f64 {
        // Use the relationship: Beta(a,b) = Gamma(a,1) / (Gamma(a,1) + Gamma(b,1))
        // For simplicity, we use the fact that for integer or near-integer parameters,
        // we can sample via order statistics of uniform random variables.
        // For general case, we use the approximation via gamma distributions.
        let a = self.alpha;
        let b = self.beta;

        // Use the standard gamma distribution sampling
        let x = sample_gamma(rng, a, 1.0);
        let y = sample_gamma(rng, b, 1.0);

        if x + y == 0.0 {
            0.5
        } else {
            x / (x + y)
        }
    }

    /// Update the distribution with an observation.
    /// `success` counts as alpha increment, `failure` counts as beta increment.
    pub fn update(&mut self, success: bool) {
        if success {
            self.alpha += 1.0;
        } else {
            self.beta += 1.0;
        }
    }

    /// The mean of the distribution (expected success rate).
    pub fn mean(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }

    /// The variance of the distribution.
    pub fn variance(&self) -> f64 {
        let sum = self.alpha + self.beta;
        (self.alpha * self.beta) / (sum * sum * (sum + 1.0))
    }
}

impl Default for BetaDistribution {
    fn default() -> Self {
        Self::new()
    }
}

/// Sample from a Gamma distribution using the Marsaglia-Tsang method.
fn sample_gamma<R: Rng>(rng: &mut R, shape: f64, scale: f64) -> f64 {
    if shape < 1.0 {
        // Use the fact that Gamma(a) = Gamma(a+1) * U^(1/a)
        let u: f64 = rng.gen();
        return sample_gamma(rng, shape + 1.0, scale) * u.powf(1.0 / shape);
    }

    // Marsaglia-Tsang for shape >= 1
    let d = shape - 1.0 / 3.0;
    let c = 1.0 / (9.0 * d).sqrt();

    loop {
        let u: f64 = rng.gen();
        let v: f64 = rng.gen();

        // Box-Muller for normal approximation
        let z = (-2.0 * (1.0 - u).ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos();
        let x = d * (1.0 + c * z).powi(3);

        if x > 0.0 {
            // Acceptance condition
            let u2: f64 = rng.gen();
            if u2 < 1.0 - 0.0331 * z.powi(4) {
                return x * scale;
            }
            if u2 < (-z * z / 2.0).exp() {
                return x * scale;
            }
        }
    }
}

/// A strategy in the meta-ranker's pool with its performance distribution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyEntry {
    pub kind: StrategyKind,
    pub distribution: BetaDistribution,
    pub total_impressions: u64,
    pub total_successes: u64,
    /// Whether this strategy is locked by the admin (excluded from auto-disable).
    pub locked: bool,
    /// Whether this strategy is disabled.
    pub disabled: bool,
}

impl StrategyEntry {
    pub fn new(kind: StrategyKind) -> Self {
        StrategyEntry {
            kind,
            distribution: BetaDistribution::new(),
            total_impressions: 0,
            total_successes: 0,
            locked: false,
            disabled: false,
        }
    }

    pub fn with_prior(kind: StrategyKind, alpha: f64, beta: f64) -> Self {
        StrategyEntry {
            kind,
            distribution: BetaDistribution::with_prior(alpha, beta),
            total_impressions: 0,
            total_successes: 0,
            locked: false,
            disabled: false,
        }
    }

    /// Record an impression and whether it was successful.
    pub fn record_impression(&mut self, success: bool) {
        self.total_impressions += 1;
        if success {
            self.total_successes += 1;
        }
        self.distribution.update(success);
    }

    /// Expected success rate (mean of Beta distribution).
    pub fn expected_rate(&self) -> f64 {
        self.distribution.mean()
    }

    /// Whether this strategy has enough data to be considered for exploitation.
    pub fn has_sufficient_data(&self, min_impressions: u64) -> bool {
        self.total_impressions >= min_impressions
    }
}

/// Configuration for the meta-ranker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaRankerConfig {
    /// Percentage of discovery slots filled by random/exploratory strategies.
    pub exploration_percent: u8,
    /// Percentage filled by current best-ranked strategies.
    pub exploitation_percent: u8,
    /// Minimum impressions before a strategy is ranked.
    pub min_impressions_per_strategy: u64,
    /// Maximum number of active strategies.
    pub max_active_strategies: usize,
    /// Auto-disable threshold (success rate < this fraction of best strategy's rate).
    pub auto_disable_threshold: f64,
    /// Exploration bonus for newly installed candidates.
    pub candidate_exploration_bonus: f64,
    /// Resource limits for marketplace algorithms, keyed by subscription tier (spec §9.10.2).
    pub algo_limits: AlgoTierLimits,
}

/// Resource limits for marketplace algorithms per subscription tier (spec §9.10.2).
/// Free/unauthenticated users get lighter variants; subscribers get richer, more
/// compute-intensive algorithms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlgoTierLimits {
    /// Free / unauthenticated tier (default).
    pub base: AlgoResourceLimit,
    /// Author subscription tier.
    pub author: AlgoResourceLimit,
    /// Curator / Patron subscription tier.
    pub curator: AlgoResourceLimit,
}

/// Per-invocation resource limits for a single tier (spec §9.10.2).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AlgoResourceLimit {
    /// Maximum execution time in milliseconds.
    pub timeout_ms: u64,
    /// Maximum memory allocation in megabytes.
    pub max_size_mb: u64,
    /// Maximum host API calls per invocation.
    pub max_api_calls: u64,
}

/// Hard global maximum resource limits — no algorithm may exceed these regardless of tier.
pub const HARD_GLOBAL_TIMEOUT_MS: u64 = 200;
pub const HARD_GLOBAL_MAX_SIZE_MB: u64 = 50;

impl Default for MetaRankerConfig {
    fn default() -> Self {
        MetaRankerConfig {
            exploration_percent: 15,
            exploitation_percent: 85,
            min_impressions_per_strategy: 50,
            max_active_strategies: 20,
            auto_disable_threshold: 0.3,
            candidate_exploration_bonus: 2.0,
            algo_limits: AlgoTierLimits::default(),
        }
    }
}

impl Default for AlgoTierLimits {
    fn default() -> Self {
        Self {
            base: AlgoResourceLimit {
                timeout_ms: 25,
                max_size_mb: 5,
                max_api_calls: 500,
            },
            author: AlgoResourceLimit {
                timeout_ms: 50,
                max_size_mb: 10,
                max_api_calls: 1000,
            },
            curator: AlgoResourceLimit {
                timeout_ms: 100,
                max_size_mb: 20,
                max_api_calls: 2000,
            },
        }
    }
}

/// Select the appropriate resource limit for a user's subscription tier.
/// Returns the hardest limit between the tier's allowance and the global maximum.
pub fn select_algo_limit(tier: &str, limits: &AlgoTierLimits) -> AlgoResourceLimit {
    let tier_limit = match tier {
        "author" => limits.author,
        "curator" => limits.curator,
        _ => limits.base,
    };
    // Apply hard global maximum safety valves
    AlgoResourceLimit {
        timeout_ms: tier_limit.timeout_ms.min(HARD_GLOBAL_TIMEOUT_MS),
        max_size_mb: tier_limit.max_size_mb.min(HARD_GLOBAL_MAX_SIZE_MB),
        max_api_calls: tier_limit.max_api_calls,
    }
}

/// The meta-ranker: maintains a pool of strategies with Beta distributions,
/// uses Thompson Sampling to select strategies, and records outcomes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaRanker {
    pub strategies: Vec<StrategyEntry>,
    pub config: MetaRankerConfig,
}

impl MetaRanker {
    /// Create a new meta-ranker with all built-in strategies.
    pub fn new(config: MetaRankerConfig) -> Self {
        let strategies = StrategyKind::all()
            .into_iter()
            .map(StrategyEntry::new)
            .collect();
        MetaRanker { strategies, config }
    }

    /// Add a new strategy to the pool (for marketplace algorithms).
    pub fn add_strategy(&mut self, kind: StrategyKind, candidate_bonus: bool) {
        if self.strategies.iter().any(|s| s.kind == kind) {
            return; // already exists
        }
        let entry = if candidate_bonus {
            // Give new candidates a prior that encourages exploration
            StrategyEntry::with_prior(kind, 1.0 * self.config.candidate_exploration_bonus, 1.0)
        } else {
            StrategyEntry::new(kind)
        };
        self.strategies.push(entry);
    }

    /// Remove a strategy from the pool.
    pub fn remove_strategy(&mut self, kind: StrategyKind) {
        self.strategies.retain(|s| s.kind != kind);
    }

    /// Select which strategies should fill slots for the next batch.
    /// Returns a list of (strategy_kind, slot_percentage) tuples.
    /// `random` is used for Thompson Sampling.
    pub fn select_strategies<R: Rng>(
        &self,
        rng: &mut R,
        total_slots: usize,
    ) -> Vec<(StrategyKind, usize)> {
        if total_slots == 0 || self.strategies.is_empty() {
            return vec![];
        }

        // Separate active strategies into those with sufficient data and those without
        let active: Vec<&StrategyEntry> = self.strategies.iter().filter(|s| !s.disabled).collect();

        if active.is_empty() {
            return vec![];
        }

        // Sample from each strategy's Beta distribution
        let mut samples: Vec<(StrategyKind, f64)> = active
            .iter()
            .map(|s| (s.kind, s.distribution.sample(rng)))
            .collect();

        // Sort by sampled value descending
        samples.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        // Apply max_active_strategies limit
        if samples.len() > self.config.max_active_strategies {
            samples.truncate(self.config.max_active_strategies);
        }

        // Calculate exploration slots
        let exploration_slots =
            (total_slots as f64 * self.config.exploration_percent as f64 / 100.0).ceil() as usize;
        let exploitation_slots = total_slots.saturating_sub(exploration_slots);

        // Top strategy gets exploitation budget
        let mut result: Vec<(StrategyKind, usize)> = Vec::new();

        if let Some(&(top_kind, _)) = samples.first() {
            result.push((top_kind, exploitation_slots));
        }

        // Distribute exploration slots proportional to sample values
        if exploration_slots > 0 && samples.len() > 1 {
            let total_sample: f64 = samples.iter().map(|(_, s)| s).sum();
            if total_sample > 0.0 {
                for &(kind, sample) in &samples {
                    let share = (exploration_slots as f64 * sample / total_sample).round() as usize;
                    if share > 0 {
                        result.push((kind, share));
                    }
                }
            } else {
                // Uniform if all samples are zero
                let per_strategy = exploration_slots / samples.len();
                for &(kind, _) in &samples {
                    result.push((kind, per_strategy));
                }
            }
        }

        // Merge duplicates
        let mut merged: std::collections::HashMap<StrategyKind, usize> =
            std::collections::HashMap::new();
        for (kind, slots) in result {
            *merged.entry(kind).or_insert(0) += slots;
        }

        merged.into_iter().collect()
    }

    /// Record an outcome for a strategy.
    pub fn record_outcome(&mut self, kind: StrategyKind, success: bool) {
        if let Some(entry) = self.strategies.iter_mut().find(|s| s.kind == kind) {
            entry.record_impression(success);
        }
    }

    /// Rebalance: auto-disable strategies that underperform significantly.
    /// Returns the list of strategies that were disabled.
    pub fn rebalance(&mut self) -> Vec<StrategyKind> {
        let min_impressions = self.config.min_impressions_per_strategy;
        let threshold = self.config.auto_disable_threshold;

        // Find the best strategy's success rate
        let best_rate = self
            .strategies
            .iter()
            .filter(|s| !s.disabled && s.has_sufficient_data(min_impressions))
            .map(|s| s.expected_rate())
            .fold(0.0, f64::max);

        let mut disabled = Vec::new();
        for entry in &mut self.strategies {
            if entry.disabled || entry.locked {
                continue;
            }
            if !entry.has_sufficient_data(min_impressions) {
                continue;
            }
            if entry.expected_rate() < best_rate * threshold {
                entry.disabled = true;
                disabled.push(entry.kind);
            }
        }
        disabled
    }

    /// Lock a strategy (admin override: never auto-disable).
    pub fn lock_strategy(&mut self, kind: StrategyKind, locked: bool) {
        if let Some(entry) = self.strategies.iter_mut().find(|s| s.kind == kind) {
            entry.locked = locked;
        }
    }

    /// Enable or disable a strategy.
    pub fn set_disabled(&mut self, kind: StrategyKind, disabled: bool) {
        if let Some(entry) = self.strategies.iter_mut().find(|s| s.kind == kind) {
            entry.disabled = disabled;
        }
    }

    /// Get a summary of all strategies for the admin dashboard.
    pub fn summary(&self) -> Vec<StrategySummary> {
        self.strategies
            .iter()
            .map(|s| StrategySummary {
                kind: s.kind,
                impressions: s.total_impressions,
                successes: s.total_successes,
                expected_rate: s.expected_rate(),
                variance: s.distribution.variance(),
                locked: s.locked,
                disabled: s.disabled,
            })
            .collect()
    }
}

/// Summary of a strategy's performance for the admin dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategySummary {
    pub kind: StrategyKind,
    pub impressions: u64,
    pub successes: u64,
    pub expected_rate: f64,
    pub variance: f64,
    pub locked: bool,
    pub disabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    #[test]
    fn beta_distribution_new_is_uniform() {
        let beta = BetaDistribution::new();
        assert_eq!(beta.alpha, 1.0);
        assert_eq!(beta.beta, 1.0);
    }

    #[test]
    fn beta_distribution_mean() {
        let beta = BetaDistribution::with_prior(8.0, 2.0);
        assert!((beta.mean() - 0.8).abs() < 0.001);
    }

    #[test]
    fn beta_distribution_update() {
        let mut beta = BetaDistribution::new();
        beta.update(true);
        beta.update(true);
        beta.update(false);
        assert_eq!(beta.alpha, 3.0); // 1 prior + 2 successes
        assert_eq!(beta.beta, 2.0); // 1 prior + 1 failure
        assert!((beta.mean() - 0.6).abs() < 0.001);
    }

    #[test]
    fn beta_distribution_sample_bounds() {
        let beta = BetaDistribution::with_prior(8.0, 2.0);
        let mut rng = StdRng::seed_from_u64(42);
        for _ in 0..1000 {
            let s = beta.sample(&mut rng);
            assert!(s >= 0.0 && s <= 1.0, "sample {} out of bounds", s);
        }
    }

    #[test]
    fn beta_distribution_sample_mean() {
        let beta = BetaDistribution::with_prior(8.0, 2.0);
        let mut rng = StdRng::seed_from_u64(42);
        let n = 100_000;
        let sum: f64 = (0..n).map(|_| beta.sample(&mut rng)).sum();
        let mean = sum / n as f64;
        // Should be close to 0.8
        assert!(
            (mean - 0.8).abs() < 0.01,
            "sample mean {} not close to 0.8",
            mean
        );
    }

    #[test]
    fn strategy_entry_record_impression() {
        let mut entry = StrategyEntry::new(StrategyKind::Popularity);
        entry.record_impression(true);
        entry.record_impression(false);
        entry.record_impression(true);
        assert_eq!(entry.total_impressions, 3);
        assert_eq!(entry.total_successes, 2);
        // Beta(1,1) prior + 2 successes + 1 failure = Beta(3,2) → mean = 3/5 = 0.6
        assert!((entry.expected_rate() - 0.6).abs() < 0.01);
    }

    #[test]
    fn meta_ranker_new_has_all_builtins() {
        let config = MetaRankerConfig::default();
        let ranker = MetaRanker::new(config);
        assert_eq!(ranker.strategies.len(), 13);
    }

    #[test]
    fn meta_ranker_add_strategy() {
        let config = MetaRankerConfig::default();
        let mut ranker = MetaRanker::new(config);
        let initial_count = ranker.strategies.len();
        ranker.add_strategy(StrategyKind::TasteGravity, false); // duplicate, no-op
        assert_eq!(ranker.strategies.len(), initial_count);
    }

    #[test]
    fn meta_ranker_select_strategies_basic() {
        let config = MetaRankerConfig::default();
        let ranker = MetaRanker::new(config);
        let mut rng = StdRng::seed_from_u64(42);
        let selection = ranker.select_strategies(&mut rng, 100);

        assert!(!selection.is_empty());
        // Total slots should be close to 100
        let total: usize = selection.iter().map(|(_, s)| s).sum();
        assert!(total <= 100);
    }

    #[test]
    fn meta_ranker_top_strategy_gets_exploitation() {
        // Set up a ranker where one strategy is clearly dominant
        let config = MetaRankerConfig::default();
        let mut ranker = MetaRanker::new(config);

        // Give TasteGravity many successes
        for _ in 0..100 {
            ranker.record_outcome(StrategyKind::TasteGravity, true);
        }
        // Give Popularity many failures
        for _ in 0..100 {
            ranker.record_outcome(StrategyKind::Popularity, false);
        }

        let mut rng = StdRng::seed_from_u64(42);
        let mut taste_gravity_slots = 0;
        let mut popularity_slots = 0;

        // Run many times to average out randomness
        for _ in 0..100 {
            let selection = ranker.select_strategies(&mut rng, 100);
            for (kind, slots) in &selection {
                match kind {
                    StrategyKind::TasteGravity => taste_gravity_slots += slots,
                    StrategyKind::Popularity => popularity_slots += slots,
                    _ => {}
                }
            }
        }

        // TasteGravity should get significantly more slots than Popularity
        assert!(
            taste_gravity_slots > popularity_slots * 2,
            "taste_gravity={} should be >> popularity={}",
            taste_gravity_slots,
            popularity_slots
        );
    }

    #[test]
    fn meta_ranker_rebalance_disables_underperformers() {
        let config = MetaRankerConfig::default();
        let mut ranker = MetaRanker::new(config);

        // Make TasteGravity the best strategy
        for _ in 0..100 {
            ranker.record_outcome(StrategyKind::TasteGravity, true);
        }
        // Make Popularity a terrible strategy
        for _ in 0..100 {
            ranker.record_outcome(StrategyKind::Popularity, false);
        }

        let disabled = ranker.rebalance();
        assert!(disabled.contains(&StrategyKind::Popularity));
        assert!(!disabled.contains(&StrategyKind::TasteGravity));
    }

    #[test]
    fn meta_ranker_rebalance_respects_lock() {
        let config = MetaRankerConfig::default();
        let mut ranker = MetaRanker::new(config);

        // Make TasteGravity the best strategy
        for _ in 0..100 {
            ranker.record_outcome(StrategyKind::TasteGravity, true);
        }
        // Make Popularity terrible but locked
        for _ in 0..100 {
            ranker.record_outcome(StrategyKind::Popularity, false);
        }
        ranker.lock_strategy(StrategyKind::Popularity, true);

        let disabled = ranker.rebalance();
        assert!(!disabled.contains(&StrategyKind::Popularity));
        assert!(
            ranker
                .strategies
                .iter()
                .find(|s| s.kind == StrategyKind::Popularity)
                .unwrap()
                .locked
        );
    }

    #[test]
    fn meta_ranker_summary() {
        let config = MetaRankerConfig::default();
        let mut ranker = MetaRanker::new(config);
        ranker.record_outcome(StrategyKind::TasteGravity, true);
        ranker.record_outcome(StrategyKind::Popularity, false);

        let summary = ranker.summary();
        assert_eq!(summary.len(), 13);

        let tg = summary
            .iter()
            .find(|s| s.kind == StrategyKind::TasteGravity)
            .unwrap();
        assert_eq!(tg.impressions, 1);
        assert_eq!(tg.successes, 1);

        let pop = summary
            .iter()
            .find(|s| s.kind == StrategyKind::Popularity)
            .unwrap();
        assert_eq!(pop.impressions, 1);
        assert_eq!(pop.successes, 0);
    }

    #[test]
    fn strategy_kind_id_roundtrip() {
        for kind in StrategyKind::all() {
            let id = kind.id();
            assert!(!id.is_empty());
        }
    }

    #[test]
    fn meta_ranker_zero_slots() {
        let config = MetaRankerConfig::default();
        let ranker = MetaRanker::new(config);
        let mut rng = StdRng::seed_from_u64(42);
        let selection = ranker.select_strategies(&mut rng, 0);
        assert!(selection.is_empty());
    }

    #[test]
    fn meta_ranker_candidate_bonus() {
        let config = MetaRankerConfig::default();
        let mut ranker = MetaRanker::new(config);
        let initial_count = ranker.strategies.len();

        // Add a custom strategy (not in built-in pool)
        let custom_kind = StrategyKind::TasteGravity; // reuse but with bonus
        ranker.add_strategy(custom_kind, true);
        // No duplicate added
        assert_eq!(ranker.strategies.len(), initial_count);
    }

    #[test]
    fn algo_tier_limits_default() {
        let limits = AlgoTierLimits::default();
        assert_eq!(limits.base.timeout_ms, 25);
        assert_eq!(limits.base.max_size_mb, 5);
        assert_eq!(limits.base.max_api_calls, 500);
        assert_eq!(limits.author.timeout_ms, 50);
        assert_eq!(limits.curator.timeout_ms, 100);
    }

    #[test]
    fn select_algo_limit_base() {
        let limits = AlgoTierLimits::default();
        let limit = select_algo_limit("free", &limits);
        assert_eq!(limit.timeout_ms, 25);
        assert_eq!(limit.max_size_mb, 5);
        assert_eq!(limit.max_api_calls, 500);
    }

    #[test]
    fn select_algo_limit_author() {
        let limits = AlgoTierLimits::default();
        let limit = select_algo_limit("author", &limits);
        assert_eq!(limit.timeout_ms, 50);
        assert_eq!(limit.max_size_mb, 10);
        assert_eq!(limit.max_api_calls, 1000);
    }

    #[test]
    fn select_algo_limit_curator() {
        let limits = AlgoTierLimits::default();
        let limit = select_algo_limit("curator", &limits);
        assert_eq!(limit.timeout_ms, 100);
        assert_eq!(limit.max_size_mb, 20);
        assert_eq!(limit.max_api_calls, 2000);
    }

    #[test]
    fn select_algo_limit_hard_global_cap() {
        let limits = AlgoTierLimits {
            base: AlgoResourceLimit {
                timeout_ms: 25,
                max_size_mb: 5,
                max_api_calls: 500,
            },
            author: AlgoResourceLimit {
                timeout_ms: 500,  // exceeds 200ms hard cap
                max_size_mb: 100, // exceeds 50MB hard cap
                max_api_calls: 1000,
            },
            curator: AlgoResourceLimit {
                timeout_ms: 100,
                max_size_mb: 20,
                max_api_calls: 2000,
            },
        };
        let limit = select_algo_limit("author", &limits);
        assert_eq!(limit.timeout_ms, 200); // capped
        assert_eq!(limit.max_size_mb, 50); // capped
        assert_eq!(limit.max_api_calls, 1000); // not capped
    }
}
