//! The PostgreSQL gate for `taxonomy_export` — the bug the SQLite tests could not see.
//!
//! `work_count` is `INT4` in PostgreSQL and the row decodes it into `i64`. SQLite hands back whatever it
//! stored so every unit test passed; on the live instance the export answered 500 with
//! `mismatched types; Rust type i64 (as SQL type INT8) is not compatible with SQL type INT4`, caught only
//! because the route was called against the deployed database.
//!
//! This file is the register that would have caught it, and it is read-only: it counts and pages the
//! vocabulary, and writes nothing.
//!
//! ```bash
//! DARKHOLD_TEST_PG_URL='postgres://…' cargo test -p darkhold-db --test taxonomy_export_postgres -- --ignored --nocapture
//! ```

use darkhold_db::{Database, DatabaseConfig};
use std::time::Duration;

fn database_config(url: String) -> DatabaseConfig {
    DatabaseConfig {
        url,
        max_connections: 2,
        acquire_timeout: Duration::from_secs(10),
        slow_query_warn: Duration::ZERO,
    }
}

/// An `#[ignore]`d test is only run on purpose, so a missing URL is a **failure**, not a skip.
///
/// This file was written after a live-only decode bug, and the first version of it passed against an unset
/// variable by returning early — a green run over nothing, which is the exact trap the gate exists to close.
async fn connect() -> Database {
    let url = std::env::var("DARKHOLD_TEST_PG_URL").expect(
        "DARKHOLD_TEST_PG_URL must be set to a PostgreSQL catalogue (running an #[ignore]d gate is an          explicit act; skipping it would report success for work that never happened)",
    );
    Database::connect(&database_config(url))
        .await
        .expect("connect to DARKHOLD_TEST_PG_URL")
}

#[tokio::test]
#[ignore = "needs DARKHOLD_TEST_PG_URL and a catalogue"]
async fn a_page_decodes_on_postgres() {
    let db = connect().await;

    let version = darkhold_db::taxonomy_export::version(&db)
        .await
        .expect("version on postgres");
    assert!(version.contains('@'), "version: {version}");

    let (items, total) = darkhold_db::taxonomy_export::page(&db, None, 5, 0)
        .await
        .expect("page decodes on postgres");
    assert_eq!(items.len(), 5.min(total as usize));

    // A row that decoded is a row with a number in it: `work_count` is the INT4 column, and zero is a
    // legitimate value, so assert the type decoded rather than the value.
    for item in &items {
        assert!(!item.canonical.is_empty());
        let _work_count: i64 = item.work_count;
    }

    let (_, tags) = darkhold_db::taxonomy_export::page(&db, Some("tag"), 2, 0)
        .await
        .expect("kind filter on postgres");
    assert!(tags <= total);

    let aliases = darkhold_db::taxonomy_export::aliases(&db)
        .await
        .expect("aliases on postgres");
    eprintln!("postgres: {total} entities, {} aliases, version {version}", aliases.len());
}
