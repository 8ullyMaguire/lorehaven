//! The source revision cache (spec §10.4, §11.7).
//!
//! # What this is for
//!
//! A source that publishes an `ETag` or a `Last-Modified` is offering to tell
//! us "nothing has changed" for the price of a request with no body. That is
//! worth taking: an update check across a library, or a scheduled re-read of a
//! work, would otherwise re-download every page to discover that none of them
//! moved.
//!
//! # What it is not
//!
//! It is not a snapshot, and nothing here may be mistaken for one. An entry is
//! keyed by four things — the source, the page, the *adapter version* that read
//! it, and the security scope it was read under — and every one of those is load
//! bearing:
//!
//! * the adapter version, because a parser change means the bytes that were
//!   cached may no longer be what this build would store, so its own entries
//!   stop being used the moment the adapter's version moves;
//! * the security scope, because a page fetched with a reader's credential must
//!   never be served to a request without it. A per-source cache would leak
//!   gated content between readers, which is why the scope is part of the key
//!   rather than an afterthought.
//!
//! Every row expires on its own clock. Nothing in this module deletes a blob
//! directly: `content_blobs` is shared, and a checksum this cache stops wanting
//! may be wanted by somebody else — the collector in
//! [`crate::storage::BlobStore::delete_if_unreferenced`] is the only thing that
//! removes bytes, and only once no reference remains.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::identity::now_rfc3339;
use crate::{Backend, Database};

/// Which page of which source, read by which adapter, under whose authority.
///
/// A struct rather than a tuple because four strings of the same type in a row
/// is a bug waiting for the one caller who transposes two of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionKey<'a> {
    /// The source's key (`royalroad`).
    pub source_key: &'a str,
    /// The page, canonically: the URL the fetch actually landed on.
    pub revision_key: &'a str,
    /// The adapter version that read it. Moving this invalidates the entry.
    pub adapter_version: &'a str,
    /// Who the read was made as. `public`, or the pseud the credential belongs
    /// to — never a bare source key, which would share gated content.
    pub security_scope: &'a str,
}

/// A cached revision: the bytes, and the validators to ask with next time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevisionEntry {
    /// The checksum of the stored body, in `content_blobs`.
    pub checksum: String,
    /// The `ETag` the source gave.
    pub etag: Option<String>,
    /// The `Last-Modified` the source gave.
    pub last_modified: Option<String>,
    /// When this stops being usable, as RFC 3339.
    pub expires_at: String,
}

impl RevisionEntry {
    /// Whether the entry is worth a conditional request.
    ///
    /// An entry with neither validator can be read but not asked about, so it is
    /// only useful for its bytes — and those bytes are only useful if something
    /// else decides to serve them. A caller that cannot make a conditional
    /// request should treat this as "not cached".
    #[must_use]
    pub fn is_usable_conditionally(&self) -> bool {
        self.etag.is_some() || self.last_modified.is_some()
    }
}

/// The stored entry for a key, if it exists and has not expired.
///
/// Expiry is filtered in SQL rather than in Rust so a caller cannot forget it:
/// a function called `find_fresh` that returned stale rows would be worse than
/// no function at all.
///
/// # Errors
/// A database failure.
pub async fn find_fresh(db: &Database, key: &RevisionKey<'_>) -> Result<Option<RevisionEntry>> {
    let now = now_rfc3339();
    let sql = db.sql(
        "SELECT checksum, etag, last_modified, expires_at
         FROM source_revision_cache_entries
         WHERE source_key = ? AND revision_key = ? AND adapter_version = ?
           AND security_scope = ? AND expires_at > ?",
        "SELECT checksum, etag, last_modified, expires_at
         FROM source_revision_cache_entries
         WHERE source_key = $1 AND revision_key = $2 AND adapter_version = $3
           AND security_scope = $4 AND expires_at > $5",
    );
    let row: Option<RevisionRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(key.source_key)
                .bind(key.revision_key)
                .bind(key.adapter_version)
                .bind(key.security_scope)
                .bind(&now)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(key.source_key)
                .bind(key.revision_key)
                .bind(key.adapter_version)
                .bind(key.security_scope)
                .bind(&now)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(Into::into))
}

