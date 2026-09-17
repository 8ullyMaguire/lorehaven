//! Shared application state.

use std::sync::Arc;
use std::time::Instant;

use lorehaven_db::Database;
use lorehaven_scrapers::registry::Registry;

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
    /// The source adapters.
    ///
    /// One instance for the whole process, shared by the preview a reader waits
    /// on and the import the worker runs later. Two registries would mean a URL
    /// could route to one adapter when previewed and another when imported, and
    /// the reader would have confirmed a plan that nothing then followed.
    registry: Registry,
    /// The external converters this instance found at startup.
    ///
    /// Detected once rather than per request: what an instance can produce is a
    /// property of the machine it runs on, and an interface that offered a format
    /// on one request and not the next would be describing nothing.
    converters: crate::exports::Converters,
    /// The configured narration engine, built once at startup.
    ///
    /// Held rather than rebuilt per job for the same reason the converters are:
    /// which voices an instance can speak in is a property of the host, and a
    /// worker that rebuilt the engine per chunk would re-read the same paths on
    /// every one. A `Result` because an instance may name an engine this build
    /// does not have — that is reportable state, not a startup failure, so a
    /// deployment with a typo in `tts.engine` still serves its library.
    tts: Result<Arc<dyn crate::tts::TtsEngine>, String>,
}

impl AppState {
    /// Build state from a resolved configuration and an open database.
    #[must_use]
    pub fn new(config: Config, db: Database) -> Self {
        let rate_limiter = RateLimiter::new(config.rate_limits);
        let converters = crate::exports::Converters::detect();
        // `which piper` is the same discovery the converters use, so an operator
        // who installed Piper next to Calibre gets both without configuring
        // either by hand.
        let piper = converters.discover_program("piper");
        let tts = crate::tts::build_engine(&config.tts, piper.as_deref())
            .map(Arc::<dyn crate::tts::TtsEngine>::from)
            .map_err(|error| error.to_string());
        Self {
            inner: Arc::new(Inner {
                config,
                db,
                rate_limiter,
                started_at: Instant::now(),
                registry: lorehaven_scrapers::sites::default_registry(),
                converters,
                tts,
            }),
        }
    }

    /// The active configuration.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    /// The source adapters.
    #[must_use]
    pub fn registry(&self) -> &Registry {
        &self.inner.registry
    }

    /// The external converters this instance found at startup.
    #[must_use]
    pub fn converters(&self) -> &crate::exports::Converters {
        &self.inner.converters
    }

    /// The configured narration engine, or the reason this instance has none.
    ///
    /// Callers get a typed `Result` rather than an `Option` so the reason — a
    /// name this build does not know, a missing voice model — travels with the
    /// failure instead of being flattened into "unavailable".
    pub fn tts_engine(&self) -> Result<&dyn crate::tts::TtsEngine, &str> {
        match &self.inner.tts {
            Ok(engine) => Ok(engine.as_ref()),
            Err(reason) => Err(reason.as_str()),
        }
    }

    /// Whether a narration can be produced on this instance.
    #[must_use]
    pub fn can_narrate(&self) -> bool {
        self.inner
            .tts
            .as_ref()
            .map(|engine| engine.is_available())
            .unwrap_or(false)
    }

    /// Replace the adapters.
    ///
    /// For tests, which need an adapter that answers from a recorded page: a
    /// test that reaches the network fails on a plane, and an import rule that
    /// is only exercised against a live site is a rule nobody has pinned.
    #[must_use]
    pub fn with_registry(mut self, registry: Registry) -> Self {
        Arc::get_mut(&mut self.inner)
            .expect("state is configured before it is shared")
            .registry = registry;
        self
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
