//! The reader's library: shelves, bookmarks, private tags, reading statuses,
//! saved views, storage and update checks (spec §16 as the plan numbers it;
//! §14 in the spec text).
//!
//! ```text
//! GET/POST               /shelves
//! GET/PATCH/DELETE       /shelves/:id
//! POST/DELETE            /shelves/:id/items/:libraryItemId
//! GET/POST               /bookmarks
//! GET/PATCH/DELETE       /bookmarks/:id
//! GET                    /library/items?<filters>            envelope
//! GET/PUT/DELETE         /library/items/:id/tags[/:tag]
//! PUT/DELETE             /library/items/:id/status
//! GET/POST               /saved-views
//! GET/PATCH/DELETE       /saved-views/:id
//! POST                   /library/updates/check              → 202 + job
//! GET                    /library/storage
//! ```
//!
//! # What this module is careful about
//!
//! * **The account on the session is the only account these handlers mention.**
//!   No route takes an account identifier, and every read and write goes through
//!   a `lorehaven_db::library` function that is itself scoped by account. A path
//!   identifier is therefore an *additional* condition, never the condition.
//! * **A private tag is served through its own table and never joined out.** The
//!   only query here that is not account-scoped is the public bookmark list,
//!   which is `is_public`-filtered in SQL, and it returns no shelf, tag or
//!   reading status — those are not fields on a bookmark.
//! * **A batch answers per item.** Spec §14.3 fixes the shape
//!   `{"succeeded": [...], "failed": [{"id": …, "code": …}]}` and the plan's
//!   third pitfall is that one boolean is not an answer. `BatchOutcome` is that
//!   shape, and [`BatchOutcome::summary`] is the sentence the interface shows.
//! * **The update check is a job.** It reads the network once per item, so it
//!   answers `202` with a job id rather than holding the request open (spec
//!   §14.1).

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64;
use base64::Engine as _;
use serde::Deserialize;

use lorehaven_db::library::status_json;
use lorehaven_db::storage::BlobStore;
use lorehaven_db::{imports, jobs, library};
use lorehaven_domain::library::{
    normalise_tag, BatchOutcome, LibraryQuery, LibrarySort, ReadingStatus, ViewScope,
    SUBJECT_LIBRARY_ITEM,
};
use lorehaven_domain::AppError;

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

/// The per-account routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/shelves", get(list_shelves).post(create_shelf))
        .route(
            "/shelves/{id}",
            get(get_shelf).patch(patch_shelf).delete(delete_shelf_route),
        )
        .route(
            "/shelves/{id}/items/{item_id}",
            post(add_to_shelf).delete(remove_from_shelf),
        )
        .route("/bookmarks", get(list_bookmarks).post(create_bookmark))
        .route(
            "/bookmarks/{id}",
            get(get_bookmark)
                .patch(patch_bookmark)
                .delete(delete_bookmark_route),
        )
        .route("/library/items", get(query_library_items))
        .route("/library/items/{id}/tags", get(list_item_tags))
        .route(
            "/library/items/{id}/tags/{tag}",
            put(add_item_tag).delete(remove_item_tag),
        )
        .route(
            "/library/items/{id}/status",
            get(read_status).put(set_status).delete(clear_status),
        )
        .route("/saved-views", get(list_views).post(create_view))
        .route(
            "/saved-views/{id}",
            get(get_view).patch(patch_view).delete(delete_view_route),
        )
        .route("/library/updates/check", post(start_update_check))
        .route("/library/storage", get(storage_usage))
        .route("/library/items/batch", post(batch_remove_items))
}

// ---------------------------------------------------------------------------
// Query-string parsing
// ---------------------------------------------------------------------------

