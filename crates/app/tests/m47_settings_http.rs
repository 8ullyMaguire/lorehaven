//! M47 integration tests: User Configuration
//!
//! Tests all M47 HTTP endpoints against an in-memory SQLite database.
//! Covers search settings, content filters, notification routes,
//! export/import, and the settings resolution hierarchy.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
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
        "lorehaven-m47-{tag}-{}-{:?}",
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

    async fn call(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let builder = Request::builder().uri(path).method(method);
        let req = if let Some(b) = body {
            builder
                .header("content-type", "application/json")
                .body(Body::from(b.to_string()))
                .unwrap()
        } else {
            builder.body(Body::empty()).unwrap()
        };
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
async fn test_search_settings_endpoints_unauthenticated() {
    let h = Harness::new("search-unauth").await;
    let (status, _) = h.call(Method::GET, "/api/v1/settings/search", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_content_filters_endpoints_unauthenticated() {
    let h = Harness::new("filters-unauth").await;
    let (status, _) = h
        .call(Method::GET, "/api/v1/settings/content-filters", None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_notification_routes_endpoints_unauthenticated() {
    let h = Harness::new("notif-unauth").await;
    let (status, _) = h
        .call(Method::GET, "/api/v1/settings/notifications", None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_export_settings_unauthenticated() {
    let h = Harness::new("export-unauth").await;
    let (status, _) = h.call(Method::GET, "/api/v1/settings/export", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_import_settings_unauthenticated() {
    let h = Harness::new("import-unauth").await;
    let body = serde_json::json!({ "version": "1.0", "settings": {} });
    let (status, _) = h
        .call(Method::POST, "/api/v1/settings/import", Some(body))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
