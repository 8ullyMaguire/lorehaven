//! Discovery routes: taste profiles, recipes, dashboards.
//!
//! Spec §16.5–16.8.

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use lorehaven_db::discovery::*;
use lorehaven_domain::AppError;
use serde::Deserialize;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/discovery/taste-profile",
            get(get_taste_profile)
                .put(put_taste_profile)
                .delete(delete_taste_profile),
        )
        .route("/discovery/recipes", get(list_recipes).post(post_recipe))
        .route(
            "/discovery/dashboard",
            get(get_dashboard).put(put_dashboard),
        )
}

#[derive(Debug, Deserialize)]
pub struct CursorQuery {
    cursor: Option<String>,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    20
}

async fn get_taste_profile(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let account_id = user.account_id.to_string();
    let profile = taste_profile_for(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    match profile {
        Some(p) => Ok(Json(serde_json::json!({ "signals": p.signals }))),
        None => Ok(Json(serde_json::json!({ "signals": {} }))),
    }
}

async fn put_taste_profile(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(signals): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    use time::OffsetDateTime;
    let now = OffsetDateTime::now_utc().to_string();
    let account_id = user.account_id.to_string();
    save_taste_profile(state.db(), &account_id, &signals, &now)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "status": "saved" })))
}

async fn delete_taste_profile(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let account_id = user.account_id.to_string();
    clear_taste_profile(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "status": "cleared" })))
}

async fn list_recipes(
    State(state): State<AppState>,
    RequireSession(_): RequireSession,
    Query(params): Query<CursorQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let recipes = public_recipes(state.db(), params.cursor.as_deref(), params.limit)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "items": recipes })))
}

async fn post_recipe(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(payload): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    use uuid::Uuid;
    let id = Uuid::new_v4().to_string();
    let name = payload
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Untitled");
    let document = payload.get("document").cloned().unwrap_or_default();
    let is_public = payload
        .get("is_public")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    use time::OffsetDateTime;
    let now = OffsetDateTime::now_utc().to_string();
    let owner = user.account_id.to_string();
    save_recipe(state.db(), &id, &owner, name, &document, is_public, &now)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "id": id })))
}

async fn get_dashboard(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let account_id = user.account_id.to_string();
    let layout = dashboard_layout_for(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    match layout {
        Some(l) => Ok(Json(l.slots)),
        None => Ok(Json(serde_json::json!({}))),
    }
}

async fn put_dashboard(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(slots): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    use time::OffsetDateTime;
    let now = OffsetDateTime::now_utc().to_string();
    let account_id = user.account_id.to_string();
    save_dashboard_layout(state.db(), &account_id, &slots, &now)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "status": "saved" })))
}
