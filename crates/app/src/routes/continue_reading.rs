//! Item 1 of the 100-idea audit: "Continue Reading".
//!
//! Spec: `docs/plans/100-ideas-remaining.md` §2a. Store: `crates/db/src/continue_reading.rs`.
//!
//! # One route, and it is always behind a login
//!
//! `GET /api/continue-reading` answers a question only a signed-in reader can ask: where
//! did **you** stop. There is no anonymous version of that question, so there is no
//! `MaybeSession` here — the reader-surface routes use `MaybeSession` because "new in
//! your fandoms" degrades to "new, unfiltered" for a stranger, and "most bookmarked this
//! week" is public by construction. Neither degradation exists for this one, and serving
//! an empty banner to a stranger is worse than a login wall: it reads as "you have read
//! nothing" when it means "we do not know who you are".
//!
//! # `404` means "nothing to continue", and that is not an error
//!
//! The reader finished everything, or never opened anything. Both are ordinary states,
//! and the banner simply does not render. A `200 {"continue": null}` would be the same
//! information in a shape the client has to unwrap; `404` is what the rest of the API
//! already means by "this does not exist", so the client gets one rule instead of two.
//! `is_not_found` in the frontend client keys on the status, not on the body.

use crate::auth::RequireSession;
use crate::http::ApiResult;
use crate::state::AppState;
use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

pub fn router() -> Router<AppState> {
    Router::new().route("/continue-reading", get(continue_reading))
}

/// The wire shape.
///
/// `position_permille` is carried as well as `percent` because they are not redundant.
/// The store returns the raw column so the *rounding* is the caller's decision, and a
/// client that only wants a progress bar should not have to re-derive it — while a client
/// that wants "page 12 of 340" needs the unrounded value. Emitting both means neither
/// side has to guess what the other rounded.
#[derive(Debug, Serialize)]
pub struct ContinueReadingResponse {
    pub work_id: String,
    pub title: String,
    pub position_permille: i32,
    /// Whole percent, already clamped to 0..=100 by the store.
    pub percent: u8,
    pub chapter_id: Option<String>,
    pub chapter_title: Option<String>,
    /// RFC 3339. When this reader last wrote this row — useful for "you were reading
    /// this three days ago" copy without a second round trip.
    pub updated_at: String,
}

async fn continue_reading(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<ContinueReadingResponse>> {
    let row =
        lorehaven_db::continue_reading::continue_reading(state.db(), &user.account_id.to_string())
            .await
            .map_err(|e| crate::http::ApiError(lorehaven_domain::AppError::Internal(e)))?
            .ok_or_else(|| {
                // 404, not 200-with-null. See the module comment. The `resource` noun is
                // deliberately coarse and singular: it must not distinguish "you read
                // nothing" from "everything you started is finished", because a signed-in
                // reader can enumerate their own library and does not need the API to
                // summarise it back at them.
                crate::http::ApiError(lorehaven_domain::AppError::NotFound {
                    resource: "reading progress",
                })
            })?;

    // Computed before the fields move out. `percent()` borrows `&self`, and the
    // struct-literal below moves `row.work_id` first, so the borrow would be of a
    // partially moved value. Binding it first also keeps the rounding decision visibly
    // the store's, rather than looking like arithmetic invented by the handler.
    let percent = row.percent();

    Ok(Json(ContinueReadingResponse {
        work_id: row.work_id,
        title: row.title,
        position_permille: row.position_permille,
        percent,
        chapter_id: row.chapter_id,
        chapter_title: row.chapter_title,
        updated_at: row.updated_at,
    }))
}
