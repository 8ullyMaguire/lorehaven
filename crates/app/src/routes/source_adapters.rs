//! M45-57 — Routes for curator-submitted source adapters (spec §55.2, §55.8).
//!
//! Four routes: submit, read the queue, read a submission's reviews, record a
//! review. All four sit behind `RequireSession` — an adapter's manifest names a
//! site it will crawl and the selectors it will use, so the queue is not public
//! reading. `MaybeSession` would return a 401 only when the handler noticed a
//! missing account, and a handler that forgets to notice is an anonymous route
//! that looks authenticated.
//!
//! **The trust gate is not here.** §55.2's TL3 bar is enforced in
//! `lorehaven_db::source_adapters::submit`, and this module's job on refusal is
//! only to render it. That is deliberate: the store is reachable from routes,
//! from background jobs and from admin tooling, and a bar enforced at the HTTP
//! edge is a bar the other two callers walk around. The consequence is that this
//! module must map the refusal to the right status code rather than letting a
//! generic error handler turn a policy decision into a 500 — see [`submit_adapter`].

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

use lorehaven_db::source_adapters::{self, SubmitError};

/// `POST /api/v1/extensions/source-adapters` — submit an adapter (§55.2).
#[derive(Debug, Deserialize)]
pub struct SubmitAdapterBody {
    /// The §21.1 extension manifest.
    pub manifest: String,
    /// The §55.3 declarative adapter manifest. Required for a `source_adapters`
    /// submission and validated here rather than at publish time, because a
    /// malformed YAML discovered during quorum review has already cost three
    /// reviewers their attention.
    #[serde(default)]
    pub source_manifest: Option<String>,
}

/// Submit a source adapter.
///
/// 401 unauthenticated (from `RequireSession`), 403 below §55.2's bar.
pub async fn submit_adapter(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<SubmitAdapterBody>,
) -> ApiResult<Json<Value>> {
    if let Some(src) = body.source_manifest.as_deref() {
        // Parse *and* compile at submission: §55.3's selectors must be valid CSS
        // and the base URL must pass §11.5's guards before three reviewers spend
        // attention on it. Both halves report every problem at once, so a
        // curator fixing selectors fixes them in one pass.
        lorehaven_scrapers::source_manifest::parse_and_compile(src).map_err(|problems| {
            ApiError(lorehaven_domain::AppError::field(
                "source_manifest",
                problems.join("; "),
            ))
        })?;
    }

    let account = user.account_id.to_string();
    let id = source_adapters::submit(
        state.db(),
        &account,
        &body.manifest,
        body.source_manifest.as_deref(),
    )
    .await
    .map_err(|e| match e {
        // A trust refusal is a policy outcome, not a fault: 403 with both levels
        // so the reader can see the bar. Mapping it to 500 would report "we
        // broke" for something that is working exactly as specified.
        SubmitError::Refused(r) => ApiError(lorehaven_domain::AppError::TrustLevelInsufficient {
            level: match &r {
                source_adapters::SubmitRefusal::BelowTrustBar { level, .. } => *level,
            },
            required: match &r {
                source_adapters::SubmitRefusal::BelowTrustBar { required, .. } => *required,
            },
        }),
        SubmitError::Query(q) => ApiError(lorehaven_domain::AppError::Internal(q.into())),
    })?;

    Ok(Json(json!({ "id": id, "state": "pending" })))
}

/// `GET /api/v1/extensions/source-adapters` — the review queue (§55.2).
pub async fn list_submissions(
    State(state): State<AppState>,
    RequireSession(_user): RequireSession,
) -> ApiResult<Json<Value>> {
    let rows = source_adapters::list_pending(state.db())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "submissions": rows })))
}

