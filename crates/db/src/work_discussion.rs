//! Work discussion storage: modes, linked topics, reactions, and the
//! comment-to-topic migration tool (spec §35.1, repo M31).

use anyhow::Result;
use lorehaven_domain::work_discussion::WorkDiscussionMode;
use sqlx::FromRow;

use crate::identity::now_rfc3339;
use crate::{Backend, Database};

/// A work's discussion mode, read from storage.
pub async fn work_discussion_mode(
    db: &Database,
    work_id: &str,
) -> Result<Option<WorkDiscussionMode>> {
    let sql = db.sql(
        "SELECT discussion_mode FROM works WHERE id = ? AND deleted_at IS NULL",
        "SELECT discussion_mode FROM works WHERE id::text = $1 AND deleted_at IS NULL",
    );
    let value: Option<String> = match db.backend() {
        Backend::Sqlite => sqlx::query_scalar(&sql)
            .bind(work_id)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?
            .flatten(),
        Backend::Postgres => sqlx::query_scalar(&sql)
            .bind(work_id)
            .fetch_optional(db.postgres_pool().expect("postgres"))
            .await?
            .flatten(),
    };
    Ok(value.and_then(|v| WorkDiscussionMode::parse(&v)))
}

/// Set a work's discussion mode. Returns false when the work is absent.
pub async fn set_work_discussion_mode(
    db: &Database,
    work_id: &str,
    mode: WorkDiscussionMode,
) -> Result<bool> {
    let sql = db.sql(
        "UPDATE works SET discussion_mode = ?, updated_at = ? WHERE id = ? AND deleted_at IS NULL",
        "UPDATE works SET discussion_mode = $1, updated_at = $2::timestamptz WHERE id::text = $3 AND deleted_at IS NULL",
    );
    let now = now_rfc3339();
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(mode.as_str())
            .bind(&now)
            .bind(work_id)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(mode.as_str())
            .bind(&now)
            .bind(work_id)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// A linked topic: the forum thread a work's discussion lives in.
#[derive(Debug, Clone, FromRow)]
pub struct TopicWorkLink {
    pub id: String,
    pub topic_id: String,
    pub work_id: String,
    pub chapter_id: Option<String>,
    pub created_at: String,
}

/// The linked topic for a work, if one exists.
pub async fn linked_topic(db: &Database, work_id: &str) -> Result<Option<TopicWorkLink>> {
    let sql = db.sql(
        "SELECT id, topic_id, work_id, chapter_id, created_at FROM topic_work_links WHERE work_id = ? ORDER BY created_at LIMIT 1",
        "SELECT id, topic_id, work_id, chapter_id::text AS chapter_id, created_at FROM topic_work_links WHERE work_id::text = $1 ORDER BY created_at LIMIT 1",
    );
    let row = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, TopicWorkLink>(&sql)
                .bind(work_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, TopicWorkLink>(&sql)
                .bind(work_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row)
}

/// Link a topic to a work. Idempotent per work: if a link already exists it
/// is returned unchanged, so a chapter-publish hook that fires twice creates
/// exactly one topic.
pub async fn link_topic(
    db: &Database,
    work_id: &str,
    chapter_id: Option<&str>,
    category_id: &str,
    author_pseud: &str,
    title: &str,
) -> Result<TopicWorkLink> {
    if let Some(existing) = linked_topic(db, work_id).await? {
        return Ok(existing);
    }
    let topic_id = crate::community::create_topic(db, category_id, author_pseud, title).await?;
    let id = uuid::Uuid::new_v4().to_string();
    let now = now_rfc3339();
    let sql = db.sql(
        "INSERT INTO topic_work_links (id, topic_id, work_id, chapter_id, created_at) VALUES (?, ?, ?, ?, ?)",
        "INSERT INTO topic_work_links (id, topic_id, work_id, chapter_id, created_at) VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid, $5)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(&topic_id)
                .bind(work_id)
                .bind(chapter_id)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(&topic_id)
                .bind(work_id)
                .bind(chapter_id)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(TopicWorkLink {
        id,
        topic_id,
        work_id: work_id.to_owned(),
        chapter_id: chapter_id.map(str::to_owned),
        created_at: now,
    })
}

/// One aggregate reaction count for a work.
#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct ReactionCount {
    pub vote_type: String,
    pub count: i64,
}

/// Aggregate reaction counts for a work, public form (spec §35.1).
pub async fn reaction_counts(db: &Database, work_id: &str) -> Result<Vec<ReactionCount>> {
    let sql = db.sql(
        "SELECT vote_type, COUNT(*) AS count FROM work_reactions WHERE work_id = ? GROUP BY vote_type ORDER BY count DESC, vote_type",
        "SELECT vote_type, COUNT(*) AS count FROM work_reactions WHERE work_id::text = $1 GROUP BY vote_type ORDER BY count DESC, vote_type",
    );
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, ReactionCount>(&sql)
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, ReactionCount>(&sql)
                .bind(work_id)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows)
}

/// One pseud's reaction on a work.
pub async fn reaction_by(db: &Database, work_id: &str, pseud: &str) -> Result<Option<String>> {
    let sql = db.sql(
        "SELECT vote_type FROM work_reactions WHERE work_id = ? AND pseud = ?",
        "SELECT vote_type FROM work_reactions WHERE work_id::text = $1 AND pseud::text = $2",
    );
    let row: Option<String> = match db.backend() {
        Backend::Sqlite => sqlx::query_scalar(&sql)
            .bind(work_id)
            .bind(pseud)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?
            .flatten(),
        Backend::Postgres => sqlx::query_scalar(&sql)
            .bind(work_id)
            .bind(pseud)
            .fetch_optional(db.postgres_pool().expect("postgres"))
            .await?
            .flatten(),
    };
    Ok(row)
}

/// Cast, change, or retract a reaction. One row per (work, pseud); a
/// `None` vote type retracts. Returns the outcome for the caller to report.
pub async fn set_reaction(
    db: &Database,
    work_id: &str,
    pseud: &str,
    vote_type: Option<&str>,
) -> Result<lorehaven_domain::work_discussion::ReactionOutcome> {
    use lorehaven_domain::work_discussion::ReactionOutcome;

    let existing = reaction_by(db, work_id, pseud).await?;
    let now = now_rfc3339();

    match (existing, vote_type) {
        (None, None) => Ok(ReactionOutcome::Retracted),
        (None, Some(vt)) => {
            let sql = db.sql(
                "INSERT INTO work_reactions (work_id, pseud, vote_type, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
                "INSERT INTO work_reactions (work_id, pseud, vote_type, created_at, updated_at) VALUES ($1::uuid, $2::uuid, $3, $4, $5)",
            );
            match db.backend() {
                Backend::Sqlite => {
                    sqlx::query(&sql)
                        .bind(work_id)
                        .bind(pseud)
                        .bind(vt)
                        .bind(&now)
                        .bind(&now)
                        .execute(db.sqlite_pool().expect("sqlite"))
                        .await?;
                }
                Backend::Postgres => {
                    sqlx::query(&sql)
                        .bind(work_id)
                        .bind(pseud)
                        .bind(vt)
                        .bind(&now)
                        .bind(&now)
                        .execute(db.postgres_pool().expect("postgres"))
                        .await?;
                }
            };
            Ok(ReactionOutcome::Cast)
        }
        (Some(_), None) => {
            let sql = db.sql(
                "DELETE FROM work_reactions WHERE work_id = ? AND pseud = ?",
                "DELETE FROM work_reactions WHERE work_id::text = $1 AND pseud::text = $2",
            );
            match db.backend() {
                Backend::Sqlite => {
                    sqlx::query(&sql)
                        .bind(work_id)
                        .bind(pseud)
                        .execute(db.sqlite_pool().expect("sqlite"))
                        .await?;
                }
                Backend::Postgres => {
                    sqlx::query(&sql)
                        .bind(work_id)
                        .bind(pseud)
                        .execute(db.postgres_pool().expect("postgres"))
                        .await?;
                }
            };
            Ok(ReactionOutcome::Retracted)
        }
        (Some(prev), Some(vt)) if prev == vt => Ok(ReactionOutcome::Retracted),
        (Some(_), Some(vt)) => {
            let sql = db.sql(
                "UPDATE work_reactions SET vote_type = ?, updated_at = ? WHERE work_id = ? AND pseud = ?",
                "UPDATE work_reactions SET vote_type = $1, updated_at = $2::timestamptz WHERE work_id::text = $3 AND pseud::text = $4",
            );
            match db.backend() {
                Backend::Sqlite => {
                    sqlx::query(&sql)
                        .bind(vt)
                        .bind(&now)
                        .bind(work_id)
                        .bind(pseud)
                        .execute(db.sqlite_pool().expect("sqlite"))
                        .await?;
                }
                Backend::Postgres => {
                    sqlx::query(&sql)
                        .bind(vt)
                        .bind(&now)
                        .bind(work_id)
                        .bind(pseud)
                        .execute(db.postgres_pool().expect("postgres"))
                        .await?;
                }
            };
            Ok(ReactionOutcome::Changed)
        }
    }
}

/// A comment row in its stored form, for the migration tool.
#[derive(Debug, Clone, FromRow)]
struct CommentMigrationRow {
    id: String,
    author_pseud: String,
    body: String,
    created_at: String,
}

/// The result of a comment-to-topic migration.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MigrationReport {
    /// The topic comments were moved into.
    pub topic_id: String,
    /// How many comments became posts.
    pub moved: usize,
}

