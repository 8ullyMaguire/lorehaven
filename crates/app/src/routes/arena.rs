//! Taste Calibration Arena routes (spec §0.4.2a).
//!
//! The arena presents 4 works sharing at least one major attribute (fandom,
//! genre, or length bracket). The reader picks best and worst, and the
//! forced tradeoff reveals which dimensions of taste matter most.

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use lorehaven_db;
use lorehaven_domain::taste_vector::{
    apply_arena_ballot, generate_arena_round, weights_from_elos, ArenaBallot, ArenaCard,
    ArenaRound, DimensionElo, TasteDimension,
};
use lorehaven_domain::AppError;
use serde::{Deserialize, Serialize};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/arena/next", get(get_arena_next))
        .route("/arena/vote", post(post_arena_vote))
        .route("/arena/dismiss", post(post_arena_dismiss))
        .route("/arena/weights", get(get_arena_weights))
}

/// Response for GET /arena/next.
#[derive(Debug, Serialize)]
pub struct ArenaNextResponse {
    pub round: ArenaRound,
    pub dimensions: Vec<DimensionSummary>,
}

/// A dimension summary shown to the user (never raw weights).
#[derive(Debug, Serialize)]
pub struct DimensionSummary {
    pub key: String,
    pub label: String,
    pub matches_played: u64,
}

/// Request body for POST /arena/vote.
#[derive(Debug, Deserialize)]
pub struct ArenaVoteRequest {
    pub best_work_id: String,
    pub worst_work_id: String,
    #[serde(default)]
    pub reason_tags: Vec<String>,
}

/// Response for POST /arena/vote.
#[derive(Debug, Serialize)]
pub struct ArenaVoteResponse {
    pub success: bool,
    pub message: String,
    pub next_round: Option<ArenaRound>,
}

/// Response for GET /arena/weights.
#[derive(Debug, Serialize)]
pub struct ArenaWeightsResponse {
    pub dimensions: Vec<DimensionWeightSummary>,
}

/// A dimension weight summary (never the raw weight value).
#[derive(Debug, Serialize)]
pub struct DimensionWeightSummary {
    pub key: String,
    pub label: String,
    /// Qualitative label, never a raw number (spec §0.3).
    pub influence: String,
}

/// GET /arena/next — the next arena round for the signed-in user.
async fn get_arena_next(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<ArenaNextResponse>> {
    let db = state.db();
    let account_id = user.account_id.to_string();

    // Get the account's taste dimensions (or defaults).
    let dimensions = get_account_dimensions(db, &account_id).await?;

    // Get works not yet voted on.
    let pool_rows = lorehaven_db::taste_vectors::get_arena_pool(db, &account_id, 50)
        .await
        .map_err(|e| ApiError(AppError::Internal(anyhow::anyhow!("arena pool: {e}"))))?;

    // Build ArenaCards from the pool.
    let cards: Vec<ArenaCard> = pool_rows
        .into_iter()
        .map(|(id, title, summary, fandom, tags, wc)| {
            ArenaCard {
                work_id: id,
                title,
                fandom,
                tags,
                word_count: wc,
                excerpt: summary,
                target_dimension: dimensions
                    .first()
                    .map(|d| d.key.clone())
                    .unwrap_or_else(|| "prose".to_string()),
            }
        })
        .collect();

    // Generate the arena round.
    let round = generate_arena_round(&cards, &dimensions, &["prose".to_string()])
        .ok_or_else(|| ApiError(AppError::Internal(anyhow::anyhow!("not enough works for arena round"))))?;

    // Get existing Elo ratings for dimensions.
    let elos = get_dimension_elos(db, &account_id, &dimensions).await?;

    let dimension_summaries = dimensions
        .iter()
        .zip(elos.iter())
        .map(|(d, e)| DimensionSummary {
            key: d.key.clone(),
            label: d.label.clone(),
            matches_played: e.matches_played as u64,
        })
        .collect();

    Ok(Json(ArenaNextResponse {
        round,
        dimensions: dimension_summaries,
    }))
}

/// POST /arena/vote — submit an arena ballot.
async fn post_arena_vote(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(req): Json<ArenaVoteRequest>,
) -> ApiResult<Json<ArenaVoteResponse>> {
    let db = state.db();
    let account_id = user.account_id.to_string();

    // Validate best != worst.
    if req.best_work_id == req.worst_work_id {
        return Err(ApiError(AppError::Validation {
            message: "best and worst must be different".into(),
            field_errors: Default::default(),
        }));
    }

    // Record the ballot.
    lorehaven_db::taste_vectors::record_arena_ballot(
        db,
        &account_id,
        &req.best_work_id,
        &req.worst_work_id,
        &req.reason_tags,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(anyhow::anyhow!("record ballot: {e}"))))?;

    // Get current Elo ratings.
    let dimensions = get_account_dimensions(db, &account_id).await?;
    let elos = get_dimension_elos(db, &account_id, &dimensions).await?;

    // Build a synthetic round with best/worst for the ballot.
    let synthetic_round = ArenaRound {
        cards: vec![],
    };

    let ballot = ArenaBallot {
        best_work_id: req.best_work_id.clone(),
        worst_work_id: req.worst_work_id.clone(),
        reason_tags: req.reason_tags.clone(),
    };

    let updated_elos = apply_arena_ballot(&elos, &synthetic_round, &ballot);

    // Store updated Elo ratings.
    let weight_pairs = weights_from_elos(&updated_elos);
    for (dim_key, weight) in &weight_pairs {
        let elo = updated_elos
            .iter()
            .find(|e| &e.dimension_key == dim_key)
            .unwrap();
        lorehaven_db::taste_vectors::update_arena_weights(
            db,
            &account_id,
            dim_key,
            *weight,
            elo.elo_rating,
            elo.matches_played as i64,
        )
        .await
        .map_err(|e| ApiError(AppError::Internal(anyhow::anyhow!("update weights: {e}"))))?;
    }

    // Generate next round.
    let pool_rows = lorehaven_db::taste_vectors::get_arena_pool(db, &account_id, 50)
        .await
        .map_err(|e| ApiError(AppError::Internal(anyhow::anyhow!("arena pool: {e}"))))?;
    let cards: Vec<ArenaCard> = pool_rows
        .into_iter()
        .map(|(id, title, summary, fandom, tags, wc)| ArenaCard {
            work_id: id,
            title,
            fandom,
            tags,
            word_count: wc,
            excerpt: summary,
            target_dimension: "prose".to_string(),
        })
        .collect();
    let next_round = generate_arena_round(&cards, &dimensions, &["prose".to_string()]);

    Ok(Json(ArenaVoteResponse {
        success: true,
        message: "Ballot recorded.".into(),
        next_round,
    }))
}

