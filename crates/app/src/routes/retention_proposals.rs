//! Retention proposals: the reader-facing half of §5, amendment §5.
//!
//! ```text
//! POST /api/v1/retention/proposals                    (>= retention.proposal_min_trust)
//! GET  /api/v1/retention/proposals                    (list; tally only)
//! GET  /api/v1/retention/proposals/{id}
//! POST /api/v1/retention/proposals/{id}/vote
//! ```
//!
//! The operator half is in [`super::retention`] under `/admin`, which is the
//! shape §5.2's table gives. These four are the only surfaces a reader has, and
//! **none of them returns a ballot.** That is not a convention anyone has to
//! remember: the store's `tally` returns two integers and no row type at all,
//! so there is nothing here that *could* serialise who voted which way.
//!
//! ## Why the ballots are private at all
//!
//! §45.2 refuses weights in governance because a weighted vote lets a reading
//! habit set instance policy. The same reasoning protects a reader's *choice*,
//! and it is worth being explicit that this is a second application rather than
//! a restatement. A reader who is known to have wanted less storage stored is a
//! reading of their habits and their library, which §19.2 exists to refuse —
//! arriving by the opposite route from a weight. A weight changes what a ballot
//! *counts*; a leak changes what a ballot *reveals*. Both are ways of letting
//! the record of a vote shape what an instance does, and one is a schema column
//! and the other is a response body.
//!
//! So: counts yes, rows never. A reader sees how many people support a change —
//! which is what they need in order to decide whether to join — and never who.
//!
//! ## Two error types, and why
//!
//! The two read-only handlers return `ApiError`; the two mutating ones return
//! `Response`. The split is not tidiness. A refusal here can be `409` — "a
//! ballot is already open on this setting" — and `AppError`'s only `409` is
//! `RevisionConflict`, whose message is "this resource changed since you opened
//! it". That is false here, and an error body that misdescribes what happened
//! is worse than a wrong status code. So the mutating handlers build their own
//! responses, and `trust_refusal` returns the *message* rather than a result so
//! that each caller chooses how to render it: one place decides whether a
//! reader may act, and no call site can quietly downgrade a conflict to a bad
//! request.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use lorehaven_db::retention_proposals as store;
use lorehaven_domain::error::AppError;
use lorehaven_domain::retention::BodyMode;
use lorehaven_domain::retention_quorum::QuorumOutcome;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

/// The reader-facing proposal routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/retention/proposals", get(list).post(create))
        .route("/retention/proposals/{id}", get(one))
        .route("/retention/proposals/{id}/vote", post(vote))
}

/// `None` when the reader may act; `Some(message)` when they may not.
///
/// **Returns a message rather than a `Result`**, for the reason in the module
/// header: the mutating handlers render a refusal as a `Response` and the
/// read-only ones as an `ApiError`, so the decision and the rendering have to
/// be separable. One place computes "may this reader act"; five call sites
/// decide how to say so.
///
/// The message names both the bar and the caller's own level. "trust level too
/// low" is true and useless — a reader who is told what would be enough can
/// decide whether to wait, ask, or go elsewhere. The plan's test name,
/// `a_reader_below_the_bar_cannot_open_or_file_a_proposal_and_is_told_the_bar`,
/// makes the second clause part of the requirement rather than a nicety.
async fn trust_refusal(
    state: &AppState,
    user: &crate::auth::SessionUser,
    action: &str,
) -> Result<Option<String>, String> {
    let bar = state.config().retention_governance.proposal_min_trust;
    let account = user.account_id.to_string();
    let trust = lorehaven_db::governance::trust_for(state.db(), &account)
        .await
        .map_err(|error| error.to_string())?;
    Ok((trust < bar).then(|| {
        format!(
            "trust level too low to {action}: this instance needs trust level {bar} \
             to take part in retention decisions and yours is {trust}"
        )
    }))
}

/// The body `http::ApiError` produces, built by hand for the statuses it cannot
/// express.
///
/// `ErrorEnvelope`/`ErrorBody` are private to `http`, and making them public to
/// serve one route is a worse trade than these six lines. **The two shapes must
/// agree** — the tests assert `error.code` and `error.message` on refusals from
/// both paths, so a divergence fails rather than reaching a client.
fn error_response(status: StatusCode, code: &str, message: String) -> Response {
    (
        status,
        axum::Json(json!({
            "error": { "code": code, "message": message, "field_errors": {} }
        })),
    )
        .into_response()
}

/// A `500`, for a storage fault.
///
/// Spelled once and used by every `map_err` that is a fault rather than a
/// refusal, so the two kinds cannot be confused at a call site: a refusal goes
/// through `bad_request` or `refusal_response` and carries the reader's own
/// words, and only a genuine fault lands here.
fn internal(error: impl std::fmt::Display) -> Response {
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "INTERNAL",
        error.to_string(),
    )
}

