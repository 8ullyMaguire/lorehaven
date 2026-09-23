use crate::auth::{MaybeSession, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
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

async fn require_operator(
    state: &AppState,
    user: &crate::auth::SessionUser,
) -> Result<(), ApiError> {
    let level = lorehaven_db::governance::trust_for(state.db(), &user.account_id.to_string())
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    if level >= 5 {
        Ok(())
    } else {
        Err(ApiError(AppError::AuthRequired))
    }
}

/// Overall health overview for the admin dashboard (§32.7.11).
/// Returns % of references with ≥3 healthy links and trend data.
async fn media_health_overview(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    require_operator(&state, &user).await?;

    let total = lorehaven_db::media_resilience::count_total_references(state.db())
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    let well_mirrored = lorehaven_db::media_resilience::count_well_mirrored(state.db(), 3)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    let below_threshold =
        lorehaven_db::media_resilience::count_references_below_threshold(state.db(), 3)
            .await
            .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    let health_pct = if total > 0 {
        (well_mirrored as f64 / total as f64 * 100.0).round()
    } else {
        0.0
    };

    let one_week_ago = (Utc::now() - Duration::days(7)).to_rfc3339();
    let recent_rescues =
        lorehaven_db::media_resilience::count_references_below_threshold(state.db(), 3)
            .await
            .map_err(|e| ApiError(AppError::Internal(e.into())))?;

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
    require_operator(&state, &user).await?;

    let since = query
        .since
        .clone()
        .unwrap_or_else(|| (Utc::now() - Duration::days(7)).to_rfc3339());

    let rot = lorehaven_db::media_resilience::link_rot_by_provider(state.db(), &since)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

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
    require_operator(&state, &user).await?;

    let limit = query.limit.unwrap_or(10).clamp(1, 100);

    let leaders = lorehaven_db::media_resilience::curator_leaderboard(state.db(), limit)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    Ok(Json(json!({
        "curators": leaders,
    })))
}

/// Standing bounty status: active bounties count and total available.
async fn bounty_status(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    require_operator(&state, &user).await?;

    let (active_count, total_amount) =
        lorehaven_db::media_resilience::standing_bounty_status(state.db())
            .await
            .map_err(|e| ApiError(AppError::Internal(e.into())))?;

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
    require_operator(&state, &user).await?;

    let (mirror_count, total_bytes) =
        lorehaven_db::media_resilience::local_mirror_storage(state.db())
            .await
            .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    let ipfs_pins = lorehaven_db::media_resilience::count_active_ipfs_pins(state.db())
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

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
    require_operator(&state, &user).await?;

    let providers = lorehaven_db::media_resilience::provider_reliability(state.db())
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

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
