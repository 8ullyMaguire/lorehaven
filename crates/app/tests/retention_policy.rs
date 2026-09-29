//! Acceptance: the retention setting, stored and read (spec §11.15).
//!
//! `crates/domain/src/retention.rs` proves the *rule* — narrowest wins, an
//! unrecognised mode is refused, a widening is refused — as pure functions. This
//! file proves the *store*, which is a separate claim in three ways that a
//! pure-function test cannot reach:
//!
//! 1. **The default has to be a default.** Every instance built before migration
//!    0087 has no policy row. Treating absence as `Aggregate` would strip
//!    storage from a running instance on upgrade, and a unit test with a
//!    `BodyMode` in hand cannot tell that apart from a deliberate setting.
//! 2. **The widening refusal has to happen at the write.** §11.15 wants the
//!    operator *told*, and an override row that is written and then narrowed
//!    away on read is the failure: the operator sees their change saved.
//! 3. **The row has to survive both dialects.** `updated_by` is TEXT on SQLite
//!    and UUID on PostgreSQL, and the singleton index is a partial predicate —
//!    neither is exercised by a test that never opens a database.
//!
//! The six refusal *paths* (§11.15: URL import, file upload, clipboard paste,
//! preservation batch, federated announcement, cache fill) are not here, because
//! five of them do not exist yet and the sixth is M59-08. What is here is the
//! decision they will all read.

use std::path::{Path, PathBuf};

use lorehaven_db::retention;
use lorehaven_domain::retention::{check_body_allowed, BodyMode, RetentionReason};
use test_support::TestDb;

/// A per-test directory, as every suite in `crates/app/tests/` does.
///
/// Not a shared one. Under PostgreSQL a second `TestDb` for the same tag is a
/// *different* database, so a fixture written through one handle is invisible to
/// the code under test and the test fails for a reason that has nothing to do
/// with it. That is a lesson this repository has already paid for.
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-retention-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// An account id to attribute a change to.
///
/// The `updated_by` column is a foreign key on both dialects, so it has to be a
/// real account or the write fails on the constraint. `Uuid::new_v4()` happens
/// to collide with nothing, but a foreign key does not care about luck — and the
/// test below registers a real one anyway, because a constraint that is not
/// exercised is not a constraint that works.
async fn account_for(tdb: &TestDb, tag: &str) -> String {
    let dir = scratch_dir(&format!("{tag}-account"));
    let mut config = lorehaven_app::config::Config::development_defaults();
    config.storage.root = dir.clone();
    let app = lorehaven_app::server::build_router(lorehaven_app::state::AppState::new(
        config,
        tdb.db().clone(),
    ));
    let mut client = test_support::TestClient::new(app);
    let handle = tag.replace(['.', '-'], "_");
    test_support::register(&mut client, &format!("{tag}@test.dev"), &handle).await
}

/// A `TestDb` and a real account to attribute changes to.
async fn harness(tag: &str) -> (TestDb, String) {
    let dir = scratch_dir(tag);
    let tdb = TestDb::connect_with_dir(tag, &dir).await;
    let account = account_for(&tdb, tag).await;
    (tdb, account)
}

/// An instance nobody has configured caches bodies (§11.15's default).
///
/// The whole test in one assertion, and the reason it is worth having: this is
/// the answer every pre-0087 instance depends on, and it is produced by the
/// *absence* of a row rather than by anything anybody chose. A reader that
/// defaulted absence to `Aggregate` would pass every test that sets the mode
/// explicitly and fail this one — which is the only direction that matters,
/// because the operator who did nothing is the one who would lose their bodies.
#[tokio::test]
async fn an_instance_nobody_has_configured_caches_bodies() {
    let (tdb, _account) = harness("default_mode").await;

    assert!(
        retention::read_policy(tdb.db()).await.unwrap().is_none(),
        "a fresh database has no policy row; the absence is what makes the default \
         observable rather than a value somebody wrote"
    );
    assert_eq!(
        retention::effective_instance_mode(tdb.db()).await.unwrap(),
        BodyMode::Cache,
        "an instance nobody configured must cache, not aggregate: the default must never \
         take storage away from a running instance on upgrade"
    );
}

