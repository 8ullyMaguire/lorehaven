//! The background worker: the job loop and outbox delivery (spec §10.2).
//!
//! What this module is responsible for, in the words of the acceptance
//! criteria:
//!
//! * **A worker death does not lose jobs.** Every claim takes a lease with an
//!   expiry, and [`Worker::maintenance_pass`] returns expired leases to the
//!   queue. Nothing here holds a database transaction across I/O, so a killed
//!   process leaves at most one leased job, which the next sweep picks up.
//! * **Cancellation is checked between units of work.** A handler asks
//!   [`lorehaven_db::jobs::is_cancelled`] before each unit and stops, leaving its
//!   checkpoint behind — a cancel that only took effect at the start of a ten
//!   minute job would be a lie.
//! * **An unknown kind fails loudly.** A job whose kind this build has no
//!   handler for is failed fatally with a message naming the kind, rather than
//!   being marked succeeded by a handler that did nothing.
//! * **The outbox is drained, and an event is deleted only after its handler
//!   returns success.** A topic with no handler yet is *deferred*, not
//!   delivered: nothing consumes `publish.index` until Milestone 9 builds the
//!   index, and marking it delivered now would be a claim that an index was
//!   updated.
//!
//! The worker is the second entry point into every table the web path writes,
//! so everything it does goes through the same repository functions.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use lorehaven_db::storage::BlobStore;
use lorehaven_db::{jobs, outbox};
use lorehaven_domain::jobs::{JobKind, JobState, RetryPolicy};
use lorehaven_domain::JobId;
use time::OffsetDateTime;

use crate::state::AppState;

/// How long a terminal job is kept before the maintenance sweep removes it.
const TERMINAL_JOB_RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// A future the worker awaits; boxed because the handler registry is a map.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// What an outbox topic's delivery does.
pub type TopicHandler =
    Arc<dyn Fn(&AppState, &outbox::OutboxEvent) -> BoxFuture<Result<()>> + Send + Sync>;

/// How the worker behaves.
#[derive(Debug, Clone)]
pub struct WorkerOptions {
    /// This worker's identity, recorded on every lease and attempt. Two workers
    /// must never share one, or a lost lease cannot be told apart from a stolen
    /// one.
    pub id: String,
    /// How long a claim is held before it may be reclaimed.
    pub lease: Duration,
    /// How long to wait after finding nothing to do.
    pub poll_interval: Duration,
    /// Attempts, backoff and jitter for a failed job.
    pub policy: RetryPolicy,
    /// How many outbox events to deliver per pass.
    pub batch: i64,
}

impl Default for WorkerOptions {
    fn default() -> Self {
        Self {
            id: format!("worker-{}", &uuid::Uuid::new_v4().to_string()[..8]),
            lease: Duration::from_secs(120),
            poll_interval: Duration::from_secs(1),
            policy: RetryPolicy::default(),
            batch: 50,
        }
    }
}

impl WorkerOptions {
    /// Options for a worker with a stable name.
    #[must_use]
    pub fn named(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            ..Self::default()
        }
    }
}

/// Why a handler stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandlerError {
    /// The attempt failed and may be tried again.
    Transient(String),
    /// The attempt failed in a way retrying cannot fix.
    Fatal(String),
    /// The job was cancelled, and the handler noticed between units of work.
    Cancelled,
}

impl HandlerError {
    /// The message recorded against the attempt.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Transient(message) | Self::Fatal(message) => message.clone(),
            Self::Cancelled => "cancelled".to_owned(),
        }
    }
}

/// What one pass did, so `--once` and a test can both see it.
#[derive(Debug, Default, Clone)]
pub struct PassReport {
    /// Outbox events whose handler returned success.
    pub outbox_delivered: u64,
    /// Outbox events left in place because nothing handles their topic yet.
    pub outbox_deferred: u64,
    /// Outbox events whose handler failed, and which will be retried.
    pub outbox_failed: u64,
    /// The job this pass ran, if any, and the state it ended in.
    pub job: Option<(JobId, JobState)>,
}

impl PassReport {
    /// Whether this pass had anything to do.
    #[must_use]
    pub fn did_something(&self) -> bool {
        self.job.is_some() || self.outbox_delivered > 0 || self.outbox_failed > 0
    }
}

struct Inner {
    options: WorkerOptions,
    handlers: HashMap<String, TopicHandler>,
}

/// The worker. Cloneable, because a unit of work runs on its own task so a
/// shutdown signal cannot drop it half-done.
#[derive(Clone)]
pub struct Worker {
    inner: Arc<Inner>,
}

