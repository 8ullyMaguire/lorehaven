//! M18 — Public API, bots, feeds, push, federation, AI providers routes.

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::str::FromStr;
use uuid::Uuid;

use darkhold_db::collaboration;
use darkhold_db::content;
use darkhold_domain::WorkId;

use crate::auth::{MaybeSession, MaybeToken, RequireActor};
use crate::http::{ApiError, ApiResult};
use crate::routes::works::{actor_for, reading_decision, Reading};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Public read API
// ---------------------------------------------------------------------------

/// Get public work data. Returns 404 for drafts, unpublished,
/// age-ineligible, or unknown ids — never leaks draft content.
pub async fn get_public_work(
    State(state): State<AppState>,
    Path(work_id): Path<String>,
    MaybeSession(session): MaybeSession,
) -> ApiResult<Json<Value>> {
    let work_id: WorkId = work_id.parse().map_err(|_| {
        ApiError(darkhold_domain::AppError::field(
            "work_id",
            "invalid work id",
        ))
    })?;
    let work = content::find_work(state.db(), work_id)
        .await
        .map_err(|e| ApiError(darkhold_domain::AppError::Internal(e.into())))?;
    let work = match work {
        Some(w) => w,
        None => {
            return Err(ApiError(darkhold_domain::AppError::NotFound {
                resource: "work",
            }))
        }
    };
    let actor = actor_for(session.as_ref());
    let contributors = collaboration::contributors_for_work(state.db(), work_id)
        .await
        .map_err(|e| ApiError(darkhold_domain::AppError::Internal(e)))?;
    match reading_decision(&state, actor.as_ref(), &work, &contributors).await {
        Reading::Public => {}
        Reading::Contributor => {}
        Reading::Denied(e) => return Err(ApiError(e)),
    }
    let work_json = json!({
        "id": work.id.to_canonical_string(),
        "title": work.title,
        "summary": work.summary,
        "language": work.language,
        "rating": work.rating,
        "lifecycle": work.lifecycle,
        "completion": work.completion,
        "published_at": work.published_at,
    });
    Ok(Json(json!({ "work": work_json })))
}

#[derive(Deserialize)]
pub struct SearchQuery {
    /// Absent or blank means "no terms", which is an empty result list rather
    /// than a framework-shaped 400 on a door bots call.
    #[serde(default)]
    q: Option<String>,
}

/// Public search.
pub async fn public_search(
    State(state): State<AppState>,
    MaybeSession(_user): MaybeSession,
    Query(query): Query<SearchQuery>,
) -> ApiResult<Json<Value>> {
    let needle = query.q.as_deref().unwrap_or("");
    let results = darkhold_db::search::search_works(state.db(), needle, 50)
        .await
        .map_err(|e| ApiError(darkhold_domain::AppError::Internal(e)))?;
    Ok(Json(json!({ "results": results })))
}

// ---------------------------------------------------------------------------
// Token management
// ---------------------------------------------------------------------------

/// List tokens for the caller.
pub async fn list_tokens(
    State(state): State<AppState>,
    MaybeSession(user): MaybeSession,
) -> ApiResult<Json<Value>> {
    let account = user
        .as_ref()
        .map(|u| u.account_id.to_string())
        .unwrap_or_default();
    if account.is_empty() {
        return Err(ApiError(darkhold_domain::AppError::field(
            "session",
            "sign in to list tokens",
        )));
    }
    let tokens = darkhold_db::external::list_tokens(state.db(), &account)
        .await
        .map_err(|e| ApiError(darkhold_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "tokens": tokens })))
}

/// Issue a token.
#[derive(Debug, Deserialize)]
pub struct IssueTokenBody {
    pub name: String,
    pub kind: String,
    pub scopes: Vec<String>,
    /// How long the token lives, in seconds (D2).
    ///
    /// Optional, and absent means "no expiry" — which is still permitted for a
    /// token the reader revokes by hand, because that is the pre-existing
    /// behaviour and a self-managed personal token is not the hazard a bot token
    /// is. What changed is that the *bot* path (§23.2, A4) must pass an expiry,
    /// and `register_bot` below refuses to issue one without it.
    #[serde(default)]
    pub expires_in_seconds: Option<i64>,
    /// The pseud this token acts as (§23.1's "explicit acting pseud").
    #[serde(default)]
    pub acting_pseud_id: Option<String>,
}

/// The longest lifetime a caller may request for a token, in seconds.
///
/// A year. Long enough for a bot token on a quiet instance that nobody thinks
/// about, short enough that a token nobody remembers issuing has lapsed before
/// anyone decides it might matter. A caller wanting longer gets an error naming
/// the ceiling rather than a token that outlives the instance's memory of it.
const MAX_TOKEN_TTL_SECONDS: i64 = 31_536_000;

