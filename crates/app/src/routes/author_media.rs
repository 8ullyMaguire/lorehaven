use crate::auth::{MaybeSession, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use lorehaven_domain::AppError;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use uuid::Uuid;

use lorehaven_db::media_resilience;

#[derive(Debug, Deserialize)]
pub struct PreferencesBody {
    pub auto_submit_to_archive: Option<bool>,
    pub prefer_curator_verified: Option<bool>,
    pub broken_link_notifications: Option<String>,
    pub allow_curator_edits: Option<bool>,
    pub minimum_healthy_links: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct TargetedBountyBody {
    pub work_id: String,
    pub chapter_id: Option<String>,
    pub media_reference_id: Option<String>,
    pub reward: i64,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ClaimBountyBody {
    pub bounty_id: String,
}

fn validation_err(message: &str) -> ApiError {
    ApiError(AppError::Validation {
        message: message.into(),
        field_errors: BTreeMap::new(),
    })
}

/// Get the current user's media preferences.
pub async fn get_preferences(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    let account_id = user.account_id.to_string();
    let prefs = media_resilience::get_author_preferences(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(json!({
        "account_id": prefs.account_id,
        "auto_submit_to_archive": prefs.auto_submit_to_archive,
        "prefer_curator_verified": prefs.prefer_curator_verified,
        "broken_link_notifications": prefs.broken_link_notifications,
        "allow_curator_edits": prefs.allow_curator_edits,
        "minimum_healthy_links": prefs.minimum_healthy_links,
    })))
}

/// Update the current user's media preferences.
pub async fn update_preferences(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<PreferencesBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let account_id = user.account_id.to_string();

    // Defaults match the migration defaults
    let auto_submit = body.auto_submit_to_archive.unwrap_or(true);
    let prefer_verified = body.prefer_curator_verified.unwrap_or(true);
    let notifications = body
        .broken_link_notifications
        .as_deref()
        .unwrap_or("digest_weekly");
    let allow_edits = body.allow_curator_edits.unwrap_or(true);
    let min_healthy = body.minimum_healthy_links.unwrap_or(3);

    if min_healthy < 0 {
        return Err(validation_err("minimum_healthy_links must be non-negative"));
    }

    media_resilience::upsert_author_preferences(
        state.db(),
        &account_id,
        auto_submit,
        prefer_verified,
        notifications,
        allow_edits,
        min_healthy,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    Ok((StatusCode::OK, Json(json!({ "status": "updated" }))))
}

/// Post a targeted bounty for a specific work or media reference.
pub async fn post_targeted_bounty(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<TargetedBountyBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    if body.reward <= 0 {
        return Err(validation_err("reward must be positive"));
    }

    let bounty_id = Uuid::new_v4().to_string();
    let account_id = user.account_id.to_string();

    media_resilience::post_targeted_bounty(
        state.db(),
        &bounty_id,
        &body.work_id,
        body.chapter_id.as_deref(),
        body.media_reference_id.as_deref(),
        &account_id,
        body.reward,
        body.description.as_deref(),
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    Ok((
        StatusCode::CREATED,
        Json(json!({ "bounty_id": bounty_id, "status": "open" })),
    ))
}

/// Claim a targeted bounty.
pub async fn claim_targeted_bounty(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<ClaimBountyBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let claimant = user.account_id.to_string();
    let claimed = media_resilience::claim_targeted_bounty(state.db(), &body.bounty_id, &claimant)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    if !claimed {
        return Err(validation_err("bounty not found or already claimed"));
    }

    Ok((StatusCode::OK, Json(json!({ "status": "claimed" }))))
}

/// List targeted bounties for a work.
pub async fn list_targeted_bounties(
    State(state): State<AppState>,
    MaybeSession(_session): MaybeSession,
    Path(work_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let bounties = media_resilience::list_targeted_bounties_for_work(state.db(), &work_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    let bounties_json: Vec<Value> = bounties
        .iter()
        .map(|b| {
            json!({
                "bounty_id": b.id,
                "work_id": b.work_id,
                "chapter_id": b.chapter_id,
                "media_reference_id": b.media_reference_id,
                "reward": b.reward,
                "status": b.status,
                "description": b.description,
                "claimed_by": b.claimed_by,
                "created_at": b.created_at,
            })
        })
        .collect();

    Ok(Json(
        json!({ "work_id": work_id, "bounties": bounties_json }),
    ))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/author/media-preferences",
            get(get_preferences).put(update_preferences),
        )
        .route("/author/media-health", get(get_author_media_health))
        .route("/author/targeted-bounties", post(post_targeted_bounty))
        .route(
            "/author/targeted-bounties/claim",
            post(claim_targeted_bounty),
        )
        .route(
            "/works/{work_id}/targeted-bounties",
            get(list_targeted_bounties),
        )
}

/// Get the current user's per-work media health report (§32.7.8).
pub async fn get_author_media_health(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    let account_id = user.account_id.to_string();
    let report = media_resilience::author_media_health_report(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    Ok(Json(json!({ "items": report })))
}