impl Worker {
    /// A worker with no outbox handlers registered.
    ///
    /// That is the honest default for this milestone: the topics Milestone 3
    /// writes are delivered by the search index (M9), the notifier (M16) and the
    /// federation publisher (M16), and none of them exists yet. They are left in
    /// the outbox rather than marked delivered.
    #[must_use]
    pub fn new(options: WorkerOptions) -> Self {
        Self {
            inner: Arc::new(Inner {
                options,
                handlers: HashMap::new(),
            }),
        }
    }

    /// Register a topic's delivery. Milestone 9 and Milestone 16 call this; so do
    /// the acceptance tests, which is why it is public.
    #[must_use]
    pub fn with_topic(mut self, topic: impl Into<String>, handler: TopicHandler) -> Self {
        Arc::get_mut(&mut self.inner)
            .expect("a worker is built before it is shared")
            .handlers
            .insert(topic.into(), handler);
        self
    }

    /// The options this worker runs with.
    #[must_use]
    pub fn options(&self) -> &WorkerOptions {
        &self.inner.options
    }

    /// One unit of work: deliver what the outbox can, then run at most one job.
    ///
    /// This is the seam the tests and `worker --once` use. It never waits for
    /// more work to arrive.
    pub async fn run_once(&self, state: &AppState) -> Result<PassReport> {
        let mut report = PassReport::default();
        let (delivered, deferred, failed) = self.deliver_outbox(state).await?;
        report.outbox_delivered = delivered;
        report.outbox_deferred = deferred;
        report.outbox_failed = failed;

        let now = OffsetDateTime::now_utc();
        let claimed = jobs::claim_next(
            state.db(),
            &self.inner.options.id,
            self.inner.options.lease,
            now,
        )
        .await
        .context("claiming the next job")?;

        if let Some(job) = claimed {
            let id: JobId = job.id.parse().context("a job id is a UUID")?;
            let attempt = job.attempts + 1;
            jobs::attempt_started(state.db(), id, attempt, &self.inner.options.id).await?;
            let outcome = self.run_handler(state, &job).await;
            let state_after = self.record_outcome(state, id, outcome).await?;
            report.job = Some((id, state_after));
        }

        Ok(report)
    }

    /// Loop until `shutdown` completes.
    ///
    /// The unit in flight is finished before the loop stops: a job that has
    /// started an attempt is completed or failed rather than dropped with its
    /// lease held, and the lease is only left behind if the process is killed,
    /// which is what the expiry sweep is for.
    pub async fn run<F>(&self, state: &AppState, shutdown: F) -> Result<()>
    where
        F: Future<Output = ()> + Send,
    {
        tokio::pin!(shutdown);
        let mut passes: u64 = 0;
        let mut stop = false;

        while !stop {
            let worker = self.clone();
            let state_for_unit = state.clone();
            // The unit runs on its own task so that a shutdown arriving in the
            // middle of it cannot cancel the work: the select below observes the
            // signal, but the unit is awaited to its end first.
            let mut unit = tokio::spawn(async move { worker.run_once(&state_for_unit).await });

            let report = tokio::select! {
                biased;
                _ = &mut shutdown => {
                    stop = true;
                    unit.await.context("the unit of work panicked")?
                }
                finished = &mut unit => finished.context("the unit of work panicked")?,
            }?;

            passes += 1;
            if passes % 30 == 1 {
                self.maintenance_pass(state).await?;
            }
            if !report.did_something() {
                // Nothing to do: wait, but wake immediately on a shutdown.
                tokio::select! {
                    biased;
                    _ = &mut shutdown => stop = true,
                    _ = tokio::time::sleep(self.inner.options.poll_interval) => {}
                }
            }
        }

        tracing::info!(worker = %self.inner.options.id, passes, "worker stopped");
        Ok(())
    }

    /// The sweeps that keep the queue honest: expired leases back to the queue,
    /// and terminal jobs past their retention window deleted.
    pub async fn maintenance_pass(&self, state: &AppState) -> Result<()> {
        let now = OffsetDateTime::now_utc();
        let requeued = jobs::requeue_expired_leases(state.db(), now).await?;
        if requeued > 0 {
            tracing::warn!(requeued, "leases had expired; the jobs are queued again");
        }
        let purged = jobs::purge_terminal_jobs(state.db(), now - TERMINAL_JOB_RETENTION).await?;
        if purged > 0 {
            tracing::info!(
                purged,
                "terminal jobs past their retention window were deleted"
            );
        }
        Ok(())
    }

