//! Gap C step 5 — the author-facing pre-read report route (§32.6, §23.7).
//!
//! The store and the adapter exist; this is what an author actually looks at. Three
//! routes, and each of them has a §32.6 or §23.7 requirement behind its shape:
//!
//!   * `GET /works/{work_id}/preread` — the report, **author-only**.
//!   * `DELETE /works/{work_id}/preread/{provider}` — withdraw one provider's output.
//!   * `GET /works/{work_id}/preread/providers` — what has assessed this work, so the
//!     author can see what they are consenting to before consenting.
//!
//! **§32.6 is the constraint that shapes all three**: a pre-read report is shown to the
//! author, never on the public work page, and never as a composite number. That is why
//! there is no `score` field in any response and no way to compute one — the responses
//! carry per-dimension entries, sorted worst-first, and the `missing` list with each
//! dimension's reason.
//!
//! **Why a non-owner gets a 404 and not a 403.** The 404 is deliberate and is the same
//! indistinguishability the monetization routes use: a work the caller does not own and a
//! work that does not exist must produce the same response, or the existence of an
//! unpublished draft is confirmed to anyone who asks. This matters more than usual here,
//! because a pre-read report is an *assessment of a draft* — confirming it exists tells an
//! outsider that the draft exists and that its author used an AI tool on it.
//!
//! The consequence to keep in mind when reading the handlers: **there is no separate
//! "report not found" 404.** A caller who owns the work and has no report gets a 200 with
//! `report: null`, because they are entitled to know their own work has not been assessed.
//! Only an outsider gets the indistinguishable 404.

use crate::auth::RequirePseud;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use lorehaven_db::preread_store;
use lorehaven_domain::AppError;
use serde_json::json;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/works/{work_id}/preread", get(get_preread))
        .route(
            "/works/{work_id}/preread/providers",
            get(get_preread_providers),
        )
        .route(
            "/works/{work_id}/preread/{provider}",
            axum::routing::delete(forget_preread_provider),
        )
}

/// The work, if the caller owns it. `None` for both "no such work" and "not yours".
///
/// One function so the two cannot drift: three routes all depend on this being the same
/// check, and a fourth route with a subtly different one is how a draft's report leaks.
async fn owned_work(
    state: &AppState,
    pseud: lorehaven_domain::ids::PseudId,
    work_id: &lorehaven_domain::WorkId,
) -> ApiResult<lorehaven_db::content::Work> {
    let work = lorehaven_db::content::find_work(state.db(), *work_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "work" }))?;
    // Same indistinguishable 404 as monetization: a 403 here would confirm the draft exists,
    // and a pre-read report is an assessment of an unpublished draft.
    if work.owner_pseud_id != pseud {
        return Err(ApiError(AppError::NotFound { resource: "work" }));
    }
    Ok(work)
}

/// Parse a path work id. An unparseable id is a 404 rather than a 400 — the same choice
/// monetization makes, and for the same reason: a 400 would confirm the id was almost a
/// valid work.
fn parse_work_id(raw: &str) -> ApiResult<lorehaven_domain::WorkId> {
    raw.parse::<lorehaven_domain::WorkId>()
        .map_err(|_| ApiError(AppError::NotFound { resource: "work" }))
}

