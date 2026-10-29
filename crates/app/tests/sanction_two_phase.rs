//! Two-phase sanctions — capability ledger 3.4.
//!
//! The winner is gravity, where filing a permanent ban and executing it are **separate audited
//! calls** (`/discipline/permanent-ban` then `/discipline/permanent-ban/execute`). Darkhold applied
//! a sanction in one request, so an irreversible decision was one click and one audit row.
//!
//! The tests below are ordered the way the risk is: a filed case must bite NOBODY (that is the
//! failure mode that makes a two-phase worse than none — it looks like a safeguard and behaves like
//! a sanction), executing it must make it bite, and executing it twice must be refused rather than
//! recorded.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use darkhold_app::config::Config;
use darkhold_app::server::{self, set_trust_proxy};
use darkhold_app::state::AppState;
use darkhold_db::DatabaseConfig;
use serde_json::{json, Value};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "darkhold-2phase-{}-{:?}-{:?}",
        tag,
        std::process::id(),
        std::thread::current().id(),
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn config_for(dir: &Path) -> Config {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    config.database = DatabaseConfig::new(format!(
        "sqlite://{}/darkhold.sqlite?mode=rwc",
        dir.display()
    ));
    config.rate_limits.auth = darkhold_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.write = darkhold_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.search = darkhold_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.default = darkhold_app::limiter::Quota {
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
    async fn request(&mut self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
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
            if let Some(token) = self.cookie("darkhold_csrf").map(str::to_owned) {
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

async fn register(client: &mut Client, email: &str, handle: &str) -> String {
    let (status, body) = client
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
    let (status, me) = client.get("/api/v1/auth/me").await;
    assert_eq!(status, StatusCode::OK, "{me}");
    me["account"]["id"].as_str().expect("account id").to_owned()
}

/// A steward at TL4 — the elastic level that may issue scoped bans on an instance with no senior
/// reviewer (spec §14.4).
async fn steward(harness: &Harness, email: &str, handle: &str) -> (Client, String) {
    let mut c = harness.client();
    let account = register(&mut c, email, handle).await;
    darkhold_db::governance::set_trust(harness.tdb.db(), &account, 4, "two-phase-test")
        .await
        .expect("promote steward");
    (c, account)
}

async fn active_count(harness: &Harness, account: &str) -> usize {
    darkhold_db::governance::active_sanctions(harness.tdb.db(), account)
        .await
        .expect("active sanctions")
        .len()
}

/// PHASE ONE IS INERT. A filed case exists, is pending, and changes nothing about the account —
/// which is the property that makes the second call meaningful rather than ceremonial.
#[tokio::test]
async fn a_filed_permanent_sanction_is_pending_and_bites_nobody() {
    let harness = Harness::new("file-inert").await;
    let (mut gov, _) = steward(&harness, "gov@example.com", "Gov").await;
    let mut target = harness.client();
    let target_account = register(&mut target, "target@example.com", "Target").await;

    let (status, body) = gov
        .post(
            "/api/v1/sanctions/file",
            json!({
                "account": target_account,
                "kind": "permanent",
                "reason_ref": "harassment, case 12",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "pending", "{body}");
    let id = body["id"].as_str().expect("id").to_owned();
    assert!(
        body["next"].as_str().unwrap_or("").contains("execute"),
        "the response must say what the second call is: {body}"
    );

    // The sanction exists…
    assert_eq!(
        darkhold_db::governance::sanction_state(harness.tdb.db(), &id)
            .await
            .expect("state"),
        Some("pending".to_string())
    );
    // …and bites nobody.
    assert_eq!(active_count(&harness, &target_account).await, 0);
    let (status, mine) = target.get("/api/v1/me/sanctions").await;
    assert_eq!(status, StatusCode::OK, "{mine}");
    assert_eq!(
        mine["items"].as_array().map(|a| a.len()),
        Some(0),
        "a pending case must not appear as an active sanction: {mine}"
    );

    harness.cleanup().await;
}

/// PHASE TWO APPLIES IT, and the target can see it. Without this the first test would pass on a
/// two-phase that never applies anything at all.
#[tokio::test]
async fn executing_the_file_makes_it_bite() {
    let harness = Harness::new("execute-applies").await;
    let (mut gov, _) = steward(&harness, "gov2@example.com", "Gov2").await;
    let mut target = harness.client();
    let target_account = register(&mut target, "target2@example.com", "Target2").await;

    let (_, body) = gov
        .post(
            "/api/v1/sanctions/file",
            json!({
                "account": target_account,
                "kind": "permanent",
                "reason_ref": "evading a prior suspension",
            }),
        )
        .await;
    let id = body["id"].as_str().expect("id").to_owned();

    let (status, body) = gov.post(&format!("/api/v1/sanctions/{id}/execute"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "active", "{body}");

    assert_eq!(active_count(&harness, &target_account).await, 1);
    let (_, mine) = target.get("/api/v1/me/sanctions").await;
    let items = mine["items"].as_array().cloned().unwrap_or_default();
    assert_eq!(items.len(), 1, "{mine}");
    assert_eq!(items[0]["id"], id);

    harness.cleanup().await;
}

/// EXECUTING TWICE IS REFUSED BY NAME. A second execution would write an audit row claiming an
/// action that changed nothing, and "it is done" is a claim an operator will act on.
#[tokio::test]
async fn executing_the_same_case_twice_is_refused() {
    let harness = Harness::new("execute-twice").await;
    let (mut gov, _) = steward(&harness, "gov3@example.com", "Gov3").await;
    let mut target = harness.client();
    let target_account = register(&mut target, "target3@example.com", "Target3").await;

    let (_, body) = gov
        .post(
            "/api/v1/sanctions/file",
            json!({ "account": target_account, "kind": "permanent", "reason_ref": "case 3" }),
        )
        .await;
    let id = body["id"].as_str().expect("id").to_owned();

    let (first, _) = gov.post(&format!("/api/v1/sanctions/{id}/execute"), json!({})).await;
    assert_eq!(first, StatusCode::OK);

    let (second, body) = gov.post(&format!("/api/v1/sanctions/{id}/execute"), json!({})).await;
    assert_eq!(second, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let message = body.to_string();
    assert!(
        message.contains("not pending"),
        "the refusal must say the case is past the point of execution: {message}"
    );
    assert_eq!(active_count(&harness, &target_account).await, 1, "and must not double-apply");

    harness.cleanup().await;
}

/// THE ONE-SHOT ROUTE REFUSES THE IRREVERSIBLE CASE, and says where to go instead. This is the half
/// that makes the two-phase real: if the one-call route still accepts a permanent sanction, the
/// safeguard is optional and will be skipped.
#[tokio::test]
async fn a_permanent_sanction_cannot_be_issued_in_one_call() {
    let harness = Harness::new("one-shot-refused").await;
    let (mut gov, _) = steward(&harness, "gov4@example.com", "Gov4").await;
    let mut target = harness.client();
    let target_account = register(&mut target, "target4@example.com", "Target4").await;

    let (status, body) = gov
        .post(
            "/api/v1/sanctions",
            json!({ "account": target_account, "kind": "permanent", "reason_ref": "no expiry" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let message = body.to_string();
    assert!(
        message.contains("/sanctions/file"),
        "the refusal must name the two-phase route: {message}"
    );
    assert_eq!(active_count(&harness, &target_account).await, 0, "and must apply nothing");

    harness.cleanup().await;
}

/// THE REVERSIBLE CASE IS UNCHANGED: a time-limited sanction still applies in one call. A port that
/// made every sanction two-phase would be a regression dressed as a safeguard.
#[tokio::test]
async fn a_time_limited_sanction_still_applies_in_one_call() {
    let harness = Harness::new("timed-one-call").await;
    let (mut gov, _) = steward(&harness, "gov5@example.com", "Gov5").await;
    let mut target = harness.client();
    let target_account = register(&mut target, "target5@example.com", "Target5").await;

    let (status, body) = gov
        .post(
            "/api/v1/sanctions",
            json!({
                "account": target_account,
                "kind": "suspension",
                "reason_ref": "cooling off",
                "duration_days": 7,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(active_count(&harness, &target_account).await, 1);

    harness.cleanup().await;
}
