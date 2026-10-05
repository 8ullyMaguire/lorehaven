use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::routing::get;
use axum::Json;
use chrono::Duration;
use chrono::Utc;
use lorehaven_domain::AppError;
use serde::Deserialize;
use serde_json::{json, Value};

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/admin/media-health/overview", get(media_health_overview))
        .route("/admin/media-health/link-rot", get(link_rot_report))
        .route(
            "/admin/media-health/curator-leaderboard",
            get(curator_leaderboard),
        )
        .route("/admin/media-health/bounty-status", get(bounty_status))
        .route("/admin/media-health/storage", get(storage_status))
        .route("/admin/media-health/providers", get(provider_reliability))
}

/// Refuse a caller who is not this instance's operator, without confirming that the
/// view exists.
///
/// **404, not 403.** The same rule `flows::require_operator`,
/// `admin_discovery::require_operator` and `decision_service::require_operator`
/// already follow, written down in `docs/plans/REMAINING-2026-10-03.md` (north-star
/// / flows): for an operator surface the *existence* is the disclosure, so 403
/// answers "yes, and you may not" — which tells anyone probing `/admin/media-health`
/// that this instance runs a link-rot dashboard. 404 says nothing.
///
/// This module was the outlier, and the observable consequence was that
/// `/admin/media-health` answered **403 six times** to a non-operator and rendered
/// "That did not work / access denied" on a page they are not allowed to be on. The
/// page half is fixed in `AdminMediaHealth.svelte`; this is the other half.
///
/// Note the check is against `config().administration.operator_account_id` and
/// **not** a trust-level lookup. The two modules above made the same choice for the
/// same reason: "is a trust level of 5 the operator" is a different question from
/// "is this account the operator", and the previous trust-level version here also
/// turned every database hiccup into a 403, which reads as a permissions problem
/// rather than the storage problem it is.
fn require_operator(state: &AppState, user: &crate::auth::SessionUser) -> ApiResult<()> {
    if state.config().administration.operator_account_id == Some(user.account_id) {
        return Ok(());
    }
    tracing::debug!(
        operator_configured = state.config().administration.operator_account_id.is_some(),
        "the media-health dashboard was reached by an account that is not the operator"
    );
    Err(ApiError(AppError::NotFound {
        resource: "media health",
    }))
}

/// Overall health overview for the admin dashboard (§32.7.11).
/// Returns % of references with ≥3 healthy links and trend data.
async fn media_health_overview(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    require_operator(&state, &user)?;

    let total = lorehaven_db::media_resilience::count_total_references(state.db())
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;

    let well_mirrored = lorehaven_db::media_resilience::count_well_mirrored(state.db(), 3)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;

    let below_threshold =
        lorehaven_db::media_resilience::count_references_below_threshold(state.db(), 3)
            .await
            .map_err(|e| ApiError(AppError::Internal(e)))?;

    let health_pct = if total > 0 {
        (well_mirrored as f64 / total as f64 * 100.0).round()
    } else {
        0.0
    };

    let one_week_ago = (Utc::now() - Duration::days(7)).to_rfc3339();
    let recent_rescues =
        lorehaven_db::media_resilience::count_references_below_threshold(state.db(), 3)
            .await
            .map_err(|e| ApiError(AppError::Internal(e)))?;

    Ok(Json(json!({
        "total_references": total,
        "well_mirrored": well_mirrored,
        "below_threshold": below_threshold,
        "health_pct": health_pct,
        "min_healthy_threshold": 3,
        "recent_rescues_7d": recent_rescues,
        "one_week_ago": one_week_ago,
    })))
}

/// Link rot report: how many links died per provider in the given time window.
#[derive(Debug, Deserialize)]
struct LinkRotQuery {
    since: Option<String>,
}

async fn link_rot_report(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Query(query): Query<LinkRotQuery>,
) -> ApiResult<Json<Value>> {
    require_operator(&state, &user)?;

    let since = query
        .since
        .clone()
        .unwrap_or_else(|| (Utc::now() - Duration::days(7)).to_rfc3339());

    let rot = lorehaven_db::media_resilience::link_rot_by_provider(state.db(), &since)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;

    let total_rot: i64 = rot.iter().map(|(_, c)| c).sum();

    Ok(Json(json!({
        "since": since,
        "total_rot": total_rot,
        "by_provider": rot,
    })))
}

/// Curator leaderboard: top curators by reward amount and count.
#[derive(Debug, Deserialize)]
struct LeaderboardQuery {
    limit: Option<i64>,
}

async fn curator_leaderboard(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Query(query): Query<LeaderboardQuery>,
) -> ApiResult<Json<Value>> {
    require_operator(&state, &user)?;

    let limit = query.limit.unwrap_or(10).clamp(1, 100);

    let leaders = lorehaven_db::media_resilience::curator_leaderboard(state.db(), limit)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;

    Ok(Json(json!({
        "curators": leaders,
    })))
}

/// Standing bounty status: active bounties count and total available.
async fn bounty_status(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    require_operator(&state, &user)?;

    let (active_count, total_amount) =
        lorehaven_db::media_resilience::standing_bounty_status(state.db())
            .await
            .map_err(|e| ApiError(AppError::Internal(e)))?;

    Ok(Json(json!({
        "active_bounties": active_count,
        "total_available": total_amount,
    })))
}

/// Storage usage: local mirror and IPFS pin counts.
async fn storage_status(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    require_operator(&state, &user)?;

    let (mirror_count, total_bytes) =
        lorehaven_db::media_resilience::local_mirror_storage(state.db())
            .await
            .map_err(|e| ApiError(AppError::Internal(e)))?;

    let ipfs_pins = lorehaven_db::media_resilience::count_active_ipfs_pins(state.db())
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;

    Ok(Json(json!({
        "local_mirrors": mirror_count,
        "total_bytes": total_bytes,
        "ipfs_pins": ipfs_pins,
    })))
}

/// Provider reliability ranking.
async fn provider_reliability(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    require_operator(&state, &user)?;

    let providers = lorehaven_db::media_resilience::provider_reliability(state.db())
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;

    let ranked: Vec<Value> = providers
        .iter()
        .map(|(prov, healthy, total)| {
            let rate = if *total > 0 {
                *healthy as f64 / *total as f64
            } else {
                0.0
            };
            json!({
                "provider": prov,
                "healthy": healthy,
                "total": total,
                "health_rate": (rate * 100.0).round() / 100.0,
            })
        })
        .collect();

    Ok(Json(json!({
        "providers": ranked,
    })))
}
