//! Works, chapters, revisions and publication endpoints (spec §8).
//!
//! ```text
//! GET    /works                         the acting pseud's own works
//! POST   /works                         create a draft
//! GET    /works/:id                     read (contributor view, or public)
//! PATCH  /works/:id                     edit metadata
//! POST   /works/:id/publish             publish / republish, idempotent
//! POST   /works/:id/withdraw            withdraw
//! POST   /works/:id/chapters            append a chapter
//! GET    /works/:id/chapters/:chapter   read a chapter
//! POST   /works/:id/reorder-chapters    reorder
//! PATCH  /chapters/:id                  rename, and/or save text
//! GET    /chapters/:id/revisions        revision history
//! POST   /chapters/:id/restore-revision bring an old revision back
//! ```
//!
//! Two rules hold across every handler, and both are enforced by *loading*
//! rather than by checking afterwards:
//!
//! * **A work is reached through the acting pseud's contributions.** An account
//!   that switches pseuds does not thereby gain access to a work it wrote under
//!   another face (spec §8 acceptance). A work the acting pseud does not
//!   contribute to is `404`, not `403`: whether it exists is not disclosed.
//! * **Reading goes through the one eligibility service** in
//!   `lorehaven_domain::policy`, so a draft, a withdrawn work or an
//!   over-rating work is refused identically wherever it is asked for.
//!
//! And one property the reader endpoints must hold (spec §8 acceptance):
//! **public readers never receive an unpublished revision.** The reader path
//! serves the chapter's *current* revision, and the lifecycle check happens
//! before the chapter is loaded at all.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use lorehaven_db::collaboration;
use lorehaven_db::content::{
    self, ContentError, PublicationOutcome, RevisionInput, RevisionSummary, Work, WorkPatch,
};
use lorehaven_db::identity;
use lorehaven_db::permission;
use lorehaven_db::reading;
use lorehaven_db::taxonomy;
use lorehaven_db::work_metrics;
use lorehaven_domain::content::Contributor;
use lorehaven_domain::document::Document;
use lorehaven_domain::permission::{
    ExclusionTarget, LineageEdge, LineageKind, Permission, PermissionStatement,
};
use lorehaven_domain::policy::{
    can_access_content, AccessPolicy, Actor, ContentFacts, Decision, DenyReason, Lifecycle,
    Visibility,
};
use lorehaven_domain::{AppError, ChapterId, PseudId, RevisionId, WorkId};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

use crate::auth::{MaybeSession, RequireSession, SessionUser};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

/// Work, chapter and revision routes.
///
/// Split in two because the reader surface and the writing surface have
/// different audiences and therefore different rate-limit classes: a visitor
/// reading a published chapter must not share a bucket with an author saving
/// drafts.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/works", get(list_works).post(create_work))
        .route("/works/{id}", patch(update_work))
        .route("/works/{id}/kudos", post(toggle_kudos))
        .route("/works/{id}/publish", post(publish_work))
        .route("/works/{id}/withdraw", post(withdraw_work))
        .route("/works/{id}/chapters", post(add_chapter))
        .route("/works/{id}/reorder-chapters", post(reorder_chapters))
        .route(
            "/works/{id}/permissions",
            get(get_work_permissions).put(put_work_permissions),
        )
        .route("/works/{id}/lineage", get(list_lineage).post(add_lineage))
        .route("/works/{id}/fork", post(fork_work))
        .route("/chapters/{id}", patch(update_chapter))
        .route("/chapters/{id}/revisions", get(list_revisions))
        .route("/chapters/{id}/restore-revision", post(restore_revision))
}

