//! Acceptance: a gated body must be indistinguishable from an absent one
//! (spec §7.7.3).
//!
//! The sibling file `body_audience.rs` proves the gate *decides* correctly —
//! that a qualifying reader is admitted and a non-qualifying one is refused.
//! That is a different claim, and it is not enough. A reader who is refused
//! learns two things from a 403 that a 404 would not tell them: that the work
//! **exists**, and that the reason they are outside is *audience-shaped* rather
//! than a rating ceiling, a sign-in requirement, or a paywall. Either fact is an
//! existence oracle for a body the instance operator has chosen to withhold.
//!
//! So the test here is a **paired-response comparison**, not a set of status
//! assertions:
//!
//! ```text
//! a_gated_readers_responses_are_indistinguishable_from_a_non_existent_works
//! ```
//!
//! For every surface, the same reader asks for a work that exists but is held
//! for someone else, and for a work id that was never created. The two
//! responses must agree on **status, headers and payload**, after the requested
//! id is normalised out of both bodies — because the id is the one thing that
//! legitimately differs, and a comparison that forgot to normalise it would
//! fail on a leak that is not one.
//!
//! **A test asserting `status == 404` is not this test.** It passes while an
//! `X-Body-Cached: true` header, a `Retry-After`, a different `content-type`,
//! or a body carrying `"chapter_count": 3` leaks the fact. That is precisely
//! the shape of defect §7.7.3 exists to prevent, and it is why the comparison
//! is over the whole response.
//!
//! ## The defect this file found
//!
//! `reading_decision` maps `DenyReason::NotPublished` to `NotFound` (404) and
//! every other refusal to `ContentRestricted` (403) — a deliberate and correct
//! mapping for a *rating* ceiling, where telling the reader "this is rated
//! above what you may see" is the whole point of the rating system. But
//! `DenyReason::BodyNotInAudience` was falling into the same 403 arm through
//! the `DenyReason` catch-all, so a gated body answered 403 and an absent one
//! answered 404. One HTTP status distinguished them, and it is the status code
//! a client branches on.
//!
//! The fix maps `BodyNotInAudience` to `NotFound { resource: "work" }`, joining
//! `NotPublished` and `BlockedByAuthor` — the three reasons that all mean *you
//! cannot tell that this exists*, which is what they now also say on the wire.
//! The reason enum is unchanged: the domain still records exactly why, and only
//! the rendering collapses.

use axum::http::StatusCode;
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use serde_json::json;

/// A work id that is well-formed and will never exist.
///
/// Well-formed on purpose. A malformed id is rejected at the parse boundary
/// with a *different* error (`invalid_work_id` / 400), so comparing against one
/// would prove nothing about the audience gate — it would compare a 404 against
/// a 400 and call the difference a pass or a fail depending on luck. The
/// comparison must be between two answers the service would both have produced
/// for a real request.
const ABSENT_WORK_ID: &str = "00000000-0000-4000-8000-000000000000";

fn router_for(tdb: &test_support::TestDb, dir: &std::path::Path) -> axum::Router {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    server::build_router(AppState::new(config, tdb.db().clone()))
}

