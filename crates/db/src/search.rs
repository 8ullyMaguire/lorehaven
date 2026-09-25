//! Search repository: rebuild index, search works, search in-work.
//!
//! Spec §15.9–15.10. Both dialects.

use crate::{Backend, Database};
use anyhow::{Context, Result};
use lorehaven_domain::ids::WorkId;
use serde::Serialize;

mod ast_search;

pub use ast_search::search_works_ast;
pub use ast_search::search_works_ast_filtered;

/// A single search result.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub work_id: String,
    pub title: String,
    pub author_handle: String,
    pub word_count: i64,
    pub score: i64,
}

/// A paragraph anchor for in-work search.
#[derive(Debug, Clone, Serialize)]
pub struct InWorkMatch {
    pub pos: i64,
    pub snippet: String,
}

/// Rebuild the index for a single work.
///
/// Idempotent: stages the new term set, then swaps inside one transaction.
pub async fn rebuild_work_index(db: &Database, work_id: &WorkId, body_text: &str) -> Result<()> {
    match db.backend() {
        Backend::Sqlite => rebuild_work_index_sqlite(db, work_id, body_text).await,
        Backend::Postgres => rebuild_work_index_postgres(db, work_id, body_text).await,
    }
}

async fn rebuild_work_index_sqlite(db: &Database, work_id: &WorkId, body_text: &str) -> Result<()> {
    let pool = db.sqlite_pool().expect("sqlite");
    let mut tx = pool.begin().await.context("begin transaction")?;

    sqlx::query("DELETE FROM works_index_terms WHERE work_id = ?")
        .bind(work_id.to_string())
        .execute(&mut *tx)
        .await
        .context("delete old index terms")?;

    sqlx::query("DELETE FROM works_index WHERE work_id = ?")
        .bind(work_id.to_string())
        .execute(&mut *tx)
        .await
        .context("delete old index row")?;

    sqlx::query("INSERT INTO works_index (work_id, body_text) VALUES (?, ?)")
        .bind(work_id.to_string())
        .bind(body_text)
        .execute(&mut *tx)
        .await
        .context("insert work index row")?;

    let terms: Vec<(String, i64)> = body_text
        .split_whitespace()
        .enumerate()
        .map(|(i, term)| {
            let normalized = term
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase();
            (normalized, i as i64)
        })
        .filter(|(t, _)| !t.is_empty())
        .collect();

    for (term, pos) in &terms {
        sqlx::query("INSERT INTO works_index_terms (work_id, term, pos) VALUES (?, ?, ?)")
            .bind(work_id.to_string())
            .bind(term)
            .bind(pos)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("insert term {}", term))?;
    }

    tx.commit().await?;
    Ok(())
}

async fn rebuild_work_index_postgres(
    db: &Database,
    work_id: &WorkId,
    body_text: &str,
) -> Result<()> {
    let pool = db.postgres_pool().expect("postgres");
    let mut tx = pool.begin().await.context("begin transaction")?;

    sqlx::query("DELETE FROM works_index_terms WHERE work_id::text = ?")
        .bind(work_id.to_string())
        .execute(&mut *tx)
        .await
        .context("delete old index terms")?;

    sqlx::query("DELETE FROM works_index WHERE work_id::text = ?")
        .bind(work_id.to_string())
        .execute(&mut *tx)
        .await
        .context("delete old index row")?;

    sqlx::query("INSERT INTO works_index (work_id, body_text) VALUES (?::uuid, ?)")
        .bind(work_id.to_string())
        .bind(body_text)
        .execute(&mut *tx)
        .await
        .context("insert work index row")?;

    let terms: Vec<(String, i64)> = body_text
        .split_whitespace()
        .enumerate()
        .map(|(i, term)| {
            let normalized = term
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase();
            (normalized, i as i64)
        })
        .filter(|(t, _)| !t.is_empty())
        .collect();

    for (term, pos) in &terms {
        sqlx::query("INSERT INTO works_index_terms (work_id, term, pos) VALUES (?::uuid, ?, ?)")
            .bind(work_id.to_string())
            .bind(term)
            .bind(pos)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("insert term {}", term))?;
    }

    tx.commit().await?;
    Ok(())
}

