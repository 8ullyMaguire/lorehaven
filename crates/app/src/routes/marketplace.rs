//! M16 — Marketplace routes: listings, commissions, extensions, webhooks, gallery.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::str::FromStr;
use uuid::Uuid;

use crate::auth::MaybeSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

/// List listings.
pub async fn list_listings(
    State(state): State<AppState>,
    MaybeSession(_user): MaybeSession,
) -> ApiResult<Json<Value>> {
    let items = lorehaven_db::marketplace::list_listings(state.db(), None, None, 50)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "listings": items })))
}

/// Create a listing.
#[derive(Debug, Deserialize)]
pub struct CreateListingBody {
    pub kind: String,
    pub work_id: Option<String>,
    pub terms: Value,
}

pub async fn create_listing(
    State(state): State<AppState>,
    MaybeSession(user): MaybeSession,
    Json(body): Json<CreateListingBody>,
) -> ApiResult<Json<Value>> {
    let account = user.map(|u| u.account_id.to_string()).unwrap_or_default();
    if account.is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "session",
            "sign in to create listings",
        )));
    }

    let kind = lorehaven_domain::marketplace::ListingKind::from_str(&body.kind)
        .map_err(|e| ApiError(lorehaven_domain::AppError::field("kind", &e)))?;

    let terms = serde_json::to_string(&body.terms).unwrap();
    let id = lorehaven_db::marketplace::create_listing(
        state.db(),
        kind,
        &account,
        body.work_id.as_deref(),
        &terms,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({ "id": id })))
}

/// Create a commission.
#[derive(Debug, Deserialize)]
pub struct CreateCommissionBody {
    pub listing_id: String,
}

pub async fn create_commission(
    State(state): State<AppState>,
    MaybeSession(user): MaybeSession,
    Json(body): Json<CreateCommissionBody>,
) -> ApiResult<Json<Value>> {
    let account = user.map(|u| u.account_id.to_string()).unwrap_or_default();
    if account.is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "session",
            "sign in to request commissions",
        )));
    }

    let id = lorehaven_db::marketplace::create_commission(state.db(), &body.listing_id, &account)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({ "id": id })))
}

/// Transition a commission.
#[derive(Debug, Deserialize)]
pub struct TransitionCommissionBody {
    pub from_state: String,
    pub to_state: String,
    pub ledger_ref: Option<String>,
}

pub async fn transition_commission(
    State(state): State<AppState>,
    Path(commission_id): Path<String>,
    MaybeSession(_user): MaybeSession,
    Json(body): Json<TransitionCommissionBody>,
) -> ApiResult<Json<Value>> {
    let from = lorehaven_domain::marketplace::CommissionState::from_str(&body.from_state)
        .map_err(|e| ApiError(lorehaven_domain::AppError::field("from_state", &e)))?;
    let to = lorehaven_domain::marketplace::CommissionState::from_str(&body.to_state)
        .map_err(|e| ApiError(lorehaven_domain::AppError::field("to_state", &e)))?;

    lorehaven_db::marketplace::transition_commission(
        state.db(),
        &commission_id,
        &from,
        &to,
        body.ledger_ref.as_deref(),
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({ "success": true })))
}

/// List extensions submitted by the caller.
pub async fn list_extensions(
    State(state): State<AppState>,
    MaybeSession(user): MaybeSession,
) -> ApiResult<Json<Value>> {
    let account = user.map(|u| u.account_id.to_string()).unwrap_or_default();
    if account.is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "session",
            "sign in to list extensions",
        )));
    }
    let extensions = lorehaven_db::marketplace::list_extensions(state.db(), Some(&account))
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "extensions": extensions })))
}

/// Get a single extension by slug. Returns 404 for unknown slug.
pub async fn get_extension(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    MaybeSession(_user): MaybeSession,
) -> ApiResult<Json<Value>> {
    let ext = lorehaven_db::marketplace::get_extension(state.db(), &slug)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    match ext {
        Some(e) => Ok(Json(json!({ "extension": e }))),
        None => Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "extension",
        })),
    }
}

/// Grant an extension.
#[derive(Debug, Deserialize)]
pub struct GrantExtensionBody {
    pub manifest_id: String,
    pub version: String,
    pub capabilities: Vec<String>,
}

