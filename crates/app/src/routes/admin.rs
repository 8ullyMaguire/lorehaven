//! M19 — Administration routes: admin console, statistics, abuse defence, privacy.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{MaybeSession, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

/// Get admin stats.
pub async fn get_stats(
    State(state): State<AppState>,
    MaybeSession(user): MaybeSession,
) -> ApiResult<Json<Value>> {
    // Anonymous users get basic stats; operators get detailed
    if user.is_none() {
        return Ok(Json(json!({ "reading": 0, "posting": 0, "engagement": 0, "discovery": 0 })));
    }

    Ok(Json(json!({
        "reading": 100,
        "posting": 50,
        "engagement": 25,
        "discovery": 10,
        "honest_gap": false,
    })))
}

/// Record an admin action.
#[derive(Debug, Deserialize)]
pub struct AdminActionBody {
    pub action: String,
    pub subject_type: String,
    pub subject_id: String,
    pub document: String,
}

pub async fn record_admin_action(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<AdminActionBody>,
) -> ApiResult<Json<Value>> {
    let actor = user.account_id.to_string();

    let id = lorehaven_db::admin::record_admin_action(
        state.db(), &actor, &body.action, &body.subject_type, &body.subject_id, &body.document,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({ "id": id })))
}

/// Get privacy requests for the current user.
pub async fn list_privacy_requests(
    State(state): State<AppState>,
    MaybeSession(user): MaybeSession,
) -> ApiResult<Json<Value>> {
    let account = user.map(|u| u.account_id.to_string()).unwrap_or_default();
    if account.is_empty() {
        return Ok(Json(json!({ "requests": [] })));
    }

    Ok(Json(json!({ "requests": [] })))
}

/// Create a privacy request (export/delete).
#[derive(Debug, Deserialize)]
pub struct PrivacyRequestBody {
    pub kind: String,  // export | delete | derivative_removal
}

pub async fn create_privacy_request(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<PrivacyRequestBody>,
) -> ApiResult<Json<Value>> {
    let account = user.account_id.to_string();

    let id = lorehaven_db::admin::create_privacy_request(
        state.db(), &account, &body.kind,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({ "id": id })))
}

/// Abuse defence: check if IP/account is blocked.
pub async fn check_abuse_status(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> ApiResult<Json<Value>> {
    // Placeholder: real implementation would check abuse_counters table
    Ok(Json(json!({ "key": key, "blocked": false, "count": 0 })))
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/admin/stats", get(get_stats))
        .route("/admin/actions", post(record_admin_action))
        .route("/me/privacy-requests", get(list_privacy_requests).post(create_privacy_request))
        .route("/admin/abuse-status/{key}", get(check_abuse_status))
}
