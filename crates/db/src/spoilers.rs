use crate::{Backend, Database};
use anyhow::Result;
use lorehaven_domain::spoilers::{WarningAction, WarningType};
use std::str::FromStr;

/// Upsert a reader's progress through a work (spec §35.4).
pub async fn upsert_reader_progress(
    db: &Database,
    account: &str,
    work_id: &str,
    last_chapter: i64,
) -> Result<()> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO reader_work_progress (account, work_id, last_chapter, updated_at)
                 VALUES (?, ?, ?, ?)
                 ON CONFLICT(account, work_id) DO UPDATE SET last_chapter = excluded.last_chapter, updated_at = excluded.updated_at"
            )
            .bind(account)
            .bind(work_id)
            .bind(last_chapter)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO reader_work_progress (account, work_id, last_chapter, updated_at)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT(account, work_id) DO UPDATE SET last_chapter = excluded.last_chapter, updated_at = excluded.updated_at"
            )
            .bind(account)
            .bind(work_id)
            .bind(last_chapter)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(())
}

/// Get a reader's progress through a work, if any.
pub async fn get_reader_progress(
    db: &Database,
    account: &str,
    work_id: &str,
) -> Result<Option<i64>> {
    let sql = db.sql(
        "SELECT last_chapter FROM reader_work_progress WHERE account = ? AND work_id = ?",
        "SELECT last_chapter FROM reader_work_progress WHERE account = $1 AND work_id = $2",
    );
    let row = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (i64,)>(&sql)
                .bind(account)
                .bind(work_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (i64,)>(&sql)
                .bind(account)
                .bind(work_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.map(|r| r.0))
}

/// Set a topic's spoiler scope (spec §35.4).
pub async fn set_topic_spoiler_scope(
    db: &Database,
    topic_id: &str,
    chapter: Option<i64>,
) -> Result<()> {
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("UPDATE forum_topics SET spoiler_scope_chapter = ? WHERE id = ?")
                .bind(chapter)
                .bind(topic_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query("UPDATE forum_topics SET spoiler_scope_chapter = $1 WHERE id = $2")
                .bind(chapter)
                .bind(topic_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Add a content warning to a post (spec §35.4).
pub async fn add_content_warning(
    db: &Database,
    post_id: &str,
    warning_type: WarningType,
    severity: i64,
    custom_text: Option<&str>,
) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO content_warnings (id, post_id, warning_type, severity, custom_text, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)"
            )
            .bind(&id)
            .bind(post_id)
            .bind(warning_type.as_str())
            .bind(severity)
            .bind(custom_text)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO content_warnings (id, post_id, warning_type, severity, custom_text, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6)"
            )
            .bind(&id)
            .bind(post_id)
            .bind(warning_type.as_str())
            .bind(severity)
            .bind(custom_text)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(id)
}

/// List content warnings for a post.
pub async fn list_content_warnings(db: &Database, post_id: &str) -> Result<Vec<ContentWarningRow>> {
    let sql = db.sql(
        "SELECT warning_type, severity, custom_text FROM content_warnings WHERE post_id = ?",
        "SELECT warning_type, severity, custom_text FROM content_warnings WHERE post_id = $1",
    );
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, ContentWarningRow>(&sql)
                .bind(post_id)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, ContentWarningRow>(&sql)
                .bind(post_id)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ContentWarningRow {
    pub warning_type: String,
    pub severity: i64,
    pub custom_text: Option<String>,
}

/// Upsert a post draft (autosave, spec §35.4).
pub async fn upsert_draft(db: &Database, account: &str, topic_id: &str, body: &str) -> Result<()> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO post_drafts (id, account, topic_id, body, updated_at)
                 VALUES (?, ?, ?, ?, ?)
                 ON CONFLICT(account, topic_id) DO UPDATE SET body = excluded.body, updated_at = excluded.updated_at"
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(account)
            .bind(topic_id)
            .bind(body)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO post_drafts (id, account, topic_id, body, updated_at)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT(account, topic_id) DO UPDATE SET body = excluded.body, updated_at = excluded.updated_at"
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(account)
            .bind(topic_id)
            .bind(body)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(())
}

/// Get a draft for an account+topic, if any.
pub async fn get_draft(db: &Database, account: &str, topic_id: &str) -> Result<Option<String>> {
    let sql = db.sql(
        "SELECT body FROM post_drafts WHERE account = ? AND topic_id = ?",
        "SELECT body FROM post_drafts WHERE account = $1 AND topic_id = $2",
    );
    let row = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String,)>(&sql)
                .bind(account)
                .bind(topic_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String,)>(&sql)
                .bind(account)
                .bind(topic_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.map(|r| r.0))
}

