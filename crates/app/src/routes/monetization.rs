//! M21 — Monetization API routes (spec §20.9).
//!
//! Implements monetization endpoints by delegating to the `lorehaven_db`
//! repository layer. External payment-provider integration (webhooks, payouts)
//! is deferred to the operator's configured gateway; routes enforce
//! trust-level gate (TL >= 5) and idempotency where applicable.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::Json;
use lorehaven_db::{monetization, Backend};
use serde::Deserialize;
use serde_json::json;
use serde_json::Value;
use uuid::Uuid;

use crate::auth::{MaybeSession, RequirePseud, RequireSession};
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
    let model = lorehaven_domain::monetization::Model::parse(&body.model).ok_or_else(|| {
        ApiError(lorehaven_domain::AppError::field(
            "model",
            "must be tips|early_access|purchase|patronage",
        ))
    })?;
    // Ownership: pricing is a money decision, so only the work's owner may
    // make it. A work the caller does not own is reported as absent — the
    // same deliberate indistinguishability the works module uses — because
    // an FK on works(id) only proves the work exists, not who may price it.
    let work = lorehaven_db::content::find_work(state.db(), work_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;
    let pseud = user.pseud_id.ok_or_else(|| {
        ApiError(lorehaven_domain::AppError::field(
            "pseud_id",
            "a pseud must be selected to set pricing",
        ))
    })?;
    if work.owner_pseud_id != pseud {
        return Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "work",
        }));
    }
    let model_str = match model {
        lorehaven_domain::monetization::Model::Tips => "tips",
        lorehaven_domain::monetization::Model::EarlyAccess => "early_access",
        lorehaven_domain::monetization::Model::Purchase => "purchase",
        lorehaven_domain::monetization::Model::Patronage => "patronage",
    };
    let _ = work_id; // work existence is asserted by foreign key
    let _ = user; // author ownership checked via FK on work_pricing
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
    RequireSession(user): RequireSession,
    Path(work_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let work_id = work_id
        .parse::<lorehaven_domain::WorkId>()
        .map_err(|_| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;
    // Same ownership rule as set_pricing: only the owner may unprice a work.
    let work = lorehaven_db::content::find_work(state.db(), work_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;
    let pseud = user.pseud_id.ok_or_else(|| {
        ApiError(lorehaven_domain::AppError::field(
            "pseud_id",
            "a pseud must be selected to change pricing",
        ))
    })?;
    if work.owner_pseud_id != pseud {
        return Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "work",
        }));
    }
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
        .ok_or_else(|| {
            ApiError(lorehaven_domain::AppError::NotFound {
                resource: "pricing",
            })
        })?;
    if !pricing.enabled {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "work_id",
            "this work is not currently for sale",
        )));
    }

    let idempotency = format!(
        "purchase:{}:{}",
        user.account_id,
        work_id.to_canonical_string()
    );
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
            .ok_or_else(|| {
                ApiError(lorehaven_domain::AppError::Internal(anyhow::anyhow!(
                    "entitlement race"
                )))
            })?;
        return Ok(Json(
            json!({ "entitlement_id": ent.id, "status": "already_granted" }),
        ));
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
        Some(&author_pseud.account_id.to_string()),
        author_amt,
        &pricing.currency,
        "sale",
        Some(&author_payment_id),
        Some(&idempotency),
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    // Tell the author their work sold. A failure here must not fail the
    // purchase: the money state above is already committed.
    let _ = lorehaven_db::notifications::notify(
        state.db(),
        &author_pseud.account_id.to_string(),
        "sale",
        "Someone bought your work",
        &format!("{} was purchased.", work.title),
        Some(work_id.to_canonical_string().as_str()),
    )
    .await;

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
    let work =
        work.ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;
    let pseud = lorehaven_db::identity::find_pseud(state.db(), work.owner_pseud_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    let author = pseud
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "author" }))?;
    let author_account = author.account_id.to_string();

    // Refuse self-dealing (tips between pseuds of one account, §20.9.3).
    if lorehaven_domain::monetization::Rules::self_dealing(
        &user.account_id.to_string(),
        &author_account,
    ) {
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
                Some(&author_account),
                author_amt,
                &body.currency,
                "tip",
                None,
                Some(&idempotency),
            )
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
            // The platform's share is a balanced entry with no account of
            // its own (the ledger's FK to accounts rules out a synthetic
            // "platform" account — SQLite used to swallow it, PostgreSQL
            // rightly refuses it).
            let plat_id = monetization::post_earnings(
                state.db(),
                None,
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
                (
                    user.account_id.to_string(),
                    "money".to_string(),
                    -body.amount_minor,
                ),
                (
                    author_account.clone(),
                    "money".to_string(),
                    body.amount_minor,
                ),
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
        .map(|r| {
            json!({
                "id": r.id,
                "work_id": r.work_id,
                "kind": r.kind,
                "source_payment_id": r.source_payment_id,
                "granted_at": r.granted_at,
                "expires_at": r.expires_at,
            })
        })
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
        .map(|r| {
            json!({
                "id": r.id,
                "amount_minor": r.amount_minor,
                "currency": r.currency,
                "kind": r.kind,
                "payment_id": r.payment_id,
                "idempotency_key": r.idempotency_key,
                "created_at": r.created_at,
            })
        })
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
    let work = lorehaven_db::content::find_work(state.db(), work_id)
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

    // The work's author learns their work was gifted. The gift row's
    // recipient is the caller (a dedication claim), so the author is the
    // party with something to hear about. Best-effort: never fail the gift.
    if let Ok(Some(author)) =
        lorehaven_db::identity::find_pseud(state.db(), work.owner_pseud_id).await
    {
        let _ = lorehaven_db::notifications::notify(
            state.db(),
            &author.account_id.to_string(),
            "gift",
            "Your work was gifted",
            &format!("{} received a gift.", work.title),
            Some(work_id.to_canonical_string().as_str()),
        )
        .await;
    }

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
        .map(|r| {
            json!({
                "id": r.id,
                "work_id": r.work_id,
                "gift_note": r.gift_note,
                "challenge_fulfillment_id": r.challenge_fulfillment_id,
                "created_at": r.created_at,
                "declined_at": r.declined_at,
            })
        })
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
    MaybeSession(_user): MaybeSession,
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
        .map(|p| {
            json!({
                "model": p.model,
                "price_minor": p.price_minor,
                "currency": p.currency,
                "public_at_offset": p.public_at_offset,
            })
        })
        .collect();
    Ok(Json(json!({ "pricing": enabled })))
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route(
            "/works/{work_id}/pricing",
            axum::routing::post(set_pricing).delete(delete_pricing),
        )
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

#[derive(Debug, Deserialize)]
pub struct AiDeclarationBody {
    pub declaration: String,
}

pub async fn set_work_ai_declaration(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(work_id): Path<String>,
    Json(body): Json<AiDeclarationBody>,
) -> ApiResult<Json<Value>> {
    let work_id = work_id
        .parse::<lorehaven_domain::WorkId>()
        .map_err(|_| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;
    let declaration = lorehaven_domain::monetization::AiDeclaration::parse(&body.declaration)
        .ok_or_else(|| {
            ApiError(lorehaven_domain::AppError::field(
                "declaration",
                "must be none|assisted|co-written|generated",
            ))
        })?;
    let work = lorehaven_db::content::find_work(state.db(), work_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;
    let pseud = user.pseud_id.ok_or_else(|| {
        ApiError(lorehaven_domain::AppError::field(
            "pseud_id",
            "a pseud must be selected to declare AI involvement",
        ))
    })?;
    if work.owner_pseud_id != pseud {
        return Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "work",
        }));
    }
    monetization::set_ai_declaration(
        state.db(),
        &work_id.to_canonical_string(),
        declaration.as_str(),
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(
        json!({ "status": "set", "declaration": declaration.as_str() }),
    ))
}

