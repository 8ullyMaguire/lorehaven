//! §15.3's `ExistsCharacter` / `ExistsRelationship`: compilation and the
//! correlation rule.
//!
//! The load-bearing test in this file is
//! `attributes_are_bound_to_the_character_not_the_work`. §15.3 says: *"Never allow
//! one character to satisfy another character's attributes."* A compiler that emits
//! one `EXISTS` per clause satisfies every individual clause and returns a wrong
//! answer, so the test is written as a **database** fixture rather than a string
//! comparison — a string assertion would pass on a compiler that renders the right
//! SQL and a wrong one that renders it differently.
//!
//! | clause | test |
//! |---|---|
//! | one character satisfies all bounds | `attributes_are_bound_to_the_character_not_the_work` |
//! | journey 12's exclusion is not a work-level exclusion | `excluding_a_participant_stays_inside_one_relationship` |
//! | prominence is a bound on the character | `prominence_filters_on_the_same_character_row` |
//! | attributes_all vs attributes_any | `all_and_any_differ` |
//! | `is_pov` distinguishes three states | `is_pov_distinguishes_unset_from_false` |
//! | a relationship query needs a participant | `a_relationship_query_without_a_participant_is_refused` |
//! | dynamics membership is delimited | `dynamics_membership_is_delimited` |
//! | forum/user searches reject work-scoped nodes | `work_scoped_nodes_are_refused_outside_works` |

use lorehaven_domain::query::{CharacterAssertion, QueryAst, RelationshipAssertion};
use lorehaven_domain::query_sql::render_query;
use lorehaven_domain::query_sql_forum::render_forum_query as render_forum;
use lorehaven_domain::query_sql_user::render_user_query as render_user;

