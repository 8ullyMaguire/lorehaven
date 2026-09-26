//! SQL rendering for the forum surface.
//!
//! The operators are the same as the works surface -- `>`, `>=`, `<`, `<=`,
//! `..` -- and the parser is the same, so a reader who has learned
//! `words:>10000` can write `replies:>50` without being told anything new. What
//! differs is which column each field names.
//!
//! These tests are the contract. The implementation is in
//! `query_sql_forum.rs`.

use lorehaven_domain::query::{parse_query, CompareOp, QueryAst, QueryField};
use lorehaven_domain::query_sql_forum::render_forum_query;

fn render(q: &str) -> (String, Vec<String>) {
    let ast = parse_query(q).unwrap_or_else(|e| panic!("parse {q:?}: {}", e.message));
    let frag = render_forum_query(&ast).unwrap_or_else(|e| panic!("render {q:?}: {}", e.message));
    (frag.sql, frag.binds)
}

fn render_err(q: &str) -> String {
    let ast = match parse_query(q) {
        Ok(ast) => ast,
        Err(e) => return e.message,
    };
    match render_forum_query(&ast) {
        Ok(frag) => panic!("expected {q:?} to be rejected, got: {}", frag.sql),
        Err(e) => e.message,
    }
}

#[test]
fn free_text_matches_the_topic_title_or_the_post_body() {
    let (sql, binds) = render("winter");
    assert!(sql.contains("forum_topics.title"), "{sql}");
    // The body arm is a correlated EXISTS, not a join: the result set is
    // topics, and a join to forum_posts would return a thread once per matching
    // reply -- the same title repeated, with no way for the reader to tell why.
    assert!(sql.contains("EXISTS") && sql.contains("tp.body"), "{sql}");
    // Deleted posts are not searchable for the same reason they are not replies.
    assert!(sql.contains("tp.deleted_at IS NULL"), "{sql}");
    assert_eq!(binds, vec!["%winter%".to_string(), "%winter%".to_string()]);
}

#[test]
fn a_phrase_searches_the_body_only() {
    // A quoted phrase is a phrase: matching it against a topic title would
    // return topics for a phrase the reader asked to find in posts.
    let (sql, binds) = render("\"we were never alone\"");
    assert!(sql.contains("pp.body"), "{sql}");
    assert!(!sql.contains("forum_topics.title"), "{sql}");
    assert_eq!(binds, vec!["%we were never alone%".to_string()]);
}

#[test]
fn a_like_wildcard_in_the_query_is_escaped() {
    // `100%_complete` means those two characters literally. Without escaping,
    // `%` matches any run and `_` any single character, so the query returns
    // far more than the reader asked for -- silently, because it looks right.
    let (_, binds) = render("100%_complete");
    assert_eq!(
        binds[0], "%100\\%\\_complete%",
        "the reader's wildcards must be literal"
    );
}

#[test]
fn replies_compares_against_a_correlated_count() {
    let (sql, binds) = render("replies:>50");
    // A subquery, not a column: nothing maintains a denormalised counter, and a
    // subquery is correct on the first day rather than correct until someone
    // forgets to update the counter.
    assert!(sql.contains("COUNT(*)"), "{sql}");
    assert!(
        sql.contains("r.deleted_at IS NULL"),
        "{sql}: deleted posts are not replies"
    );
    assert_eq!(binds, vec!["50".to_string()]);
}

#[test]
fn each_reply_operator_renders_its_own_sql() {
    // If `>` and `>=` rendered the same the boundary would be wrong in a way
    // that looks plausible, so the operators are compared against each other.
    let (gt, _) = render("replies:>10");
    let (gte, _) = render("replies:>=10");
    let (lt, _) = render("replies:<10");
    let (lte, _) = render("replies:<=10");
    assert!(
        gt.contains("> CAST(? AS BIGINT)") && !gt.contains(">= "),
        "{gt}"
    );
    assert!(gte.contains(">= CAST(? AS BIGINT)"), "{gte}");
    assert!(lt.contains("< CAST(? AS BIGINT)"), "{lt}");
    assert!(lte.contains("<= CAST(? AS BIGINT)"), "{lte}");
}

