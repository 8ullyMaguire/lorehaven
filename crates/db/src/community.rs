//! Community repository: comments, forums, groups, messaging, blocking.
//!
//! Spec §17. Both dialects.

use crate::{Backend, Database};
use anyhow::Result;
use lorehaven_domain::blocking::BlockScope;
use serde::Serialize;

/// A comment row.
#[derive(Debug, Clone, Serialize)]
pub struct Comment {
    pub id: String,
    pub subject_type: String,
    pub subject_id: String,
    pub author_pseud: String,
    pub body: String,
    pub created_at: String,
    pub edited_at: Option<String>,
    pub deleted_at: Option<String>,
}

/// Insert a comment. The positivity classification is done by the caller
/// (route layer) before this insert, and the classification_id is stored
/// on the comment row.
pub async fn insert_comment(
    db: &Database,
    subject_type: &str,
    subject_id: &str,
    author_pseud: &str,
    body: &str,
    classification_id: Option<&str>,
) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO comments (id, subject_type, subject_id, author_pseud, body, body_version, classification_id, created_at, edited_at, deleted_at)
         VALUES (?, ?, ?, ?, ?, 'v1', ?, ?, NULL, NULL)",
        "INSERT INTO comments (id, subject_type, subject_id, author_pseud, body, body_version, classification_id, created_at, edited_at, deleted_at)
         VALUES (?::uuid, ?, ?, ?::uuid, ?, 'v1', ?, ?::uuid, ?, NULL, NULL)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(subject_type)
                .bind(subject_id)
                .bind(author_pseud)
                .bind(body)
                .bind(classification_id)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(subject_type)
                .bind(subject_id)
                .bind(author_pseud)
                .bind(body)
                .bind(classification_id)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(id)
}

/// Soft-delete a comment (author or subject owner).
pub async fn soft_delete_comment(
    db: &Database,
    comment_id: &str,
    actor_pseud: &str,
) -> Result<bool> {
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "UPDATE comments SET deleted_at = ?, body = '[deleted]' WHERE id = ? AND author_pseud = ? AND deleted_at IS NULL",
        "UPDATE comments SET deleted_at = ?, body = '[deleted]' WHERE id = $1::uuid AND author_pseud = $2::uuid AND deleted_at IS NULL",
    );
    let rows = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(&now)
            .bind(comment_id)
            .bind(actor_pseud)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(&now)
            .bind(comment_id)
            .bind(actor_pseud)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    Ok(rows > 0)
}

/// Check if a block exists between two accounts.
pub async fn is_blocked(
    db: &Database,
    blocker: &str,
    blocked: &str,
    scope: BlockScope,
) -> Result<bool> {
    let sql = db.sql(
        "SELECT 1 FROM blocks WHERE blocker = ? AND blocked = ? AND (scope = 'all' OR scope = ?)",
        "SELECT 1 FROM blocks WHERE blocker = $1::uuid AND blocked = $2::uuid AND (scope = 'all' OR scope = $3)",
    );
    let row: Option<(i64,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(blocker)
                .bind(blocked)
                .bind(scope.as_str())
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(blocker)
                .bind(blocked)
                .bind(scope.as_str())
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.is_some())
}

