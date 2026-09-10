//! The job queue: enqueue, claim, lease, retry, cancel and report (spec §10.1,
//! §10.2).
//!
//! The rules this module exists to hold:
//!
//! * **A claim is one statement.** Two workers racing for the same queue must
//!   not both get the same row, so the claim is a single `UPDATE … WHERE id =
//!   (SELECT …)` — with `FOR UPDATE SKIP LOCKED` on PostgreSQL and inside a
//!   write transaction on SQLite — and never a `SELECT` followed by an
//!   `UPDATE`.
//! * **Every lease expires.** A worker that is killed mid-job leaves a lease
//!   behind, and [`requeue_expired_leases`] is what returns that job to the
//!   queue. Without it, one `kill -9` takes a job out of circulation forever.
//! * **A worker may only finish what it still holds.** `complete` and `fail`
//!   carry `lease_owner = ?`; zero rows affected means the lease was lost and
//!   the worker must stop rather than write an outcome on top of whoever holds
//!   it now.
//! * **A retry is scheduled, not spun.** A failed attempt moves the job back to
//!   `queued` with a future `available_at` from
//!   [`lorehaven_domain::jobs::next_attempt_at`], and gives up at
//!   `max_attempts`.
//! * **Cancellation is a state, not a signal.** The worker checks the stored
//!   state between units of work (`is_cancelled`), because a cancel that only
//!   took effect at the start of a ten-minute job would be a lie.
//!
//! As everywhere else in this crate, every statement is written once per dialect
//! and binds only `String`/`i64` (ADR 0004).

use std::time::Duration;

use anyhow::Result;
use sqlx::FromRow;
use time::OffsetDateTime;

use lorehaven_domain::jobs::{may_retry, next_attempt_at, JobKind, JobState, RetryPolicy};
use lorehaven_domain::{AccountId, JobId};

use crate::identity::now_rfc3339;
use crate::{Backend, Database};

/// A job row.
#[derive(Debug, Clone, FromRow)]
pub struct Job {
    /// Primary key.
    pub id: String,
    /// What the job is for.
    pub kind: String,
    /// Where it is in its life.
    pub state: String,
    /// The handler's input, as JSON.
    pub payload: String,
    /// The caller's dedupe key, when there is one.
    pub idempotency_key: Option<String>,
    /// Higher runs first.
    pub priority: i64,
    /// Attempts made so far.
    pub attempts: i64,
    /// Attempts allowed.
    pub max_attempts: i64,
    /// The earliest moment it may run.
    pub available_at: String,
    /// The worker holding the lease.
    pub lease_owner: Option<String>,
    /// When the lease ends if it is not renewed.
    pub lease_expires_at: Option<String>,
    /// How far along, in thousandths.
    pub progress_permille: i64,
    /// Where a resumed attempt picks up.
    pub checkpoint: Option<String>,
    /// The last failure, for the person who has to explain it.
    pub last_error: Option<String>,
    /// Who asked for it.
    pub requested_by: Option<String>,
    /// Created at, RFC 3339.
    pub created_at: String,
    /// Updated at, RFC 3339.
    pub updated_at: String,
    /// Optimistic concurrency.
    pub version: i64,
}

impl Job {
    /// The parsed state, or `None` for a value this build does not know.
    #[must_use]
    pub fn state(&self) -> Option<JobState> {
        JobState::parse(&self.state)
    }

    /// The parsed kind, or `None`.
    #[must_use]
    pub fn kind(&self) -> Option<JobKind> {
        JobKind::parse(&self.kind)
    }

    /// Whether this job is finished.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        self.state().is_some_and(JobState::is_terminal)
    }
}

/// One attempt at a job.
#[derive(Debug, Clone, FromRow)]
pub struct JobAttempt {
    /// Primary key.
    pub id: String,
    /// The job.
    pub job_id: String,
    /// 1-based attempt number.
    pub attempt: i64,
    /// Started at, RFC 3339.
    pub started_at: String,
    /// Finished at, or `None` while it is running.
    pub finished_at: Option<String>,
    /// The outcome, once it has one.
    pub outcome: Option<String>,
    /// The failure, redacted by the worker before it is stored.
    pub error: Option<String>,
    /// Which worker ran it.
    pub worker: String,
}

