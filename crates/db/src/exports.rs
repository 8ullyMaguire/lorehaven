//! Export jobs, download grants and device deliveries (spec §13).
//!
//! # What the two records are for
//!
//! An **export job** is the reader's request and its outcome: what they asked to
//! be rendered, in which format, whether it is ready, and where the bytes are.
//! A **download grant** is the capability that lets them fetch it. Keeping them
//! apart is what makes the download URL a token that can be revoked, expired and
//! spent rather than a second lookup key for somebody's library.
//!
//! # Retention, and the one thing that has to be got right
//!
//! Spec §13.2 and the milestone plan both put an export's life at seven days,
//! after which the row, its grant and its output blob go. The blob is the part
//! that needs care: it is stored through [`crate::storage::BlobStore`], which is content
//! addressed, and the collector deletes blobs that nothing references. **An
//! export's output must therefore be referenced** while it is alive, or the next
//! collection run deletes a file the reader is about to download. The reference
//! is what [`record_output`] writes, and [`purge_expired`] is what takes it away
//! again — a two-step shape that mirrors how imported chapters are kept alive.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::identity::now_rfc3339;
use crate::{sql_owned, Backend, Database};

/// The owner type recorded in `content_references` for an export's output.
///
/// A constant because it is a foreign key in all but name: the collector asks
/// `content_references` whether a checksum is still wanted, and a typo here would
/// make every export's output look unreferenced and quietly deletable.
pub const EXPORT_OWNER_TYPE: &str = "export_job";

macro_rules! run {
    ($db:expr, $sql:expr, |$query:ident| $body:expr) => {
        async {
            match $db.backend() {
                Backend::Sqlite => {
                    let $query = sqlx::query(&$sql);
                    let affected = ($body)
                        .execute($db.sqlite_pool().expect("sqlite handle"))
                        .await?
                        .rows_affected();
                    Ok::<u64, sqlx::Error>(affected)
                }
                Backend::Postgres => {
                    let $query = sqlx::query(&$sql);
                    let affected = ($body)
                        .execute($db.postgres_pool().expect("postgres handle"))
                        .await?
                        .rows_affected();
                    Ok::<u64, sqlx::Error>(affected)
                }
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// One export, as the reader and the worker see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportJob {
    /// Identifier.
    pub id: String,
    /// The queue row this export rides on.
    pub job_id: String,
    /// Whose export it is. Never another account's.
    pub account_id: String,
    /// `work` or `library_item`.
    pub subject_type: String,
    /// What is being exported.
    pub subject_id: String,
    /// The format, as written when it was asked for.
    pub format: String,
    /// Per-export choices, as JSON.
    pub options_json: Option<String>,
    /// When the reader acknowledged that a delivery provider will see the
    /// content (spec §13.4).
    pub privacy_acknowledged_at: Option<String>,
    /// `queued`, `running`, `ready`, `failed` or `cancelled`.
    pub state: String,
    /// The rendered artifact's checksum, once it exists.
    pub output_blob_checksum: Option<String>,
    /// Its size in bytes.
    pub output_bytes: Option<i64>,
    /// Which converter produced it (spec §13.1's evidence requirement).
    pub converter_version: Option<String>,
    /// Why it failed, in the reader's words.
    pub error_message: Option<String>,
    /// When it was asked for.
    pub created_at: String,
    /// When it last changed.
    pub updated_at: String,
    /// Optimistic concurrency.
    pub version: i64,
}

impl ExportJob {
    /// Whether the reader has something to download.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.state == "ready" && self.output_blob_checksum.is_some()
    }

    /// Whether the export is still going to change on its own.
    #[must_use]
    pub fn is_open(&self) -> bool {
        matches!(self.state.as_str(), "queued" | "running")
    }
}

/// A download capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadGrant {
    /// Identifier.
    pub id: String,
    /// The export it opens.
    pub export_job_id: String,
    /// SHA-256 of the token, hex. Never the token.
    pub token_hash: String,
    /// When it stops working, RFC 3339.
    pub expires_at: String,
    /// When it was spent, if it has been.
    pub used_at: Option<String>,
    /// Whether spending it once is enough.
    pub single_use: bool,
    /// When it was minted.
    pub created_at: String,
}

/// A device the reader has registered for delivery or notifications.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserDevice {
    /// Identifier.
    pub id: String,
    /// Whose device it is.
    pub account_id: String,
    /// What the reader calls it.
    pub label: String,
    /// A Web Push subscription, as the browser gave it.
    pub push_subscription_json: Option<String>,
    /// When it was last seen.
    pub last_seen_at: Option<String>,
    /// When it was added.
    pub created_at: String,
    /// When it was last changed.
    pub updated_at: String,
}

