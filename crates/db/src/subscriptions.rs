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

async fn fetch_subs_sqlite(pool: &sqlx::SqlitePool, pseud_id: &str) -> Result<Vec<SubscriptionRow>, sqlx::Error> {
    let rows = sqlx::query("SELECT id, subject_type, subject_id, state, created_at FROM content_subscriptions WHERE subscriber_pseud_id = ? ORDER BY created_at DESC")
        .bind(pseud_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(|r| SubscriptionRow {
        id: r.get::<String, _>("id"),
        subject_type: r.get::<String, _>("subject_type"),
        subject_id: r.get::<String, _>("subject_id"),
        state: r.get::<String, _>("state"),
        created_at: r.get::<String, _>("created_at"),
    }).collect())
}

async fn fetch_subs_postgres(pool: &sqlx::postgres::PgPool, pseud_id: &str) -> Result<Vec<SubscriptionRow>, sqlx::Error> {
    let rows = sqlx::query("SELECT id, subject_type, subject_id, state, created_at FROM content_subscriptions WHERE subscriber_pseud_id = $1 ORDER BY created_at DESC")
        .bind(pseud_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(|r| SubscriptionRow {
        id: r.get::<String, _>("id"),
        subject_type: r.get::<String, _>("subject_type"),
        subject_id: r.get::<String, _>("subject_id"),
        state: r.get::<String, _>("state"),
        created_at: r.get::<String, _>("created_at"),
    }).collect())
}

async fn fetch_sub_ids_sqlite(pool: &sqlx::SqlitePool, subject_type: &str, subject_id: &str) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT subscriber_pseud_id FROM content_subscriptions WHERE subject_type = ? AND subject_id = ? AND state = 'active'")
        .bind(subject_type).bind(subject_id)
        .fetch_all(pool)
        .await
}

async fn fetch_sub_ids_postgres(pool: &sqlx::postgres::PgPool, subject_type: &str, subject_id: &str) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT subscriber_pseud_id FROM content_subscriptions WHERE subject_type = $1 AND subject_id = $2 AND state = 'active'")
        .bind(subject_type).bind(subject_id)
        .fetch_all(pool)
        .await
}

async fn fetch_alerts_sqlite(pool: &sqlx::SqlitePool, pseud_id: &str) -> Result<Vec<AlertRow>, sqlx::Error> {
    let rows = sqlx::query("SELECT id, saved_search_id, frequency, last_run_at, created_at FROM search_alerts WHERE subscriber_pseud_id = ? ORDER BY created_at DESC")
        .bind(pseud_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(|r| AlertRow {
        id: r.get::<String, _>("id"),
        saved_search_id: r.get::<String, _>("saved_search_id"),
        frequency: r.get::<String, _>("frequency"),
        last_run_at: r.get::<Option<String>, _>("last_run_at"),
        created_at: r.get::<String, _>("created_at"),
    }).collect())
}

async fn fetch_alerts_postgres(pool: &sqlx::postgres::PgPool, pseud_id: &str) -> Result<Vec<AlertRow>, sqlx::Error> {
    let rows = sqlx::query("SELECT id, saved_search_id, frequency, last_run_at, created_at FROM search_alerts WHERE subscriber_pseud_id = $1 ORDER BY created_at DESC")
        .bind(pseud_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(|r| AlertRow {
        id: r.get::<String, _>("id"),
        saved_search_id: r.get::<String, _>("saved_search_id"),
        frequency: r.get::<String, _>("frequency"),
        last_run_at: r.get::<Option<String>, _>("last_run_at"),
        created_at: r.get::<String, _>("created_at"),
    }).collect())
}

async fn count_subs_sqlite(pool: &sqlx::SqlitePool, subject_type: &str, subject_id: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM content_subscriptions WHERE subject_type = ? AND subject_id = ? AND state = 'active'")
        .bind(subject_type).bind(subject_id)
        .fetch_one(pool)
        .await
}

async fn count_subs_postgres(pool: &sqlx::postgres::PgPool, subject_type: &str, subject_id: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM content_subscriptions WHERE subject_type = $1 AND subject_id = $2 AND state = 'active'")
        .bind(subject_type).bind(subject_id)
        .fetch_one(pool)
        .await
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Subscribe to content. Returns the subscription id.
pub async fn subscribe(
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
                "INSERT INTO content_subscriptions (id, subscriber_pseud_id, subject_type, subject_id, state, created_at)
                 VALUES ($1, $2, $3, $4, 'active', $5)
                 ON CONFLICT(subscriber_pseud_id, subject_type, subject_id) DO UPDATE SET state = 'active'"
            )
            .bind(&id).bind(subscriber_pseud_id).bind(subject_type).bind(subject_id).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

/// Pause or resume a subscription.
pub async fn set_subscription_state(db: &Database, id: &str, state: &str) -> Result<u64, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let r = sqlx::query("UPDATE content_subscriptions SET state = ? WHERE id = ?")
                .bind(state).bind(id)
                .execute(db.sqlite_pool().expect("sqlite")).await?;
            Ok(r.rows_affected())
        }
        Backend::Postgres => {
            let r = sqlx::query("UPDATE content_subscriptions SET state = $1 WHERE id = $2")
                .bind(state).bind(id)
                .execute(db.postgres_pool().expect("postgres")).await?;
            Ok(r.rows_affected())
        }
    }
}