/// A deterministic jitter seed for a job, so a retry's delay is reproducible.
fn jitter_seed(job: JobId) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in job.to_string().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Queue a job. Replaying one idempotency key enqueues one job and returns its
/// id: the second call finds the first job rather than adding a second.
pub async fn enqueue(
    db: &Database,
    kind: JobKind,
    payload: &str,
    idempotency_key: Option<&str>,
    requested_by: Option<AccountId>,
    priority: i64,
    policy: &RetryPolicy,
) -> Result<JobId> {
    let id = JobId::new();
    let now = now_rfc3339();
    let requested_by = requested_by.map(|account| account.to_string());

    let insert = db.sql(
        "INSERT INTO jobs
             (id, kind, state, payload, idempotency_key, priority, attempts,
              max_attempts, available_at, progress_permille, requested_by,
              created_at, updated_at, version)
         VALUES (?, ?, 'queued', ?, ?, ?, 0, ?, ?, 0, ?, ?, ?, 1)
         ON CONFLICT (idempotency_key) WHERE idempotency_key IS NOT NULL DO NOTHING",
        "INSERT INTO jobs
             (id, kind, state, payload, idempotency_key, priority, attempts,
              max_attempts, available_at, progress_permille, requested_by,
              created_at, updated_at, version)
         VALUES (?::uuid, ?, 'queued', ?, ?, ?, 0, ?, ?, 0, ?::uuid, ?, ?, 1)
         ON CONFLICT (idempotency_key) WHERE idempotency_key IS NOT NULL DO NOTHING",
    );

    let existing = db.sql(
        "SELECT id FROM jobs WHERE idempotency_key = ?",
        "SELECT id::text AS id FROM jobs WHERE idempotency_key = ?",
    );

    let max_attempts = i64::from(policy.max_attempts);
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite handle");
            let affected = sqlx::query(&insert)
                .bind(id.to_string())
                .bind(kind.as_str())
                .bind(payload)
                .bind(idempotency_key)
                .bind(priority)
                .bind(max_attempts)
                .bind(&now)
                .bind(requested_by.as_deref())
                .bind(&now)
                .bind(&now)
                .execute(pool)
                .await?
                .rows_affected();
            if affected > 0 {
                return Ok(id);
            }
            let row: Option<(String,)> = sqlx::query_as(&existing)
                .bind(idempotency_key)
                .fetch_optional(pool)
                .await?;
            Ok(row.and_then(|(found,)| found.parse().ok()).unwrap_or(id))
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            let affected = sqlx::query(&insert)
                .bind(id.to_string())
                .bind(kind.as_str())
                .bind(payload)
                .bind(idempotency_key)
                .bind(priority)
                .bind(max_attempts)
                .bind(&now)
                .bind(requested_by.as_deref())
                .bind(&now)
                .bind(&now)
                .execute(pool)
                .await?
                .rows_affected();
            if affected > 0 {
                return Ok(id);
            }
            let row: Option<(String,)> = sqlx::query_as(&existing)
                .bind(idempotency_key)
                .fetch_optional(pool)
                .await?;
            Ok(row.and_then(|(found,)| found.parse().ok()).unwrap_or(id))
        }
    }
}

