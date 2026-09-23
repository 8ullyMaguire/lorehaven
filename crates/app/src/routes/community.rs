//! Community routes: comments, forums, groups, messaging, blocks, presence.
//!
//! Spec §17.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use lorehaven_domain::blocking::BlockScope;
use lorehaven_db::community::ForumPost;
use serde::Deserialize;
use std::str::FromStr;
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};

use crate::auth::{MaybeSession, RequirePseud, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        // comments
        .route(
            "/works/{id}/comments",
            get(get_work_comments).post(post_comment),
        )
        .route("/comments/{id}/delete", post(delete_comment))
        // forums
        .route("/forums", get(get_forums))
        .route(
            "/forums/{category}/topics",
            get(get_forums_topics).post(post_topic),
        )
        .route("/topics/{id}", get(get_topic))
        .route(
            "/topics/{id}/replies",
            get(get_topic_replies).post(post_reply),
        )
        .route("/topics/{id}/lock", post(lock_topic))
        // forum search (spec §17.4)
        .route("/forum-search", get(forum_search))
        // topic subscriptions & unread tracking (spec §17.4, §5.5)
        .route("/topics/{id}/subscribe", post(subscribe_topic))
        .route("/topics/{id}/unsubscribe", post(unsubscribe_topic))
        .route("/topics/{id}/mark-read", post(mark_topic_read))
        .route("/topics/{id}/unread", get(get_unread_count))
        // groups
        .route("/groups", get(get_groups).post(post_group))
        .route("/groups/{id}", get(get_group))
        .route("/groups/{id}/join", post(join_group))
        .route("/groups/{id}/leave", post(leave_group))
        .route("/groups/{id}/role", put(put_role))
        // messaging
        .route(
            "/conversations",
            get(get_conversations).post(post_conversation),
        )
        .route(
            "/conversations/{id}/messages",
            get(get_conv_messages).post(post_message),
        )
        // blocks and mutes
        .route("/me/blocks", get(get_blocks).post(post_block))
        .route("/me/blocks/{id}", delete(delete_block))
        .route("/me/mutes", get(get_mutes).post(post_mute))
        .route("/me/mutes/{id}", delete(delete_mute))
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
    /// Optional anchor kind: "paragraph" or "timestamp".
    anchor_kind: Option<String>,
    /// Anchor value (paragraph index or timestamp).
    anchor_value: Option<String>,
    /// Chapter id for paragraph anchors.
    anchor_chapter_id: Option<String>,
}

async fn get_work_comments(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Query(params): Query<CursorQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let items = lorehaven_db::community::list_comments(
        state.db(),
        "work",
        &id,
        &user.account_id.to_string(),
        params.cursor.as_deref(),
        params.limit,
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
        return Err(ApiError(lorehaven_domain::AppError::field(
            "body",
            "A comment needs some words.",
        )));
    }
    let work_id: lorehaven_domain::WorkId = id
        .parse()
        .map_err(|_| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;
    // A ThreadOnly work has no comment surface at all: the reaction bar and
    // the linked forum thread are where feedback lives (spec 35.0). The
    // refusal is the server's, never the interface's alone.
    let discussion_mode = lorehaven_db::work_discussion::work_discussion_mode(state.db(), &id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    let effective = lorehaven_domain::work_discussion::resolve_discussion_mode(
        discussion_mode,
        state.config().forum.work_discussion_default,
    );
    if !effective.comments_enabled() {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "work",
            "This work's discussion lives in its forum thread; use the Discuss link.",
        )));
    }
    let author_account = lorehaven_db::positivity::author_account_for_work(state.db(), work_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    let (allow, deny, prefs) = match author_account {
        Some(author) => {
            let prefs = lorehaven_db::positivity::preferences_for(state.db(), author)
                .await
                .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
            let (allow, deny) =
                lorehaven_db::positivity::list_membership(state.db(), author, pseud_id)
                    .await
                    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
            (allow, deny, prefs)
        }
        None => (
            false,
            false,
            lorehaven_domain::positivity::FeedbackPreferences::default(),
        ),
    };
    // Validate anchor fields if provided.
    let (anchor_kind, anchor_value, anchor_chapter_id) = match (
        &body.anchor_kind,
        &body.anchor_value,
        &body.anchor_chapter_id,
    ) {
        (Some(kind_str), Some(value), chapter_id) => {
            let kind = lorehaven_domain::anchor::AnchorKind::from_str(kind_str)
                .map_err(|e| ApiError(lorehaven_domain::AppError::field("anchor_kind", &e)))?;
            lorehaven_domain::anchor::validate_anchor(kind, value, chapter_id.as_deref())
                .map_err(|e| ApiError(lorehaven_domain::AppError::field("anchor", &e)))?;
            (
                Some(kind_str.clone()),
                Some(value.clone()),
                chapter_id.clone(),
            )
        }
        (None, None, _) => (None, None, None),
        _ => {
            return Err(ApiError(lorehaven_domain::AppError::field(
                "anchor",
                "anchor_kind and anchor_value must be provided together",
            )));
        }
    };

    let id = lorehaven_db::community::insert_comment(
        state.db(),
        "work",
        &id,
        &pseud_id.to_string(),
        &body.body,
        None,
        anchor_kind.as_deref(),
        anchor_value.as_deref(),
        anchor_chapter_id.as_deref(),
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    // Record @handle mentions in the comment body (spec §17.5). Best-effort.
    let _ = lorehaven_db::community::record_mentions(
        state.db(),
        "comment",
        &id,
        &pseud_id.to_string(),
    )
    .await;

    let stored = match lorehaven_db::positivity::classify_comment(
        state.db(),
        &id,
        &body.body,
        &prefs,
        allow,
        deny,
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            lorehaven_db::community::soft_delete_comment(state.db(), &id, &pseud_id.to_string())
                .await
                .ok();
            return Err(ApiError(lorehaven_domain::AppError::Internal(e)));
        }
    };
    let receipt = lorehaven_domain::positivity::sender_receipt(stored.outcome);
    Ok(Json(serde_json::json!({ "id": id, "receipt": receipt })))
}

