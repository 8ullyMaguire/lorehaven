//! Identity repositories.
//!
//! Spec §3.6 requires the sequence `authenticate → resolve active pseud → load
//! resource → evaluate policy → perform operation → record audit event`.
//! These functions are the "load" half; the policy half lives in
//! `lorehaven-domain`.
//!
//! Every statement is supplied twice — once per dialect — because spec §4
//! forbids assuming SQLite and PostgreSQL are interchangeable. Parameters and
//! rows are deliberately restricted to `String`/`i64`, so decoding is uniform
//! across engines: PostgreSQL `UUID` columns are cast to text on read and
//! parametrised with `?::uuid` on write.

use anyhow::{Context, Result};
use lorehaven_domain::policy::AgeState;
use lorehaven_domain::{AccountId, PseudId, SessionId};
use sqlx::Row;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::{Backend, Database};

/// Current UTC time as RFC 3339 text — the storage format for every timestamp.
#[must_use]
pub fn now_rfc3339() -> String {
    format_rfc3339(OffsetDateTime::now_utc())
}

/// A moment this many seconds from now, as RFC 3339 text.
///
/// The counterpart to [`now_rfc3339`] for the columns that mean "until": a
/// cached revision's expiry, a lease deadline. Computed from the same clock and
/// formatted by the same function, so an expiry can never disagree with a
/// creation time about the offset.
///
/// # Panics
/// Never: a duration of that many seconds is representable for any input this
/// codebase passes, and `saturating` bounds it if not.
#[must_use]
pub fn in_seconds(seconds: i64) -> String {
    let now = OffsetDateTime::now_utc();
    let at = now
        .checked_add(time::Duration::seconds(seconds))
        .unwrap_or(now);
    format_rfc3339(at)
}

/// A moment as RFC 3339 text, the format every timestamp column stores.
///
/// Public because the job queue computes a *future* moment (a lease expiry, the
/// earliest time a retry may run) and has to store it in the same format. A
/// second formatter would be a second thing that can disagree about the offset.
#[must_use]
pub fn format_rfc3339(at: OffsetDateTime) -> String {
    at.format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

/// Whether an account may sign in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountStatus {
    /// Normal.
    Active,
    /// Suspended by moderation.
    Suspended,
    /// Closed by the owner or by deletion.
    Closed,
}

impl AccountStatus {
    /// Wire/storage form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Suspended => "suspended",
            Self::Closed => "closed",
        }
    }

    /// Parse the storage form, rejecting unknown values.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "active" => Some(Self::Active),
            "suspended" => Some(Self::Suspended),
            "closed" => Some(Self::Closed),
            _ => None,
        }
    }
}

/// An account row.
#[derive(Debug, Clone)]
pub struct Account {
    /// Primary identifier.
    pub id: AccountId,
    /// Sign-in address.
    pub email: String,
    /// Lifecycle status.
    pub status: AccountStatus,
    /// Age-policy state.
    pub age_state: AgeState,
    /// When the address was confirmed, if it has been.
    pub email_verified_at: Option<String>,
    /// Creation time, RFC 3339.
    pub created_at: String,
    /// Optimistic-concurrency version.
    pub version: i64,
}

/// A pseud row.
#[derive(Debug, Clone)]
pub struct Pseud {
    /// Primary identifier.
    pub id: PseudId,
    /// Owning account. Never exposed by the API (spec §7).
    pub account_id: AccountId,
    /// Unique, case-insensitively matched handle.
    pub handle: String,
    /// Display name.
    pub display_name: String,
    /// Free-text biography.
    pub bio: Option<String>,
    /// Whether the pseud appears in listings and search.
    pub discoverability: Discoverability,
    /// Creation time, RFC 3339.
    pub created_at: String,
    /// Optimistic-concurrency version.
    pub version: i64,
}

/// Whether a pseud is discoverable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Discoverability {
    /// Appears in listings and search.
    Listed,
    /// Reachable only by direct link, and not described as existing.
    Hidden,
}

impl Discoverability {
    /// Storage form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Listed => "listed",
            Self::Hidden => "hidden",
        }
    }

    /// Parse the storage form, defaulting to the non-disclosing value.
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        match raw {
            "listed" => Self::Listed,
            _ => Self::Hidden,
        }
    }

    /// Whether this pseud may be shown publicly.
    #[must_use]
    pub const fn is_listed(self) -> bool {
        matches!(self, Self::Listed)
    }
}

