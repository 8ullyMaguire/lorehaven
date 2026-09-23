//! Integration tests for flexible bounties (spec §20.3.2, M18 Phase 4.1).
//!
//! Covers:
//! - Standard bounty creation (backward-compatible)
//! - Crowdfunded bounty creation, contribution, and activation
//! - Reverse bounty creation (prepaid, open immediately)
//! - Disallowed bounty type rejection
//! - Amount bounds enforcement

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::DatabaseConfig;
use serde_json::{json, Value};
use std::path::PathBuf;
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-bounties-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn config_for(dir: &PathBuf) -> Config {
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

const PASSWORD: &str = "test-pass-1234";

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
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        for (name, value) in &self.cookies {
            builder = builder.header(header::COOKIE, format!("{name}={value}"));
        }
        let body = match body {
            Some(v) => Body::from(v.to_string()),
            None => Body::empty(),
        };
        let response = self
            .app
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        self.capture(&response);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into_owned()));
        (status, json)
    }
    async fn post(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("POST", uri, Some(body)).await
    }
    async fn get(&mut self, uri: &str) -> (StatusCode, Value) {
        self.request("GET", uri, None).await
    }
    async fn register(&mut self, email: &str, handle: &str) -> String {
        let (status, body) = self
            .post(
                "/api/v1/auth/register",
                json!({
                    "email": email,
                    "password": PASSWORD,
                    "handle": handle,
                    "display_name": handle,
                    "age_band": "adult",
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "register {handle}: {body}");
        let (status, me) = self.get("/api/v1/auth/me").await;
        assert_eq!(status, StatusCode::OK, "{me}");
        me["account"]["id"].as_str().expect("account id").to_owned()
    }
}

#[tokio::test]
async fn standard_bounty_creation_succeeds() {
    let h = Harness::new("standard").await;
    let mut c = h.client();
    c.register("std-user@b.test", "stduser").await;
    let (status, body) = c
        .post(
            "/api/v1/bounties",
            json!({
                "job_kind": "illustration",
                "terms": "A dragon in watercolour",
                "amount": 50,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "expected 200, got {body:?}");
    assert_eq!(body["created"], true);
}

#[tokio::test]
async fn crowdfunded_bounty_contribution_activates() {
    let h = Harness::new("crowdfunded").await;
    let mut c = h.client();
    c.register("cf-user@b.test", "cfuser").await;

    // Create crowdfunded bounty (amount 100, threshold 100%).
    let (status, body) = c
        .post(
            "/api/v1/bounties",
            json!({
                "job_kind": "beta_read",
                "terms": "Feedback on chapter 1-3",
                "amount": 100,
                "bounty_type": "crowdfunded",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "create failed: {body:?}");
    let bounty_id = body["id"].as_str().unwrap().to_string();

    // Contribute 50 — not enough to activate.
    let (status, body) = c
        .post(
            &format!("/api/v1/bounties/{bounty_id}/contribute"),
            json!({ "amount": 50 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "contribute failed: {body:?}");
    assert_eq!(body["funded_amount"], 50);
    assert_eq!(body["activated"], false);

    // Contribute another 50 — activates.
    let (status, body) = c
        .post(
            &format!("/api/v1/bounties/{bounty_id}/contribute"),
            json!({ "amount": 50 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "second contribute failed: {body:?}");
    assert_eq!(body["funded_amount"], 100);
    assert_eq!(body["activated"], true);
}

#[tokio::test]
async fn reverse_bounty_is_open_immediately() {
    let h = Harness::new("reverse").await;
    let mut c = h.client();
    c.register("rev-user@b.test", "revuser").await;

    let (status, body) = c
        .post(
            "/api/v1/bounties",
            json!({
                "job_kind": "fic_prompt",
                "terms": "A coffee shop AU",
                "amount": 200,
                "bounty_type": "reverse",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "reverse create failed: {body:?}");
    assert_eq!(body["created"], true);

    // List bounties — reverse should appear as 'open'.
    let (status, body) = c.get("/api/v1/bounties").await;
    assert_eq!(status, StatusCode::OK, "list failed: {body:?}");
    let bounties = body["bounties"].as_array().unwrap();
    assert!(
        bounties
            .iter()
            .any(|b| b["type"] == "reverse" && b["state"] == "open"),
        "reverse bounty not open: {bounties:?}"
    );
}

#[tokio::test]
async fn disallowed_bounty_type_rejected() {
    let h = Harness::new("disallowed").await;
    let mut c = h.client();
    c.register("dis-user@b.test", "disuser").await;

    let (status, _body) = c
        .post(
            "/api/v1/bounties",
            json!({
                "job_kind": "anything",
                "terms": "test",
                "amount": 50,
                "bounty_type": "collaborative",
            }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "expected 422 for disallowed type"
    );
}

#[tokio::test]
async fn bounty_amount_out_of_bounds_rejected() {
    let h = Harness::new("bounds").await;
    let mut c = h.client();
    c.register("bnd-user@b.test", "bnduser").await;

    // Below minimum (default min is 10).
    let (status, _) = c
        .post(
            "/api/v1/bounties",
            json!({ "job_kind": "x", "terms": "y", "amount": 1 }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "expected 422 for amount below min"
    );

    // Above maximum (default max is 10000).
    let (status, _) = c
        .post(
            "/api/v1/bounties",
            json!({ "job_kind": "x", "terms": "y", "amount": 999999 }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "expected 422 for amount above max"
    );
}
