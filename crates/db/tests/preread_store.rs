//! Gap C step 4: persisting pre-read reports.
//!
//! The store is in `crates/db/src/preread_store.rs`, the schema in migration 0111, and the
//! value type in `lorehaven_domain::preread`. What these tests are for, beyond "it round
//! trips":
//!
//!   * **no composite score exists anywhere in the path.** §32.6 forbids displaying
//!     composite quality scores publicly and §0.3 forbids payment moving a ranking signal,
//!     so a stored single number would be a ranking signal one query away from being
//!     wired up. There is no column to write one to.
//!   * **re-scoring replaces rather than accumulates.** §23.7 gives no retention policy for
//!     AI output, so a table that grows one row per run is content the operator cannot
//!     delete.
//!   * **withdrawing consent is per provider.** §23.7 lets a reader opt out of specific
//!     providers, so "forget this provider" must not also discard another's output.
//!   * **a corrupt stored score becomes absent, not zero.** A stored 0.0 is a claim; a
//!     0.0 produced by a malformed row is a lie, and it is shown to an author deciding
//!     what to do with their work.
//!
//! The setup is written out here rather than shared with `test_support`, because
//! `test_support` depends on this crate and a dev-dependency the other way is a cycle.
//! The PostgreSQL arm creates a database unique to this call: using the environment
//! variable verbatim would point every test in the file at the shared `postgres`
//! database, and a concurrent run would see another run's rows.

use lorehaven_db::preread_store::{forget_provider, providers_for, report_for, save_report};
use lorehaven_db::{Backend, Database};
use lorehaven_domain::ai::{AiAbstain, PreReadVerdict};
use lorehaven_domain::preread::{DimensionOutcome, DimensionStatus, PreReadReport};
use std::collections::BTreeMap;
use std::time::Duration;

const T0: &str = "2026-01-01T00:00:00Z";

fn make_config(url: String) -> lorehaven_db::DatabaseConfig {
    lorehaven_db::DatabaseConfig {
        url,
        max_connections: 5,
        acquire_timeout: Duration::from_secs(10),
        slow_query_warn: Duration::ZERO,
    }
}

/// A migrated scratch database on whichever backend the selector names.
async fn connect(tag: &str) -> Database {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "lorehaven-preread-{}-{}-{}",
        tag,
        uuid::Uuid::new_v4(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let url = match std::env::var("LOREHAVEN_TEST_PG_URL") {
        Ok(admin) => {
            let name = format!(
                "lh_preread_{}_{}",
                std::process::id(),
                uuid::Uuid::new_v4().simple()
            );
            let admin_db = Database::connect(&make_config(admin.clone()))
                .await
                .expect("connect to the admin database");
            sqlx::query(&format!("CREATE DATABASE {name}"))
                .execute(admin_db.postgres_pool().expect("postgres pool"))
                .await
                .expect("create a scratch database");
            admin_db.close().await;
            let (prefix, _) = admin
                .rsplit_once('/')
                .expect("the admin URL ends in a database");
            format!("{prefix}/{name}")
        }
        Err(_) => format!("sqlite://{}/lorehaven.sqlite?mode=rwc", dir.display()),
    };
    let db = Database::connect(&make_config(url)).await.expect("connect");
    db.migrate().await.expect("migrate");
    db
}

/// Run one statement with `?N#u` for a native uuid (text on SQLite) and `?N#i` for an
/// integer. A single helper rather than two near-identical arms in every test, because
/// the dialect split is mechanical and its repetition would be noise.
async fn exec(db: &Database, tmpl: &str, args: &[&str]) {
    match db.backend() {
        Backend::Sqlite => {
            let sql = tmpl.replace("#u", "").replace("#i", "");
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
                    .replace(&format!("?{n}#i"), &format!("${n}::bigint"))
                    .replace(&format!("?{n}"), &format!("${n}"))
            });
            let mut q = sqlx::query(&pg);
            for a in args {
                match uuid::Uuid::parse_str(a) {
                    Ok(u) => q = q.bind(u),
                    Err(_) => q = q.bind(*a),
                }
            }
            q.execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("fixture insert");
        }
    }
}

/// Insert a raw report row, bypassing the store. Used only where the point is a value the
/// store could never have written.
async fn raw_row(db: &Database, work: &str, dimensions: &str) {
    let id = uuid::Uuid::new_v4().to_string();
    match db.backend() {
        Backend::Sqlite => {
            exec(
                db,
                "INSERT INTO preread_reports (id, work_id, provider, task, dimensions, missing, scored_at, created_at, updated_at) VALUES (?1#u, ?2#u, 'ollama', 'pre_read_scoring', ?3, '{}', ?4, ?4, ?4)",
                &[&id, work, dimensions, T0],
            )
            .await;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO preread_reports (id, work_id, provider, task, dimensions, missing, scored_at, created_at, updated_at) VALUES ($1, $2::uuid, 'ollama', 'pre_read_scoring', $3::jsonb, '{}'::jsonb, $4, $4, $4)",
            )
            .bind(uuid::Uuid::parse_str(&id).expect("uuid"))
            .bind(uuid::Uuid::parse_str(work).expect("uuid"))
            .bind(dimensions)
            .bind(T0)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("raw insert");
        }
    }
}

