//! The secret store's rows (spec §10.5).
//!
//! This module stores and returns ciphertext. It has no key material, no notion
//! of who may read what, and no ability to decrypt: that separation is the
//! point. A bug in a route cannot turn these functions into a way to read a
//! credential, because there is nothing here that could.
//!
//! Milestone 5 created the table and the cipher; Milestone 6 is the first code
//! to write a row, which is why the first caller is a source credential.

use anyhow::{Context, Result};
use sqlx::FromRow;

use crate::{sql_owned, Backend, Database};

/// One encrypted record.
///
/// The plaintext is not here and cannot be produced here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretRow {
    /// The row's id, which is what `source_credentials.secret_id` names.
    pub id: String,
    /// The owning resource type, e.g. `source_credential`.
    pub owner_type: String,
    /// The owning resource id.
    pub owner_id: String,
    /// The name of the secret within its owner.
    pub name: String,
    /// Which key encrypted this row.
    pub key_id: String,
    /// The nonce, base64.
    pub nonce: String,
    /// The ciphertext, base64.
    pub ciphertext: String,
    /// When the row was written.
    pub created_at: String,
    /// When it last changed.
    pub updated_at: String,
    /// Incremented on every write.
    pub version: i64,
}

/// The columns every read of `secrets` selects, in one place.
///
/// Two readers that drift apart is how a column ends up bound to the wrong
/// field, and this table is the one place where that mistake would be a
/// credential leak rather than a wrong date.
const COLUMNS: &str = "id, owner_type, owner_id, name, key_id, nonce, ciphertext,
                       created_at, updated_at, version";

#[derive(Debug, FromRow)]
struct SecretRowRecord {
    id: String,
    owner_type: String,
    owner_id: String,
    name: String,
    key_id: String,
    nonce: String,
    ciphertext: String,
    created_at: String,
    updated_at: String,
    version: i64,
}

impl From<SecretRowRecord> for SecretRow {
    fn from(row: SecretRowRecord) -> Self {
        Self {
            id: row.id,
            owner_type: row.owner_type,
            owner_id: row.owner_id,
            name: row.name,
            key_id: row.key_id,
            nonce: row.nonce,
            ciphertext: row.ciphertext,
            created_at: row.created_at,
            updated_at: row.updated_at,
            version: row.version,
        }
    }
}

/// Write a secret, replacing any earlier secret with the same owner and name.
///
/// The id is reused on replacement rather than reissued, so a row pointing at
/// this secret does not have to be updated by whoever rotates the value. The
/// version is incremented, so a reader that cached the old ciphertext can tell
/// that it is stale.
pub async fn put_secret(
    db: &Database,
    owner_type: &str,
    owner_id: &str,
    name: &str,
    key_id: &str,
    nonce: &str,
    ciphertext: &str,
) -> Result<String> {
    let now = crate::identity::now_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();
    let sql = db.sql(
        "INSERT INTO secrets (id, owner_type, owner_id, name, key_id, nonce, ciphertext,
                              created_at, updated_at, version)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1)
         ON CONFLICT (owner_type, owner_id, name) DO UPDATE SET
             key_id = excluded.key_id,
             nonce = excluded.nonce,
             ciphertext = excluded.ciphertext,
             updated_at = excluded.updated_at,
             version = secrets.version + 1",
        "INSERT INTO secrets (id, owner_type, owner_id, name, key_id, nonce, ciphertext,
                              created_at, updated_at, version)
         VALUES (?::uuid, ?, ?, ?, ?, ?, ?, ?, ?, 1)
         ON CONFLICT (owner_type, owner_id, name) DO UPDATE SET
             key_id = excluded.key_id,
             nonce = excluded.nonce,
             ciphertext = excluded.ciphertext,
             updated_at = excluded.updated_at,
             version = secrets.version + 1",
    );
    // The returned id is the existing row's when the name was already taken, so
    // a caller that stored the same thing twice is told the same id twice.
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(owner_type)
                .bind(owner_id)
                .bind(name)
                .bind(key_id)
                .bind(nonce)
                .bind(ciphertext)
                .bind(&now)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await
                .with_context(|| format!("storing the {owner_type} secret {name}"))?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(owner_type)
                .bind(owner_id)
                .bind(name)
                .bind(key_id)
                .bind(nonce)
                .bind(ciphertext)
                .bind(&now)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await
                .with_context(|| format!("storing the {owner_type} secret {name}"))?;
        }
    }
    let stored = get_secret(db, owner_type, owner_id, name)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the secret vanished immediately after being written"))?;
    Ok(stored.id)
}

/// Read a secret's ciphertext. The caller holds the key, not this function.
pub async fn get_secret(
    db: &Database,
    owner_type: &str,
    owner_id: &str,
    name: &str,
) -> Result<Option<SecretRow>> {
    let sql = sql_owned(
        db,
        format!("SELECT {COLUMNS} FROM secrets WHERE owner_type = ? AND owner_id = ? AND name = ?"),
        format!(
            "SELECT {COLUMNS} FROM secrets
             WHERE owner_type = ? AND owner_id = ?::uuid AND name = ?"
        ),
    );
    let row: Option<SecretRowRecord> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(owner_type)
                .bind(owner_id)
                .bind(name)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(owner_type)
                .bind(owner_id)
                .bind(name)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(SecretRow::from))
}

/// Read a secret by id, which is how a row that names one finds it.
pub async fn get_secret_by_id(db: &Database, id: &str) -> Result<Option<SecretRow>> {
    let sql = sql_owned(
        db,
        format!("SELECT {COLUMNS} FROM secrets WHERE id = ?"),
        format!("SELECT {COLUMNS} FROM secrets WHERE id = ?::uuid"),
    );
    let row: Option<SecretRowRecord> = match db.backend() {
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
    Ok(row.map(SecretRow::from))
}

/// Remove a secret. `false` means there was nothing to remove.
pub async fn delete_secret(db: &Database, id: &str) -> Result<bool> {
    let sql = db.sql(
        "DELETE FROM secrets WHERE id = ?",
        "DELETE FROM secrets WHERE id = ?::uuid",
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(id)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(id)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}
