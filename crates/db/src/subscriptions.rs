//! M21 — Content subscriptions and saved-search alerts repository (spec §23.3, §14.2).

use sqlx::Row;
use uuid::Uuid;

use crate::{Backend, Database};

// ---------------------------------------------------------------------------
// Common types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SubscriptionRow {
    pub id: String,
    pub subject_type: String,
    pub subject_id: String,
    pub state: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct AlertRow {
    pub id: String,
    pub saved_search_id: String,
    pub frequency: String,
    pub last_run_at: Option<String>,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// Internal helpers: each dialect returns a common type
// ---------------------------------------------------------------------------

async fn fetch_subs_sqlite(
    pool: &sqlx::SqlitePool,
    pseud_id: &str,
) -> Result<Vec<SubscriptionRow>, sqlx::Error> {
    let rows = sqlx::query("SELECT id, subject_type, subject_id, state, created_at FROM content_subscriptions WHERE subscriber_pseud_id = ? ORDER BY created_at DESC")
        .bind(pseud_id)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| SubscriptionRow {
            id: r.get::<String, _>("id"),
            subject_type: r.get::<String, _>("subject_type"),
            subject_id: r.get::<String, _>("subject_id"),
            state: r.get::<String, _>("state"),
            created_at: r.get::<String, _>("created_at"),
        })
        .collect())
}

async fn fetch_subs_postgres(
    pool: &sqlx::postgres::PgPool,
    pseud_id: &str,
) -> Result<Vec<SubscriptionRow>, sqlx::Error> {
    let rows = sqlx::query("SELECT id::text, subject_type, subject_id, state, created_at FROM content_subscriptions WHERE subscriber_pseud_id = $1::uuid ORDER BY created_at DESC")
        .bind(pseud_id)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| SubscriptionRow {
            id: r.get::<String, _>("id"),
            subject_type: r.get::<String, _>("subject_type"),
            subject_id: r.get::<String, _>("subject_id"),
            state: r.get::<String, _>("state"),
            created_at: r.get::<String, _>("created_at"),
        })
        .collect())
}

async fn fetch_active_sub_owner_sqlite(
    pool: &sqlx::SqlitePool,
    subject_type: &str,
    subject_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    let r: Option<String> = sqlx::query_scalar("SELECT subscriber_pseud_id FROM content_subscriptions WHERE subject_type = ? AND subject_id = ? AND state = 'active'")
        .bind(subject_type).bind(subject_id)
        .fetch_optional(pool)
        .await?;
    Ok(r)
}

async fn fetch_active_sub_owner_postgres(
    pool: &sqlx::postgres::PgPool,
    subject_type: &str,
    subject_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    let r: Option<String> = sqlx::query_scalar("SELECT subscriber_pseud_id::text FROM content_subscriptions WHERE subject_type = $1 AND subject_id = $2 AND state = 'active'")
        .bind(subject_type).bind(subject_id)
        .fetch_optional(pool)
        .await?;
    Ok(r)
}

async fn fetch_alerts_sqlite(
    pool: &sqlx::SqlitePool,
    owner_pseud_id: &str,
) -> Result<Vec<AlertRow>, sqlx::Error> {
    let rows = sqlx::query("SELECT id, saved_search_id, frequency, last_run_at, created_at FROM search_alerts WHERE owner_pseud_id = ? ORDER BY created_at DESC")
        .bind(owner_pseud_id)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| AlertRow {
            id: r.get::<String, _>("id"),
            saved_search_id: r.get::<String, _>("saved_search_id"),
            frequency: r.get::<String, _>("frequency"),
            last_run_at: r.get::<Option<String>, _>("last_run_at"),
            created_at: r.get::<String, _>("created_at"),
        })
        .collect())
}

