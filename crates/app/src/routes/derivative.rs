//! Derivative pipeline routes (M25 / spec §32.4).

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{MaybeSession, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::routes::works::{reading_for_work, require_contributor, Reading};
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
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let db = state.db();
    // A rendition is part of the work's face, so the work's own visibility
    // decides who may ask for the list; a work nobody else can read answers
    // with the work door's answer.
    if let Reading::Denied(error) = reading_for_work(&state, &id, session.as_ref()).await? {
        return Err(ApiError(error));
    }
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
    RequireSession(session): RequireSession,
    Path(id): Path<String>,
    Json(body): Json<RequestDerivative>,
) -> ApiResult<Json<Value>> {
    let db = state.db();

    // Only a contributor may ask for a rendition: it is built from the work's
    // own bytes and becomes part of what the work offers.
    require_contributor(&state, &id, &session).await?;

    let derivative_kind =
        lorehaven_domain::derivative::DerivativeKind::parse(&body.derivative_kind)
            .ok_or_else(|| validation("unknown derivative_kind"))?;

    // A kind this instance cannot produce is refused before a row exists: a
    // queued derivative that can never build is a row an operator has to
    // investigate, and the refusal can name what to install (§13's
    // CONVERTER_UNAVAILABLE contract, the same one exports use).
    if let Some(program) = missing_program(&state, derivative_kind) {
        return Err(ApiError::from(
            lorehaven_domain::AppError::ConverterUnavailable {
                message: format!(
                    "{program} is not installed on this instance, and {derivative_kind} \
                 derivatives need it: {}",
                    derivative_kind.install_hint()
                ),
            },
        ));
    }

    // The parent must be a blob this instance holds: a row pointing at a
    // checksum nobody has is a job that fails after a queue round-trip, and the
    // operator reading that failure learns nothing the door could not have said.
    let store = lorehaven_db::storage::BlobStore::new(state.config().storage.root.clone());
    if store.stat(db, &body.parent_checksum).await?.is_none() {
        return Err(validation(&format!(
            "no stored blob has the checksum {}",
            body.parent_checksum
        )));
    }

    let new = lorehaven_db::derivative::NewDerivative {
        work_id: &id,
        edition_kind: &body.edition_kind,
        derivative_kind,
        parent_checksum: &body.parent_checksum,
    };
    let derivative_id = lorehaven_db::derivative::create_derivative(db, new).await?;

    // Queue the build. The payload names the derivative and nothing else, so a
    // queue row never carries a work's text or its blob.
    let job_id = lorehaven_db::jobs::enqueue(
        db,
        lorehaven_domain::jobs::JobKind::Derivative,
        &json!({ "derivative_id": derivative_id }).to_string(),
        Some(&format!("derivative:{derivative_id}")),
        None,
        0,
        &lorehaven_domain::jobs::RetryPolicy::default(),
    )
    .await?;
    lorehaven_db::derivative::attach_derivative_job(db, &derivative_id, &job_id.to_string())
        .await?;

    Ok(Json(json!({
        "id": derivative_id,
        "state": "queued",
        "job_id": job_id.to_string(),
    })))
}

/// The first required program this instance does not have, if any.
///
/// The document renditions resolve their program through the converter layer
/// (Calibre or pandoc), which reports its own unavailability, so only the
/// kinds with a fixed program are checked here.
fn missing_program(
    state: &AppState,
    kind: lorehaven_domain::derivative::DerivativeKind,
) -> Option<&'static str> {
    kind.required_programs()
        .iter()
        .find(|program| state.converters().discover_program(program).is_none())
        .copied()
}

pub async fn get_derivative(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
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
    // The work decides, so a rendition of a work the caller cannot read is not
    // disclosed either — including a failed one, whose error_message quotes the
    // program's output.
    if let Reading::Denied(error) =
        reading_for_work(&state, &derivative.work_id, session.as_ref()).await?
    {
        return Err(ApiError(error));
    }
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
