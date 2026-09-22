use crate::auth::{MaybeSession, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::Json;
use lorehaven_domain::AppError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/admin/local-mirrors", post(add_local_mirror))
        .route("/admin/local-mirrors/{mirror_id}", delete(deactivate_local_mirror))
        .route("/media/references/{reference_id}/mirrors", get(list_local_mirrors))
        .route("/admin/dmca-takedowns", post(file_dmca_takedown).get(list_dmca_takedowns))
        .route("/admin/dmca-takedowns/{takedown_id}", put(resolve_dmca_takedown))
        .route("/media/references/{reference_id}/ipfs-pins", get(list_ipfs_pins).post(add_ipfs_pin))
}

// ---------------------------------------------------------------------------
// Trust level helper
// ---------------------------------------------------------------------------

async fn require_operator(state: &AppState, user: &crate::auth::SessionUser) -> Result<(), ApiError> {
    let level = lorehaven_db::governance::trust_for(state.db(), &user.account_id.to_string())
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    if level >= 5 {
        Ok(())
    } else {
        Err(ApiError(AppError::AuthRequired))
    }
}

// ---------------------------------------------------------------------------
// Local mirror management
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AddLocalMirrorPayload {
    pub media_reference_id: String,
    pub storage_path: String,
    pub original_url: String,
    pub file_size_bytes: u64,
    pub content_type: String,
    pub checksum_sha256: String,
}

pub async fn add_local_mirror(State(state): State<AppState>, RequireSession(user): RequireSession, Json(payload): Json<AddLocalMirrorPayload>) -> ApiResult<impl axum::response::IntoResponse> {
    require_operator(&state, &user).await?;
    let id = Uuid::new_v4().to_string();
    let mirrored_by = user.account_id.to_string();
    lorehaven_db::media_resilience::insert_local_mirror(
        state.db(), &id, &payload.media_reference_id, &payload.storage_path,
        &payload.original_url, payload.file_size_bytes as i64, &payload.content_type,
        &payload.checksum_sha256, &mirrored_by,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id }))))
}

#[derive(Debug, Serialize)]
pub struct LocalMirrorView {
    pub id: String,
    pub media_reference_id: String,
    pub storage_path: String,
    pub original_url: String,
    pub file_size_bytes: u64,
    pub content_type: String,
    pub checksum_sha256: String,
    pub mirrored_by: String,
    pub status: String,
    pub mirrored_at: String,
}

pub async fn list_local_mirrors(State(state): State<AppState>, Path(reference_id): Path<String>, _session: MaybeSession) -> ApiResult<Json<Vec<LocalMirrorView>>> {
    let mirrors = lorehaven_db::media_resilience::list_local_mirrors(state.db(), &reference_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    let views: Vec<LocalMirrorView> = mirrors.iter().map(|m| LocalMirrorView {
        id: m.id.clone(), media_reference_id: m.media_reference_id.clone(),
        storage_path: m.storage_path.clone(), original_url: m.original_url.clone(),
        file_size_bytes: m.file_size_bytes as u64, content_type: m.content_type.clone(),
        checksum_sha256: m.checksum_sha256.clone(), mirrored_by: m.mirrored_by.clone(),
        status: m.status.clone(), mirrored_at: m.mirrored_at.clone(),
    }).collect();
    Ok(Json(views))
}

pub async fn deactivate_local_mirror(State(state): State<AppState>, RequireSession(user): RequireSession, Path(mirror_id): Path<String>) -> ApiResult<StatusCode> {
    require_operator(&state, &user).await?;
    lorehaven_db::media_resilience::deactivate_local_mirror(state.db(), &mirror_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// IPFS pin management
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AddIpfsPinPayload {
    pub media_reference_id: String,
    pub cid: String,
    pub pin_service: String,
    pub file_size_bytes: u64,
}

pub async fn add_ipfs_pin(State(state): State<AppState>, RequireSession(user): RequireSession, Json(payload): Json<AddIpfsPinPayload>) -> ApiResult<impl axum::response::IntoResponse> {
    require_operator(&state, &user).await?;
    let id = Uuid::new_v4().to_string();
    lorehaven_db::media_resilience::insert_ipfs_pin(
        state.db(), &id, &payload.media_reference_id, &payload.cid,
        &payload.pin_service, payload.file_size_bytes as i64,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id }))))
}

#[derive(Debug, Serialize)]
pub struct IpfsPinView {
    pub id: String,
    pub media_reference_id: String,
    pub cid: String,
    pub pin_service: String,
    pub status: String,
    pub file_size_bytes: u64,
    pub pinned_at: String,
}

pub async fn list_ipfs_pins(State(state): State<AppState>, Path(reference_id): Path<String>, _session: MaybeSession) -> ApiResult<Json<Vec<IpfsPinView>>> {
    let pins = lorehaven_db::media_resilience::list_ipfs_pins(state.db(), &reference_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    let views: Vec<IpfsPinView> = pins.iter().map(|p| IpfsPinView {
        id: p.id.clone(), media_reference_id: p.media_reference_id.clone(),
        cid: p.cid.clone(), pin_service: p.pin_service.clone(),
        status: p.status.clone(), file_size_bytes: p.file_size_bytes as u64,
        pinned_at: p.pinned_at.clone(),
    }).collect();
    Ok(Json(views))
}

// ---------------------------------------------------------------------------
// DMCA takedowns
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct FileDmcaPayload {
    pub local_mirror_id: String,
    pub claimant_name: String,
    pub claimant_email: String,
    pub original_work_description: String,
    pub complaint_text: String,
}

#[derive(Debug, Deserialize)]
pub struct ResolveDmcaPayload {
    pub approved: bool,
}

pub async fn file_dmca_takedown(State(state): State<AppState>, _session: MaybeSession, Json(payload): Json<FileDmcaPayload>) -> ApiResult<impl axum::response::IntoResponse> {
    let id = Uuid::new_v4().to_string();
    lorehaven_db::media_resilience::file_dmca_takedown(
        state.db(), &id, &payload.local_mirror_id, &payload.claimant_name,
        &payload.claimant_email, &payload.original_work_description, &payload.complaint_text,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id }))))
}

pub async fn list_dmca_takedowns(State(state): State<AppState>, RequireSession(user): RequireSession) -> ApiResult<Json<serde_json::Value>> {
    require_operator(&state, &user).await?;
    Ok(Json(serde_json::json!({ "takedowns": [] })))
}

pub async fn resolve_dmca_takedown(State(state): State<AppState>, RequireSession(user): RequireSession, Path(takedown_id): Path<String>, Json(payload): Json<ResolveDmcaPayload>) -> ApiResult<StatusCode> {
    require_operator(&state, &user).await?;
    let resolved_by = user.account_id.to_string();
    lorehaven_db::media_resilience::resolve_dmca_takedown(
        state.db(), &takedown_id, payload.approved, &resolved_by,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(StatusCode::NO_CONTENT)
}
