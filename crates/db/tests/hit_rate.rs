//! §53.5 — hit rate over real data, green on SQLite and PostgreSQL.
//!
//! The unit tests in `crates/db/src/hit_rate.rs` check the arithmetic and the two
//! dialects' bounds. This file checks the part that only a database can answer:
//! that the three tables join the way the metric assumes, that a shown-but-ignored
//! work is a *miss*, and that the undefined case is distinct from zero.
//!
//! `pseuds.account_id` is the join that earns its keep. `rating` and
//! `recommendation_slots` both key on `pseud_id`, but `reading_status` keys on
//! `account_id` — so the completion branch reads through `pseuds`, and a wrong
//! direction there yields a hit rate of zero that looks like a working query
//! returning bad news.

use lorehaven_db::Database;
use std::time::Duration;

const T0: i64 = 1_767_225_600; // 2026-01-01
const DAY: i64 = 86_400;

fn make_config(url: String) -> lorehaven_db::DatabaseConfig {
    lorehaven_db::DatabaseConfig {
        url,
        max_connections: 5,
        acquire_timeout: Duration::from_secs(10),
        slow_query_warn: Duration::ZERO,
    }
}

/// A scratch database on whichever backend the selector names.
///
/// `LOREHAVEN_TEST_PG_URL` is honoured so this file runs on both engines without
/// being edited, matching the convention the app crate's suites use. The db crate
/// cannot depend on `test_support` — `test_support` depends on it — so the setup
/// is written out here rather than shared.
async fn connect(tag: &str) -> Database {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "lorehaven-hit-{}-{}-{}",
        tag,
        uuid::Uuid::new_v4(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let url = match std::env::var("LOREHAVEN_TEST_PG_URL") {
        Ok(u) => u,
        Err(_) => format!("sqlite://{}/lorehaven.sqlite?mode=rwc", dir.display()),
    };
    let db = Database::connect(&make_config(url)).await.expect("connect");
    db.migrate().await.expect("migrate");
    db
}

/// A reader with a pseud, an account, and `n` works shown in the window.
struct Fixture {
    tdb: Database,
    pseud: String,
    account: String,
    works: Vec<String>,
}

impl Fixture {
    async fn build(tag: &str, works: usize) -> Self {
        let db = connect(tag).await;

        // `accounts_email_normalized` is UNIQUE, and on PostgreSQL the database is
        // shared across runs rather than per-suite, so a fixed address collides the
        // second time this file runs. The suffix is per-run, not per-test.
        let run = uuid::Uuid::new_v4().to_string();
        let account = uuid::Uuid::new_v4().to_string();
        let pseud = uuid::Uuid::new_v4().to_string();
        insert(&db, "INSERT INTO accounts (id, email, created_at, updated_at) \
                     VALUES (?1#u, ?2, '2026-01-01T00:00:00Z'::timestamptz, '2026-01-01T00:00:00Z')",
                &[&account, &format!("{tag}-{run}@example.com")]).await;
        insert(&db, "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, \
                          updated_at) \
                     VALUES (?1#u, ?2#u, ?3, ?3, '2026-01-01T00:00:00Z'::timestamptz, '2026-01-01T00:00:00Z')",
                &[&pseud, &account, &format!("p{tag}-{run}")]).await;

        let mut ids = Vec::with_capacity(works);
        for i in 0..works {
            let w = uuid::Uuid::new_v4().to_string();
            insert(
                &db,
                "INSERT INTO works (id, owner_pseud_id, title, created_at, \
                         updated_at, generated_content_posture) \
                         VALUES (?1#u, ?2#u, ?3, '2026-01-01T00:00:00Z'::timestamptz, \
                         '2026-01-01T00:00:00Z', 'forbid')",
                &[&w, &pseud, &format!("W{i}")],
            )
            .await;
            ids.push(w);
        }
        Self {
            tdb: db,
            pseud,
            account,
            works: ids,
        }
    }

    /// Serve `work` in the window. Served twice, deliberately: the metric counts a
    /// work once, so a re-served work must not move the denominator.
    async fn show(&self, work: &str, at: i64) {
        // Two request ids, and the first work is served under the first of them
        // TWICE. The metric counts a work once, so a re-served work must not move
        // the denominator -- this is the shape that catches a slot-counting query.
        let first = uuid::Uuid::new_v4().to_string();
        let second = uuid::Uuid::new_v4().to_string();
        let created = rfc3339(at);
        for request in [first.clone(), first, second] {
            let id = uuid::Uuid::new_v4().to_string();
            insert(
                &self.tdb,
                "INSERT INTO recommendation_slots (id, pseud_id, work_id, request_id, \
                     position, reasons, blend_score, created_at) \
                 VALUES (?1#u, ?2#u, ?3#u, ?4#u, 0, '[]', 0, ?5#t)",
                &[&id, &self.pseud, work, &request, &created],
            )
            .await;
        }
    }

    async fn rate(&self, work: &str, stars: i64) {
        let id = uuid::Uuid::new_v4().to_string();
        insert(&self.tdb,
            "INSERT INTO rating (id, account_id, pseud_id, work_id, stars, is_public, \
                 created_at, updated_at) \
             VALUES (?1#u, ?2#u, ?3#u, ?4#u, ?5#i, false, '2026-01-05T00:00:00Z'::timestamptz, '2026-01-05T00:00:00Z')",
            &[&id, &self.account, &self.pseud, &work.to_string(), &stars.to_string()]).await;
    }

    async fn finish(&self, work: &str) {
        let id = uuid::Uuid::new_v4().to_string();
        insert(
            &self.tdb,
            "INSERT INTO reading_status (id, account_id, subject_type, subject_id, \
                 status, updated_at) \
             VALUES (?1#u, ?2#u, 'work', ?3#u, 'finished', '2026-01-05T00:00:00Z')",
            &[&id, &self.account, &work.to_string()],
        )
        .await;
    }

    /// The window covering the whole fixture.
    async fn hit_rate(&self) -> lorehaven_db::hit_rate::HitRate {
        let pseud = uuid::Uuid::parse_str(&self.pseud).expect("a uuid");
        lorehaven_db::hit_rate::hit_rate(&self.tdb, pseud, T0, T0 + 30 * DAY)
            .await
            .expect("hit rate")
    }
}

/// RFC 3339 from unix seconds, in the shape `created_at` stores on SQLite.
///
/// Written out rather than pulled from `chrono`, which this crate does not depend
/// on. The window under test starts at a known midnight, so the civil-date maths
/// is a division and a modulus rather than a calendar lookup -- which keeps the
/// fixture free of a dependency the assertion does not need.
fn rfc3339(unix: i64) -> String {
    let days = unix.div_euclid(DAY);
    let secs = unix.rem_euclid(DAY);
    let (h, mi, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    // 2026-01-01 is day 20454 since the epoch; the fixture's window starts there, so
    // an offset in days is enough and no month arithmetic is required.
    let (y, m, d) = (
        2026 + (days - 20_454) / 365,
        1 + (days - 20_454) % 365 / 30,
        1 + (days - 20_454) % 30,
    );
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

async fn insert(db: &lorehaven_db::Database, sqlite: &str, args: &[&str]) {
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            let sqlite = &sqlite
                .replace("#u", "")
                .replace("#t", "")
                .replace("#i", "")
                .replace("::timestamptz", "")
                .replace(", false, ", ", 0, ");
            let mut q = sqlx::query(sqlite);
            for a in args {
                q = q.bind(*a);
            }
            q.execute(db.sqlite_pool().expect("sqlite pool"))
                .await
                .expect("insert");
        }
        lorehaven_db::Backend::Postgres => {
            // `id` is TEXT on SQLite and native `uuid` on PostgreSQL, so a text
            // bind is rejected with 42804. The cast in the SQL and the typed bind
            // below are the same fix as `::text` in a SELECT -- one arm per
            // dialect, because a fixture that only ran on SQLite would not have
            // found this.
            // `?N` -> `$N` and the query text as given. The uuid casts are applied
            // by the caller through the `{uuid}` placeholder: which slot is a uuid
            // depends on the statement (the pseuds insert binds id, account_id and
            // handle in slots 1-3), so a fixed positional rule casts the handle and
            // PostgreSQL rejects it with 22P02.
            let postgres = (1..=5).fold(sqlite.to_string(), |acc, n| {
                acc.replace(&format!("?{n}#u"), &format!("${n}::uuid"))
                    .replace(&format!("?{n}#t"), &format!("${n}::timestamptz"))
                    .replace(&format!("?{n}#i"), &format!("${n}::bigint"))
                    .replace(&format!("?{n}"), &format!("${n}"))
            });

            let mut q = sqlx::query(&postgres);
            for a in args {
                // Detected by content, not by position: the inserts in this file
                // bind uuid ids in slots 1, 3 and 4 depending on the statement, and
                // an index-based rule silently binds a handle as a uuid (or a work
                // id as text) in whichever statement it does not fit. `Uuid::parse_str`
                // succeeding IS the test -- every non-uuid value here is an email, a
                // handle, a timestamp or a star count, none of which parse.
                match uuid::Uuid::parse_str(a) {
                    Ok(u) => q = q.bind(u),
                    Err(_) => match a.parse::<i64>() {
                        Ok(n) => q = q.bind(n),
                        Err(_) => q = q.bind(*a),
                    },
                };
            }
            q.execute(db.postgres_pool().expect("postgres pool"))
                .await
                .expect("insert");
        }
    }
}

// ── the tests ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_work_shown_and_rated_four_is_a_hit() {
    let f = Fixture::build("hr-star4", 1).await;
    f.show(&f.works[0], T0).await;
    f.rate(&f.works[0], 4).await;
    let h = f.hit_rate().await;
    assert_eq!(h.shown, 1);
    assert_eq!(h.hits, 1);
    assert_eq!(h.rate, Some(1.0));
}

