//! M15 — Economy repository: ledger, holds, caps, queue, subscriptions.

use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

use crate::{Backend, Database};
use lorehaven_domain::economy::TxnType;

// ---------------------------------------------------------------------------
// Internal helpers: each dialect returns a common type
// ---------------------------------------------------------------------------

async fn fetch_balances_sqlite(
    pool: &sqlx::SqlitePool,
    account: &str,
) -> Result<Vec<(String, i64)>, sqlx::Error> {
    let rows = sqlx::query("SELECT bucket, SUM(amount_bp) AS total FROM credit_entries WHERE account = ? GROUP BY bucket")
        .bind(account)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| (r.get::<String, _>("bucket"), r.get::<i64, _>("total")))
        .collect())
}

async fn fetch_balances_postgres(
    pool: &sqlx::postgres::PgPool,
    account: &str,
) -> Result<Vec<(String, i64)>, sqlx::Error> {
    let rows = sqlx::query("SELECT bucket, SUM(amount_bp) AS total FROM credit_entries WHERE account = $1 GROUP BY bucket")
        .bind(account)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| (r.get::<String, _>("bucket"), r.get::<i64, _>("total")))
        .collect())
}

async fn fetch_queue_sqlite(
    pool: &sqlx::SqlitePool,
    job_id: &str,
) -> Result<Option<(String, i64)>, sqlx::Error> {
    let row = sqlx::query("SELECT priority_class, position FROM queue_slots WHERE job_id = ?")
        .bind(job_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| {
        (
            r.get::<String, _>("priority_class"),
            r.get::<i64, _>("position"),
        )
    }))
}

async fn fetch_queue_postgres(
    pool: &sqlx::postgres::PgPool,
    job_id: &str,
) -> Result<Option<(String, i64)>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT priority_class, position::bigint AS position FROM queue_slots WHERE job_id = $1",
    )
    .bind(job_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| {
        (
            r.get::<String, _>("priority_class"),
            r.get::<i64, _>("position"),
        )
    }))
}

async fn fetch_usage_sqlite(
    pool: &sqlx::SqlitePool,
    account: &str,
    day: &str,
) -> Result<Vec<(String, i64)>, sqlx::Error> {
    let rows =
        sqlx::query("SELECT action, count FROM usage_counters WHERE account = ? AND day = ?")
            .bind(account)
            .bind(day)
            .fetch_all(pool)
            .await?;
    Ok(rows
        .iter()
        .map(|r| {
            (
                r.get::<String, _>("action"),
                r.get::<i32, _>("count") as i64,
            )
        })
        .collect())
}

