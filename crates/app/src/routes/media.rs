//! M22 — Generalized media API routes (spec §32; the redesign's §4).
//!
//! Contract skeleton: every door returns 501 NOT IMPLEMENTED until the
//! implementing phases fill bodies (R2 query engine, R3 parity, R4 archive,
//! R5 adult/audio). The paths, methods and rate classes are the contract;
//! `milestone_22.rs` pins them. When a body lands, its 501 stub is replaced
//! by the behavior — reads become public-or-eligible per §7.6, writes
//! require sessions with API scopes (§23.1).
//!
//! All media doors live under `/api/v1` (the public API surface, §23.1).
//! The M13 event-collection routes keep `/collections`; media collections
//! are `/api/v1/media-collections` — different vocabularies, no silent
//! merging (ADR 0019).

use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

use crate::state::AppState;

fn not_implemented(door: &str) -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "error": {
                "code": "NOT_IMPLEMENTED",
                "message": format!(
                    "the '{door}' media door is a contract stub; it is implemented by spec §32 R2+"
                ),            }
        })),
    )
}

// --- query doors (the same engine behind every list; spec §32.4) -----------

async fn list_media() -> impl IntoResponse {
    not_implemented("GET /api/v1/media")
}

async fn get_media(_id: Path<String>) -> impl IntoResponse {
    not_implemented("GET /api/v1/media/{id}")
}

async fn list_media_files(_id: Path<String>) -> impl IntoResponse {
    not_implemented("GET /api/v1/media/{id}/files")
}

async fn list_media_editions(_id: Path<String>) -> impl IntoResponse {
    not_implemented("GET /api/v1/media/{id}/editions")
}

async fn creator_media(_id: Path<String>) -> impl IntoResponse {
    not_implemented("GET /api/v1/creators/{id}/media")
}

async fn distributor_media(_id: Path<String>) -> impl IntoResponse {
    not_implemented("GET /api/v1/distributors/{id}/media")
}

async fn media_collection_media(_id: Path<String>) -> impl IntoResponse {
    not_implemented("GET /api/v1/media-collections/{id}/media")
}

async fn canon_media(_id: Path<String>) -> impl IntoResponse {
    not_implemented("GET /api/v1/canons/{id}/media")
}

async fn space_media(_id: Path<String>) -> impl IntoResponse {
    not_implemented("GET /api/v1/spaces/{id}/media")
}

// --- actor and collection reads --------------------------------------------

async fn list_creators() -> impl IntoResponse {
    not_implemented("GET /api/v1/creators")
}

async fn get_creator(_id: Path<String>) -> impl IntoResponse {
    not_implemented("GET /api/v1/creators/{id}")
}

async fn list_distributors() -> impl IntoResponse {
    not_implemented("GET /api/v1/distributors")
}

async fn get_distributor(_id: Path<String>) -> impl IntoResponse {
    not_implemented("GET /api/v1/distributors/{id}")
}

async fn list_media_collections() -> impl IntoResponse {
    not_implemented("GET /api/v1/media-collections")
}

async fn get_media_collection(_id: Path<String>) -> impl IntoResponse {
    not_implemented("GET /api/v1/media-collections/{id}")
}

// --- writes (complex query, actor and collection CRUD) ----------------------

async fn post_media_query() -> impl IntoResponse {
    not_implemented("POST /api/v1/media/query")
}

async fn post_creator() -> impl IntoResponse {
    not_implemented("POST /api/v1/creators")
}

async fn patch_creator(_id: Path<String>) -> impl IntoResponse {
    not_implemented("PATCH /api/v1/creators/{id}")
}

async fn post_distributor() -> impl IntoResponse {
    not_implemented("POST /api/v1/distributors")
}

async fn post_media_collection() -> impl IntoResponse {
    not_implemented("POST /api/v1/media-collections")
}

async fn put_media_collection(_id: Path<String>) -> impl IntoResponse {
    not_implemented("PUT /api/v1/media-collections/{id}")
}

/// Read doors — public rate class; eligibility is applied inside the bodies.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/media", get(list_media))
        .route("/media/{id}", get(get_media))
        .route("/media/{id}/files", get(list_media_files))
        .route("/media/{id}/editions", get(list_media_editions))
        .route("/creators", get(list_creators))
        .route("/creators/{id}", get(get_creator))
        .route("/creators/{id}/media", get(creator_media))
        .route("/distributors", get(list_distributors))
        .route("/distributors/{id}", get(get_distributor))
        .route("/distributors/{id}/media", get(distributor_media))
        .route("/media-collections", get(list_media_collections))
        .route("/media-collections/{id}", get(get_media_collection))
        .route("/media-collections/{id}/media", get(media_collection_media))
        .route("/canons/{id}/media", get(canon_media))
        .route("/spaces/{id}/media", get(space_media))
}

/// Write doors — writer rate class; sessions and scopes arrive with bodies.
pub fn write_router() -> Router<AppState> {
    Router::new()
        .route("/media/query", post(post_media_query))
        .route("/creators", post(post_creator))
        .route("/creators/{id}", axum::routing::patch(patch_creator))
        .route("/distributors", post(post_distributor))
        .route("/media-collections", post(post_media_collection))
        .route(
            "/media-collections/{id}",
            axum::routing::put(put_media_collection),
        )
}
