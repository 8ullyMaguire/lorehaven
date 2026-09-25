/// A linked work: just enough for the backlink card on a topic page.
use anyhow::Result;
use serde::Serialize;

use crate::{Backend, Database};

#[derive(Debug, Serialize)]
pub struct LinkedWork {
    pub id: String,
    pub title: String,
    pub author_handles: Vec<String>,
}

/// Find the work linked to a topic. Used for the backlink card on the topic
/// page. Returns `None` when no work is linked or the work is gone.
pub async fn work_for_topic(db: &Database, topic_id: &str) -> Result<Option<LinkedWork>> {
    // Step 1: find the work id behind this topic.
    let link_sql = db.sql(
        "SELECT work_id FROM topic_work_links WHERE topic_id = ?",
        "SELECT work_id::text FROM topic_work_links WHERE topic_id::text = $1",
    );
    let link_row: Option<(String,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&link_sql)
                .bind(topic_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&link_sql)
                .bind(topic_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    let work_id = match link_row {
        Some((id,)) => id,
        None => return Ok(None),
    };

    // Step 2: read the work's title (deleted works don't surface).
    let title_sql = db.sql(
        "SELECT title FROM works WHERE id = ? AND deleted_at IS NULL",
        "SELECT title FROM works WHERE id::text = $1 AND deleted_at IS NULL",
    );
    let title: Option<String> = match db.backend() {
        Backend::Sqlite => sqlx::query_scalar(&title_sql)
            .bind(&work_id)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?
            .flatten(),
        Backend::Postgres => sqlx::query_scalar(&title_sql)
            .bind(&work_id)
            .fetch_optional(db.postgres_pool().expect("postgres"))
            .await?
            .flatten(),
    };
    let Some(title) = title else {
        return Ok(None);
    };

    // Step 3: credited authors, in their display order.
    let author_sql = db.sql(
        "SELECT pe.handle FROM work_contributors wc
             JOIN pseuds pe ON wc.pseud_id = pe.id
          WHERE wc.work_id = ? AND wc.public_attribution = 1
          ORDER BY wc.created_at",
        "SELECT pe.handle FROM work_contributors wc
             -- `public_attribution` is INTEGER: this schema's PostgreSQL
             -- migrations mirror the SQLite types, so the flag takes 1 and
             -- `= true` is `operator does not exist: bigint = boolean`.
             JOIN pseuds pe ON wc.pseud_id = pe.id::uuid
          WHERE wc.work_id::text = $1 AND wc.public_attribution = 1
          ORDER BY wc.created_at",
    );
    let handles: Vec<String> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&author_sql)
                .bind(&work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&author_sql)
                .bind(&work_id)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };

    Ok(Some(LinkedWork {
        id: work_id,
        title,
        author_handles: handles,
    }))
}
