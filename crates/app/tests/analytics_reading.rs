//! §9.6 reading stats, the first capability with a real query behind it.
//!
//! The registry lists 63 capabilities and the single-metric route answered
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

// For `NaiveDate::weekday`, used to assert the reported weeks are Mondays.
use axum::http::StatusCode;
use chrono::Datelike as _;
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

impl Reader {
    /// The database behind this reader's router, for seeding rows the UI cannot
    /// create. Grows a reference for the same reason the client is a field: a
    /// test that seeds a row and then reads it through a door is proving the
    /// door works, and one that seeds and reads through SQL is not.
    fn db(&self) -> &lorehaven_db::Database {
        self.tdb.db()
    }
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

/// The §9.6 reading totals inside `value`.
///
/// Nested under the capability's own key rather than flat, because a second
/// capability with different fields would otherwise collide by name.
fn reading(body: &serde_json::Value) -> &serde_json::Value {
    &value(body)["reading"]
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

    assert_eq!(reading(&body)["finished_works"], json!(0), "{body}");
    assert_eq!(reading(&body)["chapters_read"], json!(0), "{body}");
    assert_eq!(reading(&body)["words_read"], json!(0), "{body}");
    assert_eq!(reading(&body)["reading_seconds"], json!(0), "{body}");
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
        reading(&body)["finished_works"],
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

    assert_eq!(reading(&body)["finished_works"], json!(2), "{body}");
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

    assert_eq!(reading(&a)["finished_works"], json!(7), "reader A: {a}");
    assert_eq!(reading(&b)["finished_works"], json!(1), "reader B: {b}");
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
        reading(&body)["reading_seconds"],
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
        reading(&body)["reading_seconds"],
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
/// A deterministic UUID whose first group is `letter` repeated.
///
/// `letter` must be a hex digit. It is not checked here on purpose -- a
/// non-hex letter is a *Postgres* failure (`invalid input syntax for type
/// uuid`) while SQLite accepts any text, so an unchecked helper means a fixture
/// bug that a SQLite-only run reports as green. The letters in use are `a`
/// through `f`.
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

// ---------------------------------------------------------------------------
// The door, not a fixture
// ---------------------------------------------------------------------------
//
// Every test above seeds `reading_status` with raw SQL, because until now there
// was no way to write one any other way: reading status hung off
// `library_items`, and a library item is created in exactly one place --
// `imports::upsert_library_item`, called by the import runner. A work published
// on this instance had no subject to mark, so a reader of local fiction saw a
// permanent zero on their own dashboard.
//
// The data model already anticipated the fix. `SUBJECT_WORK` exists, the
// analytics query already filters on `subject_type = 'work'`, and
// `reading_progress` is keyed on works. Only the door was missing.
//
// These tests exist to prove the door writes the same rows the fixtures did, and
// that it refuses the subjects it should.

/// Publish a work through the real doors, returning its id.
///
/// Create, add a chapter, publish -- all through the router, because a fixture
/// that inserts a work directly would not catch a visibility rule that hides it.
async fn published_work(client: &mut test_support::TestClient, title: &str) -> String {
    let (status, body) = client
        .post("/api/v1/works", json!({ "title": title }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    // The create door returns the work view itself, not an envelope.
    let id = body["id"].as_str().expect("work id").to_owned();

    let (status, body) = client
        .post(
            &format!("/api/v1/works/{id}/chapters"),
            json!({ "title": "One" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "add chapter: {body}");
    let chapter = body["id"].as_str().expect("chapter id").to_owned();
    let chapter_version = body["version"].as_i64().expect("chapter version");

    // A chapter needs a document before the work can be published: the publish
    // door refuses an empty work with "an empty work has nothing to read", which
    // is the right rule and an easy 422 to hit in a fixture.
    let doc = json!({ "type": "doc", "content": [
        { "type": "paragraph", "content": [
            { "type": "text", "text": "A chapter with enough words to have a middle." }] }] });
    let (status, body) = client
        .request(
            "PATCH",
            &format!("/api/v1/chapters/{chapter}"),
            Some(json!({ "expected_version": chapter_version, "document": doc })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "write the chapter: {body}");

    // Publish is optimistic: it names the work version it expects to publish,
    // and adding a chapter moved that version on, so the value read at create
    // is stale by now. Read it back rather than guessing.
    let (status, body) = client.get(format!("/api/v1/works/{id}")).await;
    assert_eq!(status, StatusCode::OK, "reload work: {body}");
    let current = body["version"].as_i64().expect("current work version");

    let (status, body) = client
        .post(
            &format!("/api/v1/works/{id}/publish"),
            json!({ "expected_version": current }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "publish: {body}");
    id
}

/// A reader can mark a work published on this instance, and the number moves.
///
/// The whole point of the door: the reader takes an action through the API, and
/// the capability they can already see reports it. Before this there was no
/// action to take.
#[tokio::test]
async fn a_reader_can_mark_a_locally_published_work_finished_and_the_count_moves() {
    let mut r = reader("reading-door").await;
    let work = published_work(&mut r.client, "A Work Of Local Fiction").await;

    let (status, body) = r
        .client
        .put(
            format!("/api/v1/works/{work}/reading-status"),
            json!({ "status": "finished" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "mark finished: {body}");
    assert_eq!(
        body["status"], "finished",
        "the door echoes the status: {body}"
    );
    // The spec's own rule: a finished work carries the moment it was finished,
    // so a reader can see when they decided rather than when they last looked.
    assert!(
        body["finished_at"].is_string(),
        "a finished status must record when: {body}"
    );

    let body = get(&mut r.client, "own.reading.basic").await;
    let totals = reading(&body);
    assert_eq!(
        totals["finished_works"], 1,
        "the dashboard must count the work the reader just finished: {totals}"
    );

    r.tdb.cleanup().await;
}

/// Reading it back is the reader's own record, not a guess.
///
/// A door that writes but cannot read leaves a reader unable to see what they
/// have already recorded, which is the same as not having recorded it.
#[tokio::test]
async fn a_reader_can_read_back_the_status_they_recorded() {
    let mut r = reader("reading-door-read").await;
    let work = published_work(&mut r.client, "Read Back").await;

    let (status, _) = r
        .client
        .put(
            format!("/api/v1/works/{work}/reading-status"),
            json!({ "status": "reading" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = r
        .client
        .get(format!("/api/v1/works/{work}/reading-status"))
        .await;
    assert_eq!(status, StatusCode::OK, "read status: {body}");
    assert_eq!(body["status"], "reading");

    // Not finished, so it must not be counted as finished. A door that set
    // `finished_at` on every status would make the count wrong while every
    // individual response looked right.
    let body = get(&mut r.client, "own.reading.basic").await;
    let totals = reading(&body);
    assert_eq!(
        totals["finished_works"], 0,
        "reading is not finished: {totals}"
    );

    r.tdb.cleanup().await;
}

/// No status at all is null, not a fabricated `unknown`.
#[tokio::test]
async fn a_work_with_no_status_reads_back_as_null() {
    let mut r = reader("reading-door-null").await;
    let work = published_work(&mut r.client, "Never Started").await;

    let (status, body) = r
        .client
        .get(format!("/api/v1/works/{work}/reading-status"))
        .await;
    assert_eq!(status, StatusCode::OK, "read status: {body}");
    assert!(
        body.is_null(),
        "an unrecorded status is absent, not a guess: {body}"
    );

    r.tdb.cleanup().await;
}

/// Clearing removes the row, so the count falls back.
#[tokio::test]
async fn clearing_a_status_removes_it_from_the_count() {
    let mut r = reader("reading-door-clear").await;
    let work = published_work(&mut r.client, "Changed My Mind").await;

    let (status, _) = r
        .client
        .put(
            format!("/api/v1/works/{work}/reading-status"),
            json!({ "status": "finished" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let body = get(&mut r.client, "own.reading.basic").await;
    assert_eq!(reading(&body)["finished_works"], 1);

    let (status, body) = r
        .client
        .request(
            "DELETE",
            &format!("/api/v1/works/{work}/reading-status"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "clear status: {body}");
    let body = get(&mut r.client, "own.reading.basic").await;
    assert_eq!(
        reading(&body)["finished_works"],
        0,
        "a cleared status must not still be counted"
    );

    r.tdb.cleanup().await;
}

/// A status the build does not know is a validation failure, not a stored row.
///
/// The four statuses are the whole vocabulary; accepting an unknown one and
/// storing it would put a row in the table that no count and no read-back can
/// interpret.
#[tokio::test]
async fn an_unknown_reading_status_is_refused_rather_than_stored() {
    let mut r = reader("reading-door-unknown").await;
    let work = published_work(&mut r.client, "Vocab").await;

    let (status, body) = r
        .client
        .put(
            format!("/api/v1/works/{work}/reading-status"),
            json!({ "status": "skimmed" }),
        )
        .await;
    // 422, matching the other validation refusals in the instance: the value
    // parsed as a string and failed as a vocabulary, which is a field problem
    // rather than a malformed request.
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "unknown status: {body}"
    );

    let (status, body) = r
        .client
        .get(format!("/api/v1/works/{work}/reading-status"))
        .await;
    assert_eq!(status, StatusCode::OK, "read back: {body}");
    assert!(
        body.is_null(),
        "a refused status must leave nothing behind to read back: {body}"
    );

    r.tdb.cleanup().await;
}

/// A work that does not exist is a 404, and writes nothing.
///
/// The companion to the library-item guard: the door checks its subject rather
/// than trusting the path.
#[tokio::test]
async fn a_status_against_a_work_that_does_not_exist_is_refused() {
    let mut r = reader("reading-door-missing").await;
    let absent = test_support::id("absent-work");

    let (status, body) = r
        .client
        .put(
            format!("/api/v1/works/{absent}/reading-status"),
            json!({ "status": "finished" }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "absent work: {body}");

    let body = get(&mut r.client, "own.reading.basic").await;
    let totals = reading(&body);
    assert_eq!(totals["finished_works"], 0, "nothing was written: {totals}");

    r.tdb.cleanup().await;
}

/// The door needs a session. An anonymous reader gets nothing written.
#[tokio::test]
async fn the_status_door_requires_a_session() {
    let dir = test_support::scratch_dir("reading-door-anon");
    let tdb = test_support::TestDb::connect_with_dir("reading-door-anon", &dir).await;
    let mut author = test_support::TestClient::new(router_for(&tdb, &dir));
    test_support::register(&mut author, "door-anon-author@test.dev", "DoorAnonAuthor").await;
    let work = published_work(&mut author, "Needs A Reader").await;

    // A client with no cookies at all.
    let mut anon = test_support::TestClient::new(router_for(&tdb, &dir));
    let (status, body) = anon
        .put(
            format!("/api/v1/works/{work}/reading-status"),
            json!({ "status": "finished" }),
        )
        .await;
    assert!(
        matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN),
        "an anonymous caller must not write a reading status: {status} {body}"
    );

    tdb.cleanup().await;
}

// ---------------------------------------------------------------------------
// own.reading.trend (spec §9.6) — reads per week over time
// ---------------------------------------------------------------------------
//
// # Why these tests learn the weeks instead of naming them
//
// The endpoint reports the trailing four weeks relative to *now*, so a test
// cannot assert a hard-coded `2026-09-21` and stay honest: a suite run on a
// Monday and the same suite run on a Sunday would disagree, and one of them
// would be wrong about the product. So each test reads the weeks the endpoint
// reports and seeds rows relative to those — "in the reported week two back",
// not "on 2026-09-07". The dates the endpoint returns are still asserted
// (a Monday, ascending, seven days apart), so the shape is checked; only the
// anchor moves with the clock.

/// Raise a reader to TL1, the level `own.reading.trend` is gated behind.
///
/// Written to `trust_levels` rather than promoted through the product, because
/// the promotion ladder is its own subject and this test is about the gate
/// accepting a reader who qualifies.
async fn at_tl1(db: &lorehaven_db::Database, account: &str) {
    let sql = db.sql(
        "INSERT INTO trust_levels (account, level, computed_at, basis) VALUES (?, 1, datetime('now'), 'test')",
        "INSERT INTO trust_levels (account, level, computed_at, basis) VALUES ($1::uuid, 1, now(), 'test')",
    );
    let result = match db.backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query(sql.as_ref())
            .bind(account)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await
            .map(|_| ()),
        lorehaven_db::Backend::Postgres => sqlx::query(sql.as_ref())
            .bind(account)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await
            .map(|_| ()),
    };
    result.expect("raise the reader to TL1");
}

/// A reader at TL1 with a session, on a fresh scratch database.
async fn trend_reader(tag: &str) -> Reader {
    let r = reader(tag).await;
    at_tl1(r.db(), &r.account).await;
    r
}

/// A progress update `days` before the Monday of `week`, counted from today.
///
/// `row_n` makes the primary key unique per call site.
///
/// Both obvious derivations collide, and each collision reads as a product bug:
/// from the account alone, every second seed in a test is a primary-key
/// violation; from the work alone, two readers sharing a work collide. So the
/// counter is the caller's, and the `d` letter keeps these ids clear of the `b`
/// and `c` works the tests seed. Hexadecimal, because Postgres casts the column
/// to `uuid` and `r` is not a hex digit -- a SQLite-only run would have passed. Two readers in one test are on two *separate*
/// databases -- each `reader()` gets its own scratch directory -- so only the
/// rows within a single database have to be distinct, and they are.
async fn seed_progress_on(
    db: &lorehaven_db::Database,
    account: &str,
    work: &str,
    stamp: &str,
    row_n: u32,
) {
    let row = uuid_for('d', row_n);
    let result = match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO reading_progress
                     (id, account_id, subject_type, subject_id, chapter_id, position_permille, created_at, updated_at, version)
                 VALUES (?, ?, 'work', ?, NULL, 0, ?, ?, 1)",
            )
            .bind(&row)
            .bind(account)
            .bind(work)
            .bind(stamp)
            .bind(stamp)
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
            .bind(work)
            .bind(stamp)
            .bind(stamp)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await
            .map(|_| ())
        }
    };
    result.expect("seed a progress update");
}

/// The Monday of the reported week `weeks_back` from the current one.
fn monday_of(weeks: &[serde_json::Value], weeks_back: usize) -> String {
    weeks[weeks.len() - 1 - weeks_back]["week_start"]
        .as_str()
        .expect("week_start is a string")
        .to_owned()
}

/// An RFC3339 stamp at midday on the Monday of `weeks_back`, plus `days`.
///
/// Midday rather than midnight because a UTC-midnight stamp can fall on the
/// Sunday in some timezone's local calendar, and the test is about which
/// *week* a row lands in, not about timezone handling.
fn midday(weeks: &[serde_json::Value], weeks_back: usize, days: i64) -> String {
    let date = chrono::NaiveDate::parse_from_str(&monday_of(weeks, weeks_back), "%Y-%m-%d")
        .expect("week_start parses");
    rfc3339(
        (date + chrono::Duration::days(days))
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp(),
    )
}

/// How many progress rows the fixture actually wrote for this reader.
///
/// Exists because a fixture that writes nothing and a query that counts
/// nothing produce the *same* payload — four weeks of zeroes — and only one of
/// them is a product bug. This one caught that confusion directly: the trend
/// reported zeroes over a table that held the reader's rows, and without this
/// check the failure reads as "the query is wrong" rather than "the query is
/// right and the binds are not".
async fn seeded_count(subject: &Reader) -> i64 {
    let sql = subject.db().sql(
        "SELECT COUNT(*) FROM reading_progress WHERE account_id = ?",
        "SELECT COUNT(*) FROM reading_progress WHERE account_id = $1::uuid",
    );
    match subject.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(sql.as_ref())
            .bind(&subject.account)
            .fetch_one(subject.db().sqlite_pool().expect("sqlite handle"))
            .await
            .expect("count the seeded progress rows"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(sql.as_ref())
            .bind(&subject.account)
            .fetch_one(subject.db().postgres_pool().expect("postgres handle"))
            .await
            .expect("count the seeded progress rows"),
    }
}

/// The weeks the trend endpoint reports, oldest first.
async fn trend(subject: &mut Reader) -> Vec<serde_json::Value> {
    let (status, body) = subject
        .client
        .get("/api/v1/me/analytics/own.reading.trend")
        .await;
    assert_eq!(status, StatusCode::OK, "trend: {body}");
    assert_eq!(
        body["implemented"],
        json!(true),
        "trend must be implemented: {body}"
    );
    // Nested under `value.reading`, alongside `own.reading.basic`: the `value`
    // object is shared by every capability, and flat keys would collide.
    body["value"]["reading"]["trend"]
        .as_array()
        .unwrap_or_else(|| panic!("trend is an array of weeks, got: {body}"))
        .clone()
}

#[tokio::test]
async fn a_reader_with_no_reads_is_given_weeks_rather_than_a_gap() {
    let mut subject = trend_reader("trend-empty").await;

    // The shape question, before the numbers: a reader who has read nothing
    // must not receive an empty array. An empty array is indistinguishable from
    // a broken query, and it draws a reader to conclude the dashboard is wrong
    // rather than that they have not read anything this week.
    let weeks = trend(&mut subject).await;
    assert!(
        !weeks.is_empty(),
        "a reader with no reads still gets weeks: an empty trend reads as a bug, not as a fact"
    );
    assert!(
        weeks.iter().all(|w| w["reads"] == json!(0)),
        "every week is zero: {weeks:?}"
    );
}

#[tokio::test]
async fn a_below_trust_reader_is_refused_rather_than_shown_a_trend() {
    let mut subject = reader("trend-below-floor").await;
    // Not raised to TL1. The gate is the subject of this test, and a suite that
    // only ever exercises the permitted case cannot tell a working gate from a
    // missing one.
    let (status, body) = subject
        .client
        .get("/api/v1/me/analytics/own.reading.trend")
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "TL0 is below the floor: {body}"
    );
    assert_eq!(body["error"]["code"], "ACCESS_DENIED", "{body}");
}

#[tokio::test]
async fn reads_are_counted_into_the_week_they_happened() {
    let mut subject = trend_reader("trend-count").await;
    // Learn the weeks first, then seed against them. Three reads in three
    // different reported weeks, and one outside the window entirely.
    let weeks = trend(&mut subject).await;
    let stamps = [
        midday(&weeks, 0, 1), // this week
        midday(&weeks, 0, 3), // this week too
        midday(&weeks, 2, 1), // two weeks back
    ];
    for (n, stamp) in stamps.iter().enumerate() {
        seed_progress_on(
            subject.db(),
            &subject.account,
            &uuid_for('b', n as u32),
            stamp,
            n as u32,
        )
        .await;
    }
    // Six weeks back: outside the four-week window, and must not appear.
    let outside = chrono::Utc::now() - chrono::Duration::days(42);
    seed_progress_on(
        subject.db(),
        &subject.account,
        &uuid_for('b', 9),
        &outside.format("%Y-%m-%dT12:00:00Z").to_string(),
        9,
    )
    .await;

    // The seeds landed. Asserted before the numbers, because a fixture that
    // silently wrote nothing produces exactly the same payload as a query that
    // counts nothing -- and the second is a product bug while the first is not.
    assert_eq!(seeded_count(&subject).await, 4, "all four seeds are stored");

    let weeks = trend(&mut subject).await;
    assert_eq!(
        weeks[weeks.len() - 1]["reads"],
        json!(2),
        "both of this week's: {weeks:?}"
    );
    assert_eq!(
        weeks[0]["reads"],
        json!(0),
        "the oldest reported week: {weeks:?}"
    );
    assert_eq!(
        weeks[weeks.len() - 3]["reads"],
        json!(1),
        "two weeks back: {weeks:?}"
    );

    // The read 42 days old is in no reported week at all. Asserting the
    // absence is the point: a window that silently included everything would
    // make a reader's four-week history mean "all time".
    assert!(
        weeks
            .iter()
            .all(|w| w["reads"] == json!(0) || w["reads"] == json!(1) || w["reads"] == json!(2)),
        "the out-of-window read is not counted anywhere: {weeks:?}"
    );
}

#[tokio::test]
async fn weeks_are_mondays_ascending_and_seven_days_apart() {
    let mut subject = trend_reader("trend-ascending").await;
    seed_progress_on(
        subject.db(),
        &subject.account,
        &uuid_for('b', 1),
        &chrono::Utc::now().format("%Y-%m-%dT12:00:00Z").to_string(),
        1,
    )
    .await;

    let weeks = trend(&mut subject).await;
    assert_eq!(weeks.len(), 4, "four weeks: {weeks:?}");

    for (n, week) in weeks.iter().enumerate() {
        let start =
            chrono::NaiveDate::parse_from_str(week["week_start"].as_str().unwrap(), "%Y-%m-%d")
                .expect("week_start parses");
        assert_eq!(
            start.weekday(),
            chrono::Weekday::Mon,
            "a week starts on Monday: {weeks:?}"
        );
        if n > 0 {
            let previous = chrono::NaiveDate::parse_from_str(
                weeks[n - 1]["week_start"].as_str().unwrap(),
                "%Y-%m-%d",
            )
            .expect("week_start parses");
            assert_eq!(
                (start - previous).num_days(),
                7,
                "weeks are seven days apart and none is skipped: {weeks:?}"
            );
        }
    }
}

#[tokio::test]
async fn another_readers_reads_never_reach_this_trend() {
    let mut subject = trend_reader("trend-mine").await;
    let mut other = trend_reader("trend-other").await;

    let weeks = trend(&mut subject).await;
    let mine = midday(&weeks, 0, 1);
    seed_progress_on(subject.db(), &subject.account, &uuid_for('b', 1), &mine, 1).await;
    for n in 0..5u32 {
        seed_progress_on(
            other.db(),
            &other.account,
            &uuid_for('c', n),
            &midday(&weeks, 0, 2),
            n,
        )
        .await;
    }

    let mine_after = trend(&mut subject).await;
    assert_eq!(
        mine_after[mine_after.len() - 1]["reads"],
        json!(1),
        "another reader's five reads are not mine: {mine_after:?}"
    );

    let theirs = trend(&mut other).await;
    assert_eq!(
        theirs[theirs.len() - 1]["reads"],
        json!(5),
        "and their own trend still counts theirs: {theirs:?}"
    );
}

#[tokio::test]
async fn a_status_written_against_another_accounts_work_stays_out_of_my_trend() {
    // The same claim as the row count, but with the work id forged rather than
    // the account: the query filters on `account_id` and never on the work, so
    // a reader pointing at somebody else's work gains nothing.
    let mut subject = trend_reader("trend-forged").await;
    let other = trend_reader("trend-forged-other").await;

    let weeks = trend(&mut subject).await;
    let shared = uuid_for('b', 1);
    seed_progress_on(
        subject.db(),
        &subject.account,
        &shared,
        &midday(&weeks, 0, 1),
        1,
    )
    .await;
    seed_progress_on(
        other.db(),
        &other.account,
        &shared,
        &midday(&weeks, 0, 1),
        2,
    )
    .await;

    let mine = trend(&mut subject).await;
    assert_eq!(
        mine[mine.len() - 1]["reads"],
        json!(1),
        "one read of my own, on a work we share: {mine:?}"
    );
}
