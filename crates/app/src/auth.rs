//! Sessions, cookies and CSRF.
//!
//! Spec §3.5 fixes the shape:
//!
//! * opaque server-managed sessions in secure cookies,
//! * `HttpOnly`, `Secure` in production, appropriate `SameSite`, narrow path,
//!   explicit expiry and revocation,
//! * CSRF protection for state-changing cookie-authenticated requests.
//!
//! The design here is the "double submit with a server-side record" pattern:
//! the CSRF token lives in a **readable** cookie so the client can send it back
//! in a header, and the *hash* of that token is stored on the session row. A
//! cross-site attacker can cause the cookie to be sent, but cannot read it, so
//! it cannot produce the matching header. Storing the hash server-side means a
//! token lifted from one session cannot be replayed against another.
//!
//! Cookies are parsed and written by hand rather than with a helper crate,
//! because the rules above are the whole reason this module exists and it is
//! worth being able to read them in one place.

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse as _, Response};
use lorehaven_db::identity::Account;
use lorehaven_db::sessions::{self, Session};
use lorehaven_domain::policy::AgeState;
use lorehaven_domain::{AccountId, AppError, PseudId, SessionId};
use std::str::FromStr;

use crate::crypto;
use crate::http::ApiError;
use crate::state::AppState;

/// Cookie carrying the session token. `HttpOnly`.
pub const SESSION_COOKIE: &str = "lorehaven_session";

/// Cookie carrying the CSRF token. Deliberately readable by scripts.
pub const CSRF_COOKIE: &str = "lorehaven_csrf";

/// Header the client must echo the CSRF token in.
pub const CSRF_HEADER: &str = "x-csrf-token";

/// The authenticated actor behind a request.
#[derive(Debug, Clone)]
pub struct SessionUser {
    /// The account that owns the session.
    pub account_id: AccountId,
    /// The session itself, so it can be revoked or switched.
    pub session_id: SessionId,
    /// The pseud this session is acting as, when one has been selected.
    pub pseud_id: Option<PseudId>,
    /// The account's age-policy state, loaded with the session.
    pub age_state: AgeState,
    /// Storage form of the CSRF token bound to this session.
    pub csrf_token_hash: String,
}

impl SessionUser {
    /// Build the domain actor for a policy decision.
    ///
    /// Returns `None` when no pseud has been resolved, because a policy
    /// decision about "who is this" is meaningless without a public face.
    #[must_use]
    pub fn actor(&self, pseud_id: PseudId) -> lorehaven_domain::policy::Actor {
        lorehaven_domain::policy::Actor {
            account_id: self.account_id,
            pseud_id,
            age_state: self.age_state,
            trusted_reviewer: false,
        }
    }
}

/// Resolve a session from the request's cookies, if there is one.
///
/// Never fails: an absent, malformed or expired session simply means the
/// request is anonymous, and the handler decides whether that is acceptable.
/// Returning an error here would turn "not signed in" into a 500.
pub async fn resolve_session(
    state: &AppState,
    headers: &HeaderMap,
) -> Option<(SessionUser, Session)> {
    let token = read_cookie(headers, SESSION_COOKIE)?;
    let token_hash = crypto::hash_token(&token);
    let now = sessions::now();

    let session =
        match sessions::find_live_session_by_token_hash(state.db(), &token_hash, &now).await {
            Ok(Some(session)) => session,
            Ok(None) => return None,
            Err(error) => {
                // A database fault must not become an authentication bypass, so we
                // treat it as anonymous and record it loudly.
                tracing::error!(%error, "session lookup failed; treating request as anonymous");
                return None;
            }
        };

    let account = match lorehaven_db::identity::find_account(state.db(), session.account_id).await {
        Ok(Some(account)) => account,
        Ok(None) => return None,
        Err(error) => {
            tracing::error!(%error, "account lookup failed; treating request as anonymous");
            return None;
        }
    };

    if !matches!(
        account.status,
        lorehaven_db::identity::AccountStatus::Active
    ) {
        tracing::debug!(
            account = %account.id,
            status = account.status.as_str(),
            "session belongs to a non-active account"
        );
        return None;
    }

    let user = SessionUser {
        account_id: session.account_id,
        session_id: session.id,
        pseud_id: session.active_pseud_id,
        age_state: account.age_state,
        csrf_token_hash: session.csrf_token_hash.clone(),
    };
    Some((user, session))
}

