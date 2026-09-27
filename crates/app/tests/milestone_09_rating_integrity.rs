//! M09/M37 — Rating integrity (`crates/db/src/rating_integrity.rs`, spec §9.4,
//! §37 signal integrity), the anti-brigading surface behind `/api/v1/rating`.
//!
//! Seven `pub async fn`. **The module already has six tests — and every one of
//! them builds a SQLite-only `Database` inline**, ignoring `TestDb` and
//! `LOREHAVEN_TEST_PG_URL` entirely. So the module reads as well covered while
//! its entire PostgreSQL path has never executed. That is exactly the shape of
//! bug this suite exists to find, and the docstring below records what it found.
//!
//! The schema is the most hostile in the data layer for this purpose:
//! `rating_anomaly_events` is `UUID` / `TIMESTAMPTZ` / `JSONB` / `INTEGER` all
//! at once, and `works.contested` is a boolean in the row while the SQLite
//! migration stores it as an integer.
//!
//! Two invariants are §9.4's and worth stating, because both are easy to break
//! with a plausible-looking change:
//!
//! - **Trust weights, credits do not.** `get_trust_weighted_rating_summary`
//!   multiplies each star value by the rater's `trust_levels.level`, defaulting
//!   to 1. It reads no credit balance.
//! - **The publication threshold is three.** Fewer than three public,
//!   non-deleted ratings returns `None`, not a mean over a sample too small to
//!   publish. The constant lives in the module so the rule and the query cannot
//!   drift apart.

use std::path::PathBuf;

use lorehaven_db::identity::{create_account, create_pseud, AccountStatus};
use lorehaven_db::rating_integrity::{
    clear_rating_anomaly_event, clear_work_contested, get_trust_weighted_rating_summary,
    get_work_anomaly_events, insert_rating_anomaly_event, is_work_contested, set_work_contested,
    RatingAnomalyEvent,
};
use lorehaven_domain::ids::{AccountId, PseudId, WorkId};
use lorehaven_domain::policy::AgeState;
use test_support::TestDb;
use uuid::Uuid;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-rating-integrity-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

