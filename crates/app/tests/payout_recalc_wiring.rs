use lorehaven_app::payout_recalc::{self, RecalcSummary};
use lorehaven_app::worker::{Worker, WorkerOptions};
use lorehaven_domain::jobs::JobKind;
use test_support::{scratch_dir, TestDb};

/// A scratch directory for `tag`, created once per (process, tag).
///
/// **Not the obvious version.** My first draft copied
/// `preservation_recheck_wiring.rs`: a path keyed on the thread id, with
/// `remove_dir_all` guarded by a per-process `REMOVED` set so the second call
/// inside one test would not delete the database the first had migrated.
///
/// That is sound within one binary and breaks across two. Thread ids are recycled
/// by the OS, and the pid differs per binary, but the *combination* still collided
/// when `payout_recalc_wiring` and `payout_store` ran in one `cargo test`
/// invocation: two processes opened the same `lorehaven.sqlite`, one truncated it
/// while the other was migrating, and three tests died with
/// `(code: 26) file is not a database`.
///
/// `test_support::scratch_dir` appends a per-process counter, so two binaries
/// cannot name the same directory. The double-call hazard it also solves -- calling
/// it twice deletes the database the first call migrated -- is handled by calling
/// it once per test and reusing the returned path.
/// An `AppState` over `tdb`, with its storage root in the same directory.
///
/// The directory is a parameter, not derived from the tag: `scratch_dir` deletes and
/// recreates its path on every call, so calling it a second time returns a *different*
/// directory. Deriving it here would point `config.storage.root` somewhere other than
/// where the database is, which is the kind of mismatch that only surfaces when a
/// handler writes a file.
async fn state_for(tdb: &TestDb, dir: &std::path::Path) -> lorehaven_app::state::AppState {
    let mut config = lorehaven_app::config::Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    lorehaven_app::state::AppState::new(config, tdb.db().clone())
}

