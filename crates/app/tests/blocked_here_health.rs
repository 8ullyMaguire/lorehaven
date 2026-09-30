//! M53-04: a `blocked-here` source is excluded from the health sweep, and the
//! exclusion is reported rather than silently applied.
//!
//! ## What is actually being tested
//!
//! Spec §11.8: *"Do not label a source unavailable because one user's
//! credentials expired."* The same sentence applies harder to a wall this **build
//! host** cannot get past, and this suite pins the place that matters: the
//! health sweep.
//!
//! The failure this prevents is specific and it is not hypothetical. A host that
//! cannot reach `example.com` fails every import it attempts against it. §11.8's
//! rule is three failures with no success is `unavailable`. So without the skip,
//! the first three attempts from a walled host rewrite a source that is serving
//! every other reader perfectly to `unavailable` — permanently, on a
//! non-actionable basis, and about a site that is up.
//!
//! **Asserting only that `verification_status` was written would pass whether or
//! not health was corrupted.** That is why the central test here drives the real
//! sweep with real failed imports behind it and then reads `health` back.
//!
//! ## Why the excluded set is returned rather than swallowed
//!
//! A sweep that silently skipped three sources is indistinguishable from one that
//! had no reason to. So `recompute_source_health_with_skips` returns what it
//! declined to touch, and an operator reading "14 sources, all healthy" can see
//! that one was never examined. This is §11.7's argument — "adapter counts are an
//! outcome of verified implementation" — applied to health.
//!
//! ## Both engines
//!
//! `verification_status` is a `TEXT` column in both arms, but the test that reads
//! it back goes through the dialect-spelled `Source` decode, so running only on
//! SQLite would leave the Postgres select list unexamined — and a column added to
//! one arm's `SOURCE_COLUMNS` but not the other fails only there.

use lorehaven_db::identity::{self, AccountStatus};
use lorehaven_db::imports::{
    self, recompute_source_health_with_skips, BLOCKED_HERE, HEALTH_WINDOW_DAYS,
};
use lorehaven_db::jobs;
use lorehaven_domain::jobs::{JobKind, RetryPolicy};
use lorehaven_domain::policy::AgeState;
use test_support::TestDb;

/// A source this build cannot reach.
const BLOCKED: &str = "walled-source";

/// A source that works. Present so the sweep has something it *should* touch,
/// which is what makes "the sweep skipped one and not the other" a claim rather
/// than an observation about a single row.
const REACHABLE: &str = "open-source";

/// The wall, in the words an operator would use.
const REASON: &str = "Cloudflare interstitial, HTTP 403, from this build host";

/// A database with migrations applied and no source rows.
///
/// `connect_with_dir` rather than a bare in-memory database because the
/// PostgreSQL arm has to run for real: `verification_status` is spelled the same
/// in both files but is reached through the dialect-spelled select list, and a
/// SQLite-only run would leave that select list unexamined.
async fn harness(tag: &str) -> TestDb {
    let _ = lorehaven_app::logging::init(&lorehaven_app::config::LoggingConfig {
        filter: "error".to_owned(),
        format: lorehaven_app::config::LogFormat::Pretty,
    });
    let dir = test_support::scratch_dir(&format!("m53-04-{tag}"));
    TestDb::connect_with_dir(tag, &dir).await
}

async fn seed(db: &TestDb, key: &str) {
    imports::upsert_source(db.db(), key, key, "0.1.0", "{}")
        .await
        .expect("seed the source row");
}