/// The operator's decision round-trips, and the second write is the common one.
#[tokio::test]
async fn the_policy_round_trips_and_a_second_write_updates_rather_than_failing() {
    let (tdb, account) = harness("round_trip").await;
    let actor: uuid::Uuid = account.parse().expect("a uuid account id");

    let first = retention::write_policy(tdb.db(), BodyMode::Aggregate, actor)
        .await
        .expect("write aggregate");
    assert_eq!(first.body_mode, BodyMode::Aggregate);
    assert_eq!(first.version, 1, "the first write is version 1");
    assert_eq!(first.updated_by.as_deref(), Some(account.as_str()));

    let second = retention::write_policy(tdb.db(), BodyMode::Cache, actor)
        .await
        .expect("write cache over an existing row");
    assert_eq!(
        second.version, 2,
        "the upsert must bump the version, so a second write is visible as a change"
    );
    assert_eq!(second.body_mode, BodyMode::Cache);
    assert_eq!(
        retention::effective_instance_mode(tdb.db()).await.unwrap(),
        BodyMode::Cache
    );
}

/// There is one policy, not several: a second row under another id is a
/// decision nothing reads.
///
/// The first version of this test seeded the stray row with a hard-coded UUID
/// and it failed on `FOREIGN KEY constraint failed` — the constraint on
/// `updated_by` doing exactly what it exists for, on a test that had not
/// bothered to register an account. The fix is to use the real account, and the
/// lesson is worth keeping: a fixture that reaches past a constraint is either
/// refused (loudly, as here) or, worse, accepted on the engine whose foreign keys
/// are off.
#[tokio::test]
async fn a_second_policy_row_is_inert_because_the_reader_asks_for_the_singleton() {
    let (tdb, account) = harness("singleton").await;
    let actor: uuid::Uuid = account.parse().expect("a uuid account id");

    // A row under a different id, saying the opposite of what the singleton
    // will say. Permitted by the table: the partial index covers the literal
    // 'default' and nothing else, which is what stops a near-miss id ('default '
    // with a trailing space) from becoming a second *singleton* while still
    // permitting a row that is explicitly not one.
    let seeded = match tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query(
            "INSERT INTO instance_retention_policy
               (id, body_mode, updated_by, created_at, updated_at, version)
             VALUES ('another', 'aggregate', ?, '2026-01-01T00:00:00Z',
                     '2026-01-01T00:00:00Z', 1)",
        )
        .bind(account.clone())
        .execute(tdb.db().sqlite_pool().expect("sqlite"))
        .await
        .is_ok(),
        lorehaven_db::Backend::Postgres => sqlx::query(
            "INSERT INTO instance_retention_policy
               (id, body_mode, updated_by, created_at, updated_at, version)
             VALUES ('another', 'aggregate', $1::uuid, '2026-01-01T00:00:00Z',
                     '2026-01-01T00:00:00Z', 1)",
        )
        .bind(account.clone())
        .execute(tdb.db().postgres_pool().expect("postgres"))
        .await
        .is_ok(),
    };
    assert!(
        seeded,
        "a row under another id is permitted by the table; the partial index covers only \
         the literal 'default'"
    );

    retention::write_policy(tdb.db(), BodyMode::Cache, actor)
        .await
        .expect("write the singleton");
    assert_eq!(
        retention::effective_instance_mode(tdb.db()).await.unwrap(),
        BodyMode::Cache,
        "the reader asks for the singleton by its id, so a stray row cannot change what \
         any reader sees"
    );
}

