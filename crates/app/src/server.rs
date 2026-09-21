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

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use axum::extract::Request;
use axum::http::{header, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::Router;
use lorehaven_db::migrate::{self, MigrationReport};
use lorehaven_db::outbox;
use lorehaven_db::Database;
use lorehaven_domain::ids::RequestId;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tracing::Instrument;

use crate::cli::ServeArgs;
use crate::config::{ensure_dir, Config, Environment};
use crate::http::with_request_id;
use crate::limiter::{self, Classified, RouteClass, TrustProxy};
use crate::routes;
use crate::state::AppState;
use crate::{assets, auth, safety, version};

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

    // Fixed for the process lifetime, so it is published once rather than
    // looked up on every request.
    set_trust_proxy(config.security.trust_proxy);

    apply_migration_policy(&config, &db, MigrationPolicy::for_config(&config, args)).await?;

    let state = AppState::new(config.clone(), db);
    let backend = state.db().backend().as_str();

    // One process is the easier deployment for a self-hosted instance; a second
    // `lorehaven worker` process is the better one when the queue is busy. Both
    // are supported and both stop on the same signal.
    if args.with_worker {
        let worker = crate::worker::Worker::new(crate::worker::WorkerOptions::default())
            .with_topic("publish.index", {
                let handler: crate::worker::TopicHandler = Arc::new(|_state: &AppState, event: &outbox::OutboxEvent| {
                    let work_id = event.payload.split("\"work_id\":\"").nth(1)
                        .and_then(|s| s.split('"').next())
                        .unwrap_or("")
                        .to_owned();
                    let state = _state.clone();
                    Box::pin(async move {
                        if work_id.is_empty() {
                            return Err(anyhow::anyhow!("missing work_id in payload"));
                        }
                        let job_payload = serde_json::json!({ "work_id": work_id }).to_string();
                        lorehaven_db::jobs::enqueue(
                            state.db(),
                            lorehaven_domain::jobs::JobKind::Reindex,
                            &job_payload,
                            Some(&format!("reindex:{}", work_id)),
                            None,
                            5,
                            &lorehaven_domain::jobs::RetryPolicy::default(),
                        ).await?;
                        Ok(())
                    })
                });
                handler
            })
            .with_topic("withdraw.deindex", {
                let handler: crate::worker::TopicHandler = Arc::new(|_state: &AppState, event: &outbox::OutboxEvent| {
                    let work_id = event.payload.split("\"work_id\":\"").nth(1)
                        .and_then(|s| s.split('"').next())
                        .unwrap_or("")
                        .to_owned();
                    let state = _state.clone();
                    Box::pin(async move {
                        if work_id.is_empty() {
                            return Err(anyhow::anyhow!("missing work_id in payload"));
                        }
                        let sql = state.db().sql(
                            "DELETE FROM works_index_terms WHERE work_id = ?; DELETE FROM works_index WHERE work_id = ?",
                            "DELETE FROM works_index_terms WHERE work_id = $1::uuid; DELETE FROM works_index WHERE work_id = $1::uuid",
                        );
                        match state.db().backend() {
                            lorehaven_db::Backend::Sqlite => {
                                sqlx::query(&sql).bind(&work_id).bind(&work_id).execute(state.db().sqlite_pool().expect("sqlite")).await?;
                            }
                            lorehaven_db::Backend::Postgres => {
                                sqlx::query(&sql).bind(&work_id).bind(&work_id).execute(state.db().postgres_pool().expect("postgres")).await?;
                            }
                        }
                        Ok(())
                    })
                });
                handler
            })

            .with_topic("publish.notify", {
                let handler: crate::worker::TopicHandler = Arc::new(|_state: &AppState, event: &outbox::OutboxEvent| {
                    let state = _state.clone();
                    let event = event.clone();
                    Box::pin(async move {
                        crate::webhook_delivery::deliver_notification(&state, &event).await
                    })
                });
                handler
            });
        let worker_state = state.clone();
        tracing::info!(worker = %worker.options().id, "worker running in this process");
        tokio::spawn(async move {
            if let Err(error) = worker.run(&worker_state, shutdown_signal()).await {
                tracing::error!(%error, "the worker stopped with an error");
            }
        });
    }

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

    // `into_make_service_with_connect_info` is what makes the peer address
    // available to the rate limiter. Without it every anonymous request has no
    // address to be keyed by, and the limiter quietly does nothing.
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
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
///
/// Order is not incidental. Reading from the inside out:
///
/// ```text
/// handlers
///   ← rate limit      needs the session, so it sits inside session loading
///   ← CSRF            needs the session
///   ← session load    attaches SessionUser, or leaves the request anonymous
///   ← body limit      rejects an oversized body before it is parsed
///   ← timeout
///   ← CORS
///   ← security headers
///   ← request context outermost, so nothing escapes the request id
/// ```
///
/// One cost worth naming: because the limiter runs after session loading, a
/// flood that carries session cookies performs one indexed lookup per request
/// until its address bucket trips. The bucket bounds that, and the alternative —
/// limiting before we know who is asking — would give every account behind a
/// shared address the same allowance.
pub fn build_router(state: AppState) -> Router {
    let config = state.config().clone();
    let cors = build_cors(&config);

    // Routes that are authenticated and state-changing.
    let account_routes: Router<AppState> = Router::new()
        .merge(classified(routes::auth::router(), RouteClass::Auth, &state))
        .merge(classified(
            routes::pseuds::router(),
            RouteClass::Write,
            &state,
        ))
        .merge(classified(
            routes::settings::router(),
            RouteClass::Write,
            &state,
        ))
        .merge(classified(
            routes::works::router(),
            RouteClass::Write,
            &state,
        ))
        .merge(classified(
            routes::collaborators::router(),
            RouteClass::Write,
            &state,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::verify_csrf,
        ));

    let api: Router<AppState> = Router::new()
        .merge(classified(
            routes::meta::router(),
            RouteClass::Default,
            &state,
        ))
        // Reader routes are reachable with no session at all, so they carry the
        // default class rather than the writers' tighter allowance.
        .merge(classified(
            routes::works::read_router(),
            RouteClass::Default,
            &state,
        ))
        // Reading progress, ratings, history, notes and typography. A visitor
        // may reach none of these without a session, so they live under the
        // writers' subtree with the Write class.
        .merge(classified(
            routes::reading::router(),
            RouteClass::Write,
            &state,
        ))
        .merge(routes::reading::authed_router())
        // Positivity filter: the author's own defaults, per-work policy and
        // inbox. Session-scoped like the library: every route needs one and
        // most write, so the whole tree sits under the writers' class.
        .merge(classified(
            routes::feedback::router(),
            RouteClass::Write,
            &state,
        ))
        .merge(classified(
            routes::search::router(),
            RouteClass::Default,
            &state,
        ))
        .merge(classified(
            routes::discovery::router(),
            RouteClass::Default,
            &state,
        ))
        // The reader's library: shelves, bookmarks, private tags, reading
        // statuses, saved views, storage and the update check. Every route
        // needs a session and most of them write, so the whole tree sits under
        // the writers' class rather than being split for the two reads.
        .merge(classified(
            routes::library::router(),
            RouteClass::Write,
            &state,
        ))
        // The job queue: a caller's own jobs, and the cancel action.
        .merge(classified(
            routes::jobs::router(),
            RouteClass::Write,
            &state,
        ))
        // Imports, the source catalogue, the library and source credentials.
        // A write class rather than a read class even for the reads: every one
        // of these is scoped to the caller and the preview may reach out to a
        // source, so they are not cheap shared reads.
        .merge(classified(
            routes::imports::router(),
            RouteClass::Write,
            &state,
        ))
        // Exports: a reader's own files, and the token-addressed download that
        // needs no session because the token is the credential.
        .merge(classified(
            routes::exports::authed_router(),
            RouteClass::Export,
            &state,
        ))
        .merge(classified(
            routes::exports::router(),
            RouteClass::Default,
            &state,
        ))
        // The operator surface. Gated on configuration inside the handlers,
        // because "who is an operator" is a decision about an account and not
        // about a route tree.
        .merge(classified(
            routes::jobs::admin_router(),
            RouteClass::Write,
            &state,
        ))
        .merge(classified(
            routes::imports::admin_router(),
            RouteClass::Write,
            &state,
        ))
        .merge(account_routes)
        // Account permission statement (M27). Write class: session-scoped mutation.
        .merge(classified(
            routes::account_permissions::router(),
            RouteClass::Write,
            &state,
        ))
        .merge(classified(
            routes::rating_integrity::router(),
            RouteClass::Default,
            &state,
        ))
        // Recommendation transparency (M29): "why am I seeing this" explanations.
        .merge(classified(
            routes::recommendation_transparency::router(),
            RouteClass::Default,
            &state,
        ))
        // Taxonomy (nodes, aliases, tags) — M10. Every route needs a session and
        // most write, so it sits under the writers' class.
        .merge(classified(
            routes::taxonomy::router(),
            RouteClass::Write,
            &state,
        ))
        // Community (comments, forums, groups, messaging, blocks) — M12 groundwork.
        // Every route needs a session and most write, so writers' class.
        .merge(classified(
            routes::community::router(),
            RouteClass::Write,
            &state,
        ))
        // Work discussion modes, reaction bar, linked topics (spec 35.1, M31).
        .merge(classified(
            routes::work_discussion::router(),
            RouteClass::Write,
            &state,
        ))
        // Typed votes, budgets, meta-moderation and karma (spec 35.2, M32).
        // Writers' class: every route here needs a session, and the reads are
        // per-viewer because the transparency tier depends on who is asking.
        .merge(classified(
            routes::typed_votes::router(),
            RouteClass::Write,
            &state,
        ))
        // Spoilers, content warnings, drafts, scheduled posts, readability
        // (spec 35.4, M34).
        .merge(classified(
            routes::spoilers::router(),
            RouteClass::Write,
            &state,
        ))
        // Thread modes: reading groups, critique circles, wiki pins, prompts
        // (spec 35.3, M33). Same writers' class as typed votes.
        .merge(classified(
            routes::thread_modes::router(),
            RouteClass::Write,
            &state,
        ))
        // Moderation and community health (spec 35.5, M35): sanctions,
        // slow mode, federation scope, featured posts.
        .merge(classified(
            routes::moderation::router(),
            RouteClass::Write,
            &state,
        ))
        // Events (collections, challenges, requests, wishlists, events) — M13.
        .merge(classified(
            routes::events::router(),
            RouteClass::Write,
            &state,
        ))
        // Governance (reports, moderation, sanctions, appeals, trust) — M14.
        .merge(classified(
            routes::governance::router(),
            RouteClass::Write,
            &state,
        ))
        // Economy (credits, fair queues, bounties, billing) — M15.
        .merge(classified(
            routes::economy::router(),
            RouteClass::Default,
            &state,
        ))
        // Marketplace (listings, commissions, extensions, webhooks, gallery) — M16.
        .merge(classified(
            routes::marketplace::router(),
            RouteClass::Default,
            &state,
        ))
        // Translation pipeline — M17.
        .merge(classified(
            routes::translation::router(),
            RouteClass::Default,
            &state,
        ))
        // Public API, bots, feeds, push, AI — M18.
        .merge(classified(
            routes::external::router(),
            RouteClass::Default,
            &state,
        ))
        // Administration — M19.
        .merge(classified(
            routes::admin::router(),
            RouteClass::Default,
            &state,
        ))
        // M21 skeleton — spec revision 2026-09-14: monetization, gifts,
        // content subscriptions, saved-search alerts, ai_training assertion.
        .merge(classified(
            routes::monetization::router(),
            RouteClass::Write,
            &state,
        ))
        .merge(classified(
            routes::monetization::read_router(),
            RouteClass::Default,
            &state,
        ))
        .merge(classified(
            routes::monetization::gifts_router(),
            RouteClass::Write,
            &state,
        ))
        .merge(classified(
            routes::subscriptions::router(),
            RouteClass::Write,
            &state,
        ))
        .merge(classified(
            routes::subscriptions::ai_training_router(),
            RouteClass::Write,
            &state,
        ))
        // Generalized media — M22 (spec §32). Query doors are public reads;
        // eligibility is applied inside the bodies. Actor and collection
        // mutations are writer-class; sessions and scopes arrive with bodies.
        .merge(classified(
            routes::media::router(),
            RouteClass::Default,
            &state,
        ))
        .merge(classified(
            routes::media::write_router(),
            RouteClass::Write,
            &state,
        ))
        // TTS narration (M26 / spec §32.5). Edition reads are public;
        // requesting a narration requires a session.
        .merge(classified(
            routes::narration::router(),
            RouteClass::Write,
            &state,
        ))
        // Creator dashboard (M24 / spec §32.3, §24.3). Session-scoped: the
        // aggregates are about the acting pseud's own works, banded so a small
        // count cannot name a reader.
        .merge(classified(
            routes::dashboard::router(),
            RouteClass::Default,
            &state,
        ))
        // Decision service (M30 / spec §34).
        .merge(classified(
            routes::decision_service::router(),
            RouteClass::Default,
            &state,
        ))
        // Controlled digital lending (M25 / spec §32.4). All endpoints require
        // a session: borrower identity is needed to grant or revoke a loan.
        .merge(classified(
            routes::lending::router(),
            RouteClass::Write,
            &state,
        ))
        // Derivative pipeline — §32.4. Session-scoped reads plus enqueue
        // writes; actual building happens in the worker.
        .merge(classified(
            routes::derivative::router(),
            RouteClass::Write,
            &state,
        ))
        // Notifications inbox — §5.5. Session-scoped reads plus idempotent
        // mark-read writes; rows are produced by replies, sales and gifts.
        .merge(classified(
            routes::notifications::router(),
            RouteClass::Write,
            &state,
        ))
        // Session loading wraps everything under /api/v1 so that the CSRF layer
        // and the limiter installed per subtree can both see who is asking.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::load_session,
        ));

    let health = classified(routes::health::router(), RouteClass::Default, &state);

    let router: Router<AppState> = Router::new()
        .merge(health)
        // Federation: instance themes, ActivityPub inbox/outbox, similarity — M36.
        // Merged at the root level (not under /api/v1) so ActivityPub endpoints
        // are reachable at /federation/inbox, /federation/actor, etc.
        .merge(routes::federation::routes())
        .nest("/api/v1", api)
        .fallback(assets::serve)
        .layer(RequestBodyLimitLayer::new(config.server.max_body_bytes))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            config.server.request_timeout,
        ))
        .layer(middleware::from_fn(add_trust_proxy_flag))
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

