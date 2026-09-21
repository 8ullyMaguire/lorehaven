//! Resource Directory routes (spec §39).
//!
//! Community-curated ranked lists of external resources and internal
//! references. Administrators seed the two instance lists; members submit
//! entries; the operator moderates; everyone votes, weighted by trust and
//! (silently) taste affinity. Refusals name their reason (§39.3); weights
//! are never disclosed (§39.4).

use crate::auth::{MaybeSession, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use lorehaven_db::directory as db;
use lorehaven_domain::directory as domain;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/directory/lists", get(list_lists).post(create_list))
        .route("/directory/lists/{slug}", get(get_list))
        .route("/directory/entries", get(list_entries).post(submit_entry))
        .route("/directory/entries/{id}", get(get_entry).delete(remove_entry))
        .route("/directory/entries/{id}/approve", post(approve_entry))
        .route("/directory/entries/{id}/vote", post(vote))
        .route("/directory/categories", get(categories))
        .route("/directory/moderation", get(moderation_queue))
}

// --- Lists ---------------------------------------------------------------

async fn create_list(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
    Json(body): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    require_operator(&state, &session).await?;

    let slug = body["slug"].as_str().unwrap_or("").trim().to_lowercase();
    let title = body["title"].as_str().unwrap_or("").trim().to_owned();
    let description = body["description"].as_str().unwrap_or("").trim().to_owned();
    let kind = body["kind"].as_str().unwrap_or("external").trim().to_owned();

    if slug.is_empty() || !slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err(bad_request("slug must be lowercase letters, digits and dashes"));
    }
    domain::validate_title(&title).map_err(|e| bad_request(&e.reason))?;
    domain::validate_description(&description).map_err(|e| bad_request(&e.reason))?;
    if !matches!(kind.as_str(), "external" | "internal") {
        return Err(bad_request("kind must be external or internal"));
    }

    if db::list_by_slug(state.db(), &slug).await.map_err(internal)?.is_some() {
        return Err(bad_request("slug already exists"));
    }

    let id = uuid::Uuid::new_v4().to_string();
    let now = lorehaven_db::sessions::now();
    db::create_list(
        state.db(), &id, &slug, &title, &description, &kind,
        true, 0,
        &session.account_id.to_string(),
        &now,
    )
    .await
    .map_err(internal)?;

    let list = db::list_by_slug(state.db(), &slug).await.map_err(internal)?;
    Ok((StatusCode::CREATED, Json(json!({ "list": list }))))
}

async fn list_lists(
    State(state): State<AppState>,
    MaybeSession(_session): MaybeSession,
) -> ApiResult<Json<Value>> {
    let lists = db::list_lists(state.db()).await.map_err(internal)?;
    Ok(Json(json!({ "items": lists })))
}

async fn get_list(
    State(state): State<AppState>,
    MaybeSession(_session): MaybeSession,
    Path(slug): Path<String>,
) -> ApiResult<Json<Value>> {
    let list = db::list_by_slug(state.db(), &slug)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "list" }))?;
    Ok(Json(json!({ "list": list })))
}

// --- Entries ---------------------------------------------------------------

#[derive(Deserialize)]
struct EntryQuery {
    list: Option<String>,
    category: Option<String>,
    q: Option<String>,
    sort: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Deserialize)]
struct SubmitEntry {
    list: String,
    kind: String,
    category: String,
    title: String,
    url: Option<String>,
    description: Option<String>,
    ref_id: Option<String>,
    tags: Option<Vec<String>>,
}

