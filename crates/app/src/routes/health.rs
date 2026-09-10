//! Health endpoints.
//!
//! Spec §5 requires two distinct endpoints, and the distinction matters:
//!
//! * `/health/live` answers "is this process alive?" — it must never touch the
//!   database, or a database outage would get the process killed by a
//!   supervisor that reads liveness as "restart me".
//! * `/health/ready` answers "can this process serve traffic?" — it checks the
//!   dependencies that requests actually need.

use std::collections::BTreeMap;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::state::AppState;
use crate::version;

/// Health routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
}

#[derive(Debug, Serialize)]
struct LiveResponse {
    status: &'static str,
    version: &'static str,
    build: String,
    environment: &'static str,
    uptime_ms: u64,
}

/// Liveness: the process is running and can answer.
async fn live(State(state): State<AppState>) -> Json<LiveResponse> {
    Json(LiveResponse {
        status: "ok",
        version: version::VERSION,
        build: version::build_id(),
        environment: state.config().environment.as_str(),
        uptime_ms: state.uptime_ms(),
    })
}

#[derive(Debug, Serialize)]
struct ReadyResponse {
    status: &'static str,
    build: String,
    checks: BTreeMap<&'static str, Check>,
}

#[derive(Debug, Serialize)]
struct Check {
    ok: bool,
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    remedy: Option<String>,
}

impl Check {
    fn ok(detail: impl Into<String>) -> Self {
        Self {
            ok: true,
            detail: detail.into(),
            remedy: None,
        }
    }

    fn failed(detail: impl Into<String>, remedy: impl Into<String>) -> Self {
        Self {
            ok: false,
            detail: detail.into(),
            remedy: Some(remedy.into()),
        }
    }
}

/// Readiness: every essential dependency is usable.
async fn ready(State(state): State<AppState>) -> (StatusCode, Json<ReadyResponse>) {
    let config = state.config();
    let db = state.db();
    let mut checks = BTreeMap::new();

    // --- database -----------------------------------------------------------
    checks.insert(
        "database",
        match db.ping().await {
            Ok(()) => Check::ok(format!(
                "{} reachable at {}",
                db.backend().as_str(),
                db.redacted_url()
            )),
            Err(error) => Check::failed(
                format!("{} unreachable: {error}", db.backend().as_str()),
                "check the database URL and that the server is running",
            ),
        },
    );

    // --- migrations ---------------------------------------------------------
    let known = lorehaven_db::migrate::catalogue(db.backend()).len();
    checks.insert(
        "migrations",
        match lorehaven_db::migrate::pending(db).await {
            Ok(pending) if pending.is_empty() => Check::ok(format!("{known} migration(s) applied")),
            Ok(pending) => Check::failed(
                format!("{} of {known} migration(s) applied", known - pending.len()),
                "run `lorehaven migrate`",
            ),
            Err(error) => Check::failed(
                format!("cannot read the migration ledger: {error}"),
                "run `lorehaven migrate`",
            ),
        },
    );

    // --- storage ------------------------------------------------------------
    checks.insert(
        "storage",
        match check_storage(&config.storage.root).await {
            Ok(detail) => Check::ok(detail),
            Err(error) => Check::failed(
                format!("{}: {error}", config.storage.root.display()),
                "ensure the directory exists and is writable by the service user",
            ),
        },
    );

    let ready = checks.values().all(|check| check.ok);
    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status,
        Json(ReadyResponse {
            status: if ready { "ready" } else { "degraded" },
            build: version::build_id(),
            checks,
        }),
    )
}

/// Confirm the storage root exists and accepts writes.
///
/// Writing a probe file is the only check that catches the real failures —
/// a read-only mount, a full disk, a wrong owner — rather than merely an
/// absent directory.
pub async fn check_storage(root: &std::path::Path) -> anyhow::Result<String> {
    tokio::fs::create_dir_all(root)
        .await
        .map_err(|error| anyhow::anyhow!("cannot create directory: {error}"))?;

    let probe = root.join(format!(".lorehaven-write-probe-{}", std::process::id()));
    tokio::fs::write(&probe, b"probe")
        .await
        .map_err(|error| anyhow::anyhow!("cannot write: {error}"))?;
    tokio::fs::remove_file(&probe)
        .await
        .map_err(|error| anyhow::anyhow!("cannot remove probe file: {error}"))?;

    Ok(format!("{} is writable", root.display()))
}
