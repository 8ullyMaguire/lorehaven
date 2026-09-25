//! M45 — Roadmap consensus routes (spec §44).
//!
//! The board is public; voting and suggesting are TL>=1; stage moves are
//! operator-only.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{MaybeSession, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use lorehaven_db::{governance, roadmap};
use lorehaven_domain::consensus::{K_FACTOR, START_RATING};
use lorehaven_domain::AppError;
use std::collections::BTreeMap;

pub fn read_router() -> Router<AppState> {
    Router::new()
        .route("/roadmap", get(get_board))
        .route("/roadmap/changelog", get(get_changelog))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/roadmap/arena", get(get_arena).post(post_arena_vote))
        .route("/roadmap/suggest", post(post_suggest))
}

pub fn admin_router() -> Router<AppState> {
    Router::new().route("/admin/roadmap/move", post(post_move_card))
}

/// `GET /api/v1/roadmap` — the full board, grouped by stage. Public (spec §44.5).
pub async fn get_board(
    _maybe: MaybeSession,
    State(state): State<AppState>,
) -> ApiResult<Json<Value>> {
    let cards = roadmap::list_cards(state.db(), None)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    let grouped = group_by_stage(cards);
    Ok(Json(json!({ "board": grouped })))
}

/// Group cards by stage, each bucket ordered by Elo DESC.
fn group_by_stage(cards: Vec<lorehaven_db::roadmap::Card>) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    for card in cards {
        let entry = map
            .entry(card.stage.clone())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Value::Array(arr) = entry {
            arr.push(json!({
                "id": card.id,
                "title": card.title,
                "category": card.category,
                "elo_rating": card.elo_rating,
                "matches_played": card.matches_played,
                "times_best": card.times_best,
                "times_worst": card.times_worst,
            }));
        }
    }
    map
}

/// `GET /api/v1/roadmap/arena` — one ballot (4 idea cards + pre-match Elo).
pub async fn get_arena(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    let account_id = user.account_id.to_string();
    let trust = governance::trust_for(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    if trust < 1 {
        return Err(ApiError(AppError::Validation {
            message: "trust level too low to get an arena ballot".into(),
            field_errors: BTreeMap::new(),
        }));
    }

    let candidates = roadmap::arena_candidates(state.db(), 4)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    if candidates.len() < 4 {
        return Ok(Json(
            json!({ "ballot": null, "reason": "not_enough_cards" }),
        ));
    }

    let ballot_id = uuid::Uuid::new_v4().to_string();
    let card_ids: Vec<String> = candidates.iter().map(|c| c.id.clone()).collect();
    let served_elo: Vec<(String, f64)> = candidates
        .iter()
        .map(|c| (c.id.clone(), c.elo_rating))
        .collect();

    roadmap::create_ballot(state.db(), &ballot_id, &card_ids, &served_elo, &account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    Ok(Json(json!({
        "ballot_id": ballot_id,
        "cards": candidates.iter().map(|c| json!({
            "id": c.id,
            "title": c.title,
            "category": c.category,
            "elo_rating": c.elo_rating,
        })).collect::<Vec<_>>(),
        "served_elo": served_elo,
    })))
}

/// `POST /api/v1/roadmap/arena` — submit a best/worst vote.
#[derive(Debug, Deserialize)]
pub struct ArenaVoteBody {
    pub ballot_id: String,
    pub best_id: String,
    pub worst_id: String,
}

pub async fn post_arena_vote(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<ArenaVoteBody>,
) -> ApiResult<Json<Value>> {
    let account_id = user.account_id.to_string();
    let trust = governance::trust_for(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    if trust < 1 {
        return Err(ApiError(AppError::Validation {
            message: "trust level too low to vote".into(),
            field_errors: BTreeMap::new(),
        }));
    }

    if body.best_id == body.worst_id {
        return Err(ApiError(AppError::Validation {
            message: "best and worst must be distinct".into(),
            field_errors: BTreeMap::new(),
        }));
    }

    // One vote per ballot per account: atomic UPDATE … WHERE voted_at IS NULL.
    let voted = roadmap::mark_voted(state.db(), &body.ballot_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    if !voted {
        return Err(ApiError(AppError::Validation {
            message: "already voted on this ballot".into(),
            field_errors: BTreeMap::new(),
        }));
    }

    // Fetch served Elo and compute updates.
    let (card_ids, served_elo, _) = roadmap::fetch_ballot(state.db(), &body.ballot_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "ballot" }))?;

    let find_rating = |id: &str| {
        served_elo
            .iter()
            .find(|(cid, _)| cid == id)
            .map(|(_, r)| *r)
    };

    let best_rating = find_rating(&body.best_id).ok_or_else(|| {
        ApiError(AppError::Validation {
            message: "best_id not in this ballot".into(),
            field_errors: BTreeMap::new(),
        })
    })?;
    let worst_rating = find_rating(&body.worst_id).ok_or_else(|| {
        ApiError(AppError::Validation {
            message: "worst_id not in this ballot".into(),
            field_errors: BTreeMap::new(),
        })
    })?;

    // Collect unchosen card IDs and their ratings.
    let unchosen: Vec<(String, f64)> = card_ids
        .iter()
        .filter(|id| *id != &body.best_id && *id != &body.worst_id)
        .map(|id| {
            let r = find_rating(id).unwrap_or(START_RATING);
            (id.clone(), r)
        })
        .collect();

    let unchosen_ratings: Vec<f64> = unchosen.iter().map(|(_, r)| *r).collect();
    let outcome = lorehaven_domain::consensus::maxdiff_elo_updates(
        best_rating,
        worst_rating,
        &unchosen_ratings,
        K_FACTOR,
    );

    // Build updates: (card_id, new_elo, is_best, is_worst).
    let mut updates: Vec<(String, f64, bool, bool)> = Vec::new();
    updates.push((
        body.best_id.clone(),
        best_rating + outcome.best_delta,
        true,
        false,
    ));
    updates.push((
        body.worst_id.clone(),
        worst_rating + outcome.worst_delta,
        false,
        true,
    ));
    for (i, (id, orig_r)) in unchosen.iter().enumerate() {
        updates.push((
            id.clone(),
            orig_r + outcome.unchosen_deltas[i],
            false,
            false,
        ));
    }

    roadmap::apply_elo_and_counters(state.db(), &updates)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    Ok(Json(json!({ "status": "recorded" })))
}

/// `POST /api/v1/roadmap/suggest` — suggest a feature (TL>=1).
#[derive(Debug, Deserialize)]
pub struct SuggestBody {
    pub title: String,
}

pub async fn post_suggest(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<SuggestBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let account_id = user.account_id.to_string();
    let trust = governance::trust_for(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    if trust < 1 {
        return Err(ApiError(AppError::Validation {
            message: "trust level too low to suggest".into(),
            field_errors: BTreeMap::new(),
        }));
    }

    let card = roadmap::find_card_by_title_normalized(state.db(), &body.title)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    let card_id = card.as_ref().map(|c| c.id.clone());
    roadmap::insert_suggestion(state.db(), &account_id, &body.title, card_id.as_deref())
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "status": "recorded",
            "matched_existing": card_id.is_some(),
        })),
    ))
}