/// A `422` for a message the caller must fix.
///
/// **422, not 400**, because that is what `AppError::Validation` answers
/// everywhere else in this codebase — roadmap.rs, the admin retention routes,
/// every other validation refusal. These hand-built responses exist only
/// because the two mutating handlers need a 409 the enum cannot express, and
/// they should not have introduced a second spelling of the same rule: one
/// gate answering 422 on the read path and 400 on the write path is not a
/// distinction a client can act on.
fn unprocessable(message: impl Into<String>) -> Response {
    error_response(
        StatusCode::UNPROCESSABLE_ENTITY,
        "VALIDATION_FAILED",
        message.into(),
    )
}

/// Turn a store refusal into a response.
///
/// `409` for "one is already open", `422` for everything else, decided by the
/// store's message rather than by a second parallel classification. That is not
/// elegant — matching on a string is fragile — and it is still the better of the
/// two available shapes: the alternative is an error *enum* threaded through the
/// store for four refusals, and a string matched in one place and asserted in
/// one test is a smaller thing to get wrong than an enum a fifth refusal will
/// not carry. A rewording that changes the intent fails that test rather than
/// silently becoming a `422`.
///
/// An unrecognised message becomes a `400`, which is the safe direction: a
/// mis-classified *conflict* is a confusing error, and a mis-classified
/// validation failure is not a data problem.
fn refusal_response(message: &str) -> Response {
    if message.to_ascii_lowercase().contains("already open") {
        error_response(StatusCode::CONFLICT, "CONFLICT", message.to_owned())
    } else {
        unprocessable(message.to_owned())
    }
}

/// The body for opening a proposal.
#[derive(Debug, Deserialize)]
struct CreateBody {
    /// The mode being proposed, as a string.
    ///
    /// A `String` parsed by hand rather than a `BodyMode`, for the reason the
    /// admin routes already give: serde's error for an unknown variant names
    /// the type and the field, and a reader who typed `aggregte` deserves to be
    /// told it is `aggregate` or `cache`.
    proposed_mode: String,
    /// The setting it applies to. Absent means the instance-wide setting.
    #[serde(default)]
    source_key: Option<String>,
    /// Why. Required; the store refuses an empty one.
    rationale: String,
}

/// The body for casting a ballot.
#[derive(Debug, Deserialize)]
struct VoteBody {
    support: bool,
}

/// The proposal, as a reader sees it. **No ballot fields exist here, and the
/// hand-written projection is the reason.**
///
/// Not `serde_json::to_value(&proposal)`, because a derive serialises whatever
/// the store's type grows next — and the store's type is one `pub` field away
/// from holding an `account_id` the response must not have. Naming every field
/// makes the privacy property a *type* rather than a review item: a field added
/// to the store cannot appear in a response that does not mention it.
///
/// `opened_by` is deliberately absent too, and for a second reason: naming every
/// author of every proposal would turn a governance surface into a list of who
/// cares about storage. §19.15 says a vote grants the proposer nothing personal,
/// and the flip side is that the proposer is not made visible either.
async fn as_json(state: &AppState, proposal: &store::RetentionProposal) -> ApiResult<Value> {
    let tally = store::tally(
        state.db(),
        &proposal.id,
        state.config().retention_governance.widen_quorum,
    )
    .await
    .map_err(|error| ApiError(AppError::Internal(error.into())))?;

    let (quorum, further_needed) = match tally.quorum {
        QuorumOutcome::Reached {
            required,
            supporters,
        } => (
            json!({ "reached": true, "required": required, "supporters": supporters }),
            Some(0),
        ),
        QuorumOutcome::Short {
            required,
            supporters,
            needed,
        } => (
            json!({ "reached": false, "required": required, "supporters": supporters }),
            Some(needed),
        ),
    };

    Ok(json!({
        "id": proposal.id,
        "proposed_mode": proposal.proposed_mode.as_str(),
        "source_key": proposal.source_key,
        "rationale": proposal.rationale,
        "closes_at": proposal.closes_at,
        "state": proposal.state.as_str(),
        "tallied_at": proposal.tallied_at,
        "created_at": proposal.created_at,
        "version": proposal.version,
        // Counts, never rows. See the module header.
        "tally": {
            "supporters": tally.supporters,
            "opposed": tally.opposed,
            "quorum": quorum,
            "further_needed": further_needed,
        }
    }))
}

