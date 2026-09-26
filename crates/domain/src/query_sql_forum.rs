//! SQL rendering for the forum surface.
//!
//! The operators are the same as the works surface -- `>`, `>=`, `<`, `<=`,
//! `..` -- and the parser is the same, so a reader who has learned
//! `words:>10000` can write `replies:>50` without being told anything new. What
//! differs is which column each field names.
//!
//! Three rules carry over from `query_sql.rs` and are not restated here:
//!
//!   * a field belonging to another surface is an error, never a silent zero;
//!   * a negated predicate over a nullable column is coalesced, because
//!     `NOT NULL` is NULL and would drop every such row;
//!   * a value is always bound, never interpolated.
//!
//! The caller supplies the `FROM`/`JOIN` that puts `forum_categories`,
//! `forum_topics` and `forum_posts` in scope.

use crate::query::{CompareOp, QueryAst, QueryError, QueryField};
use crate::query_sql::SqlFragment;

/// Render a forum query AST to a SQL WHERE fragment.
pub fn render_forum_query(ast: &QueryAst) -> Result<SqlFragment, QueryError> {
    render_node(ast)
}

fn render_node(ast: &QueryAst) -> Result<SqlFragment, QueryError> {
    match ast {
        QueryAst::Text(text) => {
            // Both arms are coalesced. The title is NOT NULL by the schema, but
            // the `NOT` below reaches the whole expression, and one NULL arm
            // makes the whole thing NULL, which `NOT` then drops -- so a topic
            // whose post body is missing would be excluded from `NOT winter`.
            //
            // The body arm is a correlated `EXISTS`, not a join and not a
            // subquery in the SELECT list: the result set is topics, and a join
            // to `forum_posts` would return the same thread once per matching
            // reply. `EXISTS` says "some post says this" and leaves the row
            // count alone, and it short-circuits on the first match.
            //
            // Deleted posts are excluded here for the same reason they are
            // excluded from the reply count: a reader cannot see them, so a
            // search must not find them either.
            let sql = "(COALESCE(LOWER(forum_topics.title), '') LIKE LOWER(?) \
                       OR EXISTS (SELECT 1 FROM forum_posts tp \
                                 WHERE tp.topic_id = forum_topics.id \
                                   AND tp.deleted_at IS NULL \
                                   AND COALESCE(LOWER(tp.body), '') LIKE LOWER(?)))";
            let pattern = format!("%{}%", escape_like(text));
            Ok(SqlFragment::new(sql)
                .with_bind(pattern.clone())
                .with_bind(pattern))
        }
        QueryAst::Phrase(phrase) => {
            // A quoted phrase is a phrase: matching it against a topic title
            // would return topics for a phrase the reader asked to find in
            // posts.
            let sql = "EXISTS (SELECT 1 FROM forum_posts pp \
                       WHERE pp.topic_id = forum_topics.id \
                         AND pp.deleted_at IS NULL \
                         AND COALESCE(LOWER(pp.body), '') LIKE LOWER(?))";
            Ok(SqlFragment::new(sql).with_bind(format!("%{}%", escape_like(phrase))))
        }
        QueryAst::Fielded(field, value) => render_fielded(*field, value),
        QueryAst::Comparison(field, op, value) => render_comparison(*field, *op, value),
        QueryAst::And(parts) => join(parts, " AND "),
        QueryAst::Or(parts) => join(parts, " OR "),
        QueryAst::Not(inner) => {
            let inner = render_node(inner)?;
            let sql = if needs_null_guard(&inner.sql) {
                format!("NOT COALESCE({}, false)", inner.sql)
            } else {
                format!("NOT ({})", inner.sql)
            };
            Ok(SqlFragment {
                sql,
                binds: inner.binds,
            })
        }
    }
}