/// POST /arena/dismiss — skip the arena (falls back to quiz or egalitarian).
async fn post_arena_dismiss(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Arena dismissed. Falling back to egalitarian recommendations."
    })))
}

/// GET /arena/weights — the signed-in user's arena weight summary.
async fn get_arena_weights(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<ArenaWeightsResponse>> {
    let db = state.db();
    let account_id = user.account_id.to_string();
    let dimensions = get_account_dimensions(db, &account_id).await?;
    let elos = get_dimension_elos(db, &account_id, &dimensions).await?;
    let weights = weights_from_elos(&elos);

    let summaries = dimensions
        .iter()
        .map(|d| {
            let weight = weights
                .iter()
                .find(|(k, _)| k == &d.key)
                .map(|(_, w)| *w)
                .unwrap_or(0.0);
            // Convert to qualitative label (never raw number, spec §0.3).
            let influence = if weight > 0.3 {
                "Strong"
            } else if weight > 0.15 {
                "Moderate"
            } else {
                "Weak"
            };
            DimensionWeightSummary {
                key: d.key.clone(),
                label: d.label.clone(),
                influence: influence.to_string(),
            }
        })
        .collect();

    Ok(Json(ArenaWeightsResponse {
        dimensions: summaries,
    }))
}

/// Get the account's taste dimensions (or defaults).
async fn get_account_dimensions(
    db: &lorehaven_db::Database,
    account_id: &str,
) -> ApiResult<Vec<TasteDimension>> {
    // Try to get from taste profile.
    let profile = lorehaven_db::taste_vectors::get_taste_vector(db, account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(anyhow::anyhow!("taste vector: {e}"))))?;

    let default_keys: Vec<String> = lorehaven_domain::taste_vector::DEFAULT_DIMENSIONS
        .iter()
        .map(|s| s.to_string())
        .collect();

    let dims = profile
        .map(|(vec, _, _)| {
            vec.iter()
                .enumerate()
                .map(|(i, _)| TasteDimension {
                    key: default_keys
                        .get(i)
                        .cloned()
                        .unwrap_or_else(|| format!("dim_{}", i)),
                    label: format!("Dimension {}", i + 1),
                    admin_target: 0.5,
                    weight: 1.0 / (vec.len() as f64).max(1.0),
                })
                .collect()
        })
        .unwrap_or_else(|| {
            lorehaven_domain::taste_vector::DEFAULT_DIMENSIONS
                .iter()
                .map(|key| TasteDimension {
                    key: key.to_string(),
                    label: key.to_string(),
                    admin_target: 0.5,
                    weight: 1.0 / lorehaven_domain::taste_vector::DEFAULT_DIMENSIONS.len() as f64,
                })
                .collect()
        });

    Ok(dims)
}

/// Get or create Elo ratings for dimensions.
async fn get_dimension_elos(
    db: &lorehaven_db::Database,
    account_id: &str,
    dimensions: &[TasteDimension],
) -> ApiResult<Vec<DimensionElo>> {
    let rows = lorehaven_db::taste_vectors::get_arena_weights(db, account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(anyhow::anyhow!("arena weights: {e}"))))?;

    let elos = dimensions
        .iter()
        .map(|d| {
            let row = rows.iter().find(|(key, _, _, _)| key == &d.key);
            DimensionElo {
                dimension_key: d.key.clone(),
                elo_rating: row.map(|(_, _, elo, _)| *elo).unwrap_or(1500.0),
                matches_played: row.map(|(_, _, _, mp)| *mp as u32).unwrap_or(0),
            }
        })
        .collect();

    Ok(elos)
}
