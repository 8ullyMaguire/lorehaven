//! Milestone 24 — anchored comments (spec §12, §30.4; migration 0027).

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_db::{Backend, DatabaseConfig};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m24-{tag}-{:?}",
        std::process::id(),
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
                self.cookies.iter().map(|(n, v)| format!("{n}={v}")).collect::<Vec<_>>().join("; "),
            );
        }
        if !matches!(method, "GET" | "HEAD" | "OPTIONS") {
            if let Some(token) = self.cookie("lorehaven_csrf") {
                builder = builder.header("x-csrf-token", token);
            }
        }
        let request = match body {
            Some(v) => builder.header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&v).expect("serialise"))).expect("request"),
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
    let (status, _) = client.post("/api/v1/auth/register", json!({ "email": email, "password": "a-long-enough-passphrase", "handle": handle, "display_name": handle, "age_band": "adult" })).await;
    assert_eq!(status, StatusCode::CREATED, "register failed for {email}");
}

/// Create a published work with a chapter and return (work_id, chapter_id).
async fn create_work_with_chapter(client: &mut Client, title: &str) -> (String, String) {
    // Create work
    let (status, body) = client.post("/api/v1/works", json!({ "title": title })).await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    let work_id = body["id"].as_str().unwrap().to_owned();
    let work_version = body["version"].as_i64().unwrap();

    // Create chapter
    let (status, body) = client.post(&format!("/api/v1/works/{work_id}/chapters"), json!({ "title": "Chapter 1" })).await;
    assert_eq!(status, StatusCode::CREATED, "create chapter: {body}");
    let chapter_id = body["id"].as_str().unwrap().to_owned();
    let chapter_version = body["version"].as_i64().unwrap();

    // Save chapter document
    let doc = json!({ "type": "doc", "content": [
        { "type": "paragraph", "content": [{ "type": "text", "text": "First paragraph." }] },
        { "type": "paragraph", "content": [{ "type": "text", "text": "Second paragraph." }] },
        { "type": "paragraph", "content": [{ "type": "text", "text": "Third paragraph." }] },
    ] });
    let (status, _) = client.patch(&format!("/api/v1/chapters/{chapter_id}"), json!({ "expected_version": chapter_version, "document": doc })).await;
    assert_eq!(status, StatusCode::OK, "save chapter");

    // Publish work
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

    // Post a whole-work comment
    let (status, body) = client.post(
        &format!("/api/v1/works/{work_id}/comments"),
        json!({ "body": "Whole work comment" }),
    ).await;
    assert_eq!(status, StatusCode::OK, "whole-work comment: {body}");
    let whole_comment_id = body["id"].as_str().unwrap().to_owned();

    // Post an anchored comment (paragraph 1 in the chapter)
    let (status, body) = client.post(
        &format!("/api/v1/works/{work_id}/comments"),
        json!({ "body": "Anchored to paragraph 1", "anchor_kind": "paragraph", "anchor_value": "1", "anchor_chapter_id": chapter_id }),
    ).await;
    assert_eq!(status, StatusCode::OK, "anchored comment: {body}");
    let anchored_comment_id = body["id"].as_str().unwrap().to_owned();

    // List comments — both should appear with anchor info
    let (status, body) = client.get(&format!("/api/v1/works/{work_id}/comments")).await;
    assert_eq!(status, StatusCode::OK, "list comments");
    let items = body["items"].as_array().expect("items array");
    assert!(items.len() >= 2, "expected at least 2 comments, got {}", items.len());

    // Check the anchored comment has anchor fields
    let anchored = items.iter().find(|c| c["id"].as_str().unwrap() == anchored_comment_id).expect("anchored comment in list");
    assert_eq!(anchored["anchor_kind"].as_str().unwrap(), "paragraph");
    assert_eq!(anchored["anchor_value"].as_str().unwrap(), "1");
    assert_eq!(anchored["anchor_chapter_id"].as_str().unwrap(), chapter_id);

    // Check the whole-work comment has null anchor
    let whole = items.iter().find(|c| c["id"].as_str().unwrap() == whole_comment_id).expect("whole comment in list");
    assert!(whole["anchor_kind"].is_null(), "whole comment anchor_kind should be null");
    assert!(whole["anchor_value"].is_null(), "whole comment anchor_value should be null");

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

    // Paragraph anchor without chapter_id — should fail
    let (status, _) = client.post(
        &format!("/api/v1/works/{work_id}/comments"),
        json!({ "body": "bad anchor", "anchor_kind": "paragraph", "anchor_value": "1" }),
    ).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "paragraph anchor needs chapter_id");

    // Timestamp anchor with chapter_id — should fail
    let (status, _) = client.post(
        &format!("/api/v1/works/{work_id}/comments"),
        json!({ "body": "bad anchor", "anchor_kind": "timestamp", "anchor_value": "42", "anchor_chapter_id": chapter_id }),
    ).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "timestamp anchor must not have chapter_id");

    // Invalid timestamp format — should fail
    let (status, _) = client.post(
        &format!("/api/v1/works/{work_id}/comments"),
        json!({ "body": "bad timestamp", "anchor_kind": "timestamp", "anchor_value": "abc" }),
    ).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "invalid timestamp format rejected");

    // Invalid anchor_kind — should fail
    let (status, _) = client.post(
        &format!("/api/v1/works/{work_id}/comments"),
        json!({ "body": "bad kind", "anchor_kind": "offset", "anchor_value": "1" }),
    ).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "unknown anchor_kind rejected");

    // Only anchor_kind without anchor_value — should fail
    let (status, _) = client.post(
        &format!("/api/v1/works/{work_id}/comments"),
        json!({ "body": "incomplete anchor", "anchor_kind": "paragraph" }),
    ).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "incomplete anchor rejected");

    // Valid timestamp anchor — should succeed
    let (status, _) = client.post(
        &format!("/api/v1/works/{work_id}/comments"),
        json!({ "body": "valid timestamp", "anchor_kind": "timestamp", "anchor_value": "01:23:45" }),
    ).await;
    assert_eq!(status, StatusCode::OK, "valid timestamp anchor accepted");

    println!("PASS: anchored_comment_validation");
}

