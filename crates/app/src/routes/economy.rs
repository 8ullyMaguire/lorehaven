//! M15 — Economy routes: credits, quotes, holds, bounties, subscriptions.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use time::OffsetDateTime;

use crate::auth::{MaybeSession, RequireSession};
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
    State(state): State<AppState>,
    MaybeSession(_user): MaybeSession,
) -> ApiResult<Json<Value>> {
    let bounties = lorehaven_db::bounties::list_flexible_bounties(state.db())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "bounties": bounties })))
}

/// Create a bounty (any type: standard, crowdfunded, reverse).
#[derive(Debug, Deserialize)]
pub struct CreateBountyBody {
    pub job_kind: String,
    pub terms: Value,
    pub amount: i64,
    #[serde(default = "default_bounty_type")]
    pub bounty_type: String,
}

fn default_bounty_type() -> String {
    "standard".to_string()
}

pub async fn create_bounty(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<CreateBountyBody>,
) -> ApiResult<Json<Value>> {
    let account = user.account_id.to_string();
    let id = uuid::Uuid::new_v4().to_string();
    let now = OffsetDateTime::now_utc();
    let created_at = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        now.year(),
        now.month() as u8,
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );

    let allowed_types = &state.config().bounties.allowed_types;
    if !allowed_types.contains(&body.bounty_type) {
        return Err(ApiError(lorehaven_domain::AppError::Validation {
            message: format!(
                "bounty type '{}' is not allowed (allowed: {:?})",
                body.bounty_type, allowed_types
            ),
            field_errors: Default::default(),
        }));
    }

    if body.amount < state.config().bounties.min_amount
        || body.amount > state.config().bounties.max_amount
    {
        return Err(ApiError(lorehaven_domain::AppError::Validation {
            message: format!(
                "bounty amount must be between {} and {}",
                state.config().bounties.min_amount,
                state.config().bounties.max_amount
            ),
            field_errors: Default::default(),
        }));
    }

    let (state_str, funded) = match body.bounty_type.as_str() {
        "reverse" => ("open", body.amount), // prepaid by creator
        "crowdfunded" => ("funding", 0),    // starts empty, activates when funded
        _ => ("open", body.amount),         // standard: escrow in place
    };

    lorehaven_db::bounties::create_bounty_typed(
        state.db(),
        &lorehaven_db::bounties::Bounty {
            id: id.clone(),
            bounty_type: body.bounty_type.clone(),
            job_kind: body.job_kind,
            terms: body.terms.to_string(),
            amount: body.amount,
            funded_amount: funded,
            state: state_str.to_string(),
            created_by: account,
            created_at,
            activated_at: None,
        },
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "id": id, "created": true })))
}

/// Contribute to a crowdfunded bounty.
#[derive(Debug, Deserialize)]
pub struct ContributeBountyBody {
    pub amount: i64,
}

pub async fn contribute_to_bounty(
    State(state): State<AppState>,
    Path(bounty_id): Path<String>,
    RequireSession(user): RequireSession,
    Json(body): Json<ContributeBountyBody>,
) -> ApiResult<Json<Value>> {
    let account = user.account_id.to_string();
    let threshold = state.config().bounties.crowdfund_activation_threshold;
    let (funded, activated) = lorehaven_db::bounties::contribute_to_bounty(
        state.db(),
        &bounty_id,
        &account,
        body.amount,
        threshold,
    )
    .await
    .map_err(|e| match e {
        lorehaven_db::bounties::FlexibleBountyError::NotFound => {
            ApiError(lorehaven_domain::AppError::NotFound {
                resource: "bounty",
            })
        }
        lorehaven_db::bounties::FlexibleBountyError::NotCrowdfunded => {
            ApiError(lorehaven_domain::AppError::Validation {
                message: "bounty is not crowdfunded".into(),
                field_errors: Default::default(),
            })
        }
        lorehaven_db::bounties::FlexibleBountyError::NotFunding => {
            ApiError(lorehaven_domain::AppError::Validation {
                message: "bounty is not accepting contributions".into(),
                field_errors: Default::default(),
            })
        }
        lorehaven_db::bounties::FlexibleBountyError::InvalidAmount => {
            ApiError(lorehaven_domain::AppError::Validation {
                message: "contribution must be positive".into(),
                field_errors: Default::default(),
            })
        }
        lorehaven_db::bounties::FlexibleBountyError::Sql(e) => {
            ApiError(lorehaven_domain::AppError::Internal(e.into()))
        }
    })?;
    Ok(Json(json!({ "funded_amount": funded, "activated": activated })))
}

/// Claim a bounty.
pub async fn claim_bounty(
    State(state): State<AppState>,
    Path(bounty_id): Path<String>,
    RequireSession(user): RequireSession,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<Json<Value>> {
    let account = user.account_id.to_string();
    lorehaven_db::economy::claim_bounty(state.db(), &bounty_id, &account)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
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
        .route("/bounties/{bounty_id}/contribute", post(contribute_to_bounty))
        .route("/subscription", get(get_subscription))
}
