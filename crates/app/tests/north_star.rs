//! M45-23 — the north-star metric and its per-mechanism attribution (spec §53.5).
//!
//! Seven cases, and each one is written to fail against the plausible wrong
//! implementation rather than merely to pass against the right one. That distinction is
//! the whole reason this file exists: a metric like this is easy to implement in a way
//! that returns plausible numbers for the empty case and quietly wrong numbers for every
//! other one.
//!
//! | # | case | implementation it rules out |
//! |---|---|---|
//! | 1 | a loved work with no slot row is counted, as `unattributed` | filtering unattributable rows out |
//! | 2 | the **earliest** slot wins | "last slot" or "all slots" |
//! | 3 | a slot recorded *after* the rating claims nothing | attributing by work alone |
//! | 4 | `median_days_to_find` is `None`, and serialises as `null` | `Some(0.0)`; a serialiser that turns `None` into `0` |
//! | 5 | 3 stars is not loved; 4 is | the `>= 4` boundary drifting |
//! | 6 | `finished` **alone** earns §53.6's denominator | reusing §53.5's "finished or rated >= 4" |
//! | 7 | shares account for every loved work | a mechanism name missing somewhere |
//!
//! Cases 2 and 3 are the pair that matters most: they are what separates "which mechanism
//! surfaced this" from "which mechanism was nearby". An implementation that ignores
//! ordering passes case 2 with one slot and passes case 1 with no slots, and only case 3
//! catches it.
//!
//! The store is driven directly rather than through the route. The route's own
//! constraints — operator-only, 404-not-403, no per-account detail (§53.2) — are
//! asserted in `north_star_routes.rs`; testing them here would mean seeding a full HTTP
//! world to reach a query that has no HTTP in it.

use lorehaven_db::north_star;
use test_support::TestDb;

const SINCE: &str = "2026-01-01T00:00:00Z";
const UNTIL: &str = "2026-12-01T00:00:00Z";

struct World {
    /// The account created by `seed_base`. Carried here so no test has to mint its own:
    /// every test did, each with a slightly different label, and each produced a rating
    /// whose `account_id` matched no row -- reported as `FOREIGN KEY constraint failed`,
    /// which names no column and so sent the first five failures looking at the wrong
    /// table entirely.
    account: String,
    _db: TestDb,
    /// The SQLite file lives here. Held so it is not swept mid-test.
    _dir: std::path::PathBuf,
}

/// A pseud, a work, a slot and a rating, wired the way the store expects.
struct Seed {
    pseud: String,
    /// The label this fixture was built with, so ids derived inside the helpers stay
    /// distinct per test. `test_support::id` hashes its label, so a shared label would
    /// make two tests insert the same primary key.
    tag: String,
}

async fn seed_base(tag: &str) -> (World, Seed) {
    // `connect_with_dir` rather than a bare connect, and `id` takes a label. Labels are
    // deterministic (fnv1a of the string), so every row in this file is reproducible --
    // and two DIFFERENT labels give two different ids, which is what the multi-work
    // cases need.
    let dir = test_support::scratch_dir(tag);
    let tdb = TestDb::connect_with_dir(tag, &dir).await;
    let s = Seed {
        tag: tag.to_string(),
        pseud: test_support::id(&format!("ns-pseud-{tag}")),
    };
    let account = test_support::id(&format!("ns-account-{tag}"));
    let db = tdb.db();
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?, ?, ?, ?)",
            )
            .bind(&account)
            .bind(format!("{account}@ns.test"))
            .bind(SINCE)
            .bind(SINCE)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("account");

            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&s.pseud)
            .bind(&account)
            .bind(format!("p{}", &s.pseud[..8]))
            .bind("A Reader")
            .bind(SINCE)
            .bind(SINCE)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("pseud");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO accounts (id, email, created_at, updated_at)
                 VALUES ($1::uuid, $2, $3, $4)",
            )
            .bind(&account)
            .bind(format!("{account}@ns.test"))
            .bind(SINCE)
            .bind(SINCE)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("account");

            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6)",
            )
            .bind(&s.pseud)
            .bind(&account)
            .bind(format!("p{}", &s.pseud[..8]))
            .bind("A Reader")
            .bind(SINCE)
            .bind(SINCE)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("pseud");
        }
    }
    let world = World {
        account,
        _db: tdb,
        _dir: dir,
    };
    (world, s)
}