/// Routes a visitor may reach with no session at all.
///
/// Both handlers still load the session when there is one, because a
/// contributor reading their own draft must be answered by the same URL that
/// answers a stranger with `404`.
pub fn read_router() -> Router<AppState> {
    Router::new()
        .route("/works/{id}", get(read_work))
        .route("/works/{id}/chapters/{chapter}", get(read_chapter))
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

/// A work as its contributor sees it.
#[derive(Debug, Serialize)]
struct AuthorWorkView {
    id: WorkId,
    title: String,
    summary: String,
    language: String,
    rating: String,
    visibility: String,
    lifecycle: String,
    completion: String,
    /// Optimistic-concurrency version; the next `PATCH` must send it back.
    version: i64,
    created_at: String,
    updated_at: String,
    published_at: Option<String>,
    withdrawn_at: Option<String>,
    show_public_ratings: bool,
    /// The effective discussion mode (thread_only/comments_only/both).
    discussion_mode: String,
    /// What the *acting* pseud may do with it.
    role: String,
    chapters: Vec<ChapterView>,
    contributors: Vec<ContributorView>,
    /// Why the work cannot be published yet, if it cannot.
    publication_blockers: Vec<String>,
}

/// A contributor as the author's own view shows them.
#[derive(Debug, Serialize)]
struct ContributorView {
    pseud_id: PseudId,
    handle: String,
    display_name: String,
    role: String,
    public_attribution: bool,
}

/// A chapter, with its text only when the caller asked for one chapter.
#[derive(Debug, Serialize)]
struct ChapterView {
    id: ChapterId,
    title: String,
    order_key: i64,
    word_count: i64,
    revision_count: i64,
    version: i64,
    updated_at: String,
    created_at: String,
    has_content: bool,
    current_revision_id: Option<RevisionId>,
}

impl From<content::Chapter> for ChapterView {
    fn from(chapter: content::Chapter) -> Self {
        // Computed before the fields are moved out of `chapter`.
        let has_content = chapter.has_content();
        Self {
            id: chapter.id,
            title: chapter.title,
            order_key: chapter.order_key,
            word_count: chapter.word_count,
            revision_count: chapter.revision_count,
            version: chapter.version,
            updated_at: chapter.updated_at,
            created_at: chapter.created_at,
            has_content,
            current_revision_id: chapter.current_revision_id,
        }
    }
}

/// A chapter's text, as whoever may read it receives it.
#[derive(Debug, Serialize)]
struct ChapterContentView {
    chapter: ChapterView,
    /// The editor document, as the schema stores it. Contributors only.
    document: Option<serde_json::Value>,
    /// Derived sanitized HTML.
    sanitized_html: String,
    /// Derived plain text.
    plain_text: String,
    word_count: i64,
    revision_number: Option<i64>,
    revision_id: Option<RevisionId>,
    /// The acting pseud may change this chapter.
    editable: bool,
    /// Neighbouring chapters, so a reader can move without a table of contents.
    previous_chapter_id: Option<ChapterId>,
    next_chapter_id: Option<ChapterId>,
    work: ChapterWorkView,
}

/// The little bit of work metadata a chapter page needs.
#[derive(Debug, Serialize)]
struct ChapterWorkView {
    id: WorkId,
    title: String,
    lifecycle: String,
    /// Public author credits.
    authors: Vec<PublicAuthor>,
}

/// A publicly credited author.
#[derive(Debug, Serialize)]
struct PublicAuthor {
    handle: String,
    display_name: String,
    role: String,
}

/// A public work page.
#[derive(Debug, Serialize)]
struct PublicWorkView {
    id: WorkId,
    title: String,
    summary: String,
    language: String,
    rating: String,
    visibility: String,
    completion: String,
    published_at: Option<String>,
    show_public_ratings: bool,
    /// The effective discussion mode (thread_only/comments_only/both).
    discussion_mode: String,
    /// The public aggregate rating, when the owner allows it and the minimum
    /// count is met. `null` otherwise, including below the threshold.
    rating_summary: Option<PublicRatingSummaryView>,
    /// Public engagement counts (views, kudos, bookmarks, …). `null` when the
    /// owner has turned off public rating display, which gates the whole
    /// metric bar (spec §9.5: "Work owners may disable display of public
    /// rating aggregates").
    metrics: Option<PublicWorkMetricsView>,
    authors: Vec<PublicAuthor>,
    chapters: Vec<ChapterView>,
}

/// Public engagement counts for a work card. Counts, never averages: each
/// number is "how many readers did X" (spec §9.4 "count once per reader per
/// target").
#[derive(Debug, Serialize)]
struct PublicWorkMetricsView {
    views: i64,
    complete_reads: i64,
    reactions: i64,
    kudos: i64,
    bookmarks: i64,
    collection_adds: i64,
    reviews: i64,
}

/// The public aggregate rating, with the count and the method it used.
///
/// The method travels with the number because spec §9.5 requires the aggregate
/// to state how it was computed, and a mean is not the only thing it might one
/// day be.
#[derive(Debug, Serialize)]
struct PublicRatingSummaryView {
    count: i64,
    mean_stars: f64,
    method: &'static str,
}

/// The words every aggregate carries, so the rule lives in one place.
const MEAN_OF_PUBLIC_RATINGS: &str =
    "mean of public ratings, shown only at or above the minimum count";

/// A chapter in the revision history list.
#[derive(Debug, Serialize)]
struct RevisionView {
    id: String,
    revision_number: i64,
    word_count: i64,
    note: Option<String>,
    created_at: String,
    restored_from_id: Option<String>,
    author_handle: String,
    /// Whether this is the revision readers currently get.
    current: bool,
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CreateWorkRequest {
    #[serde(default)]
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateWorkRequest {
    expected_version: i64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    rating: Option<String>,
    #[serde(default)]
    visibility: Option<String>,
    #[serde(default)]
    completion: Option<String>,
    #[serde(default)]
    show_public_ratings: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct PublishRequest {
    expected_version: i64,
    #[serde(default)]
    idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateChapterRequest {
    #[serde(default)]
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateChapterRequest {
    expected_version: i64,
    #[serde(default)]
    title: Option<String>,
    /// The editor document. Absent leaves the text alone; present appends a
    /// revision.
    #[serde(default)]
    document: Option<serde_json::Value>,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReorderRequest {
    chapters: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RestoreRequest {
    revision_id: String,
}

/// Pagination-ish query for the revision list.
#[derive(Debug, Deserialize)]
struct ReadQuery {
    /// `?document=0` asks for text without the editor document.
    #[serde(default)]
    document: Option<String>,
}

// ---------------------------------------------------------------------------
// Works
// ---------------------------------------------------------------------------

async fn list_works(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Vec<content::OwnedWork>>> {
    let pseud = acting_pseud(&user)?;
    let works = content::works_for_pseud(state.db(), pseud).await?;
    Ok(Json(works))
}

async fn create_work(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(request): Json<CreateWorkRequest>,
) -> ApiResult<(StatusCode, Json<AuthorWorkView>)> {
    let pseud = acting_pseud(&user)?;
    require_participation(&user)?;

    let title = validate_title(request.title.as_deref().unwrap_or(""), false)?;
    let work = content::create_work(state.db(), pseud, &title, None).await?;

    let view = author_view(&state, &user, &work).await?;
    Ok((StatusCode::CREATED, Json(view)))
}

async fn read_work(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let work_id = parse_work_id(&id)?;
    let work = content::find_work(state.db(), work_id)
        .await?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "work" }))?;

    let contributors = collaboration::contributors_for_work(state.db(), work_id).await?;
    let actor = actor_for(session.as_ref());

    match reading_decision(&state, actor.as_ref(), &work, &contributors).await {
        Reading::Contributor => {}
        Reading::Public => {
            // A public reader never receives the editor document, and never
            // receives a chapter that is not currently published.
            let view = public_view(&state, &work).await?;
            record_view_for_work(&state, &work_id, session.as_ref())
                .await
                .ok();
            return Ok(Json(serde_json::to_value(view).map_err(internal)?));
        }
        Reading::Denied(error) => return Err(ApiError(error)),
    }

    let user = session.ok_or(ApiError(AppError::AuthRequired))?;
    let view = author_view(&state, &user, &work).await?;
    Ok(Json(serde_json::to_value(view).map_err(internal)?))
}

async fn update_work(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Json(request): Json<UpdateWorkRequest>,
) -> ApiResult<Json<AuthorWorkView>> {
    let (work, contributors) = author_work(&state, &user, &id).await?;

    if let Decision::Deny(reason) = can_edit(&user, &contributors) {
        return Err(refusal(reason));
    }

    let patch = work_patch(&request)?;

    let changed =
        content::update_work(state.db(), work.id, request.expected_version, &patch).await?;

    if !changed {
        let actual = content::current_work_version(state.db(), work.id).await?;
        return Err(ApiError(AppError::RevisionConflict {
            expected: request.expected_version,
            actual,
        }));
    }

    let updated = reload(&state, work.id).await?;

    // Lifecycle incentives (spec §9.8): a work newly marked complete fires a
    // one-time completion event; a work revived from abandoned fires a
    // one-time resurrection event. Both are best-effort — an incentive failure
    // must never fail the edit.
    if let Some(completion) = &patch.completion {
        let work_id = work.id.to_string();
        if *completion == "complete" && work.completion != "complete" {
            if let Err(error) =
                lorehaven_db::engagement::record_lifecycle_event(state.db(), &work_id, "completion")
                    .await
            {
                tracing::warn!(work = %work_id, %error, "failed to record completion event");
            }
        }
        if *completion != "abandoned" && work.completion == "abandoned" {
            if let Err(error) = lorehaven_db::engagement::record_lifecycle_event(
                state.db(),
                &work_id,
                "resurrection",
            )
            .await
            {
                tracing::warn!(work = %work_id, %error, "failed to record resurrection event");
            }
        }
    }

    // A work that was listed and is no longer listable (or whose rating moved)
    // must leave the surfaces that cached it (spec §8 acceptance). The
    // deindexing is enqueued, not performed here: the request that changes a
    // work is not the request that should spend time on a search cluster.
    if let Some(visibility) = &request.visibility {
        let was_listable = work.visibility == "public";
        let now_listable = visibility == "public";
        if was_listable && !now_listable {
            lorehaven_db::outbox::enqueue(
                state.db(),
                "visibility.deindex",
                &serde_json::json!({ "work_id": work.id }).to_string(),
                Some(&format!("work:{}:visibility:{}", work.id, updated.version)),
            )
            .await?;
        }
    }
    if request.rating.is_some() && request.rating.as_deref() != Some(work.rating.as_str()) {
        lorehaven_db::outbox::enqueue(
            state.db(),
            "rating.reindex",
            &serde_json::json!({ "work_id": work.id }).to_string(),
            Some(&format!("work:{}:rating:{}", work.id, updated.version)),
        )
        .await?;
    }

    Ok(Json(author_view(&state, &user, &updated).await?))
}

async fn publish_work(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Json(request): Json<PublishRequest>,
) -> ApiResult<Json<AuthorWorkView>> {
    let (work, _) = author_work(&state, &user, &id).await?;
    let actor = user.actor(acting_pseud(&user)?);

    let outcome = content::publish_work(
        state.db(),
        &work,
        &actor,
        request.expected_version,
        request.idempotency_key.as_deref(),
    )
    .await
    .map_err(from_content)?;

    let updated = reload(&state, work.id).await?;

    if matches!(outcome, PublicationOutcome::AlreadyApplied) {
        tracing::debug!(work = %work.id, "publication replayed for an idempotency key");
    }

    Ok(Json(author_view(&state, &user, &updated).await?))
}

async fn withdraw_work(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Json(request): Json<PublishRequest>,
) -> ApiResult<Json<AuthorWorkView>> {
    let (work, _) = author_work(&state, &user, &id).await?;
    let actor = user.actor(acting_pseud(&user)?);

    content::withdraw_work(
        state.db(),
        &work,
        &actor,
        request.expected_version,
        request.idempotency_key.as_deref(),
    )
    .await
    .map_err(from_content)?;

    let updated = reload(&state, work.id).await?;
    Ok(Json(author_view(&state, &user, &updated).await?))
}

// ---------------------------------------------------------------------------
// Chapters
// ---------------------------------------------------------------------------

async fn add_chapter(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Json(request): Json<CreateChapterRequest>,
) -> ApiResult<(StatusCode, Json<ChapterView>)> {
    let (work, contributors) = author_work(&state, &user, &id).await?;

    if let Decision::Deny(reason) = can_edit(&user, &contributors) {
        return Err(refusal(reason));
    }

    let title = validate_title(request.title.as_deref().unwrap_or(""), false)?;
    let chapter = content::create_chapter(state.db(), work.id, &title).await?;

    Ok((StatusCode::CREATED, Json(ChapterView::from(chapter))))
}

async fn update_chapter(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Json(request): Json<UpdateChapterRequest>,
) -> ApiResult<Json<ChapterView>> {
    let chapter_id = parse_chapter_id(&id)?;
    let chapter = content::find_chapter(state.db(), chapter_id)
        .await?
        .ok_or_else(|| {
            ApiError(AppError::NotFound {
                resource: "chapter",
            })
        })?;

    // Loading the parent work through the acting pseud is the authorization
    // step: a stranger's chapter is not found rather than found-and-refused.
    let (work, contributors) = author_work(&state, &user, &chapter.work_id.to_string()).await?;
    let _ = work;

    if let Decision::Deny(reason) = can_edit(&user, &contributors) {
        return Err(refusal(reason));
    }

    // Text first: a save that appends a revision also bumps the chapter's
    // version, so a rename sent with the same expected version afterwards would
    // conflict with itself.
    if let Some(document) = &request.document {
        let parsed = Document::from_json(document).map_err(|error| {
            ApiError(AppError::field(
                "document",
                format!("This document is not in the editor's schema ({error})."),
            ))
        })?;

        let note = request
            .note
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty());
        let input = RevisionInput::from_document(&parsed, note.map(str::to_owned));

        content::append_revision(
            state.db(),
            chapter.id,
            chapter.work_id,
            acting_pseud(&user)?,
            &input,
            Some(request.expected_version),
            None,
        )
        .await
        .map_err(from_content)?;
    }

    if request.title.is_some() {
        let title = validate_title(request.title.as_deref().unwrap_or(""), false)?;
        let changed = content::update_chapter(
            state.db(),
            chapter.id,
            request.expected_version,
            Some(&title),
        )
        .await?;
        if !changed && request.document.is_none() {
            let current = content::find_chapter(state.db(), chapter.id)
                .await?
                .ok_or_else(|| {
                    ApiError(AppError::NotFound {
                        resource: "chapter",
                    })
                })?;
            return Err(ApiError(AppError::RevisionConflict {
                expected: request.expected_version,
                actual: current.version,
            }));
        }
    }

    let updated = content::find_chapter(state.db(), chapter.id)
        .await?
        .ok_or_else(|| {
            ApiError(AppError::NotFound {
                resource: "chapter",
            })
        })?;

    Ok(Json(ChapterView::from(updated)))
}

async fn reorder_chapters(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Json(request): Json<ReorderRequest>,
) -> ApiResult<Json<Vec<ChapterView>>> {
    let (work, contributors) = author_work(&state, &user, &id).await?;

    if let Decision::Deny(reason) = can_edit(&user, &contributors) {
        return Err(refusal(reason));
    }

    let ordered: Vec<ChapterId> = request
        .chapters
        .iter()
        .map(|raw| parse_chapter_id(raw))
        .collect::<ApiResult<Vec<_>>>()?;

    content::reorder_chapters(state.db(), work.id, &ordered)
        .await
        .map_err(from_content)?;

    let chapters = content::chapters_for_work(state.db(), work.id).await?;
    Ok(Json(chapters.into_iter().map(ChapterView::from).collect()))
}

async fn read_chapter(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path((id, chapter)): Path<(String, String)>,
    Query(query): Query<ReadQuery>,
) -> ApiResult<Json<ChapterContentView>> {
    let work_id = parse_work_id(&id)?;
    let chapter_id = parse_chapter_id(&chapter)?;

    let work = content::find_work(state.db(), work_id)
        .await?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "work" }))?;
    let contributors = collaboration::contributors_for_work(state.db(), work_id).await?;
    let actor = actor_for(session.as_ref());

    let (is_contributor, is_public) =
        match reading_decision(&state, actor.as_ref(), &work, &contributors).await {
            Reading::Contributor => (true, false),
            Reading::Public => (false, true),
            Reading::Denied(error) => return Err(ApiError(error)),
        };
    let _ = is_public;

    let chapters = content::chapters_for_work(state.db(), work_id).await?;
    let chapter = chapters
        .iter()
        .find(|candidate| candidate.id == chapter_id)
        .cloned()
        .ok_or_else(|| {
            ApiError(AppError::NotFound {
                resource: "chapter",
            })
        })?;

    let revision = match chapter.current_revision_id {
        Some(id) => content::find_revision(state.db(), id).await?,
        None => None,
    };

    let position = chapters
        .iter()
        .position(|candidate| candidate.id == chapter_id);
    let previous_chapter_id = position
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| chapters.get(index))
        .map(|chapter| chapter.id);
    let next_chapter_id = position
        .and_then(|index| chapters.get(index + 1))
        .map(|chapter| chapter.id);

    let authors = collaboration::public_contributors(state.db(), work_id)
        .await?
        .into_iter()
        .map(|(handle, display_name, role)| PublicAuthor {
            handle,
            display_name,
            role,
        })
        .collect();

    // The editor document is a contributor-only field: a public reader gets the
    // sanitized rendering and nothing that could be re-saved over the original.
    let want_document = query.document.as_deref() != Some("0");
    let document = if is_contributor && want_document {
        revision
            .as_ref()
            .and_then(|revision| serde_json::from_str(&revision.document_json).ok())
    } else {
        None
    };

    Ok(Json(ChapterContentView {
        chapter: ChapterView::from(chapter),
        document,
        sanitized_html: revision
            .as_ref()
            .map(|revision| revision.sanitized_html.clone())
            .unwrap_or_default(),
        plain_text: revision
            .as_ref()
            .map(|revision| revision.plain_text.clone())
            .unwrap_or_default(),
        word_count: revision.as_ref().map_or(0, |revision| revision.word_count),
        revision_number: revision.as_ref().map(|revision| revision.revision_number),
        revision_id: revision.as_ref().map(|revision| revision.id),
        editable: is_contributor,
        previous_chapter_id,
        next_chapter_id,
        work: ChapterWorkView {
            id: work.id,
            title: work.title.clone(),
            lifecycle: work.lifecycle.clone(),
            authors,
        },
    }))
}

async fn list_revisions(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<RevisionView>>> {
    let chapter_id = parse_chapter_id(&id)?;
    let chapter = content::find_chapter(state.db(), chapter_id)
        .await?
        .ok_or_else(|| {
            ApiError(AppError::NotFound {
                resource: "chapter",
            })
        })?;
    let (_work, contributors) = author_work(&state, &user, &chapter.work_id.to_string()).await?;

    if let Decision::Deny(reason) = can_edit(&user, &contributors) {
        return Err(refusal(reason));
    }

    let current = chapter.current_revision_id.map(|id| id.to_string());
    let revisions: Vec<RevisionSummary> =
        content::revisions_for_chapter(state.db(), chapter_id).await?;

    Ok(Json(
        revisions
            .into_iter()
            .map(|revision| RevisionView {
                current: current.as_deref() == Some(revision.id.as_str()),
                id: revision.id,
                revision_number: revision.revision_number,
                word_count: revision.word_count,
                note: revision.note,
                created_at: revision.created_at,
                restored_from_id: revision.restored_from_id,
                author_handle: revision.author_handle,
            })
            .collect(),
    ))
}

async fn restore_revision(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Json(request): Json<RestoreRequest>,
) -> ApiResult<Json<ChapterView>> {
    let chapter_id = parse_chapter_id(&id)?;
    let chapter = content::find_chapter(state.db(), chapter_id)
        .await?
        .ok_or_else(|| {
            ApiError(AppError::NotFound {
                resource: "chapter",
            })
        })?;
    let (_work, contributors) = author_work(&state, &user, &chapter.work_id.to_string()).await?;

    if let Decision::Deny(reason) = can_edit(&user, &contributors) {
        return Err(refusal(reason));
    }

    let revision_id: RevisionId = request.revision_id.parse().map_err(|_| {
        ApiError(AppError::NotFound {
            resource: "revision",
        })
    })?;

    content::restore_revision(state.db(), chapter, revision_id, acting_pseud(&user)?)
        .await
        .map_err(from_content)?;

    let updated = content::find_chapter(state.db(), chapter_id)
        .await?
        .ok_or_else(|| {
            ApiError(AppError::NotFound {
                resource: "chapter",
            })
        })?;

    Ok(Json(ChapterView::from(updated)))
}

// ---------------------------------------------------------------------------
// Authorization helpers
// ---------------------------------------------------------------------------

/// The pseud this session is acting as.
fn acting_pseud(user: &SessionUser) -> ApiResult<PseudId> {
    user.pseud_id.ok_or(ApiError(AppError::AccessDenied))
}

/// Whether this account may write at all (spec §7, age policy).
fn require_participation(user: &SessionUser) -> ApiResult<()> {
    if user.age_state.may_participate() {
        Ok(())
    } else {
        Err(ApiError(AppError::AccessDenied))
    }
}

/// Build the domain actor for a request, if there is one.
///
/// Returns `None` for an anonymous visitor *and* for a signed-in account that
/// has not chosen a pseud: a policy decision about "who is this" is meaningless
/// without a public face, and the eligibility service treats `None` as
/// anonymous rather than guessing.
pub(crate) fn actor_for(user: Option<&SessionUser>) -> Option<Actor> {
    let user = user?;
    let pseud_id = user.pseud_id?;
    Some(user.actor(pseud_id))
}

/// Load a work *through* the acting pseud's contributions.
///
/// A work the acting pseud does not contribute to is reported as absent, and
/// so is a work that does not exist: the two answers are deliberately
/// indistinguishable to the caller.
async fn author_work(
    state: &AppState,
    user: &SessionUser,
    raw_id: &str,
) -> ApiResult<(Work, Vec<Contributor>)> {
    let work_id = parse_work_id(raw_id)?;
    let work = content::find_work(state.db(), work_id)
        .await?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "work" }))?;

    let contributors = collaboration::contributors_for_work(state.db(), work_id).await?;
    let pseud = acting_pseud(user)?;

    if !contributors.iter().any(|c| c.pseud_id == pseud) {
        return Err(ApiError(AppError::NotFound { resource: "work" }));
    }

    Ok((work, contributors))
}

