//! §53.5 — the north-star metric at `/admin/metrics/north-star`.
//!
//! This is the operator's read on whether discovery is working. The plan
//! (`docs/plans/m45-23-north-star.md` step 3) constrains it to the same three things
//! `flows.rs` and `admin_discovery.rs` already carry, and for the same reasons.
//!
//! **404, not 403.** A non-operator gets `NotFound`, because for an operator view the
//! *existence* of the view is itself a disclosure: 403 answers "yes, and you may not",
//! which tells a reader probing URLs that this instance tracks its own discovery
//! quality. The reasoning is `flows::require_operator`'s, copied rather than reinvented
//! because a subtly different rule here would be indistinguishable from an accident.
//!
//! **No per-account anything.** §53.2. `NorthStar::carries_account_detail()` asserts it at
//! the type level, and the response has no field that could carry one — the store returns
//! per-*mechanism* counts, never per-*account*.
//!
//! **No target and no grade.** §53.5: "The number is read, not chased." There is no
//! "you should be at 40%" field and nothing in this response is derived from one. §0.3
//! and the standing rule against a composite score mean there is also no single
//! north-star number to sort on: the response carries two named measures.
//!
//! **`missing_inputs` is forwarded verbatim.** §53.5 requires the metric to report its own
//! incompleteness, and that is the difference between "the recipe is failing" and "the
//! recipe has not been measured yet" — opposite responses to the same number. The route
//! does not fill in a default or hide an empty list, because a silent zero is exactly what
//! §53.5 forbids.

use axum::extract::State;
use axum::routing::get;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

use lorehaven_db::north_star as store;
use lorehaven_domain::error::AppError;
use lorehaven_domain::north_star::NorthStar;

/// The default window when the caller names none.
///
/// Thirty days back to *now*, the same default `flows.rs` uses and for the same reason: an
/// unbounded window would make every figure depend on how long the instance has been
/// running, so the same recipe would report differently on two days and a change could
/// never be attributed to a change.
const DEFAULT_WINDOW_SECONDS: i64 = 30 * 24 * 60 * 60;

/// Where a caller wants the numbers measured.
///
/// Plain RFC-3339 strings compared as text. `rating.created_at` and
/// `reading_status.updated_at` are both TEXT columns, so a string comparison is a time
/// comparison — but `recommendation_slots.created_at` is TIMESTAMPTZ, which is why the
/// store casts that side and not this one.
#[derive(Debug, Default, Deserialize)]
pub struct NorthStarQuery {
    pub since: Option<String>,
    pub until: Option<String>,
}

/// Refuse a caller who is not this instance's operator, without confirming the view exists.
///
/// 404, not 403, for the reason in the module docs. Identical to
/// `flows::require_operator`; a copy rather than a shared helper because the two routes
/// have different `resource` strings and that string is what an operator reads in a log.
fn require_operator(state: &AppState, user: &crate::auth::SessionUser) -> ApiResult<()> {
    if state.config().administration.operator_account_id == Some(user.account_id) {
        return Ok(());
    }
    tracing::debug!(
        operator_configured = state.config().administration.operator_account_id.is_some(),
        "the north-star view was reached by an account that is not the operator"
    );
    Err(ApiError(AppError::NotFound {
        resource: "north-star metric",
    }))
}

/// `GET /admin/metrics/north-star` — the feed-quality rate and its attribution.
pub async fn get_north_star(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    axum::extract::Query(query): axum::extract::Query<NorthStarQuery>,
) -> ApiResult<Json<Value>> {
    require_operator(&state, &user)?;

    let (since, until) = window(&query);
    let metric = store::north_star(state.db(), &since, &until)
        .await
        .map_err(|e| ApiError(AppError::Internal(anyhow::Error::new(e))))?;

    Ok(Json(render(&metric, &since, &until)))
}

/// Build the response body.
///
/// Kept separate from the handler so the JSON shape is one function that can be read in
/// one sitting, and so a test can render a `NorthStar` without an HTTP world around it.
fn render(metric: &NorthStar, since: &str, until: &str) -> Value {
    let rows: Vec<Value> = metric
        .by_mechanism
        .iter()
        .map(|m| {
            json!({
                "key": m.key,
                "loved_works": m.loved_works,
                "share": m.share,
            })
        })
        .collect();
    let missing: Vec<&str> = metric.missing_inputs.iter().map(|m| m.as_str()).collect();

    json!({
        "since": since,
        "until": until,
        // Two named measures and no arithmetic on them. §0.3.
        "works_rated_per_month": metric.works_rated_per_month,
        "median_days_to_find": metric.median_days_to_find,
        // The raw counts beside the rates, so a reader can check the rate rather than
        // trust it.
        "rated_works": metric.rated_works,
        "loved_works": metric.loved_works,
        "unattributed": metric.unattributed,
        "by_mechanism": rows,
        // Forwarded verbatim: §53.5 requires the response to report its own missing
        // inputs. An empty list means it measured everything.
        "missing_inputs": missing,
        "note": "Read, not chased: no target, no grade, no composite score. \
                 `unattributed` counts loved works this instance never served — an import, \
                 a sister instance, or an author's own shelf. A slot served after a \
                 rating cannot have caused it, so it claims nothing.",
    })
}

/// Resolve the requested window, or the default one.
///
/// `identity::in_seconds` rather than `chrono`, for the reason `flows.rs` gives and which
/// holds identically here: one clock and one formatter, so a bound can never disagree with
/// the timestamps it is compared against about the offset.
fn window(query: &NorthStarQuery) -> (String, String) {
    let until = query
        .until
        .clone()
        .unwrap_or_else(lorehaven_db::identity::now_rfc3339);
    let since = query
        .since
        .clone()
        .unwrap_or_else(|| lorehaven_db::identity::in_seconds(-DEFAULT_WINDOW_SECONDS));
    (since, until)
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new().route("/admin/metrics/north-star", get(get_north_star))
}
