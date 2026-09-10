//! The transactional outbox (spec §8.4, §10.1).
//!
//! Every side effect that must happen exactly once — a notification, a search
//! index update, a cache invalidation — is written as a row in the *same
//! transaction* as the change that caused it. The worker (Milestone 5) drains
//! this table later. Nothing in a transaction ever sends mail or calls out.
//!
//! Idempotency comes from `dedupe_key`: the unique index refuses a second row
//! with the same key, so a retried publication cannot notify twice, no matter
//! how many times the client retries (spec §8 acceptance).

use anyhow::{Context, Result};
use sqlx::FromRow;

use lorehaven_domain::OutboxEventId;

use crate::identity::now_rfc3339;
use crate::{Backend, Database};

/// A queued side effect.
#[derive(Debug, Clone, FromRow)]
pub struct OutboxEvent {
    /// Identifier.
    pub id: String,
    /// What kind of side effect this is.
    pub topic: String,
    /// Its JSON payload.
    pub payload: String,
    /// How many times delivery has been attempted.
    pub attempts: i64,
    /// Creation time, RFC 3339.
    pub created_at: String,
}

/// Record a side effect to be delivered later.
///
/// `dedupe_key` makes the write idempotent: the second insert of the same key
/// changes nothing, and the caller is told so rather than being handed an
/// error it cannot act on.
pub async fn enqueue(
    db: &Database,
    topic: &str,
    payload: &str,
    dedupe_key: Option<&str>,
) -> Result<bool> {
    let id = OutboxEventId::new();
    let now = now_rfc3339();

    let sql = db.sql(
        "INSERT INTO outbox_events (id, topic, payload, dedupe_key, created_at, available_at, attempts)
         VALUES (?, ?, ?, ?, ?, ?, 0)
         ON CONFLICT DO NOTHING",
        "INSERT INTO outbox_events (id, topic, payload, dedupe_key, created_at, available_at, attempts)
         VALUES (?::uuid, ?, ?, ?, ?, ?, 0)
         ON CONFLICT DO NOTHING",
    );

    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(id.to_string())
            .bind(topic)
            .bind(payload)
            .bind(dedupe_key)
            .bind(&now)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await
            .context("enqueueing outbox event")?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(id.to_string())
            .bind(topic)
            .bind(payload)
            .bind(dedupe_key)
            .bind(&now)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await
            .context("enqueueing outbox event")?
            .rows_affected(),
    };

    Ok(affected > 0)
}

/// Undelivered events that are due.
pub async fn pending(db: &Database, limit: i64) -> Result<Vec<OutboxEvent>> {
    let sql = db.sql(
        "SELECT id, topic, payload, attempts, created_at FROM outbox_events
          WHERE delivered_at IS NULL AND claimed_at IS NULL AND available_at <= ?
          ORDER BY created_at ASC LIMIT ?",
        "SELECT id::text AS id, topic, payload, attempts, created_at FROM outbox_events
          WHERE delivered_at IS NULL AND claimed_at IS NULL AND available_at <= ?
          ORDER BY created_at ASC LIMIT ?",
    );

    let rows: Vec<OutboxEvent> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(now_rfc3339())
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(now_rfc3339())
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(rows)
}

/// Mark an event delivered.
pub async fn mark_delivered(db: &Database, id: OutboxEventId) -> Result<()> {
    let sql = db.sql(
        "UPDATE outbox_events SET delivered_at = ?, claimed_at = NULL, attempts = attempts + 1
          WHERE id = ?",
        "UPDATE outbox_events SET delivered_at = ?, claimed_at = NULL, attempts = attempts + 1
          WHERE id = ?::uuid",
    );
    let now = now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&now)
                .bind(id.to_string())
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&now)
                .bind(id.to_string())
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(())
}

/// Record a failed delivery attempt and when to try again.
pub async fn mark_failed(
    db: &Database,
    id: OutboxEventId,
    error: &str,
    retry_at: &str,
) -> Result<()> {
    let sql = db.sql(
        "UPDATE outbox_events
            SET attempts = attempts + 1, claimed_at = NULL, available_at = ?, last_error = ?
          WHERE id = ?",
        "UPDATE outbox_events
            SET attempts = attempts + 1, claimed_at = NULL, available_at = ?, last_error = ?
          WHERE id = ?::uuid",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(retry_at)
                .bind(error)
                .bind(id.to_string())
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(retry_at)
                .bind(error)
                .bind(id.to_string())
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(())
}

/// How many events are still undelivered.
pub async fn undelivered_count(db: &Database) -> Result<i64> {
    let sql = "SELECT COUNT(*) FROM outbox_events WHERE delivered_at IS NULL";
    match db.backend() {
        Backend::Sqlite => Ok(sqlx::query_scalar(sql)
            .fetch_one(db.sqlite_pool().expect("sqlite handle"))
            .await?),
        Backend::Postgres => Ok(sqlx::query_scalar(sql)
            .fetch_one(db.postgres_pool().expect("postgres handle"))
            .await?),
    }
}

/// Topics recorded for a work, for introspection in tests and `doctor`.
pub async fn topics_for_work(db: &Database, work: lorehaven_domain::WorkId) -> Result<Vec<String>> {
    let sql = db.sql(
        "SELECT topic FROM outbox_events WHERE payload LIKE ? ORDER BY created_at ASC",
        "SELECT topic FROM outbox_events WHERE payload LIKE ? ORDER BY created_at ASC",
    );
    let pattern = format!("%\"work_id\":\"{work}\"%");

    let rows: Vec<(String,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(pattern)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(pattern)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(rows.into_iter().map(|(topic,)| topic).collect())
}
