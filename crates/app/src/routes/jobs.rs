//! The job queue's HTTP surface (spec §10.1).
//!
//! ```text
//! POST   /jobs                           Write class, RequireSession, dev only
//! POST   /jobs/:id/cancel                 Write class, RequireSession
//! GET    /jobs?cursor=…                   the caller's own jobs, envelope
//! GET    /admin/jobs?state=…&cursor=…     operators only
//! POST   /admin/jobs/:id/retry            operators only
//! ```
//!
//! Three rules this module exists to hold:
//!
//! * **A caller sees their own jobs and nobody else's.** `/jobs` filters on
//!   `requested_by`; the id on the path is never enough to reach somebody else's
//!   row.
//! * **`/admin` is gated on configuration, not on a role.** There is no staff
//!   model yet: `config.administration.operator_account_id` names one account,
//!   and Milestone 13 replaces it with a trust level. A non-operator gets `404`,
//!   not `403` — saying "forbidden" would confirm that the admin surface exists
//!   and that they are not on it.
//! * **Cancelling a finished job is not an error.** The caller asked for the end
//!   state, and it already holds; the answer says what the job's state is, so a
//!   reader who clicked twice is not shown a failure.
//!
//! `POST /jobs` exists so the queue is reachable before Milestone 6 supplies the
//! endpoints that actually need a queue. It is limited to *development*: an
//! import, an export and a reindex are enqueued by the request that wants them,
//! with the payload that request chose, and an account able to enqueue an
//! arbitrary payload could ask for work nobody meant to run. M6 replaces it.

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

use lorehaven_db::jobs::{self, Job};
use lorehaven_domain::jobs::{can_cancel, JobKind, JobState, RetryPolicy};
use lorehaven_domain::{AppError, JobId};

use crate::auth::{RequirePseud, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

/// The caller's own queue, and the cancel action.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/jobs", get(list_jobs).post(start_job))
        .route("/jobs/{id}/cancel", post(cancel_job))
}

/// The operator's view. Gated on `config.administration.operator_account_id`.
pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/admin/jobs", get(list_all_jobs))
        .route("/admin/jobs/{id}/retry", post(retry_job))
}

/// The body of `POST /jobs`: what to run, and how much of it.
#[derive(Debug, Deserialize)]
struct StartJobRequest {
    /// Only `maintenance` is accepted, and only its `probe` task.
    #[serde(default = "default_kind")]
    kind: String,
    /// Passed through to the handler. `{"task":"probe","steps":N,"delay_ms":N}`
    /// is the diagnostic; anything else is refused.
    #[serde(default)]
    payload: Option<serde_json::Value>,
}

fn default_kind() -> String {
    "maintenance".to_owned()
}

/// The self-service enqueue. Development only, and deliberately narrow.
async fn start_job(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(request): Json<StartJobRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<JobView>)> {
    if !state.config().environment.is_development() {
        // Not "forbidden": this surface does not exist in a real deployment.
        return Err(ApiError(AppError::NotFound { resource: "page" }));
    }
    if request.kind != JobKind::Maintenance.as_str() {
        return Err(ApiError(AppError::Validation {
            message: format!(
                "{} is not a job a request may start; only {} is accepted here",
                request.kind,
                JobKind::Maintenance.as_str()
            ),
            field_errors: Default::default(),
        }));
    }

    // The payload is checked before it is stored. A row nobody can run is worse
    // than a refusal, because the refusal can tell the caller what was wrong.
    let payload = request
        .payload
        .unwrap_or_else(|| serde_json::json!({ "task": "probe" }));
    let task = payload
        .get("task")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("probe");
    if task != "probe" {
        return Err(ApiError(AppError::Validation {
            message: format!(
                "{task:?} is not a task a request may start; only \"probe\" is accepted here"
            ),
            field_errors: Default::default(),
        }));
    }

    let job_id = jobs::enqueue(
        state.db(),
        JobKind::Maintenance,
        &payload.to_string(),
        None,
        Some(user.account_id),
        0,
        // The same policy a default worker runs with. The import and export
        // endpoints in M6 carry their own, because their retry behaviour is
        // theirs to choose; a diagnostic job has no opinion.
        &RetryPolicy::default(),
    )
    .await?;

    let job = jobs::find(state.db(), job_id)
        .await?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "job" }))?;
    // 202, not 201: the answer carries the job's *id*, not its result. The work
    // has been accepted and has not happened yet.
    Ok((axum::http::StatusCode::ACCEPTED, Json(JobView::from(job))))
}