async fn fetch_alerts_postgres(
    pool: &sqlx::postgres::PgPool,
    owner_pseud_id: &str,
) -> Result<Vec<AlertRow>, sqlx::Error> {
    let rows = sqlx::query("SELECT id::text, saved_search_id::text, frequency, last_run_at, created_at FROM search_alerts WHERE owner_pseud_id = $1::uuid ORDER BY created_at DESC")
        .bind(owner_pseud_id)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| AlertRow {
            id: r.get::<String, _>("id"),
            saved_search_id: r.get::<String, _>("saved_search_id"),
            frequency: r.get::<String, _>("frequency"),
            last_run_at: r.get::<Option<String>, _>("last_run_at"),
            created_at: r.get::<String, _>("created_at"),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Subscribe a pseud to a content subject. Returns the subscription id.
pub async fn subscribe_work(
    db: &Database,
    subscriber_pseud_id: &str,
    subject_type: &str,
    subject_id: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO content_subscriptions (id, subscriber_pseud_id, subject_type, subject_id, state, created_at)
                 VALUES (?, ?, ?, ?, 'active', ?)
                 ON CONFLICT(subscriber_pseud_id, subject_type, subject_id) DO UPDATE SET state = 'active'"
            )
            .bind(&id).bind(subscriber_pseud_id).bind(subject_type).bind(subject_id).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO content_subscriptions (id, subscriber_pseud_id, subject_type, subject_id, state, created_at) VALUES ($1::uuid, $2::uuid, $3, $4, 'active', $5)
                 ON CONFLICT(subscriber_pseud_id, subject_type, subject_id) DO UPDATE SET state = 'active'"
            )
            .bind(&id).bind(subscriber_pseud_id).bind(subject_type).bind(subject_id).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

/// Pause or resume a subscription. Returns rows affected.
pub async fn set_subscription_state(
    db: &Database,
    id: &str,
    state: &str,
) -> Result<u64, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let r = sqlx::query("UPDATE content_subscriptions SET state = ? WHERE id = ?")
                .bind(state)
                .bind(id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(r.rows_affected())
        }
        Backend::Postgres => {
            let r = sqlx::query("UPDATE content_subscriptions SET state = $1 WHERE id = $2::uuid")
                .bind(state)
                .bind(id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(r.rows_affected())
        }
    }
}

/// Unsubscribe. Returns rows affected.
pub async fn unsubscribe_work(
    db: &Database,
    subscriber_pseud_id: &str,
    subject_type: &str,
    subject_id: &str,
) -> Result<u64, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let r = sqlx::query("DELETE FROM content_subscriptions WHERE subscriber_pseud_id = ? AND subject_type = ? AND subject_id = ?")
                .bind(subscriber_pseud_id).bind(subject_type).bind(subject_id)
                .execute(db.sqlite_pool().expect("sqlite")).await?;
            Ok(r.rows_affected())
        }
        Backend::Postgres => {
            let r = sqlx::query("DELETE FROM content_subscriptions WHERE subscriber_pseud_id = $1::uuid AND subject_type = $2 AND subject_id = $3")
                .bind(subscriber_pseud_id).bind(subject_type).bind(subject_id)
                .execute(db.postgres_pool().expect("postgres")).await?;
            Ok(r.rows_affected())
        }
    }
}

/// List all subscriptions for a pseud.
pub async fn list_subscriptions(
    db: &Database,
    pseud_id: &str,
) -> Result<Vec<SubscriptionRow>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => fetch_subs_sqlite(db.sqlite_pool().expect("sqlite"), pseud_id).await,
        Backend::Postgres => {
            fetch_subs_postgres(db.postgres_pool().expect("postgres"), pseud_id).await
        }
    }
}

/// Count active subscribers for a subject.
pub async fn count_subscribers(
    db: &Database,
    subject_type: &str,
    subject_id: &str,
) -> Result<i64, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            count_subs_sqlite(db.sqlite_pool().expect("sqlite"), subject_type, subject_id).await
        }
        Backend::Postgres => {
            count_subs_postgres(
                db.postgres_pool().expect("postgres"),
                subject_type,
                subject_id,
            )
            .await
        }
    }
}

async fn count_subs_sqlite(
    pool: &sqlx::SqlitePool,
    subject_type: &str,
    subject_id: &str,
) -> Result<i64, sqlx::Error> {
    let r: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM content_subscriptions WHERE subject_type = ? AND subject_id = ? AND state = 'active'")
        .bind(subject_type).bind(subject_id)
        .fetch_one(pool)
        .await?;
    Ok(r)
}

