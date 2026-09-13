//! Routes for collections, challenges, requests/exchanges, wishlists, events.
//!
//! Spec §18.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::RequirePseud;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

/// A limit query shared by the list endpoints. Cursor pagination is not
/// wired yet (the lists answer in one page); the field returns with the
/// keyset work.
#[derive(Debug, Deserialize)]
pub struct CursorQuery {
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    20
}

pub fn router() -> Router<AppState> {
    Router::new()
        // Collections
        .route("/collections", get(get_collections).post(post_collection))
        .route("/collections/{id}", get(get_collection).put(put_collection))
        .route(
            "/collections/{id}/items",
            get(get_collection_items).post(post_collection_item),
        )
        // Challenges
        .route("/challenges", get(get_challenges).post(post_challenge))
        .route("/challenges/{id}", get(get_challenge))
        .route("/challenges/{id}/entries", post(post_challenge_entry))
        // Requests / exchanges
        .route("/requests", get(get_requests).post(post_request))
        .route("/requests/{id}/claims", post(post_claim))
        .route("/claims/{id}/fulfil", post(post_fulfil_claim))
        // Wishlists
        .route("/wishlists/{account}", get(get_wishlist))
        .route("/wishlist-items", post(post_wishlist_item))
        // Events (writing events)
        .route("/events", get(get_events).post(post_event))
        .route("/events/{id}", get(get_event))
        .route("/events/{id}/join", post(join_event))
}

// ---------------------------------------------------------------------------
// Collections
// ---------------------------------------------------------------------------

async fn get_collections(
    State(state): State<AppState>,
    Query(params): Query<CursorQuery>,
) -> ApiResult<Json<Value>> {
    let rows = lorehaven_db::events::list_public_collections(state.db(), params.limit)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(json!({ "items": rows })))
}

#[derive(Debug, Deserialize)]
struct CreateCollectionBody {
    name: String,
    description: Option<String>,
    item_policy: String,
    is_public: bool,
}

async fn post_collection(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Json(body): Json<CreateCollectionBody>,
) -> ApiResult<Json<Value>> {
    let id = lorehaven_db::events::create_collection(
        state.db(),
        &body.name,
        body.description.as_deref(),
        &pseud_id.to_string(),
        &body.item_policy,
        body.is_public,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(json!({ "id": id })))
}

async fn get_collection(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let collection = lorehaven_db::events::get_collection(state.db(), &id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    match collection {
        Some(c) => {
            let items = lorehaven_db::events::list_collection_items(state.db(), &c.id)
                .await
                .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
            Ok(Json(json!({ "collection": c, "items": items })))
        }
        None => Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "collection",
        })),
    }
}

#[allow(clippy::needless_pass_by_value)]
async fn put_collection(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
    Json(body): Json<CreateCollectionBody>,
) -> ApiResult<StatusCode> {
    let rows = lorehaven_db::events::update_collection(
        state.db(),
        &id,
        &pseud_id.to_string(),
        &body.name,
        body.description.as_deref(),
        &body.item_policy,
        body.is_public,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    if !rows {
        return Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "collection",
        }));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn get_collection_items(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let items = lorehaven_db::events::list_collection_items(state.db(), &id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(json!({ "items": items })))
}

#[derive(Debug, Deserialize)]
struct AddCollectionItemBody {
    work_id: String,
    note: Option<String>,
}

async fn post_collection_item(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(collection_id): Path<String>,
    Json(body): Json<AddCollectionItemBody>,
) -> ApiResult<StatusCode> {
    let collection = lorehaven_db::events::get_collection(state.db(), &collection_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    let Some(collection) = collection else {
        return Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "collection",
        }));
    };
    let policy =
        lorehaven_domain::events::ItemPolicy::parse(&collection.item_policy).ok_or_else(|| {
            ApiError(lorehaven_domain::AppError::field(
                "item_policy",
                "invalid policy",
            ))
        })?;
    let action = lorehaven_domain::events::collection_add_permission(
        policy,
        collection.owner == pseud_id.to_string(),
    );
    match action {
        lorehaven_domain::events::CollectionAction::CanAdd => {
            lorehaven_db::events::add_collection_item(
                state.db(),
                &collection_id,
                &body.work_id,
                &pseud_id.to_string(),
                body.note.as_deref(),
            )
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
            Ok(StatusCode::NO_CONTENT)
        }
        lorehaven_domain::events::CollectionAction::CanPropose => {
            // For M13, inserts directly; a moderation queue lands with M14.
            lorehaven_db::events::add_collection_item(
                state.db(),
                &collection_id,
                &body.work_id,
                &pseud_id.to_string(),
                body.note.as_deref(),
            )
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
            Ok(StatusCode::ACCEPTED)
        }
        lorehaven_domain::events::CollectionAction::CannotAdd => {
            Err(ApiError(lorehaven_domain::AppError::AccessDenied))
        }
    }
}

// ---------------------------------------------------------------------------
// Challenges
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CreateChallengeBody {
    name: String,
    rules: String,
    schedule: String,
}

async fn get_challenges(
    State(state): State<AppState>,
    Query(params): Query<CursorQuery>,
) -> ApiResult<Json<Value>> {
    let rows = lorehaven_db::events::list_challenges(state.db(), params.limit)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(json!({ "items": rows })))
}

async fn post_challenge(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Json(body): Json<CreateChallengeBody>,
) -> ApiResult<Json<Value>> {
    let id = lorehaven_db::events::create_challenge(
        state.db(),
        &body.name,
        &body.rules,
        &body.schedule,
        &pseud_id.to_string(),
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(json!({ "id": id })))
}

