//! §16.10a — the draw: a hand of cards dealt from the reader's own pool.
//!
//! The amendment is `docs/spec-amendments/draw-slot.md`; the deal itself is
//! [`darkhold_domain::draw`], which is pure and tested without a database.
//!
//! **A draw is a presentation, not a recommender.** The pool comes from the same
//! `rec_engine` registry `/discover` uses, the deal only chooses *which* of those
//! candidates the reader sees this time, and a work outside the pool cannot be
//! dealt. That is what makes the mechanism column meaningful: a card's chance of
//! appearing is its rank's chance, so the north-star metric can compare "found
//! through the draw" with "found through the feed" and be comparing two
//! presentations of one ranking rather than two rankings.
//!
//! **Every dealt card is a recorded `recommendation_slot`** with
//! `mechanism = "draw"`, and the take is recorded on the same row
//! (`chosen_at`, migration 0128). Served-and-taken and served-and-passed are then
//! two reads of one table, which is the only version of this feature that teaches
//! the recommender anything — Ascend's `draws.chosen_index` was the same idea, and
//! it is the part of that project worth adopting.
//!
//! # What is deliberately absent
//!
//! No rarity, no pity counter, no currency, no collection. Sending an empty deal
//! is `200` with `empty_reason` rather than an error, for the reason §54.6 states:
//! a rail with nothing to show is an answer, not a fault.

use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use darkhold_db::recommendation_slots::SlotRecord;
use darkhold_domain::draw::{deal, seed, DEFAULT_HAND, MAX_HAND};
use darkhold_domain::recommendation_transparency::{InstanceCuration, SlotMechanism, SlotReason};
use darkhold_domain::AppError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

/// How many candidates the deal draws *from*.
///
/// Sixty, not the whole catalogue: the registry is a ranking, so a pool of sixty
/// is the works the reader's engines consider worth serving at all. A larger pool
/// would make the draw a browse of the catalogue wearing a card suit. A hand of
/// three from sixty is one chance in twenty per card, and the hands differ across
/// nonces — which is the point of asking twice.
const POOL: usize = 60;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/discovery/draw", get(get_draw))
        .route("/discovery/draw/pick", post(post_draw_pick))
}

#[derive(Debug, Deserialize)]
struct DrawQuery {
    /// Which hand to deal. Same nonce, same day, same reader → the same hand.
    nonce: Option<u64>,
    /// How many cards. Clamped to [`MAX_HAND`] by the deal.
    hand: Option<usize>,
}

#[derive(Debug, Serialize)]
struct DrawCard {
    work_id: String,
    title: String,
    author_handle: String,
    /// The card's place in the hand, 0-based.
    position: i64,
    /// The slot that will explain this card, and that the pick names.
    slot_id: String,
}

#[derive(Debug, Deserialize)]
struct PickBody {
    request_id: String,
    work_id: String,
}