impl Seed {
    /// Insert a work row and return its id.
    ///
    /// `rating.work_id`, `recommendation_slots.work_id` and
    /// `reading_status.subject_id` all carry a foreign key to `works(id)`, so a rating
    /// cannot be written against a made-up id: SQLite reports
    /// `FOREIGN KEY constraint failed` and PostgreSQL reports a violation. Every loved
    /// work in this file therefore has to exist as a work first — which is also more
    /// honest, since the metric is about works.
    async fn new_work(db: &lorehaven_db::Database, owner: &str, label: &str) -> String {
        let id = test_support::id(label);
        match db.backend() {
            lorehaven_db::Backend::Sqlite => {
                sqlx::query(
                    "INSERT INTO works (id, owner_pseud_id, title, visibility, lifecycle,
                                        created_at, updated_at)
                     VALUES (?, ?, ?, 'public', 'published', ?, ?)",
                )
                .bind(&id)
                .bind(owner)
                .bind(format!("Work {label}"))
                .bind(SINCE)
                .bind(SINCE)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("work");
            }
            lorehaven_db::Backend::Postgres => {
                sqlx::query(
                    "INSERT INTO works (id, owner_pseud_id, title, visibility, lifecycle,
                                        created_at, updated_at)
                     VALUES ($1::uuid, $2::uuid, $3, 'public', 'published', $4, $5)",
                )
                .bind(&id)
                .bind(owner)
                .bind(format!("Work {label}"))
                .bind(SINCE)
                .bind(SINCE)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("work");
            }
        }
        id
    }

    /// Insert a `recommendation_slots` row for `work`, served at `served_at`.
    ///
    /// Written by hand rather than through `record_response`, because that function
    /// stamps `created_at` with `now()` and cases 2 and 3 are entirely about which of two
    /// timestamps comes first. A helper that cannot set the timestamp under test cannot
    /// test ordering at all.
    ///
    /// `mechanism` is passed explicitly and is the same column the serve path writes, so
    /// the attribution half of the query is exercised against real data.
    #[allow(clippy::too_many_arguments)]
    async fn slot_at(
        &self,
        db: &lorehaven_db::Database,
        work: &str,
        pseud: &str,
        served_at: &str,
        mechanism: &str,
    ) {
        let id = test_support::id(&format!("ns-slotrow-{mechanism}-{served_at}-{work}"));
        match db.backend() {
            lorehaven_db::Backend::Sqlite => {
                sqlx::query(
                    "INSERT INTO recommendation_slots
                       (id, pseud_id, work_id, request_id, position, reasons, blend_score,
                        created_at, mechanism)
                     VALUES (?, ?, ?, ?, 0, '[]', 0, ?, ?)",
                )
                .bind(&id)
                .bind(pseud)
                .bind(work)
                .bind(test_support::id(&format!(
                    "ns-slotreq-{mechanism}-{served_at}-{work}"
                )))
                .bind(served_at)
                .bind(mechanism)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("insert slot");
            }
            lorehaven_db::Backend::Postgres => {
                // `created_at` is TIMESTAMPTZ here (unlike `rating.created_at`, which is
                // TEXT) so the bind is cast. That asymmetry is the store's central hazard
                // and it is why the test has to be written per arm.
                sqlx::query(
                    "INSERT INTO recommendation_slots
                       (id, pseud_id, work_id, request_id, position, reasons, blend_score,
                        created_at, mechanism)
                     VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid, 0, '[]'::jsonb, 0, $5::timestamptz, $6)",
                )
                .bind(&id)
                .bind(pseud)
                .bind(work)
                .bind(test_support::id(&format!("ns-slotreq-{mechanism}-{served_at}-{work}")))
                .bind(served_at)
                .bind(mechanism)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("insert slot");
            }
        }
    }

    /// Insert a rating of `stars` for `work` at `rated_at`.
    async fn rating(
        &self,
        db: &lorehaven_db::Database,
        account: &str,
        work: &str,
        stars: i64,
        rated_at: &str,
    ) {
        match db.backend() {
            lorehaven_db::Backend::Sqlite => {
                // Every FK target must exist before the insert, and `787` names none of
                // them. Checked explicitly so the failure says which one is missing.
                for (label, sql) in [
                    ("accounts", "SELECT 1 FROM accounts WHERE id = ?"),
                    ("pseuds", "SELECT 1 FROM pseuds WHERE id = ?"),
                    ("works", "SELECT 1 FROM works WHERE id = ?"),
                ] {
                    let bound = match label {
                        "accounts" => account,
                        "pseuds" => self.pseud.as_str(),
                        _ => work,
                    };
                    let found: Option<(i64,)> = sqlx::query_as(sql)
                        .bind(bound)
                        .fetch_optional(db.sqlite_pool().expect("sqlite"))
                        .await
                        .unwrap_or(None);
                    assert!(
                        found.is_some(),
                        "FK target missing: {label} {bound} is not in the table"
                    );
                }
                sqlx::query(
                    "INSERT INTO rating (id, account_id, pseud_id, work_id, stars, is_public,
                                         created_at, updated_at)
                     VALUES (?, ?, ?, ?, ?, 1, ?, ?)",
                )
                .bind(test_support::id(&format!("ns-rate-{work}-{rated_at}")))
                .bind(account)
                .bind(&self.pseud)
                .bind(work)
                .bind(stars)
                .bind(rated_at)
                .bind(rated_at)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .unwrap_or_else(|e| {
                    // Name every id involved. `FOREIGN KEY constraint failed` on SQLite
                    // names no column, so the first version of this test failed five times
                    // with an error that could not distinguish a missing `works` row from a
                    // missing `pseuds` row from a duplicate primary key.
                    panic!(
                        "rating(work={work}, account={account}, pseud={}, at={rated_at}): {e}",
                        self.pseud
                    )
                });
            }
            lorehaven_db::Backend::Postgres => {
                sqlx::query(
                    "INSERT INTO rating (id, account_id, pseud_id, work_id, stars, is_public,
                                         created_at, updated_at)
                     VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid, $5, true, $6, $7)",
                )
                .bind(test_support::id(&format!("ns-rate-{work}-{rated_at}")))
                .bind(account)
                .bind(&self.pseud)
                .bind(work)
                .bind(stars)
                .bind(rated_at)
                .bind(rated_at)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("rating");
            }
        }
    }

    /// Mark `work` finished for `account`, at `finished_at`.
    async fn finish(
        &self,
        db: &lorehaven_db::Database,
        account: &str,
        work: &str,
        finished_at: &str,
    ) {
        match db.backend() {
            lorehaven_db::Backend::Sqlite => {
                sqlx::query(
                    "INSERT INTO reading_status (id, account_id, subject_type, subject_id,
                                                 status, started_at, finished_at, updated_at)
                     VALUES (?, ?, 'work', ?, 'finished', ?, ?, ?)",
                )
                .bind(test_support::id(&format!("ns-rs-{}", self.tag)))
                .bind(account)
                .bind(work)
                .bind(finished_at)
                .bind(finished_at)
                .bind(finished_at)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("finish");
            }
            lorehaven_db::Backend::Postgres => {
                sqlx::query(
                    "INSERT INTO reading_status (id, account_id, subject_type, subject_id,
                                                 status, started_at, finished_at, updated_at)
                     VALUES ($1::uuid, $2::uuid, 'work', $3::uuid, 'finished', $4, $5, $6)",
                )
                .bind(test_support::id(&format!("ns-rs-{}", self.tag)))
                .bind(account)
                .bind(work)
                .bind(finished_at)
                .bind(finished_at)
                .bind(finished_at)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("finish");
            }
        }
    }
}

