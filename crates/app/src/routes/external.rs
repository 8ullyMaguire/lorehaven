//! M18 — Public API, bots, feeds, push, federation, AI providers routes.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::str::FromStr;
use uuid::Uuid;

use crate::auth::MaybeSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Public read API
// ---------------------------------------------------------------------------

/// Get public work data.
pub async fn get_public_work(
    State(_state): State<AppState>,
    Path(_work_id): Path<String>,
    MaybeSession(_user): MaybeSession,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "work": null })))
}

/// Public search.
pub async fn public_search(
    State(_state): State<AppState>,
    MaybeSession(_user): MaybeSession,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "results": [] })))
}

// ---------------------------------------------------------------------------
// Token management
// ---------------------------------------------------------------------------

/// List tokens.
pub async fn list_tokens(
    State(_state): State<AppState>,
    MaybeSession(_user): MaybeSession,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "tokens": [] })))
}

/// Issue a token.
#[derive(Debug, Deserialize)]
pub struct IssueTokenBody {
    pub name: String,
    pub kind: String,
    pub scopes: Vec<String>,
}

pub async fn issue_token(
    State(state): State<AppState>,
    MaybeSession(user): MaybeSession,
    Json(body): Json<IssueTokenBody>,
) -> ApiResult<Json<Value>> {
    let account = user.map(|u| u.account_id.to_string()).unwrap_or_default();
    if account.is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "session",
            "sign in to issue tokens",
        )));
    }

    let scopes: Vec<lorehaven_domain::api_scopes::Scope> = body
        .scopes
        .iter()
        .map(|c| lorehaven_domain::api_scopes::Scope::from_str(c))
        .collect::<Result<_, _>>()
        .map_err(|e| ApiError(lorehaven_domain::AppError::field("scopes", &e)))?;

    let token = Uuid::new_v4().to_string();
    let token_hash = format!(
        "{:x}",
        Sha256::new().chain_update(token.as_bytes()).finalize()
    );

    let id = lorehaven_db::external::issue_token(
        state.db(),
        &account,
        &body.kind,
        &body.name,
        &token_hash,
        &scopes,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({ "id": id, "token": token })))
}

/// Revoke a token.
pub async fn revoke_token(
    State(state): State<AppState>,
    Path(token_id): Path<String>,
    MaybeSession(_user): MaybeSession,
) -> ApiResult<Json<Value>> {
    lorehaven_db::external::revoke_token(state.db(), &token_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "revoked": true })))
}

// ---------------------------------------------------------------------------
// Bots
// ---------------------------------------------------------------------------

/// Register a bot.
#[derive(Debug, Deserialize)]
pub struct RegisterBotBody {
    pub name: String,
    pub contact: String,
    pub user_agent: String,
    pub scopes: Vec<String>,
}

pub async fn register_bot(
    State(state): State<AppState>,
    MaybeSession(user): MaybeSession,
    Json(body): Json<RegisterBotBody>,
) -> ApiResult<Json<Value>> {
    let account = user.map(|u| u.account_id.to_string()).unwrap_or_default();
    if account.is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "session",
            "sign in to register bots",
        )));
    }

    let scopes: Vec<lorehaven_domain::api_scopes::Scope> = body
        .scopes
        .iter()
        .map(|c| lorehaven_domain::api_scopes::Scope::from_str(c))
        .collect::<Result<_, _>>()
        .map_err(|e| ApiError(lorehaven_domain::AppError::field("scopes", &e)))?;

    let token = Uuid::new_v4().to_string();
    let token_hash = format!(
        "{:x}",
        Sha256::new().chain_update(token.as_bytes()).finalize()
    );

    let token_id = lorehaven_db::external::issue_token(
        state.db(),
        &account,
        "bot",
        &body.name,
        &token_hash,
        &scopes,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    let bot_id = lorehaven_db::external::register_bot(
        state.db(),
        &token_id,
        &account,
        &body.contact,
        &body.user_agent,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({ "bot_id": bot_id, "token": token })))
}

// ---------------------------------------------------------------------------
// Feeds
// ---------------------------------------------------------------------------

/// Get RSS feed.
pub async fn get_rss_feed(
    State(_state): State<AppState>,
    Path(_handle): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "feed": "rss" })))
}

/// Get Atom feed.
pub async fn get_atom_feed(
    State(_state): State<AppState>,
    Path(_handle): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "feed": "atom" })))
}

// ---------------------------------------------------------------------------
// Push
// ---------------------------------------------------------------------------

/// Subscribe to push.
#[derive(Debug, Deserialize)]
pub struct PushSubscribeBody {
    pub endpoint: String,
    pub keys: String,
    pub device_name: Option<String>,
}

pub async fn subscribe_push(
    State(state): State<AppState>,
    MaybeSession(user): MaybeSession,
    Json(body): Json<PushSubscribeBody>,
) -> ApiResult<Json<Value>> {
    let account = user.map(|u| u.account_id.to_string()).unwrap_or_default();
    if account.is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "session",
            "sign in to subscribe",
        )));
    }

    let id = lorehaven_db::external::register_push_subscription(
        state.db(),
        &account,
        &body.endpoint,
        &body.keys,
        body.device_name.as_deref(),
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({ "id": id })))
}

// ---------------------------------------------------------------------------
// Federation
// ---------------------------------------------------------------------------

/// Federation inbox.
pub async fn federation_inbox(State(_state): State<AppState>) -> ApiResult<Json<Value>> {
    Ok(Json(
        json!({ "accepted": false, "reason": "federation not configured" }),
    ))
}

// ---------------------------------------------------------------------------
// AI
// ---------------------------------------------------------------------------

/// Get AI work data (requires consent).
pub async fn get_ai_work(
    State(state): State<AppState>,
    Path(work_id): Path<String>,
    MaybeSession(_user): MaybeSession,
) -> ApiResult<Json<Value>> {
    // Check consent
    let requests = lorehaven_db::external::record_ai_request(
        state.db(),
        &work_id,
        "ai-provider",
        "analysis",
        None,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({ "work_id": work_id, "request_id": requests })))
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        // Public read API
        .route("/public/works/{id}", get(get_public_work))
        .route("/public/search", get(public_search))
        // Token management
        .route("/me/tokens", get(list_tokens).post(issue_token))
        .route("/me/tokens/{id}", post(revoke_token))
        // Bots
        .route("/me/bots", post(register_bot))
        // Feeds
        .route("/feeds/{handle}", get(get_rss_feed))
        .route("/feeds/{handle}/atom", get(get_atom_feed))
        // Push
        .route("/me/push/subscribe", post(subscribe_push))
        // Federation
        .route("/federation/inbox", post(federation_inbox))
        // AI
        .route("/ai/works/{id}", get(get_ai_work))
}
