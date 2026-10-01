//! Migration 0104 must APPLY on whichever backend the harness picks, and its
//! constraints must BITE once it has.
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
async fn refused(db: &TestDb, sql: &str) -> bool {
    exec(&db, sql).await.is_err()
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
    assert!(
        applied.iter().any(|m| m.contains("0104")),
        "0104 must be in the applied set, got {applied:?}"
    );
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
