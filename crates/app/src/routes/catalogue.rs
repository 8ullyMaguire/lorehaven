//! The public catalogue: search the fanfiction index by source and quality.
//!
//! ```text
//! GET /catalogue?source=sb&sort=quality&q=winter&min_likes=100&limit=20&offset=0
//! ```
//!
//! # Why this route is public and unauthenticated
//!
//! Every row is metadata about a story that lives somewhere else — title, author,
//! summary, word count, and the endorsement count the source itself publishes.
//! No story text is stored here, and no reader's shelf, tag or reading status is
//! reachable through it: `library_items` is per-account in the *writer's* sense,
//! and this route reads the catalogue accounts only. The estate rule that keeps
//! the catalogue metadata-only is what makes an unauthenticated read safe; if it
//! ever stopped being metadata-only, this route would have to stop being public.
//!
//! # Why `sort=quality` is `likes`
//!
//! Because that is what the operator means by quality: the community's own
//! endorsement, normalised across sources by the crawler. `sort=aggregate` adds
//! the same story's likes from every other source plus the kudos it earned on
//! darkhold, which is what "the ones from everywhere plus the ones from here"
//! asks for. The response carries all three numbers so the UI never has to
//! pretend they are the same thing.

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use darkhold_db::catalogue::{self, CatalogueQuery, CatalogueRow, CatalogueSort, SourceCount};
use darkhold_domain::AppError;

/// A query string the route cannot honour: a `422` that names the field.
fn invalid(message: impl Into<String>) -> ApiError {
    AppError::Validation {
        message: message.into(),
        field_errors: std::collections::BTreeMap::new(),
    }
    .into()
}

/// The query string, in the shape axum reads it.
#[derive(Debug, Deserialize)]
pub struct CatalogueParams {
    source: Option<String>,
    #[serde(default)]
    min_likes: Option<i64>,
    q: Option<String>,
    sort: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    offset: Option<i64>,
}

/// What the route answers.
#[derive(Debug, Serialize)]
pub struct CataloguePage {
    items: Vec<CatalogueRow>,
    sort: String,
    limit: i64,
    offset: i64,
    /// The sources present in this page, so a UI can label the filter honestly.
    sources: Vec<String>,
}

/// Mount the catalogue's read routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/catalogue", get(search))
        .route("/catalogue/sources", get(sources))
}

async fn search(
    State(state): State<AppState>,
    Query(params): Query<CatalogueParams>,
) -> ApiResult<Json<CataloguePage>> {
    let sort = match params.sort.as_deref() {
        None => CatalogueSort::Likes,
        Some(raw) => CatalogueSort::parse(raw).ok_or_else(|| invalid(format!("unknown sort: {raw}")))?,
    };
    let limit = params.limit.unwrap_or(20);
    if !(1..=100).contains(&limit) {
        return Err(invalid("limit must be between 1 and 100"));
    }
    let query = CatalogueQuery {
        source: params.source.clone(),
        min_likes: params.min_likes,
        q: params.q.clone(),
        sort: Some(sort),
        limit: Some(limit),
        offset: Some(params.offset.unwrap_or(0)),
    };
    let items = catalogue::search(state.db(), &query).await?;
    let mut sources: Vec<String> = items.iter().map(|row| row.source_key.clone()).collect();
    sources.sort();
    sources.dedup();
    Ok(Json(CataloguePage {
        items,
        sort: sort.as_str().to_owned(),
        limit,
        offset: query.offset.unwrap_or(0),
        sources,
    }))
}

/// The distinct source keys the catalogue holds, with how many rows each has.
async fn sources(State(state): State<AppState>) -> ApiResult<Json<Vec<SourceCount>>> {
    let rows = catalogue::sources(state.db()).await?;
    Ok(Json(rows))
}
