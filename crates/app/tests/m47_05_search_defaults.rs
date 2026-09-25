//! M47-05 acceptance: search defaults pre-populating every search surface.
//!
//! These tests verify the search endpoint response shape includes the
//! `filters` object (min_words, max_words, sort, max_rating) so the
//! frontend can render the active filters and the user can see/override them.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_db::Database;
use lorehaven_db::DatabaseConfig;
use serde_json::Value;
use std::path::Path;
use std::path::PathBuf;
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m47-05-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn config_for(dir: &Path) -> Config {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    config.database = DatabaseConfig::new(format!(
        "sqlite://{}/lorehaven.sqlite?mode=rwc",
        dir.display()
    ));
    config
}

struct Harness {
    _dir: PathBuf,
    config: Config,
    db: Database,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let config = config_for(&dir);
        let db = Database::connect(&config.database)
            .await
            .expect("db connect");
        db.migrate().await.expect("migrations");
        Self {
            _dir: dir,
            config,
            db,
        }
    }

    fn router(&self) -> axum::Router {
        server::build_router(AppState::new(self.config.clone(), self.db.clone()))
    }
}

#[tokio::test]
async fn test_search_returns_filters_shape() {
    let h = Harness::new("search-filters-shape").await;
    let resp = h
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/v1/search?q=any")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 10_000_000)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    // Without auth, no defaults apply — but the response shape must include
    // the filters object with all four default fields (spec §46.4).
    assert!(json["items"].is_array());
    assert!(json["filters"].is_object());
    assert!(json["filters"]["min_words"].is_null());
    assert!(json["filters"]["max_words"].is_null());
    assert!(json["filters"]["sort"].is_null());
    assert!(json["filters"]["max_rating"].is_null());
}

#[tokio::test]
async fn test_search_empty_results_includes_filters() {
    let h = Harness::new("search-empty-results").await;
    let resp = h
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/v1/search?q=zzzznonexistentwork")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 10_000_000)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(json["items"].as_array().unwrap().is_empty());
    assert!(json["filters"].is_object());
}

#[tokio::test]
async fn test_search_with_limit_param() {
    let h = Harness::new("search-limit").await;
    let resp = h
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/v1/search?q=test&limit=5")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 10_000_000)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(json["items"].is_array());
    assert!(json["filters"].is_object());
}

#[tokio::test]
async fn test_search_no_query_returns_ok() {
    let h = Harness::new("search-no-query").await;
    let resp = h
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/v1/search")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 10_000_000)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(json["items"].is_array());
    assert!(json["filters"].is_object());
}