/// Delete a draft (after successful post or explicit discard).
pub async fn delete_draft(db: &Database, account: &str, topic_id: &str) -> Result<bool> {
    let sql = db.sql(
        "DELETE FROM post_drafts WHERE account = ? AND topic_id = ?",
        "DELETE FROM post_drafts WHERE account = $1 AND topic_id = $2",
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(account)
            .bind(topic_id)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(account)
            .bind(topic_id)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// Set a reader's warning pref (spec §35.4).
pub async fn set_warning_pref(
    db: &Database,
    account: &str,
    warning_type: WarningType,
    action: WarningAction,
) -> Result<()> {
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO reader_warning_prefs (account, warning_type, action)
                 VALUES (?, ?, ?)
                 ON CONFLICT(account, warning_type) DO UPDATE SET action = excluded.action",
            )
            .bind(account)
            .bind(warning_type.as_str())
            .bind(action.as_str())
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO reader_warning_prefs (account, warning_type, action)
                 VALUES ($1, $2, $3)
                 ON CONFLICT(account, warning_type) DO UPDATE SET action = excluded.action",
            )
            .bind(account)
            .bind(warning_type.as_str())
            .bind(action.as_str())
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(())
}

/// Get a reader's warning prefs.
pub async fn list_warning_prefs(
    db: &Database,
    account: &str,
) -> Result<Vec<(WarningType, WarningAction)>> {
    let sql = db.sql(
        "SELECT warning_type, action FROM reader_warning_prefs WHERE account = ?",
        "SELECT warning_type, action FROM reader_warning_prefs WHERE account = $1",
    );
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String, String)>(&sql)
                .bind(account)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String, String)>(&sql)
                .bind(account)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows
        .into_iter()
        .filter_map(|(t, a)| {
            Some((
                WarningType::from_str(&t).ok()?,
                WarningAction::from_str(&a).ok()?,
            ))
        })
        .collect())
}

/// Schedule a post for future publication (spec §35.4).
pub async fn schedule_post(db: &Database, post_id: &str, scheduled_at: &str) -> Result<()> {
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("UPDATE forum_posts SET scheduled_at = ?, published = 0 WHERE id = ?")
                .bind(scheduled_at)
                .bind(post_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query("UPDATE forum_posts SET scheduled_at = $1, published = 0 WHERE id = $2")
                .bind(scheduled_at)
                .bind(post_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// List due scheduled posts (spec §35.4). Returns post IDs ready to publish.
pub async fn list_due_scheduled_posts(db: &Database, now: &str, limit: i64) -> Result<Vec<String>> {
    let sql = db.sql(
        "SELECT id FROM forum_posts WHERE published = 0 AND scheduled_at IS NOT NULL AND scheduled_at <= ? ORDER BY scheduled_at LIMIT ?",
        "SELECT id FROM forum_posts WHERE published = 0 AND scheduled_at IS NOT NULL AND scheduled_at <= $1 ORDER BY scheduled_at LIMIT $2",
    );
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String,)>(&sql)
                .bind(now)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String,)>(&sql)
                .bind(now)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows.into_iter().map(|r| r.0).collect())
}

/// Publish a scheduled post (mark as published).
pub async fn publish_scheduled_post(db: &Database, post_id: &str) -> Result<bool> {
    let sql = db.sql(
        "UPDATE forum_posts SET published = 1, scheduled_at = NULL WHERE id = ? AND published = 0",
        "UPDATE forum_posts SET published = 1, scheduled_at = NULL WHERE id = $1 AND published = 0",
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(post_id)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(post_id)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}
