//! M11 — Discovery: recommendations, taste profiles.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::{Database, DatabaseConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m11-{tag}-{}-{:?}",
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

struct Client {
    app: axum::Router,
    cookies: Vec<(String, String)>,
}

impl Client {
    fn new(app: axum::Router) -> Self {
        Self {
            app,
            cookies: Vec::new(),
        }
    }
    fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
    fn capture(&mut self, response: &axum::response::Response) {
        for value in response.headers().get_all(header::SET_COOKIE) {
            let Ok(text) = value.to_str() else { continue };
            let Some((pair, _)) = text.split_once(';') else {
                continue;
            };
            if let Some((name, value)) = pair.split_once('=') {
                let name = name.trim().to_owned();
                let value = value.trim().to_owned();
                self.cookies.retain(|(k, _)| k != &name);
                if !value.is_empty() {
                    self.cookies.push((name, value));
                }
            }
        }
    }
    async fn request(
        &mut self,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if !self.cookies.is_empty() {
            builder = builder.header(
                header::COOKIE,
                self.cookies
                    .iter()
                    .map(|(n, v)| format!("{n}={v}"))
                    .collect::<Vec<_>>()
                    .join("; "),
            );
        }
        if !matches!(method, "GET" | "HEAD" | "OPTIONS") {
            if let Some(token) = self.cookie("lorehaven_csrf").map(str::to_owned) {
                builder = builder.header("x-csrf-token", token);
            }
        }
        let request = match body {
            Some(v) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&v).expect("serialise")))
                .expect("request"),
            None => builder.body(Body::empty()).expect("request"),
        };
        let response = self.app.clone().oneshot(request).await.expect("response");
        let status = response.status();
        self.capture(&response);
        let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024)
            .await
            .expect("body");
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        };
        (status, value)
    }
    async fn get(&mut self, uri: &str) -> (StatusCode, Value) {
        self.request("GET", uri, None).await
    }
    async fn post(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("POST", uri, Some(body)).await
    }
}

struct Harness {
    dir: PathBuf,
    db: Database,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        set_trust_proxy(false);
        let _ = lorehaven_app::logging::init(&lorehaven_app::config::LoggingConfig {
            filter: "error".to_owned(),
            format: lorehaven_app::config::LogFormat::Pretty,
        });
        let dir = scratch_dir(tag);
        let db = Database::connect(&DatabaseConfig::new(format!(
            "sqlite://{}/lorehaven.sqlite?mode=rwc",
            dir.display()
        )))
        .await
        .expect("connect");
        let _ = db.migrate().await.expect("migrate");
        Self { dir, db }
    }
    fn client(&self) -> Client {
        Client::new(server::build_router(AppState::new(
            config_for(&self.dir),
            self.db.clone(),
        )))
    }
    async fn cleanup(self) {
        self.db.close().await;
        let _ = std::fs::remove_dir_all(self.dir);
    }
}

const PASSWORD: &str = "a-long-enough-passphrase";

