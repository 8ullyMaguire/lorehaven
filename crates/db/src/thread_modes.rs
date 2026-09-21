use crate::{Backend, Database};
use anyhow::Result;

/// Create a forum topic with a thread mode (spec §35.3). The mode is data; an
/// existing topic keeps working unchanged with the default 'plain' mode.
pub async fn create_topic(
    db: &Database,
    id: &str,
    category_id: &str,
    author_pseud: &str,
    title: &str,
    mode: &str,
) -> Result<()> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO forum_topics (id, category_id, author_pseud, title, created_at, last_post_at, locked, mode)
                 VALUES (?, ?, ?, ?, ?, ?, 0, ?)",
            )
            .bind(id)
            .bind(category_id)
            .bind(author_pseud)
            .bind(title)
            .bind(&now)
            .bind(&now)
            .bind(mode)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO forum_topics (id, category_id, author_pseud, title, created_at, last_post_at, locked, mode)
                 VALUES ($1::uuid, $2::uuid, $3, $4, $5, $5, FALSE, $6)",
            )
            .bind(id)
            .bind(category_id)
            .bind(author_pseud)
            .bind(title)
            .bind(&now)
            .bind(mode)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
    }
    Ok(())
}

/// Add a reading-group schedule section (spec §35.3).
pub async fn add_schedule_section(
    db: &Database,
    topic_id: &str,
    position: i64,
    title: &str,
    chapter_start: i64,
    chapter_end: i64,
    unlocks_at: &str,
) -> Result<()> {
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO topic_schedules (topic_id, position, title, chapter_start, chapter_end, unlocks_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(topic_id)
            .bind(position)
            .bind(title)
            .bind(chapter_start)
            .bind(chapter_end)
            .bind(unlocks_at)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO topic_schedules (topic_id, position, title, chapter_start, chapter_end, unlocks_at)
                 VALUES ($1::uuid, $2, $3, $4, $5, $6)",
            )
            .bind(topic_id)
            .bind(position)
            .bind(title)
            .bind(chapter_start)
            .bind(chapter_end)
            .bind(unlocks_at)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
    }
    Ok(())
}

/// Get a topic's schedule sections, ordered by position (spec §35.3).
pub async fn get_schedule(
    db: &Database,
    topic_id: &str,
) -> Result<Vec<serde_json::Value>> {
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (i64, String, i64, i64, String)>(
                "SELECT position, title, chapter_start, chapter_end, unlocks_at
                 FROM topic_schedules WHERE topic_id = ? ORDER BY position",
            )
            .bind(topic_id)
            .fetch_all(db.sqlite_pool().expect("sqlite"))
            .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (i64, String, i64, i64, String)>(
                "SELECT position, title, chapter_start, chapter_end, unlocks_at
                 FROM topic_schedules WHERE topic_id = $1 ORDER BY position",
            )
            .bind(topic_id)
            .fetch_all(db.postgres_pool().expect("postgres"))
            .await?
        }
    };
    let mut result = Vec::new();
    for (pos, title, start, end, unlocks) in rows {
        result.push(serde_json::json!({
            "position": pos,
            "title": title,
            "chapter_start": start,
            "chapter_end": end,
            "unlocks_at": unlocks,
        }));
    }
    Ok(result)
}

/// Create a wiki pin for a topic (spec §35.3).
pub async fn create_wiki_pin(
    db: &Database,
    topic_id: &str,
    post_id: &str,
    body: &str,
    edited_by: &str,
) -> Result<()> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO topic_wiki_pins (topic_id, post_id, body, revision, edited_by, edited_at)
                 VALUES (?, ?, ?, 0, ?, ?)",
            )
            .bind(topic_id)
            .bind(post_id)
            .bind(body)
            .bind(edited_by)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO topic_wiki_pins (topic_id, post_id, body, revision, edited_by, edited_at)
                 VALUES ($1::uuid, $2::uuid, $3, 0, $4, $5)",
            )
            .bind(topic_id)
            .bind(post_id)
            .bind(body)
            .bind(edited_by)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
    }
    Ok(())
}