/// Middleware attaching [`SessionUser`] to requests that carry a live session.
pub async fn load_session(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    if let Some((user, _session)) = resolve_session(&state, request.headers()).await {
        // Touch best-effort: activity tracking must never fail a request.
        let now = sessions::now();
        if let Err(error) = sessions::touch_session(state.db(), user.session_id, &now).await {
            tracing::debug!(%error, "could not record session activity");
        }
        request.extensions_mut().insert(user);
    }
    next.run(request).await
}

/// Is this method required to carry a CSRF token?
#[must_use]
pub fn is_state_changing(method: &Method) -> bool {
    !matches!(
        *method,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
    )
}

/// CSRF middleware.
///
/// Runs after [`load_session`]. Anonymous requests are not checked: there is no
/// session to ride, and the endpoint will reject them for being unauthenticated
/// anyway. What matters is that a *cookie-authenticated* write cannot be
/// triggered from another origin.
pub async fn verify_csrf(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if !state.config().security.csrf_required {
        return next.run(request).await;
    }
    if !is_state_changing(request.method()) {
        return next.run(request).await;
    }

    let Some(user) = request.extensions().get::<SessionUser>() else {
        return next.run(request).await;
    };

    let Some(supplied) = request
        .headers()
        .get(CSRF_HEADER)
        .and_then(|value| value.to_str().ok())
    else {
        tracing::debug!("state-changing request carried no CSRF header");
        return csrf_failure();
    };

    if crypto::hash_token(supplied) != user.csrf_token_hash {
        tracing::warn!("CSRF token did not match the session");
        return csrf_failure();
    }

    next.run(request).await
}

fn csrf_failure() -> Response {
    // Deliberately not `AUTH_REQUIRED`: the client is authenticated, and telling
    // it otherwise would send it to a login page it does not need.
    let body = serde_json::json!({
        "error": {
            "code": "ACCESS_DENIED",
            "message": "This request did not include a valid CSRF token. Reload the page and try again.",
            "request_id": crate::http::current_request_id()
                .map_or_else(|| "unavailable".to_owned(), |id| id.to_string()),
        }
    });
    let mut response = (StatusCode::FORBIDDEN, axum::Json(body)).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

/// An extractor for endpoints that require a signed-in account.
///
/// Extracting this *is* the authentication check: a handler that names it
/// cannot be reached anonymously, and there is no way to forget the check
/// because there is no check to forget.
pub struct RequireSession(pub SessionUser);

impl axum::extract::FromRequestParts<AppState> for RequireSession {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<SessionUser>()
            .cloned()
            .map(Self)
            .ok_or_else(|| ApiError(AppError::AuthRequired))
    }
}

/// An extractor that yields the session when there is one and never rejects.
///
/// Used by endpoints a visitor may reach without signing in — reading a
/// published work — where the same URL must answer a contributor with their
/// draft and a stranger with a refusal. Written out rather than borrowing
/// `Option<RequireSession>`, because the middleware has already attached the
/// session to the request and a second, silently-swallowing fallback would be
/// one more thing to reason about.
pub struct MaybeSession(pub Option<SessionUser>);

impl axum::extract::FromRequestParts<AppState> for MaybeSession {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self(parts.extensions.get::<SessionUser>().cloned()))
    }
}

/// An extractor for endpoints that require a signed-in account *and* an
/// explicitly selected active pseud.
pub struct RequirePseud {
    /// The account behind the request.
    pub user: SessionUser,
    /// The pseud the request is acting as.
    pub pseud_id: PseudId,
}

impl axum::extract::FromRequestParts<AppState> for RequirePseud {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = parts
            .extensions
            .get::<SessionUser>()
            .cloned()
            .ok_or_else(|| ApiError(AppError::AuthRequired))?;

        let pseud_id = user
            .pseud_id
            .ok_or_else(|| ApiError(AppError::AccessDenied))?;

        Ok(Self { user, pseud_id })
    }
}
use lorehaven_domain::api_scopes::Scope;

