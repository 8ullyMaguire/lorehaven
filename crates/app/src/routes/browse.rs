use crate::auth::{MaybeSession, RequirePseud};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::routing::{delete, get};
use axum::{Json, Router};
use lorehaven_domain::browse::Sort;
use lorehaven_domain::AppError;
use serde::{Deserialize, Serialize};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/browse/surfaces", get(list_surfaces))
        .route(
            "/browse/sort/{surface}",
            get(get_sort).put(set_sort).delete(delete_sort),
        )
        .route("/people", get(list_people))
        .route("/tags", get(list_tags))
        .route("/tags/{tag}", get(list_works_by_tag))
        .route("/fandoms", get(list_fandoms))
        .route("/fandoms/{fandom}", get(list_works_by_fandom))
}

// --- Surfaces -----------------------------------------------------------

#[derive(Debug, Serialize)]
struct SurfaceInfo {
    key: String,
    label: String,
    default_sort: String,
}

async fn list_surfaces(State(_state): State<AppState>, MaybeSession(_session): MaybeSession) -> ApiResult<Json<Vec<SurfaceInfo>>> {
    let surfaces = vec![
        SurfaceInfo { key: "discover".into(), label: "Discover".into(), default_sort: Sort::ForYou.as_str().into() },
        SurfaceInfo { key: "people".into(), label: "People".into(), default_sort: Sort::Az.as_str().into() },
        SurfaceInfo { key: "library".into(), label: "Library".into(), default_sort: Sort::New.as_str().into() },
        SurfaceInfo { key: "tags".into(), label: "Tags".into(), default_sort: Sort::Az.as_str().into() },
        SurfaceInfo { key: "fandoms".into(), label: "Fandoms".into(), default_sort: Sort::Az.as_str().into() },
        SurfaceInfo { key: "collections".into(), label: "Collections".into(), default_sort: Sort::New.as_str().into() },
        SurfaceInfo { key: "series".into(), label: "Series".into(), default_sort: Sort::New.as_str().into() },
        SurfaceInfo { key: "authors".into(), label: "Authors".into(), default_sort: Sort::Az.as_str().into() },
    ];
    Ok(Json(surfaces))
}

// --- Sort state ---------------------------------------------------------

#[derive(Debug, Serialize)]
struct SortStateResponse {
    surface: String,
    sort: String,
    source: String,
}

async fn get_sort(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(surface): Path<String>,
) -> ApiResult<Json<SortStateResponse>> {
    let surface_default = default_for(&surface);

    let Some(user) = session else {
        return Ok(Json(SortStateResponse {
            surface,
            sort: surface_default.as_str().into(),
            source: "default".into(),
        }));
    };

    let Some(pseud_id) = user.pseud_id else {
        return Ok(Json(SortStateResponse {
            surface,
            sort: surface_default.as_str().into(),
            source: "default".into(),
        }));
    };

    let pref = lorehaven_db::browse::get_sort_preference(state.db(), &pseud_id.to_string(), &surface)
        .await
        .map_err(|e| ApiError(AppError::internal("reading sort preference", e)))?;

    let (sort, source) = match pref {
        Some(pref) if Sort::parse(&pref.sort_value).is_some() => {
            (pref.sort_value, "preference".to_string())
        }
        _ => (surface_default.as_str().into(), "default".into()),
    };
    Ok(Json(SortStateResponse { surface, sort, source }))
}

#[derive(Debug, Deserialize)]
struct SetSortBody {
    sort: String,
}

async fn set_sort(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(surface): Path<String>,
    Json(body): Json<SetSortBody>,
) -> ApiResult<Json<SortStateResponse>> {
    let sort = Sort::parse(&body.sort).ok_or_else(|| {
        ApiError(AppError::field(
            "sort",
            format!("unknown sort `{}`; accepted: {}", body.sort, Sort::accepted_set()),
        ))
    })?;

    lorehaven_db::browse::set_sort_preference(state.db(), &pseud_id.to_string(), &surface, sort.as_str())
        .await
        .map_err(|e| ApiError(AppError::internal("saving sort preference", e)))?;

    Ok(Json(SortStateResponse {
        surface,
        sort: sort.as_str().into(),
        source: "preference".into(),
    }))
}

