//! The query language, applied to the forum.
//!
//! The parser and the renderer live in `lorehaven_domain`; this is the
//! statement around the rendered fragment and the rows it produces. It is
//! deliberately the same shape as `ast_search.rs` — same typed error, same
//! dialect pair, same bind order — so the two surfaces cannot drift apart in
//! the ways that only show up on one backend.

use crate::{sql_owned, Backend, Database};
use lorehaven_domain::query::parse_query;
use lorehaven_domain::query_sql_forum::render_forum_query;
use serde::Serialize;

use super::SearchError;

/// One thread matching a forum search.
#[derive(Debug, Clone, Serialize)]
pub struct ForumSearchResult {
    pub topic_id: String,
    pub title: String,
    pub author_handle: String,
    pub category: String,
    pub reply_count: i64,
    pub last_post_at: Option<String>,
}

/// Search forum threads with the shared query language.
///
/// `replies:>50`, `category:meta`, `author:nightowl`, `active:>2026-01-15`
/// and `locked:true` all work here with the same operators as the works
/// surface, and a field belonging to another surface is a typed error rather
/// than an empty page.
///
/// An empty query matches nothing. The works search treats an empty query as a
/// browse, but that would put every post on the instance into one response
/// here, so "no query" has to mean "nothing" on this surface.
pub async fn search_forum_ast(
    db: &Database,
    query: &str,
    limit: i64,
) -> anyhow::Result<Vec<ForumSearchResult>> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let ast = parse_query(query).map_err(|e| SearchError::parse(e.message, e.offset))?;
    let fragment = render_forum_query(&ast).map_err(|e| SearchError::render(e.message))?;

    // Both dialect arms are the same string, which is the point: every column
    // this statement touches has the same type on both backends, so there is
    // nothing to branch on and nothing that can drift. `sql_owned` renumbers
    // the PostgreSQL copy to `$n` and leaves the SQLite copy alone, and a
    // dialect pair that happens to be identical still goes through the same
    // path as one that is not.
    let statement = render_statement(&fragment.sql);
    let sql = sql_owned(db, statement.clone(), statement);

    let rows: Vec<(String, String, String, String, i64, Option<String>)> = match db.backend() {
        Backend::Sqlite => {
            let mut q =
                sqlx::query_as::<_, (String, String, String, String, i64, Option<String>)>(&sql);
            for b in &fragment.binds {
                q = q.bind(b.clone());
            }
            q.bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            let mut q =
                sqlx::query_as::<_, (String, String, String, String, i64, Option<String>)>(&sql);
            for b in &fragment.binds {
                q = q.bind(b.clone());
            }
            q.bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };

    Ok(rows
        .into_iter()
        .map(
            |(topic_id, title, author_handle, category, reply_count, last_post_at)| {
                ForumSearchResult {
                    topic_id,
                    title,
                    author_handle,
                    category,
                    reply_count,
                    last_post_at,
                }
            },
        )
        .collect())
}

fn render_statement(user_where: &str) -> String {
    // The only schema divergence this statement has to survive is `pseuds.id`:
    // `UUID` on PostgreSQL, `TEXT` on SQLite, while `forum_topics.author_pseud`
    // is `TEXT` on both. `CAST(... AS TEXT)` is the one spelling both accept --
    // uuid-to-text on PostgreSQL, text-to-text on SQLite -- and `::text` is not,
    // because SQLite has no `::` operator and rejects the statement outright.
    //
    // So the join needs no dialect split, and neither does anything else:
    // `forum_topics.id` is TEXT on both, so it is selected as-is rather than
    // through a `::text` that would be a no-op on one dialect and a syntax
    // error on the other. Every placeholder is written once and renumbered by
    // `sql_owned`, which is what keeps the two dialects from drifting.
    //
    // The legacy fix would be a migration retyping the community FKs to UUID to
    // match `pseuds`. That is a real cleanup and it is not this change's job --
    // `community.rs` has been carrying the same split for the same reason.
    format!(
        "SELECT forum_topics.id, forum_topics.title, pseuds.handle AS author_handle, \
         forum_categories.name AS category, \
         CAST((SELECT COUNT(*) FROM forum_posts fp \
               WHERE fp.topic_id = forum_topics.id AND fp.deleted_at IS NULL) \
             AS BIGINT) AS reply_count, \
         forum_topics.last_post_at \
         FROM forum_topics \
         JOIN forum_categories ON forum_categories.id = forum_topics.category_id \
         JOIN pseuds ON CAST(pseuds.id AS TEXT) = forum_topics.author_pseud \
         WHERE ({user_where}) \
         ORDER BY COALESCE(forum_topics.last_post_at, forum_topics.created_at) DESC, \
                  forum_topics.title ASC \
         LIMIT ?"
    )
}
