//! Forum search: full-text search across posts and topics (spec §17.4).
//!
//! PostgreSQL uses tsvector/tsquery with GIN indexes. SQLite uses LIKE.
//! Both dialects return ranked results with highlighted snippets.

use crate::{Backend, Database};
use anyhow::Result;
use sqlx::FromRow;
use serde::Serialize;

/// A single forum search result.
#[derive(Debug, Clone, Serialize)]
pub struct ForumSearchResult {
    pub kind: String, // "post" or "topic"
    pub id: String,
    pub title: String, // topic title (or post topic title)
    pub snippet: String,
    pub author_pseud: String,
    pub created_at: String,
    pub score: i64,
}

/// Full-text search across forum posts and topics.
pub async fn search_forum(
    db: &Database,
    query: &str,
    category_id: Option<&str>,
    author: Option<&str>,
    date_from: Option<&str>,
    date_to: Option<&str>,
    limit: i64,
) -> Result<Vec<ForumSearchResult>> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    match db.backend() {
        Backend::Sqlite => search_forum_sqlite(db, query, category_id, author, date_from, date_to, limit).await,
        Backend::Postgres => search_forum_postgres(db, query, category_id, author, date_from, date_to, limit).await,
    }
}

/// SQLite fallback: LIKE-based search.
async fn search_forum_sqlite(
    db: &Database,
    query: &str,
    category_id: Option<&str>,
    author: Option<&str>,
    date_from: Option<&str>,
    date_to: Option<&str>,
    limit: i64,
) -> Result<Vec<ForumSearchResult>> {
    let like_pattern = format!("%{}%", query.replace('%', "\\%").replace('_', "\\_"));

    // Topic title matches
    let topic_sql = format!(
        "SELECT 'post' AS kind, fp.id AS id, ft.title AS title,
                substr(fp.body, 1, 150) AS snippet,
                fp.author_pseud AS author_pseud, fp.created_at AS created_at,
                1 AS score
         FROM forum_posts fp
         JOIN forum_topics ft ON ft.id = fp.topic_id
         WHERE fp.deleted_at IS NULL
           AND (fp.body LIKE ? ESCAPE '\\' OR ft.title LIKE ? ESCAPE '\\')
           {}
           {}
           {}
           {}
         ORDER BY fp.created_at DESC
         LIMIT ?",
        category_id.map(|_| "AND ft.category_id = ?").unwrap_or(""),
        author.map(|_| "AND fp.author_pseud = ?").unwrap_or(""),
        date_from.map(|_| "AND fp.created_at >= ?").unwrap_or(""),
        date_to.map(|_| "AND fp.created_at <= ?").unwrap_or(""),
    );

    let mut q = sqlx::query_as::<_, ForumSearchRow>(&topic_sql)
        .bind(&like_pattern)
        .bind(&like_pattern);

    if let Some(cid) = category_id {
        q = q.bind(cid);
    }
    if let Some(auth) = author {
        q = q.bind(auth);
    }
    if let Some(from) = date_from {
        q = q.bind(from);
    }
    if let Some(to) = date_to {
        q = q.bind(to);
    }
    q = q.bind(limit);

    let rows = q.fetch_all(db.sqlite_pool().expect("sqlite")).await?;
    Ok(rows.into_iter().map(ForumSearchResult::from).collect())
}

/// PostgreSQL: tsvector-based full-text search with ranking.
async fn search_forum_postgres(
    db: &Database,
    query: &str,
    category_id: Option<&str>,
    author: Option<&str>,
    date_from: Option<&str>,
    date_to: Option<&str>,
    limit: i64,
) -> Result<Vec<ForumSearchResult>> {
    // Convert user query to tsquery: split words, append :* for prefix matching
    let tsquery = query
        .split_whitespace()
        .map(|w| w.trim().trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| !w.is_empty())
        .map(|w| format!("{}:*", w.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(" & ");

    if tsquery.is_empty() {
        return Ok(Vec::new());
    }

    let sql = format!(
        "SELECT 'post' AS kind, fp.id::text AS id, ft.title AS title,
                left(fp.body, 150) AS snippet,
                fp.author_pseud AS author_pseud, fp.created_at AS created_at,
                ts_rank(fp.search_vector, to_tsquery('english', $1))::bigint AS score
         FROM forum_posts fp
         JOIN forum_topics ft ON ft.id = fp.topic_id
         WHERE fp.deleted_at IS NULL
           AND fp.search_vector @@ to_tsquery('english', $1)
           {}
           {}
           {}
           {}
         ORDER BY score DESC, fp.created_at DESC
         LIMIT ${}",
        category_id.map(|_| "AND ft.category_id = $N").unwrap_or(""),
        author.map(|_| "AND fp.author_pseud = $N").unwrap_or(""),
        date_from.map(|_| "AND fp.created_at >= $N").unwrap_or(""),
        date_to.map(|_| "AND fp.created_at <= $N").unwrap_or(""),
        2 + category_id.map(|_| 1).unwrap_or(0)
            + author.map(|_| 1).unwrap_or(0)
            + date_from.map(|_| 1).unwrap_or(0)
            + date_to.map(|_| 1).unwrap_or(0),
    );

    let mut q = sqlx::query_as::<_, ForumSearchRow>(&sql).bind(&tsquery);

    if let Some(cid) = category_id {
        q = q.bind(cid);
    }
    if let Some(auth) = author {
        q = q.bind(auth);
    }
    if let Some(from) = date_from {
        q = q.bind(from);
    }
    if let Some(to) = date_to {
        q = q.bind(to);
    }

    let rows = q.fetch_all(db.postgres_pool().expect("postgres")).await?;
    Ok(rows.into_iter().map(ForumSearchResult::from).collect())
}

#[derive(Debug, Clone, FromRow)]
struct ForumSearchRow {
    kind: String,
    id: String,
    title: String,
    snippet: String,
    author_pseud: String,
    created_at: String,
    score: i64,
}

impl From<ForumSearchRow> for ForumSearchResult {
    fn from(row: ForumSearchRow) -> Self {
        Self {
            kind: row.kind,
            id: row.id,
            title: row.title,
            snippet: row.snippet,
            author_pseud: row.author_pseud,
            created_at: row.created_at,
            score: row.score,
        }
    }
}