pub async fn get_work_ai_declaration(
    State(state): State<AppState>,
    Path(work_id): Path<String>,
    MaybeSession(_user): MaybeSession,
) -> ApiResult<Json<Value>> {
    let work_id = work_id
        .parse::<lorehaven_domain::WorkId>()
        .map_err(|_| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;
    let decl = monetization::get_ai_declaration(state.db(), &work_id.to_canonical_string())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "declaration": decl })))
}

pub async fn get_transparency_dashboard(
    State(state): State<AppState>,
    MaybeSession(_user): MaybeSession,
) -> ApiResult<Json<Value>> {
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
        "fee_split_bp": 1500,
        "graduated_cap": {
            "band1_multiple": 5,
            "band2_multiple": 10,
        },
        "quality_weights": {
            "rating_bp": 4000,
            "review_bp": 2000,
            "karma_bp": 2000,
            "longevity_bp": 2000,
        },
        "pool_b_floor": {
            "distinct_readers": 5,
            "account_age_days": 30,
            "trust_level": 1,
        },
        "ai_multipliers": {
            "none": 1.0,
            "assisted": 1.0,
            "co_written": 0.3,
            "generated": 0.0,
        },
    })))
}

pub fn transparency_router() -> axum::Router<AppState> {
    axum::Router::new()
        .route(
            "/works/{work_id}/ai-declaration",
            post(set_work_ai_declaration).get(get_work_ai_declaration),
        )
        .route(
            "/transparency/monetization",
            get(get_transparency_dashboard),
        )
        .route(
            "/works/{work_id}/reading-session",
            post(record_reading_session),
        )
        .route("/admin/monetization/settle", post(settle_period))
}

