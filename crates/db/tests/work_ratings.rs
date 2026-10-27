//! The works ladder on both engines (spec §12.4a).
//!
//! `docs/spec-amendments/arena-work-ladder.md` records the decision; this file is the part that
//! proves the store does what the decision says, on SQLite *and* PostgreSQL. The unit tests in
//! `crates/db/src/work_ratings.rs` cover the arithmetic of who-beat-whom and the shape of the
//! opponent lists; they cannot tell you whether the transaction, the upsert or the `BIGINT`
//! decode works on the engine that serves readers.
//!
//! # Why one test function rather than six
//!
//! The PostgreSQL half runs against a scratch SCHEMA (the harness in
//! `crates/db/tests/continue_reading_t1.rs`, which this copies), and a schema costs a migration
//! run. Six test functions would pay that six times and would also race each other's schema.
//! One sequential body pays it once and asserts the independent claims in order; the failure
//! message names which claim failed, which is what a per-test split buys anyway.
//!
//! # The claims
//!
//! 1. A four-card ballot writes four rows and **ten** comparisons (3 + 2 + 2 + 3) — the number
//!    that distinguishes "one period per work" from "five independent updates".
//! 2. The best work rises, the worst falls, and the two middles sit between them.
//! 3. The pair is symmetric about the initial rating: gains sum to losses when every work starts
//!    at the same rating. A store that applied the winner's update and then re-read it before
//!    scoring the loser would break this.
//! 4. A second ballot adds to `matches` rather than replacing it, and `rated_at` moves.
//! 5. A ballot with no middles writes two rows and two comparisons — the old-client path.
//! 6. A ballot whose best and worst are the same work writes nothing and does not error.
//! 7. The leaderboard comes back highest-first.

use darkhold_db::work_ratings::{self, StoredRating};
use darkhold_db::{Backend, Database, DatabaseConfig};
use std::time::Duration;

const SCHEMA_DB: &str = "darkhold_rs1";
const NOW: &str = "2026-10-27T12:00:00Z";

fn config(url: String) -> DatabaseConfig {
    DatabaseConfig {
        url,
        max_connections: 3,
        acquire_timeout: Duration::from_secs(15),
        slow_query_warn: Duration::ZERO,
    }
}

