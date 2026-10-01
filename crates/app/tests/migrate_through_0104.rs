//! Migrations 0104 and 0105 must APPLY on whichever backend the harness picks, and
//! their constraints must BITE once they have.
//!
//! Both halves are needed, and the first version of this file had only the first,
//! which is how a real defect got through:
//!
//!   * `migration_catalogue` compares declared column sets. It never executes the
//!     DDL, so a migration the database refuses is invisible to it.
//!   * The system `sqlite3` CLI is 3.53.4 and accepts
//!     `ALTER TABLE ... ADD CONSTRAINT`. The SQLite the app links -- bundled by
//!     `libsqlite3-sys 0.30.1` -- is **3.46.0**, which predates that syntax (added
//!     in 3.50.0) and fails with `near "CONSTRAINT": syntax error`. A CLI-only probe
//!     passes against a migration the default engine cannot apply.
//!
//! So 0104's SQLite half enforces its ALTER-able constraints with triggers (the
//! idiom 0103 established) while PostgreSQL keeps real CHECKs. This file proves
//! both engines by provoking every constraint through the application's own pool,
//! which is the only path that exercises the SQLite the app actually links.

use lorehaven_db::Backend;
use test_support::{scratch_dir, TestDb};

/// Create a node. Ids are written as literals rather than bound because the
/// statements below are fixed strings for both dialects, and a `?1` that survives
/// `TestDb::sql` is exactly the class of bug this file is about.
async fn node(db: &TestDb, id: &str) {
    let sql = db.sql(&format!(
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at) \
         VALUES ('{id}', 'character', '{id}', '{}', '2026-01-01T00:00:00Z')",
        id.to_lowercase()
    ));
    exec(&db, &sql).await.expect("a node inserts");
}

/// Run a statement on whichever pool the harness built.
async fn exec(db: &TestDb, sql: &str) -> Result<(), sqlx::Error> {
    let sql = db.sql(sql);
    match db.db().backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .execute(db.db().sqlite_pool().expect("sqlite"))
            .await
            .map(|_| ()),
        Backend::Postgres => sqlx::query(&sql)
            .execute(db.db().postgres_pool().expect("postgres"))
            .await
            .map(|_| ()),
    }
}

/// Whether the database refused a statement. A constraint that does not bite
/// reports `false` here, which is the whole point.
///
/// `why` is the error text, and it matters more than it looks: a mutation check
/// showed this helper reporting `true` for a duplicate work-scoped warning on
/// PostgreSQL even with plain `UNIQUE` in place -- which PostgreSQL demonstrably
/// permits, since NULL != NULL in a unique index. Some *other* error was being
/// counted as the constraint biting. An assertion that cannot say which error it
/// saw will do that again.
async fn refused(db: &TestDb, sql: &str) -> bool {
    exec(&db, sql).await.is_err()
}

/// The error text a statement produced, or None if it succeeded.
async fn refusal(db: &TestDb, sql: &str) -> Option<String> {
    exec(&db, sql).await.err().map(|e| e.to_string())
}

