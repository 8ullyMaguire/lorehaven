use crate::auth::{MaybeSession, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use lorehaven_domain::media_resilience::VerificationType;
use lorehaven_domain::AppError;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use uuid::Uuid;

use lorehaven_db::media_resilience;

#[derive(Debug, Deserialize)]
pub struct VerifyLinkBody {
    pub availability_link_id: String,
    pub verification_type: String,
    pub confidence: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct CuratorBountyQuery {
    pub media_reference_id: String,
}

fn validation_err(message: &str) -> ApiError {
    ApiError(AppError::Validation {
        message: message.into(),
        field_errors: BTreeMap::new(),
    })
}

/// Opt the current user into the media curator role (spec §32.7.5).
pub async fn opt_in_curator(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let account_id = user.account_id.to_string();
    media_resilience::opt_in_curator(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok((StatusCode::CREATED, Json(json!({ "status": "opted_in" }))))
}

/// Opt the current user out of the media curator role.
pub async fn opt_out_curator(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let account_id = user.account_id.to_string();
    media_resilience::opt_out_curator(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok((StatusCode::OK, Json(json!({ "status": "opted_out" }))))
}

/// Check whether the current user is an active curator.
pub async fn get_my_curator_status(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    let account_id = user.account_id.to_string();
    let is_curator = media_resilience::is_active_curator(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(json!({ "is_curator": is_curator })))
}

/// List all active curators (public).
pub async fn list_curators(
    State(state): State<AppState>,
    MaybeSession(_session): MaybeSession,
) -> ApiResult<Json<Value>> {
    let curators = media_resilience::list_active_curators(state.db())
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(json!({ "curators": curators })))
}

/// Verify an availability link (curator action, contributes to quorum).
pub async fn verify_link(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(reference_id): Path<String>,
    Json(body): Json<VerifyLinkBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let curator_id = user.account_id.to_string();

    // Cannot verify your own added links — anti-gaming check
    let links = media_resilience::find_availability_links_for_reference(state.db(), &reference_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    let link = links.iter().find(|l| l.id == body.availability_link_id);
    let Some(link) = link else {
        return Err(ApiError(AppError::NotFound {
            resource: "availability_link",
        }));
    };
    if link.added_by.as_deref() == Some(&curator_id) {
        return Err(validation_err(
            "curators cannot verify links they added themselves",
        ));
    }

    // Check if curator already verified this link
    let already = media_resilience::curator_verified_link(
        state.db(),
        &curator_id,
        &body.availability_link_id,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    if already {
        return Err(validation_err("you have already verified this link"));
    }

    // Parse verification type
    let verification_type = match body.verification_type.as_str() {
        "exact_match" => VerificationType::ExactMatch,
        "perceptual_match" => VerificationType::PerceptualMatch,
        "reverify" => VerificationType::Reverify,
        _ => return Err(validation_err("invalid verification_type")),
    };

    let confidence = body.confidence.unwrap_or(1.0).clamp(0.0, 1.0);

    let id = Uuid::new_v4().to_string();
    media_resilience::record_link_verification(
        state.db(),
        &id,
        &body.availability_link_id,
        &reference_id,
        &curator_id,
        verification_type,
        confidence,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    let verifier_count =
        media_resilience::count_link_verifiers(state.db(), &body.availability_link_id)
            .await
            .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    let quorum = verifier_count >= 2;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "status": "verified",
            "verifier_count": verifier_count,
            "quorum_reached": quorum,
        })),
    ))
}

/// Check quorum status for a media reference's links.
pub async fn get_quorum_status(
    State(state): State<AppState>,
    MaybeSession(_session): MaybeSession,
    Path(reference_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let links = media_resilience::find_availability_links_for_reference(state.db(), &reference_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;

    let mut link_statuses = Vec::new();
    for link in &links {
        let count = media_resilience::count_link_verifiers(state.db(), &link.id)
            .await
            .map_err(|e| ApiError(AppError::Internal(e.into())))?;
        link_statuses.push(json!({
            "link_id": link.id,
            "url": link.url,
            "verifier_count": count,
            "quorum_reached": count >= 2,
        }));
    }

    Ok(Json(
        json!({ "reference_id": reference_id, "links": link_statuses }),
    ))
}

/// Find standing bounties matching a media reference.
pub async fn get_matching_bounties(
    State(state): State<AppState>,
    MaybeSession(_session): MaybeSession,
    Query(query): Query<CuratorBountyQuery>,
) -> ApiResult<Json<Value>> {
    let healthy_count =
        media_resilience::count_healthy_links(state.db(), &query.media_reference_id)
            .await
            .map_err(|e| ApiError(AppError::Internal(e)))?;

    let links = media_resilience::find_availability_links_for_reference(
        state.db(),
        &query.media_reference_id,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e)))?;
    let has_archive = links.iter().any(|l| {
        l.provider == lorehaven_domain::media_resilience::LinkProvider::InternetArchive.as_str()
            && l.status == lorehaven_domain::media_resilience::LinkStatus::Healthy.as_str()
    });

    let bounties = media_resilience::find_matching_standing_bounties(
        state.db(),
        &query.media_reference_id,
        healthy_count,
        0,
        has_archive,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    let bounties_json: Vec<Value> = bounties
        .iter()
        .map(|b| {
            json!({
                "bounty_id": b.bounty_id,
                "name": b.name,
                "reward": b.reward,
                "provider": b.provider,
                "healthy_links_below": b.healthy_links_below,
            })
        })
        .collect();

    Ok(Json(json!({
        "media_reference_id": query.media_reference_id,
        "healthy_links": healthy_count,
        "has_archive_link": has_archive,
        "matching_bounties": bounties_json,
    })))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/curators/opt-in", post(opt_in_curator))
        .route("/curators/opt-out", post(opt_out_curator))
        .route("/curators/status", get(get_my_curator_status))
        .route("/curators", get(list_curators))
        .route("/media/references/{reference_id}/verify", post(verify_link))
        .route(
            "/media/references/{reference_id}/quorum",
            get(get_quorum_status),
        )
        .route("/curator/bounties", get(get_matching_bounties))
}
