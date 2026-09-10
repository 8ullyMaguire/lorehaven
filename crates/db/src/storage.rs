//! Content-addressed blob storage (spec §10.3, §10.4).
//!
//! A blob's identity is the SHA-256 of its bytes. The storage key is derived
//! from that checksum — `objects/<first two hex>/<checksum>` under the
//! configured storage root — so no user-supplied string ever becomes a path,
//! and the same bytes stored twice are one file.
//!
//! The rules this module exists to hold:
//!
//! * **A blob is deleted only when nothing references it.**
//!   [`BlobStore::delete_if_unreferenced`] asks `content_references` and is the
//!   only safe deletion in the system. A "clean up old blobs" job that deletes
//!   by age will delete a blob a reader is streaming; there is no function here
//!   that does that, deliberately.
//! * **A write is atomic.** Bytes are written to a temporary file in the same
//!   directory and renamed into place, so a crash never leaves a half-written
//!   file under a name that claims to be complete.
//! * **A re-put of the same bytes changes nothing.** It does not move
//!   `last_referenced_at` forward, because that timestamp is what a collection
//!   sweep reads, and resurrecting a blob another transaction is in the middle
//!   of removing is how a delete loses a race with an upload that was already
//!   there.
//! * **A checksum is not an authorization credential** (spec §10.4). `get` takes
//!   a checksum and returns bytes: the *caller* must have established that the
//!   account it is serving for may see the blob. Two accounts can hold the same
//!   bytes, so nothing here may be exposed as an endpoint keyed on the checksum.
//!
//! The store is a value type holding the root path rather than a field on
//! `Database`: the database does not know where files live, and the
//! configuration that does is the application's.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use sqlx::FromRow;

use crate::identity::now_rfc3339;
use crate::{Backend, Database};

/// What a blob row says about a stored file.
#[derive(Debug, Clone, FromRow, PartialEq, Eq)]
pub struct BlobStat {
    /// SHA-256 of the bytes, hex.
    pub checksum: String,
    /// Path under the storage root.
    pub storage_key: String,
    /// Size in bytes.
    pub byte_size: i64,
    /// Media type as recorded at upload.
    pub content_type: String,
    /// Which retention class the blob is in.
    pub retention_class: String,
    /// When it was first stored.
    pub created_at: String,
    /// When a reference last pointed at it.
    pub last_referenced_at: String,
}

/// The storage root and the rules for turning a checksum into a path.
#[derive(Debug, Clone)]
pub struct BlobStore {
    root: PathBuf,
}

impl BlobStore {
    /// A store rooted at `root`. Nothing is created until something is written.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The configured root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The storage key for a checksum: two hex characters of fan-out, then the
    /// checksum. One directory per 256 blobs keeps directories small on every
    /// filesystem this will run on.
    #[must_use]
    pub fn storage_key(checksum: &str) -> String {
        let shard = checksum.get(0..2).unwrap_or("00");
        format!("objects/{shard}/{checksum}")
    }

    /// The absolute path of a stored blob.
    #[must_use]
    pub fn path_for(&self, checksum: &str) -> PathBuf {
        self.root.join(Self::storage_key(checksum))
    }

    /// Store bytes and return `(checksum, storage_key)`.
    ///
    /// Idempotent: the same bytes stored twice are one row and one file, and the
    /// second call does not touch `last_referenced_at`.
    pub async fn put(
        &self,
        db: &Database,
        bytes: &[u8],
        content_type: &str,
    ) -> Result<(String, String)> {
        self.put_with_retention(db, bytes, content_type, "snapshot")
            .await
    }

    /// Store a blob the fetch cache owns (spec §10.4).
    ///
    /// The same bytes as [`BlobStore::put`] but classified `fetch_cache`, which
    /// is a statement about why the bytes are here: a cached source page is an
    /// optimisation, and an operator clearing the cache is opting out of it.
    /// Getting the class wrong is not fatal — nothing deletes by class yet — but
    /// it is the difference between a row that explains itself and one that does
    /// not.
    ///
    /// # Errors
    /// As [`BlobStore::put`].
    pub async fn put_fetch_cache(
        &self,
        db: &Database,
        bytes: &[u8],
        content_type: &str,
    ) -> Result<(String, String)> {
        self.put_with_retention(db, bytes, content_type, "fetch_cache")
            .await
    }

