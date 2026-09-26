//! M29 — Recommendation transparency: "why am I seeing this" explanations, the
//! private attention report, and the tag-wrangling queue (spec §33.3).
//!
//! ```text
//! GET  /discovery/slots/{slot_id}/explanation       why a recommendation appears
//! GET  /me/attention-report                        private reading attention report
//! PUT  /me/attention-report                        turn the report on or off
//! POST /admin/tag-wrangling/proposals              propose a tag merge/alias
//! GET  /admin/tag-wrangling/proposals              list pending proposals
//! POST /admin/tag-wrangling/proposals/{id}/approve  approve a proposal
//! POST /admin/tag-wrangling/proposals/{id}/revert   undo an approved merge
//! GET  /tag-wrangling/log                          public log of applied merges
//! ```
//!
//! The explanation is a read of the recorded slot, never a recomputation. See
//! `lorehaven_db::recommendation_slots`'s header for why: `time_decay_strategy`
//! reads the clock inside its scoring query, so a replay would explain a
//! different ranking than the one the reader received.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use lorehaven_db::recommendation_slots as slots;
use lorehaven_domain::governance::{TL_REGULAR, TL_STEWARD};
use lorehaven_domain::recommendation_transparency::{
    validate_proposal, AttentionReport, SlotExplanation, WranglingKind,
};

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/discovery/slots/{slot_id}/explanation", get(explain_slot))
        .route("/me/attention-report", get(get_attention_report))
        .route(
            "/me/attention-report",
            axum::routing::put(set_attention_report),
        )
        .route("/admin/tag-wrangling/proposals", post(propose_wrangling))
        .route(
            "/admin/tag-wrangling/proposals",
            get(list_wrangling_proposals),
        )
        .route(
            "/admin/tag-wrangling/proposals/{id}/approve",
            post(approve_wrangling),
        )
        .route(
            "/admin/tag-wrangling/proposals/{id}/revert",
            post(revert_wrangling),
        )
        .route("/tag-wrangling/log", get(public_wrangling_log))
}

/// The caller's trust level, defaulting to `TL_NEW` when it cannot be read.
///
/// Defaulting down rather than up: an unreadable trust row must not be a
/// standing to rewrite the taxonomy.
async fn caller_trust(state: &AppState, account: &str) -> i64 {
    lorehaven_db::governance::trust_for(state.db(), account)
        .await
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Slot explanations
// ---------------------------------------------------------------------------

/// Why this slot was served.
///
/// The reader's own pseud scopes the query, so another reader's slot id is a
/// miss rather than a disclosure. The absence of a pseud is also a miss: a slot
/// belongs to the pseud that received it, and §3.3 prefers 404 to revealing
/// that a private object exists.
async fn explain_slot(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(slot_id): Path<String>,
) -> ApiResult<Json<SlotExplanation>> {
    let Some(pseud_id) = user.pseud_id else {
        return Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "recommendation slot",
        }));
    };
    match slots::explain_slot(state.db(), pseud_id.into(), &slot_id).await {
        Ok(Some(explanation)) => Ok(Json(explanation)),
        // Not this reader's slot, or no such slot: both are 404, and the door
        // does not distinguish them, so the id cannot be probed for existence.
        Ok(None) => Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "recommendation slot",
        })),
        Err(e) => Err(ApiError(lorehaven_domain::AppError::Internal(e.into()))),
    }
}

// ---------------------------------------------------------------------------
// Attention report (§33.3(b))
// ---------------------------------------------------------------------------

/// The reader's attention report.
///
/// Disabled returns 200 with `enabled: false` and no lines, which is the
/// spec's "stays disabled until the reader enables it" rather than an error:
/// asking is how a client discovers the preference.
async fn get_attention_report(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<AttentionReport>> {
    let Some(pseud_id) = user.pseud_id else {
        return Ok(Json(AttentionReport::disabled()));
    };
    let enabled = slots::attention_enabled(state.db(), pseud_id.into())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    if !enabled {
        return Ok(Json(AttentionReport::disabled()));
    }
    let lines = slots::attention_lines(state.db(), pseud_id.into())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    // `build` returns None when disabled and guarantees a held-back line when
    // enabled, so the §33.3(b) criterion holds here rather than in the handler
    // where a future edit could drop it.
    Ok(Json(
        AttentionReport::build(true, lines).unwrap_or_else(AttentionReport::disabled),
    ))
}

#[derive(Debug, Deserialize)]
pub struct AttentionPreference {
    pub enabled: bool,
}

async fn set_attention_report(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<AttentionPreference>,
) -> ApiResult<Json<Value>> {
    let Some(pseud_id) = user.pseud_id else {
        return Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "pseud",
        }));
    };
    slots::set_attention_enabled(state.db(), pseud_id.into(), body.enabled)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "enabled": body.enabled })))
}

// ---------------------------------------------------------------------------
// Tag wrangling (§33.3(c))
// ---------------------------------------------------------------------------