/// Case 4 and §53.5: with nothing measured, the median is `None` — undefined, not zero.
#[tokio::test]
async fn an_empty_window_reports_undefined_rather_than_zero() {
    let (w, _s) = seed_base("ns-empty").await;
    let metric = north_star::north_star(w._db.db(), SINCE, UNTIL)
        .await
        .expect("north star on an empty window");

    assert_eq!(metric.rated_works, 0);
    assert_eq!(
        metric.median_days_to_find, None,
        "no completed pair is UNDEFINED, not 0 days -- a zero reads as instant discovery"
    );
    assert_eq!(
        metric.works_rated_per_month, 0.0,
        "and the rate is 0 only because the count is 0"
    );
    // §53.5: it reports its own missing inputs, which is what distinguishes "nothing
    // happened" from "nothing was measured".
    let missing: Vec<&str> = metric.missing_inputs.iter().map(|m| m.as_str()).collect();
    assert!(
        missing.contains(&"ratings"),
        "no ratings is a missing input, not a zero: {missing:?}"
    );
    assert!(
        missing.contains(&"slots"),
        "no slots is a missing input: {missing:?}"
    );
    // And it serialises as JSON null rather than 0. Asserted on the JSON, not the Rust
    // type: a serialiser that turned `None` into `0` would pass a type-level test.
    let json = serde_json::to_value(&metric).expect("serialise");
    assert_eq!(
        json["median_days_to_find"],
        serde_json::Value::Null,
        "the wire form must be null, not 0: {json}"
    );
}

