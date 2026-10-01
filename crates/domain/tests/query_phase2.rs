//! §15.4.1: scoped sub-predicates, `min_match`, expansion, and the complexity
//! budget. New in M46-05.
//!
//! The load-bearing test here is
//! `a_scoped_predicate_is_one_exists_not_two`. §15.4.1.1 exists because §15.3's rule
//! — never let one character satisfy another character's attributes — was not
//! reachable from the text surface, and a test that only checked the *right* SQL
//! would pass against an uncorrelated compilation that happens to contain the right
//! column names. So the wrong shape is asserted alongside the right one, and they
//! must differ.

use lorehaven_domain::query::{
    parse_query, parse_query_with, QueryAst, QueryBudget, QueryError, ScopedField, ScopedPredicate,
};
use lorehaven_domain::query_sql::render_query;

fn ast(input: &str) -> QueryAst {
    parse_query(input).unwrap_or_else(|e| panic!("{input:?} should parse, got {e:?}"))
}

fn sql(input: &str) -> String {
    render_query(&ast(input))
        .unwrap_or_else(|e| panic!("{input:?} should render, got {e:?}"))
        .sql
}

fn err(input: &str) -> QueryError {
    parse_query(input).expect_err(&format!("{input:?} should be refused"))
}

// ---------------------------------------------------------------- scoped predicates

