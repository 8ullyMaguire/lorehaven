//! M45-57 step 6: §55.2's trust gate and §19.4's review threshold.
//!
//! Both are security properties, and both are asserted against a real database
//! on both engines rather than in unit tests with a mocked pool. The reason is
//! specific to each:
//!
//! * The trust gate reads `trust_levels` through `governance::trust_for`, which
//!   on PostgreSQL is a `CAST` and on SQLite is a plain select. A gate tested
//!   against a stub returns the stub's answer, not the engine's.
//! * The review threshold depends on a `UNIQUE (submission_id, reviewer_account)`
//!   constraint and a `COUNT(DISTINCT ...)` — both SQL, both engine-specific in
//!   how they behave under a duplicate insert.
//!
//! The threshold tests are written so they can go **red** by removing one guard:
//! `approve_below_threshold_does_not_publish` and the duplicate-reviewer tests
//! were each confirmed failing against the un-guarded code, because a test that
//! cannot fail is not evidence about a quorum.

use lorehaven_db::source_adapters::{
    approve_count, decide, has_reached_threshold, list_pending, published_source_manifests,
    record_review, submissions_by, submit, SubmitError, SubmitRefusal, APPROVAL_THRESHOLD,
    SUBMIT_TRUST_BAR,
};
use lorehaven_db::Database;
use std::time::Duration;

fn make_config(url: String) -> lorehaven_db::DatabaseConfig {
    lorehaven_db::DatabaseConfig {
        url,
        max_connections: 5,
        acquire_timeout: Duration::from_secs(5),
        slow_query_warn: Duration::ZERO,
    }
}

/// A scratch database on whichever backend `LOREHAVEN_TEST_PG_URL` names.
///
/// Copied from `seen_exclusion.rs` for the reason documented there: the db crate
/// cannot depend on `test_support`, and the PostgreSQL URL's database must be
/// this call's own — reusing the shared `postgres` database makes every later
/// run fail the append-only migration checksum guard.
async fn connect(tag: &str) -> Database {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "lorehaven-adapters-{}-{}-{}",
        tag,
        uuid::Uuid::new_v4(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let url = match std::env::var("LOREHAVEN_TEST_PG_URL") {
        Ok(admin) => {
            let name = format!(
                "lh_adapters_{}_{}",
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

async fn insert_account(db: &Database, id: &str) {
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO accounts (id, status, email, age_state, created_at, updated_at, permission_statement)
                 VALUES (?1, 'active', ?2, 'adult', ?3, ?3, 'minimal')",
            )
            .bind(id)
            .bind(format!("{id}@example.com"))
            .bind("2026-01-01T00:00:00Z")
            .execute(db.sqlite_pool().expect("sqlite pool"))
            .await
            .expect("insert account");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO accounts (id, status, email, age_state, created_at, updated_at, permission_statement)
                 VALUES ($1::uuid, 'active', $2, 'adult', $3, $3, 'minimal')",
            )
            .bind(id)
            .bind(format!("{id}@example.com"))
            .bind("2026-01-01T00:00:00Z")
            .execute(db.postgres_pool().expect("postgres pool"))
            .await
            .expect("insert account");
        }
    }
}

/// Put an account at an exact trust level, so the gate is tested against its
/// boundary rather than against whatever `trust_for` defaults to.
async fn set_trust(db: &Database, account: &str, level: i64) {
    let now = "2026-01-01T00:00:00Z";
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO trust_levels (account, level, computed_at, basis) VALUES (?1, ?2, ?3, 'test')",
            )
            .bind(account)
            .bind(level)
            .bind(now)
            .execute(db.sqlite_pool().expect("sqlite pool"))
            .await
            .expect("set trust");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO trust_levels (account, level, computed_at, basis) VALUES ($1::uuid, $2, $3, 'test')",
            )
            .bind(account)
            .bind(level)
            .bind(now)
            .execute(db.postgres_pool().expect("postgres pool"))
            .await
            .expect("set trust");
        }
    }
}

const CURATOR: &str = "aaaaaaaa-0000-0000-0000-000000000001";
const TRUSTED: &str = "bbbbbbbb-0000-0000-0000-000000000002";
const REVIEWER_1: &str = "cccccccc-0000-0000-0000-000000000003";
const REVIEWER_2: &str = "dddddddd-0000-0000-0000-000000000004";
const REVIEWER_3: &str = "eeeeeeee-0000-0000-0000-000000000005";

async fn seed() -> Database {
    let db = connect("gate").await;
    for id in [CURATOR, TRUSTED, REVIEWER_1, REVIEWER_2, REVIEWER_3] {
        insert_account(&db, id).await;
    }
    db
}