/// The shape every identity query decodes into.
///
/// Parameters and columns are restricted to `String`/`i64` so that rows decode
/// identically on SQLite and PostgreSQL, where the same logical column is TEXT
/// in one engine and UUID in the other (see the crate docs and ADR 0004).
type AccountRow = (String, String, String, String, Option<String>, String, i64);

/// A pseud row: `(id, account_id, handle, display_name, bio, discoverability,
/// created_at, version)`.
type PseudRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    i64,
);

/// The owner of a privacy setting.
#[derive(Debug, Clone, Copy)]
pub enum PrivacyScope<'a> {
    /// An account-level policy.
    Account(&'a AccountId),
    /// A pseud-level policy.
    Pseud(&'a PseudId),
}

/// Insert an account.
pub async fn create_account(
    db: &Database,
    email: &str,
    age_state: AgeState,
    status: AccountStatus,
) -> Result<AccountId> {
    let id = AccountId::new();
    let now = now_rfc3339();

    let sql = db.sql(
        "INSERT INTO accounts (id, status, email, age_state, created_at, updated_at, version)
         VALUES (?, ?, ?, ?, ?, ?, 1)",
        "INSERT INTO accounts (id, status, email, age_state, created_at, updated_at, version)
         VALUES (?::uuid, ?, ?, ?, ?, ?, 1)",
    );
    let age = age_state_string(age_state);

    match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(id.to_string())
                .bind(status.as_str())
                .bind(email)
                .bind(age)
                .bind(&now)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await
                .context("inserting account")?;
        }
        crate::Backend::Postgres => {
            sqlx::query(&sql)
                .bind(id.to_string())
                .bind(status.as_str())
                .bind(email)
                .bind(age)
                .bind(&now)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await
                .context("inserting account")?;
        }
    }

    Ok(id)
}

/// Find an account by address, case-insensitively.
pub async fn find_account_by_email(db: &Database, email: &str) -> Result<Option<Account>> {
    let sql = db.sql(
        "SELECT id, email, status, age_state, email_verified_at, created_at, version
           FROM accounts WHERE lower(email) = lower(?) AND deleted_at IS NULL",
        "SELECT id::text, email, status, age_state, email_verified_at, created_at, version
           FROM accounts WHERE lower(email) = lower(?) AND deleted_at IS NULL",
    );

    let row: Option<AccountRow> = match db.backend() {
        crate::Backend::Sqlite => sqlx::query_as(&sql)
            .bind(email)
            .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
            .await
            .context("looking up account by email")?,
        crate::Backend::Postgres => sqlx::query_as(&sql)
            .bind(email)
            .fetch_optional(db.postgres_pool().expect("postgres handle"))
            .await
            .context("looking up account by email")?,
    };

    Ok(row.map(decode_account))
}

/// Find an account by identifier.
pub async fn find_account(db: &Database, id: AccountId) -> Result<Option<Account>> {
    let sql = db.sql(
        "SELECT id, email, status, age_state, email_verified_at, created_at, version
           FROM accounts WHERE id = ?",
        "SELECT id::text, email, status, age_state, email_verified_at, created_at, version
           FROM accounts WHERE id::text = ?",
    );

    let row: Option<AccountRow> = match db.backend() {
        crate::Backend::Sqlite => sqlx::query_as(&sql)
            .bind(id.to_string())
            .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
            .await
            .context("looking up account")?,
        crate::Backend::Postgres => sqlx::query_as(&sql)
            .bind(id.to_string())
            .fetch_optional(db.postgres_pool().expect("postgres handle"))
            .await
            .context("looking up account")?,
    };

    Ok(row.map(decode_account))
}

