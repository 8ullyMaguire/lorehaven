//! Taste Vanguard Role (spec §16.18, M18 Phase 4.2).
//!
//! Selection methods:
//! - `resonance_threshold` — top N% by resonance with admin taste profile
//! - `admin_appointment` — manual grant by admin
//! - `contribution_volume` — top curators by bookmark/review volume

use serde_json::Value;
use sqlx::Row;

use crate::{Backend, Database};

fn pool_err() -> sqlx::Error {
    sqlx::Error::PoolClosed
}

// ---------------------------------------------------------------------------
// Vanguard role (spec §16.18)
// ---------------------------------------------------------------------------

/// Grant the Vanguard role to an account. Idempotent.
pub async fn grant_vanguard(
    db: &Database,
    account_id: &str,
    method: &str,
    granted_by: Option<&str>,
    expires_at: Option<&str>,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().ok_or(pool_err())?;
            sqlx::query(
                "INSERT INTO vanguard_roles (account_id, granted_at, method, expires_at, granted_by)
                 VALUES (?, ?, ?, ?, ?)
                 ON CONFLICT(account_id)
                 DO UPDATE SET granted_at = ?, method = ?, expires_at = ?, granted_by = ?",
            )
            .bind(account_id)
            .bind(&now)
            .bind(method)
            .bind(expires_at)
            .bind(granted_by)
            .bind(&now)
            .bind(method)
            .bind(expires_at)
            .bind(granted_by)
            .execute(pool)
            .await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().ok_or(pool_err())?;
            sqlx::query(
                "INSERT INTO vanguard_roles (account_id, granted_at, method, expires_at, granted_by)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT(account_id)
                 DO UPDATE SET granted_at = $2, method = $3, expires_at = $4, granted_by = $5",
            )
            .bind(account_id)
            .bind(&now)
            .bind(method)
            .bind(expires_at)
            .bind(granted_by)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

/// Revoke the Vanguard role from an account.
pub async fn revoke_vanguard(db: &Database, account_id: &str) -> Result<(), sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().ok_or(pool_err())?;
            sqlx::query("DELETE FROM vanguard_roles WHERE account_id = ?")
                .bind(account_id)
                .execute(pool)
                .await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().ok_or(pool_err())?;
            sqlx::query("DELETE FROM vanguard_roles WHERE account_id = $1")
                .bind(account_id)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

/// Check whether an account currently holds the Vanguard role.
pub async fn is_vanguard(db: &Database, account_id: &str) -> Result<bool, sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    let count: i64 = match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().ok_or(pool_err())?;
            sqlx::query_scalar(
                "SELECT COUNT(*) FROM vanguard_roles
                 WHERE account_id = ? AND (expires_at IS NULL OR expires_at > ?)",
            )
            .bind(account_id)
            .bind(&now)
            .fetch_one(pool)
            .await?
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().ok_or(pool_err())?;
            sqlx::query_scalar(
                "SELECT COUNT(*) FROM vanguard_roles
                 WHERE account_id = $1 AND (expires_at IS NULL OR expires_at > $2)",
            )
            .bind(account_id)
            .bind(&now)
            .fetch_one(pool)
            .await?
        }
    };
    Ok(count > 0)
}