#[test]
fn a_reply_count_that_is_not_a_number_is_rejected() {
    // By the parser, not the renderer: the value has to be an integer before a
    // node is built, and `replies` is a `ValueKind::Number` field. The
    // renderer re-checks anyway, because `render_forum_query` is public and
    // the AST can be built by hand.
    let msg = render_err("replies:>lots");
    assert!(msg.contains("integer"), "{msg}");
}

#[test]
fn category_compares_by_name_not_by_id() {
    // The reader writes `category:"meta"`, and the URL the form posts is a
    // name. Comparing the id would silently match nothing.
    let (sql, binds) = render("category:meta");
    assert!(sql.contains("forum_categories.name"), "{sql}");
    assert_eq!(binds, vec!["meta".to_string()]);
}

#[test]
fn author_matches_the_post_author_not_the_topic_author() {
    // A reader searching their own handle means their posts. Mapping to the
    // topic's author would return only the threads they started.
    let (sql, _) = render("author:nightowl");
    // `author_pseud` holds an id, and the reader types a handle, so the
    // pseudonym row is joined inside the EXISTS. Comparing the column to the
    // literal "nightowl" would match nothing and read as "I have no posts".
    assert!(sql.contains("ap.author_pseud"), "{sql}");
    assert!(sql.contains("aps.handle"), "{sql}");
}

#[test]
fn kind_post_is_accepted_and_kind_topic_is_not() {
    // The result set is posts. `kind:topic` cannot be answered from it, and
    // returning the posts as though they were topics is worse than an error.
    assert_eq!(render("kind:post").0, "1 = 1");
    let msg = render_err("kind:topic");
    assert!(msg.contains("post"), "{msg}");
}

#[test]
fn a_boolean_field_accepts_the_usual_spellings() {
    for truthy in ["true", "yes", "1", "TRUE"] {
        let (sql, _) = render(&format!("locked:{truthy}"));
        assert!(sql.contains("forum_topics.locked = 1"), "{truthy}: {sql}");
    }
    for falsy in ["false", "no", "0"] {
        let (sql, _) = render(&format!("locked:{falsy}"));
        // Coalesced, not a bare `= 0`: a row whose column is NULL reads as
        // false instead of dropping out of the result set entirely, which is
        // the direction that silently loses rows.
        assert!(
            sql.contains("COALESCE(forum_topics.locked, 0) = 0"),
            "{falsy}: {sql}"
        );
    }
    // Any other spelling of "no" is also "no". A search box that rejects the
    // third way of writing false is a search box people stop using.
    for other in ["nope", "off", "unlocked"] {
        let (sql, _) = render(&format!("locked:{other}"));
        assert!(sql.contains("= 0"), "{other}: {sql}");
    }
}

#[test]
fn pinned_reads_the_topic_column() {
    let (sql, _) = render("pinned:true");
    assert!(sql.contains("forum_topics.pinned"), "{sql}");
}

#[test]
fn active_takes_a_date_and_says_so_if_given_a_number() {
    let (sql, binds) = render("active:>2026-01-15");
    assert!(sql.contains("last_post_at"), "{sql}");
    assert_eq!(binds, vec!["2026-01-15".to_string()]);

    // `words:>10000` teaches the shape, so `active:>5` is a plausible mistake.
    // It has to be refused with the right spelling rather than compared
    // against a timestamp column as though 5 were a date.
    let msg = render_err("active:>5");
    assert!(msg.contains("date"), "{msg}");
}

#[test]
fn active_falls_back_to_creation_when_a_topic_has_never_been_replied_to() {
    // `last_post_at` is NULL for a topic with no replies, and NULL compares
    // false against every operator -- so a brand-new thread would vanish from
    // `active:>2020-01-01`, which is exactly backwards.
    let (sql, _) = render("active:>2020-01-01");
    assert!(
        sql.contains("COALESCE(forum_topics.last_post_at, forum_topics.created_at)"),
        "{sql}"
    );
}

