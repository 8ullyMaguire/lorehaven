//! §32.7.2 perceptual match proposals, end to end through the router.
//!
//! The db-level behaviour is covered in `media_resilience.rs`; this file drives
//! the HTTP surface, because the interesting part of an API is what it does with
//! a stranger's request: an unauthenticated call, a signed-in reader who is not
//! a curator, a curator acting twice, and a path parameter that is not an id.

use axum::http::StatusCode;
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_db::media_resilience;
use lorehaven_domain::media_resilience::{LinkProvider, MediaKind};
use serde_json::json;
use test_support::{register, scratch_dir, TestClient, TestDb};
use uuid::Uuid;

struct Harness {
    _dir: std::path::PathBuf,
    tdb: TestDb,
    config: Config,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        server::set_trust_proxy(false);
        let dir = scratch_dir(tag);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        let mut config = Config::development_defaults();
        config.storage.root = dir.to_path_buf();
        // Describe the backend the pool is actually on. Hardcoding the SQLite
        // URL made the app read a database nobody had migrated, so every
        // authenticated request came back 401 under PG while passing locally.
        config.database = if tdb.is_postgres() {
            lorehaven_db::DatabaseConfig::new(
                std::env::var("LOREHAVEN_TEST_PG_URL").unwrap_or_default(),
            )
        } else {
            lorehaven_db::DatabaseConfig::new(format!(
                "sqlite://{}/lorehaven.sqlite?mode=rwc",
                dir.display()
            ))
        };
        Self {
            _dir: dir,
            tdb,
            config,
        }
    }

    fn client(&self) -> TestClient {
        TestClient::new(server::build_router(AppState::new(
            self.config.clone(),
            self.tdb.db().clone(),
        )))
    }
}

/// Promote an account to the trust level the proposal endpoints require.
async fn make_operator(db: &lorehaven_db::Database, account_id: &str) {
    lorehaven_db::governance::set_trust(db, account_id, 5, "test:curator")
        .await
        .expect("grant trust");
}

/// Two references one dHash step apart, with a link on each and one proposal.
async fn seed_proposal(db: &lorehaven_db::Database, distance: u32) -> (String, String, String) {
    let existing = Uuid::new_v4().to_string();
    let candidate = Uuid::new_v4().to_string();
    for (rid, hash) in [(&existing, "sha256:aaa"), (&candidate, "sha256:bbb")] {
        media_resilience::insert_media_reference(db, rid, hash, MediaKind::Image)
            .await
            .expect("insert reference");
    }
    for (rid, url) in [
        (&existing, "https://cdn.example.com/original.png"),
        (&candidate, "https://cdn.example.com/reencode.png"),
    ] {
        media_resilience::insert_availability_link(
            db,
            &Uuid::new_v4().to_string(),
            rid,
            url,
            LinkProvider::Other,
            None,
            100,
        )
        .await
        .expect("insert link");
    }
    media_resilience::record_match_proposal(
        db,
        &candidate,
        &existing,
        "sha256:bbb",
        Some("0123456789abcdef"),
        distance,
    )
    .await
    .expect("record proposal");
    let proposal_id = media_resilience::list_pending_match_proposals(db, 10)
        .await
        .expect("list pending")[0]
        .proposal
        .id
        .clone();
    (existing, candidate, proposal_id)
}