/// The §55.2 gate refuses below TL3, and says which level it found.
#[tokio::test]
async fn submit_below_trust_bar_is_refused() {
    let db = seed().await;
    set_trust(&db, CURATOR, SUBMIT_TRUST_BAR - 1).await;

    let err = submit(&db, CURATOR, "id: low-trust", None)
        .await
        .expect_err("below the bar must be refused");

    match err {
        SubmitError::Refused(SubmitRefusal::BelowTrustBar { level, required }) => {
            assert_eq!(
                level,
                SUBMIT_TRUST_BAR - 1,
                "the refusal names the level found"
            );
            assert_eq!(required, SUBMIT_TRUST_BAR);
        }
        other => panic!("expected a trust refusal, got {other:?}"),
    }

    // And nothing was written.
    assert!(
        list_pending(&db).await.expect("list").is_empty(),
        "a refused submission must not leave a row"
    );
}

/// Exactly at the bar is allowed — the gate is `>=`, not `>`.
///
/// This is the boundary case that a `>` typo would pass, which is why it is its
/// own test rather than folded into the refusal case.
#[tokio::test]
async fn submit_exactly_at_trust_bar_is_allowed() {
    let db = seed().await;
    set_trust(&db, TRUSTED, SUBMIT_TRUST_BAR).await;

    let id = submit(
        &db,
        TRUSTED,
        "id: at-the-bar",
        Some("source_id: at-the-bar"),
    )
    .await
    .expect("exactly at the bar must be allowed");

    let row = lorehaven_db::source_adapters::by_id(&db, &id)
        .await
        .expect("by_id")
        .expect("the submission exists");
    assert_eq!(row.state, "pending", "submission is not deployment");
}

/// The §19.4 threshold: three *distinct* reviewers, not three rows.
#[tokio::test]
async fn approval_needs_three_distinct_reviewers() {
    let db = seed().await;
    set_trust(&db, TRUSTED, SUBMIT_TRUST_BAR).await;
    let sub = submit(&db, TRUSTED, "id: quorum", Some("source_id: quorum"))
        .await
        .expect("submit");

    record_review(&db, &sub, REVIEWER_1, "approve", None)
        .await
        .expect("r1");
    record_review(&db, &sub, REVIEWER_2, "approve", None)
        .await
        .expect("r2");
    assert_eq!(approve_count(&db, &sub).await.expect("count"), 2);
    assert!(
        !has_reached_threshold(&db, &sub).await.expect("threshold"),
        "two approvals must not reach a threshold of {APPROVAL_THRESHOLD}"
    );

    record_review(&db, &sub, REVIEWER_3, "approve", None)
        .await
        .expect("r3");
    assert_eq!(approve_count(&db, &sub).await.expect("count"), 3);
    assert!(has_reached_threshold(&db, &sub).await.expect("threshold"));
}

/// The same reviewer voting twice is still one vote.
///
/// The `ON CONFLICT DO NOTHING` and the UNIQUE constraint are both load-bearing
/// here. Without the constraint the second insert is an error; without the
/// `DO NOTHING` it is a hard failure a caller must handle. Either way the count
/// must remain one.
#[tokio::test]
async fn a_reviewer_voting_twice_counts_once() {
    let db = seed().await;
    set_trust(&db, TRUSTED, SUBMIT_TRUST_BAR).await;
    let sub = submit(&db, TRUSTED, "id: dup", None).await.expect("submit");

    record_review(&db, &sub, REVIEWER_1, "approve", None)
        .await
        .expect("first vote");
    // A second attempt must not be an error and must not add a vote.
    record_review(&db, &sub, REVIEWER_1, "approve", Some("reconsidered"))
        .await
        .expect("second vote is absorbed, not rejected");

    assert_eq!(
        approve_count(&db, &sub).await.expect("count"),
        1,
        "one reviewer is one vote however many times they press the button"
    );
    assert!(
        !has_reached_threshold(&db, &sub).await.expect("threshold"),
        "a single reviewer must never reach the threshold by repeating themselves"
    );
}

/// Abstentions and rejections do not count toward approval.
#[tokio::test]
async fn only_approvals_count_toward_the_threshold() {
    let db = seed().await;
    set_trust(&db, TRUSTED, SUBMIT_TRUST_BAR).await;
    let sub = submit(&db, TRUSTED, "id: mixed", None)
        .await
        .expect("submit");

    record_review(&db, &sub, REVIEWER_1, "approve", None)
        .await
        .expect("r1");
    record_review(&db, &sub, REVIEWER_2, "reject", None)
        .await
        .expect("r2");
    record_review(&db, &sub, REVIEWER_3, "abstain", None)
        .await
        .expect("r3");

    assert_eq!(approve_count(&db, &sub).await.expect("count"), 1);
    assert!(!has_reached_threshold(&db, &sub).await.expect("threshold"));
}

