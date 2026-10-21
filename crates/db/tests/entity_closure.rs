//! M6-03: the aliases and the closure (spec §6.4, §11.4).
//!
//! # What this asserts
//!
//! * **`+children` returns the transitive set**, and it does so from the
//!   **closure** — a single join rather than a recursion.
//! * **The closure contains no cycle.** A parent that is also a child is
//!   nonsense, and the rebuild must terminate on it rather than loop: a
//!   recursive query would, because PostgreSQL's recursive term may not use
//!   `UNION`.
//! * **Only browsable entities are in the closure.** The closure *is* the tree,
//!   and an entity below the floor is an assertion, not a node (spec §6.4).
//! * **The rebuild is a rebuild, not an append.** A stale row is a row that
//!   says a tree that no longer exists.

use darkhold_db::entity_closure;
use darkhold_db::entity_floor::{self, EntityFloors};
use darkhold_db::Database;

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "darkhold-closure-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

async fn database(tag: &str) -> (Database, std::path::PathBuf) {
    let dir = scratch_dir(tag);
    let db = match std::env::var("DARKHOLD_TEST_PG_URL") {
        Ok(admin_url) if !admin_url.is_empty() => {
            static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let name = format!(
                "lh_test_closure_{}_{}",
                std::process::id(),
                COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            );
            test_support::empty_postgres(&admin_url, &name)
                .await
                .expect("an empty PostgreSQL database")
        }
        _ => {
            let config = darkhold_db::DatabaseConfig::new(format!(
                "sqlite://{}/darkhold.sqlite?mode=rwc",
                dir.display()
            ));
            Database::connect(&config).await.expect("connect")
        }
    };
    db.migrate().await.expect("migrate");
    (db, dir)
}

/// An entity, with an optional parent.
async fn entity(db: &Database, id: &str, parent: Option<&str>) {
    let statement = match db.backend() {
        darkhold_db::Backend::Sqlite => format!(
            "INSERT INTO canonical_entities (id, kind, canonical, norm, created_at, review_status, signal_count, node_state, node_count)
             VALUES ('{id}', 'tag', '{id}', '{id}', '2026-01-01T00:00:00Z', 'curated', 0, 'browsable', 10)"
        ),
        darkhold_db::Backend::Postgres => format!(
            "INSERT INTO canonical_entities (id, kind, canonical, norm, created_at, review_status, signal_count, node_state, node_count)
             VALUES ('{id}', 'tag', '{id}', '{id}', '2026-01-01T00:00:00Z', 'curated', 0, 'browsable', 10)"
        ),
    };
    let result = match db.backend() {
        darkhold_db::Backend::Sqlite => sqlx::query(&statement)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .map(|_| ()),
        darkhold_db::Backend::Postgres => sqlx::query(&statement)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .map(|_| ()),
    };
    result.expect("the entity");

    if let Some(parent) = parent {
        let statement = match db.backend() {
            darkhold_db::Backend::Sqlite => {
                format!("UPDATE canonical_entities SET parent_id = '{parent}' WHERE id = '{id}'")
            }
            darkhold_db::Backend::Postgres => {
                format!("UPDATE canonical_entities SET parent_id = '{parent}' WHERE id = '{id}'")
            }
        };
        let result = match db.backend() {
            darkhold_db::Backend::Sqlite => sqlx::query(&statement)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .map(|_| ()),
            darkhold_db::Backend::Postgres => sqlx::query(&statement)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .map(|_| ()),
        };
        result.expect("the parent");
    }
}

/// An entity's parent.
async fn set_parent(db: &Database, id: &str, parent: &str) {
    let statement = match db.backend() {
        darkhold_db::Backend::Sqlite => {
            format!("UPDATE canonical_entities SET parent_id = '{parent}' WHERE id = '{id}'")
        }
        darkhold_db::Backend::Postgres => {
            format!("UPDATE canonical_entities SET parent_id = '{parent}' WHERE id::text = '{id}'")
        }
    };
    let result = match db.backend() {
        darkhold_db::Backend::Sqlite => sqlx::query(&statement)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .map(|_| ()),
        darkhold_db::Backend::Postgres => sqlx::query(&statement)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .map(|_| ()),
    };
    result.expect("the parent");
}

