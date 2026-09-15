//! M15 — Economy: credits, fair queues, bounties, billing.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::DatabaseConfig;
use serde_json::Value;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m15-{}-{:?}-{:?}",
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_new_account_has_zero_credits() {
    let harness = Harness::new("credits-zero").await;
    let mut client = harness.client();

    let (status, body) = client.get("/api/v1/credits").await;
    assert_eq!(status, StatusCode::OK, "credits: {body}");
    assert_eq!(body["balances"].as_array().unwrap().len(), 0);
    // Anonymous user sees "anonymous" tier (no session).
    assert_eq!(body["tier"].as_str().unwrap(), "anonymous");

    harness.cleanup().await;
}

#[tokio::test]
async fn a_credit_transaction_can_be_posted_and_is_balanced() {
    let harness = Harness::new("txn-balanced").await;
    let _client = harness.client();

    // Post a balanced transaction: alice earns 100, bob spends 100
    let entries = vec![
        ("alice".to_string(), "earned".to_string(), 100),
        ("bob".to_string(), "earned".to_string(), -100),
    ];
    let txn_id = lorehaven_db::economy::post_transaction(
        harness.tdb.db(),
        lorehaven_domain::economy::TxnType::Earn,
        "test-txn-1",
        "test-ref",
        &entries,
    )
    .await
    .expect("post transaction");

    assert!(!txn_id.is_empty());

    // Verify balances
    let alice_balances = lorehaven_db::economy::balances(harness.tdb.db(), "alice")
        .await
        .expect("alice balances");
    let bob_balances = lorehaven_db::economy::balances(harness.tdb.db(), "bob")
        .await
        .expect("bob balances");

    assert_eq!(alice_balances.len(), 1);
    assert_eq!(alice_balances[0].0, "earned");
    assert_eq!(alice_balances[0].1, 100);

    assert_eq!(bob_balances.len(), 1);
    assert_eq!(bob_balances[0].0, "earned");
    assert_eq!(bob_balances[0].1, -100);

    harness.cleanup().await;
}

#[tokio::test]
async fn idempotency_key_replay_returns_same_txn_id() {
    let harness = Harness::new("idempotency").await;
    let _client = harness.client();

    let entries = vec![
        ("alice".to_string(), "earned".to_string(), 50),
        ("bob".to_string(), "earned".to_string(), -50),
    ];

    let id1 = lorehaven_db::economy::post_transaction(
        harness.tdb.db(),
        lorehaven_domain::economy::TxnType::Earn,
        "idem-key-1",
        "ref-1",
        &entries,
    )
    .await
    .expect("first post");

    let id2 = lorehaven_db::economy::post_transaction(
        harness.tdb.db(),
        lorehaven_domain::economy::TxnType::Earn,
        "idem-key-1",
        "ref-1",
        &entries,
    )
    .await
    .expect("replay post");

    assert_eq!(id1, id2, "idempotency replay must return same txn id");

    harness.cleanup().await;
}

#[tokio::test]
async fn a_hold_can_be_reserved_released_and_captured() {
    let harness = Harness::new("holds").await;
    let _client = harness.client();

    let hold_id =
        lorehaven_db::economy::reserve_hold(harness.tdb.db(), "alice", "job-123", 100, 3600)
            .await
            .expect("reserve hold");

    // Release the hold
    lorehaven_db::economy::release_hold(harness.tdb.db(), &hold_id)
        .await
        .expect("release hold");

    // Capture the hold (actual charge = 80)
    lorehaven_db::economy::capture_hold(harness.tdb.db(), &hold_id, 80)
        .await
        .expect("capture hold");

    harness.cleanup().await;
}

#[tokio::test]
async fn fair_queue_preserves_order_within_class() {
    let harness = Harness::new("fair-queue").await;
    let _client = harness.client();

    let pos1 = lorehaven_db::economy::enqueue_job(harness.tdb.db(), "job-a", "free")
        .await
        .expect("enqueue a");
    let pos2 = lorehaven_db::economy::enqueue_job(harness.tdb.db(), "job-b", "free")
        .await
        .expect("enqueue b");
    let pos3 = lorehaven_db::economy::enqueue_job(harness.tdb.db(), "job-c", "free")
        .await
        .expect("enqueue c");

    assert_eq!(pos1, 1);
    assert_eq!(pos2, 2);
    assert_eq!(pos3, 3);

    // Priority class gets its own position 1
    let pos_pri = lorehaven_db::economy::enqueue_job(harness.tdb.db(), "job-pri", "priority")
        .await
        .expect("enqueue priority");
    assert_eq!(pos_pri, 1);

    // Verify positions are observable
    let q1 = lorehaven_db::economy::queue_position(harness.tdb.db(), "job-a")
        .await
        .expect("queue position a");
    assert_eq!(q1, Some(("free".to_string(), 1)));

    harness.cleanup().await;
}

#[tokio::test]
async fn usage_counters_increment_and_roll_over() {
    let harness = Harness::new("usage").await;
    let _client = harness.client();

    let day = "2026-09-14";

    let (count1, _cap) =
        lorehaven_db::economy::bump_counter(harness.tdb.db(), "alice", "read_chapter", day, 10)
            .await
            .expect("bump 1");
    assert_eq!(count1, 1);

    let (count2, _cap) =
        lorehaven_db::economy::bump_counter(harness.tdb.db(), "alice", "read_chapter", day, 10)
            .await
            .expect("bump 2");
    assert_eq!(count2, 2);

    // Different day resets
    let day2 = "2026-09-15";
    let (count3, _cap) =
        lorehaven_db::economy::bump_counter(harness.tdb.db(), "alice", "read_chapter", day2, 10)
            .await
            .expect("bump 3");
    assert_eq!(count3, 1);

    // Verify usage retrieval
    let usage = lorehaven_db::economy::usage_for(harness.tdb.db(), "alice", day)
        .await
        .expect("usage");
    assert_eq!(usage.len(), 1);
    assert_eq!(usage[0].0, "read_chapter");
    assert_eq!(usage[0].1, 2);

    harness.cleanup().await;
}