#[tokio::test]
async fn migration_0027_creates_anchor_columns() {
    let dir = scratch_dir("migration-0027");
    let tdb = test_support::TestDb::connect_with_dir("migration-0027", &dir).await;

    // Verify the anchor columns exist by inserting a comment with an anchor
    let db = tdb.db();
    let work_id = uuid::Uuid::new_v4().to_string();
    let chapter_id = uuid::Uuid::new_v4().to_string();
    let pseud_id = uuid::Uuid::new_v4().to_string();
    let account_id = uuid::Uuid::new_v4().to_string();
    let now = lorehaven_db::identity::now_rfc3339();

    // Insert an account + pseud first (FK constraints). password_hash lives in password_credentials.
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("INSERT INTO accounts (id, email, status, age_state, created_at, updated_at) VALUES (?, ?, 'active', 'declared_adult', ?, ?)")
                .bind(&account_id).bind("test@example.com").bind(&now).bind(&now)
                .execute(db.sqlite_pool().unwrap()).await.unwrap();
            sqlx::query("INSERT INTO password_credentials (account_id, password_hash, algorithm, created_at, updated_at) VALUES (?, ?, 'argon2id', ?, ?)")
                .bind(&account_id).bind("hash").bind(&now).bind(&now)
                .execute(db.sqlite_pool().unwrap()).await.unwrap();
            sqlx::query("INSERT INTO pseuds (id, account_id, handle, display_name, bio, discoverability, created_at, version, updated_at) VALUES (?, ?, ?, '', '', 'public', ?, 1, ?)")
                .bind(&pseud_id).bind(&account_id).bind("test").bind(&now).bind(&now)
                .execute(db.sqlite_pool().unwrap()).await.unwrap();
            sqlx::query("INSERT INTO comments (id, subject_type, subject_id, author_pseud, body, body_version, anchor_kind, anchor_value, anchor_chapter_id, created_at) VALUES (?, 'work', ?, ?, 'body', 'v1', 'paragraph', '3', ?, ?)")
                .bind(uuid::Uuid::new_v4().to_string()).bind(&work_id).bind(&pseud_id).bind(&chapter_id).bind(&now)
                .execute(db.sqlite_pool().unwrap()).await.unwrap();
        }
        Backend::Postgres => {
            sqlx::query("INSERT INTO accounts (id, email, status, age_state, created_at, updated_at) VALUES ($1::uuid, $2, 'active', 'declared_adult', $3, $3)")
                .bind(&account_id).bind("test@example.com").bind(&now)
                .execute(db.postgres_pool().unwrap()).await.unwrap();
            sqlx::query("INSERT INTO password_credentials (account_id, password_hash, algorithm, created_at, updated_at) VALUES ($1::uuid, $2, 'argon2id', $3, $3)")
                .bind(&account_id).bind("hash").bind(&now)
                .execute(db.postgres_pool().unwrap()).await.unwrap();
            sqlx::query("INSERT INTO pseuds (id, account_id, handle, display_name, bio, discoverability, created_at, version, updated_at) VALUES ($1::uuid, $2::uuid, $3, '', '', 'public', $4, 1, $4)")
                .bind(&pseud_id).bind(&account_id).bind("test").bind(&now)
                .execute(db.postgres_pool().unwrap()).await.unwrap();
            sqlx::query("INSERT INTO comments (id, subject_type, subject_id, author_pseud, body, body_version, anchor_kind, anchor_value, anchor_chapter_id, created_at) VALUES ($1::uuid, 'work', $2::text, $3::uuid, 'body', 'v1', 'paragraph', '3', $4::uuid, $5)")
                .bind(uuid::Uuid::new_v4().to_string()).bind(&work_id).bind(&pseud_id).bind(&chapter_id).bind(&now)
                .execute(db.postgres_pool().unwrap()).await.unwrap();
        }
    }

    println!("PASS: migration_0027_creates_anchor_columns");
}
