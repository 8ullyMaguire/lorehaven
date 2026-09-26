//! The query language, applied to users.
//!
//! Deliberately the same shape as `forum_ast_search.rs` — same typed error,
//! same `sql_owned` dialect dispatch, same bind order — so the three surfaces
//! cannot drift apart in the ways that only show up on one backend.
//!
//! The result set is pseudonyms, joined to their accounts. A reader searching
//! for someone types the handle next to a post, so the handle is what is
//! returned and what `user:` matches; the account is here only because
//! `joined:` falls back to its `created_at` for a pseudonym added to an older
//! account, and because a `LEFT JOIN` means a pseudonym with no account row
//! still appears in its own search results.

use crate::{sql_owned, Backend, Database};
use lorehaven_domain::query::parse_query;
use lorehaven_domain::query_sql_user::render_user_query;
use serde::Serialize;

use super::SearchError;

/// One pseudonym matching a user search.
#[derive(Debug, Clone, Serialize)]
pub struct UserSearchResult {
    pub pseud_id: String,
    pub handle: String,
    pub display_name: Option<String>,
    pub joined_at: Option<String>,
}

/// Search pseudonyms with the shared query language.
///
/// `user:nightowl`, `fandoms:"Good Omens"`, `works:>10`, `joined:<2026-01-01`
/// and `works:5..50` all work here with the same operators as the works and
/// forum surfaces, and a field belonging to another surface is a typed error
/// rather than an empty page.
///
/// An empty query matches nothing. The works search treats an empty query as a
/// browse, but that would put every pseudonym on the instance into one response
/// here, so "no query" has to mean "nothing" on this surface.
pub async fn search_users_ast(
    db: &Database,
    query: &str,
    limit: i64,
) -> anyhow::Result<Vec<UserSearchResult>> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let ast = parse_query(query).map_err(|e| SearchError::parse(e.message, e.offset))?;
    let fragment = render_user_query(&ast).map_err(|e| SearchError::render(e.message))?;

    // Both dialect arms are the same string. `pseuds.id` is UUID on PostgreSQL
    // and TEXT on SQLite, but nothing here compares it to a TEXT column --
    // `works.owner_pseud_id` is UUID on both, and the fandom arm is a
    // correlated EXISTS between two same-typed columns. So there is nothing to
    // branch on, and `sql_owned` renumbers the PostgreSQL copy alone.
    let statement = render_statement(&fragment.sql);
    let sql = sql_owned(db, statement.clone(), statement);

    type Row = (String, String, Option<String>, Option<String>);
    let rows: Vec<Row> = match db.backend() {
        Backend::Sqlite => {
            let mut q = sqlx::query_as::<_, Row>(&sql);
            for b in &fragment.binds {
                q = q.bind(b.clone());
            }
            q.bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            let mut q = sqlx::query_as::<_, Row>(&sql);
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
            |(pseud_id, handle, display_name, joined_at)| UserSearchResult {
                pseud_id,
                handle,
                display_name,
                joined_at,
            },
        )
        .collect())
}

fn render_statement(user_where: &str) -> String {
    // `LEFT JOIN accounts`, not `JOIN`. A pseudonym can outlive the account row
    // it was attached to, and an inner join would drop exactly those readers
    // from their own search results -- the worst possible failure for a surface
    // whose whole job is finding a person.
    //
    // `pseuds.id` is `UUID` on PostgreSQL and `TEXT` on SQLite, and sqlx
    // decodes each into the type the dialect reports. Selecting it bare means
    // a `String` column on one backend and a `Uuid` on the other, so the row
    // tuple cannot be one Rust type for both. `CAST(... AS TEXT)` is the one
    // spelling both accept, and it makes the decoded type agree with the other
    // three columns.
    format!(
        "SELECT CAST(pseuds.id AS TEXT) AS id, pseuds.handle, pseuds.display_name, \
         COALESCE(accounts.created_at, pseuds.created_at) AS joined_at \
         FROM pseuds \
         LEFT JOIN accounts ON CAST(accounts.id AS TEXT) = CAST(pseuds.account_id AS TEXT) \
         WHERE ({user_where}) \
         ORDER BY pseuds.handle ASC \
         LIMIT ?"
    )
}