async fn list_entries(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(q): Query<EntryQuery>,
) -> ApiResult<Json<Value>> {
    let viewer = session.as_ref().map(|s| s.account_id.to_string());
    let is_operator = is_operator_check(&state, viewer.as_deref()).await.map_err(internal)?;
    let sort = match q.sort.as_deref() {
        Some("new") => db::DirectorySort::New,
        _ => db::DirectorySort::Top,
    };

    // Resolve slug -> list id.
    let list_id = match &q.list {
        Some(slug) => db::list_by_slug(state.db(), slug).await.map_err(internal)?.map(|l| l.id),
        None => None,
    };

    let filter = db::DirectoryEntryFilter {
        list_id,
        category: q.category.clone(),
        q: q.q.clone(),
        sort,
        limit: q.limit.unwrap_or(50).clamp(1, 100),
        offset: q.offset.unwrap_or(0).max(0),
        viewer,
        is_operator,
    };
    let entries = db::list_entries(state.db(), &filter).await.map_err(internal)?;
    Ok(Json(json!({ "items": entries })))
}

async fn submit_entry(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
    Json(body): Json<SubmitEntry>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let list = db::list_by_slug(state.db(), &body.list)
        .await
        .map_err(internal)?
        .ok_or_else(|| bad_request("unknown list"))?;
    if body.kind != "external" && body.kind != "internal" {
        return Err(bad_request("kind must be external or internal"));
    }
    if body.kind == "external" && body.url.is_none() {
        return Err(bad_request("external entries need a url"));
    }
    if body.kind == "internal" && body.ref_id.is_none() {
        return Err(bad_request("internal entries need a ref_id"));
    }
    let allowed: Vec<String> = state.config().directory.categories();
    if !allowed.iter().any(|c| c == &body.category) {
        return Err(bad_request("category_not_allowed"));
    }
    let url = body.url.clone().unwrap_or_default();
    if body.kind == "external" {
        domain::validate_url(&url).map_err(|e| bad_request(&e.reason))?;
    }
    domain::validate_title(&body.title).map_err(|e| bad_request(&e.reason))?;
    let description = body.description.clone().unwrap_or_default();
    domain::validate_description(&description).map_err(|e| bad_request(&e.reason))?;
    let tags = domain::normalize_tags(&body.tags.clone().unwrap_or_default());

    let id = uuid::Uuid::new_v4().to_string();
    let now = lorehaven_db::sessions::now();
    db::submit_entry(
        state.db(), &id, &list.id, &body.kind, &body.category, &body.title,
        &url, &description, body.ref_id.as_deref(), &tags,
        &session.account_id.to_string(), &now,
    )
    .await
    .map_err(internal)?;

    let entry = db::get_entry(state.db(), &id, Some(&session.account_id.to_string()), false)
        .await
        .map_err(internal)?
        .expect("just inserted");
    // tags_json is storage; the API speaks a parsed array.
    let mut view = serde_json::to_value(&entry).expect("serialise entry");
    view["tags"] = json!(entry.tags());
    Ok((StatusCode::CREATED, Json(json!({ "entry": view }))))
}

async fn get_entry(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let viewer = session.as_ref().map(|s| s.account_id.to_string());
    let is_operator = is_operator_check(&state, viewer.as_deref()).await.map_err(internal)?;
    let entry = db::get_entry(state.db(), &id, viewer.as_deref(), is_operator)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "entry" }))?;
    Ok(Json(json!({ "entry": entry })))
}

async fn remove_entry(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    require_operator(&state, &session).await?;
    let removed = db::remove_entry(state.db(), &id).await.map_err(internal)?;
    if !removed {
        return Err(ApiError(lorehaven_domain::AppError::NotFound { resource: "entry" }));
    }
    Ok(Json(json!({ "removed": true })))
}

async fn approve_entry(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    require_operator(&state, &session).await?;
    let now = lorehaven_db::sessions::now();
    let approved = db::approve_entry(state.db(), &id, &session.account_id.to_string(), &now)
        .await
        .map_err(internal)?;
    if !approved {
        return Err(ApiError(lorehaven_domain::AppError::NotFound { resource: "entry" }));
    }
    let entry = db::get_entry(state.db(), &id, None, true)
        .await
        .map_err(internal)?
        .expect("just approved");
    Ok(Json(json!({ "entry": entry })))
}

// --- Voting ----------------------------------------------------------------

