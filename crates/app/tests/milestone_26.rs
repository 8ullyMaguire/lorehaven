//! Milestone 26 — adult taxonomy behind age gates (spec §32.5).

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_db::DatabaseConfig;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lorehaven-m26-{tag}-{:?}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn config_for(dir: &Path) -> Config {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    config.database = DatabaseConfig::new(format!("sqlite://{}/lorehaven.sqlite?mode=rwc", dir.display()));
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
            builder = builder.header(header::COOKIE, self.cookies.iter().map(|(n, v)| format!("{n}={v}")).collect::<Vec<_>>().join("; "));
        }
        if !matches!(method, "GET" | "HEAD" | "OPTIONS") {
            if let Some(token) = self.cookie("lorehaven_csrf") {
                builder = builder.header("x-csrf-token", token);
            }
        }
        let request = match body {
            Some(v) => builder.header(header::CONTENT_TYPE, "application/json").body(Body::from(serde_json::to_vec(&v).expect("serialise"))).expect("request"),
            None => builder.body(Body::empty()).expect("request"),
        };
        let response = self.app.clone().oneshot(request).await.expect("response");
        self.capture(&response);
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024).await.expect("body");
        let value = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap_or(Value::Null) };
        (status, value)
    }
    async fn get(&mut self, uri: &str) -> (StatusCode, Value) { self.send("GET", uri, None).await }
    async fn post(&mut self, uri: &str, body: Value) -> (StatusCode, Value) { self.send("POST", uri, Some(body)).await }
    async fn patch(&mut self, uri: &str, body: Value) -> (StatusCode, Value) { self.send("PATCH", uri, Some(body)).await }
}

async fn register(client: &mut Client, email: &str, handle: &str, age_band: &str) {
    let (status, body) = client.post("/api/v1/auth/register", json!({ "email": email, "password": "a-long-enough-passphrase", "handle": handle, "display_name": handle, "age_band": age_band })).await;
    assert_eq!(status, StatusCode::CREATED, "register failed for {email}: {body}");
}

async fn create_and_publish_work(client: &mut Client, title: &str, rating: &str) -> String {
    let (status, body) = client.post("/api/v1/works", json!({ "title": title })).await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    let work_id = body["id"].as_str().unwrap().to_owned();
    let work_version = body["version"].as_i64().unwrap();

    let (status, body) = client.post(&format!("/api/v1/works/{work_id}/chapters"), json!({ "title": "Chapter 1" })).await;
    assert_eq!(status, StatusCode::CREATED, "create chapter: {body}");
    let chapter_id = body["id"].as_str().unwrap().to_owned();
    let chapter_version = body["version"].as_i64().unwrap();

    let doc = json!({ "type": "doc", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "Content." }] }] });
    let (status, _) = client.patch(&format!("/api/v1/chapters/{chapter_id}"), json!({ "expected_version": chapter_version, "document": doc })).await;
    assert_eq!(status, StatusCode::OK, "save chapter");

    let (status, _) = client.post(&format!("/api/v1/works/{work_id}/publish"), json!({ "expected_version": work_version, "idempotency_key": format!("m26-{work_id}") })).await;
    assert_eq!(status, StatusCode::OK, "publish work");

    let (status, _) = client.patch(&format!("/api/v1/works/{work_id}"), json!({ "expected_version": work_version + 1, "rating": rating })).await;
    assert_eq!(status, StatusCode::OK, "set rating");

    work_id
}

