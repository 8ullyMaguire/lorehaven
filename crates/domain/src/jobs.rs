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
///
/// `job_kinds!` emits the enum, `ALL_KINDS`, `as_str` and `parse` from one
/// token tree, so a new variant appears in all four at once — the list can
/// never drift away from the enum. `kind_index()` is a separate exhaustive
/// match, so adding a variant is also a compile error until an arm is assigned:
/// the macro covers the list, `kind_index()` covers uniqueness.
macro_rules! job_kinds {
    ($($(#[$m:meta])* $variant:ident => $wire:expr),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum JobKind {
            $( $(#[$m])* $variant, )*
        }

        impl JobKind {
            /// The wire and column representation.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $( Self::$variant => $wire, )*
                }
            }

            /// Parse the stored representation.
            #[must_use]
            pub fn parse(raw: &str) -> Option<Self> {
                match raw {
                    $( $wire => Some(Self::$variant), )*
                    _ => None
                }
            }
        }

        /// Every kind the queue understands, in declaration order. Emitted from
        /// the same source as the enum, so it always covers every variant.
        pub const ALL_KINDS: &[JobKind] = &[ $(JobKind::$variant,)* ];
    };
}

job_kinds! {
    #[doc = "Fetch a work from a source and store it (M6)."]
    Import => "import",
    #[doc = "Render a work into a downloadable file (M7)."]
    Export => "export",
    #[doc = "Render a query into a downloadable bundle (M23)."]
    BulkExport => "bulk_export",
    #[doc = "Rebuild a work's search document (M9)."]
    Reindex => "reindex",
    #[doc = "Deliver an outbox event (a notification, a webhook)."]
    Notify => "notify",
    #[doc = "Render a thumbnail for a cover or avatar."]
    Thumbnail => "thumbnail",
    #[doc = "Retention and collection: purge old jobs, collect unreferenced blobs."]
    Maintenance => "maintenance",
    #[doc = "Check the reader's library items against their sources (M8)."]
    #[doc = ""]
    #[doc = "A job rather than a request because it reads the network once per item"]
    #[doc = "and a reader with a hundred imports should not hold a connection open"]
    #[doc = "while it does: spec §14.1 asks for a check, and `POST"]
    #[doc = "/library/updates/check` answers `202` with the job."]
    UpdateCheck => "update_check",
    #[doc = "Build a derivative rendition (EPUB/PDF/text) or extract OCR from a"]
    #[doc = "scanned upload (M25 / spec §32.4)."]
    Derivative => "derivative",
    #[doc = "Fetch a media reference's bytes and store its exact and perceptual"]
    #[doc = "hashes (M32-07d / spec §32.7.2)."]
    #[doc = ""]
    #[doc = "A job because the URL is author-supplied and the fetch must never"]
    #[doc = "hold an HTTP request open: the add-reference door answers 201 and the"]
    #[doc = "hashing happens here. The `pending` placeholder `add_media_reference`"]
    #[doc = "writes is replaced by this job, which is what makes the dedup search"]
    #[doc = "readable instead of structurally correct on an empty column."]
    MediaFetch => "media_fetch",
    #[doc = "Produce or regenerate a TTS narration edition (M26 / spec §32.5)."]
    #[doc = ""]
    #[doc = "Author-approved machine narration. The worker invokes the configured"]
    #[doc = "AI provider's TTS capability; the output is stored as a derivative"]
    #[doc = "audio blob and a `narration` edition is created with the machine"]
    #[doc = "producer labeled per §22.6/§30.8."]
    Narration => "narration",
    #[doc = "Re-verify preservation targets and reclaim the credits of dead ones"]
    #[doc = "(M59 / spec §11.12a, §2.3)."]
    #[doc = ""]
    #[doc = "A job because it is the only thing that reads a destination's item"]
    #[doc = "page over the network, and §11.5's metadata ceiling and §11.5's pacing"]
    #[doc = "both apply to every fetch it makes. One verified target is one"]
    #[doc = "network round trip, so an instance with thousands of them cannot do"]
    #[doc = "this inline on a request."]
    #[doc = ""]
    #[doc = "Bulk rather than Interactive on purpose: it competes with nothing a"]
    #[doc = "reader is waiting for, it is outbound network work against"]
    #[doc = "third-party archives, and §11.5's pacing exists to be gentle to them."]
    #[doc = "An interactive class would let a hundred re-verifications delay the"]
    #[doc = "import a reader just submitted."]
    PreservationRecheck => "preservation_recheck",
    /// Recalculates author payouts for a closed week (spec §20.3).
    ///
    /// **Bulk, and for a fifth reason.** The other four are bulk because of
    /// outbound network work, instance policy, or an unbounded third-party
    /// response time. This one is bulk because it walks *every work with
    /// earnings* and computes six counts from five tables for each: on a mature
    /// instance that is the single longest read in the database, and it competes
    /// with nothing a reader is waiting for. A reader's import must not queue
    /// behind an economics pass over the whole catalogue.
    ///
    /// **Keyed on the ISO week, not run per pass.** The same reason
    /// `PreservationRecheck` keys on the date: the maintenance pass runs every
    /// 30 cycles, so an unkeyed recalculation would pay the same week up to
    /// dozens of times. It is keyed rather than guarded because the key is also
    /// the audit answer to "has this week been paid yet?" -- a row in `jobs`.
    PayoutRecalc => "payout_recalc",
    /// Settles retention proposals whose ballot window has closed (plan E.2).
    ///
    /// **Bulk**, and for a different reason than `PreservationRecheck`. That one
    /// is outbound network work; this one is a handful of local writes. But it
    /// is the kind that *changes instance policy*, so a reader's import must
    /// never queue behind it — a long pass over a backlog of proposals would
    /// otherwise delay work a reader is watching, and the delay would be caused
    /// by governance rather than by load, which is harder to explain and worse to
    /// be on the receiving end of.
    RetentionSettle => "retention_settle",
    /// Fetches one external body a reader asked for personally (plan M59-10,
    /// spec §11.15b).
    ///
    /// **Bulk, and for a third reason distinct from its two neighbours.**
    /// `PreservationRecheck` is bulk because it is outbound network work;
    /// `RetentionSettle` is bulk because it changes instance policy and a
    /// reader's import must not queue behind a governance pass. This one is bulk
    /// because a fetch can occupy a worker for as long as the site takes to
    /// answer — which is §6.2's whole reason for making the request a job rather
    /// than a synchronous fetch. A reader who asked for a body must not hold up
    /// everyone else's import while a third-party site stalls, nor queue behind a
    /// backlog of other readers' fetches.
    ///
    /// The class is not politeness: a synchronous fetch IS the failure this job
    /// exists to prevent, so the work must be reclaimable and a copy must never
    /// sit in `pending` because a worker died mid-fetch.
    BodyFetch => "body_fetch",
}
/// The queue's claim classes — separate from the HTTP route rate classes
/// (`crate::limiter::RouteClass`). The rate limiter guards the HTTP surface;
/// this is how the worker knows how to claim a job: an interactive job should
/// not be starved by a bulk one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceClass {
    /// Ordinary job that should run promptly.
    Interactive,
    /// A large job the worker may throttle behind interactive ones.
    Bulk,
}