/// Seed `count` *finished, failed* imports against a source.
///
/// This is the part the central test cannot fake. §11.8's rule is "three failures
/// with no success is `unavailable`", so a test that asserts health survives the
/// sweep has to actually put three failures behind it — otherwise there is no
/// evidence for the sweep to act on, it changes nothing, and the assertion passes
/// on an implementation that skips nothing at all.
///
/// The rows are written through the real `create_account`, `create_pseud`,
/// `create_import_job` and `set_import_state`, because `import_jobs` has foreign
/// keys onto `accounts` and `pseuds` and the timestamp the sweep's window reads
/// is `updated_at` — both of which a hand-written `INSERT` would have to fake.
async fn seed_failed_imports(db: &TestDb, source_key: &str, count: usize) {
    let account = identity::create_account(
        db.db(),
        &format!("reader-{source_key}@example.org"),
        AgeState::DeclaredAdult,
        AccountStatus::Active,
    )
    .await
    .expect("create the reader");
    let pseud = identity::create_pseud(db.db(), account, "reader", "Reader")
        .await
        .expect("create the pseud");

    for index in 0..count {
        let id = lorehaven_domain::ImportJobId::new().to_string();
        // The queue row has to be real: `import_jobs.job_id` is a foreign key
        // onto `jobs`, so a made-up id is refused by the database — which is
        // correct, because an import pointing at a job nobody can claim is not an
        // import. Creating it through `enqueue` also means the row this test
        // reads back is one the worker could actually have picked up.
        let job_id = jobs::enqueue(
            db.db(),
            JobKind::Import,
            "{}",
            None,
            Some(account),
            0,
            &RetryPolicy::default(),
        )
        .await
        .expect("enqueue the import's job");

        imports::create_import_job(
            db.db(),
            &id,
            &job_id.to_string(),
            &account.to_string(),
            &pseud.to_string(),
            source_key,
            &format!("https://{source_key}/work/{index}"),
            "library",
            false,
        )
        .await
        .expect("create the import");
        imports::set_import_state(db.db(), &id, "failed", None, None)
            .await
            .expect("mark it failed");
    }
}

/// The central test: a blocked source keeps `unknown` health through a sweep that
/// has [`FAILURES_TO_UNAVAILABLE`] failed imports behind it.
///
/// That constant is exactly the threshold that would otherwise flip this source
/// to `unavailable`. Fewer failures would not prove anything; more would be a
/// weaker version of the same claim. The count is passed in rather than hardcoded
/// so the two cannot drift apart — a test that hardcoded `3` would still pass if
/// §11.8's threshold moved to 4.
#[tokio::test]
async fn a_blocked_sources_health_is_untouched_by_enough_failed_imports() {
    let db = harness("blocked-health").await;
    seed(&db, BLOCKED).await;

    assert_eq!(
        imports::find_source(db.db(), BLOCKED)
            .await
            .expect("read")
            .expect("the row exists")
            .health,
        "unknown",
        "the starting point is health nobody has claimed"
    );

    imports::set_source_verification(db.db(), BLOCKED, Some(BLOCKED_HERE), Some(REASON))
        .await
        .expect("mark it blocked-here");

    seed_failed_imports(
        &db,
        BLOCKED,
        usize::try_from(imports::FAILURES_TO_UNAVAILABLE).expect("it is positive"),
    )
    .await;

    let (_changes, skipped) = recompute_source_health_with_skips(db.db(), HEALTH_WINDOW_DAYS)
        .await
        .expect("the sweep runs");

    let after = imports::find_source(db.db(), BLOCKED)
        .await
        .expect("read")
        .expect("the row exists");

    assert_eq!(
        after.health, "unknown",
        "a source this host cannot reach had its health rewritten by a sweep \
         that never examined it: {after:?}"
    );
    assert_eq!(
        skipped.len(),
        1,
        "the sweep must say what it declined to touch, not skip it quietly"
    );
    assert_eq!(skipped[0].key, BLOCKED);
    assert_eq!(
        skipped[0].reason, REASON,
        "the reason an operator acts on has to survive to the report"
    );

    db.cleanup().await;
}

