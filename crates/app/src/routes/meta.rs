//! Application metadata.
//!
//! `/api/v1/meta` exists so the frontend can render real values — instance
//! name, active policy, enabled features — instead of mock data (spec §1.1:
//! "A screen containing mock data is not an implemented feature"). It is also
//! the first endpoint an integration touches when checking compatibility.

use crate::auth::MaybeSession;
use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::state::AppState;
use crate::version;

/// Metadata routes.
pub fn router() -> Router<AppState> {
    Router::new().route("/meta", get(meta))
}

#[derive(Debug, Serialize)]
struct ThemeSummary {
    mode: String,
    allow_user_opt_out: bool,
    influence_sources: Vec<String>,
}

#[derive(Debug, Serialize)]
struct MetaResponse {
    name: String,
    /// The instance's one-line identity. It is public, because the
    /// instance's identity is public (spec 12.1, D-8).
    tagline: String,
    version: &'static str,
    build: String,
    api_version: &'static str,
    environment: &'static str,
    base_url: String,
    policy: PolicySummary,
    /// What this instance classifies with, disclosed to every reader (amendment
    /// §3.4). See [`DecisionsSummary`].
    decisions: DecisionsSummary,
    topics: Vec<TopicSummary>,
    theme: ThemeSummary,
    /// The instance theme's publication and its signal weights (spec §12.1, D-11).
    instance_theme: InstanceThemeSummary,
    /// The compute tiers — who gets which algorithm (spec §12.6).
    ///
    /// **Published, because the tier a reader is on has to be visible to them**
    /// and because the boundaries are configuration a reader can see.
    tiers: TiersSummary,
    /// The mood gate (spec §12.3).
    moods: MoodsSummary,
    /// Channel voting (spec §12.10, §12.12).
    channels: ChannelsSummary,
    /// The content policy and its version (spec §15.7, D-9).
    content_policy: ContentPolicySummary,
}

/// The instance theme's publication, its weights and its opposition weight
/// (spec §12.1, D-11).
#[derive(Debug, Serialize)]
struct InstanceThemeSummary {
    /// Whether the instance theme is published. **It is a public, versioned
    /// editorial position**, so a client can say what the instance is for.
    published: bool,
    /// The three signal weights, in the order the specification fixes them:
    /// **works ≫ authors > tags**. The ordering is the design (D-11) and the
    /// numbers are configuration.
    signal_weights: SignalWeightsSummary,
    /// How strongly a work carrying a tag the theme is *against* is demoted.
    opposition_weight: f64,
}

/// The three signal weights (spec §12.1, D-11).
#[derive(Debug, Serialize)]
struct SignalWeightsSummary {
    works: f64,
    authors: f64,
    tags: f64,
}

/// The compute tiers (spec §12.6).
#[derive(Debug, Serialize)]
struct TiersSummary {
    /// T0 is always available: the house theme is a real recommendation.
    house: String,
    /// The trust level T1 needs.
    personal_min_trust: i64,
    /// Whether the Supporter tier reaches T2.
    full_supporter: bool,
    /// The trust level that reaches T2.
    full_min_trust: i64,
    /// The resonance that reaches T2.
    full_min_resonance: f64,
}

/// The mood gate (spec §12.3).
#[derive(Debug, Serialize)]
struct MoodsSummary {
    enabled: bool,
    min_trust_level: i64,
    supporter_only: bool,
    max_moods_per_account: i64,
}

/// Channel voting (spec §12.10, §12.12).
#[derive(Debug, Serialize)]
struct ChannelsSummary {
    voting_enabled: bool,
    min_trust_level: i64,
    min_resonance: f64,
    supporter_only: bool,
    period_days: i64,
    max_shift_per_period: f64,
    seed_floor: f64,
    tau: f64,
}

/// The instance's content policy and the version of it (spec §15.7, D-9).
///
/// **The version travels with the policy**, because a policy that changes
/// without a version change cannot be reasoned about: a reader who agreed to
/// version 2 has not agreed to version 3.
#[derive(Debug, Serialize)]
struct ContentPolicySummary {
    /// The policy's version. Bumping it re-asks (spec §14.3).
    version: i64,
    /// The age gate's mode: `required` on an adult-content instance (D-9).
    age_gate: String,
    /// Whether a guest must accept consent before the instance creates one.
    consent_required_for_guest: bool,
}

/// How this instance classifies, and with what (amendment §3.4).
///
/// Reported to EVERY reader, signed in or not, and that is the point. Spec
/// §0.4.3 asks an instance to disclose what it is that decides; §11.15's
/// retention setting gets the same treatment. A reader whose comments are
/// filtered by a 678 MB local model is owed the knowledge that a model did it,
/// and `/meta` is the one endpoint an integration already reads before it
/// renders anything.
///
/// **A deterministic instance discloses `"deterministic"` rather than omitting
/// the field.** Omission would be the same as absence, and a client that reads
/// a missing field as "unknown, assume the worst" has learned nothing; a client
/// that reads it as "not present" is entitled to that reading, and "present, and
/// it is the instance's own classifiers" is the truth.
///
/// The thresholds travel with it. An operator who has moved `accept_threshold`
/// has changed what this instance holds to be a work, and that is a setting a
/// reader is being graded against.
#[derive(Debug, Serialize)]
struct DecisionsSummary {
    /// `"deterministic"` or `"calibrated"`.
    provider: &'static str,
    /// The posterior at or above which an acceptance is kept. Meaningless under
    /// `deterministic`, and still reported: a client rendering "your instance
    /// grades with its own rules" should not have to special-case the field's
    /// absence to avoid implying a number that is not in force.
    accept_threshold: f64,
    /// Below this the model is not consulted at all.
    consult_floor: f64,
}

