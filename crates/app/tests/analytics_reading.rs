//! §9.6 reading stats, the first capability with a real query behind it.
//!
//! The registry lists 57 capabilities and the single-metric route answered
//! `implemented: false` for every one of them, so the dashboard was a list of
//! definitions rather than numbers. `own.reading.basic` is the first one to
//! answer with a query, and §9.6 is what it answers.
//!
//! # What §9.6 asks for, and the two things it forbids
//!
//! It asks for "works marked finished, chapters read, estimated words read,
//! approximate reading time, optional personal streak". Two constraints in the
//! same paragraph decide the shape of the query:
//!
//! * **"Do not count opens as proof of reading."** So nothing here is derived
//!   from `reading_history_entry` or from `reading_progress.created_at` -- both
//!   are written by an *open*. Finished works come from `reading_status`,
//!   which records a decision the reader made, and chapters come from progress
//!   rows that were advanced.
//! * **"Label estimates as such."** Reading time is an estimate, so the payload
//!   carries the method's `approximation` alongside the number, and this test
//!   asserts the cap is disclosed rather than trusting it.
//!
//! # The number that is not obvious
//!
//! `Method` for this capability says session length is "wall-clock between two
//! progress updates on the same chapter, capped at 30 minutes per gap". That is
//! a cap on a *gap*, so a reader who left a tab open overnight contributes 30
//! minutes, not eight hours. A test that seeds two updates 20 hours apart and
//! expects 72000 seconds is testing arithmetic; one that expects 1800 is
//! testing the cap, which is the part a reader can actually feel.

use axum::http::StatusCode;
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use serde_json::json;

/// Build a router on the open-library preset.
///
/// The preset matters: `development_defaults()` is `curated_boutique`, a
/// gallery, and a gallery withholds reader-facing capabilities. A test written
/// against the wrong preset is testing the gate, not the metric.
fn router_for(tdb: &test_support::TestDb, dir: &std::path::Path) -> axum::Router {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    config.instance.preset = "open_library".to_owned();
    server::build_router(AppState::new(config, tdb.db().clone()))
}

/// A registered reader, their account id, and a session, on a fresh database.
struct Reader {
    tdb: test_support::TestDb,
    account: String,
    client: test_support::TestClient,
}

/// A registered reader with a session, on a fresh scratch database.
///
/// The account id comes back from registration itself rather than from a second
/// query, so a fixture cannot disagree with the row the door actually wrote.
async fn reader(tag: &str) -> Reader {
    let dir = test_support::scratch_dir(tag);
    let tdb = test_support::TestDb::connect_with_dir(tag, &dir).await;
    let mut client = test_support::TestClient::new(router_for(&tdb, &dir));
    let account = test_support::register(&mut client, &format!("{tag}@test.dev"), tag).await;
    Reader {
        tdb,
        account,
        client,
    }
}