/// Claim the next runnable job for `worker`, taking a lease for `lease`.
///
/// One statement, because two workers racing must not both get the same row.
/// `None` means the queue is empty (or everything is waiting or leased).
pub async fn claim_next(
    db: &Database,
    worker: &str,
    lease: Duration,
    now: OffsetDateTime,
) -> Result<Option<Job>> {
    let now_text = crate::identity::format_rfc3339(now);
    let expires = crate::identity::format_rfc3339(now + lease);
    let lease_secs = i64::try_from(lease.as_secs()).unwrap_or(i64::MAX);

    let claim = db.sql(
        "UPDATE jobs
            SET state = 'leased', lease_owner = ?, lease_expires_at = ?,
                updated_at = ?, version = version + 1
          WHERE id = (SELECT id FROM jobs
                       WHERE state = 'queued' AND available_at <= ?
                       ORDER BY priority DESC, available_at ASC
                       LIMIT 1)
        RETURNING id",
        "UPDATE jobs
            SET state = 'leased', lease_owner = ?, lease_expires_at = ?,
                updated_at = ?, version = version + 1
           FROM (SELECT id FROM jobs
                  WHERE state = 'queued' AND available_at <= ?
                  ORDER BY priority DESC, available_at ASC
                  LIMIT 1
                  FOR UPDATE SKIP LOCKED) AS claimed
          WHERE jobs.id = claimed.id
        RETURNING jobs.id::text AS id",
    );
    let _ = lease_secs;

    let find = db.sql(
        "SELECT id, kind, state, payload, idempotency_key, priority, attempts,
                max_attempts, available_at, lease_owner, lease_expires_at,
                progress_permille, checkpoint, last_error, requested_by,
                created_at, updated_at, version
           FROM jobs WHERE id = ?",
        "SELECT id::text AS id, kind, state, payload, idempotency_key, priority,
                attempts, max_attempts, available_at, lease_owner,
                lease_expires_at, progress_permille, checkpoint, last_error,
                requested_by::text AS requested_by, created_at, updated_at, version
           FROM jobs WHERE id = ?::uuid",
    );

    match db.backend() {
        Backend::Sqlite => {
            // The write lock is taken by the BEGIN: two claims serialise rather
            // than reading the same row.
            let pool = db.sqlite_pool().expect("sqlite handle");
            let mut tx = pool.begin().await?;
            let claimed: Option<(String,)> = sqlx::query_as(&claim)
                .bind(worker)
                .bind(&expires)
                .bind(&now_text)
                .bind(&now_text)
                .fetch_optional(&mut *tx)
                .await?;
            let Some((id,)) = claimed else {
                tx.commit().await?;
                return Ok(None);
            };
            let job: Option<Job> = sqlx::query_as(&find)
                .bind(&id)
                .fetch_optional(&mut *tx)
                .await?;
            tx.commit().await?;
            Ok(job)
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            let claimed: Option<(String,)> = sqlx::query_as(&claim)
                .bind(worker)
                .bind(&expires)
                .bind(&now_text)
                .bind(&now_text)
                .fetch_optional(pool)
                .await?;
            let Some((id,)) = claimed else {
                return Ok(None);
            };
            let job: Option<Job> = sqlx::query_as(&find).bind(&id).fetch_optional(pool).await?;
            Ok(job)
        }
    }
}

/// Renew a lease. `false` means the lease is no longer held by this worker.
pub async fn heartbeat(
    db: &Database,
    job: JobId,
    worker: &str,
    lease: Duration,
    now: OffsetDateTime,
) -> Result<bool> {
    let sql = db.sql(
        "UPDATE jobs SET lease_expires_at = ?, updated_at = ?, version = version + 1
          WHERE id = ? AND lease_owner = ? AND state IN ('leased', 'running')",
        "UPDATE jobs SET lease_expires_at = ?, updated_at = ?, version = version + 1
          WHERE id = ?::uuid AND lease_owner = ? AND state IN ('leased', 'running')",
    );
    let expires = crate::identity::format_rfc3339(now + lease);
    let now_text = crate::identity::format_rfc3339(now);

    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(&expires)
            .bind(&now_text)
            .bind(job.to_string())
            .bind(worker)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(&expires)
            .bind(&now_text)
            .bind(job.to_string())
            .bind(worker)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// Record that a worker has begun an attempt.
///
/// This is also what moves the job's own `attempts` counter: an attempt that has
/// begun has been *made*, and a finished job that reported `0` attempts while
/// `job_attempts` held a row for it is a column that calls its own history a
/// lie. Deciding on the retry reads this counter back.
pub async fn attempt_started(
    db: &Database,
    job: JobId,
    attempt: i64,
    worker: &str,
) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = now_rfc3339();
    let count = db.sql(
        "UPDATE jobs SET attempts = ?, updated_at = ?, version = version + 1 WHERE id = ?",
        "UPDATE jobs SET attempts = ?, updated_at = ?, version = version + 1 WHERE id = ?::uuid",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&count)
                .bind(attempt)
                .bind(&now)
                .bind(job.to_string())
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&count)
                .bind(attempt)
                .bind(&now)
                .bind(job.to_string())
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    let sql = db.sql(
        "INSERT INTO job_attempts (id, job_id, attempt, started_at, worker)
         VALUES (?, ?, ?, ?, ?)",
        "INSERT INTO job_attempts (id, job_id, attempt, started_at, worker)
         VALUES (?::uuid, ?::uuid, ?, ?, ?)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(job.to_string())
                .bind(attempt)
                .bind(&now)
                .bind(worker)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(job.to_string())
                .bind(attempt)
                .bind(&now)
                .bind(worker)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(id)
}

