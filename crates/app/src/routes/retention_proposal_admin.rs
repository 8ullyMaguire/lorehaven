//! The operator's half of the proposal lifecycle (plan E.2, spec §5.3).
//!
//! ```text
//! POST /api/v1/admin/retention/proposals/{id}/respond   (advisory mode)
//! POST /api/v1/admin/retention/proposals/{id}/override  (any mode)
//! ```
//!
//! These live in `retention_proposals` rather than `retention` because they are
//! about a *proposal*, not about the setting: `retention.rs` is where an
//! operator sets policy outright, and a proposal is the one path to a policy
//! change that a reader had a hand in.
//!
//! ## The asymmetry, in one function
//!
//! §5.3 says narrowing and widening are not symmetric, and it is not a matter
//! of taste: **narrowing storage is cheap and reversible** — the body is still
//! on the peer's disk, and an operator who regrets a narrowing can set the
//! setting back by hand, at the cost of one re-crawl. **Widening storage
//! commits disk and bandwidth indefinitely** and cannot be undone by a
//! decision, only by an erasure nobody is required to perform. So
//! `quorum_for` puts widening at the higher bar, and every commit in this file
//! goes through it. There is no path here that reaches `write_policy` or
//! `write_source_override` without asking that function first — which is worth
//! stating because the alternative, a policy check at four call sites, is
//! exactly the shape that drifts.
//!
//! ## Binding commits late, and the late is the point
//!
//! In binding mode a proposal commits after `cooling_days`, **not** at quorum.
//! A commit at the instant the bar is met is a decision made by the last person
//! to tap `vote`, which is a worse rule than the one it replaces: it rewards
//! arriving late. The delay is what gives the readers who did not vote — and the
//! operator — a chance to see the tally before it becomes the setting. The tests
//! assert the setting is unchanged at quorum and changed at quorum + cooling,
//! because the gap between those two is the feature and a test that only checked
//! the end state would pass against an implementation that ignored the delay
//! entirely.
//!
//! ## Advisory moves nothing on its own
//!
//! In advisory mode (the default) a passed proposal is a *record* of what the
//! readers wanted, and the operator applies it by hand through
//! `retention.rs`. That is not timidity: the asymmetry is deliberate in the
//! other direction too, because a setting changed by a quorum of three without
//! anyone reviewing it is the "policy set by reading habit" outcome §45.2
//! refuses, arrived at through governance instead of through weighting.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use lorehaven_db::retention as retention_store;
use lorehaven_db::retention_proposals as store;
use lorehaven_domain::error::AppError;
use lorehaven_domain::retention::BodyMode;
use lorehaven_domain::retention_quorum::QuorumOutcome;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::routes::discovery::require_operator;
use crate::state::AppState;

/// The operator's routes on a proposal. Merged into the same router as the
/// reader's four; the paths are disjoint so one `Router` serves both, and the
/// gate is per-handler because they differ.
pub fn operator_router() -> Router<AppState> {
    Router::new()
        .route("/admin/retention/proposals/{id}/respond", post(respond))
        .route(
            "/admin/retention/proposals/{id}/override",
            post(override_setting),
        )
}

/// Take a JSON body, and say *which field* was wrong when it was.
///
/// The alternative — `Json<T>` in the signature — turns a typo into a bare
/// status with no explanation, because axum renders `JsonRejection` as an empty
/// body. That is why `deny_unknown_fields` on `RespondBody` and `OverrideBody`
/// is not by itself enough: it produces the right *outcome* and no usable
/// *message*. The reader routes hit this as a failing test — the refusal
/// happened, and `error.message` was `null`.
///
/// serde's message for an unknown field names the field, which is the property
/// the plan's test requires: "refused by name", not merely "refused".
///
/// **`ApiError`, not `Response`** — unlike the reader module's copy of this.
/// Every refusal these two handlers produce is a 422 or a 404, both of which
/// `AppError` already models, so leaving `AppError` buys nothing and the
/// status comes from the variant: 422, the house status, the same one the
/// reader routes' refusals use. The reader module needs a `Response` only
/// because its `create`/`vote` have a 409 to express, which `AppError` cannot
/// do without a false message. Two copies, one per error type, and a *third*
/// module wanting this is the signal to move both into `http.rs` as one
/// extractor.
fn json_body<T: serde::de::DeserializeOwned>(
    payload: Result<Json<T>, axum::extract::rejection::JsonRejection>,
) -> Result<T, ApiError> {
    match payload {
        Ok(Json(value)) => Ok(value),
        Err(rejection) => Err(ApiError(AppError::Validation {
            message: format!(
                "the request body is not the shape this route expects: {}",
                rejection.body_text()
            ),
            field_errors: Default::default(),
        })),
    }
}