/// GET one capability for the signed-in reader.
async fn get(client: &mut test_support::TestClient, capability: &str) -> serde_json::Value {
    let (status, body) = client
        .get(format!("/api/v1/me/analytics/{capability}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{capability}: {body}");
    body
}

/// The `value` object, where a real number replaces `not_implemented`.
fn value(body: &serde_json::Value) -> &serde_json::Value {
    &body["value"]
}

// -- the metric answers at all ------------------------------------------------

/// The capability the registry has always listed now answers with numbers.
///
/// This is the assertion that fails against the tree as it stands, because the
/// route hardcodes `implemented: false` for all 57 scopes. A test written only
/// against the numbers would be vacuous while the query returned zero rows.
#[tokio::test]
async fn own_reading_basic_answers_with_a_query_rather_than_a_definition() {
    let Reader {
        tdb, mut client, ..
    } = reader("readbasic").await;

    let body = get(&mut client, "own.reading.basic").await;

    assert_eq!(
        body["implemented"],
        json!(true),
        "the capability claims no query behind it: {body}"
    );
    assert_ne!(
        value(&body)["status"],
        json!("not_implemented"),
        "the value is still a placeholder: {body}"
    );
    tdb.cleanup().await;
}

/// A reader who has read nothing gets zeros, not an error and not a null.
///
/// The floor does not apply -- these count the viewer's *own* behaviour, so a
/// true zero is a true fact. A null would be a client bug waiting to happen:
/// `unwrap_or(0)` is how a suppressed count becomes a claim that nobody did
/// anything.
#[tokio::test]
async fn a_reader_who_has_read_nothing_gets_zeros_rather_than_a_null() {
    let Reader {
        tdb, mut client, ..
    } = reader("readzero").await;

    let body = get(&mut client, "own.reading.basic").await;

    assert_eq!(value(&body)["finished_works"], json!(0), "{body}");
    assert_eq!(value(&body)["chapters_read"], json!(0), "{body}");
    assert_eq!(value(&body)["words_read"], json!(0), "{body}");
    assert_eq!(value(&body)["reading_seconds"], json!(0), "{body}");
    tdb.cleanup().await;
}

// -- the numbers are the ones the spec means ----------------------------------

/// A finished work counts once; a work merely opened does not count at all.
///
/// This is §9.6's "do not count opens as proof of reading", stated as three
/// rows in one test. If the query ever reaches for `reading_history_entry`, or
/// counts `reading_status` rows without filtering on `status = 'finished'`,
/// this is the assertion that catches it.
#[tokio::test]
async fn a_finished_work_counts_and_a_work_left_open_does_not() {
    let Reader {
        tdb,
        account,
        mut client,
    } = reader("readfin").await;

    seed_status(tdb.db(), &account, 1, "finished").await;
    seed_status(tdb.db(), &account, 2, "reading").await;
    seed_status(tdb.db(), &account, 3, "dropped").await;

    let body = get(&mut client, "own.reading.basic").await;

    assert_eq!(
        value(&body)["finished_works"],
        json!(1),
        "one of the three statuses is a finish; the other two are not: {body}"
    );
    tdb.cleanup().await;
}

/// A reader's own counts are exact, with no floor applied.
///
/// The floor is a rule about *other* people. `own.reading.basic` is the viewer's
/// own reading, so banding it would band a fact about the person asking, and
/// the payload would answer "fewer than 5" to somebody who knows they read
/// three works.
#[tokio::test]
async fn a_readers_own_counts_are_exact_and_carry_no_floor() {
    let Reader {
        tdb,
        account,
        mut client,
    } = reader("readflr").await;

    // Two finished works: below `K_SELF` (5), and reported as a number.
    seed_status(tdb.db(), &account, 1, "finished").await;
    seed_status(tdb.db(), &account, 2, "finished").await;

    let body = get(&mut client, "own.reading.basic").await;

    assert_eq!(value(&body)["finished_works"], json!(2), "{body}");
    assert!(
        value(&body).get("fewer_than").is_none(),
        "own counts are not banded: {body}"
    );
    tdb.cleanup().await;
}

/// One reader's reading is not another's.
///
/// The filter is in SQL, not in the route afterwards, because a post-filter has
/// already read the other reader's rows -- which is the leak the registry's
/// banding exists to prevent, and which an access log would record.
#[tokio::test]
async fn one_readers_reading_is_not_another_readers() {
    let dir = test_support::scratch_dir("readiso");
    let tdb = test_support::TestDb::connect_with_dir("readiso", &dir).await;
    let router = router_for(&tdb, &dir);

    let mut mine = test_support::TestClient::new(router.clone());
    let account_a = test_support::register(&mut mine, "mine@test.dev", "mine_reader").await;
    let mut theirs = test_support::TestClient::new(router);
    let account_b = test_support::register(&mut theirs, "theirs@test.dev", "theirs_rd").await;

    for n in 1..=7 {
        seed_status(tdb.db(), &account_a, n, "finished").await;
    }
    seed_status(tdb.db(), &account_b, 99, "finished").await;

    let a = get(&mut mine, "own.reading.basic").await;
    let b = get(&mut theirs, "own.reading.basic").await;

    assert_eq!(value(&a)["finished_works"], json!(7), "reader A: {a}");
    assert_eq!(value(&b)["finished_works"], json!(1), "reader B: {b}");
    tdb.cleanup().await;
}

// -- the estimate is labelled, and the cap is real ---------------------------

/// A long gap between progress updates is capped, and the payload says so.
///
/// Two updates on the same chapter 20 hours apart are 72000 seconds of
/// wall-clock, which is what "approximate reading time" must not report: the
/// documented cap is 30 minutes per gap, so 1800 is the honest answer.
#[tokio::test]
async fn a_long_gap_between_progress_updates_is_capped_and_disclosed() {
    let Reader {
        tdb,
        account,
        mut client,
    } = reader("readcap").await;

    seed_status(tdb.db(), &account, 1, "finished").await;
    seed_progress_gap(tdb.db(), &account, 20 * 3600).await;

    let body = get(&mut client, "own.reading.basic").await;

    assert_eq!(
        value(&body)["reading_seconds"],
        json!(1800),
        "a 20-hour gap is capped at 30 minutes: {body}"
    );
    let approximation = body["meta"]["approximation"].as_str().unwrap_or_default();
    assert!(
        approximation.contains("30"),
        "the cap the number was subject to is disclosed: {body}"
    );
    tdb.cleanup().await;
}

/// A gap shorter than the cap is reported as-is, so the cap is a ceiling and
/// not a constant.
///
/// Without this, the test above passes against a query that always answers
/// 1800 -- which is a number, not a measurement.
#[tokio::test]
async fn a_short_gap_is_reported_uncapped() {
    let Reader {
        tdb,
        account,
        mut client,
    } = reader("readshort").await;

    seed_status(tdb.db(), &account, 1, "finished").await;
    seed_progress_gap(tdb.db(), &account, 600).await;

    let body = get(&mut client, "own.reading.basic").await;

    assert_eq!(
        value(&body)["reading_seconds"],
        json!(600),
        "a 10-minute gap is under the cap, so it is reported whole: {body}"
    );
    tdb.cleanup().await;
}

// -- helpers ------------------------------------------------------------------

/// One `reading_status` row, with deterministic UUIDs per `(account index, n)`.
///
/// `subject_type` is `'work'`, matching the library door, and the ids are real
/// UUIDs because the PostgreSQL twin types `subject_id` as UUID: a name-shaped
/// id is valid TEXT on SQLite and rejected by PG, so such a fixture passes one
/// dialect and dies on the other.
async fn seed_status(db: &lorehaven_db::Database, account: &str, n: u32, status: &str) {
    let work = uuid_for('a', n);
    let row = uuid_for('e', n);
    // `?` on SQLite, `$n::uuid` on PostgreSQL: the twin types `id` and
    // `subject_id` as UUID, so a text bind needs the cast on the placeholder.
    // Never on the column -- that is the other bug.
    let finished = if status == "finished" {
        Some("2026-09-20T00:00:00Z")
    } else {
        None
    };
    let result = match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO reading_status
                     (id, account_id, subject_type, subject_id, status, started_at, finished_at, updated_at, version)
                 VALUES (?, ?, 'work', ?, ?, ?, ?, ?, 1)",
            )
            .bind(&row)
            .bind(account)
            .bind(&work)
            .bind(status)
            .bind("2026-09-01T00:00:00Z")
            .bind(finished)
            .bind("2026-09-20T00:00:00Z")
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await
            .map(|_| ())
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO reading_status
                     (id, account_id, subject_type, subject_id, status, started_at, finished_at, updated_at, version)
                 VALUES ($1::uuid, $2::uuid, 'work', $3::uuid, $4, $5, $6, $7, 1)",
            )
            .bind(&row)
            .bind(account)
            .bind(&work)
            .bind(status)
            .bind("2026-09-01T00:00:00Z")
            .bind(finished)
            .bind("2026-09-20T00:00:00Z")
            .execute(db.postgres_pool().expect("postgres handle"))
            .await
            .map(|_| ())
        }
    };
    result.expect("seed a reading status");
}