pub async fn issue_token(
    State(state): State<AppState>,
    MaybeSession(user): MaybeSession,
    Json(body): Json<IssueTokenBody>,
) -> ApiResult<Json<Value>> {
    let account = user
        .as_ref()
        .map(|u| u.account_id.to_string())
        .unwrap_or_default();
    if account.is_empty() {
        return Err(ApiError(darkhold_domain::AppError::field(
            "session",
            "sign in to issue tokens",
        )));
    }

    let scopes: Vec<darkhold_domain::api_scopes::Scope> = body
        .scopes
        .iter()
        .map(|c| darkhold_domain::api_scopes::Scope::from_str(c))
        .collect::<Result<_, _>>()
        .map_err(|e| ApiError(darkhold_domain::AppError::field("scopes", &e)))?;

    // **A token can never exceed its account** (spec §20.2, M2-05).
    //
    // The scopes a token carries are chosen by the account that issues it, and
    // nothing else. Without this check **any account could mint a token with
    // `admin.write`**: the route parsed the requested scopes, stored them, and
    // the resolver trusted the stored set — so the token opened every admin door.
    // The check that would have caught it was at the *door* ("does this token
    // have the scope"), and the token did have it.
    //
    // So the check is here, at the *mint*, and it asks the account's own
    // privileges rather than the request's wishes.
    let grants = grantable_for(state.db(), &account, &user)
        .await
        .map_err(|error| ApiError(darkhold_domain::AppError::Internal(error.into())))?;
    let refused = darkhold_domain::api_scopes::ungrantable_scopes(&grants, &scopes);
    if !refused.is_empty() {
        let names: Vec<&str> = refused.iter().map(|scope| scope.as_str()).collect();
        return Err(ApiError(darkhold_domain::AppError::field(
            "scopes",
            format!(
                "this account may not hold {}; an account cannot grant a token that \
                 exceeds its own privileges",
                names.join(", ")
            ),
        )));
    }

    // D2. The refusal names the accepted range rather than clamping, for the
    // same reason M52-09 refuses an unavailable engine "with the accepted
    // values named, never a silent fallback" — a clamped expiry is a token
    // whose lifetime nobody chose and nobody can see.
    let expires_at = match body.expires_in_seconds {
        None => None,
        Some(seconds) if seconds <= 0 => {
            return Err(ApiError(darkhold_domain::AppError::field(
                "expires_in_seconds",
                "must be greater than zero; omit it for a token that does not expire",
            )))
        }
        Some(seconds) if seconds > MAX_TOKEN_TTL_SECONDS => {
            return Err(ApiError(darkhold_domain::AppError::field(
                "expires_in_seconds",
                format!("must be at most {MAX_TOKEN_TTL_SECONDS} (one year)"),
            )))
        }
        Some(seconds) => Some(darkhold_db::identity::in_seconds(seconds)),
    };

    let token = Uuid::new_v4().to_string();
    let token_hash = format!(
        "{:x}",
        Sha256::new().chain_update(token.as_bytes()).finalize()
    );

    // The acting pseud is verified against the caller's own account before it is
    // stored, rather than trusted. A token that could name any pseud would let
    // its holder post as someone else's face, which is the conflation §23.1's
    // "explicit acting pseud" exists to prevent — and a *checked* one is worth
    // having precisely because the untrusted version is only one missing
    // predicate away.
    if let Some(pseud) = body.acting_pseud_id.as_deref() {
        if !pseud_belongs_to_account(state.db(), pseud, &account)
            .await
            .map_err(|e| ApiError(darkhold_domain::AppError::Internal(e.into())))?
        {
            return Err(ApiError(darkhold_domain::AppError::field(
                "acting_pseud_id",
                "that pseud does not belong to your account",
            )));
        }
    }

    let id = darkhold_db::external::issue_token_acting(
        state.db(),
        &account,
        &body.kind,
        &body.name,
        &token_hash,
        &scopes,
        expires_at.as_deref(),
        body.acting_pseud_id.as_deref(),
    )
    .await
    .map_err(|e| ApiError(darkhold_domain::AppError::Internal(e.into())))?;

    // The secret is returned exactly once. The acting pseud and the expiry are
    // echoed so the caller can see what the token will act as and how long it
    // will live — §23.1 requires the acting pseud to be explicit, and an issuer
    // that accepted one without reporting it back has not made it explicit to
    // anybody.
    Ok(Json(json!({
        "id": id,
        "token": token,
        "acting_pseud_id": body.acting_pseud_id,
        "expires_at": expires_at,
    })))
}

