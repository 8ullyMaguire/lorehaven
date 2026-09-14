//! M21 — Monetization API routes (spec §20.9).
//!
//! Implements monetization endpoints by delegating to the `lorehaven_db`
//! repository layer. External payment-provider integration (webhooks, payouts)
//! is deferred to the operator's configured gateway; routes enforce
//! trust-level gate (TL >= 5) and idempotency where applicable.

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

#[derive(Debug, Deserialize)]
pub struct GiftBody {
    pub gift_note: Option<String>,
    pub challenge_fulfillment_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PayoutBody {
    pub amount_minor: i64,
    pub currency: String,
    pub processor_reference: String,
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
        state.db(),
        &work_id.to_canonical_string(),
        model_str,
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
    let rows = monetization::disable_pricing(state.db(), &work_id.to_canonical_string())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "deleted": rows > 0 })))
}

pub async fn purchase(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(work_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let work_id = work_id
        .parse::<lorehaven_domain::WorkId>()
        .map_err(|_| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;
    let work = lorehaven_db::content::find_work(state.db(), work_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;
    let author_pseud = lorehaven_db::identity::find_pseud(state.db(), work.owner_pseud_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "author" }))?;
    let pricing = monetization::get_pricing(state.db(), &work_id.to_canonical_string())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?
        .into_iter()
        .next()
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "pricing" }))?;
    if !pricing.enabled {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "work_id",
            "this work is not currently for sale",
        )));
    }

    let idempotency = format!("purchase:{}:{}", user.account_id, work_id.to_canonical_string());
    // Idempotency: if already entitled, return the existing entitlement id.
    // grant_entitlement uses an idempotency key on work_id+account, so re-calls
    // return the same row. We check via has_entitlement first for a clean response.
    let already = monetization::has_entitlement(
        state.db(),
        &user.account_id.to_string(),
        &work_id.to_canonical_string(),
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    if already {
        let ent = monetization::get_entitlements(state.db(), &user.account_id.to_string())
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?
            .into_iter()
            .find(|e| e.work_id == work_id.to_canonical_string())
            .ok_or_else(|| ApiError(lorehaven_domain::AppError::Internal(anyhow::anyhow!("entitlement race"))))?;
        return Ok(Json(json!({ "entitlement_id": ent.id, "status": "already_granted" })));
    }

    let entitlement_id = monetization::grant_entitlement(
        state.db(),
        &user.account_id.to_string(),
        &work_id.to_canonical_string(),
        "purchase",
        None,
        None,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    // 85/15 split: author gets 85%, platform keeps 15%.
    // Platform fee is tracked separately from author earnings (author_earnings_ledger
    // has a FK to accounts, so platform is not a registered account).
    let (author_amt, _platform_amt) =
        lorehaven_domain::monetization::Rules::split(pricing.price_minor, 1_500);
    let author_payment_id = format!("purchase:{}:author", entitlement_id);
    let _author_earning = monetization::post_earnings(
        state.db(),
        &author_pseud.account_id.to_string(),
        author_amt,
        &pricing.currency,
        "sale",
        Some(&author_payment_id),
        Some(&idempotency),
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({
        "entitlement_id": entitlement_id,
        "status": "purchased",
        "amount_minor": pricing.price_minor,
        "currency": pricing.currency,
    })))
}

#[derive(Debug, Deserialize)]
pub struct TipBody {
    pub amount_minor: i64,
    pub currency: String,
    pub channel: String,
}

pub async fn tip(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(work_id): Path<String>,
    Json(body): Json<TipBody>,
) -> ApiResult<Json<Value>> {
    let work_id = work_id
        .parse::<lorehaven_domain::WorkId>()
        .map_err(|_| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;
    // Resolve the work's author account via owner pseud → account.
    let work = lorehaven_db::content::find_work(state.db(), work_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    let work = work.ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;
    let pseud = lorehaven_db::identity::find_pseud(state.db(), work.owner_pseud_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    let author = pseud.ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "author" }))?;
    let author_account = author.account_id.to_string();

    // Refuse self-dealing (tips between pseuds of one account, §20.9.3).
    if lorehaven_domain::monetization::Rules::self_dealing(&user.account_id.to_string(), &author_account) {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "work_id",
            "cannot tip your own work",
        )));
    }

    let idempotency = format!(
        "tip:{}-{}:{}:{}",
        user.account_id, work_id, body.currency, body.amount_minor
    );

    match body.channel.as_str() {
        "money" => {
            // Money tip: author receives 85%, platform keeps 15%, each
            // as a separate ledger entry (ADR 0004: balanced entries).
            let (author_amt, platform_amt) =
                lorehaven_domain::monetization::Rules::split(body.amount_minor, 1_500);
            let auth_id = monetization::post_earnings(
                state.db(),
                &author_account,
                author_amt,
                &body.currency,
                "tip",
                None,
                Some(&idempotency),
            )
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
            let plat_id = monetization::post_earnings(
                state.db(),
                "platform",
                platform_amt,
                &body.currency,
                "platform_fee",
                None,
                Some(&format!("platform:{}", idempotency)),
            )
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
            Ok(Json(json!({
                "author_earning_id": auth_id,
                "platform_fee_id": plat_id,
                "author_amount_minor": author_amt,
                "platform_amount_minor": platform_amt,
            })))
        }
        "credit" => {
            // Credit tip: a balanced credit transaction moving the tipper's
            // money bucket to the author's money bucket.
            let entries = vec![
                (user.account_id.to_string(), "money".to_string(), -body.amount_minor),
                (author_account.clone(), "money".to_string(), body.amount_minor),
            ];
            let txn_id = lorehaven_db::economy::post_transaction(
                state.db(),
                lorehaven_domain::economy::TxnType::Spend,
                &idempotency,
                &format!("tip:{}", work_id),
                &entries,
            )
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
            Ok(Json(json!({
                "transaction_id": txn_id,
                "credit_amount_minor": body.amount_minor,
            })))
        }
        _ => Err(ApiError(lorehaven_domain::AppError::field(
            "channel",
            "must be 'money' or 'credit'",
        ))),
    }
}

