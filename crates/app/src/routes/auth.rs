//! Authentication endpoints (spec §7).
//!
//! ```text
//! POST   /auth/register
//! POST   /auth/login
//! POST   /auth/logout
//! POST   /auth/password-reset
//! POST   /auth/password-reset/complete
//! POST   /auth/sessions/revoke-all
//! GET    /auth/sessions
//! DELETE /auth/sessions/:id
//! GET    /auth/me
//! ```
//!
//! Two rules run through the whole module:
//!
//! * **Never say whether an address has an account.** Registration is the one
//!   exception, and only because it must reject a duplicate. Everywhere else —
//!   login, password reset — the response is identical for "wrong password" and
//!   "no such person", and the password hash is verified even when there is no
//!   account, so the timing matches too.
//! * **Never log a credential.** Reset tokens are logged only in development,
//!   and only because there is no mail transport yet; see `password_reset`.

use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use lorehaven_db::identity::{self, AccountStatus, PrivacyScope};
use lorehaven_db::sessions::{self, RecoveryPurpose};
use lorehaven_domain::policy::AgeState;
use lorehaven_domain::{AccountId, AppError, PseudId, SessionId};
use serde::{Deserialize, Serialize};

use crate::auth::{
    self, CookiePolicy, MaybeSession, RequireSession, SessionUser, CSRF_COOKIE, CSRF_HEADER,
    SESSION_COOKIE,
};
use crate::http::{ApiError, ApiResult};
use crate::privacy;
use crate::state::AppState;
use crate::{crypto, version};

/// Authentication routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/password-reset", post(password_reset))
        .route(
            "/auth/password-reset/complete",
            post(password_reset_complete),
        )
        .route("/auth/sessions", get(list_sessions))
        .route("/auth/sessions/revoke-all", post(revoke_all_sessions))
        .route("/auth/sessions/{id}", delete(revoke_session))
        .route("/auth/me", get(me))
}

/// Minimum password length.
///
/// Twelve, not eight: this is a public-instance password that protects an
/// account and everything under it, and length is the only requirement that
/// reliably helps without pushing people towards a password manager they do not
/// have.
const MIN_PASSWORD_LENGTH: usize = 12;

/// Maximum password length, to bound the cost of hashing.
const MAX_PASSWORD_LENGTH: usize = 256;

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RegisterRequest {
    email: String,
    password: String,
    /// The handle for the first pseud.
    handle: String,
    /// Display name for the first pseud; defaults to the handle.
    #[serde(default)]
    display_name: Option<String>,
    /// `adult`, `minor` or `unknown`. Absent means `unknown`.
    ///
    /// Spec §7: avoid collecting full birth dates. A band is all the policy
    /// needs, and it is all we ask for.
    #[serde(default)]
    age_band: Option<String>,
}

#[derive(Debug, Serialize)]
struct AccountResponse {
    account: AccountView,
    /// What this account may currently do, so the interface does not have to
    /// guess or duplicate the policy.
    capabilities: Capabilities,
}

#[derive(Debug, Serialize)]
struct AccountView {
    id: AccountId,
    email: String,
    age_state: &'static str,
    /// Whether an address has been confirmed. Not enforced yet; reported
    /// honestly rather than implied.
    email_verified: bool,
    session_expires_at: String,
}

#[derive(Debug, Serialize)]
struct Capabilities {
    can_read: bool,
    can_write: bool,
    can_message: bool,
    can_be_listed: bool,
    /// Highest rating this account may be shown, after both policy and
    /// preference are applied.
    max_rating: &'static str,
    /// Plain-language explanation when something is unavailable.
    #[serde(skip_serializing_if = "Option::is_none")]
    restriction_note: Option<String>,
}

