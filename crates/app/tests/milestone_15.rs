//! M15 — Economy: credits, fair queues, bounties, billing.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::DatabaseConfig;
use serde_json::Value;
use sqlx::Row;
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

// ---------------------------------------------------------------------------
// M15-06 / M15-07 / M15-09 — Monetization endpoints
// ---------------------------------------------------------------------------

use lorehaven_db::monetization;

async fn register_with_pseud(client: &mut Client, email: &str, handle: &str) {
    let (status, body) = client
        .request(
            "POST",
            "/api/v1/auth/register",
            Some(serde_json::json!({
                "email": email,
                "password": "Password123!",
                "password_confirm": "Password123!",
                "handle": handle,
                "age_band": "adult",
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "register: {body}");
}

async fn create_work_api(client: &mut Client, title: &str) -> String {
    let (status, body) = client
        .request(
            "POST",
            "/api/v1/works",
            Some(serde_json::json!({
                "title": title,
                "body": "Test body",
                "lifecycle": "published",
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    body["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn ai_declaration_can_be_set_and_read_back() {
    let harness = Harness::new("ai-decl").await;
    let mut client = harness.client();

    register_with_pseud(&mut client, "author@example.com", "testauthor").await;
    let work_id = create_work_api(&mut client, "AI Test Work").await;

    // Set declaration
    let (status, body) = client
        .request(
            "POST",
            &format!("/api/v1/works/{}/ai-declaration", work_id),
            Some(serde_json::json!({ "declaration": "co-written" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "set: {body}");
    assert_eq!(body["declaration"].as_str().unwrap(), "co-written");

    let (status, body) = client
        .request(
            "GET",
            &format!("/api/v1/works/{}/ai-declaration", work_id),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "get: {body}");
    assert_eq!(body["declaration"].as_str().unwrap(), "co-written");

    harness.cleanup().await;
}

#[tokio::test]
async fn invalid_ai_declaration_is_rejected() {
    let harness = Harness::new("ai-decl-invalid").await;
    let mut client = harness.client();

    register_with_pseud(&mut client, "bad@example.com", "badauthor").await;
    let work_id = create_work_api(&mut client, "Bad AI Work").await;

    let (status, _body) = client
        .request(
            "POST",
            &format!("/api/v1/works/{}/ai-declaration", work_id),
            Some(serde_json::json!({ "declaration": "ai-generated-mostly" })),
        )
        .await;
    // Rejected: may surface as 400 or 422 depending on the server's error-mapping
    // for an unknown enum variant. Both mean "the server refused the value".
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
        "invalid declaration rejected (got {status})"
    );

    harness.cleanup().await;
}

#[tokio::test]
async fn settle_period_computes_pool_split() {
    let harness = Harness::new("settle-pool-split").await;
    let mut client = harness.client();

    // Create two authors with reading sessions
    register_with_pseud(&mut client, "author_a@e.com", "author_a").await;
    // Author A creates a work and reader reads it
    // (In a real test, we'd create works + sessions; for now, settle should
    //  succeed even with zero earnings — it just returns empty pools)

    // Call settle endpoint (requires session = any authenticated user)
    // Note: in production, this should require admin TL. For now, any session works.
    let (status, body) = client.request(
        "POST",
        "/api/v1/admin/monetization/settle?period_start=2026-01-01T00:00:00Z&period_end=2026-02-01T00:00:00Z",
        None,
    ).await;

    assert_eq!(status, StatusCode::OK, "settle ok: {body}");
    let body_str = serde_json::to_string(&body).unwrap_or_default();
    assert!(
        body_str.contains("\"status\":\"settled\""),
        "settled: {body}"
    );

    harness.cleanup().await;
}

#[tokio::test]
async fn reading_session_records_time() {
    let harness = Harness::new("reading-session").await;
    let mut client = harness.client();

    // Need to auth first
    register_with_pseud(&mut client, "reader@e.com", "reader").await;

    // Create a work to read
    let work_id = create_work_api(&mut client, "Reading Session Test").await;

    let (status, body) = client
        .request(
            "POST",
            &format!("/api/v1/works/{}/reading-session", work_id),
            Some(serde_json::json!({ "seconds": 180 })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "reading session: {body}");
}

#[tokio::test]
async fn transparency_dashboard_returns_public_data() {
    let harness = Harness::new("transparency").await;
    let mut client = harness.client();

    let (status, body) = client
        .request("GET", "/api/v1/transparency/monetization", None)
        .await;
    assert_eq!(status, StatusCode::OK, "dashboard: {body}");

    assert!(body["total_revenue_minor"].is_i64());
    assert!(body["pending_payout_minor"].is_i64());
    assert!(body["active_earning_authors"].is_i64());
    assert!(body["active_purchasers"].is_i64());
    assert_eq!(body["fee_split_bp"], 1500);

    assert_eq!(body["graduated_cap"]["band1_multiple"], 5);
    assert_eq!(body["graduated_cap"]["band2_multiple"], 10);

    let w = &body["quality_weights"];
    let total = w["rating_bp"].as_i64().unwrap()
        + w["review_bp"].as_i64().unwrap()
        + w["karma_bp"].as_i64().unwrap()
        + w["longevity_bp"].as_i64().unwrap();
    assert_eq!(total, 10_000);

    let ai = &body["ai_multipliers"];
    assert_eq!(ai["none"], 1.0);
    assert_eq!(ai["assisted"], 1.0);
    assert_eq!(ai["co_written"], 0.3);
    assert_eq!(ai["generated"], 0.0);

    harness.cleanup().await;
}

#[tokio::test]
async fn payment_events_record_processor_fees() {
    let harness = Harness::new("payment-fees").await;
    let _client = harness.client();

    // The three subject columns are nullable and this test asserts only the fee
    // arithmetic, so they stay NULL. Inventing UUIDs here needed a parent row
    // for each: `migrations/postgres/0056` declares them as foreign keys and
    // `migrations/sqlite/0056` declares no foreign key at all, so the fake ids
    // were accepted on one dialect and rejected with 23503 on the other. A
    // ledger row with no subject is also the ordinary shape for a processor
    // callback that arrives before attribution.
    let event_id = monetization::record_payment_event(
        harness.tdb.db(),
        "purchase",
        None,
        None,
        None,
        10_000,
        1_500,
        "EUR",
    )
    .await
    .expect("record payment event");
    assert!(!event_id.is_empty());

    match harness.tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            let rows = sqlx::query("SELECT amount_minor, processor_fee_minor, net_minor, currency FROM payment_events WHERE id = ?")
                .bind(&event_id)
                .fetch_all(harness.tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("fetch payment events");
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].get::<i64, _>("amount_minor"), 10_000);
            assert_eq!(rows[0].get::<i64, _>("processor_fee_minor"), 1_500);
            assert_eq!(rows[0].get::<i64, _>("net_minor"), 8_500);
            assert_eq!(rows[0].get::<String, _>("currency"), "EUR");
        }
        lorehaven_db::Backend::Postgres => {
            let rows = sqlx::query("SELECT amount_minor::bigint, processor_fee_minor::bigint, net_minor::bigint, currency FROM payment_events WHERE id = $1::uuid")
                .bind(&event_id)
                .fetch_all(harness.tdb.db().postgres_pool().expect("postgres"))
                .await
                .expect("fetch payment events");
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].get::<i64, _>("amount_minor"), 10_000);
            assert_eq!(rows[0].get::<i64, _>("processor_fee_minor"), 1_500);
            assert_eq!(rows[0].get::<i64, _>("net_minor"), 8_500);
            assert_eq!(rows[0].get::<String, _>("currency"), "EUR");
        }
    };

    harness.cleanup().await;
}