/// Unsubscribe (hard delete).
pub async fn unsubscribe(db: &Database, id: &str) -> Result<u64, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let r = sqlx::query("DELETE FROM content_subscriptions WHERE id = ?")
                .bind(id).execute(db.sqlite_pool().expect("sqlite")).await?;
            Ok(r.rows_affected())
        }
        Backend::Postgres => {
            let r = sqlx::query("DELETE FROM content_subscriptions WHERE id = $1")
                .bind(id).execute(db.postgres_pool().expect("postgres")).await?;
            Ok(r.rows_affected())
        }
    }
}

/// Get subscriptions for a pseud.
pub async fn get_subscriptions(db: &Database, pseud_id: &str) -> Result<Vec<SubscriptionRow>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => fetch_subs_sqlite(db.sqlite_pool().expect("sqlite"), pseud_id).await,
        Backend::Postgres => fetch_subs_postgres(db.postgres_pool().expect("postgres"), pseud_id).await,
    }
}

/// Get active subscriber pseud ids for a subject.
pub async fn get_subscribers(db: &Database, subject_type: &str, subject_id: &str) -> Result<Vec<String>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => fetch_sub_ids_sqlite(db.sqlite_pool().expect("sqlite"), subject_type, subject_id).await,
        Backend::Postgres => fetch_sub_ids_postgres(db.postgres_pool().expect("postgres"), subject_type, subject_id).await,
    }
}

/// Count active subscribers for a subject.
pub async fn count_subscribers(db: &Database, subject_type: &str, subject_id: &str) -> Result<i64, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => count_subs_sqlite(db.sqlite_pool().expect("sqlite"), subject_type, subject_id).await,
        Backend::Postgres => count_subs_postgres(db.postgres_pool().expect("postgres"), subject_type, subject_id).await,
    }
}

/// Create a saved-search alert.
pub async fn create_alert(
    db: &Database,
    subscriber_pseud_id: &str,
    saved_search_id: &str,
    frequency: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO search_alerts (id, subscriber_pseud_id, saved_search_id, frequency, last_run_at, created_at)
                 VALUES (?, ?, ?, ?, NULL, ?)
                 ON CONFLICT(subscriber_pseud_id, saved_search_id) DO UPDATE SET frequency = excluded.frequency"
            )
            .bind(&id).bind(subscriber_pseud_id).bind(saved_search_id).bind(frequency).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO search_alerts (id, subscriber_pseud_id, saved_search_id, frequency, last_run_at, created_at)
                 VALUES ($1, $2, $3, $4, NULL, $5)
                 ON CONFLICT(subscriber_pseud_id, saved_search_id) DO UPDATE SET frequency = excluded.frequency"
            )
            .bind(&id).bind(subscriber_pseud_id).bind(saved_search_id).bind(frequency).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

/// Update alert frequency.
pub async fn update_alert(db: &Database, id: &str, frequency: &str) -> Result<u64, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let r = sqlx::query("UPDATE search_alerts SET frequency = ? WHERE id = ?")
                .bind(frequency).bind(id)
                .execute(db.sqlite_pool().expect("sqlite")).await?;
            Ok(r.rows_affected())
        }
        Backend::Postgres => {
            let r = sqlx::query("UPDATE search_alerts SET frequency = $1 WHERE id = $2")
                .bind(frequency).bind(id)
                .execute(db.postgres_pool().expect("postgres")).await?;
            Ok(r.rows_affected())
        }
    }
}

/// Delete an alert.
pub async fn delete_alert(db: &Database, id: &str) -> Result<u64, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let r = sqlx::query("DELETE FROM search_alerts WHERE id = ?")
                .bind(id).execute(db.sqlite_pool().expect("sqlite")).await?;
            Ok(r.rows_affected())
        }
        Backend::Postgres => {
            let r = sqlx::query("DELETE FROM search_alerts WHERE id = $1")
                .bind(id).execute(db.postgres_pool().expect("postgres")).await?;
            Ok(r.rows_affected())
        }
    }
}

/// Get alerts for a pseud.
pub async fn get_alerts(db: &Database, pseud_id: &str) -> Result<Vec<AlertRow>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => fetch_alerts_sqlite(db.sqlite_pool().expect("sqlite"), pseud_id).await,
        Backend::Postgres => fetch_alerts_postgres(db.postgres_pool().expect("postgres"), pseud_id).await,
    }
}

/// Update the last_run_at timestamp for an alert.
pub async fn mark_alert_run(db: &Database, id: &str) -> Result<u64, sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let r = sqlx::query("UPDATE search_alerts SET last_run_at = ? WHERE id = ?")
                .bind(&now).bind(id)
                .execute(db.sqlite_pool().expect("sqlite")).await?;
            Ok(r.rows_affected())
        }
        Backend::Postgres => {
            let r = sqlx::query("UPDATE search_alerts SET last_run_at = $1 WHERE id = $2")
                .bind(&now).bind(id)
                .execute(db.postgres_pool().expect("postgres")).await?;
            Ok(r.rows_affected())
        }
    }
}