/// Close the open attempt, with its outcome and error.
///
/// Public because the worker has three outcomes it must record and only two of
/// them go through [`complete`]/[`fail`] — a cancelled attempt ends neither way.
pub async fn close_attempt(
    db: &Database,
    job: JobId,
    worker: &str,
    outcome: &str,
    error: Option<&str>,
) -> Result<()> {
    let sql = db.sql(
        "UPDATE job_attempts
            SET finished_at = ?, outcome = ?, error = ?
          WHERE job_id = ? AND worker = ? AND finished_at IS NULL",
        "UPDATE job_attempts
            SET finished_at = ?, outcome = ?, error = ?
          WHERE job_id = ?::uuid AND worker = ? AND finished_at IS NULL",
    );
    let now = now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&now)
                .bind(outcome)
                .bind(error)
                .bind(job.to_string())
                .bind(worker)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&now)
                .bind(outcome)
                .bind(error)
                .bind(job.to_string())
                .bind(worker)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(())
}

/// Finish a job successfully.
///
/// `false` means this worker no longer holds the lease: the job was reclaimed
/// while the attempt ran, and writing the outcome now would attribute one
/// worker's work to another.
pub async fn complete(db: &Database, job: JobId, worker: &str) -> Result<bool> {
    let sql = db.sql(
        "UPDATE jobs
            SET state = 'succeeded', progress_permille = 1000, checkpoint = NULL,
                lease_owner = NULL, lease_expires_at = NULL, last_error = NULL,
                updated_at = ?, version = version + 1
          WHERE id = ? AND lease_owner = ? AND state IN ('leased', 'running')",
        "UPDATE jobs
            SET state = 'succeeded', progress_permille = 1000, checkpoint = NULL,
                lease_owner = NULL, lease_expires_at = NULL, last_error = NULL,
                updated_at = ?, version = version + 1
          WHERE id = ?::uuid AND lease_owner = ? AND state IN ('leased', 'running')",
    );
    let now = now_rfc3339();
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(&now)
            .bind(job.to_string())
            .bind(worker)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(&now)
            .bind(job.to_string())
            .bind(worker)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    close_attempt(db, job, worker, "succeeded", None).await?;
    Ok(affected > 0)
}