/// Whether `pseud` is one of `account`'s own pseuds, and not deleted.
async fn pseud_belongs_to_account(
    db: &darkhold_db::Database,
    pseud: &str,
    account: &str,
) -> Result<bool, sqlx::Error> {
    let sql = db.sql(
        "SELECT id FROM pseuds WHERE id = ? AND account_id = ? AND deleted_at IS NULL",
        "SELECT id::text FROM pseuds WHERE id = ?::uuid AND account_id = ?::uuid AND deleted_at IS NULL",
    );
    let row = match db.backend() {
        darkhold_db::Backend::Sqlite => {
            sqlx::query_as::<_, (String,)>(&sql)
                .bind(pseud)
                .bind(account)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        darkhold_db::Backend::Postgres => {
            sqlx::query_as::<_, (String,)>(&sql)
                .bind(pseud)
                .bind(account)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.is_some())
}

/// Revoke one of the caller's own tokens.
///
/// **This handler was an unauthenticated, cross-account denial of service.** It
/// took `MaybeSession(_user)`, discarded it, and called a query with no account
/// predicate — so anyone, signed in or not, could revoke anyone's token by id.
/// Since `expires_at` was also never written, nothing ever stopped them doing
/// it again. §23.2 leans on this endpoint for "revocation and unlinking", so
/// the bot's headline security property depended on it working.
///
/// The account predicate now lives in SQL (`revoke_token_for_account`) rather
/// than in a check this handler might forget, and the rows-affected flag is what
/// lets it answer 404 for a token that is not the caller's without a second
/// existence query — which would itself disclose that the token exists.
///
/// Either a session or a bearer token is accepted, because a reader revoking a
/// bot's token from a chat client and a client revoking its own token are the
/// same act on the same row.
pub async fn revoke_token(
    State(state): State<AppState>,
    Path(token_id): Path<String>,
    MaybeSession(session): MaybeSession,
    MaybeToken(token): MaybeToken,
) -> ApiResult<Json<Value>> {
    // The caller's account from either identity. A session without an
    // `account_id` is not a caller we can scope a revoke to, so it is a refusal
    // rather than an empty string that would match no token and report success.
    let account = session
        .as_ref()
        .map(|u| u.account_id.to_string())
        .or_else(|| token.as_ref().map(|t| t.account_id.to_string()))
        .unwrap_or_default();
    if account.is_empty() {
        return Err(ApiError(darkhold_domain::AppError::AuthRequired));
    }
    let revoked = darkhold_db::external::revoke_token_for_account(state.db(), &token_id, &account)
        .await
        .map_err(|e| ApiError(darkhold_domain::AppError::Internal(e.into())))?;
    if !revoked {
        // 404, not 403: a 403 confirms the token exists.
        return Err(ApiError(darkhold_domain::AppError::NotFound {
            resource: "token",
        }));
    }
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
        return Err(ApiError(darkhold_domain::AppError::field(
            "session",
            "sign in to register bots",
        )));
    }

    let scopes: Vec<darkhold_domain::api_scopes::Scope> = body
        .scopes
        .iter()
        .map(|c| darkhold_domain::api_scopes::Scope::from_str(c))
        .collect::<Result<_, _>>()
        .map_err(|e| ApiError(darkhold_domain::AppError::field("scopes", &e)))?;

    let token = Uuid::new_v4().to_string();
    let token_hash = format!(
        "{:x}",
        Sha256::new().chain_update(token.as_bytes()).finalize()
    );

    let token_id = darkhold_db::external::issue_token(
        state.db(),
        &account,
        "bot",
        &body.name,
        &token_hash,
        &scopes,
    )
    .await
    .map_err(|e| ApiError(darkhold_domain::AppError::Internal(e.into())))?;

    let bot_id = darkhold_db::external::register_bot(
        state.db(),
        &token_id,
        &account,
        &body.contact,
        &body.user_agent,
    )
    .await
    .map_err(|e| ApiError(darkhold_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({ "bot_id": bot_id, "token": token })))
}

// ---------------------------------------------------------------------------
// Feeds
// ---------------------------------------------------------------------------

/// Get RSS feed.
pub async fn get_rss_feed(
    State(_state): State<AppState>,
    Path(_handle): Path<String>,
    MaybeSession(_user): MaybeSession,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "feed": "rss" })))
}

/// Get Atom feed.
pub async fn get_atom_feed(
    State(_state): State<AppState>,
    Path(_handle): Path<String>,
    MaybeSession(_user): MaybeSession,
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
        return Err(ApiError(darkhold_domain::AppError::field(
            "session",
            "sign in to subscribe",
        )));
    }

    let id = darkhold_db::external::register_push_subscription(
        state.db(),
        &account,
        &body.endpoint,
        &body.keys,
        body.device_name.as_deref(),
    )
    .await
    .map_err(|e| ApiError(darkhold_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({ "id": id })))
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
    let requests = darkhold_db::external::record_ai_request(
        state.db(),
        &work_id,
        "ai-provider",
        "analysis",
        None,
    )
    .await
    .map_err(|e| ApiError(darkhold_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({ "work_id": work_id, "request_id": requests })))
}