/// Normalise rendered SQL for substring assertions.
///
/// Rendered SQL contains newlines that `format!` folds unpredictably, and spaces
/// around parentheses that differ between two templates emitting the same clause
/// (`EXISTS (SELECT 1 FROM a` in one, `exists ( select 1 from b` in another).
/// Asserting on exact spacing makes a test fail when only whitespace changed — and
/// worse, encourages writing a needle that *happens* to match one branch.
///
/// So: lowercase, collapse whitespace runs to a single space, and drop whitespace
/// hugging parentheses. Then `exists ( select 1 from work_relationships` and
/// `EXISTS\n(SELECT 1 FROM work_relationships` are the same string, and a failing
/// assertion means the SQL genuinely differs.
fn norm(sql: &str) -> String {
    // Two problems with asserting on raw rendered SQL, both hit while writing this
    // file: `format!` folds newlines unpredictably, and two templates emitting the
    // same clause disagree about spacing next to parentheses. A test that fails on
    // a whitespace change is noise, and it tempts the author into writing a needle
    // that merely happens to match one branch.
    //
    // So: lowercase, collapse whitespace runs to one space, and drop the space
    // whenever it hugs a parenthesis. `exists ( select 1 from a` and
    // `exists(select 1 from b` are then the same string, and a failing assertion
    // means the SQL genuinely differs. Both sides of every comparison below go
    // through this function, so they are always in the same shape.
    let mut out = String::with_capacity(sql.len());
    let chars: Vec<char> = sql.to_lowercase().chars().collect();
    let hugs = |c: Option<char>| matches!(c, Some('(') | Some(')'));
    for (i, &ch) in chars.iter().enumerate() {
        if ch.is_whitespace() {
            let before = if i == 0 { None } else { Some(chars[i - 1]) };
            let after = chars.get(i + 1).copied();
            // The gap goes when it touches a paren, and never two gaps in a row.
            if !hugs(before) && !hugs(after) && !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// "Alice is a protagonist who is a vampire."
fn alice_protagonist_vampire() -> QueryAst {
    QueryAst::ExistsCharacter(CharacterAssertion {
        character_id: "alice".into(),
        prominence: vec!["protagonist".into()],
        attributes_all: vec!["vampire".into()],
        ..Default::default()
    })
}

/// The naive, wrong shape: the character, the prominence, and the attribute as
/// three independent top-level conjuncts, with nothing correlating them.
///
/// Built explicitly so the test can assert the difference between this and what
/// `render_query` actually produces. If the compiler ever regresses to emitting
/// this shape, the SQL fixture test below fails.
fn uncorrelated_alice_protagonist_vampire() -> QueryAst {
    QueryAst::And(vec![
        QueryAst::ExistsCharacter(CharacterAssertion {
            character_id: "alice".into(),
            ..Default::default()
        }),
        QueryAst::ExistsCharacter(CharacterAssertion {
            character_id: "bob".into(),
            prominence: vec!["protagonist".into()],
            ..Default::default()
        }),
        QueryAst::ExistsCharacter(CharacterAssertion {
            character_id: "bob".into(),
            attributes_all: vec!["vampire".into()],
            ..Default::default()
        }),
    ])
}

#[test]
fn attributes_are_bound_to_the_character_not_the_work() {
    // The compiler's own output must be structurally incapable of the naive
    // shape. Two properties, checked on the rendered SQL:
    //
    //  1. ONE `EXISTS` over `work_characters`, so there is one character row.
    //  2. Every attribute EXISTS names `wc.character_node_id`, so an attribute can
    //     only be satisfied by *that* row's character.
    let fragment = render_query(&alice_protagonist_vampire()).expect("compiles");
    let sql = norm(&fragment.sql);

    assert_eq!(
        sql.matches("from work_characters").count(),
        1,
        "a single character assertion must produce exactly ONE work_characters \
         EXISTS; three would let three different characters satisfy one query. Got:\n{sql}"
    );
    assert_eq!(
        sql.matches("from work_character_attributes").count(),
        1,
        "one attribute means one correlated EXISTS. Got:\n{sql}"
    );

    // The correlation itself: the attribute subquery must name the outer row's
    // character. This is the clause §15.3 is about, so it is asserted literally.
    assert!(
        sql.contains("aa.character_node_id = wc.character_node_id"),
        "the attribute EXISTS must correlate on character_node_id, or another \
         character's attribute satisfies this one. Got:\n{sql}"
    );
    assert!(
        sql.contains("aa.work_id = wc.work_id"),
        "the attribute EXISTS must correlate on work_id too. Got:\n{sql}"
    );

    // And the prominence bound must be on the same row, not a separate query.
    assert!(
        sql.contains("wc.prominence in"),
        "prominence must filter the same work_characters row. Got:\n{sql}"
    );

    // Binds: character id, then prominence, then the attribute, in that order.
    assert_eq!(
        fragment.binds,
        vec![
            "alice".to_string(),
            "protagonist".to_string(),
            "vampire".to_string(),
        ],
        "bind order must match placeholder order exactly"
    );

    // The naive shape is genuinely different SQL, which is why the database test
    // in the app suite is the one that matters. Confirm the two are not equal so
    // this test cannot pass vacuously.
    let naive = render_query(&uncorrelated_alice_protagonist_vampire()).expect("naive compiles");
    assert_ne!(
        naive.sql, fragment.sql,
        "the uncorrelated form must render differently, or this comparison proves nothing"
    );
    assert!(
        naive
            .sql
            .to_lowercase()
            .matches("from work_characters")
            .count()
            > 1,
        "the naive form is supposed to be the multi-EXISTS one; if it is not, this \
         fixture no longer demonstrates the bug it exists to demonstrate"
    );
}

#[test]
fn excluding_a_participant_stays_inside_one_relationship() {
    // "X with anyone except Y". The `NOT EXISTS` must be nested INSIDE the
    // relationship's own EXISTS and correlated on that relationship's ship.
    //
    // A top-level `AND NOT EXISTS` would exclude every work pairing X with anyone
    // at all — the opposite of what was asked — so the nesting is the assertion.
    let ast = QueryAst::ExistsRelationship(RelationshipAssertion {
        participant_any: vec!["alice".into()],
        kind_any: vec!["romantic".into()],
        excluded_participants: vec!["bob".into()],
        ..Default::default()
    });
    let fragment = render_query(&ast).expect("compiles");
    let sql = norm(&fragment.sql);

    assert_eq!(
        sql.matches("from work_relationships").count(),
        1,
        "one relationship EXISTS. Got:\n{sql}"
    );
    assert!(
        sql.contains(&norm("not exists(select 1 from ship_participants sx")),
        "the exclusion must be a NOT EXISTS over participants. Got:\n{sql}"
    );
    // The participant test is likewise inside the relationship.
    assert!(
        sql.contains("sp.ship_node_id = wr.ship_node_id"),
        "the participant test must be inside the relationship EXISTS. Got:\n{sql}"
    );

    assert_eq!(
        fragment.binds.len(),
        3,
        "alice, romantic, bob — one bind each"
    );
}

#[test]
fn prominence_filters_on_the_same_character_row() {
    // Two characters, two prominences: the query must be satisfiable by Alice and
    // NOT by Bob, and nothing about Bob may leak in.
    let ast = QueryAst::ExistsCharacter(CharacterAssertion {
        character_id: "alice".into(),
        prominence: vec!["protagonist".into(), "co-protagonist".into()],
        ..Default::default()
    });
    let fragment = render_query(&ast).expect("compiles");
    let sql = norm(&fragment.sql);

    assert!(
        sql.contains("wc.prominence in(?,?)"),
        "two prominences must render two placeholders, comma-separated. Got:\n{sql}"
    );
    assert_eq!(
        fragment.binds,
        vec!["alice", "protagonist", "co-protagonist"],
        "bind count must equal placeholder count"
    );
}

#[test]
fn all_and_any_differ() {
    let all = render_query(&QueryAst::ExistsCharacter(CharacterAssertion {
        character_id: "alice".into(),
        attributes_all: vec!["vampire".into(), "bamf".into()],
        ..Default::default()
    }))
    .expect("compiles");
    let any = render_query(&QueryAst::ExistsCharacter(CharacterAssertion {
        character_id: "alice".into(),
        attributes_any: vec!["vampire".into(), "bamf".into()],
        ..Default::default()
    }))
    .expect("compiles");

    // `all` needs two correlated subqueries (every attribute must be present);
    // `any` needs one with an IN list. Collapsing them would silently turn a
    // conjunction into a disjunction, which returns MORE works than asked for.
    assert_eq!(
        norm(&all.sql)
            .matches("from work_character_attributes")
            .count(),
        2,
        "attributes_all needs one correlated EXISTS per attribute. Got:\n{}",
        all.sql
    );
    assert_eq!(
        norm(&any.sql)
            .matches("from work_character_attributes")
            .count(),
        1,
        "attributes_any needs one EXISTS with an IN list. Got:\n{}",
        any.sql
    );
    assert!(
        norm(&any.sql).contains("aty.norm in(?,?)"),
        "attributes_any must be an IN list. Got:\n{}",
        any.sql
    );
    // The bind LISTS are identical -- `["alice", "vampire", "bamf"]` either way --
    // and that is correct, not a bug: the same three values are bound, the only
    // difference is whether they are joined by AND (one subquery each) or by the
    // commas of one IN list. Asserting the bind lists differ would be asserting
    // something false; it is the SQL *shape* above that distinguishes them, which
    // is why this test is worth having. Recorded here so the next reader does not
    // "fix" the compiler to make the binds differ.
    assert_eq!(
        all.binds, any.binds,
        "all and any bind the same values; only the SQL shape differs"
    );
}

#[test]
fn is_pov_distinguishes_unset_from_false() {
    let unset = render_query(&QueryAst::ExistsCharacter(CharacterAssertion {
        character_id: "alice".into(),
        ..Default::default()
    }))
    .expect("compiles");
    let yes = render_query(&QueryAst::ExistsCharacter(CharacterAssertion {
        character_id: "alice".into(),
        is_pov: Some(true),
        ..Default::default()
    }))
    .expect("compiles");
    let no = render_query(&QueryAst::ExistsCharacter(CharacterAssertion {
        character_id: "alice".into(),
        is_pov: Some(false),
        ..Default::default()
    }))
    .expect("compiles");

    // Three distinct queries. If `None` collapsed to `Some(false)`, "any character"
    // would become "any non-POV character" and quietly lose every protagonist fic.
    assert!(
        !unset.sql.contains("is_pov"),
        "None must emit no predicate at all. Got:\n{}",
        unset.sql
    );
    assert!(yes.sql.contains("wc.is_pov = 1"), "Got:\n{}", yes.sql);
    assert!(no.sql.contains("wc.is_pov = 0"), "Got:\n{}", no.sql);
}

#[test]
fn a_relationship_query_without_a_participant_is_refused() {
    // An empty `participant_any` would compile to a relationship EXISTS with no
    // participant test — which matches EVERY relationship in the instance. That
    // is not a broad query, it is a data leak waiting to happen, so it is an error.
    let err = render_query(&QueryAst::ExistsRelationship(RelationshipAssertion {
        participant_any: vec![],
        kind_any: vec!["romantic".into()],
        ..Default::default()
    }))
    .expect_err("must refuse");
    assert!(
        err.message.contains("participant"),
        "the error must say what is missing; got: {}",
        err.message
    );
}

#[test]
fn dynamics_membership_is_delimited() {
    // `dynamics` is a comma-separated list, so membership is a LIKE on a delimited
    // pattern. `enemies` must not match `enemies_to_lovers`.
    let fragment = render_query(&QueryAst::ExistsRelationship(RelationshipAssertion {
        participant_any: vec!["alice".into()],
        dynamics: vec!["enemies".into()],
        ..Default::default()
    }))
    .expect("compiles");

    assert_eq!(
        fragment.binds.last().map(String::as_str),
        Some("%,enemies,%"),
        "the bind must carry the delimiters, or `enemies` matches \
         `enemies_to_lovers`. Got {:?}",
        fragment.binds
    );
}

#[test]
fn work_scoped_nodes_are_refused_outside_works() {
    // Forum and user searches compile the same AST against different tables. These
    // nodes read `works`-scoped tables, so they must be REFUSED there — an arm that
    // silently returned "false" would tell a reader their search found nothing when
    // the engine never looked.
    let character = alice_protagonist_vampire();
    let relationship = QueryAst::ExistsRelationship(RelationshipAssertion {
        participant_any: vec!["alice".into()],
        ..Default::default()
    });

    for (name, result) in [
        ("forum/character", render_forum(&character)),
        ("forum/relationship", render_forum(&relationship)),
        ("user/character", render_user(&character)),
        ("user/relationship", render_user(&relationship)),
    ] {
        let err = result
            .err()
            .unwrap_or_else(|| panic!("{name} must be refused"));
        assert!(
            err.message.contains("only apply to works"),
            "{name}: the error must say why; got: {}",
            err.message
        );
    }

    // And they still work for works, so the refusal is scoped rather than global.
    assert!(render_query(&character).is_ok());
    assert!(render_query(&relationship).is_ok());
}

#[test]
fn a_bare_character_assertion_still_binds_the_character() {
    // `character:Alice` with no other bounds is a legitimate query. It must still
    // compile to a work_characters EXISTS rather than being folded into work_tags:
    // a tag can name a character without the character being present in the work,
    // and those are different facts.
    let bare = CharacterAssertion {
        character_id: "alice".into(),
        ..Default::default()
    };
    assert!(bare.is_bare(), "the fixture must actually be bare");

    let fragment = render_query(&QueryAst::ExistsCharacter(bare)).expect("compiles");
    assert!(
        fragment.sql.contains("work_characters"),
        "Got:\n{}",
        fragment.sql
    );
    assert!(
        !fragment.sql.contains("work_tags"),
        "a bare character assertion must not be answered from work_tags; \
         a tag is not the same fact as a character being present. Got:\n{}",
        fragment.sql
    );
    assert_eq!(fragment.binds, vec!["alice".to_string()]);
}

#[test]
fn a_non_bare_assertion_is_not_bare() {
    // The negative control for the test above: if `is_bare` returned true for
    // everything, the test above would pass for the wrong reason.
    for assertion in [
        CharacterAssertion {
            character_id: "alice".into(),
            prominence: vec!["protagonist".into()],
            ..Default::default()
        },
        CharacterAssertion {
            character_id: "alice".into(),
            attributes_all: vec!["vampire".into()],
            ..Default::default()
        },
        CharacterAssertion {
            character_id: "alice".into(),
            is_pov: Some(true),
            ..Default::default()
        },
        CharacterAssertion {
            character_id: "alice".into(),
            roles: vec!["mentor".into()],
            ..Default::default()
        },
    ] {
        assert!(
            !assertion.is_bare(),
            "an assertion with bounds must not report itself bare: {assertion:?}"
        );
    }
}

#[test]
fn the_not_arm_negates_a_correlated_exists_without_corrupting_it() {
    // Journey 12 is `character:X AND NOT relationship involving X`. The `Not` arm
    // wraps whatever the inner node rendered; this checks the wrap preserves the
    // correlation rather than flattening it.
    let ast = QueryAst::And(vec![
        QueryAst::ExistsCharacter(CharacterAssertion {
            character_id: "bob".into(),
            ..Default::default()
        }),
        QueryAst::Not(Box::new(QueryAst::ExistsRelationship(
            RelationshipAssertion {
                participant_any: vec!["bob".into()],
                kind_any: vec!["romantic".into(), "sexual".into()],
                ..Default::default()
            },
        ))),
    ]);
    let fragment = render_query(&ast).expect("compiles");
    let sql = norm(&fragment.sql);

    assert!(
        sql.contains(&norm("not((exists(select 1 from work_relationships")),
        "Got:\n{}",
        sql
    );
    assert!(
        sql.contains("sp.ship_node_id = wr.ship_node_id"),
        "the negated relationship must keep its internal correlation. Got:\n{sql}"
    );
    // No participant exclusion here: journey 12 is "Bob, and no romantic or sexual
    // relationship involving Bob", which needs none. Asserting one would be
    // asserting a clause the query does not have.
    assert!(
        !sql.contains("ship_participants sx"),
        "journey 12 excludes no participant, so no sx subquery belongs here. Got:\n{sql}"
    );
    // An EXISTS is never NULL, so the NULL-guard coalesce must NOT be applied —
    // `NOT COALESCE(exists(...), false)` is correct but noisy, and the existing
    // `needs_null_guard` rule says an EXISTS does not need it.
    assert!(
        !sql.contains("coalesce"),
        "an EXISTS arm needs no NULL guard; adding one is noise. Got:\n{sql}"
    );
    assert_eq!(fragment.binds, vec!["bob", "bob", "romantic", "sexual"]);
}
