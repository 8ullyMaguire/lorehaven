//! Logging in by handle, not only by address.
//!
//! `POST /api/v1/auth/login` treats its `email` field as an *identifier*: a value containing `@` is looked up
//! as a mailbox, anything else as the handle of one of the account's pseuds. These tests pin both halves and
//! the failure modes around them — an unknown handle and a wrong password must both be a plain 401, because a
//! 500 (or a different status per branch) is an existence oracle for handles.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use darkhold_app::config::Config;
use darkhold_app::server;
use darkhold_app::state::AppState;
use darkhold_db::Database;
use darkhold_db::DatabaseConfig;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tower::ServiceExt;

const GOOD_PASSWORD: &str = "a-long-enough-passphrase";
const HANDLE: &str = "Critical_Nexus";
const ADDRESS: &str = "critical-nexus@example.com";

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "darkhold-login-{tag}-{}-{:?}",
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
        "sqlite://{}/darkhold.sqlite?mode=rwc",
        dir.display()
    ));
    config
}

struct Harness {
    _dir: PathBuf,
    config: Config,
    db: Database,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let config = config_for(&dir);
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

    async fn call(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let builder = Request::builder().uri(path).method(method);
        let req = if let Some(b) = body {
            builder
                .header("content-type", "application/json")
                .body(Body::from(b.to_string()))
                .unwrap()
        } else {
            builder.body(Body::empty()).unwrap()
        };
        let resp = self.router().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 10_000_000)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json)
    }

    async fn register(&self) -> (StatusCode, Value) {
        self.call(
            Method::POST,
            "/api/v1/auth/register",
            Some(json!({
                "email": ADDRESS,
                "password": GOOD_PASSWORD,
                "handle": HANDLE,
                "display_name": "Critical Nexus",
                "age_band": "adult",
            })),
        )
        .await
    }

    async fn login(&self, identifier: &str, password: &str) -> (StatusCode, Value) {
        self.call(
            Method::POST,
            "/api/v1/auth/login",
            Some(json!({ "email": identifier, "password": password })),
        )
        .await
    }
}

/// The point of the change: the handle alone signs you in.
#[tokio::test]
async fn a_handle_signs_in_without_an_address() {
    let h = Harness::new("handle").await;
    let (status, body) = h.register().await;
    assert!(status.is_success(), "register: {status} {body}");

    let (status, body) = h.login(HANDLE, GOOD_PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "handle login: {body}");

    // Case-insensitively, matching how handles are displayed and typed.
    let (status, _) = h.login("critical_nexus", GOOD_PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "handles are case-insensitive");

    // And the address still works, because nothing was taken away.
    let (status, _) = h.login(ADDRESS, GOOD_PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "email login must not regress");
}

/// The two ways a handle login can fail are both a plain 401.
///
/// A different status per branch — 404 for an unknown handle, 401 for a wrong password — would let anyone
/// enumerate which names exist here, on a site whose whole point is a pseudonym.
#[tokio::test]
async fn a_failed_handle_login_says_nothing_about_who_exists() {
    let h = Harness::new("handle-fails").await;
    let (status, body) = h.register().await;
    assert!(status.is_success(), "register: {status} {body}");

    let (wrong_password, _) = h.login(HANDLE, "not-the-passphrase-at-all").await;
    let (unknown_handle, _) = h.login("nobody-here-at-all", GOOD_PASSWORD).await;

    assert_eq!(wrong_password, StatusCode::UNAUTHORIZED);
    assert_eq!(
        unknown_handle,
        StatusCode::UNAUTHORIZED,
        "an unknown handle must not be distinguishable from a wrong password"
    );
}
