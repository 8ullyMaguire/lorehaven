//! §53 — the operator's faucet/sink view, at `/admin/economy/flows`.
//!
//! A11's worry was inflation, and this is the surface that lets an operator see whether it
//! is happening: a balance and a composition, per §53.2. Three decisions are load-bearing and
//! each is commented where it is made.
//!
//! **The composition is in the response, not just the totals.** §53.2 asks for "a balance and
//! a composition", and a composition without its parts cannot be acted on — an operator told
//! only that 4,200 credits went unclassified cannot find them. So `mechanisms` carries every
//! key, its side, its net and whether it was declared.
//!
//! **An undeclared mechanism is shown, not hidden.** §53.1: a mechanism with no declaration is
//! a mechanism the operator cannot reason about, and a dashboard that silently omitted it
//! would report a smaller economy than exists. The store already returns such a mechanism with
//! its net intact and `Flow::Undeclared`; this route passes both through rather than
//! filtering, and reports `undeclared` as a count so the omission is impossible to miss.
//!
//! **The threshold is reported and nothing more.** §53.2 forbids an automatic throttle, and
//! §0.3 makes bought ranking and bought trust non-negotiable — a threshold that clamped would
//! be the economy deciding what a reader may earn. `over_threshold` is a boolean for a human
//! to act on. Nothing in this response is derived from it.
//!
//! Two inherited constraints, both from the sibling `admin_discovery` view and both
//! non-negotiable here: a non-operator gets **404, not 403**, because for an operator view
//! the existence is itself a disclosure; and the response carries **no per-account detail**,
//! which §53.2 forbids outright and which `FlowSummary::carries_account_detail` asserts at
//! the type level.

use axum::extract::State;
use axum::routing::get;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

use lorehaven_db::flow_store;
use lorehaven_domain::error::AppError;
use lorehaven_domain::flows::FlowSummary;

/// The default window when the caller names none.
///
/// Thirty days back, to *now*. Not "all time": an unbounded window would make every figure
/// in the response depend on how long the instance has been running, so the same economy
/// would report differently on two days and a change could never be attributed to a change.
/// Thirty days is also the shortest window an operator reads as "recent" — a week is noisy
/// around a quiet instance, a year is a different question than the one being asked.
///
/// The bounds are inclusive at both ends, matching `flow_store::mechanisms_in_window`, so a
/// caller who wants exactly this window should send exactly these bounds.
/// Seconds, not days, because `identity::in_seconds` takes seconds and the whole point of
/// using it is that the bound is computed by the same clock as the timestamps.
const DEFAULT_WINDOW_SECONDS: i64 = 30 * 24 * 60 * 60;

/// Where a caller wants the numbers measured.
///
/// `since`/`until` are plain RFC 3339 strings compared as text, which is correct for the
/// ledger's `created_at` column: it is written in one format by one function, so a string
/// comparison is a time comparison. A caller that sends a partial date (`2026-10-01`)
/// narrows the window rather than erroring, because the comparison is textual and that is
/// what the caller asked for.
#[derive(Debug, Default, Deserialize)]
pub struct FlowsQuery {
    pub since: Option<String>,
    pub until: Option<String>,
}

/// Refuse a caller who is not this instance's operator, without confirming the view exists.
///
/// The same reasoning as `admin_discovery::require_operator` and for the same reason: 403 says
/// "yes, and you may not", which tells a reader probing the URL that this instance runs an
/// economy dashboard. 404 says nothing.
fn require_operator(state: &AppState, user: &crate::auth::SessionUser) -> ApiResult<()> {
    if state.config().administration.operator_account_id == Some(user.account_id) {
        return Ok(());
    }
    tracing::debug!(
        operator_configured = state.config().administration.operator_account_id.is_some(),
        "the economy flows view was reached by an account that is not the operator"
    );
    Err(ApiError(AppError::NotFound {
        resource: "economy flows",
    }))
}

/// The threshold a window is compared against, in credits.
///
/// An operator's configured mint ceiling. `exceeds()` compares the window's **net** against
/// it, because the thing worth warning about is the economy growing, not one faucet being
/// generous. This is a constant today rather than a configuration lookup: there is no operator
/// setting for it yet, and inventing one would add a knob nothing reads. When the setting
/// arrives, this is the single line that changes.
const MINT_THRESHOLD: i64 = 50_000;

/// `GET /admin/economy/flows` — the instance economy as faucets and sinks.
pub async fn get_flows(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    axum::extract::Query(query): axum::extract::Query<FlowsQuery>,
) -> ApiResult<Json<Value>> {
    require_operator(&state, &user)?;

    let (since, until) = window(&query);
    let mechanisms = flow_store::mechanisms_in_window(state.db(), &since, &until)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;

    // `compose` rather than summing here. It already counts undeclared mechanisms and puts
    // their credits in the net while keeping them off both sides, which is exactly the rule
    // this view exists to honour — and recomputing it would be a second implementation of a
    // rule with a doc comment explaining why it is the way it is.
    let summary = FlowSummary::compose(&mechanisms);

    let rows: Vec<Value> = mechanisms
        .iter()
        .map(|m| {
            json!({
                "key": m.key,
                "flow": m.declaration.flow.as_str(),
                "net_credits": m.net_credits,
                // Straight from `Flow::is_declared`, so the flag cannot disagree with the
                // side it is shown beside.
                "declared": m.declaration.flow.is_declared(),
            })
        })
        .collect();

    Ok(Json(json!({
        "since": since,
        "until": until,
        "faucet_credits": summary.faucet_credits,
        "sink_credits": summary.sink_credits,
        "net_credits": summary.net_credits,
        "undeclared": summary.undeclared,
        "threshold": MINT_THRESHOLD,
        // Reported. Nothing below this line is derived from it, and no value above it is
        // altered because of it.
        "over_threshold": summary.exceeds(MINT_THRESHOLD),
        "mechanisms": rows,
        // The response says plainly that a number here is not a completeness claim: absence
        // of a row is not evidence that no unclassified movement exists.
        "note": "Aggregate mechanisms only. No per-account balances. An absent \
                 mechanism is not a claim that no credit moved.",
    })))
}

/// Resolve the requested window, or the default one.
fn window(query: &FlowsQuery) -> (String, String) {
    let until = query
        .until
        .clone()
        .unwrap_or_else(lorehaven_db::identity::now_rfc3339);
    // `identity::in_seconds`, not `chrono`. Two reasons, and the second is the one that
    // matters: it computes from the same clock and formats with the same function
    // `now_rfc3339` uses, so a window bound can never disagree with the timestamps it is
    // compared against about the offset. A separate `chrono` computation would be a second
    // source of "now" and a second formatter, which is the pair of things that disagree.
    let since = query
        .since
        .clone()
        .unwrap_or_else(|| lorehaven_db::identity::in_seconds(-DEFAULT_WINDOW_SECONDS));
    (since, until)
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new().route("/admin/economy/flows", get(get_flows))
}
