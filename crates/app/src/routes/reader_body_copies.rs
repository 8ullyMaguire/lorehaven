//! A reader's own copy of an external body (spec §11.15b, amendment §6.2–6.4).
//!
//! **The request is a job, not a synchronous fetch.** `POST` writes a `pending`
//! copy and returns `202`; the fetch is `JobKind::BodyFetch` in `worker.rs`,
//! bounded by the source's robots posture exactly as any import is. A reader's
//! request must not be able to hold a worker open for as long as a site takes to
//! answer, and that is what §6.2 means by calling it a job.
//!
//! **The bar gates the request and nothing else.** §6.2 is explicit that a
//! request "does not create a readers'-tier around it": once a copy exists, every
//! reader eligible for the work reads the same thing. So this module checks the
//! bar exactly once, on the way in, and no read path consults it — the guard
//! test is a source scan in `crates/app/tests/config_sections.rs`, because a
//! behavioural test could only show that today's readers agree and the property
//! is about code that does not exist yet.
//!
//! **A refused request leaves no row.** A row saying "this reader asked and was
//! refused" is a record of a reader wanting bytes this instance does not hold,
//! and §6.2's audit requirement is satisfied by what a *granted* request records
//! (`retention_body_requests`), not by a trail of refusals.
//!
//! **The order is source → mode → trust**, and the middle step is deliberately
//! ahead of the last. A low-trust reader refused for their standing would never
//! be told the instance is `aggregate`, and that is a leak of the instance's
//! retention mode to someone not entitled to it. It is also true regardless of
//! who asks: no amount of trust makes an `aggregate` instance store text.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use serde_json::json;

use crate::auth::RequireSession;
use crate::state::AppState;

use super::retention_proposals::{error_response, internal};

pub(crate) fn router() -> Router<AppState> {
    Router::new().route("/works/{id}/body-request", get(status).post(request))
}

/// What `GET` answers, and what `POST` returns.
///
/// `state` and `reason_code` are the request's own story; `plain_text` is the
/// bytes, and is `None` unless the copy is `ready`. Deliberately **no
/// `account_id`**: the copy is the caller's own, and serialising whose copy it is
/// would put an account id in a response that has no reason to name one.
///
/// **No `id`.** The first draft carried `format!("{work_id}:{account_id}")` — a
/// composite handle that embedded the reader's own account id, which the privacy
/// test caught: `a_reader_cannot_read_another_readers_copy` asserts the response
/// carries no account id, and it did. The test was right and the route was
/// wrong. There is nothing here that needs an id: the work id plus the caller's
/// own session already identify the copy, and no third party has business holding
/// a handle for somebody else's request.
#[derive(Debug, Serialize)]
pub struct BodyRequestView {
    pub work_id: String,
    pub source_key: String,
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    pub requested_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settled_at: Option<String>,
    /// The bytes, when the copy is ready. The reader's own; downloadable and
    /// offline-capable because this is the text itself, not a preview or a link
    /// that expires.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plain_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sanitized_html: Option<String>,
}