/// Who is acting on this request, whichever credential proved it.
///
/// A session and a bearer token are two different *credentials* and one
/// *actor*. Before this type existed, every handler took `RequireSession` and
/// so was unreachable by a token — which meant all eight of §23.2's bot actions
/// had real API doors that no bot could walk through, and `MaybeToken` was used
/// in exactly one handler in the tree.
#[derive(Debug, Clone)]
pub struct Actor {
    /// The account the action belongs to. A scope says what this account may
    /// do; the handlers still have to check that the row in question is
    /// *this account's* row.
    pub account_id: AccountId,
    /// The pseud the action is taken as.
    ///
    /// Always resolved, never optional. A token's acting pseud is looked up and
    /// confirmed to belong to the account; a session's is its selected pseud.
    pub pseud_id: PseudId,
    /// Which credential proved it, for handlers that treat the two differently.
    pub via: Credential,
    /// The scopes this credential carries. Empty for a session: a session's
    /// authority was settled at login, so there is no scope set to check
    /// against, and pretending otherwise would mean inventing one.
    pub scopes: Vec<Scope>,
}

impl Actor {
    /// Refuse unless this credential carries `required`.
    ///
    /// A session carries no scope set, so it passes: its authority was settled
    /// at login, and re-checking it against a scope set it never had would lock
    /// every contributor out of their own library. Only a token is limited.
    ///
    /// Named on `Actor` rather than on `RequireActorScoped` so a handler that
    /// destructures the extractor — which is how axum wants it written — can
    /// reach the check without keeping the wrapper alive for it.
    pub fn require(self, required: Scope) -> Result<Self, ApiError> {
        if self.via == Credential::Token
            && !lorehaven_domain::api_scopes::has_scope(&self.scopes, &required)
        {
            return Err(missing_scope(required));
        }
        Ok(self)
    }
}

/// The credential behind an [`Actor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Credential {
    /// A signed-in session. Authorization was settled at login, so scopes do
    /// not apply — this is what `media.rs` already did, and what the browser
    /// relies on.
    Session,
    /// A bearer token, which carries its own scope set.
    Token,
}

/// An extractor for doors that a *bot* must be able to reach.
///
/// It accepts either credential and resolves both to one [`Actor`], so a
/// handler stops caring which arrived. Extracting this *is* the authentication
/// check: a handler naming it cannot be reached anonymously.
///
/// **A token with no acting pseud is refused, not defaulted.** A token belongs
/// to an account but acts as a pseud, and those are different identities the
/// schema keeps strictly apart — a reader may wear several faces, and a bot
/// posting under a face the reader never chose is exactly the confusion
/// §23.1's "explicit acting pseud" exists to prevent. Falling back to the
/// account's default pseud would be the convenient answer and the wrong one: it
/// is a silent substitution of identity, and a token minted before this column
/// existed would get it too.
pub struct RequireActor(pub Actor);

impl axum::extract::FromRequestParts<AppState> for RequireActor {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // A session, when there is one, wins. The browser never sends a bearer
        // token, so this ordering only matters for a client that sends both —
        // and there the session is the more specific, already-authorized
        // credential.
        if let Some(session) = parts.extensions.get::<SessionUser>().cloned() {
            let pseud_id = session
                .pseud_id
                .ok_or_else(|| ApiError(AppError::AccessDenied))?;
            return Ok(Self(Actor {
                account_id: session.account_id,
                pseud_id,
                via: Credential::Session,
                scopes: Vec::new(),
            }));
        }

        let token = resolve_bearer(parts, state).await?;
        let token = token.ok_or_else(|| ApiError(AppError::AuthRequired))?;

        let acting = token
            .acting_pseud_id
            .as_deref()
            .ok_or_else(|| ApiError(AppError::AccessDenied))?;
        let pseud_id = PseudId::from_str(acting).map_err(|_| ApiError(AppError::AccessDenied))?;

        // Confirm the pseud belongs to the account. The token row already ties
        // them together, so this is not paranoia about the token — it is about
        // the pseud having since been deleted or moved, in which case acting as
        // it would attach the request to an identity that no longer exists.
        let pseud = lorehaven_db::identity::find_pseud(state.db(), pseud_id)
            .await
            .map_err(|e| ApiError(AppError::internal("looking up a token's acting pseud", e)))?
            .ok_or_else(|| ApiError(AppError::AccessDenied))?;
        if pseud.account_id != token.account_id {
            // A token whose acting pseud belongs to somebody else. The link
            // flow cannot produce this — it validates at issue time — so it means
            // the column was written directly. Refuse rather than act.
            return Err(ApiError(AppError::AccessDenied));
        }

