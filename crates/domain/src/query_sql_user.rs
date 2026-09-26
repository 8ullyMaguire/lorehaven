//! SQL rendering for the user surface.
//!
//! The user search returns *pseudonyms*, not accounts, and the difference
//! decides almost every mapping: a pseudonym is what appears next to a post and
//! what a reader types, and an account is what owns the works behind every
//! pseudonym. So the result set and every fielded filter are rooted at
//! `pseuds`.
//!
//! Only `user`, `fandoms`, `works` and `joined` are rendered here. The other
//! user fields the registry knows about are refused loudly rather than mapped
//! to something approximately right — see `wrong_surface`.

use crate::query::{CompareOp, QueryAst, QueryError, QueryField};
use crate::query_sql::SqlFragment;

/// Render a user-surface query.
pub fn render_user_query(ast: &QueryAst) -> Result<SqlFragment, QueryError> {
    render_node(ast)
}

fn render_node(ast: &QueryAst) -> Result<SqlFragment, QueryError> {
    match ast {
        // Free text matches the handle and the display name. Not the bio: a
        // reader searching for a person wants the person, and a bio is prose
        // that would match on any word in it and return a long tail of
        // strangers.
        QueryAst::Text(text) => {
            let sql = "(COALESCE(LOWER(pseuds.handle), '') LIKE LOWER(?) \
                       OR COALESCE(LOWER(pseuds.display_name), '') LIKE LOWER(?))";
            let pattern = format!("%{}%", escape_like(text));
            Ok(SqlFragment::new(sql)
                .with_bind(pattern.clone())
                .with_bind(pattern))
        }
        QueryAst::Phrase(phrase) => Ok(SqlFragment::new(
            "COALESCE(LOWER(pseuds.display_name), '') LIKE LOWER(?)",
        )
        .with_bind(format!("%{}%", escape_like(phrase)))),
        QueryAst::Fielded(field, value) => render_fielded(*field, value),
        QueryAst::Comparison(field, op, value) => render_comparison(*field, *op, value),
        QueryAst::And(parts) => join(parts, " AND "),
        QueryAst::Or(parts) => join(parts, " OR "),
        QueryAst::Not(inner) => {
            let inner = render_node(inner)?;
            // Null-safe for the same reason as the other surfaces: a NULL arm
            // makes the whole expression NULL, and `NOT NULL` is NULL, so the
            // row drops out of a `NOT` that it should have been inside.
            let sql = if needs_null_guard(&inner.sql) {
                format!("NOT COALESCE({}, FALSE)", inner.sql)
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

fn join(parts: &[QueryAst], op: &str) -> Result<SqlFragment, QueryError> {
    if parts.is_empty() {
        return Err(QueryError::new("empty expression", 0));
    }
    let mut sql = Vec::with_capacity(parts.len());
    let mut binds = Vec::new();
    for part in parts {
        let f = render_node(part)?;
        sql.push(f.sql);
        binds.extend(f.binds);
    }
    Ok(SqlFragment {
        sql: format!("({})", sql.join(op)),
        binds,
    })
}

/// A redundant `COALESCE` costs a little; a missed one drops every row with a
/// NULL out of a `NOT` it belongs in. So this errs toward guarding.
fn needs_null_guard(sql: &str) -> bool {
    sql.contains(" LIKE ")
}

fn escape_like(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn render_fielded(field: QueryField, value: &str) -> Result<SqlFragment, QueryError> {
    match field {
        // The pseudonym, not the account. A reader searching `nightowl` means
        // the name next to a post; matching an email would both fail (they do
        // not know it) and, if it ever matched, hand an address to anyone who
        // guessed one.
        QueryField::User => {
            Ok(SqlFragment::new("LOWER(pseuds.handle) = LOWER(?)").with_bind(value.to_owned()))
        }
        // `user:fandoms:"Good Omens"` — a pseudonym who has written *in* this
        // fandom. A correlated EXISTS, not a column: nothing denormalises a
        // pseudonym's fandoms, and a column would be wrong the first time
        // somebody posted under a new one.
        //
        // The alias arm is the part that is easy to forget and impossible to
        // notice. `taxonomy_nodes.canonical` holds the spelling the instance
        // chose; `taxonomy_aliases` holds every variant a reader might type.
        // Matching only the canonical form means a reader who types the variant
        // gets an empty page and concludes nobody writes in that fandom.
        //
        // `norm` is the normalised key on both tables, so the comparison is
        // exact rather than a LIKE -- a fandom called "Good Omens" must not
        // also match "Good Omenshole".
        QueryField::UserFandom => Ok(SqlFragment::new(
            "EXISTS (SELECT 1 FROM works uf_w \
               JOIN work_tags uf_t ON uf_t.work_id = uf_w.id \
               JOIN taxonomy_nodes uf_n ON uf_n.id = uf_t.node_id \
              WHERE uf_w.owner_pseud_id = pseuds.id \
                AND uf_w.deleted_at IS NULL \
                AND (LOWER(uf_n.norm) = LOWER(?) \
                     OR EXISTS (SELECT 1 FROM taxonomy_aliases uf_a \
                                 WHERE uf_a.node_id = uf_n.id \
                                   AND LOWER(uf_a.norm) = LOWER(?))))",
        )
        .with_bind(value.to_owned())
        .with_bind(value.to_owned())),
        other => Err(wrong_surface(other, "user")),
    }
}

fn render_comparison(
    field: QueryField,
    op: CompareOp,
    value: &str,
) -> Result<SqlFragment, QueryError> {
    // The value is checked here as well as in the parser, because this
    // renderer is public and an AST can be built by hand.
    if field != QueryField::Joined && value.parse::<i64>().is_err() {
        return Err(QueryError::new(
            format!("{} takes a whole number, got {value:?}", field.as_str()),
            0,
        ));
    }

    let sql = match field {
        // A count of live works, correlated -- nothing maintains a denormalised
        // per-pseudonym counter, and a subquery is correct on the first day
        // rather than correct until someone forgets to update the counter.
        //
        // `lifecycle` and `visibility` are both checked, not just
        // `deleted_at`: a draft is invisible in the listing and a private work
        // is nobody's to read, so counting either would rank a dormant account
        // above an active one and tell a reader they had written more than
        // anyone can see.
        //
        // `CAST(? AS BIGINT)` is not decoration. The bound arrives as a
        // `String` and SQLite leaves it as text on the right of an integer, so
        // `3 > '1'` is *false* and every `works:>N` matches nothing -- which
        // reads exactly like an instance with no authors.
        QueryField::Works => format!(
            "(SELECT COUNT(*) FROM works uw \
              WHERE uw.owner_pseud_id = pseuds.id \
                AND uw.deleted_at IS NULL \
                AND uw.lifecycle = 'published' \
                AND uw.visibility = 'public') {} CAST(? AS BIGINT)",
            op.as_str()
        ),
        // A pseudonym added to an existing account has no account creation date
        // of its own, and NULL compares false against every operator -- so
        // without the fallback every pseudonym on a year-old account vanishes
        // from `joined:>2020-01-01`, which is exactly backwards.
        //
        // No `CAST`: timestamps are ISO-8601 text in both schemas and order
        // lexicographically. Casting a date to an integer would compare
        // `2026-01-15` as the number 2026 and silently return the wrong rows.
        QueryField::Joined => {
            format!(
                "COALESCE(accounts.created_at, pseuds.created_at) {} ?",
                op.as_str()
            )
        }
        other => return Err(wrong_surface(other, "user")),
    };
    Ok(SqlFragment::new(&sql).with_bind(value.to_owned()))
}

/// The message a reader gets when a query names another surface's field.
///
/// Names both halves — what is wrong, and what this surface does have — so the
/// fix is obvious from the error alone rather than from a second attempt.
fn wrong_surface(field: QueryField, surface: &str) -> QueryError {
    QueryError::new(
        format!(
            "{} is not a {surface} field ({surface} fields: user, fandoms, works, joined)",
            field.as_str()
        ),
        0,
    )
}
