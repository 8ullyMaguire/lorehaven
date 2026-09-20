//! Moderation and community health routes (spec §35.5, repo M35).
//!
//! Covers: sanctions (graduated response ladder), slow mode, federation scope,
//! featured posts, zero-result search tracking, activity sparklines.

use axum::extract::{Path, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use lorehaven_domain::moderation::SanctionLevel;
use lorehaven_domain::typed_votes::is_moderator;

use crate::auth::RequirePseud;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use lorehaven_db::moderation;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/mod/sanctions", post(post_sanction))
        .route("/mod/sanctions/check", get(get_sanction_check))
        .route("/topics/{id}/slow-mode", put(put_slow_mode))
        .route("/topics/{id}/federation-scope", put(put_federation_scope))
        .route("/posts/{id}/feature", post(post_feature))
        .route("/forum/health", get(get_health))
}

async fn moderator_check(state: &AppState, pseud_id: &str) -> Result<bool, ApiError> {
    let trust = lorehaven_db::governance::trust_for(state.db(), pseud_id)
        .await
        .map_err(|e| internal(e.into()))?;
    Ok(is_moderator(trust))
}

// ---------------------------------------------------------------------------
// Sanctions
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct SanctionBody {
    account: String,
    category_id: Option<String>,
    level: String,
    reason: String,
    expires_at: Option<String>,
}

async fn post_sanction(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Json(body): Json<SanctionBody>,
) -> ApiResult<Json<serde_json::Value>> {
    if !moderator_check(&state, &pseud_id.to_string()).await? {
        return Err(ApiError(lorehaven_domain::AppError::AccessDenied));
    }
    let level = SanctionLevel::from_str(&body.level)
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::field("level", "unknown sanction level")))?;
    let id = moderation::apply_sanction(
        state.db(),
        &body.account,
        body.category_id.as_deref(),
        level,
        &body.reason,
        &pseud_id.to_string(),
        body.expires_at.as_deref(),
    )
    .await
    .map_err(|e| internal(e.into()))?;
    Ok(Json(json!({ "id": id, "applied": true })))
}

#[derive(Debug, Deserialize)]
struct SanctionCheckQuery {
    account: String,
    category_id: Option<String>,
}

async fn get_sanction_check(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<SanctionCheckQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let sanction = moderation::check_sanction(
        state.db(),
        &query.account,
        query.category_id.as_deref(),
    )
    .await
    .map_err(|e| internal(e.into()))?;
    Ok(Json(match sanction {
        Some(s) => json!({
            "active": true,
            "level": s.level,
            "expires_at": s.expires_at,
        }),
        None => json!({ "active": false }),
    }))
}

// ---------------------------------------------------------------------------
// Topic settings
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct SlowModeBody {
    seconds: i64,
}

async fn put_slow_mode(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
    Json(body): Json<SlowModeBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let topic = lorehaven_db::community::topic_by_id(state.db(), &id)
        .await
        .map_err(|e| internal(e.into()))?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "topic" }))?;
    if topic.author_pseud != pseud_id.to_string()
        && !moderator_check(&state, &pseud_id.to_string()).await?
    {
        return Err(ApiError(lorehaven_domain::AppError::AccessDenied));
    }
    moderation::set_slow_mode(state.db(), &id, body.seconds)
        .await
        .map_err(|e| internal(e.into()))?;
    Ok(Json(json!({ "slow_mode_seconds": body.seconds })))
}

#[derive(Debug, Deserialize)]
struct FederationScopeBody {
    scope: String,
}

async fn put_federation_scope(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
    Json(body): Json<FederationScopeBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let scope = lorehaven_domain::moderation::FederationScope::from_str(&body.scope)
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::field("scope", "must be 'public', 'local', or 'unlisted'")))?;
    let topic = lorehaven_db::community::topic_by_id(state.db(), &id)
        .await
        .map_err(|e| internal(e.into()))?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "topic" }))?;
    if topic.author_pseud != pseud_id.to_string()
        && !moderator_check(&state, &pseud_id.to_string()).await?
    {
        return Err(ApiError(lorehaven_domain::AppError::AccessDenied));
    }
    moderation::set_federation_scope(state.db(), &id, scope.as_str())
        .await
        .map_err(|e| internal(e.into()))?;
    Ok(Json(json!({ "federation_scope": scope.as_str() })))
}

// ---------------------------------------------------------------------------
// Featured posts
// ---------------------------------------------------------------------------

async fn post_feature(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    if !moderator_check(&state, &pseud_id.to_string()).await? {
        return Err(ApiError(lorehaven_domain::AppError::AccessDenied));
    }
    moderation::feature_post(state.db(), &id, &pseud_id.to_string())
        .await
        .map_err(|e| internal(e.into()))?;
    Ok(Json(json!({ "featured": true })))
}

// ---------------------------------------------------------------------------
// Community health
// ---------------------------------------------------------------------------

async fn get_health(State(state): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    Ok(Json(json!({ "status": "ok" })))
}

fn internal(e: anyhow::Error) -> ApiError {
    ApiError(lorehaven_domain::AppError::Internal(e))
}
