//! M6-02: the entity floor (spec §6.4, R-23).
//!
//! # What this asserts
//!
//! * **A tag on one work is searchable and countable, and it is not a node in
//!   the tree.** The assertion is stored; the *node* is not. That distinction
//!   is the card's own: *"the assertion is still stored, still searchable and
//!   still countable, but it is not a node in the tree."*
//! * **The browse's total is the count above the floor**, not the count of the
//!   rows in the table. A reader who sees "12 of 900" learns something, and a
//!   reader who sees "900" learns nothing.
//! * **The floor is per kind**, and the default is five works.
//! * **The recount is idempotent** — running it twice changes nothing the
//!   second time, which is what makes it a bin's job and not a script.
//!
//! # Why the floor is not optional
//!
//! 97 % of the freeform namespace is singletons: 587,258 of 634,190 names are
//! on exactly one work. Without a floor, "browse the taxonomy" is 634,231
//! rows of one-off user phrases, and every count on the page is a count of
//! one.

use std::collections::BTreeMap;

use darkhold_db::entity_floor::{self, EntityFloors, NodeState, DEFAULT_FLOOR};
use darkhold_db::Database;

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "darkhold-floor-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// A database with the migrations applied.
async fn database(tag: &str) -> (Database, std::path::PathBuf) {
    let dir = scratch_dir(tag);
    let db = match std::env::var("DARKHOLD_TEST_PG_URL") {
        Ok(admin_url) if !admin_url.is_empty() => {
            static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let name = format!(
                "lh_test_floor_{}_{}",
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

/// Create a work and a tag on it, and return the tag's node id.
async fn tagged_work(db: &Database, title: &str, tag: &str, works: usize) -> String {
    let owner = uuid::Uuid::new_v4().to_string();
    let work = uuid::Uuid::new_v4().to_string();
    let node = uuid::Uuid::new_v4().to_string();

    let statements = vec![
        format!(
            "INSERT INTO accounts (id, email, created_at, updated_at) VALUES ('{owner}', 'floor-{owner}@x.test', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        ),
        format!(
            "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) VALUES ('{owner}', '{owner}', 'floor{owner}', 'Floor', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        ),
        format!(
            "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at) VALUES ('{work}', '{owner}', '{title}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        ),
        format!(
            "INSERT INTO canonical_entities (id, kind, canonical, norm, created_at, review_status, signal_count, node_state) VALUES ('{node}', 'tag', '{tag}', '{tag}', '2026-01-01T00:00:00Z', 'curated', 0, 'browsable')"
        ),
    ];
    for statement in statements {
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
        result.expect("the statement");
    }

    // The tag on the work, `works` times -- the same work for the first, and
    // more works for the rest, so the count is the number of works.
    for index in 0..works {
        let tagged = if index == 0 {
            work.clone()
        } else {
            let extra = uuid::Uuid::new_v4().to_string();
            let statement = format!(
                "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at) VALUES ('{extra}', '{owner}', 'Extra {index}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
            );
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
            result.expect("the extra work");
            extra
        };
        let statement = format!(
            "INSERT INTO work_tags (work_id, node_id, weight, added_at) VALUES ('{tagged}', '{node}', 1, '2026-01-01T00:00:00Z')"
        );
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
        result.expect("the tag");
    }

    node
}

#[tokio::test]
async fn a_tag_on_one_work_is_searchable_and_countable_and_not_a_node() {
    let (db, _dir) = database("one").await;
    let floors = EntityFloors::default();

    // A tag on ONE work, and a tag on SIX.
    let lonely = tagged_work(&db, "Lonely", "lonely-tag", 1).await;
    let popular = tagged_work(&db, "Popular", "popular-tag", 6).await;

    let report = entity_floor::recount(&db, &floors).await.expect("recount");
    assert_eq!(report.counted, 2, "two entities were counted: {report:?}");

    // **The lonely tag is an assertion, not a node.**
    let (state, count) = entity_floor::state_of(&db, &lonely)
        .await
        .expect("the state")
        .expect("the entity");
    assert_eq!(
        state,
        NodeState::AssertionOnly,
        "one work is below the floor"
    );
    assert_eq!(count, 1, "and it is still counted: {count}");

    // **The popular tag is a node.**
    let (state, count) = entity_floor::state_of(&db, &popular)
        .await
        .expect("the state")
        .expect("the entity");
    assert_eq!(state, NodeState::Browsable, "six works clears the floor");
    assert_eq!(count, 6, "and the count is right: {count}");

    // **And the browse shows only the node.**
    let (page, total) = entity_floor::browsable(&db, "tag", 50, 0)
        .await
        .expect("the browse");
    assert_eq!(
        total, 1,
        "**the total is the count above the floor**: {total}"
    );
    assert_eq!(page.len(), 1, "and the page holds the one node: {page:?}");
    assert_eq!(page[0].1, "popular-tag", "which is the popular one");
}

#[tokio::test]
async fn the_assertion_is_still_searchable() {
    let (db, _dir) = database("searchable").await;
    let floors = EntityFloors::default();
    let lonely = tagged_work(&db, "Lonely", "lonely-tag", 1).await;
    entity_floor::recount(&db, &floors).await.expect("recount");

    // **The floor decides what appears in the TREE, and not what the instance
    // knows.** The assertion is still searchable, and a reader who searched
    // for the tag finds the work that carries it.
    let found = darkhold_db::taxonomy::search_nodes(&db, Some("tag"), "lonely", 10)
        .await
        .expect("the search");
    assert!(
        found.iter().any(|node| node.id == lonely),
        "**the assertion is still searchable**: {found:?}"
    );
}

#[tokio::test]
async fn the_floor_is_per_kind() {
    let (db, _dir) = database("perkind").await;
    // A warning is worth a node at one work and a freeform tag is not worth
    // one at one work, so the kinds do not share a floor by default.
    let floors = EntityFloors {
        default: DEFAULT_FLOOR,
        by_kind: BTreeMap::from([("warning".to_owned(), 1)]),
    };
    assert_eq!(floors.for_kind("warning"), 1);
    assert_eq!(floors.for_kind("tag"), DEFAULT_FLOOR);

    // A warning with one work clears ITS floor.
    let warning = tagged_work(&db, "Warned", "a-warning", 1).await;
    let sql = match db.backend() {
        darkhold_db::Backend::Sqlite => {
            "UPDATE canonical_entities SET kind = 'warning' WHERE id = ?"
        }
        darkhold_db::Backend::Postgres => {
            "UPDATE canonical_entities SET kind = 'warning' WHERE id::text = $1"
        }
    };
    match db.backend() {
        darkhold_db::Backend::Sqlite => {
            sqlx::query(sql)
                .bind(&warning)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("the kind");
        }
        darkhold_db::Backend::Postgres => {
            sqlx::query(sql)
                .bind(&warning)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("the kind");
        }
    }
    entity_floor::recount(&db, &floors).await.expect("recount");
    let (state, _) = entity_floor::state_of(&db, &warning)
        .await
        .expect("the state")
        .expect("the entity");
    assert_eq!(
        state,
        NodeState::Browsable,
        "one work clears the warning's floor of one"
    );
}

#[tokio::test]
async fn the_recount_is_idempotent() {
    let (db, _dir) = database("idempotent").await;
    let floors = EntityFloors::default();
    tagged_work(&db, "Lonely", "lonely-tag", 1).await;
    tagged_work(&db, "Popular", "popular-tag", 6).await;

    let first = entity_floor::recount(&db, &floors).await.expect("recount");
    let second = entity_floor::recount(&db, &floors).await.expect("recount");
    assert_eq!(
        first.counted, second.counted,
        "**the recount is idempotent**: {first:?} then {second:?}"
    );
    assert_eq!(
        first.browsable, second.browsable,
        "and the states do not move: {first:?} then {second:?}"
    );
}
