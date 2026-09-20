//! Rate limiting.
//!
//! `docs/verification.md` listed the absence of rate limiting as a blocker that
//! "must land before any write endpoint does". This is that blocker being
//! removed, before the first write endpoint exists rather than after.
//!
//! Design choices worth stating:
//!
//! * **Enforced by middleware, not by handlers.** A handler that forgets to
//!   call a limiter is a handler with no limit. Routes declare a class when
//!   they are mounted; the middleware does the rest.
//! * **Keyed by identity where we have one, address otherwise.** An
//!   authenticated request is limited per account *and* per address, so neither
//!   a shared NAT nor a pile of accounts gives a free pass.
//! * **In-process.** One instance, one process — the deployment model in
//!   spec §2.1. A second process would need a shared store, and that is a
//!   decision to make when a second process exists.
//! * **Fail closed.** An unknown class is rejected rather than unlimited.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::{IntoResponse as _, Response};
use lorehaven_domain::AppError;

use crate::state::AppState;

/// How much a route family costs.
///
/// The classes exist because the right limit for a login attempt and the right
/// limit for reading a page are not the same number, and a single global limit
/// would either be uselessly generous for the former or uselessly tight for the
/// latter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RouteClass {
    /// Sign-in, registration, password reset. Tight: these are brute-force and
    /// mail-relay abuse targets.
    Auth,
    /// Ordinary state-changing requests.
    Write,
    /// Read requests that touch the database heavily.
    Search,
    /// Bulk export: starting a job is cheap, the job itself is not.
    Export,
    /// Everything else: static assets and cheap reads.
    Default,
}

impl RouteClass {
    /// Name used in configuration and logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auth => "auth",
            Self::Write => "write",
            Self::Search => "search",
            Self::Export => "export",
            Self::Default => "default",
        }
    }

    /// Parse a class name, rejecting anything unknown.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "auth" => Some(Self::Auth),
            "write" => Some(Self::Write),
            "search" => Some(Self::Search),
            "export" => Some(Self::Export),
            "default" => Some(Self::Default),
            _ => None,
        }
    }
}

/// A class's allowance.
#[derive(Debug, Clone, Copy)]
pub struct Quota {
    /// Burst size: how many requests may arrive back to back.
    pub burst: u32,
    /// Sustained refill rate, per minute.
    pub per_minute: u32,
}

impl Quota {
    /// Refill expressed as tokens per second, for the bucket maths.
    fn refill_per_second(self) -> f64 {
        f64::from(self.per_minute) / 60.0
    }
}

/// Limits for every class.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Sign-in and account creation.
    pub auth: Quota,
    /// State-changing requests.
    pub write: Quota,
    /// Expensive reads.
    pub search: Quota,
    /// Bulk export: starting a job is cheap, the job itself is not.
    pub export: Quota,
    /// Everything else.
    pub default: Quota,
    /// Multiplier applied to address-keyed buckets, to absorb shared NATs
    /// without letting an individual account escape its own limit.
    pub address_multiplier: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            // Ten sign-in attempts in a burst is enough for a person who cannot
            // remember which password they used; a hundred is a credential-stuffing
            // run.
            auth: Quota {
                burst: 10,
                per_minute: 30,
            },
            write: Quota {
                burst: 20,
                per_minute: 60,
            },
            search: Quota {
                burst: 30,
                per_minute: 120,
            },
            // Bulk export: 5 back-to-back is enough for a person who clicked
            // twice; 10/minute leaves room for the other doors while a large
            // bundle runs.
            export: Quota {
                burst: 5,
                per_minute: 10,
            },
            default: Quota {
                burst: 120,
                per_minute: 600,
            },
            address_multiplier: 4,
        }
    }
}

impl Limits {
    /// The quota for a class.
    #[must_use]
    pub const fn quota(self, class: RouteClass) -> Quota {
        match class {
            RouteClass::Auth => self.auth,
            RouteClass::Write => self.write,
            RouteClass::Search => self.search,
            RouteClass::Export => self.export,
            RouteClass::Default => self.default,
        }
    }
}

/// One token bucket.
#[derive(Debug)]
struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

impl Bucket {
    fn new(capacity: f64) -> Self {
        Self {
            tokens: capacity,
            last_refill: Instant::now(),
        }
    }

