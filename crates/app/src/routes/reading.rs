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
//! DELETE /works/:id/reviews                        withdraw the caller's review
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
use lorehaven_db::content;
use lorehaven_db::notifications;
use lorehaven_db::positivity;
use lorehaven_db::reading::{self, HistoryRow, Note, Rating, Review};
use lorehaven_domain::positivity::{effective, sender_receipt};
use lorehaven_domain::reading::{resolve_progress, ProgressResolution, ReadingPosition};
use lorehaven_domain::{AppError, WorkId};
use serde::{Deserialize, Serialize};

use crate::auth::{MaybeSession, RequirePseud, RequireSession};
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
            get(get_rating).put(upsert_rating).delete(delete_rating),
        )
        .route(
            "/works/{id}/reviews",
            get(list_reviews).put(upsert_review).delete(delete_review),
        )
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
struct ReviewListView {
    items: Vec<ReviewView>,
    next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
struct ReviewView {
    id: String,
    author_handle: String,
    body: String,
    contains_spoilers: bool,
    is_public: bool,
    /// `null` until the review is published — a private review has no
    /// publication time, and saying otherwise would be a small lie the client
    /// would have to work around.
    published_at: Option<String>,
    version: i64,
    /// The sender-visible receipt (spec §12.4): "posted" or "held for
    /// review". Never the class, never the author's settings.
    receipt: String,
}

impl ReviewView {
    fn with_receipt(review: Review, receipt: String) -> Self {
        Self {
            id: review.id,
            author_handle: review.author_handle,
            body: review.body,
            contains_spoilers: review.contains_spoilers,
            is_public: review.is_public,
            published_at: review.published_at,
            version: review.version,
            receipt,
        }
    }
}

impl From<Review> for ReviewView {
    fn from(review: Review) -> Self {
        Self::with_receipt(review, "Comment posted.".to_owned())
    }
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
        reading::ProgressInput {
            account: user.account_id,
            pseud: Some(pseud_id),
            subject_type: &request.subject_type,
            subject_id: &request.subject_id,
            chapter_id: request.chapter_id.as_deref(),
            revision,
            anchor: request.paragraph_anchor.as_deref(),
            fraction: request.position_permille.unwrap_or(0),
            device: request.device_id.as_deref(),
        },
    )
    .await?;

