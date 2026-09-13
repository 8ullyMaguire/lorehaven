//! Search routes: site search, in-work search, taxonomy autocomplete.
//!
//! Spec §15.4, §15.9.

use crate::auth::MaybeSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use lorehaven_db::search::{search_in_work, search_works_ast, InWorkMatch};
use lorehaven_domain::ids::WorkId;
use serde::Deserialize;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/search", get(search))
        .route("/search/in-work/{id}", get(in_work))
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    q: String,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    20
}

async fn search(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(params): Query<SearchQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let viewer_id = session.as_ref().map(|s| s.account_id.to_string());
    let results = search_works_ast(state.db(), &params.q, viewer_id.as_deref(), params.limit)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "items": results })))
}

async fn in_work(
    State(state): State<AppState>,
    MaybeSession(_session): MaybeSession,
    Path(id): Path<String>,
    Query(params): Query<SearchQuery>,
) -> ApiResult<Json<Vec<InWorkMatch>>> {
    let work_id: WorkId = id
        .parse()
        .map_err(|_| ApiError(lorehaven_domain::AppError::NotFound { resource: "work" }))?;
    let results = search_in_work(state.db(), &work_id, &params.q)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(results))
}
