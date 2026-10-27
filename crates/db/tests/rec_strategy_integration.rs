//! M52-05 integration: recommendation strategies produce ranked results.

use darkhold_db::rec_strategy::{
    author_graph_strategy, bandit_strategy, completion_weight_strategy, cooccurrence_strategy,
    curator_prior_strategy, hidden_classics_strategy, sequential_strategy, tag_graph_strategy,
    time_decay_strategy, RecContext, RecRegistry, RecStrategyFn,
};
use darkhold_db::Database;
use std::time::Duration;

fn make_config(url: String) -> darkhold_db::DatabaseConfig {
    darkhold_db::DatabaseConfig {
        url,
        max_connections: 5,
        acquire_timeout: Duration::from_secs(5),
        slow_query_warn: Duration::ZERO,
    }
}

/// Create a unique temp directory for an isolated test database.
fn temp_db_dir() -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("darkhold-test-{}", uuid::Uuid::new_v4()));
    dir
}

async fn setup_db() -> (Database, std::path::PathBuf) {
    let dir = temp_db_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let url = format!("sqlite://{}/darkhold.sqlite?mode=rwc", dir.display());
    let config = make_config(url);
    let db = Database::connect(&config).await.unwrap();
    db.migrate().await.unwrap();
    (db, dir)
}

fn cleanup(dir: &std::path::Path) {
    let _ = std::fs::remove_dir_all(dir);
}

fn ctx(account_id: &str) -> RecContext {
    RecContext {
        account_id: account_id.to_string(),
        seen: vec![],
        cap: 10,
    }
}

#[tokio::test]
async fn registry_with_cooccurrence_empty_for_no_bookmarks() {
    let (db, dir) = setup_db().await;
    let mut reg = RecRegistry::new(60.0);
    reg.register("cooccurrence", cooccurrence_strategy());
    let result = reg.generate(&db, ctx("user-1")).await.unwrap();
    cleanup(&dir);
    assert!(result.is_empty());
}

#[tokio::test]
async fn registry_with_tag_graph_empty_for_no_bookmarks() {
    let (db, dir) = setup_db().await;
    let mut reg = RecRegistry::new(60.0);
    reg.register("tag_graph", tag_graph_strategy());
    let result = reg.generate(&db, ctx("user-1")).await.unwrap();
    cleanup(&dir);
    assert!(result.is_empty());
}

#[tokio::test]
async fn registry_with_author_graph_empty_for_no_bookmarks() {
    let (db, dir) = setup_db().await;
    let mut reg = RecRegistry::new(60.0);
    reg.register("author_graph", author_graph_strategy());
    let result = reg.generate(&db, ctx("user-1")).await.unwrap();
    cleanup(&dir);
    assert!(result.is_empty());
}

#[tokio::test]
async fn registry_with_time_decay_empty_for_no_bookmarks() {
    let (db, dir) = setup_db().await;
    let mut reg = RecRegistry::new(60.0);
    reg.register("time_decay", time_decay_strategy());
    let result = reg.generate(&db, ctx("user-1")).await.unwrap();
    cleanup(&dir);
    assert!(result.is_empty());
}

#[tokio::test]
async fn registry_with_completion_weight_empty_for_no_works() {
    let (db, dir) = setup_db().await;
    let mut reg = RecRegistry::new(60.0);
    reg.register("completion_weight", completion_weight_strategy());
    let result = reg.generate(&db, ctx("user-1")).await.unwrap();
    cleanup(&dir);
    assert!(result.is_empty());
}

#[tokio::test]
async fn registry_with_curator_prior_empty_for_no_bookmarks() {
    let (db, dir) = setup_db().await;
    let mut reg = RecRegistry::new(60.0);
    reg.register("curator_prior", curator_prior_strategy());
    let result = reg.generate(&db, ctx("user-1")).await.unwrap();
    cleanup(&dir);
    assert!(result.is_empty());
}

#[tokio::test]
async fn registry_with_bandit_empty_for_no_works() {
    let (db, dir) = setup_db().await;
    let mut reg = RecRegistry::new(60.0);
    reg.register("bandit", bandit_strategy());
    let result = reg.generate(&db, ctx("user-1")).await.unwrap();
    cleanup(&dir);
    assert!(result.is_empty());
}