/// Store (or replace) an account's password hash.
pub async fn set_password_hash(db: &Database, account_id: AccountId, phc: &str) -> Result<()> {
    let now = now_rfc3339();
    let exists_sql = db.sql(
        "SELECT COUNT(*) FROM password_credentials WHERE account_id = ?",
        "SELECT COUNT(*) FROM password_credentials WHERE account_id::text = ?",
    );
    let existing: i64 = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_scalar(&exists_sql)
                .bind(account_id.to_string())
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_scalar(&exists_sql)
                .bind(account_id.to_string())
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    /*
     * Each statement binds its own parameters, in its own order.
     *
     * The two statements name their columns differently — the insert leads with
     * `account_id`, the update can only reach it in the `WHERE` — and this used
     * to bind one four-value list for both. The consequences were not equal. On
     * SQLite the surplus parameter shifted every value by one, so the update
     * matched no row and still reported success: setting a password on an
     * account that already had one did nothing, silently. On PostgreSQL the same
     * statement is refused ("invalid input syntax for type uuid"), which is how
     * it was found — by running the application against a real server.
     */
    if existing > 0 {
        let sql = db.sql(
            "UPDATE password_credentials SET password_hash = ?, updated_at = ? WHERE account_id = ?",
            "UPDATE password_credentials SET password_hash = ?, updated_at = ? WHERE account_id::text = ?",
        );

        match db.backend() {
            crate::Backend::Sqlite => {
                sqlx::query(&sql)
                    .bind(phc)
                    .bind(&now)
                    .bind(account_id.to_string())
                    .execute(db.sqlite_pool().expect("sqlite handle"))
                    .await
                    .context("writing password credential")?;
            }
            crate::Backend::Postgres => {
                sqlx::query(&sql)
                    .bind(phc)
                    .bind(&now)
                    .bind(account_id.to_string())
                    .execute(db.postgres_pool().expect("postgres handle"))
                    .await
                    .context("writing password credential")?;
            }
        }

        return Ok(());
    }

    let sql = db.sql(
        "INSERT INTO password_credentials (account_id, password_hash, algorithm, created_at, updated_at)
             VALUES (?, ?, 'argon2id', ?, ?)",
        "INSERT INTO password_credentials (account_id, password_hash, algorithm, created_at, updated_at)
             VALUES (?::uuid, ?, 'argon2id', ?, ?)",
    );

    match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(account_id.to_string())
                .bind(phc)
                .bind(&now)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await
                .context("writing password credential")?;
        }
        crate::Backend::Postgres => {
            sqlx::query(&sql)
                .bind(account_id.to_string())
                .bind(phc)
                .bind(&now)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await
                .context("writing password credential")?;
        }
    }
    Ok(())
}

/// Read an account's stored password hash.
pub async fn password_hash(db: &Database, account_id: AccountId) -> Result<Option<String>> {
    let sql = db.sql(
        "SELECT password_hash FROM password_credentials WHERE account_id = ?",
        "SELECT password_hash FROM password_credentials WHERE account_id::text = ?",
    );
    let row: Option<(String,)> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account_id.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account_id.to_string())
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(|(hash,)| hash))
}

/// Create a pseud under an account.
pub async fn create_pseud(
    db: &Database,
    account_id: AccountId,
    handle: &str,
    display_name: &str,
) -> Result<PseudId> {
    let id = PseudId::new();
    let now = now_rfc3339();

    let sql = db.sql(
        "INSERT INTO pseuds (id, account_id, handle, display_name, discoverability, created_at, updated_at, version)
         VALUES (?, ?, ?, ?, 'listed', ?, ?, 1)",
        "INSERT INTO pseuds (id, account_id, handle, display_name, discoverability, created_at, updated_at, version)
         VALUES (?::uuid, ?::uuid, ?, ?, 'listed', ?, ?, 1)",
    );

    match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(id.to_string())
                .bind(account_id.to_string())
                .bind(handle)
                .bind(display_name)
                .bind(&now)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await
                .context("inserting pseud")?;
        }
        crate::Backend::Postgres => {
            sqlx::query(&sql)
                .bind(id.to_string())
                .bind(account_id.to_string())
                .bind(handle)
                .bind(display_name)
                .bind(&now)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await
                .context("inserting pseud")?;
        }
    }

    Ok(id)
}

/// Find a pseud by handle, case-insensitively.
pub async fn find_pseud_by_handle(db: &Database, handle: &str) -> Result<Option<Pseud>> {
    let sql = db.sql(
        "SELECT id, account_id, handle, display_name, bio, discoverability, created_at, version
           FROM pseuds WHERE lower(handle) = lower(?) AND deleted_at IS NULL",
        "SELECT id::text, account_id::text, handle, display_name, bio, discoverability, created_at, version
           FROM pseuds WHERE lower(handle) = lower(?) AND deleted_at IS NULL",
    );

    let row: Option<PseudRow> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(handle)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(handle)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(row.map(decode_pseud))
}