#[derive(FromRow)]
struct ExportRow {
    id: String,
    job_id: String,
    account_id: String,
    subject_type: String,
    subject_id: String,
    format: String,
    options_json: Option<String>,
    privacy_acknowledged_at: Option<String>,
    state: String,
    output_blob_checksum: Option<String>,
    output_bytes: Option<i64>,
    converter_version: Option<String>,
    error_message: Option<String>,
    created_at: String,
    updated_at: String,
    version: i64,
}

impl ExportRow {
    fn into_job(self) -> Result<ExportJob> {
        Ok(ExportJob {
            id: self.id,
            job_id: self.job_id,
            account_id: self.account_id,
            subject_type: self.subject_type,
            subject_id: self.subject_id,
            format: self.format,
            options_json: self.options_json,
            privacy_acknowledged_at: self.privacy_acknowledged_at,
            state: self.state,
            output_blob_checksum: self.output_blob_checksum,
            output_bytes: self.output_bytes,
            converter_version: self.converter_version,
            error_message: self.error_message,
            created_at: self.created_at,
            updated_at: self.updated_at,
            version: self.version,
        })
    }
}

#[derive(FromRow)]
struct GrantRow {
    id: String,
    export_job_id: String,
    token_hash: String,
    expires_at: String,
    used_at: Option<String>,
    single_use: i64,
    created_at: String,
}

impl GrantRow {
    fn into_grant(self) -> DownloadGrant {
        DownloadGrant {
            id: self.id,
            export_job_id: self.export_job_id,
            token_hash: self.token_hash,
            expires_at: self.expires_at,
            used_at: self.used_at,
            single_use: self.single_use != 0,
            created_at: self.created_at,
        }
    }
}

const EXPORT_COLUMNS: &str =
    "id, job_id, account_id, subject_type, subject_id, format, options_json, \
                              privacy_acknowledged_at, state, output_blob_checksum, output_bytes, \
                              converter_version, error_message, created_at, updated_at, version";

const GRANT_COLUMNS: &str =
    "id, export_job_id, token_hash, expires_at, used_at, single_use, created_at";

const EXPORT_COLUMNS_PG: &str =
    "id::text AS id, job_id::text AS job_id, account_id::text AS account_id,      subject_id::text AS subject_id, subject_type, format, options_json, privacy_acknowledged_at,      state, output_blob_checksum, output_bytes, converter_version, error_message,      created_at, updated_at, version";

const GRANT_COLUMNS_PG: &str =
    "id::text AS id, export_job_id::text AS export_job_id, token_hash, expires_at,      used_at, single_use, created_at";

// ---------------------------------------------------------------------------
// Export jobs
// ---------------------------------------------------------------------------

/// What a new export is asked for with.
///
/// A struct rather than eight positional arguments, because five of them are
/// strings and the compiler cannot tell a subject id from an account id.
#[derive(Debug, Clone, Copy)]
pub struct NewExport<'a> {
    /// The export's identifier.
    pub id: &'a str,
    /// The queue row it rides on.
    pub job_id: &'a str,
    /// Whose export it is.
    pub account_id: &'a str,
    /// `work` or `library_item`.
    pub subject_type: &'a str,
    /// What is being exported.
    pub subject_id: &'a str,
    /// The format asked for.
    pub format: &'a str,
    /// Per-export choices, as JSON.
    pub options_json: Option<&'a str>,
}

