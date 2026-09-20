//! Thread mode routes (spec §35.3, repo M33).
//!
//! Each topic carries a mode that restructures one surface. The mode is data;
//! a plain topic is unaffected, and a reading-group topic gets a schedule of
//! sections. The routes here manage the mode and its supporting data.

use axum::extract::{Path, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use lorehaven_domain::thread_modes::ThreadMode;
use lorehaven_domain::typed_votes::is_moderator;

use crate::auth::RequirePseud;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use lorehaven_db::thread_modes;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/topics/{id}/mode", get(get_mode).put(put_mode))
        .route("/topics/{id}/schedule", get(get_schedule).post(add_schedule_section))
        .route("/topics/{id}/wiki-pin", get(get_wiki_pin).post(post_wiki_pin).put(approve_wiki_pin))
        .route("/topics/{id}/critique/join", post(join_critique))
        .route("/topics/{id}/critique/queue", get(get_critique_queue))
}

// ---------------------------------------------------------------------------
// Mode
// ---------------------------------------------------------------------------

async fn get_mode(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let topic = lorehaven_db::community::topic_by_id(state.db(), &id)
        .await
        .map_err(internal)?;
    match topic {
        Some(t) => Ok(Json(json!({ "mode": t.mode.or(Some("plain".to_string())).unwrap_or_default() }))),
        None => Err(ApiError(lorehaven_domain::AppError::NotFound { resource: "topic" })),
    }
}

#[derive(Debug, Deserialize)]
struct PutModeBody {
    mode: String,
}

async fn put_mode(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
    Json(body): Json<PutModeBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let mode = ThreadMode::from_str(&body.mode)
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::field("mode", "unknown thread mode")))?;
    let topic = lorehaven_db::community::topic_by_id(state.db(), &id)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "topic" }))?;
    // Only the topic author or a moderator can change the mode.
    let trust = lorehaven_db::governance::trust_for(state.db(), &pseud_id.to_string())
        .await
        .map_err(internal)?;
    if topic.author_pseud != pseud_id.to_string() && !is_moderator(trust)
    {
        return Err(ApiError(lorehaven_domain::AppError::access_denied(
            "Only the topic author or a moderator can change the thread mode.",
        )));
    }
    lorehaven_db::community::set_topic_mode(state.db(), &id, mode.as_str())
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "mode": mode.as_str() })))
}

// ---------------------------------------------------------------------------
// Reading group schedule
// ---------------------------------------------------------------------------

async fn get_schedule(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let sections = thread_modes::get_schedule(state.db(), &id)
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "sections": sections })))
}

#[derive(Debug, Deserialize)]
struct ScheduleSectionBody {
    position: i64,
    title: String,
    chapter_start: i64,
    chapter_end: i64,
    unlocks_at: String,
}

async fn add_schedule_section(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
    Json(body): Json<ScheduleSectionBody>,
) -> ApiResult<Json<serde_json::Value>> {
    // Only the topic author can add schedule sections.
    let topic = lorehaven_db::community::topic_by_id(state.db(), &id)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "topic" }))?;
    if topic.author_pseud != pseud_id.to_string() {
        return Err(ApiError(lorehaven_domain::AppError::access_denied(
            "Only the topic author can add schedule sections.",
        )));
    }
    thread_modes::add_schedule_section(
        state.db(),
        &id,
        body.position,
        &body.title,
        body.chapter_start,
        body.chapter_end,
        &body.unlocks_at,
    )
    .await
    .map_err(internal)?;
    Ok(Json(json!({ "added": true })))
}

// ---------------------------------------------------------------------------
// Wiki pin
// ---------------------------------------------------------------------------

async fn get_wiki_pin(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let pin = thread_modes::get_wiki_pin(state.db(), &id)
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "wiki_pin": pin })))
}

#[derive(Debug, Deserialize)]
struct WikiPinBody {
    body: String,
}

async fn post_wiki_pin(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
    Json(body): Json<WikiPinBody>,
) -> ApiResult<Json<serde_json::Value>> {
    // Create a new wiki pin (requires a post_id — for simplicity we create a
    // dummy post that will serve as the pin container).
    let post_id = uuid::Uuid::new_v4().to_string();
    thread_modes::create_wiki_pin(state.db(), &id, &post_id, &body.body, &pseud_id.to_string())
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "created": true })))
}

#[derive(Debug, Deserialize)]
struct ApprovePinBody {
    post_id: String,
}

async fn approve_wiki_pin(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
    Json(body): Json<ApprovePinBody>,
) -> ApiResult<Json<serde_json::Value>> {
    // Only the topic author or moderators can approve.
    let topic = lorehaven_db::community::topic_by_id(state.db(), &id)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "topic" }))?;
    let trust = lorehaven_db::governance::trust_for(state.db(), &pseud_id.to_string())
        .await
        .map_err(internal)?;
    if topic.author_pseud != pseud_id.to_string() && !is_moderator(trust)
    {
        return Err(ApiError(lorehaven_domain::AppError::access_denied(
            "Only the topic author or a moderator can approve the wiki pin.",
        )));
    }
    thread_modes::approve_wiki_pin(state.db(), &id, &body.post_id, &pseud_id.to_string())
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "approved": true })))
}

// ---------------------------------------------------------------------------
// Critique circle
// ---------------------------------------------------------------------------

async fn join_critique(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let position = thread_modes::join_critique(state.db(), &id, &pseud_id.to_string())
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "joined": true, "position": position })))
}

async fn get_critique_queue(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let queue = thread_modes::get_critique_queue(state.db(), &id)
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "queue": queue })))
}

fn internal(e: anyhow::Error) -> ApiError {
    ApiError(lorehaven_domain::AppError::Internal(e))
}
