//! AST-based search: parse queries and render them to SQL with visibility
//! filtering. Both dialects.

use super::content_filter_sql::{self, FilterRule};
use crate::search::SearchResult;
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
    search_works_ast_impl(db, query, viewer_id, limit, &[]).await
}

/// Search with content-filter exclusion (spec §46.4 — a blocked tag never
/// reaches the client).
///
/// `filters` is a list of `(filter_type, value)` pairs, e.g. `("tag", "enemies
/// to lovers")`. The exclusion is rendered into the statement as a `NOT EXISTS`
/// against the work's taxonomy nodes, so it applies *before* `LIMIT` and costs
/// no extra round trips — see `content_filter_sql` for why both of those used
/// to be wrong.
pub async fn search_works_ast_filtered(
    db: &Database,
    query: &str,
    viewer_id: Option<&str>,
    limit: i64,
    filters: &[(String, String)],
) -> Result<Vec<SearchResult>> {
    let rules: Vec<FilterRule> = filters
        .iter()
        .map(|(filter_type, value)| FilterRule::new(filter_type.clone(), value.clone()))
        .collect();
    search_works_ast_impl(db, query, viewer_id, limit, &rules).await
}

/// Search works using the AST parser and dialect-aware SQL.
///
/// `rules` is the viewer's content filters. An empty slice excludes nothing, so
/// `search_works_ast` — which passes one — is exactly the unfiltered search and
/// the two paths cannot drift apart.
async fn search_works_ast_impl(
    db: &Database,
    query: &str,
    viewer_id: Option<&str>,
    limit: i64,
    rules: &[FilterRule],
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

    // Both statements are written with positional `?` and both go through
    // `sql_owned`, which renumbers only the PostgreSQL one. The old code
    // renumbered just the user fragment and hand-computed the `LIMIT` index,
    // then bound the viewer once for PostgreSQL and twice for SQLite because
    // the PostgreSQL form reused `$1` in the `blocks` subquery. Writing both
    // forms the same way removes that asymmetry: the bind list below is then
    // literally the same for either backend.
    let signed_in = viewer_id.is_some();
    let sql = sql_owned(
        db,
        render_statement(signed_in, &user_where, rules, Dialect::Sqlite),
        render_statement(signed_in, &user_where, rules, Dialect::Postgres),
    );

    // Bind order, identical for both dialects: the viewer once per placeholder
    // that fills it, the user fragment's binds, the filter rules, then the
    // limit. sqlx binds in placeholder order, and `sql_owned` made that order
    // the same on both sides.
    let filter_binds = content_filter_sql::binds(rules);
    let rows: Vec<(String, String, String, i64, i64)> = match db.backend() {
        Backend::Sqlite => {
            let mut q = sqlx::query_as::<_, (String, String, String, i64, i64)>(&sql);
            if let Some(vid) = viewer_id {
                q = q.bind(vid); // visibility subquery
            }
            for b in &user_binds {
                q = q.bind(b.clone());
            }
            if let Some(vid) = viewer_id {
                q = q.bind(vid); // NOT EXISTS blocks
            }
            for b in &filter_binds {
                q = q.bind(b);
            }
            q.bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            let mut q = sqlx::query_as::<_, (String, String, String, i64, i64)>(&sql);
            if let Some(vid) = viewer_id {
                q = q.bind(vid); // visibility subquery
            }
            for b in &user_binds {
                q = q.bind(b.clone());
            }
            if let Some(vid) = viewer_id {
                q = q.bind(vid); // NOT EXISTS blocks
            }
            for b in &filter_binds {
                q = q.bind(b);
            }
            q.bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
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

/// Which dialect to render for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dialect {
    Sqlite,
    Postgres,
}

/// Renders the search statement for one dialect.
///
/// `signed_in` selects the branch filter: a signed-in viewer sees their own
/// unlisted works and gets the `blocks` exclusion; an anonymous one sees only
/// public published works. `user_where` is the parsed query's SQL, already
/// written with `?`. `rules` is spliced in as a `NOT EXISTS` before the limit.
fn render_statement(
    signed_in: bool,
    user_where: &str,
    rules: &[FilterRule],
    dialect: Dialect,
) -> String {
    // Three type-level differences between the dialects, and only three. Each
    // is a *decoding* concern: the row is fetched into
    // `(String, String, String, i64, i64)`, and a column whose SQL type is not
    // exactly `TEXT`/`INT8` fails to decode even when the value is right.
    //
    //   * `works.id` is TEXT in SQLite, UUID in PostgreSQL. Needs `::text` to
    //     land in a `String`.
    //   * `SUM(cr.word_count)`: `bigint` in SQLite, but `numeric` in
    //     PostgreSQL -- `SUM` widens. `COALESCE(x, 0)` over a `numeric` is
    //     still `numeric`, so the `::bigint` cast goes *inside*, on the sum,
    //     not outside on the coalesce.
    //   * `COUNT(...)` is `bigint` in both, and needs no cast.
    //
    // Everything else, including every placeholder, is written identically and
    // renumbered by `sql_owned`. `COALESCE` is kept on both sides so an
    // empty aggregate is 0 rather than NULL.
    let (id_expr, sum_expr) = match dialect {
        Dialect::Sqlite => ("works.id", "CAST(SUM(cr.word_count) AS BIGINT)"),
        Dialect::Postgres => ("works.id::text", "CAST(SUM(cr.word_count) AS BIGINT)"),
    };

    // The work-id column, aliased. `work_count` and `score` must decode as
    // integers on both sides, so the aggregate is coalesced to 0 either way.
    let select = format!(
        "SELECT {id_expr}, works.title, pseuds.handle AS author_handle, \
         (SELECT COALESCE({sum_expr}, 0) FROM chapters c \
          JOIN chapter_revisions cr ON cr.id = c.current_revision_id \
          WHERE c.work_id = works.id AND c.deleted_at IS NULL) AS word_count, \
         COUNT(works_index_terms.term) AS score \
         FROM works \
         LEFT JOIN works_index_terms ON works_index_terms.work_id = works.id \
         LEFT JOIN works_index ON works_index.work_id = works.id \
         JOIN pseuds ON pseuds.id = works.owner_pseud_id "
    );

    // The visibility + block filter, which differs by whether there is a viewer.
    // `?` is used for the account id in both, and both the visibility subquery
    // and the `blocks` subquery take the viewer's id — two placeholders, two
    // binds, in that order.
    // `blocks.blocker` and `blocks.blocked` are `TEXT` in both dialects (see
    // 0013_community.sql), while `pseuds.account_id` is `UUID` in PostgreSQL.
    // Comparing them needs a cast on the pseuds side there, and none on the
    // SQLite side -- which is why this is per-dialect rather than shared.
    let blocked_account = match dialect {
        Dialect::Sqlite => "pseuds.account_id",
        Dialect::Postgres => "pseuds.account_id::text",
    };
    // Same reason on the other side of the comparison: the viewer's account id
    // arrives bound as a `String`, and `pseuds.account_id` is `UUID` in
    // PostgreSQL. A `?` bound as text against a UUID column is `operator does
    // not exist: uuid = text` -- the error that made the signed-in arm
    // unreachable on PostgreSQL, which is every authenticated search.
    let account_id = match dialect {
        Dialect::Sqlite => "account_id = ?",
        Dialect::Postgres => "account_id = ?::uuid",
    };
    let visibility = if signed_in {
        format!(
            "WHERE ((works.lifecycle = 'published' AND works.visibility != 'restricted') \
         OR works.owner_pseud_id IN (SELECT id FROM pseuds WHERE {account_id})) \
         AND ({user_where}) \
         AND NOT EXISTS (SELECT 1 FROM blocks \
                         WHERE blocks.blocker = ? \
                           AND blocks.blocked = {blocked_account} \
                           AND blocks.scope = 'all')"
        )
    } else {
        format!(
            "WHERE (works.lifecycle = 'published' AND works.visibility = 'public') \
         AND ({user_where})"
        )
    };

    // The content-filter exclusion, inside the paged statement so it applies
    // before the limit. This is the fix: the previous implementation applied
    // filters after paging, so a page could come back short.
    let exclusion = match dialect {
        Dialect::Sqlite => content_filter_sql::predicate(rules),
        Dialect::Postgres => content_filter_sql::predicate_pg(rules),
    };
    // An empty predicate must not leave a stray `AND` behind.
    let exclusion = if exclusion.is_empty() {
        String::new()
    } else {
        format!("\n         AND {exclusion}")
    };

    // PostgreSQL requires the grouped columns to include the one in ORDER BY,
    // so the two dialects' GROUP BY differ. SQLite tolerates the wider list, so
    // it is given the same one rather than a special case.
    format!(
        "{select}{visibility}{exclusion} \
         GROUP BY works.id, works.updated_at, pseuds.handle \
         ORDER BY score DESC, works.updated_at DESC \
         LIMIT ?"
    )
}