/// Declare a route tree's rate-limit class and install the limiter for it.
///
/// The class marker is inserted into the request extensions by a middleware
/// that runs before the limiter, so the limiter can read the class.
fn classified<S>(router: Router<S>, class: RouteClass, state: &AppState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let state = state.clone();
    router
        // Added first, therefore innermost: the limiter runs after the
        // Classified extension has been inserted by the layer below.
        .layer(middleware::from_fn_with_state(state, limiter::enforce))
        .layer(middleware::from_fn(
            move |mut request: Request, next: Next| async move {
                request.extensions_mut().insert(Classified(class));
                next.run(request).await
            },
        ))
}

/// Record whether a reverse proxy is trusted, for rate-limit keying.
///
/// This is a marker, not a decision: it copies configuration into the request
/// so that [`limiter::client_address`] can consult it without holding state.
async fn add_trust_proxy_flag(mut request: Request, next: Next) -> Response {
    let trusted = TRUST_PROXY.load(std::sync::atomic::Ordering::Relaxed);
    request.extensions_mut().insert(TrustProxy(trusted));
    next.run(request).await
}

/// Whether `X-Forwarded-For` may be believed. Fixed for the process lifetime.
static TRUST_PROXY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Publish the trust-proxy decision. Called once during startup, before the
/// listener is bound.
pub fn set_trust_proxy(trusted: bool) {
    TRUST_PROXY.store(trusted, std::sync::atomic::Ordering::Relaxed);
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
///
/// Public because the worker uses it too, so `serve --with-worker` and
/// `lorehaven worker` both stop on the same `SIGTERM`/`SIGINT` a container
/// runtime sends.
pub async fn shutdown_signal() {
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
        let args = ServeArgs {
            no_migrate: false,
            with_worker: false,
        };
        assert_eq!(
            MigrationPolicy::for_config(&config, &args),
            MigrationPolicy::VerifyOnly
        );
    }

    #[test]
    fn development_applies_and_can_opt_out() {
        let config = Config::development_defaults();
        assert_eq!(
            MigrationPolicy::for_config(
                &config,
                &ServeArgs {
                    no_migrate: false,
                    with_worker: false
                }
            ),
            MigrationPolicy::Apply
        );
        assert_eq!(
            MigrationPolicy::for_config(
                &config,
                &ServeArgs {
                    no_migrate: true,
                    with_worker: false
                }
            ),
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