/// Convert a work's comment thread into forum posts on its linked topic.
///
/// Every comment becomes a post in original order, authorship and timestamps
/// preserved; the comments are then soft-deleted with a tombstone body that
/// points at the topic. Idempotent: comments already migrated (soft-deleted
/// with the tombstone marker) are skipped, so a second run moves zero rows.
pub async fn migrate_comments_to_topic(
    db: &Database,
    work_id: &str,
    category_id: &str,
    author_pseud: &str,
    title: &str,
) -> Result<MigrationReport> {
    const TOMBSTONE: &str = "[moved to the work's discussion thread]";

    let link = link_topic(db, work_id, None, category_id, author_pseud, title).await?;

    let select = db.sql(
        "SELECT id, author_pseud, body, created_at FROM comments WHERE subject_type = 'work' AND subject_id = ? AND deleted_at IS NULL ORDER BY created_at",
        "SELECT id, author_pseud, body, created_at FROM comments WHERE subject_type = 'work' AND subject_id::text = $1 AND deleted_at IS NULL ORDER BY created_at",
    );
    let comments: Vec<CommentMigrationRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, CommentMigrationRow>(&select)
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, CommentMigrationRow>(&select)
                .bind(work_id)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };

    let mut moved = 0usize;
    for c in &comments {
        // The post keeps the comment's author and timestamp; the tombstone
        // body records where the text went.
        crate::community::create_post_with_timestamp(
            db,
            &link.topic_id,
            &c.author_pseud,
            &c.body,
            &c.created_at,
        )
        .await?;
        let delete = db.sql(
            "UPDATE comments SET deleted_at = ?, body = ? WHERE id = ?",
            "UPDATE comments SET deleted_at = $1::timestamptz, body = $2 WHERE id = $3",
        );
        let now = now_rfc3339();
        match db.backend() {
            Backend::Sqlite => {
                sqlx::query(&delete)
                    .bind(&now)
                    .bind(TOMBSTONE)
                    .bind(&c.id)
                    .execute(db.sqlite_pool().expect("sqlite"))
                    .await?;
            }
            Backend::Postgres => {
                sqlx::query(&delete)
                    .bind(&now)
                    .bind(TOMBSTONE)
                    .bind(&c.id)
                    .execute(db.postgres_pool().expect("postgres"))
                    .await?;
            }
        };
        moved += 1;
    }

    Ok(MigrationReport {
        topic_id: link.topic_id,
        moved,
    })
}
