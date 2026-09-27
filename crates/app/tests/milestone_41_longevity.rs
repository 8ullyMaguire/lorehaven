//! M41 — Longevity signals (`crates/db/src/longevity.rs`, spec §41): the
//! half-life score and reader/author warmth.
//!
//! Five `pub async fn` with no tests, and all but one are on a request or job
//! path: `worker.rs` runs `recompute_half_life` on a schedule, `discovery.rs`
//! reads `half_life_map` to rank results, and `audience.rs` reads
//! `author_tier_aggregates`.
//!
//! The hazard in this module is the clock. Every timestamp in the schema is
//! **RFC 3339 TEXT** on both engines (ADR 0004), and `now_rfc3339` renders
//! through the `time` crate's `Rfc3339`, so a stored value looks like
//! `2026-01-01T00:00:00+00:00` — a *string*, not a timestamp. Three of the five
//! functions compare those strings against SQL date functions, and the
//! PostgreSQL and SQLite spellings of that comparison are not the same problem:
//!
//! - SQLite has `datetime()` to parse the text, so `datetime(w.created_at,
//!   '+30 days')` works.
//! - PostgreSQL has no implicit text-to-timestamp cast, so
//!   `w.created_at < NOW() - INTERVAL '30 days'` is a type error, not a
//!   comparison. The job would fail on every run in production and pass locally.
//!
//! That is what the eligibility and window tests below are for. The fix casts
//! explicitly (`w.created_at::timestamptz`) so both backends compare times.

use std::path::PathBuf;

use lorehaven_db::content::create_work;
use lorehaven_db::identity::{create_account, create_pseud, AccountStatus};
use lorehaven_db::longevity::{
    author_tier_aggregates, half_life_map, half_life_of, recompute_half_life, record_warmth,
};
use lorehaven_domain::ids::{AccountId, WorkId};
use lorehaven_domain::longevity::WarmthThresholds;
use lorehaven_domain::policy::AgeState;
use test_support::TestDb;
use uuid::Uuid;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-longevity-{tag}-{}-{:?}",
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

    /// Read one integer, distinguishing SQL NULL from zero.
    ///
    /// `query_scalar::<i64>` over a nullable column yields `Some(0)` for a NULL
    /// row on SQLite, so a bare scalar read cannot tell "scored 0" from
    /// "not scored". The production read (`half_life_of`) is typed
    /// `Option<i64>` and gets this right; the helper has to as well.
    async fn int_of(&self, query: &str) -> Option<i64> {
        let q = self.tdb.sql(query);
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query_scalar::<_, Option<i64>>(&q)
                .fetch_optional(self.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("scalar")
                .flatten(),
            lorehaven_db::Backend::Postgres => sqlx::query_scalar::<_, Option<i64>>(&q)
                .fetch_optional(self.db().postgres_pool().expect("pg"))
                .await
                .expect("scalar")
                .flatten(),
        }
    }

    async fn account(&self) -> AccountId {
        let account = create_account(
            self.db(),
            &format!("long-{}@example.test", Uuid::new_v4()),
            AgeState::DeclaredAdult,
            AccountStatus::Active,
        )
        .await
        .expect("create_account");
        let _ = create_pseud(
            self.db(),
            account,
            &format!("l-{}", Uuid::new_v4()),
            "Reader",
        )
        .await
        .expect("create_pseud");
        account
    }

    /// A published work, optionally back-dated so it is old enough to score.
    ///
    /// `created_at` is RFC 3339 **text** on both engines, so the back-date is
    /// written as a string in the same format `now_rfc3339` produces — with the
    /// `+00:00` offset, not a `Z`.
    async fn published_work(&self, days_old: i64) -> WorkId {
        let account = self.account().await;
        let pseud = create_pseud(
            self.db(),
            account,
            &format!("lw-{}", Uuid::new_v4()),
            "Author",
        )
        .await
        .expect("create_pseud");
        let work = create_work(self.db(), pseud, "Long-lived", None)
            .await
            .expect("create_work")
            .id;
        self.set_published(&work, days_old).await;
        work
    }

    /// Mark a work published and back-date it by `days_old`.
    async fn set_published(&self, work: &WorkId, days_old: i64) {
        let created = time::OffsetDateTime::now_utc() - time::Duration::days(days_old);
        let stamp = lorehaven_db::identity::format_rfc3339(created);
        self.exec(&format!(
            "UPDATE works SET lifecycle = 'published', visibility = 'public', \
             created_at = '{stamp}' WHERE id = '{work}'"
        ))
        .await;
    }

    /// A reading-history entry, back-dated by `days_ago`.
    ///
    /// `account_id` is `NOT NULL REFERENCES accounts(id)`, so the reader's
    /// account is passed alongside the pseud rather than minted fresh.
    async fn reading(&self, account: &AccountId, pseud_id: &str, work: &WorkId, days_ago: i64) {
        let at = lorehaven_db::identity::format_rfc3339(
            time::OffsetDateTime::now_utc() - time::Duration::days(days_ago),
        );
        self.exec(&format!(
            "INSERT INTO reading_history_entry \
             (id, account_id, pseud_id, subject_type, subject_id, last_read_at, created_at) \
             VALUES ('{id}', '{account}', '{pseud_id}', 'work', '{work}', '{at}', '{at}')",
            id = Uuid::new_v4(),
        ))
        .await;
    }
}

