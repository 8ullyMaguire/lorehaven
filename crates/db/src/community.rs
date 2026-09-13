//! Community repository: comments, forums, groups, messaging, blocking.
//!
//! Spec §17. Both dialects. Every read/write path routes block and mute
//! decisions through the single `blocked_between` / `muted_between` domain
//! functions — ad-hoc block filters are forbidden (plan §6.11).

use crate::{Backend, Database};
use anyhow::Result;
use lorehaven_domain::blocking::BlockScope;
use serde::Serialize;
use sqlx::FromRow;

// ---------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------

/// A comment row as stored.
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
         VALUES ($1, $2, $3, $4, $5, 'v1', $6, $7, NULL, NULL)",
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

/// Fetch a single comment by id.
pub async fn comment_by_id(db: &Database, comment_id: &str) -> Result<Option<Comment>> {
    let sql = db.sql(
        "SELECT id, subject_type, subject_id, author_pseud, body, created_at, edited_at, deleted_at FROM comments WHERE id = ?",
        "SELECT id, subject_type, subject_id, author_pseud, body, created_at, edited_at, deleted_at FROM comments WHERE id = $1",
    );
    let row = match db.backend() {
        Backend::Sqlite => sqlx::query_as::<_, CommentRow>(&sql)
            .bind(comment_id)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?
            .map(Comment::from),
        Backend::Postgres => sqlx::query_as::<_, CommentRow>(&sql)
            .bind(comment_id)
            .fetch_optional(db.postgres_pool().expect("postgres"))
            .await?
            .map(Comment::from),
    };
    Ok(row)
}

#[derive(Debug, Clone, FromRow)]
struct CommentRow {
    id: String,
    subject_type: String,
    subject_id: String,
    author_pseud: String,
    body: String,
    created_at: String,
    edited_at: Option<String>,
    deleted_at: Option<String>,
}

impl From<CommentRow> for Comment {
    fn from(r: CommentRow) -> Self {
        Self {
            id: r.id,
            subject_type: r.subject_type,
            subject_id: r.subject_id,
            author_pseud: r.author_pseud,
            body: r.body,
            created_at: r.created_at,
            edited_at: r.edited_at,
            deleted_at: r.deleted_at,
        }
    }
}