/// The same failures against a source this build *can* reach do flip its health.
///
/// Without this the central test could pass on a sweep that never writes health
/// at all. The two together are what make it a claim about the skip rather than
/// about the sweep being inert.
#[tokio::test]
async fn the_same_failures_do_flip_a_source_this_build_can_reach() {
    let db = harness("blocked-control").await;
    seed(&db, REACHABLE).await;

    seed_failed_imports(
        &db,
        REACHABLE,
        usize::try_from(imports::FAILURES_TO_UNAVAILABLE).expect("it is positive"),
    )
    .await;

    let (changes, skipped) = recompute_source_health_with_skips(db.db(), HEALTH_WINDOW_DAYS)
        .await
        .expect("the sweep runs");

    assert_eq!(
        imports::find_source(db.db(), REACHABLE)
            .await
            .expect("read")
            .expect("the row exists")
            .health,
        "unavailable",
        "the control case must actually flip, or the central test proves nothing"
    );
    assert!(
        skipped.is_empty(),
        "a reachable source is never skipped: {skipped:?}"
    );
    assert_eq!(changes.len(), 1, "and the sweep reports the change it made");

    db.cleanup().await;
}

/// The sweep skips the blocked source and still does its job to the other one.
///
/// Without this, `a_blocked_sources_health_is_untouched_by_three_failed_imports`
/// would pass on an implementation that skips *everything* — which is a sweep
/// that reports every source healthy forever, and would satisfy the first test
/// completely.
#[tokio::test]
async fn the_sweep_still_examines_a_source_it_can_reach() {
    let db = harness("blocked-partial").await;
    seed(&db, BLOCKED).await;
    seed(&db, REACHABLE).await;

    imports::set_source_verification(db.db(), BLOCKED, Some(BLOCKED_HERE), Some(REASON))
        .await
        .expect("mark one blocked-here");

    // No imports at all: the honest outcome is no change and no skip, because
    // there was no evidence to act on either way.
    let (changes, skipped) = recompute_source_health_with_skips(db.db(), HEALTH_WINDOW_DAYS)
        .await
        .expect("the sweep runs");

    assert_eq!(
        changes.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(),
        Vec::<&str>::new(),
        "with no finished imports there is nothing to change"
    );
    assert_eq!(
        skipped.len(),
        1,
        "only the blocked source is skipped, and it is the only one"
    );
    assert_eq!(skipped[0].key, BLOCKED);

    db.cleanup().await;
}

/// A `blocked-here` with no reason is refused.
///
/// §11.7 says counts are an outcome of verified implementation, and a
/// `blocked-here` nobody can explain is indistinguishable from a source that was
/// never checked. Storing one produces an operator with a flag and nothing to do
/// about it.
#[tokio::test]
async fn blocked_here_without_a_reason_is_refused() {
    let db = harness("blocked-noreason").await;
    seed(&db, BLOCKED).await;

    for note in [None, Some(""), Some("   ")] {
        let result =
            imports::set_source_verification(db.db(), BLOCKED, Some(BLOCKED_HERE), note).await;
        assert!(
            result.is_err(),
            "blocked-here was accepted with note {note:?}, which no operator can act on"
        );
    }

    // And nothing was written on the way to those refusals.
    let row = imports::find_source(db.db(), BLOCKED)
        .await
        .expect("read")
        .expect("the row exists");
    assert_eq!(
        row.verification_status, None,
        "a refused call still changed the row"
    );

    db.cleanup().await;
}

/// The default is the honest one: a source nobody has classified keeps `NULL`,
/// and is treated as verified for support counts.
///
/// 92 migrations write `sources` rows. A NOT NULL sentinel would either rewrite
/// all of them or leave every reader handling two spellings of "fine", so NULL
/// means the default — and this asserts that a freshly seeded row really does
/// read as unclassified rather than as an empty string.
#[tokio::test]
async fn an_unclassified_source_is_null_rather_than_a_sentinel() {
    let db = harness("blocked-default").await;
    seed(&db, REACHABLE).await;

    let row = imports::find_source(db.db(), REACHABLE)
        .await
        .expect("read")
        .expect("the row exists");

    assert_eq!(
        row.verification_status, None,
        "an unclassified source must read as NULL, not as a sentinel string"
    );
    assert_eq!(row.verification_note, None);

    db.cleanup().await;
}

