use std::sync::Arc;

use anyhow::Result;

use lorehaven_db::rec_strategy::{
    bandit_strategy, cooccurrence_strategy, completion_weight_strategy,
    curator_prior_strategy, RecContext, RecRegistry,
};
use lorehaven_db::Database;

/// Build a pluggable-mode registry from config (spec §16.1a, §16.3).
///
/// Strategy set is derived from `rec_enabled_strategies`:
/// - Empty vector → all strategies enabled
/// - Non-empty → only listed strategies are registered
pub fn build_registry(config: &crate::config::DiscoveryConfig) -> RecRegistry {
    let mut reg = RecRegistry::new(config.rec_rrf_k);

    // Available strategies (spec §16.1a).
    let available: Vec<(&str, lorehaven_db::rec_strategy::RecStrategyFn)> = vec![
        ("cooccurrence", cooccurrence_strategy()),
        ("bandit", bandit_strategy()),
        ("completion_weight", completion_weight_strategy()),
        ("curator_prior", curator_prior_strategy()),
        // TODO: time_decay, tag_graph, author_graph, sequential
        // TODO: external sidecar (feature-gated)
    ];

    let enabled = &config.rec_enabled_strategies;

    for (name, strategy) in available {
        if enabled.is_empty() || enabled.iter().any(|s| s == name) {
            reg.register(name, strategy);
        }
    }

    reg
}

/// Generate recommendations using the pluggable strategy registry.
pub async fn generate_with_registry(
    db: &Database,
    registry: &RecRegistry,
    account_id: &str,
    limit: usize,
) -> Result<Vec<String>> {
    let ctx = RecContext {
        account_id: account_id.to_string(),
        seen: vec![],
        cap: limit,
    };
    registry.generate(db, ctx).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(enabled: Vec<String>) -> crate::config::DiscoveryConfig {
        crate::config::DiscoveryConfig {
            rec_enabled_strategies: enabled,
            rec_rrf_k: 60.0,
            ..crate::config::DiscoveryConfig::default()
        }
    }

    #[test]
    fn build_registry_empty_enables_all() {
        let config = test_config(vec![]);
        let reg = build_registry(&config);
        // 4 strategies registered (cooccurrence, bandit, completion_weight, curator_prior)
        assert_eq!(reg.strategy_count(), 4);
    }

    #[test]
    fn build_registry_selects_subset() {
        let config = test_config(vec!["cooccurrence".to_string()]);
        let reg = build_registry(&config);
        assert_eq!(reg.strategy_count(), 1);
    }

    #[test]
    fn build_registry_unknown_strategy_ignored() {
        let config = test_config(vec!["nonexistent".to_string()]);
        let reg = build_registry(&config);
        assert_eq!(reg.strategy_count(), 0);
    }
}
