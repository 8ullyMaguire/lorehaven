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
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use lorehaven_domain::media::{CollectionKind, CreatorKind, DistributorKind};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::str::FromStr;

use crate::auth::{MaybeSession, MaybeToken, RequireSession, SessionUser, TokenUser};
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
}

#[derive(Deserialize)]
pub struct PutCollectionRequest {
    pub title: Option<String>,
    pub description: Option<String>,
}

// ---------------------------------------------------------------------------
// Read doors (the same engine behind every list; spec §32.4)
// ---------------------------------------------------------------------------

/// Hash the eligible representation, not global change timestamps. Private
/// revalidation prevents a shared cache from reusing a signed-in response.
fn conditional_response(
    headers: &HeaderMap,
    body: Vec<u8>,
    content_type: &'static str,
) -> axum::response::Response {
    let tag = format!("\"{:x}\"", Sha256::digest(&body));
    let unchanged = headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|value| {
            value.trim() == "*" || value.trim().strip_prefix("W/").unwrap_or(value.trim()) == tag
        });
    let mut response = if unchanged {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        ([(header::CONTENT_TYPE, content_type)], body).into_response()
    };
    response
        .headers_mut()
        .insert(header::ETAG, HeaderValue::from_str(&tag).expect("hex ETag"));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-cache"),
    );
    response.headers_mut().insert(
        header::VARY,
        HeaderValue::from_static("Cookie, Authorization"),
    );
    response
}

async fn list_media(
    State(state): State<AppState>,
    Query(params): Query<MediaQuery>,
    MaybeSession(session): MaybeSession,
    MaybeToken(token): MaybeToken,
    headers: HeaderMap,
) -> impl IntoResponse {
    // API scope enforcement (spec §23.1): a bearer token must carry
    // ContentRead to list media. Session callers bypass scope checks.
    if let Some(t) = &token {
        use lorehaven_domain::api_scopes::Scope;
        if !t.scopes.contains(&Scope::ContentRead) {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"error": {"code": "FORBIDDEN", "message": "token lacks content.read scope"}})),
            )
                .into_response();
        }
    }
    let db = state.db();
    // Empty/absent q means match-all: no text facet is built, so the
    // query never touches works_index.
    let query = match params.q.as_deref().filter(|q| !q.trim().is_empty()) {
        Some(q) => match lorehaven_domain::query::parse_query(q).and_then(|ast| {
            lorehaven_domain::query_sql::render_query(&ast)?;
            Ok(ast)
        }) {
            Ok(ast) => Some(ast),
            Err(error) => return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": {"code": "VALIDATION_FAILED", "message": error.message, "offset": error.offset}})),
            ).into_response(),
        },
        None => None,
    };
    let account_id = session.as_ref().map(|u| u.account_id.to_string());

    // Validate quality/date filters up front — fail fast with a clear code
    // rather than letting an invalid value propagate into SQL.
    if params.quality_min.is_some() && params.quality_kind.is_none() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": {"code": "VALIDATION_FAILED", "message": "quality_kind is required when quality_min is set"}})),
        )
            .into_response();
    }
    if let Some(from) = &params.date_from {
        if time::OffsetDateTime::parse(from, &time::format_description::well_known::Rfc3339).is_err() {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": {"code": "VALIDATION_FAILED", "message": "date_from must be RFC 3339"}})),
            )
                .into_response();
        }
    }
    if let Some(to) = &params.date_to {
        if time::OffsetDateTime::parse(to, &time::format_description::well_known::Rfc3339).is_err() {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": {"code": "VALIDATION_FAILED", "message": "date_to must be RFC 3339"}})),
            )
                .into_response();
        }
    }

    let limit = match params.validate_pagination() {
        Ok(limit) => limit,
        Err(message) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": {"code": "VALIDATION_FAILED", "message": message}})),
            )
                .into_response()
        }
    };
    let (items, total, next_cursor) = match lorehaven_db::media::list_media_filtered(
        db,
        query.as_ref(),
        account_id.as_deref(),
        limit,
        params.cursor.as_deref(),
        params.quality_kind.as_deref(),
        params.quality_min,
        params.date_from.as_deref(),
        params.date_to.as_deref(),
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

    conditional_response(
        &headers,
        serde_json::to_vec(&json!({
            "items": items,
            "total": total,
            "limit": limit,
            "next_cursor": next_cursor,
        }))
        .expect("media JSON"),
        "application/json",
    )
}