/// `POST /api/v1/admin/roadmap/move` — move a card to a new stage.
#[derive(Debug, Deserialize)]
pub struct MoveBody {
    pub card_id: String,
    pub stage: String,
    pub reason: String,
}

pub async fn post_move_card(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<MoveBody>,
) -> ApiResult<Json<Value>> {
    // Operator-only.
    let account_id = user.account_id.to_string();
    let is_operator = governance::is_operator(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    if !is_operator {
        return Err(ApiError(AppError::AccessDenied));
    }

    let stage = body.stage.as_str();
    if !lorehaven_domain::consensus::STAGES.contains(&stage) {
        return Err(ApiError(AppError::Validation {
            message: format!("invalid stage: {stage}"),
            field_errors: BTreeMap::new(),
        }));
    }

    let cards = roadmap::list_cards(state.db(), None)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    let card = cards
        .iter()
        .find(|c| c.id == body.card_id)
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "card" }))?;

    let from_stage = card.stage.clone();
    roadmap::record_move(
        state.db(),
        &body.card_id,
        &from_stage,
        stage,
        &body.reason,
        &account_id,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    // Update the card's stage directly.
    roadmap::update_card_stage(state.db(), &body.card_id, stage)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    Ok(Json(json!({ "status": "moved" })))
}

/// `GET /api/v1/roadmap/changelog` — public feed of stage moves.
pub async fn get_changelog(
    State(state): State<AppState>,
    MaybeSession(_session): MaybeSession,
) -> ApiResult<Json<Value>> {
    let moves = roadmap::list_moves(state.db(), 50, 0)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(json!({
        "moves": moves.iter().map(|(card_id, from, to, reason, moved_by, at)| {
            json!({
                "card_id": card_id,
                "from_stage": from,
                "to_stage": to,
                "reason": reason,
                "moved_by": moved_by,
                "created_at": at,
            })
        }).collect::<Vec<_>>()
    })))
}