#[test]
fn a_scoped_predicate_is_one_exists_not_two() {
    let s = sql(r#"relationship:(with:"Alice" with:"Bob" type:romantic)"#);

    // ONE outer EXISTS over work_relationships. If this ever reads 2, the bounds
    // have been split into sibling EXISTS clauses and the correlation is gone --
    // which is precisely the bug §15.4.1.1 was written to prevent.
    assert_eq!(
        s.matches("SELECT 1 FROM work_relationships wr WHERE wr.work_id = works.id")
            .count(),
        1,
        "the bounds must be correlated in ONE EXISTS, not one per bound: {s}"
    );
    // And the type bound must be INSIDE that EXISTS, not a sibling of it.
    let exists_at = s.find("work_relationships").expect("the relation is named");
    let rel_type_at = s
        .find("wr.rel_type = ?")
        .expect("the type bound is present");
    assert!(
        rel_type_at > exists_at,
        "wr.rel_type must appear after the relation is opened, so it is inside the \
         same EXISTS: {s}"
    );
    // Every clause lives in one pair of parentheses, i.e. one predicate.
    assert_eq!(
        s.matches("SELECT 1 FROM ship_participants").count(),
        4,
        "three participant clauses (all-named, no-unnamed, each-named) means one \
         more than the two the uncorrelated form would need -- assert the count so a \
         dropped clause is visible: {s}"
    );
}

#[test]
fn a_scoped_predicate_binds_every_named_participant() {
    let f = render_query(&ast(
        r#"relationship:(with:"Alice" with:"Bob" with:"Carol" type:romantic)"#,
    ))
    .expect("renders");
    // The third participant gets its own equality bind, which is clause (c). Without
    // it, asking for a poly would also match a duo -- the M46-01 measurement.
    assert_eq!(
        f.binds.iter().filter(|b| b.as_str() == "Carol").count(),
        1,
        "every named participant must be checked for membership: {:?}",
        f.binds
    );
    assert!(f.binds.iter().any(|b| b.as_str() == "romantic"));
}

#[test]
fn the_delimiters_survive_so_a_name_cannot_match_a_longer_one() {
    let f = render_query(&ast(r#"relationship:(with:"Alice")"#)).expect("renders");
    // `//` on both sides, so `LIKE '%/alice/%'` is false for `alice_v2` and true for
    // `alice`. A single `/` would make the trailing participant fail its own test.
    assert!(
        f.binds.iter().any(|b| b == "/Alice//"),
        "the joined value must be delimited on both sides: {:?}",
        f.binds
    );
}

#[test]
fn a_scoped_predicate_needs_a_participant() {
    let e = err("relationship:(type:romantic)");
    assert!(
        e.message.contains("with:"),
        "a scoped relationship with no `with:` would match every relationship in the \
         instance, so the error must name what is missing: {}",
        e.message
    );
}

#[test]
fn an_unknown_scoped_key_is_refused_by_name() {
    let e = err(r#"relationship:(with:"Alice" wibble:"x")"#);
    assert!(
        e.message.contains("wibble") && e.message.contains("with"),
        "an unknown bound must be named along with the accepted ones: {}",
        e.message
    );
}

#[test]
fn scoping_a_field_with_no_single_row_is_refused() {
    let e = err(r#"tag:(with:"Alice")"#);
    assert!(
        e.message.contains("relationship"),
        "only a relation has one row to scope to: {}",
        e.message
    );
}

#[test]
fn a_scoped_predicate_round_trips_through_the_ast() {
    let parsed = ast(r#"relationship:(with:"Alice" type:romantic)"#);
    let QueryAst::Scoped(ScopedPredicate { relation, fields }) = &parsed else {
        panic!("expected a Scoped node, got {parsed:?}")
    };
    assert_eq!(*relation, lorehaven_domain::query::QueryField::Relationship);
    assert_eq!(
        fields,
        &vec![
            ScopedField {
                key: "with".into(),
                value: "Alice".into()
            },
            ScopedField {
                key: "type".into(),
                value: "romantic".into()
            },
        ],
        "field order is the reader's order and the pretty-printer must round-trip it"
    );
}

#[test]
fn a_scoped_predicate_with_no_pairs_is_refused() {
    let e = err("relationship:()");
    assert!(
        e.message.contains("key:value"),
        "an empty scoped predicate is not a filter: {}",
        e.message
    );
}

// ---------------------------------------------------------------------- min_match

#[test]
fn min_match_counts_satisfied_terms_and_is_not_a_disjunction() {
    let f = render_query(&ast(
        r#"min_match(2, tag:"Found Family", tag:"Hurt/Comfort", tag:"Banter")"#,
    ))
    .expect("renders");
    let s = &f.sql;

    // `>= n`, not `= n`: "at least two" must still match a work satisfying all three.
    assert!(
        s.contains(">= 2"),
        "min_match is a lower bound, not an exact count: {s}"
    );
    // One arm per term.
    assert_eq!(
        s.matches(" AS ok").count(),
        3,
        "each term is counted separately: {s}"
    );
    // And the three tags are bound.
    for tag in ["found family", "hurt/comfort", "banter"] {
        assert!(
            f.binds.iter().any(|b| b == tag),
            "{tag} must be bound, normalised like every taxonomy field: {:?}",
            f.binds
        );
    }
}

#[test]
fn min_match_is_distinguishable_from_or() {
    let m = sql(r#"min_match(2, tag:A, tag:B)"#);
    let o = sql(r#"tag:A OR tag:B"#);
    assert_ne!(
        m, o,
        "min_match(2, a, b) and `a OR b` are different questions and must not \
         compile to the same thing"
    );
    // Neither may degrade into a plain AND, which is the other plausible mistake.
    let a = sql(r#"tag:A AND tag:B"#);
    assert_ne!(m, a, "min_match must not compile to a conjunction");
}

#[test]
fn each_min_match_arm_is_scored_to_one_or_zero() {
    // A NULL predicate must count as UNSATISFIED, not vanish and take the whole SUM
    // with it. The two shorter forms both fail at runtime rather than at compile
    // time, so the shape is pinned here: `COALESCE(bool, 0)` is a PostgreSQL type
    // error, `SUM(bool)` does not exist, and `::int` is a SQLite parse error. Only
    // CASE WHEN is accepted by both engines.
    let f = render_query(&ast(r#"min_match(2, summary:"x", title:"y")"#)).expect("renders");
    assert_eq!(
        f.sql.matches("CASE WHEN (").count(),
        2,
        "each arm is scored: {}",
        f.sql
    );
    assert_eq!(
        f.sql.matches("THEN 1 ELSE 0 END").count(),
        2,
        "each arm must resolve to 1 or 0, never NULL: {}",
        f.sql
    );
    assert!(
        !f.sql.contains("COALESCE"),
        "COALESCE(bool, 0) is a type error on PostgreSQL: {}",
        f.sql
    );
    assert!(
        !f.sql.contains("::int") && !f.sql.contains("::integer"),
        "a PG cast is a parse error on SQLite: {}",
        f.sql
    );
}

#[test]
fn min_match_refuses_counts_that_can_never_be_satisfied() {
    let e = err(r#"min_match(4, tag:"A", tag:"B")"#);
    assert!(
        e.message.contains("never") || e.message.contains("never be satisfied"),
        "an unsatisfiable count must say so rather than return nothing: {}",
        e.message
    );
    let e = err(r#"min_match(0, tag:"A")"#);
    assert!(
        e.message.contains("everything"),
        "min_match(0, ...) matches everything, so it is not a filter: {}",
        e.message
    );
}

#[test]
fn min_match_needs_terms_and_a_count() {
    assert!(err("min_match(2)").message.contains("term"));
    assert!(err("min_match(tag:\"A\")").message.contains("how many"));
    assert!(err("min_match(1, tag:\"A\"").message.contains("')'"));
}

#[test]
fn min_matches_is_not_min_match() {
    // A longer identifier sharing a prefix must not be swallowed by the function
    // check. `min_matches:2` has no such field, so it is free text -- the point is
    // that it is NOT read as `min_match` and reported as a missing argument.
    match parse_query("min_matches:2").expect("free text parses") {
        QueryAst::Text(t) => assert_eq!(t, "min_matches:2"),
        other => panic!("expected free text, got {other:?}"),
    }
    // And the real function still works immediately after it.
    assert!(parse_query(r#"min_match(1, tag:"A")"#).is_ok());
}

// --------------------------------------------------------------------- expansion

#[test]
fn plus_children_becomes_a_closure_lookup() {
    let s = sql(r#"tag:"Fake Dating"+children"#);
    assert!(
        s.contains("taxonomy_closure"),
        "expansion reads the closure: {s}"
    );
    assert!(
        s.contains("tc.rel = 'parent'"),
        "expansion must follow `parent` only. `implies` is curator-only and default \
         off (§9's risk list), so following it here would silently widen every \
         expansion: {s}"
    );
    // The term itself still matches, so a work tagged with the parent is included.
    assert!(
        s.starts_with('(') && s.contains(" OR EXISTS"),
        "the base term and the descendant set are alternatives: {s}"
    );
}

#[test]
fn an_explicit_depth_becomes_a_bound() {
    let s = sql(r#"tag:"Fake Dating"+children^2"#);
    assert!(
        s.contains("tc.depth <= 2"),
        "the ^2 cap must be in the SQL: {s}"
    );
    let unbounded = sql(r#"tag:"Fake Dating"+children"#);
    assert!(
        !unbounded.contains("tc.depth <="),
        "an uncapped expansion omits the bound, leaving the closure's own size to \
         bound it: {unbounded}"
    );
}

#[test]
fn expansion_is_refused_on_a_field_with_no_closure() {
    let e = err("words:1000+children");
    assert!(
        e.message.contains("taxonomy") || e.message.contains("children"),
        "a non-taxonomy field has no children, and putting +children in the value \
         would match nothing: {}",
        e.message
    );
}

#[test]
fn the_only_expansion_is_children() {
    let e = err(r#"tag:"X"+parents"#);
    assert!(
        e.message.contains("+children"),
        "the error must name the one supported marker: {}",
        e.message
    );
    let e = err(r#"tag:"X"+children^x"#);
    assert!(
        e.message.contains("number"),
        "a depth needs a number: {}",
        e.message
    );
}

#[test]
fn the_expanded_term_is_the_one_the_reader_typed() {
    // Regression guard: the suffix is read from the input after the value, and the
    // value is recovered from the text before it. If the recovery walked back too
    // far it would carry `tag:"` with it and resolve nothing.
    let f = render_query(&ast(r#"tag:"Fake Dating"+children"#)).expect("renders");
    assert!(
        f.binds.iter().any(|b| b == "fake dating"),
        "the closure must resolve the reader's term, normalised like every other \
         taxonomy field: {:?}",
        f.binds
    );
    assert!(
        !f.binds.iter().any(|b| b.contains("tag:")),
        "no bind may carry the field name: {:?}",
        f.binds
    );
}

// ----------------------------------------------------------------- the budget

#[test]
fn the_default_budget_is_the_specs() {
    let b = QueryBudget::default();
    assert_eq!(b.max_depth, 8);
    assert_eq!(b.max_terms, 24);
    assert_eq!(b.max_expansion, 200);
    assert_eq!(b.max_min_match_arity, 8);
}

#[test]
fn a_deep_query_is_refused_by_name() {
    let deep = format!("{}tag:\"a\"{}", "(".repeat(12), ")".repeat(12));
    let e = err(&deep);
    assert!(
        e.message.contains("deep") && e.message.contains("8"),
        "the error must name the bound and the value: {}",
        e.message
    );
}

#[test]
fn too_many_terms_is_refused() {
    let many = (0..30)
        .map(|i| format!("tag:\"t{i}\""))
        .collect::<Vec<_>>()
        .join(" ");
    let e = err(&many);
    assert!(
        e.message.contains("24"),
        "the term limit must be named: {}",
        e.message
    );
}

#[test]
fn min_match_arity_is_bounded() {
    let terms = (0..12)
        .map(|i| format!("tag:\"t{i}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let e = err(&format!("min_match(2, {terms})"));
    assert!(
        e.message.contains("8"),
        "the min_match arity limit must be named: {}",
        e.message
    );
}

#[test]
fn an_expansion_deeper_than_the_cap_is_refused_not_clamped() {
    let e = err(r#"tag:"X"+children^5000"#);
    assert!(
        e.message.contains("5000") && e.message.contains("200"),
        "a cap past the limit must be refused so the reader learns it will not run \
         as written, not silently clamped: {}",
        e.message
    );
}

#[test]
fn a_zero_bound_means_no_limit_rather_than_match_nothing() {
    // A cap of zero is never a useful configuration, and treating it as "match
    // nothing" would silently break every query on an instance that set it.
    let budget = QueryBudget {
        max_depth: 0,
        max_terms: 0,
        max_expansion: 0,
        max_min_match_arity: 0,
    };
    let deep = format!("{}tag:\"a\"{}", "(".repeat(30), ")".repeat(30));
    let many = (0..200)
        .map(|i| format!("tag:\"t{i}\""))
        .collect::<Vec<_>>()
        .join(" ");
    for q in [deep, many, r#"tag:"X"+children^9999"#.to_owned()] {
        assert!(
            parse_query_with(&q, budget).is_ok(),
            "an unlimited budget must not refuse {q:?}"
        );
    }
}

#[test]
fn a_ordinary_query_is_inside_every_bound() {
    // The budget must not fire on the grammar's own examples from §15.4.1.
    for q in [
        r#"fandom:"Example Fandom" AND title:"winter""#,
        r#"body:"we were never alone" -tag:"major character death""#,
        r#"mood:comfort AND status:complete"#,
        r#"relationship:(with:"Alice" type:romantic)"#,
        r#"min_match(2, tag:"A", tag:"B")"#,
        r#"tag:"Fake Dating"+children^2"#,
        r#"words:20000..100000"#,
        r#"NOT tag:"x""#,
    ] {
        assert!(parse_query(q).is_ok(), "{q:?} must be inside every bound");
    }
}