async fn count_subs_postgres(
    pool: &sqlx::postgres::PgPool,
    subject_type: &str,
    subject_id: &str,
) -> Result<i64, sqlx::Error> {
    let r: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM content_subscriptions WHERE subject_type = $1 AND subject_id = $2 AND state = 'active'")
        .bind(subject_type).bind(subject_id)
        .fetch_one(pool)
        .await?;
    Ok(r)
}

/// Get the owner pseud of the most recent active subscriber for a subject.
pub async fn active_subscriber_for_subject(
    db: &Database,
    subject_type: &str,
    subject_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            fetch_active_sub_owner_sqlite(
                db.sqlite_pool().expect("sqlite"),
                subject_type,
                subject_id,
            )
            .await
        }
        Backend::Postgres => {
            fetch_active_sub_owner_postgres(
                db.postgres_pool().expect("postgres"),
                subject_type,
                subject_id,
            )
            .await
        }
    }
}

/// Create a saved-search alert. Returns the alert id.
///
/// The `owner_pseud_id` column matches the migration schema (not
/// `subscriber_pseud_id`).
pub async fn create_alert(
    db: &Database,
    owner_pseud_id: &str,
    saved_search_id: &str,
    frequency: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO search_alerts (id, owner_pseud_id, saved_search_id, frequency, last_run_at, created_at)
                 VALUES (?, ?, ?, ?, NULL, ?)
                 ON CONFLICT(owner_pseud_id, saved_search_id) DO UPDATE SET frequency = excluded.frequency"
            )
            .bind(&id).bind(owner_pseud_id).bind(saved_search_id).bind(frequency).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO search_alerts (id, owner_pseud_id, saved_search_id, frequency, last_run_at, created_at) VALUES ($1::uuid, $2::uuid, $3::uuid, $4, NULL, $5)
                 ON CONFLICT(owner_pseud_id, saved_search_id) DO UPDATE SET frequency = excluded.frequency"
            )
            .bind(&id).bind(owner_pseud_id).bind(saved_search_id).bind(frequency).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

/// List alerts for an owner pseud.
pub async fn list_alerts(
    db: &Database,
    owner_pseud_id: &str,
) -> Result<Vec<AlertRow>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            fetch_alerts_sqlite(db.sqlite_pool().expect("sqlite"), owner_pseud_id).await
        }
        Backend::Postgres => {
            fetch_alerts_postgres(db.postgres_pool().expect("postgres"), owner_pseud_id).await
        }
    }
}

/// Delete an alert. Returns rows affected.
pub async fn delete_alert(db: &Database, id: &str) -> Result<u64, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let r = sqlx::query("DELETE FROM search_alerts WHERE id = ?")
                .bind(id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(r.rows_affected())
        }
        Backend::Postgres => {
            let r = sqlx::query("DELETE FROM search_alerts WHERE id = $1::uuid")
                .bind(id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(r.rows_affected())
        }
    }
}

/// Mark an alert as recently run.
pub async fn mark_alert_run(db: &Database, id: &str) -> Result<u64, sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let r = sqlx::query("UPDATE search_alerts SET last_run_at = ? WHERE id = ?")
                .bind(&now)
                .bind(id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(r.rows_affected())
        }
        Backend::Postgres => {
            let r = sqlx::query("UPDATE search_alerts SET last_run_at = $1 WHERE id = $2::uuid")
                .bind(&now)
                .bind(id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(r.rows_affected())
        }
    }
}

/// AI-training opt-in for an author pseud.
pub async fn ai_training_opt_in(
    db: &Database,
    pseud_id: &str,
    opt_in: bool,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO author_ai_training (pseud_id, opt_in, updated_at) VALUES (?, ?, ?)
                 ON CONFLICT(pseud_id) DO UPDATE SET opt_in = excluded.opt_in, updated_at = excluded.updated_at"
            )
            .bind(pseud_id).bind(opt_in).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO author_ai_training (pseud_id, opt_in, updated_at) VALUES ($1, $2, $3)
                 ON CONFLICT(pseud_id) DO UPDATE SET opt_in = excluded.opt_in, updated_at = excluded.updated_at"
            )
            .bind(pseud_id).bind(opt_in).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(())
}