/// A draft work to hang reports on. Reports need a real work because the FK is
/// ON DELETE CASCADE and that cascade is itself part of what needs testing.
async fn work(db: &Database) -> String {
    let account = uuid::Uuid::new_v4().to_string();
    let pseud = uuid::Uuid::new_v4().to_string();
    let work = uuid::Uuid::new_v4().to_string();
    exec(
        db,
        "INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?1#u, ?2, ?3, ?3)",
        &[&account, &format!("pr-{account}@example.com"), T0],
    )
    .await;
    exec(
        db,
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) VALUES (?1#u, ?2#u, ?3, ?3, ?4, ?4)",
        &[&pseud, &account, &format!("pr{}", &work[..8]), T0],
    )
    .await;
    exec(
        db,
        "INSERT INTO works (id, owner_pseud_id, title, lifecycle, published_at, created_at, updated_at, generated_content_posture) VALUES (?1#u, ?2#u, 'Draft', 'draft', ?3, ?3, ?3, 'forbid')",
        &[&work, &pseud, T0],
    )
    .await;
    work
}

/// A report with the given (dimension, score) pairs and nothing missing.
fn report(work_id: &str, dimensions: &[(&str, f64)]) -> PreReadReport {
    PreReadReport {
        work_id: work_id.to_string(),
        dimensions: dimensions
            .iter()
            .map(|(d, s)| {
                (
                    d.to_string(),
                    DimensionOutcome {
                        dimension: d.to_string(),
                        score: *s,
                        note: "assessed".to_string(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>(),
        missing: BTreeMap::new(),
    }
}

// ── round trip ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_report_round_trips_with_every_dimension_intact() {
    let db = connect("round").await;
    let work = work(&db).await;

    save_report(
        &db,
        &report(&work, &[("length", 0.35), ("tone", 0.82), ("pacing", 0.5)]),
        "ollama",
        "2026-10-02T00:00:00Z",
    )
    .await
    .expect("save");

    let loaded = report_for(&db, &work, "ollama")
        .await
        .expect("load")
        .expect("a report");
    assert_eq!(loaded.work_id, work);
    assert_eq!(loaded.dimensions.len(), 3);
    assert_eq!(loaded.dimensions["length"].score, 0.35);
    assert_eq!(loaded.dimensions["tone"].score, 0.82);
    assert_eq!(loaded.dimensions["length"].note, "assessed");
    assert!(loaded.is_complete(), "nothing was missing, so nothing is");
    // Ordering is by score descending — the question the author has is "what is worst",
    // so ascending would answer the one they did not ask.
    let ranked: Vec<&str> = loaded
        .ranked()
        .iter()
        .map(|d| d.dimension.as_str())
        .collect();
    assert_eq!(ranked, vec!["tone", "pacing", "length"]);
}

#[tokio::test]
async fn a_report_built_from_real_verdicts_round_trips_the_same_way() {
    // The store's input type is `PreReadReport`, and `from_verdicts` is how a caller gets
    // one from provider output. Exercising that path checks the two agree rather than only
    // the hand-built struct.
    let db = connect("verdicts").await;
    let work = work(&db).await;

    let verdicts = vec![
        PreReadVerdict::new("length", 0.3, "long").expect("in range"),
        PreReadVerdict::new("tone", 0.6, "even").expect("in range"),
    ];
    // `from_verdicts` also takes the configured dimensions, so it can record the ones the
    // provider did not answer. That is what makes a partial provider reply reportable
    // rather than silently short.
    let dimensions = vec!["length".to_string(), "tone".to_string()];
    let built = PreReadReport::from_verdicts(&work, &dimensions, verdicts).expect("built");
    save_report(&db, &built, "ollama", "2026-10-02T00:00:00Z")
        .await
        .expect("save");

    let loaded = report_for(&db, &work, "ollama")
        .await
        .expect("load")
        .expect("a report");
    assert_eq!(loaded.dimensions["length"].score, 0.3);
    assert_eq!(loaded.dimensions["tone"].note, "even");
    assert!(loaded.is_complete());
}

#[tokio::test]
async fn a_work_with_no_report_is_none_rather_than_an_error() {
    let db = connect("none").await;
    let work = work(&db).await;
    assert!(report_for(&db, &work, "ollama")
        .await
        .expect("query")
        .is_none());
}

#[tokio::test]
async fn a_report_from_another_provider_is_not_returned() {
    // Two providers can both have assessed this work, and asking for one must not get the
    // other's answer — that would misattribute AI output to a provider the reader did not
    // consent to.
    let db = connect("provider-isolation").await;
    let work = work(&db).await;
    save_report(
        &db,
        &report(&work, &[("length", 0.5)]),
        "openai-compatible",
        "2026-10-02T00:00:00Z",
    )
    .await
    .expect("save");

    assert!(report_for(&db, &work, "ollama")
        .await
        .expect("query")
        .is_none());
    assert!(report_for(&db, &work, "openai-compatible")
        .await
        .expect("query")
        .is_some());
}

#[tokio::test]
async fn a_missing_dimension_survives_the_round_trip_with_its_reason() {
    // "Not configured" and "the provider returned something unparseable" are different
    // facts to debug a week later, so the reason is persisted, not just the absence.
    let db = connect("missing").await;
    let work = work(&db).await;

    let mut r = report(&work, &[("length", 0.4)]);
    r.missing.insert(
        "tone".to_string(),
        DimensionStatus::Abstained(AiAbstain::NotConfigured),
    );
    r.missing
        .insert("tags".to_string(), DimensionStatus::NotConfigured);
    save_report(&db, &r, "ollama", "2026-10-02T00:00:00Z")
        .await
        .expect("save");

    let loaded = report_for(&db, &work, "ollama")
        .await
        .expect("load")
        .expect("a report");
    assert!(!loaded.is_complete(), "two dimensions never came back");
    assert_eq!(loaded.missing.len(), 2);
    assert!(
        matches!(
            loaded.missing.get("tone"),
            Some(DimensionStatus::Abstained(AiAbstain::NotConfigured))
        ),
        "the abstain reason survives: {:?}",
        loaded.missing
    );
    assert_eq!(
        loaded.missing.get("tags"),
        Some(&DimensionStatus::NotConfigured),
        "and so does 'never asked'"
    );
}

// ── replacement, not accumulation ───────────────────────────────────────────

#[tokio::test]
async fn re_scoring_replaces_the_report_rather_than_appending() {
    let db = connect("replace").await;
    let work = work(&db).await;

    save_report(
        &db,
        &report(&work, &[("length", 0.2)]),
        "ollama",
        "2026-10-01T00:00:00Z",
    )
    .await
    .expect("first save");
    save_report(
        &db,
        &report(&work, &[("length", 0.9), ("tone", 0.7)]),
        "ollama",
        "2026-10-02T00:00:00Z",
    )
    .await
    .expect("second save");

    let loaded = report_for(&db, &work, "ollama")
        .await
        .expect("load")
        .expect("a report");
    assert_eq!(
        loaded.dimensions.len(),
        2,
        "the newer assessment replaced the older"
    );
    assert_eq!(
        loaded.dimensions["length"].score, 0.9,
        "not the 0.2 from yesterday"
    );

    let count: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar("SELECT COUNT(*) FROM preread_reports WHERE work_id = ?1")
                .bind(&work)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("count")
        }
        Backend::Postgres => {
            sqlx::query_scalar("SELECT COUNT(*) FROM preread_reports WHERE work_id = $1")
                .bind(uuid::Uuid::parse_str(&work).expect("uuid"))
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await
                .expect("count")
        }
    };
    assert_eq!(count, 1, "one report per (work, provider, task), not a log");
}

// ── consent is per provider ─────────────────────────────────────────────────

#[tokio::test]
async fn withdrawing_one_provider_leaves_anothers_output() {
    // §23.7: "users may opt out of specific providers". Forgetting one provider must not
    // discard a different provider's report about the same work.
    let db = connect("forget").await;
    let work = work(&db).await;

    for provider in ["ollama", "openai-compatible"] {
        save_report(
            &db,
            &report(&work, &[("length", 0.5)]),
            provider,
            "2026-10-02T00:00:00Z",
        )
        .await
        .expect("save");
    }

    assert_eq!(
        providers_for(&db, &work).await.expect("providers"),
        vec!["ollama", "openai-compatible"],
        "both providers have a report"
    );

    assert_eq!(
        forget_provider(&db, &work, "ollama").await.expect("forget"),
        1,
        "one row removed"
    );
    assert!(report_for(&db, &work, "ollama")
        .await
        .expect("query")
        .is_none());
    assert!(
        report_for(&db, &work, "openai-compatible")
            .await
            .expect("query")
            .is_some(),
        "the other provider's report survives"
    );
    assert_eq!(
        providers_for(&db, &work).await.expect("providers"),
        vec!["openai-compatible"],
    );
}

#[tokio::test]
async fn forgetting_a_provider_that_has_no_report_is_not_an_error() {
    // Idempotent, because a consent screen that lists stale providers would otherwise fail
    // on the second click.
    let db = connect("forget-idempotent").await;
    let work = work(&db).await;
    assert_eq!(
        forget_provider(&db, &work, "ollama").await.expect("forget"),
        0
    );
}

#[tokio::test]
async fn deleting_a_work_deletes_its_reports() {
    // The FK is ON DELETE CASCADE, so a deleted draft does not leave AI assessments of it
    // behind — which would be a draft's text-derived data outliving the draft itself.
    let db = connect("cascade").await;
    let work = work(&db).await;
    save_report(
        &db,
        &report(&work, &[("length", 0.5)]),
        "ollama",
        "2026-10-02T00:00:00Z",
    )
    .await
    .expect("save");

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("DELETE FROM works WHERE id = ?1")
                .bind(&work)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("delete the work");
        }
        Backend::Postgres => {
            sqlx::query("DELETE FROM works WHERE id = $1")
                .bind(uuid::Uuid::parse_str(&work).expect("uuid"))
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("delete the work");
        }
    }

    assert!(report_for(&db, &work, "ollama")
        .await
        .expect("query")
        .is_none());
}