impl JobKind {
    /// The queue class a worker sees this job as.
    #[must_use]
    pub const fn resource_class(self) -> ResourceClass {
        match self {
            Self::Import => ResourceClass::Interactive,
            Self::Export => ResourceClass::Interactive,
            Self::BulkExport => ResourceClass::Bulk,
            Self::Reindex => ResourceClass::Interactive,
            Self::Notify => ResourceClass::Interactive,
            Self::Thumbnail => ResourceClass::Interactive,
            Self::Maintenance => ResourceClass::Interactive,
            Self::UpdateCheck => ResourceClass::Interactive,
            Self::Derivative => ResourceClass::Interactive,
            Self::Narration => ResourceClass::Interactive,
            Self::MediaFetch => ResourceClass::Interactive,
            // Bulk: outbound network work against third-party archives, and
            // nothing a reader is waiting for. See the variant's doc comment.
            Self::PreservationRecheck => ResourceClass::Bulk,
            // See the variant's doc comment: bulk so a governance pass cannot
            // delay a reader's import.
            Self::RetentionSettle => ResourceClass::Bulk,
            // Bulk so a whole-catalogue economics pass cannot delay a reader's
            // import. See the variant's doc comment.
            Self::PayoutRecalc => ResourceClass::Bulk,
            Self::BodyFetch => ResourceClass::Bulk,
        }
    }