/// Record a failed attempt: retry it with the policy's backoff, or fail it.
///
/// Returns the state the job ended up in. A job that has run out of attempts
/// or that failed fatally is `Failed`; otherwise it is `Queued` with a future
/// `available_at`.
///
/// Two budgets meet here and the smaller wins. The row carries the
/// `max_attempts` the requester asked for at enqueue time; the caller passes the
/// worker's policy. A worker configured to try less often must not overrule the
/// request, and a request for one attempt must not be retried five times
/// because the worker's policy is generous.
pub async fn fail(
    db: &Database,
    job: JobId,
    worker: &str,
    error: &str,
    policy: &RetryPolicy,
    fatal: bool,
    now: OffsetDateTime,
) -> Result<JobState> {
    // Read the attempt count and the request's budget under the lease, then
    // decide.
    let find = db.sql(
        "SELECT attempts, max_attempts FROM jobs WHERE id = ? AND lease_owner = ?",
        "SELECT attempts, max_attempts FROM jobs WHERE id = ?::uuid AND lease_owner = ?",
    );
    let attempts: Option<(i64, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&find)
                .bind(job.to_string())
                .bind(worker)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&find)
                .bind(job.to_string())
                .bind(worker)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    let Some((previous_attempts, requested_max_attempts)) = attempts else {
        // The lease is gone. Nothing to record against, and nothing to retry.
        close_attempt(db, job, worker, "lost_lease", Some(error)).await?;
        return Ok(JobState::Queued);
    };

    let effective = RetryPolicy {
        max_attempts: policy
            .max_attempts
            .min(u32::try_from(requested_max_attempts).unwrap_or(u32::MAX)),
        ..*policy
    };
    // The attempt just made is the one `attempt_started` recorded; adding one
    // here would count every attempt twice and spend the budget at half rate.
    let attempt_number = previous_attempts.max(1);
    let will_retry = !fatal
        && may_retry(
            u32::try_from(attempt_number).unwrap_or(u32::MAX),
            &effective,
        );
    let next_state = if will_retry {
        JobState::Queued
    } else {
        JobState::Failed
    };
    let next_at = next_attempt_at(
        u32::try_from(attempt_number).unwrap_or(u32::MAX),
        &effective,
        jitter_seed(job),
        now,
    );

    let sql = db.sql(
        "UPDATE jobs
            SET state = ?, attempts = ?, available_at = ?, last_error = ?,
                lease_owner = NULL, lease_expires_at = NULL, updated_at = ?,
                version = version + 1
          WHERE id = ? AND lease_owner = ?",
        "UPDATE jobs
            SET state = ?, attempts = ?, available_at = ?, last_error = ?,
                lease_owner = NULL, lease_expires_at = NULL, updated_at = ?,
                version = version + 1
          WHERE id = ?::uuid AND lease_owner = ?",
    );
    let available_at = if will_retry {
        crate::identity::format_rfc3339(next_at)
    } else {
        crate::identity::format_rfc3339(now)
    };
    let now_text = crate::identity::format_rfc3339(now);

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(next_state.as_str())
                .bind(attempt_number)
                .bind(&available_at)
                .bind(error)
                .bind(&now_text)
                .bind(job.to_string())
                .bind(worker)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(next_state.as_str())
                .bind(attempt_number)
                .bind(&available_at)
                .bind(error)
                .bind(&now_text)
                .bind(job.to_string())
                .bind(worker)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }

    close_attempt(db, job, worker, next_state.as_str(), Some(error)).await?;
    Ok(next_state)
}

/// Cancel a job. `false` means it was already finished, and a finished job is
/// left exactly as it is.
pub async fn cancel(db: &Database, job: JobId) -> Result<bool> {
    let sql = db.sql(
        "UPDATE jobs
            SET state = 'cancelled', lease_owner = NULL, lease_expires_at = NULL,
                updated_at = ?, version = version + 1
          WHERE id = ? AND state IN ('queued', 'leased', 'running')",
        "UPDATE jobs
            SET state = 'cancelled', lease_owner = NULL, lease_expires_at = NULL,
                updated_at = ?, version = version + 1
          WHERE id = ?::uuid AND state IN ('queued', 'leased', 'running')",
    );
    let now = now_rfc3339();
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(&now)
            .bind(job.to_string())
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(&now)
            .bind(job.to_string())
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// Return leases that have expired to the queue. Returns how many were
/// requeued, so a maintenance run can say what it did.
///
/// The attempts counter is *not* incremented here: the worker that died did not
/// report an outcome, and charging it an attempt would let a repeatedly killed
/// worker burn a job's whole budget without ever failing.
pub async fn requeue_expired_leases(db: &Database, now: OffsetDateTime) -> Result<u64> {
    let sql = db.sql(
        "UPDATE jobs
            SET state = 'queued', lease_owner = NULL, lease_expires_at = NULL,
                updated_at = ?, version = version + 1
          WHERE state IN ('leased', 'running')
            AND lease_expires_at IS NOT NULL AND lease_expires_at <= ?",
        "UPDATE jobs
            SET state = 'queued', lease_owner = NULL, lease_expires_at = NULL,
                updated_at = ?, version = version + 1
          WHERE state IN ('leased', 'running')
            AND lease_expires_at IS NOT NULL AND lease_expires_at <= ?",
    );
    let now_text = crate::identity::format_rfc3339(now);
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(&now_text)
            .bind(&now_text)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(&now_text)
            .bind(&now_text)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected)
}