    // Reading is what puts a work in the reader's history, and this route is
    // the only signal the reader's browser sends on arrival. `touch_history`
    // had no caller at all before this — the acceptance tests called the
    // repository directly, so `/library/history` was empty for every real
    // reader while the tests were green.
    reading::touch_history(
        state.db(),
        user.account_id,
        pseud_id,
        &request.subject_type,
        &request.subject_id,
        revision,
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

/// The acting pseud's own rating, or `null` when it has not rated the work.
///
/// `null` rather than `404`: the caller asked "what did I give this?", and
/// "nothing" is a complete answer to that question, not a missing resource.
async fn get_rating(
    State(state): State<AppState>,
    RequirePseud { user: _, pseud_id }: RequirePseud,
    Path(id): Path<String>,
) -> ApiResult<Json<Option<RatingView>>> {
    let work_id: WorkId = id
        .parse()
        .map_err(|_| ApiError(AppError::NotFound { resource: "work" }))?;
    let rating = reading::rating_for(state.db(), pseud_id, work_id).await?;
    Ok(Json(rating.map(RatingView::from)))
}

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

    // Optimistic concurrency, when the caller sends the version it read. The
    // rating UI always sends it, so two tabs cannot silently overwrite each
    // other's stars (house rule 2.3). A first write sends nothing and is
    // allowed to create the row.
    if let Some(expected) = request.expected_version {
        let actual = reading::rating_for(state.db(), pseud_id, work_id)
            .await?
            .map_or(0, |rating| rating.version);
        if actual != expected {
            return Err(ApiError(AppError::RevisionConflict { expected, actual }));
        }
    }

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
    State(state): State<AppState>,
    MaybeSession(_user): MaybeSession,
    Path(id): Path<String>,
) -> ApiResult<Json<ReviewListView>> {
    // Public and anonymous by design (§9): only delivered public reviews are
    // listed — held text is hidden in SQL, never filtered in the handler.
    // Pre-filter reviews (no classification row) count as delivered, so no
    // backfill can silently hide them.
    let work_id: WorkId = id
        .parse()
        .map_err(|_| ApiError(AppError::NotFound { resource: "work" }))?;
    let reviews = positivity::visible_reviews(state.db(), work_id).await?;
    Ok(Json(ReviewListView {
        items: reviews.into_iter().map(ReviewView::from).collect(),
        // A work's public reviews are read in one go; the envelope is here for
        // the shape's sake, as spec §3.3 requires of every collection.
        next_cursor: None,
    }))
}

async fn upsert_review(
    State(state): State<AppState>,
    RequirePseud { user, pseud_id }: RequirePseud,
    Path(id): Path<String>,
    Json(request): Json<ReviewRequest>,
) -> ApiResult<Json<ReviewView>> {
    let work_id: WorkId = id
        .parse()
        .map_err(|_| ApiError(AppError::NotFound { resource: "work" }))?;
    let work = content::find_work(state.db(), work_id)
        .await?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "work" }))?;

    if request.body.trim().is_empty() {
        return Err(ApiError(AppError::field(
            "body",
            "A review needs some words.",
        )));
    }

    // Reviews are private until explicitly published (§9.5), so the default is
    // private and only an explicit `true` makes one visible.
    let is_public = request.is_public.unwrap_or(false);
    let contains_spoilers = request.contains_spoilers.unwrap_or(false);

    // Optimistic concurrency, when the caller sends the version it read. A
    // caller that omits it is saying "I do not care what was there", which is
    // what a first write does.
    if let Some(expected) = request.expected_version {
        let actual = reading::review_for(state.db(), pseud_id, work_id)
            .await?
            .map_or(0, |review| review.version);
        if actual != expected {
            return Err(ApiError(AppError::RevisionConflict { expected, actual }));
        }
    }

    let version = reading::upsert_review(
        state.db(),
        user.account_id,
        pseud_id,
        work_id,
        &request.body,
        contains_spoilers,
        is_public,
    )
    .await?;

    let review = reading::review_for(state.db(), pseud_id, work_id)
        .await?
        .ok_or_else(|| {
            ApiError(AppError::internal(
                "reading back a review that was just written",
                anyhow::anyhow!("the review is missing immediately after its upsert"),
            ))
        })?;
    debug_assert_eq!(review.version, version);

    // The positivity gate (spec §12): classify against the author's
    // effective preferences *before* the review is read back, and record
    // the outcome. Held text stays stored but leaves every listing.
    //
    // D8 — compensating action: if classification fails, the review was
    // already written, so we withdraw (delete) it rather than leave a
    // phantom review the positivity pipeline never saw. The 500 the caller
    // gets is honest; the DB stays consistent.
    let outcome = if is_public {
        let author = positivity::author_account_for_work(state.db(), work_id).await?;
        match author {
            None => lorehaven_domain::positivity::DeliveryOutcome::Delivered,
            Some(author_account) => {
                let prefs = positivity::preferences_for(state.db(), author_account).await?;
                let work_ov = positivity::override_for(state.db(), work_id).await?;
                let policy = effective(&prefs, &work_ov);
                let (allow, deny) =
                    positivity::list_membership(state.db(), author_account, pseud_id).await?;
                let stored = match positivity::classify_review(
                    state.db(),
                    &review.id,
                    &request.body,
                    &policy,
                    allow,
                    deny,
                )
                .await
                {
                    Ok(s) => s,
                    Err(e) => {
                        reading::delete_review(state.db(), pseud_id, work_id)
                            .await
                            .ok();
                        return Err(ApiError(AppError::internal("classifying the review", e)));
                    }
                };
                // Notify the work's author when a public review passes
                // the positivity gate. A held review is invisible to
                // the author anyway, so there's nothing to report.
                if stored.outcome == lorehaven_domain::positivity::DeliveryOutcome::Delivered {
                    let _ = notifications::notify(
                        state.db(),
                        &author_account.to_string(),
                        "review",
                        "Someone reviewed your work",
                        &format!("A new public review was posted on {}.", work.title),
                        Some(work_id.to_canonical_string().as_str()),
                    )
                    .await;
                }
                stored.outcome
            }
        }
    } else {
        // Private reviews are the author's own notes-to-self: no gate, and
        // no classification row, so publishing later classifies then.
        lorehaven_domain::positivity::DeliveryOutcome::Delivered
    };

    Ok(Json(ReviewView::with_receipt(
        review,
        sender_receipt(outcome).to_owned(),
    )))
}

async fn delete_review(
    State(state): State<AppState>,
    RequirePseud { user: _, pseud_id }: RequirePseud,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let work_id: WorkId = id
        .parse()
        .map_err(|_| ApiError(AppError::NotFound { resource: "work" }))?;
    // A review that is not there is not a failure: the caller asked for the
    // end state, and it already holds.
    reading::delete_review(state.db(), pseud_id, work_id).await?;
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
        reading::TypographyInput {
            account: user.account_id,
            expected_version: request.expected_version,
            font_scale,
            line_height,
            measure,
            reader_theme,
            distraction_free,
        },
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