async fn get_challenge(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let challenge = lorehaven_db::events::get_challenge(state.db(), &id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    match challenge {
        Some(c) => Ok(Json(json!({ "challenge": c }))),
        None => Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "challenge",
        })),
    }
}

#[derive(Debug, Deserialize)]
struct EnterChallengeBody {
    work_id: String,
}

async fn post_challenge_entry(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(challenge_id): Path<String>,
    Json(body): Json<EnterChallengeBody>,
) -> ApiResult<Json<Value>> {
    // Gather work facts (word count from the content table).
    let word_count = lorehaven_db::events::work_word_count(state.db(), &body.work_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    let facts = lorehaven_domain::events::WorkFacts {
        word_count: Some(word_count),
        ..Default::default()
    };
    let results =
        lorehaven_db::events::enter_challenge(state.db(), &challenge_id, &body.work_id, &facts)
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    let all_pass = results.iter().all(|r| r.passed);
    let _ = pseud_id;
    Ok(Json(
        json!({ "entered": true, "all_pass": all_pass, "checks": results }),
    ))
}

// ---------------------------------------------------------------------------
// Requests / Exchanges
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CreateRequestBody {
    prompt: String,
    anonym_until: Option<String>,
}

async fn get_requests(
    State(state): State<AppState>,
    Query(params): Query<CursorQuery>,
) -> ApiResult<Json<Value>> {
    let rows = lorehaven_db::events::list_requests(state.db(), params.limit)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(json!({ "items": rows })))
}

async fn post_request(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Json(body): Json<CreateRequestBody>,
) -> ApiResult<Json<Value>> {
    let id = lorehaven_db::events::create_request(
        state.db(),
        &pseud_id.to_string(),
        &body.prompt,
        body.anonym_until.as_deref(),
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(json!({ "id": id })))
}

#[derive(Debug, Deserialize)]
struct ClaimRequestBody {
    claimant: String,
}

async fn post_claim(
    State(state): State<AppState>,
    RequirePseud { .. }: RequirePseud,
    Path(request_id): Path<String>,
    Json(body): Json<ClaimRequestBody>,
) -> ApiResult<Json<Value>> {
    let claimed = lorehaven_db::events::claim_request(state.db(), &request_id, &body.claimant)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    if !claimed {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "claim",
            "a request can only hold one active claim",
        )));
    }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
struct FulfilRequestBody {
    work_id: String,
    claimant: String,
}

async fn post_fulfil_claim(
    State(state): State<AppState>,
    RequirePseud { .. }: RequirePseud,
    Path(request_id): Path<String>,
    Json(body): Json<FulfilRequestBody>,
) -> ApiResult<Json<Value>> {
    let fulfils =
        lorehaven_db::events::fulfil_claim(state.db(), &request_id, &body.claimant, &body.work_id)
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    if !fulfils {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "claim",
            "no active claim found for this claimant",
        )));
    }
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------------------
// Wishlists
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct WishlistItemBody {
    node_id: Option<String>,
    work_id: Option<String>,
    note: Option<String>,
}

async fn get_wishlist(
    State(state): State<AppState>,
    MaybeSession(maybe_user): MaybeSession,
    Path(account): Path<String>,
) -> ApiResult<Json<Value>> {
    let viewer = maybe_user.as_ref().map(|u| u.account_id.to_string());
    let wishlist = lorehaven_db::events::get_wishlist(state.db(), &account, viewer.as_deref())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    match wishlist {
        Some(w) => Ok(Json(json!({ "wishlist": w }))),
        None => Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "wishlist",
        })),
    }
}

async fn post_wishlist_item(
    State(state): State<AppState>,
    RequirePseud { user, .. }: RequirePseud,
    Json(body): Json<WishlistItemBody>,
) -> ApiResult<Json<Value>> {
    lorehaven_db::events::upsert_wishlist(state.db(), &user.account_id.to_string(), false)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    lorehaven_db::events::add_wishlist_item(
        state.db(),
        &user.account_id.to_string(),
        body.node_id.as_deref(),
        body.work_id.as_deref(),
        body.note.as_deref(),
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------------------
// Events (writing events)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CreateEventBody {
    name: String,
    document: String,
}

async fn get_events(
    State(state): State<AppState>,
    Query(params): Query<CursorQuery>,
) -> ApiResult<Json<Value>> {
    let rows = lorehaven_db::events::list_events(state.db(), params.limit)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(json!({ "items": rows })))
}

async fn post_event(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Json(body): Json<CreateEventBody>,
) -> ApiResult<Json<Value>> {
    let id = lorehaven_db::events::create_event(
        state.db(),
        &body.name,
        &body.document,
        &pseud_id.to_string(),
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(json!({ "id": id })))
}

async fn get_event(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let event = lorehaven_db::events::get_event(state.db(), &id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    match event {
        // The participant list is part of the event's public answer; without
        // it list_event_participants has no caller and joining proves nothing
        // to the joiner.
        Some(e) => {
            let participants = lorehaven_db::events::list_event_participants(state.db(), &id)
                .await
                .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
            Ok(Json(json!({ "event": e, "participants": participants })))
        }
        None => Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "event",
        })),
    }
}

async fn join_event(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(event_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let joined = lorehaven_db::events::join_event(state.db(), &event_id, &pseud_id.to_string())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    if !joined {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "join",
            "you have already joined this event",
        )));
    }
    Ok(Json(json!({ "ok": true })))
}

use crate::auth::MaybeSession;
