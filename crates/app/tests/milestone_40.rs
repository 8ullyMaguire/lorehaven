//! M40 — Fork with provenance and permission statements (spec §40).
//!
//! These tests prove, at the HTTP layer:
//!
//! - A fork creates a new empty draft linked to its parent via lineage edge.
//! - A fork inherits parent tags but no body text.
//! - A fork of a private work is private.
//! - The remix permission statement is enforced (no/ask refuse, yes/unstated
//!   proceed).
//! - Depth limit is enforced.
//!
//! Prerequisites: M3 (drafts), M27 (permission statements, lineage, exclusion).

use std::path::PathBuf;

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
        "lorehaven-m40-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn config_for(dir: &PathBuf) -> Config {
    let mut config = Config::development_defaults();
    config.storage.root = dir.clone();
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
            let Ok(text) = value.to_str() else {
                continue;
            };
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

    #[allow(dead_code)]
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
}

const PASSWORD: &str = "a-long-enough-passphrase";

async fn register(client: &mut Client, email: &str, handle: &str) -> (String, String) {
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
    let (status, me) = client.get("/api/v1/auth/me").await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let account = me["account"]["id"].as_str().expect("account id").to_owned();
    let pseud = me["active_pseud_id"].as_str().expect("pseud id").to_owned();
    (account, pseud)
}