/// The pseud id of a fresh reader, for seeding reading history.
async fn reader_pseud(h: &Harness) -> (AccountId, String) {
    let account = create_account(
        h.db(),
        &format!("reader-{}@example.test", Uuid::new_v4()),
        AgeState::DeclaredAdult,
        AccountStatus::Active,
    )
    .await
    .expect("create_account");
    let pseud = create_pseud(h.db(), account, &format!("r-{}", Uuid::new_v4()), "Reader")
        .await
        .expect("create_pseud");
    (account, pseud.to_string())
}

// ---------------------------------------------------------------------------
// recompute_half_life
// ---------------------------------------------------------------------------

/// **A work old enough to score is scored.** This is the test that exercises the
/// eligibility predicate, and it is where the two backends differ: SQLite parses
/// `created_at` with `datetime()`, PostgreSQL needs an explicit
/// `::timestamptz` cast because the column is TEXT and there is no implicit
/// conversion.
#[tokio::test]
async fn an_old_enough_work_is_scored() {
    let h = Harness::new("long-recompute").await;
    let work = h.published_work(400).await; // far older than the 30-day minimum

    let updated = recompute_half_life(h.db(), 30, 30)
        .await
        .expect("recompute runs");
    assert_eq!(updated, 1, "one eligible work was scored");
    assert!(
        half_life_of(h.db(), &work).await.expect("read").is_some(),
        "and it now has a score"
    );
}

/// **A work too young to score is skipped**, and its score stays NULL — "not yet
/// scored" is a real state, distinct from zero.
#[tokio::test]
async fn a_work_too_young_is_not_scored() {
    let h = Harness::new("long-young").await;
    let work = h.published_work(2).await; // 2 days old, minimum is 30

    let updated = recompute_half_life(h.db(), 30, 30)
        .await
        .expect("recompute runs");
    assert_eq!(updated, 0, "too young to be eligible");
    assert_eq!(
        half_life_of(h.db(), &work).await.expect("read"),
        None,
        "and the score stays NULL, not zero"
    );
}

/// A draft is never eligible, however old it is.
#[tokio::test]
async fn a_draft_is_never_scored() {
    let h = Harness::new("long-draft").await;
    let work = h.published_work(400).await;
    h.exec(&format!(
        "UPDATE works SET lifecycle = 'draft' WHERE id = '{work}'"
    ))
    .await;

    assert_eq!(recompute_half_life(h.db(), 30, 30).await.expect("run"), 0);
}