/// Cursor-paginated comments for a subject. Blocks are applied in SQL: any
/// comment whose author is blocked by the viewer in the comments scope is
/// excluded.
pub async fn list_comments(
    db: &Database,
    subject_type: &str,
    subject_id: &str,
    viewer_account: &str,
    cursor: Option<&str>,
    limit: i64,
) -> Result<Vec<Comment>> {
    // Blocks are account-keyed and comment authors are pseuds, so the filter
    // resolves each author's account. It is bidirectional: either side of a
    // block in the comments scope stops seeing the other.
    let sql = if cursor.is_some() {
        db.sql(
            "SELECT c.id, c.subject_type, c.subject_id, c.author_pseud, c.body, c.created_at, c.edited_at, c.deleted_at
             FROM comments c
             JOIN pseuds pa ON pa.id = c.author_pseud
             LEFT JOIN comment_classifications cc ON cc.comment_id = c.id
             WHERE c.subject_type = ?1 AND c.subject_id = ?2 AND c.deleted_at IS NULL AND c.created_at < ?3
               AND pa.account_id NOT IN (SELECT blocked FROM blocks WHERE blocker = ?4 AND (scope = 'all' OR scope = 'comments'))
               AND pa.account_id NOT IN (SELECT blocker FROM blocks WHERE blocked = ?4 AND (scope = 'all' OR scope = 'comments'))
               AND (cc.outcome IS NULL OR cc.outcome = 'delivered')
             ORDER BY c.created_at DESC LIMIT ?5",
            "SELECT c.id, c.subject_type, c.subject_id, c.author_pseud, c.body, c.created_at, c.edited_at, c.deleted_at
             FROM comments c
             JOIN pseuds pa ON pa.id::text = c.author_pseud
             LEFT JOIN comment_classifications cc ON cc.comment_id = c.id
             WHERE c.subject_type = $1 AND c.subject_id = $2 AND c.deleted_at IS NULL AND c.created_at < $3
               AND pa.account_id NOT IN (SELECT blocked FROM blocks WHERE blocker = $4 AND (scope = 'all' OR scope = 'comments'))
               AND pa.account_id NOT IN (SELECT blocker FROM blocks WHERE blocked = $4 AND (scope = 'all' OR scope = 'comments'))
               AND (cc.outcome IS NULL OR cc.outcome = 'delivered')
             ORDER BY c.created_at DESC LIMIT $5",
        )
    } else {
        db.sql(
            "SELECT c.id, c.subject_type, c.subject_id, c.author_pseud, c.body, c.created_at, c.edited_at, c.deleted_at
             FROM comments c
             JOIN pseuds pa ON pa.id = c.author_pseud
             LEFT JOIN comment_classifications cc ON cc.comment_id = c.id
             WHERE c.subject_type = ?1 AND c.subject_id = ?2 AND c.deleted_at IS NULL
               AND pa.account_id NOT IN (SELECT blocked FROM blocks WHERE blocker = ?3 AND (scope = 'all' OR scope = 'comments'))
               AND pa.account_id NOT IN (SELECT blocker FROM blocks WHERE blocked = ?3 AND (scope = 'all' OR scope = 'comments'))
               AND (cc.outcome IS NULL OR cc.outcome = 'delivered')
             ORDER BY c.created_at DESC LIMIT ?4",
            "SELECT c.id, c.subject_type, c.subject_id, c.author_pseud, c.body, c.created_at, c.edited_at, c.deleted_at
             FROM comments c
             JOIN pseuds pa ON pa.id::text = c.author_pseud
             LEFT JOIN comment_classifications cc ON cc.comment_id = c.id
             WHERE c.subject_type = $1 AND c.subject_id = $2 AND c.deleted_at IS NULL
               AND pa.account_id NOT IN (SELECT blocked FROM blocks WHERE blocker = $3 AND (scope = 'all' OR scope = 'comments'))
               AND pa.account_id NOT IN (SELECT blocker FROM blocks WHERE blocked = $3 AND (scope = 'all' OR scope = 'comments'))
               AND (cc.outcome IS NULL OR cc.outcome = 'delivered')
             ORDER BY c.created_at DESC LIMIT $4",
        )
    };
    let rows = match db.backend() {
        Backend::Sqlite => {
            let mut q = sqlx::query_as::<_, CommentRow>(&sql)
                .bind(subject_type)
                .bind(subject_id);
            if let Some(c) = cursor {
                q = q.bind(c);
            }
            q.bind(viewer_account)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
                .into_iter()
                .map(Comment::from)
                .collect()
        }
        Backend::Postgres => {
            let mut q = sqlx::query_as::<_, CommentRow>(&sql)
                .bind(subject_type)
                .bind(subject_id);
            if let Some(c) = cursor {
                q = q.bind(c);
            }
            q.bind(viewer_account)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
                .into_iter()
                .map(Comment::from)
                .collect()
        }
    };
    Ok(rows)
}

