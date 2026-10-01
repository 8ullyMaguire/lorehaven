//! The query-language surface for the character substrate (§15.4, M46-01).
//!
//! `character:` and `relationship:` are §15.4's initial fields, and both used to
//! render as a `work_tags` lookup for `kind = 'character'` / `kind = 'ship'`. Nothing
//! in the tree ever wrote such a tag — 0103 is the only migration that names
//! `kind = 'character'` at all — so both fields matched nothing. They now read the
//! tables that exist.
//!
//! The `relationship:` case needed measuring rather than reasoning, and the
//! measurements are recorded in the compiler's comments. In short: an "exact
//! participant set" match needs THREE clauses, and the third is the one that gets
//! missed, because with only the other two a poly request also matches a duo.
//!
//! These tests pin the *shape* of what the compiler emits. The end-to-end proof that
//! the SQL returns the right rows lives in `crates/app/tests/`, on both engines.
//!
//! | clause | test |
//! |---|---|
//! | `character:` reads work_characters | `character_reads_work_characters_not_work_tags` |
//! | `relationship:` reads relationships and participants | `relationship_reads_the_pairing_tables` |
//! | a duo request is not a poly request | `a_duo_request_binds_one_name_per_participant` |
//! | every named participant is checked | `every_named_participant_is_a_real_clause` |
//! | an empty pairing is refused | `an_empty_pairing_is_refused` |
//! | binds are ordered to match placeholders | `bind_order_matches_placeholder_order` |

use lorehaven_domain::query::{parse_query, QueryAst, QueryField};
use lorehaven_domain::query_sql::render_query;

/// Render `character:VALUE` and return its SQL and binds.
fn character_field(value: &str) -> (String, Vec<String>) {
    let ast = QueryAst::Fielded(QueryField::Character, value.to_owned());
    let fragment = render_query(&ast).expect("character field compiles");
    (fragment.sql.to_lowercase(), fragment.binds)
}

/// Render `relationship:VALUE` and return its SQL and binds.
fn relationship_field(value: &str) -> (String, Vec<String>) {
    let ast = QueryAst::Fielded(QueryField::Relationship, value.to_owned());
    let fragment = render_query(&ast).expect("relationship field compiles");
    (fragment.sql.to_lowercase(), fragment.binds)
}

#[test]
fn character_reads_work_characters_not_work_tags() {
    let (sql, binds) = character_field("Alice");

    assert!(
        sql.contains("work_characters"),
        "`character:` must read the table that holds characters. Got:\n{sql}"
    );
    assert!(
        !sql.contains("work_tags"),
        "the old rendering was a `work_tags` lookup for `kind='character'`, and \
         nothing writes such a tag -- so it matched nothing. Got:\n{sql}"
    );

    // The reader types a name; the store keys on a node id. Resolving `norm`
    // inside the subquery keeps the parser pure, which §15.4 requires.
    assert!(
        sql.contains("taxonomy_nodes"),
        "the name must be resolved to a node id in SQL. Got:\n{sql}"
    );
    assert!(
        sql.contains("kind = 'character'"),
        "and restricted to character nodes. Got:\n{sql}"
    );
    assert_eq!(
        binds,
        vec!["alice".to_string()],
        "the bind is the lowercased name"
    );

    // Name resolution is why this can stay a `Fielded` node rather than needing a
    // pre-parse lookup: the parse output for `character:Alice` is unchanged, which
    // is what "the parser produces the same typed AST as the visual filter builder"
    // (§15.4) actually requires.
    let parsed = parse_query("character:Alice").expect("parses");
    assert!(
        matches!(parsed, QueryAst::Fielded(QueryField::Character, ref v) if v == "Alice"),
        "the AST is unchanged by this work: {parsed:?}"
    );
}

#[test]
fn relationship_reads_the_pairing_tables() {
    let (sql, binds) = relationship_field("Alice/Bob");

    assert!(
        sql.contains("work_relationships"),
        "`relationship:` must read the work's claims. Got:\n{sql}"
    );
    assert!(
        sql.contains("ship_participants"),
        "and resolve participants through the set. Got:\n{sql}"
    );
    assert!(
        !sql.contains("work_tags"),
        "the old rendering was a tag lookup that nothing populated. Got:\n{sql}"
    );
    // No `rel_type` predicate: `relationship:Alice/Bob` names a pairing and says
    // nothing about its kind, so any type filter here would narrow the query the
    // reader did not write. Asking *about* the type is `ExistsRelationship`'s
    // `kind_any`, which is a different node.
    //
    // This is asserted because the alternative is a silent, always-false filter --
    // or worse, a default of `rel_type = 'romantic'` that would quietly hide every
    // platonic ship.
    assert!(
        !sql.contains("wr.rel_type"),
        "the pairing query must not filter on relationship type; the reader did not \
         ask for one. Got:\n{sql}"
    );
    assert!(!binds.is_empty(), "the value must be bound, not inlined");
}

