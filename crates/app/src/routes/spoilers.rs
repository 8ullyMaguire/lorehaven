//! Spoiler and readability routes (spec §35.4, repo M34).
//!
//! Covers: spoiler scope on topics, content warnings, reader progress,
//! draft autosave, scheduled posts, and warning prefs.

use axum::extract::{Path, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use lorehaven_domain::spoilers::{WarningAction, WarningType};

use crate::auth::RequirePseud;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use lorehaven_db::spoilers;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/topics/{id}/spoiler-scope", put(put_spoiler_scope))
        .route("/works/{id}/progress", get(get_progress).put(put_progress))
        .route("/posts/{id}/warnings", get(get_warnings).post(post_warning))
        .route("/topics/{id}/draft", get(get_draft).post(post_draft).delete(delete_draft))
        .route("/posts/{id}/schedule", post(post_schedule))
        .route("/posts/scheduled/due", get(get_due_scheduled))
        .route("/posts/scheduled/{id}/publish", post(post_publish_scheduled))
        .route("/me/warning-prefs", get(get_warning_prefs).put(put_warning_pref))
}

// ---------------------------------------------------------------------------
// Spoiler scope
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct SpoilerScopeBody {
    chapter: Option<i64>,
}

async fn put_spoiler_scope(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
    Json(body): Json<SpoilerScopeBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let topic = lorehaven_db::community::topic_by_id(state.db(), &id)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "topic" }))?;
    // Only the topic author or a moderator (TL3+) can set spoiler scope.
    let trust = lorehaven_db::governance::trust_for(state.db(), &pseud_id.to_string())
        .await
        .map_err(internal)?;
    if topic.author_pseud != pseud_id.to_string()
        && !lorehaven_domain::typed_votes::is_moderator(trust)
    {
        return Err(ApiError(lorehaven_domain::AppError::access_denied(
            "Only the topic author or a moderator can set spoiler scope.",
        )));
    }
    spoilers::set_topic_spoiler_scope(state.db(), &id, body.chapter)
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "spoiler_scope_chapter": body.chapter })))
}

// ---------------------------------------------------------------------------
// Reader progress
// ---------------------------------------------------------------------------

async fn get_progress(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let progress = spoilers::get_reader_progress(state.db(), &pseud_id.to_string(), &id)
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "last_chapter": progress.unwrap_or(0) })))
}

#[derive(Debug, Deserialize)]
struct ProgressBody {
    last_chapter: i64,
}

async fn put_progress(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
    Json(body): Json<ProgressBody>,
) -> ApiResult<Json<serde_json::Value>> {
    spoilers::upsert_reader_progress(state.db(), &pseud_id.to_string(), &id, body.last_chapter)
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "last_chapter": body.last_chapter })))
}

// ---------------------------------------------------------------------------
// Content warnings
// ---------------------------------------------------------------------------

async fn get_warnings(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let warnings = spoilers::list_content_warnings(state.db(), &id)
        .await
        .map_err(internal)?;
    let warnings: Vec<serde_json::Value> = warnings
        .into_iter()
        .map(|w| {
            json!({
                "warning_type": w.warning_type,
                "severity": w.severity,
                "custom_text": w.custom_text,
            })
        })
        .collect();
    Ok(Json(json!({ "warnings": warnings })))
}

#[derive(Debug, Deserialize)]
struct WarningBody {
    warning_type: String,
    severity: Option<i64>,
    custom_text: Option<String>,
}

async fn post_warning(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
    Json(body): Json<WarningBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let warning_type = WarningType::from_str(&body.warning_type)
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::field("warning_type", "unknown type")))?;
    let severity = body.severity.unwrap_or(1);
    // Only the post author can add warnings (verified via post lookup).
    let post = lorehaven_db::community::post_by_id(state.db(), &id)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "post" }))?;
    if post.author_pseud != pseud_id.to_string() {
        return Err(ApiError(lorehaven_domain::AppError::access_denied(
            "Only the post author can add content warnings.",
        )));
    }
    spoilers::add_content_warning(state.db(), &id, warning_type, severity, body.custom_text.as_deref())
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "added": true })))
}

// ---------------------------------------------------------------------------
// Draft autosave
// ---------------------------------------------------------------------------

async fn get_draft(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let body = spoilers::get_draft(state.db(), &pseud_id.to_string(), &id)
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "body": body.unwrap_or_default() })))
}

#[derive(Debug, Deserialize)]
struct DraftBody {
    body: String,
}

async fn post_draft(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
    Json(body): Json<DraftBody>,
) -> ApiResult<Json<serde_json::Value>> {
    spoilers::upsert_draft(state.db(), &pseud_id.to_string(), &id, &body.body)
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "saved": true })))
}

async fn delete_draft(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let deleted = spoilers::delete_draft(state.db(), &pseud_id.to_string(), &id)
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "deleted": deleted })))
}

// ---------------------------------------------------------------------------
// Scheduled posts
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ScheduleBody {
    scheduled_at: String,
}

async fn post_schedule(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
    Json(body): Json<ScheduleBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let post = lorehaven_db::community::post_by_id(state.db(), &id)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "post" }))?;
    if post.author_pseud != pseud_id.to_string() {
        return Err(ApiError(lorehaven_domain::AppError::access_denied(
            "Only the post author can schedule it.",
        )));
    }
    spoilers::schedule_post(state.db(), &id, &body.scheduled_at)
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "scheduled": true })))
}

async fn get_due_scheduled(
    State(state): State<AppState>,
) -> ApiResult<Json<serde_json::Value>> {
    let now = lorehaven_db::identity::now_rfc3339();
    let due = spoilers::list_due_scheduled_posts(state.db(), &now, 50)
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "post_ids": due })))
}

async fn post_publish_scheduled(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let published = spoilers::publish_scheduled_post(state.db(), &id)
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "published": published })))
}

// ---------------------------------------------------------------------------
// Warning prefs
// ---------------------------------------------------------------------------

async fn get_warning_prefs(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
) -> ApiResult<Json<serde_json::Value>> {
    let prefs = spoilers::list_warning_prefs(state.db(), &pseud_id.to_string())
        .await
        .map_err(internal)?;
    let prefs: Vec<serde_json::Value> = prefs
        .into_iter()
        .map(|(t, a)| {
            json!({ "warning_type": t.as_str(), "action": a.as_str() })
        })
        .collect();
    Ok(Json(json!({ "prefs": prefs })))
}

#[derive(Debug, Deserialize)]
struct WarningPrefBody {
    warning_type: String,
    action: String,
}

async fn put_warning_pref(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Json(body): Json<WarningPrefBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let warning_type = WarningType::from_str(&body.warning_type)
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::field("warning_type", "unknown type")))?;
    let action = WarningAction::from_str(&body.action)
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::field("action", "must be 'blur' or 'show'")))?;
    spoilers::set_warning_pref(state.db(), &pseud_id.to_string(), warning_type, action)
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "set": true })))
}

fn internal(e: anyhow::Error) -> ApiError {
    ApiError(lorehaven_domain::AppError::Internal(e))
}
