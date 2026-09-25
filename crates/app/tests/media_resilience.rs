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
use test_support::id;
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

/// The router, plus the database its `AppState` actually holds.
///
/// The two have to travel together: under PostgreSQL each `TestDb` creates its
/// own scratch database, so a fixture written through a second `TestDb` would
/// land in a database the router never queries, and the test would fail for
/// reasons that have nothing to do with the code under test. Under SQLite the
/// scratch file is shared, which hides the mistake.
async fn build_app(dir: &Path) -> (axum::Router, test_support::TestDb) {
    let config = config_for(dir);
    let tdb = test_support::TestDb::connect_with_dir("mr", dir).await;
    let state = AppState::new(config, tdb.db().clone());
    set_trust_proxy(false);
    (server::build_router(state), tdb)
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

    let ref_id = id("test-ref-001");
    media_resilience::insert_media_reference(
        db,
        &ref_id,
        "hash-abc",
        lorehaven_domain::media_resilience::MediaKind::Image,
    )
    .await
    .expect("insert reference");

    let reference = media_resilience::find_media_reference_by_id(db, &ref_id)
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

    let ref_id = id("test-ref-002");
    media_resilience::insert_media_reference(
        db,
        &ref_id,
        "hash-def",
        lorehaven_domain::media_resilience::MediaKind::Image,
    )
    .await
    .expect("insert reference");

    let link_id = id("test-link-001");
    media_resilience::insert_availability_link(
        db,
        &link_id,
        &ref_id,
        "https://example.com/image.jpg",
        lorehaven_domain::media_resilience::LinkProvider::Other,
        Some(&id("user-001")),
        100,
    )
    .await
    .expect("insert link");

    let links = media_resilience::find_availability_links_for_reference(db, &ref_id)
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
        &link_id,
        lorehaven_domain::media_resilience::LinkStatus::Healthy,
        0,
    )
    .await
    .expect("update status");

    let healthy = media_resilience::count_healthy_links(db, &ref_id)
        .await
        .expect("count healthy");
    assert_eq!(healthy, 1);
}

#[tokio::test]
async fn media_resilience_curator_rewards() {
    let dir = scratch_dir("rewards");
    let tdb = test_support::TestDb::connect_with_dir("mr-rewards", &dir).await;
    let db = tdb.db();

    let ref_id = id("test-ref-003");
    let link_id = id("test-link-003");
    media_resilience::insert_media_reference(
        db,
        &ref_id,
        "hash-ghi",
        lorehaven_domain::media_resilience::MediaKind::Image,
    )
    .await
    .expect("insert reference");
    media_resilience::insert_availability_link(
        db,
        &link_id,
        &ref_id,
        "https://example.com/pic.png",
        lorehaven_domain::media_resilience::LinkProvider::Imgur,
        None,
        50,
    )
    .await
    .expect("insert link");

    media_resilience::insert_curator_reward(
        db,
        &id("user-001"),
        lorehaven_domain::media_resilience::CuratorAction::MirrorAdd,
        Some(&ref_id),
        Some(&link_id),
        15,
    )
    .await
    .expect("insert reward");

    let total = media_resilience::sum_curator_rewards_today(db, &id("user-001"))
        .await
        .expect("sum rewards");
    assert_eq!(total, 15);
}