    /// The shared body of both `put` variants.
    async fn put_with_retention(
        &self,
        db: &Database,
        bytes: &[u8],
        content_type: &str,
        retention: &str,
    ) -> Result<(String, String)> {
        let checksum = hex::encode(Sha256::digest(bytes));
        let storage_key = Self::storage_key(&checksum);

        // Already stored? Then this is a no-op, including the timestamp.
        if self.stat(db, &checksum).await?.is_some() && self.path_for(&checksum).exists() {
            return Ok((checksum, storage_key));
        }

        let path = self.path_for(&checksum);
        let directory = path
            .parent()
            .context("a blob path always has a parent directory")?
            .to_path_buf();
        tokio::fs::create_dir_all(&directory)
            .await
            .with_context(|| format!("creating {}", directory.display()))?;

        // Write beside the destination and rename: a reader either sees the old
        // file or the complete new one, never a half-written blob under a name
        // that claims to be complete.
        let temporary = directory.join(format!(".{checksum}.{}.tmp", std::process::id()));
        tokio::fs::write(&temporary, bytes)
            .await
            .with_context(|| format!("writing {}", temporary.display()))?;
        tokio::fs::rename(&temporary, &path)
            .await
            .with_context(|| format!("renaming into {}", path.display()))?;

        let now = now_rfc3339();
        let byte_size = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
        let sql = db.sql(
            "INSERT INTO content_blobs
                 (checksum, storage_key, byte_size, content_type, retention_class,
                  created_at, last_referenced_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (checksum) DO NOTHING",
            "INSERT INTO content_blobs
                 (checksum, storage_key, byte_size, content_type, retention_class,
                  created_at, last_referenced_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (checksum) DO NOTHING",
        );
        match db.backend() {
            Backend::Sqlite => {
                sqlx::query(&sql)
                    .bind(&checksum)
                    .bind(&storage_key)
                    .bind(byte_size)
                    .bind(content_type)
                    .bind(retention)
                    .bind(&now)
                    .bind(&now)
                    .execute(db.sqlite_pool().expect("sqlite handle"))
                    .await?;
            }
            Backend::Postgres => {
                sqlx::query(&sql)
                    .bind(&checksum)
                    .bind(&storage_key)
                    .bind(byte_size)
                    .bind(content_type)
                    .bind(retention)
                    .bind(&now)
                    .bind(&now)
                    .execute(db.postgres_pool().expect("postgres handle"))
                    .await?;
            }
        }
        Ok((checksum, storage_key))
    }

    /// Read a blob's bytes.
    ///
    /// `None` means the store does not hold it. A row without its file is an
    /// error, not `None`: "the database says we have it and the disk says we do
    /// not" is a fault to investigate, and reporting it as "not found" would hide
    /// data loss.
    pub async fn get(&self, db: &Database, checksum: &str) -> Result<Option<Vec<u8>>> {
        if self.stat(db, checksum).await?.is_none() {
            return Ok(None);
        }
        let path = self.path_for(checksum);
        match tokio::fs::read(&path).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => bail!(
                "blob {checksum} is recorded in the database but {} is missing",
                path.display()
            ),
            Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// The blob's row, without reading its bytes.
    pub async fn stat(&self, db: &Database, checksum: &str) -> Result<Option<BlobStat>> {
        let sql = db.sql(
            "SELECT checksum, storage_key, byte_size, content_type, retention_class,
                    created_at, last_referenced_at
               FROM content_blobs WHERE checksum = ?",
            "SELECT checksum, storage_key, byte_size, content_type, retention_class,
                    created_at, last_referenced_at
               FROM content_blobs WHERE checksum = ?",
        );
        Ok(match db.backend() {
            Backend::Sqlite => {
                sqlx::query_as(&sql)
                    .bind(checksum)
                    .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                    .await?
            }
            Backend::Postgres => {
                sqlx::query_as(&sql)
                    .bind(checksum)
                    .fetch_optional(db.postgres_pool().expect("postgres handle"))
                    .await?
            }
        })
    }

    /// Record that `owner_type`/`owner_id` keeps this blob alive.
    pub async fn reference(
        &self,
        db: &Database,
        checksum: &str,
        owner_type: &str,
        owner_id: &str,
    ) -> Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_rfc3339();
        let sql = db.sql(
            "INSERT INTO content_references (id, checksum, owner_type, owner_id, created_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT (checksum, owner_type, owner_id) DO NOTHING",
            "INSERT INTO content_references (id, checksum, owner_type, owner_id, created_at)
             VALUES (?::uuid, ?, ?, ?, ?)
             ON CONFLICT (checksum, owner_type, owner_id) DO NOTHING",
        );
        match db.backend() {
            Backend::Sqlite => {
                sqlx::query(&sql)
                    .bind(&id)
                    .bind(checksum)
                    .bind(owner_type)
                    .bind(owner_id)
                    .bind(&now)
                    .execute(db.sqlite_pool().expect("sqlite handle"))
                    .await?;
            }
            Backend::Postgres => {
                sqlx::query(&sql)
                    .bind(&id)
                    .bind(checksum)
                    .bind(owner_type)
                    .bind(owner_id)
                    .bind(&now)
                    .execute(db.postgres_pool().expect("postgres handle"))
                    .await?;
            }
        }
        self.touch(db, checksum).await
    }

    /// Drop one reference. `true` means a row was removed.
    ///
    /// The blob itself is untouched: removal is [`Self::delete_if_unreferenced`]
    /// and nothing else.
    pub async fn unreference(
        &self,
        db: &Database,
        checksum: &str,
        owner_type: &str,
        owner_id: &str,
    ) -> Result<bool> {
        let sql = db.sql(
            "DELETE FROM content_references
              WHERE checksum = ? AND owner_type = ? AND owner_id = ?",
            "DELETE FROM content_references
              WHERE checksum = ? AND owner_type = ? AND owner_id = ?",
        );
        let affected = match db.backend() {
            Backend::Sqlite => sqlx::query(&sql)
                .bind(checksum)
                .bind(owner_type)
                .bind(owner_id)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?
                .rows_affected(),
            Backend::Postgres => sqlx::query(&sql)
                .bind(checksum)
                .bind(owner_type)
                .bind(owner_id)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?
                .rows_affected(),
        };
        if affected > 0 {
            self.touch(db, checksum).await?;
        }
        Ok(affected > 0)
    }

    /// Remove the blob and its file, but only if nothing references it.
    ///
    /// `true` means it was removed. This is the only deletion in the system: the
    /// check and the delete happen in one transaction, so two collectors cannot
    /// both decide to remove the same blob, and a reference that arrives after
    /// the check finds a blob row that is already gone and must store the bytes
    /// again rather than resurrect a file mid-delete.
    pub async fn delete_if_unreferenced(&self, db: &Database, checksum: &str) -> Result<bool> {
        let sql = db.sql(
            "DELETE FROM content_blobs
              WHERE checksum = ?
                AND NOT EXISTS (SELECT 1 FROM content_references WHERE checksum = ?)",
            "DELETE FROM content_blobs
              WHERE checksum = ?
                AND NOT EXISTS (SELECT 1 FROM content_references WHERE checksum = ?)",
        );

        let removed = match db.backend() {
            Backend::Sqlite => {
                let pool = db.sqlite_pool().expect("sqlite handle");
                let mut tx = pool.begin().await?;
                let affected = sqlx::query(&sql)
                    .bind(checksum)
                    .bind(checksum)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected();
                tx.commit().await?;
                affected > 0
            }
            Backend::Postgres => {
                let pool = db.postgres_pool().expect("postgres handle");
                let mut tx = pool.begin().await?;
                let affected = sqlx::query(&sql)
                    .bind(checksum)
                    .bind(checksum)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected();
                tx.commit().await?;
                affected > 0
            }
        };

        if !removed {
            return Ok(false);
        }

        let path = self.path_for(checksum);
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
            Err(error) => Err(error).with_context(|| format!("removing {}", path.display())),
        }
    }

    /// Every blob row that nothing references, oldest reference first. This is
    /// the collection sweep's input; it deletes nothing.
    pub async fn unreferenced(&self, db: &Database, limit: i64) -> Result<Vec<String>> {
        let sql = db.sql(
            "SELECT b.checksum FROM content_blobs b
              WHERE NOT EXISTS (
                    SELECT 1 FROM content_references r WHERE r.checksum = b.checksum)
              ORDER BY b.last_referenced_at ASC LIMIT ?",
            "SELECT b.checksum FROM content_blobs b
              WHERE NOT EXISTS (
                    SELECT 1 FROM content_references r WHERE r.checksum = b.checksum)
              ORDER BY b.last_referenced_at ASC LIMIT ?",
        );
        let rows: Vec<(String,)> = match db.backend() {
            Backend::Sqlite => {
                sqlx::query_as(&sql)
                    .bind(limit)
                    .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                    .await?
            }
            Backend::Postgres => {
                sqlx::query_as(&sql)
                    .bind(limit)
                    .fetch_all(db.postgres_pool().expect("postgres handle"))
                    .await?
            }
        };
        Ok(rows.into_iter().map(|(checksum,)| checksum).collect())
    }

    /// How many bytes this instance is storing, and how many blobs.
    pub async fn usage(&self, db: &Database) -> Result<(i64, i64)> {
        let sql = db.sql(
            "SELECT COUNT(*) AS blobs, COALESCE(SUM(byte_size), 0) AS bytes FROM content_blobs",
            "SELECT COUNT(*)::bigint AS blobs, COALESCE(SUM(byte_size), 0)::bigint AS bytes
               FROM content_blobs",
        );
        let (blobs, bytes): (i64, i64) = match db.backend() {
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
        Ok((blobs, bytes))
    }

    /// Move `last_referenced_at` forward. Called when a reference changes, never
    /// on a re-put of bytes that are already stored.
    async fn touch(&self, db: &Database, checksum: &str) -> Result<()> {
        let sql = db.sql(
            "UPDATE content_blobs SET last_referenced_at = ? WHERE checksum = ?",
            "UPDATE content_blobs SET last_referenced_at = ? WHERE checksum = ?",
        );
        let now = now_rfc3339();
        match db.backend() {
            Backend::Sqlite => {
                sqlx::query(&sql)
                    .bind(&now)
                    .bind(checksum)
                    .execute(db.sqlite_pool().expect("sqlite handle"))
                    .await?;
            }
            Backend::Postgres => {
                sqlx::query(&sql)
                    .bind(&now)
                    .bind(checksum)
                    .execute(db.postgres_pool().expect("postgres handle"))
                    .await?;
            }
        }
        Ok(())
    }
}
