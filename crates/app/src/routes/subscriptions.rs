//! M21 — Content subscriptions and saved-search alerts (spec §23.3, §14.2).
//!
//! Skeleton: contract routes returning `501 NOT_IMPLEMENTED`; shapes pinned
//! by `crates/app/tests/milestone_21.rs`.

use axum::extract::{Path, State};
use axum::routing::{post, put};
use axum::Json;
use serde::Deserialize;
use serde_json::Value;

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

fn todo() -> ApiError {
    ApiError(lorehaven_domain::AppError::NotImplemented)
}

#[derive(Debug, Deserialize)]
pub struct SubscribeBody {
    /// work | series | collection | fandom | author (§23.3)
    pub subject_type: String,
    pub subject_id: String,
}

/// Subscribe to a work, series, collection, fandom or author.
pub async fn subscribe(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
    Json(_body): Json<SubscribeBody>,
) -> ApiResult<Json<Value>> {
    Err(todo())
}

/// Pause or resume; a paused subscription keeps its record (§23.3).
pub async fn update_subscription(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(_id): Path<String>,
    Json(_body): Json<Value>,
) -> ApiResult<Json<Value>> {
    Err(todo())
}

pub async fn unsubscribe(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(_id): Path<String>,
) -> ApiResult<Json<Value>> {
    Err(todo())
}

/// The caller's subscriptions. Private: this is the only listing that exists.
pub async fn my_subscriptions(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
) -> ApiResult<Json<Value>> {
    Err(todo())
}

#[derive(Debug, Deserialize)]
pub struct AlertBody {
    pub saved_search_id: String,
    /// daily | weekly (§14.2 — bounded in frequency)
    pub frequency: String,
}

/// Enable an alert on a saved view.
pub async fn create_alert(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
    Json(_body): Json<AlertBody>,
) -> ApiResult<Json<Value>> {
    Err(todo())
}

pub async fn update_alert(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(_id): Path<String>,
    Json(_body): Json<Value>,
) -> ApiResult<Json<Value>> {
    Err(todo())
}

pub async fn delete_alert(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(_id): Path<String>,
) -> ApiResult<Json<Value>> {
    Err(todo())
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/subscriptions", post(subscribe).get(my_subscriptions))
        .route(
            "/subscriptions/{id}",
            put(update_subscription).delete(unsubscribe),
        )
        .route("/search-alerts", post(create_alert))
        .route(
            "/search-alerts/{id}",
            put(update_alert).delete(delete_alert),
        )
}

/// The `ai_training` author assertion (§24.14) — a stated preference shown as
/// metadata and exported with the work. Belongs to the works surface.
pub async fn set_ai_training(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(_work_id): Path<String>,
    Json(_body): Json<Value>,
) -> ApiResult<Json<Value>> {
    Err(todo())
}

pub fn ai_training_router() -> axum::Router<AppState> {
    axum::Router::new().route("/works/{work_id}/ai-training", put(set_ai_training))
}
