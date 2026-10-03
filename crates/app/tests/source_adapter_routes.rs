//! M45-57 — §55.2's submission pipeline over HTTP (spec §55.8's route lines).
//!
//! The store tests prove the rules hold when the store is called directly. This
//! file proves the *routes* carry them, and it is the only place two of §55.8's
//! acceptance lines are observable at all:
//!
//! - "A curator below TL3 is refused, and the refusal names the bar." A store
//!   test asserts the error type; only this asserts the 403 and the body.
//! - "A submitted adapter serves no reader traffic before publication." Also
//!   observable: a pending submission appears in the queue and is not in the
//!   published set.
//!
//! Every case runs on whichever engine the harness chose, so the suite is run
//! twice — see `docs/plans/REMAINING-2026-10-03.md` for the two invocations.

use axum::http::StatusCode;
use serde_json::{json, Value};

use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;

use test_support::{scratch_dir, TestClient, TestDb};

/// A §55.3 manifest that must compile. Kept as a literal so the route is
/// exercised against the same text a curator would actually send.
const GOOD_MANIFEST: &str = r#"
source_id: example-site
name: Example Fictions
base_url: https://example-fictions.test
rate_limit_per_second: 1.0
work_pattern: /works/{id}
chapter_pattern: /works/{id}/chapters/{num}
selectors:
  title: "h1.title"
  author: "span.byline"
  summary: "div.summary"
  body: "div#ch-body"
  tags: "ul.tags li"
  word_count: "span.words"
  date_published: "time.published"
pagination:
  type: none
auth:
  type: none
"#;

/// A manifest whose `base_url` is §11.5's cloud metadata service.
///
/// This is the case that would be missed by a test asserting only on
/// `validate_url`: that function checks syntactic shape and resolves nothing,
/// so it *accepts* `169.254.169.254`. The refusal has to come from the address
/// rules, which is what §55.8's second acceptance line is actually about.
const METADATA_MANIFEST: &str = r#"
source_id: metadata-service
name: Instance Metadata
base_url: http://169.254.169.254
rate_limit_per_second: 1.0
work_pattern: /latest/meta-data/{id}
chapter_pattern: /latest/meta-data/{id}/{num}
selectors:
  title: "h1"
  author: "span"
  summary: "div"
  body: "div"
  tags: "li"
  word_count: "span"
  date_published: "time"
pagination:
  type: none
auth:
  type: none
"#;

/// A manifest whose body selector is not CSS.
const BROKEN_SELECTOR_MANIFEST: &str = r#"
source_id: broken-site
name: Broken Selectors
base_url: https://broken-selectors.test
rate_limit_per_second: 1.0
work_pattern: /works/{id}
chapter_pattern: /works/{id}/chapters/{num}
selectors:
  title: "h1[[[title"
  author: "span.byline"
  summary: "div.summary"
  body: "div#ch-body"
  tags: "ul.tags li"
  word_count: "span.words"
  date_published: "time.published"
pagination:
  type: none
auth:
  type: none
"#;

/// The §21.1 extension manifest. `source_adapters` is §21.2's category for
/// this feature, and the row is only accepted if the manifest declares it.
const EXTENSION_MANIFEST: &str = r#"{
  "id": "example-site-adapter",
  "name": "Example Fictions adapter",
  "version": "1.0.0",
  "entrypoint": "source.yaml",
  "category": "source_adapters",
  "capabilities": ["network"]
}"#;

fn router_for(tdb: &TestDb, dir: &std::path::Path) -> axum::Router {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    server::build_router(AppState::new(config, tdb.db().clone()))
}

/// A registered reader at exactly `trust`.
///
/// `set_trust` upserts, and registration writes no trust row at all — so a
/// fixture that only INSERTed would leave every account at TL0, and a gate that
/// refuses everyone reads as a gate that works. That is a GREEN(BAD) this
/// suite must not have, which is why the level is set here rather than assumed.
async fn reader_at(tdb: &TestDb, dir: &std::path::Path, tag: &str, trust: i64) -> TestClient {
    let mut client = TestClient::new(router_for(tdb, dir));
    let handle = tag.replace(['.', '-'], "_");
    let account = test_support::register(&mut client, &format!("{tag}@test.dev"), &handle).await;
    lorehaven_db::governance::set_trust(tdb.db(), &account, trust, "{}")
        .await
        .expect("set the trust level");
    client
}

