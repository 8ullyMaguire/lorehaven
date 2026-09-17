//! Jobs: the state machine, the retry policy and the cancellation rule
//! (spec §10.1, §10.2).
//!
//! Spec §10.1 gives the machine:
//!
//! ```text
//! queued → running → succeeded
//! running → retry_wait → queued
//! running → failed
//! queued/running → canceled
//! ```
//!
//! `retry_wait` is *not a state* here: a job waiting to be retried is `Queued`
//! with an `available_at` in the future, so one indexed query answers "what runs
//! next" and a worker that dies between two statements cannot leave a job in a
//! state nothing claims. The state names are the closed set the `jobs.state`
//! column stores.
//!
//! Everything in this module is pure — no database, no clock, no randomness that
//! is not derived from an argument — so the rules can be tested without a
//! server, and the worker and the repository cannot each grow their own version
//! of them.

use std::time::Duration;

use time::OffsetDateTime;

/// Where a job is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobState {
    /// Waiting for a worker. `available_at` may be in the future (a retry).
    Queued,
    /// A worker has claimed it and holds a lease.
    Leased,
    /// The worker has begun doing units of work.
    Running,
    /// Finished, and the outcome was success.
    Succeeded,
    /// Finished, and the outcome was a terminal failure.
    Failed,
    /// Stopped by the person who asked for it, at a checkpoint.
    Cancelled,
}

impl JobState {
    /// The wire and column representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Leased => "leased",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Parse the stored representation. Unknown values are an error rather than
    /// a default, because a job in a state this build does not understand must
    /// be visible, not silently queued.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "queued" => Self::Queued,
            "leased" => Self::Leased,
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }

    /// Whether the job is finished, one way or another.
    ///
    /// A terminal job is never claimed again, never retried, and never
    /// cancelled: `can_cancel` and the claim query both read this.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

/// What a job is for. The set is closed: a new kind is a code change, because
/// the worker needs a handler for it and a silently unhandled kind would sit in
/// the queue forever.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobKind {
    /// Fetch a work from a source and store it (M6).
    Import,
    /// Render a work into a downloadable file (M7).
    Export,
    /// Rebuild a work's search document (M9).
    Reindex,
    /// Deliver an outbox event (a notification, a webhook).
    Notify,
    /// Render a thumbnail for a cover or avatar.
    Thumbnail,
    /// Retention and collection: purge old jobs, collect unreferenced blobs.
    Maintenance,
    /// Check the reader's library items against their sources (M8).
    ///
    /// A job rather than a request because it reads the network once per item
    /// and a reader with a hundred imports should not hold a connection open
    /// while it does: spec §14.1 asks for a check, and `POST
    /// /library/updates/check` answers `202` with the job.
    UpdateCheck,
    /// Build a derivative rendition (EPUB/PDF/text) or extract OCR from a
    /// scanned upload (M25 / spec §32.4).
    Derivative,
    /// Produce or regenerate a TTS narration edition (M26 / spec §32.5).
    ///
    /// Author-approved machine narration. The worker invokes the configured
    /// AI provider's TTS capability; the output is stored as a derivative
    /// audio blob and a `narration` edition is created with the machine
    /// producer labeled per §22.6/§30.8.
    Narration,
}

impl JobKind {
    /// The wire and column representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Import => "import",
            Self::Export => "export",
            Self::Reindex => "reindex",
            Self::Notify => "notify",
            Self::Thumbnail => "thumbnail",
            Self::Maintenance => "maintenance",
            Self::UpdateCheck => "update_check",
            Self::Derivative => "derivative",
            Self::Narration => "narration",
        }
    }

    /// Parse the stored representation.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "import" => Self::Import,
            "export" => Self::Export,
            "reindex" => Self::Reindex,
            "notify" => Self::Notify,
            "thumbnail" => Self::Thumbnail,
            "maintenance" => Self::Maintenance,
            "update_check" => Self::UpdateCheck,
            "derivative" => Self::Derivative,
            "narration" => Self::Narration,
            _ => return None,
        })
    }
}

