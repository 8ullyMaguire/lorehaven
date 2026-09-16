//! M18 — Public API, bots, feeds, push, federation, AI providers.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::{Backend, DatabaseConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m18-{}-{:?}-{:?}",
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

const PASSWORD: &str = "a-long-enough-passphrase";

async fn register(client: &mut Client, email: &str, handle: &str) {
    let (status, body) = client.post("/api/v1/auth/register", json!({ "email": email, "password": PASSWORD, "handle": handle, "display_name": handle, "age_band": "adult" })).await;
    assert_eq!(status, StatusCode::CREATED, "register {handle}: {body}");
    let _account = body["account"].as_object().expect("account object");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn api_scope_vocabulary_works() {
    let harness = Harness::new("scopes").await;
    let mut client = harness.client();

    // Register a user to get a valid account
    register(&mut client, "token-test@example.com", "TokenUser").await;

    // Get the account ID from the database
    let account_id = {
        let sql = harness.tdb.db().sql(
            "SELECT id FROM accounts WHERE email = ?",
            "SELECT id::text FROM accounts WHERE email = $1",
        );
        match harness.tdb.db().backend() {
            Backend::Sqlite => sqlx::query_scalar::<_, String>(&sql)
                .bind("token-test@example.com")
                .fetch_one(harness.tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("account exists"),
            Backend::Postgres => sqlx::query_scalar::<_, String>(&sql)
                .bind("token-test@example.com")
                .fetch_one(harness.tdb.db().postgres_pool().expect("postgres"))
                .await
                .expect("account exists"),
        }
    };

    let scopes = vec![
        lorehaven_domain::api_scopes::Scope::ContentRead,
        lorehaven_domain::api_scopes::Scope::LibraryRead,
    ];

    // Issue token
    let id = lorehaven_db::external::issue_token(
        harness.tdb.db(),
        &account_id,
        "personal",
        "test-token",
        "hash123",
        &scopes,
    )
    .await
    .expect("issue token");
    assert!(!id.is_empty());

    // Resolve token
    let result = lorehaven_db::external::resolve_token(harness.tdb.db(), "hash123")
        .await
        .expect("resolve token");
    assert!(result.is_some());
    let (resolved_account_id, resolved_scopes) = result.unwrap();
    assert_eq!(resolved_account_id, account_id);
    assert_eq!(resolved_scopes.len(), 2);
    assert!(resolved_scopes.contains(&"content.read".to_string()));

    // Revoke token
    lorehaven_db::external::revoke_token(harness.tdb.db(), &id)
        .await
        .expect("revoke token");
    let result = lorehaven_db::external::resolve_token(harness.tdb.db(), "hash123")
        .await
        .expect("resolve after revoke");
    assert!(result.is_none());

    harness.cleanup().await;
}

#[tokio::test]
async fn feed_building_produces_valid_xml() {
    let harness = Harness::new("feeds").await;
    let _client = harness.client();

    let items = vec![lorehaven_domain::feeds::FeedItem {
        title: "Test Story".to_string(),
        link: "https://example.com/works/1".to_string(),
        description: "A test story".to_string(),
        published_at: "2026-09-14T12:00:00Z".to_string(),
        guid: "https://example.com/works/1".to_string(),
    }];

    let rss =
        lorehaven_domain::feeds::build_rss("My Feed", "https://example.com", "Description", &items);
    assert!(rss.contains(r#"<rss version="2.0">"#));
    assert!(rss.contains("<title>My Feed</title>"));
    assert!(rss.contains("<item>"));

    let atom = lorehaven_domain::feeds::build_atom(
        "My Feed",
        "https://example.com",
        "Description",
        &items,
    );
    assert!(atom.contains(r#"<feed xmlns="http://www.w3.org/2005/Atom">"#));
    assert!(atom.contains("<entry>"));

    harness.cleanup().await;
}

#[tokio::test]
async fn feed_handle_generation() {
    let harness = Harness::new("feed-handles").await;
    let _client = harness.client();

    let id = lorehaven_db::external::upsert_feed_handle(
        harness.tdb.db(),
        "work",
        "my-story-123",
        "work-my-story-123",
    )
    .await
    .expect("upsert feed handle");
    assert!(!id.is_empty());

    harness.cleanup().await;
}

#[tokio::test]
async fn push_subscription_can_be_registered() {
    let harness = Harness::new("push").await;
    let _client = harness.client();

    let id = lorehaven_db::external::register_push_subscription(
        harness.tdb.db(),
        "test-account",
        "https://push.example.com/endpoint",
        "p256dh=abc&auth=def",
        Some("My Phone"),
    )
    .await
    .expect("register push subscription");
    assert!(!id.is_empty());

    harness.cleanup().await;
}

#[tokio::test]
async fn federation_inbound_can_be_recorded() {
    let harness = Harness::new("federation").await;
    let _client = harness.client();

    let id = lorehaven_db::external::record_inbound(
        harness.tdb.db(),
        "peer.example.com",
        "Create",
        "https://peer.example.com/objects/123",
    )
    .await
    .expect("record inbound");
    assert!(!id.is_empty());

    harness.cleanup().await;
}

#[tokio::test]
async fn ai_request_can_be_recorded() {
    let harness = Harness::new("ai").await;
    let _client = harness.client();

    let id = lorehaven_db::external::record_ai_request(
        harness.tdb.db(),
        "work-123",
        "ai-provider",
        "analysis",
        Some("txn-456"),
    )
    .await
    .expect("record AI request");
    assert!(!id.is_empty());

    harness.cleanup().await;
}

#[tokio::test]
async fn bot_can_be_registered() {
    let harness = Harness::new("bots").await;
    let mut client = harness.client();

    // Register a user to get a valid account
    register(&mut client, "bot-owner@example.com", "BotOwner").await;

    // Get the account ID from the database
    let account_id = {
        let sql = harness.tdb.db().sql(
            "SELECT id FROM accounts WHERE email = ?",
            "SELECT id::text FROM accounts WHERE email = $1",
        );
        match harness.tdb.db().backend() {
            Backend::Sqlite => sqlx::query_scalar::<_, String>(&sql)
                .bind("bot-owner@example.com")
                .fetch_one(harness.tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("account exists"),
            Backend::Postgres => sqlx::query_scalar::<_, String>(&sql)
                .bind("bot-owner@example.com")
                .fetch_one(harness.tdb.db().postgres_pool().expect("postgres"))
                .await
                .expect("account exists"),
        }
    };

    // Issue token for bot
    let scopes = vec![lorehaven_domain::api_scopes::Scope::ContentRead];
    let token_id = lorehaven_db::external::issue_token(
        harness.tdb.db(),
        &account_id,
        "bot",
        "my-bot",
        "bot-hash",
        &scopes,
    )
    .await
    .expect("issue bot token");

    // Register bot
    let bot_id = lorehaven_db::external::register_bot(
        harness.tdb.db(),
        &token_id,
        &account_id,
        "owner@example.com",
        "MyBot/1.0",
    )
    .await
    .expect("register bot");

    assert!(!bot_id.is_empty());

    harness.cleanup().await;
}