async fn register(
    State(state): State<AppState>,
    MaybeSession(_user): MaybeSession,
    headers: HeaderMap,
    Json(request): Json<RegisterRequest>,
) -> ApiResult<Response> {
    if !state.config().accounts.registration_open {
        return Err(ApiError(AppError::AccessDenied));
    }

    let email = normalise_email(&request.email)?;
    let age_state = resolve_age_state(&state, request.age_band.as_deref())?;
    validate_password(&request.password)?;
    let handle = validate_handle(&request.handle)?;
    let display_name = request
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&handle)
        .to_owned();
    validate_display_name(&display_name)?;

    // Duplicate address: the one place we disclose that an account exists,
    // because refusing is the only useful answer and no account is described.
    if identity::find_account_by_email(state.db(), &email)
        .await?
        .is_some()
    {
        return Err(ApiError(AppError::field(
            "email",
            "An account already uses this address.",
        )));
    }
    if identity::find_pseud_by_handle(state.db(), &handle)
        .await?
        .is_some()
    {
        return Err(ApiError(AppError::field(
            "handle",
            "That handle is already taken.",
        )));
    }

    let account_id =
        identity::create_account(state.db(), &email, age_state, AccountStatus::Active).await?;

    // A failure after this point would leave an account with no way to sign in,
    // so the credential is written immediately after creation and any later
    // failure is reported as a fault the operator can act on.
    let hash = crypto::hash_password(&request.password).map_err(|error| {
        ApiError(AppError::internal(
            "hashing the password for a new account",
            error,
        ))
    })?;
    identity::set_password_hash(state.db(), account_id, &hash).await?;

    let pseud_id = identity::create_pseud(state.db(), account_id, &handle, &display_name).await?;

    // Spec §7: privacy defaults are *stored* from onboarding, not computed
    // later. A failure here must not leave a half-configured account.
    store_onboarding_defaults(&state, account_id, pseud_id, age_state).await?;

    // Record the age declaration as an assessment. Not a verification — the
    // method string says which it is.
    record_age_assessment(&state, account_id, age_state).await?;

    let account = identity::find_account(state.db(), account_id)
        .await?
        .ok_or_else(|| {
            ApiError(AppError::Internal(anyhow::anyhow!(
                "account vanished after creation"
            )))
        })?;

    let issued = auth::issue_session(
        &state,
        &account,
        Some(pseud_id),
        user_agent(&headers).as_deref(),
    )
    .await?;

    tracing::info!(account = %account_id, age_state = ?age_state, "account registered");

    let capabilities = capabilities_for(&state, age_state, None);
    let body = AccountResponse {
        account: account_view(&account, &issued.expires_at),
        capabilities,
    };

    Ok(session_response(&state, StatusCode::CREATED, body, &issued))
}

// ---------------------------------------------------------------------------
// Login and logout
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

async fn login(
    State(state): State<AppState>,
    MaybeSession(_user): MaybeSession,
    headers: HeaderMap,
    Json(request): Json<LoginRequest>,
) -> ApiResult<Response> {
    let email = request.email.trim().to_ascii_lowercase();

    let account = identity::find_account_by_email(state.db(), &email).await?;

    // Always hash, even with no account, so a wrong address and a wrong
    // password take the same time.
    let stored = match &account {
        Some(account) => identity::password_hash(state.db(), account.id).await?,
        None => None,
    };
    let hash = stored.clone().unwrap_or_else(|| DUMMY_PHC.to_owned());
    let matches = crypto::verify_password(&request.password, &hash).unwrap_or(false);

    let Some(account) = account else {
        tracing::info!(address_hash = %crypto::hash_token(&email), "login for an unknown address");
        return Err(ApiError(AppError::InvalidCredentials));
    };
    if stored.is_none() {
        tracing::warn!(account = %account.id, "account has no password credential");
        return Err(ApiError(AppError::InvalidCredentials));
    }

    if !matches {
        tracing::info!(account = %account.id, "login with a wrong password");
        return Err(ApiError(AppError::InvalidCredentials));
    }
    if !matches!(account.status, AccountStatus::Active) {
        tracing::info!(account = %account.id, status = account.status.as_str(), "login to a non-active account");
        return Err(ApiError(AppError::AccessDenied));
    }

    // Sign in as the oldest pseud, so the session always has a face.
    let pseuds = identity::pseuds_for_account(state.db(), account.id).await?;
    let initial = pseuds.first().map(|pseud| pseud.id);

    let issued =
        auth::issue_session(&state, &account, initial, user_agent(&headers).as_deref()).await?;

    // Record the login for streak tracking (spec §9.7.1). Best-effort: a
    // streak failure must never block a successful sign-in.
    if let Err(error) =
        lorehaven_db::engagement::record_login(state.db(), &account.id.to_string()).await
    {
        tracing::warn!(account = %account.id, %error, "failed to record login streak");
    }

    let settings = sessions::content_settings(state.db(), account.id).await?;
    let capabilities = capabilities_for(&state, account.age_state, Some(&settings));

    let body = AccountResponse {
        account: account_view(&account, &issued.expires_at),
        capabilities,
    };
    Ok(session_response(&state, StatusCode::OK, body, &issued))
}