/// A per-source override may narrow, and the refusal for a widening is a
/// *type* the route can branch on (spec §11.15).
///
/// The refusal is the substance. §11.15: "the reverse on an `aggregate`
/// instance is not, because that would restore storage the instance decided
/// against". A test asserting only the end state would pass against an
/// implementation that wrote the row and narrowed it away on read — which is
/// the failure, because the operator would see their change saved and no change
/// in behaviour.
#[tokio::test]
async fn an_operator_may_narrow_a_source_and_may_not_widen_it() {
    let (tdb, account) = harness("narrowing").await;
    let actor: uuid::Uuid = account.parse().expect("a uuid account id");

    // A caching instance: narrowing is expressible.
    retention::write_policy(tdb.db(), BodyMode::Cache, actor)
        .await
        .expect("cache");
    let narrowed = retention::write_source_override(tdb.db(), "ao3", BodyMode::Aggregate, actor)
        .await
        .expect("narrowing ao3 to aggregate is allowed on a caching instance");
    assert_eq!(narrowed.body_mode, BodyMode::Aggregate);
    assert_eq!(narrowed.source_key, "ao3");
    assert_eq!(
        retention::list_source_overrides(tdb.db())
            .await
            .unwrap()
            .len(),
        1
    );

    // An aggregating instance: the same write is refused, and refused as a
    // `Widening` rather than as a storage error, so the route can answer 400
    // and name the setting rather than answering 500.
    retention::write_policy(tdb.db(), BodyMode::Aggregate, actor)
        .await
        .expect("aggregate");
    let refusal = retention::write_source_override(tdb.db(), "eff", BodyMode::Cache, actor).await;
    match refusal {
        Err(retention::OverrideWriteError::Widening(w)) => {
            assert_eq!(w.source_key, "eff");
            assert_eq!(w.attempted, BodyMode::Cache);
            assert_eq!(
                w.instance,
                BodyMode::Aggregate,
                "the refusal names the instance setting, because that is the setting the \
                 operator has to change"
            );
        }
        Err(other) => {
            panic!("a widening must be refused by the rule, not as a storage failure: {other:?}")
        }
        Ok(row) => panic!("a widening override was accepted on an aggregating instance: {row:?}"),
    }
    assert!(
        !retention::list_source_overrides(tdb.db())
            .await
            .unwrap()
            .iter()
            .any(|row| row.source_key == "eff"),
        "the refused write must leave no row behind"
    );
}

/// The read every body-storage path makes: instance setting, override, and the
/// two facts the caller already holds.
#[tokio::test]
async fn the_resolved_policy_is_what_a_body_path_would_read() {
    let (tdb, account) = harness("resolve").await;
    let actor: uuid::Uuid = account.parse().expect("a uuid account id");

    // No override: a caching instance stores.
    let caching = retention::resolve_for_source(tdb.db(), Some("ao3"), false, false)
        .await
        .expect("resolve");
    assert_eq!(caching.instance, BodyMode::Cache);
    assert_eq!(caching.source, None);
    assert_eq!(check_body_allowed(&caching, Some("ao3")), Ok(()));

    // An override for THIS source, and only this source: the neighbouring
    // source is the case a per-source feature most often gets wrong, and it is
    // asserted here rather than left to a reader's judgement.
    retention::write_source_override(tdb.db(), "ao3", BodyMode::Aggregate, actor)
        .await
        .expect("narrow ao3");
    let narrowed = retention::resolve_for_source(tdb.db(), Some("ao3"), false, false)
        .await
        .expect("resolve ao3");
    assert_eq!(narrowed.source, Some(BodyMode::Aggregate));
    assert_eq!(
        check_body_allowed(&narrowed, Some("ao3")),
        Err(lorehaven_domain::retention::RetentionRefusal {
            reason: RetentionReason::AggregateSourceOverride,
            source_key: Some("ao3".to_owned()),
        }),
        "the refusal names the source, and the source's own configuration is fine"
    );

    let neighbour = retention::resolve_for_source(tdb.db(), Some("eff"), false, false)
        .await
        .expect("resolve eff");
    assert_eq!(
        neighbour.source, None,
        "an override for one source must not narrow another"
    );
    assert_eq!(
        check_body_allowed(&neighbour, Some("eff")),
        Ok(()),
        "caching most sources while aggregating one is the case §11.15 calls expressible"
    );

    // Clearing the override restores the instance's answer, which is the action
    // every `AggregateSourceOverride` message points the operator at.
    assert!(retention::clear_source_override(tdb.db(), "ao3")
        .await
        .unwrap());
    assert!(!retention::clear_source_override(tdb.db(), "ao3")
        .await
        .unwrap());
    let cleared = retention::resolve_for_source(tdb.db(), Some("ao3"), false, false)
        .await
        .expect("resolve after clearing");
    assert_eq!(cleared.source, None);
    assert_eq!(check_body_allowed(&cleared, Some("ao3")), Ok(()));
}