#[tokio::test]
async fn three_stars_is_not_a_hit() {
    // §53.5 says "≥4", so the boundary has to be a test rather than a reading of
    // the query. Three is the case a future edit to `HIT_STARS` would break.
    let f = Fixture::build("hr-star3", 1).await;
    f.show(&f.works[0], T0).await;
    f.rate(&f.works[0], 3).await;
    let h = f.hit_rate().await;
    assert_eq!(h.shown, 1);
    assert_eq!(h.hits, 0, "three stars is below the threshold");
    assert_eq!(h.rate, Some(0.0));
}

#[tokio::test]
async fn finishing_counts_as_a_hit_too() {
    let f = Fixture::build("hr-finish", 1).await;
    f.show(&f.works[0], T0).await;
    f.finish(&f.works[0]).await;
    let h = f.hit_rate().await;
    assert_eq!(
        h.hits, 1,
        "the reading_status branch joins through pseuds.account_id"
    );
}

#[tokio::test]
async fn a_shown_but_ignored_work_is_a_miss() {
    // The decision that matters most. Counting only acted-on works would let the
    // metric improve by showing the operator fewer things, which is the failure a
    // ranking metric must not have.
    let f = Fixture::build("hr-miss", 2).await;
    f.show(&f.works[0], T0).await;
    f.rate(&f.works[0], 5).await;
    f.show(&f.works[1], T0).await;
    // f.works[1] is never rated and never finished.
    let h = f.hit_rate().await;
    assert_eq!(h.shown, 2);
    assert_eq!(h.hits, 1);
    assert_eq!(h.rate, Some(0.5), "the ignored work is in the denominator");
}