async fn fetch_usage_postgres(
    pool: &sqlx::postgres::PgPool,
    account: &str,
    day: &str,
) -> Result<Vec<(String, i64)>, sqlx::Error> {
    let rows =
        sqlx::query("SELECT action, count FROM usage_counters WHERE account = $1 AND day = $2")
            .bind(account)
            .bind(day)
            .fetch_all(pool)
            .await?;
    Ok(rows
        .iter()
        .map(|r| {
            (
                r.get::<String, _>("action"),
                r.get::<i32, _>("count") as i64,
            )
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Post a balanced credit transaction. Replay-safe via idempotency key.
pub async fn post_transaction(
    db: &Database,
    txn_type: TxnType,
    idempotency_key: &str,
    reference: &str,
    entries: &[(String, String, i64)], // (account, bucket, amount)
) -> Result<String, sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    let txn_id = Uuid::new_v4().to_string();

    // Check for replay
    let existing: Option<String> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar("SELECT id FROM credit_transactions WHERE idempotency_key = ?")
                .bind(idempotency_key)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar("SELECT id FROM credit_transactions WHERE idempotency_key = $1")
                .bind(idempotency_key)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };

    if let Some(id) = existing {
        return Ok(id);
    }

    // Insert transaction + entries
    match db.backend() {
        Backend::Sqlite => {
            let mut tx = db.sqlite_pool().expect("sqlite").begin().await?;
            sqlx::query(
                "INSERT INTO credit_transactions (id, type, idempotency_key, reference, created_at)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(&txn_id)
            .bind(txn_type.as_str())
            .bind(idempotency_key)
            .bind(reference)
            .bind(&now)
            .execute(&mut *tx)
            .await?;

            for (account, bucket, amount) in entries {
                sqlx::query(
                    "INSERT INTO credit_entries (transaction_id, account, bucket, amount_bp, created_at)
                     VALUES (?, ?, ?, ?, ?)"
                )
                .bind(&txn_id).bind(account).bind(bucket).bind(amount).bind(&now)
                .execute(&mut *tx).await?;
            }
            tx.commit().await?;
        }
        Backend::Postgres => {
            let mut tx = db.postgres_pool().expect("postgres").begin().await?;
            sqlx::query(
                "INSERT INTO credit_transactions (id, type, idempotency_key, reference, created_at)
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(&txn_id)
            .bind(txn_type.as_str())
            .bind(idempotency_key)
            .bind(reference)
            .bind(&now)
            .execute(&mut *tx)
            .await?;

            for (account, bucket, amount) in entries {
                sqlx::query(
                    "INSERT INTO credit_entries (transaction_id, account, bucket, amount_bp, created_at)
                     VALUES ($1, $2, $3, $4, $5)"
                )
                .bind(&txn_id).bind(account).bind(bucket).bind(amount).bind(&now)
                .execute(&mut *tx).await?;
            }
            tx.commit().await?;
        }
    }
    Ok(txn_id)
}

/// Get balances by bucket for an account.
pub async fn balances(db: &Database, account: &str) -> Result<Vec<(String, i64)>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => fetch_balances_sqlite(db.sqlite_pool().expect("sqlite"), account).await,
        Backend::Postgres => {
            fetch_balances_postgres(db.postgres_pool().expect("postgres"), account).await
        }
    }
}

/// Reserve a credit hold for a job.
pub async fn reserve_hold(
    db: &Database,
    account: &str,
    job_id: &str,
    amount: i64,
    ttl_seconds: i64,
) -> Result<String, sqlx::Error> {
    let now = time::OffsetDateTime::now_utc();
    let expires_at = now + time::Duration::seconds(ttl_seconds);
    let hold_id = Uuid::new_v4().to_string();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO credit_holds (id, account, amount, job_id, expires_at)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(&hold_id)
            .bind(account)
            .bind(amount)
            .bind(job_id)
            .bind(expires_at.to_string())
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO credit_holds (id, account, amount, job_id, expires_at)
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(&hold_id)
            .bind(account)
            .bind(amount)
            .bind(job_id)
            .bind(expires_at.to_string())
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(hold_id)
}

/// Release a hold (failure path).
pub async fn release_hold(db: &Database, hold_id: &str) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("UPDATE credit_holds SET released_at = ? WHERE id = ?")
                .bind(&now)
                .bind(hold_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query("UPDATE credit_holds SET released_at = $1 WHERE id = $2")
                .bind(&now)
                .bind(hold_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Capture a hold (actual charge).
pub async fn capture_hold(db: &Database, hold_id: &str, actual: i64) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("UPDATE credit_holds SET captured_at = ?, amount = ? WHERE id = ?")
                .bind(&now)
                .bind(actual)
                .bind(hold_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query("UPDATE credit_holds SET captured_at = $1, amount = $2 WHERE id = $3")
                .bind(&now)
                .bind(actual)
                .bind(hold_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Enqueue a job with priority class.
pub async fn enqueue_job(
    db: &Database,
    job_id: &str,
    priority_class: &str,
) -> Result<i64, sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    let max_pos: Option<i64> = match db.backend() {
        Backend::Sqlite => {
            let val: Option<i64> = sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE(MAX(position), 0) FROM queue_slots WHERE priority_class = ?",
            )
            .bind(priority_class)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?;
            val
        }
        Backend::Postgres => {
            let val: Option<i64> = sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(position), 0)::bigint FROM queue_slots WHERE priority_class = $1")
                .bind(priority_class)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?;
            val
        }
    };

    let position = max_pos.map(|m| m + 1).unwrap_or(1);
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO queue_slots (job_id, priority_class, position, enqueued_at)
                 VALUES (?, ?, ?, ?)",
            )
            .bind(job_id)
            .bind(priority_class)
            .bind(position)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO queue_slots (job_id, priority_class, position, enqueued_at)
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(job_id)
            .bind(priority_class)
            .bind(position)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(position)
}

/// Get queue position for a job.
pub async fn queue_position(
    db: &Database,
    job_id: &str,
) -> Result<Option<(String, i64)>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => fetch_queue_sqlite(db.sqlite_pool().expect("sqlite"), job_id).await,
        Backend::Postgres => {
            fetch_queue_postgres(db.postgres_pool().expect("postgres"), job_id).await
        }
    }
}

/// Bump a usage counter and return (count, cap).
pub async fn bump_counter(
    db: &Database,
    account: &str,
    action: &str,
    day: &str,
    cap: i64,
) -> Result<(i64, i64), sqlx::Error> {
    let count: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(
                "INSERT INTO usage_counters (account, action, day, count)
                 VALUES (?, ?, ?, 1)
                 ON CONFLICT(account, action, day) DO UPDATE SET count = count + 1
                 RETURNING count",
            )
            .bind(account)
            .bind(action)
            .bind(day)
            .fetch_one(db.sqlite_pool().expect("sqlite"))
            .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(
                "INSERT INTO usage_counters (account, action, day, count)
                 VALUES ($1, $2, $3, 1)
                 ON CONFLICT(account, action, day) DO UPDATE SET count = usage_counters.count + 1
                 RETURNING count::bigint",
            )
            .bind(account)
            .bind(action)
            .bind(day)
            .fetch_one(db.postgres_pool().expect("postgres"))
            .await?
        }
    };
    Ok((count, cap))
}