fn can_edit(user: &SessionUser, contributors: &[Contributor]) -> Decision {
    match user.pseud_id {
        Some(pseud) => lorehaven_domain::content::can_edit_work(
            &lorehaven_domain::policy::Actor {
                account_id: user.account_id,
                pseud_id: pseud,
                age_state: user.age_state,
                trusted_reviewer: false,
            },
            contributors,
        ),
        None => Decision::Deny(DenyReason::NotAuthenticated),
    }
}

/// How a request for a work is answered.
pub(crate) enum Reading {
    /// The actor contributes to it: the author view, drafts included.
    Contributor,
    /// The actor may read the published work.
    Public,
    /// Refused, with the error the caller receives.
    Denied(AppError),
}

/// Check whether the actor holds a valid entitlement for a priced work.
/// Returns `Ok(true)` if entitled, `Ok(false)` if not entitled (but no
/// DB error occurred), and `Err` if the DB lookup itself fails.
async fn check_entitlement(state: &AppState, actor: &Actor, work_id: &WorkId) -> ApiResult<bool> {
    let pricing =
        lorehaven_db::monetization::get_pricing(state.db(), &work_id.to_canonical_string())
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    let Some(pricing) = pricing else {
        // Not a priced work — no entitlement needed.
        return Ok(true);
    };
    if pricing.model == "tips" || !pricing.enabled {
        // Tips model is free-to-read; disabled pricing is effectively no pricing.
        return Ok(true);
    }
    let account_id = actor.account_id.to_string();
    let has = lorehaven_db::monetization::has_entitlement(
        state.db(),
        &account_id,
        &work_id.to_canonical_string(),
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(has)
}

pub(crate) async fn reading_decision(
    state: &AppState,
    actor: Option<&Actor>,
    work: &Work,
    contributors: &[Contributor],
) -> Reading {
    let actor_is_contributor =
        actor.is_some_and(|actor| contributors.iter().any(|c| c.pseud_id == actor.pseud_id));

    let facts = ContentFacts {
        lifecycle: work.lifecycle_state(),
        visibility: parse_visibility(&work.visibility),
        rating: lorehaven_db::sessions::parse_rating(&work.rating),
        actor_is_contributor,
        author_blocked_actor: false,
        via_deep_link: true,
    };

    let policy = AccessPolicy::default();
    let _ = state;

    if actor_is_contributor {
        return Reading::Contributor;
    }

    match can_access_content(actor, &facts, &policy) {
        Decision::Allow => {
            if !matches!(work.lifecycle_state(), Lifecycle::Published) {
                return Reading::Denied(AppError::NotFound { resource: "work" });
            }
            // Entitlement check: a priced work (model == "purchase")
            // requires a valid entitlement (purchase, patronage,
            // early_access, or gift). Tips-model works are free to read.
            match actor {
                Some(actor) => match check_entitlement(state, actor, &work.id).await {
                    Ok(true) => {} // entitled or free-to-read
                    Ok(false) => return Reading::Denied(AppError::ContentRestricted),
                    Err(error) => return Reading::Denied(error.0),
                },
                None => {
                    // Anonymous: deny if the work is priced (purchase model).
                    match lorehaven_db::monetization::is_work_priced(
                        state.db(),
                        &work.id.to_canonical_string(),
                    )
                    .await
                    {
                        Ok(true) => return Reading::Denied(AppError::ContentRestricted),
                        Ok(false) => {}
                        Err(e) => return Reading::Denied(AppError::Internal(e.into())),
                    }
                }
            }
            Reading::Public
        }
        Decision::Deny(reason) => Reading::Denied(match reason {
            // Absence is reported as absence: a draft, a withheld work or a
            // work belonging to someone the actor is blocked by all look the
            // same from outside (spec §3.3).
            DenyReason::NotPublished | DenyReason::BlockedByAuthor => {
                AppError::NotFound { resource: "work" }
            }
            DenyReason::SignInRequired => AppError::AuthRequired,
            DenyReason::NotAuthenticated
            | DenyReason::AnonymousReadingDisabled
            | DenyReason::RatingExceedsPolicy
            | DenyReason::NotAContributor
            | DenyReason::InsufficientRole => AppError::ContentRestricted,
            // `DenyReason` is non-exhaustive: a reason added later must not
            // silently become "allow", so the default is refusal.
            _ => AppError::ContentRestricted,
        }),
    }
}

/// How this caller may see a work, looked up by id.
///
/// The one place a route outside this module asks the visibility question, so
/// a door that hangs something off a work (an edition, a derivative) applies
/// the same answer the work door does rather than re-deriving it.
pub(crate) async fn reading_for_work(
    state: &AppState,
    work_id: &str,
    session: Option<&SessionUser>,
) -> ApiResult<Reading> {
    let id = parse_work_id(work_id)?;
    let work = content::find_work(state.db(), id)
        .await?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "work" }))?;
    let contributors = collaboration::contributors_for_work(state.db(), id).await?;
    Ok(reading_decision(state, actor_for(session).as_ref(), &work, &contributors).await)
}

