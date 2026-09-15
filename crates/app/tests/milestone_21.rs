//! Milestone 21 — contract tests for the 2026-09-14 spec-revision skeleton.
//!
//! The skeleton ships migration 0022, domain rule modules, and API contract
//! routes that return 501. These tests pin the CONTRACT so the implementing
//! agent can fill bodies without reshaping them: when a body is implemented,
//! the corresponding 501 assertion is replaced by the behavior test.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::{Database, DatabaseConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness — the same shape as the other milestone tests.
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m21-{}-{:?}-{:?}",
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
    async fn send(&mut self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
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
        self.send("POST", uri, Some(body)).await
    }
    async fn put(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.send("PUT", uri, Some(body)).await
    }
    async fn get(&mut self, uri: &str) -> (StatusCode, Value) {
        self.send("GET", uri, None).await
    }
}

struct Fixture {
    dir: PathBuf,
    db: Database,
}

impl Fixture {
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
            report.applied.iter().any(|id| id.contains("spec_revision")),
            "the 0022 spec-revision migration must be part of the catalogue: {report:?}"
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

// ---------------------------------------------------------------------------
// Migration 0022: the revision tables exist and accept the contract shapes.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn migration_0022_creates_the_revision_tables() {
    let fx = Fixture::new("tables").await;
    {
        let pool = fx.db.sqlite_pool().expect("sqlite pool");
        for table in [
            "work_pricing",
            "work_entitlements",
            "author_earnings_ledger",
            "payouts",
            "monetization_assertions",
            "work_gifts",
            "content_subscriptions",
            "search_alerts",
        ] {
            let n: i64 = sqlx::query_scalar(&format!(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='{table}'"
            ))
            .fetch_one(pool)
            .await
            .expect("sqlite_master query");
            assert_eq!(n, 1, "table {table} must exist after 0022");
        }
        let cols: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('works')")
            .fetch_all(pool)
            .await
            .expect("pragma_table_info");
        assert!(
            cols.iter().any(|c| c == "ai_training"),
            "works.ai_training must exist after 0022"
        );
    }
    fx.cleanup().await;
}

// ---------------------------------------------------------------------------
// Domain rules: the §20.9.3 invariants are pinned as pure tests.
// ---------------------------------------------------------------------------

use lorehaven_domain::monetization::{Eligibility, Model, Rules as MoneyRules};
use lorehaven_domain::subscriptions::Rules as SubRules;

#[test]
fn credits_and_money_are_separate_ledgers() {
    // The platform never converts credits to money (§20.9.3).
    assert!(!MoneyRules::credits_convert_to_money());
}

#[test]
fn imported_works_are_never_monetizable_in_original_mode() {
    assert!(!MoneyRules::imported_work_monetizable(
        Eligibility::Original
    ));
    assert!(!MoneyRules::imported_work_monetizable(
        Eligibility::Disabled
    ));
    assert!(MoneyRules::imported_work_monetizable(
        Eligibility::AnyWithAssertion
    ));
    assert!(!MoneyRules::assertion_required(Eligibility::Disabled));
}

#[test]
fn early_access_is_a_scheduled_unlock_not_a_lock() {
    // At public_at the chapter is free to everyone, permanently.
    assert!(!MoneyRules::early_access_unlocked(1_000, 999));
    assert!(MoneyRules::early_access_unlocked(1_000, 1_000));
}

#[test]
fn paid_works_gain_no_ranking_advantage() {
    assert!(!MoneyRules::ranking_boost_for_paid());
}

#[test]
fn self_dealing_between_pseuds_of_one_account_is_refused() {
    assert!(MoneyRules::self_dealing("acct-a", "acct-a"));
    assert!(!MoneyRules::self_dealing("acct-a", "acct-b"));
}

#[test]
fn the_split_is_eighty_five_fifteen_in_the_authors_favour() {
    let (author, platform) = MoneyRules::split(10_000, 1_500);
    assert_eq!((author, platform), (8_500, 1_500));
}

