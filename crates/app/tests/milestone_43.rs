//! M43 — Browse ordering vocabulary (spec §43).
//!
//! Tests cover the §43.2 vocabulary (one Sort enum everywhere),
//! the §43.4 stickiness API (GET /browse/sort/:surface, PUT, DELETE),
//! and the per-surface default-sort contract.

use std::path::Path;
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
        "lorehaven-m43-{tag}-{}-{:?}",
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

struct Harness {
    _dir: PathBuf,
    tdb: test_support::TestDb,
    config: Config,
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
        let config = config_for(&dir);
        Self {
            _dir: dir,
            tdb,
            config,
        }
    }

    fn client(&self) -> Client {
        Client::new(server::build_router(AppState::new(
            self.config.clone(),
            self.tdb.db().clone(),
        )))
    }
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
                .body(Body::from(v.to_string()))
                .expect("build request"),
            None => builder.body(Body::empty()).expect("build request"),
        };
        let response = self.app.clone().oneshot(request).await.expect("oneshot");
        let status = response.status();
        self.capture(&response);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let json: Value = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).expect("json body")
        };
        (status, json)
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
    async fn post(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("POST", uri, Some(body)).await
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

#[tokio::test]
async fn discovery_sort_query_param_overrides_default() {
    let harness = Harness::new("discovery-sort-query").await;
    let mut client = harness.client();

    let (status, body) = client.get("/api/v1/discovery?sort=top").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["sort"], "top");
}

#[tokio::test]
async fn discovery_sort_default_for_anonymous() {
    let harness = Harness::new("discovery-sort-default").await;
    let mut client = harness.client();

    let (status, body) = client.get("/api/v1/discovery").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["sort"], "for-you");
}

#[tokio::test]
async fn discovery_sort_query_param_unknown_falls_through_to_default() {
    let harness = Harness::new("discovery-sort-unknown").await;
    let mut client = harness.client();

    // Unknown sort values should not 400 — they fall through to default (spec §43.4).
    let (status, body) = client.get("/api/v1/discovery?sort=bogus").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["sort"], "for-you");
}

#[tokio::test]
async fn list_surfaces_returns_all_browse_surfaces() {
    let harness = Harness::new("list-surfaces").await;
    let mut client = harness.client();

    let (status, body) = client.get("/api/v1/browse/surfaces").await;
    assert_eq!(status, StatusCode::OK);

    let surfaces = body.as_array().expect("surfaces array");
    assert!(!surfaces.is_empty());

    let discover = surfaces
        .iter()
        .find(|s| s["key"] == "discover")
        .expect("discover surface");
    assert_eq!(discover["default_sort"], "for-you");

    let people = surfaces
        .iter()
        .find(|s| s["key"] == "people")
        .expect("people surface");
    assert_eq!(people["default_sort"], "az");
}

#[tokio::test]
async fn get_sort_returns_default_for_anonymous_surface() {
    let harness = Harness::new("get-sort-default").await;
    let mut client = harness.client();

    let (status, body) = client.get("/api/v1/browse/sort/discover").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["surface"], "discover");
    assert_eq!(body["sort"], "for-you");
    assert_eq!(body["source"], "default");
}

#[tokio::test]
async fn set_sort_stores_preference_and_get_returns_it() {
    let harness = Harness::new("set-sort-stores").await;
    let mut client = harness.client();
    register(&mut client, "m43-a@t.test", "m43a").await;

    let (status, body) = client
        .put("/api/v1/browse/sort/discover", json!({ "sort": "top" }))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["sort"], "top");
    assert_eq!(body["source"], "preference");

    let (status, body) = client.get("/api/v1/browse/sort/discover").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["sort"], "top");
    assert_eq!(body["source"], "preference");
}

#[tokio::test]
async fn set_sort_rejects_unknown_sort() {
    let harness = Harness::new("set-sort-rejects").await;
    let mut client = harness.client();
    register(&mut client, "m43-b@t.test", "m43b").await;

    let (status, body) = client
        .put("/api/v1/browse/sort/discover", json!({ "sort": "random" }))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let msg = body["error"]["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("unknown sort"),
        "expected 'unknown sort' in message: {body}"
    );
    assert!(
        msg.contains("accepted:"),
        "error should list accepted values: {body}"
    );
}

#[tokio::test]
async fn delete_sort_restores_default() {
    let harness = Harness::new("delete-sort").await;
    let mut client = harness.client();
    register(&mut client, "m43-c@t.test", "m43c").await;

    client
        .put("/api/v1/browse/sort/discover", json!({ "sort": "top" }))
        .await;

    let (status, body) = client.delete("/api/v1/browse/sort/discover").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["sort"], "for-you");
    assert_eq!(body["source"], "default");
}

#[tokio::test]
async fn sort_preferences_are_per_surface() {
    let harness = Harness::new("sort-per-surface").await;
    let mut client = harness.client();
    register(&mut client, "m43-d@t.test", "m43d").await;

    client
        .put("/api/v1/browse/sort/discover", json!({ "sort": "top" }))
        .await;

    client
        .put("/api/v1/browse/sort/people", json!({ "sort": "new" }))
        .await;

    let (status, body) = client.get("/api/v1/browse/sort/discover").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["sort"], "top");

    let (status, body) = client.get("/api/v1/browse/sort/people").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["sort"], "new");
}

#[tokio::test]
async fn sort_default_contract_discover_for_you_rest_new() {
    let harness = Harness::new("default-contract").await;
    let mut client = harness.client();

    let cases: Vec<(&str, &str)> = vec![
        ("discover", "for-you"),
        ("people", "az"),
        ("library", "new"),
        ("tags", "az"),
        ("fandoms", "az"),
        ("collections", "new"),
        ("series", "new"),
        ("authors", "az"),
    ];
    for (surface, expected_default) in cases {
        let uri = format!("/api/v1/browse/sort/{surface}");
        let (status, body) = client.get(&uri).await;
        assert_eq!(status, StatusCode::OK, "surface {surface}");
        assert_eq!(
            body["sort"], expected_default,
            "surface {surface}: expected {expected_default}, got {}",
            body["sort"]
        );
        assert_eq!(body["source"], "default");
    }
}

#[tokio::test]
async fn set_sort_then_delete_then_get_returns_default() {
    let harness = Harness::new("set-delete-get").await;
    let mut client = harness.client();
    register(&mut client, "m43-e@t.test", "m43e").await;

    client
        .put("/api/v1/browse/sort/people", json!({ "sort": "trending" }))
        .await;
    client.delete("/api/v1/browse/sort/people").await;

    let (status, body) = client.get("/api/v1/browse/sort/people").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["sort"], "az");
    assert_eq!(body["source"], "default");
}
