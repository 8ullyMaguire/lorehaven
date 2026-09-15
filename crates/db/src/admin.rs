//! M19 — Admin repository: admin actions, feature flags, abuse tracking, privacy requests.

use uuid::Uuid;

use crate::{Backend, Database};

// ---------------------------------------------------------------------------
// Admin Actions
// ---------------------------------------------------------------------------

pub async fn record_admin_action(
    db: &Database,
    actor: &str,
    action: &str,
    subject_type: &str,
    subject_id: &str,
    document: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO admin_actions (id, actor, action, subject_type, subject_id, document, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)"
            )
            .bind(&id).bind(actor).bind(action).bind(subject_type).bind(subject_id).bind(document).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO admin_actions (id, actor, action, subject_type, subject_id, document, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)"
            )
            .bind(&id).bind(actor).bind(action).bind(subject_type).bind(subject_id).bind(document).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

// ---------------------------------------------------------------------------
// Abuse Counters
// ---------------------------------------------------------------------------

pub async fn increment_abuse_counter(
    db: &Database,
    key: &str,
    window: &str,
) -> Result<i64, sqlx::Error> {
    let count = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(
                "INSERT INTO abuse_counters (key, window, count) VALUES (?, ?, 1)
                 ON CONFLICT(key, window) DO UPDATE SET count = count + 1 RETURNING count",
            )
            .bind(key)
            .bind(window)
            .fetch_one(db.sqlite_pool().expect("sqlite"))
            .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(
                "INSERT INTO abuse_counters (key, window, count) VALUES ($1, $2, 1)
                 ON CONFLICT(key, window) DO UPDATE SET count = count + 1 RETURNING count",
            )
            .bind(key)
            .bind(window)
            .fetch_one(db.postgres_pool().expect("postgres"))
            .await?
        }
    };
    Ok(count)
}

// ---------------------------------------------------------------------------
// Privacy Requests
// ---------------------------------------------------------------------------

pub async fn create_privacy_request(
    db: &Database,
    account: &str,
    kind: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO privacy_requests (id, account, kind, state, requested_at)
                 VALUES (?, ?, ?, 'pending', ?)",
            )
            .bind(&id)
            .bind(account)
            .bind(kind)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO privacy_requests (id, account, kind, state, requested_at)
                 VALUES ($1, $2, $3, 'pending', $4)",
            )
            .bind(&id)
            .bind(account)
            .bind(kind)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(id)
}

pub async fn complete_privacy_request(
    db: &Database,
    request_id: &str,
    result_ref: Option<&str>,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("UPDATE privacy_requests SET state = 'done', completed_at = ?, result_ref = ? WHERE id = ?")
                .bind(&now).bind(result_ref).bind(request_id)
                .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query("UPDATE privacy_requests SET state = 'done', completed_at = $1, result_ref = $2 WHERE id = $3")
                .bind(&now).bind(result_ref).bind(request_id)
                .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(())
}
