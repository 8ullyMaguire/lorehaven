//! M17 — Translation routes: jobs, units, memory, glossaries, reviews, publications.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::str::FromStr;

use crate::auth::MaybeSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

/// Create a translation job.
#[derive(Debug, Deserialize)]
pub struct CreateTranslationBody {
    pub source_lang: String,
    pub target_lang: String,
    pub provider: String,
}

pub async fn create_translation(
    State(state): State<AppState>,
    Path(work_id): Path<String>,
    MaybeSession(user): MaybeSession,
    Json(body): Json<CreateTranslationBody>,
) -> ApiResult<Json<Value>> {
    let account = user.map(|u| u.account_id.to_string()).unwrap_or_default();
    if account.is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "session",
            "sign in to request translations",
        )));
    }

    let id = lorehaven_db::translation::create_job(
        state.db(),
        &work_id,
        &body.source_lang,
        &body.target_lang,
        &body.provider,
        None,
        &account,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({ "id": id })))
}

/// Get translation job status.
pub async fn get_translation(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
    MaybeSession(_user): MaybeSession,
) -> ApiResult<Json<Value>> {
    let job = lorehaven_db::translation::get_job(state.db(), &job_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    match job {
        Some(j) => Ok(Json(j)),
        None => Err(ApiError(lorehaven_domain::AppError::field(
            "job",
            "not found",
        ))),
    }
}

/// Transition a translation job.
#[derive(Debug, Deserialize)]
pub struct TransitionJobBody {
    pub from_state: String,
    pub to_state: String,
}

pub async fn transition_translation(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
    MaybeSession(_user): MaybeSession,
    Json(body): Json<TransitionJobBody>,
) -> ApiResult<Json<Value>> {
    let from = lorehaven_domain::translation::TranslationJobState::from_str(&body.from_state)
        .map_err(|e| ApiError(lorehaven_domain::AppError::field("from_state", &e)))?;
    let to = lorehaven_domain::translation::TranslationJobState::from_str(&body.to_state)
        .map_err(|e| ApiError(lorehaven_domain::AppError::field("to_state", &e)))?;

    lorehaven_db::translation::transition_job(state.db(), &job_id, &from, &to)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({ "success": true })))
}

/// List translation memory entries for the caller.
pub async fn list_memory(
    State(state): State<AppState>,
    MaybeSession(user): MaybeSession,
) -> ApiResult<Json<Value>> {
    let account = user.map(|u| u.account_id.to_string()).unwrap_or_default();
    if account.is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "session",
            "sign in to list translation memory",
        )));
    }
    let memory = lorehaven_db::translation::list_memory(state.db(), &account)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "memory": memory })))
}

/// Add glossary term.
#[derive(Debug, Deserialize)]
pub struct AddGlossaryBody {
    pub source_lang: String,
    pub target_lang: String,
    pub term: String,
    pub translation: String,
    pub case_sensitive: Option<bool>,
}

pub async fn add_glossary_term(
    State(state): State<AppState>,
    MaybeSession(user): MaybeSession,
    Json(body): Json<AddGlossaryBody>,
) -> ApiResult<Json<Value>> {
    let account = user.map(|u| u.account_id.to_string()).unwrap_or_default();
    if account.is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "session",
            "sign in to manage glossaries",
        )));
    }

    let id = lorehaven_db::translation::add_glossary_term(
        state.db(),
        &account,
        None,
        &body.source_lang,
        &body.target_lang,
        &body.term,
        &body.translation,
        body.case_sensitive.unwrap_or(false),
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({ "id": id })))
}

/// Open a review gate.
#[derive(Debug, Deserialize)]
pub struct OpenReviewBody {
    pub reviewer: String,
    pub gate: String,
}

pub async fn open_review(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
    MaybeSession(_user): MaybeSession,
    Json(body): Json<OpenReviewBody>,
) -> ApiResult<Json<Value>> {
    let gate = lorehaven_domain::translation::ReviewGate::from_str(&body.gate)
        .map_err(|e| ApiError(lorehaven_domain::AppError::field("gate", &e)))?;

    let id =
        lorehaven_db::translation::open_review_gate(state.db(), &job_id, &body.reviewer, &gate)
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({ "id": id })))
}

/// Decide a review gate.
#[derive(Debug, Deserialize)]
pub struct DecideReviewBody {
    pub state: String,
    pub notes: Option<String>,
}

pub async fn decide_review(
    State(state): State<AppState>,
    Path(review_id): Path<String>,
    MaybeSession(_user): MaybeSession,
    Json(body): Json<DecideReviewBody>,
) -> ApiResult<Json<Value>> {
    lorehaven_db::translation::decide_review_gate(
        state.db(),
        &review_id,
        &body.state,
        body.notes.as_deref(),
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({ "success": true })))
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/works/{id}/translations", post(create_translation))
        .route("/translations/{job_id}", get(get_translation))
        .route(
            "/translations/{job_id}/transition",
            post(transition_translation),
        )
        .route("/me/translation-memory", get(list_memory))
        .route("/me/translation-glossaries", post(add_glossary_term))
        .route("/translations/{job_id}/reviews", post(open_review))
        .route(
            "/translation-reviews/{review_id}/decide",
            post(decide_review),
        )
}