struct Harness {
    tdb: TestDb,
    #[allow(dead_code)]
    dir: PathBuf,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        Self { tdb, dir }
    }

    fn db(&self) -> &lorehaven_db::Database {
        self.tdb.db()
    }

    fn is_pg(&self) -> bool {
        self.db().backend() == lorehaven_db::Backend::Postgres
    }

    /// An account that exists in the database, for the `cleared_by` foreign key.
    ///
    /// `AccountId::new()` mints a valid id for a row that is not there, and
    /// PostgreSQL enforces the reference while SQLite does not — the module's
    /// own tests get away with `AccountId::new()` precisely because they only
    /// ever run on SQLite.
    async fn real_account(&self) -> AccountId {
        let account = create_account(
            self.db(),
            &format!("clearer-{}@example.test", Uuid::new_v4()),
            AgeState::DeclaredAdult,
            AccountStatus::Active,
        )
        .await
        .expect("create_account");
        account
    }

    async fn exec(&self, query: &str) {
        let q = self.tdb.sql(query);
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => {
                sqlx::query(&q)
                    .execute(self.db().sqlite_pool().expect("sqlite"))
                    .await
                    .expect("exec");
            }
            lorehaven_db::Backend::Postgres => {
                sqlx::query(&q)
                    .execute(self.db().postgres_pool().expect("pg"))
                    .await
                    .expect("exec");
            }
        }
    }

    async fn count(&self, table: &str) -> i64 {
        let q = self.tdb.sql(&format!("SELECT COUNT(*) FROM {table}"));
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query_scalar::<_, i64>(&q)
                .fetch_one(self.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("count"),
            lorehaven_db::Backend::Postgres => sqlx::query_scalar::<_, i64>(&q)
                .fetch_one(self.db().postgres_pool().expect("pg"))
                .await
                .expect("count"),
        }
    }

    /// Read one column of one row as text, with the row picked out by a literal
    /// in the query.
    ///
    /// `TestDb::fetch_text` binds `?` to its `value` argument and collapses NULL
    /// into "no row", so it cannot tell an empty string from a NULL — which is
    /// exactly the distinction `contested_reason = ''` versus NULL turns on.
    /// Read one column as text, casting for PostgreSQL where the caller wrote a
    /// `::text` suffix. `::text` is not SQLite syntax, so a shared query cannot
    /// carry it — hence the rewrite rather than a per-callsite `match`.
    async fn text_of(&self, query: &str) -> Option<String> {
        // `::text` is PostgreSQL syntax and SQLite rejects the `:` outright, so
        // the cast is stripped there. Callers can then write one query for both
        // backends and name the cast they mean.
        let owned;
        let query = if self.is_pg() {
            query
        } else {
            owned = query.replace("::text", "");
            owned.as_str()
        };
        // `query_scalar::<_, String>` errors on a NULL column rather than
        // yielding None, and several of these assertions are about a column
        // being NULL after a clear — so the Option is the point.
        let row: Option<Option<String>> = match self.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query_scalar::<_, Option<String>>(query)
                .fetch_optional(self.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("scalar"),
            lorehaven_db::Backend::Postgres => sqlx::query_scalar::<_, Option<String>>(query)
                .fetch_optional(self.db().postgres_pool().expect("pg"))
                .await
                .expect("scalar"),
        };
        row.flatten()
    }

    /// A work to rate. Uses the same `create_work` the module's own tests use, so
    /// the schema is whatever production creates rather than a hand-rolled row.
    async fn work(&self) -> (WorkId, AccountId, PseudId) {
        let account = create_account(
            self.db(),
            &format!("ri-{}@example.test", Uuid::new_v4()),
            AgeState::DeclaredAdult,
            AccountStatus::Active,
        )
        .await
        .expect("create_account");
        let pseud = create_pseud(
            self.db(),
            account,
            &format!("ri-{}", Uuid::new_v4()),
            "Rater",
        )
        .await
        .expect("create_pseud");
        let work = lorehaven_db::content::create_work(self.db(), pseud, "Rated Work", None)
            .await
            .expect("create_work");
        (work.id, account, pseud)
    }

    /// A rater who gives `stars` public stars, optionally with a trust level.
    ///
    /// A fresh account and pseud per call, because `rating` has a unique index
    /// on `(account_id, work_id)` and two ratings from one account would be a
    /// re-rating, not a second rater.
    async fn rate(&self, work: &WorkId, stars: i64, trust: Option<i64>) -> AccountId {
        let account = create_account(
            self.db(),
            &format!("rater-{}@example.test", Uuid::new_v4()),
            AgeState::DeclaredAdult,
            AccountStatus::Active,
        )
        .await
        .expect("create_account");
        let pseud = create_pseud(
            self.db(),
            account,
            &format!("rater-{}", Uuid::new_v4()),
            "Rater",
        )
        .await
        .expect("create_pseud");

        if let Some(level) = trust {
            self.exec(&format!(
                "INSERT INTO trust_levels (account, level, computed_at, basis) \
                 VALUES ('{account}', {level}, '2026-01-01T00:00:00Z', 'test')"
            ))
            .await;
        }

        let now = "2026-01-01T00:00:00Z";
        self.exec(&format!(
            "INSERT INTO rating (id, account_id, pseud_id, work_id, stars, is_public, \
             created_at, updated_at, version) \
             VALUES ('{id}', '{account}', '{pseud}', '{work}', {stars}, {pub_}, '{now}', '{now}', 1)",
            pub_ = if self.is_pg() { "TRUE" } else { "1" },
            id = Uuid::new_v4()
        ))
        .await;
        account
    }

    /// An anomaly event with sensible defaults.
    fn event(&self, work: &WorkId, kind: &str) -> RatingAnomalyEvent {
        RatingAnomalyEvent {
            id: Uuid::new_v4().to_string(),
            work_id: work.to_string(),
            cohort_id: None,
            kind: kind.to_string(),
            severity: 3,
            detail: r#"{"ratings_in_window":10,"threshold":5}"#.to_string(),
            detected_at: "2026-01-01T00:00:00Z".to_string(),
            cleared_at: None,
            cleared_by: None,
        }
    }
}

// ---------------------------------------------------------------------------
// get_trust_weighted_rating_summary
// ---------------------------------------------------------------------------

/// Below the publication threshold there is no summary at all — §9.4's rule is
/// about not publishing a mean over too small a sample, and returning a mean
/// would violate it.
#[tokio::test]
async fn fewer_than_three_ratings_publishes_nothing() {
    let h = Harness::new("ri-threshold-0").await;
    let (work, _, _) = h.work().await;
    assert!(
        get_trust_weighted_rating_summary(h.db(), &work)
            .await
            .expect("summary")
            .is_none(),
        "zero ratings is below the threshold"
    );
}

#[tokio::test]
async fn one_rating_publishes_nothing() {
    let h = Harness::new("ri-threshold-1").await;
    let (work, _, _) = h.work().await;
    h.rate(&work, 5, None).await;
    assert!(get_trust_weighted_rating_summary(h.db(), &work)
        .await
        .expect("summary")
        .is_none());
}

#[tokio::test]
async fn two_ratings_publishes_nothing() {
    let h = Harness::new("ri-threshold-2").await;
    let (work, _, _) = h.work().await;
    h.rate(&work, 5, None).await;
    h.rate(&work, 1, None).await;
    assert!(
        get_trust_weighted_rating_summary(h.db(), &work)
            .await
            .expect("summary")
            .is_none(),
        "two is still short of three"
    );
}