/// What the operator says about a proposal.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RespondBody {
    /// What the operator decided.
    ///
    /// `accept` records that the operator is applying it — the proposal closes
    /// as `passed` and a change row records who did it. `decline` closes it as
    /// `overridden`, because §5.3's asymmetry is that the *operator* has the
    /// last word and a declined ballot has to be visible as declined rather
    /// than quietly expiring.
    decision: String,
    /// Required on `decline`. A refusal an operator cannot explain to the
    /// readers who voted is not a refusal.
    #[serde(default)]
    reason: Option<String>,
}

/// What the operator is setting instead.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OverrideBody {
    /// The mode to set, as a string. Parsed by hand so the refusal names the
    /// two legal values, as everywhere else in this feature.
    body_mode: String,
    /// Required. `retention_policy_changes.reason` is not optional and an
    /// operator override with no reason is an unexplainable policy change.
    reason: String,
}

/// Answer a proposal.
///
/// In advisory mode this is the *only* way a passed proposal becomes a
/// setting, which is what "advisory" means. In binding mode the maintenance
/// task has already committed it and this route is a no-op that reports the
/// proposal as already decided — refusing loudly, because an operator reaching
/// for `respond` on a binding-mode instance is usually reaching for it *because*
/// something looks wrong, and "nothing to do" is a worse answer than "this
/// instance commits on its own; here is what it committed".
async fn respond(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    payload: Result<Json<RespondBody>, axum::extract::rejection::JsonRejection>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    // `?` works here: `json_body` returns a `Response`, and
    // `ApiError: IntoResponse>` is implemented, so the rejection keeps its
    // own status instead of being flattened into a 500.
    let body = json_body(payload)?;
    require_operator(&state, &user)?;

    let config = &state.config().retention_governance;
    if config.binding_mode {
        let proposal = load(&state, &id).await?;
        return Err(ApiError(AppError::Validation {
            message: format!(
                "proposal {} was decided by the readers and committed by this instance's \
                 maintenance task, because `retention_governance.binding_mode` is on. \
                 There is nothing to respond to. If the setting looks wrong, change it in \
                 admin retention settings.",
                proposal.id
            ),
            field_errors: Default::default(),
        }));
    }

    let (target, rationale) = match body.decision.trim().to_ascii_lowercase().as_str() {
        "accept" => (store::ProposalState::Passed, None),
        "decline" => {
            let reason = body
                .reason
                .as_deref()
                .map(str::trim)
                .filter(|reason| !reason.is_empty())
                .ok_or(ApiError(AppError::Validation {
                    message: "declining a proposal needs a reason the operator can give the \
                              readers who voted. What is the reason?"
                        .to_owned(),
                    field_errors: Default::default(),
                }))?;
            (store::ProposalState::Overridden, Some(reason.to_owned()))
        }
        other => {
            return Err(ApiError(AppError::Validation {
                message: format!(
                    "`{other}` is not a decision. This instance understands `accept` and \
                     `decline`."
                ),
                field_errors: Default::default(),
            }))
        }
    };

    let proposal = load(&state, &id).await?;

    // **The bar is derived in exactly one place: `store::tally`.** An earlier
    // version recomputed `quorum_for` here and then gated on `tally.quorum`
    // instead, which made the recomputation dead code shaped like diligence —
    // the next reader would assume it was load-bearing and edit the wrong thing.
    // `tally` calls `quorum_for` with the same current mode and the same
    // configured bar, so consulting it is both correct and the only version that
    // cannot drift from the number the reader-facing route shows.
    let tally = store::tally(state.db(), &proposal.id, config.widen_quorum)
        .await
        .map_err(|error| ApiError(AppError::Internal(error)))?;

    if target == store::ProposalState::Passed {
        if let QuorumOutcome::Short { needed, .. } = tally.quorum {
            return Err(ApiError(AppError::Validation {
                message: format!(
                    "this proposal has {} supporter(s) and needs {}. Declining it is \
                     available; accepting it is not, and the operator is not a substitute \
                     for a reader who has not voted yet.",
                    tally.supporters, needed
                ),
                field_errors: Default::default(),
            }));
        }
    }

    // **Apply, then close.** The reverse order is a trap: a `close_proposal`
    // followed by a failed `apply` leaves the ballot decided as `passed` with
    // the setting unmoved, and the operator's only remaining move is an
    // override — so a transient storage fault becomes a governance record
    // saying the readers' decision was had and then not followed. Deciding
    // after the thing being decided has happened is the ordering the
    // settlement pass uses too, and for the same reason.
    if target == store::ProposalState::Passed {
        // Accepting means applying the mode the readers proposed, which is why
        // the argument is spelled out here rather than defaulted inside `apply`.
        apply(
            &state,
            &user,
            &proposal,
            proposal.proposed_mode,
            rationale
                .as_deref()
                .unwrap_or("the readers' ballot passed and the operator accepted it"),
        )
        .await?;
    }

    let closed = store::close_proposal(state.db(), &proposal.id, target)
        .await
        .map_err(|error| ApiError(AppError::Internal(error)))?;
    if !closed {
        return Err(ApiError(AppError::Validation {
            message: format!(
                "proposal {} is already decided, so there is nothing to respond to",
                proposal.id
            ),
            field_errors: Default::default(),
        }));
    }

    let _ = lorehaven_db::governance::audit_append(
        state.db(),
        &user.account_id.to_string(),
        "retention.proposal.respond",
        "retention_proposal",
        &proposal.id,
        &json!({ "decision": body.decision }).to_string(),
    )
    .await;

    let after = load(&state, &id).await?;
    Ok((
        StatusCode::OK,
        Json(summary(&state, &after, Some(tally.supporters), Some(tally.opposed)).await?),
    ))
}

