//! M45 Roadmap Consensus acceptance (spec §44).
//!
//! Full integration tests require running instance with seeded cards and
//! authenticated users — see scripts/seed_roadmap.py and the board UI tests
//! for those flows. Here we verify the public board endpoint works after
//! migration 0067.

use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_db::Database;
use lorehaven_db::DatabaseConfig;
use serde_json::{json, Value};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-roadmap-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn config_for(dir: &PathBuf) -> Config {
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
        let db = Database::connect(&config.database).await.expect("db connect");
        db.migrate().await.expect("migrations");
        Self { _dir: dir, config, db }
    }

    fn router(&self) -> axum::Router {
        server::build_router(AppState::new(self.config.clone(), self.db.clone()))
    }
}

#[tokio::test]
async fn test_get_board_public() {
    let h = Harness::new("board-public").await;
    let resp = h.router()
        .oneshot(Request::builder().uri("/api/v1/roadmap").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 10_000_000).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(json["board"].is_object());
}

#[tokio::test]
async fn test_changelog_empty_public() {
    let h = Harness::new("changelog-empty").await;
    let resp = h.router()
        .oneshot(Request::builder().uri("/api/v1/roadmap/changelog").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 10_000_000).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    let moves = json["moves"].as_array().unwrap();
    assert!(moves.is_empty(), "fresh install has no moves");
}