async fn delete_comment(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let deleted =
        lorehaven_db::community::soft_delete_comment(state.db(), &id, &pseud_id.to_string())
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    if !deleted {
        return Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "comment",
        }));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Forums
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateTopicBody {
    title: String,
    #[serde(default)]
    mode: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateReplyBody {
    body: String,
}

async fn get_forums(
    State(state): State<AppState>,
    RequireSession(_): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let categories = lorehaven_db::community::list_forum_categories(state.db())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "items": categories })))
}

/// Attach the author handle to every topic/post serialized here: the tables
/// store authors as pseud id strings, and a reader reads a handle. Ids that
/// resolve to nothing fall back to the stored string.
async fn with_author_handles<T: serde::Serialize>(
    state: &AppState,
    items: Vec<T>,
    pseud_id: impl Fn(&T) -> &String,
) -> Result<Vec<serde_json::Value>, lorehaven_domain::AppError> {
    let ids: Vec<String> = items.iter().map(pseud_id).cloned().collect();
    let handles = lorehaven_db::identity::handles_for_pseud_ids(state.db(), &ids)
        .await
        .map_err(|e| lorehaven_domain::AppError::Internal(e.into()))?;
    Ok(items
        .into_iter()
        .map(|item| {
            let mut v = serde_json::to_value(&item).unwrap_or_else(|_| serde_json::json!({}));
            if let Some(id) = v
                .get("author_pseud")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
            {
                if let Some(handle) = handles.get(&id) {
                    v["author_handle"] = serde_json::Value::String(handle.clone());
                }
            }
            v
        })
        .collect())
}

async fn get_forums_topics(
    State(state): State<AppState>,
    RequireSession(_): RequireSession,
    Path(category): Path<String>,
    Query(params): Query<CursorQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let topics = lorehaven_db::community::list_topics_in_category(
        state.db(),
        &category,
        params.cursor.as_deref(),
        params.limit,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    let items = with_author_handles(&state, topics, |t| &t.author_pseud)
        .await
        .map_err(ApiError)?;
    Ok(Json(serde_json::json!({ "items": items })))
}

async fn post_topic(
    State(state): State<AppState>,
    RequirePseud { user, pseud_id }: RequirePseud,
    Path(category): Path<String>,
    Json(body): Json<CreateTopicBody>,
) -> ApiResult<Json<serde_json::Value>> {
    // The schema carries no foreign key on category_id, so the write path
    // refuses a dangling topic itself.
    let min_trust = lorehaven_db::community::category_min_trust(state.db(), &category)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    let Some(min_trust) = min_trust else {
        return Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "forum category",
        }));
    };
    let trust = lorehaven_db::governance::trust_for(state.db(), &user.account_id.to_string())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    if trust < min_trust {
        return Err(ApiError(lorehaven_domain::AppError::AccessDenied));
    }
    let id = lorehaven_db::community::create_topic_with_mode(
        state.db(),
        &category,
        &pseud_id.to_string(),
        &body.title,
        body.mode.as_deref().unwrap_or("plain"),
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "id": id })))
}