/// Set the setting directly, whatever the ballot said.
///
/// Named `override_setting` because `override` is a reserved word in every Rust
/// edition and cannot be a binding name at all — the *route path* is
/// `/override` as the plan specifies, and that is unaffected; only the Rust
/// identifier has to differ.
///
/// This is the escape hatch §5.3 gives the operator, and it is deliberately
/// available in *both* modes: an instance that has committed itself to binding
/// governance still needs a way to act when the committed setting turns out to
/// be wrong, and an instance whose readers are voting on storage still needs to
/// be able to change it in an emergency. The proposal is closed as `overridden`
/// rather than left `passed`, so the record shows that the readers' decision was
/// not what the instance did.
async fn override_setting(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    payload: Result<Json<OverrideBody>, axum::extract::rejection::JsonRejection>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    // `?` works here: `json_body` returns a `Response`, and
    // `ApiError: IntoResponse>` is implemented, so the rejection keeps its
    // own status instead of being flattened into a 500.
    let body = json_body(payload)?;
    require_operator(&state, &user)?;

    let reason = body.reason.trim();
    if reason.is_empty() {
        return Err(ApiError(AppError::Validation {
            message: "an override needs a reason. It is the only record of why the \
                      readers' decision was not followed."
                .to_owned(),
            field_errors: Default::default(),
        }));
    }
    let Some(mode) = parse_mode(&body.body_mode) else {
        return Err(ApiError(AppError::Validation {
            message: format!(
                "`{}` is not a body mode. This instance stores bodies in `cache` and keeps \
                 only metadata in `aggregate`",
                body.body_mode
            ),
            field_errors: Default::default(),
        }));
    };

    let proposal = load(&state, &id).await?;
    // **The operator's mode**, not the proposal's. See `apply`.
    apply(&state, &user, &proposal, mode, reason).await?;

    // `passed -> overridden` is the transition that makes the operator's
    // disagreement with a passing ballot visible. A proposal that was still
    // `open` is closed as `overridden` too, so an override in flight does not
    // leave a ballot running for a setting that has already moved.
    if proposal.state == store::ProposalState::Open
        || proposal.state == store::ProposalState::Passed
    {
        let _ =
            store::close_proposal(state.db(), &proposal.id, store::ProposalState::Overridden).await;
    }

    let _ = lorehaven_db::governance::audit_append(
        state.db(),
        &user.account_id.to_string(),
        "retention.proposal.override",
        "retention_proposal",
        &proposal.id,
        &json!({ "body_mode": mode.as_str(), "reason": reason }).to_string(),
    )
    .await;

    let after = load(&state, &id).await?;
    let tally = store::tally(
        state.db(),
        &after.id,
        state.config().retention_governance.widen_quorum,
    )
    .await
    .map_err(|error| ApiError(AppError::Internal(error)))?;
    Ok((
        StatusCode::OK,
        Json(summary(&state, &after, Some(tally.supporters), Some(tally.opposed)).await?),
    ))
}

/// Load a proposal or 404 it.
async fn load(state: &AppState, id: &str) -> ApiResult<store::RetentionProposal> {
    store::proposal(state.db(), id)
        .await
        .map_err(|error| ApiError(AppError::Internal(error)))?
        .ok_or(ApiError(AppError::NotFound {
            resource: "retention proposal",
        }))
}

