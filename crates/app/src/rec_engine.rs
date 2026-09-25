use anyhow::Result;

use lorehaven_db::rec_strategy::{default_strategies, RecContext, RecRegistry, StrategyFactory};
use lorehaven_db::Database;

/// Every strategy name this build can produce, in a stable order.
///
/// Sourced from `rec_strategy::default_strategies` rather than restated here.
/// The two lists used to be independent: this one carried
/// `// TODO: time_decay, tag_graph, author_graph, sequential` while all eight
/// strategies were implemented and tested in `rec_strategy.rs`. Eight strategies
/// passed their tests and four of them could never run in production. One list
/// means the next strategy cannot be half-wired again.
///
/// Returns owned strings: the factories map is rebuilt on each call, so its
/// keys do not outlive it.
pub fn available_strategies() -> Vec<String> {
    let mut names: Vec<String> = default_strategies().keys().cloned().collect();
    // HashMap iteration order is not stable, and an unstable registry order
    // makes the RRF blend's tie-breaking untestable.
    names.sort();
    names
}

/// Build a pluggable-mode registry from config (spec §16.1a, §16.3).
///
/// Strategy set is derived from `rec_enabled_strategies`:
/// - Empty vector → all strategies enabled
/// - Non-empty → only listed strategies are registered
pub fn build_registry(config: &crate::config::DiscoveryConfig) -> RecRegistry {
    let factories: std::collections::HashMap<String, StrategyFactory> = default_strategies();
    let mut reg = RecRegistry::new(config.rec_rrf_k);
    let enabled = &config.rec_enabled_strategies;

    for name in available_strategies() {
        if !enabled.is_empty() && !enabled.contains(&name) {
            continue;
        }
        if let Some(factory) = factories.get(&name) {
            reg.register(name, factory());
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
    fn build_registry_empty_enables_every_available_strategy() {
        let config = test_config(vec![]);
        let reg = build_registry(&config);
        // Asserted against the source of truth, not a literal. The literal `4`
        // is what let four implemented strategies sit unreachable behind a
        // TODO while this test stayed green.
        assert_eq!(reg.strategy_count(), available_strategies().len());
        assert!(
            reg.strategy_count() >= 8,
            "spec §16.1a names eight strategies; found {}",
            reg.strategy_count()
        );
    }

    #[test]
    fn every_spec_strategy_is_reachable() {
        // The regression this change exists to prevent, named explicitly: each
        // strategy the spec lists must be registrable, not merely implemented.
        for name in [
            "cooccurrence",
            "time_decay",
            "tag_graph",
            "author_graph",
            "sequential",
            "completion_weight",
            "curator_prior",
            "bandit",
        ] {
            let reg = build_registry(&test_config(vec![name.to_string()]));
            assert!(
                reg.contains(name),
                "spec §16.1a strategy {name} is implemented but unreachable"
            );
        }
    }

    #[test]
    fn available_strategies_is_sorted_and_stable() {
        // RRF tie-breaking is only testable if registry order is deterministic,
        // and it comes from a HashMap.
        let first = available_strategies();
        let second = available_strategies();
        assert_eq!(first, second);
        let mut sorted = first.clone();
        sorted.sort();
        assert_eq!(first, sorted);
    }

    #[test]
    fn build_registry_selects_subset() {
        let config = test_config(vec!["cooccurrence".to_string()]);
        let reg = build_registry(&config);
        assert_eq!(reg.strategy_count(), 1);
        assert_eq!(reg.names(), vec!["cooccurrence"]);
    }

    #[test]
    fn build_registry_unknown_strategy_ignored() {
        let config = test_config(vec!["nonexistent".to_string()]);
        let reg = build_registry(&config);
        assert_eq!(reg.strategy_count(), 0);
    }
}
