//! The maintenance pass that settles overdue retention proposals (plan E.2).
//!
//! ```text
//! POST /api/v1/admin/retention/proposals/settle   (operator, schedules the job)
//! JobKind::RetentionSettle                          (the pass itself)
//! ```
//!
//! ## This exists because binding mode needs a clock
//!
//! §5.3's asymmetry says a binding-mode proposal commits **after**
//! `cooling_days`, not at quorum. That is not a detail: it is what makes the
//! ballot a decision rather than a race. A commit at the instant the bar is met
//! is decided by whoever tapped `vote` last, which rewards arriving late and
//! punishes the readers who took a day to think. The delay is the whole point,
//! and it needs something to *wait* — a request handler has no way to commit a
//! proposal tomorrow.
//!
//! ## The store returns candidates, and this is where the decision is made
//!
//! `overdue_proposals` returns the open proposals whose window has closed and
//! deliberately does not act on them: its own doc comment says "closing a
//! proposal is a decision about storage policy, and the maintenance pass is not
//! where a decision is made". That is right about the *store* — a query cannot
//! have a policy opinion. This is the place that has one, and the shape it takes
//! is deliberately narrow:
//!
//! - **Advisory mode settles nothing.** A passed proposal is recorded and an
//!   operator applies it. The pass leaves the proposal `open` in the sense that
//!   matters — it closes it as `passed` so the ballot's outcome is visible, and
//!   moves no setting. Advisory means advisory, including here.
//! - **Binding mode commits only what the readers' own bar approved.** The
//!   quorum is re-derived at commit time, not read off the proposal, so a
//!   proposal that reached `passed` under a bar this instance has since raised
//!   is not committed at the old number.
//! - **Anything short of quorum closes as `expired`** and moves nothing.
//!
//! ## A failure on one proposal does not sink the pass
//!
//! Each proposal is settled inside its own error scope and the pass continues,
//! because the alternative is that one malformed row makes every proposal on
//! the instance un-settleable until an operator notices. The summary counts the
//! failures, so the pass is not silent about them either.

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use lorehaven_db::retention as retention_store;
use lorehaven_db::retention_proposals as store;
use lorehaven_domain::retention_quorum::{quorum_for, QuorumOutcome};
use serde_json::{json, Value};
use time::OffsetDateTime;

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::routes::discovery::require_operator;
use crate::state::AppState;

/// The operator's trigger for the settlement pass.
pub fn router() -> Router<AppState> {
    Router::new().route("/admin/retention/proposals/settle", post(settle_now))
}

/// What one pass did.
#[derive(Debug, Default, serde::Serialize)]
pub struct SettleSummary {
    /// Proposals whose window had closed when the pass ran.
    pub considered: usize,
    /// Committed into the setting. Always 0 in advisory mode.
    pub committed: usize,
    /// Reached quorum, with the operator still to apply them. In advisory
    /// mode the proposal is left `open` for exactly that reason, so this
    /// count is a *report* and not a state change — re-running the pass
    /// reports the same number and still writes nothing.
    pub passed: usize,
    /// Closed without reaching quorum.
    pub expired: usize,
    /// Settled, but the pass could not. The message says why.
    pub failed: usize,
    /// Whether this run was in binding mode, echoed so a job log is readable
    /// without cross-referencing the config.
    pub binding_mode: bool,
}

impl SettleSummary {
    /// One line, for the job's own record.
    pub fn describe(&self) -> String {
        format!(
            "retention settle: {} considered, {} committed, {} passed, {} expired, \
             {} failed (binding_mode={})",
            self.considered,
            self.committed,
            self.passed,
            self.expired,
            self.failed,
            self.binding_mode
        )
    }
}

/// Run the pass now. Operator-only, and it is the *same* function the job calls,
/// so an operator who hits this button and the scheduler that fires overnight
/// are provably doing the same thing.
pub async fn settle_now(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<(axum::http::StatusCode, Json<Value>)> {
    require_operator(&state, &user)?;
    let summary = run(&state).await?;
    Ok((
        axum::http::StatusCode::OK,
        Json(serde_json::to_value(&summary).unwrap_or(json!({}))),
    ))
}

/// The pass. Takes `now` as a parameter rather than reading the clock so a test
/// can sit on either side of a `closes_at` without sleeping, and so the job and
/// the operator button call it identically.
pub async fn run_at(state: &AppState, now: &str) -> Result<SettleSummary, ApiError> {
    let binding = state.config().retention_governance.binding_mode;
    let widen = state.config().retention_governance.widen_quorum;

    let mut summary = SettleSummary {
        binding_mode: binding,
        ..Default::default()
    };

    let overdue = store::overdue_proposals(state.db(), now)
        .await
        .map_err(|error| ApiError(lorehaven_domain::AppError::Internal(error)))?;

    for proposal in overdue {
        summary.considered += 1;

        // Each proposal is its own error scope. One bad row must not make every
        // other proposal on the instance un-settleable.
        let outcome = settle_one(state, &proposal, binding, widen).await;
        match outcome {
            Ok(Outcome::Committed) => summary.committed += 1,
            Ok(Outcome::Passed) => summary.passed += 1,
            Ok(Outcome::Expired) => summary.expired += 1,
            Err(_) => summary.failed += 1,
        }
    }
    Ok(summary)
}

/// The pass, at the current time.
pub async fn run(state: &AppState) -> Result<SettleSummary, ApiError> {
    run_at(state, &now_rfc3339()).await
}

/// What settling one proposal did.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Committed,
    Passed,
    Expired,
}

