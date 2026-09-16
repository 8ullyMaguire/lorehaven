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

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::response::Response as AxumResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use lorehaven_domain::media::{CollectionKind, CreatorKind, DistributorKind};
use lorehaven_domain::query::QueryAst;
use serde::Deserialize;
use serde_json::json;
use std::str::FromStr;

use crate::auth::{MaybeSession, RequireSession};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Helper: query parsing
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Request bodies for write doors
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct PatchCreatorRequest {
    pub name: Option<String>,
    pub role: Option<String>,
}

#[derive(Deserialize)]
pub struct PutCollectionRequest {
    pub title: Option<String>,
    pub description: Option<String>,
}

// ---------------------------------------------------------------------------
// Read doors (the same engine behind every list; spec §32.4)
// ---------------------------------------------------------------------------

async fn list_media(
    State(state): State<AppState>,
    Query(params): Query<MediaQuery>,
    MaybeSession(session): MaybeSession,
) -> impl IntoResponse {
    let db = state.db();
    let query = QueryAst::Text(params.q.clone());
    let account_id = session.as_ref().map(|u| u.account_id.to_string());

    let limit = params.limit.unwrap_or(50).min(200);
    let (items, total, _next_cursor) = match lorehaven_db::media::list_media_filtered(
        db,
        &query,
        account_id.as_deref(),
        limit,
        params.cursor.as_deref(),
    )
    .await
    {
        Ok(res) => res,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response()
        }
    };

    Json(json!({
        "items": items,
        "total": total,
        "limit": limit,
    }))
    .into_response()
}