/// Record a new export. The caller has already queued the job it rides on.
pub async fn create_export(db: &Database, new: NewExport<'_>) -> Result<ExportJob> {
    let now = now_rfc3339();
    let sql = db.sql(
        "INSERT INTO export_jobs (id, job_id, account_id, subject_type, subject_id, format, \
         options_json, state, created_at, updated_at, version) \
         VALUES (?, ?, ?, ?, ?, ?, ?, 'queued', ?, ?, 1)",
        "INSERT INTO export_jobs (id, job_id, account_id, subject_type, subject_id, format, \
         options_json, state, created_at, updated_at, version) \
         VALUES (?::uuid, ?::uuid, ?::uuid, ?, ?::uuid, ?, ?, 'queued', ?, ?, 1)",
    );
    run!(db, &sql, |query| {
        query
            .bind(new.id)
            .bind(new.job_id)
            .bind(new.account_id)
            .bind(new.subject_type)
            .bind(new.subject_id)
            .bind(new.format)
            .bind(new.options_json)
            .bind(&now)
            .bind(&now)
    })
    .await
    .with_context(|| {
        format!(
            "recording an export of {} {}",
            new.subject_type, new.subject_id
        )
    })?;

    find_export(db, new.id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the export vanished after insert"))
}

/// One export, by identifier. Not account-scoped: the routes scope their reads.
pub async fn find_export(db: &Database, id: &str) -> Result<Option<ExportJob>> {
    let sql = sql_owned(
        db,
        format!("SELECT {EXPORT_COLUMNS} FROM export_jobs WHERE id = ?"),
        format!("SELECT {EXPORT_COLUMNS_PG} FROM export_jobs WHERE id::text = ?"),
    );
    let row: Option<ExportRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    row.map(ExportRow::into_job).transpose()
}

/// One export, scoped to the account that asked for it.
pub async fn find_export_for(
    db: &Database,
    id: &str,
    account_id: &str,
) -> Result<Option<ExportJob>> {
    let sql = sql_owned(
        db,
        format!("SELECT {EXPORT_COLUMNS} FROM export_jobs WHERE id = ? AND account_id = ?"),
        format!(
            "SELECT {EXPORT_COLUMNS_PG} FROM export_jobs WHERE id::text = ? AND account_id::text = ?"
        ),
    );
    let row: Option<ExportRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(id)
                .bind(account_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(id)
                .bind(account_id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    row.map(ExportRow::into_job).transpose()
}

/// A reader's own exports, newest first.
pub async fn list_exports(
    db: &Database,
    account_id: &str,
    state: Option<&str>,
    limit: i64,
    after: Option<(&str, &str)>,
) -> Result<Vec<ExportJob>> {
    let limit = limit.clamp(1, 200);
    let sql = sql_owned(
        db,
        format!(
            "SELECT {EXPORT_COLUMNS} FROM export_jobs \
             WHERE account_id = ? AND (? IS NULL OR state = ?) \
               AND (? IS NULL OR (created_at, id) < (?, ?)) \
             ORDER BY created_at DESC, id DESC LIMIT ?"
        ),
        format!(
            "SELECT {EXPORT_COLUMNS_PG} FROM export_jobs \
             WHERE account_id::text = ? AND (? IS NULL OR state = ?) \
               AND (?::text IS NULL OR (created_at, id) < (?::text, ?::uuid)) \
             ORDER BY created_at DESC, id DESC LIMIT ?"
        ),
    );
    let (after_at, after_id) = after.map_or((None, None), |(at, id)| (Some(at), Some(id)));
    let rows: Vec<ExportRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .bind(state)
                .bind(state)
                .bind(after_at)
                .bind(after_at)
                .bind(after_id)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .bind(state)
                .bind(state)
                .bind(after_at)
                .bind(after_at)
                .bind(after_id)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    rows.into_iter().map(ExportRow::into_job).collect()
}

/// An export of the same subject and format that has not finished.
///
/// Asking twice while one is running is a client mistake rather than a new
/// request, and finding the first is what keeps an impatient reader from
/// queueing five copies of the same EPUB (spec §10.1: one job, one artifact).
pub async fn find_open_export(
    db: &Database,
    account_id: &str,
    subject_type: &str,
    subject_id: &str,
    format: &str,
) -> Result<Option<ExportJob>> {
    let sql = sql_owned(
        db,
        format!(
            "SELECT {EXPORT_COLUMNS} FROM export_jobs \
             WHERE account_id = ? AND subject_type = ? AND subject_id = ? AND format = ? \
               AND state IN ('queued', 'running') \
             ORDER BY created_at DESC LIMIT 1"
        ),
        format!(
            "SELECT {EXPORT_COLUMNS_PG} FROM export_jobs \
             WHERE account_id::text = ? AND subject_type = ? AND subject_id::text = ? AND format = ? \
               AND state IN ('queued', 'running') \
             ORDER BY created_at DESC LIMIT 1"
        ),
    );
    let row: Option<ExportRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .bind(subject_type)
                .bind(subject_id)
                .bind(format)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .bind(subject_type)
                .bind(subject_id)
                .bind(format)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    row.map(ExportRow::into_job).transpose()
}

/// Move an export to `running` (or any other state) without touching its output.
pub async fn set_export_state(db: &Database, id: &str, state: &str) -> Result<()> {
    let now = now_rfc3339();
    let sql = db.sql(
        "UPDATE export_jobs SET state = ?, updated_at = ?, version = version + 1 WHERE id = ?",
        "UPDATE export_jobs SET state = ?, updated_at = ?, version = version + 1 WHERE id::text = ?",
    );
    run!(db, &sql, |query| { query.bind(state).bind(&now).bind(id) })
        .await
        .with_context(|| format!("moving export {id} to {state}"))?;
    Ok(())
}

/// Record a rendered artifact and mark the export ready.
///
/// The reference is written first and the row second, because the collector
/// deletes blobs nothing references: a row pointing at an unreferenced blob is a
/// download that 404s, while a reference with no row is one orphaned file the
/// next sweep collects. Of the two, only the second is recoverable.
pub async fn record_output(
    db: &Database,
    id: &str,
    checksum: &str,
    bytes: i64,
    converter_version: Option<&str>,
) -> Result<()> {
    let now = now_rfc3339();
    let sql = db.sql(
        "UPDATE export_jobs SET state = 'ready', output_blob_checksum = ?, output_bytes = ?, \
         converter_version = ?, error_message = NULL, updated_at = ?, version = version + 1 \
         WHERE id = ?",
        "UPDATE export_jobs SET state = 'ready', output_blob_checksum = ?, output_bytes = ?, \
         converter_version = ?, error_message = NULL, updated_at = ?, version = version + 1 \
         WHERE id::text = ?",
    );
    run!(db, &sql, |query| {
        query
            .bind(checksum)
            .bind(bytes)
            .bind(converter_version)
            .bind(&now)
            .bind(id)
    })
    .await
    .with_context(|| format!("recording the output of export {id}"))?;
    Ok(())
}

/// Mark an export failed, with the reason the reader will see.
pub async fn fail_export(db: &Database, id: &str, message: &str) -> Result<()> {
    let now = now_rfc3339();
    let sql = db.sql(
        "UPDATE export_jobs SET state = 'failed', error_message = ?, updated_at = ?, \
         version = version + 1 WHERE id = ?",
        "UPDATE export_jobs SET state = 'failed', error_message = ?, updated_at = ?, \
         version = version + 1 WHERE id::text = ?",
    );
    run!(db, &sql, |query| {
        query.bind(message).bind(&now).bind(id)
    })
    .await
    .with_context(|| format!("failing export {id}"))?;
    Ok(())
}

/// Record that the reader acknowledged the delivery privacy notice (spec §13.4).
pub async fn acknowledge_privacy(db: &Database, id: &str) -> Result<()> {
    let now = now_rfc3339();
    let sql = db.sql(
        "UPDATE export_jobs SET privacy_acknowledged_at = ?, updated_at = ?, version = version + 1 \
         WHERE id = ? AND privacy_acknowledged_at IS NULL",
        "UPDATE export_jobs SET privacy_acknowledged_at = ?, updated_at = ?, version = version + 1 \
         WHERE id::text = ? AND privacy_acknowledged_at IS NULL",
    );
    run!(db, &sql, |query| { query.bind(&now).bind(&now).bind(id) })
        .await
        .with_context(|| format!("acknowledging the privacy notice on export {id}"))?;
    Ok(())
}

/// What a retention sweep removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PurgedExport {
    /// The export that aged out.
    pub id: String,
    /// Its output blob's checksum, if it had one — the caller unreferences it.
    pub output_blob_checksum: Option<String>,
}

/// Delete exports older than a cutoff (spec §13.2's seven days).
///
/// Returns what was removed *before* removing it, because the caller has to drop
/// the blob references and this function deliberately does not: it holds a
/// database handle and the reference lives in the same database, but deleting a
/// file is storage's business and each layer should do one thing.
pub async fn purge_expired(
    db: &Database,
    older_than: &str,
    limit: i64,
) -> Result<Vec<PurgedExport>> {
    let limit = limit.clamp(1, 1000);
    let select = sql_owned(
        db,
        format!(
            "SELECT {EXPORT_COLUMNS} FROM export_jobs WHERE created_at < ? \
             ORDER BY created_at LIMIT ?"
        ),
        format!(
            "SELECT {EXPORT_COLUMNS_PG} FROM export_jobs WHERE created_at < ? \
             ORDER BY created_at LIMIT ?"
        ),
    );
    let rows: Vec<ExportRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&select)
                .bind(older_than)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&select)
                .bind(older_than)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    let purged: Vec<PurgedExport> = rows
        .into_iter()
        .map(|row| PurgedExport {
            id: row.id,
            output_blob_checksum: row.output_blob_checksum,
        })
        .collect();
    if purged.is_empty() {
        return Ok(purged);
    }

    // One statement per row rather than an `IN` list, because the dialect
    // difference for a bound list is not worth the polymorphism: the sweep is a
    // background task with a bounded batch.
    for export in &purged {
        let sql = db.sql(
            "DELETE FROM export_jobs WHERE id = ?",
            "DELETE FROM export_jobs WHERE id::text = ?",
        );
        run!(db, &sql, |query| query.bind(&export.id))
            .await
            .with_context(|| format!("purging export {}", export.id))?;
    }
    Ok(purged)
}