/// Record progress and the checkpoint a resumed attempt restarts from.
pub async fn progress(
    db: &Database,
    job: JobId,
    permille: i64,
    checkpoint: Option<&str>,
) -> Result<()> {
    let sql = db.sql(
        "UPDATE jobs
            SET progress_permille = ?, checkpoint = COALESCE(?, checkpoint),
                state = CASE WHEN state = 'leased' THEN 'running' ELSE state END,
                updated_at = ?, version = version + 1
          WHERE id = ? AND state IN ('leased', 'running')",
        "UPDATE jobs
            SET progress_permille = ?, checkpoint = COALESCE(?, checkpoint),
                state = CASE WHEN state = 'leased' THEN 'running' ELSE state END,
                updated_at = ?, version = version + 1
          WHERE id = ?::uuid AND state IN ('leased', 'running')",
    );
    let permille = permille.clamp(0, 1000);
    let now = now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(permille)
                .bind(checkpoint)
                .bind(&now)
                .bind(job.to_string())
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(permille)
                .bind(checkpoint)
                .bind(&now)
                .bind(job.to_string())
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(())
}

/// Whether the stored state says this job has been cancelled. The worker asks
/// this **between** units of work, not only at the start.
pub async fn is_cancelled(db: &Database, job: JobId) -> Result<bool> {
    let sql = db.sql(
        "SELECT state FROM jobs WHERE id = ?",
        "SELECT state FROM jobs WHERE id = ?::uuid",
    );
    let state: Option<(String,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(job.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(job.to_string())
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(state.is_some_and(|(state,)| state == JobState::Cancelled.as_str()))
}

/// One job by id.
pub async fn find(db: &Database, job: JobId) -> Result<Option<Job>> {
    let sql = db.sql(
        "SELECT id, kind, state, payload, idempotency_key, priority, attempts,
                max_attempts, available_at, lease_owner, lease_expires_at,
                progress_permille, checkpoint, last_error, requested_by,
                created_at, updated_at, version
           FROM jobs WHERE id = ?",
        "SELECT id::text AS id, kind, state, payload, idempotency_key, priority,
                attempts, max_attempts, available_at, lease_owner,
                lease_expires_at, progress_permille, checkpoint, last_error,
                requested_by::text AS requested_by, created_at, updated_at, version
           FROM jobs WHERE id = ?::uuid",
    );
    Ok(match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(job.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(job.to_string())
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    })
}

/// The jobs an account asked for, newest first. This is `/jobs`: a caller sees
/// their own queue and nobody else's.
///
/// `after` is the last row of the previous page, as `(created_at, id)`. The
/// comparison is spelled out rather than written as a row value so both engines
/// read it the same way, and the id breaks ties between two rows created in the
/// same instant — without it, a page boundary can repeat or skip a row.
pub async fn jobs_for(
    db: &Database,
    account: AccountId,
    limit: i64,
    after: Option<(&str, &str)>,
) -> Result<Vec<Job>> {
    let sql = match after {
        Some(_) => db.sql(
            "SELECT id, kind, state, payload, idempotency_key, priority, attempts,
                    max_attempts, available_at, lease_owner, lease_expires_at,
                    progress_permille, checkpoint, last_error, requested_by,
                    created_at, updated_at, version
               FROM jobs WHERE requested_by = ?
                 AND (created_at < ? OR (created_at = ? AND id < ?))
              ORDER BY created_at DESC, id DESC LIMIT ?",
            "SELECT id::text AS id, kind, state, payload, idempotency_key, priority,
                    attempts, max_attempts, available_at, lease_owner,
                    lease_expires_at, progress_permille, checkpoint, last_error,
                    requested_by::text AS requested_by, created_at, updated_at, version
               FROM jobs WHERE requested_by = ?::uuid
                 AND (created_at < ? OR (created_at = ? AND id::text < ?))
              ORDER BY created_at DESC, id DESC LIMIT ?",
        ),
        None => db.sql(
            "SELECT id, kind, state, payload, idempotency_key, priority, attempts,
                    max_attempts, available_at, lease_owner, lease_expires_at,
                    progress_permille, checkpoint, last_error, requested_by,
                    created_at, updated_at, version
               FROM jobs WHERE requested_by = ?
              ORDER BY created_at DESC, id DESC LIMIT ?",
            "SELECT id::text AS id, kind, state, payload, idempotency_key, priority,
                    attempts, max_attempts, available_at, lease_owner,
                    lease_expires_at, progress_permille, checkpoint, last_error,
                    requested_by::text AS requested_by, created_at, updated_at, version
               FROM jobs WHERE requested_by = ?::uuid
              ORDER BY created_at DESC, id DESC LIMIT ?",
        ),
    };

    Ok(match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite handle");
            let query = sqlx::query_as(&sql).bind(account.to_string());
            match after {
                Some((created_at, id)) => {
                    query
                        .bind(created_at)
                        .bind(created_at)
                        .bind(id)
                        .bind(limit)
                        .fetch_all(pool)
                        .await?
                }
                None => query.bind(limit).fetch_all(pool).await?,
            }
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            let query = sqlx::query_as(&sql).bind(account.to_string());
            match after {
                Some((created_at, id)) => {
                    query
                        .bind(created_at)
                        .bind(created_at)
                        .bind(id)
                        .bind(limit)
                        .fetch_all(pool)
                        .await?
                }
                None => query.bind(limit).fetch_all(pool).await?,
            }
        }
    })
}

