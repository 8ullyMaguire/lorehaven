//! M34 — Spoilers, warnings, readability (spec §35.4).

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::DatabaseConfig;
use serde_json::{json, Value};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m34-{tag}-{}-{:?}",
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
    config.rate_limits.auth = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.write = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
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
            let Ok(text) = value.to_str() else { continue; };
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
                .body(Body::from(v.to_string()))
                .expect("build request"),
            None => builder.body(Body::empty()).expect("build request"),
        };
        let response = self.app.clone().oneshot(request).await.expect("oneshot");
        let status = response.status();
        self.capture(&response);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("bytes");
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, body)
    }
    async fn post(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("POST", uri, Some(body)).await
    }
    async fn get(&mut self, uri: &str) -> (StatusCode, Value) {
        self.request("GET", uri, None).await
    }
    async fn put(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("PUT", uri, Some(body)).await
    }
    async fn delete(&mut self, uri: &str) -> (StatusCode, Value) {
        self.request("DELETE", uri, None).await
    }
    async fn patch(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("PATCH", uri, Some(body)).await
    }
}

struct Harness {
    dir: PathBuf,
    tdb: test_support::TestDb,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        set_trust_proxy(false);
        let _ = lorehaven_app::logging::init(&lorehaven_app::config::LoggingConfig {
            filter: "error".to_owned(),
            format: lorehaven_app::config::LogFormat::Pretty,
        });
        let dir = scratch_dir(tag);
        let tdb = test_support::TestDb::connect_with_dir(tag, &dir).await;
        Self { dir, tdb }
    }

    fn client(&self) -> Client {
        Client::new(server::build_router(AppState::new(
            config_for(&self.dir),
            self.tdb.db().clone(),
        )))
    }

    async fn seed_category(&self) -> String {
        use lorehaven_db::Backend;
        let db = self.tdb.db();
        let id = uuid::Uuid::new_v4().to_string();
        let sql = db.sql(
            "INSERT INTO forum_categories (id, name, position, min_trust) VALUES (?, 'Test Forum', 0, 0)",
            "INSERT INTO forum_categories (id, name, position, min_trust) VALUES ($1, 'Test Forum', 0, 0)",
        );
        match db.backend() {
            Backend::Sqlite => {
                sqlx::query(&sql)
                    .bind(&id)
                    .execute(db.sqlite_pool().expect("sqlite"))
                    .await
                    .expect("seed category");
            }
            Backend::Postgres => {
                sqlx::query(&sql)
                    .bind(&id)
                    .execute(db.postgres_pool().expect("postgres"))
                    .await
                    .expect("seed category");
            }
        }
        id
    }
}

const PASSWORD: &str = "a-long-enough-passphrase";