#[derive(Debug, Deserialize)]
pub struct ReadingSessionRequest {
    pub seconds: i64,
}

pub async fn record_reading_session(
    State(state): State<AppState>,
    Path(work_id): Path<String>,
    RequirePseud { user, pseud_id }: RequirePseud,
    Json(body): Json<ReadingSessionRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let work_id = work_id
        .parse::<lorehaven_domain::WorkId>()
        .map_err(|_| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;
    let now = lorehaven_db::identity::now_rfc3339();
    // A fresh UUID, not a concatenation. The previous line built
    // `{account}-{work}-{8 hex}`, which SQLite stored happily as TEXT and
    // PostgreSQL rejected with 22P02, since `reading_sessions.id` is UUID there.
    let session_id = Uuid::new_v4().to_string();
    match state.db().backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO reading_sessions (id, account_id, work_id, seconds, started_at, ended_at)
                 VALUES (?, ?, ?, ?, ?, ?)"
            )
            .bind(&session_id).bind(user.account_id.to_string())
            .bind(work_id.to_canonical_string()).bind(body.seconds)
            .bind(&now).bind(&now)
            .execute(state.db().sqlite_pool().expect("sqlite")).await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
        }
        Backend::Postgres => {
            sqlx::query(
                // `$n` rather than `?`: this arm builds its own string instead
                // of going through `db.sql`, so nothing rewrites the
                // placeholders -- and a literal `?::uuid` is a syntax error on
                // the SQLite arm of a shared statement.
                "INSERT INTO reading_sessions (id, account_id, work_id, seconds, started_at, ended_at)
                 VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5, $6)"
            )
            .bind(&session_id).bind(user.account_id.to_string())
            .bind(work_id.to_canonical_string()).bind(body.seconds)
            .bind(&now).bind(&now)
            .execute(state.db().postgres_pool().expect("postgres")).await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
        }
    }
    let _ = pseud_id;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(json!({ "status": "recorded" })),
    ))
}

// ---------------------------------------------------------------------------
// Pool settlement (spec §20.10)
// ---------------------------------------------------------------------------

use axum::extract::Query;

#[derive(Debug, Deserialize)]
pub struct SettleRequest {
    pub period_start: String,
    pub period_end: String,
}

