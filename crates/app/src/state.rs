//! Shared application state.

use std::sync::Arc;
use std::time::Instant;

use lorehaven_db::Database;

use crate::config::Config;
use crate::limiter::RateLimiter;

/// Cloneable handle to everything a request needs.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    config: Config,
    db: Database,
    rate_limiter: RateLimiter,
    started_at: Instant,
}

impl AppState {
    /// Build state from a resolved configuration and an open database.
    #[must_use]
    pub fn new(config: Config, db: Database) -> Self {
        let rate_limiter = RateLimiter::new(config.rate_limits);
        Self {
            inner: Arc::new(Inner {
                config,
                db,
                rate_limiter,
                started_at: Instant::now(),
            }),
        }
    }

    /// The active configuration.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    /// The database handle.
    #[must_use]
    pub fn db(&self) -> &Database {
        &self.inner.db
    }

    /// The rate limiter, shared across every request in this process.
    #[must_use]
    pub fn rate_limiter(&self) -> &RateLimiter {
        &self.inner.rate_limiter
    }

    /// Milliseconds since the process began serving.
    #[must_use]
    pub fn uptime_ms(&self) -> u64 {
        u64::try_from(self.inner.started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}