/// The direct-door rule (ADR 0002): public and unlisted are reachable by
/// anyone with the link; restricted requires a session (skeleton §7.6 —
/// the shared eligibility service replaces this when wired); drafts and
/// unknown states only for the owner. Ineligible callers get 404, not
/// 403 — do not reveal that a hidden work exists.
async fn direct_door_eligible(
    state: &AppState,
    media: &lorehaven_db::media::MediaRecord,
    session: Option<&SessionUser>,
    token: Option<&TokenUser>,
) -> bool {
    use lorehaven_domain::policy::{can_access_content, AccessPolicy, ContentFacts};
    // If a bearer token is presented, enforce API scopes (spec §23.1): the
    // token must carry ContentRead to read media. A session caller bypasses
    // scope checks (session authorization is established at login).
    if let Some(t) = token {
        use lorehaven_domain::api_scopes::Scope;
        if !t.scopes.contains(&Scope::ContentRead) {
            return false;
        }
    }
    // Token-authenticated callers contribute as their account; session callers
    // contribute as their pseud. Either identity can satisfy contributor checks.
    let token_contributor =
        token.is_some_and(|t| t.account_id.to_string() == media.owning_account_id);
    let session_contributor =
        session.is_some_and(|user| user.account_id.to_string() == media.owning_account_id);
    if token_contributor || session_contributor {
        // Short-circuit: the caller owns this media. Build an actor from whichever
        // identity is available so the shared eligibility service still applies.
        let actor = session.and_then(|user| user.pseud_id.map(|pseud| user.actor(pseud)));
        return lorehaven_domain::policy::can_access_content(
            actor.as_ref(),
            &lorehaven_domain::policy::ContentFacts {
                lifecycle: {
                    let parse = |value: &str| serde_json::Value::String(value.to_owned());
                    serde_json::from_value(parse(&media.lifecycle))
                        .unwrap_or(lorehaven_domain::policy::Lifecycle::Draft)
                },
                visibility: {
                    let parse = |value: &str| serde_json::Value::String(value.to_owned());
                    serde_json::from_value(parse(&media.visibility))
                        .unwrap_or(lorehaven_domain::policy::Visibility::Public)
                },
                rating: {
                    let parse = |value: &str| serde_json::Value::String(value.to_owned());
                    serde_json::from_value(parse(&media.rating))
                        .unwrap_or(lorehaven_domain::policy::ContentRating::General)
                },
                actor_is_contributor: true,
                author_blocked_actor: false,
                via_deep_link: true,
            },
            &lorehaven_domain::policy::AccessPolicy::default(),
        )
        .is_allowed();
    }
    let actor = session.and_then(|user| user.pseud_id.map(|pseud| user.actor(pseud)));
    let parse = |value: &str| serde_json::Value::String(value.to_owned());
    let (Ok(lifecycle), Ok(visibility), Ok(rating)) = (
        serde_json::from_value(parse(&media.lifecycle)),
        serde_json::from_value(parse(&media.visibility)),
        serde_json::from_value(parse(&media.rating)),
    ) else {
        return false;
    };
    let mut policy = AccessPolicy::default();
    if let Some(user) = session {
        let Ok(settings) =
            lorehaven_db::sessions::content_settings(state.db(), user.account_id).await
        else {
            return false;
        };
        policy.adult_max_rating = policy.adult_max_rating.min(settings.max_rating);
        policy.minor_max_rating = policy.minor_max_rating.min(settings.max_rating);
        policy.unknown_age_max_rating = policy.unknown_age_max_rating.min(settings.max_rating);
    }
    can_access_content(
        actor.as_ref(),
        &ContentFacts {
            lifecycle,
            visibility,
            rating,
            actor_is_contributor: session
                .is_some_and(|user| user.account_id.to_string() == media.owning_account_id),
            author_blocked_actor: false,
            via_deep_link: true,
        },
        &policy,
    )
    .is_allowed()
}