#[tokio::test]
async fn registry_blends_all_strategies_without_error() {
    let (db, dir) = setup_db().await;
    let mut reg = RecRegistry::new(60.0);
    reg.register("cooccurrence", cooccurrence_strategy());
    reg.register("tag_graph", tag_graph_strategy());
    reg.register("author_graph", author_graph_strategy());
    reg.register("time_decay", time_decay_strategy());
    reg.register("completion_weight", completion_weight_strategy());
    reg.register("curator_prior", curator_prior_strategy());
    reg.register("bandit", bandit_strategy());
    let result = reg.generate(&db, ctx("user-1")).await.unwrap();
    cleanup(&dir);
    assert!(result.is_empty());
}

// =====================================================================================
// T-R6-02: the suite must seed rows, not only assert emptiness.
//
// Every test above runs against an empty database, so the file is **equally green when a query is
// broken**. That is what happened: nine strategies were dead on PostgreSQL — a uuid decoded as
// text, a boolean compared to an integer, `ORDER BY` outside a `DISTINCT`, `julianday()`, an INT4
// decoded as i64 — while this file passed. Worse, seeding it found two strategies that could not
// return a row on ANY database: `cooccurrence` and `time_decay` joined `bookmarks` to itself on
// `account_id`, pairing a reader's shelf with the same reader's shelf and then excluding everything
// on it. Both are fixed cross-reader; this test is what would have caught them.
//
// The empty-state tests stay: an empty catalogue must return empty rather than error.
// =====================================================================================

const SCHEMA_DB: &str = "darkhold_rs2";
const NOW: &str = "2026-10-27T12:00:00Z";