/// `GET /api/v1/extensions/source-adapters/{id}` — one submission.
pub async fn get_submission(
    State(state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let row = source_adapters::by_id(state.db(), &id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?
        .ok_or_else(|| {
            ApiError(lorehaven_domain::AppError::NotFound {
                resource: "adapter submission",
            })
        })?;
    Ok(Json(json!({ "submission": row })))
}

/// `GET /api/v1/extensions/source-adapters/{id}/reviews` — the review record.
///
/// §19.4's threshold is public information for anyone voting: a reviewer who
/// cannot see the other verdicts is reviewing blind. So this returns the whole
/// record and the current count, not just "has it passed".
pub async fn list_reviews(
    State(state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let reviews = source_adapters::reviews_for(state.db(), &id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    let approvals = source_adapters::approve_count(state.db(), &id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({
        "reviews": reviews,
        "approvals": approvals,
        "threshold": source_adapters::APPROVAL_THRESHOLD,
    })))
}

#[derive(Debug, Deserialize)]
pub struct RecordReviewBody {
    /// `approve`, `reject` or `abstain`. Validated in the store so the rule
    /// holds however many callers there are.
    pub verdict: String,
    #[serde(default)]
    pub note: Option<String>,
}

/// `POST /api/v1/extensions/source-adapters/{id}/reviews` — record a verdict.
pub async fn record_review(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Json(body): Json<RecordReviewBody>,
) -> ApiResult<Json<Value>> {
    let review_id = source_adapters::record_review(
        state.db(),
        &id,
        &user.account_id.to_string(),
        &body.verdict,
        body.note.as_deref(),
    )
    .await
    .map_err(|e| match e {
        // A verdict outside the set is a validation failure, not a fault: 422
        // naming the allowed values. Mapping it to 500 — which is what the single
        // `sqlx::Error` return type forced — told a reviewer "something went wrong
        // on our side" for a typo in their own request, and told us nothing either.
        source_adapters::ReviewError::BadVerdict { verdict } => {
            ApiError(lorehaven_domain::AppError::field(
                "verdict",
                &format!("must be approve, reject or abstain; got {verdict:?}"),
            ))
        }
        source_adapters::ReviewError::Query(q) => {
            ApiError(lorehaven_domain::AppError::Internal(q.into()))
        }
    })?;

    let approvals = source_adapters::approve_count(state.db(), &id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({
        "id": review_id,
        "approvals": approvals,
        "threshold": source_adapters::APPROVAL_THRESHOLD,
        "reached": approvals >= source_adapters::APPROVAL_THRESHOLD,
    })))
}

/// The router. Merged into `server.rs`'s `api` router, which is then nested
/// under `/api/v1` — so these paths are **relative to that nest** and must not
/// repeat it. Every other module merged there (`marketplace.rs` and its
/// `/listings`, `/extensions/{slug}`) is spelled the same way, and
/// `route_inventory::ROUTE_TABLE` stores the relative form, which its own test
/// compares against.
///
/// The first version spelled them `/api/v1/extensions/source-adapters`, which
/// registered the real paths at `/api/v1/api/v1/...` and 404'd. Nothing failed at
/// compile time; `registered_routes_are_tabled` passed, because the table and the
/// module agreed on the same wrong string. The symptom was a bare **405** on
/// every call, because `/api/v1/extensions/{slug}` from `marketplace.rs` still
/// existed and matched the requested path with the wrong method. A 405 on a route
/// you just added is the tell that the path exists and yours does not.
///
/// Every route here is in `route_inventory::ROUTE_TABLE`, whose test
/// `registered_routes_are_tabled` fails on the first omission.
pub fn router() -> axum::Router<AppState> {
    // One `.route(...)` per line on purpose: `route_inventory.rs`'s walker is
    // line-based (`if !line.contains(".route(")`) and a wrapped call collects
    // nothing, which its own `registered.is_empty()` guard then reports as a
    // broken walk. Written across lines, this module registers four routes and
    // the test reports zero and fails for the wrong reason.
    //
    // `marketplace.rs` already owns `/extensions/{slug}` as a GET. A static
    // segment outranks a parameter in axum, so `/extensions/source-adapters`
    // resolves correctly — but the parameter route stays a live collision for any
    // other method, which is why the collection path must remain its own static
    // segment rather than becoming `/extensions/{slug}/submit`.
    axum::Router::new()
        .route(
            "/extensions/source-adapters",
            post(submit_adapter).get(list_submissions),
        )
        .route("/extensions/source-adapters/{id}", get(get_submission))
        .route(
            "/extensions/source-adapters/{id}/reviews",
            get(list_reviews).post(record_review),
        )
}
