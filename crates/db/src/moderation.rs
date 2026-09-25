use crate::{Backend, Database};
use anyhow::Result;
use lorehaven_domain::moderation::SanctionLevel;

/// Apply a sanction (spec §35.5).
pub async fn apply_sanction(
    db: &Database,
    account: &str,
    category_id: Option<&str>,
    level: SanctionLevel,
    reason: &str,
    actor: &str,
    expires_at: Option<&str>,
) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO forum_sanctions (id, account, category_id, level, reason, actor, created_at, expires_at, active)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1)"
            )
            .bind(&id)
            .bind(account)
            .bind(category_id)
            .bind(level.as_str())
            .bind(reason)
            .bind(actor)
            .bind(&now)
            .bind(expires_at)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO forum_sanctions (id, account, category_id, level, reason, actor, created_at, expires_at, active)
                 VALUES ($1, $2::uuid, $3, $4, $5, $6::uuid, $7, $8, 1)"
            )
            .bind(&id)
            .bind(account)
            .bind(category_id)
            .bind(level.as_str())
            .bind(reason)
            .bind(actor)
            .bind(&now)
            .bind(expires_at)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(id)
}

/// Check for an active sanction against an account in a category scope.
pub async fn check_sanction(
    db: &Database,
    account: &str,
    category_id: Option<&str>,
) -> Result<Option<SanctionRow>> {
    let now = crate::identity::now_rfc3339();
    let sql = if category_id.is_some() {
        db.sql(
            "SELECT level, expires_at FROM forum_sanctions
             WHERE account = ? AND active = 1
             AND (category_id = ? OR category_id IS NULL)
             AND (expires_at IS NULL OR expires_at > ?)
             ORDER BY created_at DESC LIMIT 1",
            "SELECT level, expires_at FROM forum_sanctions
             WHERE account = $1::uuid AND active = 1
             AND (category_id = $2 OR category_id IS NULL)
             AND (expires_at IS NULL OR expires_at > $3)
             ORDER BY created_at DESC LIMIT 1",
        )
    } else {
        db.sql(
            "SELECT level, expires_at FROM forum_sanctions
             WHERE account = ? AND active = 1 AND category_id IS NULL
             AND (expires_at IS NULL OR expires_at > ?)
             ORDER BY created_at DESC LIMIT 1",
            "SELECT level, expires_at FROM forum_sanctions
             WHERE account = $1::uuid AND active = 1 AND category_id IS NULL
             AND (expires_at IS NULL OR expires_at > $2)
             ORDER BY created_at DESC LIMIT 1",
        )
    };
    let row = match db.backend() {
        Backend::Sqlite => {
            let mut q = sqlx::query_as::<_, SanctionRow>(&sql).bind(account);
            if let Some(cid) = category_id {
                q = q.bind(cid);
            }
            q.bind(&now)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            let mut q = sqlx::query_as::<_, SanctionRow>(&sql).bind(account);
            if let Some(cid) = category_id {
                q = q.bind(cid);
            }
            q.bind(&now)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SanctionRow {
    pub level: String,
    pub expires_at: Option<String>,
}

/// Set slow mode on a topic (spec §35.5).
pub async fn set_slow_mode(db: &Database, topic_id: &str, seconds: i64) -> Result<()> {
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("UPDATE forum_topics SET slow_mode_seconds = ? WHERE id = ?")
                .bind(seconds)
                .bind(topic_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query("UPDATE forum_topics SET slow_mode_seconds = $1 WHERE id = $2")
                .bind(seconds)
                .bind(topic_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Set federation scope on a topic (spec §35.5).
pub async fn set_federation_scope(db: &Database, topic_id: &str, scope: &str) -> Result<()> {
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("UPDATE forum_topics SET federation_scope = ? WHERE id = ?")
                .bind(scope)
                .bind(topic_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query("UPDATE forum_topics SET federation_scope = $1 WHERE id = $2")
                .bind(scope)
                .bind(topic_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Feature a post (best-of curation, spec §35.5).
pub async fn feature_post(db: &Database, post_id: &str, curator: &str) -> Result<()> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("UPDATE forum_posts SET featured = 1, featured_by = ?, featured_at = ? WHERE id = ?")
                .bind(curator)
                .bind(&now)
                .bind(post_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query("UPDATE forum_posts SET featured = 1, featured_by = $1, featured_at = $2 WHERE id = $3")
                .bind(curator)
                .bind(&now)
                .bind(post_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Record a zero-result search (spec §35.5).
pub async fn record_search_miss(db: &Database, query_hash: &str, query_text: &str) -> Result<()> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO forum_search_misses (id, query_hash, query_text, first_seen_at, last_seen_at, count)
                 VALUES (?, ?, ?, ?, ?, 1)
                 ON CONFLICT(query_hash) DO UPDATE SET last_seen_at = excluded.last_seen_at, count = count + 1"
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(query_hash)
            .bind(query_text)
            .bind(&now)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO forum_search_misses (id, query_hash, query_text, first_seen_at, last_seen_at, count)
                 VALUES ($1, $2, $3, $4, $5, 1)
                 ON CONFLICT(query_hash) DO UPDATE SET last_seen_at = excluded.last_seen_at, count = forum_search_misses.count + 1"
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(query_hash)
            .bind(query_text)
            .bind(&now)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(())
}

/// Record daily activity for a topic (spec §35.5).
pub async fn record_topic_activity(
    db: &Database,
    topic_id: &str,
    activity_date: &str,
) -> Result<()> {
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO forum_topic_activity (topic_id, activity_date, reply_count)
                 VALUES (?, ?, 1)
                 ON CONFLICT(topic_id, activity_date) DO UPDATE SET reply_count = reply_count + 1",
            )
            .bind(topic_id)
            .bind(activity_date)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO forum_topic_activity (topic_id, activity_date, reply_count)
                 VALUES ($1, $2, 1)
                 ON CONFLICT(topic_id, activity_date) DO UPDATE SET reply_count = forum_topic_activity.reply_count + 1"
            )
            .bind(topic_id)
            .bind(activity_date)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(())
}

/// Mark that a post's first-vote notification was sent (spec §35.5).
pub async fn mark_first_vote_notified(
    db: &Database,
    post_id: &str,
    account: &str,
    vote_count: i64,
) -> Result<()> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT OR IGNORE INTO forum_post_first_vote_notified (post_id, account, notified_at, vote_count)
                 VALUES (?, ?, ?, ?)"
            )
            .bind(post_id)
            .bind(account)
            .bind(&now)
            .bind(vote_count)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO forum_post_first_vote_notified (post_id, account, notified_at, vote_count)
                 VALUES ($1, $2::uuid, $3, $4)
                 ON CONFLICT(post_id, account) DO NOTHING"
            )
            .bind(post_id)
            .bind(account)
            .bind(&now)
            .bind(vote_count)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(())
}