/// Read one text value.
async fn text(db: &TestDb, sql: &str) -> String {
    let sql = db.sql(sql);
    match db.db().backend() {
        Backend::Sqlite => sqlx::query_scalar(&sql)
            .fetch_one(db.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("query runs"),
        Backend::Postgres => sqlx::query_scalar(&sql)
            .fetch_one(db.db().postgres_pool().expect("postgres"))
            .await
            .expect("query runs"),
    }
}

/// Read one integer value. Separate from [`text`] because `COUNT(*)` is INT8 on
/// PostgreSQL and INTEGER on SQLite, and neither decodes into a `String`.
async fn number(db: &TestDb, sql: &str) -> i64 {
    let sql = db.sql(sql);
    match db.db().backend() {
        Backend::Sqlite => sqlx::query_scalar(&sql)
            .fetch_one(db.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("query runs"),
        Backend::Postgres => sqlx::query_scalar(&sql)
            .fetch_one(db.db().postgres_pool().expect("postgres"))
            .await
            .expect("query runs"),
    }
}

#[tokio::test]
async fn the_full_chain_applies_through_0104() {
    let dir = scratch_dir("migrate_all_0104");
    let db = TestDb::connect_with_dir("migrate_all_0104", &dir).await;
    let backend = if db.is_postgres() {
        "postgres"
    } else {
        "sqlite"
    };
    let applied = db.applied_migrations();
    println!(
        "BACKEND={backend} COUNT={} LAST={:?}",
        applied.len(),
        applied.last()
    );
    for want in ["0104", "0105"] {
        assert!(
            applied.iter().any(|m| m.contains(want)),
            "{want} must be in the applied set, got {applied:?}"
        );
    }
    db.cleanup().await;
}

#[tokio::test]
async fn an_existing_node_defaults_to_active() {
    let dir = scratch_dir("migrate_0104_default");
    let db = TestDb::connect_with_dir("migrate_0104_default", &dir).await;
    node(&db, "n1").await;
    assert_eq!(
        text(&db, "SELECT status FROM taxonomy_nodes WHERE id = 'n1'").await,
        "active",
        "a node created after the migration must default to active, not NULL"
    );
    db.cleanup().await;
}

#[tokio::test]
async fn the_alter_able_constraints_bite() {
    let dir = scratch_dir("migrate_0104_constraints");
    let db = TestDb::connect_with_dir("migrate_0104_constraints", &dir).await;
    node(&db, "n1").await;
    node(&db, "n2").await;

    assert!(
        refused(
            &db,
            "UPDATE taxonomy_nodes SET status = 'bogus' WHERE id = 'n1'"
        )
        .await,
        "an unrecognised lifecycle status must be refused"
    );
    assert!(
        refused(
            &db,
            "UPDATE taxonomy_nodes SET status = 'merged' WHERE id = 'n1'"
        )
        .await,
        "status='merged' with merged_into NULL must be refused -- a merge that \
         resolves to nothing is not a merge"
    );
    assert!(
        refused(
            &db,
            "UPDATE taxonomy_nodes SET status = 'active', merged_into = 'n2' WHERE id = 'n1'"
        )
        .await,
        "merged_into set on a node that is not merged must be refused"
    );
    assert!(
        refused(
            &db,
            "UPDATE taxonomy_nodes SET status = 'merged', merged_into = 'nope' WHERE id = 'n1'"
        )
        .await,
        "a merge must not point at a node that does not exist"
    );

    // The positive case, so the checks are not simply refusing everything.
    exec(
        &db,
        "UPDATE taxonomy_nodes SET status = 'merged', merged_into = 'n2' WHERE id = 'n1'",
    )
    .await
    .expect("a well-formed merge is accepted");
    exec(
        &db,
        "UPDATE taxonomy_nodes SET status = 'active', merged_into = NULL WHERE id = 'n1'",
    )
    .await
    .expect("unmerging is accepted");

    // Triggers are registered separately for INSERT and UPDATE, so both are
    // provoked: an INSERT-only trigger set would pass the five assertions above.
    assert!(
        refused(
            &db,
            "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at, status) \
             VALUES ('n3', 'character', 'n3', 'n3', '2026-01-01T00:00:00Z', 'bogus')"
        )
        .await,
        "the INSERT trigger must fire as well as the UPDATE one"
    );
    assert!(
        refused(
            &db,
            "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at, status, merged_into) \
             VALUES ('n4', 'character', 'n4', 'n4', '2026-01-01T00:00:00Z', 'merged', NULL)"
        )
        .await,
        "the INSERT form of the merge check must fire too"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn the_new_tables_refuse_impossible_shapes() {
    let dir = scratch_dir("migrate_0104_tables");
    let db = TestDb::connect_with_dir("migrate_0104_tables", &dir).await;
    node(&db, "n1").await;
    node(&db, "n2").await;

    // CREATE TABLE constraints, so both engines enforce them natively and no
    // dialect split is needed here.
    assert!(
        refused(
            &db,
            "INSERT INTO taxonomy_node_scope VALUES ('n1','n1','2026-01-01T00:00:00Z')"
        )
        .await,
        "a node scoped to itself must be refused"
    );
    assert!(
        refused(
            &db,
            "INSERT INTO taxonomy_edges VALUES ('n1','n1','parent',NULL,'2026-01-01T00:00:00Z')"
        )
        .await,
        "a self edge must be refused: it makes the closure computation non-terminating"
    );
    assert!(
        refused(
            &db,
            "INSERT INTO taxonomy_edges VALUES ('n1','n2','hates',NULL,'2026-01-01T00:00:00Z')"
        )
        .await,
        "an unrecognised relation must be refused rather than stored and ignored"
    );
    assert!(
        refused(
            &db,
            "INSERT INTO taxonomy_closure VALUES ('n1','n2','parent',-1,'2026-01-01T00:00:00Z')"
        )
        .await,
        "a negative depth must be refused"
    );
    assert!(
        refused(
            &db,
            "INSERT INTO taxonomy_edges VALUES ('n1','nope','parent',NULL,'2026-01-01T00:00:00Z')"
        )
        .await,
        "an edge to a node that does not exist must be refused"
    );

    // And the shapes the plan depends on must be accepted.
    for sql in [
        "INSERT INTO taxonomy_node_scope VALUES ('n1','n2','2026-01-01T00:00:00Z')",
        "INSERT INTO taxonomy_edges VALUES ('n1','n2','parent',NULL,'2026-01-01T00:00:00Z')",
        "INSERT INTO taxonomy_closure VALUES ('n1','n1','parent',0,'2026-01-01T00:00:00Z')",
    ] {
        exec(&db, sql)
            .await
            .unwrap_or_else(|e| panic!("must be accepted: {sql}\n{e}"));
    }

    assert_eq!(
        number(
            &db,
            "SELECT COUNT(*) FROM taxonomy_closure WHERE ancestor_id = 'n1' AND rel = 'parent'"
        )
        .await,
        1,
        "the depth-0 self row is what +children needs: without it expansion \
         excludes the node the reader asked for"
    );

    db.cleanup().await;
}

/// A work to hang warnings, votes and policies off.
///
/// `works` has exactly four NOT NULL columns with no default -- `id`,
/// `owner_pseud_id`, `created_at`, `updated_at` -- read from
/// `information_schema` rather than guessed, after two rounds of "null value in
/// column ..." from PostgreSQL. A work needs an owner, and an owner needs an
/// account, so the fixture creates the whole chain.
///
/// Ids are literal strings cast on PostgreSQL only, because `works.id` is TEXT on
/// SQLite and UUID on PostgreSQL and a bound string does not match a uuid column.
/// That asymmetry is the same one `TestDb::fetch_text_column` exists to paper over.
async fn work(db: &TestDb, id: &str) {
    let short = &id[..8];
    let cast = if db.is_postgres() { "::uuid" } else { "" };
    exec(
        db,
        &format!(
            "INSERT INTO accounts (id, email, created_at, updated_at) \
             VALUES ('{id}'{cast}, 'w{short}@example.invalid', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        ),
    )
    .await
    .expect("an account inserts");
    exec(
        db,
        &format!(
            "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
             VALUES ('{id}'{cast}, '{id}'{cast}, 'wp{short}', 'W Pseud {short}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        ),
    )
    .await
    .expect("a pseud inserts");
    exec(
        db,
        &format!(
            "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at) \
             VALUES ('{id}'{cast}, '{id}'{cast}, 'w', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        ),
    )
    .await
    .expect("a work inserts");
}

/// A taxonomy node of kind 'warning', which is what work_warnings references.
async fn warning_node(db: &TestDb, id: &str) {
    let sql = format!(
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at) \
         VALUES ('{id}', 'warning', '{id}', '{}', '2026-01-01T00:00:00Z')",
        id.to_lowercase()
    );
    exec(db, &sql).await.expect("a warning node inserts");
}

fn lit(db: &TestDb, s: &str) -> String {
    // Casts a uuid literal on PostgreSQL only, for the same reason `work` does.
    if db.is_postgres() {
        s.replace("__UUID__", "::uuid")
    } else {
        s.replace("__UUID__", "")
    }
}

#[tokio::test]
async fn work_warnings_refuse_impossible_shapes() {
    let dir = scratch_dir("migrate_0105_warnings");
    let db = TestDb::connect_with_dir("migrate_0105_warnings", &dir).await;
    work(&db, "11111111-1111-1111-1111-111111111111").await;
    warning_node(&db, "wn1").await;

    // `chapter` and `node` are passed already-quoted-or-NULL by the caller; the two
    // enum-ish values are passed BARE and quoted exactly once here. Quoting in both
    // places produced ''"on_page"'' and a syntax error at "on_page", which is the
    // kind of double-quoting bug that reads as a database problem.
    let insert = |chapter: &str, node: &str, severity: &str, depiction: &str, declaration: &str| {
        format!(
            "INSERT INTO work_warnings (work_id, chapter_id, warning_node_id, severity, depiction, declaration, created_at, updated_at) \
             VALUES ('11111111-1111-1111-1111-111111111111'__UUID__, {chapter}, {node}, {severity}, '{depiction}', '{declaration}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        )
    };

    // The domains the plan leaves as bare column names.
    assert!(
        refused(
            &db,
            &lit(&db, &insert("NULL", "'wn1'", "3", "on_page", "declared"))
        )
        .await,
        "severity outside 1|2 must be refused"
    );
    assert!(
        refused(
            &db,
            &lit(&db, &insert("NULL", "'wn1'", "1", "hinted", "declared"))
        )
        .await,
        "an unrecognised depiction must be refused rather than stored and ignored"
    );
    assert!(
        refused(
            &db,
            &lit(&db, &insert("NULL", "'wn1'", "1", "on_page", "probably"))
        )
        .await,
        "an unrecognised declaration must be refused"
    );
    assert!(
        refused(
            &db,
            &lit(&db, &insert("NULL", "'nope'", "1", "on_page", "declared"))
        )
        .await,
        "a warning must not reference a taxonomy node that does not exist"
    );

    // `none_apply` and `creator_chose_not_to_say` are different states and both
    // must be storable -- the point of keeping them apart. Values are passed BARE
    // and quoted once, inside `insert`.
    for declaration in [
        "declared",
        "none_apply",
        "creator_chose_not_to_say",
        "reader_flagged",
    ] {
        let node = format!("wn_{declaration}");
        warning_node(&db, &node).await;
        exec(
            &db,
            &lit(
                &db,
                &insert("NULL", &format!("'{node}'"), "1", "on_page", declaration),
            ),
        )
        .await
        .unwrap_or_else(|e| panic!("{declaration} must be accepted:\n{e}"));
    }

    // Null-chapter uniqueness: NULL does not compare equal to NULL, so this is
    // where a plain UNIQUE would silently let a work-scoped warning land twice.
    warning_node(&db, "wn_dup").await;
    exec(
        &db,
        &lit(&db, &insert("NULL", "'wn_dup'", "1", "on_page", "declared")),
    )
    .await
    .expect("first work-scoped warning");
    let dup = lit(&db, &insert("NULL", "'wn_dup'", "1", "on_page", "declared"));
    let err = refusal(&db, &dup).await;
    assert!(
        err.is_some(),
        "the same work-scoped warning must not be insertable twice: a NULL chapter \
         does not collide with itself under a plain UNIQUE, which is the whole \
         reason the uniqueness is an expression index / NULLS NOT DISTINCT"
    );
    println!(
        "DUP REFUSAL ON {backend}: {err:?}",
        backend = if db.is_postgres() {
            "postgres"
        } else {
            "sqlite"
        }
    );
    // A chapter-scoped one is a different row and must be accepted.
    exec(
        &db,
        &lit(
            &db,
            &insert("'ch1'", "'wn_dup'", "1", "on_page", "declared"),
        ),
    )
    .await
    .expect("the same warning scoped to a chapter is a different row");

    db.cleanup().await;
}

#[tokio::test]
async fn tag_votes_refuse_weights_outside_the_domain() {
    let dir = scratch_dir("migrate_0105_votes");
    let db = TestDb::connect_with_dir("migrate_0105_votes", &dir).await;
    work(&db, "22222222-2222-2222-2222-222222222222").await;
    warning_node(&db, "tn1").await;

    let vote = |v: &str, voter: &str| {
        format!(
            "INSERT INTO work_tag_votes (work_id, node_id, voter_pseud_id, vote, created_at, updated_at) \
             VALUES ('22222222-2222-2222-2222-222222222222'__UUID__, 'tn1', '{voter}', {v}, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        )
    };
    for good in ["-1", "0", "1"] {
        exec(&db, &lit(&db, &vote(good, &format!("p{good}"))))
            .await
            .unwrap_or_else(|e| panic!("vote {good} must be accepted:\n{e}"));
    }
    assert!(
        refused(&db, &lit(&db, &vote("5", "p5"))).await,
        "a vote weight outside -1|0|1 must be refused"
    );
    assert!(
        refused(&db, &lit(&db, &vote("-2", "pm2"))).await,
        "a negative weight beyond -1 must be refused"
    );
    // One vote per (work, tag, voter): a second vote is a conflict, not a second
    // row that would double-count the confidence aggregate.
    assert!(
        refused(&db, &lit(&db, &vote("1", "p1"))).await,
        "a reader must not hold two votes on the same tag for the same work"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn index_policy_refuses_non_boolean_flags() {
    let dir = scratch_dir("migrate_0105_policy");
    let db = TestDb::connect_with_dir("migrate_0105_policy", &dir).await;
    work(&db, "33333333-3333-3333-3333-333333333333").await;

    // The default matters: a work with no explicit policy must not be embeddable,
    // because §47.10/§49.9 forbid inferred data influencing anything.
    exec(
        &db,
        &lit(
            &db,
            "INSERT INTO work_index_policy (work_id, created_at, updated_at) \
             VALUES ('33333333-3333-3333-3333-333333333333'__UUID__, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        ),
    )
    .await
    .expect("a policy row inserts");
    // Read as text, not as a number: a bare INTEGER column decodes as INT4 on
    // PostgreSQL and INTEGER on SQLite, and neither matches the i64/INT8 the
    // `number` helper asks for. `CAST(... AS TEXT)` is portable here because the
    // column is already an integer -- unlike a uuid column, where the cast is
    // valid on one engine and a syntax error on the other.
    assert_eq!(
        text(
            &db,
            "SELECT CAST(allow_embedding AS TEXT) FROM work_index_policy"
        )
        .await,
        "0",
        "embedding must be off unless a policy says otherwise: §47.10/§49.9 forbid \
         inferred data influencing anything, so the default cannot be permissive"
    );
    assert_eq!(
        text(
            &db,
            "SELECT CAST(fetch_remote AS TEXT) FROM work_index_policy"
        )
        .await,
        "1",
        "fetching defaults on: a work in the catalogue is indexable by default"
    );

    assert!(
        refused(
            &db,
            &lit(&db, "UPDATE work_index_policy SET allow_embedding = 2")
        )
        .await,
        "a flag outside 0|1 must be refused"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn passages_refuse_impossible_offsets() {
    let dir = scratch_dir("migrate_0105_passages");
    let db = TestDb::connect_with_dir("migrate_0105_passages", &dir).await;
    work(&db, "44444444-4444-4444-4444-444444444444").await;

    let passage = |seq: i32, start: i32, end: i32, chapter: &str| {
        format!(
            "INSERT INTO work_passages (id, work_id, chapter_id, seq, text, start_offset, end_offset, created_at) \
             VALUES ('p{seq}{start}', '44444444-4444-4444-4444-444444444444'__UUID__, {chapter}, {seq}, 'x', {start}, {end}, '2026-01-01T00:00:00Z')"
        )
    };
    exec(&db, &lit(&db, &passage(0, 0, 120, "NULL")))
        .await
        .expect("a work-level passage inserts");
    exec(&db, &lit(&db, &passage(1, 100, 220, "'ch1'")))
        .await
        .expect("a chapter passage inserts");

    assert!(
        refused(&db, &lit(&db, &passage(2, 200, 100, "'ch1'"))).await,
        "a passage running backwards must be refused"
    );
    assert!(
        refused(&db, &lit(&db, &passage(3, -1, 50, "'ch1'"))).await,
        "a negative start offset must be refused"
    );
    assert!(
        refused(&db, &lit(&db, &passage(1, 0, 50, "'ch1'"))).await,
        "two passages of one chapter must not share a position"
    );
    // The same seq in a different chapter is a different position.
    exec(&db, &lit(&db, &passage(1, 0, 50, "'ch2'")))
        .await
        .expect("the same seq in another chapter is a different passage");

    db.cleanup().await;
}