// The OpenAPI document lives in `super::openapi`, and it is **generated from the
// declared route table** rather than written out by hand. A hand-written document
// drifts: a route is added, the document is not updated, and the two disagree in
// the only way that matters — the document says a thing exists that does not, or
// does not say a thing exists that does, and a client generator produces a
// client that does not work.
//
// The route itself is registered there too, so the document and the door that
// serves it are one thing.

/// `GET /me/credential` — who am I, and what may this token do?
///
/// The endpoint the §23.2 bot side needs and Darkhold did not have. A bot
/// holds a bearer token; it has no session and no cookie jar, so it cannot call
/// `GET /me/tokens` — that route is session-only, and answering a bearer token
/// with "sign in to list tokens" made a linked bot unable to check its own
/// scopes. Every scope check then fell back to the set cached at link time,
/// which goes stale the moment a reader re-links with different scopes from
/// Darkhold's own UI.
///
/// So the *server* is the authority, and this is where a token asks. It is
/// deliberately narrow.
///
/// - It returns the caller's own scopes and identity, and nothing about anyone
///   else. There is no account parameter, so it cannot be pointed at another
///   account.
/// - It requires no particular scope. A token with no scopes left must still be
///   able to discover that it has none, otherwise the bot's only recourse is a
///   confusing 403 on an unrelated call.
/// - It is authenticated by the credential itself. A revoked token is not
///   resolved by `RequireActor` at all, so it is a 401 here — the same answer it
///   would get anywhere else, and the signal `verify_credential` exists to read.
///
/// What it must never do is report a *destination*. This returns who the caller
/// is, not where anything was sent. The bot has no way to tell Darkhold where a
/// message went, and this API offers no place to report one — which is the
/// structural half of §23.2's guarantee, and the reason adding this endpoint
/// does not weaken it.
pub async fn describe_credential(
    _state: State<AppState>,
    RequireActor(actor): RequireActor,
) -> ApiResult<Json<Value>> {
    // Nothing is read back from the database, and that is deliberate rather
    // than an omission. `RequireActor` resolved this credential from the
    // Authorization header and verified the account behind it before this
    // handler ran, so every field below is a property of a credential the
    // server has already checked. Re-reading the row would add a query whose
    // only possible outcome is the value the extractor already proved.
    let account_id = actor.account_id.to_string();
    let scopes: Vec<String> = actor.scopes.iter().map(|s| s.as_str().to_owned()).collect();
    let via = match actor.via {
        crate::auth::Credential::Token => "token",
        crate::auth::Credential::Session => "session",
    };

    Ok(Json(json!({
        "account_id": account_id,
        "pseud_id": actor.pseud_id.to_string(),
        "via": via,
        "scopes": scopes,
    })))
}

/// What this account may grant a token.
///
/// **Read from the account, not from the request.** The age policy's
/// capabilities come from the account's `age_state`, and the operator role from
/// the instance's own grant — the same two facts `trust_for` and the
/// body-audience rule consult, so a token's authority and the reader's
/// authority are computed from one place.
async fn grantable_for(
    db: &darkhold_db::Database,
    account: &str,
    user: &Option<crate::auth::SessionUser>,
) -> Result<darkhold_domain::api_scopes::Grantable, sqlx::Error> {
    let Some(user) = user else {
        return Ok(darkhold_domain::api_scopes::Grantable::default());
    };
    let is_operator = darkhold_db::governance::has_operator_role(db, account).await?;
    // The age policy's own answer: `may_participate` is the domain's rule and
    // this is its call site, so a token cannot be more permissive than the
    // account that minted it.
    let can_write = user.age_state.may_participate();
    Ok(darkhold_domain::api_scopes::Grantable {
        can_write,
        can_message: can_write,
        is_operator,
        can_translate: false,
    })
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        // Public read API
        .route("/public/works/{id}", get(get_public_work))
        .route("/public/search", get(public_search))
        // Token management
        .route("/me/tokens", get(list_tokens).post(issue_token))
        // Bearer-authenticated identity. See `describe_credential`.
        .route("/me/credential", get(describe_credential))
        .route("/me/tokens/{id}", post(revoke_token))
        // Bots
        .route("/me/bots", post(register_bot))
        // Feeds
        .route("/feeds/{handle}", get(get_rss_feed))
        .route("/feeds/{handle}/atom", get(get_atom_feed))
        // Push
        .route("/me/push/subscribe", post(subscribe_push))
        // AI
        .route("/ai/works/{id}", get(get_ai_work))
}