    /// Deliver what the outbox holds.
    ///
    /// An event is deleted only after its handler returns success. A topic with
    /// no handler is counted as deferred and left alone — the alternative is a
    /// claim that something was indexed, notified or federated when nothing was.
    async fn deliver_outbox(&self, state: &AppState) -> Result<(u64, u64, u64)> {
        let pending = outbox::pending(state.db(), self.inner.options.batch).await?;
        let mut delivered = 0;
        let mut deferred = 0;
        let mut failed = 0;

        for event in pending {
            let Some(handler) = self.inner.handlers.get(&event.topic) else {
                deferred += 1;
                continue;
            };
            let Ok(event_id) = event.id.parse::<lorehaven_domain::OutboxEventId>() else {
                tracing::error!(topic = %event.topic, "an outbox row has an id that is not a UUID");
                deferred += 1;
                continue;
            };
            match handler(state, &event).await {
                Ok(()) => {
                    outbox::mark_delivered(state.db(), event_id).await?;
                    delivered += 1;
                }
                Err(error) => {
                    // The event stays; `mark_failed` records why and pushes it
                    // out of the way for a while.
                    let message = error.to_string();
                    tracing::warn!(
                        topic = %event.topic,
                        attempts = event.attempts,
                        error = %message,
                        "an outbox event could not be delivered"
                    );
                    // Backoff is the event's own: the outbox row carries the
                    // attempt count, and the delay grows with it.
                    let retry_at = next_outbox_attempt(event.attempts, OffsetDateTime::now_utc());
                    outbox::mark_failed(state.db(), event_id, &message, &retry_at).await?;
                    failed += 1;
                }
            }
        }

        Ok((delivered, deferred, failed))
    }

    /// Run one job's handler and turn its failure into a recorded outcome.
    async fn record_outcome(
        &self,
        state: &AppState,
        job: JobId,
        outcome: Result<(), HandlerError>,
    ) -> Result<JobState> {
        let worker = self.inner.options.id.as_str();
        match outcome {
            Ok(()) => {
                if !jobs::complete(state.db(), job, worker).await? {
                    tracing::warn!(job = %job, "the lease was lost before the job could complete");
                }
                Ok(JobState::Succeeded)
            }
            Err(HandlerError::Cancelled) => {
                // The row already says cancelled: the cancel route wrote it.
                // The attempt is closed so the history does not show it running
                // forever.
                jobs::close_attempt(state.db(), job, worker, "cancelled", None).await?;
                Ok(JobState::Cancelled)
            }
            Err(HandlerError::Transient(message)) => {
                let next = jobs::fail(
                    state.db(),
                    job,
                    worker,
                    &message,
                    &self.inner.options.policy,
                    false,
                    OffsetDateTime::now_utc(),
                )
                .await?;
                tracing::warn!(job = %job, error = %message, state = next.as_str(), "job failed; retry scheduled or budget spent");
                Ok(next)
            }
            Err(HandlerError::Fatal(message)) => {
                let next = jobs::fail(
                    state.db(),
                    job,
                    worker,
                    &message,
                    &self.inner.options.policy,
                    true,
                    OffsetDateTime::now_utc(),
                )
                .await?;
                tracing::error!(job = %job, error = %message, "job failed terminally");
                Ok(next)
            }
        }
    }