/// `POST /api/v1/works/{id}/body-request` — ask for a copy.
///
/// `202` on success, because the copy is not there yet. The `Location` is the
/// `GET` on the same path, so a client that loses the response body can still
/// find it.
async fn request(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> Response {
    // 1. The work's source. No import record → refused by name, because there is
    //    no source and so no retention decision to apply.
    let Some(source_key) =
        (match lorehaven_db::reader_body_copies::source_for_work(state.db(), &id).await {
            Ok(source) => source,
            Err(error) => return internal(error),
        })
    else {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "NO_IMPORT_RECORD",
            format!("work {id} has no import record, so it has no source to fetch from"),
        );
    };

    // 2. The instance's mode for that source. Aggregate refuses by name, BEFORE
    //    the trust check, so a refused reader learns the mode rather than only
    //    their own standing.
    let resolved = match lorehaven_db::retention::resolve_for_source(
        state.db(),
        Some(&source_key),
        false,
        false,
    )
    .await
    {
        Ok(resolved) => resolved,
        Err(error) => return internal(error),
    };
    // `narrowest_mode` rather than reading `resolved.source` directly, for the
    // reason `retention_proposal_admin.rs:401` gives: the precedence between the
    // instance's mode and a source override is written down once in the domain,
    // and re-deciding it here is how two call sites start disagreeing about
    // what a source's mode is.
    let mode = lorehaven_domain::retention::narrowest_mode(resolved.instance, resolved.source);
    if mode == lorehaven_domain::retention::BodyMode::Aggregate {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "RETENTION_AGGREGATE",
            format!(
                "this instance does not store external text: source {source_key} is \
                 aggregate, so a personal copy cannot be made"
            ),
        );
    }

    // 3. The trust bar, stating both the bar and the caller's own level.
    let bar = state.config().retention.body_request_min_trust;
    let account = user.account_id.to_string();
    let trust = match lorehaven_db::governance::trust_for(state.db(), &account).await {
        Ok(trust) => trust,
        Err(error) => return internal(error),
    };
    if trust < bar {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "TRUST_TOO_LOW",
            format!(
                "trust level too low to request a personal body copy: this instance \
                 needs trust level {bar} and yours is {trust}"
            ),
        );
    }

    let chapter_key =
        match lorehaven_db::reader_body_copies::chapter_key_for_work(state.db(), &id).await {
            Ok(key) => key,
            Err(error) => return internal(error),
        };

    let copy = match lorehaven_db::reader_body_copies::request_copy(
        state.db(),
        &id,
        &user.account_id.as_uuid(),
        &source_key,
        &chapter_key,
        trust,
    )
    .await
    {
        Ok(copy) => copy,
        Err(error) => return internal(error),
    };

    // A copy that was already `ready` is not re-queued: the reader holds the
    // bytes, and a `202` promising work nobody will do is a lie. So `200` for
    // `ready` and `202` for anything the job still has to settle — and the enqueue
    // rides the same branch rather than running unconditionally. The idempotency
    // key would make a duplicate job a no-op, but not queueing work known to be
    // redundant is clearer than queueing it and relying on that.
    let status = if copy.state == lorehaven_db::reader_body_copies::CopyState::Ready {
        StatusCode::OK
    } else {
        if let Err(error) =
            super::reader_body_fetch::enqueue(&state, &copy.id, user.account_id).await
        {
            return internal(error);
        }
        StatusCode::ACCEPTED
    };
    let body = Json(json!({
        "id": copy.id,
        "work_id": copy.work_id,
        "source_key": copy.source_key,
        "state": copy.state.as_str(),
        "requested_at": copy.requested_at,
    }));
    (status, body).into_response()
}

/// `GET /api/v1/works/{id}/body-request` — the caller's own copy, or its state.
///
/// `404` when they have never asked. Not `200` with a null body: "you have no
/// copy" and "your copy is empty" are different facts and a client that cannot
/// tell them will eventually render the second as the first.
async fn status(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> Response {
    let state_of =
        lorehaven_db::reader_body_copies::status_for(state.db(), &id, &user.account_id.as_uuid())
            .await;
    let Some((copy_state, reason_code, requested_at)) = (match state_of {
        Ok(found) => found,
        Err(error) => return internal(error),
    }) else {
        return error_response(
            StatusCode::NOT_FOUND,
            "NO_BODY_REQUEST",
            format!("no body request recorded for work {id}"),
        );
    };

    // The bytes, only once the job settled the copy as ready. `body_for`
    // already filters on `state = 'ready' AND plain_text IS NOT NULL`, so a
    // pending or refused copy cannot leak a stale body through this path.
    let body = match lorehaven_db::reader_body_copies::body_for(
        state.db(),
        &id,
        &user.account_id.as_uuid(),
    )
    .await
    {
        Ok(body) => body,
        Err(error) => return internal(error),
    };

    let settled_at = body.as_ref().map(|b| b.settled_at.clone());
    let view = BodyRequestView {
        work_id: id.clone(),
        source_key: body
            .as_ref()
            .map(|b| b.source_key.clone())
            .unwrap_or_default(),
        state: copy_state.as_str(),
        reason_code,
        requested_at,
        settled_at,
        plain_text: body.as_ref().map(|b| b.plain_text.clone()),
        sanitized_html: body.and_then(|b| b.sanitized_html),
    };
    (StatusCode::OK, Json(view)).into_response()
}