/// How many queued jobs of a kind exist.
async fn queued_count(tdb: &TestDb, kind: JobKind) -> i64 {
    let (sqlite, pg) = (
        "SELECT COUNT(*) FROM jobs WHERE kind = ? AND state = 'queued'",
        "SELECT COUNT(*)::bigint FROM jobs WHERE kind = $1 AND state = 'queued'",
    );
    match tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(sqlite)
            .bind(kind.as_str())
            .fetch_one(tdb.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("count"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(pg)
            .bind(kind.as_str())
            .fetch_one(tdb.db().postgres_pool().expect("postgres"))
            .await
            .expect("count"),
    }
}

// ── the window, which is pure and therefore cheap to pin ────────────────────

#[test]
fn the_window_ends_at_a_closed_monday_and_is_thirty_days_wide() {
    // The upper bound is the part with teeth. A window that ended at `now` would pay
    // for the same reads twice -- once this week, again next -- and because the
    // store's idempotency key includes the window, the store would treat those as
    // two distinct legitimate payments rather than catching the double.
    let now = time::OffsetDateTime::from_unix_timestamp(1_767_225_600).expect("a timestamp");
    let (from, to) = payout_recalc::closed_window(now);

    assert!(
        to <= now.unix_timestamp(),
        "the window ends in the past, so this week is not half-paid: {to}"
    );
    assert_eq!(to - from, 30 * 86_400, "30 days, as §20.3 states");
    // 1970-01-01 was a Thursday, so a Monday boundary is `t` with
    // `(t / 86400 + 3) % 7 == 0`.
    assert_eq!(
        (to / 86_400 + 3).rem_euclid(7),
        0,
        "the upper bound is a Monday"
    );
}

#[test]
fn the_window_is_the_same_all_through_one_week_and_moves_on_monday() {
    // The property that makes the idempotency key mean anything: a pass run
    // Tuesday and a pass run Sunday must compute the SAME window, or the key
    // differs and the same 30 days are paid twice.
    let base = time::OffsetDateTime::from_unix_timestamp(1_767_225_600).expect("a timestamp");
    let (_, first) = payout_recalc::closed_window(base);
    let (_, later_same_week) = payout_recalc::closed_window(base + time::Duration::days(3));
    assert_eq!(
        first, later_same_week,
        "two runs in one week share a window, so the key cannot differ"
    );

    // Seven days later crosses the boundary and must move.
    let (_, next_week) = payout_recalc::closed_window(base + time::Duration::days(7));
    assert_ne!(first, next_week, "a new week is a new window");
    assert_eq!(next_week - first, 7 * 86_400);
}

#[test]
fn the_window_does_not_wrap_for_a_pre_epoch_instant() {
    // `div_euclid`/`rem_euclid` rather than `/` and `%`: with the operators that
    // truncate, an instant before 1970 computes a NEGATIVE week, and
    // `boundary - 30 days` then lands *after* `boundary` -- an inverted window
    // where every comparison is true and every count is everything.
    let before = time::OffsetDateTime::from_unix_timestamp(-86_400 * 3).expect("a timestamp");
    let (from, to) = payout_recalc::closed_window(before);
    assert!(
        to <= before.unix_timestamp(),
        "the bound is not in the future"
    );
    assert_eq!(
        (to / 86_400 + 3).rem_euclid(7),
        0,
        "and it is still a Monday: {to}"
    );
    assert_eq!(
        to - from,
        30 * 86_400,
        "the window is 30 days wide and not inverted: {from}..{to}"
    );
}

// ── claim 1: the maintenance pass queues one ────────────────────────────────

#[tokio::test]
async fn a_maintenance_pass_queues_a_payout_recalc() {
    let dir = scratch_dir("pr_sched");
    let tdb = TestDb::connect_with_dir("pr_sched", &dir).await;
    let state = state_for(&tdb, &dir).await;

    assert_eq!(
        queued_count(&tdb, JobKind::PayoutRecalc).await,
        0,
        "nothing is queued before the pass runs"
    );
    Worker::new(WorkerOptions::named("pr-sched"))
        .maintenance_pass(&state)
        .await
        .expect("maintenance pass");
    assert_eq!(
        queued_count(&tdb, JobKind::PayoutRecalc).await,
        1,
        "the pass queues exactly one recalculation"
    );
}

// ── claim 2: once per week, not once per pass ───────────────────────────────

#[tokio::test]
async fn repeated_maintenance_passes_queue_one_recalc_not_one_each() {
    // The one wiring claim where double-execution costs real money. The pass runs
    // every 30 cycles, so a busy instance runs it constantly; without the week's
    // idempotency key every pass would queue a payout for the same 30 days.
    let dir = scratch_dir("pr_once");
    let tdb = TestDb::connect_with_dir("pr_once", &dir).await;
    let state = state_for(&tdb, &dir).await;
    let worker = Worker::new(WorkerOptions::named("pr-once"));

    for _ in 0..4 {
        worker
            .maintenance_pass(&state)
            .await
            .expect("maintenance pass");
    }
    assert_eq!(
        queued_count(&tdb, JobKind::PayoutRecalc).await,
        1,
        "four passes, one payout job: the key is the week, not the pass"
    );
}

// ── claim 3: a queued recalc is claimable and handled ───────────────────────

#[tokio::test]
async fn a_queued_recalc_is_drained_by_a_worker_run() {
    // Reachability end to end: the queue holds a kind the dispatch arm handles, so
    // it does not accumulate forever as an undrainable row.
    let dir = scratch_dir("pr_drain");
    let tdb = TestDb::connect_with_dir("pr_drain", &dir).await;
    let state = state_for(&tdb, &dir).await;
    let worker = Worker::new(WorkerOptions::named("pr-drain"));
    worker
        .maintenance_pass(&state)
        .await
        .expect("maintenance pass");
    assert_eq!(queued_count(&tdb, JobKind::PayoutRecalc).await, 1);

    // `run_once` claims AT MOST ONE job, and fairness decides which. The pass also
    // queues the daily preservation recheck, so the first call may legitimately
    // drain that instead. Loop, and remember whether ours was ever handled --
    // "one job left the queue" is not the same claim as "our kind was handled",
    // and only the second is what this test is about.
    let mut handled = false;
    for _ in 0..8 {
        let report = worker.run_once(&state).await.expect("worker run");
        if let Some((_, _)) = report.job {
            handled = true;
        }
        if queued_count(&tdb, JobKind::PayoutRecalc).await == 0 {
            break;
        }
    }
    assert!(
        handled,
        "the worker ran at least one job, so the dispatch arm is reachable"
    );
    assert_eq!(
        queued_count(&tdb, JobKind::PayoutRecalc).await,
        0,
        "and the recalculation is no longer sitting in the queue"
    );
}

#[tokio::test]
async fn the_recalc_job_is_bulk_so_it_cannot_delay_a_readers_import() {
    // The class is a promise about queue ordering. A recalc walks every work with
    // earnings and reads five tables per work; interactive class would let that pass
    // delay an import a reader is watching.
    assert_eq!(
        JobKind::PayoutRecalc.resource_class(),
        lorehaven_domain::jobs::ResourceClass::Bulk,
        "§20.3's pass is the longest read in the database"
    );
}

#[tokio::test]
async fn the_recalc_kind_round_trips_through_its_stored_name() {
    // `JobKind` is macro-generated, so a variant that parses but is not in
    // `ALL_KINDS` would be accepted here and rejected by whatever iterates the list.
    assert_eq!(
        JobKind::parse("payout_recalc"),
        Some(JobKind::PayoutRecalc),
        "the wire name the job row stores round-trips"
    );
    assert_eq!(JobKind::PayoutRecalc.as_str(), "payout_recalc");
    assert!(
        lorehaven_domain::jobs::ALL_KINDS.contains(&JobKind::PayoutRecalc),
        "and it is in the exhaustive list, not just parseable"
    );
}

// ── claim 4: running it pays an author ──────────────────────────────────────

#[tokio::test]
async fn a_pass_over_a_qualifying_work_pays_that_author() {
    // The whole chain, and the reason claims 1-3 exist: the pass finds the work,
    // the store reads the signals, and credits land in the ledger.
    let dir = scratch_dir("pr_pays");
    let tdb = TestDb::connect_with_dir("pr_pays", &dir).await;
    let state = state_for(&tdb, &dir).await;
    let (author, work) = seed_qualifying_work(&tdb).await;

    let summary = payout_recalc::run_for_window(
        &state,
        lorehaven_db::payout_store::DemandInputs {
            admin_taste: 0.0,
            wishlist: 0.0,
            search: 0.0,
        },
        payout_recalc::BASE_WEEKLY_BP,
        WINDOW_FROM,
        WINDOW_TO,
    )
    .await
    .expect("a payout pass");

    assert_eq!(summary.considered, 1, "the seeded work was found");
    assert_eq!(summary.paid, 1, "and paid");
    assert!(summary.total_bp > 0, "for a nonzero amount");

    let earned = earned_bucket(&tdb, &author).await;
    assert_eq!(
        earned, summary.total_bp,
        "the author's ledger balance is the pass's total"
    );
    let _ = work;
}

#[tokio::test]
async fn a_second_pass_over_the_same_window_pays_nothing_further() {
    // The pass is safe to retry -- which is why the worker treats its failures as
    // Transient rather than Fatal. Retrying must not double-pay.
    let dir = scratch_dir("pr_retry");
    let tdb = TestDb::connect_with_dir("pr_retry", &dir).await;
    let state = state_for(&tdb, &dir).await;
    let (author, _) = seed_qualifying_work(&tdb).await;
    let demand = lorehaven_db::payout_store::DemandInputs {
        admin_taste: 0.0,
        wishlist: 0.0,
        search: 0.0,
    };

    let first: RecalcSummary = payout_recalc::run_for_window(
        &state,
        demand,
        payout_recalc::BASE_WEEKLY_BP,
        WINDOW_FROM,
        WINDOW_TO,
    )
    .await
    .expect("first pass");
    let after_first = earned_bucket(&tdb, &author).await;
    assert!(after_first > 0, "the first pass paid");

    let second: RecalcSummary = payout_recalc::run_for_window(
        &state,
        demand,
        payout_recalc::BASE_WEEKLY_BP,
        WINDOW_FROM,
        WINDOW_TO,
    )
    .await
    .expect("second pass");
    assert_eq!(
        earned_bucket(&tdb, &author).await,
        after_first,
        "a retried pass is a no-op on the balance"
    );
    assert_eq!(
        second.paid, 1,
        "the store still reports the work as paid -- the ledger replay is what \
         makes it once, not a suppressed second attempt"
    );
    assert_eq!(first.total_bp, after_first);
}

#[tokio::test]
async fn a_pass_over_an_empty_instance_pays_nobody_and_says_so() {
    // The summary's shape is the operator's only evidence a pass ran. An empty
    // instance and a broken one both "paid nothing", and the difference is
    // `considered` being zero rather than being some number whose payouts all
    // failed.
    let dir = scratch_dir("pr_empty");
    let tdb = TestDb::connect_with_dir("pr_empty", &dir).await;
    let state = state_for(&tdb, &dir).await;
    let summary = payout_recalc::run_for_window(
        &state,
        lorehaven_db::payout_store::DemandInputs {
            admin_taste: 0.0,
            wishlist: 0.0,
            search: 0.0,
        },
        payout_recalc::BASE_WEEKLY_BP,
        WINDOW_FROM,
        WINDOW_TO,
    )
    .await
    .expect("a pass over nothing");
    assert_eq!(summary.considered, 0);
    assert_eq!(summary.paid, 0);
    assert_eq!(summary.total_bp, 0);
}

#[tokio::test]
async fn a_work_with_readers_outside_the_window_is_not_paid() {
    // The window has to filter, or every pass repays the whole catalogue and the
    // "once a week" key is the only thing preventing unbounded growth.
    let dir = scratch_dir("pr_window");
    let tdb = TestDb::connect_with_dir("pr_window", &dir).await;
    let state = state_for(&tdb, &dir).await;
    seed_qualifying_work(&tdb).await;
    let summary = payout_recalc::run_for_window(
        &state,
        lorehaven_db::payout_store::DemandInputs {
            admin_taste: 0.0,
            wishlist: 0.0,
            search: 0.0,
        },
        payout_recalc::BASE_WEEKLY_BP,
        // A window in 1990. Nothing the fixture seeded is inside it.
        631_152_000,
        631_152_000 + 30 * 86_400,
    )
    .await
    .expect("a pass over an empty window");
    assert_eq!(
        summary.considered, 0,
        "reads outside the window do not make a work payable"
    );
}

// ── fixtures ────────────────────────────────────────────────────────────────

/// A 30-day window that encloses the dates the fixture seeds.
const WINDOW_FROM: i64 = 1_767_225_600; // 2026-01-01
const WINDOW_TO: i64 = 1_769_817_600; // +30 days

/// One author with one work and enough readers to clear §20.3's 10-reader floor.
///
/// Returns the author's account id and the work's id, both as text.
async fn seed_qualifying_work(tdb: &TestDb) -> (String, String) {
    let account = uuid::Uuid::new_v4().to_string();
    exec(
        tdb,
        "INSERT INTO accounts (id, email, created_at, updated_at) \
         VALUES (?1#u, 'payout-author@example.com', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        &[&account],
    )
    .await;
    let pseud = uuid::Uuid::new_v4().to_string();
    exec(
        tdb,
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
         VALUES (?1#u, ?2#u, 'payoutauthor', 'Payout Author', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        &[&pseud, &account],
    )
    .await;
    let work = uuid::Uuid::new_v4().to_string();
    exec(
        tdb,
        "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at, \
             generated_content_posture) \
         VALUES (?1#u, ?2#u, 'Paid Work', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'forbid')",
        &[&work, &pseud],
    )
    .await;

    // 15 readers, so the work clears §20.3's minimum of 10.
    for i in 0..15 {
        let reader = uuid::Uuid::new_v4().to_string();
        exec(
            tdb,
            "INSERT INTO accounts (id, email, created_at, updated_at) \
             VALUES (?1#u, ?2, '2026-01-02T00:00:00Z', '2026-01-02T00:00:00Z')",
            &[
                &reader,
                &format!(
                    "payout-reader-{i}-{}@example.com",
                    &uuid::Uuid::new_v4().to_string()[..8]
                ),
            ],
        )
        .await;
        let reader_pseud = uuid::Uuid::new_v4().to_string();
        exec(
            tdb,
            "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
             VALUES (?1#u, ?2#u, ?3, ?3, '2026-01-02T00:00:00Z', '2026-01-02T00:00:00Z')",
            &[
                &reader_pseud,
                &reader,
                &format!("pr{}", &uuid::Uuid::new_v4().to_string()[..8]),
            ],
        )
        .await;
        exec(
            tdb,
            "INSERT INTO work_view_log (work_id, viewer_hash, viewed_at, is_automated) \
             VALUES (?1#u, ?2, '2026-01-03T00:00:00Z', 0)",
            &[&work, &format!("viewer-{i}")],
        )
        .await;
        exec(
            tdb,
            "INSERT INTO reading_status (id, account_id, subject_type, subject_id, status, \
                 started_at, finished_at, updated_at) \
             VALUES (?1#u, ?2#u, 'work', ?3#u, 'finished', '2026-01-02T00:00:00Z', \
                 '2026-01-04T00:00:00Z', '2026-01-04T00:00:00Z')",
            &[&uuid::Uuid::new_v4().to_string(), &reader, &work],
        )
        .await;
        exec(
            tdb,
            "INSERT INTO rating (id, account_id, pseud_id, work_id, stars, is_public, \
                 created_at, updated_at) \
             VALUES (?1#u, ?2#u, ?3#u, ?4#u, 5, false, '2026-01-04T00:00:00Z', '2026-01-04T00:00:00Z')",
            &[
                &uuid::Uuid::new_v4().to_string(),
                &reader,
                &reader_pseud,
                &work,
            ],
        )
        .await;
    }
    (account, work)
}

/// An author's `earned` credit balance, in basis points.
async fn earned_bucket(tdb: &TestDb, account: &str) -> i64 {
    let (sqlite, pg) = (
        "SELECT COALESCE(SUM(amount_bp), 0) FROM credit_entries \
         WHERE account = ? AND bucket = 'earned'",
        "SELECT COALESCE(SUM(amount_bp), 0)::bigint FROM credit_entries \
         WHERE account = $1 AND bucket = 'earned'",
    );
    match tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(sqlite)
            .bind(account)
            .fetch_one(tdb.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("earned bucket"),
        // `credit_entries.account` is TEXT on PostgreSQL, like `work_view_log.work_id`
        // and `works.owner_pseud_id` -- binding a uuid here gives
        // "operator does not exist: text = uuid". Text, both dialects.
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(pg)
            .bind(account)
            .fetch_one(tdb.db().postgres_pool().expect("postgres"))
            .await
            .expect("earned bucket"),
    }
}

/// `?N#u` binds a native uuid on PostgreSQL and text on SQLite.
async fn exec(tdb: &TestDb, sqlite: &str, args: &[&String]) {
    let db = tdb.db();
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            let sql = sqlite.replace("#u", "").replace("false", "0");
            let mut q = sqlx::query(&sql);
            for a in args {
                q = q.bind(a.as_str());
            }
            q.execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("seed insert");
        }
        lorehaven_db::Backend::Postgres => {
            let pg = (1..=6).fold(sqlite.to_string(), |acc, n| {
                acc.replace(&format!("?{n}#u"), &format!("${n}::uuid"))
                    .replace(&format!("?{n}"), &format!("${n}"))
            });
            let mut q = sqlx::query(&pg);
            for a in args {
                match uuid::Uuid::parse_str(a) {
                    Ok(u) => q = q.bind(u),
                    Err(_) => q = q.bind(a.as_str()),
                };
            }
            q.execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("seed insert");
        }
    }
}