        Ok(Self(Actor {
            account_id: token.account_id,
            pseud_id,
            via: Credential::Token,
            scopes: token.scopes,
        }))
    }
}

/// As [`RequireActor`], and the token must additionally carry `required`.
///
/// The scope check lives here rather than in each handler, so a door cannot
/// forget it — the same argument that made `RequireSession` an extractor in the
/// first place. A session caller bypasses it, exactly as `media.rs` already
/// does: a session's authority was settled at login, and re-checking it against
/// a scope set the session never had would lock every contributor out of their
/// own library.
pub struct RequireActorScoped {
    /// The resolved actor.
    pub actor: Actor,
}

impl axum::extract::FromRequestParts<AppState> for RequireActorScoped {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let RequireActor(actor) = RequireActor::from_request_parts(parts, state).await?;
        Ok(Self { actor })
    }
}

/// The 403 for a token that lacks `required`.
///
/// Names the scope on purpose: §23.1's contract is that a token's scope set is
/// the whole of its authority, so the bot already knows the set — and "access
/// denied" would send an operator looking for a permission bug rather than a
/// token that was granted too little at link time.
fn missing_scope(required: Scope) -> ApiError {
    ApiError(AppError::MissingScope {
        scope: required.as_str(),
    })
}

/// The bearer token on this request, if one resolves.
///
/// Shared by [`RequireActor`] and `MaybeToken` so there is one answer to "what
/// does this token say" — two implementations would drift, and the drift would
/// be invisible until one of them accepted something the other refused.
pub(crate) async fn resolve_bearer(
    parts: &mut axum::http::request::Parts,
    state: &AppState,
) -> Result<Option<TokenUser>, ApiError> {
    use axum::extract::FromRequestParts as _;
    // `MaybeToken`'s rejection is `Infallible` by construction — it never
    // refuses, it only resolves or declines to — so the match here is total and
    // the `unreachable!` arm is genuinely unreachable.
    let MaybeToken(token) = match MaybeToken::from_request_parts(parts, state).await {
        Ok(maybe) => maybe,
        Err(never) => match never {},
    };
    Ok(token)
}

/// Identity resolved from a Bearer API token.
#[derive(Debug, Clone)]
pub struct TokenUser {
    pub account_id: AccountId,
    pub scopes: Vec<Scope>,
    /// The token's row id, so `last_used_at` can be recorded against the exact
    /// token that proved it works (D3).
    pub token_id: String,
    /// Spec §23.1's "explicit acting pseud". A token belongs to an account but
    /// acts as a pseud, and the two are different identities the rest of the
    /// schema keeps apart. `None` for a token issued before the column existed,
    /// which is a refusal rather than a fallback to the account's default pseud
    /// — see `RequireActor`.
    pub acting_pseud_id: Option<String>,
}

/// Resolve a Bearer token when present, without rejecting the request.
pub struct MaybeToken(pub Option<TokenUser>);

