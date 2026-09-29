//! Acceptance: the decision audit trail, stored and read back (spec §11.14,
//! amendment `calibrated-decision-models.md` §3.5).
//!
//! `crates/decisions/src/reconcile.rs` proves the *rule* — the model may only
//! narrow an acceptance to a hold — as a pure function over two values. This file
//! proves the *store*, which is a separate claim in ways a pure-function test
//! cannot reach:
//!
//! 1. **A NULL posterior has to survive the round trip as NULL.** The claim the
//!    amendment rests on is that a model-less decision is *not* a zero-confidence
//!    decision. A column defaulting to 0.0, or a reader that turns `None` into
//!    `0.0`, reports every deterministic decision as a model saying "no" — and an
//!    operator tuning a threshold from that record tunes against a fiction. A
//!    unit test with an `Option<f64>` in hand cannot tell this apart from a
//!    deliberate 0.0.
//! 2. **The rows that prove the asymmetry have to be storable.** The property
//!    `has_a_model_narrowed_anything` checks is only meaningful if a row where
//!    `deterministic != outcome` can actually be written and read back with both
//!    values intact.
//! 3. **Both dialects.** `created_at` is TEXT on SQLite and `timestamptz` on
//!    PostgreSQL, and `posterior` is REAL and DOUBLE PRECISION. A reader that
//!    forgot the Postgres cast fails on one engine and passes on the other, which
//!    is the worst kind of bug: the engine nobody deploys to is the one that is
//!    wrong.
//! 4. **The bound is a bound.** `limit` comes from a request, and an audit read
//!    with no ceiling is a way to read somebody else's whole instance.

use std::path::PathBuf;

use lorehaven_db::decision_audit::{
    self, AuditEntry, AuditProvider, AuditQuery, NewAuditEntry, MAX_AUDIT_PAGE,
};
use test_support::TestDb;

/// A per-test directory, as every suite in `crates/app/tests/` does.
///
/// Not a shared one. Under PostgreSQL a second `TestDb` for the same tag is a
/// *different* database, so a fixture written through one handle is invisible to
/// the code under test and the test fails for a reason that has nothing to do
/// with it. That is a lesson this repository has already paid for.
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-decision-audit-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

async fn harness(tag: &str) -> TestDb {
    let dir = scratch_dir(tag);
    TestDb::connect_with_dir(tag, &dir).await
}

/// A recorded decision, with everything the store takes.
fn entry(
    subject: &str,
    deterministic: &str,
    posterior: Option<f64>,
    outcome: &str,
    provider: AuditProvider,
) -> NewAuditEntry {
    NewAuditEntry {
        task: "import_quality".to_owned(),
        subject: subject.to_owned(),
        deterministic: deterministic.to_owned(),
        posterior,
        threshold: Some(0.90),
        outcome: outcome.to_owned(),
        provider,
    }
}

async fn all(tdb: &TestDb) -> Vec<AuditEntry> {
    decision_audit::list(
        tdb.db(),
        &AuditQuery {
            limit: MAX_AUDIT_PAGE,
            ..AuditQuery::default()
        },
    )
    .await
    .expect("the audit reads")
}

/// An instance that has never recorded a decision reads as empty, not as broken.
///
/// The endpoint has always returned `{ "items": [] }` and this makes that honest
/// for the right reason: there is a table behind it, and it is empty because
/// nothing has been decided yet.
#[tokio::test]
async fn an_instance_with_no_decisions_reads_as_empty() {
    let tdb = harness("empty").await;
    assert!(
        all(&tdb).await.is_empty(),
        "a fresh instance has decided nothing, and that is a fact about the \
         decisions rather than a missing table"
    );
}

