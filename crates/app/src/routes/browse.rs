use crate::auth::{MaybeSession, RequirePseud};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use lorehaven_domain::browse::Sort;
use lorehaven_domain::AppError;
use serde::{Deserialize, Serialize};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/browse/surfaces", get(list_surfaces))
        .route("/browse/sort/{surface}", get(get_sort).put(set_sort))
        .route("/browse/sort/{surface}", axum::routing::delete(delete_sort))
}

#[derive(Debug, Serialize)]
struct SurfaceInfo {
    key: String,
    label: String,
    default_sort: String,
}

async fn list_surfaces(
    State(_state): State<AppState>,
) -> ApiResult<Json<Vec<SurfaceInfo>>> {
    let surfaces = vec![
        SurfaceInfo { key: "discover".into(), label: "Discover".into(), default_sort: "for-you".into() },
        SurfaceInfo { key: "people".into(), label: "People".into(), default_sort: "az".into() },
        SurfaceInfo { key: "library".into(), label: "Library".into(), default_sort: "new".into() },
        SurfaceInfo { key: "tags".into(), label: "Tags".into(), default_sort: "az".into() },
        SurfaceInfo { key: "fandoms".into(), label: "Fandoms".into(), default_sort: "az".into() },
        SurfaceInfo { key: "collections".into(), label: "Collections".into(), default_sort: "new".into() },
        SurfaceInfo { key: "series".into(), label: "Series".into(), default_sort: "new".into() },
        SurfaceInfo { key: "authors".into(), label: "Authors".into(), default_sort: "az".into() },
    ];
    Ok(Json(surfaces))
}

#[derive(Debug, Serialize)]
struct SortStateResponse {
    surface: String,
    sort: String,
    source: String,
}

/// GET /browse/sort/:surface — public. Returns the default if no session,
/// or the caller's stored preference if one exists.
async fn get_sort(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(surface): Path<String>,
) -> ApiResult<Json<SortStateResponse>> {
    let surface_default = default_for(&surface);

    // If there's no session, return the default.
    let Some(user) = session else {
        return Ok(Json(SortStateResponse {
            surface,
            sort: surface_default.to_string(),
            source: "default".into(),
        }));
    };

    // If there's no pseud selected, also return default.
    let Some(pseud_id) = user.pseud_id else {
        return Ok(Json(SortStateResponse {
            surface,
            sort: surface_default.to_string(),
            source: "default".into(),
        }));
    };

    let pseud_str = pseud_id.to_string();
    let pref = lorehaven_db::browse::get_sort_preference(state.db(), &pseud_str, &surface)
        .await
        .map_err(|e| ApiError(AppError::internal("reading sort preference", e)))?;

    let (sort, source) = match pref.as_ref() {
        Some(pref) => {
            if let Some(sort) = Sort::parse(&pref.sort_value) {
                (sort.as_str().to_string(), "preference".to_string())
            } else {
                (surface_default.to_string(), "default".to_string())
            }
        }
        None => (surface_default.to_string(), "default".to_string()),
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
        ApiError(AppError::field("sort", format!("unknown sort `{}`; accepted: {}", body.sort, Sort::accepted_set())))
    })?;

    let pseud_str = pseud_id.to_string();
    lorehaven_db::browse::set_sort_preference(state.db(), &pseud_str, &surface, sort.as_str())
        .await
        .map_err(|e| ApiError(AppError::internal("saving sort preference", e)))?;

    Ok(Json(SortStateResponse { surface, sort: sort.as_str().to_string(), source: "preference".into() }))
}

async fn delete_sort(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(surface): Path<String>,
) -> ApiResult<Json<SortStateResponse>> {
    let pseud_str = pseud_id.to_string();
    lorehaven_db::browse::delete_sort_preference(state.db(), &pseud_str, &surface)
        .await
        .map_err(|e| ApiError(AppError::internal("deleting sort preference", e)))?;

    let sort = default_for(&surface).to_string();
    Ok(Json(SortStateResponse { surface, sort, source: "default".into() }))
}

fn default_for(surface: &str) -> Sort {
    match surface {
        "discover" => Sort::ForYou,
        "people" | "tags" | "fandoms" | "authors" | "moods" => Sort::Az,
        "library" | "collections" | "series" | "reading-paths" => Sort::New,
        _ => Sort::New,
    }
}