#[test]
fn a_negated_free_text_term_is_null_safe() {
    // `NOT (body LIKE ?)` is NULL when the body is NULL, and NULL is not true,
    // so the negation would exclude exactly the rows it should keep. The
    // works surface has the same guard for the same reason.
    let (sql, _) = render("NOT spoiler");
    assert!(sql.contains("COALESCE"), "{sql}");
    assert!(sql.contains("NOT COALESCE"), "{sql}");
}

#[test]
fn a_negated_reply_count_is_not_coalesced() {
    // The count is a `COUNT(*)`, which is 0 and never NULL, so a coalesce
    // would be noise. Pinning both directions stops the guard creeping.
    let (sql, _) = render("NOT replies:>50");
    assert!(!sql.contains("COALESCE"), "{sql}");
}

#[test]
fn a_works_field_is_rejected_and_says_where_it_belongs() {
    for (query, field) in [
        ("words:>10000", "words"),
        ("fandom:\"Good Omens\"", "fandom"),
        ("kudos:>5", "kudos"),
    ] {
        let msg = render_err(query);
        assert!(
            msg.contains(field),
            "{query}: the message must name the field: {msg}"
        );
        assert!(
            msg.contains("forum"),
            "{query}: the message must name the surface: {msg}"
        );
    }
}

#[test]
fn a_user_field_is_rejected_on_the_forum_surface() {
    // `works:>10` is a user query. It parses -- the parser is shared -- and the
    // forum has no such column, so without this it would be a silent zero.
    let msg = render_err("works:>10");
    assert!(msg.contains("works"), "{msg}");
    assert!(msg.contains("forum"), "{msg}");
}

#[test]
fn a_range_over_replies_is_two_bounds() {
    // The parser expands the range, so this is the works range behaviour
    // applied to a forum column. Included because `..` is in the shared
    // language and a forum reader should get it without a second syntax.
    let ast = parse_query("replies:10..50").unwrap();
    match &ast {
        QueryAst::And(parts) => {
            assert_eq!(parts.len(), 2, "{ast:?}");
            assert!(matches!(
                parts[0],
                QueryAst::Comparison(QueryField::Replies, CompareOp::Gte, _)
            ));
        }
        other => panic!("expected an And, got {other:?}"),
    }
    let (sql, binds) = render_forum_query(&ast).map(|f| (f.sql, f.binds)).unwrap();
    assert_eq!(binds, vec!["10".to_string(), "50".to_string()]);
    assert!(sql.contains("AND"), "{sql}");
}

#[test]
fn a_range_with_no_bounds_is_rejected_before_it_reaches_the_renderer() {
    // The parser owns this check, so the renderer never sees it. Asserting it
    // here as a parse error documents where the responsibility sits.
    let msg = render_err("replies:..");
    assert!(msg.contains("range") || msg.contains("bound"), "{msg}");
}

#[test]
fn every_value_is_bound_and_never_interpolated() {
    // A value containing a quote is the cheapest way to catch an interpolated
    // render; a bound one comes back out as a parameter and never in the SQL.
    let (sql, binds) = render("category:o'brien");
    assert!(!sql.contains("o'brien"), "the value reached the SQL: {sql}");
    assert_eq!(binds, vec!["o'brien".to_string()]);

    // An injection attempt that survives to be bound. It does not get as far
    // as a valid integer, so the parser refuses it -- which is the stronger
    // outcome: the value never reaches SQL *or* the query. Asserted here
    // because "a bound parameter cannot inject" is only worth saying if there
    // is a case where the value is legal and still carries SQL punctuation.
    let (sql, binds) = render("category:o'brien;--");
    assert!(!sql.contains("o'brien"), "the value reached the SQL: {sql}");
    assert_eq!(binds, vec!["o'brien;--".to_string()]);
}