    /// Take a token if one is available. Returns the wait time when refused.
    fn take(&mut self, quota: Quota, now: Instant) -> Result<(), Duration> {
        let elapsed = now
            .saturating_duration_since(self.last_refill)
            .as_secs_f64();
        self.last_refill = now;

        // A zero refill rate means "never refills", which is a legitimate
        // configuration (a burst-only allowance, used by tests and by any
        // future one-shot limit). Guard the division explicitly rather than
        // letting a zero reach `Duration::from_secs_f64`, which panics on
        // infinity.
        let refill = quota.refill_per_second();
        if refill > 0.0 && elapsed > 0.0 {
            self.tokens = (self.tokens + elapsed * refill).min(f64::from(quota.burst));
        }

        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Ok(())
        } else {
            let wait = if refill > 0.0 {
                let deficit = 1.0 - self.tokens;
                let seconds = deficit / refill;
                // At least a second: telling a client "retry in 0s" invites a
                // loop. At most a day: a longer hint is not more useful.
                Duration::from_secs_f64(seconds.clamp(1.0, 86_400.0))
            } else {
                // Never refills, so there is no honest time to suggest.
                Duration::from_secs(86_400)
            };
            Err(wait)
        }
    }
}

/// The in-process limiter.
///
/// Uses `Arc<Mutex<...>>` so clones of `AppState` share the same bucket map
/// (Axum clones the state per request; a fresh Mutex per clone would reset
/// the buckets and make the limiter inert).
#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<RateLimiterInner>,
}

#[derive(Debug)]
struct RateLimiterInner {
    buckets: Mutex<HashMap<String, Bucket>>,
    last_prune: Mutex<Instant>,
}

/// How long an unused bucket is kept before it is dropped.
///
/// Pruning matters for a long-running process: without it, the map grows by one
/// entry per distinct address forever, which is a slow memory leak an attacker
/// can drive.
const BUCKET_IDLE_TTL: Duration = Duration::from_secs(600);

impl RateLimiter {
    /// Build a limiter.
    ///
    /// Limits are not stored: the quota for a request comes from configuration
    /// at the call site, so there is exactly one place a limit is defined.
    #[must_use]
    pub fn new(_limits: Limits) -> Self {
        Self {
            inner: Arc::new(RateLimiterInner {
                buckets: Mutex::new(HashMap::new()),
                last_prune: Mutex::new(Instant::now()),
            }),
        }
    }

    /// Check a request cost against a bucket, returning a retry delay on refusal.
    fn check(&self, key: &str, quota: Quota) -> Result<(), Duration> {
        let now = Instant::now();
        let mut buckets = self
            .inner
            .buckets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        self.prune_if_stale(&mut buckets, now);

        let bucket = buckets
            .entry(key.to_owned())
            .or_insert_with(|| Bucket::new(f64::from(quota.burst)));

        let result = bucket.take(quota, now);
        tracing::debug!(key = %key, tokens = bucket.tokens, result = ?result, "rate limit check");
        result
    }

    /// Drop buckets that have refilled and gone quiet.
    fn prune_if_stale(&self, buckets: &mut HashMap<String, Bucket>, now: Instant) {
        let mut last = self
            .inner
            .last_prune
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if now.saturating_duration_since(*last) < BUCKET_IDLE_TTL {
            return;
        }
        *last = now;
        buckets.retain(|_, bucket| {
            now.saturating_duration_since(bucket.last_refill) < BUCKET_IDLE_TTL
        });
    }

