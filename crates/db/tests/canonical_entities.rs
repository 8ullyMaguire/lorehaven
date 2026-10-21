//! M6-01: the taxonomy becomes the canonical entities — and the two tables
//! become one.
//!
//! # What this asserts
//!
//! * **The table is renamed**, and the data is not rewritten: a rename is a
//!   change of vocabulary and not of content (spec §5).
//! * **The two tables are merged.** The estate had `taxonomy_nodes` — the
//!   live vocabulary, what the tag browser reads and what `work_tags`
//!   references — *and* `canonical_entities`, which was the exchange's own
//!   parallel table. Migration 0082 saw the duplication and could not fix it:
//!   it put `review_status` and `signal_count` on the taxonomy's node, and
//!   the exchange kept writing to its own table. So the counts a signal
//!   reinforced never reached the node the review queue reads.
//! * **The counts add.** A name the exchange has seen five times and the
//!   taxonomy knows about is a name with eight signals, and the review
//!   queue's ordering *is* that count.
//! * **The columns are the specification's**: `slug`, `canonical_name`,
//!   `parent_id`, `work_count`, `review_status`, `signal_count` (spec §6.4).
//!
//! # Why the merge is the interesting part
//!
//! The rename is mechanical. The merge is not: it is the point of the card,
//! because **two tables describing one state in two shapes** is how a
//! "curated" label ends up meaning "exists in the new table" rather than "was
//! reviewed".

use std::path::PathBuf;

use darkhold_db::{Backend, Database};

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "darkhold-entities-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run a statement on whichever pool the dialect built.
async fn exec(db: &Database, sql: &str) -> Result<(), sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => sqlx::query(sql)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .map(|_| ()),
        Backend::Postgres => sqlx::query(sql)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .map(|_| ()),
    }
}

