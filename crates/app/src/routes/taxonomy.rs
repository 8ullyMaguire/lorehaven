//! Taxonomy routes: autocomplete, tagging, alias management.
//!
//! Spec §15.1–15.3.

use crate::auth::{RequirePseud, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use darkhold_db::taxonomy::{self, TagAssertion, TagRole, TagSource};
use darkhold_db::{collaboration, media_resilience};
use darkhold_domain::WorkId;
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
    /// The claim's role: `primary` · `secondary` · `background`.
    ///
    /// The caller may say how strongly the work leans on a tag; it may not
    /// say *where the claim came from*. That is the request's provenance, and
    /// provenance is a property of the code path that made the claim, not of
    /// the JSON it carried.
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

/// Tag a work, as one of its authors or as a curator.
///
/// **The provenance is derived, never accepted.** `source` says where a claim
/// came from, so letting the request name it would make the column a claim
/// about a claim: a session could post `{"source": "imported"}` and launder a
/// person's assertion into the importer's, or post `{"source": "curator"}` and
/// dress it in authority it does not have. The two code paths that may tag a
/// work here are the author's (this pseud is one of the work's contributors)
/// and a curator's (this account holds an active curator role); anything else
/// is refused, because the specification's fifth provenance value is not
/// "a signed-in reader" -- a reader's channel is a vote
/// (`work_tag_votes`), which feeds `confidence` and never gravity
/// (ADR 0026).
async fn tag_work(
    State(state): State<AppState>,
    RequirePseud { user, pseud_id }: RequirePseud,
    Path(id): Path<String>,
    Json(body): Json<TagWorkBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let work_id: WorkId = id
        .parse()
        .map_err(|_| ApiError(darkhold_domain::AppError::NotFound { resource: "work" }))?;

    let contributors = collaboration::contributors_for_work(state.db(), work_id)
        .await
        .map_err(|e| ApiError(darkhold_domain::AppError::Internal(e)))?;
    let source = if contributors.iter().any(|c| c.pseud_id == pseud_id) {
        TagSource::Author
    } else {
        let account_id = user.account_id.to_string();
        let is_curator = media_resilience::is_active_curator(state.db(), &account_id)
            .await
            .map_err(|e| ApiError(darkhold_domain::AppError::Internal(e.into())))?;
        if !is_curator {
            // Not an author of this work and not a curator: there is no
            // provenance value that would describe the claim honestly, so
            // there is no claim.
            return Err(ApiError(darkhold_domain::AppError::AccessDenied));
        }
        TagSource::Curator
    };

    // The role is a strength, not a provenance, so the caller may choose it --
    // but the default is the modest one: a bare tag is not the work's headline
    // description until someone says it is. (The importer and the inheritance
    // path assert `primary` when they copy a source's own tags; that is the
    // source's claim being carried over, not this route's judgement.)
    let assertion = TagAssertion {
        prominence: body.prominence.or(body.weight).unwrap_or(0),
        source,
        role: body
            .role
            .as_deref()
            .map(TagRole::parse)
            .unwrap_or(TagRole::Secondary),
    };
    let tag = taxonomy::tag_work(state.db(), &work_id.to_string(), &body.node_id, assertion)
        .await
        .map_err(|e| ApiError(darkhold_domain::AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "tag": tag })))
}