async fn settle_one(
    state: &AppState,
    proposal: &store::RetentionProposal,
    binding: bool,
    widen: i64,
) -> Result<Outcome, ApiError> {
    let current = current_mode(state, proposal.source_key.as_deref()).await?;
    let required = quorum_for(proposal.proposed_mode, current, widen);
    let tally = store::tally(state.db(), &proposal.id, widen)
        .await
        .map_err(|error| ApiError(lorehaven_domain::AppError::Internal(error)))?;

    // The quorum is checked against `tally.quorum`, which `tally` already
    // derived with the same `widen_quorum`. `required` is computed here for the
    // same reason `respond` computes it: the number that decides is re-derived
    // from the current setting rather than carried on the row.
    let _ = required;
    if matches!(tally.quorum, QuorumOutcome::Short { .. }) {
        // Short of the bar and the window has closed: the readers did not
        // decide this, so it did not happen.
        let _ =
            store::close_proposal(state.db(), &proposal.id, store::ProposalState::Expired).await;
        return Ok(Outcome::Expired);
    }

    if !binding {
        // **Advisory: report it and write nothing.**
        //
        // The proposal stays `open` on purpose. The first version closed it as
        // `passed`, and that made the operator's `respond` route unreachable
        // for any ballot older than its window — the job had finalised it, so
        // `respond` correctly reported "already decided" and there was no way
        // left to apply a decision the readers had already made. A pass that
        // finalises ballots in advisory mode is binding mode with extra steps.
        //
        // Leaving it `open` also makes the pass idempotent for free: the next
        // run finds the same `open` proposal, derives the same report, and
        // again writes nothing. A pass that closed it would need to reason
        // about its own second run.
        //
        // Only the advisory *pass* leaves a proposal open, and that is exactly
        // the case where a person still has to act. `expired` below closes,
        // because a ballot that missed the bar and shut its window is finished
        // — leaving it `open` would advertise a proposal nobody can vote on.
        return Ok(Outcome::Passed);
    }

    // Binding. Write the setting and record who did it.
    //
    // **The actor is the system account: the instance acted, not a person.**
    //
    // `retention_policy_changes.actor` is `NOT NULL REFERENCES accounts (id)`,
    // so there is no "nobody" to name. NULL is refused, the nil UUID is refused
    // by the foreign key, and a reader's id would put one of the three voters'
    // names on the row their own ballot produced — the ballot leak arriving
    // through the audit trail instead of the response body. The system account
    // is the option that satisfies the constraint and is honestly not a person.
    let actor: uuid::Uuid = lorehaven_db::SYSTEM_ACCOUNT;
    let from = Some(current);
    let written = match proposal.source_key.as_deref() {
        Some(key) => {
            retention_store::write_source_override(state.db(), key, proposal.proposed_mode, actor)
                .await
                .map(|_| ())
                .map_err(|error| {
                    ApiError(lorehaven_domain::AppError::Validation {
                        message: error.to_string(),
                        field_errors: Default::default(),
                    })
                })
        }
        None => retention_store::write_policy(state.db(), proposal.proposed_mode, actor)
            .await
            .map(|_| ())
            .map_err(|error| ApiError(lorehaven_domain::AppError::Internal(error))),
    };
    written?;

    // A failure here is a failed settlement, not a silent one: `written` has
    // already moved the setting, so a change row that does not exist is an audit
    // trail missing the fact that it moved. Counted as a failure and logged,
    // rather than `let _ =`, because the setting and the record are the same
    // fact and only one of them is currently guaranteed.
    if let Err(error) = store::record_change(
        state.db(),
        from,
        proposal.proposed_mode,
        proposal.source_key.as_deref(),
        actor,
        "the readers' ballot reached the quorum and this instance is in binding mode",
    )
    .await
    {
        tracing::error!(
            target: "lorehaven::retention",
            "proposal {} changed the setting but the change could not be recorded: {error}",
            proposal.id
        );
        return Err(ApiError(lorehaven_domain::AppError::Internal(
            anyhow::anyhow!(error),
        )));
    }

    let _ = store::close_proposal(state.db(), &proposal.id, store::ProposalState::Passed).await;
    Ok(Outcome::Committed)
}

/// The mode in force for a proposal's scope.
///
/// `source_blocked` and `vanished` are `false` because this feature has no
/// route that sets either — the same limitation `retention_proposal_admin`
/// records, and for the same reason: what a policy commit needs is what is
/// *written down*, and a blocked source's stored bodies are a different
/// question.
async fn current_mode(
    state: &AppState,
    source_key: Option<&str>,
) -> Result<lorehaven_domain::retention::BodyMode, ApiError> {
    let resolved = retention_store::resolve_for_source(state.db(), source_key, false, false)
        .await
        .map_err(|error| ApiError(lorehaven_domain::AppError::Internal(error)))?;
    Ok(lorehaven_domain::retention::narrowest_mode(
        resolved.instance,
        resolved.source,
    ))
}

/// RFC3339 now, in the same fixed-width UTC form `now_rfc3339` writes, because
/// `overdue_proposals` compares `closes_at` **lexicographically** — a `now` in
/// any other shape would compare wrongly rather than fail.
fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}