#[derive(Debug, Serialize)]
struct TopicSummary {
    name: String,
    public: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    bonus_credits: Option<u32>,
}

/// The instance's content policy, as the frontend needs to apply it.
///
/// This is reported rather than assumed: spec §7 lets an instance decide
/// whether anonymous reading is available at all, and the UI must not offer
/// what the server will refuse.
#[derive(Debug, Serialize)]
struct PolicySummary {
    /// The instance's accessibility posture, spelled out (spec §0.4.7). The
    /// boolean below is its effect on anonymous reading; the two travel
    /// together so a client never has to infer one from the other.
    instance_mode: &'static str,
    anonymous_reading: bool,
    anonymous_max_rating: &'static str,
    unknown_age_max_rating: &'static str,
    minor_max_rating: &'static str,
    adult_max_rating: &'static str,
    registration_open: bool,
    csrf_required: bool,
}

async fn meta(
    State(state): State<AppState>,
    MaybeSession(_user): MaybeSession,
) -> Json<MetaResponse> {
    let config = state.config();
    // The summary is how a client (and a person reading the raw JSON) learns
    // that the instance is closed to anonymous readers, so it must reflect the
    // configured posture rather than the library default (spec §0.4.7).
    let policy = config.instance.mode.access_policy();

    Json(MetaResponse {
        name: config.site.name.clone(),
        tagline: config.site.tagline.clone(),
        version: version::VERSION,
        build: version::build_id(),
        api_version: version::API_VERSION,
        environment: config.environment.as_str(),
        base_url: config.site.base_url.clone(),
        policy: PolicySummary {
            instance_mode: config.instance.mode.as_str(),
            anonymous_reading: policy.anonymous_reading_enabled,
            anonymous_max_rating: rating_name(policy.anonymous_max_rating),
            unknown_age_max_rating: rating_name(policy.unknown_age_max_rating),
            minor_max_rating: rating_name(policy.minor_max_rating),
            adult_max_rating: rating_name(policy.adult_max_rating),
            registration_open: true,
            csrf_required: config.security.csrf_required,
        },
        decisions: DecisionsSummary {
            provider: config.decisions.provider.as_str(),
            accept_threshold: config.decisions.accept_threshold,
            consult_floor: config.decisions.consult_floor,
        },
        topics: config
            .site
            .topics
            .iter()
            .map(|t| TopicSummary {
                name: t.name.clone(),
                public: t.public,
                bonus_credits: if t.bonus_credits > 0 {
                    Some(t.bonus_credits)
                } else {
                    None
                },
            })
            .collect(),
        instance_theme: InstanceThemeSummary {
            published: config.theme.published,
            signal_weights: SignalWeightsSummary {
                works: config.theme.signal_weights.works,
                authors: config.theme.signal_weights.authors,
                tags: config.theme.signal_weights.tags,
            },
            opposition_weight: config.theme.opposition_weight,
        },
        tiers: TiersSummary {
            house: config.tiers.house.clone(),
            personal_min_trust: config.tiers.personal_min_trust,
            full_supporter: config.tiers.full_supporter,
            full_min_trust: config.tiers.full_min_trust,
            full_min_resonance: config.tiers.full_min_resonance,
        },
        moods: MoodsSummary {
            enabled: config.discovery.moods.enabled,
            min_trust_level: config.discovery.moods.min_trust_level,
            supporter_only: config.discovery.moods.supporter_only,
            max_moods_per_account: config.discovery.moods.max_moods_per_account,
        },
        channels: ChannelsSummary {
            voting_enabled: config.channels.voting_enabled,
            min_trust_level: config.channels.min_trust_level,
            min_resonance: config.channels.min_resonance,
            supporter_only: config.channels.supporter_only,
            period_days: config.channels.period_days,
            max_shift_per_period: config.channels.max_shift_per_period,
            seed_floor: config.channels.seed_floor,
            tau: config.channels.tau,
        },
        content_policy: ContentPolicySummary {
            version: config.consent.version,
            age_gate: config.consent.age_gate.clone(),
            consent_required_for_guest: config.consent.required_for_guest,
        },
        theme: ThemeSummary {
            mode: config.theme.mode.clone(),
            allow_user_opt_out: config.theme.allow_user_opt_out,
            influence_sources: config
                .theme
                .influence_sources
                .iter()
                .map(|s| {
                    match s.kind {
                        crate::config::InfluenceSourceKind::OperatorTopics => "operator_topics",
                        crate::config::InfluenceSourceKind::AdminTaste => "admin_taste",
                        crate::config::InfluenceSourceKind::LongTermUsers => "long_term_users",
                    }
                    .to_string()
                })
                .collect(),
        },
    })
}

fn rating_name(rating: darkhold_domain::policy::ContentRating) -> &'static str {
    use darkhold_domain::policy::ContentRating;
    match rating {
        ContentRating::General => "general",
        ContentRating::Teen => "teen",
        ContentRating::Mature => "mature",
        ContentRating::Explicit => "explicit",
    }
}