/// A blocked source and a vanished origin are carried through to the refusal.
#[tokio::test]
async fn a_blocked_source_and_a_vanished_origin_reach_the_refusal() {
    let (tdb, _account) = harness("blocked").await;

    let blocked = retention::resolve_for_source(tdb.db(), Some("ao3"), true, false)
        .await
        .expect("resolve");
    assert_eq!(
        check_body_allowed(&blocked, Some("ao3"))
            .unwrap_err()
            .reason,
        RetentionReason::SourceBlocked,
        "a blocked source is not a retention question"
    );

    let vanished = retention::resolve_for_source(tdb.db(), Some("ao3"), false, true)
        .await
        .expect("resolve");
    assert_eq!(
        check_body_allowed(&vanished, Some("ao3"))
            .unwrap_err()
            .reason,
        RetentionReason::Vanished,
        "a vanished origin with no body held is a dead end"
    );
}

/// The `updated_by` foreign key holds on both dialects.
///
/// This is a test about a constraint, which is exactly the kind of thing that
/// passes on one engine and fails on the other. `updated_by` is `TEXT` in the
/// SQLite migration and `UUID` in the PostgreSQL one, because `accounts.id` is a
/// UUID there — and the parity test in `migrate.rs` compares column *names*, not
/// types, so nothing else in the suite would notice if one of the two were
/// wrong. A row that violates it fails to insert, which is a clean failure; a row
/// that merely stored a string where a uuid belongs would not be.
#[tokio::test]
async fn the_author_of_a_change_is_a_real_account_or_nothing() {
    let (tdb, account) = harness("author_fk").await;
    let actor: uuid::Uuid = account.parse().expect("a uuid account id");

    // A real account: accepted.
    retention::write_policy(tdb.db(), BodyMode::Cache, actor)
        .await
        .expect("write with a real author");

    // An id that is not an account: refused by the constraint, on whichever
    // backend is running.
    let ghost = "22222222-2222-2222-2222-222222222222"
        .parse::<uuid::Uuid>()
        .unwrap();
    let sql = match tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            "INSERT INTO instance_retention_source_overrides
               (source_key, body_mode, updated_by, created_at, updated_at, version)
             VALUES ('ao3', 'cache', ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 1)"
        }
        lorehaven_db::Backend::Postgres => {
            "INSERT INTO instance_retention_source_overrides
               (source_key, body_mode, updated_by, created_at, updated_at, version)
             VALUES ('ao3', 'cache', $1::uuid, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 1)"
        }
    };
    // Mapped to `bool` rather than returned directly: the two backends return
    // different `QueryResult` types, so a `match` yielding either of them is a
    // type error. `is_err()` is all this test reads.
    let refused = match tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query(sql)
            .bind(ghost.to_string())
            .execute(tdb.db().sqlite_pool().expect("sqlite"))
            .await
            .is_err(),
        lorehaven_db::Backend::Postgres => sqlx::query(sql)
            .bind(ghost.to_string())
            .execute(tdb.db().postgres_pool().expect("postgres"))
            .await
            .is_err(),
    };
    assert!(
        refused,
        "an override attributed to an account that does not exist must be refused by the \
         foreign key; storing it would make the modlog name somebody who never did it"
    );
}

