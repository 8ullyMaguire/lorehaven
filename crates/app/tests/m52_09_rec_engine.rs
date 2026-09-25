//! Acceptance: the reader's recommendation-engine preference (spec §16.1b, M52-09).
//!
//! The unit tests in `rec_preference.rs` pin the resolution logic. These pin
//! the things that only exist end to end: that the choice persists, that it is
//! stored per pseud rather than per account, that an unavailable engine is a
//! 400 naming what is accepted, and that a recorded choice the operator later
//! disables is reported rather than silently replaced.
//!
//! Driven through the real router with a real session, because every one of
//! those properties is a property of the wiring rather than of a function.
use std::path::Path;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_db::{Database, DatabaseConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

const GOOD_PASSWORD: &str = "a-long-enough-passphrase";
const ENDPOINT: &str = "/api/v1/settings/recommendations";

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m5209-{tag}-{}-{:?}",
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
    // These tests share the process-global rate-limit buckets at 127.0.0.1, so
    // the development-default auth burst is exhausted by neighbours long before
    // this file's own requests are done.
    config.rate_limits.auth = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.write = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.default = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    // Pluggable mode, so the resolver is on the path that produces
    // recommendations rather than on the legacy branch.
    config.discovery.rec_mode = "pluggable".to_owned();
    config
}

struct Harness {
    _dir: std::path::PathBuf,
    config: Config,
    #[allow(dead_code)]
    db: Database,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        Self::with_strategies(tag, &[]).await
    }

    /// A harness whose operator has restricted the enabled strategy set.
    ///
    /// `&[]` means "all enabled", which is the config default.
    async fn with_strategies(tag: &str, enabled: &[&str]) -> Self {
        let dir = scratch_dir(tag);
        let mut config = config_for(&dir);
        config.discovery.rec_enabled_strategies = enabled.iter().map(|s| s.to_string()).collect();
        let db = Database::connect(&config.database)
            .await
            .expect("db connect");
        db.migrate().await.expect("migrations");
        Self {
            _dir: dir,
            config,
            db,
        }
    }

    fn router(&self) -> axum::Router {
        server::build_router(AppState::new(self.config.clone(), self.db.clone()))
    }

    fn client(&self) -> Client {
        Client {
            app: self.router(),
            cookies: Vec::new(),
        }
    }
}

struct Client {
    app: axum::Router,
    cookies: Vec<(String, String)>,
}

impl Client {
    fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn capture_cookies(&mut self, response: &axum::response::Response) {
        for value in response.headers().get_all(header::SET_COOKIE) {
            let Ok(raw) = value.to_str() else { continue };
            let pair = raw.split(';').next().unwrap_or(raw);
            let Some((name, val)) = pair.split_once('=') else {
                continue;
            };
            let (name, val) = (name.to_owned(), val.to_owned());
            if val.is_empty() {
                self.cookies.retain(|(key, _)| key != &name);
            } else {
                self.cookies.retain(|(key, _)| key != &name);
                self.cookies.push((name, val));
            }
        }
    }

    fn cookie_header(&self) -> String {
        self.cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    async fn request(
        &mut self,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        let cookies = self.cookie_header();
        if !cookies.is_empty() {
            builder = builder.header(header::COOKIE, cookies);
        }
        let state_changing = !matches!(method, "GET" | "HEAD" | "OPTIONS");
        if state_changing {
            if let Some(token) = self.cookie("lorehaven_csrf") {
                builder = builder.header("x-csrf-token", token.to_owned());
            }
        }
        let request = match body {
            Some(ref value) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(value).expect("serialise")))
                .expect("request"),
            None => builder.body(Body::empty()).expect("request"),
        };
        let response = self.app.clone().oneshot(request).await.expect("response");
        let status = response.status();
        self.capture_cookies(&response);
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
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

    async fn patch(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("PATCH", uri, Some(body)).await
    }

    /// Register and return a client holding that reader's session.
    async fn registered(tag: &str, handle: &str) -> Client {
        let harness = Harness::new(tag).await;
        // The harness is kept alive by the directory it owns living in the same
        // scope; the router holds its own Arcs, so the client is independent.
        let mut client = harness.client();
        let (status, body) = client
            .post(
                "/api/v1/auth/register",
                json!({
                    "email": format!("{tag}@example.com"),
                    "password": GOOD_PASSWORD,
                    "handle": handle,
                    "display_name": handle,
                    "age_band": "adult",
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "register body: {body}");
        client
    }
}

#[tokio::test]
async fn a_reader_who_has_never_chosen_gets_the_instance_default() {
    let mut reader = Client::registered("fresh", "Fresh").await;

    let (status, body) = reader.get(ENDPOINT).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(
        body["engine"].is_null(),
        "an unset preference reads back as null, not as a named engine: {body}"
    );
    assert_eq!(body["choice"]["state"], "instance_default");
    assert!(
        !body["available"].as_array().expect("available").is_empty(),
        "the reader is shown what they may pick"
    );
}

#[tokio::test]
async fn a_chosen_engine_is_remembered_and_honored() {
    let mut reader = Client::registered("choose", "Chooser").await;

    let (status, body) = reader
        .patch(ENDPOINT, json!({ "engine": "tag_graph" }))
        .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["engine"], "tag_graph");
    assert_eq!(body["choice"]["state"], "honored");

    // A fresh request, so this is persistence rather than an echo of the
    // response the client is still holding.
    let (status, reread) = reader.get(ENDPOINT).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reread["engine"], "tag_graph", "the choice is remembered");
    assert_eq!(reread["choice"]["state"], "honored");
}