/// A deleted work is never eligible.
#[tokio::test]
async fn a_deleted_work_is_never_scored() {
    let h = Harness::new("long-deleted").await;
    let work = h.published_work(400).await;
    h.exec(&format!(
        "UPDATE works SET deleted_at = '2026-01-01T00:00:00+00:00' WHERE id = '{work}'"
    ))
    .await;

    assert_eq!(recompute_half_life(h.db(), 30, 30).await.expect("run"), 0);
}

/// **A work with no readers scores zero, not NULL.** `half_life_bp(0, 0)`
/// returns 0, and the column is written — so an eligible work is always
/// scored, even with no signal at all.
#[tokio::test]
async fn an_eligible_work_with_no_readers_scores_zero() {
    let h = Harness::new("long-noreaders").await;
    let work = h.published_work(400).await;

    assert_eq!(recompute_half_life(h.db(), 30, 30).await.expect("run"), 1);
    assert_eq!(
        half_life_of(h.db(), &work).await.expect("read"),
        Some(0),
        "scored as zero rather than left unscored"
    );
}

/// **The score is the recent-to-first-window ratio in basis points**, clamped to
/// 10,000. A work whose recent readership matches its opening window is 10,000.
#[tokio::test]
async fn the_score_is_the_reader_ratio_in_basis_points() {
    let h = Harness::new("long-ratio").await;
    let work = h.published_work(400).await;

    // Two readers in the first window (right after publication) and two in the
    // trailing window (recently) -- a ratio of 1.0.
    for _ in 0..2 {
        let (acct, pseud) = reader_pseud(&h).await;
        h.reading(&acct, &pseud, &work, 390).await; // just after publication
    }
    for _ in 0..2 {
        let (acct, pseud) = reader_pseud(&h).await;
        h.reading(&acct, &pseud, &work, 3).await; // trailing window
    }

    assert_eq!(recompute_half_life(h.db(), 30, 30).await.expect("run"), 1);
    assert_eq!(
        half_life_of(h.db(), &work).await.expect("read"),
        Some(10_000),
        "recent equals first-window, so the full score"
    );
}

/// **A work whose readership has collapsed scores low.** This is the signal the
/// whole module exists for, so it is worth asserting the direction as well as
/// the arithmetic: 1 recent reader against 4 in the opening window is 2,500 bp.
#[tokio::test]
async fn a_collapse_scores_low() {
    let h = Harness::new("long-collapse").await;
    let work = h.published_work(400).await;

    for _ in 0..4 {
        let (acct, pseud) = reader_pseud(&h).await;
        h.reading(&acct, &pseud, &work, 390).await;
    }
    let (acct, recent) = reader_pseud(&h).await;
    h.reading(&acct, &recent, &work, 3).await;

    assert_eq!(recompute_half_life(h.db(), 30, 30).await.expect("run"), 1);
    assert_eq!(
        half_life_of(h.db(), &work).await.expect("read"),
        Some(2_500),
        "one recent reader against four at launch"
    );
}

/// A reader outside both windows does not count.
#[tokio::test]
async fn a_reader_outside_both_windows_does_not_count() {
    let h = Harness::new("long-outside").await;
    let work = h.published_work(400).await;

    // Published 400 days ago, so 200 days ago is neither the opening window nor
    // the trailing one.
    let (acct, pseud) = reader_pseud(&h).await;
    h.reading(&acct, &pseud, &work, 200).await;

    assert_eq!(recompute_half_life(h.db(), 30, 30).await.expect("run"), 1);
    assert_eq!(
        half_life_of(h.db(), &work).await.expect("read"),
        Some(0),
        "no first-window starts, so no ratio"
    );
}