/// One job, as the interface shows it.
#[derive(Debug, Serialize)]
struct JobView {
    id: String,
    kind: String,
    state: String,
    payload: String,
    progress_permille: i64,
    checkpoint: Option<String>,
    last_error: Option<String>,
    attempts: i64,
    max_attempts: i64,
    available_at: String,
    created_at: String,
    updated_at: String,
    version: i64,
    /// The account that asked for the job, when one did.
    ///
    /// Kept on the row after the account is deleted, so an operator can still
    /// see that *a* job ran — which is why this is a nullable string and not a
    /// join that would vanish with the account.
    requested_by: Option<String>,
    /// Whether the caller may still cancel it, so the button is not offered
    /// where the server would refuse (spec §3.3, and the frontend's rule).
    cancellable: bool,
}

impl From<Job> for JobView {
    fn from(job: Job) -> Self {
        let state = job.state().unwrap_or(JobState::Failed);
        Self {
            id: job.id,
            kind: job.kind,
            state: job.state,
            payload: job.payload,
            progress_permille: job.progress_permille,
            checkpoint: job.checkpoint,
            last_error: job.last_error,
            attempts: job.attempts,
            max_attempts: job.max_attempts,
            available_at: job.available_at,
            created_at: job.created_at,
            updated_at: job.updated_at,
            version: job.version,
            requested_by: job.requested_by,
            cancellable: can_cancel(state),
        }
    }
}

/// The collection envelope (spec §3.3): never a bare array.
#[derive(Debug, Serialize)]
struct JobListView {
    items: Vec<JobView>,
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JobsQuery {
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

/// How many rows a page holds. One constant, so the page and its cursor agree.
const PAGE: i64 = 50;

/// Decode `created_at|id`, the pair the next page starts after.
///
/// A malformed cursor is refused rather than treated as "start from the
/// beginning": silently rewinding a reader to page one looks like it worked.
fn decode_cursor(raw: &str) -> ApiResult<(String, String)> {
    let decoded = BASE64
        .decode(raw)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .ok_or_else(|| {
            ApiError(AppError::Validation {
                message: "the cursor is not one this collection issued".to_owned(),
                field_errors: Default::default(),
            })
        })?;
    let (created_at, id) = decoded.split_once('|').ok_or_else(|| {
        ApiError(AppError::Validation {
            message: "the cursor is not one this collection issued".to_owned(),
            field_errors: Default::default(),
        })
    })?;
    Ok((created_at.to_owned(), id.to_owned()))
}

/// Encode the last row of a page as the cursor for the next one.
fn encode_cursor(job: &Job) -> String {
    BASE64.encode(format!("{}|{}", job.created_at, job.id))
}

/// Turn one page of rows into the envelope the API contract requires.
fn page(mut rows: Vec<Job>, limit: i64) -> JobListView {
    // A full page means there may be more; the next cursor is the last row's
    // position. A short page means the end, and `next_cursor` is null so a
    // client stops asking.
    let next_cursor = if i64::try_from(rows.len()).unwrap_or(i64::MAX) >= limit {
        rows.last().map(encode_cursor)
    } else {
        None
    };
    JobListView {
        items: rows.drain(..).map(JobView::from).collect(),
        next_cursor,
    }
}

async fn list_jobs(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Query(query): Query<JobsQuery>,
) -> ApiResult<Json<JobListView>> {
    let after = query.cursor.as_deref().map(decode_cursor).transpose()?;
    let rows = jobs::jobs_for(
        state.db(),
        user.account_id,
        PAGE,
        after.as_ref().map(|(at, id)| (at.as_str(), id.as_str())),
    )
    .await?;
    Ok(Json(page(rows, PAGE)))
}

async fn cancel_job(
    State(state): State<AppState>,
    RequirePseud { user, .. }: RequirePseud,
    Path(id): Path<String>,
) -> ApiResult<Json<JobView>> {
    let job_id = parse_job_id(&id)?;
    let job = jobs::find(state.db(), job_id)
        .await?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "job" }))?;

    // One account's job is not another's to cancel, and the answer must not
    // reveal that the job exists.
    if job.requested_by.as_deref() != Some(user.account_id.to_string().as_str()) {
        return Err(ApiError(AppError::NotFound { resource: "job" }));
    }

    jobs::cancel(state.db(), job_id).await?;
    let job = jobs::find(state.db(), job_id)
        .await?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "job" }))?;
    Ok(Json(JobView::from(job)))
}