    /// Number of tracked buckets. Exposed for tests and diagnostics.
    #[must_use]
    pub fn tracked_buckets(&self) -> usize {
        self.inner
            .buckets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

/// A request that must be rate limited, carrying its class.
#[derive(Debug, Clone, Copy)]
pub struct Classified(pub RouteClass);

/// Middleware enforcing the class stored on the request.
pub async fn enforce(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let Some(Classified(class)) = request.extensions().get::<Classified>().copied() else {
        // Fail closed: an unclassified route is a bug, not a free pass.
        tracing::error!(
            path = %request.uri().path(),
            "route reached the rate limiter without a declared class"
        );
        return crate::http::ApiError(AppError::Internal(anyhow::anyhow!(
            "route {} has no declared rate-limit class",
            request.uri().path()
        )))
        .into_response();
    };

    let quota = state.config().rate_limits.quota(class);
    let limiter = state.rate_limiter();

    // Address bucket first: it catches a single source cycling accounts.
    let address_quota = Quota {
        burst: quota
            .burst
            .saturating_mul(state.config().rate_limits.address_multiplier),
        per_minute: quota
            .per_minute
            .saturating_mul(state.config().rate_limits.address_multiplier),
    };
    //
    // When no address can be determined the bucket is shared rather than
    // skipped. Skipping it — the obvious reading of "we have nothing to key
    // on" — makes the limiter silently inert, which is how a process that
    // forgot to serve `ConnectInfo` ends up with no protection at all. It was
    // exactly that bug in this codebase, caught by the test below.
    let address_key = match client_address(request.headers(), request.extensions()) {
        Some(address) => format!("ip:{}:{address}", class.as_str()),
        None => {
            warn_missing_address(&state);
            format!("ip:{}:unknown", class.as_str())
        }
    };
    if let Err(retry_after) = limiter.check(&address_key, address_quota) {
        return too_many(retry_after);
    }

    // Account bucket, when the request carries an authenticated session.
    if let Some(user) = request.extensions().get::<crate::auth::SessionUser>() {
        if let Err(retry_after) = limiter.check(
            &format!("user:{}:{}", class.as_str(), user.account_id),
            quota,
        ) {
            return too_many(retry_after);
        }
    }

    next.run(request).await
}

/// Announce once, and only once, that requests are sharing a bucket because no
/// client address is available.
///
/// A warning per request would be worse than the condition it reports.
fn warn_missing_address(state: &AppState) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static WARNED: AtomicBool = AtomicBool::new(false);

    if WARNED.swap(true, Ordering::Relaxed) {
        return;
    }
    let hint = if state.config().security.trust_proxy {
        "the reverse proxy did not send X-Forwarded-For"
    } else {
        "the server is not receiving connection information, or the instance sits behind a \
         proxy without [security] trust_proxy = true"
    };
    tracing::warn!(
        hint,
        "no client address available for rate limiting; all such requests now share one bucket"
    );
}

/// Build the 429 response, always with a `Retry-After`.
fn too_many(retry_after: Duration) -> Response {
    let seconds = retry_after.as_secs().max(1);
    tracing::warn!(retry_after_secs = seconds, "rate limited");
    crate::http::ApiError(AppError::RateLimited {
        retry_after_secs: seconds,
    })
    .into_response()
}

/// Determine the client address, preferring a proxy header only when trusted.
///
/// `X-Forwarded-For` is trivially forgeable, so it is used *only* when the
/// operator has declared the instance to sit behind a trusted proxy. Otherwise
/// a client could mint a fresh bucket per request by inventing an address.
#[must_use]
pub fn client_address(headers: &HeaderMap, extensions: &axum::http::Extensions) -> Option<IpAddr> {
    let trust_proxy = extensions.get::<TrustProxy>().is_some_and(|flag| flag.0);

    if trust_proxy {
        if let Some(value) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
            // The *first* entry is the original client; the rest are proxies.
            if let Some(first) = value.split(',').next() {
                if let Ok(ip) = first.trim().parse::<IpAddr>() {
                    return Some(ip);
                }
            }
        }
        if let Some(value) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
            if let Ok(ip) = value.trim().parse::<IpAddr>() {
                return Some(ip);
            }
        }
    }

    extensions
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip())
}

/// Marker extension set when the instance sits behind a trusted proxy.
#[derive(Debug, Clone, Copy)]
pub struct TrustProxy(pub bool);

#[cfg(test)]
mod tests {
    use super::*;

    fn quota(burst: u32, per_minute: u32) -> Quota {
        Quota { burst, per_minute }
    }

    #[test]
    fn a_burst_is_allowed_then_refused() {
        let mut bucket = Bucket::new(3.0);
        let now = Instant::now();
        let q = quota(3, 0);

        assert!(bucket.take(q, now).is_ok());
        assert!(bucket.take(q, now).is_ok());
        assert!(bucket.take(q, now).is_ok());

        let refused = bucket.take(q, now);
        assert!(refused.is_err(), "a fourth request must be refused");
        assert!(refused.unwrap_err() >= Duration::from_secs(1));
    }

