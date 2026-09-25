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
    version: &'static str,
    build: String,
    api_version: &'static str,
    environment: &'static str,
    base_url: String,
    policy: PolicySummary,
    topics: Vec<TopicSummary>,
    theme: ThemeSummary,
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

fn rating_name(rating: lorehaven_domain::policy::ContentRating) -> &'static str {
    use lorehaven_domain::policy::ContentRating;
    match rating {
        ContentRating::General => "general",
        ContentRating::Teen => "teen",
        ContentRating::Mature => "mature",
        ContentRating::Explicit => "explicit",
    }
}