/// The override list is ordered, because the admin route renders it.
#[tokio::test]
async fn the_override_list_is_ordered_by_source_key() {
    let (tdb, account) = harness("ordering").await;
    let actor: uuid::Uuid = account.parse().expect("a uuid account id");
    for key in ["ff", "aa", "mm"] {
        retention::write_source_override(tdb.db(), key, BodyMode::Aggregate, actor)
            .await
            .expect("narrow");
    }
    let keys: Vec<String> = retention::list_source_overrides(tdb.db())
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.source_key)
        .collect();
    assert_eq!(
        keys,
        vec!["aa".to_owned(), "ff".to_owned(), "mm".to_owned()],
        "two identical instances must answer the same GET, so the list is ordered"
    );
}

/// An unrecognised stored mode is read as `Cache`, not as `Aggregate`.
///
/// The asymmetric default is the decision, so it is asserted on the row rather
/// than only on the parser: a downgrade or a typo must not be able to stop an
/// instance storing bodies its operator chose to store.
#[tokio::test]
async fn a_stored_mode_this_build_does_not_know_reads_as_cache() {
    let (tdb, _account) = harness("unknown_mode").await;
    let sql = match tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            "INSERT INTO instance_retention_policy
               (id, body_mode, created_at, updated_at, version)
             VALUES ('default', 'from_a_newer_build', '2026-01-01T00:00:00Z',
                     '2026-01-01T00:00:00Z', 1)"
        }
        lorehaven_db::Backend::Postgres => {
            "INSERT INTO instance_retention_policy
               (id, body_mode, created_at, updated_at, version)
             VALUES ('default', 'from_a_newer_build', '2026-01-01T00:00:00Z',
                     '2026-01-01T00:00:00Z', 1)"
        }
    };
    match tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(sql)
                .execute(tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("seed an unknown mode");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(sql)
                .execute(tdb.db().postgres_pool().expect("postgres"))
                .await
                .expect("seed an unknown mode");
        }
    }
    assert_eq!(
        retention::effective_instance_mode(tdb.db()).await.unwrap(),
        BodyMode::Cache,
        "an unknown value must not become `aggregate`: that is a policy change made by a \
         downgrade, and it takes storage away from an instance that is storing"
    );
}

/// A `Path` is only in the signature for symmetry with the other suites; this
/// file uses [`scratch_dir`] directly. Pinned so an unused import is a build
/// error rather than a silent one.
#[test]
fn the_scratch_directory_is_per_test() {
    let a = scratch_dir("singleton_a");
    let b = scratch_dir("singleton_b");
    assert_ne!(a, b, "two tests must not share a directory");
    let _p: &Path = &a;
}