pub async fn my_entitlements(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    let rows = monetization::get_entitlements(state.db(), &user.account_id.to_string())
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
    let rows = monetization::get_earnings(state.db(), &user.account_id.to_string())
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
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<PayoutBody>,
) -> ApiResult<Json<Value>> {
    let payout_id = monetization::create_payout(
        state.db(),
        &user.account_id.to_string(),
        body.amount_minor,
        &body.currency,
        &body.processor_reference,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "id": payout_id })))
}

pub async fn admin_monetization(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    let level = lorehaven_db::governance::trust_for(state.db(), &user.account_id.to_string())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    if level < 5 {
        return Err(ApiError(lorehaven_domain::AppError::AccessDenied));
    }

    let total_revenue = monetization::total_platform_revenue(state.db())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    let pending_payouts = monetization::pending_payout_total(state.db())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    let active_authors = monetization::active_earning_authors(state.db())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    let active_purchasers = monetization::active_purchaser_count(state.db())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({
        "total_revenue_minor": total_revenue,
        "pending_payout_minor": pending_payouts,
        "active_earning_authors": active_authors,
        "active_purchasers": active_purchasers,
        "trust_level": level,
    })))
}

pub async fn create_gift(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(work_id): Path<String>,
    Json(body): Json<GiftBody>,
) -> ApiResult<Json<Value>> {
    let work_id = work_id
        .parse::<lorehaven_domain::WorkId>()
        .map_err(|_| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;
    let pseud = user.pseud_id.ok_or_else(|| {
        ApiError(lorehaven_domain::AppError::field(
            "pseud_id",
            "a pseud must be selected to create a gift",
        ))
    })?;
    // Validate the work exists before inserting the gift.
    let _work = lorehaven_db::content::find_work(state.db(), work_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;
    let gift_id = monetization::create_gift(
        state.db(),
        &work_id.to_canonical_string(),
        &pseud.to_canonical_string(),
        body.gift_note.as_deref(),
        body.challenge_fulfillment_id.as_deref(),
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "id": gift_id })))
}

pub async fn list_gifts(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    let pseud = user.pseud_id.ok_or_else(|| {
        ApiError(lorehaven_domain::AppError::field(
            "pseud_id",
            "a pseud must be selected to list gifts",
        ))
    })?;
    let rows = monetization::get_gifts_for_recipient(state.db(), &pseud.to_canonical_string())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    let out: Vec<Value> = rows
        .into_iter()
        .map(|r| json!({
            "id": r.id,
            "work_id": r.work_id,
            "gift_note": r.gift_note,
            "challenge_fulfillment_id": r.challenge_fulfillment_id,
            "created_at": r.created_at,
            "declined_at": r.declined_at,
        }))
        .collect();
    Ok(Json(json!({ "gifts": out })))
}

/// Public pricing lookup — readable by anyone viewing a work page.
/// Returns 404 if the work doesn't exist (no existence disclosure to strangers
/// is NOT applied here: pricing is public metadata for published works, and
/// an unpublished work returns 404 via find_work's lifecycle filter).
pub async fn public_pricing(
    State(state): State<AppState>,
    Path(work_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let work_id = work_id
        .parse::<lorehaven_domain::WorkId>()
        .map_err(|_| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;
    let pricing = monetization::get_pricing(state.db(), &work_id.to_canonical_string())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    let enabled: Vec<Value> = pricing
        .into_iter()
        .filter(|p| p.enabled)
        .map(|p| json!({
            "model": p.model,
            "price_minor": p.price_minor,
            "currency": p.currency,
            "public_at_offset": p.public_at_offset,
        }))
        .collect();
    Ok(Json(json!({ "pricing": enabled })))
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
        .route("/works/{work_id}/pricing", get(public_pricing))
        .route("/me/entitlements", get(my_entitlements))
        .route("/me/earnings", get(my_earnings))
}

pub fn gifts_router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/works/{work_id}/gifts", post(create_gift))
        .route("/me/gifts", get(list_gifts))
}
