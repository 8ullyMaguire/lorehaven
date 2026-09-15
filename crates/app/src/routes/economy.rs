//! M15 — Economy routes: credits, quotes, holds, bounties, subscriptions.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use time::OffsetDateTime;

use crate::auth::MaybeSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

/// Get current user's credit balances and recent transactions.
pub async fn get_credits(
    State(state): State<AppState>,
    MaybeSession(user): MaybeSession,
) -> ApiResult<Json<Value>> {
    let account = user.map(|u| u.account_id.to_string()).unwrap_or_default();
    if account.is_empty() {
        return Ok(Json(json!({
            "balances": [],
            "recent": [],
            "tier": "anonymous",
        })));
    }

    let balances = lorehaven_db::economy::balances(state.db(), &account)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    let balance_map: Vec<Value> = balances
        .into_iter()
        .map(|(bucket, amount)| json!({ "bucket": bucket, "amount_bp": amount }))
        .collect();

    let tier = "free".to_owned();

    Ok(Json(json!({
        "balances": balance_map,
        "tier": tier,
    })))
}

/// Request a job quote.
#[derive(Debug, Deserialize)]
pub struct QuoteBody {
    pub kind: String,
}

pub async fn get_quote(
    State(_state): State<AppState>,
    MaybeSession(_user): MaybeSession,
    Json(body): Json<QuoteBody>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({
        "kind": body.kind,
        "estimated_bp": 10,
        "currency": "credits",
    })))
}

/// Reserve a hold for a job.
#[derive(Debug, Deserialize)]
pub struct ReserveBody {
    pub job_id: String,
    pub amount: i64,
}

pub async fn post_reserve(
    State(_state): State<AppState>,
    MaybeSession(user): MaybeSession,
    Json(_body): Json<ReserveBody>,
) -> ApiResult<Json<Value>> {
    let account = user.map(|u| u.account_id.to_string()).unwrap_or_default();
    if account.is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "session",
            "sign in to reserve credits",
        )));
    }
    Ok(Json(json!({ "reserved": true })))
}

/// Get queue position for a job.
pub async fn get_queue_position(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
    MaybeSession(_user): MaybeSession,
) -> ApiResult<Json<Value>> {
    let pos = lorehaven_db::economy::queue_position(state.db(), &job_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    match pos {
        Some((class, position)) => Ok(Json(json!({
            "job_id": job_id,
            "priority_class": class,
            "position": position,
        }))),
        None => Err(ApiError(lorehaven_domain::AppError::field(
            "job",
            "not found in queue",
        ))),
    }
}

/// Get current user's usage vs caps.
pub async fn get_usage(
    State(state): State<AppState>,
    MaybeSession(user): MaybeSession,
) -> ApiResult<Json<Value>> {
    let account = user.map(|u| u.account_id.to_string()).unwrap_or_default();
    if account.is_empty() {
        return Ok(Json(json!({ "usage": [] })));
    }

    let now = OffsetDateTime::now_utc();
    let day = format!(
        "{:04}-{:02}-{:02}",
        now.year(),
        now.month() as u8,
        now.day()
    );
    let usage = lorehaven_db::economy::usage_for(state.db(), &account, &day)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    let items: Vec<Value> = usage
        .into_iter()
        .map(|(action, count)| json!({ "action": action, "count": count }))
        .collect();

    Ok(Json(json!({ "day": day, "usage": items })))
}

/// List bounties.
pub async fn list_bounties(
    State(_state): State<AppState>,
    MaybeSession(_user): MaybeSession,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "bounties": [] })))
}

/// Create a bounty.
#[derive(Debug, Deserialize)]
pub struct CreateBountyBody {
    pub job_kind: String,
    pub terms: Value,
    pub amount: i64,
}

pub async fn create_bounty(
    State(_state): State<AppState>,
    MaybeSession(user): MaybeSession,
    Json(_body): Json<CreateBountyBody>,
) -> ApiResult<Json<Value>> {
    let account = user.map(|u| u.account_id.to_string()).unwrap_or_default();
    if account.is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "session",
            "sign in to create bounties",
        )));
    }
    Ok(Json(json!({ "created": true })))
}

/// Claim a bounty.
pub async fn claim_bounty(
    State(_state): State<AppState>,
    Path(_bounty_id): Path<String>,
    MaybeSession(user): MaybeSession,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<Json<Value>> {
    let account = user.map(|u| u.account_id.to_string()).unwrap_or_default();
    if account.is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "session",
            "sign in to claim bounties",
        )));
    }
    Ok(Json(json!({ "claimed": true })))
}

/// Get subscription status.
pub async fn get_subscription(
    State(_state): State<AppState>,
    MaybeSession(user): MaybeSession,
) -> ApiResult<Json<Value>> {
    let account = user.map(|u| u.account_id.to_string()).unwrap_or_default();
    if account.is_empty() {
        return Ok(Json(json!({ "tier": "free", "state": "none" })));
    }
    Ok(Json(json!({ "tier": "free", "state": "active" })))
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/credits", get(get_credits))
        .route("/credits/quote", get(get_quote))
        .route("/credits/reserve", post(post_reserve))
        .route("/jobs/{job_id}/queue-position", get(get_queue_position))
        .route("/usage", get(get_usage))
        .route("/bounties", get(list_bounties).post(create_bounty))
        .route("/bounties/{bounty_id}/claim", post(claim_bounty))
        .route("/subscription", get(get_subscription))
}
