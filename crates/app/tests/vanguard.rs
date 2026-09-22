//! Integration tests for Taste Vanguard Role (spec §16.18, M18 Phase 4.2).
//!
//! Covers:
//! - Grant/revoke vanguard role (admin only)
//! - Vanguard status check
//! - Pin/unpin works (vanguard only)
//! - List vanguards (admin only)
//! - Non-vanguard users cannot pin
//! - Vanguard role config defaults

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
        "lorehaven-vanguard-{tag}-{}-{:?}",
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
    async fn delete(&mut self, uri: &str) -> (StatusCode, Value) {
        self.request("DELETE", uri, None).await
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

/// Make an account the instance operator (TL6) straight in the DB.
async fn make_operator(tdb: &test_support::TestDb, account_id: &str) {
    lorehaven_db::directory::set_account_trust_for_tests(tdb.db(), account_id, 6).await;
}

async fn create_work(client: &mut Client, title: &str) -> String {
    let (status, body) = client
        .post(
            "/api/v1/works",
            json!({
                "title": title,
                "summary": "A test work for vanguard tests.",
                "rating": "general",
                "status": "in_progress",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    body["id"].as_str().expect("work id").to_owned()
}

#[tokio::test]
async fn grant_and_check_vanguard_role() {
    let harness = Harness::new("grant-vanguard").await;
    let mut operator = harness.client();
    let (op_account, _) = register(&mut operator, "op@v.test", "vanguard-op").await;
    make_operator(&harness.tdb, &op_account).await;

    let mut user = harness.client();
    let (user_account, _) = register(&mut user, "user@v.test", "vanguard-user").await;

    // Grant vanguard role.
    let (status, body) = operator
        .post(
            "/api/v1/vanguards",
            json!({
                "account_id": user_account,
                "selection_method": "admin_appointment",
                "resonance_score": 0.85,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "grant vanguard: {body}");

    // Check vanguard status.
    let (status, body) = user.get("/api/v1/vanguard/status").await;
    assert_eq!(status, StatusCode::OK, "vanguard status: {body}");
    assert_eq!(body["is_vanguard"], true, "should be vanguard after grant");
}

#[tokio::test]
async fn revoke_vanguard_role() {
    let harness = Harness::new("revoke-vanguard").await;
    let mut operator = harness.client();
    let (op_account, _) = register(&mut operator, "op@v.test", "vanguard-op").await;
    make_operator(&harness.tdb, &op_account).await;

    let mut user = harness.client();
    let (user_account, _) = register(&mut user, "user@v.test", "vanguard-user").await;

    // Grant first.
    let (status, _body) = operator
        .post(
            "/api/v1/vanguards",
            json!({
                "account_id": user_account,
                "selection_method": "admin_appointment",
                "resonance_score": 0.85,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    // Revoke.
    let revoke_uri = format!("/api/v1/vanguards/{}", user_account);
    let (status, body) = operator.delete(&revoke_uri).await;
    assert_eq!(status, StatusCode::OK, "revoke vanguard: {body}");

    // Confirm no longer vanguard.
    let (status, body) = user.get("/api/v1/vanguard/status").await;
    assert_eq!(status, StatusCode::OK, "vanguard status after revoke: {body}");
    assert_eq!(body["is_vanguard"], false);
}

#[tokio::test]
async fn non_vanguard_cannot_pin_work() {
    let harness = Harness::new("non-vanguard-pin").await;
    let mut user = harness.client();
    let (_account, _) = register(&mut user, "user@v.test", "regular-user").await;
    let work_id = create_work(&mut user, "pin test work").await;

    let pin_uri = format!("/api/v1/vanguard/pins/{}", work_id);
    let (status, body) = user
        .post(
            &pin_uri,
            json!({
                "pin_reason": "curated",
                "message": "This is a great work.",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "non-vanguard pin: {body}");
}

#[tokio::test]
async fn vanguard_can_pin_and_unpin_work() {
    let harness = Harness::new("vanguard-pin").await;
    let mut operator = harness.client();
    let (op_account, _) = register(&mut operator, "op@v.test", "vanguard-op").await;
    make_operator(&harness.tdb, &op_account).await;

    let mut user = harness.client();
    let (user_account, _) = register(&mut user, "user@v.test", "vanguard-user").await;
    let work_id = create_work(&mut user, "vanguard pin work").await;

    // Grant vanguard.
    let (status, _body) = operator
        .post(
            "/api/v1/vanguards",
            json!({
                "account_id": user_account,
                "selection_method": "admin_appointment",
                "resonance_score": 0.9,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    // Pin.
    let pin_uri = format!("/api/v1/vanguard/pins/{}", work_id);
    let (status, body) = user
        .post(
            &pin_uri,
            json!({
                "pin_reason": "curated",
                "message": "Excellent character work.",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "pin work: {body}");

    // Verify pin exists.
    let pins_uri = format!("/api/v1/vanguard/pins/{}", work_id);
    let (status, body) = user.get(&pins_uri).await;
    assert_eq!(status, StatusCode::OK, "get pins: {body}");
    let pins = body["pins"].as_array().expect("pins array");
    assert_eq!(pins.len(), 1);
    assert_eq!(pins[0]["account_id"], user_account);

    // Unpin.
    let unpin_uri = format!("/api/v1/vanguard/pins/{}", work_id);
    let (status, body) = user.delete(&unpin_uri).await;
    assert_eq!(status, StatusCode::OK, "unpin work: {body}");

    // Verify pin is gone (soft-deleted).
    let pins_uri = format!("/api/v1/vanguard/pins/{}", work_id);
    let (status, body) = user.get(&pins_uri).await;
    assert_eq!(status, StatusCode::OK, "get pins after unpin: {body}");
    let pins = body["pins"].as_array().expect("pins array");
    assert_eq!(pins.len(), 0);
}

#[tokio::test]
async fn list_vanguards_requires_admin() {
    let harness = Harness::new("list-vanguards").await;
    let mut user = harness.client();
    let (_account, _) = register(&mut user, "user@v.test", "regular-user").await;

    let (status, body) = user.get("/api/v1/vanguards").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "list vanguards: {body}");
}