#[tokio::test]
async fn the_proposal_queue_is_closed_to_anonymous_callers() {
    let h = Harness::new("mr-prop-anon").await;
    let mut client = h.client();
    let (status, body) = client.get("/api/v1/media/match-proposals").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

#[tokio::test]
async fn a_signed_in_reader_cannot_see_or_decide_proposals() {
    let h = Harness::new("mr-prop-reader").await;
    seed_proposal(h.tdb.db(), 2).await;
    let mut client = h.client();
    let account = register(&mut client, "reader-prop@t.test", "readerprop").await;
    assert!(!account.is_empty());

    // Signing in is not enough. Merging references discards a row, so the bar is
    // the operator trust level, not "has a session".
    let (status, body) = client.get("/api/v1/media/match-proposals").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "queue: {body}");

    let proposal_id = media_resilience::list_pending_match_proposals(h.tdb.db(), 10)
        .await
        .expect("list pending")[0]
        .proposal
        .id
        .clone();
    let (status, body) = client
        .post(
            &format!("/api/v1/media/match-proposals/{proposal_id}"),
            json!({ "decision": "confirm" }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "confirm: {body}");

    // And nothing was merged behind the refusal.
    assert_eq!(
        media_resilience::count_pending_match_proposals(h.tdb.db())
            .await
            .expect("count"),
        1
    );
}

#[tokio::test]
async fn an_operator_sees_the_queue_with_the_distance_and_confidence() {
    let h = Harness::new("mr-prop-queue").await;
    let (existing, _candidate, _) = seed_proposal(h.tdb.db(), 2).await;
    let mut client = h.client();
    let account = register(&mut client, "op-queue@t.test", "opqueue").await;
    make_operator(h.tdb.db(), &account).await;

    let (status, body) = client.get("/api/v1/media/match-proposals").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total_pending"], 1, "{body}");
    let first = &body["pending"][0];
    assert_eq!(first["proposal"]["existing_reference_id"], existing);
    assert_eq!(first["proposal"]["hamming_distance"], 2, "{body}");
    // The curator is shown a number, not a boolean: the spec says "present the
    // curator with a match confidence score".
    let confidence = first["proposal"]["match_confidence"]
        .as_f64()
        .expect("confidence is a number");
    assert!(confidence > 0.9, "{confidence}");
    assert_eq!(
        first["existing_content_hash"], "sha256:aaa",
        "the curator needs both hashes to judge without rendering the images"
    );
}

#[tokio::test]
async fn confirming_through_the_api_merges_the_references() {
    let h = Harness::new("mr-prop-confirm").await;
    let (existing, candidate, proposal_id) = seed_proposal(h.tdb.db(), 1).await;
    let mut client = h.client();
    let account = register(&mut client, "op-confirm@t.test", "opconfirm").await;
    make_operator(h.tdb.db(), &account).await;

    let (status, body) = client
        .post(
            &format!("/api/v1/media/match-proposals/{proposal_id}"),
            json!({ "decision": "confirm", "note": "same artwork, re-encoded" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "confirmed", "{body}");
    assert_eq!(body["merged"], true, "{body}");

    // One reference, two links -- the spec's claim about deduplication.
    assert!(
        media_resilience::find_media_reference_by_id(h.tdb.db(), &candidate)
            .await
            .expect("look up candidate")
            .is_none()
    );
    assert_eq!(
        media_resilience::count_total_links(h.tdb.db(), &existing)
            .await
            .expect("links"),
        2
    );
}

#[tokio::test]
async fn rejecting_through_the_api_keeps_them_apart_and_ends_the_question() {
    let h = Harness::new("mr-prop-reject").await;
    let (existing, candidate, proposal_id) = seed_proposal(h.tdb.db(), 6).await;
    let mut client = h.client();
    let account = register(&mut client, "op-reject@t.test", "opreject").await;
    make_operator(h.tdb.db(), &account).await;

    let (status, body) = client
        .post(
            &format!("/api/v1/media/match-proposals/{proposal_id}"),
            json!({ "decision": "reject" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["merged"], false, "{body}");

    assert!(
        media_resilience::find_media_reference_by_id(h.tdb.db(), &candidate)
            .await
            .expect("look up candidate")
            .is_some()
    );
    assert_eq!(
        media_resilience::count_total_links(h.tdb.db(), &existing)
            .await
            .expect("links"),
        1
    );
    // The queue is empty. A rejection is an answer, not a deferral.
    let (status, body) = client.get("/api/v1/media/match-proposals").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total_pending"], 0, "{body}");
}

#[tokio::test]
async fn a_decision_that_is_neither_confirm_nor_reject_is_refused() {
    let h = Harness::new("mr-prop-bad").await;
    let (_existing, candidate, proposal_id) = seed_proposal(h.tdb.db(), 2).await;
    let mut client = h.client();
    let account = register(&mut client, "op-bad@t.test", "opbad").await;
    make_operator(h.tdb.db(), &account).await;

    // The two decisions are opposites, so a typo must not default to one of them.
    for bad in ["merge", "CONFIRM", "", "delete"] {
        let (status, body) = client
            .post(
                &format!("/api/v1/media/match-proposals/{proposal_id}"),
                json!({ "decision": bad }),
            )
            .await;
        assert!(
            status == StatusCode::UNPROCESSABLE_ENTITY || status == StatusCode::BAD_REQUEST,
            "decision `{bad}` should be refused, got {status}: {body}"
        );
    }
    // Nothing moved through any of them.
    assert!(
        media_resilience::find_media_reference_by_id(h.tdb.db(), &candidate)
            .await
            .expect("look up candidate")
            .is_some()
    );
    assert_eq!(
        media_resilience::count_pending_match_proposals(h.tdb.db())
            .await
            .expect("count"),
        1
    );
}

#[tokio::test]
async fn deciding_twice_is_a_404_not_a_second_merge() {
    let h = Harness::new("mr-prop-twice").await;
    let (existing, candidate, proposal_id) = seed_proposal(h.tdb.db(), 1).await;
    let mut client = h.client();
    let account = register(&mut client, "op-twice@t.test", "optwice").await;
    make_operator(h.tdb.db(), &account).await;

    let (status, _) = client
        .post(
            &format!("/api/v1/media/match-proposals/{proposal_id}"),
            json!({ "decision": "reject" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // A second click, or a retried request. 404, and -- the part that matters --
    // the earlier rejection still stands.
    let (status, body) = client
        .post(
            &format!("/api/v1/media/match-proposals/{proposal_id}"),
            json!({ "decision": "confirm" }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(
        media_resilience::find_media_reference_by_id(h.tdb.db(), &candidate)
            .await
            .expect("look up candidate")
            .is_some()
    );
    assert_eq!(
        media_resilience::count_total_links(h.tdb.db(), &existing)
            .await
            .expect("links"),
        1
    );
}

#[tokio::test]
async fn a_path_parameter_that_is_not_an_id_is_a_404_not_a_500() {
    let h = Harness::new("mr-prop-badid").await;
    seed_proposal(h.tdb.db(), 2).await;
    let mut client = h.client();
    let account = register(&mut client, "op-badid@t.test", "opbadid").await;
    make_operator(h.tdb.db(), &account).await;

    // PostgreSQL raises 22P02 on a non-uuid cast where SQLite matches nothing, so
    // this has to be answered before the query on both backends.
    for bad in ["not-a-uuid", "12345", "%20"] {
        let (status, body) = client
            .post(
                &format!("/api/v1/media/match-proposals/{bad}"),
                json!({ "decision": "confirm" }),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "`{bad}` should be a 404: {body}"
        );
    }
    // A well-formed id that was never proposed is the same answer, because from
    // the curator's side there is nothing to act on either way.
    let (status, body) = client
        .post(
            &format!("/api/v1/media/match-proposals/{}", Uuid::new_v4()),
            json!({ "decision": "confirm" }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn the_queue_limit_is_the_callers_and_cannot_be_asked_for_everything() {
    let h = Harness::new("mr-prop-limit").await;
    let db = h.tdb.db();
    let existing = Uuid::new_v4().to_string();
    media_resilience::insert_media_reference(db, &existing, "sha256:aaa", MediaKind::Image)
        .await
        .expect("insert reference");
    for i in 0..4 {
        let candidate = Uuid::new_v4().to_string();
        media_resilience::insert_media_reference(
            db,
            &candidate,
            &format!("sha256:c{i}"),
            MediaKind::Image,
        )
        .await
        .expect("insert candidate");
        media_resilience::record_match_proposal(
            db,
            &candidate,
            &existing,
            &format!("sha256:c{i}"),
            Some("0123456789abcdef"),
            i as u32 + 1,
        )
        .await
        .expect("record proposal");
    }
    let mut client = h.client();
    let account = register(&mut client, "op-limit@t.test", "oplimit").await;
    make_operator(db, &account).await;

    let (status, body) = client.get("/api/v1/media/match-proposals?limit=2").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["pending"].as_array().expect("array").len(),
        2,
        "{body}"
    );
    // The count is honest about what was withheld: four are waiting, two shown.
    assert_eq!(body["total_pending"], 4, "{body}");

    // A hostile or buggy limit must not become "no limit".
    for limit in [-1, 0, 100000] {
        let (status, body) = client
            .get(&format!("/api/v1/media/match-proposals?limit={limit}"))
            .await;
        assert_eq!(status, StatusCode::OK, "limit {limit}: {body}");
        let shown = body["pending"].as_array().expect("array").len();
        assert!((1..=200).contains(&shown), "limit {limit} returned {shown}");
    }
}