/// One TL3 curator's submission, through the route.
async fn submit(client: &mut TestClient, source_manifest: &str) -> (StatusCode, Value) {
    client
        .post(
            "/api/v1/extensions/source-adapters",
            json!({ "manifest": EXTENSION_MANIFEST, "source_manifest": source_manifest }),
        )
        .await
}

// ---------------------------------------------------------------------------
// The trust gate — §55.8's "a curator below TL3 is refused, and the refusal
// names the bar"
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_curator_below_tl3_is_refused_and_the_refusal_names_the_bar() {
    let dir = scratch_dir("sa_below_tl3");
    let tdb = TestDb::connect_with_dir("sa-below-tl3", &dir).await;

    // TL2 is one below the bar, which is the case a `> 3` comparison would also
    // refuse. TL0 is the ordinary brand-new account.
    for (tag, trust) in [("sa_tl0", 0_i64), ("sa_tl2", 2)] {
        let mut client = reader_at(&tdb, &dir, tag, trust).await;
        let (status, body) = submit(&mut client, GOOD_MANIFEST).await;

        assert_eq!(
            status,
            axum::http::StatusCode::FORBIDDEN,
            "{tag} at TL{trust} must be refused, not queued: {body}"
        );
        // §55.8 requires the refusal to NAME the bar. A bare 403 sends the
        // curator to edit a plugin manifest instead of working toward a trust
        // threshold, which is the thing that would actually help.
        let rendered = body.to_string();
        assert!(
            rendered.contains("3") && rendered.contains(&trust.to_string()),
            "the refusal must name both the bar (3) and the level found ({trust}): {rendered}"
        );
        assert!(
            rendered.contains("TRUST_LEVEL_INSUFFICIENT"),
            "a trust refusal has its own error code so it cannot be mistaken for a \
             capability refusal: {rendered}"
        );
    }
}

#[tokio::test]
async fn a_curator_at_tl3_is_accepted_and_the_submission_is_pending_not_published() {
    let dir = scratch_dir("sa_at_tl3");
    let tdb = TestDb::connect_with_dir("sa-at-tl3", &dir).await;
    let mut client = reader_at(&tdb, &dir, "sa_tl3", 3).await;

    let (status, body) = submit(&mut client, GOOD_MANIFEST).await;
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "a TL3 curator must be able to submit: {body}"
    );
    assert_eq!(
        body["state"], "pending",
        "§55.7's third invariant: submission confers nothing, so the row lands pending: {body}"
    );

    let id = body["id"].as_str().expect("a submission id").to_owned();

    // The same property read from the published set. `published_source_manifests`
    // is what a registry would load, so a pending submission appearing there
    // would mean a curator's unreviewed selectors serve reader traffic —
    // §55.8's "a submitted adapter serves no reader traffic before publication".
    // `published_source_manifests` returns `(submitter, manifest_text)` pairs, so the
    // check reads the second element. Checking the submitter instead would pass
    // for a submission that WAS published — the assertion would be about a field
    // that has nothing to do with what the query filters on.
    let published = lorehaven_db::source_adapters::published_source_manifests(tdb.db())
        .await
        .expect("published manifests");
    assert!(
        !published
            .iter()
            .any(|(_submitter, manifest)| manifest.contains("example-fictions.test")),
        "a pending submission must not be published; the published set was {published:?}"
    );

    // And it is visible to reviewers, which is what makes quorum possible.
    let (status, queue) = client.get("/api/v1/extensions/source-adapters").await;
    assert_eq!(status, axum::http::StatusCode::OK, "{queue}");
    let ids: Vec<&str> = queue["submissions"]
        .as_array()
        .expect("an array of submissions")
        .iter()
        .map(|s| s["id"].as_str().expect("an id"))
        .collect();
    assert!(
        ids.contains(&id.as_str()),
        "the queue must show the pending submission: {queue}"
    );
}

