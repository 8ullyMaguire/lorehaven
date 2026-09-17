//! TTS narration routes (M26 / spec §32.5).
//!
//! # The two gates on this surface
//!
//! *Requesting* a narration and *approving* one are both author actions, and
//! they are deliberately separate. A request creates a `narration` edition in
//! draft and queues the synthesis job; the audio lands attached to that draft.
//! Approval is what publishes it, and it refuses an edition whose audio has not
//! been produced. So the machine never publishes itself, and the author never
//! approves something that is not there.
//!
//! The audio door is public once the edition is published — a narration is an
//! edition of a work, and §32.5's acceptance is that it "appears in editions
//! lists, downloads as audio". While the edition is a draft, only a contributor
//! to the work may fetch it.

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{MaybeSession, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::routes::works::{reading_for_work, require_contributor, Reading};
use crate::state::AppState;
use lorehaven_db::storage::BlobStore;
use lorehaven_domain::jobs::{JobKind, RetryPolicy};
use lorehaven_domain::AppError;

#[derive(Debug, Deserialize)]
pub struct RequestNarration {
    /// The machine producer to credit as narrator. Defaults to the configured
    /// engine's name, which is what an instance with one engine should record.
    #[serde(default)]
    pub provider: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/works/{id}/editions",
            get(list_editions).post(request_narration),
        )
        .route("/editions/{id}", get(get_edition))
        .route("/editions/{id}/approve", post(approve_edition))
        .route("/editions/{id}/audio", get(download_audio))
}

fn not_found(resource: &'static str) -> ApiError {
    ApiError::from(AppError::NotFound { resource })
}

fn validation(message: &str) -> ApiError {
    ApiError::from(AppError::Validation {
        message: message.to_owned(),
        field_errors: Default::default(),
    })
}

/// The editions of a work this caller may see.
pub async fn list_editions(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let db = state.db();
    let reading = reading_for_work(&state, &id, session.as_ref()).await?;
    let editions = lorehaven_db::media::list_media_editions(db, &id).await?;
    let visible: Vec<_> = match reading {
        Reading::Contributor => editions,
        // A published edition is part of the work's public face; a draft one is
        // the author's working copy, and saying it exists would leak the shape
        // of work nobody else can read yet.
        Reading::Public => editions
            .into_iter()
            .filter(|edition| edition.published_at.is_some())
            .collect(),
        Reading::Denied(error) => return Err(ApiError(error)),
    };
    Ok(Json(json!({ "editions": visible })))
}

/// Create a narration edition and queue its synthesis.
pub async fn request_narration(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
    Path(id): Path<String>,
    Json(body): Json<RequestNarration>,
) -> ApiResult<Json<Value>> {
    let db = state.db();

    require_contributor(&state, &id, &session).await?;

    // The producer is the engine this instance will actually run. Naming a
    // producer the instance cannot run would put a false credit on the edition,
    // so a mismatch is refused rather than recorded.
    //
    // An engine that was built but is not usable (no binary on PATH, no voice
    // model) is refused here too: queueing a job that cannot possibly succeed
    // makes the reader wait for a worker pass to learn what the host already
    // knows, and leaves a failed row behind. The refusal carries the same
    // sentence `lorehaven doctor` prints.
    let engine = state.tts_engine().map_err(|reason| {
        validation(&format!("this instance has no narration engine: {reason}"))
    })?;
    engine
        .health()
        .map_err(|error| validation(&format!("this instance cannot narrate: {error}")))?;
    let engine_name = engine.name().to_owned();
    let provider = body
        .provider
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .unwrap_or(&engine_name)
        .to_owned();

    // Create the narration edition in draft state.
    let edition_label = format!("Machine narration ({provider})");
    let edition_id =
        lorehaven_db::narration::create_narration_edition(db, &id, &edition_label, None).await?;

    // Credit the provider as narrator (§22.6 machine-producer credit).
    lorehaven_db::narration::add_narration_creator(db, &edition_id, &provider, "narrator").await?;

    // Queue the synthesis. The payload names the edition and nothing else: a
    // private work's text must not be in a queue row.
    let job_id = lorehaven_db::jobs::enqueue(
        db,
        JobKind::Narration,
        &json!({ "edition_id": edition_id }).to_string(),
        None,
        None,
        0,
        &RetryPolicy::default(),
    )
    .await?;

    Ok(Json(json!({
        "edition_id": edition_id,
        "edition_kind": "narration",
        "label": edition_label,
        "state": "draft",
        "machine_produced": true,
        "narrator": provider,
        "engine": engine_name,
        "job_id": job_id.to_string(),
    })))
}

