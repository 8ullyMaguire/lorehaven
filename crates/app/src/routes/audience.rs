use crate::auth::RequireSession;
use crate::http::ApiError;
use crate::state::AppState;
use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

/// Author-facing audience panel (spec §41.2).
pub fn router() -> Router<AppState> {
    Router::new().route("/me/audience", get(get_audience))
}

/// Get the warmth tier aggregates for all works owned by the caller.
///
/// Returns the tier counts — no per-reader data (spec §41.3: "Never a list of
/// names, never a per-reader value, never shown to the reader themselves").
async fn get_audience(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> crate::http::ApiResult<Json<serde_json::Value>> {
    let tiers =
        lorehaven_db::longevity::author_tier_aggregates(state.db(), &user.account_id.to_string())
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;

    let aggregates = lorehaven_domain::longevity::aggregate_tiers(&tiers);
    Ok(Json(serde_json::json!({
        "lurk": aggregates.lurk,
        "react": aggregates.react,
        "comment": aggregates.comment,
        "create": aggregates.create,
        "total": aggregates.total,
    })))
}