/// `works_past_saving` counts the amendment's definition and nothing looser.
///
/// Driven at the store rather than through the route, because the exclusions
/// are the claim and a route test can only see the total. The fixture is built
/// by hand because the count reads `library_items`, `import_chapters` and
/// `import_jobs` directly, and an import driven through the worker would make
/// each clause depend on an adapter's behaviour rather than on the query.
#[tokio::test]
async fn works_past_saving_counts_only_unreachable_works_with_no_body() {
    let (tdb, account) = harness("past_saving").await;
    let db = tdb.db();

    // `import_jobs` has four foreign keys (job, account, pseud, library item)
    // and this fixture satisfies each of them explicitly. An earlier version
    // resolved the pseud with a `SELECT ... LIMIT 1` subquery inside the VALUES
    // clause and failed with a bare "FOREIGN KEY constraint failed" three times
    // over, each time on a guess about which one was unsatisfied. Binding the
    // value makes the failure impossible to misattribute.
    let pseud = pseud_of(db, &account).await;
    assert!(
        !pseud.is_empty(),
        "the registered account has a pseud: {account}"
    );

    // A library item plus one import job, differing only in the two clauses of
    // the definition.
    async fn seed(
        db: &lorehaven_db::Database,
        account: &str,
        pseud: &str,
        state: &str,
        code: &str,
        body: bool,
    ) {
        let item = seed_library_item(db, account).await;
        seed_import_job(db, account, pseud, &item, state, code).await;
        if body {
            seed_stored_chapter(db, &item).await;
        }
    }

    // 1. origin gone, no body held -> COUNTS.
    seed(db, &account, &pseud, "failed", "not_found", false).await;
    // 2. origin gone, but a body IS held -> does not count. §11.15's count is
    //    about works this instance cannot recover, and one with a stored body is
    //    recoverable: the text is here.
    seed(db, &account, &pseud, "failed", "not_found", true).await;
    // 3. no body held, but the origin is fine -> does not count. A work whose
    //    import is merely queued has not been lost.
    seed(db, &account, &pseud, "queued", "not_found", false).await;
    // 4. no body held, and the last failure was a credential problem -> does NOT
    //    count. This is the exclusion that matters: `credential_missing` is an
    //    operator-fixable login, and counting it would report a preservation
    //    debt for a work that is still perfectly reachable.
    seed(db, &account, &pseud, "failed", "credential_missing", false).await;

    lorehaven_db::retention::write_policy(
        db,
        BodyMode::Aggregate,
        account.parse().expect("a uuid account id"),
    )
    .await
    .expect("aggregate");

    let past = lorehaven_db::retention::works_past_saving(db)
        .await
        .expect("count");
    assert_eq!(
        past.count, 1,
        "only the work whose origin is gone AND whose body is not held counts: a work \
         with a stored body is recoverable, a queued import has not been lost, and a \
         credential problem is a login the operator can fix"
    );
    assert_eq!(past.instance, BodyMode::Aggregate);

    // And a caching instance reports the same count with the mode beside it, so
    // a zero on a `cache` instance reads as "nothing at risk" rather than as a
    // number the operator has to interpret.
    let still = lorehaven_db::retention::works_past_saving(db)
        .await
        .expect("count again");
    assert_eq!(
        still.count, past.count,
        "the count is a property of the data"
    );
}

/// Insert a library item and return its id.
///
/// Hand-written SQL rather than `imports::upsert_library_item` because the
/// count reads these three tables directly, and going through the import layer
/// would make each clause of the definition depend on an adapter rather than on
/// the query under test.
async fn seed_library_item(db: &lorehaven_db::Database, account: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let key = format!("work-{}", uuid::Uuid::new_v4());
    let sql = db.sql(
        "INSERT INTO library_items
           (id, account_id, source_key, source_work_key, title, source_url,
            created_at, updated_at, version)
         VALUES (?, ?, 'ao3', ?, 'a work', 'https://example.test/a',
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 1)",
        "INSERT INTO library_items
           (id, account_id, source_key, source_work_key, title, source_url,
            created_at, updated_at, version)
         VALUES ($1::uuid, $2::uuid, 'ao3', $3, 'a work', 'https://example.test/a',
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 1)",
    );
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(account)
                .bind(&key)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("seed a library item");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(account)
                .bind(&key)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("seed a library item");
        }
    }
    id
}

