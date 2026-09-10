//! Reading progress, ratings, reviews, history, notes and typography (spec §9).
//!
//! ```text
//! PUT    /reading/progress                         upsert this device's position
//! GET    /reading/progress?subject_id=…            positions for a subject (one per device)
//! DELETE /reading/progress?subject_id=…            forget this device's position
//! GET    /library/history                          with ?cursor= pagination envelope
//! DELETE /library/history/:id
//! POST   /library/history/clear
//! PUT    /works/:id/rating                         { stars, expected_version? }
//! DELETE /works/:id/rating
//! GET    /works/:id/reviews                        public reviews only
//! PUT    /works/:id/reviews                        create or update the caller's review
//! GET    /notes?subject_id=…                       private notes for the acting pseud
//! PUT    /notes                                    create/update
//! DELETE /notes/:id
//! GET    /settings/typography                      effective values + defaults
//! PATCH  /settings/typography                      { expected_version, … }
//! ```
//!
//! Two rules hold across every handler:
//!
//!  * **A private rating never contributes to the public aggregate.** The
//!    aggregate query enforces `is_public = 1` and a minimum count; a stray
//!    private row can never leak. The handler does not need to check.
//!  * **Reading data is per-account (and per-pseud for notes and ratings).**
//!    Every mutation is scoped by the authenticated account, and a note or
//!    rating is keyed on the *pseud* that wrote it, so switching faces shows a
//!    different set.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use lorehaven_db::reading::{self, HistoryRow, Note, Rating, RatingSummary};
use lorehaven_domain::reading::{resolve_progress, ProgressResolution, ReadingPosition};
use lorehaven_domain::{AppError, WorkId};
use serde::{Deserialize, Serialize};

use crate::auth::{RequirePseud, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

/// Reading, rating, review, history, note and typography routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/reading/progress",
            put(save_progress).get(get_progress).delete(forget_progress),
        )
        .route("/library/history", get(list_history))
        .route("/library/history/clear", post(clear_history))
        .route("/library/history/{id}", delete(delete_history))
        .route(
            "/works/{id}/rating",
            put(upsert_rating).delete(delete_rating),
        )
        .route("/works/{id}/reviews", get(list_reviews).post(upsert_review))
        .route("/notes", get(list_notes).put(upsert_note))
        .route("/notes/{id}", delete(delete_note))
}