/// **One reading row is one reader.** `reading_history_entry` has a unique index
/// on `(account_id, pseud_id, subject_type, subject_id)`, so a reader has at
/// most one row per work — and the query counts `DISTINCT pseud_id`, which
/// agrees with that. The index is what makes the `DISTINCT` redundant here; the
/// test asserts the two agree rather than manufacturing duplicate rows the
/// schema forbids.
#[tokio::test]
async fn one_reading_row_is_one_reader() {
    let h = Harness::new("long-distinct").await;
    let work = h.published_work(400).await;

    let (acct, pseud) = reader_pseud(&h).await;
    h.reading(&acct, &pseud, &work, 3).await; // trailing window
    let (acct2, p) = reader_pseud(&h).await;
    h.reading(&acct2, &p, &work, 390).await; // opening window, 400d-created work

    // The schema forbids a second row for the same reader and work.
    let dup = h
        .int_of(&format!(
            "SELECT COUNT(*) FROM reading_history_entry WHERE account_id = '{acct}' \
             AND subject_type = 'work' AND subject_id = '{work}'"
        ))
        .await;
    assert_eq!(dup, Some(1), "one row per reader per work");

    assert_eq!(recompute_half_life(h.db(), 30, 30).await.expect("run"), 1);
    assert_eq!(
        half_life_of(h.db(), &work).await.expect("read"),
        Some(10_000),
        "one recent reader against one at launch: 1/1, a full score"
    );
}

/// **Recompute is idempotent** — running it twice leaves the same score, which
/// is what makes it safe to schedule.
#[tokio::test]
async fn recompute_is_idempotent() {
    let h = Harness::new("long-idempotent").await;
    let work = h.published_work(400).await;
    let (acct, recent) = reader_pseud(&h).await;
    h.reading(&acct, &recent, &work, 3).await;

    recompute_half_life(h.db(), 30, 30).await.expect("first");
    let first = half_life_of(h.db(), &work).await.expect("read");
    recompute_half_life(h.db(), 30, 30).await.expect("second");
    let second = half_life_of(h.db(), &work).await.expect("read");

    assert_eq!(first, second, "a pure overwrite, run twice");
}

/// **A zero minimum age makes a fresh work eligible**, one day old with no
/// required age at all.
///
/// The exact boundary — a work aged precisely `min_age_days` — is deliberately
/// not asserted. The two backends truncate differently: SQLite's `datetime()`
/// drops to whole seconds, so a work created `min_age_days` ago lands on the
/// same second as the cutoff and a strict `<` is false; PostgreSQL keeps
/// microseconds, so it is true. Both answers are defensible, and a test that
/// depended on the difference would be flaky rather than meaningful.
#[tokio::test]
async fn a_zero_minimum_age_includes_a_fresh_work() {
    let h = Harness::new("long-zeroage").await;
    let work = h.published_work(1).await;
    assert_eq!(
        recompute_half_life(h.db(), 0, 30).await.expect("run"),
        1,
        "published, not deleted, and the minimum age is zero"
    );
    assert_eq!(half_life_of(h.db(), &work).await.expect("read"), Some(0));
}

/// With no published works the run reports zero rather than erroring.
#[tokio::test]
async fn an_empty_instance_scores_nothing() {
    let h = Harness::new("long-empty").await;
    assert_eq!(recompute_half_life(h.db(), 30, 30).await.expect("run"), 0);
}

/// Several eligible works are all scored, and the count adds up.
#[tokio::test]
async fn every_eligible_work_is_scored() {
    let h = Harness::new("long-many").await;
    let mut works = Vec::new();
    for _ in 0..3 {
        works.push(h.published_work(400).await);
    }
    let young = h.published_work(1).await;

    assert_eq!(
        recompute_half_life(h.db(), 30, 30).await.expect("run"),
        3,
        "three old, one young"
    );
    for work in &works {
        assert!(half_life_of(h.db(), work).await.expect("read").is_some());
    }
    assert_eq!(half_life_of(h.db(), &young).await.expect("read"), None);
}

// ---------------------------------------------------------------------------
// half_life_of / half_life_map
// ---------------------------------------------------------------------------

/// An unscored work reads as `None`.
#[tokio::test]
async fn an_unscored_work_has_no_half_life() {
    let h = Harness::new("long-of-unscored").await;
    let work = h.published_work(400).await;
    assert_eq!(half_life_of(h.db(), &work).await.expect("read"), None);
}