async fn get_media(
    State(state): State<AppState>,
    Path(id): Path<String>,
    MaybeSession(session): MaybeSession,
) -> impl IntoResponse {
    let db = state.db();
    let account_id = session.as_ref().map(|u| u.account_id.to_string());

    match lorehaven_db::media::find_media(db, &id).await {
        Ok(Some(media))
            if media.visibility == "public"
                || account_id.as_deref() == media.owning_account_id.as_deref() =>
        {
            Json(media).into_response()
        }
        Ok(Some(_)) => (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "not eligible"})),
        )
            .into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn list_media_files(
    State(state): State<AppState>,
    Path(id): Path<String>,
    MaybeSession(session): MaybeSession,
) -> impl IntoResponse {
    let db = state.db();
    let account_id = session.as_ref().map(|u| u.account_id.to_string());

    match lorehaven_db::media::find_media(db, &id).await {
        Ok(Some(media))
            if media.visibility == "public"
                || account_id.as_deref() == media.owning_account_id.as_deref() =>
        {
            // TODO: actual files query
            Json(json!({"files": []})).into_response()
        }
        Ok(Some(_)) => (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "not eligible"})),
        )
            .into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn list_media_editions(
    State(state): State<AppState>,
    Path(id): Path<String>,
    MaybeSession(session): MaybeSession,
) -> impl IntoResponse {
    let db = state.db();
    let account_id = session.as_ref().map(|u| u.account_id.to_string());

    match lorehaven_db::media::find_media(db, &id).await {
        Ok(Some(media))
            if media.visibility == "public"
                || account_id.as_deref() == media.owning_account_id.as_deref() =>
        {
            let editions: Vec<lorehaven_db::media::MediaEdition> = Vec::new();
            Json(json!({"editions": editions})).into_response()
        }
        Ok(Some(_)) => (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "not eligible"})),
        )
            .into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn creator_media(
    State(state): State<AppState>,
    Path(id): Path<String>,
    MaybeSession(session): MaybeSession,
) -> impl IntoResponse {
    let db = state.db();
    let account_id = session.as_ref().map(|u| u.account_id.to_string());

    match lorehaven_db::media::creator_media(db, &id, account_id.as_deref()).await {
        Ok(items) => Json(json!({"items": items})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn distributor_media(
    State(state): State<AppState>,
    Path(id): Path<String>,
    MaybeSession(session): MaybeSession,
) -> impl IntoResponse {
    let db = state.db();
    let account_id = session.as_ref().map(|u| u.account_id.to_string());

    match lorehaven_db::media::distributor_media(db, &id, account_id.as_deref()).await {
        Ok(items) => Json(json!({"items": items})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn media_collection_media(
    State(state): State<AppState>,
    Path(id): Path<String>,
    MaybeSession(session): MaybeSession,
) -> impl IntoResponse {
    let db = state.db();
    let account_id = session.as_ref().map(|u| u.account_id.to_string());

    match lorehaven_db::media::collection_media(db, &id, account_id.as_deref()).await {
        Ok(items) => Json(json!({"items": items})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn canon_media(State(_state): State<AppState>, Path(_id): Path<String>) -> impl IntoResponse {
    // TODO: implement canon-scoped media listing
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "error": {
                "code": "NOT_IMPLEMENTED",
                "message": "canon media not yet implemented"
            }
        })),
    )
        .into_response()
}

async fn space_media(State(_state): State<AppState>, Path(_id): Path<String>) -> impl IntoResponse {
    // TODO: implement space-scoped media listing
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "error": {
                "code": "NOT_IMPLEMENTED",
                "message": "space media not yet implemented"
            }
        })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Actor and collection reads
// ---------------------------------------------------------------------------

async fn list_creators(
    State(state): State<AppState>,
    MaybeSession(_session): MaybeSession,
) -> impl IntoResponse {
    let db = state.db();
    match lorehaven_db::media::list_creators(db).await {
        Ok(items) => Json(json!({"items": items})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn get_creator(
    State(state): State<AppState>,
    Path(id): Path<String>,
    MaybeSession(_session): MaybeSession,
) -> impl IntoResponse {
    let db = state.db();
    match lorehaven_db::media::find_creator(db, &id).await {
        Ok(Some(creator)) => Json(creator).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn list_distributors(
    State(state): State<AppState>,
    MaybeSession(_session): MaybeSession,
) -> impl IntoResponse {
    let db = state.db();
    match lorehaven_db::media::list_distributors(db).await {
        Ok(items) => Json(json!({"items": items})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn get_distributor(
    State(state): State<AppState>,
    Path(id): Path<String>,
    MaybeSession(_session): MaybeSession,
) -> impl IntoResponse {
    let db = state.db();
    match lorehaven_db::media::find_distributor(db, &id).await {
        Ok(Some(dist)) => Json(dist).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn list_media_collections(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
) -> impl IntoResponse {
    let db = state.db();
    let account_id = session.as_ref().map(|u| u.account_id.to_string());
    match lorehaven_db::media::list_collections(db, account_id.as_deref()).await {
        Ok(items) => Json(json!({"items": items})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn get_media_collection(
    State(state): State<AppState>,
    Path(id): Path<String>,
    MaybeSession(_session): MaybeSession,
) -> impl IntoResponse {
    let db = state.db();
    match lorehaven_db::media::find_collection(db, &id).await {
        Ok(Some(coll)) => Json(coll).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Writes (complex query, actor and collection CRUD)
// ---------------------------------------------------------------------------

async fn post_media_query(
    State(state): State<AppState>,
    Json(body): Json<MediaQuery>,
) -> impl IntoResponse {
    // Same as GET but POST body for complex queries
    list_media(State(state), Query(body), MaybeSession(None)).await
}

async fn post_creator(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<CreateCreatorRequest>,
) -> impl IntoResponse {
    let db = state.db();
    let _ = user; // TODO: use user.account_id for owning_account_id
    let creator = lorehaven_db::media::NewCreator {
        kind: CreatorKind::from_str(&body.kind).unwrap_or(CreatorKind::External),
        pseud_id: body.pseud_id,
        display_name: &body.display_name,
        source_key: body.source_key.as_deref(),
        source_creator_id: body.source_creator_id.as_deref(),
        canonical_url: body.canonical_url.as_deref(),
        verified_at: None,
    };
    match lorehaven_db::media::create_creator(db, &creator).await {
        Ok(id) => (StatusCode::CREATED, Json(json!({"id": id}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn patch_creator(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<PatchCreatorRequest>,
) -> impl IntoResponse {
    let db = state.db();
    match lorehaven_db::media::patch_creator(db, &id, body.name.as_deref(), body.role.as_deref())
        .await
    {
        Ok(true) => (StatusCode::OK, Json(json!({ "status": "updated" }))).into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn post_distributor(
    State(state): State<AppState>,
    RequireSession(_user): RequireSession,
    Json(body): Json<CreateDistributorRequest>,
) -> impl IntoResponse {
    let db = state.db();
    let distributor = lorehaven_db::media::NewDistributor {
        name: &body.name,
        kind: DistributorKind::from_str(&body.kind).unwrap_or(DistributorKind::Self_),
        source_key: body.source_key.as_deref(),
        canonical_url: body.canonical_url.as_deref(),
    };
    match lorehaven_db::media::create_distributor(db, &distributor).await {
        Ok(id) => (StatusCode::CREATED, Json(json!({"id": id}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn post_media_collection(
    State(state): State<AppState>,
    RequireSession(_user): RequireSession,
    Json(body): Json<CreateCollectionRequest>,
) -> impl IntoResponse {
    let db = state.db();
    let collection = lorehaven_db::media::NewCollection {
        kind: CollectionKind::from_str(&body.kind).unwrap_or(CollectionKind::Series),
        owning_account_id: body.owning_account_id.as_deref(),
        title: &body.title,
        description: body.description.as_deref(),
        visibility: &body.visibility,
    };
    match lorehaven_db::media::create_collection(db, &collection).await {
        Ok(id) => (StatusCode::CREATED, Json(json!({"id": id}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn put_media_collection(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<PutCollectionRequest>,
) -> impl IntoResponse {
    let db = state.db();
    match lorehaven_db::media::put_collection(
        db,
        &id,
        body.title.as_deref(),
        body.description.as_deref(),
    )
    .await
    {
        Ok(true) => (StatusCode::OK, Json(json!({ "status": "updated" }))).into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Atom/RSS feed for media listings.
async fn media_feed(
    State(state): State<AppState>,
    Query(params): Query<MediaQuery>,
    MaybeSession(session): MaybeSession,
) -> impl IntoResponse {
    let db = state.db();
    let query = QueryAst::Text(params.q.clone());
    let account_id = session.as_ref().map(|u| u.account_id.to_string());
    let limit = params.limit.unwrap_or(50).min(200);

    match lorehaven_db::media::list_media_filtered(db, &query, account_id.as_deref(), limit, None).await {
        Ok((items, total, _next_cursor)) => {
            // Build simple Atom feed
            let entries: Vec<String> = items.iter().map(|m| {
                format!(r#"<entry><title>{}</title><id>{}</id><updated>{}</updated></entry>"#,
                    m.title, m.id, m.updated_at)
            }).collect();
            let xml = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
<title>Media Feed</title>
<id>urn:uuid:media-feed</id>
<totalResults>{}</totalResults>
{}
</feed>"#,
                total, entries.join("")
            );
            ([("content-type", "application/atom+xml")], xml).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()})))
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Read doors — public rate class; eligibility is applied inside the bodies.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/media", get(list_media))
        .route("/media/feed", get(media_feed))
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

// ---------------------------------------------------------------------------
// Request/response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct MediaQuery {
    pub q: String,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCreatorRequest {
    pub kind: String,
    pub pseud_id: Option<String>,
    pub display_name: String,
    pub source_key: Option<String>,
    pub source_creator_id: Option<String>,
    pub canonical_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDistributorRequest {
    pub name: String,
    pub kind: String, // DistributorKind
    pub source_key: Option<String>,
    pub canonical_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCollectionRequest {
    pub kind: String, // CollectionKind
    pub owning_account_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub visibility: String,
}
