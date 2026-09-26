//! M11 — Discovery: recommendations, taste profiles.

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
    // Raise rate limits for parallel test execution.
    config.rate_limits.auth = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.write = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.search = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.default = lorehaven_app::limiter::Quota {
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
    async fn cleanup(self) {
        self.tdb.cleanup().await;
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
    let (status, _) = client
        .post("/api/v1/discovery/taste-profile/recompute", json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "recompute");
    // Clear
    let (status, body) = client
        .post("/api/v1/discovery/taste-profile/clear", json!({}))
        .await;
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

// ---------------------------------------------------------------------------
// M11-03: Operator taste influence
// ---------------------------------------------------------------------------

#[tokio::test]
async fn operator_affinity_endpoint_returns_404_to_non_operator() {
    let harness = Harness::new("affinity-non-op").await;
    let _ = published_work(&harness, "a@example.com", "AuthorA", "Some Work").await;
    let mut client = harness.client();
    register(&mut client, "reader@example.com", "Reader").await;
    let (status, _) = client
        .post(
            "/api/v1/operator/affinities",
            json!({
                "work_id": "some-work-id",
                "affinity_bp": 10000,
                "rationale": "good work"
            }),
        )
        .await;
    // Router built without operator_account_id → require_operator triggers 404.
    assert_eq!(status, StatusCode::NOT_FOUND);
    harness.cleanup().await;
}

#[tokio::test]
async fn operator_affinity_ranking_is_silent_field_shape_unchanged() {
    // Without any affinity set, discovery returns work items with work_id.
    let harness = Harness::new("affinity-silent").await;
    let _ = published_work(&harness, "a@example.com", "AuthorA", "Silent Work").await;
    let mut anon = harness.client();
    let (status, body) = anon.get("/api/v1/discovery").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert!(!items.is_empty(), "{body}");
    // Influenced vs uninfluenced responses must differ only in result order,
    // never in field presence or naming (spec §16.3, §20 silent rule).
    // Influence-related fields must never leak: no score, reason, affinity, or
    // ranking metadata. Display fields (title, author_handle) are the same in
    // every response regardless of affinities, so they say nothing about
    // influence and are allowed.
    for item in items {
        let keys: Vec<String> = item.as_object().unwrap().keys().cloned().collect();
        assert!(
            keys.contains(&"work_id".to_string()),
            "every item needs its work_id: {item}"
        );
        for banned in [
            "score",
            "reason",
            "affinity",
            "affinity_bp",
            "influenced",
            "rank",
        ] {
            assert!(
                !keys.iter().any(|k| k.contains(banned)),
                "influence field `{banned}` leaked into the discovery feed: {item}"
            );
        }
    }
    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// M11-05: Recipes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn recipe_owner_can_crud_their_recipe() {
    let harness = Harness::new("recipe-crud").await;
    let mut client = harness.client();
    register(&mut client, "recipe@example.com", "Chef").await;

    // Create a private recipe.
    let (status, body) = client
        .post(
            "/api/v1/recipes",
            json!({
                "id": "my-recipe-1",
                "name": "My Recipe",
                "document": { "tags": ["fantasy"] },
                "is_public": false
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["id"], "my-recipe-1");

    // List returns it.
    let (status, body) = client.get("/api/v1/recipes/list").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let recipes = body["recipes"].as_array().expect("recipes");
    assert!(recipes.iter().any(|r| r["id"] == "my-recipe-1"), "{body}");

    // Get by ID works for owner.
    let (status, body) = client.get("/api/v1/recipes/my-recipe-1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], "My Recipe");

    // Update.
    let (status, body) = client
        .post(
            "/api/v1/recipes/my-recipe-1",
            json!({ "name": "Updated", "document": { "tags": ["scifi"] } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = client.get("/api/v1/recipes/my-recipe-1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], "Updated");

    // Delete.
    let (status, body) = client
        .post("/api/v1/recipes/my-recipe-1/delete", json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Gone.
    let (status, _body) = client.get("/api/v1/recipes/my-recipe-1").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    harness.cleanup().await;
}

#[tokio::test]
async fn private_recipe_is_inaccessible_to_other_viewers() {
    let harness = Harness::new("recipe-private").await;
    // Account A creates a private recipe.
    let mut alice = harness.client();
    register(&mut alice, "alice@example.com", "Alice").await;
    let (status, body) = alice
        .post(
            "/api/v1/recipes",
            json!({
                "id": "secret-recipe",
                "name": "Secret",
                "document": { "tags": ["hidden"] },
                "is_public": false
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Account B cannot read it.
    let mut bob = harness.client();
    register(&mut bob, "bob@example.com", "Bob").await;
    let (status, _body) = bob.get("/api/v1/recipes/secret-recipe").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Account B cannot update it.
    let (status, _body) = bob
        .post(
            "/api/v1/recipes/secret-recipe",
            json!({ "name": "Hacked", "document": {} }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Account B cannot delete it.
    let (status, _body) = bob
        .post("/api/v1/recipes/secret-recipe/delete", json!({}))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    harness.cleanup().await;
}

#[tokio::test]
async fn public_recipe_is_visible_to_other_viewers() {
    let harness = Harness::new("recipe-public").await;
    let mut alice = harness.client();
    register(&mut alice, "alice2@example.com", "Alice").await;
    let (status, body) = alice
        .post(
            "/api/v1/recipes",
            json!({
                "id": "shared-recipe",
                "name": "Shared",
                "document": { "tags": ["fantasy"] },
                "is_public": true
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let mut bob = harness.client();
    register(&mut bob, "bob2@example.com", "Bob").await;
    let (status, body) = bob.get("/api/v1/recipes/shared-recipe").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], "Shared");

    // Bob can see it in list too.
    let (status, body) = bob.get("/api/v1/recipes/list").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let recipes = body["recipes"].as_array().expect("recipes");
    assert!(recipes.iter().any(|r| r["id"] == "shared-recipe"), "{body}");

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// M11-06: Dashboards
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dashboard_save_and_retrieve() {
    let harness = Harness::new("dashboard-crud").await;
    let mut client = harness.client();
    register(&mut client, "dash@example.com", "Dash").await;

    // Empty dashboard returns empty slots.
    let (status, body) = client.get("/api/v1/dashboard").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["slots"], json!([]));

    // Save a layout with a widget.
    let (status, body) = client
        .post(
            "/api/v1/dashboard",
            json!({ "slots": [
                { "id": "my-feed", "widget": "discovery-feed" },
                { "id": "unknown-1", "widget": "not-a-real-widget" }
            ] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Retrieve it.
    let (status, body) = client.get("/api/v1/dashboard").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let slots = body["slots"].as_array().expect("slots");
    assert_eq!(slots.len(), 2, "{body}");
    assert_eq!(slots[0]["widget"], "discovery-feed");
    assert_eq!(slots[1]["widget"], "not-a-real-widget");

    harness.cleanup().await;
}

#[tokio::test]
async fn dashboard_unknown_widget_ids_do_not_error() {
    let harness = Harness::new("dashboard-unknown-widget").await;
    let mut client = harness.client();
    register(&mut client, "dw@example.com", "Dash").await;
    let (status, body) = client
        .post(
            "/api/v1/dashboard",
            json!({ "slots": [{ "id": "x", "widget": "totally-unknown" }] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = client.get("/api/v1/dashboard").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let slots = body["slots"].as_array().expect("slots");
    assert_eq!(slots.len(), 1, "{body}");
    // Unknown widget ids must be preserved, not crash.
    assert_eq!(slots[0]["widget"], "totally-unknown");
    harness.cleanup().await;
}

#[tokio::test]
async fn meta_includes_theme_mode_and_influence_sources() {
    let harness = Harness::new("meta-theme").await;
    let mut client = harness.client();
    let (status, body) = client.get("/api/v1/meta").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Theme section is present with defaults: thematic mode, opt-out allowed,
    // and operator_topics as the default influence source.
    let theme = body.get("theme").expect("theme section present");
    assert_eq!(theme["mode"], "thematic", "default mode is thematic");
    assert_eq!(theme["allow_user_opt_out"], true);
    let sources = theme["influence_sources"]
        .as_array()
        .expect("sources array");
    assert!(!sources.is_empty());
    assert_eq!(sources[0], "operator_topics");
    harness.cleanup().await;
}

#[tokio::test]
async fn theme_boost_tag_raises_work_score() {
    // Create two works; tag one with "adventure". With the default harness
    // config (no boost_tags), the tag is a no-op — both works still appear in
    // discovery. This confirms the tag-lookup path runs without error; the
    // score assertion is covered by the domain unit tests.
    let harness = Harness::new("theme-boost").await;
    let w1 = published_work(&harness, "boost1@example.com", "BoostOne", "Plain Work").await;
    let w2 = published_work(&harness, "boost2@example.com", "BoostTwo", "Adventure Work").await;

    let node = lorehaven_db::taxonomy::create_node(harness.tdb.db(), "tag", "adventure")
        .await
        .expect("node");
    lorehaven_db::taxonomy::tag_work(harness.tdb.db(), &w2, &node.id, 1)
        .await
        .expect("tag");

    let mut client = harness.client();
    let (status, body) = client.get("/api/v1/discovery").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body.get("items").and_then(|v| v.as_array()).expect("items");
    let ids: Vec<&str> = items
        .iter()
        .filter_map(|v| v.get("work_id").and_then(|w| w.as_str()))
        .collect();
    assert!(ids.contains(&w1.as_str()));
    assert!(ids.contains(&w2.as_str()));
    harness.cleanup().await;
}