/// How one attempt ended, as the worker reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptOutcome {
    /// The job is done.
    Succeeded,
    /// The attempt failed and the job may be tried again.
    Failed,
    /// The attempt failed in a way that retrying cannot fix (a source that has
    /// gone, a payload this build cannot parse).
    Fatal,
    /// The job was cancelled, and the worker noticed between units of work.
    Cancelled,
}

/// How many attempts, how long between them, and how much the wait grows.
///
/// `jitter_permille` is the *extra* wait, in thousandths, that a retry may add
/// on top of the computed delay. It can only make a retry later, never earlier:
/// jitter exists to spread a stampede of retries, and a jitter that could move a
/// retry *forward* would let a failing dependency be hit sooner than the policy
/// allows, which makes the stampede worse rather than better.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetryPolicy {
    /// Attempts allowed before the job fails terminally.
    pub max_attempts: u32,
    /// The wait before the second attempt.
    pub base_delay: Duration,
    /// The multiplier applied per further attempt.
    pub backoff: f64,
    /// The most extra wait jitter may add, in thousandths of the delay.
    pub jitter_permille: u16,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay: Duration::from_secs(30),
            backoff: 2.0,
            jitter_permille: 250,
        }
    }
}

impl RetryPolicy {
    /// The delay before attempt `attempt` (1-based: attempt 1 is the first
    /// retry, which waits `base_delay`).
    #[must_use]
    pub fn delay_before(&self, attempt: u32) -> Duration {
        if attempt <= 1 {
            return self.base_delay;
        }
        let exponent = i32::try_from(attempt - 1).unwrap_or(i32::MAX);
        let factor = self.backoff.powi(exponent);
        let seconds = self.base_delay.as_secs_f64() * factor;
        if !seconds.is_finite() || seconds <= 0.0 {
            return self.base_delay;
        }
        // A day is longer than any delay this platform needs, and capping here
        // keeps an absurd `attempt` from producing a date that overflows.
        Duration::from_secs_f64(seconds.min(86_400.0))
    }
}

/// The moment a failed attempt may be retried. Pure: `now` is passed in, and
/// the jitter is derived from `jitter_seed` rather than from a random source, so
/// the answer is reproducible in a test.
#[must_use]
pub fn next_attempt_at(
    attempt: u32,
    policy: &RetryPolicy,
    jitter_seed: u64,
    now: OffsetDateTime,
) -> OffsetDateTime {
    let delay = policy.delay_before(attempt);
    let spread = u64::from(policy.jitter_permille);
    // A small integer hash of the seed: deterministic, and spread across the
    // whole jitter window rather than clustered.
    let mut mixed = jitter_seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    mixed ^= mixed >> 29;
    let fraction = if spread == 0 { 0 } else { mixed % (spread + 1) };
    let extra = delay.as_millis() * u128::from(fraction) / 1000;
    let extra = Duration::from_millis(u64::try_from(extra).unwrap_or(u64::MAX));
    now + delay + extra
}

/// Whether a job may be stopped where it is.
///
/// A finished job may not be cancelled: it has already produced its outcome, and
/// reporting success would be a lie about what the worker did.
#[must_use]
pub const fn can_cancel(state: JobState) -> bool {
    !state.is_terminal()
}

/// The state a job moves to when a worker reports an outcome.
///
/// A cancelled job stays cancelled: an attempt that finishes after the reader
/// cancelled it does not un-cancel it. Whether a *failed* attempt becomes a
/// retry or a terminal failure is the repository's decision, because it is the
/// one that knows the attempt count against the policy.
#[must_use]
pub const fn next_state(state: JobState, outcome: AttemptOutcome) -> JobState {
    if matches!(state, JobState::Cancelled) {
        return JobState::Cancelled;
    }
    match outcome {
        AttemptOutcome::Succeeded => JobState::Succeeded,
        AttemptOutcome::Failed => JobState::Queued,
        AttemptOutcome::Fatal => JobState::Failed,
        AttemptOutcome::Cancelled => JobState::Cancelled,
    }
}

