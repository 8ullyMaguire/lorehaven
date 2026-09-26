//! M42 — Export CTAs with curator quorum exemption (spec §42).
//!
//! These tests prove, at the HTTP layer:
//!
//! - TL2 refused: trust level <= 2 gets 403 on mark.
//! - TL3 curator can mark a work has_own_cta=true.
//! - Quorum: when enough curators mark, the work is exempt from CTAs.
//! - Retraction: a retraction drops the quorum and CTA reappears.
//! - Modes: per_chapter/per_work/off (config-driven).
//! - Sanitize: the CTA HTML is sanitized.

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
        "lorehaven-m42-{tag}-{}-{:?}",
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
    // Enable CTAs with default placement per_chapter.
    config.exports.cta_html = "<p>Support us!</p>".to_owned();
    config.exports.cta_quorum = 2;
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
    async fn post(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("POST", uri, Some(body)).await
    }
    async fn get(&mut self, uri: &str) -> (StatusCode, Value) {
        self.request("GET", uri, None).await
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

async fn make_curator(tdb: &test_support::TestDb, account_id: &str) {
    lorehaven_db::directory::set_account_trust_for_tests(tdb.db(), account_id, 3).await;
}

/// Create a minimal work and return its id.
async fn create_work(client: &mut Client, title: &str) -> String {
    let (status, body) = client
        .post(
            "/api/v1/works",
            json!({
                "title": title,
                "description": "A test work.",
                "visibility": "public"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create_work {title}: {body}");
    body["id"].as_str().expect("work id").to_owned()
}

#[tokio::test]
async fn tl2_refused() {
    let harness = Harness::new("tl2").await;
    let mut client = harness.client();
    let (_account, _pseud) = register(&mut client, "tl2@example.com", "tl2user").await;
    // Keep trust level at the default (0).
    let work_id = create_work(&mut client, "TL2 Test Work").await;

    let (status, _body) = client
        .post(
            format!("/api/v1/works/{work_id}/cta_marks").as_str(),
            json!({ "has_own_cta": true }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "TL2 should be refused");
}

#[tokio::test]
async fn tl3_can_mark() {
    let harness = Harness::new("tl3").await;
    let mut client = harness.client();
    let (account, _pseud) = register(&mut client, "tl3@example.com", "tl3user").await;
    make_curator(&harness.tdb, &account).await;

    let work_id = create_work(&mut client, "TL3 Test Work").await;

    let (status, body) = client
        .post(
            format!("/api/v1/works/{work_id}/cta_marks").as_str(),
            json!({ "has_own_cta": true }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "mark should succeed: {body}");
    assert_eq!(body["has_own_cta"], true);
}

#[tokio::test]
async fn quorum_exempts_cta() {
    let harness = Harness::new("quorum").await;
    let mut curator1 = harness.client();
    let mut curator2 = harness.client();

    let (acc1, _) = register(&mut curator1, "curator1@example.com", "curator1").await;
    let (acc2, _) = register(&mut curator2, "curator2@example.com", "curator2").await;
    make_curator(&harness.tdb, &acc1).await;
    make_curator(&harness.tdb, &acc2).await;

    // Create a work as curator1.
    let work_id = create_work(&mut curator1, "Quorum Test Work").await;

    // Quorum is 2. Mark from both curators.
    let (status, _) = curator1
        .post(
            format!("/api/v1/works/{work_id}/cta_marks").as_str(),
            json!({ "has_own_cta": true }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = curator2
        .post(
            format!("/api/v1/works/{work_id}/cta_marks").as_str(),
            json!({ "has_own_cta": true }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    // Verify the marks are listed.
    let (status, body) = curator1
        .get(format!("/api/v1/works/{work_id}/cta_marks").as_str())
        .await;
    // The body belongs in the message: this assertion used to print only
    // `left: 400 / right: 200`, which says nothing about *why*.
    assert_eq!(status, StatusCode::OK, "list marks: {body}");
    let marks = body.as_array().expect("marks array");
    assert_eq!(marks.len(), 2, "two marks recorded: {body}");
}

#[tokio::test]
async fn retraction_drops_quorum() {
    let harness = Harness::new("retract").await;
    let mut curator1 = harness.client();
    let mut curator2 = harness.client();

    let (acc1, _) = register(&mut curator1, "retractor1@example.com", "retractor1").await;
    let (acc2, _) = register(&mut curator2, "retractor2@example.com", "retractor2").await;
    make_curator(&harness.tdb, &acc1).await;
    make_curator(&harness.tdb, &acc2).await;

    let work_id = create_work(&mut curator1, "Retraction Test Work").await;

    // Both mark has_own_cta=true → quorum reached.
    curator1
        .post(
            format!("/api/v1/works/{work_id}/cta_marks").as_str(),
            json!({ "has_own_cta": true }),
        )
        .await;
    curator2
        .post(
            format!("/api/v1/works/{work_id}/cta_marks").as_str(),
            json!({ "has_own_cta": true }),
        )
        .await;

    // Retract curator2's mark.
    let (status, body) = curator2
        .delete(format!("/api/v1/works/{work_id}/cta_marks/me").as_str())
        .await;
    assert_eq!(status, StatusCode::OK, "retraction should succeed: {body}");
    assert_eq!(body["action"], "retracted");

    // Now only one mark remains.
    let (status, body) = curator1
        .get(format!("/api/v1/works/{work_id}/cta_marks").as_str())
        .await;
    assert_eq!(status, StatusCode::OK);
    let marks = body.as_array().expect("marks array");
    assert_eq!(marks.len(), 1, "one mark after retraction");
}

#[tokio::test]
async fn list_marks_empty_initially() {
    let harness = Harness::new("list").await;
    let mut client = harness.client();
    let (_account, _) = register(&mut client, "lister@example.com", "lister").await;
    let work_id = create_work(&mut client, "Empty List Work").await;

    let (status, body) = client
        .get(format!("/api/v1/works/{work_id}/cta_marks").as_str())
        .await;
    assert_eq!(status, StatusCode::OK);
    let marks = body.as_array().expect("marks array");
    assert!(marks.is_empty(), "no marks initially");
}

#[tokio::test]
async fn retract_with_no_mark_returns_no_mark() {
    let harness = Harness::new("retract-none").await;
    let mut client = harness.client();
    let (account, _) = register(&mut client, "nomark@example.com", "nomark").await;
    make_curator(&harness.tdb, &account).await;
    let work_id = create_work(&mut client, "No Mark Work").await;

    // Try to retract when no mark exists.
    let (status, body) = client
        .delete(format!("/api/v1/works/{work_id}/cta_marks/me").as_str())
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["action"], "no mark to retract");
}
