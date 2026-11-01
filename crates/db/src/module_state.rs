//! §59.6 — per-user module state (M65-04).
//!
//! ## What this table is, and what it is not
//!
//! One row per account per module the reader has **actually changed**. No row means *inherit*: the
//! instance's decision is the answer. A table of "yes" rows for every account would make "what did
//! this reader change" unanswerable, and the settings page would have to diff two lists to show it.
//!
//! ## Narrowing is the writer's job, not the schema's
//!
//! §59.3 says an account may only narrow. That rule lives here in one place — [`set_off`] writes
//! `enabled = 0`, and the only other write is [`clear`], which removes the row — so there is no
//! function in this module that can widen a surface. The route above this module therefore cannot
//! widen one by mistake either: it has nothing to call.
//!
//! The `enabled` column exists anyway because a row's meaning is "this reader has an opinion", and
//! the column records which opinion. A later spec change that permits per-user widening fills it
//! rather than backfilling a table that never stored the fact.
//!
//! ## One indexed read on the way into a surface
//!
//! [`is_off`] is called by the module gate for `per_user` surfaces when the request carries a
//! session. It is a primary-key lookup on a table that holds a handful of rows, and the M65-04 plan
//! records the cache that would skip it (M65-09) rather than half-building one here.

use anyhow::Result;
use sqlx::Row;

use crate::{Backend, Database};

/// Whether this reader has switched `module_id` off.
///
/// `false` for a reader with no row, which is every reader who has not changed anything.
pub async fn is_off(db: &Database, account: &str, module_id: &str) -> Result<bool> {
    // `enabled` is INT4 on PostgreSQL, and this crate reads integers as i64 — an
    // uncast `get::<i64>` panics on the PG arm while SQLite stays green. M45-25
    // learned that with `likes`.
    let sql = db.sql(
        "SELECT enabled AS enabled FROM module_user_state WHERE account_id = ? AND module_id = ?",
        "SELECT CAST(enabled AS BIGINT) AS enabled FROM module_user_state \
         WHERE account_id = $1 AND module_id = $2",
    );
    let enabled = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(account)
            .bind(module_id)
            .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .map(|row| row.get::<i64, _>("enabled")),
        Backend::Postgres => sqlx::query(&sql)
            .bind(account)
            .bind(module_id)
            .fetch_optional(db.postgres_pool().expect("postgres handle"))
            .await?
            .map(|row| row.get::<i64, _>("enabled")),
    };
    Ok(enabled == Some(0))
}

/// Every module this reader has switched off, in id order.
///
/// The catalogue reports this so a settings page can render "you changed these" without asking
/// about every row: the list is what changed, and it is usually empty.
pub async fn overrides_for(db: &Database, account: &str) -> Result<Vec<String>> {
    let sql = db.sql(
        "SELECT module_id FROM module_user_state WHERE account_id = ? AND enabled = 0 \
         ORDER BY module_id ASC",
        "SELECT module_id FROM module_user_state WHERE account_id = $1 AND enabled = 0 \
         ORDER BY module_id ASC",
    );
    let rows: Vec<String> = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(account)
            .fetch_all(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .iter()
            .map(|row| row.get::<String, _>("module_id"))
            .collect(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(account)
            .fetch_all(db.postgres_pool().expect("postgres handle"))
            .await?
            .iter()
            .map(|row| row.get::<String, _>("module_id"))
            .collect(),
    };
    Ok(rows)
}

/// Switch `module_id` off for this reader. Idempotent.
pub async fn set_off(db: &Database, account: &str, module_id: &str, now: &str) -> Result<()> {
    let sql = db.sql(
        "INSERT INTO module_user_state (account_id, module_id, enabled, updated_at) \
         VALUES (?, ?, 0, ?) \
         ON CONFLICT (account_id, module_id) DO UPDATE SET enabled = 0, updated_at = excluded.updated_at",
        "INSERT INTO module_user_state (account_id, module_id, enabled, updated_at) \
         VALUES ($1, $2, 0, $3) \
         ON CONFLICT (account_id, module_id) DO UPDATE SET enabled = 0, updated_at = EXCLUDED.updated_at",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(account)
                .bind(module_id)
                .bind(now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(account)
                .bind(module_id)
                .bind(now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(())
}

/// Drop this reader's row for `module_id`: the instance's decision applies again.
///
/// Returns whether a row was there, so a caller can answer "was anything changed" honestly rather
/// than reporting an `enabled: true` that was already true.
pub async fn clear(db: &Database, account: &str, module_id: &str) -> Result<bool> {
    let sql = db.sql(
        "DELETE FROM module_user_state WHERE account_id = ? AND module_id = ?",
        "DELETE FROM module_user_state WHERE account_id = $1 AND module_id = $2",
    );
    let removed = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(account)
            .bind(module_id)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(account)
            .bind(module_id)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(removed > 0)
}

/// How many override rows exist on this instance, over all accounts.
///
/// Reported rather than used as a gate: the count is what an operator wants to know before
/// wondering whether the feature works at all, and it is the number M65-09 would cache.
pub async fn count(db: &Database) -> Result<i64> {
    let sql = db.sql(
        "SELECT COUNT(*) AS n FROM module_user_state",
        "SELECT CAST(COUNT(*) AS BIGINT) AS n FROM module_user_state",
    );
    let n = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .fetch_one(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .get::<i64, _>("n"),
        Backend::Postgres => sqlx::query(&sql)
            .fetch_one(db.postgres_pool().expect("postgres handle"))
            .await?
            .get::<i64, _>("n"),
    };
    Ok(n)
}
