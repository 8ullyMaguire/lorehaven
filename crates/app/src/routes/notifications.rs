//! §5.5/§12 — the notifications inbox API.
//!
//! Read-only plus mark-read; rows are written by the surfaces that produce
//! them (forum replies, sales, gifts). Everything here is session-scoped:
//! an account sees only its own inbox.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde_json::json;

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

/// GET /notifications — the caller's inbox, newest first, with an unread count.
async fn list_notifications(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let account_id = user.account_id.to_string();
    let (rows, unread) = tokio::try_join!(
        lorehaven_db::notifications::list(state.db(), &account_id, 200),
        lorehaven_db::notifications::unread_count(state.db(), &account_id),
    )
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    let items: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|n| {
            json!({
                "id": n.id,
                "kind": n.kind,
                "title": n.title,
                "body": n.body,
                "read": !n.unread,
                "created_at": n.created_at,
                "work_id": n.work_id,
            })
        })
        .collect();
    Ok(Json(json!({ "items": items, "unread_count": unread })))
}

/// POST /notifications/read-all — idempotent.
async fn read_all(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<StatusCode> {
    let account_id = user.account_id.to_string();
    lorehaven_db::notifications::mark_all_read(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /notifications/{id}/read — idempotent: an unknown id or an
/// already-read entry both settle to 204 rather than advertising existence.
async fn mark_read(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let account_id = user.account_id.to_string();
    lorehaven_db::notifications::mark_read(state.db(), &account_id, &id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/notifications", get(list_notifications))
        .route("/notifications/read-all", post(read_all))
        .route("/notifications/{id}/read", post(mark_read))
}
