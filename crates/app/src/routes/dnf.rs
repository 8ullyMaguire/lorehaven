use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use lorehaven_domain::{dnf::DnfReason, AppError};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use uuid::Uuid;

use crate::auth::{MaybeSession, RequirePseud};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use lorehaven_domain::ids::{AccountId, PseudId, WorkId};
use lorehaven_db::dnf;

#[derive(Debug, Deserialize)]
struct DnfRequest {
    reason: String,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    is_public: Option<bool>,
}

#[derive(Debug, Serialize)]
struct DnfResponse {
    id: String,
    account_id: String,
    pseud_id: String,
    work_id: String,
    reason: String,
    note: Option<String>,
    is_public: bool,
    created_at: String,
    updated_at: String,
}

impl From<dnf::DnfRow> for DnfResponse {
    fn from(row: dnf::DnfRow) -> Self {
        Self {
            id: row.id,
            account_id: row.account_id,
            pseud_id: row.pseud_id,
            work_id: row.work_id,
            reason: row.reason,
            note: row.note,
            is_public: row.is_public,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/works/{id}/dnf", post(upsert_dnf).put(upsert_dnf).delete(delete_dnf).get(get_dnf))
        .route("/works/{id}/dnf/reasons", get(get_dnf_reasons))
        .route("/me/dnf", get(list_my_dnf))
}

#[derive(Debug, Serialize)]
struct DnfReasonsResponse {
    work_id: String,
    reasons: Vec<ReasonCount>,
}

#[derive(Debug, Serialize)]
struct ReasonCount {
    reason: String,
    count: i64,
}

/// Create or update a DNF record for a work.
async fn upsert_dnf(
    pseud: RequirePseud,
    Path(work_id): Path<Uuid>,
    State(state): State<AppState>,
    Json(req): Json<DnfRequest>,
) -> ApiResult<Json<DnfResponse>> {
    let db = state.db();

    let reason_str = DnfReason::from_str(&req.reason)
        .map_err(|e| ApiError(AppError::Validation {
            message: e,
            field_errors: Default::default(),
        }))?
        .as_str()
        .to_string();

    let is_public = req.is_public.unwrap_or(false);
    let now = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();

    dnf::upsert_dnf(
        db,
        AccountId::from_uuid(pseud.user.account_id.as_uuid()),
        PseudId::from_uuid(pseud.pseud_id.as_uuid()),
        WorkId::from_uuid(work_id),
        &reason_str,
        req.note.as_deref(),
        is_public,
        &now,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    let row = dnf::read_dnf(db, PseudId::from_uuid(pseud.pseud_id.as_uuid()), WorkId::from_uuid(work_id))
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "dnf" }))?;

    Ok(Json(row.into()))
}

/// Remove the caller's DNF record for a work.
async fn delete_dnf(
    pseud: RequirePseud,
    Path(work_id): Path<Uuid>,
    State(state): State<AppState>,
) -> ApiResult<StatusCode> {
    let now = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();

    let deleted = dnf::delete_dnf(state.db(), PseudId::from_uuid(pseud.pseud_id.as_uuid()), WorkId::from_uuid(work_id), &now)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError(AppError::NotFound { resource: "dnf" }))
    }
}

/// Get the caller's own DNF record for a work.
async fn get_dnf(
    pseud: RequirePseud,
    Path(work_id): Path<Uuid>,
    State(state): State<AppState>,
) -> ApiResult<Json<DnfResponse>> {
    let row = dnf::read_dnf(state.db(), PseudId::from_uuid(pseud.pseud_id.as_uuid()), WorkId::from_uuid(work_id))
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "dnf" }))?;

    Ok(Json(row.into()))
}

/// Get aggregated public DNF reason counts for a work.
async fn get_dnf_reasons(
    _session: MaybeSession,
    Path(work_id): Path<Uuid>,
    State(state): State<AppState>,
) -> ApiResult<Json<DnfReasonsResponse>> {
    let counts = dnf::aggregate_dnf_counts(state.db(), WorkId::from_uuid(work_id))
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    let reasons = counts
        .into_iter()
        .map(|(reason, count)| ReasonCount { reason, count })
        .collect();

    Ok(Json(DnfReasonsResponse {
        work_id: work_id.to_string(),
        reasons,
    }))
}

/// List all DNF records for the authenticated pseud.
async fn list_my_dnf(
    pseud: RequirePseud,
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<DnfResponse>>> {
    let rows = dnf::list_dnf_by_pseud(state.db(), PseudId::from_uuid(pseud.pseud_id.as_uuid()))
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    Ok(Json(rows.into_iter().map(Into::into).collect()))
}