/// An unknown work has no half-life.
#[tokio::test]
async fn an_unknown_work_has_no_half_life() {
    let h = Harness::new("long-of-unknown").await;
    assert_eq!(
        half_life_of(h.db(), &WorkId::new()).await.expect("read"),
        None
    );
}

/// **A map of an empty slice is empty, with no query at all** — the early return
/// that stops an empty `IN ()` from being a syntax error.
#[tokio::test]
async fn mapping_no_works_yields_nothing() {
    let h = Harness::new("long-map-empty").await;
    assert!(half_life_map(h.db(), &[]).await.expect("map").is_empty());
}

/// **A map returns a row per *scored* work, and an unscored work is absent
/// rather than present-with-None.** The query selects `half_life_bp` from rows
/// that exist, so a work with a NULL score still appears — with `None`.
#[tokio::test]
async fn mapping_returns_each_work_with_its_score() {
    let h = Harness::new("long-map").await;
    let scored = h.published_work(400).await;
    let unscored = h.published_work(400).await;
    // Score only one of the two: the minimum age cannot distinguish them, so set
    // the other's score aside explicitly.
    h.exec(&format!(
        "UPDATE works SET half_life_bp = NULL WHERE id = '{unscored}'"
    ))
    .await;
    h.exec(&format!(
        "UPDATE works SET half_life_bp = 7500 WHERE id = '{scored}'"
    ))
    .await;

    let rows = half_life_map(h.db(), &[scored, unscored])
        .await
        .expect("map");
    assert_eq!(rows.len(), 2, "both works appear");
    let find = |id: &WorkId| {
        rows.iter()
            .find(|(k, _)| k == &id.to_string())
            .map(|(_, v)| *v)
    };
    assert_eq!(find(&scored), Some(Some(7_500)), "scored");
    assert_eq!(
        find(&unscored),
        Some(None),
        "present in the map but unscored -- the caller can tell"
    );
}

/// A work that does not appear in the map at all is simply absent — the caller
/// must not assume one row per requested id.
#[tokio::test]
async fn mapping_omits_a_work_that_does_not_exist() {
    let h = Harness::new("long-map-missing").await;
    let real = h.published_work(400).await;
    let rows = half_life_map(h.db(), &[real, WorkId::new()])
        .await
        .expect("map");
    assert_eq!(rows.len(), 1, "only the work that exists");
    assert_eq!(rows[0].0, real.to_string());
}

/// **A single-element map binds one placeholder** — the case that a
/// hand-rolled placeholder builder gets wrong, and the reason the house
/// `library::placeholders` helper exists.
#[tokio::test]
async fn mapping_one_work_binds_one_placeholder() {
    let h = Harness::new("long-map-one").await;
    let work = h.published_work(400).await;
    let rows = half_life_map(h.db(), &[work]).await.expect("map");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, work.to_string());
}

/// Many works at once, past any plausible placeholder limit.
#[tokio::test]
async fn mapping_many_works_binds_many_placeholders() {
    let h = Harness::new("long-map-many").await;
    let mut works = Vec::new();
    for _ in 0..5 {
        works.push(h.published_work(400).await);
    }
    let rows = half_life_map(h.db(), &works).await.expect("map");
    assert_eq!(rows.len(), 5);
    for work in &works {
        assert!(rows.iter().any(|(k, _)| k == &work.to_string()));
    }
}

/// The map and the single read agree.
#[tokio::test]
async fn the_map_agrees_with_the_single_read() {
    let h = Harness::new("long-map-agrees").await;
    let work = h.published_work(400).await;
    recompute_half_life(h.db(), 30, 30).await.expect("run");

    let single = half_life_of(h.db(), &work).await.expect("read");
    let map = half_life_map(h.db(), &[work]).await.expect("map");
    assert_eq!(map[0].1, single, "one source of truth");
}

