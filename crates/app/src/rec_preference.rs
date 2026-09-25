//! The reader's recommendation-engine preference (spec §16.1b, M52-09).
//!
//! One resolver, used by every surface that produces a recommendation. A
//! surface that builds its own list without going through [`resolve`] is a
//! defect rather than a variation, because the whole point of the preference
//! is that it is honored *everywhere*, not on the surface that happened to
//! remember.
//!
//! The precedence rules, in one place:
//!
//! 1. **The operator's enabled set decides what exists.** A reader cannot
//!    choose a strategy the operator has disabled, and a preference recorded
//!    before the operator disabled it is remembered but not honored.
//! 2. **An unset preference means the instance default**, never a null engine
//!    and never a silent choice made on the reader's behalf.
//! 3. **The reader is told when their choice is unavailable.** A reader who
//!    picked `tag_graph` and finds it gone should learn the instance changed,
//!    rather than believe their choice is being applied. The stored value is
//!    not cleared — re-enabling restores it, which is what "remembered" means.
//!
//! This module is pure: it takes the registry and the stored string and
//! returns a decision. The database read lives in the route, so the decision
//! logic is testable without a database.

use lorehaven_db::rec_strategy::RecRegistry;
use serde::Serialize;

/// The settings key the preference is stored under.
pub const SETTING_KEY: &str = "discovery.rec_engine";

/// What a reader's preference resolves to, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum EngineChoice {
    /// No preference recorded; the instance's own blend produces the results.
    InstanceDefault {
        /// The strategies actually in play, so the surface can name them.
        using: Vec<String>,
    },
    /// The reader's recorded strategy is enabled and will be used.
    Honored { engine: String },
    /// The reader's recorded strategy is no longer enabled by the operator.
    ///
    /// The instance default is used, and `engine` is reported so the surface
    /// can say *which* choice stopped being honored. Without that, the reader
    /// sees results from an engine they did not choose and has no way to know.
    Unavailable { engine: String, using: Vec<String> },
}

impl EngineChoice {
    /// The strategies the caller should blend.
    ///
    /// This is the point of the type: an unavailable choice and a honored one
    /// both still have to produce recommendations, and the caller cannot tell
    /// the difference unless it asks. The reporting is a separate concern from
    /// the blending, and conflating them is how a silent substitution happens.
    pub fn effective_registry(&self, registry: &RecRegistry) -> Option<RecRegistry> {
        match self {
            Self::InstanceDefault { .. } | Self::Unavailable { .. } => Some(registry.clone()),
            Self::Honored { engine } => registry.only(engine),
        }
    }

    /// The engine name to show as in effect, if any.
    pub fn effective_engine(&self) -> Option<&str> {
        match self {
            Self::InstanceDefault { .. } | Self::Unavailable { .. } => None,
            Self::Honored { engine } => Some(engine),
        }
    }

    /// Whether the reader's recorded choice is being applied.
    pub fn is_honored(&self) -> bool {
        matches!(self, Self::Honored { .. })
    }
}

/// Resolve a reader's stored preference against the operator's registry.
///
/// `stored` is the raw value from `search_settings`, or `None` when the reader
/// has never chosen. A `stored` value that is not a string is treated as unset
/// rather than an error: a corrupt row should degrade to the instance default
/// and be visible in the settings surface, not take down every recommendation
/// surface on the instance.
pub fn resolve(registry: &RecRegistry, stored: Option<&str>) -> EngineChoice {
    let using: Vec<String> = registry.names().into_iter().map(str::to_owned).collect();

    let Some(engine) = stored.map(str::trim).filter(|s| !s.is_empty()) else {
        return EngineChoice::InstanceDefault { using };
    };

    if registry.contains(engine) {
        EngineChoice::Honored {
            engine: engine.to_owned(),
        }
    } else {
        EngineChoice::Unavailable {
            engine: engine.to_owned(),
            using,
        }
    }
}

/// Validate a reader's choice before storing it.
///
/// Returns the normalized engine name. The empty string is valid and means
/// "instance default" — a reader may always clear their choice.
///
/// An engine the operator has not enabled is an error naming the accepted
/// values, on the same reasoning as the instance mode of §0.4.7: a setting
/// that silently falls back is worse than a setting that refuses. The route
/// turns this into a 422, which is what `AppError::Validation` maps to.
pub fn validate(registry: &RecRegistry, requested: &str) -> Result<String, Vec<String>> {
    let engine = requested.trim();
    if engine.is_empty() {
        return Ok(String::new());
    }
    if registry.contains(engine) {
        Ok(engine.to_owned())
    } else {
        Err(registry.names().into_iter().map(str::to_owned).collect())
    }
}

