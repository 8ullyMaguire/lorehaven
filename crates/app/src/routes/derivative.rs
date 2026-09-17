//! Derivative pipeline routes (M25 / spec §32.4).

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct RequestDerivative {
    pub edition_kind: String,
    pub derivative_kind: String,
    pub parent_checksum: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/works/{id}/derivatives",
            get(list_derivatives).post(request_derivative),
        )
        .route("/derivatives/{id}", get(get_derivative))
}

fn validation(message: &str) -> ApiError {
    ApiError::from(lorehaven_domain::AppError::Validation {
        message: message.into(),
        field_errors: std::collections::BTreeMap::new(),
    })
}

pub async fn list_derivatives(
    State(state): State<AppState>,
    RequireSession(_session): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let db = state.db();
    let derivatives = lorehaven_db::derivative::list_derivatives(db, &id).await?;
    let items: Vec<Value> = derivatives
        .into_iter()
        .map(|d| {
            json!({
                "id": d.id,
                "work_id": d.work_id,
                "edition_kind": d.edition_kind,
                "derivative_kind": d.derivative_kind,
                "parent_checksum": d.parent_checksum,
                "output_checksum": d.output_checksum,
                "output_bytes": d.output_bytes,
                "output_mime_type": d.output_mime_type,
                "state": d.state,
                "job_id": d.job_id,
                "built_at": d.built_at,
                "verified_at": d.verified_at,
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

pub async fn request_derivative(
    State(state): State<AppState>,
    RequireSession(_session): RequireSession,
    Path(id): Path<String>,
    Json(body): Json<RequestDerivative>,
) -> ApiResult<Json<Value>> {
    let db = state.db();

    let derivative_kind =
        lorehaven_domain::derivative::DerivativeKind::parse(&body.derivative_kind)
            .ok_or_else(|| validation("unknown derivative_kind"))?;

    let new = lorehaven_db::derivative::NewDerivative {
        work_id: &id,
        edition_kind: &body.edition_kind,
        derivative_kind,
        parent_checksum: &body.parent_checksum,
    };

    let derivative_id = lorehaven_db::derivative::create_derivative(db, new).await?;

    // TODO: enqueue a Derivative job to actually build the rendition.
    Ok(Json(json!({ "id": derivative_id, "state": "queued" })))
}

pub async fn get_derivative(
    State(state): State<AppState>,
    RequireSession(_session): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let db = state.db();
    let derivative = lorehaven_db::derivative::find_derivative(db, &id)
        .await?
        .ok_or_else(|| {
            ApiError::from(lorehaven_domain::AppError::NotFound {
                resource: "derivative",
            })
        })?;
    Ok(Json(json!({
        "id": derivative.id,
        "work_id": derivative.work_id,
        "edition_kind": derivative.edition_kind,
        "derivative_kind": derivative.derivative_kind,
        "parent_checksum": derivative.parent_checksum,
        "output_checksum": derivative.output_checksum,
        "output_bytes": derivative.output_bytes,
        "output_mime_type": derivative.output_mime_type,
        "state": derivative.state,
        "job_id": derivative.job_id,
        "error_message": derivative.error_message,
        "built_at": derivative.built_at,
        "verified_at": derivative.verified_at,
    })))
}