/// List every proposal, newest first. **Tally only.**
async fn list(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    if let Some(message) = trust_refusal(&state, &user, "see retention proposals")
        .await
        .map_err(|error| ApiError(AppError::Internal(anyhow::anyhow!(error))))?
    {
        return Err(ApiError(AppError::Validation {
            message,
            field_errors: std::collections::BTreeMap::new(),
        }));
    }
    let proposals = store::list_proposals(state.db())
        .await
        .map_err(|error| ApiError(AppError::Internal(error.into())))?;
    // Sequential, not `join_all`: a governance list is tens of rows at most
    // and one connection is simpler than a pool of futures whose error type has
    // to be reconciled anyway.
    let mut items = Vec::with_capacity(proposals.len());
    for proposal in &proposals {
        items.push(as_json(&state, proposal).await?);
    }
    Ok(Json(json!({ "items": items })))
}

/// One proposal. **Tally only.**
async fn one(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    if let Some(message) = trust_refusal(&state, &user, "see a retention proposal")
        .await
        .map_err(|error| ApiError(AppError::Internal(anyhow::anyhow!(error))))?
    {
        return Err(ApiError(AppError::Validation {
            message,
            field_errors: std::collections::BTreeMap::new(),
        }));
    }
    let proposal = store::proposal(state.db(), &id)
        .await
        .map_err(|error| ApiError(AppError::Internal(error.into())))?
        .ok_or(ApiError(AppError::NotFound {
            resource: "retention proposal",
        }))?;
    Ok(Json(as_json(&state, &proposal).await?))
}

/// Open a proposal.
async fn create(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<CreateBody>,
) -> Result<(StatusCode, Json<Value>), Response> {
    if let Some(message) = trust_refusal(&state, &user, "open a retention proposal")
        .await
        .map_err(|error| internal(error))?
    {
        return Err(unprocessable(message));
    }

    let Some(proposed_mode) = parse_mode(&body.proposed_mode) else {
        return Err(unprocessable(format!(
            "`{}` is not a body mode. This instance stores bodies in `cache` and \
             keeps only metadata in `aggregate`",
            body.proposed_mode
        )));
    };

    // The window is the configured cooling period, stored as a timestamp on the
    // proposal rather than recomputed from a day count at read time — so an
    // operator changing `proposal_cooling_days` does not silently move the
    // deadline of a ballot somebody is already voting in.
    let cooling = state.config().retention_governance.proposal_cooling_days;
    let closes_at = (time::OffsetDateTime::now_utc() + time::Duration::days(cooling))
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(internal)?;

    let proposal = store::create_proposal(
        state.db(),
        body.source_key.as_deref(),
        proposed_mode,
        &body.rationale,
        user.account_id.as_uuid(),
        &closes_at,
    )
    .await
    .map_err(|error| refusal_response(&error.to_string()))?;

    match as_json(&state, &proposal).await {
        Ok(json) => Ok((StatusCode::CREATED, Json(json))),
        // `as_json`'s only failure is a storage fault, which `ApiResult` would
        // turn into a 500 anyway; the same shape keeps the two error types from
        // meeting at a `?`.
        Err(ApiError(AppError::Internal(error))) => Err(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL",
            error.to_string(),
        )),
        Err(other) => Err(unprocessable(other.to_string())),
    }
}

/// Cast or change a ballot.
///
/// Always `200`, including for a reader changing their mind. The store's
/// upsert decides whether a ballot was created or replaced and the route does
/// not report which, because "created" for a changed vote would make a client's
/// idempotency bookkeeping lie about a thing that already existed.
async fn vote(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Json(body): Json<VoteBody>,
) -> Result<(StatusCode, Json<Value>), Response> {
    if let Some(message) = trust_refusal(&state, &user, "vote on a retention proposal")
        .await
        .map_err(|error| internal(error))?
    {
        return Err(unprocessable(message));
    }
    store::cast_vote(state.db(), &id, user.account_id.as_uuid(), body.support)
        .await
        .map_err(|error| refusal_response(&error.to_string()))?;
    let proposal = store::proposal(state.db(), &id)
        .await
        .map_err(internal)?
        .ok_or_else(|| {
            error_response(
                StatusCode::NOT_FOUND,
                "NOT_FOUND",
                "retention proposal not found".to_owned(),
            )
        })?;
    match as_json(&state, &proposal).await {
        Ok(json) => Ok((StatusCode::OK, Json(json))),
        Err(ApiError(AppError::Internal(error))) => Err(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL",
            error.to_string(),
        )),
        Err(other) => Err(unprocessable(other.to_string())),
    }
}

/// Parse a mode, naming the two legal values in the refusal.
fn parse_mode(value: &str) -> Option<BodyMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "cache" => Some(BodyMode::Cache),
        "aggregate" => Some(BodyMode::Aggregate),
        _ => None,
    }
}