/// Refuse a caller who may read a work but may not act on it.
///
/// A work the caller cannot read at all is reported as absent (§3.3) — the
/// `Denied` arm carries that error — while a reader who can see it is told
/// plainly that this is a contributor action.
pub(crate) async fn require_contributor(
    state: &AppState,
    work_id: &str,
    session: &SessionUser,
) -> ApiResult<()> {
    match reading_for_work(state, work_id, Some(session)).await? {
        Reading::Contributor => Ok(()),
        Reading::Public => Err(ApiError(AppError::AccessDenied)),
        Reading::Denied(error) => Err(ApiError(error)),
    }
}

fn refusal(reason: DenyReason) -> ApiError {
    tracing::debug!(reason = reason.as_str(), "content permission denied");
    match reason {
        // Not a contributor at all: the resource is not disclosed.
        DenyReason::NotAContributor | DenyReason::NotAuthenticated => {
            ApiError(AppError::NotFound { resource: "work" })
        }
        _ => ApiError(AppError::AccessDenied),
    }
}

// ---------------------------------------------------------------------------
// View builders
// ---------------------------------------------------------------------------

async fn author_view(
    state: &AppState,
    user: &SessionUser,
    work: &Work,
) -> ApiResult<AuthorWorkView> {
    let chapters = content::chapters_for_work(state.db(), work.id).await?;
    let contributors = collaboration::contributors_for_work(state.db(), work.id).await?;
    let (_, with_content) = content::chapter_facts(state.db(), work.id).await?;

    let role = user
        .pseud_id
        .and_then(|pseud| {
            contributors
                .iter()
                .find(|c| c.pseud_id == pseud)
                .map(|c| c.role)
        })
        .map(|role| role.as_str().to_owned())
        .unwrap_or_else(|| "none".to_owned());

    // The blockers are computed here rather than in the interface, so a client
    // cannot believe a work is publishable when the server would refuse.
    let facts = content::publication_facts(state.db(), work).await?;
    let publication_blockers = match lorehaven_domain::content::publication_readiness(&facts) {
        Ok(()) => Vec::new(),
        Err(error) => error
            .field_errors()
            .into_iter()
            .map(|(field, message)| format!("{field}: {message}"))
            .collect(),
    };
    let _ = with_content;

    let mut contributor_views = Vec::with_capacity(contributors.len());
    for contributor in &contributors {
        let pseud = identity::find_pseud(state.db(), contributor.pseud_id)
            .await?
            .ok_or_else(|| ApiError(AppError::NotFound { resource: "pseud" }))?;
        contributor_views.push(ContributorView {
            pseud_id: pseud.id,
            handle: pseud.handle,
            display_name: pseud.display_name,
            role: contributor.role.as_str().to_owned(),
            public_attribution: contributor.public_attribution,
        });
    }

    Ok(AuthorWorkView {
        id: work.id,
        title: work.title.clone(),
        summary: work.summary.clone(),
        language: work.language.clone(),
        rating: work.rating.clone(),
        visibility: work.visibility.clone(),
        lifecycle: work.lifecycle.clone(),
        completion: work.completion.clone(),
        version: work.version,
        created_at: work.created_at.clone(),
        updated_at: work.updated_at.clone(),
        published_at: work.published_at.clone(),
        withdrawn_at: work.withdrawn_at.clone(),
        show_public_ratings: work.show_public_ratings,
        discussion_mode: work.discussion_mode.clone(),
        role,
        chapters: chapters.into_iter().map(ChapterView::from).collect(),
        contributors: contributor_views,
        publication_blockers,
    })
}