/// Search published public works by a free-text term (simple body search).
///
/// This is the anonymous door (`GET /api/v1/public/search`), so the predicate
/// is the anonymous one from `ast_search`: only `published` works with
/// `visibility = 'public'`. The term index is not a visibility boundary — the
/// worker fills it from chapter text regardless of lifecycle, and a deindex
/// event can lag or be overtaken by a `Reindex` that lands after a withdrawal,
/// so the route is what keeps an unpublished work out of the results (§3.3).
///
/// `word_count` is summed from live chapter revisions, the same way
/// `ast_search` computes it: the denormalised `works.word_count` column is not
/// maintained, so reading it reported zero for every result.
pub async fn search_works(db: &Database, needle: &str, limit: i64) -> Result<Vec<SearchResult>> {
    let needle = needle.trim();
    if needle.is_empty() {
        return Ok(Vec::new());
    }

    let words: Vec<&str> = needle.split_whitespace().collect();
    if words.is_empty() {
        return Ok(Vec::new());
    }

    match db.backend() {
        Backend::Sqlite => {
            let like_placeholders = words
                .iter()
                .map(|_| "t.term LIKE ?")
                .collect::<Vec<_>>()
                .join(" OR ");
            let sql = format!(
                "SELECT w.id, w.title, a.handle AS author_handle, \
                        (SELECT COALESCE(CAST(SUM(cr.word_count) AS BIGINT), 0) \
                         FROM chapters c \
                         JOIN chapter_revisions cr ON cr.id = c.current_revision_id \
                         WHERE c.work_id = w.id AND c.deleted_at IS NULL) AS word_count, \
                        COUNT(t.term) AS score \
                 FROM works_index_terms t \
                 JOIN works w ON w.id = t.work_id \
                 JOIN pseuds a ON a.id = w.owner_pseud_id \
                 WHERE ({}) AND w.lifecycle = 'published' AND w.visibility = 'public' \
                 GROUP BY w.id \
                 ORDER BY score DESC \
                 LIMIT ?",
                like_placeholders
            );
            let mut query = sqlx::query_as::<_, (String, String, String, i64, i64)>(&sql);
            for word in &words {
                query = query.bind(format!("{}%", word.trim().to_lowercase()));
            }
            query = query.bind(limit);
            let rows = query
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await
                .context("search works")?;
            Ok(rows
                .into_iter()
                .map(
                    |(id, title, author_handle, word_count, score)| SearchResult {
                        work_id: id,
                        title,
                        author_handle,
                        word_count,
                        score,
                    },
                )
                .collect())
        }
        Backend::Postgres => {
            let like_placeholders = words
                .iter()
                .enumerate()
                .map(|(i, _)| format!("t.term LIKE ${}", i + 1))
                .collect::<Vec<_>>()
                .join(" OR ");
            let sql = format!(
                "SELECT w.id::text, w.title, a.handle AS author_handle, \
                        (SELECT COALESCE(SUM(cr.word_count), 0)::bigint \
                         FROM chapters c \
                         JOIN chapter_revisions cr ON cr.id = c.current_revision_id \
                         WHERE c.work_id = w.id AND c.deleted_at IS NULL) AS word_count, \
                        COUNT(t.term) AS score \
                 FROM works_index_terms t \
                 JOIN works w ON w.id = t.work_id \
                 JOIN pseuds a ON a.id = w.owner_pseud_id::uuid \
                 WHERE ({}) AND w.lifecycle = 'published' AND w.visibility = 'public' \
                 GROUP BY w.id, w.title, a.handle \
                 ORDER BY score DESC \
                 LIMIT ${}",
                like_placeholders,
                words.len() + 1
            );
            let mut query = sqlx::query_as::<_, (String, String, String, i64, i64)>(&sql);
            for word in &words {
                query = query.bind(format!("{}%", word.trim().to_lowercase()));
            }
            query = query.bind(limit);
            let rows = query
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await
                .context("search works")?;
            Ok(rows
                .into_iter()
                .map(
                    |(id, title, author_handle, word_count, score)| SearchResult {
                        work_id: id,
                        title,
                        author_handle,
                        word_count,
                        score,
                    },
                )
                .collect())
        }
    }
}

/// Search within a single work.
pub async fn search_in_work(
    db: &Database,
    work_id: &WorkId,
    needle: &str,
) -> Result<Vec<InWorkMatch>> {
    let needle = needle.trim().to_lowercase();
    if needle.is_empty() {
        return Ok(Vec::new());
    }

    match db.backend() {
        Backend::Sqlite => {
            let sql = "SELECT pos, term FROM works_index_terms                        WHERE work_id = ? AND term LIKE ? || '%'                        ORDER BY pos ASC LIMIT 100";
            let rows = sqlx::query_as::<_, (i64, String)>(sql)
                .bind(work_id.to_string())
                .bind(&needle)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await
                .context("search in work")?;
            Ok(rows
                .into_iter()
                .map(|(pos, snippet)| InWorkMatch { pos, snippet })
                .collect())
        }
        Backend::Postgres => {
            let sql = "SELECT pos, term FROM works_index_terms                        WHERE work_id = $1::uuid AND term LIKE $2 || '%'                        ORDER BY pos ASC LIMIT 100";
            let rows = sqlx::query_as::<_, (i64, String)>(sql)
                .bind(work_id.to_string())
                .bind(&needle)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await
                .context("search in work")?;
            Ok(rows
                .into_iter()
                .map(|(pos, snippet)| InWorkMatch { pos, snippet })
                .collect())
        }
    }
}
