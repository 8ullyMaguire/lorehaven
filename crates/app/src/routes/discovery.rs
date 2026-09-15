//! Discovery routes: recommendations, taste profiles.
//!
//! Spec §16.1–16.2.

use crate::auth::{MaybeSession, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, State};
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
        .nest("/recipes", recipe_routes())
        .nest("/dashboard", dashboard_routes())
}

fn recipe_routes() -> Router<AppState> {
    Router::new()
        .route("/", post(create_recipe))
        .route("/{id}", get(get_recipe_route).post(update_recipe_route))
        .route("/{id}/delete", post(delete_recipe_route))
        .route("/list", get(list_recipes_route))
}

fn dashboard_routes() -> Router<AppState> {
    Router::new()
        .route("/", get(get_dashboard))
        .route("/", post(save_dashboard))
}

async fn get_discovery(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
) -> ApiResult<Json<serde_json::Value>> {
    let limit = 20;
    let account_id: Option<String> = session.as_ref().map(|s| s.account_id.to_string());

    // Build candidate lists from each recommendation engine, then blend.
    let mut engines: Vec<Vec<lorehaven_domain::discovery::Candidate>> = Vec::new();

    if let Some(ref account_id) = account_id {
        // Engine 1: tag-based personalization (from taste profile).
        let personalized =
            lorehaven_db::discovery::personalized_recommendations(state.db(), account_id, limit)
                .await
                .map_err(|e| ApiError(AppError::Internal(e)))?;
        engines.push(
            personalized
                .into_iter()
                .enumerate()
                .map(|(idx, id)| lorehaven_domain::discovery::Candidate {
                    work_id: id,
                    score: (limit - idx as i64),
                    reason: "tags".into(),
                })
                .collect(),
        );
    }

    // Engine 2: popularity-based (public engine).
    let popular = lorehaven_db::discovery::public_recommendations(state.db(), limit)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    engines.push(
        popular
            .into_iter()
            .enumerate()
            .map(|(idx, id)| lorehaven_domain::discovery::Candidate {
                work_id: id,
                score: (limit - idx as i64),
                reason: "popular".into(),
            })
            .collect(),
    );

    // Merge candidates from all engines deterministically.
    let blended = lorehaven_domain::discovery::blend(&engines);
    let blended = blended.into_iter().take(limit as usize).collect::<Vec<_>>();

    // Apply operator affinity ranking (silent reordering, no field changes).
    let affinities = lorehaven_db::discovery::list_operator_affinities(state.db())
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    let affinity_map: std::collections::HashMap<String, i64> = affinities
        .into_iter()
        .map(|a| (a.work_id, a.affinity_bp))
        .collect();

    let ranked = if !affinity_map.is_empty() {
        lorehaven_domain::discovery::apply_affinity_ranking(blended, &affinity_map)
    } else {
        // Sort by descending score when no affinities are set.
        let mut r: Vec<lorehaven_domain::discovery::Candidate> = blended.clone();
        r.sort_by_key(|c| -c.score);
        r
    };

    let mut items: Vec<serde_json::Value> = ranked
        .into_iter()
        .map(|c| serde_json::json!({ "work_id": c.work_id.to_string() }))
        .collect();

    // Apply diversity: per-fandom caps and exploration slots for signed-in
    // readers (configuration-driven). Anonymous readers pass through as-is.
    if let Some(ref account_id) = account_id {
        if let Some(profile) = lorehaven_db::discovery::taste_profile_for(state.db(), account_id)
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
                .filter_map(|item| item["work_id"].as_str().and_then(|s| s.parse().ok()))
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
    Err(ApiError(AppError::NotFound { resource: "page" }))
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

// ---------------------------------------------------------------------------
// M11-05: Recipes
// ---------------------------------------------------------------------------

async fn create_recipe(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    let recipe_id = body
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError(AppError::field("id", "id is required")))?
        .to_string();
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError(AppError::field("name", "name is required")))?
        .to_string();
    let document = body
        .get("document")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    let is_public = body
        .get("is_public")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let owner = user.account_id.to_string();

    lorehaven_db::discovery::save_recipe(
        state.db(),
        &recipe_id,
        &owner,
        &name,
        &document,
        is_public,
        &lorehaven_db::identity::now_rfc3339(),
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e)))?;

    Ok(Json(serde_json::json!({ "id": recipe_id })))
}

async fn get_recipe_route(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let viewer = user.account_id.to_string();
    let recipe = lorehaven_db::discovery::get_recipe(state.db(), &id, &viewer)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    match recipe {
        Some(r) => Ok(Json(serde_json::json!({
            "id": r.id,
            "owner": r.owner,
            "name": r.name,
            "document": r.document,
            "is_public": r.is_public,
            "created_at": r.created_at,
        }))),
        None => Err(ApiError(AppError::NotFound { resource: "recipe" })),
    }
}

async fn list_recipes_route(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let viewer = user.account_id.to_string();
    let recipes = lorehaven_db::discovery::list_recipes(state.db(), &viewer)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "recipes": recipes })))
}

async fn update_recipe_route(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    let owner = user.account_id.to_string();
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError(AppError::field("name", "name is required")))?
        .to_string();
    let document = body
        .get("document")
        .cloned()
        .unwrap_or(serde_json::json!({}));

    let updated = lorehaven_db::discovery::update_recipe(state.db(), &id, &owner, &name, &document)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;

    if !updated {
        return Err(ApiError(AppError::NotFound { resource: "recipe" }));
    }
    Ok(Json(serde_json::json!({ "status": "updated" })))
}

async fn delete_recipe_route(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let owner = user.account_id.to_string();
    let deleted = lorehaven_db::discovery::delete_recipe(state.db(), &id, &owner)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    if !deleted {
        return Err(ApiError(AppError::NotFound { resource: "recipe" }));
    }
    Ok(Json(serde_json::json!({ "status": "deleted" })))
}

// ---------------------------------------------------------------------------
// M11-06: Dashboards
// ---------------------------------------------------------------------------

async fn get_dashboard(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let account = user.account_id.to_string();
    let layout = lorehaven_db::discovery::get_dashboard_layout(state.db(), &account)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    match layout {
        Some(l) => Ok(Json(serde_json::json!({ "slots": l.slots }))),
        None => Ok(Json(serde_json::json!({ "slots": [] }))),
    }
}

async fn save_dashboard(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    let account = user.account_id.to_string();
    let slots = body.get("slots").cloned().unwrap_or(serde_json::json!([]));
    lorehaven_db::discovery::save_dashboard_layout(state.db(), &account, &slots)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "status": "saved" })))
}