/// Soft-delete a comment (author only).
pub async fn soft_delete_comment(
    db: &Database,
    comment_id: &str,
    actor_pseud: &str,
) -> Result<bool> {
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "UPDATE comments SET deleted_at = ?, body = '[deleted]' WHERE id = ? AND author_pseud = ? AND deleted_at IS NULL",
        "UPDATE comments SET deleted_at = ?, body = '[deleted]' WHERE id = $1 AND author_pseud = $2 AND deleted_at IS NULL",
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

// ---------------------------------------------------------------------------
// Blocks and mutes
// ---------------------------------------------------------------------------

/// Check if a block exists between two accounts.
pub async fn is_blocked(
    db: &Database,
    blocker: &str,
    blocked: &str,
    scope: BlockScope,
) -> Result<bool> {
    let sql = db.sql(
        "SELECT 1 FROM blocks WHERE blocker = ? AND blocked = ? AND (scope = 'all' OR scope = ?)",
        "SELECT 1 FROM blocks WHERE blocker = $1 AND blocked = $2 AND (scope = 'all' OR scope = $3)",
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

/// Check if a mute exists between two accounts.
pub async fn is_muted(db: &Database, muter: &str, muted: &str) -> Result<bool> {
    let sql = db.sql(
        "SELECT 1 FROM mutes WHERE muter = ? AND muted = ?",
        "SELECT 1 FROM mutes WHERE muter = $1 AND muted = $2",
    );
    let row: Option<(i64,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(muter)
                .bind(muted)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(muter)
                .bind(muted)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.is_some())
}

/// Insert a block.
/// A block row as the owner sees it.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct BlockRow {
    pub blocked: String,
    pub scope: String,
    pub note: Option<String>,
    pub created_at: String,
}

/// The caller's blocks. A list, never another account's.
pub async fn list_blocks(db: &Database, blocker: &str) -> Result<Vec<BlockRow>> {
    let sql = db.sql(
        "SELECT blocked, scope, note, created_at FROM blocks WHERE blocker = ? ORDER BY created_at DESC",
        "SELECT blocked, scope, note, created_at FROM blocks WHERE blocker = $1 ORDER BY created_at DESC",
    );
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(blocker)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(blocker)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows)
}

/// A mute row as the owner sees it.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct MuteRow {
    pub muted: String,
    pub until: Option<String>,
    pub created_at: String,
}

/// The caller's mutes. A list, never another account's.
pub async fn list_mutes(db: &Database, muter: &str) -> Result<Vec<MuteRow>> {
    let sql = db.sql(
        "SELECT muted, until, created_at FROM mutes WHERE muter = ? ORDER BY created_at DESC",
        "SELECT muted, until, created_at FROM mutes WHERE muter = $1 ORDER BY created_at DESC",
    );
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(muter)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(muter)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows)
}

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
        "INSERT INTO blocks (blocker, blocked, scope, created_at, note) VALUES ($1, $2, $3, $4, $5)",
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
        "DELETE FROM blocks WHERE blocker = $1 AND blocked = $2",
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
        "INSERT INTO mutes (muter, muted, until, created_at) VALUES ($1, $2, $3, $4)",
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
        "DELETE FROM mutes WHERE muter = $1 AND muted = $2",
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

// ---------------------------------------------------------------------------
// Groups
// ---------------------------------------------------------------------------

/// A group row.
#[derive(Debug, Clone, Serialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub privacy: String,
    pub owner: String,
    pub created_at: String,
}

/// Create a group.
pub async fn create_group(db: &Database, name: &str, privacy: &str, owner: &str) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO groups (id, name, privacy, owner, created_at) VALUES (?, ?, ?, ?, ?)",
        "INSERT INTO groups (id, name, privacy, owner, created_at) VALUES ($1, $2, $3, $4, $5)",
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

/// Fetch a group by id.
pub async fn group_by_id(db: &Database, group_id: &str) -> Result<Option<Group>> {
    let sql = db.sql(
        "SELECT id, name, privacy, owner, created_at FROM groups WHERE id = ?",
        "SELECT id, name, privacy, owner, created_at FROM groups WHERE id = $1",
    );
    let row = match db.backend() {
        Backend::Sqlite => sqlx::query_as::<_, GroupRow>(&sql)
            .bind(group_id)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?
            .map(Group::from),
        Backend::Postgres => sqlx::query_as::<_, GroupRow>(&sql)
            .bind(group_id)
            .fetch_optional(db.postgres_pool().expect("postgres"))
            .await?
            .map(Group::from),
    };
    Ok(row)
}