/// Case 1: a loved work this instance never served is counted, as `unattributed`.
#[tokio::test]
async fn a_loved_work_with_no_slot_is_counted_as_unattributed() {
    let (w, s) = seed_base("ns-unattributed").await;
    let account = w.account.clone();
    let db = w._db.db();
    // The work exists and is loved, and was never served: an import, a sister instance, or
    // an author's own shelf all look like this.
    let work = Seed::new_work(db, &s.pseud, "ns-unattributed-work").await;
    s.rating(db, &account, &work, 4, "2026-06-01T00:00:00Z")
        .await;

    let metric = north_star::north_star(db, SINCE, UNTIL)
        .await
        .expect("north star");

    assert_eq!(
        metric.loved_works, 1,
        "a loved work with no slot row is still a loved work"
    );
    assert_eq!(
        metric.unattributed, 1,
        "and it is attributed to `unattributed`, NOT filtered out of the denominator"
    );
    assert_eq!(
        metric.median_days_to_find, None,
        "never served here means there is no gap to measure, not a gap of 0"
    );
    assert!(
        metric.shares_account_for_everything(),
        "the breakdown plus unattributed must equal the total"
    );
    // `unattributed` is a top-level count, NOT a row in `by_mechanism`. It is reported
    // rather than omitted -- that is the headline property -- but folding it into the
    // breakdown as well would double-count it and make its own invariant fail.
    assert!(
        !metric
            .by_mechanism
            .iter()
            .any(|m| m.key == lorehaven_domain::north_star::NorthStar::UNATTRIBUTED),
        "`unattributed` belongs in the top-level field, not in by_mechanism: {:?}",
        metric.by_mechanism
    );
    assert_eq!(
        metric.unattributed as f64 / metric.loved_works as f64,
        1.0,
        "and its share of the total is 1.0"
    );
}

/// Cases 2 and 5: the `>= 4` boundary, and a 3-star rating is not a loved work.
#[tokio::test]
async fn three_stars_is_not_a_loved_work_and_four_is() {
    let (w, s) = seed_base("ns-stars").await;
    let account = w.account.clone();
    let db = w._db.db();

    // One three-star and one four-star rating on two different works.
    let three = Seed::new_work(db, &s.pseud, "ns-three-star-work").await;
    let four = Seed::new_work(db, &s.pseud, "ns-four-star-work").await;
    for (work, stars) in [(&three, 3i64), (&four, 4i64)] {
        s.rating(db, &account, work, stars, "2026-06-01T00:00:00Z")
            .await;
    }

    let metric = north_star::north_star(db, SINCE, UNTIL)
        .await
        .expect("north star");
    assert_eq!(
        metric.loved_works, 1,
        "only the 4-star rating is a loved work; 3 stars is below the boundary"
    );
}

/// Case 3: a slot served AFTER the rating cannot have caused it.
///
/// This is the case that separates attribution from adjacency, and the only one that
/// catches an implementation which ignores ordering.
#[tokio::test]
async fn a_slot_recorded_after_the_rating_claims_nothing() {
    let (w, s) = seed_base("ns-after").await;
    let account = w.account.clone();
    let db = w._db.db();

    // Rated in June, served in August: the serving is later, so it did not surface the
    // work to this reader.
    let work = Seed::new_work(db, &s.pseud, "ns-after-work").await;
    s.rating(db, &account, &work, 4, "2026-06-01T00:00:00Z")
        .await;
    s.slot_at(
        db,
        &work,
        &s.pseud,
        "2026-08-01T00:00:00Z",
        "discovery_feed",
    )
    .await;

    let metric = north_star::north_star(db, SINCE, UNTIL)
        .await
        .expect("north star");
    assert_eq!(
        metric.unattributed, 1,
        "a slot served two months after the rating cannot have surfaced it"
    );
    assert!(
        !metric
            .by_mechanism
            .iter()
            .any(|m| m.key == "discovery_feed"),
        "so `discovery_feed` must not appear in the breakdown at all: {:?}",
        metric.by_mechanism
    );
}

