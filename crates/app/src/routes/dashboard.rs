//! Creator dashboard (spec §32.3, §24.3; M24).
//!
//! One door, `GET /api/v1/me/dashboard`, returning what a creator may know
//! about their own works: how many there are, how much text is in them, and
//! what readers did — aggregated, positivity-framed, and banded.
//!
//! # What it deliberately does not return
//!
//! No reader, no pseud, no account, no per-reader row, and no exact count below
//! [`CREATOR_DASHBOARD_FLOOR`]. §24.3's analytics rules are about what an
//! aggregate may say: a dashboard that named the three readers who bookmarked a
//! story would be a tracking surface wearing a statistics page, and one that
//! returned "3" invites the same deduction without naming anyone.
//!
//! It also does not report how many of the author's comments were held by the
//! positivity filter. §12 frames the author's view as what arrived rather than
//! what was withheld, and §24.3 forbids public-shaming numbers; a "held" counter
//! on an author's own dashboard is a number about other people's words that the
//! author has no action to take on.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::auth::RequirePseud;
use crate::http::ApiResult;
use crate::state::AppState;
use lorehaven_db::analytics::{creator_totals, CREATOR_DASHBOARD_FLOOR};

pub fn router() -> Router<AppState> {
    Router::new().route("/me/dashboard", get(creator_dashboard))
}

/// A count as the dashboard reports it: exact at or above the floor, a band
/// below it.
///
/// The band is a string rather than a number on purpose — a JSON `5` that means
/// "somewhere between zero and four" is the kind of field a client renders as
/// an exact figure, and the whole point is that it cannot be.
fn banded(count: i64, floor: i64) -> Value {
    if count >= floor {
        json!(count)
    } else {
        json!(format!("fewer_than_{floor}"))
    }
}

pub async fn creator_dashboard(
    State(state): State<AppState>,
    RequirePseud { user: _, pseud_id }: RequirePseud,
) -> ApiResult<Json<Value>> {
    let totals = creator_totals(state.db(), &pseud_id.to_string()).await?;
    let floor = CREATOR_DASHBOARD_FLOOR;

    // A mean is only reported when its count is: an average of two ratings is
    // as identifying as the count itself, and an average of none is not a
    // number at all.
    let (rating_count, mean_stars) = if totals.ratings >= floor {
        (
            json!(totals.ratings),
            json!((totals.rating_stars as f64 / totals.ratings as f64 * 100.0).round() / 100.0),
        )
    } else {
        (banded(totals.ratings, floor), Value::Null)
    };

    Ok(Json(json!({
        "floor": floor,
        "note": "counts below the floor are reported as bands; no reader is ever identified",
        "works": {
            // The author's own inventory, exact: how many works they have
            // written is not a fact about any reader, and banding it would only
            // make the dashboard less useful than the writing page.
            "total": totals.works,
            "published": totals.published,
            "unpublished": totals.unpublished,
            "chapters": totals.chapters,
            "words": totals.words,
        },
        "readers": {
            "bookmarks": banded(totals.bookmarks, floor),
            "ratings": rating_count,
            "mean_stars": mean_stars,
            "reviews": banded(totals.reviews, floor),
        },
        "positivity": {
            // What arrived, framed the way §12.9 frames the author's view.
            "comments_delivered": banded(totals.comments_delivered, floor),
        },
    })))
}