// ── a corrupt row is absent, not zero ───────────────────────────────────────

#[tokio::test]
async fn a_stored_score_that_is_not_a_number_becomes_absent_rather_than_zero() {
    // Written by hand, the way a bad migration or a hand-edited row would. A stored 0.0 is
    // a claim ("this scored zero"); a 0.0 produced from a malformed row is a lie — and it
    // is shown to an author deciding what to do with their work.
    let db = connect("corrupt").await;
    let work = work(&db).await;
    raw_row(
        &db,
        &work,
        r#"{"length": {"score": "very high", "note": "n/a"},
            "tone": {"score": 0.7, "note": "even"}}"#,
    )
    .await;

    let loaded = report_for(&db, &work, "ollama")
        .await
        .expect("load")
        .expect("a report");
    assert!(
        !loaded.dimensions.contains_key("length"),
        "\"very high\" is not a score, so the dimension is absent rather than 0.0"
    );
    assert_eq!(
        loaded.dimensions.get("tone").map(|d| d.score),
        Some(0.7),
        "the dimension that WAS a number still loads"
    );
}

#[tokio::test]
async fn a_stored_score_out_of_range_is_dropped_even_though_the_column_allows_it() {
    // The SQLite CHECK constrains the JSON's *shape*, not the numbers inside it, so a
    // hand-written row can still carry a 4.2. It must not reach an author as a score.
    let db = connect("range").await;
    let work = work(&db).await;
    raw_row(
        &db,
        &work,
        r#"{"length": {"score": 4.2, "note": "n/a"},
            "tone": {"score": -1.0, "note": "n/a"}}"#,
    )
    .await;

    let loaded = report_for(&db, &work, "ollama")
        .await
        .expect("load")
        .expect("a report");
    assert!(
        !loaded.dimensions.contains_key("length"),
        "4.2 is out of range and must not be clamped into the report"
    );
    assert!(
        !loaded.dimensions.contains_key("tone"),
        "and neither is -1.0"
    );
    assert!(loaded.ranked().is_empty(), "so there is nothing to show");
}

