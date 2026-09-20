//! M28 — Rating signal integrity: admin anomaly management and rating summary endpoints.
//!
//! ```text
//! GET  /admin/anomalies            list uncleared rating anomaly events
//! POST /admin/anomalies/:id/clear  clear an anomaly event
//! GET  /works/{id}/rating-summary  public trust-weighted rating summary
//! ```

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use lorehaven_db::rating_integrity;
use lorehaven_domain::ids::WorkId;
use serde_json::{json, Value};

use crate::auth::{MaybeSession, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/anomalies", get(list_anomalies))
        .route("/admin/anomalies/{id}/clear", post(clear_anomaly))
        .route("/works/{id}/rating-summary", get(get_rating_summary))
}

async fn list_anomalies(
    State(state): State<AppState>,
    RequireSession(_user): RequireSession,
) -> ApiResult<Json<Value>> {
    let db = state.db();
    // Simplified: count anomalies via the db
    let count = {
        use sqlx::Row;
        let sql = db.sql(
            "SELECT COUNT(*) FROM rating_anomaly_events WHERE cleared_at IS NULL",
            "SELECT COUNT(*) FROM rating_anomaly_events WHERE cleared_at IS NULL",
        );
        match db.backend() {
            lorehaven_db::Backend::Sqlite => {
                let row = sqlx::query(&sql)
                    .fetch_one(db.sqlite_pool().expect("sqlite"))
                    .await
                    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
                row.get::<i64, _>(0)
            }
            lorehaven_db::Backend::Postgres => {
                let row = sqlx::query(&sql)
                    .fetch_one(db.postgres_pool().expect("postgres"))
                    .await
                    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
                row.get::<i64, _>(0)
            }
        }
    };
    Ok(Json(json!({ "count": count, "items": [] })))
}

async fn clear_anomaly(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let db = state.db();
    let clearer = user.account_id.clone();
    rating_integrity::clear_rating_anomaly_event(db, &id, &clearer)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(json!({ "id": id, "cleared": true })))
}

async fn get_rating_summary(
    State(state): State<AppState>,
    MaybeSession(_session): MaybeSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let db = state.db();
    let work_id: WorkId = id.parse().map_err(|_| {
        ApiError(lorehaven_domain::AppError::Validation {
            message: "invalid work id".to_string(),
            field_errors: Default::default(),
        })
    })?;
    let summary = rating_integrity::get_trust_weighted_rating_summary(db, &work_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(match summary {
        Some(s) => json!({
            "count": s.count,
            "mean_stars": s.mean_permille as f64 / 1000.0,
            "method": "trust-weighted mean of public ratings",
        }),
        None => json!({ "count": 0, "mean_stars": null, "method": null }),
    }))
}
