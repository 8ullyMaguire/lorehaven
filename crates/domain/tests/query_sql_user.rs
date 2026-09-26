//! SQL rendering for the user surface.
//!
//! The user search returns *pseudonyms*, not accounts, and the difference
//! decides almost every mapping: a pseudonym is what appears next to a post and
//! what a reader types, and an account is what owns the works behind every
//! pseudonym. So the result set and every fielded filter are rooted at `pseuds`.
//!
//! `fandoms:"Good Omens"` is the interesting one. It means "a pseudonym
//! who has written *in* this fandom", and nothing denormalises that — it is a
//! correlated EXISTS over `works` → `work_tags` → `taxonomy_nodes`. The alias
//! table matters: a reader who types a variant spelling must still find the
//! same author, and matching only `canonical` would make the filter silently
//! miss every pseudonym whose fandom is spelled as an alias.

use lorehaven_domain::query::parse_query;
use lorehaven_domain::query_sql_user::render_user_query;

fn render(q: &str) -> (String, Vec<String>) {
    let ast = parse_query(q).unwrap_or_else(|e| panic!("parse {q:?}: {}", e.message));
    let frag = render_user_query(&ast).unwrap_or_else(|e| panic!("render {q:?}: {}", e.message));
    (frag.sql, frag.binds)
}

fn render_err(q: &str) -> String {
    let problem = match parse_query(q) {
        Ok(ast) => render_user_query(&ast)
            .err()
            .map(|e| e.message)
            .unwrap_or_default(),
        Err(e) => e.message,
    };
    assert!(
        !problem.is_empty(),
        "{q:?} should have been refused, but it rendered"
    );
    problem
}

#[test]
fn free_text_matches_the_handle_and_the_display_name() {
    // Not the bio. A reader searching for a person wants the person, and a bio
    // is prose that would match on any word in it.
    let (sql, binds) = render("nightowl");
    assert!(sql.contains("pseuds.handle"), "{sql}");
    assert!(sql.contains("pseuds.display_name"), "{sql}");
    assert!(!sql.contains("pseuds.bio"), "the bio is not a name: {sql}");
    assert_eq!(
        binds,
        vec!["%nightowl%".to_string(), "%nightowl%".to_string()]
    );
}

#[test]
fn the_user_field_is_the_handle_not_the_email() {
    // A reader searching `nightowl` means the name next to a post. Matching an
    // email address would fail (they do not know it) and, if it ever matched,
    // hand an address to anyone who guessed one.
    let (sql, binds) = render("user:nightowl");
    assert!(sql.contains("pseuds.handle"), "{sql}");
    assert!(!sql.contains("accounts.email"), "{sql}");
    assert_eq!(binds, vec!["nightowl".to_string()]);
}

#[test]
fn a_fandom_is_resolved_through_aliases_as_well_as_the_canonical_spelling() {
    let (sql, binds) = render("fandoms:\"Good Omens\"");
    assert!(
        sql.contains("work_tags"),
        "the link from a work to its tags: {sql}"
    );
    assert!(sql.contains("taxonomy_nodes"), "{sql}");
    // The alias arm is the part that is easy to forget and impossible to
    // notice: without it a reader who types the variant spelling gets an empty
    // page and concludes nobody writes in that fandom.
    assert!(
        sql.contains("taxonomy_aliases"),
        "a fandom spelled as an alias must still match: {sql}"
    );
    // Bound twice: the canonical arm and the alias arm each need the value,
    // and they are the same string -- which is why the statement is built once
    // and shared by both dialects.
    assert_eq!(
        binds,
        vec!["Good Omens".to_string(), "Good Omens".to_string()]
    );
}

#[test]
fn a_fandom_filter_ignores_a_deleted_work() {
    let (sql, _) = render("fandoms:\"Good Omens\"");
    // A deleted work is not something a reader can find, so it must not make an
    // author look like they write in a fandom they no longer do.
    assert!(sql.contains("deleted_at IS NULL"), "{sql}");
}