/// A NULL posterior comes back as NULL, and is not zero confidence.
///
/// The load-bearing round-trip test of the whole amendment. Every deterministic
/// decision on an instance that has not opted in has no posterior at all, and
/// there are four different reasons it can be None: the provider is
/// deterministic, the model was unreachable, the answer was refused as
/// nonsense, or there was no text to grade. A store that writes 0.0 for any of
/// them tells an operator that a model confidently said "no", and the operator
/// then moves a threshold in response to a number that was never measured.
#[tokio::test]
async fn a_null_posterior_survives_the_round_trip_as_null_and_not_as_zero() {
    let tdb = harness("null-posterior").await;
    decision_audit::record(
        tdb.db(),
        &entry(
            "work-1",
            "accepted",
            None,
            "accepted",
            AuditProvider::Deterministic,
        ),
    )
    .await
    .expect("the write lands");

    let rows = all(&tdb).await;
    assert_eq!(rows.len(), 1, "one decision was recorded");
    assert_eq!(
        rows[0].posterior, None,
        "no model was consulted, so there is no posterior. Zero would be a \
         measurement, and none was taken."
    );
    assert_eq!(
        rows[0].threshold,
        Some(0.90),
        "the threshold is still recorded even with no model: it is what a \
         future model would have been measured against"
    );
}

/// A real posterior round-trips with its precision, on both dialects.
///
/// A `f64` through a TEXT column on one engine and a DOUBLE PRECISION on the
/// other is where a 0.615 quietly becomes a 0.61, and a threshold at 0.615
/// quietly becomes a threshold nothing reaches.
#[tokio::test]
async fn a_posterior_round_trips_without_losing_precision() {
    let tdb = harness("posterior").await;
    // A value with no short decimal form, which is what makes this a real test:
    // 0.5 would survive being rounded to one place by accident.
    let posterior = 0.615_555_555_555_555_6_f64;
    decision_audit::record(
        tdb.db(),
        &entry(
            "work-1",
            "accepted",
            Some(posterior),
            "held",
            AuditProvider::Calibrated,
        ),
    )
    .await
    .expect("the write lands");

    let rows = all(&tdb).await;
    let stored = rows[0].posterior.expect("a posterior was recorded");
    assert_eq!(
        stored, posterior,
        "a threshold set to a long decimal is a threshold an operator chose, \
         and rounding it makes their choice a different one"
    );
    assert!(rows[0].posterior.expect("read again") < 1.0);
}

/// The row that proves the asymmetry is storable with both answers intact.
///
/// `deterministic` and `outcome` are equal on almost every row. The rows where
/// they differ are the only evidence the model's influence stayed one-directional,
/// so a store that could not hold both would leave the property unprovable after
/// the fact.
#[tokio::test]
async fn the_row_where_the_model_narrowed_keeps_both_answers() {
    let tdb = harness("narrowed").await;
    decision_audit::record(
        tdb.db(),
        &entry(
            "work-1",
            "accepted",
            Some(0.42),
            "held",
            AuditProvider::Calibrated,
        ),
    )
    .await
    .expect("the write lands");

    let rows = all(&tdb).await;
    assert_eq!(rows[0].deterministic, "accepted");
    assert_eq!(rows[0].outcome, "held");
    assert!(
        decision_audit::has_a_model_narrowed_anything(tdb.db())
            .await
            .expect("the property query runs"),
        "this row IS the evidence: the model took an acceptance and left a hold"
    );
}

/// An instance where the model has never done anything says so.
///
/// The companion to the test above, and the one that keeps the property
/// meaningful: a check that returned `true` on a table with no calibrated rows
/// in it would be a check that proves nothing.
#[tokio::test]
async fn an_instance_where_the_model_has_never_narrowed_anything_says_so() {
    let tdb = harness("never-narrowed").await;
    decision_audit::record(
        tdb.db(),
        &entry(
            "work-1",
            "accepted",
            None,
            "accepted",
            AuditProvider::Deterministic,
        ),
    )
    .await
    .expect("the write lands");
    decision_audit::record(
        tdb.db(),
        &entry(
            "work-2",
            "accepted",
            Some(0.99),
            "accepted",
            AuditProvider::Calibrated,
        ),
    )
    .await
    .expect("a model that AGREED is not a model that narrowed");

    assert!(
        !decision_audit::has_a_model_narrowed_anything(tdb.db())
            .await
            .expect("the property query runs"),
        "a calibrated row where the model agreed is not a narrowing, and \
         counting it would make the check read true on a healthy instance"
    );
}