// ---------------------------------------------------------------------------
// §55.3's compile-at-submission rule, enforced before quorum costs anyone time
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_manifest_pointing_at_a_private_address_is_refused_at_submission() {
    let dir = scratch_dir("sa_private_addr");
    let tdb = TestDb::connect_with_dir("sa-private-addr", &dir).await;
    let mut client = reader_at(&tdb, &dir, "sa_priv", 3).await;

    let (status, body) = submit(&mut client, METADATA_MANIFEST).await;
    assert_eq!(
        status,
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        "§55.8: a manifest naming a link-local address is refused at submission: {body}"
    );
    let rendered = body.to_string();
    assert!(
        rendered.contains("169.254.169.254"),
        "the refusal must name the address, or a curator cannot tell which of a hundred \
         URLs was the problem: {rendered}"
    );
}

#[tokio::test]
async fn a_selector_that_does_not_compile_is_refused_naming_the_selector() {
    let dir = scratch_dir("sa_bad_selector");
    let tdb = TestDb::connect_with_dir("sa-bad-selector", &dir).await;
    let mut client = reader_at(&tdb, &dir, "sa_badsel", 3).await;

    let (status, body) = submit(&mut client, BROKEN_SELECTOR_MANIFEST).await;
    assert_eq!(
        status,
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        "§55.8: a selector that fails to compile is refused: {body}"
    );
    let rendered = body.to_string();
    // §55.8's third line says "naming the selector". "title" is the field that
    // is broken, and a message naming only "selectors" would make a curator
    // re-check all seven.
    assert!(
        rendered.contains("title"),
        "the refusal must name the offending selector field: {rendered}"
    );
}

// ---------------------------------------------------------------------------
// §19.4's quorum, through the routes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn three_distinct_approvers_reach_the_threshold_and_two_do_not() {
    let dir = scratch_dir("sa_quorum");
    let tdb = TestDb::connect_with_dir("sa-quorum", &dir).await;

    // Three separate TL3 reviewers. §19.4 counts *people*, so three accounts
    // approving is the case; one account approving three times is not possible
    // and must not be mistaken for it.
    let mut curators = Vec::new();
    for n in 0..3 {
        curators.push(reader_at(&tdb, &dir, &format!("sa_cur{n}"), 3).await);
    }

    let (status, body) = submit(&mut curators[0], GOOD_MANIFEST).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    let id = body["id"].as_str().expect("a submission id").to_owned();

    let endpoint = format!("/api/v1/extensions/source-adapters/{id}/reviews");

    for (n, client) in curators.iter_mut().enumerate() {
        let (status, body) = client
            .post(
                &endpoint,
                json!({ "verdict": "approve", "note": format!("reviewed: {n}") }),
            )
            .await;
        assert_eq!(status, axum::http::StatusCode::OK, "review {n}: {body}");
        let reached = body["reached"].as_bool().expect("a reached flag");
        let approvals = body["approvals"].as_i64().expect("a count");
        assert_eq!(
            approvals,
            i64::try_from(n).unwrap() + 1,
            "one more each time: {body}"
        );
        // §19.4's threshold, and the response must not soften it. Asserted
        // separately from `approvals` because they are different facts: the
        // first version compared the two to each other, which only holds once
        // every approval has landed and reads like a quorum assertion while
        // being one about a constant.
        assert_eq!(
            body["threshold"].as_i64(),
            Some(3),
            "§19.4 requires three reviewers: {body}"
        );
        assert_eq!(
            reached,
            approvals >= 3,
            "reached must track the threshold, not the count: {body}"
        );
    }

    // Two approvals alone are not enough. Read from the review record rather than
    // trusting the flag the POST returned.
    let mut fresh = reader_at(&tdb, &dir, "sa_fresh", 3).await;
    let (status, record) = fresh.get(&endpoint).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{record}");
    assert_eq!(
        record["approvals"].as_i64(),
        Some(3),
        "three distinct reviewers have approved: {record}"
    );
    assert_eq!(record["reviews"].as_array().map(Vec::len), Some(3));
}