/// List all current vanguards (not expired).
pub async fn list_vanguards(db: &Database) -> Result<Vec<Value>, sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().ok_or(pool_err())?;
            let rows = sqlx::query(
                "SELECT account_id, granted_at, method, expires_at, granted_by
                 FROM vanguard_roles
                 WHERE expires_at IS NULL OR expires_at > ?
                 ORDER BY granted_at DESC",
            )
            .bind(&now)
            .fetch_all(pool)
            .await?;
            let mut result = Vec::new();
            for row in rows {
                result.push(serde_json::json!({
                    "account_id": row.get::<String, _>("account_id"),
                    "granted_at": row.get::<String, _>("granted_at"),
                    "method": row.get::<String, _>("method"),
                    "expires_at": row.get::<Option<String>, _>("expires_at"),
                    "granted_by": row.get::<Option<String>, _>("granted_by"),
                }));
            }
            Ok(result)
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().ok_or(pool_err())?;
            let rows = sqlx::query(
                "SELECT account_id, granted_at, method, expires_at, granted_by
                 FROM vanguard_roles
                 WHERE expires_at IS NULL OR expires_at > $1
                 ORDER BY granted_at DESC",
            )
            .bind(&now)
            .fetch_all(pool)
            .await?;
            let mut result = Vec::new();
            for row in rows {
                result.push(serde_json::json!({
                    "account_id": row.get::<String, _>("account_id"),
                    "granted_at": row.get::<String, _>("granted_at"),
                    "method": row.get::<String, _>("method"),
                    "expires_at": row.get::<Option<String>, _>("expires_at"),
                    "granted_by": row.get::<Option<String>, _>("granted_by"),
                }));
            }
            Ok(result)
        }
    }
}

// ---------------------------------------------------------------------------
// Vanguard pins (spec §16.18 — Vanguard Picks shelf)
// ---------------------------------------------------------------------------

/// Pin a work to the Vanguard Pins shelf.
pub async fn pin_work(
    db: &Database,
    account_id: &str,
    work_id: &str,
    pin_reason: &str,
    message: Option<&str>,
) -> Result<String, sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().ok_or(pool_err())?;
            sqlx::query(
                "INSERT INTO vanguard_pins (id, account_id, work_id, pin_reason, message, pinned_at)
                 VALUES (?, ?, ?, ?, ?, ?)
                 ON CONFLICT(account_id, work_id) DO UPDATE SET pinned_at = ?, pin_reason = ?, message = ?",
            )
            .bind(&id)
            .bind(account_id)
            .bind(work_id)
            .bind(pin_reason)
            .bind(message)
            .bind(&now)
            .bind(&now)
            .bind(pin_reason)
            .bind(message)
            .execute(pool)
            .await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().ok_or(pool_err())?;
            sqlx::query(
                "INSERT INTO vanguard_pins (id, account_id, work_id, pin_reason, message, pinned_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT(account_id, work_id) DO UPDATE SET pinned_at = $6, pin_reason = $4, message = $5",
            )
            .bind(&id)
            .bind(account_id)
            .bind(work_id)
            .bind(pin_reason)
            .bind(message)
            .bind(&now)
            .execute(pool)
            .await?;
        }
    }
    Ok(id)
}

/// Unpin a work (soft delete — keeps the row for audit).
pub async fn unpin_work(db: &Database, account_id: &str, work_id: &str) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().ok_or(pool_err())?;
            sqlx::query(
                "UPDATE vanguard_pins SET deleted_at = ? WHERE account_id = ? AND work_id = ?",
            )
            .bind(&now)
            .bind(account_id)
            .bind(work_id)
            .execute(pool)
            .await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().ok_or(pool_err())?;
            sqlx::query("UPDATE vanguard_pins SET deleted_at = $1 WHERE account_id = $2 AND work_id = $3 AND deleted_at IS NULL")
                .bind(&now)
                .bind(account_id)
                .bind(work_id)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