async fn register(client: &mut Client, email: &str, handle: &str) {
    let (status, body) = client
        .post(
            "/api/v1/auth/register",
            json!({
                "email": email,
                "password": PASSWORD,
                "handle": handle,
                "display_name": handle,
                "age_band": "adult"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "register {handle}: {body}");
}

async fn published_work(harness: &Harness, email: &str, handle: &str, title: &str) -> String {
    let mut author = harness.client();
    register(&mut author, email, handle).await;
    let (status, body) = author
        .post("/api/v1/works", json!({ "title": title }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    let work_id = body["id"].as_str().expect("id").to_owned();
    let work_version = body["version"].as_i64().expect("version");
    let (status, body) = author
        .post(
            &format!("/api/v1/works/{work_id}/chapters"),
            json!({ "title": "One" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "chapter: {body}");
    let chapter = body["id"].as_str().expect("chapter").to_owned();
    let chapter_version = body["version"].as_i64().expect("version");
    let doc = json!({ "type": "doc", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "A chapter with enough words to have a middle." }] }] });
    let (status, _) = author
        .request(
            "PATCH",
            &format!("/api/v1/chapters/{chapter}"),
            Some(json!({ "expected_version": chapter_version, "document": doc })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "save chapter");
    let (status, _) = author
        .post(
            &format!("/api/v1/works/{work_id}/publish"),
            json!({ "expected_version": work_version, "idempotency_key": format!("m11-{work_id}") }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "publish");
    work_id
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn discovery_returns_public_works_for_anonymous() {
    let harness = Harness::new("discovery-anon").await;
    let _ = published_work(&harness, "a@example.com", "AuthorA", "Public Work").await;
    let mut anon = harness.client();
    let (status, body) = anon.get("/api/v1/discovery").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert!(!items.is_empty(), "expected public works: {body}");
    harness.cleanup().await;
}

#[tokio::test]
async fn discovery_returns_public_works_for_signed_in() {
    let harness = Harness::new("discovery-signedin").await;
    let _ = published_work(&harness, "a@example.com", "AuthorA", "Another Work").await;
    let mut client = harness.client();
    register(&mut client, "reader@example.com", "Reader").await;
    let (status, body) = client.get("/api/v1/discovery").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert!(!items.is_empty(), "expected works: {body}");
    harness.cleanup().await;
}

#[tokio::test]
async fn taste_profile_empty_initially() {
    let harness = Harness::new("taste-empty").await;
    let mut client = harness.client();
    register(&mut client, "reader@example.com", "Reader").await;
    let (status, body) = client.get("/api/v1/discovery/taste-profile").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["signals"], json!({}), "{body}");
    harness.cleanup().await;
}

#[tokio::test]
async fn taste_profile_clear_returns_empty() {
    let harness = Harness::new("taste-clear").await;
    let mut client = harness.client();
    register(&mut client, "reader@example.com", "Reader").await;
    // Recompute first (will be empty if no history)
    let (status, _) = client.post("/api/v1/discovery/taste-profile/recompute", json!({})).await;
    assert_eq!(status, StatusCode::OK, "recompute");
    // Clear
    let (status, body) = client.post("/api/v1/discovery/taste-profile/clear", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "cleared", "{body}");
    // Now should be empty
    let (status, body) = client.get("/api/v1/discovery/taste-profile").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["signals"], json!({}), "{body}");
    harness.cleanup().await;
}

#[tokio::test]
async fn taste_profile_requires_session() {
    let harness = Harness::new("taste-auth").await;
    let mut anon = harness.client();
    let (status, _) = anon.get("/api/v1/discovery/taste-profile").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    harness.cleanup().await;
}

#[tokio::test]
async fn discovery_anonymous_can_access() {
    let harness = Harness::new("discovery-auth").await;
    let _ = published_work(&harness, "a@example.com", "AuthorA", "Open Work").await;
    let mut anon = harness.client();
    let (status, body) = anon.get("/api/v1/discovery").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    harness.cleanup().await;
}

#[tokio::test]
async fn discovery_returns_cursor_envelope() {
    let harness = Harness::new("discovery-envelope").await;
    let _ = published_work(&harness, "a@example.com", "AuthorA", "Envelope Work").await;
    let mut anon = harness.client();
    let (status, body) = anon.get("/api/v1/discovery").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.get("items").is_some(), "expected items field: {body}");
    harness.cleanup().await;
}

#[tokio::test]
async fn discovery_returns_work_ids() {
    let harness = Harness::new("discovery-ids").await;
    let work_id = published_work(&harness, "a@example.com", "AuthorA", "ID Work").await;
    let mut anon = harness.client();
    let (status, body) = anon.get("/api/v1/discovery").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert!(!items.is_empty(), "{body}");
    assert_eq!(items[0]["work_id"], work_id, "{body}");
    harness.cleanup().await;
}