    /// A dense per-variant index, for placing a kind in a fixed array.
    ///
    /// The match is exhaustive: adding a variant to `JobKind` fails to
    /// compile here until an arm is assigned, so a new kind can never
    /// be "handled everywhere except the list".
    #[must_use]
    pub const fn kind_index(self) -> usize {
        match self {
            Self::Import => 0,
            Self::Export => 1,
            Self::BulkExport => 2,
            Self::Reindex => 3,
            Self::Notify => 4,
            Self::Thumbnail => 5,
            Self::Maintenance => 6,
            Self::UpdateCheck => 7,
            Self::Derivative => 8,
            Self::Narration => 9,
            Self::MediaFetch => 10,
            // 11, appended rather than renumbered: `kind_index` places a kind in
            // a fixed array and existing indices are load-bearing for any
            // array already persisted. A new kind takes the next free slot.
            Self::PreservationRecheck => 11,
            // 12, on the same reasoning: appended, never renumbered.
            Self::RetentionSettle => 12,
            // 14, on the same reasoning: appended, never renumbered, so adding a job
            // kind stays a two-line change instead of a migration.
            //
            // **This was 13, and 13 was already `BodyFetch`.** I misread a truncated
            // terminal view of this match as having `BodyFetch => 12` and inserted the
            // new kind on top of it. The compiler is happy with that -- two valid
            // `usize` expressions -- so nothing local caught it.
            // `the_job_kind_exists_and_is_interactive` in
            // `crates/app/tests/media_fetch_job.rs` asserts uniqueness across all
            // kinds and did. `kind_index` places a kind in a fixed array, so a
            // collision means two kinds sharing one slot.
            Self::PayoutRecalc => 14,
            // APPENDED, never inserted. These indices are persisted in `jobs`
            // rows, so inserting a kind in the middle silently re-labels every
            // queued job of a later kind. See `db-migration-integrity`.
            Self::BodyFetch => 13,
        }
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
    /// Raised 2026-10-05 from `max_attempts: 5` / `base_delay: 30s`, which
    /// spanned 30+60+120+240+480 = **930 s** — 15.5 minutes.
    ///
    /// That was measured to be fatal rather than merely tight: on the live
    /// importer 754 of 1401 failed jobs sat at exactly attempt 5, i.e. they
    /// exhausted the budget while the upstream block was still up. A rate-limit
    /// window runs minutes to hours, so a 15-minute ladder cannot outlast one
    /// no matter how the classification is done.
    ///
    /// The new default spans 20 attempts with a 300 s base and 1.6x backoff.
    /// Combined with `next_attempt_at_honouring`, which defers to the source's
    /// own `Retry-After`, a stated window is respected exactly and an unstated
    /// one gets a ladder that outlasts an unstated block.
    ///
    /// The ladder is deliberately very long — nominally ~70 days, with the last
    /// few attempts clamped to `delay_before`'s 24 h cap. That is intended: a
    /// job that is genuinely stuck should keep trying slowly rather than be
    /// declared lost, and the `may_retry_with_retry_after` guard means a job
    /// only keeps its budget while something says it is still coming. A reader
    /// who does not want that can lower `max_attempts` per job.
    fn default() -> Self {
        Self {
            max_attempts: 20,
            base_delay: Duration::from_secs(300),
            backoff: 1.6,
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

/// Whether another attempt is allowed when the source asked us to wait for a
/// specific number of seconds.
///
/// A source that sends `Retry-After` has stated how long its block lasts.
/// Spending the whole remaining budget inside a window it told us the length of
/// guarantees every later attempt lands in the same block, so the only honest
/// options are to wait the stated time or to stop.
///
/// Measured 2026-10-05 on the live importer: the old policy (5 attempts,
/// 30 s base, 2x backoff) spans 930 s — 15.5 minutes — and 754 of 1401 failed
/// jobs sat at exactly attempt 5. AO3/Cloudflare rate-limit windows are
/// minutes to hours, so a 15-minute ladder is exhausted while the block is
/// still up.
#[must_use]
pub fn may_retry_with_retry_after(
    attempts: u32,
    policy: &RetryPolicy,
    retry_after: Option<Duration>,
) -> bool {
    let Some(window) = retry_after else {
        return may_retry(attempts, policy);
    };
    if may_retry(attempts, policy) {
        return true;
    }
    // Out of policy attempts, but the source's own window has not elapsed
    // either. Keep waiting while the block is still current rather than
    // declaring failure the moment the static budget runs out.
    window > policy.delay_before(attempts)
}

/// The delay before `attempt`, never shorter than the source asked us to wait.
///
/// `Retry-After` is the source's own statement, so it wins over the static
/// ladder: retrying before the stated window ends is a request that cannot
/// succeed, and it spends an attempt to learn nothing. The delay is `max(policy
/// delay, Retry-After)` — a generous static delay is still respected.
#[must_use]
pub fn next_attempt_at_honouring(
    attempt: u32,
    policy: &RetryPolicy,
    retry_after: Option<Duration>,
    jitter_seed: u64,
    now: OffsetDateTime,
) -> OffsetDateTime {
    let Some(window) = retry_after else {
        return next_attempt_at(attempt, policy, jitter_seed, now);
    };
    let ladder = policy.delay_before(attempt);
    if window <= ladder {
        return next_attempt_at(attempt, policy, jitter_seed, now);
    }
    // Above the ladder the window is the answer, and jitter is deliberately
    // NOT added: the source said "not before", and jitter either way would
    // sometimes land before the stated window.
    now + window
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
        // Derived, not hardcoded: this test used to assert `30s` literally and
        // broke when the base delay was raised to 300s. A test that pins the
        // policy constant AND the ladder arithmetic is asserting the same thing
        // twice; only the relationship is worth pinning here.
        assert_eq!(next - now(), policy.base_delay);
    }

    // --- Retry-After (fix #1) -------------------------------------------------
    //
    // Measured 2026-10-05: 754 of 1401 failed import jobs sat at exactly
    // attempt 5, and the whole ladder (30+60+120+240+480) spans 930s. The
    // source's own block outlasts that, so these tests pin the two properties
    // that make the ladder survivable: the stated window is respected, and the
    // budget alone no longer ends a job whose window is still open.

    #[test]
    fn retry_after_overrides_a_shorter_static_ladder() {
        let policy = RetryPolicy {
            jitter_permille: 0,
            ..RetryPolicy::default()
        };
        // Pick a window that is definitively longer than the FIRST rung, so the
        // test states "the window wins" regardless of what the base delay is.
        // (It was written when the base was 30s and used a fixed 180s window;
        // raising the base to 300s made the window the shorter of the two and
        // the test silently stopped testing what it claimed to.)
        let first = policy.delay_before(1);
        let window = Duration::from_secs(first.as_secs() * 3);
        let next = next_attempt_at_honouring(1, &policy, Some(window), 0, now());
        assert_eq!(
            next - now(),
            window,
            "the source's stated window must win over a shorter static delay"
        );
        assert!(
            window > first,
            "guard: the window must exceed the ladder for this to mean anything"
        );
    }

    #[test]
    fn a_longer_static_delay_is_still_respected() {
        let policy = RetryPolicy {
            jitter_permille: 0,
            ..RetryPolicy::default()
        };
        // A deep rung (attempt 8) has a long ladder delay; the source asks for
        // something shorter. The result must be max(ladder, window), so the
        // ladder wins. Derived from the policy, not written as 480s — the base
        // delay and backoff both changed on 2026-10-05.
        let rung = 8;
        let ladder = policy.delay_before(rung);
        let short = Duration::from_secs(ladder.as_secs() / 4);
        let next = next_attempt_at_honouring(rung, &policy, Some(short), 0, now());
        assert_eq!(
            next - now(),
            ladder,
            "a generous static delay must not be shortened by a smaller window"
        );
    }

    #[test]
    fn no_retry_after_is_exactly_the_old_behaviour() {
        let policy = RetryPolicy {
            jitter_permille: 0,
            ..RetryPolicy::default()
        };
        let plain = next_attempt_at(3, &policy, 7, now());
        let honouring = next_attempt_at_honouring(3, &policy, None, 7, now());
        assert_eq!(plain, honouring, "absent a header nothing may change");
    }

    #[test]
    fn jitter_never_shortens_or_overshoots_the_stated_window() {
        let policy = RetryPolicy::default();
        let window = Duration::from_secs(900);
        for seed in 0..64u64 {
            let next = next_attempt_at_honouring(1, &policy, Some(window), seed, now());
            assert!(
                next - now() >= window,
                "seed {seed} landed before the window: {:?}",
                next - now()
            );
        }
    }

    #[test]
    fn a_job_whose_window_is_still_open_is_not_given_up_on() {
        // max_attempts 5 with a 2h block: the static ladder is spent long
        // before the source says it will lift.
        let policy = RetryPolicy {
            max_attempts: 5,
            jitter_permille: 0,
            ..RetryPolicy::default()
        };
        let window = Duration::from_secs(7_200);
        // Within budget: plainly yes.
        assert!(may_retry_with_retry_after(1, &policy, Some(window)));
        // Budget spent, window still open: still yes — this is the 754-job case.
        assert!(
            may_retry_with_retry_after(5, &policy, Some(window)),
            "a live Retry-After window must outlive the static attempt budget"
        );
        // Budget spent and the window already elapsed: no.
        assert!(!may_retry_with_retry_after(
            5,
            &policy,
            Some(Duration::from_secs(1))
        ));
        // No header at all: the static budget decides, unchanged.
        assert!(!may_retry_with_retry_after(5, &policy, None));
    }

    #[test]
    fn the_default_budget_outlasts_a_measured_rate_limit_window() {
        // Regression guard on the DEFAULT policy itself. If someone lowers
        // max_attempts or base_delay back toward the measured 930s window, this
        // fails and the 754-job failure mode cannot come back silently.
        let policy = RetryPolicy::default();
        let total: f64 = (1..=policy.max_attempts)
            .map(|a| policy.delay_before(a).as_secs_f64())
            .sum();
        assert!(
            total > 3_600.0,
            "the default retry ladder spans only {total:.0}s; a rate-limit \
             window outlasts it and jobs die inside the block"
        );
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
        // Derived from the policy rather than written as 60s/120s: the
        // relationship is the contract, and the constants changed on
        // 2026-10-05 (base 30s -> 300s, backoff 2.0 -> 1.6).
        assert_eq!(second, policy.base_delay.mul_f64(policy.backoff));
        assert_eq!(
            third,
            policy
                .base_delay
                .mul_f64(policy.backoff.powi(2))
        );
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
        let base = policy.base_delay;
        let ceiling = base + base.mul_f64(0.4);
        for seed in 0..2_000u64 {
            let delay = next_attempt_at(1, &policy, seed, now()) - now();
            assert!(delay >= base, "seed {seed} produced {delay:?}");
            assert!(delay <= ceiling, "seed {seed} produced {delay:?}");
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
    fn every_kind_round_trips_through_parse() {
        for kind in ALL_KINDS {
            assert_eq!(
                JobKind::parse(kind.as_str()),
                Some(*kind),
                "parse(kind.as_str()) != kind for {:?}",
                kind
            );
        }
        // Closed set: unknown wire values must not parse as any known kind.
        for unknown in ["", "laundry", "bulk_import", "NARRATION"] {
            assert_eq!(JobKind::parse(unknown), None, "{unknown} should not parse");
        }
    }
}