/// A scratch database on whichever backend `DARKHOLD_TEST_PG_URL` names.
///
/// On PostgreSQL: one schema inside the shared `darkhold_rs1` database, with `search_path`
/// scoped through the connection string. On SQLite: a unique temp file.
async fn connect(tag: &str) -> (Database, Option<String>) {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "darkhold-wr-{}-{}-{}",
        tag,
        uuid::Uuid::new_v4(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");

    let mut schema = None;
    let url = match std::env::var("DARKHOLD_TEST_PG_URL") {
        Ok(admin) => {
            let admin_db = Database::connect(&config(admin.clone()))
                .await
                .expect("connect to the admin database");
            let exists: Option<i32> =
                sqlx::query_scalar("SELECT 1 FROM pg_database WHERE datname = $1")
                    .bind(SCHEMA_DB)
                    .fetch_one(admin_db.postgres_pool().expect("postgres pool"))
                    .await
                    .unwrap_or(None);
            if exists.is_none() {
                // Two threads can both see "absent"; the loser's error is the outcome, not a
                // fault. See the same comment in continue_reading_t1.rs.
                let _ = sqlx::query(&format!("CREATE DATABASE {SCHEMA_DB}"))
                    .execute(admin_db.postgres_pool().expect("postgres pool"))
                    .await;
            }
            admin_db.close().await;

            let (prefix, _) = admin
                .rsplit_once('/')
                .expect("the admin URL ends in a database");
            let scratch = format!("{prefix}/{SCHEMA_DB}");
            let scratch_db = Database::connect(&config(scratch.clone()))
                .await
                .expect("connect to the scratch database");
            let s = format!("wr_{}_{}", std::process::id(), uuid::Uuid::new_v4().simple());
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

    let db = Database::connect(&config(url)).await.expect("connect");
    // One migrator at a time per process: migration 0092 runs CREATE EXTENSION IF NOT EXISTS
    // pgcrypto, and extensions are DATABASE-scoped while schemas are not, so parallel migrators
    // collide on the extension and not on the schema.
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = LOCK.lock().await;
    db.migrate().await.expect("migrate a scratch schema");
    drop(_guard);
    (db, schema)
}

async fn drop_schema(schema: &str) {
    let Ok(admin) = std::env::var("DARKHOLD_TEST_PG_URL") else {
        return;
    };
    let (prefix, _) = admin
        .rsplit_once('/')
        .expect("the admin URL ends in a database");
    let Ok(db) = Database::connect(&config(format!("{prefix}/{SCHEMA_DB}"))).await else {
        return;
    };
    let _ = sqlx::query(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .execute(db.postgres_pool().expect("postgres pool"))
        .await;
    db.close().await;
}

/// One statement, with `?N#u` for a native uuid and `?N#ts` for a timestamp that both engines store
/// differently.
///
/// Three columns in play disagree between dialects: `work_ratings.work_id` / `works.id` are UUID on
/// PostgreSQL and TEXT on SQLite (hence `#u`); `arena_ballots.created_at` is TIMESTAMPTZ on
/// PostgreSQL while `works.created_at` and `accounts.created_at` are TEXT (hence `#ts`). Binding a
/// text timestamp into the first one fails with `column "created_at" is of type timestamp with time
/// zone but expression is of type text` — the same class of fault, and this time the message names
/// the column.
async fn exec(db: &Database, tmpl: &str, args: &[&str]) {
    match db.backend() {
        Backend::Sqlite => {
            let sql = tmpl.replace("#u", "").replace("#ts", "").replace("#t", "");
            let mut q = sqlx::query(&sql);
            for a in args {
                q = q.bind(*a);
            }
            q.execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("fixture insert");
        }
        Backend::Postgres => {
            let pg = (1..=8).fold(tmpl.to_string(), |acc, n| {
                acc.replace(&format!("?{n}#u"), &format!("${n}::uuid"))
                    .replace(&format!("?{n}#ts"), &format!("${n}::timestamptz"))
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

/// Account → pseud → four published works, returning the four work ids.
async fn seed(db: &Database, tag: &str) -> Vec<String> {
    let account = uuid::Uuid::new_v4().to_string();
    let pseud = uuid::Uuid::new_v4().to_string();
    exec(
        db,
        "INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?1#u, ?2, ?3, ?3)",
        &[
            &account,
            &format!("{tag}-{}@example.test", &account[..8]),
            NOW,
        ],
    )
    .await;
    exec(
        db,
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
         VALUES (?1#u, ?2#u, ?3, ?3, ?4, ?4)",
        &[&pseud, &account, &format!("p{}", &pseud[..8]), NOW],
    )
    .await;

    let mut works = Vec::new();
    for index in 0..4 {
        let work = uuid::Uuid::new_v4().to_string();
        exec(
            db,
            "INSERT INTO works (id, owner_pseud_id, title, lifecycle, completion, created_at,
                                updated_at, generated_content_posture)
             VALUES (?1#u, ?2#u, ?3, 'published', 'complete', ?4, ?4, 'forbid')",
            &[&work, &pseud, &format!("{tag} work {index}"), NOW],
        )
        .await;
        works.push(work);
    }
    works
}

async fn matches_for(db: &Database, work: &str) -> StoredRating {
    work_ratings::load(db, work)
        .await
        .expect("load a stored rating")
}

#[tokio::test]
async fn the_works_ladder_records_one_period_per_work_on_both_engines() {
    let (db, schema) = connect("ladder").await;
    let works = seed(&db, "ladder").await;
    let engine = match db.backend() {
        Backend::Sqlite => "sqlite",
        Backend::Postgres => "postgres",
    };
    println!("engine: {engine}");

    // 1. A four-card ballot is ten comparisons.
    let outcome = work_ratings::record_ballot(
        &db,
        &works[0],
        &works[3],
        &[works[1].clone(), works[2].clone()],
        1.0,
    )
    .await
    .expect("record a ballot");
    assert_eq!(outcome.works, 4, "engine {engine}: every card is rated");
    assert_eq!(
        outcome.comparisons, 10,
        "engine {engine}: a four-card ballot is 3+2+2+3 comparisons, not five independent ones"
    );

    // 2. Better ranks above worse, and the counts are per-work.
    let best = matches_for(&db, &works[0]).await;
    let middle1 = matches_for(&db, &works[1]).await;
    let middle2 = matches_for(&db, &works[2]).await;
    let worst = matches_for(&db, &works[3]).await;

    assert_eq!(best.matches, 3, "engine {engine}: best was compared three times");
    assert_eq!(middle1.matches, 2, "engine {engine}: a middle was compared twice");
    assert_eq!(middle2.matches, 2, "engine {engine}: a middle was compared twice");
    assert_eq!(worst.matches, 3, "engine {engine}: worst was compared three times");

    assert!(
        best.mu > middle1.mu && middle1.mu > worst.mu,
        "engine {engine}: ordering is wrong: best {} middle {} worst {}",
        best.mu,
        middle1.mu,
        worst.mu
    );
    assert!(
        best.mu > 1500.0 && worst.mu < 1500.0,
        "engine {engine}: best rose and worst fell: {} / {}",
        best.mu,
        worst.mu
    );

    // 3. Symmetry: every work started at the initial rating, so the gains must equal the
    //    losses. A store that scored the loser against the winner's NEW rating breaks this.
    let drift: f64 = [best, middle1, middle2, worst]
        .iter()
        .map(|rating| rating.mu - StoredRating::initial().mu)
        .sum();
    assert!(
        drift.abs() < 1e-9,
        "engine {engine}: the ballot is not symmetric, drift {drift}"
    );

    // 4. Uncertainty shrank, and a second ballot accumulates rather than overwrites.
    assert!(
        best.phi < StoredRating::initial().phi,
        "engine {engine}: three comparisons must reduce the rating deviation"
    );
    work_ratings::record_ballot(
        &db,
        &works[0],
        &works[3],
        &[works[1].clone(), works[2].clone()],
        1.0,
    )
    .await
    .expect("record a second ballot");
    let best_again = matches_for(&db, &works[0]).await;
    assert_eq!(
        best_again.matches, 6,
        "engine {engine}: matches must accumulate, not reset"
    );
    assert!(
        best_again.mu > best.mu,
        "engine {engine}: a second win against the same opponents must still raise the rating"
    );

    // 5. The old-client path: best and worst only.
    let two = seed(&db, "pair").await;
    let pair = work_ratings::record_ballot(&db, &two[0], &two[1], &[], 1.0)
        .await
        .expect("record a two-card ballot");
    assert_eq!(pair.works, 2, "engine {engine}: only the cards named are rated");
    assert_eq!(pair.comparisons, 2, "engine {engine}: one comparison, both sides");
    assert_eq!(matches_for(&db, &two[2]).await.matches, 0, "engine {engine}: an unshown work is untouched");

    // 6. A ballot that ranks a work above itself does nothing, and does not error.
    let nothing = work_ratings::record_ballot(&db, &two[0], &two[0], &[], 1.0)
        .await
        .expect("a degenerate ballot is not an error");
    assert_eq!((nothing.works, nothing.comparisons), (0, 0), "engine {engine}");

    // 7. The leaderboard is ordered, and carries what was written.
    let board = work_ratings::top_rated(&db, 10)
        .await
        .expect("read the leaderboard");
    assert!(board.len() >= 4, "engine {engine}: leaderboard is short");
    assert!(
        board.windows(2).all(|pair| pair[0].1.mu >= pair[1].1.mu),
        "engine {engine}: the leaderboard is not descending"
    );
    assert_eq!(board[0].0, works[0].to_string(), "engine {engine}: the best work must lead");

    // 8. The engine the feed reads: the floor is `matches`, and the ordering is the ladder's.
    //
    // Two ballots have been cast by now (step 4), so the best and worst cards carry six comparisons
    // and each middle four: 3+3 and 2+2. A floor of five must therefore return exactly the pair that
    // was compared against everything.
    let eligible = work_ratings::top_rated_for(&db, None, None, 10, 5)
        .await
        .expect("read the ladder for a feed");
    let eligible_ids: Vec<&str> = eligible.iter().map(|r| r.work_id.as_str()).collect();
    assert_eq!(
        eligible_ids,
        vec![works[0].as_str(), works[3].as_str()],
        "engine {engine}: a floor of five comparisons must return the best and the worst, best first"
    );
    assert_eq!(eligible[0].matches, 6, "engine {engine}: two ballots are six comparisons for the best");
    assert!(
        eligible[0].mu > eligible[1].mu,
        "engine {engine}: the ladder must lead with the higher rating"
    );

    // A lower floor lets the middles in, and the best still leads.
    let looser = work_ratings::top_rated_for(&db, None, None, 10, 4)
        .await
        .expect("read the ladder with a lower floor");
    assert_eq!(looser.len(), 4, "engine {engine}: a floor of four must admit the middles");
    assert_eq!(looser[0].work_id, works[0].to_string(), "engine {engine}");

    // 9. A work the reader has already compared is not offered back.
    //
    // `arena_ballots` is written by the route, not by `record_ballot`, so the row is inserted here.
    let voter = uuid::Uuid::new_v4().to_string();
    exec(
        &db,
        "INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?1#u, ?2, ?3, ?3)",
        &[&voter, &format!("voter-{}@example.test", &voter[..8]), NOW],
    )
    .await;
    exec(
        &db,
        "INSERT INTO arena_ballots (account_id, best_work_id, worst_work_id, created_at)
         VALUES (?1#u, ?2#u, ?3#u, ?4#ts)",
        &[&voter, &works[0], &works[3], NOW],
    )
    .await;
    let after_vote = work_ratings::top_rated_for(&db, Some(&voter), None, 10, 4)
        .await
        .expect("read the ladder for a reader who has voted");
    assert!(
        after_vote
            .iter()
            .all(|r| r.work_id != works[0] && r.work_id != works[3]),
        "engine {engine}: a work the reader just judged must not be recommended back"
    );
    assert_eq!(after_vote.len(), 2, "engine {engine}: the unjudged middles remain");

    // 10. A private work is not a candidate, however well it is rated.
    exec(
        &db,
        "UPDATE works SET visibility = 'private' WHERE id = ?1#u",
        &[&works[1]],
    )
    .await;
    let after_private = work_ratings::top_rated_for(&db, None, None, 10, 4)
        .await
        .expect("read the ladder with a private work in it");
    assert!(
        after_private.iter().all(|r| r.work_id != works[1]),
        "engine {engine}: a private work must never be a feed candidate"
    );

    if let Some(schema) = schema {
        drop_schema(&schema).await;
    }
}
