//! M21 — Content subscriptions and saved-search alerts (spec §23.3, §14.2).
//!
//! Implements the subscription, alert, and AI-training routes by delegating
//! to `lorehaven_db::subscriptions` repository functions. All write routes
//! require an authenticated session with a selected pseud.

use axum::extract::{Path, State};
use axum::routing::{post, put};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use lorehaven_db::subscriptions;

#[derive(Debug, Deserialize)]
pub struct SubscribeBody {
    /// work | series | collection | fandom | author (§23.3)
    pub subject_type: String,
    pub subject_id: String,
}

/// Subscribe to a work, series, collection, fandom or author.
pub async fn subscribe(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<SubscribeBody>,
) -> ApiResult<Json<Value>> {
    let pseud = user.pseud_id.ok_or_else(|| {
        ApiError(lorehaven_domain::AppError::field(
            "pseud_id",
            "a pseud must be selected to subscribe",
        ))
    })?;
    let id = subscriptions::subscribe_work(
        state.db(),
        &pseud.to_canonical_string(),
        &body.subject_type,
        &body.subject_id,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "id": id, "status": "subscribed" })))
}

/// Pause or resume; a paused subscription keeps its record (§23.3).
pub async fn update_subscription(
    State(state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let state_val = body
        .get("state")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ApiError(lorehaven_domain::AppError::field("state", "must be 'active' or 'paused'"))
        })?;
    if state_val != "active" && state_val != "paused" {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "state",
            "must be 'active' or 'paused'",
        )));
    }
    let rows = subscriptions::set_subscription_state(state.db(), &id, state_val)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    if rows == 0 {
        return Err(ApiError(lorehaven_domain::AppError::NotFound { resource: "subscription" }));
    }
    Ok(Json(json!({ "updated": rows })))
}

pub async fn unsubscribe(
    State(state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let rows = subscriptions::delete_alert(state.db(), &id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "deleted": rows })))
}

/// The caller's subscriptions. Private: this is the only listing that exists.
pub async fn my_subscriptions(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    let pseud = user.pseud_id.ok_or_else(|| {
        ApiError(lorehaven_domain::AppError::field(
            "pseud_id",
            "a pseud must be selected to list subscriptions",
        ))
    })?;
    let rows = subscriptions::list_subscriptions(state.db(), &pseud.to_canonical_string())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    let subs: Vec<Value> = rows
        .into_iter()
        .map(|r| json!({
            "id": r.id,
            "subject_type": r.subject_type,
            "subject_id": r.subject_id,
            "state": r.state,
            "created_at": r.created_at,
        }))
        .collect();
    Ok(Json(json!({ "subscriptions": subs })))
}

#[derive(Debug, Deserialize)]
pub struct AlertBody {
    pub saved_search_id: String,
    /// daily | weekly (§14.2 — bounded in frequency)
    pub frequency: String,
}

/// Enable an alert on a saved view.
pub async fn create_alert(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<AlertBody>,
) -> ApiResult<Json<Value>> {
    let pseud = user.pseud_id.ok_or_else(|| {
        ApiError(lorehaven_domain::AppError::field(
            "pseud_id",
            "a pseud must be selected to create an alert",
        ))
    })?;
    let id = subscriptions::create_alert(
        state.db(),
        &pseud.to_canonical_string(),
        &body.saved_search_id,
        &body.frequency,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "id": id, "status": "created" })))
}

pub async fn update_alert(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let frequency = body
        .get("frequency")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ApiError(lorehaven_domain::AppError::field("frequency", "must be 'daily' or 'weekly'"))
        })?;
    if frequency != "daily" && frequency != "weekly" {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "frequency",
            "must be 'daily' or 'weekly'",
        )));
    }
    // delete + recreate to update the frequency (ON CONFLICT upsert).
    let rows = subscriptions::delete_alert(state.db(), &id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    if rows == 0 {
        return Err(ApiError(lorehaven_domain::AppError::NotFound { resource: "alert" }));
    }
    let pseud = user.pseud_id.ok_or_else(|| {
        ApiError(lorehaven_domain::AppError::field(
            "pseud_id",
            "a pseud must be selected to update an alert",
        ))
    })?;
    let new_id = subscriptions::create_alert(
        state.db(),
        &pseud.to_canonical_string(),
        body
            .get("saved_search_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ApiError(lorehaven_domain::AppError::field("saved_search_id", "required"))
            })?,
        frequency,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "id": new_id })))
}

pub async fn delete_alert(
    State(state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let rows = subscriptions::delete_alert(state.db(), &id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "deleted": rows })))
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
    State(state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(work_id): Path<String>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let pref = body
        .get("ai_training")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ApiError(lorehaven_domain::AppError::field("ai_training", "must be 'allow' or 'deny'"))
        })?;
    let opt_in = match pref {
        "allow" => true,
        "deny" => false,
        _ => {
            return Err(ApiError(lorehaven_domain::AppError::field(
                "ai_training",
                "must be 'allow' or 'deny'",
            )));
        }
    };
    // Resolve work to owner pseud via content.
    let wid = work_id
        .parse::<lorehaven_domain::WorkId>()
        .map_err(|_| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;
    let work = lorehaven_db::content::find_work(state.db(), wid)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;
    let _author_pseud = lorehaven_db::identity::find_pseud(state.db(), work.owner_pseud_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "author" }))?;
    if opt_in {
        subscriptions::ai_training_opt_in(state.db(), &work.owner_pseud_id.to_canonical_string(), true)
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    } else {
        subscriptions::ai_training_opt_out(state.db(), &work.owner_pseud_id.to_canonical_string())
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    }
    Ok(Json(json!({ "ai_training": pref, "work_id": work_id })))
}

pub fn ai_training_router() -> axum::Router<AppState> {
    axum::Router::new().route("/works/{work_id}/ai-training", put(set_ai_training))
}
