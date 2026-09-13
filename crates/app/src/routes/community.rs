//! Community routes: comments, forums, groups, messaging, blocks, presence.
//!
//! Spec §17.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use lorehaven_domain::blocking::BlockScope;
use serde::Deserialize;

use crate::auth::{RequirePseud, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        // comments
        .route("/works/:id/comments", get(get_work_comments).post(post_comment))
        .route("/comments/:id/delete", post(delete_comment))
        // forums
        .route("/forums", get(get_forums))
        .route("/forums/:category/topics", get(get_forums_topics).post(post_topic))
        .route("/topics/:id", get(get_topic))
        .route("/topics/:id/replies", get(get_topic_replies).post(post_reply))
        .route("/topics/:id/lock", post(lock_topic))
        // groups
        .route("/groups", get(get_groups).post(post_group))
        .route("/groups/:id", get(get_group))
        .route("/groups/:id/join", post(join_group))
        .route("/groups/:id/leave", post(leave_group))
        .route("/groups/:id/role", put(put_role))
        // messaging
        .route("/conversations", get(get_conversations).post(post_conversation))
        .route("/conversations/:id/messages", get(get_conv_messages).post(post_message))
        // blocks and mutes
        .route("/me/blocks", get(get_blocks).post(post_block))
        .route("/me/blocks/:id", delete(delete_block))
        .route("/me/mutes", get(get_mutes).post(post_mute))
        .route("/me/mutes/:id", delete(delete_mute))
        // presence
        .route("/presence/stream", get(get_presence_stream))
}

// ---------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CursorQuery {
    cursor: Option<String>,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    20
}

#[derive(Debug, Deserialize)]
pub struct CreateCommentBody {
    body: String,
}

async fn get_work_comments(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Query(params): Query<CursorQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let items = lorehaven_db::community::list_comments(
        state.db(), "work", &id, &user.account_id.to_string(),
        params.cursor.as_deref(), params.limit,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "items": items })))
}

async fn post_comment(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
    Json(body): Json<CreateCommentBody>,
) -> ApiResult<Json<serde_json::Value>> {
    if body.body.trim().is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::field("body", "A comment needs some words.")));
    }
    let id = lorehaven_db::community::insert_comment(
        state.db(), "work", &id, &pseud_id.to_string(), &body.body, None,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "id": id })))
}

async fn delete_comment(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let deleted = lorehaven_db::community::soft_delete_comment(
        state.db(), &id, &pseud_id.to_string(),
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    if !deleted {
        return Err(ApiError(lorehaven_domain::AppError::NotFound { resource: "comment" }));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Forums
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateTopicBody {
    title: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateReplyBody {
    body: String,
}

async fn get_forums(State(state): State<AppState>, RequireSession(_): RequireSession) -> ApiResult<Json<serde_json::Value>> {
    let _ = state;
    Ok(Json(serde_json::json!({ "items": [] })))
}

async fn get_forums_topics(
    State(state): State<AppState>, RequireSession(_): RequireSession,
    Path(category): Path<String>, Query(_params): Query<CursorQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let _ = (state, category);
    Ok(Json(serde_json::json!({ "items": [] })))
}

async fn post_topic(
    State(state): State<AppState>, RequirePseud { pseud_id, .. }: RequirePseud,
    Path(category): Path<String>, Json(body): Json<CreateTopicBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let id = lorehaven_db::community::create_topic(state.db(), &category, &pseud_id.to_string(), &body.title)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "id": id })))
}

async fn get_topic(
    State(state): State<AppState>, RequireSession(_): RequireSession, Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let topic = lorehaven_db::community::topic_by_id(state.db(), &id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    match topic {
        Some(t) => Ok(Json(serde_json::json!({ "topic": t }))),
        None => Err(ApiError(lorehaven_domain::AppError::NotFound { resource: "topic" })),
    }
}

async fn get_topic_replies(
    State(state): State<AppState>, RequireSession(_): RequireSession,
    Path(id): Path<String>, Query(params): Query<CursorQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let items = lorehaven_db::community::list_posts(state.db(), &id, params.cursor.as_deref(), params.limit)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "items": items })))
}

async fn post_reply(
    State(state): State<AppState>, RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>, Json(body): Json<CreateReplyBody>,
) -> ApiResult<Json<serde_json::Value>> {
    if body.body.trim().is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::field("body", "A reply needs some words.")));
    }
    let id = lorehaven_db::community::create_post(state.db(), &id, &pseud_id.to_string(), &body.body)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "id": id })))
}

async fn lock_topic(State(state): State<AppState>, RequirePseud { .. }: RequirePseud, Path(_id): Path<String>) -> ApiResult<StatusCode> {
    let _ = state;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Groups
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateGroupBody {
    name: String,
    privacy: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRoleBody {
    account: String,
    role: String,
}

async fn get_groups(
    State(state): State<AppState>, RequireSession(user): RequireSession,
    Query(params): Query<CursorQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let items = lorehaven_db::community::list_groups(state.db(), &user.account_id.to_string(), params.limit)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "items": items })))
}

async fn post_group(
    State(state): State<AppState>, RequirePseud { user, .. }: RequirePseud,
    Json(body): Json<CreateGroupBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let id = lorehaven_db::community::create_group(state.db(), &body.name, &body.privacy, &user.account_id.to_string())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "id": id })))
}