/// Approve a wiki pin edit (spec §35.3).
pub async fn approve_wiki_pin(
    db: &Database,
    topic_id: &str,
    post_id: &str,
    approved_by: &str,
) -> Result<()> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "UPDATE topic_wiki_pins SET approved_by = ?, approved_at = ?, revision = revision + 1
                 WHERE topic_id = ? AND post_id = ?",
            )
            .bind(approved_by)
            .bind(&now)
            .bind(topic_id)
            .bind(post_id)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "UPDATE topic_wiki_pins SET approved_by = $1, approved_at = $2, revision = revision + 1
                 WHERE topic_id = $3 AND post_id = $4",
            )
            .bind(approved_by)
            .bind(&now)
            .bind(topic_id)
            .bind(post_id)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
    }
    Ok(())
}

/// Get the approved wiki pin for a topic (spec §35.3).
pub async fn get_wiki_pin(
    db: &Database,
    topic_id: &str,
) -> Result<Option<serde_json::Value>> {
    let row = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String, i64, String, String)>(
                "SELECT body, revision, edited_by, edited_at FROM topic_wiki_pins
                 WHERE topic_id = ? AND approved_by IS NOT NULL",
            )
            .bind(topic_id)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String, i64, String, String)>(
                "SELECT body, revision, edited_by, edited_at FROM topic_wiki_pins
                 WHERE topic_id = $1 AND approved_by IS NOT NULL",
            )
            .bind(topic_id)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?
        }
    };
    Ok(row.map(|(body, rev, by, at)| {
        serde_json::json!({
            "body": body,
            "revision": rev,
            "edited_by": by,
            "edited_at": at,
        })
    }))
}

/// Join a critique circle (spec §35.3). Returns the assigned turn position.
pub async fn join_critique(
    db: &Database,
    topic_id: &str,
    pseud: &str,
) -> Result<i64> {
    let _now = crate::identity::now_rfc3339();
    // Get current max position
    let max_pos: Option<i64> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(
                "SELECT MAX(position) FROM critique_queue WHERE topic_id = ?",
            )
            .bind(topic_id)
            .fetch_one(db.sqlite_pool().expect("sqlite"))
            .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(
                "SELECT MAX(position) FROM critique_queue WHERE topic_id = $1",
            )
            .bind(topic_id)
            .fetch_one(db.sqlite_pool().expect("sqlite"))
            .await?
        }
    };
    let position = max_pos.unwrap_or(-1) + 1;
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO critique_queue (topic_id, pseud, position, posted_at)
                 VALUES (?, ?, ?, NULL)",
            )
            .bind(topic_id)
            .bind(pseud)
            .bind(position)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO critique_queue (topic_id, pseud, position, posted_at)
                 VALUES ($1::uuid, $2, $3, NULL)",
            )
            .bind(topic_id)
            .bind(pseud)
            .bind(position)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
    }
    Ok(position)
}

/// Get the current turn queue for a critique circle (spec §35.3).
pub async fn get_critique_queue(
    db: &Database,
    topic_id: &str,
) -> Result<Vec<serde_json::Value>> {
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String, i64, Option<String>)>(
                "SELECT pseud, position, excerpt FROM critique_queue
                 WHERE topic_id = ? ORDER BY position",
            )
            .bind(topic_id)
            .fetch_all(db.sqlite_pool().expect("sqlite"))
            .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String, i64, Option<String>)>(
                "SELECT pseud, position, excerpt FROM critique_queue
                 WHERE topic_id = $1 ORDER BY position",
            )
            .bind(topic_id)
            .fetch_all(db.sqlite_pool().expect("sqlite"))
            .await?
        }
    };
    let mut result = Vec::new();
    for (pseud, pos, excerpt) in rows {
        result.push(serde_json::json!({
            "pseud": pseud,
            "position": pos,
            "excerpt": excerpt,
        }));
    }
    Ok(result)
}

/// Set a topic's thread mode.
pub async fn set_topic_mode(
    db: &Database,
    topic_id: &str,
    mode: &str,
) -> Result<()> {
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("UPDATE forum_topics SET mode = ? WHERE id = ?")
                .bind(mode)
                .bind(topic_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query("UPDATE forum_topics SET mode = $1 WHERE id = $2")
                .bind(mode)
                .bind(topic_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}