/// Two progress updates on one chapter, `gap_seconds` apart.
///
/// The `created_at` stamps are the two ends of the gap, and `chapter_id` is
/// NULL so the two rows are "the same chapter" in the sense the method means --
/// the same work, and therefore the same reading session.
async fn seed_progress_gap(db: &lorehaven_db::Database, account: &str, gap_seconds: i64) {
    // A fixed base instant, so the test does not depend on the wall clock.
    let base = 1_772_000_000;
    for n in 0..2u32 {
        let stamp = rfc3339(base + i64::from(n) * gap_seconds);
        let row = uuid_for('f', n);
        let work = uuid_for('a', 1);
        let result = match db.backend() {
            lorehaven_db::Backend::Sqlite => {
                sqlx::query(
                    "INSERT INTO reading_progress
                         (id, account_id, subject_type, subject_id, chapter_id, position_permille, created_at, updated_at, version)
                     VALUES (?, ?, 'work', ?, NULL, 0, ?, ?, 1)",
                )
                .bind(&row)
                .bind(account)
                .bind(&work)
                .bind(&stamp)
                .bind(&stamp)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await
                .map(|_| ())
            }
            lorehaven_db::Backend::Postgres => {
                sqlx::query(
                    "INSERT INTO reading_progress
                         (id, account_id, subject_type, subject_id, chapter_id, position_permille, created_at, updated_at, version)
                     VALUES ($1::uuid, $2::uuid, 'work', $3::uuid, NULL, 0, $4, $5, 1)",
                )
                .bind(&row)
                .bind(account)
                .bind(&work)
                .bind(&stamp)
                .bind(&stamp)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await
                .map(|_| ())
            }
        };
        result.expect("seed a progress row");
    }
}

/// A deterministic UUID from one letter and a number.
///
/// Fixed rather than random so a failing test names the same rows on a rerun,
/// and a real UUID rather than a name because PostgreSQL rejects the latter.
fn uuid_for(letter: char, n: u32) -> String {
    format!(
        "{letter}{letter}{letter}{letter}{letter}{letter}{letter}{letter}-0000-4000-8000-{n:012}"
    )
}

/// A Unix timestamp as RFC 3339, without pulling in a date library.
///
/// House convention: both dialects store timestamps as RFC 3339 TEXT, so the
/// same string is storable either way and no `::timestamptz` cast appears in
/// the query.
fn rfc3339(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Howard Hinnant's `civil_from_days`, days since 1970-01-01 to (y, m, d).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