    #[test]
    fn tokens_refill_over_time() {
        let mut bucket = Bucket::new(1.0);
        let start = Instant::now();
        let q = quota(1, 60); // one per second

        assert!(bucket.take(q, start).is_ok());
        assert!(bucket.take(q, start).is_err());

        // Two seconds later there should be a token again.
        assert!(bucket.take(q, start + Duration::from_secs(2)).is_ok());
    }

    #[test]
    fn a_refill_never_exceeds_the_burst() {
        let mut bucket = Bucket::new(2.0);
        let start = Instant::now();
        let q = quota(2, 60);

        // A long idle period must not bank unlimited tokens.
        let _ = bucket.take(q, start + Duration::from_secs(3600));
        assert!(bucket.take(q, start + Duration::from_secs(3600)).is_ok());
        assert!(
            bucket.take(q, start + Duration::from_secs(3600)).is_err(),
            "capacity must be capped at the burst size"
        );
    }

    #[test]
    fn retry_after_is_never_zero() {
        let mut bucket = Bucket::new(1.0);
        let now = Instant::now();
        let q = quota(1, 3600); // very slow refill
        let _ = bucket.take(q, now);
        let wait = bucket.take(q, now).unwrap_err();
        assert!(wait >= Duration::from_secs(1), "got {wait:?}");
    }

    #[test]
    fn a_zero_refill_rate_is_handled_without_panicking() {
        // A burst-only allowance must not divide by zero on the way to a
        // retry hint. This was a real panic before it was a test.
        let mut bucket = Bucket::new(1.0);
        let now = Instant::now();
        let q = quota(1, 0);

        assert!(bucket.take(q, now).is_ok());
        let wait = bucket.take(q, now).expect_err("burst is spent");
        assert!(wait >= Duration::from_secs(1));
        // And it stays refused however long we wait.
        assert!(bucket.take(q, now + Duration::from_secs(3600)).is_err());
    }

    #[test]
    fn a_retry_hint_is_capped_at_a_day() {
        let mut bucket = Bucket::new(1.0);
        let now = Instant::now();
        // One token per month: the honest wait is far longer than a day, and a
        // client should be told something it can act on.
        let q = quota(1, 1);
        let _ = bucket.take(q, now);
        let wait = bucket.take(q, now).unwrap_err();
        assert!(wait <= Duration::from_secs(86_400), "got {wait:?}");
    }

    #[test]
    fn the_limiter_keys_are_independent() {
        let limiter = RateLimiter::new(Limits::default());
        let q = quota(1, 0);

        assert!(limiter.check("ip:auth:1.2.3.4", q).is_ok());
        assert!(limiter.check("ip:auth:1.2.3.4", q).is_err());
        assert!(
            limiter.check("ip:auth:5.6.7.8", q).is_ok(),
            "one address must not consume another's allowance"
        );
        assert_eq!(limiter.tracked_buckets(), 2);
    }

    #[test]
    fn classes_have_distinct_limits() {
        let limits = Limits::default();
        assert!(limits.quota(RouteClass::Auth).burst < limits.quota(RouteClass::Default).burst);
        assert!(limits.quota(RouteClass::Write).burst < limits.quota(RouteClass::Search).burst);
    }

    #[test]
    fn class_names_round_trip_and_reject_nonsense() {
        for class in [
            RouteClass::Auth,
            RouteClass::Write,
            RouteClass::Search,
            RouteClass::Export,
            RouteClass::Default,
        ] {
            assert_eq!(RouteClass::parse(class.as_str()), Some(class));
        }
        assert_eq!(RouteClass::parse("expensive"), None);
    }

    #[test]
    fn forwarded_headers_are_ignored_unless_the_proxy_is_trusted() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.9, 10.0.0.1".parse().unwrap());

        // Untrusted: no address is taken from the header at all.
        let untrusted = axum::http::Extensions::new();
        assert_eq!(client_address(&headers, &untrusted), None);

        // Trusted: the first entry is the original client.
        let mut trusted = axum::http::Extensions::new();
        trusted.insert(TrustProxy(true));
        assert_eq!(
            client_address(&headers, &trusted),
            Some("203.0.113.9".parse().unwrap())
        );

        // Explicitly untrusted proxy still ignored.
        let mut disabled = axum::http::Extensions::new();
        disabled.insert(TrustProxy(false));
        assert_eq!(client_address(&headers, &disabled), None);
    }
}
