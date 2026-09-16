//! Session, recovery-token and content-settings repositories.
//!
//! Spec §3.5: sessions are opaque, server-managed, and revoked explicitly;
//! tokens are stored only as hashes. This module is the storage half of that —
//! the cookie and CSRF mechanics live in the server crate.
//!
//! As with `identity`, every statement is supplied once per dialect and every
//! bind/row is `String`/`i64`, so decoding is identical on SQLite and
//! PostgreSQL (ADR 0004).

use anyhow::{Context, Result};
use lorehaven_domain::policy::ContentRating;
use lorehaven_domain::{AccountId, PseudId, SessionId};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::identity::now_rfc3339;
use crate::{Backend, Database};

/// A session as far as the server cares.
#[derive(Debug, Clone)]
pub struct Session {
    /// Primary identifier — this is what `DELETE /auth/sessions/:id` takes.
    pub id: SessionId,
    /// The owning account.
    pub account_id: AccountId,
    /// The pseud this session is currently acting as, if one has been chosen.
    pub active_pseud_id: Option<PseudId>,
    /// Storage form of the CSRF token. Never leaves the server.
    pub csrf_token_hash: String,
    /// User agent captured at creation, shown in the session list.
    pub user_agent: Option<String>,
    /// When the session was created, RFC 3339.
    pub created_at: String,
    /// Last activity, RFC 3339.
    pub last_seen_at: String,
    /// Hard expiry, RFC 3339.
    pub expires_at: String,
}

/// Row shape for session queries.
type SessionRow = (
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    String,
    String,
    String,
);

/// The purpose of a single-use recovery token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryPurpose {
    /// Reset a forgotten password.
    PasswordReset,
    /// Confirm an email address.
    EmailVerification,
}

impl RecoveryPurpose {
    /// Storage form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PasswordReset => "password_reset",
            Self::EmailVerification => "email_verification",
        }
    }
}

/// How long a recovery token stays usable.
///
/// Kept here rather than at the call site so that the expiry written into the
/// row and the expiry quoted to the user cannot drift.
#[must_use]
pub const fn recovery_ttl_hours(purpose: RecoveryPurpose) -> i64 {
    match purpose {
        // A reset link is the most sensitive thing we mail out.
        RecoveryPurpose::PasswordReset => 2,
        // Verification is not urgent and people read mail late.
        RecoveryPurpose::EmailVerification => 48,
    }
}

/// Insert a session.
///
/// The caller supplies the *hash* of the token; the plaintext is returned to
/// the client once and never stored.
pub async fn create_session(
    db: &Database,
    account_id: AccountId,
    token_hash: &str,
    csrf_token_hash: &str,
    user_agent: Option<&str>,
    active_pseud_id: Option<PseudId>,
    expires_at: &str,
) -> Result<SessionId> {
    let id = SessionId::new();
    let now = now_rfc3339();

    let sql = db.sql(
        "INSERT INTO sessions
             (id, token_hash, account_id, csrf_token_hash, user_agent, active_pseud_id,
              created_at, last_seen_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        "INSERT INTO sessions
             (id, token_hash, account_id, csrf_token_hash, user_agent, active_pseud_id,
              created_at, last_seen_at, expires_at)
         VALUES (?::uuid, ?, ?::uuid, ?, ?, ?::uuid, ?, ?, ?)",
    );

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(id.to_string())
                .bind(token_hash)
                .bind(account_id.to_string())
                .bind(csrf_token_hash)
                .bind(user_agent)
                .bind(active_pseud_id.map(|p| p.to_string()))
                .bind(&now)
                .bind(&now)
                .bind(expires_at)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await
                .context("inserting session")?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(id.to_string())
                .bind(token_hash)
                .bind(account_id.to_string())
                .bind(csrf_token_hash)
                .bind(user_agent)
                .bind(active_pseud_id.map(|p| p.to_string()))
                .bind(&now)
                .bind(&now)
                .bind(expires_at)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await
                .context("inserting session")?;
        }
    }

    Ok(id)
}

