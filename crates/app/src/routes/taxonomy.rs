//! Taxonomy routes: autocomplete, tagging, alias management.
//!
//! Spec §15.1–15.3.

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use darkhold_db::taxonomy;
use serde::Deserialize;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/taxonomy", get(autocomplete).post(create_taxonomy_node))
        .route("/taxonomy/{id}", get(get_node))
        .route("/taxonomy/aliases", post(create_taxonomy_alias))
        .route("/works/{id}/tags", post(tag_work))
}

#[derive(Debug, Deserialize)]
pub struct AutocompleteQuery {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    prefix: Option<String>,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    20
}

#[derive(Debug, Deserialize)]
pub struct CreateNodeBody {
    kind: String,
    canonical: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateAliasBody {
    alias: String,
    node_id: String,
}

#[derive(Debug, Deserialize)]
pub struct TagWorkBody {
    node_id: String,
    /// The tag's prominence: how much the work leans on it.
    ///
    /// **Prominence and not weight**, because the number was always a
    /// prominence and was called the other thing (spec §6.4).
    #[serde(default)]
    prominence: Option<i64>,
    /// The old spelling, still accepted.
    #[serde(default)]
    weight: Option<i64>,
    /// Where the claim came from: `author` · `curator` · `imported` ·
    /// `exchanged` · `inferred`.
    #[serde(default)]
    source: Option<String>,
    /// The claim's role: `primary` · `secondary` · `background`.
    #[serde(default)]
    role: Option<String>,
}

async fn autocomplete(
    State(state): State<AppState>,
    RequireSession(_): RequireSession,
    Query(params): Query<AutocompleteQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let kind = params.kind.as_deref();
    let prefix = params.prefix.as_deref().unwrap_or("");
    let nodes = taxonomy::search_nodes_fuzzy(state.db(), kind, prefix, params.limit)
        .await
        .map_err(|e| ApiError(darkhold_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "items": nodes })))
}

async fn get_node(
    State(state): State<AppState>,
    RequireSession(_): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let node = taxonomy::node_by_id(state.db(), &id)
        .await
        .map_err(|e| ApiError(darkhold_domain::AppError::Internal(e)))?;
    match node {
        Some(n) => Ok(Json(serde_json::json!({ "node": n }))),
        None => Err(ApiError(darkhold_domain::AppError::NotFound {
            resource: "node",
        })),
    }
}

async fn create_taxonomy_node(
    State(state): State<AppState>,
    RequireSession(_): RequireSession,
    Json(body): Json<CreateNodeBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let node = taxonomy::create_node(state.db(), &body.kind, &body.canonical)
        .await
        .map_err(|e| ApiError(darkhold_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "node": node })))
}

async fn create_taxonomy_alias(
    State(state): State<AppState>,
    RequireSession(_): RequireSession,
    Json(body): Json<CreateAliasBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let alias = taxonomy::create_alias(state.db(), &body.alias, &body.node_id, "author")
        .await
        .map_err(|e| ApiError(darkhold_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "alias": alias })))
}

async fn tag_work(
    State(state): State<AppState>,
    RequireSession(_): RequireSession,
    Path(id): Path<String>,
    Json(body): Json<TagWorkBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let work_id: String = id
        .parse()
        .map_err(|_| ApiError(darkhold_domain::AppError::NotFound { resource: "work" }))?;
    // **The provenance is the caller's, and an unrecognised one is not
    // stored as an unrecognised one.** A source the instance does not know
    // would be read by every surface that shows a tag, and each would have
    // its own idea of what it means.
    let source = body
        .source
        .as_deref()
        .map(darkhold_db::taxonomy::TagSource::parse)
        .unwrap_or(darkhold_db::taxonomy::TagSource::Imported);
    let role = body
        .role
        .as_deref()
        .map(darkhold_db::taxonomy::TagRole::parse)
        .unwrap_or(darkhold_db::taxonomy::TagRole::Primary);
    let assertion = darkhold_db::taxonomy::TagAssertion {
        prominence: body.prominence.or(body.weight).unwrap_or(0),
        source,
        role,
    };
    let tag = taxonomy::tag_work(state.db(), &work_id, &body.node_id, assertion)
        .await
        .map_err(|e| ApiError(darkhold_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "tag": tag })))
}
