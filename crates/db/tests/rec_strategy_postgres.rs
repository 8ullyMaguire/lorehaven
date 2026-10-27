//! The PostgreSQL gate for `rec_strategy` — the bug CI on SQLite cannot see.
//!
//! Every strategy query decodes its id column into `String`. On SQLite an id *is* text, so
//! the whole module passed its integration test while being completely dead on PostgreSQL:
//! `mismatched types; Rust type String (as SQL type TEXT) is not compatible with SQL type
//! UUID`, surfacing as a 500 on `/discover` for every signed-in reader and as a shadow
//! comparison that could never run.
//!
//! The register that would have caught it is a two-engine run of the strategies themselves,
//! which is what this file is. It is `#[ignore]`d and env-gated because it needs a real
//! PostgreSQL with a real catalogue in it, and the queries are **read-only** — the run against
//! a live instance inspects rankings and writes nothing.
//!
//! ```bash
//! DARKHOLD_TEST_PG_URL='postgres://…' cargo test -p darkhold-db --test rec_strategy_postgres -- --ignored --nocapture
//! ```
//!
//! Set `DARKHOLD_REC_STRATEGY_ACCOUNT` to an account id with history in the catalogue. Without
//! it the test picks the account holding the most bookmarks, which is what makes the run
//! meaningful: an account with no history exercises none of the queries' interesting paths.

use darkhold_db::rec_strategy::{default_strategies, RecContext, RecRegistry};
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

/// The account to run the strategies for: the configured one, or the busiest reader.
///
/// "Busiest" falls back to any account at all, because the gate is about the *decode* and not
/// about personalisation: a strategies run for a reader with no history still executes the
/// catalogue-wide queries, and those are the ones with the uuid column in them.
async fn account_with_history(db: &Database) -> String {
    if let Ok(account) = std::env::var("DARKHOLD_REC_STRATEGY_ACCOUNT") {
        return account;
    }
    let sql = db.sql(
        "SELECT account_id FROM bookmarks GROUP BY account_id ORDER BY COUNT(*) DESC LIMIT 1",
        "SELECT account_id::text FROM bookmarks GROUP BY account_id ORDER BY COUNT(*) DESC LIMIT 1",
    );
    let row: Option<(String,)> = match db.backend() {
        darkhold_db::Backend::Sqlite => {
            sqlx::query_as(&sql)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("the bookmarks table must be readable")
        }
        darkhold_db::Backend::Postgres => {
            sqlx::query_as(&sql)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await
                .expect("the bookmarks table must be readable")
        }
    };
    if let Some((account,)) = row {
        return account;
    }
    // No bookmarks at all: take any account, so the catalogue-wide strategies still run.
    let fallback = db.sql(
        "SELECT id FROM accounts ORDER BY created_at LIMIT 1",
        "SELECT id::text FROM accounts ORDER BY created_at LIMIT 1",
    );
    let row: Option<(String,)> = match db.backend() {
        darkhold_db::Backend::Sqlite => {
            sqlx::query_as(&fallback)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("the accounts table must be readable")
        }
        darkhold_db::Backend::Postgres => {
            sqlx::query_as(&fallback)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await
                .expect("the accounts table must be readable")
        }
    };
    row.expect("the catalogue has no accounts at all").0
}

#[tokio::test]
#[ignore = "needs a real PostgreSQL catalogue; run explicitly with DARKHOLD_TEST_PG_URL set"]
async fn every_strategy_decodes_on_postgres() {
    let url = match std::env::var("DARKHOLD_TEST_PG_URL") {
        Ok(url) if !url.is_empty() => url,
        _ => {
            println!("skipped: DARKHOLD_TEST_PG_URL is not set");
            return;
        }
    };

    let db = Database::connect(&database_config(url))
        .await
        .expect("connect to PostgreSQL");
    assert_eq!(
        db.backend(),
        darkhold_db::Backend::Postgres,
        "this gate is meaningless on SQLite"
    );

    let account = account_with_history(&db).await;
    println!("account: {account}");

    let factories = default_strategies();
    let mut names: Vec<&String> = factories.keys().collect();
    names.sort();

    let mut failures: Vec<String> = Vec::new();
    let mut produced: Vec<String> = Vec::new();
    let mut rows_reached_the_decoder = 0usize;

    for name in names {
        let factory = factories.get(name).expect("named strategy");
        let mut registry = RecRegistry::new(60.0);
        registry.register(name.clone(), (factory)());
        let context = RecContext {
            account_id: account.clone(),
            seen: Vec::new(),
            cap: 5,
        };

        match registry.generate(&db, context).await {
            Ok(ids) => {
                // A decode failure is the bug; an empty result is not. Several strategies
                // legitimately return nothing on a catalogue with no curator marks or no
                // reading history, and treating that as a failure would make this gate
                // pass only on one particular database.
                for id in &ids {
                    if uuid::Uuid::parse_str(id).is_err() {
                        failures.push(format!("{name}: produced a non-uuid id {id:?}"));
                    }
                }
                if !ids.is_empty() {
                    rows_reached_the_decoder += 1;
                }
                produced.push(format!("{name}: {} ids", ids.len()));
            }
            Err(error) => failures.push(format!("{name}: {error}")),
        }
    }

    for line in &produced {
        println!("  ok  {line}");
    }
    assert!(
        failures.is_empty(),
        "{} strateg(ies) failed on PostgreSQL:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
    // A run where every strategy returned zero rows proves nothing at all: the decoder is
    // never reached, and the exact bug this file exists for would report a clean pass. So a
    // pass has to include at least one non-empty result.
    assert!(
        rows_reached_the_decoder > 0,
        "every strategy returned zero rows, so no id was ever decoded — this gate did not \
         exercise the thing it is for"
    );
}