#[tokio::test]
async fn media_resilience_get_route() {
    let dir = scratch_dir("route");
    let (app, tdb) = build_app(&dir).await;
    let db = tdb.db();

    let ref_id = id("test-ref-004");
    media_resilience::insert_media_reference(
        db,
        &ref_id,
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
    let (app, _tdb) = build_app(&dir).await;

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

    let ref_id = id("test-ref-005");
    media_resilience::insert_media_reference(
        db,
        &ref_id,
        "hash-mno",
        lorehaven_domain::media_resilience::MediaKind::Image,
    )
    .await
    .expect("insert reference");

    media_resilience::insert_availability_link(
        db,
        &id("link-005"),
        &ref_id,
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
        // `media_references.id` is UUID on PostgreSQL.
        Backend::Postgres => "UPDATE media_references SET perceptual_hash = $1 WHERE id = $2::uuid",
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
    media_resilience::insert_media_reference(
        db,
        &id("phash-near"),
        "sha256:near",
        MediaKind::Image,
    )
    .await
    .expect("insert near");
    set_perceptual_hash(db, &id("phash-near"), "00ffff00").await;

    let found = media_resilience::find_by_perceptual_hash(db, "00fcff00", 6)
        .await
        .expect("perceptual search");
    assert_eq!(
        found.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec![id("phash-near")],
        "a 4-bit difference is inside a threshold of 6"
    );
}

#[tokio::test]
async fn a_perceptual_match_outside_the_threshold_is_not_found() {
    let dir = scratch_dir("phash-outside");
    let tdb = test_support::TestDb::connect_with_dir("mr-phash-outside", &dir).await;
    let db = tdb.db();

    // 12 bits differ, well past a threshold of 6.
    media_resilience::insert_media_reference(db, &id("phash-far"), "sha256:far", MediaKind::Image)
        .await
        .expect("insert far");
    set_perceptual_hash(db, &id("phash-far"), "00ff00ff").await;

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
    media_resilience::insert_media_reference(db, &id("phash-two"), "sha256:two", MediaKind::Image)
        .await
        .expect("insert two");
    set_perceptual_hash(db, &id("phash-two"), "00ff").await;

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
    media_resilience::insert_media_reference(
        db,
        &id("phash-none"),
        "sha256:none",
        MediaKind::Image,
    )
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

    media_resilience::insert_media_reference(db, &id("phash-bad"), "sha256:bad", MediaKind::Image)
        .await
        .expect("insert bad");
    set_perceptual_hash(db, &id("phash-bad"), "not-a-hash").await;
    media_resilience::insert_media_reference(
        db,
        &id("phash-good"),
        "sha256:good",
        MediaKind::Image,
    )
    .await
    .expect("insert good");
    set_perceptual_hash(db, &id("phash-good"), "00ff").await;

    // The unparseable value is skipped rather than folded into a large
    // distance, and the healthy reference beside it is still returned: one
    // bad row must not hide every good match.
    let found = media_resilience::find_by_perceptual_hash(db, "00ff", 6)
        .await
        .expect("perceptual search");
    assert_eq!(
        found.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec![id("phash-good")]
    );
}

#[tokio::test]
async fn a_malformed_query_hash_returns_nothing_rather_than_everything() {
    let dir = scratch_dir("phash-badquery");
    let tdb = test_support::TestDb::connect_with_dir("mr-phash-badquery", &dir).await;
    let db = tdb.db();

    media_resilience::insert_media_reference(db, &id("phash-row"), "sha256:row", MediaKind::Image)
        .await
        .expect("insert row");
    set_perceptual_hash(db, &id("phash-row"), "00ff").await;

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

    media_resilience::insert_media_reference(db, &id("phash-exact"), "sha256:e", MediaKind::Image)
        .await
        .expect("insert exact");
    set_perceptual_hash(db, &id("phash-exact"), "00ff").await;
    media_resilience::insert_media_reference(db, &id("phash-nearby"), "sha256:n", MediaKind::Image)
        .await
        .expect("insert nearby");
    set_perceptual_hash(db, &id("phash-nearby"), "00fc").await;

    // Closest first: a curator reviewing candidates reads the strongest match
    // at the top, and the exact match is the one that can auto-attach.
    let found = media_resilience::find_by_perceptual_hash(db, "00ff", 6)
        .await
        .expect("perceptual search");
    assert_eq!(
        found.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec![id("phash-exact"), id("phash-nearby")]
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
        media_resilience::insert_media_reference(&db, &id("rs-t"), "sha256:t", MediaKind::Image)
            .await
            .expect("insert");
        set_perceptual_hash(&db, &id("rs-t"), "00ffff00").await;

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
    media_resilience::insert_media_reference(&db, &id("rs-exact"), "sha256:1", MediaKind::Image)
        .await
        .expect("insert exact");
    set_perceptual_hash(&db, &id("rs-exact"), "00ff").await;
    // "rs-near" differs in two bits: 0x0 ^ 0xc is two bits, the rest matches.
    media_resilience::insert_media_reference(&db, &id("rs-near"), "sha256:2", MediaKind::Image)
        .await
        .expect("insert near");
    set_perceptual_hash(&db, &id("rs-near"), "00fc").await;

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
        refs[0]["id"].as_str().unwrap(),
        id("rs-exact"),
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

    media_resilience::insert_media_reference(db, &id("rec-1"), "pending", MediaKind::Image)
        .await
        .expect("insert");

    let fp = MediaFingerprint {
        content_hash: "sha256:abc123".to_owned(),
        perceptual_hash: Some("00ff00ff00ff00ff".to_owned()),
        width: 800,
        height: 600,
    };
    media_resilience::record_fingerprint(db, &id("rec-1"), &(&fp).into())
        .await
        .expect("record fingerprint");

    let stored = media_resilience::find_media_reference_by_id(db, &id("rec-1"))
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

    media_resilience::insert_media_reference(db, &id("rec-2"), "pending", MediaKind::Image)
        .await
        .expect("insert");

    // A build with no image decoder can still hash the bytes exactly. The
    // perceptual hash stays NULL - storing an empty string instead would make
    // the dedup search see distance 0 against every other undecodable image and
    // merge them all into one reference.
    let fp = MediaFingerprint::without_perceptual_hash(b"\x89PNG not decodable here");
    media_resilience::record_fingerprint(db, &id("rec-2"), &(&fp).into())
        .await
        .expect("record fingerprint");

    let stored = media_resilience::find_media_reference_by_id(db, &id("rec-2"))
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

    for ref_id in [id("nodec-a"), id("nodec-b")] {
        media_resilience::insert_media_reference(db, &ref_id, "pending", MediaKind::Image)
            .await
            .expect("insert");
        let fp = MediaFingerprint::without_perceptual_hash(ref_id.as_bytes());
        media_resilience::record_fingerprint(db, &ref_id, &(&fp).into())
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

    media_resilience::insert_media_reference(db, &id("rec-3"), "pending", MediaKind::Image)
        .await
        .expect("insert");
    let fp = MediaFingerprint {
        content_hash: "sha256:def".to_owned(),
        perceptual_hash: Some("00fc00fc00fc00fc".to_owned()),
        width: 64,
        height: 64,
    };
    media_resilience::record_fingerprint(db, &id("rec-3"), &(&fp).into())
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
    assert_eq!(found[0].id, id("rec-3"));
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
    let outcome = media_resilience::record_fingerprint(db, &id("no-such-ref"), &(&fp).into()).await;
    assert!(
        outcome.is_err(),
        "recording against a missing row must fail"
    );
}

// ---------------------------------------------------------------------------
// §32.7.2 Perceptual match proposals: the curator decision the spec requires.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_perceptual_match_is_proposed_with_its_confidence() {
    let dir = scratch_dir("propose");
    let tdb = test_support::TestDb::connect_with_dir("mr-propose", &dir).await;
    let db = tdb.db();
    let existing = id("existing-ref");
    let candidate = id("candidate-ref");
    for (rid, hash) in [(&existing, "sha256:aaa"), (&candidate, "sha256:bbb")] {
        media_resilience::insert_media_reference(
            db,
            rid,
            hash,
            lorehaven_domain::media_resilience::MediaKind::Image,
        )
        .await
        .expect("insert reference");
    }

    // Distance 2 out of 64 bits, which the spec's threshold of 6 admits.
    assert!(media_resilience::record_match_proposal(
        db,
        &candidate,
        &existing,
        "sha256:bbb",
        Some("0123456789abcdef"),
        2,
    )
    .await
    .expect("record proposal"));

    let pending = media_resilience::list_pending_match_proposals(db, 50)
        .await
        .expect("list pending");
    assert_eq!(pending.len(), 1);
    let proposal = &pending[0].proposal;
    assert_eq!(proposal.candidate_reference_id, candidate);
    assert_eq!(proposal.existing_reference_id, existing);
    assert_eq!(proposal.hamming_distance, 2);
    assert_eq!(proposal.status, "pending");
    // Confidence is stored, not recomputed, so the curator sees what the search
    // used. 2 bits apart in a 64-bit hash is 62/64.
    let expected = lorehaven_domain::media_resilience::perceptual_match_confidence(2);
    assert!((proposal.match_confidence - expected).abs() < 1e-9);
    assert!(
        proposal.match_confidence > 0.9,
        "{}",
        proposal.match_confidence
    );
    assert_eq!(
        media_resilience::count_pending_match_proposals(db)
            .await
            .expect("count"),
        1
    );
}

#[tokio::test]
async fn re_proposing_the_same_pair_updates_rather_than_duplicates() {
    let dir = scratch_dir("propose-idem");
    let tdb = test_support::TestDb::connect_with_dir("mr-propose-idem", &dir).await;
    let db = tdb.db();
    let existing = id("existing-ref");
    let candidate = id("candidate-ref");
    for (rid, hash) in [(&existing, "sha256:aaa"), (&candidate, "sha256:bbb")] {
        media_resilience::insert_media_reference(
            db,
            rid,
            hash,
            lorehaven_domain::media_resilience::MediaKind::Image,
        )
        .await
        .expect("insert reference");
    }

    for distance in [1_u32, 3, 5] {
        assert!(media_resilience::record_match_proposal(
            db,
            &candidate,
            &existing,
            "sha256:bbb",
            Some("0123456789abcdef"),
            distance,
        )
        .await
        .expect("record proposal"));
    }
    // A fetch re-runs and re-hashes constantly. Three proposals for one pair of
    // images would be three rows in the curator's queue describing one question.
    let pending = media_resilience::list_pending_match_proposals(db, 50)
        .await
        .expect("list pending");
    assert_eq!(pending.len(), 1);
    // The latest search result wins.
    assert_eq!(pending[0].proposal.hamming_distance, 5);
}

#[tokio::test]
async fn confirming_a_proposal_moves_the_links_to_the_existing_reference() {
    let dir = scratch_dir("confirm");
    let tdb = test_support::TestDb::connect_with_dir("mr-confirm", &dir).await;
    let db = tdb.db();
    let existing = id("existing-ref");
    let candidate = id("candidate-ref");
    for (rid, hash) in [(&existing, "sha256:aaa"), (&candidate, "sha256:bbb")] {
        media_resilience::insert_media_reference(
            db,
            rid,
            hash,
            lorehaven_domain::media_resilience::MediaKind::Image,
        )
        .await
        .expect("insert reference");
    }
    let existing_link = id("existing-link");
    let candidate_link = id("candidate-link");
    media_resilience::insert_availability_link(
        db,
        &existing_link,
        &existing,
        "https://cdn.example.com/original.png",
        lorehaven_domain::media_resilience::LinkProvider::Other,
        None,
        100,
    )
    .await
    .expect("insert existing link");
    media_resilience::insert_availability_link(
        db,
        &candidate_link,
        &candidate,
        "https://cdn.example.com/reencode.png",
        lorehaven_domain::media_resilience::LinkProvider::Other,
        None,
        100,
    )
    .await
    .expect("insert candidate link");

    media_resilience::record_match_proposal(
        db,
        &candidate,
        &existing,
        "sha256:bbb",
        Some("0123456789abcdef"),
        1,
    )
    .await
    .expect("record proposal");
    let pending = media_resilience::list_pending_match_proposals(db, 50)
        .await
        .expect("list pending");
    let proposal_id = pending[0].proposal.id.clone();

    assert!(media_resilience::resolve_match_proposal(
        db,
        &proposal_id,
        media_resilience::ProposalDecision::Confirm,
        &id("curator-001"),
        Some("same artwork, re-encoded"),
    )
    .await
    .expect("resolve"));

    // The point of deduplicating: both copies of the image now hang off one
    // reference, so one dying link does not take the other with it.
    let links = media_resilience::find_availability_links_for_reference(db, &existing)
        .await
        .expect("links on existing");
    assert_eq!(
        links.len(),
        2,
        "both links moved to the surviving reference"
    );
    let mut urls: Vec<String> = links.iter().map(|l| l.url.clone()).collect();
    urls.sort();
    assert_eq!(
        urls,
        vec![
            "https://cdn.example.com/original.png".to_string(),
            "https://cdn.example.com/reencode.png".to_string(),
        ]
    );

    // And the duplicate reference is gone rather than left as an orphan.
    assert!(media_resilience::find_media_reference_by_id(db, &candidate)
        .await
        .expect("look up candidate")
        .is_none());

    // The queue is empty. Note the proposal row itself does not survive: both
    // foreign keys on media_match_proposals are ON DELETE CASCADE, so confirming
    // deletes the candidate reference and the proposal goes with it. That is
    // deliberate -- a merged pair has no question left -- but it means the audit
    // trail for a confirmation is the surviving reference's link set, not the
    // proposal.
    assert_eq!(
        media_resilience::count_pending_match_proposals(db)
            .await
            .expect("count"),
        0
    );
}

#[tokio::test]
async fn rejecting_a_proposal_keeps_the_references_apart() {
    let dir = scratch_dir("reject");
    let tdb = test_support::TestDb::connect_with_dir("mr-reject", &dir).await;
    let db = tdb.db();
    let existing = id("existing-ref");
    let candidate = id("candidate-ref");
    for (rid, hash) in [(&existing, "sha256:aaa"), (&candidate, "sha256:bbb")] {
        media_resilience::insert_media_reference(
            db,
            rid,
            hash,
            lorehaven_domain::media_resilience::MediaKind::Image,
        )
        .await
        .expect("insert reference");
    }
    let candidate_link = id("candidate-link");
    media_resilience::insert_availability_link(
        db,
        &candidate_link,
        &candidate,
        "https://cdn.example.com/different.png",
        lorehaven_domain::media_resilience::LinkProvider::Other,
        None,
        100,
    )
    .await
    .expect("insert candidate link");

    media_resilience::record_match_proposal(
        db,
        &candidate,
        &existing,
        "sha256:bbb",
        Some("0123456789abcdef"),
        6,
    )
    .await
    .expect("record proposal");
    let proposal_id = media_resilience::list_pending_match_proposals(db, 50)
        .await
        .expect("list pending")[0]
        .proposal
        .id
        .clone();

    assert!(media_resilience::resolve_match_proposal(
        db,
        &proposal_id,
        media_resilience::ProposalDecision::Reject,
        &id("curator-001"),
        Some("different artwork that happens to share structure"),
    )
    .await
    .expect("resolve"));

    // The candidate survives a rejection, and keeps its own link.
    assert!(media_resilience::find_media_reference_by_id(db, &candidate)
        .await
        .expect("look up candidate")
        .is_some());
    assert_eq!(
        media_resilience::count_total_links(db, &candidate)
            .await
            .expect("links"),
        1
    );
    assert_eq!(
        media_resilience::count_pending_match_proposals(db)
            .await
            .expect("count"),
        0
    );
}

#[tokio::test]
async fn a_rejected_pair_is_not_proposed_again() {
    let dir = scratch_dir("reject-again");
    let tdb = test_support::TestDb::connect_with_dir("mr-reject-again", &dir).await;
    let db = tdb.db();
    let existing = id("existing-ref");
    let candidate = id("candidate-ref");
    for (rid, hash) in [(&existing, "sha256:aaa"), (&candidate, "sha256:bbb")] {
        media_resilience::insert_media_reference(
            db,
            rid,
            hash,
            lorehaven_domain::media_resilience::MediaKind::Image,
        )
        .await
        .expect("insert reference");
    }
    media_resilience::record_match_proposal(
        db,
        &candidate,
        &existing,
        "sha256:bbb",
        Some("0123456789abcdef"),
        4,
    )
    .await
    .expect("record proposal");
    let proposal_id = media_resilience::list_pending_match_proposals(db, 50)
        .await
        .expect("list pending")[0]
        .proposal
        .id
        .clone();
    media_resilience::resolve_match_proposal(
        db,
        &proposal_id,
        media_resilience::ProposalDecision::Reject,
        &id("curator-001"),
        None,
    )
    .await
    .expect("reject");

    // The next fetch of the same image finds the same near-match. Re-asking
    // would make a rejection impossible to honour, which is why the row is kept
    // rather than deleted.
    let reproposed = media_resilience::record_match_proposal(
        db,
        &candidate,
        &existing,
        "sha256:bbb",
        Some("0123456789abcdef"),
        4,
    )
    .await
    .expect("re-propose");
    assert!(!reproposed, "a rejected pair must not re-enter the queue");
    assert_eq!(
        media_resilience::count_pending_match_proposals(db)
            .await
            .expect("count"),
        0
    );
}

#[tokio::test]
async fn resolving_twice_moves_nothing_the_second_time() {
    let dir = scratch_dir("double-resolve");
    let tdb = test_support::TestDb::connect_with_dir("mr-double", &dir).await;
    let db = tdb.db();
    let existing = id("existing-ref");
    let candidate = id("candidate-ref");
    for (rid, hash) in [(&existing, "sha256:aaa"), (&candidate, "sha256:bbb")] {
        media_resilience::insert_media_reference(
            db,
            rid,
            hash,
            lorehaven_domain::media_resilience::MediaKind::Image,
        )
        .await
        .expect("insert reference");
    }
    media_resilience::record_match_proposal(
        db,
        &candidate,
        &existing,
        "sha256:bbb",
        Some("0123456789abcdef"),
        2,
    )
    .await
    .expect("record proposal");
    let proposal_id = media_resilience::list_pending_match_proposals(db, 50)
        .await
        .expect("list pending")[0]
        .proposal
        .id
        .clone();

    assert!(media_resilience::resolve_match_proposal(
        db,
        &proposal_id,
        media_resilience::ProposalDecision::Reject,
        &id("curator-001"),
        None,
    )
    .await
    .expect("first resolve"));
    // Two curators clicking at once, or a retried request. The second must be a
    // no-op rather than a second merge.
    assert!(!media_resilience::resolve_match_proposal(
        db,
        &proposal_id,
        media_resilience::ProposalDecision::Confirm,
        &id("curator-002"),
        None,
    )
    .await
    .expect("second resolve"));
    // The rejection stands: a confirm must not be able to overturn it.
    assert!(media_resilience::find_media_reference_by_id(db, &candidate)
        .await
        .expect("look up candidate")
        .is_some());
}

#[tokio::test]
async fn resolving_an_unknown_proposal_reports_nothing_to_do() {
    let dir = scratch_dir("unknown");
    let tdb = test_support::TestDb::connect_with_dir("mr-unknown", &dir).await;
    let db = tdb.db();
    // A well-formed id that was never proposed, and one that is not an id at all.
    // Both are the same thing to a curator: there is nothing here to act on.
    assert!(!media_resilience::resolve_match_proposal(
        db,
        &uuid::Uuid::new_v4().to_string(),
        media_resilience::ProposalDecision::Confirm,
        &id("curator-001"),
        None,
    )
    .await
    .expect("unknown id"));
    assert!(!media_resilience::resolve_match_proposal(
        db,
        "not-a-uuid",
        media_resilience::ProposalDecision::Confirm,
        &id("curator-001"),
        None,
    )
    .await
    .expect("malformed id"));
}

#[tokio::test]
async fn the_curator_queue_is_ordered_by_confidence_and_bounded() {
    let dir = scratch_dir("queue");
    let tdb = test_support::TestDb::connect_with_dir("mr-queue", &dir).await;
    let db = tdb.db();
    let existing = id("existing-ref");
    media_resilience::insert_media_reference(
        db,
        &existing,
        "sha256:aaa",
        lorehaven_domain::media_resilience::MediaKind::Image,
    )
    .await
    .expect("insert reference");
    // Three candidates at widening distances, so the ordering is a real test of
    // the sort rather than of insertion order.
    for (i, distance) in [6_u32, 1, 3].into_iter().enumerate() {
        let candidate = id(&format!("candidate-{i}"));
        media_resilience::insert_media_reference(
            db,
            &candidate,
            &format!("sha256:c{i}"),
            lorehaven_domain::media_resilience::MediaKind::Image,
        )
        .await
        .expect("insert candidate");
        media_resilience::record_match_proposal(
            db,
            &candidate,
            &existing,
            &format!("sha256:c{i}"),
            Some("0123456789abcdef"),
            distance,
        )
        .await
        .expect("record proposal");
    }
    let pending = media_resilience::list_pending_match_proposals(db, 50)
        .await
        .expect("list pending");
    let distances: Vec<i32> = pending
        .iter()
        .map(|p| p.proposal.hamming_distance)
        .collect();
    assert_eq!(distances, vec![1, 3, 6], "closest match first");

    // A limit is not a suggestion: a fetch storm on one popular image would
    // otherwise grow the queue without bound.
    let top = media_resilience::list_pending_match_proposals(db, 2)
        .await
        .expect("list top 2");
    assert_eq!(top.len(), 2);
    // A negative or zero limit must not mean "no limit" in either dialect.
    for limit in [-1_i64, 0] {
        let bounded = media_resilience::list_pending_match_proposals(db, limit)
            .await
            .expect("list with bad limit");
        assert!(!bounded.is_empty() && bounded.len() <= 2, "limit {limit}");
    }
}

#[tokio::test]
async fn a_malformed_proposal_id_is_rejected_before_a_round_trip() {
    assert!(!media_resilience::match_proposal_id_is_valid("not-a-uuid"));
    assert!(!media_resilience::match_proposal_id_is_valid(""));
    assert!(media_resilience::match_proposal_id_is_valid(
        &uuid::Uuid::new_v4().to_string()
    ));
}

#[tokio::test]
async fn an_exact_content_match_attaches_without_asking_anyone() {
    let dir = scratch_dir("exact");
    let tdb = test_support::TestDb::connect_with_dir("mr-exact", &dir).await;
    let db = tdb.db();
    let existing = id("existing-ref");
    let candidate = id("candidate-ref");
    // Same bytes, so the same content hash. The spec calls this the branch that
    // needs no human: "Attach as a new AvailabilityLink to the existing
    // MediaReference. Zero new storage cost."
    for rid in [&existing, &candidate] {
        media_resilience::insert_media_reference(
            db,
            rid,
            "sha256:same",
            lorehaven_domain::media_resilience::MediaKind::Image,
        )
        .await
        .expect("insert reference");
    }
    for (rid, url) in [
        (&existing, "https://cdn.example.com/a.png"),
        (&candidate, "https://cdn.example.com/b.png"),
    ] {
        media_resilience::insert_availability_link(
            db,
            &id(url),
            rid,
            url,
            lorehaven_domain::media_resilience::LinkProvider::Other,
            None,
            100,
        )
        .await
        .expect("insert link");
    }

    let found = media_resilience::find_media_reference_by_content_hash(db, "sha256:same")
        .await
        .expect("lookup by content hash");
    let found = found.expect("a reference holds those bytes");

    assert!(
        media_resilience::attach_reference_to_existing(db, &found.id, &candidate)
            .await
            .expect("attach")
    );

    // One reference, two links: the spec's claim that a popular image has one
    // MediaReference and many AvailabilityLinks.
    assert!(media_resilience::find_media_reference_by_id(db, &candidate)
        .await
        .expect("look up candidate")
        .is_none());
    assert_eq!(
        media_resilience::count_total_links(db, &found.id)
            .await
            .expect("links"),
        2
    );
}

#[tokio::test]
async fn attaching_a_reference_to_itself_does_nothing() {
    let dir = scratch_dir("self-attach");
    let tdb = test_support::TestDb::connect_with_dir("mr-self", &dir).await;
    let db = tdb.db();
    let reference = id("only-ref");
    media_resilience::insert_media_reference(
        db,
        &reference,
        "sha256:only",
        lorehaven_domain::media_resilience::MediaKind::Image,
    )
    .await
    .expect("insert reference");

    // A re-fetch of the same URL is the ordinary case, not an error, and must not
    // delete the reference it just fetched.
    assert!(
        !media_resilience::attach_reference_to_existing(db, &reference, &reference)
            .await
            .expect("self attach")
    );
    assert!(media_resilience::find_media_reference_by_id(db, &reference)
        .await
        .expect("look up")
        .is_some());
}

#[tokio::test]
async fn attaching_to_a_missing_reference_refuses_rather_than_half_moves() {
    let dir = scratch_dir("attach-missing");
    let tdb = test_support::TestDb::connect_with_dir("mr-attach-miss", &dir).await;
    let db = tdb.db();
    let candidate = id("candidate-ref");
    media_resilience::insert_media_reference(
        db,
        &candidate,
        "sha256:cand",
        lorehaven_domain::media_resilience::MediaKind::Image,
    )
    .await
    .expect("insert reference");
    media_resilience::insert_availability_link(
        db,
        &id("candidate-link"),
        &candidate,
        "https://cdn.example.com/c.png",
        lorehaven_domain::media_resilience::LinkProvider::Other,
        None,
        100,
    )
    .await
    .expect("insert link");

    // The surviving reference does not exist. Repointing the links anyway would
    // leave them pointing at nothing, which is the one outcome worse than not
    // deduplicating.
    let ghost = uuid::Uuid::new_v4().to_string();
    assert!(
        !media_resilience::attach_reference_to_existing(db, &ghost, &candidate)
            .await
            .expect("attach to ghost")
    );
    // Nothing moved.
    assert!(media_resilience::find_media_reference_by_id(db, &candidate)
        .await
        .expect("look up")
        .is_some());
    assert_eq!(
        media_resilience::count_total_links(db, &candidate)
            .await
            .expect("links"),
        1
    );
}
