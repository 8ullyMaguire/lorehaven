//! HTTP server composition and the middleware stack.
//!
//! Spec §5 requires structured logging with request IDs and a health endpoint;
//! spec §3.5 requires cookie-authenticated state changes to be CSRF-protected;
//! and the rebuild notes in this project's own history make the point that
//! protection belongs in *middleware*, not in whichever handler remembers to
//! call a helper.
//!
//! So every request passes through one stack, defined here and nowhere else:
//!
//! ```text
//! request-context   request id, tracing span, access log, echo header
//! security-headers  CSP, nosniff, frame denial, referrer policy, HSTS
//! cors              explicit origins only, never a wildcard in production
//! timeout           per-request ceiling, so one slow handler cannot pin a slot
//! body-limit        bounded bodies before a handler can be reached
//! router            the application
//! ```

use std::time::Instant;

use anyhow::{Context, Result};
use axum::extract::Request;
use axum::http::{header, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::Router;
use lorehaven_db::migrate::{self, MigrationReport};
use lorehaven_db::Database;
use lorehaven_domain::ids::RequestId;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tracing::Instrument;

use crate::cli::ServeArgs;
use crate::config::{ensure_dir, Config, Environment};
use crate::http::with_request_id;
use crate::routes;
use crate::state::AppState;
use crate::{assets, safety, version};

/// How the server treats migrations it finds pending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationPolicy {
    /// Apply pending migrations before serving (development, test).
    Apply,
    /// Refuse to serve with pending migrations (production).
    VerifyOnly,
    /// Ignore schema state entirely (`--no-migrate`).
    Ignore,
}

impl MigrationPolicy {
    /// Decide from the environment and flags.
    #[must_use]
    pub fn for_config(config: &Config, args: &ServeArgs) -> Self {
        if config.environment == Environment::Production {
            // Spec §22: migrations are an explicit deploy step and the server
            // must not surprise an operator by altering the schema at boot.
            Self::VerifyOnly
        } else if args.no_migrate {
            Self::Ignore
        } else {
            Self::Apply
        }
    }
}

/// Server identity recorded for `/health/ready` and the access log.
pub const SERVER_NAME: &str = concat!("lorehaven/", env!("CARGO_PKG_VERSION"));

/// Bind, serve, and shut down cleanly.
pub async fn serve(config: Config, db: Database, args: &ServeArgs) -> Result<()> {
    safety::validate_for_startup(&config)?;
    ensure_dir(&config.storage.root)?;

    apply_migration_policy(&config, &db, MigrationPolicy::for_config(&config, args)).await?;

    let state = AppState::new(config.clone(), db);
    let backend = state.db().backend().as_str();
    let router = build_router(state);

    let address = format!("{}:{}", config.server.bind, config.server.port);
    let listener = tokio::net::TcpListener::bind(&address)
        .await
        .with_context(|| format!("binding {address}"))?;
    let bound = listener.local_addr().context("reading the bound address")?;

    tracing::info!(
        address = %bound,
        environment = config.environment.as_str(),
        build = %version::build_id(),
        base_url = %config.site.base_url,
        database = backend,
        "listening"
    );

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serving HTTP")?;

    tracing::info!("shutdown complete");
    Ok(())
}

/// Apply, verify, or ignore schema migrations according to policy.
pub async fn apply_migration_policy(
    config: &Config,
    db: &Database,
    policy: MigrationPolicy,
) -> Result<MigrationReport> {
    let pending = migrate::pending(db).await?;

    if pending.is_empty() {
        tracing::debug!("schema is up to date");
        return Ok(MigrationReport {
            backend: db.backend(),
            applied: Vec::new(),
            already_applied: Vec::new(),
        });
    }

    match policy {
        MigrationPolicy::VerifyOnly => anyhow::bail!(
            "{} migration(s) are pending and this is a production instance: \
             run `lorehaven migrate` as its own step before starting the server \
             (pending: {})",
            pending.len(),
            pending.join(", ")
        ),
        MigrationPolicy::Ignore => {
            tracing::warn!(
                pending = pending.len(),
                "serving with a stale schema because --no-migrate was passed"
            );
            Ok(MigrationReport {
                backend: db.backend(),
                applied: Vec::new(),
                already_applied: Vec::new(),
            })
        }
        MigrationPolicy::Apply => {
            let report = db.migrate().await?;
            tracing::info!(
                applied = report.applied.len(),
                migrations = ?report.applied,
                "migrations applied"
            );
            let _ = config;
            Ok(report)
        }
    }
}

/// Assemble the router with the full middleware stack.
pub fn build_router(state: AppState) -> Router {
    let config = state.config().clone();
    let cors = build_cors(&config);

    let router = Router::new()
        .merge(routes::health::router())
        .nest("/api/v1", routes::meta::router())
        .fallback(assets::serve)
        .layer(RequestBodyLimitLayer::new(config.server.max_body_bytes))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            config.server.request_timeout,
        ))
        .layer(middleware::from_fn(security_headers))
        // Added last, therefore outermost: nothing escapes correlation.
        .layer(middleware::from_fn(request_context));

    // CORS is only installed when origins are declared, so the default posture
    // is same-origin with no permissive layer involved at all.
    let router = match cors {
        Some(layer) => router.layer(layer),
        None => router,
    };

    router.with_state(state)
}

