//! TTS narration routes (M26 / spec §32.5).

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct RequestNarration {
    pub provider: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/works/{id}/editions",
            get(list_editions).post(request_narration),
        )
        .route("/editions/{id}", get(get_edition))
}

pub async fn list_editions(
    State(state): State<AppState>,
    RequireSession(_session): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let db = state.db();
    let editions = lorehaven_db::media::list_media_editions(db, &id).await?;
    Ok(Json(json!({ "editions": editions })))
}

pub async fn request_narration(
    State(state): State<AppState>,
    RequireSession(_session): RequireSession,
    Path(id): Path<String>,
    Json(body): Json<RequestNarration>,
) -> ApiResult<Json<Value>> {
    let db = state.db();

    // Create the narration edition in draft state.
    let edition_label = format!("Machine narration ({})", body.provider);
    let edition_id =
        lorehaven_db::narration::create_narration_edition(db, &id, &edition_label, None).await?;

    // Credit the provider as narrator.
    lorehaven_db::narration::add_narration_creator(db, &edition_id, &body.provider, "narrator")
        .await?;

    Ok(Json(json!({
        "edition_id": edition_id,
        "edition_kind": "narration",
        "label": edition_label,
        "state": "draft",
        "machine_produced": true,
    })))
}

pub async fn get_edition(
    State(state): State<AppState>,
    RequireSession(_session): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let db = state.db();
    let edition = lorehaven_db::media::find_media_edition(db, &id)
        .await?
        .ok_or_else(|| ApiError::from(lorehaven_domain::AppError::NotFound {
            resource: "edition".into(),
        }))?;
    Ok(Json(json!({
        "id": edition.id,
        "work_id": edition.work_id,
        "edition_kind": edition.edition_kind,
        "label": edition.label,
        "parent_edition_id": edition.parent_edition_id,
        "published_at": edition.published_at,
    })))
}