/// Read a reader's stored preference and resolve it against the operator's
/// registry.
///
/// This is the one function a recommendation surface calls. It exists so that
/// "honored everywhere" is a property of the wiring rather than a promise in a
/// document: a surface that wants recommendations calls this, and gets both
/// the registry to blend and the reporting state to show.
///
/// `pseud_id` is `None` for a signed-out reader, which is the same as having
/// chosen nothing — an anonymous visitor gets the instance blend and makes no
/// claim about a choice of their own.
pub async fn load_for_pseud(
    db: &lorehaven_db::Database,
    config: &crate::config::DiscoveryConfig,
    pseud_id: Option<uuid::Uuid>,
) -> EngineChoice {
    let registry = crate::rec_engine::build_registry(config);
    let Some(pseud_id) = pseud_id else {
        return resolve(&registry, None);
    };

    let stored = lorehaven_db::settings::read_search_settings(db, pseud_id)
        .await
        .ok()
        .and_then(|rows| {
            rows.into_iter()
                .find(|(key, _)| key == SETTING_KEY)
                .and_then(|(_, value)| value.as_str().map(str::to_owned))
        });

    resolve(&registry, stored.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DiscoveryConfig;
    use crate::rec_engine::{available_strategies, build_registry};

    fn registry() -> RecRegistry {
        build_registry(&DiscoveryConfig::default())
    }

    fn registry_with(names: &[&str]) -> RecRegistry {
        let config = DiscoveryConfig {
            rec_enabled_strategies: names.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        build_registry(&config)
    }

    #[test]
    fn unset_preference_is_the_instance_default() {
        let choice = resolve(&registry(), None);
        assert!(matches!(choice, EngineChoice::InstanceDefault { .. }));
        assert!(!choice.is_honored());
        assert_eq!(choice.effective_engine(), None);
    }

    #[test]
    fn empty_string_is_unset_not_a_named_engine() {
        // The setting's default is `""`. If that were treated as an engine
        // name, every reader who never chose anything would render as having
        // chosen an engine called "".
        for stored in ["", "   "] {
            let choice = resolve(&registry(), Some(stored));
            assert!(
                matches!(choice, EngineChoice::InstanceDefault { .. }),
                "{stored:?} should be unset"
            );
        }
    }

    #[test]
    fn a_chosen_enabled_engine_is_honored() {
        let choice = resolve(&registry(), Some("tag_graph"));
        assert!(choice.is_honored());
        assert_eq!(choice.effective_engine(), Some("tag_graph"));
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        let choice = resolve(&registry(), Some("  bandit  "));
        assert!(choice.is_honored());
        assert_eq!(choice.effective_engine(), Some("bandit"));
    }

    #[test]
    fn a_chosen_disabled_engine_is_reported_not_silently_substituted() {
        let operator_disabled_it = registry_with(&["cooccurrence"]);
        let choice = resolve(&operator_disabled_it, Some("tag_graph"));

        assert!(!choice.is_honored());
        match choice {
            EngineChoice::Unavailable { engine, using } => {
                assert_eq!(engine, "tag_graph", "the reader's choice is named back");
                assert_eq!(using, vec!["cooccurrence"], "the instance's set is named");
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    #[test]
    fn an_unavailable_choice_still_produces_recommendations() {
        // The reader gets the instance blend rather than an error page, and
        // the caller can report the substitution because the type says so.
        let operator_disabled_it = registry_with(&["cooccurrence"]);
        let choice = resolve(&operator_disabled_it, Some("tag_graph"));
        let effective = choice
            .effective_registry(&operator_disabled_it)
            .expect("still blends");
        assert_eq!(effective.strategy_count(), 1);
        assert_eq!(effective.names(), vec!["cooccurrence"]);
    }

    #[test]
    fn a_honored_choice_narrows_the_blend_to_one_strategy() {
        let all = registry();
        let choice = resolve(&all, Some("bandit"));
        let effective = choice.effective_registry(&all).expect("honored");
        assert_eq!(effective.strategy_count(), 1);
        assert_eq!(effective.names(), vec!["bandit"]);
    }

    #[test]
    fn the_instance_default_leaves_the_blend_untouched() {
        let all = registry();
        let choice = resolve(&all, None);
        let effective = choice.effective_registry(&all).expect("default blends");
        assert_eq!(effective.strategy_count(), all.strategy_count());
    }

    #[test]
    fn validate_accepts_an_enabled_engine() {
        let reg = registry();
        assert_eq!(validate(&reg, "bandit").unwrap(), "bandit");
    }

    #[test]
    fn validate_accepts_clearing_the_choice() {
        assert_eq!(validate(&registry(), "").unwrap(), "");
        assert_eq!(validate(&registry(), "  ").unwrap(), "");
    }

    #[test]
    fn validate_rejects_a_disabled_engine_and_names_what_is_accepted() {
        let reg = registry_with(&["cooccurrence"]);
        let accepted = validate(&reg, "bandit").expect_err("bandit is not enabled here");
        assert_eq!(accepted, vec!["cooccurrence"]);
    }

    #[test]
    fn validate_rejects_an_invented_engine() {
        let accepted = validate(&registry(), "vibes").expect_err("no such strategy");
        assert_eq!(accepted, available_strategies());
    }
}