/// Insert a block.
pub async fn insert_block(
    db: &Database,
    blocker: &str,
    blocked: &str,
    scope: BlockScope,
    note: Option<&str>,
) -> Result<()> {
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO blocks (blocker, blocked, scope, created_at, note) VALUES (?, ?, ?, ?, ?)",
        "INSERT INTO blocks (blocker, blocked, scope, created_at, note) VALUES ($1::uuid, $2::uuid, $3, $4, $5)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(blocker)
                .bind(blocked)
                .bind(scope.as_str())
                .bind(&now)
                .bind(note)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(blocker)
                .bind(blocked)
                .bind(scope.as_str())
                .bind(&now)
                .bind(note)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Delete a block.
pub async fn delete_block(db: &Database, blocker: &str, blocked: &str) -> Result<bool> {
    let sql = db.sql(
        "DELETE FROM blocks WHERE blocker = ? AND blocked = ?",
        "DELETE FROM blocks WHERE blocker = $1::uuid AND blocked = $2::uuid",
    );
    let rows = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(blocker)
            .bind(blocked)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(blocker)
            .bind(blocked)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    Ok(rows > 0)
}

/// Insert a mute.
pub async fn insert_mute(
    db: &Database,
    muter: &str,
    muted: &str,
    until: Option<&str>,
) -> Result<()> {
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO mutes (muter, muted, until, created_at) VALUES (?, ?, ?, ?)",
        "INSERT INTO mutes (muter, muted, until, created_at) VALUES ($1::uuid, $2::uuid, $3, $4)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(muter)
                .bind(muted)
                .bind(until)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(muter)
                .bind(muted)
                .bind(until)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Delete a mute.
pub async fn delete_mute(db: &Database, muter: &str, muted: &str) -> Result<bool> {
    let sql = db.sql(
        "DELETE FROM mutes WHERE muter = ? AND muted = ?",
        "DELETE FROM mutes WHERE muter = $1::uuid AND muted = $2::uuid",
    );
    let rows = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(muter)
            .bind(muted)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(muter)
            .bind(muted)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    Ok(rows > 0)
}

/// Create a conversation.
pub async fn create_conversation(db: &Database) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO conversations (id, created_at) VALUES (?, ?)",
        "INSERT INTO conversations (id, created_at) VALUES ($1::uuid, $2)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(id)
}

/// Add a participant to a conversation.
pub async fn add_participant(db: &Database, conversation_id: &str, account: &str) -> Result<()> {
    let sql = db.sql(
        "INSERT INTO conversation_participants (conversation_id, account, last_read_at, muted_until) VALUES (?, ?, NULL, NULL)",
        "INSERT INTO conversation_participants (conversation_id, account, last_read_at, muted_until) VALUES ($1::uuid, $2::uuid, NULL, NULL)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(conversation_id)
                .bind(account)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(conversation_id)
                .bind(account)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Send a message.
pub async fn send_message(
    db: &Database,
    conversation_id: &str,
    sender: &str,
    body: &str,
) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO messages (id, conversation_id, sender, body, sent_at, deleted_at) VALUES (?, ?, ?, ?, ?, NULL)",
        "INSERT INTO messages (id, conversation_id, sender, body, sent_at, deleted_at) VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5, NULL)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(conversation_id)
                .bind(sender)
                .bind(body)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(conversation_id)
                .bind(sender)
                .bind(body)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(id)
}

/// Create a group.
pub async fn create_group(db: &Database, name: &str, privacy: &str, owner: &str) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO groups (id, name, privacy, owner, created_at) VALUES (?, ?, ?, ?, ?)",
        "INSERT INTO groups (id, name, privacy, owner, created_at) VALUES ($1::uuid, $2, $3, $4::uuid, $5)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(name)
                .bind(privacy)
                .bind(owner)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(name)
                .bind(privacy)
                .bind(owner)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(id)
}

/// Add a member to a group.
pub async fn add_group_member(
    db: &Database,
    group_id: &str,
    account: &str,
    role: &str,
) -> Result<()> {
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO group_members (group_id, account, role, joined_at) VALUES (?, ?, ?, ?)",
        "INSERT INTO group_members (group_id, account, role, joined_at) VALUES ($1::uuid, $2::uuid, $3, $4)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(group_id)
                .bind(account)
                .bind(role)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(group_id)
                .bind(account)
                .bind(role)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Create a forum topic.
pub async fn create_topic(
    db: &Database,
    category_id: &str,
    author_pseud: &str,
    title: &str,
) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO forum_topics (id, category_id, author_pseud, title, created_at, last_post_at, locked) VALUES (?, ?, ?, ?, ?, ?, 0)",
        "INSERT INTO forum_topics (id, category_id, author_pseud, title, created_at, last_post_at, locked) VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5, $5, 0)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(category_id)
                .bind(author_pseud)
                .bind(title)
                .bind(&now)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(category_id)
                .bind(author_pseud)
                .bind(title)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(id)
}

/// Create a forum post.
pub async fn create_post(
    db: &Database,
    topic_id: &str,
    author_pseud: &str,
    body: &str,
) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO forum_posts (id, topic_id, author_pseud, body, created_at, deleted_at) VALUES (?, ?, ?, ?, ?, NULL)",
        "INSERT INTO forum_posts (id, topic_id, author_pseud, body, created_at, deleted_at) VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5, NULL)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(topic_id)
                .bind(author_pseud)
                .bind(body)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(topic_id)
                .bind(author_pseud)
                .bind(body)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(id)
}