#[tokio::test]
async fn clearing_the_choice_returns_to_the_instance_default() {
    let mut reader = Client::registered("clear", "Clearer").await;

    reader.patch(ENDPOINT, json!({ "engine": "bandit" })).await;
    let (status, body) = reader.patch(ENDPOINT, json!({ "engine": "" })).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body["engine"].is_null(), "cleared reads back as null");
    assert_eq!(body["choice"]["state"], "instance_default");

    let (_, reread) = reader.get(ENDPOINT).await;
    assert!(reread["engine"].is_null(), "and it stays cleared");
    assert_eq!(reread["choice"]["state"], "instance_default");
}

#[tokio::test]
async fn an_engine_the_operator_disabled_is_refused_with_the_accepted_values() {
    let harness = Harness::with_strategies("narrow", &["cooccurrence"]).await;
    let mut reader = harness.client();
    let (status, _) = reader
        .post(
            "/api/v1/auth/register",
            json!({
                "email": "narrow@example.com",
                "password": GOOD_PASSWORD,
                "handle": "Narrow",
                "display_name": "Narrow",
                "age_band": "adult",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    // `bandit` exists in this build but the operator has not enabled it. The
    // write must refuse rather than store something that cannot be honored.
    let (status, body) = reader.patch(ENDPOINT, json!({ "engine": "bandit" })).await;
    // 422, not 400: `AppError::Validation` is the house code for a
    // well-formed request whose value is not one this instance accepts, and
    // the spec text was corrected to match rather than the other way round.
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body: {body}");
    let message = body.to_string();
    assert!(
        message.contains("cooccurrence"),
        "the error names what is accepted: {message}"
    );

    // And nothing was stored, so the reader is not left with a broken choice.
    let (_, reread) = reader.get(ENDPOINT).await;
    assert!(reread["engine"].is_null(), "a refused write stores nothing");
}

#[tokio::test]
async fn an_invented_engine_is_refused() {
    let mut reader = Client::registered("invented", "Inventor").await;
    let (status, body) = reader.patch(ENDPOINT, json!({ "engine": "vibes" })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body: {body}");
}

#[tokio::test]
async fn the_preference_is_per_pseud_not_per_account() {
    // A reader wearing two faces may want two different engines; an
    // account-level preference would silently apply one engine to both.
    let harness = Harness::new("perpseud").await;
    let mut client = harness.client();
    let (status, _) = client
        .post(
            "/api/v1/auth/register",
            json!({
                "email": "twice@example.com",
                "password": GOOD_PASSWORD,
                "handle": "First",
                "display_name": "First",
                "age_band": "adult",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    // The first face chooses an engine.
    let (status, first) = client
        .patch(ENDPOINT, json!({ "engine": "tag_graph" }))
        .await;
    assert_eq!(status, StatusCode::OK, "body: {first}");
    let first_pseud = first["pseud_id"].as_str().expect("pseud id").to_owned();

    // A second face is created and activated.
    let (status, second) = client
        .post("/api/v1/pseuds", json!({ "handle": "Second" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "body: {second}");
    let second_id = second["id"].as_str().expect("pseud id").to_owned();
    let (status, _) = client
        .post(&format!("/api/v1/pseuds/{second_id}/activate"), Value::Null)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // The second face has its own, empty, preference.
    let (status, other) = client.get(ENDPOINT).await;
    assert_eq!(status, StatusCode::OK, "body: {other}");
    assert_eq!(other["pseud_id"], second_id.as_str());
    assert!(
        other["engine"].is_null(),
        "the second face must not inherit the first face's engine: {other}"
    );
    assert_eq!(other["choice"]["state"], "instance_default");

    // Back to the first face: its choice is intact.
    let (status, _) = client
        .post(
            &format!("/api/v1/pseuds/{first_pseud}/activate"),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, back) = client.get(ENDPOINT).await;
    assert_eq!(
        back["engine"], "tag_graph",
        "each pseud keeps its own choice"
    );
}

#[tokio::test]
async fn a_recorded_choice_the_operator_later_disables_is_reported_not_hidden() {
    // The operator narrows the enabled set *after* the reader has chosen. The
    // stored value must survive — re-enabling restores it — and the reader must
    // be told their choice is not in effect rather than quietly given another
    // engine's results.
    let dir = scratch_dir("disabled-later");
    let mut config = config_for(&dir);
    let db = Database::connect(&config.database)
        .await
        .expect("db connect");
    db.migrate().await.expect("migrations");

    // First boot: everything enabled, the reader chooses tag_graph.
    let client = {
        let app = server::build_router(AppState::new(config.clone(), db.clone()));
        let mut client = Client {
            app,
            cookies: Vec::new(),
        };
        let (status, _) = client
            .post(
                "/api/v1/auth/register",
                json!({
                    "email": "later@example.com",
                    "password": GOOD_PASSWORD,
                    "handle": "Later",
                    "display_name": "Later",
                    "age_band": "adult",
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, body) = client
            .patch(ENDPOINT, json!({ "engine": "tag_graph" }))
            .await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        client
    };

    // Second boot: the operator disables it.
    config.discovery.rec_enabled_strategies = vec!["cooccurrence".to_owned()];
    let app = server::build_router(AppState::new(config, db));
    let mut reader = Client {
        app,
        cookies: client.cookies,
    };

    let (status, body) = reader.get(ENDPOINT).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(
        body["engine"], "tag_graph",
        "the choice is remembered even while unavailable: {body}"
    );
    assert_eq!(
        body["choice"]["state"], "unavailable",
        "the reader is told, not silently given another engine: {body}"
    );
    assert_eq!(
        body["choice"]["engine"], "tag_graph",
        "the unavailable choice is named back to them"
    );
    assert_eq!(
        body["choice"]["using"],
        json!(["cooccurrence"]),
        "and they are told what is in effect instead"
    );
}