async fn delete_sort(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(surface): Path<String>,
) -> ApiResult<Json<SortStateResponse>> {
    lorehaven_db::browse::delete_sort_preference(state.db(), &pseud_id.to_string(), &surface)
        .await
        .map_err(|e| ApiError(AppError::internal("deleting sort preference", e)))?;

    let sort = default_for(&surface).as_str().to_string();
    Ok(Json(SortStateResponse {
        surface,
        sort,
        source: "default".into(),
    }))
}

fn default_for(surface: &str) -> Sort {
    match surface {
        "discover" => Sort::ForYou,
        "people" | "tags" | "fandoms" | "authors" | "moods" => Sort::Az,
        "library" | "collections" | "series" | "reading-paths" => Sort::New,
        _ => Sort::New,
    }
}

// --- People -------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PeopleQuery {
    #[serde(default)]
    sort: Option<String>,
    #[serde(default = "default_page_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}

fn default_page_limit() -> i64 { 50 }

#[derive(Debug, Serialize)]
struct PersonItem {
    id: String,
    handle: String,
    display_name: String,
    work_count: i64,
}

async fn list_people(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(params): Query<PeopleQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let sort = resolve_surface_sort(&state, session.as_ref(), params.sort.as_deref(), "people").await;

    let people = lorehaven_db::identity::list_discoverable_pseuds(
        state.db(),
        params.limit.clamp(1, 200),
        params.offset.max(0),
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e)))?;

    let mut items: Vec<PersonItem> = people
        .into_iter()
        .map(|(id, handle, display_name, work_count)| PersonItem {
            id,
            handle,
            display_name,
            work_count,
        })
        .collect();

    apply_sort(&mut items, &sort);

    Ok(Json(serde_json::json!({ "items": items, "sort": sort })))
}

fn apply_sort(items: &mut Vec<PersonItem>, sort: &str) {
    match sort {
        "new" => items.sort_by(|a, b| b.id.cmp(&a.id)),
        "updated" => items.sort_by(|a, b| b.id.cmp(&a.id)),
        "trending" => items.sort_by(|a, b| b.work_count.cmp(&a.work_count)),
        "top" => items.sort_by(|a, b| b.work_count.cmp(&a.work_count)),
        "az" => items.sort_by(|a, b| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase())),
        "for-you" => items.sort_by(|a, b| b.work_count.cmp(&a.work_count)),
        _ => {}
    }
}

// --- Tags ---------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct TagListQuery {
    #[serde(default)]
    sort: Option<String>,
    #[serde(default = "default_page_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}

#[derive(Debug, Serialize)]
struct TagItem {
    id: String,
    canonical: String,
    kind: String,
    work_count: i64,
}

async fn list_tags(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(params): Query<TagListQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let sort = resolve_surface_sort(&state, session.as_ref(), params.sort.as_deref(), "tags").await;

    let tags = lorehaven_db::taxonomy::list_tags(
        state.db(),
        params.limit.clamp(1, 200),
        params.offset.max(0),
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e)))?;

    let mut items: Vec<TagItem> = tags
        .into_iter()
        .map(|(id, canonical, kind, work_count)| TagItem { id, canonical, kind, work_count })
        .collect();

    apply_tag_sort(&mut items, &sort);

    Ok(Json(serde_json::json!({ "items": items, "sort": sort })))
}

fn apply_tag_sort(items: &mut Vec<TagItem>, sort: &str) {
    match sort {
        "new" => items.sort_by(|a, b| b.id.cmp(&a.id)),
        "updated" => items.sort_by(|a, b| b.id.cmp(&a.id)),
        "trending" => items.sort_by(|a, b| b.work_count.cmp(&a.work_count)),
        "top" => items.sort_by(|a, b| b.work_count.cmp(&a.work_count)),
        "az" => items.sort_by(|a, b| a.canonical.to_lowercase().cmp(&b.canonical.to_lowercase())),
        "for-you" => items.sort_by(|a, b| b.work_count.cmp(&a.work_count)),
        _ => {}
    }
}