async fn get_topic(
    State(state): State<AppState>,
    RequireSession(_): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let topic = lorehaven_db::community::topic_by_id(state.db(), &id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    match topic {
        Some(t) => {
            let items = with_author_handles(&state, vec![t], |t| &t.author_pseud)
                .await
                .map_err(ApiError)?;
            Ok(Json(serde_json::json!({ "topic": items[0] })))
        }
        None => Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "topic",
        })),
    }
}

async fn get_topic_replies(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Query(params): Query<CursorQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let posts = lorehaven_db::community::list_posts(
        state.db(),
        &id,
        params.cursor.as_deref(),
        params.limit,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    let posts = filter_blocked_posts(&state, &user.account_id.to_string(), posts).await.map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    let items = with_author_handles(&state, posts, |p| &p.author_pseud)
        .await
        .map_err(ApiError)?;
    Ok(Json(serde_json::json!({ "items": items })))
}

async fn post_reply(
    State(state): State<AppState>,
    RequirePseud { user, pseud_id }: RequirePseud,
    Path(id): Path<String>,
    Json(body): Json<CreateReplyBody>,
) -> ApiResult<Json<serde_json::Value>> {
    if body.body.trim().is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "body",
            "A reply needs some words.",
        )));
    }
    let topic = lorehaven_db::community::topic_by_id(state.db(), &id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    let topic = topic
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "topic" }))?;
    if topic.locked {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "body",
            "this topic is locked",
        )));
    }
    let min_trust = lorehaven_db::community::category_min_trust(state.db(), &topic.category_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    let min_trust = min_trust.unwrap_or(0);
    let trust = lorehaven_db::governance::trust_for(state.db(), &user.account_id.to_string())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    if trust < min_trust {
        return Err(ApiError(lorehaven_domain::AppError::AccessDenied));
    }
    // Block check: if topic author blocks the replier (or vice versa), refuse.
    if let Ok(author_pseud) = topic.author_pseud.parse::<lorehaven_domain::PseudId>() {
        if let Ok(Some(author_info)) = lorehaven_db::identity::find_pseud(state.db(), author_pseud).await {
            let author_account = author_info.account_id.to_string();
            let viewer_blocks = lorehaven_db::community::is_blocked(
                state.db(),
                &user.account_id.to_string(),
                &author_account,
                BlockScope::Comments,
            ).await.unwrap_or(false);
            let author_blocks = lorehaven_db::community::is_blocked(
                state.db(),
                &author_account,
                &user.account_id.to_string(),
                BlockScope::Comments,
            ).await.unwrap_or(false);
            if viewer_blocks || author_blocks {
                return Err(ApiError(lorehaven_domain::AppError::AccessDenied));
            }
        }
    }
    let pid =
        lorehaven_db::community::create_post(state.db(), &id, &pseud_id.to_string(), &body.body)
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;

    lorehaven_db::community::update_topic_last_post(state.db(), &id, &pid).await.map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;

    // Record @handle mentions in the post body (spec §17.5). Best-effort.
    let _ = lorehaven_db::community::record_mentions(
        state.db(),
        "forum_post",
        &pid,
        &user.account_id.to_string(),
    )
    .await;

    // Notify the topic's author about the reply — unless the replier IS the
    // topic author. forum_topics stores the author as a pseud id string, so
    // resolve it to its account. Best-effort: a notification failure must
    // not fail the reply.
    if let Ok(author_pseud) = topic.author_pseud.parse::<lorehaven_domain::PseudId>() {
        if let Ok(Some(author)) =
            lorehaven_db::identity::find_pseud(state.db(), author_pseud).await
        {
            let _ = lorehaven_db::notifications::notify(
                state.db(),
                &author.account_id.to_string(),
                "reply",
                "Someone replied to your topic",
                &format!("{} got a new reply.", topic.title),
                None,
            )
            .await;
        }
    }

    Ok(Json(serde_json::json!({ "id": pid })))
}

/// Filter out posts that the viewer has blocked or been blocked by.
async fn filter_blocked_posts(
    state: &AppState,
    viewer_account_id: &str,
    posts: Vec<ForumPost>,
) -> anyhow::Result<Vec<ForumPost>> {
    let mut visible = Vec::with_capacity(posts.len());
    for post in posts {
        if let Ok(author_pseud) = post.author_pseud.parse::<lorehaven_domain::PseudId>() {
            if let Ok(Some(pseud_info)) = lorehaven_db::identity::find_pseud(state.db(), author_pseud).await {
                let author_account = pseud_info.account_id.to_string();
                let viewer_blocks = lorehaven_db::community::is_blocked(
                    state.db(),
                    &viewer_account_id,
                    &author_account,
                    BlockScope::Comments,
                ).await.unwrap_or(false);
                let author_blocks = lorehaven_db::community::is_blocked(
                    state.db(),
                    &author_account,
                    &viewer_account_id,
                    BlockScope::Comments,
                ).await.unwrap_or(false);
                if !viewer_blocks && !author_blocks {
                    visible.push(post);
                }
            } else {
                // Author not found (deleted?) — include post to be safe
                visible.push(post);
            }
        } else {
            // Invalid pseud ID — include post
            visible.push(post);
        }
    }
    Ok(visible)
}