async fn public_view(state: &AppState, work: &Work) -> ApiResult<PublicWorkView> {
    let chapters = content::chapters_for_work(state.db(), work.id).await?;
    let authors = collaboration::public_contributors(state.db(), work.id)
        .await?
        .into_iter()
        .map(|(handle, display_name, role)| PublicAuthor {
            handle,
            display_name,
            role,
        })
        .collect();

    // Spec §9.5: the aggregate is shown only where the work's owner allows it,
    // only above the minimum count, and always with its count and its method.
    // A work whose owner has turned it off reports no aggregate at all rather
    // than a zero, which would read as "nobody liked this".
    let rating_summary = if work.show_public_ratings {
        reading::public_rating_summary(state.db(), work.id).await?
    } else {
        None
    };

    // The same owner preference gates the engagement counters: a work whose
    // owner has hidden public rating display hides the metric bar too.
    let metrics = if work.show_public_ratings {
        let m = work_metrics::get_metrics(state.db(), &work.id).await?;
        Some(PublicWorkMetricsView {
            views: m.views,
            complete_reads: m.complete_reads,
            reactions: m.reactions,
            kudos: m.kudos,
            bookmarks: m.bookmarks,
            collection_adds: m.collection_adds,
            reviews: m.reviews,
        })
    } else {
        None
    };

    Ok(PublicWorkView {
        id: work.id,
        title: work.title.clone(),
        summary: work.summary.clone(),
        language: work.language.clone(),
        rating: work.rating.clone(),
        visibility: work.visibility.clone(),
        completion: work.completion.clone(),
        published_at: work.published_at.clone(),
        show_public_ratings: work.show_public_ratings,
        discussion_mode: work.discussion_mode.clone(),
        rating_summary: rating_summary.map(|summary| PublicRatingSummaryView {
            count: summary.count,
            mean_stars: summary.mean_permille as f64 / 1000.0,
            method: MEAN_OF_PUBLIC_RATINGS,
        }),
        metrics,
        authors,
        chapters: chapters.into_iter().map(ChapterView::from).collect(),
    })
}

