//! Community routes: comments, forums, groups, messaging, blocks.
//!
//! Spec §17. Groundwork only: the tables land in migration 0013 and the
//! routes arrive with the milestone that owns each surface.

use crate::auth::RequireSession;
use crate::http::ApiResult;
use crate::state::AppState;
use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

pub fn router() -> Router<AppState> {
    Router::new().route("/community/blocks", get(list_blocks))
}

async fn list_blocks(
    State(_state): State<AppState>,
    RequireSession(_): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    // Stub until M12 wires the real blocks surface (0013_community.sql holds
    // the tables; enforcement arrives with the milestone that owns it).
    Ok(Json(serde_json::json!({ "items": [] })))
}
