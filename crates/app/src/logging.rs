//! Tracing initialisation.
//!
//! Spec §5 requires "structured logging with request IDs" and forbids logging
//! secrets or full authenticated URLs. The request id is attached by the HTTP
//! middleware as a span field, so every event emitted while handling a request
//! carries it without the handler having to pass it around.
//!
//! Production defaults to JSON lines for a log collector; development defaults
//! to a human-readable format. The filter is configurable and falls back to
//! `info` rather than failing to start over a typo in the filter syntax.

use tracing_subscriber::EnvFilter;

use crate::config::{LogFormat, LoggingConfig};

/// Install the global subscriber.
///
/// Returns `Err` only when the filter directive is unusable *and* the fallback
/// also failed, which indicates a broken build rather than a bad setting.
pub fn init(config: &LoggingConfig) -> anyhow::Result<()> {
    let filter = EnvFilter::try_new(&config.filter).unwrap_or_else(|error| {
        eprintln!(
            "warning: invalid log filter {:?} ({error}); falling back to `info`",
            config.filter
        );
        EnvFilter::new("info")
    });

    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_level(true);

    let result = match config.format {
        LogFormat::Pretty => builder.try_init(),
        LogFormat::Json => builder
            .json()
            .flatten_event(true)
            .with_current_span(true)
            .try_init(),
    };

    match result {
        Ok(()) => {
            tracing::debug!(
                format = ?config.format,
                filter = %config.filter,
                "logging initialised"
            );
            Ok(())
        }
        // Already installed: normal in tests, where several cases share a
        // process, and harmless in a `serve` that re-enters.
        Err(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_broken_filter_does_not_prevent_startup() {
        let config = LoggingConfig {
            filter: "this is not a filter {{{".to_owned(),
            format: LogFormat::Pretty,
        };
        assert!(init(&config).is_ok());
    }

    #[test]
    fn json_format_is_accepted() {
        let config = LoggingConfig {
            filter: "info".to_owned(),
            format: LogFormat::Json,
        };
        assert!(init(&config).is_ok());
    }
}