#[derive(Deserialize)]
struct VoteBody {
    value: i64,
}

async fn vote(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
    Path(id): Path<String>,
    Json(body): Json<VoteBody>,
) -> ApiResult<Json<Value>> {
    if body.value != 1 && body.value != -1 {
        return Err(bad_request("vote value must be 1 or -1"));
    }
    // Only approved entries are votable (spec §39.4: votes rank the list).
    let entry = db::get_entry(state.db(), &id, Some(&session.account_id.to_string()), false)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "entry" }))?;
    if entry.approved_by.is_none() {
        return Err(bad_request("pending entries cannot be voted on"));
    }

    let trust_level = lorehaven_db::governance::trust_for(state.db(), &session.account_id.to_string())
        .await
        .map_err(|e| internal(e.into()))? as u8;
    let affinity = lorehaven_db::discovery::taste_profile_for(state.db(), &session.account_id.to_string())
        .await
        .map_err(internal)?
        .and_then(|p| p.signals.get("affinity").and_then(|v| v.as_f64()))
        .unwrap_or(0.0)
        .clamp(-1.0, 1.0);
    let cfg = &state.config().directory;
    let weight = domain::vote_weight(
        cfg.weighting_mode(),
        trust_level,
        &cfg.trust_vote_weights,
        affinity,
        cfg.taste_floor,
        cfg.taste_ceiling,
    );

    let now = lorehaven_db::sessions::now();
    let (score, live) = db::set_vote(
        state.db(), &id, &session.account_id.to_string(), body.value, weight, &now,
    )
    .await
    .map_err(internal)?;

    // Response: score and the viewer's direction. Never the weight.
    Ok(Json(json!({
        "score": score,
        "my_vote": if live { Value::from(body.value) } else { Value::Null },
    })))
}

// --- Categories & moderation ----------------------------------------------

async fn categories(
    State(state): State<AppState>,
    MaybeSession(_session): MaybeSession,
) -> ApiResult<Json<Value>> {
    let configured = state.config().directory.categories();
    let counts = db::category_counts(state.db()).await.map_err(internal)?;
    let items: Vec<Value> = configured
        .into_iter()
        .map(|c| {
            let count = counts
                .iter()
                .find(|(cat, _)| cat == &c)
                .map(|(_, n)| *n)
                .unwrap_or(0);
            json!({ "category": c, "approved_count": count })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

async fn moderation_queue(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
) -> ApiResult<Json<Value>> {
    require_operator(&state, &session).await?;
    let pending = db::pending_entries(state.db()).await.map_err(internal)?;
    Ok(Json(json!({ "items": pending })))
}

// --- Helpers ---------------------------------------------------------------

async fn require_operator(state: &AppState, session: &crate::auth::SessionUser) -> Result<(), ApiError> {
    let level = lorehaven_db::governance::trust_for(state.db(), &session.account_id.to_string())
        .await
        .map_err(|e| internal(e.into()))? as u8;
    if level < 5 {
        return Err(ApiError(lorehaven_domain::AppError::AccessDenied));
    }
    Ok(())
}

async fn is_operator_check(state: &AppState, account_id: Option<&str>) -> Result<bool, anyhow::Error> {
    match account_id {
        Some(id) => Ok(lorehaven_db::governance::trust_for(state.db(), id).await.map_err(|e| anyhow::Error::from(e))? >= 5),
        None => Ok(false),
    }
}

fn bad_request(reason: &str) -> ApiError {
    // Spec §39.3: a refusal names its reason. The reason rides both the
    // message and a structured field so clients can branch on it.
    let mut field_errors = BTreeMap::new();
    field_errors.insert("reason".to_owned(), reason.to_owned());
    ApiError(lorehaven_domain::AppError::Validation {
        message: format!("directory: {reason}"),
        field_errors,
    })
}

fn internal(e: anyhow::Error) -> ApiError {
    ApiError(lorehaven_domain::AppError::Internal(e))
}
