//! Application metadata.
//!
//! `/api/v1/meta` exists so the frontend can render real values — instance
//! name, active policy, enabled features — instead of mock data (spec §1.1:
//! "A screen containing mock data is not an implemented feature"). It is also
//! the first endpoint an integration touches when checking compatibility.

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
struct MetaResponse {
    name: String,
    version: &'static str,
    build: String,
    api_version: &'static str,
    environment: &'static str,
    base_url: String,
    policy: PolicySummary,
}

/// The instance's content policy, as the frontend needs to apply it.
///
/// This is reported rather than assumed: spec §7 lets an instance decide
/// whether anonymous reading is available at all, and the UI must not offer
/// what the server will refuse.
#[derive(Debug, Serialize)]
struct PolicySummary {
    anonymous_reading: bool,
    anonymous_max_rating: &'static str,
    unknown_age_max_rating: &'static str,
    minor_max_rating: &'static str,
    adult_max_rating: &'static str,
    registration_open: bool,
    csrf_required: bool,
}

async fn meta(State(state): State<AppState>) -> Json<MetaResponse> {
    let config = state.config();
    let policy = lorehaven_domain::policy::AccessPolicy::default();

    Json(MetaResponse {
        name: config.site.name.clone(),
        version: version::VERSION,
        build: version::build_id(),
        api_version: version::API_VERSION,
        environment: config.environment.as_str(),
        base_url: config.site.base_url.clone(),
        policy: PolicySummary {
            anonymous_reading: policy.anonymous_reading_enabled,
            anonymous_max_rating: rating_name(policy.anonymous_max_rating),
            unknown_age_max_rating: rating_name(policy.unknown_age_max_rating),
            minor_max_rating: rating_name(policy.minor_max_rating),
            adult_max_rating: rating_name(policy.adult_max_rating),
            registration_open: true,
            csrf_required: config.security.csrf_required,
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