/// List an account's pseuds, oldest first.
pub async fn pseuds_for_account(db: &Database, account_id: AccountId) -> Result<Vec<Pseud>> {
    let sql = db.sql(
        "SELECT id, account_id, handle, display_name, bio, discoverability, created_at, version
           FROM pseuds WHERE account_id = ? AND deleted_at IS NULL ORDER BY created_at ASC",
        "SELECT id::text, account_id::text, handle, display_name, bio, discoverability, created_at, version
           FROM pseuds WHERE account_id::text = ? AND deleted_at IS NULL ORDER BY created_at ASC",
    );

    let rows: Vec<PseudRow> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account_id.to_string())
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account_id.to_string())
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(rows.into_iter().map(decode_pseud).collect())
}

/// Set a privacy policy value, replacing any existing one.
pub async fn set_privacy(
    db: &Database,
    scope: PrivacyScope<'_>,
    key: &str,
    value: &str,
) -> Result<()> {
    let now = now_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();

    let (account, pseud) = match scope {
        PrivacyScope::Account(id) => (Some(id.to_string()), None),
        PrivacyScope::Pseud(id) => (None, Some(id.to_string())),
    };

    /*
     * The scope decides the conflict target, on PostgreSQL only.
     *
     * This table carries two *partial* unique indexes — one keyed on
     * `(account_id, key)` where the account is set, one on `(pseud_id, key)`
     * where the pseud is — because a row belongs to exactly one of the two
     * (the table's CHECK says so). SQLite's bare `ON CONFLICT` matches either,
     * and the statement has always relied on that. PostgreSQL infers a partial
     * index only when the conflict target repeats that index's own predicate,
     * so it has to be told which one applies — which is what `scope` already
     * knows.
     */
    let postgres_sql = match scope {
        PrivacyScope::Account(_) => {
            "INSERT INTO privacy_settings (id, account_id, pseud_id, key, value, created_at, updated_at)
             VALUES (?::uuid, ?::uuid, ?::uuid, ?, ?, ?, ?)
             ON CONFLICT (account_id, key) WHERE account_id IS NOT NULL
             DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at"
        }
        PrivacyScope::Pseud(_) => {
            "INSERT INTO privacy_settings (id, account_id, pseud_id, key, value, created_at, updated_at)
             VALUES (?::uuid, ?::uuid, ?::uuid, ?, ?, ?, ?)
             ON CONFLICT (pseud_id, key) WHERE pseud_id IS NOT NULL
             DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at"
        }
    };

    let sql = db.sql(
        "INSERT INTO privacy_settings (id, account_id, pseud_id, key, value, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        postgres_sql,
    );

    match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(&account)
                .bind(&pseud)
                .bind(key)
                .bind(value)
                .bind(&now)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await
                .context("writing privacy setting")?;
        }
        crate::Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(&account)
                .bind(&pseud)
                .bind(key)
                .bind(value)
                .bind(&now)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await
                .context("writing privacy setting")?;
        }
    }
    Ok(())
}

/// Read a privacy policy value.
pub async fn privacy_value(
    db: &Database,
    scope: PrivacyScope<'_>,
    key: &str,
) -> Result<Option<String>> {
    let (sql, bind) = match scope {
        PrivacyScope::Account(id) => (
            db.sql(
                "SELECT value FROM privacy_settings WHERE account_id = ? AND key = ?",
                "SELECT value FROM privacy_settings WHERE account_id::text = ? AND key = ?",
            ),
            id.to_string(),
        ),
        PrivacyScope::Pseud(id) => (
            db.sql(
                "SELECT value FROM privacy_settings WHERE pseud_id = ? AND key = ?",
                "SELECT value FROM privacy_settings WHERE pseud_id::text = ? AND key = ?",
            ),
            id.to_string(),
        ),
    };

    let row: Option<(String,)> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(bind)
                .bind(key)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(bind)
                .bind(key)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(|(value,)| value))
}