    /// Dispatch one job to its handler.
    async fn run_handler(&self, state: &AppState, job: &jobs::Job) -> Result<(), HandlerError> {
        let id: JobId = job.id.parse().map_err(|_| {
            HandlerError::Fatal(format!("job {} has an id that is not a UUID", job.id))
        })?;
        let Some(kind) = job.kind() else {
            return Err(HandlerError::Fatal(format!(
                "job kind {:?} is not one this build knows",
                job.kind
            )));
        };

        match kind {
            JobKind::Maintenance => self.handle_maintenance(state, id, job).await,
            JobKind::Import => {
                let payload = serde_json::from_str(&job.payload).map_err(|error| {
                    HandlerError::Fatal(format!("the import job's payload is not JSON: {error}"))
                })?;
                let outcome = crate::imports::run(state, state.registry(), id, &payload).await;

                // Recompute the source's health from the import history, after
                // the attempt and whatever the attempt did (spec §11.8). The
                // sweep reads the history rather than this call's result, so it
                // cannot be skewed by one attempt — and a failure to sweep is
                // *not* reported as a failure of the import: the queue must act
                // on the import's own outcome, and a health row is not worth
                // five retries of a work that was fetched fine.
                if let Some(import_id) = payload
                    .get("import_job_id")
                    .and_then(serde_json::Value::as_str)
                {
                    if let Err(error) = crate::imports::settle_source_health(state, import_id).await
                    {
                        tracing::warn!(
                            import = import_id,
                            %error,
                            "could not recompute the source's health after an import"
                        );
                    }
                }
                outcome
            }
            JobKind::Notify => {
                let (_, _, failed) = self
                    .deliver_outbox(state)
                    .await
                    .map_err(|error| HandlerError::Transient(error.to_string()))?;
                if failed > 0 {
                    return Err(HandlerError::Transient(format!(
                        "{failed} outbox event(s) could not be delivered"
                    )));
                }
                Ok(())
            }
            JobKind::Export => {
                let payload = serde_json::from_str(&job.payload).map_err(|error| {
                    HandlerError::Fatal(format!("the export job's payload is not JSON: {error}"))
                })?;
                crate::exports::run(state, &payload).await
            }
            JobKind::UpdateCheck => {
                let payload = serde_json::from_str(&job.payload).map_err(|error| {
                    HandlerError::Fatal(format!("the update check's payload is not JSON: {error}"))
                })?;
                crate::library_updates::run(state, &payload).await
            }
            JobKind::Reindex => {
                let work_id: String = serde_json::from_str(&job.payload).map_err(|error| {
                    HandlerError::Fatal(format!("the reindex payload is not JSON: {error}"))
                })?;
                let work_id: lorehaven_domain::ids::WorkId = work_id
                    .parse()
                    .map_err(|_| HandlerError::Fatal(format!("invalid work id: {work_id}")))?;
                // Inline query: get the plain text of the latest published chapter.
                // A chapter is indexed when it has a live revision
                // (current_revision_id IS NOT NULL). The work's lifecycle
                // ('draft'|'published') governs *visibility* in search results,
                // not whether the text is indexed — the Reindex job fills the
                // index; the search route filters on lifecycle.
                let plain_text: String = match state.db().backend() {
                    lorehaven_db::Backend::Sqlite => sqlx::query_scalar(
                        "SELECT COALESCE(GROUP_CONCAT(cr.plain_text, ' '), '')
                         FROM chapters c
                         JOIN chapter_revisions cr ON cr.id = c.current_revision_id
                         WHERE c.work_id = ?",
                    )
                    .bind(work_id.to_string())
                    .fetch_one(state.db().sqlite_pool().expect("sqlite"))
                    .await
                    .unwrap_or_default(),
                    lorehaven_db::Backend::Postgres => sqlx::query_scalar(
                        "SELECT COALESCE(STRING_AGG(cr.plain_text, ' '), '')
                         FROM chapters c
                         JOIN chapter_revisions cr ON cr.id = c.current_revision_id::uuid
                         WHERE c.work_id = $1::uuid",
                    )
                    .bind(work_id.to_string())
                    .fetch_one(state.db().postgres_pool().expect("postgres"))
                    .await
                    .unwrap_or_default(),
                };
                lorehaven_db::search::rebuild_work_index(state.db(), &work_id, &plain_text)
                    .await
                    .map_err(|e| HandlerError::Fatal(format!("rebuild index failed: {e}")))?;
                Ok(())
            }
            JobKind::Thumbnail => Err(HandlerError::Fatal(format!(
                "no handler for a {} job in this build",
                kind.as_str()
            ))),
            JobKind::Derivative => {
                let derivative_id: String = serde_json::from_str(&job.payload).map_err(|error| {
                    HandlerError::Fatal(format!("the derivative payload is not JSON: {error}"))
                })?;
                crate::derivative::handle_derivative(state, id, &derivative_id).await
            }
        }
    }