/// The library listing's query string.
///
/// A DTO rather than `Query<LibraryQuery>` because a repeated key
/// (`?tags=a&tags=b`) is not something `serde_urlencoded` can express, and
/// `LibraryQuery` is also the shape stored in a saved view — so asking it to
/// parse a URL would make the stored form depend on the transport. Filters here
/// are comma-separated (`?tags=wip,read%20later`), which is unambiguous for tags
/// because [`normalise_tag`] collapses whitespace and the comma is not a
/// character a tag keeps.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LibraryItemsParams {
    /// Comma-separated shelf names.
    shelves: Option<String>,
    /// Comma-separated private tag names.
    tags: Option<String>,
    /// Comma-separated reading statuses.
    statuses: Option<String>,
    /// A source key.
    source: Option<String>,
    /// An RFC 3339 lower bound on the source's own update time.
    updated_since: Option<String>,
    /// One of `recent`, `title`, `updated`, `words`, `position`.
    sort: Option<String>,
    /// Opaque cursor from a previous page.
    cursor: Option<String>,
    /// Page size.
    limit: Option<i64>,
}

/// Split a comma-separated filter value.
///
/// Empty segments are dropped so a trailing comma is not an empty filter that
/// matches nothing — which would look like a bug in the reader's own query.
fn split_filter(value: Option<&String>) -> Vec<String> {
    value
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

impl LibraryItemsParams {
    /// Build the domain query, refusing values this build does not know.
    ///
    /// An unrecognised status is a validation failure rather than a silently
    /// dropped filter: dropping it would answer a different question than the
    /// one asked, and the reader would see items they had excluded.
    fn into_query(self) -> Result<LibraryQuery, ApiError> {
        let mut statuses = Vec::new();
        for raw in split_filter(self.statuses.as_ref()) {
            let status = ReadingStatus::parse(&raw).ok_or_else(|| {
                ApiError(AppError::Validation {
                    message: format!("`{raw}` is not a reading status this build knows"),
                    field_errors: Default::default(),
                })
            })?;
            statuses.push(status);
        }
        let tags = split_filter(self.tags.as_ref())
            .iter()
            .map(|raw| {
                normalise_tag(raw).ok_or_else(|| {
                    ApiError(AppError::Validation {
                        message: format!("`{raw}` is not a usable tag"),
                        field_errors: Default::default(),
                    })
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(LibraryQuery {
            shelves: split_filter(self.shelves.as_ref()),
            tags,
            statuses,
            source: self.source.filter(|s| !s.trim().is_empty()),
            updated_since: self.updated_since.filter(|s| !s.trim().is_empty()),
            sort: self
                .sort
                .as_deref()
                .map_or_else(LibrarySort::default, LibrarySort::parse),
        })
    }
}

/// The default page size.
const PAGE: i64 = 50;

/// The longest a shelf name may be, in characters.
const MAX_SHELF_NAME_CHARS: usize = 80;

/// Encode a page offset as the opaque cursor spec §3.3 asks for.
///
/// An offset, not a key. The listing can be sorted five ways and filtered on
/// five facets, and a keyset cursor would have to be re-derived for each
/// combination — so the cursor is a page number the client holds and does not
/// read, which is all the spec asks of it ("opaque"). The cost is that an
/// insert between two pages can repeat or skip one row, which is the same cost
/// every offset listing has.
fn encode_cursor(offset: i64) -> String {
    BASE64.encode(format!("o:{offset}"))
}

/// Read a cursor back, refusing anything this build did not write.
fn decode_cursor(raw: &str) -> Result<i64, ApiError> {
    let decoded = BASE64
        .decode(raw)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|text| text.strip_prefix("o:").map(str::to_owned))
        .and_then(|digits| digits.parse::<i64>().ok())
        .filter(|offset| *offset >= 0)
        .ok_or_else(|| {
            ApiError(AppError::Validation {
                message: "that cursor was not issued by this server".to_owned(),
                field_errors: Default::default(),
            })
        })?;
    Ok(decoded)
}

// ---------------------------------------------------------------------------
// Shelves
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ShelfBody {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    is_public: Option<bool>,
}

fn shelf_json(shelf: &library::Shelf, item_count: Option<i64>) -> serde_json::Value {
    serde_json::json!({
        "id": shelf.id,
        "name": shelf.name,
        "description": shelf.description,
        "is_public": shelf.is_public,
        "position": shelf.position,
        "item_count": item_count,
        "created_at": shelf.created_at,
        "updated_at": shelf.updated_at,
        "version": shelf.version,
    })
}

/// The reader's shelves.
async fn list_shelves(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let account = user.account_id.to_string();
    let shelves = library::shelves_for(state.db(), &account).await?;
    let mut items = Vec::with_capacity(shelves.len());
    for shelf in &shelves {
        let count = library::shelf_item_ids(state.db(), &account, &shelf.id)
            .await
            .map(|ids| i64::try_from(ids.len()).unwrap_or(i64::MAX))?;
        items.push(shelf_json(shelf, Some(count)));
    }
    Ok(Json(serde_json::json!({
        "items": items,
        "next_cursor": null,
    })))
}

/// Make a shelf.
async fn create_shelf(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<ShelfBody>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let name = read_shelf_name(&body.name)?;
    let shelf = library::create_shelf(
        state.db(),
        &user.account_id.to_string(),
        &name,
        body.description.as_deref().unwrap_or(""),
        body.is_public.unwrap_or(false),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(shelf_json(&shelf, Some(0)))))
}

/// One shelf, with the items on it.
async fn get_shelf(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let account = user.account_id.to_string();
    let shelf = library::find_shelf(state.db(), &account, &id)
        .await?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "shelf" }))?;
    let item_ids = library::shelf_item_ids(state.db(), &account, &shelf.id).await?;
    let count = i64::try_from(item_ids.len()).unwrap_or(i64::MAX);
    Ok(Json(serde_json::json!({
        "shelf": shelf_json(&shelf, Some(count)),
        "library_item_ids": item_ids,
    })))
}

