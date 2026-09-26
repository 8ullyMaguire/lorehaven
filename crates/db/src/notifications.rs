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

/// The raw row shape both list statements select, in the same order. Kept in
/// one place so the two SELECTs cannot drift from the decoder.
type RawRow = (String, String, String, String, Option<String>, i32, String);

impl From<RawRow> for NotificationRow {
    fn from(r: RawRow) -> Self {
        let (id, kind, title, body, work_id, unread, created_at) = r;
        NotificationRow {
            id,
            kind,
            title,
            body,
            work_id,
            unread: unread != 0,
            created_at,
        }
    }
}

// ---------------------------------------------------------------------------
// Writes
// ---------------------------------------------------------------------------

/// Insert one notification. `work_id` is optional context the UI links to.
///
/// Respects per-event channel routing (spec §46.4): if the account has disabled
/// this event type, the notification is dropped. Otherwise the resolved channel
/// (default `in_app`) is stored on the row.
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

    // Resolve delivery channel; None means the event is disabled for this account.
    let channel = match crate::settings::resolve_notification_channel(
        db,
        uuid::Uuid::parse_str(account_id)
            .map_err(|e| sqlx::Error::Protocol(format!("bad uuid: {e}")))?,
        kind,
    )
    .await
    {
        Ok(Some(ch)) => ch,
        Ok(None) => return Ok(id),      // event disabled — silently drop
        Err(_) => "in_app".to_string(), // routing lookup failed — fall back to default
    };

    let sql = db.sql(
        "INSERT INTO notifications (id, account_id, kind, title, body, work_id, delivery_channel, read_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8)",
        "INSERT INTO notifications (id, account_id, kind, title, body, work_id, delivery_channel, read_at, created_at)
         VALUES ($1, $2::uuid, $3, $4, $5, $6::uuid, $7, NULL, $8)",
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
                .bind(&channel)
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
                .bind(&channel)
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

/// The caller's inbox, newest first. Unfiltered; callers that can see a work
/// should use [`list_filtered`], which applies the reader's content filters.
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
            sqlx::query_as::<_, RawRow>(&sql)
                .bind(account_id)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, RawRow>(&sql)
                .bind(account_id)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows.into_iter().map(NotificationRow::from).collect())
}

/// [`list`], minus the works the reader has content-filtered.
///
/// A notification *with* a `work_id` carries that work's title in `title`, so
/// leaving it unfiltered leaks exactly what the filter exists to withhold. One
/// *without* a work -- `kind = system`, an instance notice -- is not filtered:
/// §46.7.1 is about works not reaching the reader, not about silencing the
/// instance at someone who filtered a tag. The predicate is built over
/// `n.work_id`, so a NULL work_id makes the NOT EXISTS trivially true and the
/// system notice is kept, which is the behaviour we want without a special case.
///
/// The exclusion is in the `WHERE` rather than applied after the `LIMIT`, for
/// the reason it is everywhere else: filtering after the limit returns a short
/// page, which the reader cannot distinguish from the end of the inbox.
pub async fn list_filtered(
    db: &Database,
    account_id: &str,
    limit: i64,
    rules: &[crate::search::content_filter_sql::FilterRule],
) -> Result<Vec<NotificationRow>, sqlx::Error> {
    let excl = crate::search::content_filter_sql::build_for(rules, "n.work_id");
    // An empty predicate must not leave a stray `AND` behind.
    let and = if excl.predicate.is_empty() {
        String::new()
    } else {
        format!(" AND {}", excl.predicate)
    };
    // `sql_owned` renumbers `?` to `$n` for PostgreSQL, so both arms are
    // written once here. The filter's binds are appended in a loop, which is
    // why the placeholders are positional and unnumbered: an explicit `?2`
    // would be renumbered too and the two numbering schemes would collide.
    let sql = crate::sql_owned(
        db,
        format!(
            "SELECT id, kind, title, body, work_id,
                CASE WHEN read_at IS NULL THEN 1 ELSE 0 END AS unread, created_at
           FROM notifications n WHERE n.account_id = ?{and}
          ORDER BY n.created_at DESC, n.id DESC LIMIT ?"
        ),
        format!(
            "SELECT id::text, kind, title, body, work_id::text,
                CASE WHEN read_at IS NULL THEN 1 ELSE 0 END AS unread, created_at
           FROM notifications n WHERE n.account_id = ?::uuid{and}
          ORDER BY n.created_at DESC, n.id DESC LIMIT ?"
        ),
    );
    // The query builder is monomorphized per backend, so each arm builds its
    // own rather than sharing one across the match.
    let rows = match db.backend() {
        Backend::Sqlite => {
            let mut q = sqlx::query_as::<_, RawRow>(&sql).bind(account_id);
            for b in &excl.binds {
                q = q.bind(b);
            }
            q.bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            let mut q = sqlx::query_as::<_, RawRow>(&sql).bind(account_id);
            for b in &excl.binds {
                q = q.bind(b);
            }
            q.bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows.into_iter().map(NotificationRow::from).collect())
}

/// [`unread_count`], counting only entries the reader can actually see.
///
/// The count is filtered for the same reason the list is, and it is the sharper
/// half: a badge that is filtered out of the list but still counted leaks the
/// existence of the very work the filter hides, through a one-digit door. The
/// visible consequence is that the badge no longer matches a row count the
/// reader can see, which is the correct trade -- §46.7.1 is an invariant, and a
/// mismatch is visible while a leak is not.
pub async fn unread_count_filtered(
    db: &Database,
    account_id: &str,
    rules: &[crate::search::content_filter_sql::FilterRule],
) -> Result<i64, sqlx::Error> {
    let excl = crate::search::content_filter_sql::build_for(rules, "n.work_id");
    let and = if excl.predicate.is_empty() {
        String::new()
    } else {
        format!(" AND {}", excl.predicate)
    };
    let sql = crate::sql_owned(
        db,
        format!("SELECT COUNT(*) FROM notifications n WHERE n.account_id = ? AND n.read_at IS NULL{and}"),
        format!("SELECT COUNT(*) FROM notifications n WHERE n.account_id = ?::uuid AND n.read_at IS NULL{and}"),
    );
    match db.backend() {
        Backend::Sqlite => {
            let mut q = sqlx::query_scalar(&sql).bind(account_id);
            for b in &excl.binds {
                q = q.bind(b);
            }
            q.fetch_one(db.sqlite_pool().expect("sqlite")).await
        }
        Backend::Postgres => {
            let mut q = sqlx::query_scalar(&sql).bind(account_id);
            for b in &excl.binds {
                q = q.bind(b);
            }
            q.fetch_one(db.postgres_pool().expect("postgres")).await
        }
    }
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
          WHERE id = $2 AND account_id = $3::uuid AND read_at IS NULL",
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