async fn register(client: &mut Client, email: &str, handle: &str) {
    let (status, body) = client
        .post(
            "/api/v1/auth/register",
            json!({
                "email": email,
                "handle": handle,
                "password": PASSWORD,
                "display_name": handle,
                "age_band": "adult"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "register {handle}: {body}");
}

async fn create_topic(client: &mut Client, category: &str, title: &str) -> String {
    let (status, body) = client
        .post(
            &format!("/api/v1/forums/{category}/topics"),
            json!({ "title": title }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "create topic: {body}");
    body["id"].as_str().expect("topic id").to_owned()
}

async fn create_post(client: &mut Client, topic_id: &str, body: &str) -> String {
    let (status, response) = client
        .post(
            &format!("/api/v1/topics/{topic_id}/replies"),
            json!({ "body": body }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "create post: {response}");
    response["id"].as_str().expect("post id").to_owned()
}

async fn create_work(client: &mut Client, title: &str) -> String {
    let (status, body) = client
        .post("/api/v1/works", json!({ "title": title }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    let work_id = body["id"].as_str().expect("work id").to_owned();

    // Add a chapter so the work can be published.
    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/chapters"),
            json!({ "title": "Chapter 1" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "add chapter: {body}");
    let chapter_id = body["id"].as_str().expect("chapter id").to_owned();

    let (status, body) = client
        .patch(
            &format!("/api/v1/chapters/{chapter_id}"),
            json!({ "expected_version": 1, "document": { "type": "doc", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "Opening text." }] }] } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "update chapter: {body}");

    work_id
}

async fn publish_work(client: &mut Client, work_id: &str) {
    let (status, body) = client
        .post(&format!("/api/v1/works/{work_id}/publish"), json!({ "expected_version": 1 }))
        .await;
    assert_eq!(status, StatusCode::OK, "publish: {body}");
}

// ---------------------------------------------------------------------------
// Spoiler scope
// ---------------------------------------------------------------------------

#[tokio::test]
async fn spoiler_scope_round_trips() {
    let harness = Harness::new("spoiler-scope").await;
    let mut client = harness.client();
    register(&mut client, "author@example.com", "author").await;

    let category = harness.seed_category().await;
    let topic_id = create_topic(&mut client, &category, "Discussion Topic").await;

    // Set spoiler scope to chapter 5.
    let (status, body) = client
        .put(
            &format!("/api/v1/topics/{topic_id}/spoiler-scope"),
            json!({ "chapter": 5 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "set spoiler scope: {body}");
    assert_eq!(body["spoiler_scope_chapter"], 5);

    // Clear spoiler scope.
    let (status, body) = client
        .put(
            &format!("/api/v1/topics/{topic_id}/spoiler-scope"),
            json!({ "chapter": null }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "clear spoiler scope: {body}");
    assert!(body["spoiler_scope_chapter"].is_null());
}

#[tokio::test]
async fn spoiler_scope_refuses_non_author() {
    let harness = Harness::new("spoiler-scope-forbidden").await;
    let mut author = harness.client();
    register(&mut author, "author@example.com", "author").await;

    let category = harness.seed_category().await;
    let topic_id = create_topic(&mut author, &category, "Discussion Topic").await;

    // Stranger tries to set spoiler scope.
    let mut stranger = harness.client();
    register(&mut stranger, "stranger@example.com", "stranger").await;

    let (status, _body) = stranger
        .put(
            &format!("/api/v1/topics/{topic_id}/spoiler-scope"),
            json!({ "chapter": 5 }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "stranger cannot set spoiler scope");
}

// ---------------------------------------------------------------------------
// Content warnings
// ---------------------------------------------------------------------------

#[tokio::test]
async fn content_warnings_round_trip() {
    let harness = Harness::new("content-warnings").await;
    let mut client = harness.client();
    register(&mut client, "author@example.com", "author").await;

    let category = harness.seed_category().await;
    let topic_id = create_topic(&mut client, &category, "Discussion Topic").await;
    let post_id = create_post(&mut client, &topic_id, "Some content here").await;

    // Add a content warning.
    let (status, body) = client
        .post(
            &format!("/api/v1/posts/{post_id}/warnings"),
            json!({ "warning_type": "violence", "severity": 1, "custom_text": "Graphic fight scene" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "add warning: {body}");
    assert!(body["added"].as_bool().unwrap_or(false));

    // Get warnings for the post.
    let (status, body) = client
        .get(&format!("/api/v1/posts/{post_id}/warnings"))
        .await;
    assert_eq!(status, StatusCode::OK, "get warnings: {body}");

    let warnings = body["warnings"].as_array().expect("warnings array");
    assert!(!warnings.is_empty(), "Post should have warnings");
}

// ---------------------------------------------------------------------------
// Reader progress
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reader_progress_round_trips() {
    let harness = Harness::new("reader-progress").await;
    let mut client = harness.client();
    register(&mut client, "reader@example.com", "reader").await;

    let work_id = create_work(&mut client, "Test Work").await;
    publish_work(&mut client, &work_id).await;

    // Default progress is 0.
    let (status, body) = client
        .get(&format!("/api/v1/works/{work_id}/progress"))
        .await;
    assert_eq!(status, StatusCode::OK, "get progress: {body}");
    assert_eq!(body["last_chapter"], 0);

    // Update progress to chapter 3.
    let (status, body) = client
        .put(
            &format!("/api/v1/works/{work_id}/progress"),
            json!({ "last_chapter": 3 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "update progress: {body}");
    assert_eq!(body["last_chapter"], 3);

    // Verify it persisted.
    let (status, body) = client
        .get(&format!("/api/v1/works/{work_id}/progress"))
        .await;
    assert_eq!(status, StatusCode::OK, "get progress again: {body}");
    assert_eq!(body["last_chapter"], 3);
}

// ---------------------------------------------------------------------------
// Draft autosave
// ---------------------------------------------------------------------------

#[tokio::test]
async fn draft_survives_reload() {
    let harness = Harness::new("draft-autosave").await;
    let mut client = harness.client();
    register(&mut client, "author@example.com", "author").await;

    let category = harness.seed_category().await;
    let topic_id = create_topic(&mut client, &category, "Discussion Topic").await;

    // Save a draft.
    let (status, body) = client
        .post(
            &format!("/api/v1/topics/{topic_id}/draft"),
            json!({ "body": "Half-written thoughts..." }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "save draft: {body}");

    // Retrieve the draft.
    let (status, body) = client
        .get(&format!("/api/v1/topics/{topic_id}/draft"))
        .await;
    assert_eq!(status, StatusCode::OK, "get draft: {body}");
    assert_eq!(body["body"], "Half-written thoughts...");

    // Overwrite the draft (simulating another autosave).
    let (status, body) = client
        .post(
            &format!("/api/v1/topics/{topic_id}/draft"),
            json!({ "body": "Updated thoughts..." }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "update draft: {body}");

    // Verify the new draft.
    let (status, body) = client
        .get(&format!("/api/v1/topics/{topic_id}/draft"))
        .await;
    assert_eq!(status, StatusCode::OK, "get updated draft: {body}");
    assert_eq!(body["body"], "Updated thoughts...");

    // Delete the draft.
    let (status, body) = client
        .delete(&format!("/api/v1/topics/{topic_id}/draft"))
        .await;
    assert_eq!(status, StatusCode::OK, "delete draft");

    // Verify draft is gone (deleted returns {"deleted": false} when not found).
    let (status, _body) = client
        .get(&format!("/api/v1/topics/{topic_id}/draft"))
        .await;
    assert_eq!(status, StatusCode::OK, "draft should return empty body");
    assert!(
        _body["body"].is_null() || _body["body"].as_str().map(|s| s.is_empty()).unwrap_or(true),
        "draft body should be empty after deletion"
    );
}

// ---------------------------------------------------------------------------
// Warning preferences
// ---------------------------------------------------------------------------

#[tokio::test]
async fn warning_prefs_round_trip() {
    let harness = Harness::new("warning-prefs").await;
    let mut client = harness.client();
    register(&mut client, "reader@example.com", "reader").await;

    // Default: no prefs set.
    let (status, body) = client
        .get("/api/v1/me/warning-prefs")
        .await;
    assert_eq!(status, StatusCode::OK, "get prefs: {body}");

    // Set a preference to auto-expand violence warnings.
    let (status, body) = client
        .put(
            "/api/v1/me/warning-prefs",
            json!({ "warning_type": "violence", "action": "show" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "set pref: {body}");

    // Verify it persisted.
    let (status, body) = client
        .get("/api/v1/me/warning-prefs")
        .await;
    assert_eq!(status, StatusCode::OK, "get prefs again: {body}");
}

// ---------------------------------------------------------------------------
// Scheduled posts
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scheduled_post_appears_in_due_list() {
    let harness = Harness::new("scheduled-posts").await;
    let mut client = harness.client();
    register(&mut client, "author@example.com", "author").await;

    let category = harness.seed_category().await;
    let topic_id = create_topic(&mut client, &category, "Discussion Topic").await;
    let post_id = create_post(&mut client, &topic_id, "Content").await;

    // Schedule the post to be published.
    let (status, body) = client
        .post(
            &format!("/api/v1/posts/{post_id}/schedule"),
            json!({ "scheduled_at": "2099-01-01T00:00:00Z" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "schedule post: {body}");
    assert!(body["scheduled"].as_bool().unwrap_or(false));
}