/// Every job, optionally filtered by state, newest first. This is
/// `/admin/jobs`, which is an operator's view and not a reader's.
///
/// `after` paginates exactly as [`jobs_for`] does, so one cursor convention
/// serves both lists.
pub async fn all_jobs(
    db: &Database,
    state: Option<&str>,
    limit: i64,
    after: Option<(&str, &str)>,
) -> Result<Vec<Job>> {
    let sql = match after {
        Some(_) => db.sql(
            "SELECT id, kind, state, payload, idempotency_key, priority, attempts,
                    max_attempts, available_at, lease_owner, lease_expires_at,
                    progress_permille, checkpoint, last_error, requested_by,
                    created_at, updated_at, version
               FROM jobs
              WHERE (? IS NULL OR state = ?)
                AND (created_at < ? OR (created_at = ? AND id < ?))
              ORDER BY created_at DESC, id DESC LIMIT ?",
            "SELECT id::text AS id, kind, state, payload, idempotency_key, priority,
                    attempts, max_attempts, available_at, lease_owner,
                    lease_expires_at, progress_permille, checkpoint, last_error,
                    requested_by::text AS requested_by, created_at, updated_at, version
               FROM jobs
              WHERE (? IS NULL OR state = ?)
                AND (created_at < ? OR (created_at = ? AND id::text < ?))
              ORDER BY created_at DESC, id DESC LIMIT ?",
        ),
        None => db.sql(
            "SELECT id, kind, state, payload, idempotency_key, priority, attempts,
                    max_attempts, available_at, lease_owner, lease_expires_at,
                    progress_permille, checkpoint, last_error, requested_by,
                    created_at, updated_at, version
               FROM jobs
              WHERE (? IS NULL OR state = ?)
              ORDER BY created_at DESC, id DESC LIMIT ?",
            "SELECT id::text AS id, kind, state, payload, idempotency_key, priority,
                    attempts, max_attempts, available_at, lease_owner,
                    lease_expires_at, progress_permille, checkpoint, last_error,
                    requested_by::text AS requested_by, created_at, updated_at, version
               FROM jobs
              WHERE (? IS NULL OR state = ?)
              ORDER BY created_at DESC, id DESC LIMIT ?",
        ),
    };

    Ok(match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite handle");
            let query = sqlx::query_as(&sql).bind(state).bind(state);
            match after {
                Some((created_at, id)) => {
                    query
                        .bind(created_at)
                        .bind(created_at)
                        .bind(id)
                        .bind(limit)
                        .fetch_all(pool)
                        .await?
                }
                None => query.bind(limit).fetch_all(pool).await?,
            }
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            let query = sqlx::query_as(&sql).bind(state).bind(state);
            match after {
                Some((created_at, id)) => {
                    query
                        .bind(created_at)
                        .bind(created_at)
                        .bind(id)
                        .bind(limit)
                        .fetch_all(pool)
                        .await?
                }
                None => query.bind(limit).fetch_all(pool).await?,
            }
        }
    })
}

