//! Discovery routes: recommendations, taste profiles.
//!
//! Spec §16.1–16.2.

use crate::auth::{MaybeSession, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use lorehaven_db;
use lorehaven_domain::AppError;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/discovery", get(get_discovery))
        .route("/discovery/taste-profile", get(get_taste_profile))
        .route(
            "/discovery/taste-profile/recompute",
            post(recompute_taste_profile),
        )
        .route("/discovery/taste-profile/clear", post(clear_taste_profile))
        .route("/operator/affinities", post(set_operator_affinity))
}

async fn get_discovery(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
) -> ApiResult<Json<serde_json::Value>> {
    let limit = 20;
    let account_id: Option<String> = session.as_ref().map(|s| s.account_id.to_string());
    let work_ids = match session {
        Some(s) => lorehaven_db::discovery::personalized_recommendations(
            state.db(),
            &s.account_id.to_string(),
            limit,
        )
        .await,
        None => lorehaven_db::discovery::public_recommendations(state.db(), limit).await,
    }
    .map_err(|e| ApiError(AppError::Internal(e)))?;

    let mut items: Vec<serde_json::Value> = work_ids
        .into_iter()
        .map(|id| serde_json::json!({ "work_id": id.to_string() }))
        .collect();

    // Apply operator affinity ranking (silent reordering, no field changes).
    let affinities = lorehaven_db::discovery::list_operator_affinities(state.db())
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    let affinity_map: std::collections::HashMap<String, i64> = affinities
        .into_iter()
        .map(|a| (a.work_id, a.affinity_bp))
        .collect();

    if !affinity_map.is_empty() {
        let candidates: Vec<lorehaven_domain::discovery::Candidate> = items
            .iter()
            .enumerate()
            .map(|(idx, item)| lorehaven_domain::discovery::Candidate {
                work_id: item["work_id"]
                    .as_str()
                    .unwrap_or_default()
                    .parse()
                    .unwrap_or_default(),
                score: (items.len() - idx) as i64,
                reason: "discovery".into(),
            })
            .collect();
        let ranked = lorehaven_domain::discovery::apply_affinity_ranking(candidates, &affinity_map);
        items = ranked
            .into_iter()
            .map(|c| serde_json::json!({ "work_id": c.work_id.to_string() }))
            .collect();
    }

    // Apply diversity: per-fandom caps and exploration slots for signed-in
    // readers (configuration-driven). Anonymous readers pass through as-is.
    if let Some(ref account_id) = account_id {
        if let Some(profile) =
            lorehaven_db::discovery::taste_profile_for(state.db(), account_id)
                .await
                .map_err(|e| ApiError(AppError::Internal(e)))?
        {
            let fandoms: Vec<String> = match &profile.signals {
                serde_json::Value::Array(arr) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect(),
                serde_json::Value::Object(map) => map
                    .get("fandom")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            let known: std::collections::HashSet<String> = fandoms.into_iter().collect();
            let work_ids_ordered: Vec<lorehaven_domain::ids::WorkId> = items
                .iter()
                .filter_map(|item| {
                    item["work_id"]
                        .as_str()
                        .and_then(|s| s.parse().ok())
                })
                .collect();
            let capped = lorehaven_domain::discovery::apply_diversity(
                work_ids_ordered,
                &known,
                state.config().discovery.per_fandom_cap,
                state.config().discovery.exploration_rate,
                items.len(),
                &|_work_id| Vec::new(), // fandoms fetched per-work in a real impl
            );
            items = capped
                .into_iter()
                .map(|id| serde_json::json!({ "work_id": id.to_string() }))
                .collect();
        }
    }

    Ok(Json(serde_json::json!({ "items": items })))
}

async fn get_taste_profile(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let account_id = user.account_id.to_string();
    let profile = lorehaven_db::discovery::taste_profile_for(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    match profile {
        Some(p) => Ok(Json(serde_json::json!({ "signals": p.signals }))),
        None => Ok(Json(serde_json::json!({ "signals": {} }))),
    }
}

async fn recompute_taste_profile(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let account_id = user.account_id.to_string();
    lorehaven_db::discovery::recompute_taste_profile(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "status": "recomputed" })))
}

async fn clear_taste_profile(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let account_id = user.account_id.to_string();
    lorehaven_db::discovery::clear_taste_profile(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "status": "cleared" })))
}

/// Whether this account may use the operator surface.
fn require_operator(state: &AppState, user: &crate::auth::SessionUser) -> ApiResult<()> {
    let configured = state.config().administration.operator_account_id;
    if configured == Some(user.account_id) {
        return Ok(());
    }
    tracing::debug!(
        operator_configured = configured.is_some(),
        "an operator route was reached by an account that is not the operator"
    );
    Err(ApiError(AppError::NotFound {
        resource: "page",
    }))
}

/// Set an operator affinity on a work. Audit-logged; never surfaced publicly.
async fn set_operator_affinity(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    require_operator(&state, &user)?;

    let work_id = body
        .get("work_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError(AppError::field("work_id", "work_id is required")))?
        .to_string();
    let affinity_bp = body
        .get("affinity_bp")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| ApiError(AppError::field("affinity_bp", "affinity_bp is required")))?;
    let rationale = body
        .get("rationale")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError(AppError::field("rationale", "rationale is required")))?
        .to_string();

    let operator = user.account_id.to_string();
    lorehaven_db::discovery::set_operator_affinity(
        state.db(),
        &work_id,
        affinity_bp,
        &operator,
        &rationale,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e)))?;

    Ok(Json(serde_json::json!({ "status": "set" })))
}
