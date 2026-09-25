//! Media resilience & availability guarantee (spec §32.7).
//!
//! These tests drive the real router against a real SQLite file.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::media_resilience;
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
