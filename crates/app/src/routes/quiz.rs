//! Onboarding taste quiz routes (spec §0.4.2).
//!
//! New users pick works they like; the system computes an initial taste vector
//! from the picked works' vectors, blended toward neutral so quiz-only data
//! never produces extreme vectors. The quiz is skippable — skipping leaves the
//! account without a vector, which the discovery pipeline treats as
//! egalitarian.

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use lorehaven_domain::AppError;
use serde::Deserialize;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/quiz/works", get(get_quiz_works))
        .route("/quiz/answers", post(post_quiz_answers))
        .route("/quiz/answers", get(get_my_quiz_answers))
        .route("/quiz/skip", post(skip_quiz))
        .route(
            "/operator/quiz-works",
            get(admin_list_quiz_works).post(admin_set_quiz_works),
        )
}

/// Query params for `GET /quiz/works`.
#[derive(Debug, Deserialize)]
pub struct QuizWorksQuery {
    #[serde(default)]
    pub limit: Option<u32>,
}

/// The quiz works: published works with a computed taste vector, newest first.
/// Falls back to recently published works when no vectors exist yet.
async fn get_quiz_works(
    State(state): State<AppState>,
    Query(params): Query<QuizWorksQuery>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let _ = user;
    let limit = params.limit.unwrap_or(12).clamp(1, 50);
    let works = lorehaven_db::taste_health::list_quiz_works(state.db(), limit as i64)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(serde_json::json!({ "works": works })))
}

/// Body for `POST /quiz/answers`: the works the user picked (and, optionally,
/// the ones they explicitly rejected).
#[derive(Debug, serde::Deserialize)]
pub struct QuizAnswersBody {
    pub picked: Vec<String>,
    #[serde(default)]
    pub rejected: Vec<String>,
}

async fn post_quiz_answers(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<QuizAnswersBody>,
) -> ApiResult<Json<serde_json::Value>> {
    if body.picked.len() + body.rejected.len() > 100 {
        return Err(ApiError(AppError::field(
            "picked",
            "at most 100 works total",
        )));
    }
    let account_id = user.account_id.to_string();
    let mut answers: Vec<(String, bool)> =
        Vec::with_capacity(body.picked.len() + body.rejected.len());
    for id in &body.picked {
        answers.push((id.clone(), true));
    }
    for id in &body.rejected {
        answers.push((id.clone(), false));
    }
    lorehaven_db::taste_health::save_quiz_answers(state.db(), &account_id, &answers)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    let vector = lorehaven_db::taste_health::compute_and_store_quiz_vector(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(serde_json::json!({
        "status": "saved",
        "vector_dimensions": vector.len(),
    })))
}

/// The user's own quiz answers.
async fn get_my_quiz_answers(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let account_id = user.account_id.to_string();
    let answers = lorehaven_db::taste_health::get_quiz_answers(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    let picked: Vec<String> = answers
        .iter()
        .filter(|(_, p)| *p)
        .map(|(w, _)| w.clone())
        .collect();
    let rejected: Vec<String> = answers
        .iter()
        .filter(|(_, p)| !*p)
        .map(|(w, _)| w.clone())
        .collect();
    Ok(Json(serde_json::json!({
        "picked": picked,
        "rejected": rejected,
    })))
}

/// Skip the quiz: records nothing, leaves the account vectorless (egalitarian).
async fn skip_quiz(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let _ = state;
    let _ = user;
    Ok(Json(serde_json::json!({ "status": "skipped" })))
}

/// Admin: list the curated quiz work set.
async fn admin_list_quiz_works(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    crate::routes::discovery::require_operator(&state, &user)?;
    let works = lorehaven_db::taste_health::list_quiz_works(state.db(), 50)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(serde_json::json!({ "works": works })))
}

/// Admin: set the curated quiz work set (replaces any prior set).
#[derive(Debug, serde::Deserialize)]
pub struct AdminQuizWorksBody {
    pub work_ids: Vec<String>,
}

async fn admin_set_quiz_works(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<AdminQuizWorksBody>,
) -> ApiResult<Json<serde_json::Value>> {
    crate::routes::discovery::require_operator(&state, &user)?;
    if body.work_ids.len() > 200 {
        return Err(ApiError(AppError::field(
            "work_ids",
            "at most 200 quiz works",
        )));
    }
    lorehaven_db::taste_health::set_admin_quiz_works(state.db(), &body.work_ids)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(serde_json::json!({ "status": "set" })))
}
