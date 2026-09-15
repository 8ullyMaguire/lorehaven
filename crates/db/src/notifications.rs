//! §5.5 — Notifications inbox repository.
//!
//! Backs the reader-facing inbox (GET /notifications,
//! POST /notifications/read-all, POST /notifications/{id}/read). Rows are
//! written by the surfaces that produce them — forum replies, sales and
//! gifts are the first three writers (M12 scope).

use uuid::Uuid;

use crate::{Backend, Database, Result};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// One inbox entry as the API returns it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct NotificationRow {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub work_id: Option<String>,
    /// `true` while `read_at` is NULL.
    pub unread: bool,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// Writes
// ---------------------------------------------------------------------------

/// Insert one notification. `work_id` is optional context the UI links to.
pub async fn notify(
    db: &Database,
    account_id: &str,
    kind: &str,
    title: &str,
    body: &str,
    work_id: Option<&str>,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO notifications (id, account_id, kind, title, body, work_id, read_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7)",
        "INSERT INTO notifications (id, account_id, kind, title, body, work_id, read_at, created_at)
         VALUES ($1, $2::uuid, $3, $4, $5, $6::uuid, NULL, $7)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(account_id)
                .bind(kind)
                .bind(title)
                .bind(body)
                .bind(work_id)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(account_id)
                .bind(kind)
                .bind(title)
                .bind(body)
                .bind(work_id)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(id)
}

// ---------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------

/// The account's inbox, newest first.
pub async fn list(
    db: &Database,
    account_id: &str,
    limit: i64,
) -> Result<Vec<NotificationRow>, sqlx::Error> {
    let sql = db.sql(
        "SELECT id, kind, title, body, work_id,
                CASE WHEN read_at IS NULL THEN 1 ELSE 0 END AS unread, created_at
           FROM notifications WHERE account_id = ?1
          ORDER BY created_at DESC, id DESC LIMIT ?2",
        "SELECT id::text, kind, title, body, work_id::text,
                CASE WHEN read_at IS NULL THEN 1 ELSE 0 END AS unread, created_at
           FROM notifications WHERE account_id = $1::uuid
          ORDER BY created_at DESC, id DESC LIMIT $2",
    );
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String, String, String, String, Option<String>, i32, String)>(&sql)
                .bind(account_id)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String, String, String, String, Option<String>, i32, String)>(&sql)
                .bind(account_id)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(
            |(id, kind, title, body, work_id, unread, created_at)| NotificationRow {
                id,
                kind,
                title,
                body,
                work_id,
                unread: unread != 0,
                created_at,
            },
        )
        .collect())
}

/// How many entries are still unread.
pub async fn unread_count(db: &Database, account_id: &str) -> Result<i64, sqlx::Error> {
    let sql = db.sql(
        "SELECT COUNT(*) FROM notifications WHERE account_id = ?1 AND read_at IS NULL",
        "SELECT COUNT(*) FROM notifications WHERE account_id = $1::uuid AND read_at IS NULL",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(account_id)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(account_id)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await
        }
    }
}

/// Mark one entry read. Idempotent: an already-read entry returns `false`
/// rather than erroring, so a double tap or a replayed request is harmless.
pub async fn mark_read(
    db: &Database,
    account_id: &str,
    notification_id: &str,
) -> Result<bool, sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "UPDATE notifications SET read_at = ?1
          WHERE id = ?2 AND account_id = ?3 AND read_at IS NULL",
        "UPDATE notifications SET read_at = $1
          WHERE id = $2::uuid AND account_id = $3::uuid AND read_at IS NULL",
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(&now)
            .bind(notification_id)
            .bind(account_id)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(&now)
            .bind(notification_id)
            .bind(account_id)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// Mark everything read. Returns how many entries this call closed.
pub async fn mark_all_read(db: &Database, account_id: &str) -> Result<u64, sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "UPDATE notifications SET read_at = ?1 WHERE account_id = ?2 AND read_at IS NULL",
        "UPDATE notifications SET read_at = $1 WHERE account_id = $2::uuid AND read_at IS NULL",
    );
    match db.backend() {
        Backend::Sqlite => Ok(sqlx::query(&sql)
            .bind(&now)
            .bind(account_id)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected()),
        Backend::Postgres => Ok(sqlx::query(&sql)
            .bind(&now)
            .bind(account_id)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected()),
    }
}