    /// The maintenance tasks. The payload names one; an unknown task is fatal.
    async fn handle_maintenance(
        &self,
        state: &AppState,
        job: JobId,
        row: &jobs::Job,
    ) -> Result<(), HandlerError> {
        let payload: serde_json::Value = serde_json::from_str(&row.payload)
            .map_err(|error| HandlerError::Fatal(format!("the payload is not JSON: {error}")))?;
        let task = payload
            .get("task")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("probe");

        match task {
            // A diagnostic job that does a unit of work per step and records a
            // checkpoint after each one. It is the job the milestone's journey
            // watches, and the one the cancellation test interrupts between
            // steps.
            "probe" => {
                let steps = payload
                    .get("steps")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(1)
                    .clamp(1, 1000);
                let delay_ms = payload
                    .get("delay_ms")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0)
                    .min(5_000);
                // `{"fail": "reason"}` makes the probe report a *transient*
                // failure after its first step. It is how an operator rehearses
                // the retry path, and how the retry policy is tested: every
                // other way a job can fail here is fatal.
                let fail_with = payload
                    .get("fail")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                for step in 1..=steps {
                    if jobs::is_cancelled(state.db(), job)
                        .await
                        .map_err(transient)?
                    {
                        return Err(HandlerError::Cancelled);
                    }
                    if let Some(reason) = &fail_with {
                        return Err(HandlerError::Transient(format!(
                            "the probe was asked to fail: {reason}"
                        )));
                    }
                    if delay_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    }
                    let permille = step * 1000 / steps;
                    jobs::progress(state.db(), job, permille, Some(&format!("step {step}")))
                        .await
                        .map_err(transient)?;
                }
                Ok(())
            }
            "reap_jobs" => {
                let deleted = jobs::purge_terminal_jobs(
                    state.db(),
                    OffsetDateTime::now_utc() - TERMINAL_JOB_RETENTION,
                )
                .await
                .map_err(transient)?;
                jobs::progress(state.db(), job, 1000, Some("reaped"))
                    .await
                    .map_err(transient)?;
                tracing::info!(deleted, "maintenance removed terminal jobs");
                Ok(())
            }
            "collect_blobs" => self.collect_blobs(state, job, &payload).await,
            // Spec §13.2: an export's output is kept seven days and then
            // removed. The sweep unreferences the blob rather than deleting it,
            // so an output that is also something else's reading copy — content
            // is shared by checksum — survives.
            "purge_exports" => {
                let removed = crate::exports::sweep(state).await?;
                jobs::progress(state.db(), job, 1000, Some("purged"))
                    .await
                    .map_err(transient)?;
                tracing::info!(removed, "maintenance removed expired exports");
                Ok(())
            }
            other => Err(HandlerError::Fatal(format!(
                "unknown maintenance task {other:?}"
            ))),
        }
    }

    /// The only blob deletion in the system, and it asks
    /// `content_references` before every removal.
    async fn collect_blobs(
        &self,
        state: &AppState,
        job: JobId,
        payload: &serde_json::Value,
    ) -> Result<(), HandlerError> {
        let limit = payload
            .get("limit")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(50)
            .clamp(1, 1000);
        let store = BlobStore::new(state.config().storage.root.clone());
        let candidates = store
            .unreferenced(state.db(), limit)
            .await
            .map_err(transient)?;
        let total = i64::try_from(candidates.len()).unwrap_or(i64::MAX).max(1);

        let mut removed = 0;
        for (index, checksum) in candidates.iter().enumerate() {
            // Between units: a long collection run must be interruptible, or a
            // cancel is a lie about a job that runs for ten minutes.
            if jobs::is_cancelled(state.db(), job)
                .await
                .map_err(transient)?
            {
                return Err(HandlerError::Cancelled);
            }
            if store
                .delete_if_unreferenced(state.db(), checksum)
                .await
                .map_err(transient)?
            {
                removed += 1;
            }
            let permille = i64::try_from(index + 1).unwrap_or(total) * 1000 / total;
            jobs::progress(
                state.db(),
                job,
                permille,
                Some(&format!("collected {removed}")),
            )
            .await
            .map_err(transient)?;
        }
        Ok(())
    }
}

/// When a failed outbox event may be tried again. Pure, so the rule is testable
/// without a database: the same base delay and growth the job queue uses, with
/// a cap so a permanently broken topic does not drift into next year.
fn next_outbox_attempt(attempts: i64, now: OffsetDateTime) -> String {
    let policy = RetryPolicy {
        max_attempts: u32::MAX,
        base_delay: Duration::from_secs(60),
        backoff: 2.0,
        jitter_permille: 0,
    };
    let attempt = u32::try_from(attempts.max(0))
        .unwrap_or(u32::MAX)
        .saturating_add(1);
    let delay = policy.delay_before(attempt.min(20));
    lorehaven_db::identity::format_rfc3339(now + delay)
}

/// A repository fault during a handler is transient: the attempt may be retried.
fn transient(error: anyhow::Error) -> HandlerError {
    HandlerError::Transient(error.to_string())
}