/// Every decision for one subject, and only that subject's.
///
/// This is the read an operator or an author actually makes — "why was this
/// held?" — and it is the query the `idx_decision_audit_subject` index exists for.
#[tokio::test]
async fn the_history_of_one_subject_is_readable_on_its_own() {
    let tdb = harness("per-subject").await;
    decision_audit::record(
        tdb.db(),
        &entry(
            "work-1",
            "accepted",
            None,
            "accepted",
            AuditProvider::Deterministic,
        ),
    )
    .await
    .expect("write 1");
    decision_audit::record(
        tdb.db(),
        &entry("work-2", "held", None, "held", AuditProvider::Deterministic),
    )
    .await
    .expect("write 2");
    decision_audit::record(
        tdb.db(),
        &entry(
            "work-1",
            "accepted",
            Some(0.5),
            "held",
            AuditProvider::Calibrated,
        ),
    )
    .await
    .expect("write 3");

    let history = decision_audit::for_subject(tdb.db(), "work-1", 10)
        .await
        .expect("the history reads");
    assert_eq!(
        history.len(),
        2,
        "work-2's decision is not in work-1's history"
    );
    assert!(history.iter().all(|row| row.subject == "work-1"));
    assert!(
        history.iter().all(|row| row.task == "import_quality"),
        "a subject's history is that subject's history, not every task's"
    );
}

/// Two decisions about one subject are two rows, not one overwritten row.
///
/// The reason `record` has no `ON CONFLICT`: an audit that edits its own history
/// is not an audit. A second decision is a second row, and reading them in order
/// is the history — which is the only thing an author is entitled to when they
/// ask why the answer changed.
#[tokio::test]
async fn a_second_decision_about_one_subject_adds_a_row_rather_than_replacing_one() {
    let tdb = harness("append-only").await;
    decision_audit::record(
        tdb.db(),
        &entry(
            "work-1",
            "accepted",
            Some(0.5),
            "held",
            AuditProvider::Calibrated,
        ),
    )
    .await
    .expect("write 1");
    decision_audit::record(
        tdb.db(),
        &entry(
            "work-1",
            "held",
            Some(0.99),
            "held",
            AuditProvider::Calibrated,
        ),
    )
    .await
    .expect("write 2");

    let history = decision_audit::for_subject(tdb.db(), "work-1", 10)
        .await
        .expect("the history reads");
    assert_eq!(
        history.len(),
        2,
        "an upsert here would let a second decision erase the first, and the \
         record of WHY the answer changed would be the thing that changed"
    );
    // Newest first, and the second write is the one that is newest.
    assert_eq!(history[0].deterministic, "held");
    assert_eq!(history[1].deterministic, "accepted");
}