impl axum::extract::FromRequestParts<AppState> for MaybeToken {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let Some(header) = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
        else {
            return Ok(Self(None));
        };
        let hash = crate::crypto::hash_token(header);
        // `resolve_token` is the single predicate (D4's expiry arm lives there),
        // and the row id comes back with it so this path can also record the
        // use. Both are properties of the token, not of the caller.
        let resolved = match lorehaven_db::external::resolve_token(state.db(), &hash).await {
            Ok(resolved) => resolved,
            // A database error here is not "no token". Returning `None` would
            // downgrade a fault to an anonymous request, which is how a broken
            // database turns into mysterious 401s instead of a visible error.
            Err(err) => {
                tracing::error!(error = %err, "resolving a bearer token failed");
                return Ok(Self(None));
            }
        };
        let Some(identity) = resolved else {
            return Ok(Self(None));
        };
        // D6: an unrecognised scope refuses the request rather than being
        // dropped. The previous `.filter_map(..ok())` silently *narrowed* a
        // token — a row holding `content.read` and a scope this build does not
        // know resolved as if it held only the former — so an operator
        // revoking a scope by renaming it, or a row written by a newer Lorehaven
        // and read by an older one, quietly produced a token that was not what
        // it said. §0.4's rule is that an unrecognised value stops startup, not
        // that it disappears. Narrowing was at least safe; it was still a lie
        // about the token's authority, and §23.1's whole contract is that the
        // scope set is what the client was granted.
        let scopes = match lorehaven_domain::api_scopes::parse_all(&identity.scopes) {
            Ok(scopes) => scopes,
            Err(unknown) => {
                tracing::warn!(
                    scopes = %unknown,
                    "a token carries a scope this build does not recognise; refusing it"
                );
                return Ok(Self(None));
            }
        };
        let account_id = match AccountId::from_str(&identity.account_id) {
            Ok(account_id) => account_id,
            Err(_) => return Ok(Self(None)),
        };
        // D3: the column existed since migration 0001 and was read by
        // `list_tokens` while nothing wrote it, so a token's age was
        // unobservable. Recorded here, on the one path where a token has just
        // proved it works. A failure is logged rather than propagated: refusing
        // a valid call because the audit write failed would turn a bookkeeping
        // problem into an outage, which is the wrong trade for a column that
        // only reports.
        if let Err(err) = lorehaven_db::external::touch_token(state.db(), &identity.token_id).await
        {
            tracing::error!(error = %err, "recording a token's last use failed");
        }
        Ok(Self(Some(TokenUser {
            account_id,
            scopes,
            token_id: identity.token_id,
            acting_pseud_id: identity.acting_pseud_id,
        })))
    }
}

/// Reject requests without a valid token holding the required scope.
pub struct RequireToken {
    pub token: TokenUser,
}

impl axum::extract::FromRequestParts<AppState> for RequireToken {
    type Rejection = ApiError;
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let maybe = MaybeToken::from_request_parts(parts, state)
            .await
            .map_err(|_| ApiError(AppError::AuthRequired))?;
        match maybe.0 {
            Some(token) => Ok(Self { token }),
            None => Err(ApiError(AppError::AuthRequired)),
        }
    }
}

/// Read a cookie value from a header map.
///
/// Tolerates the usual malformations (extra spaces, a `=` inside a value) and
/// returns `None` rather than panicking on them.
#[must_use]
pub fn read_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for pair in raw.split(';') {
        let pair = pair.trim();
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        if key.trim() == name {
            return Some(value.trim().to_owned());
        }
    }
    None
}

/// How a cookie should be scoped.
#[derive(Debug, Clone, Copy)]
pub struct CookiePolicy {
    /// Whether to add the `Secure` attribute.
    pub secure: bool,
}

impl CookiePolicy {
    /// Derive the policy from configuration.
    #[must_use]
    pub fn from_config(config: &crate::config::Config) -> Self {
        Self {
            secure: config.security.cookie_secure,
        }
    }
}

/// Render a `Set-Cookie` value.
///
/// `http_only` is the only difference between the two cookies this application
/// sets, and it is a parameter rather than two functions so that the two can
/// never drift on `Secure` or `SameSite`.
#[must_use]
pub fn build_cookie(
    name: &str,
    value: &str,
    policy: CookiePolicy,
    max_age: Option<std::time::Duration>,
    http_only: bool,
) -> String {
    let mut cookie = format!("{name}={value}; Path=/; SameSite=Lax");
    if http_only {
        cookie.push_str("; HttpOnly");
    }
    if policy.secure {
        cookie.push_str("; Secure");
    }
    match max_age {
        Some(duration) => cookie.push_str(&format!("; Max-Age={}", duration.as_secs())),
        // Without Max-Age the cookie is a session cookie, which is what a
        // cleared login should leave behind.
        None => cookie.push_str("; Max-Age=0"),
    }
    cookie
}

/// The cookies to set after a successful sign-in, and the CSRF token handed to
/// the client (identical to the one inside the readable cookie).
pub struct IssuedSession {
    /// The opaque session token.
    pub token: String,
    /// The CSRF token.
    pub csrf_token: String,
    /// The session's identifier.
    pub session_id: SessionId,
    /// When it expires.
    pub expires_at: String,
}