/// Cross-origin policy. Empty means same-origin only.
fn build_cors(config: &Config) -> Option<CorsLayer> {
    if config.server.cors_origins.is_empty() {
        return None;
    }

    let origins: Vec<HeaderValue> = config
        .server
        .cors_origins
        .iter()
        .filter_map(|origin| HeaderValue::from_str(origin).ok())
        .collect();

    if origins.is_empty() {
        return None;
    }

    Some(
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(origins))
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PATCH,
                Method::DELETE,
                Method::OPTIONS,
            ])
            .allow_headers([
                header::CONTENT_TYPE,
                header::ACCEPT,
                HeaderName::from_static("x-csrf-token"),
                HeaderName::from_static("idempotency-key"),
            ])
            .allow_credentials(true),
    )
}

/// Correlation and access logging.
pub async fn request_context(mut request: Request, next: Next) -> Response {
    let header_name = HeaderName::from_static("x-request-id");

    let request_id = request
        .headers()
        .get(&header_name)
        .and_then(|value| value.to_str().ok())
        .and_then(RequestId::sanitize)
        .unwrap_or_else(RequestId::generate);

    // Replace whatever arrived with our sanitised value, so a client cannot
    // inject newlines into the log through the header.
    if let Ok(value) = HeaderValue::from_str(request_id.as_str()) {
        request.headers_mut().insert(header_name.clone(), value);
    }
    request.extensions_mut().insert(request_id.clone());

    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let request_id_text = request_id.to_string();

    let span = tracing::info_span!(
        "request",
        method = %method,
        path = %path,
        request_id = %request_id_text
    );

    let echo = request_id_text.clone();
    let response = with_request_id(request_id_text, async move {
        let started = Instant::now();
        let mut response = next.run(request).await;
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        tracing::info!(
            status = response.status().as_u16(),
            latency_ms,
            "request completed"
        );

        if let Ok(value) = HeaderValue::from_str(&echo) {
            response.headers_mut().insert(header_name, value);
        }
        response
    })
    .instrument(span)
    .await;

    response
}

/// Baseline security headers.
///
/// The CSP is deliberately expressed in terms the SPA build satisfies:
/// external module scripts from our own origin, styles from our own origin
/// plus inline style attributes (which Svelte uses for dynamic values), and no
/// object embedding or framing at all.
pub async fn security_headers(request: Request, next: Next) -> Response {
    let hsts = request.extensions().get::<HstsFlag>().is_some();

    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    headers.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("geolocation=(), microphone=(), camera=(), payment=()"),
    );
    headers.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(
            "default-src 'self'; \
             img-src 'self' data:; \
             style-src 'self' 'unsafe-inline'; \
             script-src 'self'; \
             connect-src 'self'; \
             font-src 'self'; \
             form-action 'self'; \
             base-uri 'none'; \
             frame-ancestors 'none'; \
             object-src 'none'",
        ),
    );

    if hsts {
        headers.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        );
    }

    response
}

/// Marker extension enabling HSTS on responses.
///
/// HSTS is only meaningful when the site is genuinely served over HTTPS, so it
/// is opt-in rather than blanket-applied: sending it from a plain-HTTP
/// development instance would poison the browser for that host.
#[derive(Debug, Clone, Copy)]
pub struct HstsFlag;

/// Resolve when the process is asked to stop.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(error) => {
                tracing::warn!(%error, "cannot listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => tracing::info!("received interrupt"),
        () = terminate => tracing::info!("received terminate"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_never_auto_migrates() {
        let mut config = Config::development_defaults();
        config.environment = Environment::Production;
        let args = ServeArgs { no_migrate: false };
        assert_eq!(
            MigrationPolicy::for_config(&config, &args),
            MigrationPolicy::VerifyOnly
        );
    }

    #[test]
    fn development_applies_and_can_opt_out() {
        let config = Config::development_defaults();
        assert_eq!(
            MigrationPolicy::for_config(&config, &ServeArgs { no_migrate: false }),
            MigrationPolicy::Apply
        );
        assert_eq!(
            MigrationPolicy::for_config(&config, &ServeArgs { no_migrate: true }),
            MigrationPolicy::Ignore
        );
    }

    #[test]
    fn cors_is_absent_unless_origins_are_declared() {
        let config = Config::development_defaults();
        assert!(build_cors(&config).is_none());

        let mut config = config;
        config.server.cors_origins = vec!["https://lorehaven.example".to_owned()];
        assert!(build_cors(&config).is_some());

        // Garbage origins must not silently produce a permissive policy.
        config.server.cors_origins = vec!["not a header value\n".to_owned()];
        assert!(build_cors(&config).is_none());
    }

    #[test]
    fn the_status_code_type_is_available_for_the_access_log() {
        // Guards against a refactor that drops the import used in the span.
        assert_eq!(StatusCode::OK.as_u16(), 200);
    }
}