async fn lock_topic(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let _ = pseud_id;
    let updated = lorehaven_db::community::toggle_topic_lock(state.db(), &id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    if !updated {
        return Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "topic",
        }));
    }
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
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Query(params): Query<CursorQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let items = lorehaven_db::community::list_groups(
        state.db(),
        &user.account_id.to_string(),
        params.limit,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "items": items })))
}

async fn post_group(
    State(state): State<AppState>,
    RequirePseud { user, .. }: RequirePseud,
    Json(body): Json<CreateGroupBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let id = lorehaven_db::community::create_group(
        state.db(),
        &body.name,
        &body.privacy,
        &user.account_id.to_string(),
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "id": id })))
}

async fn get_group(
    State(state): State<AppState>,
    RequireSession(_): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let group = lorehaven_db::community::group_by_id(state.db(), &id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    match group {
        Some(g) => Ok(Json(serde_json::json!({ "group": g }))),
        None => Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "group",
        })),
    }
}

async fn join_group(
    State(state): State<AppState>,
    RequirePseud { user, .. }: RequirePseud,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    lorehaven_db::community::add_group_member(
        state.db(),
        &id,
        &user.account_id.to_string(),
        "member",
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn leave_group(
    State(state): State<AppState>,
    RequirePseud { user, .. }: RequirePseud,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let removed =
        lorehaven_db::community::remove_group_member(state.db(), &id, &user.account_id.to_string())
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    if !removed {
        return Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "group_member",
        }));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn put_role(
    State(state): State<AppState>,
    RequirePseud { .. }: RequirePseud,
    Path(id): Path<String>,
    Json(body): Json<UpdateRoleBody>,
) -> ApiResult<StatusCode> {
    let updated =
        lorehaven_db::community::update_member_role(state.db(), &id, &body.account, &body.role)
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    if !updated {
        return Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "group_member",
        }));
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

async fn get_conversations(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let items =
        lorehaven_db::community::list_conversations(state.db(), &user.account_id.to_string())
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "items": items })))
}

async fn post_conversation(
    State(state): State<AppState>,
    RequirePseud { user, .. }: RequirePseud,
    Json(body): Json<CreateConversationBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let conversation_id = lorehaven_db::community::create_conversation(state.db())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    lorehaven_db::community::add_participant(
        state.db(),
        &conversation_id,
        &user.account_id.to_string(),
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    lorehaven_db::community::add_participant(state.db(), &conversation_id, &body.participant)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "id": conversation_id })))
}

async fn get_conv_messages(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Query(params): Query<CursorQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let items = lorehaven_db::community::list_messages(
        state.db(),
        &id,
        &user.account_id.to_string(),
        params.cursor.as_deref(),
        params.limit,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "items": items })))
}

async fn post_message(
    State(state): State<AppState>,
    RequirePseud { user, .. }: RequirePseud,
    Path(id): Path<String>,
    Json(body): Json<SendMessageBody>,
) -> ApiResult<Json<serde_json::Value>> {
    if body.body.trim().is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "body",
            "A message needs some words.",
        )));
    }
    // Block-aware: check if the sender is blocked by any participant
    let participants = lorehaven_db::community::conversation_participants(state.db(), &id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    for participant in &participants {
        if participant != &user.account_id.to_string() {
            let blocked = lorehaven_db::community::is_blocked(
                state.db(),
                participant,
                &user.account_id.to_string(),
                BlockScope::Messages,
            )
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
            if blocked {
                return Err(ApiError(lorehaven_domain::AppError::field(
                    "participant",
                    "Cannot send to this participant.",
                )));
            }
        }
    }
    let id = lorehaven_db::community::send_message(
        state.db(),
        &id,
        &user.account_id.to_string(),
        &body.body,
    )
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

async fn get_blocks(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let items = lorehaven_db::community::list_blocks(state.db(), &user.account_id.to_string())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "items": items })))
}