async fn logout(
    State(state): State<AppState>,
    RequireSession(_user): RequireSession,
    request: axum::extract::Request,
) -> ApiResult<Response> {
    // The CSRF middleware has already verified the token for this request, so
    // reaching here with a session means the request came from our own origin.
    if let Some(user) = request.extensions().get::<SessionUser>().cloned() {
        let now = sessions::now();
        sessions::revoke_session(state.db(), user.account_id, user.session_id, &now).await?;
    }

    // Revoking the row is the real logout; clearing the cookies is courtesy, and
    // is done unconditionally so a stale cookie does not linger after the row
    // it names has gone.
    let policy = CookiePolicy::from_config(state.config());
    let mut response = (StatusCode::NO_CONTENT, ()).into_response();
    append_cookie(
        response.headers_mut(),
        auth::build_cookie(SESSION_COOKIE, "", policy, None, true),
    );
    append_cookie(
        response.headers_mut(),
        auth::build_cookie(CSRF_COOKIE, "", policy, None, false),
    );
    Ok(response)
}

// ---------------------------------------------------------------------------
// Password reset
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct PasswordResetRequest {
    email: String,
}

#[derive(Debug, Serialize)]
struct PasswordResetStarted {
    /// Always the same message, whatever the address.
    message: &'static str,
    /// Present only outside production, and only because there is no mail
    /// transport yet. See the module note.
    #[serde(skip_serializing_if = "Option::is_none")]
    development_token: Option<String>,
}

async fn password_reset(
    State(state): State<AppState>,
    MaybeSession(_user): MaybeSession,
    Json(request): Json<PasswordResetRequest>,
) -> ApiResult<Json<PasswordResetStarted>> {
    let email = request.email.trim().to_ascii_lowercase();
    let mut development_token = None;

    if let Some(account) = identity::find_account_by_email(state.db(), &email).await? {
        let now = sessions::now();
        // A second request invalidates the first link, so a mailbox cannot
        // accumulate working resets.
        sessions::invalidate_recovery_tokens(
            state.db(),
            account.id,
            RecoveryPurpose::PasswordReset,
            &now,
        )
        .await?;

        let token = crypto::generate_token();
        let expires_at = sessions::expires_in_hours(sessions::recovery_ttl_hours(
            RecoveryPurpose::PasswordReset,
        ));
        sessions::create_recovery_token(
            state.db(),
            account.id,
            RecoveryPurpose::PasswordReset,
            &crypto::hash_token(&token),
            &expires_at,
        )
        .await?;

        if state.config().environment.is_production() {
            // There is no SMTP transport yet (spec §2.2 lists it as optional).
            // The operator gets the fact that a link was requested, never the
            // token itself.
            tracing::info!(
                account = %account.id,
                "password reset requested; mail transport is not configured"
            );
        } else {
            tracing::info!(
                account = %account.id,
                expires_at = %expires_at,
                "password reset token issued (development: returned in the response)"
            );
            development_token = Some(token);
        }
    } else {
        tracing::info!(
            address_hash = %crypto::hash_token(&email),
            "password reset requested for an unknown address"
        );
    }

    // Identical response either way.
    Ok(Json(PasswordResetStarted {
        message: "If that address has an account, a reset link is on its way.",
        development_token,
    }))
}