/// Create a work with the given title and permission statement.
async fn create_work(client: &mut Client, title: &str, remix: &str) -> String {
    let (status, body) = client
        .post("/api/v1/works", json!({ "title": title }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    let work_id = body["id"].as_str().expect("work id").to_owned();

    // Set permission statement.
    let (status, body) = client
        .request(
            "PUT",
            &format!("/api/v1/works/{work_id}/permissions"),
            Some(json!({ "remix": remix })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "set permissions: {body}");

    // Add a chapter so the work can be published.
    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/chapters"),
            json!({ "title": "Chapter 1" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "add chapter: {body}");
    let chapter_id = body["id"].as_str().expect("chapter id").to_owned();

    // Add body text to the chapter so the work can be published.
    let (status, body) = client
        .patch(
            &format!("/api/v1/chapters/{chapter_id}"),
            json!({ "expected_version": 1, "document": { "type": "doc", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "Opening text." }] }] } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "update chapter: {body}");

    // Publish so it's visible to others (needed for fork tests with a forker).
    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/publish"),
            json!({ "expected_version": 1 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "publish: {body}");

    work_id
}

#[tokio::test]
async fn fork_creates_draft_with_lineage_and_no_body() {
    let harness = Harness::new("fork-basic").await;
    let mut author = harness.client();
    let (_author_acct, _author_pseud) = register(&mut author, "author@example.com", "author").await;

    // Author creates a work with remix allowed.
    let parent_id = create_work(&mut author, "Original Title", "yes").await;

    // Forker registers.
    let mut forker = harness.client();
    let (_forker_acct, _forker_pseud) = register(&mut forker, "forker@example.com", "forker").await;

    // Forker forks the work.
    let (status, body) = forker
        .post(&format!("/api/v1/works/{parent_id}/fork"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "fork: {body}");

    let fork_id = body["id"].as_str().expect("fork id").to_owned();

    // The fork should be a draft with no chapters.
    let (status, body) = forker.get(&format!("/api/v1/works/{fork_id}")).await;
    assert_eq!(status, StatusCode::OK, "get fork: {body}");
    assert_eq!(body["lifecycle"], "draft");
    assert_eq!(body["chapters"].as_array().expect("chapters").len(), 0);
    assert!(body["title"].as_str().unwrap().contains("Fork"));

    // Verify lineage edge exists in DB.
    let edges = lorehaven_db::permission::lineage_edges_for_work(harness.tdb.db(), &fork_id)
        .await
        .expect("lineage edges");
    assert_eq!(edges.len(), 1, "expected one lineage edge");
    let edge = &edges[0];
    assert_eq!(edge.from_work_id, parent_id);
    assert_eq!(edge.to_work_id, fork_id);
    assert_eq!(edge.kind, lorehaven_domain::permission::LineageKind::Remix);
}

#[tokio::test]
async fn fork_remix_no_is_refused() {
    let harness = Harness::new("fork-no").await;
    let mut author = harness.client();
    let (_author_acct, _author_pseud) =
        register(&mut author, "author-no@example.com", "authorno").await;

    let parent_id = create_work(&mut author, "No-Fork Title", "no").await;

    let mut forker = harness.client();
    let (_forker_acct, _forker_pseud) =
        register(&mut forker, "forker-no@example.com", "forkerno").await;

    let (status, body) = forker
        .post(&format!("/api/v1/works/{parent_id}/fork"), json!({}))
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "expected refusal: {body}"
    );
    assert_eq!(
        body["error"]["field_errors"]["remix"],
        "the author's permission statement declines remixes"
    );
}

#[tokio::test]
async fn fork_remix_ask_is_refused() {
    let harness = Harness::new("fork-ask").await;
    let mut author = harness.client();
    let (_author_acct, _author_pseud) =
        register(&mut author, "author-ask@example.com", "authorask").await;

    let parent_id = create_work(&mut author, "Ask-Fork Title", "ask").await;

    let mut forker = harness.client();
    let (_forker_acct, _forker_pseud) =
        register(&mut forker, "forker-ask@example.com", "forkerask").await;

    let (status, body) = forker
        .post(&format!("/api/v1/works/{parent_id}/fork"), json!({}))
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "expected refusal: {body}"
    );
    assert!(body["error"]["field_errors"]["remix"]
        .as_str()
        .unwrap()
        .contains("ask"));
}

#[tokio::test]
async fn fork_requires_authentication() {
    let harness = Harness::new("fork-auth").await;
    let mut author = harness.client();
    let (_author_acct, _author_pseud) =
        register(&mut author, "author-auth@example.com", "authorauth").await;

    let parent_id = create_work(&mut author, "Auth-Fork Title", "yes").await;

    // Anonymous fork attempt.
    let mut anon = harness.client();
    let (status, _body) = anon
        .post(&format!("/api/v1/works/{parent_id}/fork"), json!({}))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn fork_of_nonexistent_work_returns_404() {
    let harness = Harness::new("fork-missing").await;
    let mut forker = harness.client();
    let (_forker_acct, _forker_pseud) =
        register(&mut forker, "forker-missing@example.com", "forkermissing").await;

    let fake_id = "00000000-0000-0000-0000-000000000000";
    let (status, _body) = forker
        .post(&format!("/api/v1/works/{fake_id}/fork"), json!({}))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn fork_inherits_parent_visibility_private() {
    let harness = Harness::new("fork-visibility").await;
    let mut author = harness.client();
    let (_author_acct, _author_pseud) =
        register(&mut author, "author-vis@example.com", "authorvis").await;

    // Author creates a private work with remix yes.
    let (status, body) = author
        .post("/api/v1/works", json!({ "title": "Private Original" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create: {body}");
    let parent_id = body["id"].as_str().unwrap().to_owned();

    // Set visibility to restricted (private).
    let (status, body) = author
        .patch(
            &format!("/api/v1/works/{parent_id}"),
            json!({ "expected_version": 1, "visibility": "restricted" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "set private: {body}");

    // Fork it.
    let (status, body) = author
        .post(&format!("/api/v1/works/{parent_id}/fork"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "fork: {body}");
    let fork_id = body["id"].as_str().unwrap().to_owned();

    // Fork should also be restricted.
    let (status, body) = author.get(&format!("/api/v1/works/{fork_id}")).await;
    assert_eq!(status, StatusCode::OK, "get fork: {body}");
    assert_eq!(body["visibility"], "restricted");
}

#[tokio::test]
async fn fork_depth_limit_enforced() {
    let harness = Harness::new("fork-depth").await;

    let mut author = harness.client();
    let (_author_acct, _author_pseud) =
        register(&mut author, "author-depth@example.com", "authordepth").await;

    // Create a chain: root → fork1 → fork2 → fork3 (depth 3, max is 3).
    let mut ids = Vec::new();
    let (status, body) = author
        .post("/api/v1/works", json!({ "title": "Root Work" }))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    ids.push(body["id"].as_str().unwrap().to_owned());

    for i in 0..3 {
        let parent = ids.last().unwrap();
        let (status, body) = author
            .post(&format!("/api/v1/works/{parent}/fork"), json!({}))
            .await;
        assert_eq!(status, StatusCode::CREATED, "fork {i}: {body}");
        ids.push(body["id"].as_str().unwrap().to_owned());
    }

    // 4th fork (depth 3 already reached) should fail.
    let deepest = ids.last().unwrap();
    let (status, body) = author
        .post(&format!("/api/v1/works/{deepest}/fork"), json!({}))
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "expected depth refusal: {body}"
    );
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("maximum depth"));
}

#[tokio::test]
async fn fork_lineage_survives_parent_deletion() {
    let harness = Harness::new("fork-orphan").await;
    let mut author = harness.client();
    let (_author_acct, _author_pseud) =
        register(&mut author, "author-orphan@example.com", "authororphan").await;

    let parent_id = create_work(&mut author, "Orphanable Title", "yes").await;

    // Fork the work.
    let (status, body) = author
        .post(&format!("/api/v1/works/{parent_id}/fork"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "fork: {body}");
    let fork_id = body["id"].as_str().unwrap().to_owned();

    // Withdraw the parent (soft-delete equivalent for works).
    let (status, _body) = author
        .post(
            &format!("/api/v1/works/{parent_id}/withdraw"),
            json!({ "expected_version": 2 }),
        )
        .await;
    assert!(status.is_success(), "withdraw should succeed: {status}");

    // Fork should still be accessible.
    let (status, body) = author.get(&format!("/api/v1/works/{fork_id}")).await;
    assert_eq!(status, StatusCode::OK, "fork still exists: {body}");

    // Lineage edge should survive.
    let edges = lorehaven_db::permission::lineage_edges_for_work(harness.tdb.db(), &fork_id)
        .await
        .expect("lineage edges");
    assert_eq!(
        edges.len(),
        1,
        "lineage edge should survive parent deletion"
    );
}

#[tokio::test]
async fn fork_inherits_parent_tags() {
    let harness = Harness::new("fork-tags").await;
    let mut author = harness.client();
    let (_author_acct, _author_pseud) =
        register(&mut author, "author-tags@example.com", "authortags").await;

    let parent_id = create_work(&mut author, "Tagged Work", "yes").await;

    // Add tags to parent via DB for test simplicity.
    let _ = lorehaven_db::taxonomy::tag_work(harness.tdb.db(), &parent_id, "tag-1", 1).await;
    let _ = lorehaven_db::taxonomy::tag_work(harness.tdb.db(), &parent_id, "tag-2", 1).await;

    // Fork.
    let (status, body) = author
        .post(&format!("/api/v1/works/{parent_id}/fork"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "fork: {body}");
    let fork_id = body["id"].as_str().unwrap().to_owned();

    // Fork should inherit tags.
    let fork_tags = lorehaven_db::taxonomy::tags_for_work(harness.tdb.db(), &fork_id)
        .await
        .expect("tags for work");
    assert!(fork_tags.contains(&"tag-1".to_owned()), "tag-1 inherited");
    assert!(fork_tags.contains(&"tag-2".to_owned()), "tag-2 inherited");
}