/// The page is bounded, and a request for everything gets the bound instead.
///
/// An audit read with no ceiling is a way to read somebody else's whole
/// instance, and `limit` is a value from a request. A caller asking for zero is
/// pinned to one row rather than shown nothing, because "give me the newest
/// decisions" answered with an empty list is a bug report rather than a feature.
#[tokio::test]
async fn the_page_is_bounded_and_a_zero_request_still_returns_a_row() {
    let tdb = harness("bounded").await;
    for i in 0..3 {
        decision_audit::record(
            tdb.db(),
            &entry(
                &format!("work-{i}"),
                "accepted",
                None,
                "accepted",
                AuditProvider::Deterministic,
            ),
        )
        .await
        .expect("a write lands");
    }

    // More rows than the bound, so removing the bound is observable. The
    // earlier version of this test wrote three rows and asked for i64::MAX,
    // which passes whether or not the clamp exists — a bound that is not
    // exceeded is not a bound that is tested. MAX_AUDIT_PAGE + 2 rows is the
    // smallest setup that can tell a bounded read from an unbounded one.
    for i in 0..(MAX_AUDIT_PAGE + 2) {
        decision_audit::record(
            tdb.db(),
            &entry(
                &format!("bulk-{i}"),
                "accepted",
                None,
                "accepted",
                AuditProvider::Deterministic,
            ),
        )
        .await
        .expect("a write lands");
    }

    let over = decision_audit::list(
        tdb.db(),
        &AuditQuery {
            limit: i64::MAX,
            ..AuditQuery::default()
        },
    )
    .await
    .expect("the read runs");
    assert_eq!(
        over.len(),
        MAX_AUDIT_PAGE as usize,
        "a request for i64::MAX rows gets the bound, not the table: {} rows \
         are in it",
        MAX_AUDIT_PAGE + 3
    );

    for asked in [0_i64, -5] {
        let rows = decision_audit::list(
            tdb.db(),
            &AuditQuery {
                limit: asked,
                ..AuditQuery::default()
            },
        )
        .await
        .expect("the read runs");
        assert_eq!(
            rows.len(),
            1,
            "a limit of {asked} is nonsense, and the answer to nonsense is one \
             row rather than none"
        );
    }
}

/// A filter narrows, and a filter that matches nothing returns nothing.
///
/// The second half matters as much as the first: an operator asking "has the
/// model ever touched positivity?" must be able to be told **no**, and a filter
/// that ignored its own argument would answer yes.
#[tokio::test]
async fn a_filter_narrows_and_a_filter_matching_nothing_returns_nothing() {
    let tdb = harness("filtered").await;
    let mut positivity = entry(
        "comment-1",
        "held",
        None,
        "held",
        AuditProvider::Deterministic,
    );
    positivity.task = "positivity".to_owned();
    decision_audit::record(tdb.db(), &positivity)
        .await
        .expect("write 1");
    decision_audit::record(
        tdb.db(),
        &entry(
            "work-1",
            "accepted",
            None,
            "accepted",
            AuditProvider::Deterministic,
        ),
    )
    .await
    .expect("write 2");

    let only_positivity = decision_audit::list(
        tdb.db(),
        &AuditQuery {
            task: Some("positivity".to_owned()),
            limit: 10,
            ..AuditQuery::default()
        },
    )
    .await
    .expect("the read runs");
    assert_eq!(only_positivity.len(), 1);
    assert_eq!(only_positivity[0].task, "positivity");

    let nothing = decision_audit::list(
        tdb.db(),
        &AuditQuery {
            task: Some("mood_class".to_owned()),
            limit: 10,
            ..AuditQuery::default()
        },
    )
    .await
    .expect("the read runs");
    assert!(
        nothing.is_empty(),
        "an operator must be able to be told the answer is no"
    );

    let only_calibrated = decision_audit::list(
        tdb.db(),
        &AuditQuery {
            provider: Some(AuditProvider::Calibrated),
            limit: 10,
            ..AuditQuery::default()
        },
    )
    .await
    .expect("the read runs");
    assert!(
        only_calibrated.is_empty(),
        "no row was calibrated, and the filter says so rather than returning \
         the deterministic rows"
    );
}