#[tokio::test]
async fn a_work_served_three_times_counts_once() {
    // `Fixture::show` deliberately inserts the same work across two request_ids.
    // Counting slots would make this 3 shown instead of 1 — measuring how often the
    // operator scrolled back rather than how good the recipe is.
    let f = Fixture::build("hr-dedupe", 1).await;
    f.show(&f.works[0], T0).await;
    f.rate(&f.works[0], 4).await;
    let h = f.hit_rate().await;
    assert_eq!(h.shown, 1, "three slots, one work");
    assert_eq!(h.hits, 1);
    assert_eq!(h.rate, Some(1.0));
}

#[tokio::test]
async fn a_rating_on_a_work_that_was_never_shown_is_not_a_hit() {
    // The other half of §53.5: "of the works the operator was *shown*". Without
    // this the metric would credit the recipe for things it never surfaced.
    let f = Fixture::build("hr-unshown", 2).await;
    f.show(&f.works[0], T0).await;
    f.rate(&f.works[0], 5).await;
    f.rate(&f.works[1], 5).await;
    let h = f.hit_rate().await;
    assert_eq!(h.shown, 1, "an unshown work is not in the denominator");
    assert_eq!(h.hits, 1);
}

#[tokio::test]
async fn an_empty_window_is_undefined_not_zero() {
    let f = Fixture::build("hr-empty", 0).await;
    let h = f.hit_rate().await;
    assert_eq!(h.shown, 0);
    assert_eq!(h.rate, None, "no impressions is not total failure");
}

