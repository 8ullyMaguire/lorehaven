//! M19 — Administration, statistics, abuse defence, privacy, operations.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::{Database, DatabaseConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m19-{}-{:?}-{:?}",
        tag,
        std::process::id(),
        std::thread::current().id(),
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
        Self { app, cookies: Vec::new() }
    }
    fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    fn capture(&mut self, response: &axum::response::Response) {
        for value in response.headers().get_all(header::SET_COOKIE) {
            let Ok(text) = value.to_str() else { continue; };
            let Some((pair, _)) = text.split_once(';') else { continue; };
            if let Some((name, value)) = pair.split_once('=') {
                let name = name.trim().to_owned();
                let value = value.trim().to_owned();
                self.cookies.retain(|(k, _)| k != &name);
                if !value.is_empty() { self.cookies.push((name, value)); }
            }
        }
    }
    async fn request(&mut self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if !self.cookies.is_empty() {
            builder = builder.header(header::COOKIE, self.cookies.iter().map(|(n, v)| format!("{n}={v}")).collect::<Vec<_>>().join("; "));
        }
        if !matches!(method, "GET" | "HEAD" | "OPTIONS") {
            if let Some(token) = self.cookie("lorehaven_csrf").map(str::to_owned) {
                builder = builder.header("x-csrf-token", token);
            }
        }
        let request = match body {
            Some(v) => builder.header(header::CONTENT_TYPE, "application/json").body(Body::from(serde_json::to_vec(&v).expect("serialise"))).expect("request"),
            None => builder.body(Body::empty()).expect("request"),
        };
        let response = self.app.clone().oneshot(request).await.expect("response");
        let status = response.status();
        self.capture(&response);
        let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024).await.expect("body");
        let value = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into_owned())) };
        (status, value)
    }
    async fn get(&mut self, uri: &str) -> (StatusCode, Value) { self.request("GET", uri, None).await }
    async fn post(&mut self, uri: &str, body: Value) -> (StatusCode, Value) { self.request("POST", uri, Some(body)).await }
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
        let db = Database::connect(&DatabaseConfig::new(format!("sqlite://{}/lorehaven.sqlite?mode=rwc", dir.display()))).await.expect("connect");
        let _ = db.migrate().await.expect("migrate");
        Self { dir, db }
    }
    fn db(&self) -> &Database { &self.db }
    fn client(&self) -> Client { Client::new(server::build_router(AppState::new(config_for(&self.dir), self.db.clone()))) }
    async fn cleanup(self) { self.db.close().await; let _ = std::fs::remove_dir_all(self.dir); }
}

const PASSWORD: &str = "a-long-enough-passphrase";

async fn register(client: &mut Client, email: &str, handle: &str) {
    let (status, body) = client.post("/api/v1/auth/register", json!({ "email": email, "password": PASSWORD, "handle": handle, "display_name": handle, "age_band": "adult" })).await;
    assert_eq!(status, StatusCode::CREATED, "register {handle}: {body}");
    let _account = body["account"].as_object().expect("account object");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stats_k_anonymity_merges_small_counts() {
    let _harness = Harness::new("stats").await;

    let (key, _) = lorehaven_domain::stats::apply_k_anonymity("work-123", 3);
    assert_eq!(key, "other");

    let (key, count) = lorehaven_domain::stats::apply_k_anonymity("work-123", 10);
    assert_eq!(key, "work-123");
    assert_eq!(count, 10);

    _harness.cleanup().await;
}

#[tokio::test]
async fn admin_action_can_be_recorded() {
    let harness = Harness::new("admin-action").await;

    let id = lorehaven_db::admin::record_admin_action(
        harness.db(), "operator-1", "ban_user", "account", "user-123", "{\"reason\": \"spam\"}",
    ).await.expect("record admin action");

    assert!(!id.is_empty());

    harness.cleanup().await;
}

#[tokio::test]
async fn privacy_request_can_be_created_and_completed() {
    let harness = Harness::new("privacy").await;
    let mut client = harness.client();

    register(&mut client, "privacy@example.com", "PrivacyUser").await;

    let account_id = sqlx::query_scalar::<_, String>(
        "SELECT id FROM accounts WHERE email = ?",
    )
    .bind("privacy@example.com")
    .fetch_one(harness.db().sqlite_pool().expect("sqlite"))
    .await
    .expect("account exists");

    let id = lorehaven_db::admin::create_privacy_request(
        harness.db(), &account_id, "export",
    ).await.expect("create privacy request");

    assert!(!id.is_empty());

    lorehaven_db::admin::complete_privacy_request(
        harness.db(), &id, Some("storage/key/export.zip"),
    ).await.expect("complete privacy request");

    harness.cleanup().await;
}

#[tokio::test]
async fn abuse_counter_can_be_incremented() {
    let harness = Harness::new("abuse").await;

    let count1 = lorehaven_db::admin::increment_abuse_counter(
        harness.db(), "ip:1.2.3.4", "2026-09-14",
    ).await.expect("increment counter");
    assert_eq!(count1, 1);

    let count2 = lorehaven_db::admin::increment_abuse_counter(
        harness.db(), "ip:1.2.3.4", "2026-09-14",
    ).await.expect("increment counter again");
    assert_eq!(count2, 2);

    harness.cleanup().await;
}

#[tokio::test]
async fn admin_stats_endpoint_works() {
    let harness = Harness::new("stats-endpoint").await;
    let mut client = harness.client();

    let (status, body) = client.get("/api/v1/admin/stats").await;
    assert_eq!(status, StatusCode::OK, "stats: {body}");
    assert!(body["reading"].as_i64().is_some());

    harness.cleanup().await;
}