async fn get_media(
    State(state): State<AppState>,
    Path(id): Path<String>,
    MaybeSession(session): MaybeSession,
    MaybeToken(token): MaybeToken,
    headers: HeaderMap,
) -> impl IntoResponse {
    let db = state.db();

    match lorehaven_db::media::find_media(db, &id).await {
        Ok(Some(media))
            if direct_door_eligible(&state, &media, session.as_ref(), token.as_ref()).await =>
        {
            conditional_response(
                &headers,
                serde_json::to_vec(&media).expect("media JSON"),
                "application/json",
            )
        }
        Ok(Some(_)) => (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response(),
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
    MaybeToken(token): MaybeToken,
) -> impl IntoResponse {
    let db = state.db();

    match lorehaven_db::media::find_media(db, &id).await {
        Ok(Some(media))
            if direct_door_eligible(&state, &media, session.as_ref(), token.as_ref()).await =>
        {
            match lorehaven_db::media::list_media_files(db, &id).await {
                Ok(files) => Json(json!({"files": files})).into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": e.to_string()})),
                )
                    .into_response(),
            }
        }
        Ok(Some(_)) => (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response(),
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
    MaybeToken(token): MaybeToken,
) -> impl IntoResponse {
    let db = state.db();

    match lorehaven_db::media::find_media(db, &id).await {
        Ok(Some(media))
            if direct_door_eligible(&state, &media, session.as_ref(), token.as_ref()).await =>
        {
            match lorehaven_db::media::list_media_editions(db, &id).await {
                Ok(editions) => Json(json!({"editions": editions})).into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": e.to_string()})),
                )
                    .into_response(),
            }
        }
        Ok(Some(_)) => (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response(),
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

/// Feed for a media collection (Atom/RSS).
async fn media_collection_feed(
    State(state): State<AppState>,
    Path(id): Path<String>,
    MaybeSession(session): MaybeSession,
) -> impl IntoResponse {
    let db = state.db();
    let account_id = session.as_ref().map(|u| u.account_id.to_string());

    // id here can be either a collection id or a kind like "public_domain"
    let items = if id == "public_domain" {
        match lorehaven_db::media::list_media_by_license(db, "cc0", account_id.as_deref(), 50, None).await {
            Ok((items, _, _)) => items,
            Err(_) => Vec::new(),
        }
    } else {
        match lorehaven_db::media::collection_media(db, &id, account_id.as_deref()).await {
            Ok(items) => items,
            Err(_) => Vec::new(),
        }
    };
    media_feed_dc(items, "")
}

async fn canon_media(
    State(state): State<AppState>,
    Path(id): Path<String>,
    MaybeSession(session): MaybeSession,
    Query(params): Query<MediaQuery>,
) -> impl IntoResponse {
    let db = state.db();
    let account_id = session.as_ref().map(|u| u.account_id.to_string());

    // The scoped doors page the same way the list door does: a validated limit,
    // a cursor carrying the ordering key, and a `next_cursor` only when the page
    // was full. Before this they answered with a silent 50-row ceiling.
    let limit = match scoped_limit(&params) {
        Ok(limit) => limit,
        Err(message) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": {"code": "VALIDATION_FAILED", "message": message}})),
            )
                .into_response()
        }
    };
    let cursor = params.cursor.as_deref();

    match lorehaven_db::media::canon_media(db, &id, account_id.as_deref(), limit, cursor).await {
        Ok((items, canon_name, next_cursor)) => Json(json!({
            "items": items,
            "canon": id,
            "name": canon_name,
            "next_cursor": next_cursor,
        }))
        .into_response(),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") {
                (StatusCode::NOT_FOUND, Json(json!({"error": msg}))).into_response()
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": msg})),
                )
                    .into_response()
            }
        }
    }
}