#[derive(Debug, Deserialize)]
struct PasswordResetComplete {
    token: String,
    new_password: String,
}

async fn password_reset_complete(
    State(state): State<AppState>,
    MaybeSession(_user): MaybeSession,
    Json(request): Json<PasswordResetComplete>,
) -> ApiResult<StatusCode> {
    validate_password(&request.new_password)?;

    let now = sessions::now();
    let account_id = sessions::consume_recovery_token(
        state.db(),
        RecoveryPurpose::PasswordReset,
        &crypto::hash_token(&request.token),
        &now,
    )
    .await?
    .ok_or_else(|| {
        // One message for "expired", "already used" and "never existed": the
        // client cannot act differently on any of them, and distinguishing them
        // tells an attacker which tokens were once valid.
        ApiError(AppError::field(
            "token",
            "That reset link is not valid. Request a new one.",
        ))
    })?;

    let hash = crypto::hash_password(&request.new_password)
        .map_err(|error| ApiError(AppError::internal("hashing a new password", error)))?;
    sessions::update_password_hash(state.db(), account_id, &hash, &now).await?;

    // A password change ends every session: that is what makes "I reset my
    // password" also mean "whoever was in my account is out".
    let revoked = sessions::revoke_all_sessions(state.db(), account_id, &now).await?;

    tracing::info!(account = %account_id, sessions_revoked = revoked, "password reset completed");

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct SessionView {
    id: SessionId,
    created_at: String,
    last_seen_at: String,
    expires_at: String,
    /// Coarse client description, for a human to recognise their own devices.
    device: String,
    /// Whether this is the session making the request.
    current: bool,
}

async fn list_sessions(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Vec<SessionView>>> {
    let now = sessions::now();
    let rows = sessions::live_sessions_for_account(state.db(), user.account_id, &now).await?;

    Ok(Json(
        rows.into_iter()
            .map(|session| SessionView {
                current: session.id == user.session_id,
                device: describe_device(session.user_agent.as_deref()),
                id: session.id,
                created_at: session.created_at,
                last_seen_at: session.last_seen_at,
                expires_at: session.expires_at,
            })
            .collect(),
    ))
}

async fn revoke_session(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> ApiResult<StatusCode> {
    let session_id: SessionId = id.parse().map_err(|_| {
        ApiError(AppError::NotFound {
            resource: "session",
        })
    })?;

    let now = sessions::now();
    let revoked = sessions::revoke_session(state.db(), user.account_id, session_id, &now).await?;

    // Scoped to the owner by the query itself, so a session belonging to
    // someone else is indistinguishable from one that does not exist.
    if !revoked {
        return Err(ApiError(AppError::NotFound {
            resource: "session",
        }));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn revoke_all_sessions(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<StatusCode> {
    let now = sessions::now();
    let revoked = sessions::revoke_all_sessions(state.db(), user.account_id, &now).await?;
    tracing::info!(account = %user.account_id, revoked, "all sessions revoked");
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize)]
struct MeResponse {
    account: AccountView,
    pseuds: Vec<PseudView>,
    active_pseud_id: Option<PseudId>,
    capabilities: Capabilities,
    /// The account's trust level (spec §19.1). Needed by the frontend to
    /// gate governance actions (§45) without a round-trip.
    trust_level: i64,
}

#[derive(Debug, Serialize)]
struct PseudView {
    id: PseudId,
    handle: String,
    display_name: String,
    bio: Option<String>,
    // Note the absence of `account_id`. Spec §7 acceptance: "Pseud linkage is
    // absent from public API responses." The active pseud is identified by the
    // separate `active_pseud_id` field on the parent response, so no client
    // ever needs the link — and there is no field here to leak it by accident.
}

async fn me(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<MeResponse>> {
    let account = identity::find_account(state.db(), user.account_id)
        .await?
        .ok_or_else(|| ApiError(AppError::AuthRequired))?;

    let pseuds = identity::pseuds_for_account(state.db(), user.account_id).await?;
    let settings = sessions::content_settings(state.db(), account.id).await?;

    // Fall back to the first pseud if the session's choice has been deleted,
    // rather than reporting the session as having none.
    let active = user
        .pseud_id
        .filter(|id| pseuds.iter().any(|pseud| pseud.id == *id))
        .or_else(|| pseuds.first().map(|pseud| pseud.id));

    let account_id = account.id.to_string();
    let trust_level = lorehaven_db::governance::trust_for(state.db(), &account_id)
        .await
        .unwrap_or(0);

    Ok(Json(MeResponse {
        account: account_view(&account, &sessions::now()),
        pseuds: pseuds
            .into_iter()
            .map(|pseud| PseudView {
                id: pseud.id,
                handle: pseud.handle,
                display_name: pseud.display_name,
                bio: pseud.bio,
            })
            .collect(),
        active_pseud_id: active,
        capabilities: capabilities_for(&state, account.age_state, Some(&settings)),
        trust_level,
    }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A real Argon2id hash used when no account matches, so verification cost does
/// not reveal whether an address exists. The password it encodes is irrelevant.
const DUMMY_PHC: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHR2YWx1ZQ$\
                          5VZ7Q0v2Dq0k2m5S0m3B2nqXk7wZ4l0p1y8t6r3c9uM";

/// Normalise and sanity-check an email address.
///
/// Deliberately permissive: one `@`, no whitespace, a dot in the domain. A
/// stricter pattern rejects valid addresses, and the only real verification is
/// sending mail, which is not implemented yet — so we do not pretend otherwise.
pub(crate) fn normalise_email(raw: &str) -> Result<String, ApiError> {
    let email = raw.trim().to_ascii_lowercase();
    if email.len() > 320 {
        return Err(ApiError(AppError::field(
            "email",
            "That address is too long.",
        )));
    }
    let mut parts = email.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(ApiError(AppError::field(
            "email",
            "Enter an email address in the form name@example.com.",
        )));
    };
    if local.is_empty() || !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.')
    {
        return Err(ApiError(AppError::field(
            "email",
            "Enter an email address in the form name@example.com.",
        )));
    }
    if email.chars().any(char::is_whitespace) {
        return Err(ApiError(AppError::field(
            "email",
            "An email address cannot contain spaces.",
        )));
    }
    Ok(email)
}

pub(crate) fn validate_password(password: &str) -> Result<(), ApiError> {
    if password.chars().count() < MIN_PASSWORD_LENGTH {
        return Err(ApiError(AppError::field(
            "password",
            format!("Use at least {MIN_PASSWORD_LENGTH} characters."),
        )));
    }
    if password.len() > MAX_PASSWORD_LENGTH {
        return Err(ApiError(AppError::field(
            "password",
            "That password is too long.",
        )));
    }
    // No composition rules, and no common-password list yet: length plus the
    // rate limiter is the honest state of this. The list is a Milestone 2
    // follow-up noted in docs/verification.md rather than claimed here.
    Ok(())
}

/// Handles are the public address of a pseud, so they are restricted to
/// characters that survive being put in a URL and read aloud.
pub(crate) fn validate_handle(raw: &str) -> Result<String, ApiError> {
    let handle = raw.trim();
    if !(3..=32).contains(&handle.chars().count()) {
        return Err(ApiError(AppError::field(
            "handle",
            "A handle must be between 3 and 32 characters.",
        )));
    }
    if !handle
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(ApiError(AppError::field(
            "handle",
            "A handle may contain only letters, numbers, underscores and hyphens.",
        )));
    }
    if handle.chars().all(|c| c.is_ascii_digit()) {
        return Err(ApiError(AppError::field(
            "handle",
            "A handle needs at least one letter.",
        )));
    }
    Ok(handle.to_owned())
}

pub(crate) fn validate_display_name(name: &str) -> Result<(), ApiError> {
    if name.chars().count() > 64 {
        return Err(ApiError(AppError::field(
            "display_name",
            "A display name may be at most 64 characters.",
        )));
    }
    // Control characters would let a display name break the layout it appears
    // in, including in a terminal-based client.
    if name.chars().any(char::is_control) {
        return Err(ApiError(AppError::field(
            "display_name",
            "A display name cannot contain control characters.",
        )));
    }
    Ok(())
}

/// Turn a declared band into a policy state.
///
/// Spec §7 is explicit that a self-declared adult is *not* verified, and that
/// an under-threshold authorization workflow must not be implied into
/// existence. So the band picks the state honestly:
///
/// | declared | workflow configured | state |
/// |---|---|---|
/// | `adult` | — | `declared_adult` |
/// | `minor` | yes | `authorization_required` |
/// | `minor` | no | `restricted` |
/// | absent/unknown | — | `unknown` |
fn resolve_age_state(state: &AppState, declared: Option<&str>) -> Result<AgeState, ApiError> {
    match declared
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("adult") => Ok(AgeState::DeclaredAdult),
        Some("minor") | Some("under_threshold") => {
            if state.config().age.guardian_workflow_enabled {
                Ok(AgeState::AuthorizationRequired)
            } else {
                // No authorization workflow exists, so an under-threshold
                // account is created in a state that cannot write, message or
                // be listed. Reading remains available — and is available
                // anonymously in any case, so refusing registration would
                // withhold nothing while pretending to protect someone.
                Ok(AgeState::Restricted)
            }
        }
        Some("unknown") | None => Ok(AgeState::Unknown),
        Some(other) => Err(ApiError(AppError::field(
            "age_band",
            format!("Unrecognised age_band {other:?}; expected adult, minor or unknown."),
        ))),
    }
}

async fn store_onboarding_defaults(
    state: &AppState,
    account_id: AccountId,
    pseud_id: PseudId,
    age_state: AgeState,
) -> anyhow::Result<()> {
    // Account-scoped keys, including the two that carry the protective
    // defaults. Written from the single canonical list so onboarding and the
    // settings endpoint cannot drift.
    for (key, value) in privacy::onboarding_defaults(age_state) {
        identity::set_privacy(state.db(), PrivacyScope::Account(&account_id), key, value).await?;
    }

    // Pseud-scoped keys, so the compartmentalised settings have a stored
    // starting point too rather than being derived on read.
    for (key, value) in privacy::onboarding_pseud_defaults(age_state) {
        identity::set_privacy(state.db(), PrivacyScope::Pseud(&pseud_id), key, value).await?;
    }

    // Content preferences start at the most conservative value inside whatever
    // the policy allows, and the reader raises it deliberately.
    let settings = sessions::ContentSettings {
        max_rating: lorehaven_domain::policy::ContentRating::General,
        ..Default::default()
    };
    sessions::save_content_settings(state.db(), account_id, &settings, None).await?;
    Ok(())
}

async fn record_age_assessment(
    state: &AppState,
    account_id: AccountId,
    age_state: AgeState,
) -> anyhow::Result<()> {
    let (band, method) = match age_state {
        AgeState::DeclaredAdult => ("adult", "self_declaration"),
        AgeState::DeclaredMinor => ("minor", "self_declaration"),
        AgeState::Restricted => ("under_threshold", "self_declaration"),
        AgeState::AuthorizationRequired => ("under_threshold", "self_declaration"),
        AgeState::AuthorizedUnderPolicy => ("under_threshold", "guardian_authorization"),
        AgeState::Unknown => ("unknown", "none"),
    };

    let sql = state.db().sql(
        "INSERT INTO age_assessments (id, account_id, age_band, assurance_method, policy_version, assessed_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        "INSERT INTO age_assessments (id, account_id, age_band, assurance_method, policy_version, assessed_at, created_at)
         VALUES (?::uuid, ?::uuid, ?, ?, ?, ?, ?)",
    );
    let id = uuid::Uuid::new_v4().to_string();
    let now = sessions::now();
    let policy_version = format!(
        "{}+threshold{}",
        version::VERSION,
        state.config().age.threshold
    );

    match state.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(account_id.to_string())
                .bind(band)
                .bind(method)
                .bind(&policy_version)
                .bind(&now)
                .bind(&now)
                .execute(state.db().sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(account_id.to_string())
                .bind(band)
                .bind(method)
                .bind(&policy_version)
                .bind(&now)
                .bind(&now)
                .execute(state.db().postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(())
}

fn age_state_name(state: AgeState) -> &'static str {
    match state {
        AgeState::Unknown => "unknown",
        AgeState::DeclaredMinor => "declared_minor",
        AgeState::DeclaredAdult => "declared_adult",
        AgeState::AuthorizationRequired => "authorization_required",
        AgeState::AuthorizedUnderPolicy => "authorized_under_policy",
        AgeState::Restricted => "restricted",
    }
}

/// What an age state permits, in one place.
///
/// The interface reads this rather than re-deriving it, so a change here cannot
/// leave the UI offering something the server will refuse.
fn capabilities_for(
    state: &AppState,
    age_state: AgeState,
    settings: Option<&sessions::ContentSettings>,
) -> Capabilities {
    let policy = lorehaven_domain::policy::AccessPolicy::default();

    let (can_write, can_message, can_be_listed, note) = match age_state {
        AgeState::DeclaredAdult | AgeState::AuthorizedUnderPolicy => (true, true, true, None),
        AgeState::Unknown => (
            false,
            false,
            false,
            Some(
                "Set your age group before you can publish or message. Reading is unaffected."
                    .to_owned(),
            ),
        ),
        AgeState::DeclaredMinor => (
            false,
            false,
            false,
            Some(capability_note(state, "declared below the age threshold")),
        ),
        AgeState::AuthorizationRequired => (
            false,
            false,
            false,
            Some(capability_note(
                state,
                "awaiting a guardian authorization that is not yet set up",
            )),
        ),
        AgeState::Restricted => (
            false,
            false,
            false,
            Some(capability_note(state, "below the age threshold")),
        ),
    };

    // The ceiling is the lower of what policy allows and what the reader chose.
    let policy_ceiling = match age_state {
        AgeState::DeclaredAdult => policy.adult_max_rating,
        _ => policy.minor_max_rating,
    };
    let preference = settings.map_or(policy_ceiling, |s| s.max_rating);
    let effective = policy_ceiling.min(preference);

    Capabilities {
        can_read: true,
        can_write,
        can_message,
        can_be_listed,
        max_rating: sessions::rating_name(effective),
        restriction_note: note,
    }
}

fn capability_note(state: &AppState, because: &str) -> String {
    if state.config().age.guardian_workflow_enabled {
        format!(
            "This account is {because}. Publishing and messaging unlock once a guardian \
             authorization is recorded."
        )
    } else {
        format!(
            "This account is {because}, and this instance has no guardian authorization \
             workflow configured. You can read anything within your rating, but publishing, \
             messaging and directory listing are unavailable. An operator can enable the \
             workflow in configuration."
        )
    }
}

fn account_view(
    account: &lorehaven_db::identity::Account,
    session_expires_at: &str,
) -> AccountView {
    AccountView {
        id: account.id,
        email: account.email.clone(),
        age_state: age_state_name(account.age_state),
        email_verified: account.email_verified_at.is_some(),
        session_expires_at: session_expires_at.to_owned(),
    }
}

fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        // Truncate rather than reject: a long user agent is not an attack, but
        // storing an unbounded string is not a habit worth forming.
        .map(|value| value.chars().take(256).collect())
}

/// Reduce a user agent to something a person recognises as their own device.
#[must_use]
pub fn describe_device(user_agent: Option<&str>) -> String {
    let Some(ua) = user_agent else {
        return "Unknown device".to_owned();
    };
    if ua.trim().is_empty() {
        return "Unknown device".to_owned();
    }

    let browser = if ua.contains("Firefox/") {
        "Firefox"
    } else if ua.contains("Edg/") {
        "Edge"
    } else if ua.contains("Chrome/") {
        "Chrome"
    } else if ua.contains("Safari/") {
        "Safari"
    } else if ua.contains("curl/") {
        "curl"
    } else {
        "Browser"
    };

    let platform = if ua.contains("Android") {
        "Android"
    } else if ua.contains("iPhone") || ua.contains("iPad") {
        "iOS"
    } else if ua.contains("Windows") {
        "Windows"
    } else if ua.contains("Mac OS X") {
        "macOS"
    } else if ua.contains("Linux") {
        "Linux"
    } else {
        "an unknown system"
    };

    format!("{browser} on {platform}")
}

/// Build a response carrying the session and CSRF cookies.
fn session_response(
    state: &AppState,
    status: StatusCode,
    body: AccountResponse,
    issued: &auth::IssuedSession,
) -> Response {
    let policy = CookiePolicy::from_config(state.config());
    let ttl = state.config().security.session_ttl;

    let mut response = (status, Json(body)).into_response();

    append_cookie(
        response.headers_mut(),
        auth::build_cookie(SESSION_COOKIE, &issued.token, policy, Some(ttl), true),
    );
    // Readable by scripts on purpose: the client must echo it in a header.
    append_cookie(
        response.headers_mut(),
        auth::build_cookie(CSRF_COOKIE, &issued.csrf_token, policy, Some(ttl), false),
    );
    response.headers_mut().insert(
        header::HeaderName::from_static("x-csrf-header"),
        HeaderValue::from_static(CSRF_HEADER),
    );
    response
}

/// Append a `Set-Cookie`, keeping any that are already present.
fn append_cookie(headers: &mut HeaderMap, cookie: String) {
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        headers.append(header::SET_COOKIE, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_normalisation_is_case_insensitive() {
        assert_eq!(
            normalise_email("  Reader@Example.COM ").expect("valid"),
            "reader@example.com"
        );
    }

    #[test]
    fn malformed_addresses_are_refused_with_a_useful_message() {
        for bad in [
            "",
            "no-at-sign",
            "two@@example.com",
            "user@nodot",
            "a b@example.com",
        ] {
            let error = normalise_email(bad).expect_err("must refuse");
            assert_eq!(
                error.0.code(),
                lorehaven_domain::ErrorCode::ValidationFailed
            );
        }
    }

    #[test]
    fn a_reasonable_address_is_accepted() {
        for good in ["a@b.co", "first.last+tag@sub.example.org"] {
            assert!(normalise_email(good).is_ok(), "{good} should be accepted");
        }
    }

    #[test]
    fn short_passwords_are_refused_and_long_ones_accepted() {
        assert!(validate_password("short").is_err());
        assert!(validate_password(&"a".repeat(MIN_PASSWORD_LENGTH)).is_ok());
        assert!(validate_password(&"a".repeat(MAX_PASSWORD_LENGTH + 1)).is_err());
    }

    #[test]
    fn handles_are_constrained() {
        assert!(validate_handle("Quill").is_ok());
        assert!(validate_handle("quill-pen_2").is_ok());

        assert!(validate_handle("ab").is_err(), "too short");
        assert!(validate_handle(&"a".repeat(33)).is_err(), "too long");
        assert!(validate_handle("has space").is_err());
        assert!(validate_handle("../../etc").is_err());
        assert!(validate_handle("12345").is_err(), "needs a letter");
    }

    #[test]
    fn display_names_reject_control_characters() {
        assert!(validate_display_name("Ordinary Name").is_ok());
        assert!(validate_display_name("Line\nBreak").is_err());
        assert!(validate_display_name(&"x".repeat(65)).is_err());
    }

    #[test]
    fn devices_are_described_in_human_terms() {
        assert_eq!(
            describe_device(Some(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/120 Safari/537.36"
            )),
            "Chrome on Windows"
        );
        assert_eq!(
            describe_device(Some("curl/8.5.0")),
            "curl on an unknown system"
        );
        assert_eq!(describe_device(None), "Unknown device");
        assert_eq!(describe_device(Some("   ")), "Unknown device");
    }
}