/// `blocked-here` is not `unavailable`, and the two never appear together in the
/// sweep's own vocabulary.
///
/// The distinction is the row's whole point: §11.8's `unavailable` is a public
/// claim that a site is down for everyone, and `blocked-here` is the truth, which
/// is that *we* could not check. A build that reported the second as the first
/// would be wrong about a healthy site on every reader's dashboard.
#[tokio::test]
async fn blocked_here_is_a_different_claim_from_unavailable() {
    let db = harness("blocked-vocab").await;
    seed(&db, BLOCKED).await;
    imports::set_source_verification(db.db(), BLOCKED, Some(BLOCKED_HERE), Some(REASON))
        .await
        .expect("mark it");

    let (_changes, skipped) = recompute_source_health_with_skips(db.db(), HEALTH_WINDOW_DAYS)
        .await
        .expect("the sweep runs");

    assert!(
        changes_are_free_of(&skipped, "unavailable"),
        "the sweep's skip report used health vocabulary"
    );
    assert_eq!(
        imports::find_source(db.db(), BLOCKED)
            .await
            .expect("read")
            .expect("the row exists")
            .health,
        "unknown",
        "and health never took the unavailable spelling"
    );

    db.cleanup().await;
}

/// A source can be moved back to verified, and then the sweep examines it again.
///
/// A one-way flag would strand an operator whose host is no longer behind the
/// same wall: there would be no supported way to undo the claim, and the source
/// would be excluded from counts forever. This is the check that the flag is
/// state and not a verdict.
#[tokio::test]
async fn clearing_blocked_here_puts_the_source_back_in_scope() {
    let db = harness("blocked-clear").await;
    seed(&db, BLOCKED).await;

    imports::set_source_verification(db.db(), BLOCKED, Some(BLOCKED_HERE), Some(REASON))
        .await
        .expect("mark it");
    let (_c, skipped) = recompute_source_health_with_skips(db.db(), HEALTH_WINDOW_DAYS)
        .await
        .expect("sweep");
    assert_eq!(skipped.len(), 1, "it is skipped while blocked");

    imports::set_source_verification(db.db(), BLOCKED, None, None)
        .await
        .expect("clear it");

    let row = imports::find_source(db.db(), BLOCKED)
        .await
        .expect("read")
        .expect("the row exists");
    assert_eq!(row.verification_status, None);
    assert_eq!(
        row.verification_note, None,
        "clearing the claim must clear the note too, or a stale reason outlives it"
    );

    let (_changes, skipped) = recompute_source_health_with_skips(db.db(), HEALTH_WINDOW_DAYS)
        .await
        .expect("sweep");
    assert!(
        skipped.is_empty(),
        "a source that is no longer blocked must not still be skipped: {skipped:?}"
    );

    db.cleanup().await;
}

/// The existing `recompute_source_health` keeps its signature and its behaviour.
///
/// It now delegates, so the skip has one implementation rather than two. This
/// asserts the delegating wrapper still returns the change list alone — a
/// wrapper that started returning skips would change every caller's type without
/// any of them asking.
#[tokio::test]
async fn the_existing_sweep_still_returns_only_its_changes() {
    let db = harness("blocked-wrapper").await;
    seed(&db, BLOCKED).await;
    imports::set_source_verification(db.db(), BLOCKED, Some(BLOCKED_HERE), Some(REASON))
        .await
        .expect("mark it");

    let changes = imports::recompute_source_health(db.db(), HEALTH_WINDOW_DAYS)
        .await
        .expect("the sweep runs");
    assert!(
        changes.is_empty(),
        "there was no evidence to act on: {changes:?}"
    );

    db.cleanup().await;
}

/// Helper kept trivial on purpose: the assertion is about vocabulary appearing in
/// the skip report, and a helper that did more would be a second thing to test.
fn changes_are_free_of(skipped: &[imports::SkippedSource], word: &str) -> bool {
    !skipped
        .iter()
        .any(|skip| skip.key.contains(word) || skip.reason.contains(word))
}
