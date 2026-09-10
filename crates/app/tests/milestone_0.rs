//! Milestone 0 acceptance tests.
//!
//! Spec §5 acceptance:
//!
//! * a clean checkout builds (covered by `cargo build`);
//! * the application starts with SQLite;
//! * the application starts with PostgreSQL;
//! * a frontend page loads from the Rust executable;
//! * `/health/live` checks process liveness;
//! * `/health/ready` checks essential dependencies;
//! * production startup rejects unsafe development configuration.
//!
//! These tests exercise the real router over real HTTP request/response objects
//! against a real SQLite file. Nothing here is mocked: the point of the suite is
//! to prove the vertical slice works end to end.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use lorehaven_app::cli::SeedArgs;
use lorehaven_app::config::Config;
use lorehaven_app::state::AppState;
use lorehaven_app::{seed, server};
use lorehaven_db::{identity, migrate, Database, DatabaseConfig};
use tower::ServiceExt;

/// A scratch directory unique to one test.
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-it-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

async fn scratch_database(dir: &Path) -> Database {
    let url = format!("sqlite://{}/lorehaven.sqlite?mode=rwc", dir.display());
    Database::connect(&DatabaseConfig::new(url))
        .await
        .expect("connect to scratch database")
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

fn seed_args() -> SeedArgs {
    SeedArgs {
        development: true,
        reset: false,
        email: "dev@lorehaven.local".to_owned(),
        password: "lorehaven-dev".to_owned(),
    }
}

#[tokio::test]
async fn migrations_apply_once_and_are_idempotent() {
    let dir = scratch_dir("migrate");
    let db = scratch_database(&dir).await;

    let first = db.migrate().await.expect("first migration run");
    assert!(
        first.changed_schema(),
        "a fresh database must apply the catalogue"
    );
    assert!(
        first.applied.iter().any(|id| id.contains("identity")),
        "the identity migration must be part of the catalogue: {:?}",
        first.applied
    );

    let second = db.migrate().await.expect("second migration run");
    assert!(
        second.applied.is_empty(),
        "re-running migrations must be a no-op, applied {:?}",
        second.applied
    );
    assert_eq!(
        second.already_applied.len(),
        migrate::catalogue(db.backend()).len()
    );

    // And the schema is really there.
    assert_eq!(identity::count(&db, "accounts").await.expect("count"), 0);

    db.close().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn the_identity_schema_enforces_case_insensitive_handle_uniqueness() {
    let dir = scratch_dir("unique");
    let db = scratch_database(&dir).await;
    db.migrate().await.expect("migrate");

    let account = identity::create_account(
        &db,
        "reader@lorehaven.local",
        lorehaven_domain::policy::AgeState::DeclaredAdult,
        identity::AccountStatus::Active,
    )
    .await
    .expect("create account");

    identity::create_pseud(&db, account, "Quill", "Quill")
        .await
        .expect("first pseud");

    let duplicate = identity::create_pseud(&db, account, "quill", "Quill Again").await;
    assert!(
        duplicate.is_err(),
        "handles must be unique case-insensitively"
    );

    // Duplicate email addresses are refused for the same reason.
    let duplicate_account = identity::create_account(
        &db,
        "READER@lorehaven.local",
        lorehaven_domain::policy::AgeState::Unknown,
        identity::AccountStatus::Active,
    )
    .await;
    assert!(
        duplicate_account.is_err(),
        "emails are unique when normalised"
    );

    db.close().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn seeding_creates_a_usable_account_and_is_idempotent() {
    let dir = scratch_dir("seed");
    let config = config_for(&dir);
    let db = scratch_database(&dir).await;
    db.migrate().await.expect("migrate");

    let first = seed::run(&config, &db, &seed_args()).await.expect("seed");
    assert_eq!(first.pseuds.len(), 2);
    assert_eq!(identity::count(&db, "accounts").await.expect("accounts"), 1);
    assert_eq!(identity::count(&db, "pseuds").await.expect("pseuds"), 2);

    // The stored credential must actually verify the advertised password.
    let hash = identity::password_hash(&db, first.account_id)
        .await
        .expect("hash lookup")
        .expect("a credential must exist");
    assert!(
        lorehaven_app::crypto::verify_password("lorehaven-dev", &hash).expect("verify"),
        "the printed development password must be the stored one"
    );

    // Running it again must not multiply rows.
    let second = seed::run(&config, &db, &seed_args())
        .await
        .expect("re-seed");
    assert_eq!(second.account_id, first.account_id);
    assert_eq!(identity::count(&db, "accounts").await.expect("accounts"), 1);
    assert_eq!(identity::count(&db, "pseuds").await.expect("pseuds"), 2);

    // Privacy defaults are recorded, not assumed at render time (spec §7).
    let policy = identity::privacy_value(
        &db,
        identity::PrivacyScope::Account(&first.account_id),
        "messaging_policy",
    )
    .await
    .expect("read privacy setting");
    assert_eq!(policy.as_deref(), Some("contacts_only"));

    db.close().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn health_live_reports_the_running_build() {
    let dir = scratch_dir("live");
    let config = config_for(&dir);
    let db = scratch_database(&dir).await;
    db.migrate().await.expect("migrate");

    let app = server::build_router(AppState::new(config, db.clone()));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json");

    assert_eq!(json["status"], "ok");
    assert!(json["version"].as_str().is_some_and(|v| !v.is_empty()));
    assert!(
        json["build"].as_str().is_some_and(|b| b.contains('+')),
        "the build identifier must carry the revision: {json}"
    );

    db.close().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn health_ready_checks_the_database_and_the_schema() {
    let dir = scratch_dir("ready");
    let config = config_for(&dir);
    let db = scratch_database(&dir).await;

    // Before migrating, readiness must fail: the database answers but the
    // schema is stale, and serving traffic would produce errors.
    let app = server::build_router(AppState::new(config.clone(), db.clone()));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "an unmigrated database must not report ready"
    );

    db.migrate().await.expect("migrate");

    let app = server::build_router(AppState::new(config, db.clone()));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(json["status"], "ready");
    assert_eq!(json["checks"]["database"]["ok"], true);
    assert_eq!(json["checks"]["migrations"]["ok"], true);
    assert_eq!(json["checks"]["storage"]["ok"], true);

    db.close().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn the_api_reports_real_instance_metadata_not_mock_data() {
    let dir = scratch_dir("meta");
    let mut config = config_for(&dir);
    config.site.name = "Test Haven".to_owned();
    let db = scratch_database(&dir).await;
    db.migrate().await.expect("migrate");

    let app = server::build_router(AppState::new(config, db.clone()));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/meta")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json");

    assert_eq!(json["name"], "Test Haven");
    assert_eq!(json["api_version"], "v1");
    assert_eq!(json["environment"], "development");
    assert_eq!(json["policy"]["anonymous_reading"], true);

    db.close().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn the_frontend_shell_is_served_from_the_binary() {
    let dir = scratch_dir("assets");
    let config = config_for(&dir);
    let db = scratch_database(&dir).await;
    db.migrate().await.expect("migrate");

    let app = server::build_router(AppState::new(config, db.clone()));

    // Root: the shell.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body");
    let html = String::from_utf8_lossy(&bytes);
    assert!(
        html.to_lowercase().contains("<!doctype html"),
        "the shell must be HTML"
    );

    // An unknown route falls back to the shell so client-side routing works.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/library/shelves")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);

    // A genuinely missing file is a 404, not a shell.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/assets/does-not-exist.js")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    db.close().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn request_ids_are_echoed_and_generated_when_absent() {
    let dir = scratch_dir("request-id");
    let config = config_for(&dir);
    let db = scratch_database(&dir).await;
    db.migrate().await.expect("migrate");
    let app = server::build_router(AppState::new(config, db.clone()));

    // A supplied id is echoed, so a client can correlate its own trace.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .header("x-request-id", "client-supplied-42")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(
        response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok()),
        Some("client-supplied-42")
    );

    // A hostile id is replaced rather than reflected into the logs.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .header("x-request-id", "bad id with spaces")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let echoed = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert_ne!(echoed, "bad id with spaces");
    assert!(!echoed.is_empty());

    db.close().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn security_headers_are_applied_to_every_response() {
    let dir = scratch_dir("headers");
    let config = config_for(&dir);
    let db = scratch_database(&dir).await;
    db.migrate().await.expect("migrate");
    let app = server::build_router(AppState::new(config, db.clone()));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    let headers = response.headers();
    assert_eq!(
        headers
            .get("x-content-type-options")
            .and_then(|v| v.to_str().ok()),
        Some("nosniff")
    );
    assert_eq!(
        headers.get("x-frame-options").and_then(|v| v.to_str().ok()),
        Some("DENY")
    );
    let csp = headers
        .get("content-security-policy")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(csp.contains("default-src 'self'"), "got {csp}");
    assert!(csp.contains("frame-ancestors 'none'"), "got {csp}");

    db.close().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn error_envelopes_carry_a_request_id_end_to_end() {
    let dir = scratch_dir("envelope");
    let config = config_for(&dir);
    let db = scratch_database(&dir).await;
    db.migrate().await.expect("migrate");
    let app = server::build_router(AppState::new(config, db.clone()));

    // The unknown-API-route fallback is the asset handler, so ask for a file
    // that cannot exist and check the transport-level contract instead.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/nope.json")
                .header("x-request-id", "envelope-7")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok()),
        Some("envelope-7")
    );

    db.close().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn production_startup_is_refused_with_development_settings() {
    let dir = scratch_dir("prod-guard");
    let mut config = config_for(&dir);
    config.environment = lorehaven_app::config::Environment::Production;
    config.assets.dir = Some(dir.clone());
    config.security.cookie_secure = false;
    config.dev.seed_enabled = true;

    let db = scratch_database(&dir).await;
    db.migrate().await.expect("migrate");

    let args = lorehaven_app::cli::ServeArgs {
        no_migrate: true,
        with_worker: false,
    };
    let error = server::serve(config, db, &args)
        .await
        .expect_err("production must refuse to start");
    let message = format!("{error:#}");
    assert!(
        message.contains("cookie-secure"),
        "the refusal must name the offending settings: {message}"
    );

    let _ = std::fs::remove_dir_all(dir);
}