/// Turn a wrangling transition failure into the status it deserves.
///
/// A proposal that is already decided is a conflict, not a bad request and
/// certainly not an internal error: the client should not retry it, and telling
/// it to would be advice that can never help.
fn wrangle_error(e: slots::WrangleError) -> ApiError {
    use lorehaven_db::recommendation_slots::WrangleError;
    match e {
        WrangleError::NotFound => ApiError(lorehaven_domain::AppError::NotFound {
            resource: "wrangling proposal",
        }),
        WrangleError::WrongState(_) | WrangleError::NoTarget(_) => {
            ApiError(lorehaven_domain::AppError::RevisionConflict {
                expected: 0,
                actual: 0,
            })
        }
    }
}

/// Recover the typed wrangling error from the `anyhow` wrapper the DB layer
/// returns, and fall back to internal for anything genuinely unexpected.
fn wrangle_error_from_anyhow(e: anyhow::Error) -> ApiError {
    match e.downcast::<slots::WrangleError>() {
        Ok(typed) => wrangle_error(typed),
        Err(other) => ApiError(lorehaven_domain::AppError::Internal(other)),
    }
}

#[derive(Debug, Deserialize)]
pub struct WranglingProposalBody {
    pub kind: String,
    pub from_node_id: String,
    /// A canonical rename has no target, so this is optional.
    pub to_node_id: Option<String>,
    pub reason: String,
}

async fn propose_wrangling(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<WranglingProposalBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    // §19.1: proposing a taxonomy rewrite takes TL_REGULAR or better.
    let trust = caller_trust(&state, &user.account_id.to_string()).await;
    if trust < TL_REGULAR {
        return Err(ApiError(lorehaven_domain::AppError::AccessDenied));
    }

    // An unknown kind is a validation failure, not a silent default: falling
    // back to `alias` would let a typo propose something other than was asked.
    let Some(kind) = WranglingKind::parse(&body.kind) else {
        return Err(ApiError(lorehaven_domain::AppError::Validation {
            message: format!("unknown proposal kind {:?}", body.kind),
            field_errors: Default::default(),
        }));
    };
    let to = body.to_node_id.as_deref();
    validate_proposal(kind, &body.from_node_id, to, &body.reason).map_err(|message| {
        ApiError(lorehaven_domain::AppError::Validation {
            message,
            field_errors: Default::default(),
        })
    })?;

    // A proposal is *authored* by a pseud (`proposed_by` is a pseud FK, like
    // every other authored thing) while the trust that permitted it is the
    // account's, because §19.1 gates on the account. Both are recorded: the
    // reviewer needs to know who wrote it and on whose standing it was made.
    let pseud = user
        .pseud_id
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "pseud" }))?;
    let id = slots::propose_wrangling(
        state.db(),
        kind,
        &body.from_node_id,
        to,
        &body.reason,
        pseud.into(),
        trust,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "status": "pending" })),
    ))
}

#[derive(Debug, Deserialize)]
pub struct PageQuery {
    pub limit: Option<i64>,
}

/// The moderation queue.
///
/// Pending proposals are a moderation surface, not a personal one, so it is
/// gated at steward level rather than session level. §33.3's rule is that no
/// *proposal or vote* appears in another **user's** surface — a reader never
/// sees this list.
async fn list_wrangling_proposals(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Query(q): Query<PageQuery>,
) -> ApiResult<Json<Value>> {
    let trust = caller_trust(&state, &user.account_id.to_string()).await;
    if trust < TL_STEWARD {
        return Err(ApiError(lorehaven_domain::AppError::AccessDenied));
    }

    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let items = slots::list_pending_wrangling(state.db(), limit)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "items": items })))
}

async fn approve_wrangling(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let trust = caller_trust(&state, &user.account_id.to_string()).await;
    if trust < TL_STEWARD {
        return Err(ApiError(lorehaven_domain::AppError::AccessDenied));
    }
    let pseud = user
        .pseud_id
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "pseud" }))?;
    slots::approve_wrangling(state.db(), &id, pseud.into(), trust)
        .await
        .map_err(wrangle_error_from_anyhow)?;
    Ok(Json(json!({ "id": id, "status": "approved" })))
}

async fn revert_wrangling(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let trust = caller_trust(&state, &user.account_id.to_string()).await;
    if trust < TL_STEWARD {
        return Err(ApiError(lorehaven_domain::AppError::AccessDenied));
    }
    let pseud = user
        .pseud_id
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "pseud" }))?;
    slots::revert_wrangling(state.db(), &id, pseud.into(), trust)
        .await
        .map_err(wrangle_error_from_anyhow)?;
    Ok(Json(json!({ "id": id, "status": "reverted" })))
}

/// §19.12's public log: what has been merged, and what was undone.
///
/// Open to unauthenticated readers, because the point of a public log is that
/// it is public. Approver trust is not included — the log says a merge happened,
/// not who had the standing to have done it.
async fn public_wrangling_log(
    State(state): State<AppState>,
    Query(q): Query<PageQuery>,
) -> ApiResult<Json<Value>> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let items = slots::public_wrangling_log(state.db(), limit)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "items": items.iter().map(|p| json!({
        "id": p.id,
        "kind": p.kind.as_str(),
        "from_node_id": p.from_node_id,
        "to_node_id": p.to_node_id,
        "reason": p.reason,
        "status": p.status,
        "created_at": p.created_at,
    })).collect::<Vec<_>>() })))
}
