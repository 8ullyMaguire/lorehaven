
// ---------------------------------------------------------------------------
// CTA marks (spec §42.2): curators record whether a work carries its own CTA
// ---------------------------------------------------------------------------

use crate::auth::RequireSession;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use lorehaven_db::exports::{cta_marks_for, mark_cta, retract_cta_mark};
use lorehaven_db::governance::trust_for;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct CtaMarkRequest {
    // `true` = the work carries its own CTA.
    pub has_own_cta: bool,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/works/:id/cta_marks",
            axum::routing::post(mark).get(list),
        )
        .route("/works/:id/cta_marks/me", axum::routing::delete(retract))
}

fn bad_request(detail: impl Into<String>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": "bad_request", "detail": detail.into() })),
    )
        .into_response()
}

/// POST /works/:id/cta_marks — record or replace this curator's mark.
async fn mark(
    State(state): State<AppState>,
    Path(work_id): Path<String>,
    RequireSession(session_user): RequireSession,
    Json(body): Json<CtaMarkRequest>,
) -> Response {
    let account_id = session_user.account_id.to_string();
    // Trust-level check: TL>2 required.
    let level = match trust_for(state.db(), &account_id).await {
        Ok(l) => l,
        Err(e) => return bad_request(format!("trust lookup failed: {e}")),
    };
    if level <= 2 {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "trust level too low",
                "detail": "CTA marking requires trust level > 2",
                "trust_level": level,
            })),
        )
            .into_response();
    }

    match mark_cta(state.db(), &work_id, &account_id, body.has_own_cta).await {
        Ok(()) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "status": "ok",
                "has_own_cta": body.has_own_cta,
                "work_id": work_id,
                "curator": account_id,
            })),
        )
            .into_response(),
        Err(e) => bad_request(format!("failed to record mark: {e}")),
    }
}

/// GET /works/:id/cta_marks — list all marks on a work.
async fn list(State(state): State<AppState>, Path(work_id): Path<String>) -> Response {
    match cta_marks_for(state.db(), &work_id).await {
        Ok(marks) => {
            let body: Vec<serde_json::Value> = marks
                .into_iter()
                .map(|m| {
                    serde_json::json!({
                        "work_id": m.work_id,
                        "curator": m.curator,
                        "has_own_cta": m.has_own_cta,
                        "marked_at": m.marked_at,
                    })
                })
                .collect();
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(e) => bad_request(format!("failed to list marks: {e}")),
    }
}

/// DELETE /works/:id/cta_marks/me — retract this curator's mark.
async fn retract(
    State(state): State<AppState>,
    Path(work_id): Path<String>,
    RequireSession(session_user): RequireSession,
) -> Response {
    let account_id = session_user.account_id.to_string();
    match retract_cta_mark(state.db(), &work_id, &account_id).await {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "action": "retracted"})),
        )
            .into_response(),
        Ok(false) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "action": "no mark to retract"})),
        )
            .into_response(),
        Err(e) => bad_request(format!("failed to retract mark: {e}")),
    }
}
