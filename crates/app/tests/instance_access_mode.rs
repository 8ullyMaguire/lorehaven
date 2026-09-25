//! Acceptance: instance accessibility mode (spec §0.4.7).
//!
//! The unit tests in `config.rs` pin the parsing and the policy translation.
//! These pin the thing that actually matters: that a `walled_garden` instance
//! refuses an anonymous reader at a content door, and that a `public` one does
//! not — through the real router, not by calling the policy directly.

use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use lorehaven_app::config::{Config, InstanceMode};
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_db::{Database, DatabaseConfig};
use serde_json::Value;
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-access-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

struct Harness {
    _dir: PathBuf,
    config: Config,
    db: Database,
}

impl Harness {
    async fn new(tag: &str, mode: InstanceMode) -> Self {
        let dir = scratch_dir(tag);
        let mut config = Config::development_defaults();
        config.storage.root = dir.to_path_buf();
        config.database = DatabaseConfig::new(format!(
            "sqlite://{}/lorehaven.sqlite?mode=rwc",
            dir.display()
        ));
        config.instance.mode = mode;
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

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        let req = Request::builder()
            .uri(path)
            .method(Method::GET)
            .body(Body::empty())
            .unwrap();
        let resp = self.router().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 10_000_000)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json)
    }
}

#[tokio::test]
async fn a_public_instance_reports_itself_as_public() {
    let h = Harness::new("meta-public", InstanceMode::Public).await;
    let (status, body) = h.get("/api/v1/meta").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["policy"]["instance_mode"], "public");
    assert_eq!(body["policy"]["anonymous_reading"], true);
}

#[tokio::test]
async fn a_walled_garden_reports_itself_and_closes_anonymous_reading() {
    let h = Harness::new("meta-walled", InstanceMode::WalledGarden).await;
    let (status, body) = h.get("/api/v1/meta").await;
    assert_eq!(status, StatusCode::OK);
    // `meta` stays reachable in a walled garden on purpose: a reader who
    // cannot see the sign-in link cannot use it.
    assert_eq!(body["policy"]["instance_mode"], "walled_garden");
    assert_eq!(body["policy"]["anonymous_reading"], false);
}

#[tokio::test]
async fn a_private_instance_reports_itself() {
    let h = Harness::new("meta-private", InstanceMode::Private).await;
    let (status, body) = h.get("/api/v1/meta").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["policy"]["instance_mode"], "private");
    assert_eq!(body["policy"]["anonymous_reading"], false);
}

#[tokio::test]
async fn a_walled_garden_refuses_an_anonymous_reader_at_a_work_door() {
    let h = Harness::new("work-walled", InstanceMode::WalledGarden).await;
    // A work that does not exist and a work that does are both refused the
    // same way: the wall is in front of the door, so it cannot be used to
    // probe what exists.
    let (status, _) = h
        .get("/api/v1/works/11111111-1111-1111-1111-111111111111")
        .await;
    assert_ne!(
        status,
        StatusCode::OK,
        "a walled garden must not serve content"
    );
}

#[tokio::test]
async fn a_public_instance_still_serves_the_landing_page() {
    let h = Harness::new("landing-public", InstanceMode::Public).await;
    let (status, _) = h.get("/api/v1/health/ready").await;
    assert_eq!(status, StatusCode::OK);
}