async fn space_media(
    State(state): State<AppState>,
    Path(id): Path<String>,
    MaybeSession(session): MaybeSession,
    Query(params): Query<MediaQuery>,
) -> impl IntoResponse {
    let db = state.db();
    let account_id = session.as_ref().map(|u| u.account_id.to_string());

    let limit = match scoped_limit(&params) {
        Ok(limit) => limit,
        Err(message) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": {"code": "VALIDATION_FAILED", "message": message}})),
            )
                .into_response()
        }
    };
    let cursor = params.cursor.as_deref();

    match lorehaven_db::media::space_media(db, &id, account_id.as_deref(), limit, cursor).await {
        Ok((items, space_name, next_cursor)) => Json(json!({
            "items": items,
            "space": id,
            "name": space_name,
            "next_cursor": next_cursor,
        }))
        .into_response(),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") {
                (StatusCode::NOT_FOUND, Json(json!({"error": msg}))).into_response()
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": msg})),
                )
                    .into_response()
            }
        }
    }
}

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
    MaybeSession(session): MaybeSession,
) -> impl IntoResponse {
    let db = state.db();
    let account_id = session.as_ref().map(|u| u.account_id.to_string());
    match lorehaven_db::media::find_collection(db, &id, account_id.as_deref()).await {
        Ok(Some(coll)) => Json(coll).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// List media collections filtered by kind (e.g., public_domain).
async fn list_media_collections_by_kind(
    State(state): State<AppState>,
    Path(kind): Path<String>,
    MaybeSession(session): MaybeSession,
) -> impl IntoResponse {
    let db = state.db();
    let account_id = session.as_ref().map(|u| u.account_id.to_string());
    // For the special "public_domain" kind, query works by rights field
    if kind == "public_domain" {
        return match lorehaven_db::media::list_media_by_license(
            db,
            "cc0",
            account_id.as_deref(),
            50,
            None,
        )
        .await
        {
            Ok((items, _, _)) => Json(json!({"items": items})).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response(),
        };
    }
    match lorehaven_db::media::list_collections_by_kind(db, &kind, account_id.as_deref()).await {
        Ok(items) => Json(json!({"kind": kind, "collections": items})).into_response(),
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
    MaybeSession(session): MaybeSession,
    Json(body): Json<MediaQuery>,
) -> impl IntoResponse {
    // Same as GET but POST body for complex queries. It is a READ, not a
    // write: anonymous callers are allowed and eligibility is applied
    // inside; it sits in the Write rate class only because complex queries
    // are expensive. The caller's session is honored, never stripped.
    list_media(
        State(state),
        Query(body),
        MaybeSession(session),
        MaybeToken(None),
        HeaderMap::new(),
    )
    .await
}

async fn post_creator(
    State(state): State<AppState>,
    RequireSession(_user): RequireSession,
    Json(body): Json<CreateCreatorRequest>,
) -> impl IntoResponse {
    let db = state.db();
    let Ok(kind) = CreatorKind::from_str(&body.kind) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": {
                    "code": "VALIDATION_FAILED",
                    "message": format!("unknown creator kind: {}", body.kind),
                }
            })),
        )
            .into_response();
    };
    // A creator record must be internally consistent (spec §32.1): a local
    // pseud points at a pseud, an external creator at its source. Refused
    // at the edge, not stored and ignored later.
    if !lorehaven_domain::media::creator_record_is_consistent(
        kind,
        body.pseud_id.is_some(),
        body.source_key.as_deref(),
        body.source_creator_id.as_deref(),
    ) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": {
                    "code": "VALIDATION_FAILED",
                    "message": "creator record is neither properly local nor properly external",
                }
            })),
        )
            .into_response();
    }
    let creator = lorehaven_db::media::NewCreator {
        kind,
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
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Json(body): Json<PatchCreatorRequest>,
) -> impl IntoResponse {
    let db = state.db();
    match lorehaven_db::governance::trust_for(db, &user.account_id.to_string()).await {
        Ok(level) if level > 5 => {}
        Ok(_) => return (StatusCode::FORBIDDEN, Json(json!({"error": {"code":"FORBIDDEN", "message":"Shared attribution changes require curator authority."}}))).into_response(),
        Err(error) => {
            tracing::error!(?error, "could not check creator curation authority");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    match lorehaven_db::media::patch_creator(db, &id, body.name.as_deref()).await {
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
    let Ok(kind) = DistributorKind::from_str(&body.kind) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": {
                    "code": "VALIDATION_FAILED",
                    "message": format!("unknown distributor kind: {}", body.kind),
                }
            })),
        )
            .into_response();
    };
    let distributor = lorehaven_db::media::NewDistributor {
        name: &body.name,
        kind: kind.to_string(),
        source_key: body.source_key.as_deref(),
        canonical_url: body.canonical_url.as_deref(),
        url: body.url.as_deref(),
        api_key: body.api_key.as_deref(),
        notes: body.notes.as_deref(),
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
    RequireSession(user): RequireSession,
    Json(body): Json<CreateCollectionRequest>,
) -> impl IntoResponse {
    let db = state.db();
    let Ok(kind) = CollectionKind::from_str(&body.kind) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": {
                    "code": "VALIDATION_FAILED",
                    "message": format!("unknown collection kind: {}", body.kind),
                }
            })),
        )
            .into_response();
    };
    // The owning account is the SESSION's account, never a client-chosen
    // id: no caller may mint a collection owned by somebody else.
    let _owner = user.account_id.to_string();
    let collection = lorehaven_db::media::NewCollection {
        kind: kind.to_string(),
        owning_account_id: Some(&user.account_id.to_string()),
        title: &body.title,
        description: body.description.as_deref(),
        visibility: body.visibility.as_str(),
        parent_collection_id: body.parent_collection_id.as_deref(),
        sort_order: body.sort_order,
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
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Json(body): Json<PutCollectionRequest>,
) -> impl IntoResponse {
    let db = state.db();
    let owner = user.account_id.to_string();
    match lorehaven_db::media::put_collection(
        db,
        &id,
        &owner,
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

/// Escape user-provided text for embedding in Atom/XML. Titles and
/// summaries are author-controlled; an unescaped title is stored XSS in
/// every feed reader.
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Atom/RSS feed for media listings.
async fn media_feed(
    State(state): State<AppState>,
    Query(params): Query<MediaQuery>,
    MaybeSession(session): MaybeSession,
    MaybeToken(token): MaybeToken,
    headers: HeaderMap,
) -> impl IntoResponse {
    // API scope enforcement (spec §23.1): a bearer token must carry
    // ContentRead to read the feed. Session callers bypass scope checks.
    if let Some(t) = &token {
        use lorehaven_domain::api_scopes::Scope;
        if !t.scopes.contains(&Scope::ContentRead) {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"error": {"code": "FORBIDDEN", "message": "token lacks content.read scope"}})),
            )
                .into_response();
        }
    }
    let db = state.db();
    // Same match-all rule as the list door: empty/absent q builds no text
    // facet, so the feed never touches works_index.
    let query = match params.q.as_deref().filter(|q| !q.trim().is_empty()) {
        Some(q) => match lorehaven_domain::query::parse_query(q).and_then(|ast| {
            lorehaven_domain::query_sql::render_query(&ast)?;
            Ok(ast)
        }) {
            Ok(ast) => Some(ast),
            Err(error) => return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": {"code": "VALIDATION_FAILED", "message": error.message, "offset": error.offset}})),
            ).into_response(),
        },
        None => None,
    };
    let account_id = session.as_ref().map(|u| u.account_id.to_string());
    let limit = match params.validate_pagination() {
        Ok(limit) => limit,
        Err(message) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": {"code": "VALIDATION_FAILED", "message": message}})),
            )
                .into_response()
        }
    };

    match lorehaven_db::media::list_media_filtered(
        db,
        query.as_ref(),
        account_id.as_deref(),
        limit,
        params.cursor.as_deref(),
        params.quality_kind.as_deref(),
        params.quality_min,
        params.date_from.as_deref(),
        params.date_to.as_deref(),
    )
    .await
    {
        Ok((items, _total, next_cursor)) => {
            let kind = params.format.as_deref().unwrap_or("atom");
            if !matches!(kind, "atom" | "rss" | "opds" | "dc" | "jsonld") {
                return (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"error": {"code": "VALIDATION_FAILED", "message": "feed format must be atom, rss, opds, dc, or jsonld"}}))).into_response();
            }
            let base = state.config().site.base_url.trim_end_matches('/');
            if kind == "dc" {
                return media_feed_dc(items, base);
            }
            if kind == "jsonld" {
                return media_feed_jsonld(items, base);
            }
            let mut self_url =
                url::Url::parse(&format!("{base}/api/v1/media/feed")).expect("configured base URL");
            {
                let mut pairs = self_url.query_pairs_mut();
                pairs
                    .append_pair("format", kind)
                    .append_pair("limit", &limit.to_string());
                if let Some(q) = params.q.as_deref().filter(|q| !q.trim().is_empty()) {
                    pairs.append_pair("q", q.trim());
                }
                if let Some(cursor) = &params.cursor {
                    pairs.append_pair("cursor", cursor);
                }
            }
            let updated = items
                .iter()
                .map(|m| m.updated_at.as_str())
                .max()
                .unwrap_or("1970-01-01T00:00:00Z");
            let mut xml = if kind == "rss" {
                format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?><rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom"><channel><title>Media Feed</title><link>{}</link><description>Eligible media matching this query</description><atom:link rel="self" href="{}" type="application/rss+xml"/>"#,
                    xml_escape(base),
                    xml_escape(self_url.as_str())
                )
            } else {
                format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?><feed xmlns="http://www.w3.org/2005/Atom"><title>Media Feed</title><id>{}</id><updated>{}</updated><author><name>Lorehaven</name></author><link rel="self" href="{}" type="application/atom+xml"/>"#,
                    xml_escape(self_url.as_str()),
                    xml_escape(updated),
                    xml_escape(self_url.as_str())
                )
            };
            if let Some(cursor) = next_cursor {
                let mut next = self_url.clone();
                next.set_query(None);
                for (key, value) in self_url.query_pairs().filter(|(key, _)| key != "cursor") {
                    next.query_pairs_mut().append_pair(&key, &value);
                }
                next.query_pairs_mut().append_pair("cursor", &cursor);
                let prefix = if kind == "rss" || kind == "opds" { "atom:" } else { "" };
                xml.push_str(&format!(
                    r#"<{prefix}link rel="next" href="{}"/>"#,
                    xml_escape(next.as_str())
                ));
            }
            for item in items {
                let link = xml_escape(&format!("{base}/works/{}", item.id));
                let title = xml_escape(&item.title);
                let summary = xml_escape(item.summary.as_deref().unwrap_or(""));
                let id = xml_escape(&format!("urn:uuid:{}", item.id));
                if kind == "rss" {
                    xml.push_str(&format!(r#"<item><title>{title}</title><link>{link}</link><guid isPermaLink="false">{id}</guid><description>{summary}</description></item>"#));
                } else {
                    xml.push_str(&format!(r#"<entry><title>{title}</title><id>{id}</id><updated>{}</updated><link href="{link}"/><summary>{summary}</summary></entry>"#, xml_escape(&item.updated_at)));
                }
            }
            xml.push_str(if kind == "rss" {
                "</channel></rss>"
            } else {
                "</feed>"
            });
            conditional_response(
                &headers,
                xml.into_bytes(),
                if kind == "rss" {
                    "application/rss+xml"
                } else if kind == "opds" {
                    "application/atom+xml;profile=opds-catalog"
                } else {
                    "application/atom+xml"
                },
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// Render media listings as Dublin Core XML (spec §32.2, IA-style item metadata).
fn media_feed_dc(
    items: Vec<lorehaven_db::media::MediaRecord>,
    base: &str,
) -> axum::response::Response {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:dc="http://purl.org/dc/elements/1.1/">"#,
    );
    for item in items {
        let link = xml_escape(&format!("{}/works/{}", base, item.id));
        let title = xml_escape(&item.title);
        let id = xml_escape(&format!("urn:uuid:{}", item.id));
        let updated = xml_escape(&item.updated_at);
        let summary = xml_escape(item.summary.as_deref().unwrap_or(""));
        xml.push_str(&format!(
            r#"<rdf:Description rdf:resource="{link}"><dc:identifier>{id}</dc:identifier><dc:title>{title}</dc:title><dc:date>{updated}</dc:date><dc:description>{summary}</dc:description></rdf:Description>"#
        ));
    }
    xml.push_str("</rdf:RDF>");
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/rdf+xml")],
        xml.into_bytes(),
    )
        .into_response()
}

/// Render media listings as JSON-LD (CreativeWork family, spec §32.2).
fn media_feed_jsonld(
    items: Vec<lorehaven_db::media::MediaRecord>,
    base: &str,
) -> axum::response::Response {
    use serde_json::json;
    let context = "https://schema.org/";
    let graph: Vec<serde_json::Value> = items
        .iter()
        .map(|item| {
            let link = format!("{}/works/{}", base, item.id);
            let summary = item.summary.as_deref().unwrap_or("");
            let updated = item.updated_at.clone();
            let media_type = item.format.clone();
            json!({
                "@context": context,
                "@type": "CreativeWork",
                "name": item.title,
                "url": link,
                "description": summary,
                "dateModified": updated,
                "genre": media_type,
            })
        })
        .collect();
    let body = json!({ "@context": context, "@graph": graph }).to_string();
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/ld+json")],
        body.into_bytes(),
    )
        .into_response()
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
        .route("/media-collections/{id}/media/feed", get(media_collection_feed))
        .route("/media-collections/kind/{kind}/media", get(list_media_collections_by_kind))
        .route("/media-collections/kind/{kind}/media/feed", get(media_collection_feed))
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
    /// Free-text query. Empty/absent = match-all (no text facet).
    pub q: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
    /// Output format override: `atom` (default), `opds`, `json`, or `dc` (Dublin Core).
    pub format: Option<String>,
    /// Quality signal kind to filter by (e.g. `editorial_review`, `reader_positivity`).
    pub quality_kind: Option<String>,
    /// Minimum aggregate quality value (inclusive). Requires `quality_kind`.
    pub quality_min: Option<i64>,
    /// RFC 3339 lower bound on `created_at` (inclusive).
    pub date_from: Option<String>,
    /// RFC 3339 upper bound on `created_at` (inclusive).
    pub date_to: Option<String>,
}

/// The limit and cursor for a scoped (canon/space) page.
///
/// The cursor is `<position>|<created_at>|<id>`: the whole ordering key. A
/// cursor that carried only the id could not advance this ordering, and one
/// that carried only the position would repeat a whole position block.
fn scoped_limit(params: &MediaQuery) -> Result<i64, &'static str> {
    let limit = params.limit.unwrap_or(50);
    if limit < 1 {
        return Err("limit must be positive");
    }
    if let Some(cursor) = &params.cursor {
        let mut parts = cursor.rsplitn(3, '|');
        let (Some(id), Some(created_at), Some(position)) =
            (parts.next(), parts.next(), parts.next())
        else {
            return Err("invalid cursor");
        };
        position
            .parse::<i64>()
            .map_err(|_| "invalid cursor position")?;
        time::OffsetDateTime::parse(created_at, &time::format_description::well_known::Rfc3339)
            .map_err(|_| "invalid cursor timestamp")?;
        uuid::Uuid::parse_str(id).map_err(|_| "invalid cursor id")?;
    }
    Ok(limit.min(200))
}

impl MediaQuery {
    fn validate_pagination(&self) -> Result<i64, &'static str> {
        let limit = self.limit.unwrap_or(50);
        if limit < 1 {
            return Err("limit must be positive");
        }
        if let Some(cursor) = &self.cursor {
            let (date, id) = cursor.rsplit_once('|').ok_or("invalid cursor")?;
            time::OffsetDateTime::parse(date, &time::format_description::well_known::Rfc3339)
                .map_err(|_| "invalid cursor timestamp")?;
            uuid::Uuid::parse_str(id).map_err(|_| "invalid cursor id")?;
        }
        Ok(limit.min(200))
    }
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
    pub url: Option<String>,
    pub api_key: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCollectionRequest {
    pub kind: String, // CollectionKind
    pub owning_account_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub visibility: String,
    pub parent_collection_id: Option<String>,
    pub sort_order: Option<i64>,
}