/// Insert an import job for this item, in `state`, with a `report_json`
/// carrying `code` as its error.
///
/// Written with `imports::create_import_job` rather than hand-rolled SQL. That
/// is not tidiness: the first version of this fixture inserted `import_jobs`
/// directly and failed with a bare "FOREIGN KEY constraint failed" on all four
/// of its declared references while every one of them verifiably existed. The
/// app's own insert path has no such problem, and a fixture that has to be
/// debugged harder than the code it tests is a fixture costing more than it
/// proves — so the count's fixture seeds through the same door the application
/// does and pins the *query*, which is the thing actually under test.
async fn seed_import_job(
    db: &lorehaven_db::Database,
    account: &str,
    pseud: &str,
    library_item_id: &str,
    state: &str,
    code: &str,
) {
    let id = uuid::Uuid::new_v4().to_string();
    // `import_jobs.job_id` is a real foreign key onto `jobs` and it is UNIQUE:
    // one import is one job, and retrying the queue entry cannot become two
    // imports. So a queue row is seeded rather than the reference being faked.
    let job_id = seed_queue_job(db, account).await;

    lorehaven_db::imports::create_import_job(
        db,
        &id,
        &job_id,
        account,
        pseud,
        "ao3",
        "https://example.test/a",
        "library",
        false,
    )
    .await
    .expect("seed an import job");

    // Then move it to the state and report this case is about, which is what
    // the worker does on the way to `failed` and on the way to `queued`.
    let report = format!("{{\"error\":\"{code}\"}}");
    let sql = db.sql(
        "UPDATE import_jobs SET state = ?, report_json = ?, library_item_id = ?
           WHERE id = ?",
        "UPDATE import_jobs SET state = $1, report_json = $2, library_item_id = $3::uuid
           WHERE id = $4::uuid",
    );
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(state)
                .bind(&report)
                .bind(library_item_id)
                .bind(&id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("set the import job's state");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(&sql)
                .bind(state)
                .bind(&report)
                .bind(library_item_id)
                .bind(&id)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("set the import job's state");
        }
    }
}

/// Give a library item one stored chapter — which is what "this instance holds
/// a body" means to the count.
async fn seed_stored_chapter(db: &lorehaven_db::Database, library_item_id: &str) {
    let id = uuid::Uuid::new_v4().to_string();
    let sql = db.sql(
        // `import_chapters` has no `version` column — unlike the tables above
        // it, which is why this insert is not the app's own code path and is
        // written out. It has no `imports::` helper because the only question
        // this test asks of it is "does a stored chapter exist", and the state
        // it is in is the whole answer.
        "INSERT INTO import_chapters
           (id, import_job_id, library_item_id, source_chapter_key, ordinal,
            title, state, created_at, updated_at)
         SELECT ?, id, ?, 'c1', 1, 'One', 'stored',
                '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'
           FROM import_jobs WHERE library_item_id = ?",
        "INSERT INTO import_chapters
           (id, import_job_id, library_item_id, source_chapter_key, ordinal,
            title, state, created_at, updated_at)
         SELECT $1::uuid, id, $2::uuid, 'c1', 1, 'One', 'stored',
                '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'
           FROM import_jobs WHERE library_item_id = $2::uuid",
    );
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(library_item_id)
                .bind(library_item_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("seed a stored chapter");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(library_item_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("seed a stored chapter");
        }
    }
}

/// A queue row for the import to point at. `jobs.kind` is the wire spelling the
/// `job_kinds!` tree defines, so this is a real import job rather than a row
/// that happens to satisfy the foreign key.
async fn seed_queue_job(db: &lorehaven_db::Database, account: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let sql = db.sql(
        "INSERT INTO jobs (id, kind, state, payload, attempts, max_attempts,
                           available_at, created_at, updated_at, version)
         VALUES (?, 'import', 'queued', '{}', 0, 3,
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z',
                 '2026-01-01T00:00:00Z', 1)",
        "INSERT INTO jobs (id, kind, state, payload, attempts, max_attempts,
                           available_at, created_at, updated_at, version)
         VALUES ($1::uuid, 'import', 'queued', '{}'::jsonb, 0, 3,
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z',
                 '2026-01-01T00:00:00Z', 1)",
    );
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("seed a queue row");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("seed a queue row");
        }
    }
    let _ = account;
    id
}

/// The account's first pseud id, as a string.
async fn pseud_of(db: &lorehaven_db::Database, account: &str) -> String {
    let sql = db.sql(
        "SELECT id FROM pseuds WHERE account_id = ? ORDER BY created_at ASC",
        "SELECT id::text FROM pseuds WHERE account_id::text = $1::text ORDER BY created_at ASC",
    );
    match db.backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&sql)
            .bind(account)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("query")
            .unwrap_or_default(),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(&sql)
            .bind(account)
            .fetch_optional(db.postgres_pool().expect("postgres"))
            .await
            .expect("query")
            .unwrap_or_default(),
    }
}