/// Record a revision, replacing any previous entry for the same key.
///
/// A replace rather than an append: the unique constraint is what makes the
/// cache a cache, and an implementation that accumulated rows per fetch would
/// grow without the reader ever getting a newer answer.
///
/// # Errors
/// A database failure. A checksum with no `content_blobs` row is a foreign key
/// violation, which is deliberate: the entry is useless without its bytes.
pub async fn upsert(db: &Database, key: &RevisionKey<'_>, entry: &RevisionEntry) -> Result<()> {
    let now = now_rfc3339();
    let sql = db.sql(
        "INSERT INTO source_revision_cache_entries
             (id, source_key, revision_key, adapter_version, security_scope,
              checksum, etag, last_modified, expires_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (source_key, revision_key, adapter_version, security_scope)
         DO UPDATE SET checksum = excluded.checksum,
                       etag = excluded.etag,
                       last_modified = excluded.last_modified,
                       expires_at = excluded.expires_at",
        "INSERT INTO source_revision_cache_entries
             (id, source_key, revision_key, adapter_version, security_scope,
              checksum, etag, last_modified, expires_at, created_at)
         VALUES ($1::uuid, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         ON CONFLICT (source_key, revision_key, adapter_version, security_scope)
         DO UPDATE SET checksum = excluded.checksum,
                       etag = excluded.etag,
                       last_modified = excluded.last_modified,
                       expires_at = excluded.expires_at",
    );
    let id = uuid::Uuid::new_v4().to_string();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(key.source_key)
                .bind(key.revision_key)
                .bind(key.adapter_version)
                .bind(key.security_scope)
                .bind(&entry.checksum)
                .bind(&entry.etag)
                .bind(&entry.last_modified)
                .bind(&entry.expires_at)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(key.source_key)
                .bind(key.revision_key)
                .bind(key.adapter_version)
                .bind(key.security_scope)
                .bind(&entry.checksum)
                .bind(&entry.etag)
                .bind(&entry.last_modified)
                .bind(&entry.expires_at)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(())
}

/// How many entries have expired.
///
/// Reported rather than deleted. The bytes a stale entry points at must not be
/// removed here: they may be referenced by a chapter a reader is reading, and
/// `content_blobs` is shared. Deleting the *entry* is safe and is all an
/// operator needs; deleting the blob is
/// [`crate::storage::BlobStore::delete_if_unreferenced`]'s decision, made after
/// counting references.
///
/// # Errors
/// A database failure.
pub async fn purge_expired(db: &Database) -> Result<u64> {
    let now = now_rfc3339();
    let sql = db.sql(
        "DELETE FROM source_revision_cache_entries WHERE expires_at <= ?",
        "DELETE FROM source_revision_cache_entries WHERE expires_at <= $1",
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected)
}

/// How many entries the cache holds, expired or not. For the operator's view.
///
/// # Errors
/// A database failure.
pub async fn count(db: &Database) -> Result<i64> {
    #[derive(FromRow)]
    struct Count {
        total: i64,
    }
    let sql = db.sql(
        "SELECT COUNT(*) AS total FROM source_revision_cache_entries",
        "SELECT COUNT(*)::BIGINT AS total FROM source_revision_cache_entries",
    );
    let row: Count = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.total)
}

/// Clear the cache, returning how many entries went.
///
/// An operator action, not a maintenance task: the entries are an optimisation
/// and losing them costs requests, so a person should be the one to decide.
///
/// # Errors
/// A database failure.
pub async fn clear(db: &Database) -> Result<u64> {
    let sql = "DELETE FROM source_revision_cache_entries";
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(sql)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await
            .with_context(|| "clearing the revision cache")?
            .rows_affected(),
        Backend::Postgres => sqlx::query(sql)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await
            .with_context(|| "clearing the revision cache")?
            .rows_affected(),
    };
    Ok(affected)
}

#[derive(FromRow)]
struct RevisionRow {
    checksum: String,
    etag: Option<String>,
    last_modified: Option<String>,
    expires_at: String,
}

impl From<RevisionRow> for RevisionEntry {
    fn from(row: RevisionRow) -> Self {
        Self {
            checksum: row.checksum,
            etag: row.etag,
            last_modified: row.last_modified,
            expires_at: row.expires_at,
        }
    }
}