/// A bogus verdict is refused at the store boundary, not by the driver.
#[tokio::test]
async fn an_invalid_verdict_is_refused_with_a_naming_error() {
    let db = seed().await;
    set_trust(&db, TRUSTED, SUBMIT_TRUST_BAR).await;
    let sub = submit(&db, TRUSTED, "id: bogus", None)
        .await
        .expect("submit");

    let err = record_review(&db, &sub, REVIEWER_1, "lgtm", None)
        .await
        .expect_err("'lgtm' is not a verdict");
    let msg = err.to_string();
    assert!(
        msg.contains("approve, reject or abstain") && msg.contains("lgtm"),
        "the error must name both the allowed set and the offending value, got: {msg}"
    );
}

/// Approving below the threshold does not change the row.
#[tokio::test]
async fn approve_below_threshold_does_not_publish() {
    let db = seed().await;
    set_trust(&db, TRUSTED, SUBMIT_TRUST_BAR).await;
    let sub = submit(&db, TRUSTED, "id: premature", Some("source_id: premature"))
        .await
        .expect("submit");
    record_review(&db, &sub, REVIEWER_1, "approve", None)
        .await
        .expect("r1");

    let changed = decide(&db, &sub, "approved", None).await.expect("decide");
    assert!(!changed, "one approval must not publish");

    let row = lorehaven_db::source_adapters::by_id(&db, &sub)
        .await
        .expect("by_id")
        .expect("row");
    assert_eq!(row.state, "pending", "state must be untouched");
    assert!(
        published_source_manifests(&db)
            .await
            .expect("published")
            .is_empty(),
        "an unapproved submission must serve no reader traffic (§55.7)"
    );
}

/// At the threshold it publishes, and only then becomes loadable.
#[tokio::test]
async fn approve_at_threshold_publishes() {
    let db = seed().await;
    set_trust(&db, TRUSTED, SUBMIT_TRUST_BAR).await;
    let sub = submit(&db, TRUSTED, "id: ready", Some("source_id: ready"))
        .await
        .expect("submit");
    for r in [REVIEWER_1, REVIEWER_2, REVIEWER_3] {
        record_review(&db, &sub, r, "approve", None)
            .await
            .expect("review");
    }

    assert!(decide(&db, &sub, "approved", None).await.expect("decide"));

    let published = published_source_manifests(&db).await.expect("published");
    assert_eq!(
        published.len(),
        1,
        "exactly the approved submission's manifest is loadable"
    );
    assert_eq!(published[0].1, "source_id: ready");
}

/// Revoking requires a reason — §19.5.
#[tokio::test]
async fn revoke_requires_a_reason() {
    let db = seed().await;
    set_trust(&db, TRUSTED, SUBMIT_TRUST_BAR).await;
    let sub = submit(&db, TRUSTED, "id: revoke", None)
        .await
        .expect("submit");

    let err = decide(&db, &sub, "revoked", None)
        .await
        .expect_err("a revocation without a reason is refused");
    assert!(err.to_string().contains("§19.5"), "error cites §19.5");

    assert!(
        decide(&db, &sub, "revoked", Some("adapter broke the source"))
            .await
            .expect("with a reason")
    );
}

/// Reviews come back oldest-first, so a reviewer reads the reasoning before
/// voting rather than after.
#[tokio::test]
async fn reviews_are_returned_oldest_first() {
    let db = seed().await;
    set_trust(&db, TRUSTED, SUBMIT_TRUST_BAR).await;
    let sub = submit(&db, TRUSTED, "id: order", None)
        .await
        .expect("submit");
    record_review(&db, &sub, REVIEWER_1, "approve", Some("first"))
        .await
        .expect("r1");
    record_review(&db, &sub, REVIEWER_2, "reject", Some("second"))
        .await
        .expect("r2");

    let reviews = lorehaven_db::source_adapters::reviews_for(&db, &sub)
        .await
        .expect("reviews");
    assert_eq!(reviews.len(), 2);
    assert_eq!(reviews[0].note.as_deref(), Some("first"));
    assert_eq!(reviews[1].note.as_deref(), Some("second"));
}

/// A curator's own submissions are scoped to them.
#[tokio::test]
async fn submissions_are_scoped_to_their_submitter() {
    let db = seed().await;
    set_trust(&db, TRUSTED, SUBMIT_TRUST_BAR).await;
    submit(&db, TRUSTED, "id: mine", None)
        .await
        .expect("submit");

    let mine = submissions_by(&db, TRUSTED).await.expect("mine");
    assert_eq!(mine.len(), 1);

    let theirs = submissions_by(&db, CURATOR).await.expect("theirs");
    assert!(
        theirs.is_empty(),
        "another curator's list must not include this submission"
    );
}