/// Exactly three publishes.
#[tokio::test]
async fn three_ratings_publishes_a_summary() {
    let h = Harness::new("ri-threshold-3").await;
    let (work, _, _) = h.work().await;
    for _ in 0..3 {
        h.rate(&work, 4, None).await;
    }
    let s = get_trust_weighted_rating_summary(h.db(), &work)
        .await
        .expect("summary")
        .expect("three is enough");
    assert_eq!(s.count, 3);
    assert_eq!(s.mean_permille, 4_000, "4.0 stars");
}

/// A private rating does not count toward the threshold — the whole point of
/// `is_public` is that a private star is not a published signal.
#[tokio::test]
async fn a_private_rating_does_not_count() {
    let h = Harness::new("ri-private").await;
    let (work, _, _) = h.work().await;
    for _ in 0..3 {
        h.rate(&work, 5, None).await;
    }
    // A fourth rating, not public.
    let account = create_account(
        h.db(),
        &format!("private-{}@example.test", Uuid::new_v4()),
        AgeState::DeclaredAdult,
        AccountStatus::Active,
    )
    .await
    .expect("account");
    let pseud = create_pseud(h.db(), account, &format!("p-{}", Uuid::new_v4()), "Quiet")
        .await
        .expect("pseud");
    h.exec(&format!(
        "INSERT INTO rating (id, account_id, pseud_id, work_id, stars, is_public, \
         created_at, updated_at, version) \
         VALUES ('{id}', '{account}', '{pseud}', '{work}', 1, {priv_}, '{now}', '{now}', 1)",
        priv_ = if h.is_pg() { "FALSE" } else { "0" },
        id = Uuid::new_v4(),
        now = "2026-01-01T00:00:00Z"
    ))
    .await;

    let s = get_trust_weighted_rating_summary(h.db(), &work)
        .await
        .expect("summary")
        .expect("still three public");
    assert_eq!(s.count, 3, "the private rating is not counted");
    assert_eq!(s.mean_permille, 5_000, "and does not drag the mean down");
}

/// A soft-deleted rating does not count either.
#[tokio::test]
async fn a_deleted_rating_does_not_count() {
    let h = Harness::new("ri-deleted").await;
    let (work, _, _) = h.work().await;
    let last = h.rate(&work, 1, None).await;
    h.rate(&work, 5, None).await;
    h.rate(&work, 5, None).await;
    h.exec(&format!(
        "UPDATE rating SET deleted_at = '2026-01-01T00:00:00Z' WHERE account_id = '{last}'"
    ))
    .await;

    // Now only two live ratings, so the threshold is not met.
    assert!(
        get_trust_weighted_rating_summary(h.db(), &work)
            .await
            .expect("summary")
            .is_none(),
        "a deleted rating is not a public rating"
    );
}

/// A rating on a *different* work does not count toward this one.
#[tokio::test]
async fn another_works_ratings_do_not_count() {
    let h = Harness::new("ri-otherwork").await;
    let (work, _, _) = h.work().await;
    let (other, _, _) = h.work().await;
    h.rate(&other, 1, None).await;
    h.rate(&other, 1, None).await;
    h.rate(&other, 1, None).await;

    assert!(
        get_trust_weighted_rating_summary(h.db(), &work)
            .await
            .expect("summary")
            .is_none(),
        "the other work's ratings are not this work's"
    );
}

/// A work with no ratings at all has no summary.
#[tokio::test]
async fn an_unrated_work_has_no_summary() {
    let h = Harness::new("ri-unrated").await;
    let (work, _, _) = h.work().await;
    assert!(get_trust_weighted_rating_summary(h.db(), &work)
        .await
        .expect("summary")
        .is_none());
}

/// An unknown work id has no summary rather than erroring.
#[tokio::test]
async fn an_unknown_work_has_no_summary() {
    let h = Harness::new("ri-unknown-work").await;
    assert!(get_trust_weighted_rating_summary(h.db(), &WorkId::new())
        .await
        .expect("summary")
        .is_none());
}

/// Equal ratings give the exact mean, in permille.
#[tokio::test]
async fn equal_ratings_give_the_exact_mean() {
    let h = Harness::new("ri-equal").await;
    let (work, _, _) = h.work().await;
    for _ in 0..3 {
        h.rate(&work, 3, None).await;
    }
    let s = get_trust_weighted_rating_summary(h.db(), &work)
        .await
        .expect("summary")
        .expect("summary");
    assert_eq!(s.mean_permille, 3_000);
}