#[tokio::test]
async fn a_window_excludes_impressions_outside_it() {
    // Both bounds, because a query with only a lower bound is the classic version
    // of this: it silently grows without end.
    let f = Fixture::build("hr-window", 2).await;
    f.show(&f.works[0], T0).await;
    f.rate(&f.works[0], 5).await;
    // Shown 60 days later — outside the 30-day window.
    f.show(&f.works[1], T0 + 60 * DAY).await;
    f.rate(&f.works[1], 5).await;

    let h = f.hit_rate().await;
    assert_eq!(h.shown, 1, "the later impression is outside the window");
    assert_eq!(h.hits, 1);

    // And it is inside a wider window, which proves the upper bound is the filter
    // rather than the fixture simply not having the row.
    let pseud = uuid::Uuid::parse_str(&f.pseud).expect("a uuid");
    let wide = lorehaven_db::hit_rate::hit_rate(&f.tdb, pseud, T0, T0 + 90 * DAY)
        .await
        .expect("hit rate");
    assert_eq!(wide.shown, 2, "a wider window sees both");
    assert_eq!(wide.rate, Some(1.0));
}

#[tokio::test]
async fn the_metric_is_scoped_to_one_pseud() {
    // Two readers, one who acted and one who did not. Without the scope the second
    // reader's signals would answer for the first.
    let f = Fixture::build("hr-scope", 1).await;
    let tag = "scope";
    f.show(&f.works[0], T0).await;
    f.rate(&f.works[0], 5).await;

    let run = uuid::Uuid::new_v4().to_string();
    let other = uuid::Uuid::new_v4().to_string();
    let other_account = uuid::Uuid::new_v4().to_string();
    insert(
        &f.tdb,
        "INSERT INTO accounts (id, email, created_at, updated_at) \
         VALUES (?1#u, ?2, '2026-01-01T00:00:00Z'::timestamptz, '2026-01-01T00:00:00Z')",
        &[&other_account, &format!("other-{tag}-{run}@example.com")],
    )
    .await;
    insert(
        &f.tdb,
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
         VALUES (?1#u, ?2#u, ?3, ?3, '2026-01-01T00:00:00Z'::timestamptz, '2026-01-01T00:00:00Z')",
        &[&other, &other_account, &format!("other{tag}-{run}")],
    )
    .await;

    let h = lorehaven_db::hit_rate::hit_rate(
        &f.tdb,
        uuid::Uuid::parse_str(&other).expect("a uuid"),
        T0,
        T0 + 30 * DAY,
    )
    .await
    .expect("hit rate");
    assert_eq!(
        h.shown, 0,
        "another reader's impressions are not this reader's"
    );
    assert_eq!(h.rate, None);
}

#[tokio::test]
async fn dropping_finished_at_from_reading_status_does_not_break_the_join() {
    // `reading_status.finished_at` is NULL for a status that was never finished,
    // and this query deliberately does NOT filter on it: a work finished in a
    // different window from the one it was shown in is still a hit for the window
    // it was shown in. This test pins that, because the obvious optimisation is to
    // add `finished_at` to the WHERE clause and it would be wrong.
    let f = Fixture::build("hr-noft", 1).await;
    f.show(&f.works[0], T0).await;
    // `Fixture::finish` leaves finished_at NULL.
    f.finish(&f.works[0]).await;
    let h = f.hit_rate().await;
    assert_eq!(
        h.hits, 1,
        "the completion is counted regardless of finished_at"
    );
}