/// Renders each part and joins them, flattening the binds in the same order.
///
/// A one-element `And` or `Or` -- which a `..` range never produces, but a
/// parenthesised single term can -- renders as the term itself rather than as
/// a dangling `AND ()`.
fn join(parts: &[QueryAst], separator: &str) -> Result<SqlFragment, QueryError> {
    let fragments: Vec<SqlFragment> = parts
        .iter()
        .map(render_node)
        .collect::<Result<Vec<_>, _>>()?;
    let sql = fragments
        .iter()
        .map(|f| f.sql.as_str())
        .collect::<Vec<_>>()
        .join(separator);
    let binds = fragments.into_iter().flat_map(|f| f.binds).collect();
    Ok(SqlFragment { sql, binds })
}

/// Whether a rendered predicate can be NULL rather than true or false.
///
/// Same rule and same asymmetry as the works renderer: a wrong `true` only
/// costs a redundant `COALESCE`, while a missed `true` is the silent
/// zero-result bug this guards against.
fn needs_null_guard(sql: &str) -> bool {
    sql.contains(" LIKE ")
}

/// `%` and `_` are wildcards in a `LIKE` pattern. A reader searching for
/// `100%_complete` means those characters literally, and unescaped they match
/// far more than they asked for -- silently, because the result looks plausible.
fn escape_like(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn render_fielded(field: QueryField, value: &str) -> Result<SqlFragment, QueryError> {
    match field {
        // By name, not by id: the reader types `category:"meta"` and the form
        // posts a name. Comparing the id would match nothing, silently.
        QueryField::Category => {
            Ok(SqlFragment::new("LOWER(forum_categories.name) = LOWER(?)").with_bind(value))
        }
        // The *post's* author, not the topic's. A reader searching their own
        // handle means their posts; the topic author would return only the
        // threads they started.
        //
        // Two things make this arm different from the other `forum_posts`
        // lookups, and both come from the schema rather than the query:
        //
        //   * `author_pseud` holds an id and the reader types a *handle*, so
        //     `pseuds` has to be joined. Comparing the column to the literal
        //     "nightowl" would match nothing and read as "I have no posts",
        //     which is a much worse answer than an error.
        //   * that join is an id-to-id comparison across a schema divergence:
        //     `pseuds.id` is `UUID` on PostgreSQL and `TEXT` on SQLite, while
        //     `forum_posts.author_pseud` is `TEXT` on both. `pseuds.id =
        //     ap.author_pseud` is therefore rejected by PostgreSQL with
        //     `operator does not exist: uuid = text` and accepted by SQLite.
        //
        // `CAST(... AS TEXT)` is the one spelling both dialects accept:
        // uuid-to-text on PostgreSQL, text-to-text on SQLite. `::text` is not --
        // SQLite has no `::` operator and rejects the statement outright. So
        // this fragment needs no dialect parameter, which is what keeps it
        // usable by the shared renderer.
        QueryField::Author => Ok(SqlFragment::new(
            "EXISTS (SELECT 1 FROM forum_posts ap \
               JOIN pseuds aps ON CAST(aps.id AS TEXT) = ap.author_pseud \
             WHERE ap.topic_id = forum_topics.id \
               AND ap.deleted_at IS NULL \
               AND LOWER(aps.handle) = LOWER(?))",
        )
        .with_bind(value)),
        QueryField::Title => {
            Ok(SqlFragment::new("LOWER(forum_topics.title) = LOWER(?)").with_bind(value))
        }
        QueryField::Kind => {
            // The result set is posts. `kind:topic` cannot be answered from it,
            // and returning the posts as though they were topics would be worse
            // than an error.
            if value.eq_ignore_ascii_case("post") {
                Ok(SqlFragment::new("1 = 1"))
            } else {
                Err(QueryError::new(
                    format!(
                        "kind accepts \"post\" on this search, not {value:?} -- \
                         a topic is returned as its newest post, not as its own row"
                    ),
                    0,
                ))
            }
        }
        QueryField::Pinned => Ok(flag(field, value)),
        QueryField::Locked => Ok(flag(field, value)),
        other => Err(foreign(other, "forum")),
    }
}

fn render_comparison(
    field: QueryField,
    op: CompareOp,
    value: &str,
) -> Result<SqlFragment, QueryError> {
    // Every comparable forum field is numeric except `active`, and the message
    // for a non-number is the same either way, so the check is made once here
    // rather than per arm.
    if field != QueryField::Active && value.parse::<i64>().is_err() {
        return Err(QueryError::new(
            format!("{} takes a whole number, got {value:?}", field.as_str()),
            0,
        ));
    }

    let sql = match field {
        // A correlated count, not a column: nothing maintains a denormalised
        // reply counter, and a subquery is correct on the first day rather
        // than correct until someone forgets to update the counter. Deleted
        // posts are not replies a reader can see.
        //
        // `CAST(? AS BIGINT)` is not decoration. The bound value is a `String`,
        // and SQLite's comparison affinity leaves a text value on the right of
        // an integer as text: `3 > '1'` is *false* in SQLite, so without the
        // cast every `replies:>N` returns nothing and reads as "this category
        // is empty". PostgreSQL coerces the same way from the other side, so the
        // cast is what makes one fragment correct on both backends.
        QueryField::Replies => format!(
            "(SELECT COUNT(*) FROM forum_posts r \
             WHERE r.topic_id = forum_topics.id AND r.deleted_at IS NULL) \
             {} CAST(? AS BIGINT)",
            op.as_str()
        ),
        // `last_post_at` is NULL for a topic nobody has replied to, and NULL
        // compares false against every operator -- so a brand-new thread would
        // vanish from `active:>2020-01-01`, which is exactly backwards. Fall
        // back to the topic's own creation.
        //
        // No `CAST` here: both sides are timestamps stored as ISO-8601 text, and
        // they order lexicographically. Casting a date to an integer would
        // compare `2026-01-15` as the number 2026 and silently return the wrong
        // threads.
        QueryField::Active => {
            if value.parse::<i64>().is_ok() {
                return Err(QueryError::new(
                    "active takes a date (after:2026-01-15), not a number",
                    0,
                ));
            }
            format!(
                "COALESCE(forum_topics.last_post_at, forum_topics.created_at) {} ?",
                op.as_str()
            )
        }
        other => return Err(foreign(other, "forum")),
    };
    Ok(SqlFragment::new(sql).with_bind(value))
}

/// A boolean field, written `field:true` / `field:false`.
fn flag(field: QueryField, value: &str) -> SqlFragment {
    let column = forum_column(field);
    let sql = match value.to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" => format!("{column} = 1"),
        // Anything else is false rather than an error: a reader who types
        // `locked:no` or `locked:false` means the same thing, and a search box
        // that rejects the third spelling of "no" is a search box people stop
        // using. `COALESCE(..., 0)` also makes a NULL row -- a pre-migration
        // topic, say -- read as false instead of dropping out.
        _ => format!("COALESCE({column}, 0) = 0"),
    };
    SqlFragment::new(sql)
}

fn forum_column(field: QueryField) -> &'static str {
    match field {
        QueryField::Pinned => "forum_topics.pinned",
        QueryField::Locked => "forum_topics.locked",
        // Unreachable: `flag` is only called for those two and every other
        // caller is rejected first. A panic rather than a default string so a
        // new boolean field cannot silently render as a column that does not
        // exist.
        other => unreachable!("{other:?} is not a forum boolean field"),
    }
}

/// The message a reader gets when a query names another surface's field.
///
/// Names both halves -- what is wrong, and what this surface does have -- so
/// the fix is obvious from the error alone rather than from a second attempt.
fn foreign(field: QueryField, surface: &str) -> QueryError {
    QueryError::new(
        format!(
            "{} is not a {surface} field ({surface} fields: category, author, \
             title, kind, replies, active, pinned, locked)",
            field.as_str()
        ),
        0,
    )
}