/// **Trust weighting, not credits.** A high-trust rater counts for more, so the
/// aggregate moves toward their stars: 5@trust5 + 1@trust1 + 1@trust1 gives
/// (25+1+1)/7 = 3.857, where the unweighted mean would be 2.333.
#[tokio::test]
async fn trust_weighting_shifts_the_aggregate() {
    let h = Harness::new("ri-trust").await;
    let (work, _, _) = h.work().await;
    h.rate(&work, 5, Some(5)).await;
    h.rate(&work, 1, Some(1)).await;
    h.rate(&work, 1, Some(1)).await;

    let s = get_trust_weighted_rating_summary(h.db(), &work)
        .await
        .expect("summary")
        .expect("summary");
    assert_eq!(s.count, 3, "the count is ratings, not weight");
    assert_eq!(
        s.mean_permille, 3_857,
        "weighted up toward the trusted rater"
    );
}

/// The uniform-trust case is the arithmetic mean, whatever the level — trust is
/// a *relative* weight, so a bloc of trust-3 raters does not outvote itself.
#[tokio::test]
async fn uniform_trust_reduces_to_the_plain_mean() {
    let h = Harness::new("ri-uniform-trust").await;
    let (work, _, _) = h.work().await;
    for _ in 0..3 {
        h.rate(&work, 5, Some(3)).await;
    }
    let s = get_trust_weighted_rating_summary(h.db(), &work)
        .await
        .expect("summary")
        .expect("summary");
    assert_eq!(s.mean_permille, 5_000);
}

/// **A missing trust level defaults to weight 1**, so a brand-new rater is
/// counted rather than ignored — the `LEFT JOIN` plus `COALESCE` pair.
#[tokio::test]
async fn a_rater_with_no_trust_level_counts_as_weight_one() {
    let h = Harness::new("ri-default-weight").await;
    let (work, _, _) = h.work().await;
    h.rate(&work, 5, Some(10)).await; // heavy
    h.rate(&work, 1, None).await; // unweighted
    h.rate(&work, 1, Some(10)).await; // heavy

    // (5*10 + 1*1 + 1*10) / (10+1+10) = 61/21 = 2.904
    let s = get_trust_weighted_rating_summary(h.db(), &work)
        .await
        .expect("summary")
        .expect("summary");
    assert_eq!(s.mean_permille, 2_904);
}

/// Trust levels of zero weight out the rating entirely while it still counts
/// toward the threshold — `COALESCE(tl.level, 1)` only defaults a *missing*
/// level, so an explicit 0 is honoured.
#[tokio::test]
async fn a_zero_trust_level_zeroes_that_raters_weight() {
    let h = Harness::new("ri-zero-trust").await;
    let (work, _, _) = h.work().await;
    h.rate(&work, 1, Some(0)).await;
    h.rate(&work, 5, Some(1)).await;
    h.rate(&work, 5, Some(1)).await;

    // (0 + 5 + 5) / (0 + 1 + 1) = 5.0
    let s = get_trust_weighted_rating_summary(h.db(), &work)
        .await
        .expect("summary")
        .expect("summary");
    assert_eq!(s.count, 3, "it still counts as a rating");
    assert_eq!(s.mean_permille, 5_000);
}

/// The permille mean is truncated, not rounded, so it never overstates.
#[tokio::test]
async fn the_permille_mean_truncates_rather_than_rounds() {
    let h = Harness::new("ri-truncate").await;
    let (work, _, _) = h.work().await;
    // (4*1 + 4*1 + 5*1) / 3 = 4.3333 -> 4333
    h.rate(&work, 4, None).await;
    h.rate(&work, 4, None).await;
    h.rate(&work, 5, None).await;

    let s = get_trust_weighted_rating_summary(h.db(), &work)
        .await
        .expect("summary")
        .expect("summary");
    assert_eq!(s.mean_permille, 4_333, "truncated, not 4334");
}

/// More than three ratings still aggregates, and the threshold is a floor not a
/// cap.
#[tokio::test]
async fn ratings_beyond_the_threshold_still_aggregate() {
    let h = Harness::new("ri-many").await;
    let (work, _, _) = h.work().await;
    for _ in 0..5 {
        h.rate(&work, 4, None).await;
    }
    let s = get_trust_weighted_rating_summary(h.db(), &work)
        .await
        .expect("summary")
        .expect("summary");
    assert_eq!(s.count, 5);
    assert_eq!(s.mean_permille, 4_000);
}

// ---------------------------------------------------------------------------
// insert_rating_anomaly_event
// ---------------------------------------------------------------------------