/// The newest decision comes first.
///
/// An audit that comes back oldest-first is a log, and a log answers "what
/// happened" only if you read all of it. Every reader of this table is asking
/// "what just happened", so the order is part of the contract rather than a
/// presentation detail.
#[tokio::test]
async fn the_newest_decision_comes_first() {
    let tdb = harness("newest-first").await;
    for (i, label) in ["first", "second", "third"].iter().enumerate() {
        decision_audit::record(
            tdb.db(),
            &entry(
                label,
                "accepted",
                None,
                "accepted",
                AuditProvider::Deterministic,
            ),
        )
        .await
        .expect("a write lands");
        // Distinct timestamps, because the ordering claim is about `created_at`
        // and three rows written inside one millisecond would test the id
        // tiebreak instead of the thing being claimed.
        tokio::time::sleep(std::time::Duration::from_millis(10 * (i as u64 + 1))).await;
    }

    let rows = all(&tdb).await;
    let subjects: Vec<&str> = rows.iter().map(|r| r.subject.as_str()).collect();
    assert_eq!(
        subjects,
        vec!["third", "second", "first"],
        "an operator reads this newest-first; oldest-first makes them read the \
         whole table to find out what just happened"
    );
}

/// A provider this build does not know reads as `deterministic`.
///
/// The direction of that guess is the point. A row written by a future version
/// with a provider this build cannot parse is a row this build must not claim
/// was model-answered — and it must not fail the whole page either, or a single
/// new row would take an operator's entire audit history offline.
#[tokio::test]
async fn an_unknown_provider_name_reads_as_deterministic_and_does_not_break_the_page() {
    let tdb = harness("unknown-provider").await;
    decision_audit::record(
        tdb.db(),
        &entry(
            "work-1",
            "accepted",
            None,
            "accepted",
            AuditProvider::Deterministic,
        ),
    )
    .await
    .expect("write 1");

    // Written raw, the way a future version's row would arrive. The two
    // dialects are branched rather than matched into one `.execute()` because
    // their pool types are different, so a single `match` cannot produce one
    // executor for both.
    // Each dialect gets its OWN placeholder style, the way the store does via
    // `db.sql`. Handing PostgreSQL the SQLite `?` form is a syntax error at or
    // near "," -- and it is this test's bug, not the store's, which is exactly
    // why a raw-SQL test is worth having: it does not inherit the store's
    // dialect handling for free.
    const INSERT_SQLITE: &str = "INSERT INTO decision_audit
            (id, task, subject, deterministic, posterior, threshold, outcome,
             provider, created_at, updated_at, version)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1)";
    const INSERT_POSTGRES: &str = "INSERT INTO decision_audit
            (id, task, subject, deterministic, posterior, threshold, outcome,
             provider, created_at, updated_at, version)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 1)";
    // Each arm discards its own result and returns `()`, because the two
    // dialects' query results are different types and a `match` cannot hand back
    // one of them. The `.expect` is inside each arm for the same reason.
    match tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(INSERT_SQLITE)
                .bind("future-row")
                .bind("import_quality")
                .bind("work-9")
                .bind("accepted")
                .bind(None::<f64>)
                .bind(Some(0.90_f64))
                .bind("accepted")
                .bind("some_future_provider")
                .bind("2999-01-01T00:00:00Z")
                .bind("2999-01-01T00:00:00Z")
                .execute(tdb.db().sqlite_pool().expect("sqlite handle"))
                .await
                .expect("a raw row from a future version lands");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(INSERT_POSTGRES)
                .bind("future-row")
                .bind("import_quality")
                .bind("work-9")
                .bind("accepted")
                .bind(None::<f64>)
                .bind(Some(0.90_f64))
                .bind("accepted")
                .bind("some_future_provider")
                .bind("2999-01-01T00:00:00Z")
                .bind("2999-01-01T00:00:00Z")
                .execute(tdb.db().postgres_pool().expect("postgres handle"))
                .await
                .expect("a raw row from a future version lands");
        }
    };

    let rows = all(&tdb).await;
    assert_eq!(
        rows.len(),
        2,
        "one unknown row must not take the page offline"
    );
    let future = rows
        .iter()
        .find(|r| r.id == "future-row")
        .expect("the future row is readable");
    assert_eq!(
        future.provider,
        AuditProvider::Deterministic,
        "a provider this build cannot parse must not be reported as one it can"
    );
}