/// Case 6: §53.6's denominator is `finished` alone, so a 4-star rating with no
/// completion does not earn it.
///
/// Reusing §53.5's "finished or rated >= 4" is the obvious implementation and the spec
/// explicitly forbids it: it puts the cheap signal (one click) inside the expensive one
/// (a reader's time). Case 5 above alone cannot catch that, because it never sets up a
/// rating without a completion.
#[tokio::test]
async fn a_four_star_rating_with_no_completion_is_still_a_loved_work() {
    let (w, s) = seed_base("ns-finished-only").await;
    let account = w.account.clone();
    let db = w._db.db();

    // Loved by rating, never finished. §53.5's definition counts this; §53.6's does not.
    let work = Seed::new_work(db, &s.pseud, "ns-rating-only-work").await;
    s.rating(db, &account, &work, 4, "2026-06-01T00:00:00Z")
        .await;

    let without_completion = north_star::north_star(db, SINCE, UNTIL)
        .await
        .expect("north star");
    assert_eq!(
        without_completion.loved_works, 1,
        "§53.5's love definition is rating OR finished"
    );
    // §53.6's denominator is completions alone, and this is the assertion that makes that
    // visible: `completions` is the input that is reported missing here.
    let missing: Vec<&str> = without_completion
        .missing_inputs
        .iter()
        .map(|m| m.as_str())
        .collect();
    assert!(
        missing.contains(&"completions"),
        "a 4-star rating with no completion must NOT satisfy §53.6's denominator: {missing:?}"
    );

    // And the other side of the same distinction: now finish a SECOND work. It is loved
    // by completion alone -- no rating at all -- and it satisfies the denominator.
    let completed = Seed::new_work(db, &s.pseud, "ns-completed-only-work").await;
    s.finish(db, &account, &completed, "2026-06-02T00:00:00Z")
        .await;

    let with_completion = north_star::north_star(db, SINCE, UNTIL)
        .await
        .expect("north star");
    assert_eq!(
        with_completion.loved_works, 1,
        "the rated work is still the only loved work -- §53.5 counts ratings, and a \
         completion for a DIFFERENT work does not become a loved work under it"
    );
    let missing: Vec<&str> = with_completion
        .missing_inputs
        .iter()
        .map(|m| m.as_str())
        .collect();
    assert!(
        !missing.contains(&"completions"),
        "a completion in the window must satisfy §53.6's denominator: {missing:?}"
    );
}

/// Case 7: the shares account for every loved work, across mechanisms plus unattributed.
#[tokio::test]
async fn shares_account_for_every_loved_work() {
    let (w, s) = seed_base("ns-shares").await;
    let account = w.account.clone();
    let db = w._db.db();

    // Three loved works: two served by the feed before the rating, one never served.
    for i in 0..3 {
        let work = Seed::new_work(db, &s.pseud, &format!("ns-share-work-{i}")).await;
        s.rating(db, &account, &work, 5, "2026-06-01T00:00:00Z")
            .await;
        if i < 2 {
            s.slot_at(
                db,
                &work,
                &s.pseud,
                "2026-05-01T00:00:00Z",
                "discovery_feed",
            )
            .await;
        }
    }

    let metric = north_star::north_star(db, SINCE, UNTIL)
        .await
        .expect("north star");
    assert_eq!(metric.loved_works, 3);
    assert!(
        metric.shares_account_for_everything(),
        "mechanisms + unattributed must equal loved_works: {:?}",
        metric.by_mechanism
    );
    // Mechanisms PLUS unattributed sum to 1.0 -- not the mechanisms alone, which is what
    // the first version of this asserted and got 2/3 for a metric that was correct.
    // The share field is per-mechanism; the remainder is the `unattributed` count.
    let total: f64 = metric.by_mechanism.iter().map(|m| m.share).sum();
    let unattributed_share = metric.unattributed as f64 / metric.loved_works as f64;
    assert!(
        (total + unattributed_share - 1.0).abs() < 1e-9,
        "mechanisms ({total}) plus unattributed ({unattributed_share}) must sum to 1.0"
    );
    let feed = metric
        .by_mechanism
        .iter()
        .find(|m| m.key == "discovery_feed")
        .expect("discovery_feed must appear");
    assert_eq!(feed.loved_works, 2);
    assert!((feed.share - 2.0 / 3.0).abs() < 1e-9);
}

/// §53.2 at the type boundary: an operator metric carries no per-account detail.
#[tokio::test]
async fn the_metric_carries_no_per_account_detail() {
    let (w, _s) = seed_base("ns-no-detail").await;
    let metric = north_star::north_star(w._db.db(), SINCE, UNTIL)
        .await
        .expect("north star");
    assert!(
        !metric.carries_account_detail(),
        "§53.2: an operator metrics view is instance-level aggregate only"
    );
}