/// A partial shelf update.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ShelfPatchBody {
    name: Option<String>,
    description: Option<String>,
    is_public: Option<bool>,
    position: Option<i64>,
    expected_version: i64,
}

/// Rename, describe, share or reorder a shelf.
///
/// The version is required and a mismatch is a `409`, not a silent overwrite:
/// two tabs renaming the same shelf is the ordinary case, and the second one
/// should be told rather than obeyed.
async fn patch_shelf(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Json(body): Json<ShelfPatchBody>,
) -> ApiResult<StatusCode> {
    let account = user.account_id.to_string();
    let existing = library::find_shelf(state.db(), &account, &id)
        .await?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "shelf" }))?;
    let name = match body.name.as_deref() {
        Some(raw) => Some(read_shelf_name(raw)?),
        None => None,
    };
    let patch = library::ShelfPatch {
        name: name.as_deref(),
        description: body.description.as_deref(),
        is_public: body.is_public,
        position: body.position,
    };
    let changed =
        library::update_shelf(state.db(), &account, &id, &patch, body.expected_version).await?;
    if !changed {
        return Err(ApiError(AppError::RevisionConflict {
            expected: body.expected_version,
            actual: existing.version,
        }));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Delete a shelf.
///
/// The items on it stay in the library — spec §14.1: "Deleting a shelf does not
/// delete its works".
async fn delete_shelf_route(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let removed = library::delete_shelf(state.db(), &user.account_id.to_string(), &id).await?;
    if !removed {
        return Err(ApiError(AppError::NotFound { resource: "shelf" }));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Put an item on a shelf.
async fn add_to_shelf(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path((id, item_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let placed =
        library::add_shelf_item(state.db(), &user.account_id.to_string(), &id, &item_id, 0).await?;
    if !placed {
        // The statement joins both ends against the account, so nothing placed
        // means the shelf or the item is not this reader's — which is answered
        // as "no such item" rather than as a permission error, because the
        // existence of somebody else's item is not ours to confirm.
        return Err(ApiError(AppError::NotFound {
            resource: "shelf or library item",
        }));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Take an item off a shelf.
async fn remove_from_shelf(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path((id, item_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let removed =
        library::remove_shelf_item(state.db(), &user.account_id.to_string(), &id, &item_id).await?;
    if !removed {
        return Err(ApiError(AppError::NotFound {
            resource: "shelf placement",
        }));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Bookmarks
// ---------------------------------------------------------------------------

/// A bookmark as it arrives.
///
/// The derived default is the interesting part: every field is absent-and-false
/// or absent-and-empty, and `is_public` is therefore `false` when the body does
/// not say — which is spec §14.1's "default bookmarks to private". The column
/// carries its own default for the same reason, so neither layer is the only
/// one holding that promise.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct BookmarkBody {
    subject_type: String,
    subject_id: String,
    chapter_id: Option<String>,
    position_permille: Option<i64>,
    note: String,
    is_public: bool,
}

fn bookmark_json(row: &library::Bookmark) -> serde_json::Value {
    serde_json::json!({
        "id": row.id,
        "subject_type": row.subject_type,
        "subject_id": row.subject_id,
        "chapter_id": row.chapter_id,
        "position_permille": row.position_permille,
        "note": row.note,
        "is_public": row.is_public,
        "created_at": row.created_at,
        "updated_at": row.updated_at,
        "version": row.version,
    })
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct BookmarksQuery {
    subject_type: Option<String>,
    subject_id: Option<String>,
}

/// The reader's bookmarks.
async fn list_bookmarks(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Query(query): Query<BookmarksQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let rows = library::bookmarks_for(
        state.db(),
        &user.account_id.to_string(),
        query.subject_type.as_deref(),
        query.subject_id.as_deref(),
    )
    .await?;
    Ok(Json(serde_json::json!({
        "items": rows.iter().map(bookmark_json).collect::<Vec<_>>(),
        "next_cursor": null,
    })))
}

/// Add a bookmark.
async fn create_bookmark(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<BookmarkBody>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    if !lorehaven_domain::library::is_known_subject(&body.subject_type) {
        return Err(ApiError(AppError::Validation {
            message: "a bookmark must be attached to a work or a library item".to_owned(),
            field_errors: Default::default(),
        }));
    }
    if let Some(permille) = body.position_permille {
        if !(0..=1000).contains(&permille) {
            return Err(ApiError(AppError::Validation {
                message: "a bookmark position is a thousandth of a chapter, so 0 to 1000"
                    .to_owned(),
                field_errors: Default::default(),
            }));
        }
    }
    let row = library::create_bookmark(
        state.db(),
        &user.account_id.to_string(),
        &library::NewBookmark {
            subject_type: &body.subject_type,
            subject_id: &body.subject_id,
            chapter_id: body.chapter_id.as_deref(),
            position_permille: body.position_permille,
            note: &body.note,
            is_public: body.is_public,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(bookmark_json(&row))))
}

/// One bookmark.
async fn get_bookmark(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let row = library::find_bookmark(state.db(), &user.account_id.to_string(), &id)
        .await?
        .ok_or_else(|| {
            ApiError(AppError::NotFound {
                resource: "bookmark",
            })
        })?;
    Ok(Json(bookmark_json(&row)))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct BookmarkPatchBody {
    note: Option<String>,
    position_permille: Option<i64>,
    is_public: Option<bool>,
    expected_version: i64,
}

/// Change a bookmark's note, position or visibility.
async fn patch_bookmark(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Json(body): Json<BookmarkPatchBody>,
) -> ApiResult<StatusCode> {
    let account = user.account_id.to_string();
    let existing = library::find_bookmark(state.db(), &account, &id)
        .await?
        .ok_or_else(|| {
            ApiError(AppError::NotFound {
                resource: "bookmark",
            })
        })?;
    let patch = library::BookmarkPatch {
        note: body.note.as_deref(),
        position_permille: body.position_permille,
        is_public: body.is_public,
    };
    let changed =
        library::update_bookmark(state.db(), &account, &id, &patch, body.expected_version).await?;
    if !changed {
        return Err(ApiError(AppError::RevisionConflict {
            expected: body.expected_version,
            actual: existing.version,
        }));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Delete a bookmark.
async fn delete_bookmark_route(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let removed = library::delete_bookmark(state.db(), &user.account_id.to_string(), &id).await?;
    if !removed {
        return Err(ApiError(AppError::NotFound {
            resource: "bookmark",
        }));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// The library listing
// ---------------------------------------------------------------------------

/// The reader's library, filtered, sorted and paged.
async fn query_library_items(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Query(params): Query<LibraryItemsParams>,
) -> ApiResult<Json<serde_json::Value>> {
    let account = user.account_id.to_string();
    let cursor = params.cursor.as_deref().map(decode_cursor).transpose()?;
    let limit = params.limit.unwrap_or(PAGE);
    let query = LibraryItemsParams {
        shelves: params.shelves.clone(),
        tags: params.tags.clone(),
        statuses: params.statuses.clone(),
        source: params.source.clone(),
        updated_since: params.updated_since.clone(),
        sort: params.sort.clone(),
        cursor: None,
        limit: None,
    }
    .into_query()?;

    let offset = cursor.unwrap_or(0);
    let page = library::query_library(state.db(), &account, &query, limit, offset).await?;

    // What the reader has done with each item, in three queries for the page
    // rather than three per card. The card shows a reading status, the reader's
    // own tags and the shelves a work sits on, and none of that is a property of
    // the work — so it is fetched here rather than joined into the listing's own
    // statement, where it would multiply rows for every tag.
    let ids: Vec<String> = page.items.iter().map(|item| item.id.clone()).collect();
    let facts = library::facts_for_items(state.db(), &account, &ids).await?;
    let returned = i64::try_from(page.items.len()).unwrap_or(i64::MAX);
    let next = offset + returned;
    let next_cursor = if next < page.total {
        Some(encode_cursor(next))
    } else {
        None
    };

    Ok(Json(serde_json::json!({
        "items": page
            .items
            .iter()
            .map(|row| {
                let facts = facts.get(&row.id);
                item_json(
                    &state,
                    row,
                    facts.and_then(|f| f.status).map(|s| s.as_str()),
                    facts.map_or(&[][..], |f| f.shelves.as_slice()),
                    facts.map_or(&[][..], |f| f.tags.as_slice()),
                )
            })
            .collect::<Vec<_>>(),
        "total": page.total,
        "next_cursor": next_cursor,
    })))
}

/// One library item as the interface consumes it.
///
/// The same projection `/imports` and the M6 library listing used, so a client
/// that already renders an item keeps working: this route replaced that one
/// rather than sitting beside it. The trailing three are the reader's own
/// relationship with the item, which the card draws in every density.
fn item_json(
    state: &AppState,
    row: &imports::LibraryItem,
    reading_status: Option<&str>,
    shelves: &[String],
    tags: &[String],
) -> serde_json::Value {
    let source_display_name = state
        .registry()
        .by_key(&lorehaven_scrapers::SourceKey::new(row.source_key.clone()))
        .map_or_else(
            |_| row.source_key.clone(),
            |adapter| adapter.display_name().to_owned(),
        );
    serde_json::json!({
        "id": row.id,
        "source_key": row.source_key,
        "source_work_key": row.source_work_key,
        "source_url": row.source_url,
        "title": row.title,
        "author_text": row.author_text,
        "author_url": row.author_url,
        "summary": row.summary,
        "language": row.language,
        "word_count": row.word_count,
        "chapter_count": row.chapter_count,
        "source_display_name": source_display_name,
        "status": row.status,
        "source_updated_at": row.source_updated_at,
        "last_synced_at": row.last_synced_at,
        "created_at": row.created_at,
        "updated_at": row.updated_at,
        "reading_status": reading_status,
        "shelves": shelves,
        "tags": tags,
    })
}

// ---------------------------------------------------------------------------
// Private tags
// ---------------------------------------------------------------------------

/// The reader's own tags on one item.
async fn list_item_tags(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let tags = library::tags_for(
        state.db(),
        &user.account_id.to_string(),
        SUBJECT_LIBRARY_ITEM,
        &id,
    )
    .await?;
    Ok(Json(serde_json::json!({
        "items": tags,
        "next_cursor": null,
    })))
}

/// Tag one item.
async fn add_item_tag(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path((id, tag)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let tag = normalise_tag(&tag).ok_or_else(|| {
        ApiError(AppError::Validation {
            message: "a tag must be between one and sixty-four characters".to_owned(),
            field_errors: Default::default(),
        })
    })?;
    // Same subject check as the status door: a tag on a subject that is not
    // the reader's item is a write into nobody's namespace, and the row it
    // leaves behind survives the item being removed.
    if !library::library_item_exists(state.db(), &user.account_id.to_string(), &id).await? {
        return Err(ApiError(AppError::NotFound {
            resource: "library item",
        }));
    }
    library::add_private_tag(
        state.db(),
        &user.account_id.to_string(),
        SUBJECT_LIBRARY_ITEM,
        &id,
        &tag,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Untag one item.
async fn remove_item_tag(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path((id, tag)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let Some(tag) = normalise_tag(&tag) else {
        // A tag that cannot exist cannot be attached, so its absence is the
        // honest answer rather than a validation failure.
        return Ok(StatusCode::NO_CONTENT);
    };
    let removed = library::remove_private_tag(
        state.db(),
        &user.account_id.to_string(),
        SUBJECT_LIBRARY_ITEM,
        &id,
        &tag,
    )
    .await?;
    if !removed {
        return Err(ApiError(AppError::NotFound {
            resource: "tag on this item",
        }));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Reading status
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct StatusBody {
    status: String,
}

/// The reader's status for one item.
///
/// The listing carries the status for every item on the page, so this is not how
/// a list draws its markers: it is for a screen that shows one item and has not
/// loaded a page to find out.
async fn read_status(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let row = library::reading_status_for(
        state.db(),
        &user.account_id.to_string(),
        SUBJECT_LIBRARY_ITEM,
        &id,
    )
    .await?;
    match row {
        Some(row) => Ok(Json(status_json(&row))),
        None => Err(ApiError(AppError::NotFound {
            resource: "reading status",
        })),
    }
}

/// Set the reader's status for an item.
async fn set_status(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Json(body): Json<StatusBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let Some(status) = ReadingStatus::parse(&body.status) else {
        return Err(ApiError(AppError::Validation {
            message: format!("`{}` is not a reading status this build knows", body.status),
            field_errors: Default::default(),
        }));
    };
    // The subject has to be a thing the reader actually has. Without this the
    // door accepts any UUID in the instance, writes a `reading_status` row
    // against a subject that does not exist, and answers 200 -- and the
    // reader's own dashboard then shows a zero that nothing on the page can
    // explain. A status with no subject is a client bug, not a stored fact.
    if !library::library_item_exists(state.db(), &user.account_id.to_string(), &id).await? {
        return Err(ApiError(AppError::NotFound {
            resource: "library item",
        }));
    }
    let row = library::set_reading_status(
        state.db(),
        &user.account_id.to_string(),
        SUBJECT_LIBRARY_ITEM,
        &id,
        status,
    )
    .await?;
    Ok(Json(status_json(&row)))
}

/// Clear the reader's status for an item.
async fn clear_status(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let removed = library::clear_reading_status(
        state.db(),
        &user.account_id.to_string(),
        SUBJECT_LIBRARY_ITEM,
        &id,
    )
    .await?;
    if !removed {
        return Err(ApiError(AppError::NotFound {
            resource: "reading status",
        }));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Saved views
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ViewBody {
    name: String,
    #[serde(default)]
    query: LibraryQuery,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    sort: Option<String>,
}

fn view_json(row: &library::SavedView) -> serde_json::Value {
    serde_json::json!({
        "id": row.id,
        "name": row.name,
        "query": row.query,
        "needs_repair": row.needs_repair,
        "query_version": row.query_version,
        "sort": row.sort.as_str(),
        "scope": row.scope.as_str(),
        "pinned": row.pinned,
        "is_public": row.is_public,
        "created_at": row.created_at,
        "updated_at": row.updated_at,
        "version": row.version,
    })
}

/// The reader's saved views.
async fn list_views(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let rows = library::saved_views_for(state.db(), &user.account_id.to_string()).await?;
    Ok(Json(serde_json::json!({
        "items": rows.iter().map(view_json).collect::<Vec<_>>(),
        "next_cursor": null,
    })))
}

/// Store a query as a view.
///
/// A public view that filters by a shelf, a private tag or a reading status is
/// refused by the domain, not by this handler — see
/// [`lorehaven_db::library::create_saved_view`], which validates before it
/// writes so no second caller can forget to.
async fn create_view(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<ViewBody>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let scope = body
        .scope
        .as_deref()
        .map_or_else(ViewScope::default, |raw| {
            ViewScope::parse(raw).unwrap_or_default()
        });
    let sort = body
        .sort
        .as_deref()
        .map_or(body.query.sort, LibrarySort::parse);
    let row = library::create_saved_view(
        state.db(),
        &user.account_id.to_string(),
        &body.name,
        &body.query,
        scope,
        body.pinned,
        sort,
    )
    .await
    .map_err(|error| {
        ApiError(AppError::Validation {
            message: error.to_string(),
            field_errors: Default::default(),
        })
    })?;
    Ok((StatusCode::CREATED, Json(view_json(&row))))
}

/// One view.
async fn get_view(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let row = library::find_saved_view(state.db(), &user.account_id.to_string(), &id)
        .await?
        .ok_or_else(|| {
            ApiError(AppError::NotFound {
                resource: "saved view",
            })
        })?;
    Ok(Json(view_json(&row)))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ViewPatchBody {
    name: Option<String>,
    pinned: Option<bool>,
    expected_version: i64,
}

/// Rename or pin a view.
async fn patch_view(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Json(body): Json<ViewPatchBody>,
) -> ApiResult<StatusCode> {
    let account = user.account_id.to_string();
    let existing = library::find_saved_view(state.db(), &account, &id)
        .await?
        .ok_or_else(|| {
            ApiError(AppError::NotFound {
                resource: "saved view",
            })
        })?;
    let changed = library::update_saved_view(
        state.db(),
        &account,
        &id,
        body.name.as_deref(),
        body.pinned,
        body.expected_version,
    )
    .await?;
    if !changed {
        return Err(ApiError(AppError::RevisionConflict {
            expected: body.expected_version,
            actual: existing.version,
        }));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Delete a view.
async fn delete_view_route(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let removed = library::delete_saved_view(state.db(), &user.account_id.to_string(), &id).await?;
    if !removed {
        return Err(ApiError(AppError::NotFound {
            resource: "saved view",
        }));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Update checks and storage
// ---------------------------------------------------------------------------

/// Queue a check of the reader's library against its sources.
async fn start_update_check(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let account = user.account_id.to_string();
    let item_count: i64 = library::storage_usage(state.db(), &account)
        .await
        .map(|usage| usage.item_count)?;
    if item_count == 0 {
        // Nothing to check. A job that reads no network and writes no record is
        // a job that looks like it did something.
        return Err(ApiError(AppError::Validation {
            message: "there is nothing in this library to check".to_owned(),
            field_errors: Default::default(),
        }));
    }
    let job_id = jobs::enqueue(
        state.db(),
        lorehaven_domain::jobs::JobKind::UpdateCheck,
        &serde_json::json!({ "account_id": account }).to_string(),
        None,
        Some(user.account_id),
        0,
        &lorehaven_domain::jobs::RetryPolicy::default(),
    )
    .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "job_id": job_id.to_string(),
            "items": item_count,
        })),
    ))
}

/// What the reader's library occupies.
async fn storage_usage(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let usage = library::storage_usage(state.db(), &user.account_id.to_string()).await?;
    Ok(Json(serde_json::json!({
        "imported_bytes": usage.imported_bytes,
        "export_bytes": usage.export_bytes,
        "total_bytes": usage.total_bytes,
        "item_count": usage.item_count,
        "blob_count": usage.blob_count,
        // Said in the response as well as in the code, because the number is
        // physical: two items sharing one stored copy count once.
        "counts": "stored bytes, each blob counted once",
    })))
}

// ---------------------------------------------------------------------------
// Batch removal
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct BatchBody {
    ids: Vec<String>,
    /// Whether to delete the imported copies as well as the references.
    #[serde(default)]
    delete_copy: bool,
}

/// Remove several library items at once.
///
/// Answers with the per-item outcome spec §14.3 fixes, and with the sentence an
/// interface should show. The storage a `delete_copy` request frees is reported
/// as bytes, which is the number the reader was shown before they confirmed.
async fn batch_remove_items(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<BatchBody>,
) -> ApiResult<Json<serde_json::Value>> {
    if body.ids.is_empty() {
        return Err(ApiError(AppError::Validation {
            message: "a batch needs at least one item".to_owned(),
            field_errors: Default::default(),
        }));
    }
    if body.ids.len() > 200 {
        return Err(ApiError(AppError::Validation {
            message: "a batch is limited to two hundred items".to_owned(),
            field_errors: Default::default(),
        }));
    }

    let account = user.account_id.to_string();
    let outcome =
        library::remove_library_items(state.db(), &account, &body.ids, body.delete_copy).await?;

    // Whatever the removal orphaned, and no more: the store checks the
    // reference count again before it deletes anything, and a blob that turned
    // out to be shared is left alone.
    let mut freed_bytes: i64 = 0;
    if !outcome.orphaned_checksums.is_empty() {
        // Sized before the deletion, because the deletion takes the row that
        // carries the size.
        let sizes = library::blob_sizes(state.db(), &outcome.orphaned_checksums).await?;
        let size_of = |checksum: &str| {
            sizes
                .iter()
                .find(|(known, _)| known == checksum)
                .map_or(0, |(_, bytes)| *bytes)
        };
        let store = BlobStore::new(state.config().storage.root.clone());
        for checksum in &outcome.orphaned_checksums {
            match store.delete_if_unreferenced(state.db(), checksum).await {
                // Only bytes that really went are reported as freed, so the
                // number the reader is shown after the fact is not larger than
                // what the disk gave back.
                Ok(true) => freed_bytes += size_of(checksum),
                Ok(false) => {}
                Err(error) => {
                    // A file that could not be removed does not un-remove the
                    // item: the reference is already gone and the next
                    // collection pass picks the blob up.
                    tracing::warn!(%checksum, %error, "could not free an orphaned blob");
                }
            }
        }
    }

    let removed = outcome.outcome.succeeded.len();
    Ok(Json(serde_json::json!({
        "succeeded": outcome.outcome.succeeded,
        "failed": outcome.outcome.failed,
        "summary": BatchOutcome::from_parts(outcome.outcome.succeeded.clone(), outcome.outcome.failed.clone())
            .summary("removed", "removed"),
        "freed_bytes": freed_bytes,
        "delete_copy": body.delete_copy,
        "removed": removed,
    })))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Validate a shelf name, returning it trimmed.
///
/// Refused rather than truncated: a shelf whose name the reader cannot see in
/// full is a shelf they cannot tell from another one.
fn read_shelf_name(raw: &str) -> Result<String, ApiError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(ApiError(AppError::Validation {
            message: "a shelf needs a name".to_owned(),
            field_errors: Default::default(),
        }));
    }
    if name.chars().count() > MAX_SHELF_NAME_CHARS {
        return Err(ApiError(AppError::Validation {
            message: format!("a shelf name is at most {MAX_SHELF_NAME_CHARS} characters"),
            field_errors: Default::default(),
        }));
    }
    Ok(name.to_owned())
}