pub async fn get_edition(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let db = state.db();
    let edition = lorehaven_db::media::find_media_edition(db, &id)
        .await?
        .ok_or_else(|| not_found("edition"))?;
    // The work's own visibility decides, and a draft edition is contributor-only
    // on top of it: an editor's working narration is not a public object.
    match reading_for_work(&state, &edition.work_id, session.as_ref()).await? {
        Reading::Contributor => {}
        Reading::Public if edition.published_at.is_some() => {}
        Reading::Public => return Err(not_found("edition")),
        Reading::Denied(error) => return Err(ApiError(error)),
    }
    let audio_checksum = lorehaven_db::narration::narration_audio_checksum(db, &id).await?;
    Ok(Json(json!({
        "id": edition.id,
        "work_id": edition.work_id,
        "edition_kind": edition.edition_kind,
        "label": edition.label,
        "parent_edition_id": edition.parent_edition_id,
        "published_at": edition.published_at,
        "state": if edition.published_at.is_some() { "published" } else { "draft" },
        "machine_produced": edition.edition_kind == "narration",
        "has_audio": audio_checksum.is_some(),
        "audio_checksum": audio_checksum,
    })))
}

/// The author's approval: publish a narration whose audio exists.
pub async fn approve_edition(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let db = state.db();
    let edition = lorehaven_db::media::find_media_edition(db, &id)
        .await?
        .ok_or_else(|| not_found("edition"))?;

    if edition.edition_kind != "narration" {
        return Err(validation(
            "only a narration edition is approved through this door",
        ));
    }
    require_contributor(&state, &edition.work_id, &session).await?;

    let checksum = lorehaven_db::narration::narration_audio_checksum(db, &id).await?;
    if checksum.is_none() {
        // Approving a narration with no audio would put a download door in
        // front of nothing, so the refusal says what is missing instead.
        return Err(validation(
            "this narration has no audio yet; the synthesis job has either not \
             run or failed",
        ));
    }

    let published = lorehaven_db::narration::approve_narration_edition(db, &id).await?;
    Ok(Json(json!({
        "id": id,
        "state": "published",
        "changed": published,
    })))
}

/// Download the narration audio.
///
/// A published edition is a public door — it is an edition of a published work,
/// and the spec's acceptance is that it downloads. A draft is contributor-only,
/// which is the same rule the editor applies to an unpublished work.
pub async fn download_audio(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let db = state.db();
    let edition = lorehaven_db::media::find_media_edition(db, &id)
        .await?
        .ok_or_else(|| not_found("edition"))?;
    if edition.edition_kind != "narration" {
        return Err(not_found("narration"));
    }
    // Every path through this door asks the same eligibility question, because
    // §32.5's promise is that an anonymous, underage or opted-out reader meets
    // no adult item in *any* door — audio included. A narration of a work the
    // caller may read answers `Public`, and is then served only if it has been
    // published: an unpublished edition is the author's working copy, and its
    // existence is not disclosed (§3.3).
    let allowed = match reading_for_work(&state, &edition.work_id, session.as_ref()).await? {
        Reading::Contributor => true,
        Reading::Public => edition.published_at.is_some(),
        Reading::Denied(error) => return Err(ApiError(error)),
    };
    if !allowed {
        return Err(not_found("narration"));
    }

    let checksum = lorehaven_db::narration::narration_audio_checksum(db, &id)
        .await?
        .ok_or_else(|| not_found("narration audio"))?;
    let store = BlobStore::new(state.config().storage.root.clone());
    let bytes = store
        .get(db, &checksum)
        .await?
        .ok_or_else(|| not_found("narration audio"))?;

    // The stored media type comes from the file row; WAV is the only format any
    // engine in this build produces, and the fallback keeps a reader's player
    // from being told `application/octet-stream`.
    let media_type = "audio/wav";
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, media_type),
            (header::CONTENT_DISPOSITION, "inline"),
        ],
        bytes,
    )
        .into_response())
}
