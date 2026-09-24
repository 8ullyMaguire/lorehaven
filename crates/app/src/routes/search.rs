//! Search routes: site search, in-work search, taxonomy autocomplete.
//!
//! Spec §15.4, §15.9.

use crate::auth::MaybeSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use lorehaven_db::search::{search_in_work, search_works_ast_filtered, InWorkMatch};
use lorehaven_db::settings as db_settings;
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
    /// Min word count filter (M47-05: applies user default if set).
    #[serde(default)]
    min_words: Option<i64>,
    /// Max word count filter (M47-05).
    #[serde(default)]
    max_words: Option<i64>,
    /// Sort order (M47-05: applies user default if set).
    #[serde(default)]
    sort: Option<String>,
    /// Rating ceiling (M47-05).
    #[serde(default)]
    max_rating: Option<String>,
}

fn default_limit() -> i64 {
    20
}

/// Resolve search defaults from the user's settings (M47-05, spec §46.4).
///
/// Explicit query parameters override stored defaults. Defaults come from the
/// user's search_settings (per-pseud, falling back to account).
async fn resolve_search_defaults(
    state: &AppState,
    session: &crate::auth::SessionUser,
    params: &SearchQuery,
) -> (Option<i64>, Option<i64>, Option<String>, Option<String>) {
    let pseud_id = session
        .pseud_id
        .as_ref()
        .map(|p| p.as_uuid())
        .unwrap_or_else(|| session.account_id.as_uuid());

    let stored = db_settings::read_search_settings(state.db(), pseud_id)
        .await
        .unwrap_or_default();
    let mut defaults_map = std::collections::HashMap::new();
    for (k, v) in &stored {
        defaults_map.insert(k.clone(), v.clone());
    }

    let min_words = params.min_words.or_else(|| {
        defaults_map.get("min_words").and_then(|v| v.as_i64())
    });
    let max_words = params.max_words.or_else(|| {
        defaults_map.get("max_words").and_then(|v| v.as_i64())
    });
    let sort = params.sort.clone().or_else(|| {
        defaults_map.get("sort").and_then(|v| v.as_str().map(String::from))
    });
    let max_rating = params.max_rating.clone().or_else(|| {
        defaults_map.get("max_rating").and_then(|v| v.as_str().map(String::from))
    });

    (min_words, max_words, sort, max_rating)
}

async fn search(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(params): Query<SearchQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let viewer_id = session.as_ref().map(|s| s.account_id.to_string());
    // M47-04: load content filters for the viewer's pseud and exclude matching works.
    let filters = if let Some(ref session) = session {
        let pseud_id = session
            .pseud_id
            .as_ref()
            .map(|p| p.as_uuid())
            .unwrap_or_else(|| session.account_id.as_uuid());
        db_settings::list_content_filters(state.db(), pseud_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|r| (r.filter_type, r.value))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    // M47-05: resolve search defaults (explicit params override stored).
    let (min_words, max_words, sort, max_rating) = match session.as_ref() {
        Some(s) => resolve_search_defaults(&state, s, &params).await,
        None => (None, None, None, None),
    };

    let results = search_works_ast_filtered(
        state.db(),
        &params.q,
        viewer_id.as_deref(),
        params.limit,
        &filters,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;

    Ok(Json(serde_json::json!({
        "items": results,
        "filters": {
            "min_words": min_words,
            "max_words": max_words,
            "sort": sort,
            "max_rating": max_rating,
        }
    })))
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