/// The names of the tables the instance has, from either dialect's catalogue.
async fn table_names(db: &Database) -> Vec<String> {
    let sql = match db.backend() {
        Backend::Sqlite => {
            "SELECT name FROM sqlite_master WHERE type = 'table' \
             AND name IN ('taxonomy_nodes', 'canonical_entities')"
        }
        Backend::Postgres => {
            "SELECT table_name FROM information_schema.tables \
             WHERE table_name IN ('taxonomy_nodes', 'canonical_entities')"
        }
    };
    match db.backend() {
        Backend::Sqlite => sqlx::query_scalar::<_, String>(sql)
            .fetch_all(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("the tables"),
        Backend::Postgres => sqlx::query_scalar::<_, String>(sql)
            .fetch_all(db.postgres_pool().expect("postgres"))
            .await
            .expect("the tables"),
    }
}

/// The columns of the canonical entities table, from either dialect.
async fn column_names(db: &Database) -> Vec<String> {
    let sql = match db.backend() {
        Backend::Sqlite => "SELECT name FROM pragma_table_info('canonical_entities')",
        Backend::Postgres => {
            "SELECT column_name FROM information_schema.columns \
             WHERE table_name = 'canonical_entities'"
        }
    };
    match db.backend() {
        Backend::Sqlite => sqlx::query_scalar::<_, String>(sql)
            .fetch_all(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("the columns"),
        Backend::Postgres => sqlx::query_scalar::<_, String>(sql)
            .fetch_all(db.postgres_pool().expect("postgres"))
            .await
            .expect("the columns"),
    }
}

/// One entity's row: the count, the review status, the display name and the
/// curator-set canonical form.
async fn entity_row(db: &Database, norm: &str) -> Option<(i64, String, String, Option<String>)> {
    // **`work_count` is an INTEGER on both dialects and an i64 here**, so the
    // PostgreSQL arm casts it: an INT4 decoded into an i64 is a decode error,
    // and the estate's dialect gate says so.
    let sql = match db.backend() {
        Backend::Sqlite => {
            "SELECT signal_count, review_status, canonical, canonical_name \
             FROM canonical_entities WHERE norm = ?"
        }
        Backend::Postgres => {
            // **The cast is not decoration.** `signal_count` is an INTEGER on
            // this dialect, which is an INT4, and an INT4 decoded into an i64
            // is a decode error. The estate's dialect gate says so, and the
            // SQLite arm needs no cast because SQLite has no INT4.
            "SELECT CAST(signal_count AS BIGINT) AS signal_count, review_status, canonical, canonical_name \
             FROM canonical_entities WHERE norm = $1"
        }
    };
    match db.backend() {
        Backend::Sqlite => sqlx::query_as::<_, (i64, String, String, Option<String>)>(sql)
            .bind(norm)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("the row"),
        Backend::Postgres => sqlx::query_as::<_, (i64, String, String, Option<String>)>(sql)
            .bind(norm)
            .fetch_optional(db.postgres_pool().expect("postgres"))
            .await
            .expect("the row"),
    }
}

/// A database with the migrations applied, and the two tables' rows in them.
async fn database_with_both_vocabularies(tag: &str) -> (Database, PathBuf) {
    let dir = scratch_dir(tag);
    // On PostgreSQL the database is **scoped and empty**: this test is about
    // migration 0124, and it has to start from the state that migration
    // merges. The clone path in the harness is the fast one and it is the
    // wrong one here -- the template is fully migrated, and the state 0124
    // merges is a schema no instance has any more.
    let db = match std::env::var("DARKHOLD_TEST_PG_URL") {
        Ok(admin_url) if !admin_url.is_empty() => {
            // **A name per call.** The three tests here run at the same time,
            // and a shared name means one of them drops the database another
            // is migrating.
            static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let name = format!(
                "lh_test_entities_{}_{}",
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

    // The migrations **up to 0123** — the state before the merge.
    darkhold_db::migrate::apply_to(&db, "0123_unit_spoiler_gate")
        .await
        .expect("migrate");

    // A node in the taxonomy's vocabulary, created by a human and curated.
    //
    // The id is a literal, because the statement is fixed for both dialects
    // and a `?1` that survives the arm is exactly the class of bug the
    // estate's own dialect gate is about.
    let taxonomy_id = uuid::Uuid::new_v4();
    let insert_taxonomy = format!(
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at, review_status, signal_count)
         VALUES ('{taxonomy_id}', 'tag', 'Slow Burn', 'slow burn', '2026-01-01T00:00:00Z', 'curated', 3)"
    );
    exec(&db, &insert_taxonomy)
        .await
        .expect("insert a taxonomy node");

    // The same name in the exchange's own vocabulary, with its own count --
    // and a name the taxonomy has never seen.
    let insert_exchange = match db.backend() {
        Backend::Sqlite => {
            "INSERT INTO canonical_entities (kind, norm, canonical, review_status, signal_count, created_at, updated_at)
             VALUES ('tag', 'slow burn', 'Slow Burn', 'unverified', 5, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
                    ('tag', 'quiet', 'Quiet', 'unverified', 2, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        }
        Backend::Postgres => {
            "INSERT INTO canonical_entities (kind, norm, canonical, review_status, signal_count, created_at, updated_at)
             VALUES ('tag', 'slow burn', 'Slow Burn', 'unverified', 5, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
                    ('tag', 'quiet', 'Quiet', 'unverified', 2, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        }
    };
    exec(&db, insert_exchange)
        .await
        .expect("insert the exchange's entities");

    // And the last migration, which merges them.
    darkhold_db::migrate::apply_to(&db, "0124_canonical_entities")
        .await
        .expect("migrate");

    (db, dir)
}

#[tokio::test]
async fn the_two_vocabularies_are_one_table_after_the_merge() {
    let (db, _dir) = database_with_both_vocabularies("merge").await;

    // **The taxonomy's table is gone, and the exchange's table has the
    // taxonomy's name.** One table, not two.
    let mut tables = table_names(&db).await;
    tables.sort();
    assert_eq!(
        tables,
        vec!["canonical_entities".to_owned()],
        "**one table, and it is the canonical one**: {tables:?}"
    );
}

#[tokio::test]
async fn the_counts_add_and_the_curated_state_wins() {
    let (db, _dir) = database_with_both_vocabularies("counts").await;

    // **The counts add**: three from the taxonomy's row and five from the
    // exchange's, for the same name.
    let (count, review_status, canonical, _) =
        entity_row(&db, "slow burn").await.expect("the merged row");
    assert_eq!(count, 8, "three plus five is eight signals: {count}");
    assert_eq!(
        review_status, "curated",
        "**and the curated state wins**: a curator's word is the one that counts"
    );
    assert_eq!(
        canonical, "Slow Burn",
        "and the display name is the one a curator chose"
    );

    // **A name only the exchange knew** is in the table too, with its count
    // and its canonical form.
    let (count, _status, _canonical, canonical_name) =
        entity_row(&db, "quiet").await.expect("the exchange's row");
    assert_eq!(count, 2, "its own count is carried over: {count}");
    assert_eq!(
        canonical_name.as_deref(),
        Some("Quiet"),
        "and the curator-set canonical form is carried over: {canonical_name:?}"
    );
}

#[tokio::test]
async fn the_new_columns_are_there_and_the_data_survived() {
    let (db, _dir) = database_with_both_vocabularies("columns").await;

    // **The specification's columns** (spec §6.4).
    let columns = column_names(&db).await;
    for wanted in [
        "id",
        "kind",
        "canonical",
        "canonical_name",
        "slug",
        "parent_id",
        "review_status",
        "signal_count",
        "work_count",
    ] {
        assert!(
            columns.iter().any(|column| column == wanted),
            "**the column `{wanted}` is there**: {columns:?}"
        );
    }

    // **And the data survived**: the taxonomy's row kept its identity, its
    // spelling and its kind. A rename is a change of vocabulary and not of
    // content.
    let (count, status, canonical, _) = entity_row(&db, "slow burn").await.expect("the row");
    assert_eq!(count, 8, "with its count");
    assert_eq!(status, "curated", "and its state");
    assert_eq!(canonical, "Slow Burn", "and its spelling");
}