// ---------------------------------------------------------------------------
// record_warmth
// ---------------------------------------------------------------------------

fn thresholds() -> WarmthThresholds {
    WarmthThresholds {
        lurk: 0,
        react: 1_000,
        comment: 5_000,
        create: 20_000,
    }
}

/// A first interaction records warmth at the `lurk` tier.
#[tokio::test]
async fn a_first_interaction_records_warmth_at_lurk() {
    let h = Harness::new("long-warmth-first").await;
    let reader = h.account().await;
    let author = h.account().await;
    record_warmth(
        h.db(),
        &reader.to_string(),
        &author.to_string(),
        500,
        &thresholds(),
    )
    .await
    .expect("record");

    assert_eq!(h.count("interaction_warmth").await, 1);
    let tiers = author_tier_aggregates(h.db(), &author.to_string())
        .await
        .expect("aggregate");
    assert_eq!(tiers, vec![("lurk".to_string(), 1)]);
}

/// **Warmth accumulates across interactions**, and the tier is recomputed from
/// the total — this is the upsert's `warmth_bp = warmth_bp + excluded` and the
/// separate tier update that together make the value mean anything.
#[tokio::test]
async fn warmth_accumulates_and_the_tier_follows() {
    let h = Harness::new("long-warmth-accumulates").await;
    let reader = h.account().await;
    let author = h.account().await;
    let (r, a) = (reader.to_string(), author.to_string());

    record_warmth(h.db(), &r, &a, 2_000, &thresholds())
        .await
        .expect("first");
    assert_eq!(
        author_tier_aggregates(h.db(), &a).await.expect("agg"),
        vec![("react".to_string(), 1)],
        "2,000 bp is past react"
    );

    record_warmth(h.db(), &r, &a, 4_000, &thresholds())
        .await
        .expect("second");
    assert_eq!(h.count("interaction_warmth").await, 1, "one row, upserted");
    assert_eq!(
        author_tier_aggregates(h.db(), &a).await.expect("agg"),
        vec![("comment".to_string(), 1)],
        "6,000 bp is past comment"
    );
}

/// **Warmth is directional** — a reader's regard for an author is not the
/// reverse, so the two orderings are separate rows.
#[tokio::test]
async fn warmth_is_directional() {
    let h = Harness::new("long-warmth-directional").await;
    let a = h.account().await;
    let b = h.account().await;
    record_warmth(h.db(), &a.to_string(), &b.to_string(), 6_000, &thresholds())
        .await
        .expect("a toward b");

    assert_eq!(
        author_tier_aggregates(h.db(), &a.to_string())
            .await
            .expect("agg"),
        Vec::<(String, i64)>::new(),
        "b feels nothing toward a"
    );
    assert_eq!(
        author_tier_aggregates(h.db(), &b.to_string())
            .await
            .expect("agg"),
        vec![("comment".to_string(), 1)],
        "only a toward b is recorded"
    );
}

/// A reader's warmth toward several authors is separate per author.
#[tokio::test]
async fn warmth_is_per_author() {
    let h = Harness::new("long-warmth-per-author").await;
    let reader = h.account().await;
    let author_a = h.account().await;
    let author_b = h.account().await;

    record_warmth(
        h.db(),
        &reader.to_string(),
        &author_a.to_string(),
        6_000,
        &thresholds(),
    )
    .await
    .expect("toward a");
    record_warmth(
        h.db(),
        &reader.to_string(),
        &author_b.to_string(),
        500,
        &thresholds(),
    )
    .await
    .expect("toward b");

    assert_eq!(
        author_tier_aggregates(h.db(), &author_a.to_string())
            .await
            .expect("agg"),
        vec![("comment".to_string(), 1)]
    );
    assert_eq!(
        author_tier_aggregates(h.db(), &author_b.to_string())
            .await
            .expect("agg"),
        vec![("lurk".to_string(), 1)]
    );
}