async fn get_group(State(state): State<AppState>, RequireSession(_): RequireSession, Path(id): Path<String>) -> ApiResult<Json<serde_json::Value>> {
    let group = lorehaven_db::community::group_by_id(state.db(), &id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    match group {
        Some(g) => Ok(Json(serde_json::json!({ "group": g }))),
        None => Err(ApiError(lorehaven_domain::AppError::NotFound { resource: "group" })),
    }
}

async fn join_group(State(state): State<AppState>, RequirePseud { user, .. }: RequirePseud, Path(id): Path<String>) -> ApiResult<StatusCode> {
    lorehaven_db::community::add_group_member(state.db(), &id, &user.account_id.to_string(), "member")
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn leave_group(State(state): State<AppState>, RequirePseud { user, .. }: RequirePseud, Path(id): Path<String>) -> ApiResult<StatusCode> {
    let removed = lorehaven_db::community::remove_group_member(state.db(), &id, &user.account_id.to_string())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    if !removed {
        return Err(ApiError(lorehaven_domain::AppError::NotFound { resource: "group_member" }));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn put_role(
    State(state): State<AppState>, RequirePseud { .. }: RequirePseud,
    Path(id): Path<String>, Json(body): Json<UpdateRoleBody>,
) -> ApiResult<StatusCode> {
    let updated = lorehaven_db::community::update_member_role(state.db(), &id, &body.account, &body.role)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    if !updated {
        return Err(ApiError(lorehaven_domain::AppError::NotFound { resource: "group_member" }));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Messaging
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateConversationBody {
    participant: String,
}

#[derive(Debug, Deserialize)]
pub struct SendMessageBody {
    body: String,
}

async fn get_conversations(State(state): State<AppState>, RequireSession(_): RequireSession) -> ApiResult<Json<serde_json::Value>> {
    let _ = state;
    Ok(Json(serde_json::json!({ "items": [] })))
}

async fn post_conversation(
    State(state): State<AppState>, RequirePseud { user, .. }: RequirePseud,
    Json(body): Json<CreateConversationBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let conversation_id = lorehaven_db::community::create_conversation(state.db())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    lorehaven_db::community::add_participant(state.db(), &conversation_id, &user.account_id.to_string())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    lorehaven_db::community::add_participant(state.db(), &conversation_id, &body.participant)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "id": conversation_id })))
}

async fn get_conv_messages(
    State(state): State<AppState>, RequireSession(user): RequireSession,
    Path(id): Path<String>, Query(params): Query<CursorQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let items = lorehaven_db::community::list_messages(
        state.db(), &id, &user.account_id.to_string(),
        params.cursor.as_deref(), params.limit,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "items": items })))
}

async fn post_message(
    State(state): State<AppState>, RequirePseud { user, .. }: RequirePseud,
    Path(id): Path<String>, Json(body): Json<SendMessageBody>,
) -> ApiResult<Json<serde_json::Value>> {
    if body.body.trim().is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::field("body", "A message needs some words.")));
    }
    // Block-aware: check if the sender is blocked by any participant
    let participants = lorehaven_db::community::conversation_participants(state.db(), &id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    for participant in &participants {
        if participant != &user.account_id.to_string() {
            let blocked = lorehaven_db::community::is_blocked(
                state.db(), participant, &user.account_id.to_string(), BlockScope::Messages,
            )
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
            if blocked {
                return Err(ApiError(lorehaven_domain::AppError::field("participant", "Cannot send to this participant.")));
            }
        }
    }
    let id = lorehaven_db::community::send_message(state.db(), &id, &user.account_id.to_string(), &body.body)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "id": id })))
}

// ---------------------------------------------------------------------------
// Blocks and mutes
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BlockBody {
    blocked: String,
    scope: String,
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MuteBody {
    muted: String,
    until: Option<String>,
}

async fn get_blocks(State(state): State<AppState>, RequireSession(_): RequireSession) -> ApiResult<Json<serde_json::Value>> {
    let _ = state;
    Ok(Json(serde_json::json!({ "items": [] })))
}

async fn post_block(State(state): State<AppState>, RequireSession(user): RequireSession, Json(body): Json<BlockBody>) -> ApiResult<StatusCode> {
    let scope = BlockScope::parse(&body.scope).ok_or_else(|| ApiError(lorehaven_domain::AppError::field("scope", "Invalid block scope.")))?;
    lorehaven_db::community::insert_block(state.db(), &user.account_id.to_string(), &body.blocked, scope, body.note.as_deref())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_block(State(state): State<AppState>, RequireSession(user): RequireSession, Path(id): Path<String>) -> ApiResult<StatusCode> {
    let removed = lorehaven_db::community::delete_block(state.db(), &user.account_id.to_string(), &id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    if !removed {
        return Err(ApiError(lorehaven_domain::AppError::NotFound { resource: "block" }));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn get_mutes(State(state): State<AppState>, RequireSession(_): RequireSession) -> ApiResult<Json<serde_json::Value>> {
    let _ = state;
    Ok(Json(serde_json::json!({ "items": [] })))
}

async fn post_mute(State(state): State<AppState>, RequireSession(user): RequireSession, Json(body): Json<MuteBody>) -> ApiResult<StatusCode> {
    lorehaven_db::community::insert_mute(state.db(), &user.account_id.to_string(), &body.muted, body.until.as_deref())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_mute(State(state): State<AppState>, RequireSession(user): RequireSession, Path(id): Path<String>) -> ApiResult<StatusCode> {
    let removed = lorehaven_db::community::delete_mute(state.db(), &user.account_id.to_string(), &id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    if !removed {
        return Err(ApiError(lorehaven_domain::AppError::NotFound { resource: "mute" }));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Presence
// ---------------------------------------------------------------------------

async fn get_presence_stream(State(state): State<AppState>, RequireSession(_): RequireSession) -> ApiResult<Json<serde_json::Value>> {
    let _ = state;
    Ok(Json(serde_json::json!({ "items": [] })))
}