async fn reload(state: &AppState, id: WorkId) -> ApiResult<Work> {
    content::find_work(state.db(), id)
        .await?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "work" }))
}

/// Record a deduplicated view for a work. The viewer hash is the account id
/// when signed in, or `"anon"` when not; `viewed_at` is truncated to the
/// hour so repeated refreshes do not inflate the counter (spec §10).
async fn record_view_for_work(
    state: &AppState,
    work_id: &WorkId,
    session: Option<&SessionUser>,
) -> ApiResult<()> {
    let viewer_hash = match session {
        Some(s) => s.account_id.to_string(),
        None => "anon".to_string(),
    };
    let viewed_at = chrono::Utc::now().format("%Y-%m-%dT%H:00:00").to_string();
    let is_new = work_metrics::record_view(
        state.db(),
        &work_id.to_string(),
        &viewer_hash,
        &viewed_at,
        false,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    if is_new {
        work_metrics::increment_views(state.db(), &work_id.to_string())
            .await
            .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    }
    Ok(())
}

/// Toggle kudos for the signed-in account on a work. Returns the new state.
async fn toggle_kudos(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let work_id = parse_work_id(&id)?;
    let kudoed = work_metrics::toggle_kudos(
        state.db(),
        &work_id.to_string(),
        &user.account_id.to_string(),
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(serde_json::json!({ "kudoed": kudoed })))
}

// ---------------------------------------------------------------------------
// Validation and conversion
// ---------------------------------------------------------------------------

fn parse_work_id(raw: &str) -> ApiResult<WorkId> {
    raw.parse()
        .map_err(|_| ApiError(AppError::NotFound { resource: "work" }))
}

fn parse_chapter_id(raw: &str) -> ApiResult<ChapterId> {
    raw.parse().map_err(|_| {
        ApiError(AppError::NotFound {
            resource: "chapter",
        })
    })
}

fn parse_visibility(raw: &str) -> Visibility {
    match raw {
        "unlisted" => Visibility::Unlisted,
        "restricted" => Visibility::Restricted,
        _ => Visibility::Public,
    }
}

/// Trim and validate a title or chapter heading. An empty one is allowed for a
/// draft; publication is what refuses it.
fn validate_title(raw: &str, required: bool) -> ApiResult<String> {
    let trimmed = raw.trim();
    if required && trimmed.is_empty() {
        return Err(ApiError(AppError::field("title", "A title is required.")));
    }
    if trimmed.chars().count() > 300 {
        return Err(ApiError(AppError::field(
            "title",
            "A title may be at most 300 characters.",
        )));
    }
    if trimmed
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(ApiError(AppError::field(
            "title",
            "A title cannot contain control characters.",
        )));
    }
    Ok(trimmed.to_owned())
}

fn validate_summary(raw: &str) -> ApiResult<String> {
    let trimmed = raw.trim();
    if trimmed.chars().count() > 5000 {
        return Err(ApiError(AppError::field(
            "summary",
            "A summary may be at most 5000 characters.",
        )));
    }
    if trimmed
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(ApiError(AppError::field(
            "summary",
            "A summary cannot contain control characters.",
        )));
    }
    Ok(trimmed.to_owned())
}

