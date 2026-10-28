//! `GET /api/v1/taxonomy/export` — the vocabulary as a document (T2.1).
//!
//! The contract is the shape and the version string, so that is what is tested: a page carries the identity
//! and both spellings of each entity, `version` changes when the vocabulary does, `kind` narrows, and a bad
//! parameter is a 422 rather than a 500 or a silent default.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use darkhold_app::config::Config;
use darkhold_app::server;
use darkhold_app::state::AppState;
use darkhold_db::{Database, DatabaseConfig};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "darkhold-taxonomy-{tag}-{}-{:?}",
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
        "sqlite://{}/darkhold.sqlite?mode=rwc",
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
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    /// One entity, written directly: the export reads rows, it does not create them.
    async fn entity(&self, id: &str, kind: &str, canonical: &str, norm: &str, name: &str) {
        sqlx::query(
            "INSERT INTO canonical_entities (id, kind, canonical, norm, canonical_name, status, node_state, work_count, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, 'active', 'browsable', 7, '2026-01-01', '2026-01-01')",
        )
        .bind(id)
        .bind(kind)
        .bind(canonical)
        .bind(norm)
        .bind(name)
        .execute(self.db.sqlite_pool().expect("sqlite"))
        .await
        .expect("insert entity");
    }
}

#[tokio::test]
async fn the_export_is_a_document_another_site_can_import() {
    let h = Harness::new("shape").await;
    h.entity("e-1", "tag", "Regression System", "regression system", "Regression System").await;
    h.entity("e-2", "character", "Ash Step", "ash step", "Ash Step").await;

    let (status, body) = h.get("/api/v1/taxonomy/export").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 2);
    assert_eq!(body["items"].as_array().map(|v| v.len()), Some(2));
    assert!(body["version"].as_str().unwrap_or("").contains('@'), "version: {}", body["version"]);

    // Ordered by `canonical`, so the test looks the row up rather than assuming a position.
    let first = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "e-1")
        .expect("the tag is in the document");
    assert_eq!(first["kind"], "tag");
    assert_eq!(first["canonical"], "Regression System");
    assert_eq!(first["norm"], "regression system");
    assert_eq!(first["work_count"], 7);
    assert!(body["aliases"].is_array(), "aliases ship even when empty");

    // `kind` narrows to one branch.
    let (status, tags) = h.get("/api/v1/taxonomy/export?kind=tag").await;
    assert_eq!(status, StatusCode::OK, "{tags}");
    assert_eq!(tags["total"], 1);
    assert_eq!(tags["items"][0]["id"], "e-1");

    // Paging says when to stop: an offset past the end is an empty page, not an error.
    let (status, tail) = h.get("/api/v1/taxonomy/export?limit=1&offset=5").await;
    assert_eq!(status, StatusCode::OK, "{tail}");
    assert_eq!(tail["items"].as_array().map(|v| v.len()), Some(0));
    assert_eq!(tail["total"], 2, "total is the corpus, not the page");
}

#[tokio::test]
async fn the_version_changes_when_the_vocabulary_does() {
    let h = Harness::new("version").await;
    h.entity("e-1", "tag", "One", "one", "One").await;
    let (_, before) = h.get("/api/v1/taxonomy/export").await;
    let v1 = before["version"].as_str().unwrap().to_string();

    h.entity("e-2", "tag", "Two", "two", "Two").await;
    let (_, after) = h.get("/api/v1/taxonomy/export").await;
    let v2 = after["version"].as_str().unwrap().to_string();

    assert_ne!(v1, v2, "an importer must be able to see that the document changed");
    assert!(v2.starts_with('2'), "the version carries the count: {v2}");
}

#[tokio::test]
async fn a_bad_parameter_is_refused_by_name() {
    let h = Harness::new("validation").await;
    let (status, body) = h.get("/api/v1/taxonomy/export?limit=0").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "VALIDATION_FAILED");

    let (status, body) = h.get("/api/v1/taxonomy/export?kind=nonsense").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(
        body["error"]["field_errors"]["kind"].is_string(),
        "the refusal names the field: {body}"
    );

    // No session is needed: this is the public face of the naming authority.
    let (status, _) = h.get("/api/v1/taxonomy/export").await;
    assert!(!status.is_client_error() || status == StatusCode::OK);
}