#[test]
fn the_models_parse_their_wire_names() {
    assert_eq!(Model::parse("tips"), Some(Model::Tips));
    assert_eq!(Model::parse("early_access"), Some(Model::EarlyAccess));
    assert_eq!(Model::parse("purchase"), Some(Model::Purchase));
    assert_eq!(Model::parse("patronage"), Some(Model::Patronage));
    assert_eq!(Model::parse("subscription"), None);
    assert_eq!(Eligibility::parse("original"), Some(Eligibility::Original));
    assert_eq!(
        Eligibility::parse("any-with-assertion"),
        Some(Eligibility::AnyWithAssertion)
    );
}

#[test]
fn subscriber_lists_are_never_visible() {
    // The subscribed author sees a count, never a list (§23.3).
    assert!(!SubRules::subscriber_list_visible());
    assert!(!SubRules::match_activity_disclosed());
}

#[test]
fn alerts_are_bounded_in_frequency() {
    assert!(!SubRules::alert_due("daily", 0, 86_399));
    assert!(SubRules::alert_due("daily", 0, 86_400));
    assert!(SubRules::alert_due("weekly", 0, 604_800));
}

// ---------------------------------------------------------------------------
// API contracts: routes exist, anonymous callers are refused with 401, and a
// signed-in caller reaches the handler (which answers 501 until implemented).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn monetization_routes_refuse_anonymous_callers() {
    let fx = Fixture::new("money-anon").await;
    let mut client = fx.client();

    let (status, _) = client
        .post(
            "/api/v1/works/w1/pricing",
            json!({"model": "purchase", "price_minor": 500, "currency": "EUR"}),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = client.post("/api/v1/works/w1/purchase", json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = client.get("/api/v1/me/entitlements").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    fx.cleanup().await;
}

#[tokio::test]
async fn a_signed_in_caller_reaches_the_monetization_contracts() {
    let fx = Fixture::new("money-auth").await;
    let mut client = fx.client();
    register(&mut client, "m21-author@example.com", "m21author").await;

    // Contract accepted at the boundary; set_pricing is now implemented.
    let (status, body) = client
        .post(
            "/api/v1/works/w1/pricing",
            json!({"model": "purchase", "price_minor": 500, "currency": "EUR", "public_at_offset": null}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");

    let (status, _) = client
        .post(
            "/api/v1/works/w1/tips",
            json!({"amount_minor": 300, "currency": "EUR", "channel": "money"}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // POST /works/{work_id}/gifts and POST /me/payouts are now implemented.
    let (status, _) = client
        .post(
            "/api/v1/works/0189dc5a-4c81-7120-8200-4758243e9e6a/gifts",
            json!({"gift_note": "thanks"}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "gifts to unknown work → 404");

    let (status, _) = client
        .post(
            "/api/v1/me/payouts",
            json!({"amount_minor": 1000, "currency": "EUR", "processor_reference": "ref1"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = client.get("/api/v1/me/gifts").await;
    assert_eq!(status, StatusCode::OK);

    // Admin monetization endpoint gates on trust level.
    let (status, _) = client.get("/api/v1/admin/monetization").await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    fx.cleanup().await;
}

#[tokio::test]
async fn monetization_list_gifts_endpoint_is_implemented() {
    let fx = Fixture::new("money-gifts").await;
    let mut client = fx.client();
    register(&mut client, "m21-gifter@example.com", "m21gifter").await;

    let (status, body) = client.get("/api/v1/me/gifts").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body, json!({ "gifts": [] }));

    fx.cleanup().await;
}

#[tokio::test]
async fn monetization_earnings_endpoint_is_implemented() {
    let fx = Fixture::new("money-earnings").await;
    let mut client = fx.client();
    register(&mut client, "m21-author2@example.com", "m21author2").await;

    // my_earnings is implemented (not a 501 stub): it returns 200 with
    // an earnings array — empty when the author has none.
    let (status, body) = client.get("/api/v1/me/earnings").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body, json!({ "earnings": [] }));

    fx.cleanup().await;
}

#[tokio::test]
async fn monetization_purchase_endpoint_is_implemented() {
    let fx = Fixture::new("money-purchase").await;
    let mut client = fx.client();
    register(&mut client, "m21-buyer@example.com", "m21buyer").await;

    // Purchasing a non-existent work returns NotFound.
    let (status, _) = client
        .post(
            "/api/v1/works/0189dc5a-4c81-7120-8200-4758243e9e6a/purchase",
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    fx.cleanup().await;
}

#[tokio::test]
async fn subscription_and_alert_routes_refuse_anonymous_callers() {
    let fx = Fixture::new("subs-anon").await;
    let mut client = fx.client();

    let (status, _) = client
        .post(
            "/api/v1/subscriptions",
            json!({"subject_type": "work", "subject_id": "w1"}),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = client
        .post(
            "/api/v1/search-alerts",
            json!({"saved_search_id": "s1", "frequency": "daily"}),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = client
        .put(
            "/api/v1/works/w1/ai-training",
            json!({"ai_training": "deny"}),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    fx.cleanup().await;
}

#[tokio::test]
async fn a_signed_in_caller_reaches_the_subscription_contracts() {
    let fx = Fixture::new("subs-auth").await;
    let mut client = fx.client();
    register(&mut client, "m21-reader@example.com", "m21reader").await;

    // Subscription routes are now implemented (no longer 501 stubs).
    let (status, _) = client
        .post(
            "/api/v1/subscriptions",
            json!({"subject_type": "work", "subject_id": "w1"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "subscribe to a work succeeds");

    // create_alert hits the DB; a nonexistent saved_search violates the FK → 500.
    let (status, _) = client
        .post(
            "/api/v1/search-alerts",
            json!({"saved_search_id": "s1", "frequency": "daily"}),
        )
        .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

    let (status, _) = client
        .put(
            "/api/v1/works/0189dc5a-4c81-7120-8200-4758243e9e6a/ai-training",
            json!({"ai_training": "deny"}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    fx.cleanup().await;
}

// ---------------------------------------------------------------------------
// Acceptance tests: entitlement enforcement on the work-read path.
// ---------------------------------------------------------------------------

/// Create a work via the API and return its id.
async fn create_work(client: &mut Client, title: &str) -> String {
    let (status, body) = client
        .post("/api/v1/works", json!({ "title": title }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    body["id"].as_str().unwrap().to_owned()
}

/// Add a chapter with content so the work can be published.
async fn add_chapter(client: &mut Client, work_id: &str, title: &str, text: &str) -> String {
    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/chapters"),
            json!({ "title": title }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "add chapter: {body}");
    let chapter_id = body["id"].as_str().unwrap().to_owned();
    let version = body["version"].as_i64().unwrap_or(1);
    let doc = json!({
        "type": "doc",
        "content": [{"type": "paragraph", "content": [{"type": "text", "text": text}]}]
    });
    let (status, _) = client
        .send(
            "PATCH",
            &format!("/api/v1/chapters/{chapter_id}"),
            Some(json!({ "expected_version": version, "document": doc })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "save chapter text");
    chapter_id
}

/// Publish a work.
async fn publish_work(client: &mut Client, work_id: &str, version: i64) {
    let (status, _) = client
        .post(
            &format!("/api/v1/works/{work_id}/publish"),
            json!({ "expected_version": version }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "publish work {work_id}");
}

#[tokio::test]
async fn a_purchased_work_is_readable_by_the_buyer() {
    let fx = Fixture::new("buy-read").await;
    let mut buyer = fx.client();
    register(&mut buyer, "m21-buyer@example.com", "m21buyer").await;

    let mut author = fx.client();
    register(&mut author, "m21-author3@example.com", "m21author3").await;

    let work_id = create_work(&mut author, "Paid Story").await;
    let _chapter_id = add_chapter(&mut author, &work_id, "Chapter 1", "Once upon a time.").await;
    publish_work(&mut author, &work_id, 1).await;

    // Price the work as purchase-only.
    let (status, _) = author
        .post(
            &format!("/api/v1/works/{work_id}/pricing"),
            json!({ "model": "purchase", "price_minor": 500, "currency": "EUR", "public_at_offset": null }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "set pricing");

    // Buyer purchases.
    let (status, _) = buyer
        .post(&format!("/api/v1/works/{work_id}/purchase"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "purchase");

    // Buyer can read.
    let (status, _) = buyer.get(&format!("/api/v1/works/{work_id}")).await;
    assert_eq!(status, StatusCode::OK, "buyer can read purchased work");

    fx.cleanup().await;
}

#[tokio::test]
async fn a_paid_work_is_paywalled_for_non_buyers() {
    let fx = Fixture::new("paywall").await;
    let mut buyer = fx.client();
    register(&mut buyer, "m21-buyer2@example.com", "m21buyer2").await;

    let mut author = fx.client();
    register(&mut author, "m21-author4@example.com", "m21author4").await;

    let work_id = create_work(&mut author, "Locked Story").await;
    let _chapter_id = add_chapter(&mut author, &work_id, "Chapter 1", "Secret content.").await;
    publish_work(&mut author, &work_id, 1).await;

    // Price the work.
    let (status, _) = author
        .post(
            &format!("/api/v1/works/{work_id}/pricing"),
            json!({ "model": "purchase", "price_minor": 300, "currency": "EUR", "public_at_offset": null }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "set pricing");

    // A different reader without an entitlement is paywalled.
    let mut other = fx.client();
    register(&mut other, "m21-other@example.com", "m21other").await;
    let (status, _) = other.get(&format!("/api/v1/works/{work_id}")).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "non-buyer gets paywalled");

    // But the author can still read their own work.
    let (status, _) = author.get(&format!("/api/v1/works/{work_id}")).await;
    assert_eq!(status, StatusCode::OK, "author can read own work");

    fx.cleanup().await;
}

#[tokio::test]
async fn tip_only_work_is_free_to_read() {
    let fx = Fixture::new("tip-free").await;
    let mut author = fx.client();
    register(&mut author, "m21-author5@example.com", "m21author5").await;

    let work_id = create_work(&mut author, "Tip Jar").await;
    let _chapter_id = add_chapter(&mut author, &work_id, "Chapter 1", "Have a story.").await;
    publish_work(&mut author, &work_id, 1).await;

    // Price as tips-only (free to read).
    let (status, _) = author
        .post(
            &format!("/api/v1/works/{work_id}/pricing"),
            json!({ "model": "tips", "price_minor": 0, "currency": "EUR", "public_at_offset": null }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "set tips pricing");

    // An anonymous reader can still read.
    let mut anon = fx.client();
    let (status, _) = anon.get(&format!("/api/v1/works/{work_id}")).await;
    assert_eq!(status, StatusCode::OK, "tips-only work is free to read");

    // Anonymous can fetch public pricing metadata.
    let (status, body) = anon.get(&format!("/api/v1/works/{work_id}/pricing")).await;
    assert_eq!(status, StatusCode::OK, "public pricing endpoint");
    assert!(body["pricing"].is_array(), "pricing is an array");

    fx.cleanup().await;
}

#[tokio::test]
async fn public_pricing_returns_price_for_purchased_work() {
    let fx = Fixture::new("pricing-endpoint").await;
    let mut author = fx.client();
    register(&mut author, "m21-author6@example.com", "m21author6").await;
    let work_id = create_work(&mut author, "Paid Story").await;
    let _chapter_id = add_chapter(&mut author, &work_id, "Chapter 1", "Content.").await;
    publish_work(&mut author, &work_id, 1).await;

    let (status, _) = author
        .post(
            &format!("/api/v1/works/{work_id}/pricing"),
            json!({ "model": "purchase", "price_minor": 500, "currency": "USD", "public_at_offset": null }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "set purchase pricing");

    // Anonymous can read the price without buying.
    let mut anon = fx.client();
    let (status, body) = anon.get(&format!("/api/v1/works/{work_id}/pricing")).await;
    assert_eq!(status, StatusCode::OK, "anonymous pricing lookup");
    assert_eq!(body["pricing"][0]["model"], "purchase");
    assert_eq!(body["pricing"][0]["price_minor"], 500);
    assert_eq!(body["pricing"][0]["currency"], "USD");

    fx.cleanup().await;
}
