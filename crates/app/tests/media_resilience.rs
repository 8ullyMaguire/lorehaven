//! Media resilience & availability guarantee (spec §32.7).
//!
//! These tests drive the real router against a real SQLite file.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::media_fetch::MediaFingerprint;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::media_resilience;
use lorehaven_domain::media_resilience::MediaKind;
use std::path::Path;
use std::path::PathBuf;
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-mr-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn config_for(dir: &std::path::Path) -> Config {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    config
}

async fn build_app(dir: &Path) -> axum::Router {
    let config = config_for(dir);
    let tdb = test_support::TestDb::connect_with_dir("mr", dir).await;
    let db = tdb.db().clone();
    let state = AppState::new(config, db);
    set_trust_proxy(false);
    server::build_router(state)
}

#[tokio::test]
async fn media_resilience_config_defaults() {
    let config = Config::development_defaults();
    assert_eq!(config.media_resilience.min_healthy_links, 2);
    assert_eq!(config.media_resilience.mirror_add_credits, 15);
    assert_eq!(config.media_resilience.archive_add_credits, 10);
    assert_eq!(config.media_resilience.verify_credits, 5);
    assert_eq!(config.media_resilience.daily_credits_cap, 500);
    assert_eq!(config.media_resilience.dead_threshold_failures, 5);
    assert_eq!(config.media_resilience.check_interval_secs, 3600);
}

#[tokio::test]
async fn media_resilience_insert_and_fetch() {
    let dir = scratch_dir("crud");
    let tdb = test_support::TestDb::connect_with_dir("mr-crud", &dir).await;
    let db = tdb.db();

    let ref_id = "test-ref-001";
    media_resilience::insert_media_reference(
        db,
        ref_id,
        "hash-abc",
        lorehaven_domain::media_resilience::MediaKind::Image,
    )
    .await
    .expect("insert reference");

    let reference = media_resilience::find_media_reference_by_id(db, ref_id)
        .await
        .expect("find reference")
        .expect("reference exists");
    assert_eq!(reference.id, ref_id);
    assert_eq!(reference.content_hash, "hash-abc");
    assert!(!reference.curator_verified);
}

#[tokio::test]
async fn media_resilience_availability_link() {
    let dir = scratch_dir("link");
    let tdb = test_support::TestDb::connect_with_dir("mr-link", &dir).await;
    let db = tdb.db();

    let ref_id = "test-ref-002";
    media_resilience::insert_media_reference(
        db,
        ref_id,
        "hash-def",
        lorehaven_domain::media_resilience::MediaKind::Image,
    )
    .await
    .expect("insert reference");

    let link_id = "test-link-001";
    media_resilience::insert_availability_link(
        db,
        link_id,
        ref_id,
        "https://example.com/image.jpg",
        lorehaven_domain::media_resilience::LinkProvider::Other,
        Some("user-001"),
        100,
    )
    .await
    .expect("insert link");

    let links = media_resilience::find_availability_links_for_reference(db, ref_id)
        .await
        .expect("find links");
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].url, "https://example.com/image.jpg");
    assert_eq!(
        links[0].status,
        lorehaven_domain::media_resilience::LinkStatus::PendingVerification.as_str()
    );

    // Update status
    media_resilience::update_link_status(
        db,
        link_id,
        lorehaven_domain::media_resilience::LinkStatus::Healthy,
        0,
    )
    .await
    .expect("update status");

    let healthy = media_resilience::count_healthy_links(db, ref_id)
        .await
        .expect("count healthy");
    assert_eq!(healthy, 1);
}