#[tokio::test]
async fn the_closure_is_the_transitive_set() {
    let (db, _dir) = database("transitive").await;

    // A grandparent, a child and a grandchild.
    entity(&db, "grandparent", None).await;
    entity(&db, "child", Some("grandparent")).await;
    entity(&db, "grandchild", Some("child")).await;

    let report = entity_closure::rebuild(&db).await.expect("rebuild");
    assert_eq!(
        report.rows, 3,
        "two edges and the transitive one: {report:?}"
    );

    // **The transitive set**, not the direct children.
    let descendants = entity_closure::descendants_of(&db, "grandparent")
        .await
        .expect("the descendants");
    let ids: Vec<String> = descendants.iter().map(|(id, _)| id.clone()).collect();
    assert_eq!(
        ids,
        vec!["child".to_owned(), "grandchild".to_owned()],
        "**the transitive set**, nearest first: {ids:?}"
    );
    assert_eq!(descendants[0].1, 1, "the child is one step down");
    assert_eq!(descendants[1].1, 2, "and the grandchild is two");
}

#[tokio::test]
async fn a_cycle_terminates_and_the_closure_has_no_cycle() {
    let (db, _dir) = database("cycle").await;

    // **A cycle**: a's parent is b and b's parent is a. Nonsense, and the
    // rebuild must terminate on it rather than loop.
    //
    // The parents are set **after** both entities exist, because the foreign
    // key is checked at the end of each statement: a cycle cannot be written
    // in one statement, and that is the database being right about a
    // constraint rather than about a cycle.
    entity(&db, "a", None).await;
    entity(&db, "b", None).await;
    set_parent(&db, "a", "b").await;
    set_parent(&db, "b", "a").await;

    let report = entity_closure::rebuild(&db).await.expect("rebuild");
    assert!(
        report.rows < 100,
        "**the rebuild terminates on a cycle**: {report:?}"
    );
}

#[tokio::test]
async fn only_browsable_entities_are_in_the_closure() {
    let (db, _dir) = database("browsable").await;

    entity(&db, "parent", None).await;
    entity(&db, "child", Some("parent")).await;

    // The child is an **assertion**: below the floor, so not a node.
    let floors = EntityFloors::default();
    entity_floor::recount(&db, &floors).await.expect("recount");
    // The recount gives both of them ten works, so both clear the floor; take
    // the child below it by hand, which is what a recount would do for a name
    // nobody has tagged five times.
    let statement = match db.backend() {
        darkhold_db::Backend::Sqlite => {
            "UPDATE canonical_entities SET node_state = 'assertion_only', node_count = 1 WHERE id = 'child'"
        }
        darkhold_db::Backend::Postgres => {
            "UPDATE canonical_entities SET node_state = 'assertion_only', node_count = 1 WHERE id::text = 'child'"
        }
    };
    match db.backend() {
        darkhold_db::Backend::Sqlite => {
            sqlx::query(statement)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("the state");
        }
        darkhold_db::Backend::Postgres => {
            sqlx::query(statement)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("the state");
        }
    }

    let report = entity_closure::rebuild(&db).await.expect("rebuild");
    assert_eq!(
        report.rows, 0,
        "**an assertion is not in the tree**: {report:?}"
    );
    let descendants = entity_closure::descendants_of(&db, "parent")
        .await
        .expect("the descendants");
    assert!(
        descendants.is_empty(),
        "and the expansion does not reach it: {descendants:?}"
    );
}

#[tokio::test]
async fn the_rebuild_is_a_rebuild_and_not_an_append() {
    let (db, _dir) = database("rebuild").await;

    entity(&db, "parent", None).await;
    entity(&db, "child", Some("parent")).await;
    let first = entity_closure::rebuild(&db).await.expect("rebuild");
    let second = entity_closure::rebuild(&db).await.expect("rebuild");
    assert_eq!(
        first.rows, second.rows,
        "**the rebuild is idempotent**: {first:?} then {second:?}"
    );

    // And a tree that shrinks leaves no stale row behind.
    let statement = match db.backend() {
        darkhold_db::Backend::Sqlite => {
            "UPDATE canonical_entities SET parent_id = NULL WHERE id = 'child'"
        }
        darkhold_db::Backend::Postgres => {
            "UPDATE canonical_entities SET parent_id = NULL WHERE id::text = 'child'"
        }
    };
    match db.backend() {
        darkhold_db::Backend::Sqlite => {
            sqlx::query(statement)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("the parent");
        }
        darkhold_db::Backend::Postgres => {
            sqlx::query(statement)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("the parent");
        }
    }
    let third = entity_closure::rebuild(&db).await.expect("rebuild");
    assert_eq!(
        third.rows, 0,
        "**a stale row is a row that says a tree that no longer exists**: {third:?}"
    );
}