/// How many jobs are in each state, for the admin dashboard.
pub async fn counts_by_state(db: &Database) -> Result<Vec<(String, i64)>> {
    let sql = db.sql(
        "SELECT state, COUNT(*) AS total FROM jobs GROUP BY state",
        "SELECT state, COUNT(*)::bigint AS total FROM jobs GROUP BY state",
    );
    Ok(match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    })
}

/// The attempts made at a job, oldest first, for the job detail view.
pub async fn attempts_for(db: &Database, job: JobId) -> Result<Vec<JobAttempt>> {
    let sql = db.sql(
        "SELECT id, job_id, attempt, started_at, finished_at, outcome, error, worker
           FROM job_attempts WHERE job_id = ? ORDER BY attempt ASC",
        "SELECT id::text AS id, job_id::text AS job_id, attempt, started_at,
                finished_at, outcome, error, worker
           FROM job_attempts WHERE job_id = ?::uuid ORDER BY attempt ASC",
    );
    Ok(match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(job.to_string())
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(job.to_string())
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    })
}

/// Delete terminal jobs that finished longer ago than the retention window
/// (30 days, spec §10.1). Returns how many rows went.
///
/// This is the *only* deletion of jobs, and it refuses to touch anything that
/// has not reached a terminal state: a queued job is somebody's pending work.
pub async fn purge_terminal_jobs(db: &Database, older_than: OffsetDateTime) -> Result<u64> {
    let sql = db.sql(
        "DELETE FROM jobs
          WHERE state IN ('succeeded', 'failed', 'cancelled') AND updated_at < ?",
        "DELETE FROM jobs
          WHERE state IN ('succeeded', 'failed', 'cancelled') AND updated_at < ?",
    );
    let cutoff = crate::identity::format_rfc3339(older_than);
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(&cutoff)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(&cutoff)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected)
}

/// Put a finished job back in the queue, for an operator's retry.
///
/// Only a terminal job may be requeued: queueing a job that a worker is running
/// would be two workers on one row. The attempt counter is reset so the retry
/// gets the policy's full budget — the operator is asking for a fresh run, not
/// for the last attempt of the old one.
pub async fn requeue(db: &Database, job: JobId) -> Result<bool> {
    let sql = db.sql(
        "UPDATE jobs
            SET state = 'queued', attempts = 0, available_at = ?, last_error = NULL,
                progress_permille = 0, checkpoint = NULL, lease_owner = NULL,
                lease_expires_at = NULL, updated_at = ?, version = version + 1
          WHERE id = ? AND state IN ('succeeded', 'failed', 'cancelled')",
        "UPDATE jobs
            SET state = 'queued', attempts = 0, available_at = ?, last_error = NULL,
                progress_permille = 0, checkpoint = NULL, lease_owner = NULL,
                lease_expires_at = NULL, updated_at = ?, version = version + 1
          WHERE id = ?::uuid AND state IN ('succeeded', 'failed', 'cancelled')",
    );
    let now = now_rfc3339();
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(&now)
            .bind(&now)
            .bind(job.to_string())
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(&now)
            .bind(&now)
            .bind(job.to_string())
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}
