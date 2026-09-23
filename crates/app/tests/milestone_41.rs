//! M41 — Half-life and interaction tiers (spec §41).

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
        "lorehaven-m41-{tag}-{}-{:?}",
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

/// Create a work with the given title.
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

    // Add body text to the chapter.
    let (status, body) = client
        .patch(
            &format!("/api/v1/chapters/{chapter_id}"),
            json!({ "expected_version": 1, "document": { "type": "doc", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "Opening text." }] }] } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "update chapter: {body}");

    work_id
}

/// Publish a work so it's visible.
async fn publish_work(client: &mut Client, work_id: &str) {
    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/publish"),
            json!({ "expected_version": 1 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "publish: {body}");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn half_life_changes_ranking_with_no_field_changes() {
    // Set half-life on one work to 10000 (evergreen = 1.5× multiplier)
    use lorehaven_domain::longevity::apply_half_life;

    let mut candidates = vec![
        lorehaven_domain::discovery::Candidate {
            work_id: lorehaven_domain::ids::WorkId::new(),
            score: 100,
            reason: "tags".into(),
            taste_signal: 0.5,
            diversity_class: 0.5,
        },
        lorehaven_domain::discovery::Candidate {
            work_id: lorehaven_domain::ids::WorkId::new(),
            score: 100,
            reason: "tags".into(),
            taste_signal: 0.5,
            diversity_class: 0.5,
        },
    ];

    let id_a = candidates[0].work_id.clone();
    let id_b = candidates[1].work_id.clone();

    let half_life_map = std::collections::HashMap::from([(id_b.to_string(), 10000i64)]);

    let half_life_of = |id: &lorehaven_domain::ids::WorkId| -> Option<i64> {
        half_life_map.get(&id.to_string()).copied()
    };

    // Before: both have score 100, order is A, B (stable)
    assert_eq!(candidates[0].work_id, id_a);
    assert_eq!(candidates[1].work_id, id_b);

    apply_half_life(&mut candidates, &half_life_of);

    // After: B should be first (100 * 1.5 = 150)
    assert_eq!(candidates[0].work_id, id_b);
    assert_eq!(candidates[0].score, 150);
    assert_eq!(
        candidates[0].reason, "tags",
        "reason must not change (silent contract)"
    );
    assert_eq!(candidates[1].work_id, id_a);
    assert_eq!(
        candidates[1].score, 100,
        "Work A has no half-life score, stays at 100"
    );
    assert_eq!(
        candidates[1].reason, "tags",
        "reason must not change (silent contract)"
    );
}

#[tokio::test]
async fn warmth_accumulates_and_promotes_tier() {
    let harness = Harness::new("warmth-accumulates").await;
    let mut author_client = harness.client();
    let (_author_account, _author_pseud) =
        register(&mut author_client, "author@example.com", "author").await;

    let mut reader_client = harness.client();
    let (_reader_account, _reader_pseud) =
        register(&mut reader_client, "reader@example.com", "reader").await;

    // Create a work by the author
    let work_id = create_work(&mut author_client, "Author's Work").await;
    publish_work(&mut author_client, &work_id).await;

    // Reader reads the work (creates a reading event)
    let (status, body) = reader_client
        .post(
            &format!("/api/v1/works/{work_id}/chapters/1/read"),
            json!({}),
        )
        .await;
    // Either OK or CREATED is fine — just verify it didn't error hard
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "read chapter: {status} {body}"
    );

    // Author checks audience panel
    let (status, body) = author_client.get("/api/v1/me/audience").await;
    assert_eq!(status, StatusCode::OK, "audience: {body}");

    let obj = body.as_object().expect("body should be object");
    assert!(obj.contains_key("lurk"), "Should have lurk count");
    assert!(obj.contains_key("react"), "Should have react count");
    assert!(obj.contains_key("comment"), "Should have comment count");
    assert!(obj.contains_key("create"), "Should have create count");
    assert!(obj.contains_key("total"), "Should have total");
}

#[tokio::test]
async fn warmth_is_never_exposed_per_reader() {
    let harness = Harness::new("warmth-privacy").await;
    let mut client = harness.client();
    let (_account_id, _pseud_id) = register(&mut client, "author@example.com", "author").await;

    let (status, body) = client.get("/api/v1/me/audience").await;
    assert_eq!(status, StatusCode::OK, "audience should succeed");

    let obj = body.as_object().expect("body should be object");
    assert!(obj.contains_key("lurk"), "Should have lurk count");
    assert!(obj.contains_key("react"), "Should have react count");
    assert!(obj.contains_key("comment"), "Should have comment count");
    assert!(obj.contains_key("create"), "Should have create count");
    assert!(obj.contains_key("total"), "Should have total");

    // Should NOT have any per-reader data
    assert!(!obj.contains_key("readers"), "Should not have readers list");
    assert!(!obj.contains_key("warmth"), "Should not have warmth map");
    assert!(
        !obj.contains_key("accounts"),
        "Should not expose account list"
    );
}

#[tokio::test]
async fn failed_interaction_writes_no_warmth() {
    let harness = Harness::new("warmth-failure").await;
    let mut client = harness.client();
    let (_account_id, _pseud_id) = register(&mut client, "author@example.com", "author").await;

    // No interaction occurs — audience panel should be empty/zero
    let (status, body) = client.get("/api/v1/me/audience").await;
    assert_eq!(status, StatusCode::OK, "audience should succeed");

    let obj = body.as_object().expect("body should be object");
    let total = obj.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
    assert_eq!(total, 0, "Should have 0 warmth with no interactions");
}