#[test]
fn a_work_count_casts_its_bound() {
    // The regression this guards is silent and total: SQLite leaves a text
    // bind on the right of an integer as text, so `3 > '1'` is false and every
    // `works:>N` matches nothing -- which looks exactly like an instance with
    // no authors.
    let (sql, _) = render("works:>10");
    assert!(sql.contains("CAST(? AS BIGINT)"), "{sql}");
}

#[test]
fn a_work_count_only_counts_published_and_public_works() {
    let (sql, _) = render("works:>10");
    // A draft is invisible in the listing and a withdrawn work is gone, so
    // counting either would rank a dormant account above an active one.
    assert!(
        sql.contains("lifecycle = 'published'"),
        "a draft is not a published work: {sql}"
    );
    assert!(
        sql.contains("visibility = 'public'"),
        "a private work is not public output: {sql}"
    );
    assert!(sql.contains("deleted_at IS NULL"), "{sql}");
}

#[test]
fn each_work_count_operator_renders_its_own_sql() {
    // If `>` and `>=` rendered the same, the boundary would be wrong in a way
    // that looks plausible.
    let (gt, _) = render("works:>10");
    let (gte, _) = render("works:>=10");
    let (lt, _) = render("works:<10");
    let (lte, _) = render("works:<=10");
    assert!(
        gt.contains("> CAST(? AS BIGINT)") && !gt.contains(">= "),
        "{gt}"
    );
    assert!(gte.contains(">= CAST(? AS BIGINT)"), "{gte}");
    assert!(lt.contains("< CAST(? AS BIGINT)"), "{lt}");
    assert!(lte.contains("<= CAST(? AS BIGINT)"), "{lte}");
}

#[test]
fn a_work_count_range_is_two_bounds() {
    let (sql, binds) = render("works:2..20");
    assert!(sql.contains(">= CAST(? AS BIGINT)"), "{sql}");
    assert!(sql.contains("<= CAST(? AS BIGINT)"), "{sql}");
    assert_eq!(binds, vec!["2".to_string(), "20".to_string()]);
}

#[test]
fn a_join_date_is_not_cast() {
    // The opposite failure. Timestamps are ISO-8601 text in both schemas and
    // order lexicographically; `CAST('2026-01-15' AS BIGINT)` compares the date
    // as the number 2026 and quietly returns the wrong rows.
    let (sql, _) = render("joined:>2026-01-15");
    assert!(!sql.contains("CAST"), "a date bound must stay text: {sql}");
    assert!(sql.contains("created_at"), "{sql}");
}

#[test]
fn a_join_date_falls_back_to_the_pseudonym() {
    let (sql, _) = render("joined:>2026-01-15");
    // A pseudonym added to an existing account has no account creation date of
    // its own, and NULL compares false against every operator -- so without the
    // fallback every pseudonym on a year-old account vanishes from
    // `joined:>2020-01-01`, which is exactly backwards.
    assert!(
        sql.contains("COALESCE(accounts.created_at, pseuds.created_at)"),
        "{sql}"
    );
}

#[test]
fn a_work_count_that_is_not_a_number_is_rejected() {
    // By the parser, not the renderer: the value has to be an integer before a
    // node is built. The renderer re-checks anyway, because
    // `render_user_query` is public and an AST can be built by hand.
    let msg = render_err("works:>lots");
    assert!(msg.contains("integer"), "{msg}");
}

#[test]
fn a_field_from_another_surface_names_the_surface_that_has_it() {
    // "No results" is indistinguishable from "nobody matches", and the reader has
    // no way to learn they used the wrong box.
    for (query, field) in [
        ("words:>10000", "words"),
        ("replies:>50", "replies"),
        ("category:meta", "category"),
    ] {
        let msg = render_err(query);
        assert!(
            msg.contains(field),
            "the message must name the field: {msg}"
        );
        assert!(msg.contains("user"), "and the surface: {msg}");
    }
}