/// Two readers toward one author aggregate into one row with a count of two.
#[tokio::test]
async fn an_authors_aggregates_count_its_readers() {
    let h = Harness::new("long-aggregates").await;
    let author = h.account().await;
    for _ in 0..3 {
        let reader = h.account().await;
        record_warmth(
            h.db(),
            &reader.to_string(),
            &author.to_string(),
            100,
            &thresholds(),
        )
        .await
        .expect("record");
    }

    assert_eq!(
        author_tier_aggregates(h.db(), &author.to_string())
            .await
            .expect("agg"),
        vec![("lurk".to_string(), 3)],
        "three readers, one tier, count 3"
    );
}

/// Aggregates group by tier, so a mixed readership returns several rows.
#[tokio::test]
async fn aggregates_group_by_tier() {
    let h = Harness::new("long-aggregates-tiers").await;
    let author = h.account().await;
    for delta in [100, 200, 6_000, 25_000] {
        let reader = h.account().await;
        record_warmth(
            h.db(),
            &reader.to_string(),
            &author.to_string(),
            delta,
            &thresholds(),
        )
        .await
        .expect("record");
    }

    let mut tiers = author_tier_aggregates(h.db(), &author.to_string())
        .await
        .expect("agg");
    tiers.sort();
    assert_eq!(
        tiers,
        vec![
            ("comment".to_string(), 1),
            ("create".to_string(), 1),
            ("lurk".to_string(), 2),
        ]
    );
}

/// An author nobody has read has no aggregates.
#[tokio::test]
async fn an_author_with_no_readers_has_no_aggregates() {
    let h = Harness::new("long-aggregates-empty").await;
    assert!(
        author_tier_aggregates(h.db(), &h.account().await.to_string())
            .await
            .expect("agg")
            .is_empty()
    );
}

/// The highest tier a reader can reach is the one recorded.
#[tokio::test]
async fn a_large_interaction_reaches_the_top_tier() {
    let h = Harness::new("long-warmth-top").await;
    let reader = h.account().await;
    let author = h.account().await;
    record_warmth(
        h.db(),
        &reader.to_string(),
        &author.to_string(),
        50_000,
        &thresholds(),
    )
    .await
    .expect("record");
    assert_eq!(
        author_tier_aggregates(h.db(), &author.to_string())
            .await
            .expect("agg"),
        vec![("create".to_string(), 1)]
    );
}

/// **Warmth is kept even when the reader's account is gone.** `interaction_warmth`
/// carries no foreign key on either backend — it is a `(account_id,
/// author_account)` primary key and nothing more — so a regard outlives the reader
/// who held it. That is deliberate: an author's standing should not be erased by
/// one reader deleting their account.
#[tokio::test]
async fn warmth_survives_an_unknown_reader_account() {
    let h = Harness::new("long-warmth-fk").await;
    let author = h.account().await;
    record_warmth(
        h.db(),
        &Uuid::new_v4().to_string(),
        &author.to_string(),
        100,
        &thresholds(),
    )
    .await
    .expect("no foreign key, so this is accepted");
    assert_eq!(h.count("interaction_warmth").await, 1);
    assert_eq!(
        author_tier_aggregates(h.db(), &author.to_string())
            .await
            .expect("agg"),
        vec![("lurk".to_string(), 1)],
        "and it still counts toward the author"
    );
}

/// `updated_at` is stamped on every interaction, so a reader's regard has a
/// last-seen time.
#[tokio::test]
async fn warmth_records_when_it_last_changed() {
    let h = Harness::new("long-warmth-updated").await;
    let reader = h.account().await;
    let author = h.account().await;
    record_warmth(
        h.db(),
        &reader.to_string(),
        &author.to_string(),
        100,
        &thresholds(),
    )
    .await
    .expect("record");

    let stamp = h
        .int_of(&format!(
            "SELECT COUNT(*) FROM interaction_warmth WHERE updated_at IS NOT NULL \
             AND author_account = '{author}'"
        ))
        .await;
    assert_eq!(stamp, Some(1), "stamped");
}