#[tokio::test]
async fn the_same_reviewer_approving_twice_still_counts_once() {
    let dir = scratch_dir("sa_double_review");
    let tdb = TestDb::connect_with_dir("sa-double-review", &dir).await;

    let mut curator = reader_at(&tdb, &dir, "sa_twice", 3).await;
    let (status, body) = submit(&mut curator, GOOD_MANIFEST).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    let id = body["id"].as_str().expect("a submission id").to_owned();
    let endpoint = format!("/api/v1/extensions/source-adapters/{id}/reviews");

    for _ in 0..3 {
        let (status, body) = curator
            .post(&endpoint, json!({ "verdict": "approve" }))
            .await;
        assert_eq!(status, axum::http::StatusCode::OK, "{body}");
        assert_eq!(
            body["approvals"].as_i64(),
            Some(1),
            "§19.4 counts reviewers, not votes, so one person's three submissions is one: {body}"
        );
        assert_eq!(
            body["reached"].as_bool(),
            Some(false),
            "and one approval can never reach the threshold: {body}"
        );
    }
}

#[tokio::test]
async fn an_unknown_verdict_is_refused_rather_than_stored() {
    let dir = scratch_dir("sa_bad_verdict");
    let tdb = TestDb::connect_with_dir("sa-bad-verdict", &dir).await;
    let mut curator = reader_at(&tdb, &dir, "sa_verdict", 3).await;

    let (status, body) = submit(&mut curator, GOOD_MANIFEST).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    let id = body["id"].as_str().expect("a submission id").to_owned();
    let endpoint = format!("/api/v1/extensions/source-adapters/{id}/reviews");

    let (status, body) = curator
        .post(&endpoint, json!({ "verdict": "approve-with-enthusiasm" }))
        .await;
    assert!(
        !status.is_success(),
        "a verdict outside the set must not be stored: {status} {body}"
    );

    // The store validates before the CHECK constraint, so the message names the
    // caller's mistake instead of arriving as a driver violation.
    let rendered = body.to_string();
    assert!(
        rendered.contains("approve") && rendered.contains("abstain"),
        "the refusal must name the allowed verdicts: {rendered}"
    );

    let (status, record) = curator.get(&endpoint).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{record}");
    assert_eq!(
        record["reviews"].as_array().map(Vec::len),
        Some(0),
        "a refused verdict left no row: {record}"
    );
}

// ---------------------------------------------------------------------------
// Authentication — §55.8's 401 line, on every route
// ---------------------------------------------------------------------------

#[tokio::test]
async fn every_source_adapter_route_needs_a_session() {
    let dir = scratch_dir("sa_unauth");
    let tdb = TestDb::connect_with_dir("sa-unauth", &dir).await;
    let mut anonymous = TestClient::new(router_for(&tdb, &dir));

    // An id that does not exist, so a 404 would be ambiguous with a 401: the
    // refusal must be about *who is asking*, not about the subject.
    let missing = "00000000-0000-4000-8000-000000000000";
    // Bound to `let`s rather than inline `format!` inside the array: a temporary in
    // a `Vec<&str>` is freed at the end of the statement and the loop below
    // borrows it.
    let collection = format!("/api/v1/extensions/source-adapters/{missing}");
    let reviews = format!("/api/v1/extensions/source-adapters/{missing}/reviews");
    let cases: Vec<(&str, &str, Option<Value>)> = vec![
        ("GET", "/api/v1/extensions/source-adapters", None),
        (
            "POST",
            "/api/v1/extensions/source-adapters",
            Some(json!({
                "manifest": EXTENSION_MANIFEST, "source_manifest": GOOD_MANIFEST
            })),
        ),
        ("GET", collection.as_str(), None),
        ("GET", reviews.as_str(), None),
        (
            "POST",
            reviews.as_str(),
            Some(json!({ "verdict": "approve" })),
        ),
    ];

    for (method, uri, body) in cases {
        let (status, response) = anonymous.request(method, uri, body).await;
        assert_eq!(
            status,
            axum::http::StatusCode::UNAUTHORIZED,
            "{method} {uri} must require a session, not disclose anything: {response}"
        );
    }
}

// ---------------------------------------------------------------------------
// Reading a submission
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_submission_that_does_not_exist_is_a_404() {
    let dir = scratch_dir("sa_missing");
    let tdb = TestDb::connect_with_dir("sa-missing", &dir).await;
    let mut client = reader_at(&tdb, &dir, "sa_missing", 3).await;

    let (status, body) = client
        .get("/api/v1/extensions/source-adapters/00000000-0000-4000-8000-000000000000")
        .await;
    assert_eq!(
        status,
        axum::http::StatusCode::NOT_FOUND,
        "an authenticated reader asking about nothing is a 404: {body}"
    );
}