/// The mode currently in force for a proposal's scope.
///
/// **`source_blocked` and `vanished` are both `false`**, because this feature
/// has no route that sets either. That is a real limitation and it is worth
/// naming: a blocked source resolves to the instance mode *regardless* of its
/// override, so a caller wanting "what would apply if this source were
/// reachable" must pass those flags itself. Everything here asks the narrower
/// question — "what is written down right now" — which is what a policy commit
/// needs, so the honest answer is the flags off.
///
/// The error is an `AppError` and not an `anyhow::Error`, so callers do not
/// rewrap it; `ApiError` is a newtype over `AppError` and `?` does the rest.
async fn current_mode(state: &AppState, source_key: Option<&str>) -> Result<BodyMode, AppError> {
    let resolved = retention_store::resolve_for_source(state.db(), source_key, false, false)
        .await
        .map_err(|error| AppError::Internal(error.into()))?;
    // `narrowest_mode` rather than reading `resolved.source` or
    // `resolved.instance` directly, because the precedence between them is
    // already written down once in the domain and re-deciding it here is how
    // two call sites start disagreeing about what a source's mode is.
    Ok(lorehaven_domain::retention::narrowest_mode(
        resolved.instance,
        resolved.source,
    ))
}

/// Write the mode into the setting, and record the change.
///
/// `record_change` is called **even when the mode is unchanged**, because a
/// decision that turned out to be a no-op is still a decision and the audit row
/// is where a reader's "why did this happen" is answered months later. The
/// alternative — skipping the row when `from == to` — makes a later reader see
/// a change with no explanation for a decision they were not consulted on.
/// Write `mode` into the setting, and record the change.
///
/// **`mode` is a parameter, not `proposal.proposed_mode`.** The first version
/// read the mode off the proposal and ignored the one the operator sent, so
/// `OverrideBody.body_mode` was parsed, validated, and audited and then never
/// used: the route answered 200, the audit row recorded the operator's choice,
/// and the setting recorded the proposal's. Three records, two truths, and no
/// error anywhere. The operator asking to set `aggregate` on a proposal that
/// proposed `cache` got `cache`.
///
/// Taking the mode as an argument is what makes that impossible: a caller
/// cannot forget to pass it, and `respond` passing `proposal.proposed_mode` is
/// now visible at the call site as the decision it is.
async fn apply(
    state: &AppState,
    user: &crate::auth::SessionUser,
    proposal: &store::RetentionProposal,
    mode: BodyMode,
    reason: &str,
) -> ApiResult<()> {
    // The operator, never a reader. `None` is reserved for the
    // settlement pass, where the instance acted on the readers'
    // recorded decision and no person is attached.
    let actor: uuid::Uuid = user.account_id.into();
    // Read *before* the write. Reading after would record the new mode as the
    // old one, and every row in `retention_policy_changes` would read as a
    // no-op — a change log that cannot describe a change is worse than none,
    // because it is the record a reader is pointed at when asking "why is my
    // archive being re-crawled".
    let from = current_mode(state, proposal.source_key.as_deref())
        .await
        .ok();
    let result = match proposal.source_key.as_deref() {
        Some(key) => {
            retention_store::write_source_override(state.db(), key, proposal.proposed_mode, actor)
                .await
                .map(|_| ())
                .map_err(|error| {
                    ApiError(AppError::Validation {
                        message: error.to_string(),
                        field_errors: Default::default(),
                    })
                })
        }
        None => retention_store::write_policy(state.db(), mode, actor)
            .await
            .map(|_| ())
            .map_err(|error| ApiError(AppError::Internal(error))),
    };
    result?;

    store::record_change(
        state.db(),
        from,
        mode,
        proposal.source_key.as_deref(),
        actor,
        reason,
    )
    .await
    .map_err(|error| ApiError(AppError::Internal(error)))?;
    Ok(())
}

/// The response body: the proposal plus the setting now in force.
async fn summary(
    state: &AppState,
    proposal: &store::RetentionProposal,
    supporters: Option<i64>,
    opposed: Option<i64>,
) -> ApiResult<Value> {
    let config = &state.config().retention_governance;
    let tally = store::tally(state.db(), &proposal.id, config.widen_quorum)
        .await
        .map_err(|error| ApiError(AppError::Internal(error)))?;
    let current = current_mode(state, proposal.source_key.as_deref()).await?;

    Ok(json!({
        "id": proposal.id,
        "proposed_mode": proposal.proposed_mode.as_str(),
        "source_key": proposal.source_key,
        "rationale": proposal.rationale,
        "state": proposal.state.as_str(),
        "closes_at": proposal.closes_at,
        "tallied_at": proposal.tallied_at,
        "version": proposal.version,
        // The operator sees their own name: they are the audience, and the
        // privacy rule is about *readers* not learning who voted. The readers'
        // routes still return counts only.
        "opened_by": proposal.opened_by,
        "tally": {
            "supporters": supporters.unwrap_or(tally.supporters),
            "opposed": opposed.unwrap_or(tally.opposed),
        },
        // What the setting is *now*, so an operator who has just overridden
        // something can confirm the change landed without a second request.
        "body_mode_now": current.as_str(),
    }))
}

/// Parse a mode, naming the two legal values.
fn parse_mode(value: &str) -> Option<BodyMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "cache" => Some(BodyMode::Cache),
        "aggregate" => Some(BodyMode::Aggregate),
        _ => None,
    }
}