#[test]
fn a_duo_request_binds_one_name_per_participant() {
    // The discriminator between a duo and a poly is that every name the reader
    // typed must be a real participant of the ship. With "Alice/Bob" that is two
    // name binds on top of the joined value.
    let (sql, binds) = relationship_field("Alice/Bob");

    assert_eq!(
        binds,
        vec![
            "/alice//bob/".to_string(),
            // The same value twice: `rewrite_placeholders` numbers every `?` in
            // order and has no back-reference, so a value used in two clauses must
            // be bound in both. Getting this wrong binds "bob" into clause (a).
            "/alice//bob/".to_string(),
            "alice".to_string(),
            "bob".to_string(),
        ],
        "the joined value twice (clauses a and b), then one bind per name (clause c)"
    );
    assert_eq!(
        sql.matches("sq.character_node_id = ?").count(),
        2,
        "two participants means two per-name EXISTS clauses. Got:\n{sql}"
    );

    // The prefix trap: `LIKE '%/alice/%'` must not match `alice_v2`. This is what
    // the `//` separator buys, so it is worth pinning.
    assert!(
        sql.contains("like '%/' ||"),
        "membership is tested on a slash-delimited pattern. Got:\n{sql}"
    );
}

#[test]
fn every_named_participant_is_a_real_clause() {
    // The third clause is the one that is easy to lose, so the test names it: a
    // poly request must check all three participants. Losing clause (c) makes a
    // duo satisfy a poly request, which is the bug this whole shape exists to avoid.
    let (sql, binds) = relationship_field("Alice/Bob/Carol");
    assert_eq!(
        sql.matches("sq.character_node_id = ?").count(),
        3,
        "three participants means three EXISTS clauses. Got:\n{sql}"
    );
    assert_eq!(binds.len(), 5, "the joined value twice plus three names");

    // The three clauses, named as the compiler's comment names them.
    assert!(
        sql.contains("and exists (select 1 from ship_participants sp"),
        "clause (a): every participant of the ship is named in the value. Got:\n{sql}"
    );
    assert!(
        sql.contains("and not exists (select 1 from ship_participants sm"),
        "clause (b): no participant of the ship is unnamed. Got:\n{sql}"
    );
    assert!(
        sql.contains("exists (select 1 from ship_participants sq"),
        "clause (c): every name in the value is a participant. Got:\n{sql}"
    );
}

#[test]
fn an_empty_pairing_is_refused() {
    // `relationship:` with no participant would satisfy clauses (a) and (b)
    // vacuously for any ship with any relationship -- which is every work that has
    // one. Refusing is the honest rendering, and matches the same rule the
    // `ExistsRelationship` arm enforces.
    for value in ["", "/", "//"] {
        let ast = QueryAst::Fielded(QueryField::Relationship, value.to_owned());
        let err = render_query(&ast).expect_err("must refuse");
        assert!(
            err.message.contains("participant"),
            "the error must say what is missing for {value:?}; got: {}",
            err.message
        );
    }

    // And a real pairing still compiles, so the refusal is not just "this field is
    // broken".
    assert!(relationship_field("Alice").1.len() >= 2);
}

#[test]
fn bind_order_matches_placeholder_order() {
    // `Database::sql` rewrites bare `?` to `$n` positionally, so a mismatch here
    // binds the wrong value to the wrong clause -- silently on SQLite, loudly on
    // PostgreSQL only if the types differ. Asserted on a three-name pairing, where
    // an off-by-one is most visible.
    let (sql, binds) = relationship_field("Alice/Bob/Carol");

    let placeholder_count = sql.matches('?').count();
    assert_eq!(
        placeholder_count,
        binds.len(),
        "one bind per placeholder, or a value lands in the wrong clause. \
         sql has {placeholder_count} placeholders and {} binds. Got:\n{sql}",
        binds.len()
    );

    // The first two placeholders are the joined value (used twice), so there are
    // 2 + n placeholders for n names.
    assert_eq!(placeholder_count, 5, "two value binds plus three names");
}

#[test]
fn a_name_is_not_inlined_into_sql() {
    // "Never pass user query text directly as SQL" (§15.4). Both fields must bind,
    // and neither may splice the reader's text into the statement.
    for (sql, binds) in [character_field("Alice"), relationship_field("Alice/Bob")] {
        assert!(
            !sql.contains("alice") && !sql.contains("bob"),
            "reader text must not appear in the SQL. Got:\n{sql}"
        );
        assert!(
            !binds.is_empty(),
            "reader text must be bound. Got binds: {binds:?}"
        );
    }
}
