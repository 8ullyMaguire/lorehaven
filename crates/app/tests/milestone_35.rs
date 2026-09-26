//! M35 — Moderation ladder, slow mode, federation scope, featured posts.
//!
//! Spec §35.5.

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
        "lorehaven-m35-{tag}-{}-{:?}",
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
    #[allow(dead_code)]
    async fn put(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("PUT", uri, Some(body)).await
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

async fn create_topic(client: &mut Client, category: &str, title: &str) -> String {
    let (status, body) = client
        .post(
            &format!("/api/v1/forums/{category}/topics"),
            json!({ "category": category, "title": title }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "create topic: {body}");
    body["id"].as_str().expect("topic id").to_owned()
}

#[tokio::test]
async fn graduated_response_ladder_roundtrip() {
    let harness = Harness::new("ladder").await;
    let mut client = harness.client();
    let (alice_account, _) = register(&mut client, "alice@example.com", "alice").await;

    let target_email = "target@example.com";
    let (target_account, _) = register(&mut client, target_email, "target").await;

    lorehaven_db::moderation::apply_sanction(
        harness.tdb.db(),
        &target_account,
        None,
        lorehaven_domain::moderation::SanctionLevel::PostThrottle,
        "Too many rapid posts",
        &alice_account,
        None,
    )
    .await
    .expect("apply_sanction");

    let sanction =
        lorehaven_db::moderation::check_sanction(harness.tdb.db(), &target_account, None)
            .await
            .expect("check_sanction");
    assert!(sanction.is_some());
    let s = sanction.expect("sanction");
    assert_eq!(s.level, "post_throttle");
}

#[tokio::test]
async fn slow_mode_roundtrip() {
    let harness = Harness::new("slow").await;
    let mut client = harness.client();
    let (_, _pseud) = register(&mut client, "alice@example.com", "alice").await;

    let topic_id = create_topic(&mut client, "general", "Test Topic").await;

    lorehaven_db::moderation::set_slow_mode(harness.tdb.db(), &topic_id, 60)
        .await
        .expect("set_slow_mode");

    let t = lorehaven_db::community::topic_by_id(harness.tdb.db(), &topic_id)
        .await
        .expect("topic")
        .expect("topic exists");
    assert_eq!(t.mode, "plain");
}

#[tokio::test]
async fn federation_scope_roundtrip() {
    let harness = Harness::new("federation").await;
    let mut client = harness.client();
    let (_, _pseud) = register(&mut client, "alice@example.com", "alice").await;

    let topic_id = create_topic(&mut client, "general", "Test Topic").await;

    lorehaven_db::moderation::set_federation_scope(harness.tdb.db(), &topic_id, "local")
        .await
        .expect("set_federation_scope");

    let t = lorehaven_db::community::topic_by_id(harness.tdb.db(), &topic_id)
        .await
        .expect("topic")
        .expect("topic exists");
    assert_eq!(t.mode, "plain");
}

#[tokio::test]
async fn featured_post_roundtrip() {
    let harness = Harness::new("featured").await;
    let mut client = harness.client();
    let (account_id, pseud) = register(&mut client, "alice@example.com", "alice").await;

    let topic_id = create_topic(&mut client, "general", "Test Topic").await;

    let post_id =
        lorehaven_db::community::create_post(harness.tdb.db(), &topic_id, &pseud, "Hello")
            .await
            .expect("create_post");

    lorehaven_db::moderation::feature_post(harness.tdb.db(), &post_id, &account_id)
        .await
        .expect("feature_post");
}

#[tokio::test]
async fn search_miss_roundtrip() {
    let harness = Harness::new("search-miss").await;
    lorehaven_db::moderation::record_search_miss(harness.tdb.db(), "some-query-hash", "some query")
        .await
        .expect("record_search_miss");

    // Read back through whichever pool the fixture is on, so the assertion is
    // the same claim on both backends instead of SQLite-only by construction.
    // `count::bigint` on the PostgreSQL side: the column is INTEGER there and
    // sqlx will not decode an INT4 into an i64. The cast belongs in the query
    // rather than in the Rust type, so the test asserts the same claim on both.
    let sql = harness.tdb.db().sql(
        "SELECT query_text, count FROM forum_search_misses WHERE query_text = ?",
        "SELECT query_text, count::bigint AS count FROM forum_search_misses WHERE query_text = $1",
    );
    let row: Option<(String, i64)> = match harness.tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_as(&sql)
            .bind("some query")
            .fetch_optional(harness.tdb.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("fetch_optional"),
        lorehaven_db::Backend::Postgres => sqlx::query_as(&sql)
            .bind("some query")
            .fetch_optional(harness.tdb.db().postgres_pool().expect("postgres"))
            .await
            .expect("fetch_optional"),
    };

    assert!(row.is_some());
    let (query, count) = row.expect("row");
    assert_eq!(query, "some query");
    assert_eq!(count, 1);
}