async fn post_block(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<BlockBody>,
) -> ApiResult<StatusCode> {
    let scope = BlockScope::parse(&body.scope).ok_or_else(|| {
        ApiError(lorehaven_domain::AppError::field(
            "scope",
            "Invalid block scope.",
        ))
    })?;
    lorehaven_db::community::insert_block(
        state.db(),
        &user.account_id.to_string(),
        &body.blocked,
        scope,
        body.note.as_deref(),
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_block(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let removed =
        lorehaven_db::community::delete_block(state.db(), &user.account_id.to_string(), &id)
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    if !removed {
        return Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "block",
        }));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn get_mutes(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let items = lorehaven_db::community::list_mutes(state.db(), &user.account_id.to_string())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "items": items })))
}

async fn post_mute(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<MuteBody>,
) -> ApiResult<StatusCode> {
    lorehaven_db::community::insert_mute(
        state.db(),
        &user.account_id.to_string(),
        &body.muted,
        body.until.as_deref(),
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_mute(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let removed =
        lorehaven_db::community::delete_mute(state.db(), &user.account_id.to_string(), &id)
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    if !removed {
        return Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "mute",
        }));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Presence
// ---------------------------------------------------------------------------

async fn get_presence_stream(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    use lorehaven_db::identity::format_rfc3339;

    // Upsert the viewer's presence record so they appear in the stream.
    let account_id = user.account_id.to_string();
    let now = format_rfc3339(OffsetDateTime::now_utc());
    lorehaven_db::community::upsert_presence(state.db(), &account_id, &now, None::<&str>, true)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;

    // Fetch all presence records and return as JSON items.
    let rows = lorehaven_db::community::list_presence(state.db())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;

    let items: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(account, last_seen_at, typing_until, enabled)| {
            let active_now = is_active_now(&last_seen_at);
            serde_json::json!({
                "account": account,
                "active_now": active_now,
                "typing": typing_until.as_ref().is_some_and(|t| {
                    parse_datetime(t).is_some_and(|dt| dt > OffsetDateTime::now_utc())
                }),
                "last_seen_at": last_seen_at,
                "enabled": enabled,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "items": items })))
}

// ---------------------------------------------------------------------------
// Topic subscriptions & unread tracking (spec §17.4, §5.5)
// ---------------------------------------------------------------------------

/// Subscribe the current user to a topic.
async fn subscribe_topic(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    lorehaven_db::community::subscribe_to_topic(
        state.db(),
        &user.account_id.to_string(),
        &id,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Unsubscribe the current user from a topic.
async fn unsubscribe_topic(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let removed = lorehaven_db::community::unsubscribe_from_topic(
        state.db(),
        &user.account_id.to_string(),
        &id,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "ok": true, "removed": removed })))
}

/// Mark a topic as read up to a specific post (or the latest post if none given).
async fn mark_topic_read(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    body: Option<Json<MarkReadBody>>,
) -> ApiResult<Json<serde_json::Value>> {
    let post_id = body
        .as_ref()
        .and_then(|b| b.post_id.clone())
        .unwrap_or_default();
    lorehaven_db::community::mark_topic_read(
        state.db(),
        &user.account_id.to_string(),
        &id,
        &post_id,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
pub struct MarkReadBody {
    pub post_id: Option<String>,
}

/// Get the number of unread posts in a topic for the current user.
async fn get_unread_count(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let count = lorehaven_db::community::unread_count_in_topic(
        state.db(),
        &user.account_id.to_string(),
        &id,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "unread": count })))
}

/// Check if a presence record is "active now" (last seen within 5 minutes).
fn is_active_now(last_seen_at: &str) -> bool {
    parse_datetime(last_seen_at).is_some_and(|dt| {
        let elapsed = OffsetDateTime::now_utc() - dt;
        elapsed < Duration::minutes(5)
    })
}

/// Parse an RFC 3339 datetime string.
fn parse_datetime(s: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(s, &Rfc3339).ok()
}

// ---------------------------------------------------------------------------
// Forum search (spec §17.4)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ForumSearchQuery {
    pub q: String,
    pub category: Option<String>,
    pub author: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    #[serde(default = "default_search_limit")]
    pub limit: i64,
}

fn default_search_limit() -> i64 {
    20
}

/// Full-text search across forum posts and topics.
async fn forum_search(
    State(state): State<AppState>,
    MaybeSession(_session): MaybeSession,
    Query(params): Query<ForumSearchQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let results = lorehaven_db::forum_search::search_forum(
        state.db(),
        &params.q,
        params.category.as_deref(),
        params.author.as_deref(),
        params.from.as_deref(),
        params.to.as_deref(),
        params.limit,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;

    Ok(Json(serde_json::json!({ "items": results })))
}

// ---------------------------------------------------------------------------