#[derive(Debug, Deserialize)]
pub struct WorksByTagQuery {
    #[serde(default)]
    sort: Option<String>,
    #[serde(default = "default_page_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}

#[derive(Debug, Serialize)]
struct WorksByTagItem {
    work_id: String,
    title: String,
    author_handle: String,
    updated_at: String,
}

async fn list_works_by_tag(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(tag): Path<String>,
    Query(params): Query<WorksByTagQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let sort = resolve_surface_sort(&state, session.as_ref(), params.sort.as_deref(), "tags").await;

    let works = lorehaven_db::taxonomy::works_by_tag(
        state.db(),
        &tag,
        params.limit.clamp(1, 200),
        params.offset.max(0),
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e)))?;

    let mut items: Vec<WorksByTagItem> = works
        .into_iter()
        .map(|(work_id, title, author_handle, updated_at)| WorksByTagItem {
            work_id,
            title,
            author_handle,
            updated_at,
        })
        .collect();

    apply_works_sort(&mut items, &sort);

    Ok(Json(serde_json::json!({ "items": items, "sort": sort, "tag": tag })))
}

// --- Fandoms ------------------------------------------------------------

#[derive(Debug, Serialize)]
struct FandomItem {
    slug: String,
    name: String,
    work_count: i64,
}

async fn list_fandoms(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(params): Query<TagListQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let sort = resolve_surface_sort(&state, session.as_ref(), params.sort.as_deref(), "fandoms").await;

    let fandoms = lorehaven_db::taxonomy::list_fandoms(
        state.db(),
        params.limit.clamp(1, 200),
        params.offset.max(0),
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e)))?;

    let mut items: Vec<FandomItem> = fandoms
        .into_iter()
        .map(|(slug, name, work_count)| FandomItem { slug, name, work_count })
        .collect();

    apply_fandom_sort(&mut items, &sort);

    Ok(Json(serde_json::json!({ "items": items, "sort": sort })))
}

fn apply_fandom_sort(items: &mut Vec<FandomItem>, sort: &str) {
    match sort {
        "new" => items.sort_by(|a, b| b.slug.cmp(&a.slug)),
        "updated" => items.sort_by(|a, b| b.slug.cmp(&a.slug)),
        "trending" => items.sort_by(|a, b| b.work_count.cmp(&a.work_count)),
        "top" => items.sort_by(|a, b| b.work_count.cmp(&a.work_count)),
        "az" => items.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        "for-you" => items.sort_by(|a, b| b.work_count.cmp(&a.work_count)),
        _ => {}
    }
}

async fn list_works_by_fandom(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(fandom): Path<String>,
    Query(params): Query<WorksByTagQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let sort = resolve_surface_sort(&state, session.as_ref(), params.sort.as_deref(), "fandoms").await;

    let works = lorehaven_db::taxonomy::works_by_fandom(
        state.db(),
        &fandom,
        params.limit.clamp(1, 200),
        params.offset.max(0),
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e)))?;

    let mut items: Vec<WorksByTagItem> = works
        .into_iter()
        .map(|(work_id, title, author_handle, updated_at)| WorksByTagItem {
            work_id,
            title,
            author_handle,
            updated_at,
        })
        .collect();

    apply_works_sort(&mut items, &sort);

    Ok(Json(serde_json::json!({ "items": items, "sort": sort, "fandom": fandom })))
}

// --- Works (shared) -----------------------------------------------------

fn apply_works_sort(items: &mut Vec<WorksByTagItem>, sort: &str) {
    match sort {
        "new" => items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at)),
        "updated" => items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at)),
        "trending" => items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at)),
        "top" => items.sort_by(|a, b| a.title.cmp(&b.title)),
        "az" => items.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase())),
        "for-you" => items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at)),
        _ => {}
    }
}

// --- Shared sort resolver ----------------------------------------------

async fn resolve_surface_sort(
    state: &AppState,
    session: Option<&crate::auth::SessionUser>,
    query_sort: Option<&str>,
    surface: &str,
) -> String {
    // 1. Query param takes precedence.
    if let Some(sort_str) = query_sort {
        if Sort::parse(sort_str).is_some() {
            return sort_str.to_string();
        }
    }

    // 2. Stored preference.
    if let Some(user) = session {
        if let Some(pseud_id) = &user.pseud_id {
            if let Ok(Some(pref)) = lorehaven_db::browse::get_sort_preference(
                state.db(),
                &pseud_id.to_string(),
                surface,
            )
            .await
            {
                if Sort::parse(&pref.sort_value).is_some() {
                    return pref.sort_value;
                }
            }
        }
    }

    // 3. Default for the surface.
    default_for(surface).as_str().to_string()
}
