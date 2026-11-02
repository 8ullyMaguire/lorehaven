//! §56 — WIP adoption, closure notes and lineage (M45-28).
//!
//! ```text
//! POST   /api/v1/works/{id}/adoption-offer      work owner:  offer an abandoned WIP
//! DELETE /api/v1/works/{id}/adoption-offer      work owner:  withdraw, no notice period
//! GET    /api/v1/adoption-offers                anyone:      what is on offer
//! POST   /api/v1/adoption-offers/{id}/adopt     reader:      adopt, gated like publishing
//! GET    /api/v1/works/{id}/contributors        anyone:      lineage, derived from revisions
//! POST   /api/v1/works/{id}/closure-note        work owner:  why it ends this way
//! GET    /api/v1/works/{id}/closure-note        anyone:      the latest note, or 404
//! ```
//!
//! # What none of these doors can do
//!
//! Move ownership. §56.1 says adoption "does not transfer ownership, does not rename the author,
//! and does not touch the work's `owner_pseud_id`", and the way that is true is that no handler
//! here mentions the column. The `POST …/adopt` door writes one row in `adoption_offers`; the
//! adopter appears in `contributors` because they wrote revisions, not because a flag says so.
//!
//! # Why `contributors` answers 200 with `available: false`
//!
//! §56.3 derives lineage from `chapter_revisions`, and that table is absent from the live
//! instance's schema (M63-33). A `500` about a missing relation is the one answer that tells a
//! reader nothing, so the door answers with the list it has — empty — and a reason. A client can
//! render "lineage is unavailable here" without a special case for a server fault.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use darkhold_db::wip_adoption::{self, ClosureNote, Lineage, Offer};
use darkhold_domain::wip_adoption::{AdoptionRefusal, OpenRefusal, WithdrawalRefusal};
use darkhold_domain::AppError;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/works/{id}/adoption-offer",
            post(open_offer).delete(withdraw_offer),
        )
        .route("/adoption-offers", get(list_offers))
        .route("/adoption-offers/{id}/adopt", post(adopt_offer))
        .route("/works/{id}/contributors", get(contributors))
        .route(
            "/works/{id}/closure-note",
            post(put_closure_note).get(get_closure_note),
        )
}

// ---------------------------------------------------------------- offers

#[derive(Debug, Serialize)]
struct OfferResponse {
    offer: Offer,
}

async fn open_offer(
    RequireSession(user): RequireSession,
    State(state): State<AppState>,
    Path(work_id): Path<String>,
) -> ApiResult<Json<OfferResponse>> {
    let pseud = user.pseud_id.ok_or_else(|| {
        ApiError(AppError::field(
            "pseud",
            "an offer is made by the pseud that owns the work: choose one first",
        ))
    })?;
    match wip_adoption::open_offer(state.db(), &work_id, &pseud.to_string())
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?
    {
        Ok(offer) => Ok(Json(OfferResponse { offer })),
        Err(OpenRefusal::NotOwner) => Err(ApiError(AppError::NotFound { resource: "work" })),
        Err(OpenRefusal::NotAbandoned) => Err(ApiError(AppError::field(
            "completion",
            "only a work marked abandoned may be offered: a hiatus is an author who means to come \
             back, and a complete work has nothing to continue",
        ))),
        Err(OpenRefusal::AlreadyOpen) => Err(ApiError(AppError::field(
            "status",
            "this work already has an open offer; one question, one answer",
        ))),
    }
}

async fn withdraw_offer(
    RequireSession(user): RequireSession,
    State(state): State<AppState>,
    Path(work_id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let pseud = user.pseud_id.ok_or_else(|| {
        ApiError(AppError::field("pseud", "withdrawing an offer needs a pseud"))
    })?;
    let pseud = pseud.to_string();
    let Some(offer) = wip_adoption::open_offer_for(state.db(), &work_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?
    else {
        return Err(ApiError(AppError::NotFound {
            resource: "open adoption offer",
        }));
    };
    match wip_adoption::withdraw(state.db(), &offer.id, &pseud)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?
    {
        Ok(offer) => Ok(Json(serde_json::json!({
            "withdrawn": true,
            "offer": offer,
        }))),
        Err(WithdrawalRefusal::NotTheAuthor) => Err(ApiError(AppError::AccessDenied)),
        Err(WithdrawalRefusal::NotOpen) => Err(ApiError(AppError::NotFound {
            resource: "open adoption offer",
        })),
    }
}

async fn list_offers(State(state): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    let offers = wip_adoption::list_open(state.db(), 100)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(serde_json::json!({
        "offers": offers,
        "count": offers.len(),
    })))
}

async fn adopt_offer(
    RequireSession(user): RequireSession,
    State(state): State<AppState>,
    Path(offer_id): Path<String>,
) -> ApiResult<Json<OfferResponse>> {
    let pseud = user.pseud_id.ok_or_else(|| {
        ApiError(AppError::field(
            "pseud",
            "adopting is a publishing act, and publishing is done by a pseud: choose one first",
        ))
    })?;
    let bar = state.config().adoption.min_trust_level;
    let caller_trust = darkhold_db::governance::trust_for(state.db(), &user.account_id.to_string())
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    match wip_adoption::adopt(
        state.db(),
        &offer_id,
        &pseud.to_string(),
        bar,
        caller_trust,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e)))?
    {
        Ok(offer) => Ok(Json(OfferResponse { offer })),
        Err(AdoptionRefusal::NotOpen) => Err(ApiError(AppError::NotFound {
            resource: "open adoption offer",
        })),
        Err(AdoptionRefusal::NotAbandoned) => Err(ApiError(AppError::field(
            "completion",
            "the work is no longer marked abandoned, so there is nothing to adopt",
        ))),
        Err(AdoptionRefusal::BelowBar { bar, caller }) => Err(ApiError(AppError::field(
            "trust",
            &format!(
                "adoption is gated at the publishing trust level: this instance requires {bar}, \
                 and this account is at {caller}"
            ),
        ))),
    }
}

// ---------------------------------------------------------------- lineage

async fn contributors(
    State(state): State<AppState>,
    Path(work_id): Path<String>,
) -> ApiResult<Json<Lineage>> {
    let lineage = wip_adoption::contributors(state.db(), &work_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(lineage))
}

// ---------------------------------------------------------------- closure notes

#[derive(Debug, Deserialize)]
struct NoteBody {
    body: String,
}

async fn put_closure_note(
    RequireSession(user): RequireSession,
    State(state): State<AppState>,
    Path(work_id): Path<String>,
    Json(body): Json<NoteBody>,
) -> ApiResult<Json<ClosureNote>> {
    let pseud = user.pseud_id.ok_or_else(|| {
        ApiError(AppError::field(
            "pseud",
            "a closure note is credited to a pseud: choose one first",
        ))
    })?;
    let note = body.body.trim();
    if note.is_empty() {
        return Err(ApiError(AppError::field(
            "body",
            "a closure note needs a sentence in it: an empty note credits nobody",
        )));
    }
    let written = wip_adoption::put_closure_note(state.db(), &work_id, &pseud.to_string(), note)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(written))
}

async fn get_closure_note(
    State(state): State<AppState>,
    Path(work_id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    match wip_adoption::latest_closure_note(state.db(), &work_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?
    {
        Some(note) => Ok(Json(serde_json::json!({ "note": note }))),
        None => Err(ApiError(AppError::NotFound {
            resource: "closure note",
        })),
    }
}