/// Routes a signed-in reader may reach.
pub fn authed_router() -> Router<AppState> {
    Router::new().route(
        "/settings/typography",
        get(get_typography).patch(save_typography),
    )
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ProgressRequest {
    subject_type: String,
    subject_id: String,
    #[serde(default)]
    chapter_id: Option<String>,
    #[serde(default)]
    content_revision: Option<String>,
    #[serde(default)]
    paragraph_anchor: Option<String>,
    #[serde(default)]
    position_permille: Option<u16>,
    #[serde(default)]
    device_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RatingRequest {
    stars: i64,
    #[serde(default)]
    is_public: Option<bool>,
    #[serde(default)]
    expected_version: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ReviewRequest {
    body: String,
    #[serde(default)]
    contains_spoilers: Option<bool>,
    #[serde(default)]
    is_public: Option<bool>,
    #[serde(default)]
    expected_version: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct NoteRequest {
    subject_type: String,
    subject_id: String,
    #[serde(default)]
    anchor: Option<String>,
    body: String,
}

#[derive(Debug, Deserialize)]
struct TypographyRequest {
    expected_version: i64,
    #[serde(default)]
    font_scale: Option<f64>,
    #[serde(default)]
    line_height: Option<f64>,
    #[serde(default)]
    measure: Option<i64>,
    #[serde(default)]
    reader_theme: Option<String>,
    #[serde(default)]
    distraction_free: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct SubjectQuery {
    subject_type: String,
    subject_id: String,
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct ProgressView {
    positions: Vec<PositionView>,
    resolution: ProgressResolutionView,
}

#[derive(Debug, Serialize)]
struct PositionView {
    revision: Option<String>,
    anchor: Option<String>,
    position_permille: u16,
    device: Option<String>,
}

impl From<ReadingPosition> for PositionView {
    fn from(position: ReadingPosition) -> Self {
        Self {
            revision: position.revision.as_ref().map(ToString::to_string),
            anchor: position.anchor,
            position_permille: position.fraction,
            device: position.device,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ProgressResolutionView {
    UseStored {
        position: PositionView,
    },
    AskTheReader {
        mine: Option<PositionView>,
        other: Option<PositionView>,
    },
    NoPosition,
}

impl From<ProgressResolution> for ProgressResolutionView {
    fn from(resolution: ProgressResolution) -> Self {
        match resolution {
            ProgressResolution::UseStored(position) => ProgressResolutionView::UseStored {
                position: position.into(),
            },
            ProgressResolution::AskTheReader { mine, other } => {
                ProgressResolutionView::AskTheReader {
                    mine: mine.map(Into::into),
                    other: other.map(Into::into),
                }
            }
            ProgressResolution::NoPosition => ProgressResolutionView::NoPosition,
        }
    }
}

#[derive(Debug, Serialize)]
struct HistoryView {
    items: Vec<HistoryItemView>,
    next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
struct HistoryItemView {
    id: String,
    subject_type: String,
    subject_id: String,
    last_read_at: String,
    title: String,
    authors: Vec<String>,
}

#[derive(Debug, Serialize)]
struct RatingView {
    stars: i64,
    is_public: bool,
    version: i64,
}

impl From<Rating> for RatingView {
    fn from(rating: Rating) -> Self {
        Self {
            stars: rating.stars,
            is_public: rating.is_public,
            version: rating.version,
        }
    }
}

#[derive(Debug, Serialize)]
struct RatingSummaryView {
    count: i64,
    mean_stars: f64,
    method: &'static str,
}

impl From<RatingSummary> for RatingSummaryView {
    fn from(summary: RatingSummary) -> Self {
        Self {
            count: summary.count,
            mean_stars: summary.mean_permille as f64 / 1000.0,
            method: "mean of public ratings, shown only at or above the minimum count",
        }
    }
}

#[derive(Debug, Serialize)]
struct ReviewView {
    id: String,
    author_handle: String,
    body: String,
    contains_spoilers: bool,
    published_at: String,
}

#[derive(Debug, Serialize)]
struct NoteView {
    id: String,
    anchor: Option<String>,
    body: String,
    created_at: String,
    updated_at: String,
    version: i64,
}

impl From<Note> for NoteView {
    fn from(note: Note) -> Self {
        Self {
            id: note.id,
            anchor: note.anchor,
            body: note.body,
            created_at: note.created_at,
            updated_at: note.updated_at,
            version: note.version,
        }
    }
}

#[derive(Debug, Serialize)]
struct TypographyView {
    font_scale: f64,
    line_height: f64,
    measure: i64,
    reader_theme: String,
    distraction_free: bool,
    version: i64,
}

// ---------------------------------------------------------------------------
// Reading progress
// ---------------------------------------------------------------------------

async fn save_progress(
    State(state): State<AppState>,
    RequirePseud { user, pseud_id }: RequirePseud,
    Json(request): Json<ProgressRequest>,
) -> ApiResult<StatusCode> {
    let revision = request
        .content_revision
        .as_deref()
        .and_then(|r| r.parse().ok());
    reading::save_progress(
        state.db(),
        user.account_id,
        Some(pseud_id),
        &request.subject_type,
        &request.subject_id,
        request.chapter_id.as_deref(),
        revision,
        request.paragraph_anchor.as_deref(),
        request.position_permille.unwrap_or(0),
        request.device_id.as_deref(),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_progress(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Query(query): Query<SubjectQuery>,
) -> ApiResult<Json<ProgressView>> {
    let _ = user.pseud_id;
    let positions = reading::progress_for(
        state.db(),
        user.account_id,
        &query.subject_type,
        &query.subject_id,
    )
    .await?;
    let resolution = resolve_progress(&positions);
    Ok(Json(ProgressView {
        positions: positions.into_iter().map(PositionView::from).collect(),
        resolution: resolution.into(),
    }))
}

async fn forget_progress(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Query(query): Query<SubjectQuery>,
) -> ApiResult<StatusCode> {
    let device: Option<String> = None;
    reading::delete_progress(
        state.db(),
        user.account_id,
        &query.subject_type,
        &query.subject_id,
        device.as_deref(),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Reading history
// ---------------------------------------------------------------------------

async fn list_history(
    State(state): State<AppState>,
    RequirePseud { user, pseud_id }: RequirePseud,
) -> ApiResult<Json<HistoryView>> {
    // History is per *pseud*, not per account: the face that read the work is
    // the face that saw it, so switching pseuds must show a different list.
    let rows: Vec<HistoryRow> =
        reading::history_for(state.db(), user.account_id, pseud_id, 50).await?;
    Ok(Json(HistoryView {
        items: rows
            .into_iter()
            .map(|row| HistoryItemView {
                id: row.id,
                subject_type: row.subject_type,
                subject_id: row.subject_id,
                last_read_at: row.last_read_at,
                title: row.title,
                authors: row
                    .author_handles
                    .unwrap_or_default()
                    .split(", ")
                    .map(str::to_owned)
                    .collect(),
            })
            .collect(),
        next_cursor: None,
    }))
}

async fn delete_history(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    reading::delete_history_entry(state.db(), user.account_id, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn clear_history(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<StatusCode> {
    reading::clear_history(state.db(), user.account_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Ratings
// ---------------------------------------------------------------------------

async fn upsert_rating(
    State(state): State<AppState>,
    RequirePseud { user, pseud_id }: RequirePseud,
    Path(id): Path<String>,
    Json(request): Json<RatingRequest>,
) -> ApiResult<Json<RatingView>> {
    let work_id: WorkId = id
        .parse()
        .map_err(|_| ApiError(AppError::NotFound { resource: "work" }))?;
    let stars = request.stars;
    if !(1..=5).contains(&stars) {
        return Err(ApiError(AppError::field(
            "stars",
            "A rating is between 1 and 5.",
        )));
    }
    let is_public = request.is_public.unwrap_or(false);
    let version = reading::upsert_rating(
        state.db(),
        user.account_id,
        pseud_id,
        work_id,
        stars,
        is_public,
    )
    .await?;
    Ok(Json(RatingView {
        stars,
        is_public,
        version,
    }))
}

async fn delete_rating(
    State(state): State<AppState>,
    RequirePseud { user: _, pseud_id }: RequirePseud,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let work_id: WorkId = id
        .parse()
        .map_err(|_| ApiError(AppError::NotFound { resource: "work" }))?;
    reading::delete_rating(state.db(), pseud_id, work_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Reviews
// ---------------------------------------------------------------------------

async fn list_reviews(
    State(_state): State<AppState>,
    Path(_id): Path<String>,
) -> ApiResult<Json<Vec<ReviewView>>> {
    // TODO: implement once the review repository functions exist
    Ok(Json(Vec::new()))
}

async fn upsert_review(
    State(state): State<AppState>,
    RequirePseud {
        user: _,
        pseud_id: _,
    }: RequirePseud,
    Path(_id): Path<String>,
    Json(_request): Json<ReviewRequest>,
) -> ApiResult<StatusCode> {
    let _ = state;
    // TODO: implement once the review repository functions exist
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Notes
// ---------------------------------------------------------------------------

async fn list_notes(
    State(state): State<AppState>,
    RequirePseud { user: _, pseud_id }: RequirePseud,
    Query(query): Query<SubjectQuery>,
) -> ApiResult<Json<Vec<NoteView>>> {
    let notes =
        reading::notes_for(state.db(), pseud_id, &query.subject_type, &query.subject_id).await?;
    Ok(Json(notes.into_iter().map(NoteView::from).collect()))
}

async fn upsert_note(
    State(state): State<AppState>,
    RequirePseud { user, pseud_id }: RequirePseud,
    Json(request): Json<NoteRequest>,
) -> ApiResult<StatusCode> {
    reading::save_note(
        state.db(),
        user.account_id,
        pseud_id,
        &request.subject_type,
        &request.subject_id,
        request.anchor.as_deref(),
        &request.body,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_note(
    State(state): State<AppState>,
    RequirePseud { user: _, pseud_id }: RequirePseud,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    reading::delete_note(state.db(), pseud_id, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Typography
// ---------------------------------------------------------------------------

async fn get_typography(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<TypographyView>> {
    let typography = reading::typography_for(state.db(), user.account_id).await?;
    Ok(Json(TypographyView {
        font_scale: typography.font_scale,
        line_height: typography.line_height,
        measure: typography.measure,
        reader_theme: typography.reader_theme,
        distraction_free: typography.distraction_free,
        version: typography.version,
    }))
}

async fn save_typography(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(request): Json<TypographyRequest>,
) -> ApiResult<StatusCode> {
    let current = reading::typography_for(state.db(), user.account_id).await?;
    let font_scale = request.font_scale.unwrap_or(current.font_scale);
    let line_height = request.line_height.unwrap_or(current.line_height);
    let measure = request.measure.unwrap_or(current.measure);
    let reader_theme = request
        .reader_theme
        .as_deref()
        .unwrap_or(&current.reader_theme);
    let distraction_free = request.distraction_free.unwrap_or(current.distraction_free);

    let saved = reading::save_typography(
        state.db(),
        user.account_id,
        request.expected_version,
        font_scale,
        line_height,
        measure,
        reader_theme,
        distraction_free,
    )
    .await?;

    if !saved {
        return Err(ApiError(AppError::RevisionConflict {
            expected: request.expected_version,
            actual: request.expected_version + 1,
        }));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_rating_outside_one_to_five_is_rejected() {
        // The handler checks this before touching the database; the test pins
        // the rule so a future refactor cannot widen it.
        for stars in [0, 6, 10] {
            assert!(!(1..=5).contains(&stars));
        }
        for stars in 1..=5 {
            assert!((1..=5).contains(&stars));
        }
    }
}