#[test]
fn the_fandom_field_is_also_spelled_the_way_a_reader_types_it() {
    // `user_fandom` is the registry's name; `fandoms:` is what a reader writes,
    // and it is the plural because a pseudonym writes in several. Both have to
    // land on the same field, or one of the two spellings silently becomes
    // free text -- and `fandoms:"Good Omens"` becomes a search for the literal
    // word "fandoms" rather than an error.
    //
    // Note the field is unprefixed: `fandoms:` is already a user-only field, so
    // `user:fandoms:x` would be a two-field form the parser has no syntax for.
    // It parses as `user:fandoms` plus a phrase, and the surface check turns
    // the stray `user` arm into the error naming `fandoms` -- which is the
    // outcome a reader needs.
    for query in ["fandoms:\"Good Omens\"", "user_fandom:\"Good Omens\""] {
        let (sql, binds) = render(query);
        assert!(sql.contains("work_tags"), "{query} -> {sql}");
        assert_eq!(
            binds,
            vec!["Good Omens".to_string(), "Good Omens".to_string()]
        );
    }
}

#[test]
fn a_like_wildcard_in_the_query_is_escaped() {
    // `100%_complete` means those two characters literally. Without escaping,
    // `%` matches any run and `_` any single character, so the query returns far
    // more than the reader asked for -- silently, because it looks right.
    let (_, binds) = render("100%_complete");
    assert_eq!(
        binds[0], "%100\\%\\_complete%",
        "the reader's wildcards must be literal"
    );
}

#[test]
fn a_negated_term_is_null_safe() {
    // `NOT nightowl` on a pseudonym with a NULL display name: without the
    // COALESCE the whole expression is NULL, and `NOT NULL` is NULL, so the row
    // is dropped from a result it belongs in.
    let (sql, _) = render("NOT nightowl");
    assert!(
        sql.to_uppercase().contains("NOT COALESCE("),
        "a negated LIKE must be null-safe: {sql}"
    );
}

#[test]
fn a_like_pattern_declares_its_escape_character() {
    // Without an `ESCAPE` clause the two backends disagree about what a
    // backslash means: PostgreSQL treats it as the default LIKE escape, SQLite
    // has no default at all. So `escape_like` -- which escapes every `%`, `_`
    // and `\` -- produces a pattern that means one thing on one backend and
    // something else on the other. A reader searching `100%` finds the right
    // rows on PostgreSQL and none on SQLite, and neither is obviously wrong.
    let (sql, binds) = render("100%");
    assert!(
        sql.contains("ESCAPE"),
        "the escape character must be declared, or the dialects disagree: {sql}"
    );
    // Twice, because the free-text arm matches the handle and the display name
    // with the same pattern.
    assert_eq!(binds, vec!["%100\\%%".to_string(), "%100\\%%".to_string()]);
    // The emitted clause, byte for byte. This is not pedantry: the SQL quote
    // sits right after the backslash, so a single backslash in the Rust source
    // is read as an escaped apostrophe and vanishes -- emitting `ESCAPE ''`,
    // which SQLite rejects ("ESCAPE expression must be a single character")
    // while PostgreSQL accepts it as an empty string. One backend errors, the
    // other matches every row, and the source reads correctly either way.
    assert!(
        sql.contains("ESCAPE '") && sql.contains("'\\'"),
        "the backslash must survive: {sql}"
    );
    assert!(
        !sql.contains("ESCAPE ''"),
        "an empty escape means every row matches: {sql}"
    );
}

#[test]
fn a_bound_value_cannot_reach_the_sql() {
    // An injection attempt that survives to be bound. It does not get as far as
    // a valid integer, so the parser refuses it -- which is the stronger
    // outcome: the value never reaches SQL *or* the query.
    let (sql, binds) = render("user:o'brien;--");
    assert!(!sql.contains("o'brien"), "the value reached the SQL: {sql}");
    assert_eq!(binds, vec!["o'brien;--".to_string()]);
}
