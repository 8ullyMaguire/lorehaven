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
    // Content filters belong to the pseud the session acts as, and which pseud
    // that is comes from `sessions.active_pseud_id` -- the same rule, and the
    // same account-id fallback, as `routes/discovery.rs`. A notification about a
    // work carries that work's title, so an inbox that ignored the reader's
    // filters would hand back exactly what the filter withholds everywhere
    // else. §46.4 lists notifications as one of the surfaces.
    let viewer_pseud: Option<uuid::Uuid> = Some(
        user.pseud_id
            .as_ref()
            .map(|p| p.as_uuid())
            .unwrap_or_else(|| user.account_id.as_uuid()),
    );
    let rules = lorehaven_db::search::content_filter_sql::for_pseud(state.db(), viewer_pseud)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    let (rows, unread) = tokio::try_join!(
        lorehaven_db::notifications::list_filtered(state.db(), &account_id, 200, &rules),
        lorehaven_db::notifications::unread_count_filtered(state.db(), &account_id, &rules),
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
    // The id is a uuid column on both dialects; refusing a malformed id here
    // keeps the answer identical (404) instead of SQLite 204 / PostgreSQL 500.
    let id = id.parse::<uuid::Uuid>().map_err(|_| {
        ApiError(lorehaven_domain::AppError::NotFound {
            resource: "notification",
        })
    })?;
    let account_id = user.account_id.to_string();
    lorehaven_db::notifications::mark_read(state.db(), &account_id, &id.to_string())
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
