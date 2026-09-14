//! M21 — Monetization API contracts (spec §20.9).
//!
//! Skeleton: every route is registered with its request/response shape and
//! returns `501 NOT_IMPLEMENTED`. The shapes ARE the contract — the
//! implementing agent fills bodies without changing them, because
//! `crates/app/tests/milestone_21.rs` pins them.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::Json;
use serde::Deserialize;
use serde_json::Value;

use crate::auth::{MaybeSession, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

fn todo() -> ApiError {
    ApiError(lorehaven_domain::AppError::NotImplemented)
}

#[derive(Debug, Deserialize)]
pub struct PricingBody {
    pub model: String,
    pub price_minor: i64,
    pub currency: String,
    /// Early-access offset in seconds before `public_at` unlocks the chapter.
    pub public_at_offset: Option<i64>,
}

/// PUT/POST the pricing configuration of a work (§20.9.2).
pub async fn set_pricing(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(_work_id): Path<String>,
    Json(_body): Json<PricingBody>,
) -> ApiResult<Json<Value>> {
    Err(todo())
}

/// Remove pricing; purchased access is never revoked by this (§20.9.3).
pub async fn delete_pricing(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(_work_id): Path<String>,
) -> ApiResult<Json<Value>> {
    Err(todo())
}

/// Buy / unlock a work under its current model.
pub async fn purchase(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(_work_id): Path<String>,
) -> ApiResult<Json<Value>> {
    Err(todo())
}

#[derive(Debug, Deserialize)]
pub struct TipBody {
    pub amount_minor: i64,
    pub currency: String,
    /// `credits` transfers credits between wallets; anything else is money and
    /// lands in the earnings ledger (§20.9.2).
    pub channel: String,
}

/// Tip an author, in credits or in money.
pub async fn tip(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(_work_id): Path<String>,
    Json(_body): Json<TipBody>,
) -> ApiResult<Json<Value>> {
    Err(todo())
}

/// The caller's durable access records (§20.9.3).
pub async fn my_entitlements(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
) -> ApiResult<Json<Value>> {
    Err(todo())
}

/// The author's earnings ledger — totals and entries, never a supporter
/// roster (§20.9.3).
pub async fn my_earnings(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
) -> ApiResult<Json<Value>> {
    Err(todo())
}

/// Request a payout through the payment processor's flow.
pub async fn request_payout(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
) -> ApiResult<Json<Value>> {
    Err(todo())
}

/// Instance-wide monetization state for operators (§20.9.1): the eligibility
/// setting and per-work pricing inventory.
pub async fn admin_monetization(
    State(_state): State<AppState>,
    MaybeSession(_user): MaybeSession,
) -> ApiResult<Json<Value>> {
    Err(todo())
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/works/{work_id}/pricing", post(set_pricing).delete(delete_pricing))
        .route("/works/{work_id}/purchase", post(purchase))
        .route("/works/{work_id}/tips", post(tip))
        .route("/me/entitlements", get(my_entitlements))
        .route("/me/earnings", get(my_earnings))
        .route("/me/payouts", post(request_payout))
        .route("/admin/monetization", get(admin_monetization))
}

/// Gift works and dedications (§18.10). Same milestone, own contract group.
pub async fn create_gift(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(_work_id): Path<String>,
    Json(_body): Json<Value>,
) -> ApiResult<Json<Value>> {
    Err(todo())
}

pub async fn list_gifts(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
) -> ApiResult<Json<Value>> {
    Err(todo())
}

pub fn gifts_router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/works/{work_id}/gifts", post(create_gift))
        .route("/me/gifts", get(list_gifts))
}