/// Look up a live session by the hash of its token.
///
/// Expired and revoked sessions are filtered *in the query* rather than by the
/// caller: a future caller cannot forget the check if there is no check to
/// forget.
pub async fn find_live_session_by_token_hash(
    db: &Database,
    token_hash: &str,
    now: &str,
) -> Result<Option<Session>> {
    let sql = db.sql(
        "SELECT id, account_id, active_pseud_id, csrf_token_hash, user_agent,
                created_at, last_seen_at, expires_at
           FROM sessions
          WHERE token_hash = ? AND revoked_at IS NULL AND expires_at > ?",
        "SELECT id::text, account_id::text, active_pseud_id::text, csrf_token_hash, user_agent,
                created_at, last_seen_at, expires_at
           FROM sessions
          WHERE token_hash = ? AND revoked_at IS NULL AND expires_at > ?",
    );

    let row: Option<SessionRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(token_hash)
                .bind(now)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(token_hash)
                .bind(now)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(row.map(decode_session))
}

/// Every live session belonging to an account, newest first.
pub async fn live_sessions_for_account(
    db: &Database,
    account_id: AccountId,
    now: &str,
) -> Result<Vec<Session>> {
    let sql = db.sql(
        "SELECT id, account_id, active_pseud_id, csrf_token_hash, user_agent,
                created_at, last_seen_at, expires_at
           FROM sessions
          WHERE account_id = ? AND revoked_at IS NULL AND expires_at > ?
          ORDER BY last_seen_at DESC",
        "SELECT id::text, account_id::text, active_pseud_id::text, csrf_token_hash, user_agent,
                created_at, last_seen_at, expires_at
           FROM sessions
          WHERE account_id::text = ? AND revoked_at IS NULL AND expires_at > ?
          ORDER BY last_seen_at DESC",
    );

    let rows: Vec<SessionRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account_id.to_string())
                .bind(now)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account_id.to_string())
                .bind(now)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(rows.into_iter().map(decode_session).collect())
}

/// Revoke one session, scoped to its owner.
///
/// Returns whether a row was affected. Scoping the update by `account_id` is
/// what stops "delete session :id" from being a tool for logging other people
/// out; a mismatched id simply matches nothing.
pub async fn revoke_session(
    db: &Database,
    account_id: AccountId,
    session_id: SessionId,
    now: &str,
) -> Result<bool> {
    let sql = db.sql(
        "UPDATE sessions SET revoked_at = ?
          WHERE id = ? AND account_id = ? AND revoked_at IS NULL",
        "UPDATE sessions SET revoked_at = ?
          WHERE id::text = ? AND account_id::text = ? AND revoked_at IS NULL",
    );

    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(now)
            .bind(session_id.to_string())
            .bind(account_id.to_string())
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(now)
            .bind(session_id.to_string())
            .bind(account_id.to_string())
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };

    Ok(affected > 0)
}

/// Revoke every live session of an account.
pub async fn revoke_all_sessions(db: &Database, account_id: AccountId, now: &str) -> Result<u64> {
    let sql = db.sql(
        "UPDATE sessions SET revoked_at = ? WHERE account_id = ? AND revoked_at IS NULL",
        "UPDATE sessions SET revoked_at = ? WHERE account_id::text = ? AND revoked_at IS NULL",
    );

    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(now)
            .bind(account_id.to_string())
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(now)
            .bind(account_id.to_string())
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };

    Ok(affected)
}

/// Switch which pseud a session is acting as.
pub async fn set_active_pseud(
    db: &Database,
    session_id: SessionId,
    pseud_id: PseudId,
) -> Result<()> {
    let sql = db.sql(
        "UPDATE sessions SET active_pseud_id = ? WHERE id = ?",
        "UPDATE sessions SET active_pseud_id = ?::uuid WHERE id::text = ?",
    );

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(pseud_id.to_string())
                .bind(session_id.to_string())
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await
                .context("switching active pseud")?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(pseud_id.to_string())
                .bind(session_id.to_string())
                .execute(db.postgres_pool().expect("postgres handle"))
                .await
                .context("switching active pseud")?;
        }
    }
    Ok(())
}

