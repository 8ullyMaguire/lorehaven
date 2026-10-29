//! `GET /api/v1/taxonomy/export` — the vocabulary as a versioned document (T2.1).
//!
//! Public and read-only: it is the naming authority's public face, and the importer on the other site has no
//! session here. (`/taxonomy` itself is the reader-facing autocomplete and needs a session; this is the
//! document, which is a different thing.)
//!
//! The shape is the contract: Ascend reads `version` to decide whether to re-import, `items` to build its own
//! ids, and `aliases` to resolve the names people actually typed.

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/taxonomy/export", get(export))
}

#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    pub kind: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct AliasOut {
    pub entity_id: String,
    pub alias: String,
    pub norm: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExportBody {
    /// Changes when the vocabulary changes. An importer compares it and skips the rest when it matches.
    pub version: String,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
    pub kind: Option<String>,
    pub items: Vec<darkhold_db::taxonomy_export::EntityRow>,
    pub aliases: Vec<AliasOut>,
}

async fn export(
    // `MaybeSession` is here for the route table's sake, not for this handler's: darkhold's contract of
    // record requires every route to name the extractor its audience means, and a route that takes no
    // session extractor at all cannot be declared `Public` — `Public` IS `MaybeSession`. An anonymous
    // caller is unaffected (the value is ignored), which is exactly what the declaration says.
    _maybe: crate::auth::MaybeSession,
    State(state): State<AppState>,
    Query(query): Query<ExportQuery>,
) -> ApiResult<Json<ExportBody>> {
    let limit = query.limit.unwrap_or(500);
    if !(1..=500).contains(&limit) {
        return Err(ApiError(darkhold_domain::AppError::field(
            "limit",
            "must be between 1 and 500",
        )));
    }
    let offset = query.offset.unwrap_or(0);
    if offset < 0 {
        return Err(ApiError(darkhold_domain::AppError::field(
            "offset",
            "must not be negative",
        )));
    }
    if let Some(kind) = query.kind.as_deref() {
        if !matches!(kind, "tag" | "character" | "relationship" | "fandom") {
            return Err(ApiError(darkhold_domain::AppError::field(
                "kind",
                "must be one of tag, character, relationship, fandom",
            )));
        }
    }

    let version = darkhold_db::taxonomy_export::version(state.db())
        .await
        .map_err(|e| ApiError(darkhold_domain::AppError::internal("taxonomy export", e)))?;
    let (items, total) =
        darkhold_db::taxonomy_export::page(state.db(), query.kind.as_deref(), limit, offset)
            .await
            .map_err(|e| ApiError(darkhold_domain::AppError::internal("taxonomy export", e)))?;
    // Aliases ride along with the first page only: repeating them on every page would triple the document.
    let aliases = if offset == 0 {
        darkhold_db::taxonomy_export::aliases(state.db())
            .await
            .map_err(|e| ApiError(darkhold_domain::AppError::Internal(e.into())))?
            .into_iter()
            .map(|(entity_id, alias, norm)| AliasOut {
                entity_id,
                alias,
                norm,
            })
            .collect()
    } else {
        Vec::new()
    };

    Ok(Json(ExportBody {
        version,
        total,
        limit,
        offset,
        kind: query.kind,
        items,
        aliases,
    }))
}