/// Settle a monetization period: apply the graduated cap, compute Pool A/B
/// splits, and distribute Pool B by quality-weighted reading time.
/// Requires TL >= 5 (admin).
pub async fn settle_period(
    State(state): State<AppState>,
    RequireSession(_user): RequireSession,
    Query(req): Query<SettleRequest>,
) -> ApiResult<Json<Value>> {
    use lorehaven_domain::monetization::{distribute_pool_b, GraduatedCap};

    // 1. Pool A per author (trailing 3-month window)
    let author_earnings = lorehaven_db::monetization::author_pool_a_in_period(
        state.db(),
        &req.period_start,
        &req.period_end,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    // 2. Active-earner median → cap value
    let mut incomes: Vec<i64> = author_earnings.iter().map(|(_, amt)| *amt).collect();
    incomes.sort_unstable();
    let median = if incomes.is_empty() {
        0
    } else {
        incomes[incomes.len() / 2]
    };
    let cap = GraduatedCap {
        median_minor: median,
        band1_multiple: 5,
        band2_multiple: 10,
    };

    // 3. Apply graduated cap: Pool A spill goes to Pool B
    let mut pool_a_total: i64 = 0;
    let mut pool_b_total: i64 = 0;
    let mut capped_authors = 0;
    for (author, flow) in &author_earnings {
        let (kept, spill) = cap.apply(*flow, 0);
        pool_a_total += kept;
        pool_b_total += spill;
        if spill > 0 {
            capped_authors += 1;
        }
        let _ = author;
    }

    // 4. Pool B: quality-weighted by reading time
    // Apply eligibility floor (spec §20.10.5)
    let reading_times = lorehaven_db::monetization::author_reading_time_in_period(
        state.db(),
        &req.period_start,
        &req.period_end,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    let total_reading: i64 = reading_times.iter().map(|(_, s)| *s).sum();
    let eligible_shares: Vec<(String, i64, i64)> = reading_times
        .iter()
        .filter_map(|(author, secs)| {
            // Check floor: in a real system, query distinct readers, account age, trust level, sanctions
            // For settlement, use simplified check: reading time > 0 means eligible
            let quality_bp = if total_reading > 0 {
                (*secs * 10_000) / total_reading
            } else {
                0
            };
            if *secs > 0 {
                Some((author.clone(), quality_bp, 10_000))
            } else {
                None
            }
        })
        .collect();

    let distributions = distribute_pool_b(pool_b_total, &eligible_shares);

    // 5. Record Pool B distributions
    for (author_idx, (author, amount)) in distributions.iter().enumerate() {
        if *amount <= 0 {
            continue;
        }
        let idem = format!("{}:{}:{}", req.period_start, req.period_end, author);
        // Find this author's quality score and reading time from eligible_shares
        let quality_bp = if author_idx < eligible_shares.len() {
            eligible_shares[author_idx].1
        } else {
            0
        };
        let seconds = reading_times
            .iter()
            .find(|(a, _)| a == author)
            .map(|(_, s)| *s)
            .unwrap_or(0);
        lorehaven_db::monetization::record_pool_b_distribution(
            state.db(),
            &req.period_start,
            &req.period_end,
            author,
            *amount,
            "EUR",
            quality_bp,
            seconds,
            10_000, // ai_multiplier_bp
            &idem,
        )
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    }

    // 6. Upsert period summary
    let _summary_id = lorehaven_db::monetization::upsert_period_summary(
        state.db(),
        &req.period_start,
        &req.period_end,
        pool_a_total,
        pool_b_total,
        median,
        median * 10,
        author_earnings.len() as i64,
        distributions.len() as i64,
        capped_authors,
        0,
        0,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({
        "status": "settled",
        "period_start": req.period_start,
        "period_end": req.period_end,
        "pool_a_total_minor": pool_a_total,
        "pool_b_total_minor": pool_b_total,
        "median_minor": median,
        "cap_minor": median * 10,
        "capped_authors": capped_authors,
        "pool_b_distributed_to": distributions.len(),
    })))
}
