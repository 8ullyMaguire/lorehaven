//! §60 — finisher signals (M45-29).
//!
//! ```text
//! GET /api/v1/authors/{pseud}/finisher-signals   anyone: what this author has finished
//! GET /api/v1/works/{id}/cadence                 anyone: how often this work gained material
//! ```
//!
//! # Two read doors and no write door
//!
//! A finisher signal is *derived* from `works.completion` and `revisions.created_at`. A derived
//! number with a write door is a number somebody eventually corrects by hand, and the correction is
//! wrong the moment the underlying work changes. So there is nothing to POST here.
//!
//! # The framing rule is in the payload, not in a template
//!
//! The review's rule is "positive framing only; never label non-finishers". The response carries
//! `finished` and `concluded` and a sentence built from them; it carries **no** count of abandoned
//! works and no per-work state. The route tests assert the absence of those keys by name, because a
//! field added later is exactly how a rule like this gets lost.
//!
//! # An unmeasurable cadence is a 200
//!
//! On the reference instance every revision landed on one of two days, because an import wrote them.
//! A median gap over that is the import's rhythm, so the door answers
//! `{"measurable": false, "reason": …}` with a 200 — the reader learns why, which a 4xx would not
//! tell them.

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use darkhold_db::finishers;
use darkhold_domain::finishers::{summary, Cadence, History};
use darkhold_domain::AppError;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/authors/{pseud}/finisher-signals", get(author_signals))
        .route("/works/{id}/cadence", get(work_cadence))
}

#[derive(Debug, Serialize)]
struct AuthorSignals {
    /// Works whose `completion` is `complete`.
    finished: i64,
    /// `complete` + `abandoned`: the denominator of "9 of 10".
    concluded: i64,
    /// "finished 9 of 10 concluded works" — always built by `domain::finishers::summary`.
    summary: String,
    cadence: Cadence,
}

async fn author_signals(
    State(state): State<AppState>,
    Path(pseud): Path<String>,
) -> ApiResult<Json<AuthorSignals>> {
    if !finishers::pseud_exists(state.db(), &pseud)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?
    {
        return Err(ApiError(AppError::NotFound { resource: "author" }));
    }
    let history: History = finishers::author_history(state.db(), &pseud)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    let cadence = finishers::author_cadence(state.db(), &pseud)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    Ok(Json(AuthorSignals {
        finished: history.finished,
        concluded: history.concluded,
        summary: summary(&history),
        cadence,
    }))
}

async fn work_cadence(
    State(state): State<AppState>,
    Path(work_id): Path<String>,
) -> ApiResult<Json<Cadence>> {
    let cadence = finishers::work_cadence(state.db(), &work_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    Ok(Json(cadence))
}
