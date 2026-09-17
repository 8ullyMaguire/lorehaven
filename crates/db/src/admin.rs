//! M19 — Admin repository: admin actions, feature flags, abuse tracking, privacy requests.

use uuid::Uuid;
use serde_json::Value;
use sqlx::Row;

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
                "INSERT INTO abuse_counters (key, \"window\", count) VALUES (?, ?, 1)
                 ON CONFLICT(key, \"window\") DO UPDATE SET count = count + 1
                 RETURNING count",
            )
            .bind(key)
            .bind(window)
            .fetch_one(db.sqlite_pool().expect("sqlite"))
            .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(
                "INSERT INTO abuse_counters (key, \"window\", count) VALUES ($1, $2, 1)
                 ON CONFLICT(key, \"window\") DO UPDATE SET count = abuse_counters.count + 1
                 RETURNING count::bigint",
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

// ---------------------------------------------------------------------------
// Privacy request listing
// ---------------------------------------------------------------------------

pub async fn list_privacy_requests(
    db: &Database,
    account: &str,
) -> Result<Vec<Value>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query("SELECT id, account, kind, state, requested_at, completed_at, result_ref FROM privacy_requests WHERE account = ? ORDER BY requested_at DESC")
                .bind(account)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(rows.iter().map(|r| {
                serde_json::json!({
                    "id": r.get::<String, _>("id"),
                    "account": r.get::<String, _>("account"),
                    "kind": r.get::<String, _>("kind"),
                    "state": r.get::<String, _>("state"),
                    "requested_at": r.get::<String, _>("requested_at"),
                    "completed_at": r.get::<Option<String>, _>("completed_at"),
                    "result_ref": r.get::<Option<String>, _>("result_ref"),
                })
            }).collect())
        }
        Backend::Postgres => {
            let rows = sqlx::query("SELECT id, account, kind, state, requested_at, completed_at, result_ref FROM privacy_requests WHERE account = $1 ORDER BY requested_at DESC")
                .bind(account)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(rows.iter().map(|r| {
                serde_json::json!({
                    "id": r.get::<String, _>("id"),
                    "account": r.get::<String, _>("account"),
                    "kind": r.get::<String, _>("kind"),
                    "state": r.get::<String, _>("state"),
                    "requested_at": r.get::<String, _>("requested_at"),
                    "completed_at": r.get::<Option<String>, _>("completed_at"),
                    "result_ref": r.get::<Option<String>, _>("result_ref"),
                })
            }).collect())
        }
    }
}

// ---------------------------------------------------------------------------
// Abuse status
// ---------------------------------------------------------------------------

pub async fn check_abuse_status(
    db: &Database,
    key: &str,
) -> Result<(bool, i64), sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query("SELECT count, blocked_until FROM abuse_counters WHERE key = ?")
                .bind(key)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?;
            match row {
                Some(r) => {
                    let blocked = r.get::<Option<String>, _>("blocked_until").is_some();
                    let count = r.get::<i64, _>("count");
                    Ok((blocked, count))
                }
                None => Ok((false, 0)),
            }
        }
        Backend::Postgres => {
            let row = sqlx::query("SELECT count, blocked_until FROM abuse_counters WHERE key = $1")
                .bind(key)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?;
            match row {
                Some(r) => Ok((r.get::<Option<String>, _>("blocked_until").is_some(), r.get::<i64, _>("count"))),
                None => Ok((false, 0)),
            }
        }
    }
}
