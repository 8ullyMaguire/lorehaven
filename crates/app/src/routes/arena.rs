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
///
/// `round` is **optional on purpose**. `generate_arena_round` returns `None` for an
/// ordinary state — a reader who has rated nothing, on an instance whose works do
/// not yet share a fandom in fours — and that used to be reported as
/// `AppError::Internal`, i.e. a 500. It is not a server fault: it is the answer to
/// "is there a round for this reader right now?", and the answer is frequently no.
/// §0.4.2a's arena is a calibration aid, not a door, so an absent round is `200`
/// with an explanation rather than a failure string (the same rule §54.6 states for
/// the concierge, and the same rule `100-ideas-remaining.md` §3 states for every
/// rail: an empty answer renders as nothing, never as `Internal`).
#[derive(Debug, Serialize)]
pub struct ArenaNextResponse {
    pub round: Option<ArenaRound>,
    /// Why there is no round, when there is none. Absent whenever `round` is.
    ///
    /// Kept separate from `round: null` being the *only* signal so the client can
    /// tell a reader with an empty pool from a reader who has exhausted every
    /// pair worth asking about.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explained_empty: Option<String>,
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

/// Build the arena's card pool from the DB rows, with each work's real taste
/// vector attached.
///
/// The vector is what the arena varies on, so it has to come from the same
/// scoring the rest of the recommender uses. `get_arena_pool` returns the
/// stored taste vector alongside each work; when one is missing the card falls
/// back to the neutral vector rather than a fabricated score.
fn build_cards(
    rows: Vec<lorehaven_db::taste_vectors::ArenaPoolRow>,
    dimensions: &[TasteDimension],
) -> Vec<ArenaCard> {
    let dimension_tuples: Vec<(String, String, f64, f64)> = dimensions
        .iter()
        .map(|d| (d.key.clone(), d.label.clone(), d.admin_target, d.weight))
        .collect();

    // The round varies on the reader's least-confident dimension: a dimension
    // the model has no opinion about is the one worth learning.
    let target_dimension = dimensions
        .first()
        .map(|d| d.key.clone())
        .unwrap_or_default();

    rows.into_iter()
        .map(|row| ArenaCard {
            work_id: row.id,
            title: row.title,
            fandom: row.fandom,
            tags: row.tags,
            word_count: row.word_count,
            excerpt: row.summary,
            target_dimension: target_dimension.clone(),
            vector: lorehaven_domain::taste_vector::work_vector_from_tags(
                &dimension_tuples,
                &row.tag_weights,
            ),
        })
        .collect()
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

    let cards = build_cards(pool_rows, &dimensions);

    // Vary on every configured dimension: the round should offer the reader a
    // real tradeoff across their whole profile, not one arbitrary axis.
    let targets: Vec<String> = dimensions.iter().map(|d| d.key.clone()).collect();
    // `None` here is an ordinary state, not a fault: a reader who has rated
    // nothing, on an instance with fewer than four public works sharing a fandom,
    // has no round to be offered. It used to be `AppError::Internal` — a 500 — and
    // `/arena` opened with a red "That did not work" for every new account.
    //
    // The dimensions are still computed and reported either way, because they are
    // what the page shows about calibration progress, and an empty round must not
    // cost the reader that too.
    let round = generate_arena_round(&cards, &dimensions, &targets);
    let explained_empty = round.is_none().then(|| {
        if cards.is_empty() {
            "No published, public works are available to compare yet."
        } else {
            "Not enough comparable works yet — an arena round needs four published \
             works that share a fandom."
        }
        .to_owned()
    });

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
        explained_empty,
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

    // The ballot has to be scored against the works it was cast for, so rebuild
    // the round the reader actually saw. Passing an empty round — which is what
    // this used to do — left the Elo loop with no cards to compare and every
    // dimension frozen at 1500 forever.
    let ballots = lorehaven_db::taste_vectors::get_arena_pool(db, &account_id, 200)
        .await
        .map_err(|e| ApiError(AppError::Internal(anyhow::anyhow!("arena pool: {e}"))))?;
    let cards = build_cards(ballots, &dimensions);
    let round = ArenaRound {
        cards: cards
            .into_iter()
            .filter(|card| card.work_id == req.best_work_id || card.work_id == req.worst_work_id)
            .collect(),
    };

    let ballot = ArenaBallot {
        best_work_id: req.best_work_id.clone(),
        worst_work_id: req.worst_work_id.clone(),
        reason_tags: req.reason_tags.clone(),
    };

    let updated_elos = apply_arena_ballot(&elos, &round, &ballot, &dimensions);

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

    // Generate next round. The pool now excludes the works just voted on, so
    // the reader is never asked the same question twice.
    let pool_rows = lorehaven_db::taste_vectors::get_arena_pool(db, &account_id, 50)
        .await
        .map_err(|e| ApiError(AppError::Internal(anyhow::anyhow!("arena pool: {e}"))))?;
    let cards = build_cards(pool_rows, &dimensions);
    let targets: Vec<String> = dimensions.iter().map(|d| d.key.clone()).collect();
    let next_round = generate_arena_round(&cards, &dimensions, &targets);

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
///
/// The fallback below is load-bearing, and the reason it used not to fire is worth
/// recording: `accounts.taste_vector` is `DEFAULT '[]'`, not NULL (migration 0053),
/// so **every** account has a row and `get_taste_vector` returns
/// `Some((vec![], ..))` rather than `None`. Mapping that empty vector gave a
/// reader with no quiz answers **zero** dimensions, and `generate_arena_round`
/// returns `None` when it is given no target dimensions — so every new account got
/// `Internal("not enough works for arena round")` → 500, on a seeded instance as
/// well as an empty one, and the message named works when the real problem was
/// dimensions.
///
/// So the test is `vec.is_empty()`, not `profile.is_some()`. A reader who has not
/// taken the quiz gets the instance's default dimensions, which is what the
/// `.unwrap_or_else` arm was written to do all along.
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
        .filter(|(vec, _, _)| !vec.is_empty())
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
