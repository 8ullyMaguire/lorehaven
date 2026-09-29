//! The recheck is *scheduled*: the maintenance pass queues one, a queued job
//! runs, and the whole path is reachable from a real `Worker`.
//!
//! **This file exists because a job kind, a handler and a green handler test
//! add up to a feature that never runs.** Every earlier piece of M59 Phase D was
//! provable without a scheduler: the kind parses, the handler handles, the
//! compare function classifies. None of that says a single recheck will ever
//! happen, and an unscheduled recheck means every credit ever paid is never
//! reclaimed — the anti-farm mechanism §2.3 requires, inert.
//!
//! So the claims here are about *reachability*, and each is a way the wiring can
//! be absent while every other test still passes:
//!
//! 1. `maintenance_pass` enqueues a `preservation_recheck`. Without this the
//!    job queue never holds one and no recheck ever runs.
//! 2. It is **once a day, not once per pass**. The pass runs every 30 cycles,
//!    which on a busy instance is minutes; §11.5's per-host pacing is a promise
//!    to third parties about how often we ask, and "every few minutes" would
//!    break it.
//! 3. A queued recheck job is **claimable and handled** — the dispatch arm
//!    exists, so the queue does not fill with a kind nothing drains.

use std::path::PathBuf;

use lorehaven_app::worker::{Worker, WorkerOptions};
use lorehaven_domain::jobs::JobKind;
use test_support::TestDb;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-presrecheck-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

async fn state_for(tdb: &TestDb, tag: &str) -> lorehaven_app::state::AppState {
    let mut config = lorehaven_app::config::Config::development_defaults();
    config.storage.root = scratch_dir(tag);
    lorehaven_app::state::AppState::new(config, tdb.db().clone())
}

/// How many queued jobs of a kind exist.
async fn queued_count(tdb: &TestDb, kind: JobKind) -> i64 {
    let sql = tdb.db().sql(
        "SELECT COUNT(*) FROM jobs WHERE kind = ? AND state = 'queued'",
        "SELECT COUNT(*)::bigint FROM jobs WHERE kind = $1 AND state = 'queued'",
    );
    match tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&sql)
            .bind(kind.as_str())
            .fetch_one(tdb.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("count"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(&sql)
            .bind(kind.as_str())
            .fetch_one(tdb.db().postgres_pool().expect("postgres"))
            .await
            .expect("count"),
    }
}

/// The maintenance pass queues a recheck. The first claim, and the one whose
/// absence makes the entire feature inert.
#[tokio::test]
async fn a_maintenance_pass_queues_a_preservation_recheck() {
    let tdb = TestDb::connect_with_dir("recheck_sched", &scratch_dir("recheck_sched")).await;
    let state = state_for(&tdb, "recheck_sched").await;

    assert_eq!(
        queued_count(&tdb, JobKind::PreservationRecheck).await,
        0,
        "nothing is queued before the pass runs"
    );
    Worker::new(WorkerOptions::named("recheck-sched"))
        .maintenance_pass(&state)
        .await
        .expect("maintenance pass");
    assert_eq!(
        queued_count(&tdb, JobKind::PreservationRecheck).await,
        1,
        "the pass must queue a recheck, or §2.3's clawback never fires and every \
         preservation reward is permanent"
    );
}

/// Several passes on one day still mean one recheck. The key is the date, and
/// `enqueue` is a no-op when it exists.
#[tokio::test]
async fn repeated_maintenance_passes_on_one_day_queue_one_recheck() {
    let tdb = TestDb::connect_with_dir("recheck_once", &scratch_dir("recheck_once")).await;
    let state = state_for(&tdb, "recheck_once").await;
    let worker = Worker::new(WorkerOptions::named("recheck-once"));

    for _ in 0..5 {
        worker
            .maintenance_pass(&state)
            .await
            .expect("maintenance pass");
    }
    assert_eq!(
        queued_count(&tdb, JobKind::PreservationRecheck).await,
        1,
        "the pass runs every 30 cycles -- minutes on a busy instance -- and §11.5's \
         pacing is a promise about how often we ask a third party. Five passes on \
         one day must mean one recheck, not five rounds of asking every archive."
    );
}

/// A queued recheck is drained by a real worker pass, and its job retires.
///
/// `run_once` is the worker's public entry point, so this drives the same
/// claim → dispatch → outcome path production uses rather than reaching into
/// the queue. With no verified targets the handler is a no-op that succeeds,
/// which is exactly the case worth pinning: **a recheck that finds nothing to
/// do must still retire its job**, or the queue backs up with work that has
/// already completed.
#[tokio::test]
async fn a_queued_recheck_is_drained_by_a_worker_pass() {
    let tdb = TestDb::connect_with_dir("recheck_claim", &scratch_dir("recheck_claim")).await;
    let state = state_for(&tdb, "recheck_claim").await;
    let worker = Worker::new(WorkerOptions::named("recheck-claim"));

    worker
        .maintenance_pass(&state)
        .await
        .expect("maintenance pass");
    assert_eq!(
        queued_count(&tdb, JobKind::PreservationRecheck).await,
        1,
        "precondition: the pass queued one"
    );

    worker
        .run_once(&state)
        .await
        .expect("a worker pass must not fail on an empty recheck");

    assert_eq!(
        queued_count(&tdb, JobKind::PreservationRecheck).await,
        0,
        "a recheck that found nothing to verify must still retire its job; a job that \
         completes is the only thing that keeps the queue from filling with work \
         that is already done, and a job left queued forever is a kind the dispatch \
         never reached"
    );
}
