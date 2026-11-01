//! §58 — supply routes (M45-25).
//!
//! ```text
//! GET  /api/v1/supply/keystones?limit=10     operator: who holds the catalogue up
//! POST /api/v1/supply/recompute              operator: run the detection pass
//! GET  /api/v1/supply/droughts?limit=10      operator: their conversation position
//! POST /api/v1/supply/route                  reader:   give me one to read and comment on
//! GET  /api/v1/supply/routing                reader:   what I was given
//! POST /api/v1/supply/routing/{id}/complete  reader:   done — and the patronage pays
//! ```
//!
//! # Why the operator doors are separate from the reader doors
//!
//! A keystone list is a statement about named authors, and §58.2 forbids publishing
//! a per-work drought count — "a public drought number is a quality judgement on the
//! author published by readers". The list therefore sits behind `require_operator`,
//! and a reader's surface is the routing: their own work, their own row.
//!
//! # Why `route` answers 200 with `route: null`
//!
//! When every keystone work has already been offered to this reader, that is a
//! result, not an error: they have done the thing the surface exists for. The
//! response says so in words, because a 404 would read as "the feature is broken".

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::routes::discovery::require_operator;
use crate::state::AppState;
use darkhold_db::supply::{self, DEFAULT_MIN_PCT_BP, DEFAULT_MIN_WORKS};
use darkhold_domain::AppError;

/// The default page for both operator lists.
const DEFAULT_LIMIT: i64 = 10;

/// A page is bounded: 100 keystones is already more authors than a reader can
/// act on, and an unbounded `limit` is how a report becomes a table dump.
fn bounded(limit: Option<i64>) -> i64 {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 100)
}

#[derive(Debug, Deserialize)]
struct LimitQuery {
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct RecomputeBody {
    #[serde(default)]
    min_works: Option<i64>,
    /// Mean percentile as a fraction (`0.75`), which is how §58.1 states it.
    #[serde(default)]
    min_pct: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct RouteBody {
    /// Pin the author, or omit to take the highest-scoring keystone with a work
    /// this reader has not been offered.
    #[serde(default)]
    author: Option<String>,
}

#[derive(Debug, Serialize)]
struct RouteResponse {
    /// `null` when there is nothing left to offer — see the module docs.
    route: Option<supply::Route>,
    /// Always set, and the only explanation a caller gets.
    reason: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/supply/keystones", get(list_keystones))
        .route("/supply/recompute", post(recompute))
        .route("/supply/droughts", get(list_droughts))
        .route("/supply/route", post(route_one))
        .route("/supply/routing", get(my_routing))
        .route("/supply/routing/{id}/complete", post(complete_one))
}

async fn list_keystones(
    RequireSession(user): RequireSession,
    State(state): State<AppState>,
    Query(q): Query<LimitQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    require_operator(&state, &user)?;
    let limit = bounded(q.limit);
    let rows = supply::keystones(state.db(), limit)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(serde_json::json!({
        "keystones": rows,
        "limit": limit,
        "measured": !rows.is_empty(),
        "reason": if rows.is_empty() {
            Some("no keystone snapshot has been computed on this instance yet: POST /supply/recompute")
        } else {
            None
        },
    })))
}

async fn recompute(
    RequireSession(user): RequireSession,
    State(state): State<AppState>,
    Json(body): Json<RecomputeBody>,
) -> ApiResult<Json<supply::KeystoneRun>> {
    require_operator(&state, &user)?;
    let min_works = body.min_works.unwrap_or(DEFAULT_MIN_WORKS);
    if !(1..=25).contains(&min_works) {
        return Err(ApiError(AppError::field(
            "min_works",
            "must be between 1 and 25; a keystone is a handful of authors, not a category",
        )));
    }
    let min_pct_bp = match body.min_pct {
        None => DEFAULT_MIN_PCT_BP,
        Some(f) if (0.0..=1.0).contains(&f) => (f * 10_000.0).round() as i64,
        Some(_) => {
            return Err(ApiError(AppError::field(
                "min_pct",
                "is a fraction of the source's engaged rows, so it lies between 0 and 1",
            )))
        }
    };
    let run = supply::recompute(state.db(), min_works, min_pct_bp)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(run))
}

async fn list_droughts(
    RequireSession(user): RequireSession,
    State(state): State<AppState>,
    Query(q): Query<LimitQuery>,
) -> ApiResult<Json<supply::DroughtReport>> {
    require_operator(&state, &user)?;
    let report = supply::drought(state.db(), bounded(q.limit))
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(report))
}

async fn route_one(
    RequireSession(user): RequireSession,
    State(state): State<AppState>,
    Json(body): Json<RouteBody>,
) -> ApiResult<Json<RouteResponse>> {
    let account = user.account_id.to_string();
    let author = body
        .author
        .as_deref()
        .map(|a| a.trim().to_lowercase())
        .filter(|a| !a.is_empty());
    let routed = supply::route(state.db(), &account, author.as_deref())
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(match routed {
        Some(route) => RouteResponse {
            reason: route.reason.clone(),
            route: Some(route),
        },
        None => RouteResponse {
            route: None,
            reason: "every keystone work has already been offered to you; nothing left to route"
                .to_string(),
        },
    }))
}

async fn my_routing(
    RequireSession(user): RequireSession,
    State(state): State<AppState>,
    Query(q): Query<LimitQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let account = user.account_id.to_string();
    let rows = supply::routes_for(state.db(), &account, bounded(q.limit))
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(serde_json::json!({ "routing": rows })))
}

async fn complete_one(
    RequireSession(user): RequireSession,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<supply::Route>> {
    let account = user.account_id.to_string();
    let done = supply::complete(state.db(), &account, &id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    match done {
        Some(route) => Ok(Json(route)),
        None => Err(ApiError(AppError::NotFound {
            resource: "routing",
        })),
    }
}