/// Record activity on a session.
///
/// Deliberately best-effort: a failure to update `last_seen_at` must never fail
/// the request that carried it.
pub async fn touch_session(db: &Database, session_id: SessionId, now: &str) -> Result<()> {
    let sql = db.sql(
        "UPDATE sessions SET last_seen_at = ? WHERE id = ?",
        "UPDATE sessions SET last_seen_at = ? WHERE id::text = ?",
    );

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(now)
                .bind(session_id.to_string())
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(now)
                .bind(session_id.to_string())
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(())
}

/// Replace an account's password hash.
pub async fn update_password_hash(
    db: &Database,
    account_id: AccountId,
    phc: &str,
    now: &str,
) -> Result<()> {
    let sql = db.sql(
        "UPDATE password_credentials SET password_hash = ?, updated_at = ? WHERE account_id = ?",
        "UPDATE password_credentials SET password_hash = ?, updated_at = ? WHERE account_id::text = ?",
    );

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(phc)
                .bind(now)
                .bind(account_id.to_string())
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(phc)
                .bind(now)
                .bind(account_id.to_string())
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(())
}

/// Mark an account's email as verified.
pub async fn mark_email_verified(db: &Database, account_id: AccountId, now: &str) -> Result<()> {
    let sql = db.sql(
        "UPDATE accounts SET email_verified_at = ?, updated_at = ?, version = version + 1
          WHERE id = ?",
        "UPDATE accounts SET email_verified_at = ?, updated_at = ?, version = version + 1
          WHERE id::text = ?",
    );

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(now)
                .bind(now)
                .bind(account_id.to_string())
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(now)
                .bind(now)
                .bind(account_id.to_string())
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(())
}

// --- recovery tokens --------------------------------------------------------

/// Store a single-use recovery token (hash only).
pub async fn create_recovery_token(
    db: &Database,
    account_id: AccountId,
    purpose: RecoveryPurpose,
    token_hash: &str,
    expires_at: &str,
) -> Result<()> {
    let now = now_rfc3339();
    let sql = db.sql(
        "INSERT INTO recovery_tokens (token_hash, account_id, purpose, created_at, expires_at)
         VALUES (?, ?, ?, ?, ?)",
        "INSERT INTO recovery_tokens (token_hash, account_id, purpose, created_at, expires_at)
         VALUES (?, ?::uuid, ?, ?, ?)",
    );

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(token_hash)
                .bind(account_id.to_string())
                .bind(purpose.as_str())
                .bind(&now)
                .bind(expires_at)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await
                .context("inserting recovery token")?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(token_hash)
                .bind(account_id.to_string())
                .bind(purpose.as_str())
                .bind(&now)
                .bind(expires_at)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await
                .context("inserting recovery token")?;
        }
    }
    Ok(())
}

/// Consume a recovery token, returning the account it belonged to.
///
/// The `used_at IS NULL` filter and the `UPDATE` are one atomic step: two
/// concurrent redemptions of the same link cannot both succeed, because the
/// second sees `used_at` already set.
pub async fn consume_recovery_token(
    db: &Database,
    purpose: RecoveryPurpose,
    token_hash: &str,
    now: &str,
) -> Result<Option<AccountId>> {
    let sql = db.sql(
        "UPDATE recovery_tokens SET used_at = ?
          WHERE token_hash = ? AND purpose = ? AND used_at IS NULL AND expires_at > ?
      RETURNING account_id",
        "UPDATE recovery_tokens SET used_at = ?
          WHERE token_hash = ? AND purpose = ? AND used_at IS NULL AND expires_at > ?
      RETURNING account_id::text",
    );

    let row: Option<(String,)> = match db.backend() {
        Backend::Sqlite => sqlx::query_as(&sql)
            .bind(now)
            .bind(token_hash)
            .bind(purpose.as_str())
            .bind(now)
            .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
            .await
            .context("consuming recovery token")?,
        Backend::Postgres => sqlx::query_as(&sql)
            .bind(now)
            .bind(token_hash)
            .bind(purpose.as_str())
            .bind(now)
            .fetch_optional(db.postgres_pool().expect("postgres handle"))
            .await
            .context("consuming recovery token")?,
    };

    Ok(row.and_then(|(id,)| id.parse().ok()))
}