/// Get usage for an account on a day.
pub async fn usage_for(
    db: &Database,
    account: &str,
    day: &str,
) -> Result<Vec<(String, i64)>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            fetch_usage_sqlite(db.sqlite_pool().expect("sqlite"), account, day).await
        }
        Backend::Postgres => {
            fetch_usage_postgres(db.postgres_pool().expect("postgres"), account, day).await
        }
    }
}

// ---------------------------------------------------------------------------
// Bounties
// ---------------------------------------------------------------------------

pub async fn create_bounty(
    db: &Database,
    id: &str,
    account: &str,
    job_kind: &str,
    terms: &str,
    amount: i64,
    created_at: &str,
) -> Result<(), sqlx::Error> {
    let sql = db.sql(
        "INSERT INTO bounties (id, job_kind, terms, escrow_transaction, state, claimant, created_by, created_at, account, amount) VALUES (?, ?, ?, 'pending', 'open', '', '', ?, ?, ?)",
        "INSERT INTO bounties (id, job_kind, terms, escrow_transaction, state, claimant, created_by, created_at, account, amount) VALUES ($1, $2, $3, 'pending', 'open', '', '', $4, $5, $6)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(id)
                .bind(job_kind)
                .bind(terms)
                .bind(created_at)
                .bind(account)
                .bind(amount)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(id)
                .bind(job_kind)
                .bind(terms)
                .bind(created_at)
                .bind(account)
                .bind(amount)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// `amount` is `INTEGER` in the schema, so it decodes as `i32`; SQLite hands
/// back `i64` for the same column. The row type is per-backend for that reason
/// and nothing else -- decoding an INT4 column into an `i64` is the error
/// "mismatched types; Rust type `i64` (as SQL type `INT8`) is not compatible with
/// SQL type `INT4", which is what this table returned on PostgreSQL for as long
/// as it had no PostgreSQL arm at all.
type BountyRow = (String, String, String, String, Option<i64>, String, String);

/// A row rendered as the JSON the API returns, so both arms agree.
fn bounty_json(
    id: &str,
    account: &str,
    job_kind: &str,
    terms: &str,
    amount: Option<i64>,
    state: &str,
    created_at: &str,
) -> Value {
    json!({
        "id": id,
        "account": account,
        "job_kind": job_kind,
        "terms": terms,
        "amount": amount.unwrap_or(0),
        "state": state,
        "created_at": created_at,
    })
}

pub async fn list_bounties(db: &Database) -> Result<Vec<Value>, sqlx::Error> {
    // `amount` is INTEGER (INT4) in PostgreSQL and `i64` in SQLite. The SQLite
    // arm cannot spell a cast that SQLite rejects, so each arm gets its own
    // text: widening in SQL is what lets a single row type decode both.
    const SELECT_SQLITE: &str = "SELECT id, account, job_kind, terms, amount, state, created_at \
                                 FROM bounties WHERE state = 'open' \
                                 ORDER BY created_at DESC LIMIT 50";
    const SELECT_POSTGRES: &str =
        "SELECT id, account, job_kind, terms, amount::bigint, state, created_at \
         FROM bounties WHERE state = 'open' \
         ORDER BY created_at DESC LIMIT 50";
    match db.backend() {
        Backend::Sqlite => {
            let rows: Vec<BountyRow> =
                sqlx::query_as(SELECT_SQLITE).fetch_all(db.sqlite_pool().expect("sqlite")).await?;
            Ok(rows
                .into_iter()
                .map(|r| bounty_json(&r.0, &r.1, &r.2, &r.3, r.4, &r.5, &r.6))
                .collect())
        }
        Backend::Postgres => {
            let rows: Vec<BountyRow> =
                sqlx::query_as(SELECT_POSTGRES).fetch_all(db.postgres_pool().expect("postgres")).await?;
            Ok(rows
                .into_iter()
                .map(|r| bounty_json(&r.0, &r.1, &r.2, &r.3, r.4, &r.5, &r.6))
                .collect())
        }
    }
}

pub async fn claim_bounty(db: &Database, id: &str, fulfilled_by: &str) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "UPDATE bounties SET state = 'claimed', fulfilled_at = ?, fulfilled_by = ? WHERE id = ? AND state = 'open'",
        "UPDATE bounties SET state = 'claimed', fulfilled_at = $1, fulfilled_by = $2 WHERE id = $3 AND state = 'open'",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(now)
                .bind(fulfilled_by)
                .bind(id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(now)
                .bind(fulfilled_by)
                .bind(id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}
