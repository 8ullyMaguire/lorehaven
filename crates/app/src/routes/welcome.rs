//! §58.6 — the welcome rota (M45-26).
//!
//! ```text
//! GET  /api/v1/welcome/rota                 reader:   what I owe, and when it is due
//! POST /api/v1/welcome/rota/claim           reader:   hand me one to welcome
//! POST /api/v1/welcome/rota/{id}/complete   reader:   I commented — here is the evidence
//! GET  /api/v1/welcome/report               operator: how much need there is, and how kept
//! ```
//!
//! # Why the reader doors answer 200 when nothing happens
//!
//! Two states here are results rather than errors, and the repository's rule for both
//! already exists (`crates/app/src/routes/supply.rs`): every keystone offered is a
//! success, and an empty result is a result. So `claim` on a catalogue where every
//! work has a comment answers `200 {assignment: null, eligible: 0, reason: …}`, and
//! `complete` without the comment answers `200 {completed: false, reason: …}`. A 404
//! would read as "the feature is broken"; a 409 would invent a status this build has
//! no error variant for.
//!
//! # Why the promise is made to a pseud
//!
//! A comment carries `comments.author_pseud`, so only a pseud can keep a welcome's
//! promise. An account whose session has not chosen one gets `422` on the field
//! `pseud` rather than an assignment nobody can complete.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::routes::discovery::require_operator;
use crate::state::AppState;
use darkhold_db::welcome::{self, Assignment, Claim, Complete};
use darkhold_domain::AppError;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/welcome/rota", get(my_rota))
        .route("/welcome/rota/claim", post(claim_one))
        .route("/welcome/rota/{id}/complete", post(complete_one))
        .route("/welcome/report", get(report))
}

#[derive(Debug, Serialize)]
struct ClaimResponse {
    assignment: Option<Assignment>,
    /// `new`, `already-open`, or `none` — a caller can tell what happened without
    /// comparing ids.
    taken: &'static str,
    /// How many works could take a welcome at all. Reported even when one was taken,
    /// because a number that only appears on failure cannot be watched.
    eligible: i64,
    reason: Option<String>,
}

async fn claim_one(
    RequireSession(user): RequireSession,
    State(state): State<AppState>,
) -> ApiResult<Json<ClaimResponse>> {
    let pseud = user.pseud_id.ok_or_else(|| {
        ApiError(AppError::field(
            "pseud",
            "a welcome is promised to a pseud, because a pseud is what writes the comment: choose one first",
        ))
    })?;
    let pseud = pseud.to_string();
    let claim = welcome::claim(state.db(), &pseud)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    let (assignment, taken, reason) = match claim {
        Claim::Assigned(a) => (Some(a), "new", None),
        Claim::AlreadyOpen(a) => (
            Some(a),
            "already-open",
            Some("you already owe one welcome; finish it before taking another".to_string()),
        ),
        Claim::NothingToWelcome { eligible } => (
            None,
            "none",
            Some(if eligible == 0 {
                "every work on this instance that could take a welcome already has a comment"
                    .to_string()
            } else {
                "every work that needs a welcome has already been offered; nothing left to take"
                    .to_string()
            }),
        ),
    };
    let eligible = match welcome::report(state.db()).await {
        Ok(r) => r.eligible,
        Err(e) => return Err(ApiError(AppError::Internal(e.into()))),
    };
    Ok(Json(ClaimResponse {
        assignment,
        taken,
        eligible,
        reason,
    }))
}

async fn my_rota(
    RequireSession(user): RequireSession,
    State(state): State<AppState>,
) -> ApiResult<Json<serde_json::Value>> {
    let Some(pseud) = user.pseud_id else {
        // No pseud means no promises were ever made to this session: an empty list is
        // the truth, not an error.
        return Ok(Json(serde_json::json!({
            "assignments": [],
            "open": 0,
            "overdue": 0,
        })));
    };
    let rows = welcome::mine(state.db(), &pseud.to_string())
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    let open = rows.iter().filter(|a| a.completed_at.is_none()).count();
    let overdue = rows.iter().filter(|a| a.overdue).count();
    Ok(Json(serde_json::json!({
        "assignments": rows,
        "open": open,
        "overdue": overdue,
    })))
}

#[derive(Debug, Serialize)]
struct CompleteResponse {
    completed: bool,
    assignment: Option<Assignment>,
    reason: Option<String>,
}

async fn complete_one(
    RequireSession(user): RequireSession,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<CompleteResponse>> {
    let Some(pseud) = user.pseud_id else {
        return Err(ApiError(AppError::field(
            "pseud",
            "this session has no active pseud, so it cannot have written the comment",
        )));
    };
    let outcome = welcome::complete(state.db(), &pseud.to_string(), &id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(match outcome {
        Complete::Done(a) => CompleteResponse {
            completed: true,
            assignment: Some(a),
            reason: None,
        },
        Complete::NoCommentYet { work_id } => CompleteResponse {
            completed: false,
            assignment: None,
            reason: Some(format!(
                "no comment by you on work {work_id} since this welcome was assigned: \
                 the rota closes on the comment, not on the button"
            )),
        },
        Complete::NotYours => {
            return Err(ApiError(AppError::NotFound {
                resource: "welcome",
            }))
        }
        Complete::NoSuchAssignment => {
            return Err(ApiError(AppError::NotFound {
                resource: "welcome",
            }))
        }
    }))
}

async fn report(
    RequireSession(user): RequireSession,
    State(state): State<AppState>,
) -> ApiResult<Json<welcome::RotaReport>> {
    require_operator(&state, &user)?;
    let report = welcome::report(state.db())
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(report))
}
