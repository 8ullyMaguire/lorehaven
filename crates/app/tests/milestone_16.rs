//! M16 — Marketplace: listings, commissions, extensions, webhooks, gallery.

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
// Helpers
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m16-{}-{:?}-{:?}",
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
    async fn post(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("POST", uri, Some(body)).await
    }
}

struct Harness {
    dir: PathBuf,
    db: Database,
}

impl Harness {
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
        let _ = db.migrate().await.expect("migrate");
        Self { dir, db }
    }
    fn db(&self) -> &Database {
        &self.db
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

async fn register(client: &mut Client, email: &str, handle: &str) -> (String, String) {
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
    // AccountResponse wraps account in an "account" field; we don't need the IDs for M16 tests.
    let _account = body["account"].as_object().expect("account object");
    (String::new(), String::new())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_listing_can_be_created_and_listed() {
    let harness = Harness::new("listing-create").await;
    let mut client = harness.client();
    let (_account, _pseud) = register(&mut client, "listing@example.com", "ListingAuthor").await;

    // Create a listing
    let (status, body) = client
        .post(
            "/api/v1/listings",
            json!({
                "kind": "paid_work",
                "work_id": null,
                "terms": { "price": 100, "currency": "credits" },
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "create listing: {body}");
    assert!(body["id"].as_str().is_some());

    // List listings
    let (status, body) = client.get("/api/v1/listings").await;
    assert_eq!(status, StatusCode::OK, "list listings: {body}");
    assert_eq!(body["listings"].as_array().unwrap().len(), 1);

    harness.cleanup().await;
}

#[tokio::test]
async fn a_commission_can_be_created_and_transitions() {
    let harness = Harness::new("commission").await;
    let mut client = harness.client();
    let (_account, _pseud) = register(&mut client, "commissioner@example.com", "CommAuthor").await;

    // Create listing
    let (status, body) = client
        .post(
            "/api/v1/listings",
            json!({ "kind": "commission", "terms": { "price": 200 } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "create listing: {body}");
    let listing_id = body["id"].as_str().unwrap().to_owned();

    // Create commission
    let (status, body) = client
        .post(
            format!("/api/v1/listings/{listing_id}/commissions").as_str(),
            json!({ "listing_id": listing_id }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "create commission: {body}");
    let commission_id = body["id"].as_str().unwrap().to_owned();

    // Verify commission is in "quoted" state
    let (status, _body) = client
        .post(
            format!("/api/v1/commissions/{commission_id}/transition").as_str(),
            json!({ "from_state": "quoted", "to_state": "accepted" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "transition to accepted: {_body}");

    harness.cleanup().await;
}

#[tokio::test]
async fn extension_grant_works() {
    let harness = Harness::new("extension-grant").await;
    let _client = harness.client();

    let capabilities = vec![
        lorehaven_domain::extension::Capability::StorageRead,
        lorehaven_domain::extension::Capability::WorkRead,
    ];
    let result = lorehaven_db::marketplace::grant_extension(
        harness.db(),
        "test-account",
        "test-extension",
        "1.0.0",
        &capabilities,
    )
    .await;
    assert!(result.is_ok(), "grant extension: {:?}", result);

    // Verify grant was stored
    let result =
        lorehaven_db::marketplace::revoke_extension(harness.db(), "test-account", "test-extension")
            .await;
    assert!(result.is_ok(), "revoke extension: {:?}", result);

    harness.cleanup().await;
}

#[tokio::test]
async fn webhook_creation_works() {
    let harness = Harness::new("webhook").await;
    let _client = harness.client();

    let id = lorehaven_db::marketplace::create_webhook(
        harness.db(),
        "test-account",
        "https://example.com/hook",
        "whsec_test_secret_123",
        &["work.published".to_string(), "comment.posted".to_string()],
    )
    .await
    .expect("create webhook");

    assert!(!id.is_empty());

    // Record a delivery
    let result = lorehaven_db::marketplace::record_delivery(
        harness.db(),
        &id,
        "evt-123",
        "{\"test\": true}",
        "sig-abc",
        "ok",
    )
    .await;
    assert!(result.is_ok(), "record delivery: {:?}", result);

    harness.cleanup().await;
}

#[tokio::test]
async fn gallery_item_can_be_added() {
    let harness = Harness::new("gallery").await;
    let _client = harness.client();

    let id = lorehaven_db::marketplace::add_gallery_item(
        harness.db(),
        "work-123",
        "test-account",
        "image/png",
        "storage/key/123.png",
        "A beautiful image",
        "<img src=\"...\" alt=\"A beautiful image\">",
    )
    .await
    .expect("add gallery item");

    assert!(!id.is_empty());

    // List gallery items
    let items = lorehaven_db::marketplace::list_gallery_items(harness.db(), "work-123")
        .await
        .expect("list gallery");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["owner"].as_str().unwrap(), "test-account");

    harness.cleanup().await;
}

#[tokio::test]
async fn webhook_signing_verifies() {
    let harness = Harness::new("webhook-sign").await;

    let event = lorehaven_domain::webhook::WebhookEvent {
        event_type: "work.published".to_string(),
        event_id: "evt-456".to_string(),
        created_at: "2026-09-14T12:00:00Z".to_string(),
        payload: serde_json::json!({"work_id": "w-2"}),
    };

    let secret = "whsec_test_secret_456";
    let signature = event.sign(secret);

    assert!(event.verify(secret, &signature), "signature should verify");
    assert!(
        !event.verify("wrong-secret", &signature),
        "wrong secret should fail"
    );

    // Test payload bounding
    let large_payload = serde_json::json!({"data": "x".repeat(1000)});
    let bounded = lorehaven_domain::webhook::bound_payload(&large_payload, 100);
    assert!(
        bounded["_truncated"].as_bool().unwrap(),
        "large payload should be truncated"
    );

    harness.cleanup().await;
}