#[tokio::test]
async fn media_resilience_curator_rewards() {
    let dir = scratch_dir("rewards");
    let tdb = test_support::TestDb::connect_with_dir("mr-rewards", &dir).await;
    let db = tdb.db();

    let ref_id = "test-ref-003";
    let link_id = "test-link-003";
    media_resilience::insert_media_reference(
        db,
        ref_id,
        "hash-ghi",
        lorehaven_domain::media_resilience::MediaKind::Image,
    )
    .await
    .expect("insert reference");
    media_resilience::insert_availability_link(
        db,
        link_id,
        ref_id,
        "https://example.com/pic.png",
        lorehaven_domain::media_resilience::LinkProvider::Imgur,
        None,
        50,
    )
    .await
    .expect("insert link");

    media_resilience::insert_curator_reward(
        db,
        "user-001",
        lorehaven_domain::media_resilience::CuratorAction::MirrorAdd,
        Some(ref_id),
        Some(link_id),
        15,
    )
    .await
    .expect("insert reward");

    let total = media_resilience::sum_curator_rewards_today(db, "user-001")
        .await
        .expect("sum rewards");
    assert_eq!(total, 15);
}

#[tokio::test]
async fn media_resilience_get_route() {
    let dir = scratch_dir("route");
    let app = build_app(&dir).await;

    let tdb = test_support::TestDb::connect_with_dir("mr-route2", &dir).await;
    let db = tdb.db();

    let ref_id = "test-ref-004";
    media_resilience::insert_media_reference(
        db,
        ref_id,
        "hash-jkl",
        lorehaven_domain::media_resilience::MediaKind::Image,
    )
    .await
    .expect("insert reference");

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/media/references/{ref_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn media_resilience_missing_returns_404() {
    let dir = scratch_dir("missing");
    let app = build_app(&dir).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/media/references/does-not-exist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn media_resilience_links_needing_check() {
    let dir = scratch_dir("check");
    let tdb = test_support::TestDb::connect_with_dir("mr-check", &dir).await;
    let db = tdb.db();

    let ref_id = "test-ref-005";
    media_resilience::insert_media_reference(
        db,
        ref_id,
        "hash-mno",
        lorehaven_domain::media_resilience::MediaKind::Image,
    )
    .await
    .expect("insert reference");

    media_resilience::insert_availability_link(
        db,
        "link-005",
        ref_id,
        "https://example.com/check.png",
        lorehaven_domain::media_resilience::LinkProvider::Other,
        None,
        50,
    )
    .await
    .expect("insert link");

    let needing_check = media_resilience::find_links_needing_check(db, 100)
        .await
        .expect("find links needing check");
    assert_eq!(needing_check.len(), 1);
}

// ---------------------------------------------------------------------------
// M32-07a: perceptual dedup (spec §32.7.2)
// ---------------------------------------------------------------------------

/// Set a reference's perceptual hash, on whichever backend the test DB is.
async fn set_perceptual_hash(db: &lorehaven_db::Database, reference_id: &str, hash: &str) {
    use lorehaven_db::Backend;
    let sql = match db.backend() {
        Backend::Sqlite => "UPDATE media_references SET perceptual_hash = ? WHERE id = ?",
        Backend::Postgres => "UPDATE media_references SET perceptual_hash = $1 WHERE id = $2",
    };
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(sql)
                .bind(hash)
                .bind(reference_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("set phash");
        }
        Backend::Postgres => {
            sqlx::query(sql)
                .bind(hash)
                .bind(reference_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("set phash");
        }
    }
}

#[tokio::test]
async fn a_perceptual_match_within_the_threshold_is_found() {
    let dir = scratch_dir("phash-within");
    let tdb = test_support::TestDb::connect_with_dir("mr-phash-within", &dir).await;
    let db = tdb.db();

    // The stored hash differs from the query hash in exactly 4 bits
    // (0x0 ^ 0x3 = two bits, 0xff ^ 0xfc = two bits), which is inside the
    // default threshold of 6. It is NOT an exact match, so the old
    // equality-only query returned nothing here.
    media_resilience::insert_media_reference(db, "phash-near", "sha256:near", MediaKind::Image)
        .await
        .expect("insert near");
    set_perceptual_hash(db, "phash-near", "00ffff00").await;

    let found = media_resilience::find_by_perceptual_hash(db, "00fcff00", 6)
        .await
        .expect("perceptual search");
    assert_eq!(
        found.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec!["phash-near"],
        "a 4-bit difference is inside a threshold of 6"
    );
}

#[tokio::test]
async fn a_perceptual_match_outside_the_threshold_is_not_found() {
    let dir = scratch_dir("phash-outside");
    let tdb = test_support::TestDb::connect_with_dir("mr-phash-outside", &dir).await;
    let db = tdb.db();

    // 12 bits differ, well past a threshold of 6.
    media_resilience::insert_media_reference(db, "phash-far", "sha256:far", MediaKind::Image)
        .await
        .expect("insert far");
    set_perceptual_hash(db, "phash-far", "00ff00ff").await;

    let found = media_resilience::find_by_perceptual_hash(db, "ffff0000", 6)
        .await
        .expect("perceptual search");
    assert!(
        found.is_empty(),
        "12 bits of difference must not match at a threshold of 6, got {:?}",
        found.iter().map(|r| &r.id).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn the_threshold_decides_what_is_found() {
    let dir = scratch_dir("phash-threshold");
    let tdb = test_support::TestDb::connect_with_dir("mr-phash-threshold", &dir).await;
    let db = tdb.db();

    // Two differing bits: hidden at a threshold of 1, visible at 2.
    media_resilience::insert_media_reference(db, "phash-two", "sha256:two", MediaKind::Image)
        .await
        .expect("insert two");
    set_perceptual_hash(db, "phash-two", "00ff").await;

    let strict = media_resilience::find_by_perceptual_hash(db, "00fc", 1)
        .await
        .expect("search at 1");
    assert!(
        strict.is_empty(),
        "2 bits must not match at a threshold of 1"
    );

    let loose = media_resilience::find_by_perceptual_hash(db, "00fc", 2)
        .await
        .expect("search at 2");
    assert_eq!(loose.len(), 1, "2 bits must match at a threshold of 2");
}

#[tokio::test]
async fn a_reference_with_no_perceptual_hash_is_never_matched() {
    let dir = scratch_dir("phash-null");
    let tdb = test_support::TestDb::connect_with_dir("mr-phash-null", &dir).await;
    let db = tdb.db();

    // perceptual_hash is nullable: a reference whose bytes were never fetched
    // has no hash. It must not match a query at any threshold, including one
    // large enough to accept every possible hash.
    media_resilience::insert_media_reference(db, "phash-none", "sha256:none", MediaKind::Image)
        .await
        .expect("insert none");

    let found = media_resilience::find_by_perceptual_hash(db, "00ff", 32)
        .await
        .expect("perceptual search");
    assert!(
        found.is_empty(),
        "a NULL perceptual_hash is not a hash to compare, got {:?}",
        found.iter().map(|r| &r.id).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn a_malformed_stored_hash_does_not_match_and_does_not_fail_the_search() {
    let dir = scratch_dir("phash-malformed");
    let tdb = test_support::TestDb::connect_with_dir("mr-phash-malformed", &dir).await;
    let db = tdb.db();

    media_resilience::insert_media_reference(db, "phash-bad", "sha256:bad", MediaKind::Image)
        .await
        .expect("insert bad");
    set_perceptual_hash(db, "phash-bad", "not-a-hash").await;
    media_resilience::insert_media_reference(db, "phash-good", "sha256:good", MediaKind::Image)
        .await
        .expect("insert good");
    set_perceptual_hash(db, "phash-good", "00ff").await;

    // The unparseable value is skipped rather than folded into a large
    // distance, and the healthy reference beside it is still returned: one
    // bad row must not hide every good match.
    let found = media_resilience::find_by_perceptual_hash(db, "00ff", 6)
        .await
        .expect("perceptual search");
    assert_eq!(
        found.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec!["phash-good"]
    );
}

#[tokio::test]
async fn a_malformed_query_hash_returns_nothing_rather_than_everything() {
    let dir = scratch_dir("phash-badquery");
    let tdb = test_support::TestDb::connect_with_dir("mr-phash-badquery", &dir).await;
    let db = tdb.db();

    media_resilience::insert_media_reference(db, "phash-row", "sha256:row", MediaKind::Image)
        .await
        .expect("insert row");
    set_perceptual_hash(db, "phash-row", "00ff").await;

    let found = media_resilience::find_by_perceptual_hash(db, "zzz", 32)
        .await
        .expect("perceptual search");
    assert!(
        found.is_empty(),
        "an unparseable query hash cannot match anything, got {:?}",
        found.iter().map(|r| &r.id).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn an_exact_match_is_still_found_and_sorts_first() {
    let dir = scratch_dir("phash-exact");
    let tdb = test_support::TestDb::connect_with_dir("mr-phash-exact", &dir).await;
    let db = tdb.db();

    media_resilience::insert_media_reference(db, "phash-exact", "sha256:e", MediaKind::Image)
        .await
        .expect("insert exact");
    set_perceptual_hash(db, "phash-exact", "00ff").await;
    media_resilience::insert_media_reference(db, "phash-nearby", "sha256:n", MediaKind::Image)
        .await
        .expect("insert nearby");
    set_perceptual_hash(db, "phash-nearby", "00fc").await;

    // Closest first: a curator reviewing candidates reads the strongest match
    // at the top, and the exact match is the one that can auto-attach.
    let found = media_resilience::find_by_perceptual_hash(db, "00ff", 6)
        .await
        .expect("perceptual search");
    assert_eq!(
        found.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec!["phash-exact", "phash-nearby"]
    );
}

/// A router with media-resilience settings the test chooses, so the config
/// actually reaches the route.
async fn build_app_with_resilience(dir: &Path, tune: impl FnOnce(&mut Config)) -> axum::Router {
    let mut config = config_for(dir);
    tune(&mut config);
    let tdb = test_support::TestDb::connect_with_dir("mr-cfg", dir).await;
    let db = tdb.db().clone();
    let state = AppState::new(config, db);
    set_trust_proxy(false);
    server::build_router(state)
}

async fn post_reverse_search(app: &axum::Router, body: &str) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/media/reverse-search")
                .header("content-type", "application/json")
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "reverse search should answer 200"
    );
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).expect("json body")
}

/// Build the router over one test DB, with a config the test can tune, and
/// hand the DB back so the test can seed it before issuing requests.
async fn app_and_db(
    dir: &Path,
    tune: impl FnOnce(&mut Config),
) -> (axum::Router, lorehaven_db::Database) {
    let mut config = config_for(dir);
    tune(&mut config);
    let tdb = test_support::TestDb::connect_with_dir("mr-rs", dir).await;
    let db = tdb.db().clone();
    let state = AppState::new(config, db.clone());
    set_trust_proxy(false);
    (server::build_router(state), db)
}

#[tokio::test]
async fn reverse_search_uses_the_configured_perceptual_threshold() {
    // A 2-bit difference (0x0 ^ 0xc): at or inside a threshold of 2 it is
    // found, and below it is not. The same data must answer differently under
    // each configuration, which is only true if the route reads config rather
    // than hardcoding a number.
    for (threshold, expected) in [(1, 0), (2, 1), (6, 1)] {
        let dir = scratch_dir(&format!("rs-threshold-{threshold}"));
        let (app, db) = app_and_db(&dir, |c| {
            c.media_resilience.perceptual_match_threshold = threshold;
        })
        .await;
        media_resilience::insert_media_reference(&db, "rs-t", "sha256:t", MediaKind::Image)
            .await
            .expect("insert");
        set_perceptual_hash(&db, "rs-t", "00ffff00").await;

        let body = post_reverse_search(&app, r#"{"hash":"00fcff00"}"#).await;
        let refs = body["references"].as_array().expect("references array");
        assert_eq!(
            refs.len(),
            expected,
            "at a threshold of {threshold} a 2-bit difference should yield {expected} match(es): {body}"
        );
    }
}

#[tokio::test]
async fn reverse_search_rejects_a_request_with_neither_url_nor_hash() {
    let dir = scratch_dir("rs-empty");
    let app = build_app_with_resilience(&dir, |_| {}).await;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/media/reverse-search")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"algorithm":"phash"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "a search with no hash and no url cannot be answered"
    );
}

#[tokio::test]
async fn reverse_search_reports_a_confidence_score_per_match() {
    let dir = scratch_dir("rs-confidence");
    let mut config = config_for(&dir);
    config.media_resilience.perceptual_match_threshold = 6;
    let tdb = test_support::TestDb::connect_with_dir("mr-rs-conf", &dir).await;
    let db = tdb.db().clone();
    // "rs-exact" stores the queried hash itself, so its distance is 0.
    media_resilience::insert_media_reference(&db, "rs-exact", "sha256:1", MediaKind::Image)
        .await
        .expect("insert exact");
    set_perceptual_hash(&db, "rs-exact", "00ff").await;
    // "rs-near" differs in two bits: 0x0 ^ 0xc is two bits, the rest matches.
    media_resilience::insert_media_reference(&db, "rs-near", "sha256:2", MediaKind::Image)
        .await
        .expect("insert near");
    set_perceptual_hash(&db, "rs-near", "00fc").await;

    let state = AppState::new(config, db);
    set_trust_proxy(false);
    let app = server::build_router(state);

    let body = post_reverse_search(&app, r#"{"hash":"00ff"}"#).await;
    let refs = body["references"].as_array().expect("references array");
    assert_eq!(refs.len(), 2, "both references are within 2 bits: {body}");

    // The spec requires a confidence score per perceptual match, and the exact
    // match must be the one a curator can act on first.
    for r in refs {
        assert!(
            r.get("match_confidence").is_some(),
            "each match carries a confidence score: {r}"
        );
        assert!(
            r.get("match_distance").is_some(),
            "each match carries the distance that produced the score: {r}"
        );
    }
    assert_eq!(
        refs[0]["id"], "rs-exact",
        "the closest match comes first: {body}"
    );
    let exact = refs[0]["match_confidence"].as_f64().expect("confidence");
    let near = refs[1]["match_confidence"].as_f64().expect("confidence");
    assert!(
        exact > near,
        "an exact match scores above a 2-bit match: {exact} vs {near}"
    );
}

// ---------------------------------------------------------------------------
// M32-07b: the fetched hashes are persisted (spec §32.7.2)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn recording_a_fingerprint_fills_the_hashes_it_carries() {
    let dir = scratch_dir("record-fp");
    let tdb = test_support::TestDb::connect_with_dir("mr-record", &dir).await;
    let db = tdb.db();

    media_resilience::insert_media_reference(db, "rec-1", "pending", MediaKind::Image)
        .await
        .expect("insert");

    let fp = MediaFingerprint {
        content_hash: "sha256:abc123".to_owned(),
        perceptual_hash: Some("00ff00ff00ff00ff".to_owned()),
        width: 800,
        height: 600,
    };
    media_resilience::record_fingerprint(db, "rec-1", &(&fp).into())
        .await
        .expect("record fingerprint");

    let stored = media_resilience::find_media_reference_by_id(db, "rec-1")
        .await
        .expect("find")
        .expect("exists");
    // The reference moves off the `pending` placeholder at the same time: an
    // exact content hash is what makes a reference findable by exact match, and
    // leaving `pending` there would mean the row claims to be unfetched while
    // carrying a real hash.
    assert_eq!(stored.content_hash, "sha256:abc123");
    assert_eq!(stored.perceptual_hash.as_deref(), Some("00ff00ff00ff00ff"));
    assert_eq!(stored.width, Some(800));
    assert_eq!(stored.height, Some(600));
}

#[tokio::test]
async fn recording_an_undecodable_body_stores_the_exact_hash_and_no_perceptual_one() {
    let dir = scratch_dir("record-nodec");
    let tdb = test_support::TestDb::connect_with_dir("mr-record-nodec", &dir).await;
    let db = tdb.db();

    media_resilience::insert_media_reference(db, "rec-2", "pending", MediaKind::Image)
        .await
        .expect("insert");

    // A build with no image decoder can still hash the bytes exactly. The
    // perceptual hash stays NULL - storing an empty string instead would make
    // the dedup search see distance 0 against every other undecodable image and
    // merge them all into one reference.
    let fp = MediaFingerprint::without_perceptual_hash(b"\x89PNG not decodable here");
    media_resilience::record_fingerprint(db, "rec-2", &(&fp).into())
        .await
        .expect("record fingerprint");

    let stored = media_resilience::find_media_reference_by_id(db, "rec-2")
        .await
        .expect("find")
        .expect("exists");
    assert!(stored.content_hash.starts_with("sha256:"));
    assert_eq!(stored.perceptual_hash, None);
}

#[tokio::test]
async fn an_undecodable_reference_is_never_returned_by_the_dedup_search() {
    let dir = scratch_dir("record-nodec-search");
    let tdb = test_support::TestDb::connect_with_dir("mr-record-ns", &dir).await;
    let db = tdb.db();

    for id in ["nodec-a", "nodec-b"] {
        media_resilience::insert_media_reference(db, id, "pending", MediaKind::Image)
            .await
            .expect("insert");
        let fp = MediaFingerprint::without_perceptual_hash(id.as_bytes());
        media_resilience::record_fingerprint(db, id, &(&fp).into())
            .await
            .expect("record");
    }

    // Two different images, neither fingerprinted. A NULL perceptual_hash must
    // not match anything, at any threshold - this is the failure mode that
    // makes an empty-string hash dangerous.
    let found = media_resilience::find_by_perceptual_hash(db, "00ff00ff00ff00ff", 32)
        .await
        .expect("search");
    assert!(
        found.is_empty(),
        "undecodable images must not match: {found:?}"
    );
}

#[tokio::test]
async fn a_recorded_fingerprint_is_found_by_the_search() {
    let dir = scratch_dir("record-found");
    let tdb = test_support::TestDb::connect_with_dir("mr-record-found", &dir).await;
    let db = tdb.db();

    media_resilience::insert_media_reference(db, "rec-3", "pending", MediaKind::Image)
        .await
        .expect("insert");
    let fp = MediaFingerprint {
        content_hash: "sha256:def".to_owned(),
        perceptual_hash: Some("00fc00fc00fc00fc".to_owned()),
        width: 64,
        height: 64,
    };
    media_resilience::record_fingerprint(db, "rec-3", &(&fp).into())
        .await
        .expect("record");

    // The round trip that makes the feature real: a hash written by the fetcher
    // is found by the search the curator runs. `00fc...` and `00ff...` differ in
    // exactly 8 bits (two per 16-bit word, four words), so the threshold has to
    // be at least 8 for this to match.
    let found = media_resilience::find_by_perceptual_hash(db, "00ff00ff00ff00ff", 8)
        .await
        .expect("search");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "rec-3");
}

#[tokio::test]
async fn recording_a_fingerprint_for_a_missing_reference_is_an_error_not_a_silent_no_op() {
    let dir = scratch_dir("record-missing");
    let tdb = test_support::TestDb::connect_with_dir("mr-record-missing", &dir).await;
    let db = tdb.db();

    let fp = MediaFingerprint::without_perceptual_hash(b"bytes");
    // A fetch that completes for a reference that no longer exists must say
    // so. Silently succeeding would report the media as mirrored when the row
    // it belongs to has gone.
    let outcome = media_resilience::record_fingerprint(db, "no-such-ref", &(&fp).into()).await;
    assert!(
        outcome.is_err(),
        "recording against a missing row must fail"
    );
}