/// List all active (non-expired, non-deleted) vanguard pins.
pub async fn list_active_pins(db: &Database) -> Result<Vec<Value>, sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().ok_or(pool_err())?;
            let rows = sqlx::query(
                "SELECT vp.id, vp.account_id, vp.work_id, vp.pin_reason, vp.message, vp.pinned_at, vp.expires_at
                 FROM vanguard_pins vp
                 JOIN vanguard_roles vr ON vp.account_id = vr.account_id
                 WHERE vp.deleted_at IS NULL
                   AND (vp.expires_at IS NULL OR vp.expires_at > ?)
                   AND (vr.expires_at IS NULL OR vr.expires_at > ?)
                 ORDER BY vp.pinned_at DESC",
            )
            .bind(&now)
            .bind(&now)
            .fetch_all(pool)
            .await?;
            let mut result = Vec::new();
            for row in rows {
                result.push(serde_json::json!({
                    "id": row.get::<String, _>("id"),
                    "account_id": row.get::<String, _>("account_id"),
                    "work_id": row.get::<String, _>("work_id"),
                    "pin_reason": row.get::<String, _>("pin_reason"),
                    "message": row.get::<Option<String>, _>("message"),
                    "pinned_at": row.get::<String, _>("pinned_at"),
                    "expires_at": row.get::<Option<String>, _>("expires_at"),
                }));
            }
            Ok(result)
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().ok_or(pool_err())?;
            let rows = sqlx::query(
                "SELECT vp.id, vp.account_id, vp.work_id, vp.pin_reason, vp.message, vp.pinned_at, vp.expires_at
                 FROM vanguard_pins vp
                 JOIN vanguard_roles vr ON vp.account_id = vr.account_id
                 WHERE vp.deleted_at IS NULL
                   AND (vp.expires_at IS NULL OR vp.expires_at > $1)
                   AND (vr.expires_at IS NULL OR vr.expires_at > $1)
                 ORDER BY vp.pinned_at DESC",
            )
            .bind(&now)
            .fetch_all(pool)
            .await?;
            let mut result = Vec::new();
            for row in rows {
                result.push(serde_json::json!({
                    "id": row.get::<String, _>("id"),
                    "account_id": row.get::<String, _>("account_id"),
                    "work_id": row.get::<String, _>("work_id"),
                    "pin_reason": row.get::<String, _>("pin_reason"),
                    "message": row.get::<Option<String>, _>("message"),
                    "pinned_at": row.get::<String, _>("pinned_at"),
                    "expires_at": row.get::<Option<String>, _>("expires_at"),
                }));
            }
            Ok(result)
        }
    }
}

// ---------------------------------------------------------------------------
// Selection methods (spec §16.18)
// ---------------------------------------------------------------------------

/// Select vanguards by contribution volume (top N curators by bookmark/review
/// count in the last 90 days).
pub async fn select_by_contribution_volume(
    db: &Database,
    limit: i64,
) -> Result<Vec<String>, sqlx::Error> {
    let since = crate::identity::in_seconds(-(90 * 86400));
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().ok_or(pool_err())?;
            let rows = sqlx::query(
                "SELECT pseud_id as account_id, COUNT(*) as cnt FROM (
                    SELECT pseud_id, work_id FROM bookmarks
                    WHERE created_at > ?
                    UNION ALL
                    SELECT pseud_id, work_id FROM reviews
                    WHERE created_at > ?
                 ) GROUP BY pseud_id ORDER BY cnt DESC LIMIT ?",
            )
            .bind(&since)
            .bind(&since)
            .bind(limit)
            .fetch_all(pool)
            .await?;
            Ok(rows
                .iter()
                .map(|r| r.get::<String, _>("account_id"))
                .collect())
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().ok_or(pool_err())?;
            let rows = sqlx::query(
                "SELECT pseud_id as account_id, COUNT(*) as cnt FROM (
                    SELECT pseud_id, work_id FROM bookmarks
                    WHERE created_at > $1
                    UNION ALL
                    SELECT pseud_id, work_id FROM reviews
                    WHERE created_at > $1
                 ) q GROUP BY pseud_id ORDER BY cnt DESC LIMIT $2",
            )
            .bind(&since)
            .bind(limit)
            .fetch_all(pool)
            .await?;
            Ok(rows
                .iter()
                .map(|r| r.get::<String, _>("account_id"))
                .collect())
        }
    }
}

/// Apply the configured selection method and refresh vanguard roles.
pub async fn select_vanguards(
    db: &Database,
    method: &str,
    limit: i64,
) -> Result<Vec<String>, sqlx::Error> {
    match method {
        "contribution_volume" => select_by_contribution_volume(db, limit).await,
        // resonance_threshold and admin_appointment are handled externally.
        _ => Ok(Vec::new()),
    }
}