/// Create, populate and publish a work through the real doors.
///
/// Through the router rather than by inserting a row: the claim under test is
/// about what a door *reveals*, and a fixture that seeds a row directly cannot
/// be caught revealing too much.
async fn published_work(client: &mut test_support::TestClient, title: &str) -> (String, String) {
    let (status, body) = client
        .post("/api/v1/works", json!({ "title": title }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
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
    (id, chapter)
}

async fn set_audience(tdb: &test_support::TestDb, work: &str, audience: &str) {
    let value = audience.to_owned();
    if tdb.is_postgres() {
        let sql = tdb.sql("UPDATE works SET body_audience = ? WHERE id = ?::uuid");
        sqlx::query(&sql)
            .bind(value.as_str())
            .bind(work)
            .execute(tdb.db().postgres_pool().expect("postgres"))
            .await
            .expect("set the work's audience");
    } else {
        sqlx::query("UPDATE works SET body_audience = ? WHERE id = ?")
            .bind(value.as_str())
            .bind(work)
            .execute(tdb.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("set the work's audience");
    }
}

/// Register a reader at a trust level that will NOT satisfy the gate.
async fn low_reader(
    tdb: &test_support::TestDb,
    dir: &std::path::Path,
    tag: &str,
) -> test_support::TestClient {
    let mut client = test_support::TestClient::new(router_for(tdb, dir));
    let handle = tag.replace(['.', '-'], "_");
    let account = test_support::register(&mut client, &format!("{tag}@test.dev"), &handle).await;
    lorehaven_db::governance::set_trust(tdb.db(), &account, 1, "{}")
        .await
        .expect("set the trust level");
    client
}

/// Everything a response carries that could distinguish one work from another.
///
/// Not `status` alone. The spec's requirement is that the two answers are the
/// same *answer*, and a client can read a header or a field, so both are
/// captured. `date` is excluded because it changes between two requests to the
/// same service; `content-length` is compared only where present, because a
/// 204/304 legitimately omits it.
/// The headers of two responses, with the per-request ones removed.
///
/// `date` is the only header that legitimately differs between two requests to
/// the same service. Everything else must match: a header that appears on one
/// response and not the other is a leak, and `X-Body-Cached` is the example the
/// spec names.
fn comparable_headers(headers: &axum::http::HeaderMap) -> Vec<(String, String)> {
    let mut kept: Vec<(String, String)> = headers
        .iter()
        .filter_map(|(name, value)| {
            let name = name.as_str().to_ascii_lowercase();
            // Both of these are per-request by design: `date` is a clock read
            // and `x-request-id` is a fresh correlation id. Normalising by
            // NAME, not by value, is the point — a leak that arrived as an
            // unexpected value under one of these names would still show up as
            // a difference in which names are present, and a leak that arrived
            // as an unexpected *name* is untouched by this.
            if name == "date" || name == "x-request-id" {
                return None;
            }
            value.to_str().ok().map(|v| (name, v.to_owned()))
        })
        .collect();
    kept.sort();
    kept
}

fn normalise(body: &serde_json::Value, ids: &[&str]) -> serde_json::Value {
    let text = body.to_string();
    let mut out = text;
    // `request_id` is a fresh UUID per response and is supposed to be. It is
    // the one field that differs between two *identical* answers, so it is
    // normalised out — and it is normalised by KEY, not by value, so that a
    // body which carried an extra unexpected id under that key would still show
    // up as a difference in shape.
    if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&out) {
        if let Some(error) = value.get_mut("error").and_then(|e| e.as_object_mut()) {
            error.insert("request_id".to_owned(), serde_json::json!("<REQUEST_ID>"));
        }
        out = value.to_string();
    }
    for id in ids {
        if !id.is_empty() {
            out = out.replace(id, "<ID>");
        }
    }
    // Re-parse so the comparison is over JSON *structure*, not over
    // serialisation: two payloads that differ only in key order or in whether
    // a `1.0` was written `1` are the same answer, and comparing strings would
    // call that a leak.
    serde_json::from_str(&out).unwrap_or(serde_json::Value::String(out))
}

/// The surfaces §7.7.3 names, as path templates.
///
/// `{}` is the work id. The set is deliberately the spec's list rather than
/// "the routes that exist": a surface that is *absent* is trivially
/// indistinguishable, and the only way to know it is absent is to ask.
fn surfaces() -> Vec<&'static str> {
    vec![
        "/api/v1/works/{}",
        "/api/v1/works/{}/chapters",
        "/api/v1/library",
        "/api/v1/notifications",
        "/api/v1/feed",
    ]
}

/// The paired comparison: for every surface, a gated work and an absent one
/// must produce the same response.
#[tokio::test]
async fn a_gated_readers_responses_are_indistinguishable_from_a_non_existent_works() {
    let dir = test_support::scratch_dir("ba_indistinct");
    let tdb = test_support::TestDb::connect_with_dir("ba-indistinct", &dir).await;

    let mut author = test_support::TestClient::new(router_for(&tdb, &dir));
    test_support::register(&mut author, "author@test.dev", "ba_ind_author").await;
    let (work, chapter) = published_work(&mut author, "Held For Someone Else").await;
    set_audience(&tdb, &work, "trust_at_least:4").await;

    let mut reader = low_reader(&tdb, &dir, "ba_ind_low").await;

    // The premise: this reader really is outside the gate. Checked but NOT
    // asserted with an early return, because an early `assert_eq!` here
    // shadows the comparison below — when the 403/404 split was present, the
    // test failed *here* and never compared a single response, so it proved the
    // fix by failing for a different reason than the one it exists to catch.
    // The status is recorded and asserted at the END, after the comparison, so
    // a split shows up as a difference between two responses (with both
    // payloads printed) rather than as a bare "403 != 404".
    let (gated_work_status, gated_work_body) = reader.get(format!("/api/v1/works/{work}")).await;
    assert_ne!(
        gated_work_status,
        StatusCode::OK,
        "the reader must actually be refused, or this comparison proves nothing: {gated_work_body}"
    );

    let mut compared = 0;
    for template in surfaces() {
        let gated_path = template.replace("{}", &work);
        let absent_path = template.replace("{}", ABSENT_WORK_ID);

        let (gated_status, gated_headers, gated_body) =
            reader.get_with_headers(gated_path.clone()).await;
        let (absent_status, absent_headers, absent_body) =
            reader.get_with_headers(absent_path.clone()).await;

        assert_eq!(
            comparable_headers(&gated_headers),
            comparable_headers(&absent_headers),
            "headers differ on {template}: a header present on one and not the \
             other is an existence oracle, and `X-Body-Cached` is the one the \
             spec names"
        );

        assert_eq!(
            gated_status, absent_status,
            "status differs on {template}: gated={gated_status} absent={absent_status}"
        );

        let ids = [work.as_str(), chapter.as_str(), ABSENT_WORK_ID];
        assert_eq!(
            normalise(&gated_body, &ids),
            normalise(&absent_body, &ids),
            "payload differs on {template}\n  gated : {gated_body}\n  absent: {absent_body}"
        );
        compared += 1;
    }

    assert!(
        compared >= 5,
        "the spec names five surfaces; got {compared}"
    );

    // The premise, asserted last, now that the comparison has actually run.
    assert_eq!(
        gated_work_status,
        StatusCode::NOT_FOUND,
        "a refused reader must be told the work is absent, not that it is \
         out of reach: a 403 is an existence oracle"
    );
    tdb.cleanup().await;
}

/// The chapter door must not be the one surface that leaks.
///
/// Separate from the table above because the chapter door is addressed by
/// *chapter* id, not work id, and a reader who cannot see the work can still
/// ask for its chapter directly. This is the oracle §7.7.3 is most worried
/// about: the work page might be correctly blank while `GET /chapters/{id}`
/// still serves the text.
#[tokio::test]
async fn the_chapter_door_does_not_leak_a_gated_body_to_a_reader_of_the_chapter_id() {
    let dir = test_support::scratch_dir("ba_chapter_leak");
    let tdb = test_support::TestDb::connect_with_dir("ba-chapter-leak", &dir).await;

    let mut author = test_support::TestClient::new(router_for(&tdb, &dir));
    test_support::register(&mut author, "author@test.dev", "ba_cl_author").await;
    let (work, chapter) = published_work(&mut author, "Held For Someone Else").await;
    set_audience(&tdb, &work, "trust_at_least:4").await;

    let mut reader = low_reader(&tdb, &dir, "ba_cl_low").await;

    let (leak_status, leak_body) = reader.get(format!("/api/v1/chapters/{chapter}")).await;
    let (absent_status, absent_body) = reader
        .get(format!("/api/v1/chapters/{ABSENT_WORK_ID}"))
        .await;

    assert_eq!(
        leak_status, absent_status,
        "the chapter door answers differently for a gated chapter: {leak_status} vs {absent_status}"
    );

    let ids = [work.as_str(), chapter.as_str(), ABSENT_WORK_ID];
    assert_eq!(
        normalise(&leak_body, &ids),
        normalise(&absent_body, &ids),
        "the chapter door answers differently in the body\n  gated : {leak_body}\n  absent: {absent_body}"
    );

    tdb.cleanup().await;
}
