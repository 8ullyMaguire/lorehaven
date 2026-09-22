use crate::auth::{MaybeSession, RequirePseud, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use lorehaven_domain::media_resilience::{LinkStatus, MediaContextKind};
use lorehaven_domain::AppError;
use serde::Deserialize;
use serde_json::{json, Value};
use lorehaven_db::media_resilience;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

fn link_to_json(row: &media_resilience::AvailabilityLinkRow) -> Value {
    json!({
        "id": row.id,
        "url": row.url,
        "provider": row.provider,
        "status": row.status,
        "priority": row.priority,
        "last_checked_at": row.last_checked_at,
        "last_healthy_at": row.last_healthy_at,
        "consecutive_failures": row.consecutive_failures,
    })
}

fn reference_to_json(
    row: &media_resilience::MediaReference,
    healthy_links: i64,
    links: &[media_resilience::AvailabilityLinkRow],
) -> Value {
    json!({
        "id": row.id,
        "media_kind": row.media_kind,
        "content_hash": row.content_hash,
        "perceptual_hash": row.perceptual_hash,
        "curator_verified": row.curator_verified,
        "healthy_links": healthy_links,
        "links": links.iter().map(link_to_json).collect::<Vec<_>>(),
    })
}

// ---------------------------------------------------------------------------
// Public routes
// ---------------------------------------------------------------------------

/// Get a media reference with its availability links.
pub async fn get_media_reference(
    State(state): State<AppState>,
    MaybeSession(_session): MaybeSession,
    Path(reference_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let Some(reference) = media_resilience::find_media_reference_by_id(state.db(), &reference_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?
    else {
        return Err(ApiError(AppError::NotFound { resource: "media_reference".into() }));
    };

    let links = media_resilience::find_availability_links_for_reference(state.db(), &reference.id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    let healthy = media_resilience::count_healthy_links(state.db(), &reference.id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    Ok(Json(reference_to_json(&reference, healthy, &links)))
}

/// Get media references for a work.
pub async fn get_work_media_references(
    State(state): State<AppState>,
    MaybeSession(_session): MaybeSession,
    Path(work_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let refs = media_resilience::find_work_media_references(state.db(), &work_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    Ok(Json(json!({ "references": refs })))
}

// ---------------------------------------------------------------------------
// Authenticated routes
// ---------------------------------------------------------------------------

/// Body for adding a media reference to a work.
#[derive(Debug, Deserialize)]
pub struct AddMediaReferenceBody {
    pub url: String,
    pub context: Option<String>,
    pub author_note: Option<String>,
    pub chapter_id: Option<String>,
}

pub async fn add_media_reference(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(work_id): Path<String>,
    Json(body): Json<AddMediaReferenceBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    // TODO: verify user can edit the work
    let context: MediaContextKind = body
        .context
        .as_deref()
        .and_then(|c| c.parse().ok())
        .unwrap_or(MediaContextKind::Reference);

    let link_id = Uuid::new_v4().to_string();
    let reference_id = Uuid::new_v4().to_string();
    let account_id = user.account_id.to_string();

    media_resilience::insert_media_reference(
        state.db(),
        &reference_id,
        "pending", // content hash computed async by the pipeline
        lorehaven_domain::media_resilience::MediaKind::Image,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    media_resilience::insert_availability_link(
        state.db(),
        &link_id,
        &reference_id,
        &body.url,
        lorehaven_domain::media_resilience::LinkProvider::Other,
        Some(&account_id),
        100,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    media_resilience::insert_work_media_reference(
        state.db(),
        &Uuid::new_v4().to_string(),
        &work_id,
        body.chapter_id.as_deref(),
        &reference_id,
        context,
        &body.url,
        body.author_note.as_deref(),
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    Ok((StatusCode::CREATED, Json(json!({
        "id": reference_id,
        "link_id": link_id,
        "status": "pending_verification"
    }))))
}

/// Report a broken link.
#[derive(Debug, Deserialize)]
pub struct ReportBrokenBody {
    pub reason: Option<String>,
}

pub async fn report_broken_link(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(link_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let links = media_resilience::find_links_needing_check(state.db(), 10000)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    let Some(link) = links.iter().find(|l| l.id == link_id) else {
        return Err(ApiError(AppError::NotFound { resource: "availability_link".into() }));
    };

    // Mark link as degraded pending verification
    media_resilience::update_link_status(
        state.db(),
        &link_id,
        LinkStatus::PendingVerification,
        link.consecutive_failures + 1,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    Ok(Json(json!({ "status": "reported" })))
}

// ---------------------------------------------------------------------------
// Curator routes
// ---------------------------------------------------------------------------

/// Body for adding an availability link (curator action).
#[derive(Debug, Deserialize)]
pub struct AddMirrorBody {
    pub url: String,
    pub provider: Option<String>,
    pub priority: Option<i64>,
}

pub async fn add_mirror_link(
    State(state): State<AppState>,
    RequirePseud { user, .. }: RequirePseud,
    Path(reference_id): Path<String>,
    Json(body): Json<AddMirrorBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    // TODO: check curator role
    let Some(_reference) = media_resilience::find_media_reference_by_id(state.db(), &reference_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?
    else {
        return Err(ApiError(AppError::NotFound { resource: "media_reference".into() }));
    };

    let provider: lorehaven_domain::media_resilience::LinkProvider = body
        .provider
        .as_deref()
        .and_then(|p| p.parse().ok())
        .unwrap_or(lorehaven_domain::media_resilience::LinkProvider::Other);

    let priority = body.priority.unwrap_or(50);

    let link_id = Uuid::new_v4().to_string();
    let account_id = user.account_id.to_string();

    media_resilience::insert_availability_link(
        state.db(),
        &link_id,
        &reference_id,
        &body.url,
        provider,
        Some(&account_id),
        priority,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    // Award curator credits for mirror add
    media_resilience::insert_curator_reward(
        state.db(),
        &account_id,
        lorehaven_domain::media_resilience::CuratorAction::MirrorAdd,
        Some(&reference_id),
        Some(&link_id),
        15,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    Ok((StatusCode::CREATED, Json(json!({
        "id": link_id,
        "status": "pending_verification",
        "credits_awarded": 15,
    }))))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/media/references/{reference_id}", get(get_media_reference))
        .route("/media/references/{reference_id}/report-broken", post(report_broken_link))
        .route("/works/{work_id}/media", get(get_work_media_references).post(add_media_reference))
        .route("/media/references/{reference_id}/mirrors", post(add_mirror_link))
}