#[tokio::test]
async fn anonymous_readers_never_see_explicit() {
    let dir = scratch_dir("anon-explicit");
    let tdb = test_support::TestDb::connect_with_dir("anon-explicit", &dir).await;
    let app = server::build_router(AppState::new(config_for(&dir), tdb.db().clone()));

    // Seed a published explicit work
    let mut seed = Client::new(app.clone());
    register(&mut seed, "author@example.com", "author", "adult").await;
    let explicit_id = create_and_publish_work(&mut seed, "Explicit Work", "explicit").await;
    let _general_id = create_and_publish_work(&mut seed, "General Work", "general").await;

    // Anonymous access
    let mut anon = Client::new(app.clone());
    let (status, body) = anon.get("/api/v1/media").await;
    assert_eq!(status, StatusCode::OK, "media list: {body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "anonymous sees only general: {}", items.len());
    assert_eq!(items[0]["title"].as_str().unwrap(), "General Work");

    // Direct access to explicit work should be refused
    let (status, _) = anon.get(&format!("/api/v1/media/{explicit_id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "explicit work should be hidden from anon");

    println!("PASS: anonymous_readers_never_see_explicit");
}

#[tokio::test]
async fn opted_out_readers_never_see_explicit() {
    let dir = scratch_dir("optout-explicit");
    let tdb = test_support::TestDb::connect_with_dir("optout-explicit", &dir).await;
    let app = server::build_router(AppState::new(config_for(&dir), tdb.db().clone()));

    // Seed
    let mut seed = Client::new(app.clone());
    register(&mut seed, "author@example.com", "author", "adult").await;
    let explicit_id = create_and_publish_work(&mut seed, "Explicit Work", "explicit").await;
    let _mature_id = create_and_publish_work(&mut seed, "Mature Work", "mature").await;
    let _general_id = create_and_publish_work(&mut seed, "General Work", "general").await;

    // Reader with teen max_rating
    let mut reader = Client::new(app.clone());
    register(&mut reader, "reader@example.com", "reader", "adult").await;
    let (status, body) = reader.post("/api/v1/settings/content-preferences", json!({ "max_rating": "teen" })).await;
    assert!(status == StatusCode::OK || status == StatusCode::CREATED, "set prefs: {body}");

    let (status, body) = reader.get("/api/v1/media").await;
    assert_eq!(status, StatusCode::OK, "media list");
    let items = body["items"].as_array().expect("items");
    let has_explicit = items.iter().any(|i| i["id"].as_str().unwrap() == explicit_id);
    assert!(!has_explicit, "teen-pref reader must not see explicit");
    let has_general = items.iter().any(|i| i["title"].as_str().unwrap() == "General Work");
    assert!(has_general, "teen-pref reader still sees general");

    println!("PASS: opted_out_readers_never_see_explicit");
}

#[tokio::test]
async fn media_rating_filters_apply_to_all_doors() {
    let dir = scratch_dir("rating-doors");
    let tdb = test_support::TestDb::connect_with_dir("rating-doors", &dir).await;
    let app = server::build_router(AppState::new(config_for(&dir), tdb.db().clone()));

    let mut seed = Client::new(app.clone());
    register(&mut seed, "author@example.com", "author", "adult").await;
    let explicit_id = create_and_publish_work(&mut seed, "Explicit", "explicit").await;
    let _mature_id = create_and_publish_work(&mut seed, "Mature", "mature").await;
    let _general_id = create_and_publish_work(&mut seed, "General", "general").await;

    // List endpoint
    let mut anon = Client::new(app.clone());
    let (status, body) = anon.get("/api/v1/media").await;
    assert_eq!(status, StatusCode::OK, "media list");
    let items = body["items"].as_array().expect("media items");
    let has_explicit = items.iter().any(|i| i["id"].as_str().unwrap() == explicit_id);
    assert!(!has_explicit, "anonymous must not see explicit in list");
    assert_eq!(items.len(), 1, "list: only general");

    // Search endpoint
    let (status, body) = anon.get("/api/v1/search?q=work").await;
    assert_eq!(status, StatusCode::OK, "search");
    let results = body["items"].as_array().expect("search results");
    let has_explicit = results.iter().any(|i| i["id"].as_str().unwrap() == explicit_id);
    assert!(!has_explicit, "anonymous must not see explicit in search");

    println!("PASS: media_rating_filters_apply_to_all_doors");
}