// ── the composite that must not exist ────────────────────────────────────────

#[tokio::test]
async fn the_api_exposes_no_way_to_read_a_single_score_for_a_work() {
    // A statement of what the API shape guarantees, checked by using the only
    // aggregations it offers. §32.6 forbids displaying composite quality scores publicly
    // and §0.3 forbids any ranking signal moving on payment; `PreReadReport` has no
    // `score()` and the table has no `score` column, so a composite would require adding
    // one of them — a visible change rather than an accidental one.
    let loaded = report("w", &[("length", 0.4), ("tone", 0.9)]);
    assert_eq!(loaded.dimensions.len(), 2);
    assert_eq!(loaded.ranked().len(), 2);
    assert_eq!(loaded.below(0.5).len(), 1);
    assert_eq!(
        loaded.below(0.0).len(),
        0,
        "`below` never invents a threshold"
    );

    // And the stored row has no place to put one, on either engine.
    let db = connect("no-column").await;
    let work = work(&db).await;
    save_report(
        &db,
        &report(&work, &[("length", 0.4), ("tone", 0.9)]),
        "ollama",
        T0,
    )
    .await
    .expect("save");
    let columns: Vec<String> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar("SELECT name FROM pragma_table_info('preread_reports')")
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("columns")
        }
        Backend::Postgres => {
            sqlx::query_scalar(
                "SELECT column_name FROM information_schema.columns WHERE table_name = 'preread_reports'",
            )
            .fetch_all(db.postgres_pool().expect("postgres"))
            .await
            .expect("columns")
        }
    };
    assert!(
        !columns.iter().any(|c| c == "score"),
        "no composite score column exists on either engine: {columns:?}"
    );
}
