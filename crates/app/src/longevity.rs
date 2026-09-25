use anyhow::Result;
use lorehaven_domain::jobs::{JobKind, RetryPolicy};

use crate::state::AppState;

/// Schedule the nightly half-life recompute (spec §41.1).
pub async fn schedule_half_life(state: &AppState) -> Result<()> {
    let payload = serde_json::json!({
        "task": "recompute_half_life",
        "min_age_days": state.config().discovery.half_life_min_age_days,
        "window_days": state.config().discovery.half_life_window_days,
    });
    lorehaven_db::jobs::enqueue(
        state.db(),
        JobKind::Maintenance,
        &payload.to_string(),
        None,
        None,
        0,
        &RetryPolicy::default(),
    )
    .await?;
    Ok(())
}

/// Recompute half-life scores (called by the maintenance job handler).
pub async fn recompute_half_life(
    state: &AppState,
    min_age_days: i64,
    window_days: i64,
) -> Result<u64> {
    lorehaven_db::longevity::recompute_half_life(state.db(), min_age_days, window_days).await
}