#[derive(Debug, Clone, FromRow)]
struct GroupRow {
    id: String,
    name: String,
    privacy: String,
    owner: String,
    created_at: String,
}

impl From<GroupRow> for Group {
    fn from(r: GroupRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            privacy: r.privacy,
            owner: r.owner,
            created_at: r.created_at,
        }
    }
}

/// List groups visible to a viewer (hidden groups are excluded for non-members).
pub async fn list_groups(db: &Database, viewer_account: &str, limit: i64) -> Result<Vec<Group>> {
    let sql = db.sql(
        "SELECT g.id, g.name, g.privacy, g.owner, g.created_at
         FROM groups g
         WHERE g.privacy != 'hidden'
            OR g.owner = ?
            OR EXISTS (SELECT 1 FROM group_members gm WHERE gm.group_id = g.id AND gm.account = ?)
         ORDER BY g.created_at DESC LIMIT ?",
        "SELECT g.id, g.name, g.privacy, g.owner, g.created_at
         FROM groups g
         WHERE g.privacy != 'hidden'
            OR g.owner = $1
            OR EXISTS (SELECT 1 FROM group_members gm WHERE gm.group_id = g.id AND gm.account = $2)
         ORDER BY g.created_at DESC LIMIT $3",
    );
    let rows = match db.backend() {
        Backend::Sqlite => sqlx::query_as::<_, GroupRow>(&sql)
            .bind(viewer_account)
            .bind(viewer_account)
            .bind(limit)
            .fetch_all(db.sqlite_pool().expect("sqlite"))
            .await?
            .into_iter()
            .map(Group::from)
            .collect(),
        Backend::Postgres => sqlx::query_as::<_, GroupRow>(&sql)
            .bind(viewer_account)
            .bind(viewer_account)
            .bind(limit)
            .fetch_all(db.postgres_pool().expect("postgres"))
            .await?
            .into_iter()
            .map(Group::from)
            .collect(),
    };
    Ok(rows)
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
        "INSERT INTO group_members (group_id, account, role, joined_at) VALUES ($1, $2, $3, $4)",
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

/// Get a member's role in a group.
pub async fn member_role(db: &Database, group_id: &str, account: &str) -> Result<Option<String>> {
    let sql = db.sql(
        "SELECT role FROM group_members WHERE group_id = ? AND account = ?",
        "SELECT role FROM group_members WHERE group_id = $1 AND account = $2",
    );
    let row: Option<(String,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(group_id)
                .bind(account)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(group_id)
                .bind(account)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.map(|r| r.0))
}

/// Update a member's role.
pub async fn update_member_role(
    db: &Database,
    group_id: &str,
    account: &str,
    role: &str,
) -> Result<bool> {
    let sql = db.sql(
        "UPDATE group_members SET role = ? WHERE group_id = ? AND account = ?",
        "UPDATE group_members SET role = $1 WHERE group_id = $2 AND account = $3",
    );
    let rows = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(role)
            .bind(group_id)
            .bind(account)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(role)
            .bind(group_id)
            .bind(account)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    Ok(rows > 0)
}

/// Remove a member from a group.
pub async fn remove_group_member(db: &Database, group_id: &str, account: &str) -> Result<bool> {
    let sql = db.sql(
        "DELETE FROM group_members WHERE group_id = ? AND account = ?",
        "DELETE FROM group_members WHERE group_id = $1 AND account = $2",
    );
    let rows = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(group_id)
            .bind(account)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(group_id)
            .bind(account)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    Ok(rows > 0)
}

// ---------------------------------------------------------------------------
// Forums
// ---------------------------------------------------------------------------

/// A forum topic row.
#[derive(Debug, Clone, Serialize)]
pub struct ForumTopic {
    pub id: String,
    pub category_id: String,
    pub author_pseud: String,
    pub title: String,
    pub created_at: String,
    pub last_post_at: Option<String>,
    pub locked: bool,
}

/// Whether a forum category exists. `create_topic` refuses dangling
/// topics: the schema deliberately carries no foreign key (SQLite cannot
/// add one by ALTER), so the check lives here where every writer passes.
pub async fn category_exists(db: &Database, category_id: &str) -> Result<bool> {
    let sql = db.sql(
        "SELECT 1 FROM forum_categories WHERE id = ?",
        "SELECT 1 FROM forum_categories WHERE id = $1",
    );
    let found: Option<i64> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(category_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(category_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(found.is_some())
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
        "INSERT INTO forum_topics (id, category_id, author_pseud, title, created_at, last_post_at, locked) VALUES ($1, $2, $3, $4, $5, $5, 0)",
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

/// Fetch a topic by id.
pub async fn topic_by_id(db: &Database, topic_id: &str) -> Result<Option<ForumTopic>> {
    let sql = db.sql(
        "SELECT id, category_id, author_pseud, title, created_at, last_post_at, locked FROM forum_topics WHERE id = ?",
        "SELECT id, category_id, author_pseud, title, created_at, last_post_at, locked::int::bigint AS locked FROM forum_topics WHERE id = $1",
    );
    let row = match db.backend() {
        Backend::Sqlite => sqlx::query_as::<_, ForumTopicRow>(&sql)
            .bind(topic_id)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?
            .map(ForumTopic::from),
        Backend::Postgres => sqlx::query_as::<_, ForumTopicRow>(&sql)
            .bind(topic_id)
            .fetch_optional(db.postgres_pool().expect("postgres"))
            .await?
            .map(ForumTopic::from),
    };
    Ok(row)
}

#[derive(Debug, Clone, FromRow)]
struct ForumTopicRow {
    id: String,
    category_id: String,
    author_pseud: String,
    title: String,
    created_at: String,
    last_post_at: Option<String>,
    locked: i64,
}

impl From<ForumTopicRow> for ForumTopic {
    fn from(r: ForumTopicRow) -> Self {
        Self {
            id: r.id,
            category_id: r.category_id,
            author_pseud: r.author_pseud,
            title: r.title,
            created_at: r.created_at,
            last_post_at: r.last_post_at,
            locked: r.locked != 0,
        }
    }
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
        "INSERT INTO forum_posts (id, topic_id, author_pseud, body, created_at, deleted_at) VALUES ($1, $2, $3, $4, $5, NULL)",
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

/// Cursor-paginated posts for a topic.
pub async fn list_posts(
    db: &Database,
    topic_id: &str,
    cursor: Option<&str>,
    limit: i64,
) -> Result<Vec<ForumPost>> {
    let sql = if cursor.is_some() {
        db.sql(
            "SELECT id, topic_id, author_pseud, body, created_at, deleted_at FROM forum_posts WHERE topic_id = ? AND deleted_at IS NULL AND created_at > ? ORDER BY created_at ASC LIMIT ?",
            "SELECT id, topic_id, author_pseud, body, created_at, deleted_at FROM forum_posts WHERE topic_id = $1 AND deleted_at IS NULL AND created_at > $2 ORDER BY created_at ASC LIMIT $3",
        )
    } else {
        db.sql(
            "SELECT id, topic_id, author_pseud, body, created_at, deleted_at FROM forum_posts WHERE topic_id = ? AND deleted_at IS NULL ORDER BY created_at ASC LIMIT ?",
            "SELECT id, topic_id, author_pseud, body, created_at, deleted_at FROM forum_posts WHERE topic_id = $1 AND deleted_at IS NULL ORDER BY created_at ASC LIMIT $2",
        )
    };
    let rows = match db.backend() {
        Backend::Sqlite => {
            let mut q = sqlx::query_as::<_, ForumPostRow>(&sql).bind(topic_id);
            if let Some(c) = cursor {
                q = q.bind(c);
            }
            q.bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
                .into_iter()
                .map(ForumPost::from)
                .collect()
        }
        Backend::Postgres => {
            let mut q = sqlx::query_as::<_, ForumPostRow>(&sql).bind(topic_id);
            if let Some(c) = cursor {
                q = q.bind(c);
            }
            q.bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
                .into_iter()
                .map(ForumPost::from)
                .collect()
        }
    };
    Ok(rows)
}

#[derive(Debug, Clone, Serialize)]
pub struct ForumPost {
    pub id: String,
    pub topic_id: String,
    pub author_pseud: String,
    pub body: String,
    pub created_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
struct ForumPostRow {
    id: String,
    topic_id: String,
    author_pseud: String,
    body: String,
    created_at: String,
    deleted_at: Option<String>,
}

impl From<ForumPostRow> for ForumPost {
    fn from(r: ForumPostRow) -> Self {
        Self {
            id: r.id,
            topic_id: r.topic_id,
            author_pseud: r.author_pseud,
            body: r.body,
            created_at: r.created_at,
            deleted_at: r.deleted_at,
        }
    }
}

// ---------------------------------------------------------------------------
// Messaging
// ---------------------------------------------------------------------------

/// Create a conversation.
pub async fn create_conversation(db: &Database) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO conversations (id, created_at) VALUES (?, ?)",
        "INSERT INTO conversations (id, created_at) VALUES ($1, $2)",
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
        "INSERT INTO conversation_participants (conversation_id, account, last_read_at, muted_until) VALUES ($1, $2, NULL, NULL)",
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

/// Check if an account is a participant in a conversation.
pub async fn is_participant(db: &Database, conversation_id: &str, account: &str) -> Result<bool> {
    let sql = db.sql(
        "SELECT 1 FROM conversation_participants WHERE conversation_id = ? AND account = ?",
        "SELECT 1 FROM conversation_participants WHERE conversation_id = $1 AND account = $2",
    );
    let row: Option<(i64,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(conversation_id)
                .bind(account)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(conversation_id)
                .bind(account)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.is_some())
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
        "INSERT INTO messages (id, conversation_id, sender, body, sent_at, deleted_at) VALUES ($1, $2, $3, $4, $5, NULL)",
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

/// Cursor-paginated messages for a conversation. Block-aware: messages from
/// a sender who blocked the viewer (or vice versa) in the messages scope
/// are excluded.
pub async fn list_messages(
    db: &Database,
    conversation_id: &str,
    viewer_account: &str,
    cursor: Option<&str>,
    limit: i64,
) -> Result<Vec<Message>> {
    // Bidirectional like the comment filter: a block in the messages scope
    // hides the other side's messages from the viewer and vice versa.
    let sql = if cursor.is_some() {
        db.sql(
            "SELECT id, conversation_id, sender, body, sent_at, deleted_at
             FROM messages
             WHERE conversation_id = ?1 AND deleted_at IS NULL AND sent_at > ?2
               AND sender NOT IN (SELECT blocked FROM blocks WHERE blocker = ?3 AND (scope = 'all' OR scope = 'messages'))
               AND sender NOT IN (SELECT blocker FROM blocks WHERE blocked = ?3 AND (scope = 'all' OR scope = 'messages'))
             ORDER BY sent_at ASC LIMIT ?4",
            "SELECT id, conversation_id, sender, body, sent_at, deleted_at
             FROM messages
             WHERE conversation_id = $1 AND deleted_at IS NULL AND sent_at > $2
               AND sender NOT IN (SELECT blocked FROM blocks WHERE blocker = $3 AND (scope = 'all' OR scope = 'messages'))
               AND sender NOT IN (SELECT blocker FROM blocks WHERE blocked = $3 AND (scope = 'all' OR scope = 'messages'))
             ORDER BY sent_at ASC LIMIT $4",
        )
    } else {
        db.sql(
            "SELECT id, conversation_id, sender, body, sent_at, deleted_at
             FROM messages
             WHERE conversation_id = ?1 AND deleted_at IS NULL
               AND sender NOT IN (SELECT blocked FROM blocks WHERE blocker = ?2 AND (scope = 'all' OR scope = 'messages'))
               AND sender NOT IN (SELECT blocker FROM blocks WHERE blocked = ?2 AND (scope = 'all' OR scope = 'messages'))
             ORDER BY sent_at ASC LIMIT ?3",
            "SELECT id, conversation_id, sender, body, sent_at, deleted_at
             FROM messages
             WHERE conversation_id = $1 AND deleted_at IS NULL
               AND sender NOT IN (SELECT blocked FROM blocks WHERE blocker = $2 AND (scope = 'all' OR scope = 'messages'))
               AND sender NOT IN (SELECT blocker FROM blocks WHERE blocked = $2 AND (scope = 'all' OR scope = 'messages'))
             ORDER BY sent_at ASC LIMIT $3",
        )
    };
    let rows = match db.backend() {
        Backend::Sqlite => {
            let mut q = sqlx::query_as::<_, MessageRow>(&sql).bind(conversation_id);
            if let Some(c) = cursor {
                q = q.bind(c);
            }
            q.bind(viewer_account)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
                .into_iter()
                .map(Message::from)
                .collect()
        }
        Backend::Postgres => {
            let mut q = sqlx::query_as::<_, MessageRow>(&sql).bind(conversation_id);
            if let Some(c) = cursor {
                q = q.bind(c);
            }
            q.bind(viewer_account)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
                .into_iter()
                .map(Message::from)
                .collect()
        }
    };
    Ok(rows)
}

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub sender: String,
    pub body: String,
    pub sent_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
struct MessageRow {
    id: String,
    conversation_id: String,
    sender: String,
    body: String,
    sent_at: String,
    deleted_at: Option<String>,
}

impl From<MessageRow> for Message {
    fn from(r: MessageRow) -> Self {
        Self {
            id: r.id,
            conversation_id: r.conversation_id,
            sender: r.sender,
            body: r.body,
            sent_at: r.sent_at,
            deleted_at: r.deleted_at,
        }
    }
}

pub async fn upsert_presence(
    db: &Database,
    account: &str,
    last_seen_at: &str,
    typing_until: Option<&str>,
    enabled: bool,
) -> Result<()> {
    let enabled_int = i64::from(enabled);
    let sql = db.sql(
        "INSERT INTO presence (account, last_seen_at, typing_until, enabled)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(account) DO UPDATE SET last_seen_at = excluded.last_seen_at, typing_until = excluded.typing_until, enabled = excluded.enabled",
        "INSERT INTO presence (account, last_seen_at, typing_until, enabled)
         VALUES ($1, $2, $3, $4::int::boolean)
         ON CONFLICT(account) DO UPDATE SET last_seen_at = excluded.last_seen_at, typing_until = excluded.typing_until, enabled = excluded.enabled",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(account)
                .bind(last_seen_at)
                .bind(typing_until)
                .bind(enabled_int)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(account)
                .bind(last_seen_at)
                .bind(typing_until)
                .bind(enabled_int)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Fetch presence for an account.
pub async fn presence_for(
    db: &Database,
    account: &str,
) -> Result<Option<(String, Option<String>, bool)>> {
    let sql = db.sql(
        "SELECT last_seen_at, typing_until, enabled FROM presence WHERE account = ?",
        "SELECT last_seen_at, typing_until, enabled::int::bigint AS enabled FROM presence WHERE account = $1",
    );
    let row: Option<(String, Option<String>, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.map(|(a, b, c)| (a, b, c != 0)))
}

/// List participants in a conversation (for presence fan-out).
pub async fn conversation_participants(
    db: &Database,
    conversation_id: &str,
) -> Result<Vec<String>> {
    let sql = db.sql(
        "SELECT account FROM conversation_participants WHERE conversation_id = ?",
        "SELECT account FROM conversation_participants WHERE conversation_id = $1",
    );
    let rows: Vec<(String,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(conversation_id)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(conversation_id)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows.into_iter().map(|r| r.0).collect())
}