/// Check a BCP 47-looking language tag.
///
/// Deliberately shaped rather than merely "alphanumeric": a free-text language
/// field is a field nobody can filter on later, and search by language is a
/// Milestone 9 requirement.
fn validate_language(raw: &str) -> ApiResult<String> {
    let trimmed = raw.trim();
    let mut parts = trimmed.split('-');
    let primary = parts.next().unwrap_or_default();
    let primary_ok =
        (2..=3).contains(&primary.len()) && primary.chars().all(|c| c.is_ascii_alphabetic());

    let rest_ok = parts.all(|part| {
        (2..=8).contains(&part.len()) && part.chars().all(|c| c.is_ascii_alphanumeric())
    });

    if !primary_ok || !rest_ok || trimmed.len() > 35 {
        return Err(ApiError(AppError::field(
            "language",
            "A language tag looks like `en`, `pt-BR` or `zh-Hans`.",
        )));
    }
    Ok(trimmed.to_owned())
}

fn validate_rating(raw: &str) -> ApiResult<String> {
    match raw {
        "general" | "teen" | "mature" | "explicit" => Ok(raw.to_owned()),
        _ => Err(ApiError(AppError::field(
            "rating",
            "A rating is one of general, teen, mature or explicit.",
        ))),
    }
}

fn validate_visibility(raw: &str) -> ApiResult<String> {
    match raw {
        "public" | "unlisted" | "restricted" => Ok(raw.to_owned()),
        _ => Err(ApiError(AppError::field(
            "visibility",
            "A visibility is one of public, unlisted or restricted.",
        ))),
    }
}

fn validate_completion(raw: &str) -> ApiResult<String> {
    match raw {
        "in_progress" | "complete" | "hiatus" | "abandoned" => Ok(raw.to_owned()),
        _ => Err(ApiError(AppError::field(
            "completion",
            "A completion state is one of in_progress, complete, hiatus or abandoned.",
        ))),
    }
}

/// Validate every field the request wants to change.
///
/// Owned strings rather than borrows: each validated form is a trimmed copy of
/// what arrived, so the patch cannot outlive its source by accident, and the
/// database layer is not tied to a transport type.
fn work_patch(request: &UpdateWorkRequest) -> ApiResult<WorkPatch> {
    let title = request
        .title
        .as_deref()
        .map(|raw| validate_title(raw, false))
        .transpose()?;
    let summary = request
        .summary
        .as_deref()
        .map(validate_summary)
        .transpose()?;
    let language = request
        .language
        .as_deref()
        .map(validate_language)
        .transpose()?;
    let rating = request.rating.as_deref().map(validate_rating).transpose()?;
    let visibility = request
        .visibility
        .as_deref()
        .map(validate_visibility)
        .transpose()?;
    let completion = request
        .completion
        .as_deref()
        .map(validate_completion)
        .transpose()?;

    Ok(WorkPatch {
        title,
        summary,
        language,
        rating,
        visibility,
        completion,
        show_public_ratings: request.show_public_ratings,
    })
}

fn internal(error: serde_json::Error) -> ApiError {
    ApiError(AppError::Internal(error.into()))
}

/// Turn a content refusal into the API error it deserves.
fn from_content(error: ContentError) -> ApiError {
    match error {
        ContentError::Refused(app) => ApiError(app),
        ContentError::Fault(cause) => ApiError(AppError::Internal(cause)),
    }
}
// ---------------------------------------------------------------------------
// Permission statements (M27)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PutWorkPermissions {
    #[serde(default)]
    pub podfic: Option<String>,
    #[serde(default)]
    pub translation: Option<String>,
    #[serde(default)]
    pub remix: Option<String>,
    #[serde(default)]
    pub continuation: Option<String>,
    #[serde(default)]
    pub redistribution: Option<String>,
    #[serde(default)]
    pub ai_training: Option<String>,
}

pub async fn get_work_permissions(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
) -> ApiResult<Json<PermissionStatement>> {
    // Reading a work's permission statement follows the work's own visibility
    if let Reading::Denied(error) = reading_for_work(&state, &id, session.as_ref()).await? {
        return Err(ApiError(error));
    }
    let db = state.db();
    let stmt = permission::get_work_permission_statement(db, &id).await?;
    Ok(Json(stmt))
}

pub async fn put_work_permissions(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Json(body): Json<PutWorkPermissions>,
) -> ApiResult<Json<PermissionStatement>> {
    // Only a contributor may change a work's permission statement
    require_contributor(&state, &id, &user).await?;
    let db = state.db();
    let mut stmt = permission::get_work_permission_statement(db, &id).await?;
    if let Some(v) = &body.podfic {
        stmt.podfic = lorehaven_domain::permission::Permission::parse(v).ok_or_else(|| {
            ApiError(AppError::Validation {
                message: format!("invalid podfic permission: {v}"),
                field_errors: Default::default(),
            })
        })?;
    }
    if let Some(v) = &body.translation {
        stmt.translation = lorehaven_domain::permission::Permission::parse(v).ok_or_else(|| {
            ApiError(AppError::Validation {
                message: format!("invalid translation permission: {v}"),
                field_errors: Default::default(),
            })
        })?;
    }
    if let Some(v) = &body.remix {
        stmt.remix = lorehaven_domain::permission::Permission::parse(v).ok_or_else(|| {
            ApiError(AppError::Validation {
                message: format!("invalid remix permission: {v}"),
                field_errors: Default::default(),
            })
        })?;
    }
    if let Some(v) = &body.continuation {
        stmt.continuation =
            lorehaven_domain::permission::Permission::parse(v).ok_or_else(|| {
                ApiError(AppError::Validation {
                    message: format!("invalid continuation permission: {v}"),
                    field_errors: Default::default(),
                })
            })?;
    }
    if let Some(v) = &body.redistribution {
        stmt.redistribution =
            lorehaven_domain::permission::Permission::parse(v).ok_or_else(|| {
                ApiError(AppError::Validation {
                    message: format!("invalid redistribution permission: {v}"),
                    field_errors: Default::default(),
                })
            })?;
    }
    if let Some(v) = &body.ai_training {
        stmt.ai_training = lorehaven_domain::permission::Permission::parse(v).ok_or_else(|| {
            ApiError(AppError::Validation {
                message: format!("invalid ai_training permission: {v}"),
                field_errors: Default::default(),
            })
        })?;
    }
    permission::set_work_permission_statement(db, &id, &stmt).await?;
    Ok(Json(stmt))
}

#[derive(Debug, Deserialize)]
pub struct AddLineageRequest {
    pub from_work_id: String,
    pub kind: String,
    pub provenance: String,
}

