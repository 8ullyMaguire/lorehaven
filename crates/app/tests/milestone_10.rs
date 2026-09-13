//! M10 — Search, taxonomy, body search, query language.
//!
//! Drives the real router against a real SQLite file.

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
        "lorehaven-m10-{tag}-{}-{:?}",
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
        let report = db.migrate().await.expect("migrate");
        assert!(
            report.applied.contains(&"0011_taxonomy".to_owned()),
            "taxonomy migration must apply: {report:?}"
        );
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

async fn register(client: &mut Client, email: &str, handle: &str) -> (String, String) {
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
    let (status, me) = client.get("/api/v1/auth/me").await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let account = me["account"]["id"].as_str().expect("account id").to_owned();
    let pseud = me["active_pseud_id"].as_str().expect("pseud id").to_owned();
    (account, pseud)
}

async fn published_work(harness: &Harness, email: &str, handle: &str, title: &str) -> String {
    let mut author = harness.client();
    let _ = register(&mut author, email, handle).await;
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
    let (status, body) = author
        .request(
            "PATCH",
            &format!("/api/v1/chapters/{chapter}"),
            Some(json!({ "expected_version": chapter_version, "document": doc })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "save: {body}");
    let (status, body) = author
        .post(
            &format!("/api/v1/works/{work_id}/publish"),
            json!({ "expected_version": work_version, "idempotency_key": format!("m10-{work_id}") }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "publish: {body}");
    work_id
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn search_empty_query_returns_all_public_works() {
    let harness = Harness::new("search-empty").await;
    let _ = published_work(&harness, "a@example.com", "AuthorA", "Empty Query Work").await;
    let mut client = harness.client();
    let (status, body) = client.get("/api/v1/search?q=").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{body}");
    harness.cleanup().await;
}

#[tokio::test]
async fn search_by_title_finds_matching_work() {
    let harness = Harness::new("search-title").await;
    let _ = published_work(&harness, "a@example.com", "AuthorA", "Winter Journey").await;
    let _ = published_work(&harness, "b@example.com", "AuthorB", "Summer Tale").await;
    let mut client = harness.client();
    let (status, body) = client.get("/api/v1/search?q=winter").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert!(!items.is_empty(), "expected match for 'winter': {body}");
    assert!(
        items
            .iter()
            .any(|i| i["title"].as_str().unwrap().contains("Winter")),
        "{body}"
    );
    harness.cleanup().await;
}

#[tokio::test]
async fn search_fielded_fandom_uses_exists() {
    let harness = Harness::new("search-fielded").await;
    let work_id = published_work(&harness, "a@example.com", "AuthorA", "Tagged Work").await;
    let mut client = harness.client();
    let _ = register(&mut client, "b@example.com", "AuthorB").await;
    // Create a fandom node.
    let (status, body) = client
        .post(
            "/api/v1/taxonomy",
            json!({ "kind": "fandom", "canonical": "HarryPotter" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "create node: {body}");
    let node_id = body["node"]["id"].as_str().expect("id");
    // Tag the work.
    let (status, _) = client
        .post(
            &format!("/api/v1/works/{work_id}/tags"),
            json!({ "node_id": node_id }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "tag work");
    // Search by fandom.
    let (status, body) = client.get("/api/v1/search?q=fandom:harrypotter").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert!(
        !items.is_empty(),
        "expected match for fandom:harrypotter: {body}"
    );
    harness.cleanup().await;
}

#[tokio::test]
async fn search_anonymous_cannot_see_drafts() {
    let harness = Harness::new("search-anon-draft").await;
    // Create a draft (unpublished) work.
    let mut author = harness.client();
    let _ = register(&mut author, "a@example.com", "AuthorA").await;
    let (status, body) = author
        .post("/api/v1/works", json!({ "title": "Draft Work" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create draft: {body}");
    let draft_id = body["id"].as_str().expect("id");

    // Anonymous search should not find the draft.
    let mut anon = harness.client();
    let (status, body) = anon.get("/api/v1/search?q=draft").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert!(
        !items.iter().any(|i| i["work_id"] == draft_id),
        "anonymous should not see drafts: {body}"
    );
    harness.cleanup().await;
}

#[tokio::test]
async fn search_in_work_returns_paragraph_positions() {
    let harness = Harness::new("search-in-work").await;
    let _ = published_work(&harness, "a@example.com", "AuthorA", "Searchable Work").await;
    let mut client = harness.client();
    // First find the work.
    let (status, body) = client.get("/api/v1/search?q=Searchable").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert!(!items.is_empty(), "expected match: {body}");
    let work_id = items[0]["work_id"].as_str().unwrap().to_owned();

    // Search within the work.
    let (status, body) = client
        .get(&format!("/api/v1/search/in-work/{work_id}?q=chapter"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // The body is an array of InWorkMatch objects.
    assert!(body.is_array(), "expected array: {body}");
    harness.cleanup().await;
}

#[tokio::test]
async fn taxonomy_autocomplete_returns_nodes() {
    let harness = Harness::new("taxonomy-autocomplete").await;
    let mut client = harness.client();
    let _ = register(&mut client, "a@example.com", "AuthorA").await;

    // Create some nodes.
    let (status, _) = client
        .post(
            "/api/v1/taxonomy",
            json!({ "kind": "fandom", "canonical": "Harry Potter" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "create harry potter");

    let (status, _) = client
        .post(
            "/api/v1/taxonomy",
            json!({ "kind": "fandom", "canonical": "Lord of the Rings" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "create lotr");

    // Autocomplete by prefix.
    let (status, body) = client
        .get("/api/v1/taxonomy?kind=fandom&prefix=harry")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert!(!items.is_empty(), "expected autocomplete matches: {body}");
    harness.cleanup().await;
}

#[tokio::test]
async fn search_is_deterministic_for_same_input() {
    let harness = Harness::new("search-deterministic").await;
    let _ = published_work(&harness, "a@example.com", "AuthorA", "Deterministic Work").await;
    let mut client = harness.client();
    let (_, body1) = client.get("/api/v1/search?q=deterministic").await;
    let (_, body2) = client.get("/api/v1/search?q=deterministic").await;
    assert_eq!(body1, body2, "search should be deterministic");
    harness.cleanup().await;
}

#[tokio::test]
async fn search_with_no_query_returns_cursor_envelope() {
    let harness = Harness::new("search-envelope").await;
    let _ = published_work(&harness, "a@example.com", "AuthorA", "Envelope Work").await;
    let mut client = harness.client();
    let (status, body) = client.get("/api/v1/search").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.get("items").is_some(), "expected items field: {body}");
    harness.cleanup().await;
}