/// Whether another attempt is allowed after `attempts` attempts have failed.
#[must_use]
pub const fn may_retry(attempts: u32, policy: &RetryPolicy) -> bool {
    attempts < policy.max_attempts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_700_000_000).expect("a valid instant")
    }

    #[test]
    fn a_first_failure_waits_the_base_delay() {
        let policy = RetryPolicy {
            jitter_permille: 0,
            ..RetryPolicy::default()
        };
        let next = next_attempt_at(1, &policy, 0, now());
        assert_eq!(next - now(), time::Duration::seconds(30));
    }

    #[test]
    fn each_retry_waits_longer() {
        let policy = RetryPolicy {
            jitter_permille: 0,
            ..RetryPolicy::default()
        };
        let first = next_attempt_at(1, &policy, 0, now()) - now();
        let second = next_attempt_at(2, &policy, 0, now()) - now();
        let third = next_attempt_at(3, &policy, 0, now()) - now();
        assert!(second > first, "{second:?} must exceed {first:?}");
        assert!(third > second, "{third:?} must exceed {second:?}");
        assert_eq!(second, time::Duration::seconds(60));
        assert_eq!(third, time::Duration::seconds(120));
    }

    #[test]
    fn a_retry_is_never_sooner_than_the_base_delay() {
        let policy = RetryPolicy {
            jitter_permille: 400,
            ..RetryPolicy::default()
        };
        // Every seed must land in [base, base + 40%]: jitter may delay a retry
        // and may never bring it forward, or a stampede of failures retries
        // sooner than the policy allows and makes the stampede worse.
        for seed in 0..2_000u64 {
            let delay = next_attempt_at(1, &policy, seed, now()) - now();
            assert!(
                delay >= time::Duration::seconds(30),
                "seed {seed} produced {delay:?}"
            );
            assert!(
                delay <= time::Duration::seconds(30) + time::Duration::milliseconds(12_000),
                "seed {seed} produced {delay:?}"
            );
        }
    }

    #[test]
    fn jitter_actually_spreads_the_wait() {
        let policy = RetryPolicy {
            jitter_permille: 400,
            ..RetryPolicy::default()
        };
        let a = next_attempt_at(1, &policy, 1, now());
        let b = next_attempt_at(1, &policy, 2, now());
        assert_ne!(a, b, "jitter that never varies is not jitter");
    }

    #[test]
    fn a_finished_job_cannot_be_cancelled() {
        assert!(can_cancel(JobState::Queued));
        assert!(can_cancel(JobState::Leased));
        assert!(can_cancel(JobState::Running));
        assert!(!can_cancel(JobState::Succeeded));
        assert!(!can_cancel(JobState::Failed));
        assert!(!can_cancel(JobState::Cancelled));
    }

    #[test]
    fn an_outcome_moves_the_job_the_way_the_spec_says() {
        assert_eq!(
            next_state(JobState::Running, AttemptOutcome::Succeeded),
            JobState::Succeeded
        );
        assert_eq!(
            next_state(JobState::Running, AttemptOutcome::Failed),
            JobState::Queued
        );
        assert_eq!(
            next_state(JobState::Running, AttemptOutcome::Fatal),
            JobState::Failed
        );
        assert_eq!(
            next_state(JobState::Running, AttemptOutcome::Cancelled),
            JobState::Cancelled
        );
        // A cancelled job is not resurrected by an attempt that finished late.
        assert_eq!(
            next_state(JobState::Cancelled, AttemptOutcome::Succeeded),
            JobState::Cancelled
        );
    }

    #[test]
    fn attempts_stop_at_the_policy_maximum() {
        let policy = RetryPolicy {
            max_attempts: 3,
            ..RetryPolicy::default()
        };
        assert!(may_retry(0, &policy));
        assert!(may_retry(2, &policy));
        assert!(!may_retry(3, &policy));
    }

    #[test]
    fn states_and_kinds_round_trip_through_their_columns() {
        for state in [
            JobState::Queued,
            JobState::Leased,
            JobState::Running,
            JobState::Succeeded,
            JobState::Failed,
            JobState::Cancelled,
        ] {
            assert_eq!(JobState::parse(state.as_str()), Some(state));
        }
        for kind in [
            JobKind::Import,
            JobKind::Export,
            JobKind::Reindex,
            JobKind::Notify,
            JobKind::Thumbnail,
            JobKind::Maintenance,
        ] {
            assert_eq!(JobKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(JobState::parse("running_away"), None);
        assert_eq!(JobKind::parse("laundry"), None);
    }
}