async fn list_all_jobs(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Query(query): Query<JobsQuery>,
) -> ApiResult<Json<JobListView>> {
    require_operator(&state, &user)?;
    if let Some(requested) = query.state.as_deref() {
        // An unknown filter is refused rather than ignored: a page that quietly
        // shows everything when asked for `state=runing` looks like it worked.
        if JobState::parse(requested).is_none() {
            return Err(ApiError(AppError::Validation {
                message: format!("unknown job state {requested:?}"),
                field_errors: Default::default(),
            }));
        }
    }
    let after = query.cursor.as_deref().map(decode_cursor).transpose()?;
    let rows = jobs::all_jobs(
        state.db(),
        query.state.as_deref(),
        PAGE,
        after.as_ref().map(|(at, id)| (at.as_str(), id.as_str())),
    )
    .await?;
    Ok(Json(page(rows, PAGE)))
}

/// Queue a fresh attempt at a job that has finished failing.
///
/// A retry is a *new* attempt on the same row rather than a new job: the
/// operator is asking the instance to try again, and the record of the original
/// request should stay one row.
async fn retry_job(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<JobView>> {
    require_operator(&state, &user)?;
    let job_id = parse_job_id(&id)?;
    let job = jobs::find(state.db(), job_id)
        .await?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "job" }))?;

    if !job.is_terminal() {
        return Err(ApiError(AppError::Validation {
            message: "this job has not finished, so there is nothing to retry; cancel it instead"
                .to_owned(),
            field_errors: Default::default(),
        }));
    }

    jobs::requeue(state.db(), job_id).await?;
    let job = jobs::find(state.db(), job_id)
        .await?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "job" }))?;
    Ok(Json(JobView::from(job)))
}

/// Whether this account may use the operator surface.
fn require_operator(state: &AppState, user: &crate::auth::SessionUser) -> ApiResult<()> {
    let configured = state.config().administration.operator_account_id;
    if configured == Some(user.account_id) {
        return Ok(());
    }
    // 404, not 403: confirming that an admin surface exists is itself a
    // disclosure. The message names the setting an operator has to change.
    tracing::debug!(
        operator_configured = configured.is_some(),
        "an operator route was reached by an account that is not the operator"
    );
    Err(ApiError(AppError::NotFound { resource: "page" }))
}

fn parse_job_id(raw: &str) -> ApiResult<JobId> {
    raw.parse()
        .map_err(|_| ApiError(AppError::NotFound { resource: "job" }))
}

/// The kinds the queue understands, for the admin filter and for a future
/// extension UI.
#[must_use]
pub fn known_kinds() -> Vec<&'static str> {
    lorehaven_domain::jobs::ALL_KINDS
        .iter()
        .map(|kind| kind.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_kind_is_the_only_one_a_request_may_start() {
        assert_eq!(default_kind(), JobKind::Maintenance.as_str());
    }

    #[test]
    fn every_kind_has_a_wire_name() {
        for kind in lorehaven_domain::jobs::ALL_KINDS {
            assert_eq!(
                JobKind::parse(kind.as_str()),
                Some(*kind),
                "round-trip for {:?}",
                kind
            );
        }
        assert!(known_kinds().contains(&"maintenance"));
    }

    #[test]
    fn a_job_view_does_not_offer_cancel_for_a_finished_job() {
        let job = Job {
            id: "11111111-1111-1111-1111-111111111111".to_owned(),
            kind: "maintenance".to_owned(),
            state: "succeeded".to_owned(),
            payload: "{}".to_owned(),
            idempotency_key: None,
            priority: 0,
            attempts: 1,
            max_attempts: 5,
            available_at: "2026-01-01T00:00:00Z".to_owned(),
            lease_owner: None,
            lease_expires_at: None,
            progress_permille: 1000,
            checkpoint: None,
            last_error: None,
            requested_by: None,
            created_at: "2026-01-01T00:00:00Z".to_owned(),
            updated_at: "2026-01-01T00:00:00Z".to_owned(),
            version: 2,
        };
        let view = JobView::from(job);
        assert!(!view.cancellable);
        assert_eq!(view.state, "succeeded");
    }
}
