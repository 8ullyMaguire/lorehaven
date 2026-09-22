use crate::auth::{MaybeSession, RequirePseud, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get};
use axum::{Json, Router};
use lorehaven_domain::AppError;
use serde::Deserialize;
use serde_json::{json, Value};
use lorehaven_db::roles;

/// Check whether the current user holds the Vanguard role.
pub async fn get_my_vanguard_status(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    let is_vanguard = roles::is_vanguard(state.db(), &user.account_id.to_string())
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(json!({ "is_vanguard": is_vanguard })))
}

/// Pin a work to the Vanguard Picks shelf.
#[derive(Debug, Deserialize)]
pub struct PinWorkBody {
    pub pin_reason: String,
    pub message: Option<String>,
}

pub async fn pin_work(
    State(state): State<AppState>,
    RequirePseud { user, .. }: RequirePseud,
    Path(work_id): Path<String>,
    Json(body): Json<PinWorkBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let account_id = user.account_id.to_string();
    let is_vanguard = roles::is_vanguard(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    if !is_vanguard {
        return Err(ApiError(AppError::AccessDenied));
    }
    let id = roles::pin_work(state.db(), &account_id, &work_id, &body.pin_reason, body.message.as_deref())
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

/// Unpin a work.
pub async fn unpin_work(
    State(state): State<AppState>,
    RequirePseud { user, .. }: RequirePseud,
    Path(work_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let account_id = user.account_id.to_string();
    roles::unpin_work(state.db(), &account_id, &work_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(json!({ "unpinned": true })))
}

/// List pins for a work (public).
pub async fn get_pins_for_work(
    State(state): State<AppState>,
    MaybeSession(_session): MaybeSession,
    Path(work_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let pins = roles::list_active_pins(state.db())
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    let filtered: Vec<Value> = pins.into_iter()
        .filter(|p| p["work_id"].as_str() == Some(work_id.as_str()))
        .collect();
    Ok(Json(json!({ "pins": filtered })))
}

/// Check whether the current user is an operator (trust level ≥ 5).
async fn require_operator(state: &AppState, user: &crate::auth::SessionUser) -> ApiResult<()> {
    let level = lorehaven_db::governance::trust_for(state.db(), &user.account_id.to_string())
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    if level < 5 {
        return Err(ApiError(AppError::AccessDenied));
    }
    Ok(())
}

/// Admin: grant Vanguard role to an account.
#[derive(Debug, Deserialize)]
pub struct GrantVanguardBody {
    pub account_id: String,
    pub selection_method: String,
    pub resonance_score: Option<f64>,
}

pub async fn grant_vanguard(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<GrantVanguardBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    require_operator(&state, &user).await?;
    roles::grant_vanguard(
        state.db(),
        &body.account_id,
        &body.selection_method,
        Some(&user.account_id.to_string()),
        None,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok((StatusCode::CREATED, Json(json!({ "granted": true }))))
}

/// Admin: revoke Vanguard role from an account.
pub async fn revoke_vanguard(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(account_id): Path<String>,
) -> ApiResult<Json<Value>> {
    require_operator(&state, &user).await?;
    roles::revoke_vanguard(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(json!({ "revoked": true })))
}

/// List all current vanguards (admin only).
pub async fn list_vanguards(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    require_operator(&state, &user).await?;
    let vanguards = roles::list_vanguards(state.db())
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(json!({ "vanguards": vanguards })))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/vanguard/status", get(get_my_vanguard_status))
        .route("/vanguard/pins/{work_id}", get(get_pins_for_work).post(pin_work).delete(unpin_work))
        .route("/vanguards", get(list_vanguards).post(grant_vanguard))
        .route("/vanguards/{account_id}", delete(revoke_vanguard))
}
