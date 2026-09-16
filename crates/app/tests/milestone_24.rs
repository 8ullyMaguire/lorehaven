//! Milestone 24 — anchored comments, orphaning, and CSV imports (spec §12, §24.8, §32.3).

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
    let dir = std::env::temp_dir().join(format!("lorehaven-m24-{tag}-{:?}", std::process::id()));
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

async fn register(client: &mut Client, email: &str, handle: &str) {
    let (status, body) = client.post("/api/v1/auth/register", json!({ "email": email, "password": "a-long-enough-passphrase", "handle": handle, "display_name": handle, "age_band": "adult" })).await;
    assert_eq!(status, StatusCode::CREATED, "register failed for {email}: {body}");
}

async fn create_work_with_chapter(client: &mut Client, title: &str) -> (String, String) {
    let (status, body) = client.post("/api/v1/works", json!({ "title": title })).await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    let work_id = body["id"].as_str().unwrap().to_owned();
    let work_version = body["version"].as_i64().unwrap();

    let (status, body) = client.post(&format!("/api/v1/works/{work_id}/chapters"), json!({ "title": "Chapter 1" })).await;
    assert_eq!(status, StatusCode::CREATED, "create chapter: {body}");
    let chapter_id = body["id"].as_str().unwrap().to_owned();
    let chapter_version = body["version"].as_i64().unwrap();

    let doc = json!({ "type": "doc", "content": [
        { "type": "paragraph", "content": [{ "type": "text", "text": "First paragraph." }] },
        { "type": "paragraph", "content": [{ "type": "text", "text": "Second paragraph." }] },
        { "type": "paragraph", "content": [{ "type": "text", "text": "Third paragraph." }] },
    ] });
    let (status, _) = client.patch(&format!("/api/v1/chapters/{chapter_id}"), json!({ "expected_version": chapter_version, "document": doc })).await;
    assert_eq!(status, StatusCode::OK, "save chapter");

    let (status, _) = client.post(&format!("/api/v1/works/{work_id}/publish"), json!({ "expected_version": work_version, "idempotency_key": format!("m24-{work_id}") })).await;
    assert_eq!(status, StatusCode::OK, "publish work");

    (work_id, chapter_id)
}

#[tokio::test]
async fn anchored_comments_round_trip() {
    let dir = scratch_dir("anchored-round-trip");
    let tdb = test_support::TestDb::connect_with_dir("anchored-round-trip", &dir).await;
    let app = server::build_router(AppState::new(config_for(&dir), tdb.db().clone()));
    let mut client = Client::new(app);

    register(&mut client, "anchored@example.com", "anchored").await;
    let (work_id, chapter_id) = create_work_with_chapter(&mut client, "Anchored Work").await;

    let (status, body) = client.post(&format!("/api/v1/works/{work_id}/comments"), json!({ "body": "Whole work comment" })).await;
    assert_eq!(status, StatusCode::OK, "whole-work comment: {body}");
    let whole_comment_id = body["id"].as_str().unwrap().to_owned();

    let (status, body) = client.post(&format!("/api/v1/works/{work_id}/comments"), json!({ "body": "Anchored to paragraph 1", "anchor_kind": "paragraph", "anchor_value": "1", "anchor_chapter_id": chapter_id })).await;
    assert_eq!(status, StatusCode::OK, "anchored comment: {body}");
    let anchored_comment_id = body["id"].as_str().unwrap().to_owned();

    let (status, body) = client.get(&format!("/api/v1/works/{work_id}/comments")).await;
    assert_eq!(status, StatusCode::OK, "list comments");
    let items = body["items"].as_array().expect("items array");
    assert!(items.len() >= 2);

    let anchored = items.iter().find(|c| c["id"].as_str().unwrap() == anchored_comment_id).expect("anchored");
    assert_eq!(anchored["anchor_kind"].as_str().unwrap(), "paragraph");
    assert_eq!(anchored["anchor_value"].as_str().unwrap(), "1");

    let whole = items.iter().find(|c| c["id"].as_str().unwrap() == whole_comment_id).expect("whole");
    assert!(whole["anchor_kind"].is_null());

    println!("PASS: anchored_comments_round_trip");
}

#[tokio::test]
async fn anchored_comment_validation() {
    let dir = scratch_dir("anchored-validation");
    let tdb = test_support::TestDb::connect_with_dir("anchored-validation", &dir).await;
    let app = server::build_router(AppState::new(config_for(&dir), tdb.db().clone()));
    let mut client = Client::new(app);

    register(&mut client, "anchored-val@example.com", "anchoredval").await;
    let (work_id, chapter_id) = create_work_with_chapter(&mut client, "Validation Work").await;

    let (status, _) = client.post(&format!("/api/v1/works/{work_id}/comments"), json!({ "body": "bad", "anchor_kind": "paragraph", "anchor_value": "1" })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "paragraph needs chapter_id");

    let (status, _) = client.post(&format!("/api/v1/works/{work_id}/comments"), json!({ "body": "bad", "anchor_kind": "timestamp", "anchor_value": "42", "anchor_chapter_id": chapter_id })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "timestamp rejects chapter_id");

    let (status, _) = client.post(&format!("/api/v1/works/{work_id}/comments"), json!({ "body": "bad", "anchor_kind": "timestamp", "anchor_value": "abc" })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "invalid timestamp rejected");

    let (status, _) = client.post(&format!("/api/v1/works/{work_id}/comments"), json!({ "body": "bad", "anchor_kind": "offset", "anchor_value": "1" })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "unknown kind rejected");

    let (status, _) = client.post(&format!("/api/v1/works/{work_id}/comments"), json!({ "body": "bad", "anchor_kind": "paragraph" })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "incomplete anchor rejected");

    let (status, _) = client.post(&format!("/api/v1/works/{work_id}/comments"), json!({ "body": "ok", "anchor_kind": "timestamp", "anchor_value": "01:23:45" })).await;
    assert_eq!(status, StatusCode::OK, "valid timestamp accepted");

    println!("PASS: anchored_comment_validation");
}

#[test]
fn goodreads_csv_parses() {
    let csv = r#"Title,Author,ISBN,My Rating,Average Rating,Shelves,Date Read
The Left Hand of Darkness,Ursula K. Le Guin,9780060500249,5,3.94,sci-fi;classic,2024-01-15"#;

    // The CSV import module from lorehaven_scrapers should parse this into a structured record.
    let parsed = lorehaven_scrapers::csv::import_shelf(csv, "goodreads").expect("parse goodreads csv");
    assert_eq!(parsed.rows.len(), 1, "one row parsed");
    let row = &parsed.rows[0];
    assert_eq!(row.title, "The Left Hand of Darkness");
    assert_eq!(row.author, "Ursula K. Le Guin");
    assert_eq!(row.my_rating, Some(5));
    assert!(row.shelves.contains(&"sci-fi".to_string()));
    assert!(row.shelves.contains(&"classic".to_string()));

    println!("PASS: goodreads_csv_parses");
}
