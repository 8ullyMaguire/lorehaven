//! M65-04: per-user module overrides, through the real router.
//!
//! The store's properties are in `crates/db/tests/module_user_state.rs`. These are
//! the ones that belong to the route and the gate:
//!
//! - **an anonymous caller cannot change a surface** — `401`, because a preference
//!   belongs to an account and there is no such thing as "the anonymous reader's
//!   settings";
//! - **a reader who switches a surface off stops reaching it, and only they do** —
//!   `404 MODULE_UNAVAILABLE` for their session, `401` for the neighbour whose
//!   session never wrote a row. This is the assertion that fails if the gate reads
//!   the row into a process-wide flag;
//! - **switching it back on deletes the row** rather than storing an "on" — the
//!   instance's decision applies again, which may still be off;
//! - **a surface whose switch belongs to the operator or the build answers `403`
//!   naming who decides**, never `404` (which would say the feature is gone);
//! - **a reader cannot widen**: `enabled: true` on a surface the instance switched
//!   off clears their row and leaves the surface off, and the response says so;
//! - **the catalogue reports the caller's state beside the instance's**, so a page
//!   can hide what *this* reader switched off without inferring it.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use darkhold_app::config::Config;
use darkhold_app::server;
use darkhold_app::state::AppState;
use serde_json::{json, Value};
use test_support::TestDb;
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery staple 7";

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "darkhold-m65-04-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// The instance this test builds: the six opt-in surfaces are off by default since
/// §59.10, so `quiz` is switched on here — which is also §59.11's `enabled` list
/// doing its job, and without it every `404` below would be about the instance
/// rather than about the reader.
fn config_for(dir: &Path, enabled: &[&str], disabled: &[&str]) -> Config {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    config.modules.enabled = enabled.iter().map(|id| (*id).to_owned()).collect();
    config.modules.disabled = disabled.iter().map(|id| (*id).to_owned()).collect();
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

    fn capture(&mut self, response: &axum::response::Response) {
        for value in response.headers().get_all(header::SET_COOKIE) {
            let Ok(text) = value.to_str() else { continue };
            let Some((pair, _)) = text.split_once(';') else {
                continue;
            };
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            let name = name.trim().to_owned();
            let value = value.trim().to_owned();
            self.cookies.retain(|(k, _)| k != &name);
            if !value.is_empty() {
                self.cookies.push((name, value));
            }
        }
    }

    async fn request(&mut self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        if !self.cookies.is_empty() {
            let jar = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            builder = builder.header(header::COOKIE, jar);
            if let Some((_, token)) = self.cookies.iter().find(|(k, _)| k == "darkhold_csrf") {
                builder = builder.header("x-csrf-token", token);
            }
        }
        let body = match body {
            Some(value) => Body::from(value.to_string()),
            None => Body::empty(),
        };
        let response = self
            .app
            .clone()
            .oneshot(builder.body(body).expect("request"))
            .await
            .expect("response");
        let status = response.status();
        self.capture(&response);
        let bytes = axum::body::to_bytes(response.into_body(), 512 * 1024)
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

    async fn patch(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("PATCH", uri, Some(body)).await
    }

    /// Register, so the client carries a session for the account it just made.
    /// Returns the account id, which a test needs when it writes a row directly.
    async fn register(&mut self, handle: &str) -> String {
        let email = format!("{handle}-{}@example.invalid", uuid::Uuid::new_v4().simple());
        let (status, body) = self
            .request(
                "POST",
                "/api/v1/auth/register",
                Some(json!({
                    "email": email,
                    "password": PASSWORD,
                    "handle": handle,
                    "display_name": handle,
                    "age_band": "adult",
                })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "register {handle}: {body}");
        let (status, me) = self.get("/api/v1/auth/me").await;
        assert_eq!(status, StatusCode::OK, "{me}");
        me["account"]["id"].as_str().expect("account id").to_owned()
    }
}

struct Harness {
    _dir: PathBuf,
    tdb: TestDb,
    config: Config,
}

impl Harness {
    async fn new(tag: &str, enabled: &[&str], disabled: &[&str]) -> Self {
        let dir = scratch_dir(tag);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        let config = config_for(&dir, enabled, disabled);
        Self {
            _dir: dir,
            tdb,
            config,
        }
    }

    fn client(&self) -> Client {
        Client::new(server::build_router(AppState::new(
            self.config.clone(),
            self.tdb.db().clone(),
        )))
    }
}

#[tokio::test]
async fn an_anonymous_caller_cannot_change_a_surface() {
    let harness = Harness::new("anon", &["quiz"], &[]).await;
    let mut client = harness.client();

    let (status, body) = client
        .patch("/api/v1/modules/quiz", json!({ "enabled": false }))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body: {body}");
    assert_eq!(body["error"]["code"], "AUTH_REQUIRED", "body: {body}");
}

#[tokio::test]
async fn a_reader_who_switched_it_off_gets_404_and_their_neighbour_does_not() {
    let harness = Harness::new("narrow", &["quiz"], &[]).await;

    let mut alice = harness.client();
    alice.register("alice").await;
    let mut bob = harness.client();
    bob.register("bob").await;

    // Before: the surface answers for both, as a `401` (it needs a session, which
    // they have — but the quiz route wants a *pseud*, and either way it is not 404).
    let (status, body) = alice.get("/api/v1/quiz/works").await;
    assert_ne!(status, StatusCode::NOT_FOUND, "quiz must exist: {body}");

    let (status, body) = alice
        .patch("/api/v1/modules/quiz", json!({ "enabled": false }))
        .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["id"], "quiz");
    assert_eq!(body["instance_enabled"], true, "the instance did not change");
    assert_eq!(body["enabled_for_caller"], false);
    assert_eq!(body["overridden"], true);

    // For alice the surface is gone, and the refusal names it.
    let (status, body) = alice.get("/api/v1/quiz/works").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert_eq!(body["error"]["code"], "MODULE_UNAVAILABLE", "body: {body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("quiz"),
        "the refusal names the module: {body}"
    );

    // For bob it is exactly where it was. A per-account table that answers for
    // somebody else is a process-wide flag with extra steps.
    let (status, body) = bob.get("/api/v1/quiz/works").await;
    assert_ne!(status, StatusCode::NOT_FOUND, "bob: {body}");
}

#[tokio::test]
async fn switching_it_back_on_removes_the_row() {
    let harness = Harness::new("clear", &["quiz"], &[]).await;
    let mut alice = harness.client();
    alice.register("alice").await;

    alice
        .patch("/api/v1/modules/quiz", json!({ "enabled": false }))
        .await;
    let (status, body) = alice.get("/api/v1/modules").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["overrides"]
            .as_array()
            .expect("overrides")
            .iter()
            .any(|id| id == "quiz"),
        "the catalogue lists the change: {body}"
    );

    let (status, body) = alice
        .patch("/api/v1/modules/quiz", json!({ "enabled": true }))
        .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["overridden"], false, "the row is gone: {body}");
    assert_eq!(body["enabled_for_caller"], true);

    // The row is deleted rather than stored as "on", so the store's count is zero.
    assert_eq!(
        darkhold_db::module_state::count(harness.tdb.db())
            .await
            .expect("count"),
        0
    );
    let (status, body) = alice.get("/api/v1/modules").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["overrides"].as_array().expect("overrides").is_empty(),
        "{body}"
    );
}

