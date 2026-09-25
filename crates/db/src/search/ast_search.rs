//! AST-based search: parse queries and render them to SQL with visibility
//! filtering. Both dialects.

use crate::search::SearchResult;
use crate::settings::ContentFilterRow;
use crate::{sql_owned, Backend, Database};
use anyhow::Result;
use lorehaven_domain::query::parse_query;
use lorehaven_domain::query_sql::render_query;

/// Search works using the AST parser and dialect-aware SQL.
pub async fn search_works_ast(
    db: &Database,
    query: &str,
    viewer_id: Option<&str>,
    limit: i64,
) -> Result<Vec<SearchResult>> {
    search_works_ast_impl(db, query, viewer_id, limit).await
}

/// Search with content-filter exclusion (spec §46.4 — blocked tag never reaches
/// the client). `filters` is a list of (filter_type, value) pairs, e.g.
/// `("tag", "enemies to lovers")`.
pub async fn search_works_ast_filtered(
    db: &Database,
    query: &str,
    viewer_id: Option<&str>,
    limit: i64,
    filters: &[(String, String)],
) -> Result<Vec<SearchResult>> {
    let results = search_works_ast_impl(db, query, viewer_id, limit).await?;
    if filters.is_empty() {
        return Ok(results);
    }
    // Post-filter: remove works whose tags match any content filter.
    // For search result sizes (≤~20) this is cheaper than dynamic SQL.
    let mut filtered = Vec::with_capacity(results.len());
    for result in results {
        let work_tags = work_tag_values(db, &result.work_id).await?;
        let excluded = filters.iter().any(|(filter_type, value)| {
            work_tags
                .iter()
                .any(|tag| tag.filter_type == *filter_type && tag.value == *value)
        });
        if !excluded {
            filtered.push(result);
        }
    }
    Ok(filtered)
}

/// Load a work's tag values with their filter types (tag/fandom/warning).
async fn work_tag_values(db: &Database, work_id: &str) -> Result<Vec<ContentFilterRow>> {
    // taxonomy_nodes.kind distinguishes tag/fandom/warning; work_tags links
    // works to nodes.
    let sql = "SELECT tn.kind AS filter_type, tn.canonical AS value \
               FROM work_tags wt \
               JOIN taxonomy_nodes tn ON tn.id = wt.node_id \
               WHERE wt.work_id = ?";
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, ContentFilterRow>(sql)
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, ContentFilterRow>(
                "SELECT tn.kind AS filter_type, tn.canonical AS value \
                 FROM work_tags wt \
                 JOIN taxonomy_nodes tn ON tn.id = wt.node_id \
                 WHERE wt.work_id::text = ?",
            )
            .bind(work_id)
            .fetch_all(db.postgres_pool().expect("postgres handle"))
            .await?
        }
    };
    Ok(rows)
}