// ---------------------------------------------------------------------------
// Download grants
// ---------------------------------------------------------------------------

/// Mint a capability for one export.
pub async fn mint_grant(
    db: &Database,
    id: &str,
    export_job_id: &str,
    token_hash: &str,
    expires_at: &str,
) -> Result<DownloadGrant> {
    let now = now_rfc3339();
    let sql = db.sql(
        "INSERT INTO download_grants (id, export_job_id, token_hash, expires_at, used_at, \
         single_use, created_at) VALUES (?, ?, ?, ?, NULL, 1, ?)",
        "INSERT INTO download_grants (id, export_job_id, token_hash, expires_at, used_at, \
         single_use, created_at) VALUES (?::uuid, ?::uuid, ?, ?, NULL, 1, ?)",
    );
    run!(db, &sql, |query| {
        query
            .bind(id)
            .bind(export_job_id)
            .bind(token_hash)
            .bind(expires_at)
            .bind(&now)
    })
    .await
    .with_context(|| format!("minting a download grant for export {export_job_id}"))?;
    Ok(DownloadGrant {
        id: id.to_owned(),
        export_job_id: export_job_id.to_owned(),
        token_hash: token_hash.to_owned(),
        expires_at: expires_at.to_owned(),
        used_at: None,
        single_use: true,
        created_at: now,
    })
}