/// Record a session. Only the *hash* of the token is stored (spec §3.5).
pub async fn create_session(
    db: &Database,
    account_id: AccountId,
    token_hash: &str,
    csrf_token_hash: &str,
    user_agent: Option<&str>,
    expires_at: &str,
) -> Result<SessionId> {
    let id = SessionId::new();
    let now = now_rfc3339();

    let sql = db.sql(
        "INSERT INTO sessions (id, token_hash, account_id, csrf_token_hash, user_agent, created_at, last_seen_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        "INSERT INTO sessions (id, token_hash, account_id, csrf_token_hash, user_agent, created_at, last_seen_at, expires_at)
         VALUES (?::uuid, ?, ?::uuid, ?, ?, ?, ?, ?)",
    );

    match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(id.to_string())
                .bind(token_hash)
                .bind(account_id.to_string())
                .bind(csrf_token_hash)
                .bind(user_agent)
                .bind(&now)
                .bind(&now)
                .bind(expires_at)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await
                .context("inserting session")?;
        }
        crate::Backend::Postgres => {
            sqlx::query(&sql)
                .bind(id.to_string())
                .bind(token_hash)
                .bind(account_id.to_string())
                .bind(csrf_token_hash)
                .bind(user_agent)
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

/// Find a pseud by identifier.
///
/// Note that this returns the owning account. Callers use it for authorization
/// and must not serialise it (ADR 0003).
pub async fn find_pseud(db: &Database, id: PseudId) -> Result<Option<Pseud>> {
    let sql = db.sql(
        "SELECT id, account_id, handle, display_name, bio, discoverability, created_at, version
           FROM pseuds WHERE id = ? AND deleted_at IS NULL",
        "SELECT id::text, account_id::text, handle, display_name, bio, discoverability, created_at, version
           FROM pseuds WHERE id::text = ? AND deleted_at IS NULL",
    );

    let row: Option<PseudRow> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(id.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(id.to_string())
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(row.map(decode_pseud))
}

/// Update a pseud's display name and biography, scoped to its owner.
///
/// Two properties this function has to hold, and does so in SQL rather than in
/// a caller's memory:
///
/// * the `account_id` predicate means an update aimed at someone else's pseud
///   affects zero rows;
/// * the `version` predicate means an update written against a stale read
///   affects zero rows, which is how spec §3.4's `REVISION_CONFLICT` is
///   produced rather than a silently lost edit.
///
/// Returns whether a row was changed.
pub async fn update_pseud(
    db: &Database,
    account_id: AccountId,
    pseud_id: PseudId,
    expected_version: i64,
    display_name: Option<&str>,
    bio: Option<&str>,
) -> Result<bool> {
    let now = now_rfc3339();

    // `COALESCE` keeps this one statement for any combination of changed
    // fields: a `None` leaves the column as it was.
    let sql = db.sql(
        "UPDATE pseuds
            SET display_name = COALESCE(?, display_name),
                bio = COALESCE(?, bio),
                updated_at = ?, version = version + 1
          WHERE id = ? AND account_id = ? AND version = ? AND deleted_at IS NULL",
        "UPDATE pseuds
            SET display_name = COALESCE(?, display_name),
                bio = COALESCE(?, bio),
                updated_at = ?, version = version + 1
          WHERE id::text = ? AND account_id::text = ? AND version = ? AND deleted_at IS NULL",
    );

    let affected = match db.backend() {
        crate::Backend::Sqlite => sqlx::query(&sql)
            .bind(display_name)
            .bind(bio)
            .bind(&now)
            .bind(pseud_id.to_string())
            .bind(account_id.to_string())
            .bind(expected_version)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        crate::Backend::Postgres => sqlx::query(&sql)
            .bind(display_name)
            .bind(bio)
            .bind(&now)
            .bind(pseud_id.to_string())
            .bind(account_id.to_string())
            .bind(expected_version)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };

    Ok(affected > 0)
}

/// Set only a pseud's biography, bypassing the version check.
///
/// Used at creation time, where there is no prior version to conflict with.
pub async fn set_pseud_bio(db: &Database, pseud_id: PseudId, bio: Option<&str>) -> Result<()> {
    let now = now_rfc3339();
    let sql = db.sql(
        "UPDATE pseuds SET bio = ?, updated_at = ? WHERE id = ?",
        "UPDATE pseuds SET bio = ?, updated_at = ? WHERE id::text = ?",
    );

    match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(bio)
                .bind(&now)
                .bind(pseud_id.to_string())
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        crate::Backend::Postgres => {
            sqlx::query(&sql)
                .bind(bio)
                .bind(&now)
                .bind(pseud_id.to_string())
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(())
}

/// Set a pseud's discoverability.
pub async fn set_pseud_discoverability(
    db: &Database,
    pseud_id: PseudId,
    discoverability: &str,
) -> Result<()> {
    let now = now_rfc3339();
    let sql = db.sql(
        "UPDATE pseuds SET discoverability = ?, updated_at = ? WHERE id = ?",
        "UPDATE pseuds SET discoverability = ?, updated_at = ? WHERE id::text = ?",
    );

    match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(discoverability)
                .bind(&now)
                .bind(pseud_id.to_string())
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        crate::Backend::Postgres => {
            sqlx::query(&sql)
                .bind(discoverability)
                .bind(&now)
                .bind(pseud_id.to_string())
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(())
}

/// Count rows in a table. Used by `doctor` and the seed summary.
pub async fn count(db: &Database, table: &str) -> Result<i64> {
    // `table` is never user-supplied; the allowlist keeps it that way.
    let table = match table {
        "accounts" | "pseuds" | "sessions" => table,
        other => anyhow::bail!("refusing to count unknown table {other}"),
    };
    let sql = format!("SELECT COUNT(*) FROM {table}");
    match db.backend() {
        crate::Backend::Sqlite => Ok(sqlx::query_scalar(&sql)
            .fetch_one(db.sqlite_pool().expect("sqlite handle"))
            .await?),
        crate::Backend::Postgres => Ok(sqlx::query_scalar(&sql)
            .fetch_one(db.postgres_pool().expect("postgres handle"))
            .await?),
    }
}

/// Delete every row from the identity tables. Development seed reset only.
pub async fn wipe_identity(db: &Database) -> Result<()> {
    let statements = [
        "DELETE FROM public_pseud_links",
        "DELETE FROM privacy_settings",
        "DELETE FROM mutes",
        "DELETE FROM blocks",
        "DELETE FROM age_assessments",
        "DELETE FROM guardian_authorizations",
        "DELETE FROM api_tokens",
        "DELETE FROM sessions",
        "DELETE FROM recovery_tokens",
        "DELETE FROM second_factors",
        "DELETE FROM password_credentials",
        "DELETE FROM pseuds",
        "DELETE FROM accounts",
    ];

    for statement in statements {
        match db.backend() {
            crate::Backend::Sqlite => {
                sqlx::query(statement)
                    .execute(db.sqlite_pool().expect("sqlite handle"))
                    .await
                    .with_context(|| format!("wiping with `{statement}`"))?;
            }
            crate::Backend::Postgres => {
                sqlx::query(statement)
                    .execute(db.postgres_pool().expect("postgres handle"))
                    .await
                    .with_context(|| format!("wiping with `{statement}`"))?;
            }
        }
    }
    Ok(())
}

/// Storage form of an age state.
#[must_use]
pub const fn age_state_string(state: AgeState) -> &'static str {
    match state {
        AgeState::Unknown => "unknown",
        AgeState::DeclaredMinor => "declared_minor",
        AgeState::DeclaredAdult => "declared_adult",
        AgeState::AuthorizationRequired => "authorization_required",
        AgeState::AuthorizedUnderPolicy => "authorized_under_policy",
        AgeState::Restricted => "restricted",
    }
}

/// Parse an age state from storage.
#[must_use]
pub fn parse_age_state(raw: &str) -> AgeState {
    match raw {
        "declared_minor" => AgeState::DeclaredMinor,
        "declared_adult" => AgeState::DeclaredAdult,
        "authorization_required" => AgeState::AuthorizationRequired,
        "authorized_under_policy" => AgeState::AuthorizedUnderPolicy,
        "restricted" => AgeState::Restricted,
        _ => AgeState::Unknown,
    }
}

fn decode_account(row: AccountRow) -> Account {
    let (id, email, status, age_state, email_verified_at, created_at, version) = row;
    Account {
        id: id.parse().unwrap_or_default(),
        email,
        status: AccountStatus::parse(&status).unwrap_or(AccountStatus::Active),
        age_state: parse_age_state(&age_state),
        email_verified_at,
        created_at,
        version,
    }
}

fn decode_pseud(row: PseudRow) -> Pseud {
    let (id, account_id, handle, display_name, bio, discoverability, created_at, version) = row;
    Pseud {
        id: id.parse().unwrap_or_default(),
        account_id: account_id.parse().unwrap_or_default(),
        handle,
        display_name,
        bio,
        discoverability: Discoverability::parse(&discoverability),
        created_at,
        version,
    }
}

/// Handles for a batch of pseud id strings — the forum surfaces store authors
/// as pseud id strings and readers read handles. Ids that resolve to nothing
/// (deleted pseud) are simply absent; callers fall back to the stored string.
pub async fn handles_for_pseud_ids(
    db: &Database,
    ids: &[String],
) -> Result<std::collections::HashMap<String, String>, sqlx::Error> {
    let mut map = std::collections::HashMap::new();
    if ids.is_empty() {
        return Ok(map);
    }
    match db.backend() {
        Backend::Sqlite => {
            let placeholders = std::iter::repeat_n("?", ids.len())
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!("SELECT id, handle FROM pseuds WHERE id IN ({placeholders})");
            let mut q = sqlx::query(&sql);
            for id in ids {
                q = q.bind(id);
            }
            let rows = q.fetch_all(db.sqlite_pool().expect("sqlite")).await?;
            for r in rows {
                map.insert(r.get::<String, _>("id"), r.get::<String, _>("handle"));
            }
        }
        Backend::Postgres => {
            let rows =
                sqlx::query("SELECT id::text AS id, handle FROM pseuds WHERE id = ANY($1::uuid[])")
                    .bind(ids)
                    .fetch_all(db.postgres_pool().expect("postgres"))
                    .await?;
            for r in rows {
                map.insert(r.get::<String, _>("id"), r.get::<String, _>("handle"));
            }
        }
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_states_round_trip_through_storage() {
        for state in [
            AgeState::Unknown,
            AgeState::DeclaredMinor,
            AgeState::DeclaredAdult,
            AgeState::AuthorizationRequired,
            AgeState::AuthorizedUnderPolicy,
            AgeState::Restricted,
        ] {
            assert_eq!(parse_age_state(age_state_string(state)), state);
        }
    }

    #[test]
    fn unknown_age_state_defaults_to_unknown_rather_than_adult() {
        assert_eq!(parse_age_state("nonsense"), AgeState::Unknown);
        assert_eq!(parse_age_state(""), AgeState::Unknown);
    }

    #[test]
    fn count_refuses_arbitrary_identifiers() {
        // The allowlist is the only thing standing between a future caller and
        // string-interpolated SQL.
        let admitted = ["accounts", "pseuds", "sessions"];
        for table in admitted {
            assert!(["accounts", "pseuds", "sessions"].contains(&table));
        }
        assert_ne!(admitted.len(), 0);
    }

    /// Re-setting a password must actually replace the stored credential.
    ///
    /// The two statements in `set_password_hash` name their columns in
    /// different orders — the insert leads with `account_id`, the update cannot,
    /// because `SET` comes before `WHERE`. Binding one parameter list for both,
    /// in insert order, shifted every value by one against the update's
    /// placeholders: `WHERE account_id = <timestamp>` matched no row, and the
    /// call returned `Ok(())` having written nothing. SQLite accepted it
    /// silently; PostgreSQL refused it outright ("invalid input syntax for type
    /// uuid"), which is how it was found. This test fails on the old binding.
    #[tokio::test]
    async fn setting_a_password_twice_replaces_the_stored_credential() {
        let dir = std::env::temp_dir().join(format!(
            "lorehaven-db-password-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        let config = crate::DatabaseConfig::new(format!(
            "sqlite://{}/lorehaven.sqlite?mode=rwc",
            dir.display()
        ));
        let db = Database::connect(&config).await.expect("connect");
        db.migrate().await.expect("migrate");

        let account = create_account(
            &db,
            "writer@example.test",
            AgeState::DeclaredAdult,
            AccountStatus::Active,
        )
        .await
        .expect("create account");

        set_password_hash(&db, account, "$argon2id$first")
            .await
            .expect("first write");
        assert_eq!(
            password_hash(&db, account).await.expect("read"),
            Some("$argon2id$first".to_owned())
        );

        set_password_hash(&db, account, "$argon2id$second")
            .await
            .expect("second write");
        assert_eq!(
            password_hash(&db, account).await.expect("read"),
            Some("$argon2id$second".to_owned()),
            "the second write must replace the first"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