pub async fn add_lineage(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Json(body): Json<AddLineageRequest>,
) -> ApiResult<Json<Value>> {
    // Only contributors to the target work may add lineage edges
    require_contributor(&state, &id, &user).await?;
    let db = state.db();
    let kind = LineageKind::parse(&body.kind).ok_or_else(|| {
        ApiError(AppError::Validation {
            message: format!("invalid lineage kind: {}", body.kind),
            field_errors: Default::default(),
        })
    })?;
    let edge = LineageEdge {
        id: uuid::Uuid::new_v4().to_string(),
        from_work_id: body.from_work_id,
        to_work_id: id,
        kind,
        provenance: body.provenance,
        created_at: lorehaven_db::identity::now_rfc3339(),
    };
    permission::insert_lineage_edge(db, &edge).await?;
    Ok(Json(json!({ "id": edge.id, "kind": edge.kind.as_str() })))
}

pub async fn list_lineage(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    if let Reading::Denied(error) = reading_for_work(&state, &id, session.as_ref()).await? {
        return Err(ApiError(error));
    }
    let db = state.db();
    let edges = permission::lineage_edges_for_work(db, &id).await?;
    let items: Vec<Value> = edges
        .iter()
        .map(|e| {
            json!({
                "id": e.id,
                "from_work_id": e.from_work_id,
                "to_work_id": e.to_work_id,
                "kind": e.kind.as_str(),
                "provenance": e.provenance,
                "created_at": e.created_at,
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_are_trimmed_and_length_checked() {
        assert_eq!(validate_title("  A Story  ", false).expect("ok"), "A Story");
        assert_eq!(validate_title("", false).expect("ok"), "");
        assert!(validate_title("", true).is_err());
        assert!(validate_title(&"x".repeat(301), false).is_err());
        assert!(validate_title("bell\u{7}", false).is_err());
    }

    #[test]
    fn enumerated_fields_reject_anything_they_do_not_know() {
        assert!(validate_rating("teen").is_ok());
        assert!(validate_rating("Teen").is_err());
        assert!(validate_rating("explicit-ish").is_err());
        assert!(validate_visibility("unlisted").is_ok());
        assert!(validate_visibility("secret").is_err());
        assert!(validate_completion("hiatus").is_ok());
        assert!(validate_completion("stalled").is_err());
        assert!(validate_language("pt-BR").is_ok());
        assert!(validate_language("english").is_err());
    }

    #[test]
    fn an_unrecognised_stored_visibility_is_not_treated_as_listable() {
        // `parse_visibility` defaults to Public for the *stored* value, which is
        // safe because the column default is public; the risk it guards against
        // is a *narrower* value being read as a wider one, and it never is.
        assert_eq!(parse_visibility("public"), Visibility::Public);
        assert_eq!(parse_visibility("unlisted"), Visibility::Unlisted);
        assert_eq!(parse_visibility("restricted"), Visibility::Restricted);
    }
}

/// Fork a work (spec §40.1). Creates a new empty draft owned by the caller,
/// linked to the parent through a `remix` lineage edge, inheriting the
/// parent's tags. No body text is copied — a fork starts empty.
///
/// Guards, in order:
/// 1. Permission statement: parent's `remix` must be `yes` or `unstated`.
/// 2. Exclusion registry: parent must not be excluded.
/// 3. Depth limit: lineage chain must be below `max_fork_depth`.
/// 4. Visibility: fork inherits parent's visibility (never widens it).
async fn fork_work(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(parent_id): Path<String>,
) -> ApiResult<(StatusCode, Json<AuthorWorkView>)> {
    let pseud = acting_pseud(&user)?;
    require_participation(&user)?;

    let parent_id = parse_work_id(&parent_id)?;
    let parent_str = parent_id.to_string();

    // The parent must exist and be readable.
    if let Reading::Denied(error) = reading_for_work(&state, &parent_str, Some(&user)).await? {
        return Err(ApiError(error));
    }

    // Guard 1: permission statement — `remix` must allow it.
    let statement = permission::get_work_permission_statement(state.db(), &parent_str).await?;
    match statement.remix {
        Permission::No => {
            return Err(ApiError(AppError::Validation {
                message: "the author has declined remixes of this work".to_owned(),
                field_errors: BTreeMap::from([(
                    "remix".to_owned(),
                    "the author's permission statement declines remixes".to_owned(),
                )]),
            }));
        }
        Permission::Ask => {
            return Err(ApiError(AppError::Validation {
                message: "the author requires you to ask before remixing this work".to_owned(),
                field_errors: BTreeMap::from([(
                    "remix".to_owned(),
                    "permission is set to ask — request permission first".to_owned(),
                )]),
            }));
        }
        Permission::Yes | Permission::Unstated => {}
    }

    // Guard 2: exclusion registry.
    if permission::is_excluded(state.db(), ExclusionTarget::Work, &parent_str).await? {
        return Err(ApiError(AppError::Validation {
            message: "this work is excluded from forking".to_owned(),
            field_errors: BTreeMap::new(),
        }));
    }

    // Guard 3: depth limit.
    let depth = permission::lineage_depth(state.db(), &parent_str).await?;
    let max_depth = state.config().works.max_fork_depth;
    if depth >= max_depth {
        return Err(ApiError(AppError::Validation {
            message: format!("this work's fork chain has reached the maximum depth of {max_depth}"),
            field_errors: BTreeMap::from([(
                "depth".to_owned(),
                format!("lineage depth {depth} >= maximum {max_depth}"),
            )]),
        }));
    }

    // Guard 4: visibility — read parent to inherit visibility.
    let parent_work = content::find_work(state.db(), parent_id.clone())
        .await?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "work" }))?;

    // Create the new draft work.
    let fork_title = format!("Fork of {}", parent_work.title.trim());
    let fork_visibility = if parent_work.visibility == "public" {
        None
    } else {
        Some(parent_work.visibility.as_str())
    };
    let new_work = content::create_work(state.db(), pseud, &fork_title, fork_visibility).await?;

    // Create lineage edge: new_work (child) remixes parent_work.
    let now = identity::now_rfc3339();
    let edge = LineageEdge {
        id: uuid::Uuid::new_v4().to_string(),
        from_work_id: parent_str.clone(),
        to_work_id: new_work.id.to_string(),
        kind: LineageKind::Remix,
        provenance: format!("forked by {} at {}", pseud, now),
        created_at: now,
    };
    permission::insert_lineage_edge(state.db(), &edge).await?;

    // Inherit tags from parent (spec §40.1).
    let parent_tags = taxonomy::tags_for_work(state.db(), &parent_str).await?;
    for node_id in parent_tags {
        let _ = taxonomy::tag_work(state.db(), &new_work.id.to_string(), &node_id, 1).await;
    }

    let view = author_view(&state, &user, &new_work).await?;
    Ok((StatusCode::CREATED, Json(view)))
}
