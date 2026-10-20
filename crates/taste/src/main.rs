//! The `taste` binary.
//!
//! **It does nothing yet, and it says so.** This is the service mode of the taste
//! engine (spec §8.2): a separate process against the same catalogue bundle, for
//! an instance whose graph does not fit beside the archive's request/response
//! process.
//!
//! The discipline this binary follows from the start: **a stub that returns a
//! plausible number is worse than no stub at all**, because a stub is a ranking
//! that looks measured and is not.
//!
//! **One thing is real, and it is the health endpoint.** `/healthz` returns
//! **503 when the process is over its own RSS cap** (spec §22.3) — and it reads
//! the process's actual resident set to decide. A `/healthz` that always
//! answered `200` would be a green light on a process the OOM killer is about
//! to take, and the operator would find out from the outage.

use std::net::SocketAddr;

use axum::routing::get;
use axum::{http::StatusCode, Json, Router};

use darkhold_taste::health;

/// The port the service listens on.
const PORT: u16 = 8180;

#[tokio::main]
async fn main() {
    println!("darkhold-taste — the taste engine (spec §8.2)");
    println!(
        "catalogue ingest, the ranker and the budget gate are not built yet; the \
         health endpoint is."
    );

    let app = Router::new().route("/healthz", get(healthz));
    let address = SocketAddr::from(([127, 0, 0, 1], PORT));
    let listener = match tokio::net::TcpListener::bind(address).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("cannot bind {address}: {error}");
            std::process::exit(1);
        }
    };
    println!("listening on http://{address}");
    if let Err(error) = axum::serve(listener, app).await {
        eprintln!("the server stopped: {error}");
        std::process::exit(1);
    }
}

/// The health endpoint.
///
/// **503 when the process is over its own cap**, and the body says why. The
/// check reads the process's real resident set — there is no path through this
/// handler that answers `200` while the process is over its cap.
async fn healthz() -> (StatusCode, Json<serde_json::Value>) {
    let report = health::check(health::CAP_FULL_BYTES);
    let status = if report.ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(serde_json::json!({
            "status": if report.ok { "ok" } else { "over-cap" },
            "rss_bytes": report.rss_bytes,
            "peak_rss_bytes": report.peak_rss_bytes,
            "cap_bytes": report.cap_bytes,
            "detail": report.detail,
        })),
    )
}
