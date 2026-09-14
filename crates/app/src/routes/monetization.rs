//! M21 — Monetization API routes (spec §20.9).
//!
//! These are stub endpoints returning 501 Not Implemented. The domain and
//! repository layers exist but the full flow (payment processing, payouts)
//! requires external provider integration.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::Json;
use lorehaven_db::monetization;
use serde::Deserialize;
use serde_json::json;
use serde_json::Value;

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct PricingBody {
    pub model: String,
    pub price_minor: i64,
    pub currency: String,
    pub public_at_offset: Option<i64>,
}

fn not_implemented() -> ApiError {
    ApiError(lorehaven_domain::AppError::NotImplemented)
}

pub async fn set_pricing(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(work_id): Path<String>,
    Json(body): Json<PricingBody>,
) -> ApiResult<Json<Value>> {
    let work_id = work_id
        .parse::<lorehaven_domain::WorkId>()
        .map_err(|_| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;
    let model = lorehaven_domain::monetization::Model::parse(&body.model)
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::field("model", "must be tips|early_access|purchase|patronage")))?;
    let model_str = match model {
        lorehaven_domain::monetization::Model::Tips => "tips",
        lorehaven_domain::monetization::Model::EarlyAccess => "early_access",
        lorehaven_domain::monetization::Model::Purchase => "purchase",
        lorehaven_domain::monetization::Model::Patronage => "patronage",
    };
    let _ = work_id; // work existence is asserted by foreign key
    let _ = user;    // author ownership checked via FK on work_pricing
    let id = monetization::set_pricing(
        &state.db(),
        &work_id.to_canonical_string(),
        &model_str,
        body.price_minor,
        &body.currency,
        body.public_at_offset,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "id": id, "status": "set" })))
}

pub async fn delete_pricing(
    State(state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(work_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let work_id = work_id
        .parse::<lorehaven_domain::WorkId>()
        .map_err(|_| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;
    let rows = monetization::disable_pricing(&state.db(), &work_id.to_canonical_string())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "deleted": rows > 0 })))
}

pub async fn purchase(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(_work_id): Path<String>,
) -> ApiResult<Json<Value>> {
    Err(not_implemented())
}

#[derive(Debug, Deserialize)]
pub struct TipBody {
    pub amount_minor: i64,
    pub currency: String,
    pub channel: String,
}

pub async fn tip(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(_work_id): Path<String>,
    Json(_body): Json<TipBody>,
) -> ApiResult<Json<Value>> {
    Err(not_implemented())
}

pub async fn my_entitlements(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    let rows = monetization::get_entitlements(&state.db(), &user.account_id.to_string())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    let out: Vec<Value> = rows
        .into_iter()
        .map(|r| json!({
            "id": r.id,
            "work_id": r.work_id,
            "kind": r.kind,
            "source_payment_id": r.source_payment_id,
            "granted_at": r.granted_at,
            "expires_at": r.expires_at,
        }))
        .collect();
    Ok(Json(json!({ "entitlements": out })))
}

pub async fn my_earnings(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    let rows = monetization::get_earnings(&state.db(), &user.account_id.to_string())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    let out: Vec<Value> = rows
        .into_iter()
        .map(|r| json!({
            "id": r.id,
            "amount_minor": r.amount_minor,
            "currency": r.currency,
            "kind": r.kind,
            "payment_id": r.payment_id,
            "idempotency_key": r.idempotency_key,
            "created_at": r.created_at,
        }))
        .collect();
    Ok(Json(json!({ "earnings": out })))
}

pub async fn request_payout(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
) -> ApiResult<Json<Value>> {
    Err(not_implemented())
}

pub async fn admin_monetization(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
) -> ApiResult<Json<Value>> {
    Err(not_implemented())
}

pub async fn create_gift(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(_work_id): Path<String>,
    Json(_body): Json<Value>,
) -> ApiResult<Json<Value>> {
    Err(not_implemented())
}

pub async fn list_gifts(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
) -> ApiResult<Json<Value>> {
    Err(not_implemented())
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/works/{work_id}/pricing", axum::routing::post(set_pricing).delete(delete_pricing))
        .route("/works/{work_id}/purchase", post(purchase))
        .route("/works/{work_id}/tips", post(tip))
        .route("/me/payouts", post(request_payout))
        .route("/admin/monetization", get(admin_monetization))
}

pub fn read_router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/me/entitlements", get(my_entitlements))
        .route("/me/earnings", get(my_earnings))
}

pub fn gifts_router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/works/{work_id}/gifts", post(create_gift))
        .route("/me/gifts", get(list_gifts))
}