/// AI-training opt-out for an author pseud.
pub async fn ai_training_opt_out(db: &Database, pseud_id: &str) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "UPDATE author_ai_training SET opt_in = 0, updated_at = ? WHERE pseud_id = ?",
            )
            .bind(&now)
            .bind(pseud_id)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "UPDATE author_ai_training SET opt_in = 0, updated_at = $1 WHERE pseud_id = $2",
            )
            .bind(&now)
            .bind(pseud_id)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(())
}

/// Get AI-training opt-in status for a pseud.
pub async fn ai_training_status(db: &Database, pseud_id: &str) -> Result<bool, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let r: Option<i64> =
                sqlx::query_scalar("SELECT opt_in FROM author_ai_training WHERE pseud_id = ?")
                    .bind(pseud_id)
                    .fetch_optional(db.sqlite_pool().expect("sqlite"))
                    .await?;
            Ok(r.unwrap_or(0) != 0)
        }
        Backend::Postgres => {
            let r: Option<bool> =
                sqlx::query_scalar("SELECT opt_in FROM author_ai_training WHERE pseud_id = $1")
                    .bind(pseud_id)
                    .fetch_optional(db.postgres_pool().expect("postgres"))
                    .await?;
            Ok(r.unwrap_or(false))
        }
    }
}

/// Post an AI-training opt-out handler for a pseud.
pub async fn ai_training_opt_out_handler(db: &Database, pseud_id: &str) -> Result<(), sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("DELETE FROM author_ai_training WHERE pseud_id = ? AND opt_in = 0")
                .bind(pseud_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query("DELETE FROM author_ai_training WHERE pseud_id = $1 AND opt_in = 0")
                .bind(pseud_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Post an AI-training opt-out handler for a pseud, with a delay.
pub async fn ai_training_opt_out_delayed(
    db: &Database,
    pseud_id: &str,
    delay_seconds: i64,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("UPDATE author_ai_training SET opt_in = 0, updated_at = datetime(?, '+' || ? || ' seconds') WHERE pseud_id = ?")
                .bind(&now).bind(delay_seconds).bind(pseud_id)
                .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query("UPDATE author_ai_training SET opt_in = 0, updated_at = $1 + ($2 || ' seconds')::interval WHERE pseud_id = $3")
                .bind(&now).bind(delay_seconds).bind(pseud_id)
                .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(())
}

/// Post an AI-training opt-out handler for a pseud, with a delay, as a transaction.
pub async fn ai_training_opt_out_delayed_tx(
    db: &Database,
    pseud_id: &str,
    delay_seconds: i64,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let mut tx = db.sqlite_pool().expect("sqlite").begin().await?;
            sqlx::query("UPDATE author_ai_training SET opt_in = 0, updated_at = datetime(?, '+' || ? || ' seconds') WHERE pseud_id = ?")
                .bind(&now).bind(delay_seconds).bind(pseud_id)
                .execute(&mut *tx).await?;
            tx.commit().await?;
        }
        Backend::Postgres => {
            let mut tx = db.postgres_pool().expect("postgres").begin().await?;
            sqlx::query("UPDATE author_ai_training SET opt_in = 0, updated_at = $1 + ($2 || ' seconds')::interval WHERE pseud_id = $3")
                .bind(&now).bind(delay_seconds).bind(pseud_id)
                .execute(&mut *tx).await?;
            tx.commit().await?;
        }
    }
    Ok(())
}

/// List AI-training opt-in status for all pseuds.
pub async fn list_ai_training_statuses(db: &Database) -> Result<Vec<(String, bool)>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query("SELECT pseud_id, opt_in FROM author_ai_training")
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(rows
                .into_iter()
                .map(|r| {
                    (
                        r.get::<String, _>("pseud_id"),
                        r.get::<i64, _>("opt_in") != 0,
                    )
                })
                .collect())
        }
        Backend::Postgres => {
            let rows = sqlx::query("SELECT pseud_id, opt_in FROM author_ai_training")
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(rows
                .into_iter()
                .map(|r| (r.get::<String, _>("pseud_id"), r.get::<bool, _>("opt_in")))
                .collect())
        }
    }
}