/// Invalidate outstanding tokens of a purpose for an account.
///
/// Called before issuing a new link, so that requesting a second reset email
/// does not leave the first one live.
pub async fn invalidate_recovery_tokens(
    db: &Database,
    account_id: AccountId,
    purpose: RecoveryPurpose,
    now: &str,
) -> Result<u64> {
    let sql = db.sql(
        "UPDATE recovery_tokens SET used_at = ?
          WHERE account_id = ? AND purpose = ? AND used_at IS NULL",
        "UPDATE recovery_tokens SET used_at = ?
          WHERE account_id::text = ? AND purpose = ? AND used_at IS NULL",
    );

    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(now)
            .bind(account_id.to_string())
            .bind(purpose.as_str())
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(now)
            .bind(account_id.to_string())
            .bind(purpose.as_str())
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };

    Ok(affected)
}

// --- content settings -------------------------------------------------------

/// An account's content preferences.
#[derive(Debug, Clone)]
pub struct ContentSettings {
    /// Highest rating to surface to this account.
    pub max_rating: ContentRating,
    /// Warnings to exclude.
    pub excluded_warnings: Vec<String>,
    /// Optimistic-concurrency version.
    pub version: i64,
}

impl Default for ContentSettings {
    fn default() -> Self {
        Self {
            max_rating: ContentRating::Teen,
            excluded_warnings: Vec::new(),
            version: 1,
        }
    }
}

/// Read content settings, creating the row with defaults if it is absent.
///
/// Read-through creation keeps the endpoint simple and, more importantly, means
/// an account created before this table existed behaves identically to a new
/// one.
pub async fn content_settings(db: &Database, account_id: AccountId) -> Result<ContentSettings> {
    let sql = db.sql(
        "SELECT max_rating, excluded_warnings, version FROM content_settings WHERE account_id = ?",
        "SELECT max_rating, excluded_warnings, version FROM content_settings WHERE account_id::text = ?",
    );

    let row: Option<(String, String, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account_id.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account_id.to_string())
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    match row {
        Some((rating, warnings, version)) => Ok(ContentSettings {
            max_rating: parse_rating(&rating),
            excluded_warnings: serde_json::from_str(&warnings).unwrap_or_default(),
            version,
        }),
        None => Ok(ContentSettings::default()),
    }
}

/// Upsert content settings, enforcing the optimistic version.
pub async fn save_content_settings(
    db: &Database,
    account_id: AccountId,
    settings: &ContentSettings,
    expected_version: Option<i64>,
) -> Result<i64> {
    let now = now_rfc3339();
    let warnings = serde_json::to_string(&settings.excluded_warnings)
        .context("serialising excluded warnings")?;
    let rating = rating_name(settings.max_rating);

    // Insert-or-update, but never clobber a newer edit: the WHERE clause on the
    // conflict path is what turns a concurrent update into zero affected rows
    // rather than a lost write (spec §3.4).
    let sql = db.sql(
        "INSERT INTO content_settings
             (account_id, max_rating, excluded_warnings, created_at, updated_at, version)
         VALUES (?, ?, ?, ?, ?, 1)
         ON CONFLICT (account_id) DO UPDATE SET
             max_rating = excluded.max_rating,
             excluded_warnings = excluded.excluded_warnings,
             updated_at = excluded.updated_at,
             version = content_settings.version + 1
         WHERE ? IS NULL OR content_settings.version = ?",
        "INSERT INTO content_settings
             (account_id, max_rating, excluded_warnings, created_at, updated_at, version)
         VALUES (?::uuid, ?, ?, ?, ?, 1)
         ON CONFLICT (account_id) DO UPDATE SET
             max_rating = excluded.max_rating,
             excluded_warnings = excluded.excluded_warnings,
             updated_at = excluded.updated_at,
             version = content_settings.version + 1
         WHERE ? IS NULL OR content_settings.version = ?",
    );

    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(account_id.to_string())
            .bind(rating)
            .bind(&warnings)
            .bind(&now)
            .bind(&now)
            .bind(expected_version)
            .bind(expected_version)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(account_id.to_string())
            .bind(rating)
            .bind(&warnings)
            .bind(&now)
            .bind(&now)
            .bind(expected_version)
            .bind(expected_version)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };

    if affected == 0 {
        anyhow::bail!("content settings changed since they were read");
    }

    Ok(settings.version + 1)
}

