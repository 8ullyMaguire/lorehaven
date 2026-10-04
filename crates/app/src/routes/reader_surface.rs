//! Reader-surface discovery routes: items 14, 27 and 33 of the 100-idea audit.
//!
//! Spec: `docs/spec-reader-surface-t1.md`. Store: `crates/db/src/reader_surface.rs`.
//!
//! # Three routes, and only one of them is behind a login
//!
//! Item 14 ("new in your fandoms") is computed from **the caller's own** public
//! bookmarks, so it needs a session: there is no reader to answer "yours" for.
//!
//! Items 27 and 33 are computed entirely from public rows, and they stay **public**. A
//! login wall in front of data the instance is willing to publish teaches readers that
//! the data is not public, and it would also mean the privacy rule in the store module
//! (count only `is_public = 1`) is invisible to anyone who cannot log in — which is
//! exactly the reader who needs to be able to check it.
//!
//! # The empty response is `{"works": []}`, never `null` and never a 404
//!
//! A client iterating the result should not have to handle null. "Nothing new in your
//! fandoms" is an answer, not a missing route — and per spec §3 the section changes
//! subject when it has no data, so a client must be able to tell "empty" from "absent"
//! without inspecting a status code.
//!
//! # The window is computed here, not in SQL
//!
//! `most_bookmarked_this_week`'s window is a bound parameter because the two engines
//! spell date arithmetic differently (spec §4). Computing it in Rust keeps one code path
//! and is what makes the store function testable with a fixed clock.

use crate::auth::{MaybeSession, RequireSession};
use crate::http::ApiResult;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use chrono::{Duration, Utc};

/// Items 27 and 33 are public; item 14 is per-reader.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/discovery/new-in-your-fandoms", get(new_in_your_fandoms))
        .route("/discovery/most-bookmarked", get(most_bookmarked))
        .route("/works/{work_id}/similar", get(similar))
}

/// Caps come from the store module, not from a literal in a handler.
///
/// A limit that differs between the store default and the route default is a limit that
/// eventually differs between what is tested and what is served.
const NEW_IN_FANDOMS: i64 = lorehaven_db::reader_surface::NEW_IN_FANDOMS_LIMIT;
const MOST_BOOKMARKED: i64 = lorehaven_db::reader_surface::MOST_BOOKMARKED_LIMIT;
const SIMILAR: i64 = lorehaven_db::reader_surface::SIMILAR_WORKS_LIMIT;

/// The leaderboard's window. Seven days is the spec's, and it is here rather than in the
/// store because the store takes the bound value, not a number of days.
const WEEK: i64 = 7;

#[derive(Debug, Deserialize)]
pub struct MostBookmarkedQuery {
    /// Overrides the seven-day window, for the admin view and for tests. Refused when
    /// implausible rather than silently clamped, because a silently-clamped `window_days`
    /// returns a leaderboard that is not the one the caller asked about.
    #[serde(default)]
    window_days: Option<i64>,
}

/// The empty-everything response shape, used by all three routes.
///
/// `works` is a real array in every case. A `null` here would force every client to
/// branch on null-ness, and the branch would be wrong the first time one component
/// forgot it.
#[derive(Debug, Serialize)]
struct WorksResponse<T: Serialize> {
    works: Vec<T>,
}

impl<T: Serialize> WorksResponse<T> {
    fn new(works: Vec<T>) -> Self {
        Self { works }
    }
}

/// `GET /discovery/new-in-your-fandoms` — item 14. **Requires a session.**
///
/// Returns `{"works": []}` when the reader has no public bookmarks. It does NOT fall back
/// to all recent works: a section that changes subject when it has no data is a section
/// nobody can learn to read (spec §3).
async fn new_in_your_fandoms(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<WorksResponse<lorehaven_db::reader_surface::SurfaceWork>>> {
    let db = state.db();
    let works = lorehaven_db::reader_surface::new_in_your_fandoms(
        db,
        &user.account_id.to_string(),
        NEW_IN_FANDOMS,
    )
    .await
    .map_err(|e| crate::http::ApiError(lorehaven_domain::AppError::Internal(e)))?;

    Ok(Json(WorksResponse::new(works)))
}

/// `GET /discovery/most-bookmarked` — item 27. **Public.**
///
/// Counts distinct PUBLIC bookmarkers in a seven-day window. The privacy rule is in the
/// store query, and `most_bookmarked_counts_only_public_bookmarks` is what enforces it.
async fn most_bookmarked(
    State(state): State<AppState>,
    // `MaybeSession` is UNUSED and deliberately so. The route is public, and the route
    // inventory's `every_route_has_correct_audience` check treats a handler with no
    // extractor as undeclared rather than public — which is the right default, because
    // "no extractor" is usually an oversight. Naming it makes the public door explicit
    // instead of implied by its absence.
    MaybeSession(_session): MaybeSession,
    Query(q): Query<MostBookmarkedQuery>,
) -> ApiResult<Json<WorksResponse<lorehaven_db::reader_surface::SurfaceWork>>> {
    // Refuse an absurd window rather than accepting it. `window_days=0` would return
    // everything since the epoch, which is the whole table under a heading that says
    // "this week" — a wrong answer with a correct-looking label.
    //
    // 422, not 400: `AppError::Validation` maps to 422 across this codebase
    // (`error.rs::status_code`), and hand-writing a 400 here would make this route the
    // only validation failure in the API answering with a different status.
    let days = q.window_days.unwrap_or(WEEK);
    if !(1..=365).contains(&days) {
        return Err(crate::http::ApiError(lorehaven_domain::AppError::field(
            "window_days",
            "must be between 1 and 365",
        )));
    }

    let window_start =
        (Utc::now() - Duration::days(days)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let works = lorehaven_db::reader_surface::most_bookmarked_this_week(
        state.db(),
        &window_start,
        MOST_BOOKMARKED,
    )
    .await
    .map_err(|e| crate::http::ApiError(lorehaven_domain::AppError::Internal(e)))?;

    Ok(Json(WorksResponse::new(works)))
}

/// `GET /works/{work_id}/similar` — item 33. **Public.**
///
/// Empty when the work has fewer than two tags or nothing clears the honesty floor, and
/// empty is the correct answer in both cases: an empty rail labelled "Similar works" is
/// worse than no rail (spec §5).
async fn similar(
    State(state): State<AppState>,
    // Public door, declared explicitly. See the note on `most_bookmarked`.
    MaybeSession(_session): MaybeSession,
    Path(work_id): Path<String>,
) -> ApiResult<Json<WorksResponse<lorehaven_db::reader_surface::SurfaceWork>>> {
    let works = lorehaven_db::reader_surface::similar_works(state.db(), &work_id, SIMILAR)
        .await
        .map_err(|e| crate::http::ApiError(lorehaven_domain::AppError::Internal(e)))?;

    Ok(Json(WorksResponse::new(works)))
}