/// Search works using the AST parser and dialect-aware SQL.
async fn search_works_ast_impl(
    db: &Database,
    query: &str,
    viewer_id: Option<&str>,
    limit: i64,
) -> Result<Vec<SearchResult>> {
    let (user_where, user_binds) = if query.trim().is_empty() {
        ("1=1".to_owned(), Vec::new())
    } else {
        let ast = parse_query(query).map_err(|e| {
            anyhow::anyhow!("query parse error: {} at offset {}", e.message, e.offset)
        })?;
        let fragment = render_query(&ast).map_err(|e| {
            anyhow::anyhow!("query render error: {} at offset {}", e.message, e.offset)
        })?;
        (fragment.sql, fragment.binds)
    };

    let sql = match viewer_id {
        Some(_) => {
            let sqlite = format!(
                "SELECT works.id, works.title, pseuds.handle AS author_handle, \
                        (SELECT COALESCE(CAST(SUM(cr.word_count) AS BIGINT), 0) FROM chapters c \
                         JOIN chapter_revisions cr ON cr.id = c.current_revision_id \
                         WHERE c.work_id = works.id AND c.deleted_at IS NULL) AS word_count, \
                        COUNT(works_index_terms.term) AS score \
                 FROM works \
                 LEFT JOIN works_index_terms ON works_index_terms.work_id = works.id \
                 LEFT JOIN works_index ON works_index.work_id = works.id \
                 JOIN pseuds ON pseuds.id = works.owner_pseud_id \
                 WHERE ((works.lifecycle = 'published' AND works.visibility != 'restricted') \
                        OR works.owner_pseud_id IN (SELECT id FROM pseuds WHERE account_id = ?)) \
                   AND ({user_where}) \
                   AND NOT EXISTS (SELECT 1 FROM blocks \
                                   WHERE blocks.blocker = ? \
                                     AND blocks.blocked = pseuds.account_id \
                                     AND blocks.scope = 'all') \
                 GROUP BY works.id, pseuds.handle \
                 ORDER BY score DESC, works.updated_at DESC \
                 LIMIT ?"
            );
            // $1 is the viewer account (visibility filter and block filter
            // share it); the user fragment's placeholders start at $2 and
            // LIMIT follows the fragment. sqlx binds values in numeric
            // placeholder order, and the binds below are viewer, fragment,
            // LIMIT — so the numbering must not skip a value.
            let user_where_pg = renumber_placeholders(&user_where, 1);
            let limit_pg = 2 + user_binds.len();
            let pg = format!(
                "SELECT works.id::text, works.title, pseuds.handle AS author_handle, \
                        (SELECT COALESCE(SUM(cr.word_count), 0)::bigint FROM chapters c \
                         JOIN chapter_revisions cr ON cr.id = c.current_revision_id \
                         WHERE c.work_id = works.id AND c.deleted_at IS NULL) AS word_count, \
                        COUNT(works_index_terms.term) AS score \
                 FROM works \
                 LEFT JOIN works_index_terms ON works_index_terms.work_id = works.id \
                 LEFT JOIN works_index ON works_index.work_id = works.id \
                 JOIN pseuds ON pseuds.id = works.owner_pseud_id \
                 WHERE ((works.lifecycle = 'published' AND works.visibility != 'restricted') \
                        OR works.owner_pseud_id IN (SELECT id FROM pseuds WHERE account_id = $1::uuid)) \
                   AND ({user_where_pg}) \
                   AND NOT EXISTS (SELECT 1 FROM blocks \
                                   WHERE blocks.blocker = $1 \
                                     AND blocks.blocked = pseuds.account_id::text \
                                     AND blocks.scope = 'all') \
                 GROUP BY works.id, works.updated_at, pseuds.handle \
                 ORDER BY score DESC, works.updated_at DESC \
                 LIMIT ${limit_pg}"
            );
            sql_owned(db, sqlite, pg)
        }
        None => {
            let sqlite = format!(
                "SELECT works.id, works.title, pseuds.handle AS author_handle, \
                        (SELECT COALESCE(CAST(SUM(cr.word_count) AS BIGINT), 0) FROM chapters c \
                         JOIN chapter_revisions cr ON cr.id = c.current_revision_id \
                         WHERE c.work_id = works.id AND c.deleted_at IS NULL) AS word_count, \
                        COUNT(works_index_terms.term) AS score \
                 FROM works \
                 LEFT JOIN works_index_terms ON works_index_terms.work_id = works.id \
                 LEFT JOIN works_index ON works_index.work_id = works.id \
                 JOIN pseuds ON pseuds.id = works.owner_pseud_id \
                 WHERE (works.lifecycle = 'published' AND works.visibility = 'public') \
                   AND ({user_where}) \
                 GROUP BY works.id, pseuds.handle \
                 ORDER BY score DESC, works.updated_at DESC \
                 LIMIT ?"
            );
            let user_where_pg = renumber_placeholders(&user_where, 0);
            let limit_pg = 1 + user_binds.len();
            let pg = format!(
                "SELECT works.id::text, works.title, pseuds.handle AS author_handle, \
                        (SELECT COALESCE(SUM(cr.word_count), 0)::bigint FROM chapters c \
                         JOIN chapter_revisions cr ON cr.id = c.current_revision_id \
                         WHERE c.work_id = works.id AND c.deleted_at IS NULL) AS word_count, \
                        COUNT(works_index_terms.term) AS score \
                 FROM works \
                 LEFT JOIN works_index_terms ON works_index_terms.work_id = works.id \
                 LEFT JOIN works_index ON works_index.work_id = works.id \
                 JOIN pseuds ON pseuds.id = works.owner_pseud_id \
                 WHERE (works.lifecycle = 'published' AND works.visibility = 'public') \
                   AND ({user_where_pg}) \
                 GROUP BY works.id, works.updated_at, pseuds.handle \
                 ORDER BY score DESC, works.updated_at DESC \
                 LIMIT ${limit_pg}"
            );
            sql_owned(db, sqlite, pg)
        }
    };

    let rows = match db.backend() {
        Backend::Sqlite => {
            let mut q = sqlx::query_as::<_, (String, String, String, i64, i64)>(&sql);
            if let Some(vid) = viewer_id {
                q = q.bind(vid); // viewer_id for visibility
            }
            for b in &user_binds {
                q = q.bind(b.clone());
            }
            if let Some(vid) = viewer_id {
                q = q.bind(vid); // viewer_id for NOT EXISTS block check
            }
            q = q.bind(limit);
            q.fetch_all(db.sqlite_pool().expect("sqlite")).await?
        }
        Backend::Postgres => {
            let mut q = sqlx::query_as::<_, (String, String, String, i64, i64)>(&sql);
            if let Some(vid) = viewer_id {
                q = q.bind(vid);
            }
            for b in &user_binds {
                q = q.bind(b.clone());
            }
            q = q.bind(limit);
            q.fetch_all(db.postgres_pool().expect("postgres")).await?
        }
    };

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

fn renumber_placeholders(sql: &str, offset: usize) -> String {
    let mut out = String::new();
    let mut counter = offset;
    for ch in sql.chars() {
        if ch == '?' {
            counter += 1;
            out.push_str(&format!("${counter}"));
        } else {
            out.push(ch);
        }
    }
    out
}