/// An event round-trips through insert and read.
///
/// **This is the test that found the PostgreSQL breakage.** The PostgreSQL arm
/// of `get_work_anomaly_events` reads `id` as `String` from a `UUID` column,
/// `detail` as `String` from a `JSONB` column and `detected_at` as `String`
/// from a `TIMESTAMPTZ` column, casting only `severity`. Under the previous
/// SQLite-only test suite that arm had never run.
#[tokio::test]
async fn an_event_round_trips() {
    let h = Harness::new("ri-event-roundtrip").await;
    let (work, _, _) = h.work().await;
    let event = h.event(&work, "burst");
    insert_rating_anomaly_event(h.db(), &event)
        .await
        .expect("insert_rating_anomaly_event");

    let events = get_work_anomaly_events(h.db(), &work)
        .await
        .expect("get_work_anomaly_events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].id, event.id);
    assert_eq!(events[0].work_id, work.to_string());
    assert_eq!(events[0].kind, "burst");
    assert_eq!(events[0].severity, 3);
    // PostgreSQL normalizes JSONB on store — keys are sorted and whitespace is
    // removed — so the bytes come back reordered. SQLite keeps the literal
    // text. What must hold on both is that the document survives, so compare the
    // parsed value rather than the serialization.
    let stored: serde_json::Value =
        serde_json::from_str(&events[0].detail).expect("detail is valid JSON");
    assert_eq!(
        stored,
        serde_json::from_str::<serde_json::Value>(&event.detail).expect("input JSON"),
        "the detail document round-trips, whatever its key order"
    );
    assert_eq!(events[0].detected_at, event.detected_at);
    assert_eq!(events[0].cleared_at, None);
    assert_eq!(events[0].cleared_by, None);
}

/// All three `kind` values the CHECK constraint allows are accepted.
#[tokio::test]
async fn every_allowed_kind_is_accepted() {
    let h = Harness::new("ri-kinds").await;
    let (work, _, _) = h.work().await;
    for kind in ["burst", "cohort_outlier", "profile_outlier"] {
        let e = h.event(&work, kind);
        insert_rating_anomaly_event(h.db(), &e)
            .await
            .unwrap_or_else(|err| panic!("{kind} should be allowed: {err}"));
    }
    assert_eq!(
        get_work_anomaly_events(h.db(), &work)
            .await
            .expect("read")
            .len(),
        3
    );
}

/// A `kind` outside the CHECK constraint is refused by the schema.
#[tokio::test]
async fn a_kind_outside_the_check_constraint_is_refused() {
    let h = Harness::new("ri-badkind").await;
    let (work, _, _) = h.work().await;
    let mut e = h.event(&work, "burst");
    e.kind = "not_a_kind".to_string();
    assert!(
        insert_rating_anomaly_event(h.db(), &e).await.is_err(),
        "the CHECK constraint holds on both backends"
    );
}

/// A cohort id round-trips as a UUID string when present.
#[tokio::test]
async fn a_cohort_id_round_trips() {
    let h = Harness::new("ri-cohort").await;
    let (work, _, _) = h.work().await;
    let cohort = Uuid::new_v4().to_string();
    let mut e = h.event(&work, "cohort_outlier");
    e.cohort_id = Some(cohort.clone());
    insert_rating_anomaly_event(h.db(), &e)
        .await
        .expect("insert");

    let events = get_work_anomaly_events(h.db(), &work).await.expect("read");
    assert_eq!(events[0].cohort_id.as_deref(), Some(cohort.as_str()));
}

/// Severity is a plain integer and round-trips.
#[tokio::test]
async fn severity_round_trips() {
    let h = Harness::new("ri-severity").await;
    let (work, _, _) = h.work().await;
    for sev in [1, 5, 100] {
        let mut e = h.event(&work, "burst");
        e.severity = sev;
        insert_rating_anomaly_event(h.db(), &e)
            .await
            .expect("insert");
    }
    let events = get_work_anomaly_events(h.db(), &work).await.expect("read");
    let mut severities: Vec<i64> = events.iter().map(|e| e.severity).collect();
    severities.sort_unstable();
    assert_eq!(severities, vec![1, 5, 100]);
}

/// A malformed `work_id` is refused on both backends, but by different rules.
///
/// PostgreSQL parses it (`Uuid::parse_str(...).context("parse work_id")`) and
/// fails on the format before the statement runs. SQLite stores the column as
/// TEXT and does no such parse, so the same value is rejected a moment later by
/// the foreign key to `works(id)`, which `not-a-uuid` also cannot satisfy.
///
/// Both arms end in an error, so a caller sees one behaviour — but for two
/// different reasons, and only the PG one is a validation of the id itself.
#[tokio::test]
async fn a_malformed_work_id_is_refused_on_both_backends() {
    let h = Harness::new("ri-baduuid").await;
    let mut e = h.event(&WorkId::new(), "burst");
    e.work_id = "not-a-uuid".to_string();
    assert!(
        insert_rating_anomaly_event(h.db(), &e).await.is_err(),
        "PG by the uuid parse, SQLite by the foreign key"
    );
    assert_eq!(
        h.count("rating_anomaly_events").await,
        0,
        "and no row is written either way"
    );
}

/// An event for a work that does not exist is refused by the foreign key.
#[tokio::test]
async fn an_event_for_an_unknown_work_is_refused() {
    let h = Harness::new("ri-fk").await;
    let e = h.event(&WorkId::new(), "burst");
    assert!(
        insert_rating_anomaly_event(h.db(), &e).await.is_err(),
        "work_id references works(id)"
    );
}

/// An event can be inserted already cleared, which is how a re-ingested signal
/// that an operator already triaged arrives.
#[tokio::test]
async fn an_event_can_be_inserted_already_cleared() {
    let h = Harness::new("ri-precleared").await;
    let (work, _, _) = h.work().await;
    let mut e = h.event(&work, "burst");
    e.cleared_at = Some("2026-01-02T00:00:00Z".to_string());
    insert_rating_anomaly_event(h.db(), &e)
        .await
        .expect("insert");

    assert!(
        get_work_anomaly_events(h.db(), &work)
            .await
            .expect("read")
            .is_empty(),
        "a cleared event is not an outstanding one"
    );
}

// ---------------------------------------------------------------------------
// clear_rating_anomaly_event
// ---------------------------------------------------------------------------

/// Clearing stamps the time and the clearer, and drops it from the outstanding
/// list.
#[tokio::test]
async fn clearing_an_event_removes_it_from_the_outstanding_list() {
    let h = Harness::new("ri-clear").await;
    let (work, _, _) = h.work().await;
    let e = h.event(&work, "burst");
    insert_rating_anomaly_event(h.db(), &e)
        .await
        .expect("insert");

    let clearer = h.real_account().await;
    clear_rating_anomaly_event(h.db(), &e.id, &clearer)
        .await
        .expect("clear");
    assert!(get_work_anomaly_events(h.db(), &work)
        .await
        .expect("read")
        .is_empty());
}

/// A cleared event is not deleted — the row survives with its audit fields, so
/// a signal that was triaged stays on the record.
#[tokio::test]
async fn a_cleared_event_is_kept_not_deleted() {
    let h = Harness::new("ri-clear-keeps").await;
    let (work, _, _) = h.work().await;
    let e = h.event(&work, "burst");
    insert_rating_anomaly_event(h.db(), &e)
        .await
        .expect("insert");
    let clearer = h.real_account().await;
    clear_rating_anomaly_event(h.db(), &e.id, &clearer)
        .await
        .expect("clear");

    assert_eq!(
        h.count("rating_anomaly_events").await,
        1,
        "the row survives the clear"
    );
    assert_eq!(
        h.text_of(&format!(
            "SELECT cleared_by::text FROM rating_anomaly_events WHERE id = '{}'",
            e.id
        ))
        .await
        .as_deref(),
        Some(clearer.to_string().as_str()),
        "and it records who cleared it"
    );
    assert!(
        h.text_of(&format!(
            "SELECT cleared_at::text FROM rating_anomaly_events WHERE id = '{}'",
            e.id
        ))
        .await
        .is_some(),
        "and when"
    );
}

/// Clearing an event that does not exist is a no-op, not an error.
#[tokio::test]
async fn clearing_an_unknown_event_is_a_no_op() {
    let h = Harness::new("ri-clear-unknown").await;
    let clearer = h.real_account().await;
    clear_rating_anomaly_event(h.db(), &Uuid::new_v4().to_string(), &clearer)
        .await
        .expect("a missing event is not an error to clear");
}

/// Clearing a second time **overwrites** the first clear's audit fields.
///
/// The `UPDATE` has no `WHERE cleared_at IS NULL`, so the second triage
/// replaces the first triager and time. For a moderation log that is arguably
/// the wrong shape — the first triage is the interesting one — so it is pinned
/// here rather than left to be discovered.
#[tokio::test]
async fn clearing_twice_overwrites_the_first_clear() {
    let h = Harness::new("ri-clear-twice").await;
    let (work, _, _) = h.work().await;
    let e = h.event(&work, "burst");
    insert_rating_anomaly_event(h.db(), &e)
        .await
        .expect("insert");

    let first = h.real_account().await;
    let second = h.real_account().await;
    let q = format!(
        "SELECT cleared_by::text FROM rating_anomaly_events WHERE id = '{}'",
        e.id
    );

    clear_rating_anomaly_event(h.db(), &e.id, &first)
        .await
        .expect("first");
    assert_eq!(
        h.text_of(&q).await.as_deref(),
        Some(first.to_string().as_str())
    );

    clear_rating_anomaly_event(h.db(), &e.id, &second)
        .await
        .expect("second");
    assert_eq!(
        h.text_of(&q).await.as_deref(),
        Some(second.to_string().as_str()),
        "the second clear replaces the first triager"
    );
}

// ---------------------------------------------------------------------------
// get_work_anomaly_events
// ---------------------------------------------------------------------------

/// Only uncleared events are listed, newest first.
#[tokio::test]
async fn only_uncleared_events_are_listed_newest_first() {
    let h = Harness::new("ri-list-order").await;
    let (work, _, _) = h.work().await;
    let mut first = h.event(&work, "burst");
    first.detected_at = "2026-01-01T00:00:00Z".to_string();
    let mut second = h.event(&work, "burst");
    second.detected_at = "2026-01-02T00:00:00Z".to_string();
    let mut third = h.event(&work, "burst");
    third.detected_at = "2026-01-03T00:00:00Z".to_string();
    for e in [&first, &second, &third] {
        insert_rating_anomaly_event(h.db(), e)
            .await
            .expect("insert");
    }
    let real_second = h.real_account().await;
    clear_rating_anomaly_event(h.db(), &second.id, &real_second)
        .await
        .expect("clear");

    let events = get_work_anomaly_events(h.db(), &work).await.expect("read");
    assert_eq!(events.len(), 2, "the cleared one is not outstanding");
    assert_eq!(events[0].id, third.id, "newest first");
    assert_eq!(events[1].id, first.id);
}

/// A work with no events lists empty.
#[tokio::test]
async fn a_work_with_no_events_lists_empty() {
    let h = Harness::new("ri-list-empty").await;
    let (work, _, _) = h.work().await;
    assert!(get_work_anomaly_events(h.db(), &work)
        .await
        .expect("read")
        .is_empty());
}

/// Events are scoped to their work.
#[tokio::test]
async fn events_are_scoped_to_their_work() {
    let h = Harness::new("ri-list-scope").await;
    let (work, _, _) = h.work().await;
    let (other, _, _) = h.work().await;
    let e = h.event(&work, "burst");
    insert_rating_anomaly_event(h.db(), &e)
        .await
        .expect("insert");

    assert_eq!(
        get_work_anomaly_events(h.db(), &other)
            .await
            .expect("read")
            .len(),
        0
    );
    assert_eq!(
        get_work_anomaly_events(h.db(), &work)
            .await
            .expect("read")
            .len(),
        1
    );
}

/// Deleting a work cascades to its events, so a removed work leaves no orphan
/// signals behind.
#[tokio::test]
async fn deleting_a_work_cascades_to_its_events() {
    let h = Harness::new("ri-cascade").await;
    let (work, _, _) = h.work().await;
    let e = h.event(&work, "burst");
    insert_rating_anomaly_event(h.db(), &e)
        .await
        .expect("insert");
    assert_eq!(h.count("rating_anomaly_events").await, 1);

    h.exec(&format!("DELETE FROM works WHERE id = '{work}'"))
        .await;

    assert_eq!(
        h.count("rating_anomaly_events").await,
        0,
        "ON DELETE CASCADE"
    );
}

/// Deleting the account that cleared an event sets `cleared_by` to NULL rather
/// than deleting the event, so the triage record outlives the triager.
#[tokio::test]
async fn deleting_the_clearer_keeps_the_event() {
    let h = Harness::new("ri-setnull").await;
    let (work, _, _) = h.work().await;
    let e = h.event(&work, "burst");
    insert_rating_anomaly_event(h.db(), &e)
        .await
        .expect("insert");
    let clearer = h.real_account().await;
    clear_rating_anomaly_event(h.db(), &e.id, &clearer)
        .await
        .expect("clear");

    h.exec(&format!("DELETE FROM accounts WHERE id = '{clearer}'"))
        .await;

    assert_eq!(
        h.count("rating_anomaly_events").await,
        1,
        "ON DELETE SET NULL keeps the event"
    );
}

// ---------------------------------------------------------------------------
// contested flag
// ---------------------------------------------------------------------------

/// A new work is not contested.
///
/// **This is the `works.contested` type split.** The SQLite migration stores the
/// column as an integer and the PostgreSQL one as a boolean, so
/// `is_work_contested` reads `i64` on one backend and `bool` on the other.
#[tokio::test]
async fn a_new_work_is_not_contested() {
    let h = Harness::new("ri-contested-initial").await;
    let (work, _, _) = h.work().await;
    assert!(!is_work_contested(h.db(), &work).await.expect("contested"));
}

/// Setting the flag with a reason marks it contested and records why.
#[tokio::test]
async fn setting_the_flag_marks_the_work_contested() {
    let h = Harness::new("ri-contested-set").await;
    let (work, _, _) = h.work().await;
    set_work_contested(h.db(), &work, "brigade_detected")
        .await
        .expect("set_work_contested");

    assert!(is_work_contested(h.db(), &work).await.expect("contested"));
    assert_eq!(
        h.text_of(&format!(
            "SELECT contested_reason FROM works WHERE id = '{work}'"
        ))
        .await
        .as_deref(),
        Some("brigade_detected")
    );
}

/// Clearing the flag also clears the reason and the timestamp, so a work that
/// was contested and then cleared carries no stale reason.
#[tokio::test]
async fn clearing_the_flag_clears_the_reason() {
    let h = Harness::new("ri-contested-clear").await;
    let (work, _, _) = h.work().await;
    set_work_contested(h.db(), &work, "brigade_detected")
        .await
        .expect("set");
    clear_work_contested(h.db(), &work).await.expect("clear");

    assert!(!is_work_contested(h.db(), &work).await.expect("contested"));
    assert!(
        !h.text_of(&format!(
            "SELECT contested_reason FROM works WHERE id = '{work}'"
        ))
        .await
        .is_some_and(|s| !s.is_empty()),
        "the reason does not outlive the flag"
    );
    assert!(
        !h.text_of(&format!(
            "SELECT contested_at FROM works WHERE id = '{work}'"
        ))
        .await
        .is_some_and(|s| !s.is_empty()),
        "and neither does the timestamp"
    );
}

/// An empty reason is stored as an empty string, not NULL — the column is NOT
/// NULL-able in intent and the caller decides what an empty reason means.
#[tokio::test]
async fn an_empty_reason_is_stored_verbatim() {
    let h = Harness::new("ri-contested-empty").await;
    let (work, _, _) = h.work().await;
    set_work_contested(h.db(), &work, "").await.expect("set");
    assert_eq!(
        h.text_of(&format!(
            "SELECT contested_reason FROM works WHERE id = '{work}'"
        ))
        .await
        .as_deref(),
        Some("")
    );
}

/// Setting twice keeps the latest reason.
#[tokio::test]
async fn setting_twice_keeps_the_latest_reason() {
    let h = Harness::new("ri-contested-twice").await;
    let (work, _, _) = h.work().await;
    set_work_contested(h.db(), &work, "first")
        .await
        .expect("first");
    set_work_contested(h.db(), &work, "second")
        .await
        .expect("second");
    assert_eq!(
        h.text_of(&format!(
            "SELECT contested_reason FROM works WHERE id = '{work}'"
        ))
        .await
        .as_deref(),
        Some("second")
    );
}

/// An unknown work is not contested, and setting it is a silent no-op.
#[tokio::test]
async fn an_unknown_work_is_not_contested() {
    let h = Harness::new("ri-contested-unknown").await;
    let unknown = WorkId::new();
    assert!(!is_work_contested(h.db(), &unknown)
        .await
        .expect("contested"));
    set_work_contested(h.db(), &unknown, "reason")
        .await
        .expect("a missing work is not an error to flag");
    clear_work_contested(h.db(), &unknown)
        .await
        .expect("nor to unflag");
    assert!(!is_work_contested(h.db(), &unknown)
        .await
        .expect("contested"));
}

/// The contested flag is per work: flagging one leaves the other alone.
#[tokio::test]
async fn the_flag_is_per_work() {
    let h = Harness::new("ri-contested-per-work").await;
    let (work, _, _) = h.work().await;
    let (other, _, _) = h.work().await;
    set_work_contested(h.db(), &work, "reason")
        .await
        .expect("set");

    assert!(is_work_contested(h.db(), &work).await.expect("contested"));
    assert!(!is_work_contested(h.db(), &other).await.expect("contested"));
}

/// A full triage: signal raised, work contested, both cleared.
#[tokio::test]
async fn a_full_triage_round_trip() {
    let h = Harness::new("ri-triage").await;
    let (work, _, _) = h.work().await;
    let e = h.event(&work, "burst");
    insert_rating_anomaly_event(h.db(), &e)
        .await
        .expect("insert");
    set_work_contested(h.db(), &work, "rating_burst")
        .await
        .expect("set");
    assert!(is_work_contested(h.db(), &work).await.expect("contested"));

    let real_second = h.real_account().await;
    clear_rating_anomaly_event(h.db(), &e.id, &real_second)
        .await
        .expect("clear event");
    clear_work_contested(h.db(), &work)
        .await
        .expect("clear flag");

    assert!(!is_work_contested(h.db(), &work).await.expect("contested"));
    assert!(get_work_anomaly_events(h.db(), &work)
        .await
        .expect("read")
        .is_empty());
    assert_eq!(
        h.count("rating_anomaly_events").await,
        1,
        "and the audit rows are still there"
    );
}
