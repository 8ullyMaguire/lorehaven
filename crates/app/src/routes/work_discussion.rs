//! Work discussion routes: modes, linked topics, reactions, migration tool.
//!
//! Spec §35.0–35.1, repo M31. The reaction bar and Discuss link live on the
//! work page; the routes here serve both and enforce the mode rules.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use lorehaven_domain::work_discussion::{is_valid_work_reaction, WorkDiscussionMode};

use crate::auth::{RequirePseud, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use lorehaven_db::work_backlink::LinkedWork;

pub fn router() -> Router<AppState> {
    Router::new()
        // The effective mode for a work, and the author's override.
        .route(
            "/works/{id}/discussion-mode",
            get(get_discussion_mode).put(put_discussion_mode),
        )
        // The typed-vote reaction bar.
        .route(
            "/works/{id}/reactions",
            get(get_reactions).post(post_reaction),
        )
        // The linked forum topic ("Discuss" link target).
        .route("/works/{id}/thread", get(get_thread))
        // The work linked to a topic (backlink card on the topic page).
        .route("/topics/{id}/work", get(get_linked_work))
        // The comment-to-topic batch migration tool (author only).
        .route("/works/{id}/migrate-comments", post(post_migrate_comments))
}

/// The effective discussion mode for a work.
async fn get_discussion_mode(
    State(state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let stored = lorehaven_db::work_discussion::work_discussion_mode(state.db(), &id)
        .await
        .map_err(internal)?;
    let effective = lorehaven_domain::work_discussion::resolve_discussion_mode(
        stored,
        state.config().forum.work_discussion_default,
    );
    Ok(Json(json!({
        "mode": effective.as_str(),
        "comments_enabled": effective.comments_enabled(),
        "thread_enabled": effective.thread_enabled(),
    })))
}

#[derive(Debug, Deserialize)]
struct PutModeBody {
    mode: String,
}

/// Set the discussion mode. Author (work contributor) only.
async fn put_discussion_mode(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
    Json(body): Json<PutModeBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let mode = WorkDiscussionMode::parse(&body.mode).ok_or_else(|| {
        ApiError(lorehaven_domain::AppError::field(
            "mode",
            "Expected thread_only, comments_only, or both.",
        ))
    })?;

    let work_id: lorehaven_domain::WorkId = id
        .parse()
        .map_err(|_| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;

    // Only a contributor may change how discussion around their work happens.
    let contributors = lorehaven_db::collaboration::contributors_for_work(state.db(), work_id)
        .await
        .map_err(internal)?;
    if !contributors.iter().any(|c| c.pseud_id == pseud_id) {
        return Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "work",
        }));
    }

    let updated = lorehaven_db::work_discussion::set_work_discussion_mode(state.db(), &id, mode)
        .await
        .map_err(internal)?;
    if !updated {
        return Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "work",
        }));
    }
    Ok(Json(json!({ "mode": mode.as_str() })))
}

/// Aggregate reaction counts (public) plus the caller's own vote.
async fn get_reactions(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let counts = lorehaven_db::work_discussion::reaction_counts(state.db(), &id)
        .await
        .map_err(internal)?;
    let mine = match user.pseud_id {
        Some(pseud) => {
            lorehaven_db::work_discussion::reaction_by(state.db(), &id, &pseud.to_string())
                .await
                .map_err(internal)?
        }
        None => None,
    };
    Ok(Json(json!({
        "counts": counts,
        "mine": mine,
        "types": lorehaven_domain::work_discussion::WORK_REACTION_TYPES,
    })))
}

#[derive(Debug, Deserialize)]
struct PostReactionBody {
    /// A vote type from the allowed set, or absent/null to retract.
    vote_type: Option<String>,
}

/// Cast, change, or retract the caller's reaction.
async fn post_reaction(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
    Json(body): Json<PostReactionBody>,
) -> ApiResult<Json<serde_json::Value>> {
    if let Some(vt) = &body.vote_type {
        if !is_valid_work_reaction(vt) {
            return Err(ApiError(lorehaven_domain::AppError::field(
                "vote_type",
                "Not one of the reaction types this surface offers.",
            )));
        }
    }
    let outcome = lorehaven_db::work_discussion::set_reaction(
        state.db(),
        &id,
        &pseud_id.to_string(),
        body.vote_type.as_deref(),
    )
    .await
    .map_err(internal)?;
    let outcome_str = match outcome {
        lorehaven_domain::work_discussion::ReactionOutcome::Cast => "cast",
        lorehaven_domain::work_discussion::ReactionOutcome::Changed => "changed",
        lorehaven_domain::work_discussion::ReactionOutcome::Retracted => "retracted",
    };
    Ok(Json(json!({ "outcome": outcome_str })))
}

/// The work's linked discussion topic, if one exists.
async fn get_thread(
    State(state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let link = lorehaven_db::work_discussion::linked_topic(state.db(), &id)
        .await
        .map_err(internal)?;
    match link {
        Some(l) => Ok(Json(json!({
            "topic_id": l.topic_id,
            "chapter_id": l.chapter_id,
        }))),
        None => Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "thread",
        })),
    }
}

/// The comment-to-topic migration tool. Author (contributor) only.
async fn post_migrate_comments(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let work_id: lorehaven_domain::WorkId = id
        .parse()
        .map_err(|_| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;

    let contributors = lorehaven_db::collaboration::contributors_for_work(state.db(), work_id)
        .await
        .map_err(internal)?;
    if !contributors.iter().any(|c| c.pseud_id == pseud_id) {
        return Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "work",
        }));
    }

    // The work's own title names the topic; the author pseud is the caller's.
    let work = lorehaven_db::content::find_work(state.db(), work_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;

    // Linked topics live in a dedicated category so they are findable; the
    // first category is the instance's own fallback when none is seeded.
    let categories = lorehaven_db::community::list_forum_categories(state.db())
        .await
        .map_err(internal)?;
    let category_id = categories.first().map(|c| c.id.clone()).ok_or_else(|| {
        ApiError(lorehaven_domain::AppError::Internal(anyhow::anyhow!(
            "no forum category exists; seed one before migrating"
        )))
    })?;

    let report = lorehaven_db::work_discussion::migrate_comments_to_topic(
        state.db(),
        &id,
        &category_id,
        &pseud_id.to_string(),
        &format!("Discussion: {}", work.title),
    )
    .await
    .map_err(internal)?;
    Ok(Json(json!({
        "topic_id": report.topic_id,
        "moved": report.moved,
    })))
}

fn internal(e: anyhow::Error) -> ApiError {
    ApiError(lorehaven_domain::AppError::Internal(e))
}

/// The work linked to a topic — backlink card data for the topic page.
async fn get_linked_work(
    State(state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<LinkedWork>> {
    let work = lorehaven_db::work_backlink::work_for_topic(state.db(), &id)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;
    Ok(Json(work))
}