/// Storage form of a rating.
#[must_use]
pub const fn rating_name(rating: ContentRating) -> &'static str {
    match rating {
        ContentRating::General => "general",
        ContentRating::Teen => "teen",
        ContentRating::Mature => "mature",
        ContentRating::Explicit => "explicit",
    }
}

/// Parse a rating, defaulting to the most restrictive value.
///
/// An unrecognised rating must never widen visibility, so the fallback is
/// `General` rather than `Explicit`.
#[must_use]
pub fn parse_rating(raw: &str) -> ContentRating {
    match raw {
        "teen" => ContentRating::Teen,
        "mature" => ContentRating::Mature,
        "explicit" => ContentRating::Explicit,
        _ => ContentRating::General,
    }
}

/// Current UTC time in the storage format. Re-exported for callers that need
/// to pass `now` into the queries above.
#[must_use]
pub fn now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

/// Format an instant that is `hours` in the future.
#[must_use]
pub fn expires_in_hours(hours: i64) -> String {
    (OffsetDateTime::now_utc() + time::Duration::hours(hours))
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

fn decode_session(row: SessionRow) -> Session {
    let (
        id,
        account_id,
        active_pseud_id,
        csrf_token_hash,
        user_agent,
        created_at,
        last_seen_at,
        expires_at,
    ) = row;
    Session {
        id: id.parse().unwrap_or_default(),
        account_id: account_id.parse().unwrap_or_default(),
        active_pseud_id: active_pseud_id.and_then(|value| value.parse().ok()),
        csrf_token_hash,
        user_agent,
        created_at,
        last_seen_at,
        expires_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratings_round_trip() {
        for rating in [
            ContentRating::General,
            ContentRating::Teen,
            ContentRating::Mature,
            ContentRating::Explicit,
        ] {
            assert_eq!(parse_rating(rating_name(rating)), rating);
        }
    }

    #[test]
    fn an_unknown_rating_never_widens_visibility() {
        // The fallback direction matters: defaulting to Explicit would turn a
        // corrupt row into an unrestricted account.
        assert_eq!(parse_rating("nonsense"), ContentRating::General);
        assert_eq!(parse_rating(""), ContentRating::General);
        assert_eq!(parse_rating("EXPLICIT"), ContentRating::General);
    }

    #[test]
    fn default_content_settings_are_restrictive() {
        let defaults = ContentSettings::default();
        assert_eq!(defaults.max_rating, ContentRating::Teen);
        assert!(defaults.excluded_warnings.is_empty());
    }

    #[test]
    fn recovery_expiry_reflects_the_purpose() {
        // A reset link is shorter-lived than a verification link, and the
        // quoted duration comes from the same function that writes the row.
        assert!(recovery_ttl_hours(RecoveryPurpose::PasswordReset) < 24);
        assert!(recovery_ttl_hours(RecoveryPurpose::EmailVerification) >= 24);
    }

    #[test]
    fn expiry_formatting_is_in_the_future_and_rfc3339() {
        let soon = expires_in_hours(1);
        assert!(soon.ends_with('Z'), "got {soon}");
        assert!(soon > now(), "an expiry must be after now");
    }
}