/// Which providers have assessed this work.
///
/// Shown *before* a report, so the author sees what exists before consenting to it. Empty
/// is a real answer, not an error — most works are never assessed.
async fn get_preread_providers(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(work_id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let work_id = parse_work_id(&work_id)?;
    let _work = owned_work(&state, pseud_id, &work_id).await?;
    let providers = preread_store::providers_for(state.db(), &work_id.as_uuid().to_string())
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    Ok(Json(json!({ "providers": providers })))
}

/// The current report for the first provider that has one.
///
/// **Why "the first provider" rather than a query parameter.** §23.7 makes a report
/// provider-specific, and an author is normally served by one provider. Making the
/// provider explicit in the path (as the withdrawal route does) rather than defaulting
/// here would be marginally cleaner, but the default is what the editor page needs and a
/// sorted `providers_for` makes the choice deterministic rather than arbitrary. The
/// response names the provider it came from, so a client never has to guess.
async fn get_preread(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(work_id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let work_id = parse_work_id(&work_id)?;
    let _work = owned_work(&state, pseud_id, &work_id).await?;

    let providers = preread_store::providers_for(state.db(), &work_id.as_uuid().to_string())
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    let Some(provider) = providers.first() else {
        // The owner of a work with no report gets a 200 and `report: null`. They are
        // entitled to know their own draft has not been assessed; only an outsider gets
        // the indistinguishable 404 above.
        return Ok(Json(json!({
            "report": null,
            "providers": [],
            // Spelled out rather than left for the client to infer, because "no report" and
            // "a report with nothing in it" must not render identically to an author
            // deciding whether to trust the output.
            "reason": "no_provider_has_assessed_this_work",
        })));
    };

    let report = preread_store::report_for(state.db(), &work_id.as_uuid().to_string(), provider)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;

    Ok(Json(match report {
        None => json!({
            "report": null,
            "providers": providers,
            // A provider is listed but its row is gone. Only reachable through concurrent
            // withdrawal, and named rather than reported as "no report" so the two are not
            // confused.
            "reason": "provider_listed_but_report_absent",
        }),
        Some(report) => json!({
            "provider": provider,
            "providers": providers,
            "report": {
                "work_id": report.work_id,
                // Per-dimension, worst first. The author's question is "what is weakest",
                // so ascending would answer the one they did not ask.
                "dimensions": report
                    .ranked()
                    .iter()
                    .map(|d| json!({
                        "dimension": d.dimension,
                        "score": d.score,
                        "note": d.note,
                    }))
                    .collect::<Vec<_>>(),
                // **The reason each missing dimension is missing**, not just its absence.
                // A report where everything came back and one where half the provider's
                // output was unparseable look identical without this list, and the
                // difference is exactly what tells an author whether to trust the score.
                "missing": report
                    .missing
                    .iter()
                    .map(|(dimension, status)| json!({
                        "dimension": dimension,
                        "reason": missing_reason(status),
                    }))
                    .collect::<Vec<_>>(),
                "complete": report.is_complete(),
            },
            // §32.6 in the response itself. No `score` key exists, and this comment is why
            // one must not be added: a composite is not displayed here, and §0.3 would
            // forbid it becoming a ranking signal. The per-dimension entries above are the
            // whole report.
        }),
    }))
}

/// Withdraw one provider's output for this work.
///
/// §23.7: a reader or author may opt out of **specific** providers, so this is scoped to
/// one provider and never deletes another's report. Returns the count so a client can tell
/// a withdrawal that happened from one that did not, instead of silently showing the same
/// screen twice.
async fn forget_preread_provider(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path((work_id, provider)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let work_id = parse_work_id(&work_id)?;
    let _work = owned_work(&state, pseud_id, &work_id).await?;
    let removed =
        preread_store::forget_provider(state.db(), &work_id.as_uuid().to_string(), &provider)
            .await
            .map_err(|e| ApiError(AppError::Internal(e)))?;
    let remaining = preread_store::providers_for(state.db(), &work_id.as_uuid().to_string())
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    Ok(Json(json!({
        "removed": removed,
        "providers": remaining,
    })))
}

/// A missing dimension's reason, as something an author can read.
///
/// The stored value is a variant name (§23.7's task categories, not free text), so each
/// arm says what actually happened rather than echoing the name. An unrecognised name
/// falls through to a truthful "a provider declined" rather than being reported as a
/// specific cause.
fn missing_reason(status: &lorehaven_domain::preread::DimensionStatus) -> String {
    use lorehaven_domain::ai::AiAbstain;
    use lorehaven_domain::preread::DimensionStatus;
    match status {
        DimensionStatus::Scored => "scored".to_string(),
        DimensionStatus::NotConfigured => "the dimension was not asked about".to_string(),
        DimensionStatus::Abstained(AiAbstain::NoConsent) => {
            "private-text consent was not given".to_string()
        }
        DimensionStatus::Abstained(AiAbstain::NotConfigured) => {
            "the provider was not configured".to_string()
        }
        DimensionStatus::Abstained(AiAbstain::InvalidOutput(_)) => {
            "the provider returned output that could not be validated".to_string()
        }
        _ => "a provider declined".to_string(),
    }
}