/// A scratch database on whichever backend `DARKHOLD_TEST_PG_URL` names.
///
/// PostgreSQL: one schema inside the shared `darkhold_rs2` database, with `search_path` scoped
/// through the connection string. SQLite: a unique temp file. Same shape as
/// `crates/db/tests/work_ratings.rs`, which copies `continue_reading_t1.rs`; sqlx does not translate
/// `?1` to `$1`, so fixtures are written once and cast per arm.
async fn connect(tag: &str) -> (Database, Option<String>) {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "darkhold-rs-{}-{}-{}",
        tag,
        uuid::Uuid::new_v4(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");

    let mut schema = None;
    let url = match std::env::var("DARKHOLD_TEST_PG_URL") {
        Ok(admin) => {
            let admin_db = Database::connect(&make_config(admin.clone()))
                .await
                .expect("connect to the admin database");
            let pool = admin_db.postgres_pool().expect("postgres pool");
            let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM pg_database WHERE datname = $1")
                .bind(SCHEMA_DB)
                .fetch_one(pool)
                .await
                .unwrap_or(None);
            if exists.is_none() {
                // Two test binaries can both see "absent"; the loser's error is the outcome, not a
                // fault.
                let _ = sqlx::query(&format!("CREATE DATABASE {SCHEMA_DB}"))
                    .execute(pool)
                    .await;
            }
            admin_db.close().await;

            let (prefix, _) = admin.rsplit_once('/').expect("the admin URL ends in a database");
            let scratch = format!("{prefix}/{SCHEMA_DB}");
            let scratch_db = Database::connect(&make_config(scratch.clone()))
                .await
                .expect("connect to the scratch database");
            let s = format!("rs_{}_{}", std::process::id(), uuid::Uuid::new_v4().simple());
            sqlx::query(&format!("CREATE SCHEMA {s}"))
                .execute(scratch_db.postgres_pool().expect("postgres pool"))
                .await
                .expect("create a scratch schema");
            scratch_db.close().await;
            schema = Some(s.clone());
            format!("{scratch}?options=-csearch_path%3D{s}")
        }
        Err(_) => format!("sqlite://{}/darkhold.sqlite?mode=rwc", dir.display()),
    };

    let db = Database::connect(&make_config(url)).await.expect("connect");
    // One migrator at a time per process: 0092 runs CREATE EXTENSION IF NOT EXISTS pgcrypto, which
    // is database-scoped while schemas are not, so parallel migrators collide on the extension.
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let guard = LOCK.lock().await;
    db.migrate().await.expect("migrate a scratch schema");
    drop(guard);
    (db, schema)
}

async fn drop_schema(schema: &str) {
    let Ok(admin) = std::env::var("DARKHOLD_TEST_PG_URL") else {
        return;
    };
    let (prefix, _) = admin.rsplit_once('/').expect("the admin URL ends in a database");
    let Ok(db) = Database::connect(&make_config(format!("{prefix}/{SCHEMA_DB}"))).await else {
        return;
    };
    let _ = sqlx::query(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .execute(db.postgres_pool().expect("postgres pool"))
        .await;
    db.close().await;
}

/// One statement, with `?N#u` for a native uuid, `?N#b` for a boolean and `?N#t` for text.
///
/// `#b` exists because SQLite stores a boolean as 0/1 and accepts the string "1" in that column,
/// while PostgreSQL rejects `expression is of type text` for a boolean target. Binding "1"/"0" with
/// `::boolean` on the PostgreSQL arm says the same thing to both.
async fn exec(db: &Database, tmpl: &str, args: &[&str]) {
    match db.backend() {
        darkhold_db::Backend::Sqlite => {
            let sql = tmpl.replace("#u", "").replace("#b", "").replace("#t", "");
            let mut q = sqlx::query(&sql);
            for a in args {
                q = q.bind(*a);
            }
            q.execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("fixture insert");
        }
        darkhold_db::Backend::Postgres => {
            let pg = (1..=12).fold(tmpl.to_string(), |acc, n| {
                acc.replace(&format!("?{n}#u"), &format!("${n}::uuid"))
                    .replace(&format!("?{n}#b"), &format!("${n}::boolean"))
                    .replace(&format!("?{n}#t"), &format!("${n}::text"))
                    .replace(&format!("?{n}"), &format!("${n}"))
            });
            let mut q = sqlx::query(&pg);
            for a in args {
                q = q.bind(*a);
            }
            q.execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("fixture insert");
        }
    }
}

/// The fixture: three accounts, five works, and enough of each signal for every strategy.
///
/// Written as a struct of ids so an assertion can name the work it expected rather than a length.
struct Seeded {
    reader: String,
    work_shared: String,
    work_co_bookmarked: String,
    work_curator_public: String,
    work_in_progress: String,
    work_viewed: String,
}

async fn seed(db: &Database) -> Seeded {
    let account = |tag: &str, curator: bool| (uuid::Uuid::new_v4().to_string(), tag.to_string(), curator);
    let reader = account("reader", false).0;
    let other = account("other", false).0;
    let curator = account("curator", true).0;

    for (id, tag, is_curator) in [
        (reader.clone(), "reader", false),
        (other.clone(), "other", false),
        (curator.clone(), "curator", true),
    ] {
        exec(
            db,
            "INSERT INTO accounts (id, email, is_curator, created_at, updated_at)
             VALUES (?1#u, ?2, ?3#b, ?4, ?4)",
            &[
                &id,
                &format!("{tag}-{}@example.test", &id[..8]),
                if is_curator { "1" } else { "0" },
                NOW,
            ],
        )
        .await;
    }

    // One pseud each: reading history and works both hang off a pseud, not an account.
    let mut pseuds = std::collections::HashMap::new();
    for (account_id, tag) in [
        (&reader, "reader"),
        (&other, "other"),
        (&curator, "curator"),
    ] {
        let pseud = uuid::Uuid::new_v4().to_string();
        exec(
            db,
            "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
             VALUES (?1#u, ?2#u, ?3, ?3, ?4, ?4)",
            &[&pseud, account_id, &format!("{tag}person"), NOW],
        )
        .await;
        pseuds.insert(account_id.clone(), pseud);
    }
    let other_pseud = pseuds.get(&other).expect("the other reader's pseud").clone();
    let reader_pseud = pseuds.get(&reader).expect("the reader's pseud").clone();
    let curator_pseud = pseuds.get(&curator).expect("the curator's pseud").clone();

    // W1 the reader shelved; W2 shares its tag and its author; W3 is bookmarked publicly by a
    // curator; W4 is in progress on the reader's history; W5 has aggregate views.
    let works = [
        (uuid::Uuid::new_v4().to_string(), "W1 shared", "complete"),
        (uuid::Uuid::new_v4().to_string(), "W2 co-bookmarked", "complete"),
        (uuid::Uuid::new_v4().to_string(), "W3 curator public", "complete"),
        (uuid::Uuid::new_v4().to_string(), "W4 in progress", "in_progress"),
        (uuid::Uuid::new_v4().to_string(), "W5 viewed", "complete"),
    ];
    let owners = [
        other_pseud.clone(),
        other_pseud.clone(),
        curator_pseud.clone(),
        other_pseud.clone(),
        other_pseud.clone(),
    ];
    for ((id, title, completion), owner) in works.iter().zip(owners.iter()) {
        exec(
            db,
            "INSERT INTO works (id, owner_pseud_id, title, lifecycle, completion, created_at,
                                updated_at, generated_content_posture)
             VALUES (?1#u, ?2#u, ?3, 'published', ?4, ?5, ?5, 'forbid')",
            &[id, owner, title, completion, NOW],
        )
        .await;
    }
    let w1 = works[0].0.clone();
    let w2 = works[1].0.clone();
    let w3 = works[2].0.clone();
    let w4 = works[3].0.clone();
    let w5 = works[4].0.clone();

    // Bookmarks: the reader shelved W1; the other reader shelved W1 and W2 (that pair is the
    // co-occurrence signal); the curator shelved W3 publicly.
    for (bookmarker, work, public) in [
        (&reader, &w1, "0"),
        (&other, &w1, "0"),
        (&other, &w2, "0"),
        (&curator, &w3, "1"),
    ] {
        exec(
            db,
            "INSERT INTO bookmarks (id, account_id, subject_type, subject_id, note, is_public,
                                    created_at, updated_at, version)
             VALUES (?1#u, ?2#u, 'work', ?3#u, '', ?4#b, ?5, ?5, 1)",
            &[&uuid::Uuid::new_v4().to_string(), bookmarker, work, public, NOW],
        )
        .await;
    }

    // One tag shared by W1 and W2, so the tag graph has an edge to walk.
    for work in [&w1, &w2] {
        exec(
            db,
            "INSERT INTO work_tags (work_id, node_id, prominence, added_at, confirmation, role,
                                    source, confidence, status)
             VALUES (?1#u, 'node-shared', 100, ?2, 'unconfirmed', 'primary', 'author', 1.0, 'applied')",
            &[work, NOW],
        )
        .await;
    }

    // The reader's history: W4 is in progress and not bookmarked, so `sequential` can offer it.
    exec(
        db,
        "INSERT INTO reading_history_entry (id, account_id, pseud_id, subject_type, subject_id,
                                            last_read_at, created_at)
         VALUES (?1#u, ?2#u, ?3#u, 'work', ?4#u, ?5, ?5)",
        &[
            &uuid::Uuid::new_v4().to_string(),
            &reader,
            &reader_pseud,
            &w4,
            NOW,
        ],
    )
    .await;

    // Views and a completion for W3, which is what `hidden_classics` needs: MIN_READERS_FOR_A_RATE
    // distinct readers and at least one finished read.
    for index in 0..12 {
        exec(
            db,
            "INSERT INTO work_view_log (work_id, viewer_hash, viewed_at, is_automated, kind, source,
                                        obscurity_at_read)
             VALUES (?1, ?2, ?3, 0, 'earned', 'unknown', 1.0)",
            &[&w3, &format!("hasher-{index}"), NOW],
        )
        .await;
    }
    exec(
        db,
        "INSERT INTO reading_status (id, account_id, subject_type, subject_id, status, updated_at,
                                     version)
         VALUES (?1#u, ?2#u, 'work', ?3#u, 'finished', ?4, 1)",
        &[&uuid::Uuid::new_v4().to_string(), &reader, &w3, NOW],
    )
    .await;

    // Aggregate views for the bandit, which orders by them.
    exec(
        db,
        "INSERT INTO work_metric_aggregates (work_id, views, complete_reads, reactions, kudos,
                                             bookmarks, collection_adds, reviews, updated_at)
         VALUES (?1#u, 42, 3, 1, 0, 0, 0, 0, ?2)",
        &[&w5, NOW],
    )
    .await;

    Seeded {
        reader,
        work_shared: w1,
        work_co_bookmarked: w2,
        work_curator_public: w3,
        work_in_progress: w4,
        work_viewed: w5,
    }
}

#[tokio::test]
async fn every_strategy_returns_rows_and_parseable_ids_on_a_seeded_catalogue() {
    let (db, schema) = connect("strategies").await;
    let engine = match db.backend() {
        darkhold_db::Backend::Sqlite => "sqlite",
        darkhold_db::Backend::Postgres => "postgres",
    };
    println!("engine: {engine}");

    let seeded = seed(&db).await;

    // Every registered strategy, in the order `default_strategies` builds them.
    let mut reg = RecRegistry::new(60.0);
    for (name, factory) in darkhold_db::rec_strategy::default_strategies() {
        reg.register(&name, factory());
    }

    let mut result = reg
        .generate(
            &db,
            RecContext {
                account_id: seeded.reader.clone(),
                seen: vec![],
                cap: 10,
            },
        )
        .await
        .expect("the registry ran");

    // A strategy that returns nothing on a catalogue seeded for it is broken, and the id it would
    // have returned must parse -- a uuid decoded as text is the failure this file exists to catch.
    assert!(
        !result.is_empty(),
        "engine {engine}: no strategy returned a row on a seeded catalogue"
    );
    for id in &result {
        assert!(
            uuid::Uuid::parse_str(id).is_ok(),
            "engine {engine}: a strategy returned {id:?}, which is not a work id"
        );
    }
    // Every strategy excludes what the reader has already shelved, and the exclusion is part of
    // each query rather than a filter afterwards -- so a strategy that forgets it would otherwise
    // recommend the reader's own bookmarks back at them.
    assert!(
        !result.contains(&seeded.work_shared),
        "engine {engine}: a work the reader already bookmarked must never be recommended"
    );

    // The named expectations: each strategy must be able to produce the work its definition
    // promises. These are what a silently-empty query fails.
    let expectations: Vec<(&str, RecStrategyFn)> = vec![
        ("cooccurrence", cooccurrence_strategy()),
        ("time_decay", time_decay_strategy()),
        ("tag_graph", tag_graph_strategy()),
        ("author_graph", author_graph_strategy()),
        ("sequential", sequential_strategy()),
        ("completion_weight", completion_weight_strategy()),
        ("curator_prior", curator_prior_strategy()),
        ("bandit", bandit_strategy()),
    ];
    for (name, strategy) in expectations {
        let rows = strategy(&db, ctx(&seeded.reader))
            .await
            .unwrap_or_else(|error| panic!("engine {engine}: {name} failed: {error}"));
        assert!(
            !rows.is_empty(),
            "engine {engine}: {name} returned nothing on a catalogue seeded for it"
        );
    }

    // And the specific pairs, so a strategy that returns the wrong rows is not green for being
    // non-empty: W2 is what every collaborative strategy should reach from W1.
    let co = cooccurrence_strategy()(&db, ctx(&seeded.reader))
        .await
        .expect("cooccurrence ran");
    assert!(
        co.contains(&seeded.work_co_bookmarked),
        "engine {engine}: cooccurrence should reach W2 through the other reader, got {co:?}"
    );
    let tags = tag_graph_strategy()(&db, ctx(&seeded.reader))
        .await
        .expect("tag_graph ran");
    assert!(
        tags.contains(&seeded.work_co_bookmarked),
        "engine {engine}: tag_graph should reach W2 through the shared tag, got {tags:?}"
    );
    let seq = sequential_strategy()(&db, ctx(&seeded.reader))
        .await
        .expect("sequential ran");
    assert!(
        seq.contains(&seeded.work_in_progress),
        "engine {engine}: sequential should offer the in-progress work, got {seq:?}"
    );
    let curated = curator_prior_strategy()(&db, ctx(&seeded.reader))
        .await
        .expect("curator_prior ran");
    assert!(
        curated.contains(&seeded.work_curator_public),
        "engine {engine}: curator_prior should reach the publicly shelved work, got {curated:?}"
    );
    let bandit = bandit_strategy()(&db, ctx(&seeded.reader))
        .await
        .expect("bandit ran");
    assert_eq!(
        bandit.first().map(String::as_str),
        Some(seeded.work_viewed.as_str()),
        "engine {engine}: the bandit should lead with the most-viewed work, got {bandit:?}"
    );
    let classics = hidden_classics_strategy()(&db, ctx(&seeded.reader))
        .await
        .expect("hidden_classics ran");
    assert!(
        classics.contains(&seeded.work_curator_public),
        "engine {engine}: hidden_classics should reach the twelve-reader work, got {classics:?}"
    );

    // The negative case survives: an account with no signal at all still gets rows from the
    // catalogue-wide strategies, and the collaborative ones say nothing rather than erroring.
    let stranger = uuid::Uuid::new_v4().to_string();
    let lonely = cooccurrence_strategy()(&db, ctx(&stranger))
        .await
        .expect("cooccurrence ran for a stranger");
    assert!(
        lonely.is_empty(),
        "engine {engine}: a reader with no bookmarks must get no co-occurrence rows"
    );

    result.clear();
    if let Some(schema) = schema {
        drop_schema(&schema).await;
    }
}