pub async fn grant_extension(
    State(state): State<AppState>,
    MaybeSession(user): MaybeSession,
    Json(body): Json<GrantExtensionBody>,
) -> ApiResult<Json<Value>> {
    let account = user.map(|u| u.account_id.to_string()).unwrap_or_default();
    if account.is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "session",
            "sign in to grant extensions",
        )));
    }

    let capabilities: Vec<lorehaven_domain::extension::Capability> = body
        .capabilities
        .iter()
        .map(|c| lorehaven_domain::extension::Capability::from_str(c))
        .collect::<Result<_, _>>()
        .map_err(|e| ApiError(lorehaven_domain::AppError::field("capabilities", &e)))?;

    lorehaven_db::marketplace::grant_extension(
        state.db(),
        &account,
        &body.manifest_id,
        &body.version,
        &capabilities,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({ "granted": true })))
}

/// Revoke an extension.
pub async fn revoke_extension(
    State(state): State<AppState>,
    Path(manifest_id): Path<String>,
    MaybeSession(user): MaybeSession,
) -> ApiResult<Json<Value>> {
    let account = user.map(|u| u.account_id.to_string()).unwrap_or_default();
    if account.is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "session",
            "sign in to revoke extensions",
        )));
    }

    lorehaven_db::marketplace::revoke_extension(state.db(), &account, &manifest_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({ "revoked": true })))
}

/// List my extension grants.
pub async fn list_my_grants(
    State(state): State<AppState>,
    MaybeSession(user): MaybeSession,
) -> ApiResult<Json<Value>> {
    let account = user.map(|u| u.account_id.to_string()).unwrap_or_default();
    if account.is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "session",
            "sign in to list grants",
        )));
    }
    let grants = lorehaven_db::marketplace::list_my_grants(state.db(), &account)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "grants": grants })))
}

/// List webhooks for the caller.
pub async fn list_webhooks(
    State(state): State<AppState>,
    MaybeSession(user): MaybeSession,
) -> ApiResult<Json<Value>> {
    let account = user.map(|u| u.account_id.to_string()).unwrap_or_default();
    if account.is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "session",
            "sign in to list webhooks",
        )));
    }
    let webhooks = lorehaven_db::marketplace::list_webhooks(state.db(), &account)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "webhooks": webhooks })))
}

/// Create a webhook.
#[derive(Debug, Deserialize)]
pub struct CreateWebhookBody {
    pub url: String,
    pub events: Vec<String>,
}

pub async fn create_webhook(
    State(state): State<AppState>,
    MaybeSession(user): MaybeSession,
    Json(body): Json<CreateWebhookBody>,
) -> ApiResult<Json<Value>> {
    let account = user.map(|u| u.account_id.to_string()).unwrap_or_default();
    if account.is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "session",
            "sign in to create webhooks",
        )));
    }

    let secret = format!("whsec_{}", Uuid::new_v4().to_string().replace('-', ""));
    let id = lorehaven_db::marketplace::create_webhook(
        state.db(),
        &account,
        &body.url,
        &secret,
        &body.events,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({ "id": id, "secret": secret })))
}

/// List gallery items for a work.
pub async fn list_gallery(
    State(state): State<AppState>,
    Path(work_id): Path<String>,
    MaybeSession(_user): MaybeSession,
) -> ApiResult<Json<Value>> {
    let items = lorehaven_db::marketplace::list_gallery_items(state.db(), &work_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "items": items })))
}

/// Add a gallery item.
#[derive(Debug, Deserialize)]
pub struct AddGalleryBody {
    pub media_type: String,
    pub storage_key: String,
    pub alt_text: String,
    pub sanitized_document: String,
}

pub async fn add_gallery_item(
    State(state): State<AppState>,
    Path(work_id): Path<String>,
    MaybeSession(user): MaybeSession,
    Json(body): Json<AddGalleryBody>,
) -> ApiResult<Json<Value>> {
    let account = user.map(|u| u.account_id.to_string()).unwrap_or_default();
    if account.is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "session",
            "sign in to add gallery items",
        )));
    }

    let id = lorehaven_db::marketplace::add_gallery_item(
        state.db(),
        &work_id,
        &account,
        &body.media_type,
        &body.storage_key,
        &body.alt_text,
        &body.sanitized_document,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({ "id": id })))
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/listings", get(list_listings).post(create_listing))
        .route("/listings/{id}/commissions", post(create_commission))
        .route("/commissions/{id}/transition", post(transition_commission))
        .route("/extensions", get(list_extensions))
        .route("/extensions/{slug}", get(get_extension))
        .route("/extensions/{slug}/grant", post(grant_extension))
        .route("/extensions/{slug}/revoke", post(revoke_extension))
        .route("/me/extension-grants", get(list_my_grants))
        .route("/me/webhooks", get(list_webhooks).post(create_webhook))
        .route(
            "/works/{id}/gallery",
            get(list_gallery).post(add_gallery_item),
        )
}