#[tokio::test]
async fn a_switch_that_is_not_the_readers_answers_403_and_says_who_decides() {
    let harness = Harness::new("forbidden", &["quiz"], &[]).await;
    let mut alice = harness.client();
    alice.register("alice").await;

    // An operator surface: the instance decides, and the reader is told so.
    let (status, body) = alice
        .patch("/api/v1/modules/catalogue", json!({ "enabled": false }))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
    assert_eq!(
        body["error"]["code"], "MODULE_NOT_USER_TOGGLEABLE",
        "body: {body}"
    );
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("operator"),
        "the refusal names the operator as the decider: {body}"
    );

    // A safety surface: `fixe`d, and the message says the instance.
    let (status, body) = alice
        .patch("/api/v1/modules/erasure", json!({ "enabled": false }))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("instance"),
        "body: {body}"
    );

    // And nothing was written for the refused ids.
    assert_eq!(
        darkhold_db::module_state::count(harness.tdb.db())
            .await
            .expect("count"),
        0
    );
}

/// A row that exists for a surface whose toggle is not the reader's must not silence
/// it. The route refuses to create one, so this writes the row the way a migration,
/// an older build or a mistaken admin would — and the gate must still answer because
/// the *toggle*, not the presence of a row, decides who may narrow.
#[tokio::test]
async fn a_row_for_an_operator_surface_does_not_silence_it() {
    let harness = Harness::new("stray-row", &["quiz"], &[]).await;
    let mut alice = harness.client();
    let account = alice.register("alice").await;

    darkhold_db::module_state::set_off(
        harness.tdb.db(),
        &account,
        "catalogue",
        "2026-11-02T00:00:00Z",
    )
    .await
    .expect("write a row the route would refuse");

    let (status, body) = alice.get("/api/v1/catalogue?limit=1").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a stray row must not silence a surface whose toggle is the operator's: {body}"
    );

    // The catalogue says the row exists and the surface is still reachable, which is
    // the honest pair: the row is there, and it does not decide anything.
    let (_, view) = alice.get("/api/v1/modules").await;
    let row = view["modules"]
        .as_array()
        .expect("modules")
        .iter()
        .find(|row| row["id"] == "catalogue")
        .cloned()
        .expect("catalogue row");
    assert_eq!(row["enabled_for_caller"], true);
}