/// Spend a capability, returning the export it opens.
///
/// The expiry and the single-use rule are enforced **in the statement**, not
/// around it: a check followed by an update is two statements with a window
/// between them, and the window is exactly where two simultaneous downloads of a
/// single-use token both succeed. The update's own `WHERE` is the check, and the
/// affected-row count is the answer.
///
/// # Errors
/// Never for an expired or spent grant, which is `Ok(None)`: "this token does not
/// work" is an ordinary answer for a download URL, not a fault.
pub async fn redeem_grant(db: &Database, token_hash: &str, now: &str) -> Result<Option<String>> {
    let sql = db.sql(
        "UPDATE download_grants SET used_at = ? \
         WHERE token_hash = ? AND expires_at > ? AND used_at IS NULL \
         RETURNING export_job_id",
        "UPDATE download_grants SET used_at = ? \
         WHERE token_hash = ? AND expires_at > ? AND used_at IS NULL \
         RETURNING export_job_id::text AS export_job_id",
    );
    let row: Option<(String,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(now)
                .bind(token_hash)
                .bind(now)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(now)
                .bind(token_hash)
                .bind(now)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(|(export_job_id,)| export_job_id))
}

/// The capabilities minted for one export.
///
/// Not a lookup the download path makes — that one goes by token hash, because
/// that is the only key the request carries. This exists so an operator (and a
/// test) can see what was issued, whether it was spent, and when it stops
/// working: a capability nobody can inspect is a capability nobody can revoke
/// deliberately.
pub async fn grants_for_export(db: &Database, export_job_id: &str) -> Result<Vec<DownloadGrant>> {
    let sql = sql_owned(
        db,
        format!(
            "SELECT {GRANT_COLUMNS} FROM download_grants WHERE export_job_id = ? \
             ORDER BY created_at DESC"
        ),
        format!(
            "SELECT {GRANT_COLUMNS_PG} FROM download_grants WHERE export_job_id::text = ? \
             ORDER BY created_at DESC"
        ),
    );
    let rows: Vec<GrantRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(export_job_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(export_job_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows.into_iter().map(GrantRow::into_grant).collect())
}

/// Delete one export, at its owner's request.
///
/// Its grants go with it: a capability pointing at a row that is gone can only
/// produce a 404, and leaving it behind would be a table that grows by a row for
/// every export anyone ever deleted.
///
/// # Errors
/// Transient for a storage failure.
pub async fn delete_export(db: &Database, id: &str) -> Result<bool> {
    let grants = db.sql(
        "DELETE FROM download_grants WHERE export_job_id = ?",
        "DELETE FROM download_grants WHERE export_job_id::text = ?",
    );
    run!(db, &grants, |query| query.bind(id)).await?;

    let sql = db.sql(
        "DELETE FROM export_jobs WHERE id = ?",
        "DELETE FROM export_jobs WHERE id::text = ?",
    );
    let affected = run!(db, &sql, |query| query.bind(id)).await?;
    Ok(affected > 0)
}

/// Delete grants that can no longer be spent.
pub async fn purge_expired_grants(db: &Database, now: &str) -> Result<u64> {
    let sql = db.sql(
        "DELETE FROM download_grants WHERE expires_at <= ?",
        "DELETE FROM download_grants WHERE expires_at <= ?",
    );
    let affected = run!(db, &sql, |query| query.bind(now))
        .await
        .context("purging expired download grants")?;
    Ok(affected)
}

// ---------------------------------------------------------------------------
// Devices
// ---------------------------------------------------------------------------

/// Register or refresh a device.
///
/// A device is keyed on the account and its push subscription rather than on the
/// endpoint's own id, so a browser that re-subscribes updates the row it already
/// has instead of accumulating one per reinstall.
pub async fn upsert_device(
    db: &Database,
    id: &str,
    account_id: &str,
    label: &str,
    push_subscription_json: Option<&str>,
) -> Result<UserDevice> {
    let now = now_rfc3339();
    let sql = db.sql(
        "INSERT INTO user_devices (id, account_id, label, push_subscription_json, last_seen_at, \
         created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT (id) DO UPDATE SET label = excluded.label, \
         push_subscription_json = excluded.push_subscription_json, \
         last_seen_at = excluded.last_seen_at, updated_at = excluded.updated_at",
        "INSERT INTO user_devices (id, account_id, label, push_subscription_json, last_seen_at, \
         created_at, updated_at) VALUES (?::uuid, ?::uuid, ?, ?, ?, ?, ?) \
         ON CONFLICT (id) DO UPDATE SET label = excluded.label, \
         push_subscription_json = excluded.push_subscription_json, \
         last_seen_at = excluded.last_seen_at, updated_at = excluded.updated_at",
    );
    run!(db, &sql, |query| {
        query
            .bind(id)
            .bind(account_id)
            .bind(label)
            .bind(push_subscription_json)
            .bind(&now)
            .bind(&now)
            .bind(&now)
    })
    .await
    .with_context(|| format!("registering device {id}"))?;

    Ok(UserDevice {
        id: id.to_owned(),
        account_id: account_id.to_owned(),
        label: label.to_owned(),
        push_subscription_json: push_subscription_json.map(str::to_owned),
        last_seen_at: Some(now.clone()),
        created_at: now.clone(),
        updated_at: now,
    })
}

/// A reader's devices.
///
/// # Errors
/// A database failure.
pub async fn list_devices(db: &Database, account_id: &str) -> Result<Vec<UserDevice>> {
    #[derive(FromRow)]
    struct DeviceRow {
        id: String,
        account_id: String,
        label: String,
        push_subscription_json: Option<String>,
        last_seen_at: Option<String>,
        created_at: String,
        updated_at: String,
    }
    let sql = db.sql(
        "SELECT id, account_id, label, push_subscription_json, last_seen_at, created_at, updated_at \
         FROM user_devices WHERE account_id = ? ORDER BY updated_at DESC",
        "SELECT id, account_id, label, push_subscription_json, last_seen_at, created_at, updated_at \
         FROM user_devices WHERE account_id::text = ? ORDER BY updated_at DESC",
    );
    let rows: Vec<DeviceRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(|row| UserDevice {
            id: row.id,
            account_id: row.account_id,
            label: row.label,
            push_subscription_json: row.push_subscription_json,
            last_seen_at: row.last_seen_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
        .collect())
}

/// Forget a device. Scoped to the account, so one reader cannot delete another's.
pub async fn delete_device(db: &Database, id: &str, account_id: &str) -> Result<bool> {
    let sql = db.sql(
        "DELETE FROM user_devices WHERE id = ? AND account_id = ?",
        "DELETE FROM user_devices WHERE id::text = ? AND account_id::text = ?",
    );
    let affected = run!(db, &sql, |query| query.bind(id).bind(account_id))
        .await
        .with_context(|| format!("deleting device {id}"))?;
    Ok(affected > 0)
}