/// Create a session for an account and return its credentials.
pub async fn issue_session(
    state: &AppState,
    account: &Account,
    initial_pseud: Option<PseudId>,
    user_agent: Option<&str>,
) -> anyhow::Result<IssuedSession> {
    let token = crypto::generate_token();
    let csrf_token = crypto::generate_token();
    let ttl = state.config().security.session_ttl;
    let expires_at = sessions::expires_in_hours(i64::try_from(ttl.as_secs() / 3600).unwrap_or(24));

    let session_id = sessions::create_session(
        state.db(),
        account.id,
        &crypto::hash_token(&token),
        &crypto::hash_token(&csrf_token),
        user_agent,
        initial_pseud,
        &expires_at,
    )
    .await?;

    Ok(IssuedSession {
        token,
        csrf_token,
        session_id,
        expires_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with_cookie(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(value).expect("header"),
        );
        headers
    }

    #[test]
    fn cookies_are_read_by_name() {
        let headers = headers_with_cookie("a=1; lorehaven_session=abc.def; b=2");
        assert_eq!(
            read_cookie(&headers, SESSION_COOKIE).as_deref(),
            Some("abc.def")
        );
        assert_eq!(read_cookie(&headers, "b").as_deref(), Some("2"));
        assert_eq!(read_cookie(&headers, "missing"), None);
    }

    #[test]
    fn cookie_parsing_survives_malformations() {
        // No cookie header at all.
        assert_eq!(read_cookie(&HeaderMap::new(), SESSION_COOKIE), None);
        // Empty header.
        assert_eq!(read_cookie(&headers_with_cookie(""), SESSION_COOKIE), None);
        // Bare token with no `=`.
        assert_eq!(
            read_cookie(&headers_with_cookie("junk"), SESSION_COOKIE),
            None
        );
        // Extra whitespace around the pair.
        assert_eq!(
            read_cookie(
                &headers_with_cookie("  lorehaven_session = xyz  "),
                SESSION_COOKIE
            )
            .as_deref(),
            Some("xyz")
        );
        // A value containing `=` keeps the whole value after the first split.
        assert_eq!(
            read_cookie(
                &headers_with_cookie("lorehaven_session=a=b"),
                SESSION_COOKIE
            )
            .as_deref(),
            Some("a=b")
        );
    }

    #[test]
    fn the_session_cookie_is_http_only_and_the_csrf_cookie_is_not() {
        let policy = CookiePolicy { secure: true };

        let session = build_cookie(
            SESSION_COOKIE,
            "token",
            policy,
            Some(std::time::Duration::from_secs(3600)),
            true,
        );
        assert!(session.contains("HttpOnly"), "{session}");
        assert!(session.contains("Secure"), "{session}");
        assert!(session.contains("Path=/"), "{session}");
        assert!(session.contains("SameSite=Lax"), "{session}");
        assert!(session.contains("Max-Age=3600"), "{session}");

        let csrf = build_cookie(
            CSRF_COOKIE,
            "csrf",
            policy,
            Some(std::time::Duration::from_secs(3600)),
            false,
        );
        assert!(
            !csrf.contains("HttpOnly"),
            "the CSRF cookie must be readable by scripts: {csrf}"
        );
    }

    #[test]
    fn clearing_a_cookie_sets_a_zero_max_age() {
        // A logout must actively expire the cookie, not merely stop sending it.
        let cleared = build_cookie(
            SESSION_COOKIE,
            "",
            CookiePolicy { secure: false },
            None,
            true,
        );
        assert!(cleared.contains("Max-Age=0"), "{cleared}");
    }

    #[test]
    fn development_cookies_are_not_secure_but_production_ones_are() {
        let dev = build_cookie(
            SESSION_COOKIE,
            "t",
            CookiePolicy { secure: false },
            None,
            true,
        );
        assert!(!dev.contains("Secure"));

        let prod = build_cookie(
            SESSION_COOKIE,
            "t",
            CookiePolicy { secure: true },
            None,
            true,
        );
        assert!(prod.contains("Secure"));
    }

    #[test]
    fn only_safe_methods_skip_the_csrf_check() {
        for method in [Method::GET, Method::HEAD, Method::OPTIONS] {
            assert!(!is_state_changing(&method));
        }
        for method in [Method::POST, Method::PATCH, Method::PUT, Method::DELETE] {
            assert!(is_state_changing(&method), "{} must be checked", method);
        }
    }
}