#[tokio::test]
async fn a_reader_cannot_widen_a_surface_the_instance_switched_off() {
    // `arena` is off at the instance: `[modules] disabled`, not a default.
    let harness = Harness::new("widen", &["quiz"], &["arena"]).await;
    let mut alice = harness.client();
    alice.register("alice").await;

    let (status, body) = alice
        .patch("/api/v1/modules/arena", json!({ "enabled": true }))
        .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(
        body["instance_enabled"], false,
        "the instance's decision is unchanged: {body}"
    );
    assert_eq!(
        body["enabled_for_caller"], false,
        "asking for it does not grant it: {body}"
    );

    let (status, body) = alice.get("/api/v1/arena/next").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert_eq!(body["error"]["code"], "MODULE_UNAVAILABLE", "body: {body}");
}

#[tokio::test]
async fn the_catalogue_reports_the_callers_state_beside_the_instances() {
    let harness = Harness::new("catalogue", &["quiz"], &[]).await;
    let mut alice = harness.client();
    alice.register("alice").await;
    let mut bob = harness.client();
    bob.register("bob").await;

    alice
        .patch("/api/v1/modules/quiz", json!({ "enabled": false }))
        .await;

    let quiz = |body: &Value| -> Value {
        body["modules"]
            .as_array()
            .expect("modules")
            .iter()
            .find(|row| row["id"] == "quiz")
            .cloned()
            .expect("quiz row")
    };

    let (_, alice_view) = alice.get("/api/v1/modules").await;
    let row = quiz(&alice_view);
    assert_eq!(row["enabled"], true, "the instance still runs it");
    assert_eq!(row["enabled_for_caller"], false, "alice narrowed it");

    let (_, bob_view) = bob.get("/api/v1/modules").await;
    let row = quiz(&bob_view);
    assert_eq!(row["enabled"], true);
    assert_eq!(row["enabled_for_caller"], true, "bob changed nothing");

    // Anonymous: `enabled_for_caller` is the instance's answer, because there is
    // nobody to hold a preference.
    let mut anon = harness.client();
    let (status, view) = anon.get("/api/v1/modules").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(quiz(&view)["enabled_for_caller"], true);
    assert!(view["overrides"].as_array().expect("overrides").is_empty());
}
