//! Milestone 26 — TTS narration edition CRUD (spec §32.5).
//!
//! Verifies that requesting a narration creates a `narration` edition in
//! draft state with the machine producer credited as narrator. Does NOT
//! test actual audio generation — that requires an AI provider integration
//! that doesn't exist yet in this build.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_db::DatabaseConfig;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tower::ServiceExt;

// Surface the app's internal error logs (http.rs drops them without a
// subscriber, which makes 500s undebuggable in tests).
#[allow(unused)]
fn init_logs() {
    let _ = tracing_subscriber::fmt().try_init();
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m26-narrate-{tag}-{:?}",
        std::process::id()
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
            let Ok(text) = value.to_str() else {
                continue;
            };
            let Some((pair, _)) = text.split_once(';') else {
                continue;
            };
            if let Some((name, value)) = pair.split_once('=') {
                let name = name.trim();
                let value = value.trim();
                self.cookies.retain(|(k, _)| k != name);
                if !value.is_empty() {
                    self.cookies.push((name.to_owned(), value.to_owned()));
                }
            }
        }
    }
    async fn send(&mut self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
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
            if let Some(token) = self.cookie("lorehaven_csrf") {
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
        self.capture(&response);
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024)
            .await
            .expect("body");
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, value)
    }
    async fn post(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.send("POST", uri, Some(body)).await
    }
    async fn patch(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.send("PATCH", uri, Some(body)).await
    }
    async fn get(&mut self, uri: &str) -> (StatusCode, Value) {
        self.send("GET", uri, None).await
    }
}

async fn register(client: &mut Client, email: &str, handle: &str) {
    let (status, body) = client.post("/api/v1/auth/register", json!({ "email": email, "password": "a-long-enough-passphrase", "handle": handle, "display_name": handle, "age_band": "adult" })).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "register failed for {email}: {body}"
    );
}

async fn create_and_publish_work(client: &mut Client, title: &str) -> String {
    let (status, body) = client
        .post("/api/v1/works", json!({ "title": title }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    let work_id = body["id"].as_str().unwrap().to_owned();
    let work_version = body["version"].as_i64().unwrap();

    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/chapters"),
            json!({ "title": "Chapter 1" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create chapter: {body}");
    let chapter_id = body["id"].as_str().unwrap().to_owned();
    let chapter_version = body["version"].as_i64().unwrap();

    let doc = json!({ "type": "doc", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "Content." }] }] });
    let (status, _) = client
        .patch(
            &format!("/api/v1/chapters/{chapter_id}"),
            json!({ "expected_version": chapter_version, "document": doc }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "save chapter");

    let (status, _) = client.post(&format!("/api/v1/works/{work_id}/publish"), json!({ "expected_version": work_version, "idempotency_key": format!("m26-narrate-{work_id}") })).await;
    assert_eq!(status, StatusCode::OK, "publish work");

    work_id
}

#[tokio::test]
async fn request_narration_creates_draft_edition_with_credited_narrator() {
    init_logs();
    let dir = scratch_dir("narrate-create");
    let tdb = test_support::TestDb::connect_with_dir("narrate-create", &dir).await;
    let app = server::build_router(AppState::new(config_for(&dir), tdb.db().clone()));

    let mut client = Client::new(app.clone());
    register(&mut client, "author@example.com", "author").await;
    let work_id = create_and_publish_work(&mut client, "Narration Work").await;

    // Request a TTS narration
    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/editions"),
            json!({ "provider": "ai-provider" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "request narration: {body}");
    assert_eq!(body["edition_kind"].as_str().unwrap(), "narration");
    assert_eq!(body["state"].as_str().unwrap(), "draft");
    assert!(body["machine_produced"].as_bool().unwrap_or(false));

    let edition_id = body["edition_id"].as_str().unwrap().to_owned();

    // Read the edition back
    let (status, body) = client.get(&format!("/api/v1/editions/{edition_id}")).await;
    assert_eq!(status, StatusCode::OK, "get edition: {body}");
    assert_eq!(body["edition_kind"].as_str().unwrap(), "narration");
    assert_eq!(body["work_id"].as_str().unwrap(), work_id);
    assert!(body["label"].as_str().unwrap().contains("ai-provider"));

    println!("PASS: request_narration_creates_draft_edition_with_credited_narrator");
}

#[tokio::test]
async fn narration_editions_listable_after_creation() {
    let dir = scratch_dir("narrate-list");
    let tdb = test_support::TestDb::connect_with_dir("narrate-list", &dir).await;
    let app = server::build_router(AppState::new(config_for(&dir), tdb.db().clone()));

    let mut client = Client::new(app.clone());
    register(&mut client, "author@example.com", "author").await;
    let work_id = create_and_publish_work(&mut client, "Listable Work").await;

    // Create narration
    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/editions"),
            json!({ "provider": "ai-provider" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "request narration: {body}");

    // List editions
    let (status, body) = client
        .get(&format!("/api/v1/works/{work_id}/editions"))
        .await;
    assert_eq!(status, StatusCode::OK, "list editions: {body}");
    let editions = body["editions"].as_array().expect("editions array");
    assert_eq!(editions.len(), 1, "expected exactly one narration edition");
    assert_eq!(editions[0]["edition_kind"].as_str().unwrap(), "narration");

    println!("PASS: narration_editions_listable_after_creation");
}