/// Deal a hand.
///
/// The pool is built through the same registry `/discover` uses, so a reader whose
/// engines are disabled or empty gets an empty hand rather than a second, quieter
/// recommendation path that nobody configured.
async fn get_draw(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Query(params): Query<DrawQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let account_id = user.account_id.to_string();
    // The pseud the session is acting as, falling back to the account id — the
    // same resolution `get_discovery` uses for its filters, so a slot recorded
    // here belongs to the same reader the feed's slots belong to.
    let viewer = user
        .pseud_id
        .as_ref()
        .map(|p| p.as_uuid())
        .unwrap_or_else(|| user.account_id.as_uuid());

    let date = chrono::Utc::now().date_naive().to_string();
    let nonce = params.nonce.unwrap_or(0);
    let hand = params.hand.unwrap_or(DEFAULT_HAND).clamp(1, MAX_HAND);

    // The pool is the candidate set `/discover` ranks, built by the same function
    // and blended the same way. What it deliberately does NOT do is run the
    // strategy registry: on this tree that path decodes work ids as `TEXT` from a
    // `UUID` column on PostgreSQL, so a draw built on it would fail on every
    // instance the feed happens to serve through the blend — and a draw that only
    // worked under `rec_mode = "pluggable"` would be a second pool with a second
    // set of bugs, which is the thing this slot exists to avoid.
    let engines = super::discovery::build_candidate_engines(
        &state,
        Some(&account_id),
        Some(viewer),
        POOL as i64,
    )
    .await?;
    let mut blended = darkhold_domain::discovery::blend(&engines);
    // `blend` merges engines but does not order; the feed orders the merge before
    // it serves it, and a pool is an order even when the hand is not.
    blended.sort_by_key(|candidate| -candidate.score);
    let pool: Vec<String> = blended
        .iter()
        .take(POOL)
        .map(|candidate| candidate.work_id.to_string())
        .collect();

    if pool.is_empty() {
        return Ok(Json(serde_json::json!({
            "cards": [],
            "request_id": serde_json::Value::Null,
            "date": date,
            "nonce": nonce,
            // The two empty states are kept apart for the reason Surprise Me
            // learned the hard way: "the catalogue has nothing to deal" and "your
            // engines produced nothing" are different messages, and one of them is
            // about the instance rather than the reader.
            "empty_reason": "the recommendation pool is empty: nothing is published, or every \
                             candidate is already in this reader's library",
        })));
    }

    let seed_text = seed(&account_id, &date, nonce);
    let dealt = deal(&seed_text, pool.len(), hand);
    let request_id = Uuid::new_v4();

    // The ids the deal chose, as uuids. A pool id that will not parse is a store
    // bug rather than a caller's fault, so it is an internal error and not a
    // silently shorter hand.
    let mut chosen_ids: Vec<Uuid> = Vec::with_capacity(dealt.len());
    for index in &dealt {
        let parsed = Uuid::parse_str(&pool[*index]).map_err(|e| {
            ApiError(AppError::Internal(anyhow::anyhow!(
                "recommendation pool held a non-uuid id {:?}: {e}",
                pool[*index]
            )))
        })?;
        chosen_ids.push(parsed);
    }

    let records: Vec<SlotRecord> = chosen_ids
        .iter()
        .enumerate()
        .map(|(position, work_id)| SlotRecord {
            pseud_id: viewer,
            work_id: *work_id,
            request_id,
            position: position as i64,
            // `Strategy` is the honest reason: the registry produced the candidate,
            // and the draw did not evaluate the match itself. Claiming
            // `TasteTags` here would attribute a match to a comparison this path
            // never made.
            reasons: vec![SlotReason::Strategy],
            taste_signal: None,
            seeded_by: None,
            recipe_stage: None,
            instance_curation: InstanceCuration::NotInvolved,
            blend_score: 0,
            mechanism: Some(SlotMechanism::Draw.as_str().to_string()),
        })
        .collect();

    let slot_ids = darkhold_db::recommendation_slots::record_response(
        state.db(),
        viewer,
        request_id,
        &records,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    let ids: Vec<String> = chosen_ids.iter().map(|id| id.to_string()).collect();
    let details = darkhold_db::discovery::work_details_for(state.db(), &ids)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    let cards: Vec<DrawCard> = ids
        .iter()
        .enumerate()
        .map(|(position, id)| {
            let (title, author_handle) = details
                .get(id)
                .cloned()
                // A work in the pool whose row vanished between ranking and
                // reading is a race, not a policy: the card is still served with
                // an empty title so the hand keeps its recorded positions, and the
                // link is what the reader follows.
                .unwrap_or_else(|| (String::new(), String::new()));
            DrawCard {
                work_id: id.clone(),
                title,
                author_handle,
                position: position as i64,
                slot_id: slot_ids.get(position).cloned().unwrap_or_default(),
            }
        })
        .collect();

    Ok(Json(serde_json::json!({
        "cards": cards,
        "request_id": request_id.to_string(),
        "date": date,
        "nonce": nonce,
        "pool_size": pool.len(),
    })))
}

/// Take one card.
///
/// Idempotent, and one take per hand: a second pick leaves the first take in place
/// and returns it (see `mark_chosen`). A `404` here means the slot is not the caller's — the same
/// answer as a slot that was never served, because making the two distinguishable
/// would answer "did this instance serve this work to somebody" for anyone who can
/// guess a uuid.
async fn post_draw_pick(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<PickBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let viewer = user
        .pseud_id
        .as_ref()
        .map(|p| p.as_uuid())
        .unwrap_or_else(|| user.account_id.as_uuid());

    if Uuid::parse_str(&body.request_id).is_err() || Uuid::parse_str(&body.work_id).is_err() {
        return Err(ApiError(AppError::NotFound { resource: "draw" }));
    }

    let chosen =
        darkhold_db::recommendation_slots::mark_chosen(state.db(), viewer, &body.request_id, &body.work_id)
            .await
            .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    // The response names the card the HAND recorded, not the one that was posted:
    // a second tap on another card is a no-op, and answering with the posted id
    // would tell the page it changed a choice the server had already refused.
    match chosen {
        Some((chosen_work_id, chosen_at)) => Ok(Json(serde_json::json!({
            "work_id": chosen_work_id,
            "chosen_at": chosen_at,
        }))),
        None => Err(ApiError(AppError::NotFound { resource: "draw" })),
    }
}
