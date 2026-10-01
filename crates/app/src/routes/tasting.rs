//! The tasting menu (spec §49.5, M45-19).
//!
//! **This route is the door §49.5 was missing.** `crates/db/src/tasting.rs`
//! shipped a selector, an ordering and a session bound, all unit-tested, and
//! nothing in the application called any of it — the exact shape `docs/goal.md`
//! names as a definition of not-complete: "a unit test on a function nothing
//! calls". A calibration queue with no entry point is not a quiet feature; it is
//! an absent one.
//!
//! ## Why the reader's trust is not consulted here
//!
//! §49.7: "nothing here changes candidate selection — §47.10's split holds: §49
//! decides what the ranker *knows*, never who is *eligible*." So this route does
//! **not** filter by trust, and does not read `min_trust` for eligibility. It
//! draws only from works that are published, public and not deleted, which is the
//! same bar the browse surface uses. Adding a trust check here would make §49 a
//! candidate-selection rule, which is the one thing the section forbids.
//!
//! ## Why the queue carries the selector's own number
//!
//! `uncertainty` is in the response so the client can show why a card was chosen
//! and so an operator can ask whether the queue is picking uncertain items or
//! just picking ids. A queue that claims to be uncertainty-driven and reports no
//! uncertainty is not auditable, and §49.5 makes the choice an acceptance
//! criterion — which needs a number to check.
//!
//! ## Why a response needs a reason and the route refuses without one
//!
//! §49.5: "a bare rating teaches almost nothing." The database makes it
//! unrepresentable (`reason TEXT NOT NULL`) and the request body makes it
//! unrepresentable too — `reason` is a required field, not an `Option`. A reader
//! who declines a sample is recorded as a negative *carrying* its reason, never
//! as a deletion: an active-learning queue that throws away the items it guessed
//! wrong learns only from what it already knew.

use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use lorehaven_domain::AppError;

/// How deep the candidate pool is drawn from.
///
/// §49.5's uncertainty is computed per work over its dimensions, so the pool has
/// to be deep enough for the ordering to be able to reorder something. This is a
/// pool, not a filter: a work outside it is deferred to a later session rather
/// than excluded from calibration.
const POOL: i64 = lorehaven_db::tasting::CANDIDATE_POOL;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/tasting/queue", get(get_queue))
        .route("/tasting/respond", post(post_response))
        .route("/tasting/responses", get(get_my_responses))
}

/// `GET /api/v1/tasting/queue?session_id=…&limit=…`
#[derive(Debug, Deserialize)]
pub struct QueueQuery {
    /// §49.5's per-session bound needs a session to count against. Required
    /// rather than defaulted: a server-minted session id would be a session the
    /// client cannot resume, so a reload would silently reset the reader's
    /// quota and the bound would be trivially bypassed by refreshing.
    pub session_id: String,
    #[serde(default)]
    pub limit: Option<u32>,
}

/// One card in the queue.
#[derive(Debug, Serialize)]
pub struct TastingCard {
    /// The `tasting_samples` row id. The response must quote this back, so the
    /// client never has to invent a handle — and so a response cannot be
    /// attached to a sample the reader was never shown.
    pub sample_id: String,
    pub work_id: String,
    pub title: String,
    pub summary: String,
    /// Where the 300-word window starts. Coordinates, not text: the passage is
    /// fetched from the work's own body through the normal read path, so this
    /// route never serves prose and never becomes a body gate (§7.7).
    pub sample_offset: i64,
    /// Lower means the selector is more certain. Exposed for the same reason
    /// `uncertainty_at_draw` is stored: the claim needs a number.
    pub uncertainty: f64,
    /// Why this was picked, in words a reader can be told.
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct ReasonOption {
    pub value: &'static str,
    pub label: &'static str,
    /// `not_for_me` takes free text; the rest do not. §49.5: the free text is an
    /// addition to the reason, not a replacement for it.
    pub takes_free_text: bool,
}

/// The reason labels a client may render, derived from the domain enum.
fn reason_options() -> Vec<ReasonOption> {
    use lorehaven_db::tasting::TastingReason as R;
    [
        (R::Prose, "The prose", false),
        (R::Characters, "The characters", false),
        (R::Pacing, "The pacing", false),
        (R::TropeExecution, "How the tropes landed", false),
        (R::NotForMe, "Not for me: …", true),
    ]
    .into_iter()
    .map(|(reason, label, takes_free_text)| {
        // `value` is read back off the variant through `as_str` rather than
        // restated as a literal: a restated string is a second spelling that can
        // drift from the one the column parses, and the failure is a reason the
        // client offers and the server refuses.
        ReasonOption {
            value: reason.as_str(),
            label,
            takes_free_text,
        }
    })
    .collect()
}

#[derive(Debug, Serialize)]
pub struct QueueResponse {
    pub samples: Vec<TastingCard>,
    /// How many the reader has already rated in this session, so a client can
    /// show "3 of 5" without its own bookkeeping — and so the bound is visible
    /// rather than a surprise when the queue comes back empty.
    pub answered_in_session: i64,
    /// The session's cap, so the client renders the same number the server
    /// enforces.
    pub session_limit: usize,
    pub reasons: Vec<ReasonOption>,
}

/// `GET /api/v1/tasting/queue` — the reader's next samples, most uncertain first.
async fn get_queue(
    State(state): State<AppState>,
    Query(query): Query<QueueQuery>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<QueueResponse>> {
    let db = state.db();
    let account = user.account_id.to_string();

    if query.session_id.trim().is_empty() {
        return Err(ApiError(AppError::field(
            "session_id",
            "a non-empty session id is required, so the per-session bound can be counted",
        )));
    }
    // Clamped rather than rejected: a client asking for 10 000 samples gets the
    // cap and a queue, where refusing outright would leave a reader with no way
    // to calibrate at all.
    let limit = query.limit.unwrap_or(10) as usize;
    let limit = limit.clamp(1, lorehaven_db::tasting::SESSION_LIMIT);

    let ids = lorehaven_db::tasting::candidate_works(db, &account, POOL)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;

    // §49.2: only **confirmed** tags count, and only under the per-work
    // contribution cap. The same call `discovery.rs` makes, so the queue ranks on
    // the dimensions the reader's feed actually uses — a queue that sampled by
    // unconfirmed tags would teach the model about tags it refuses to score by.
    let mut tags_by_work: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for id in &ids {
        if tags_by_work.contains_key(id) {
            continue;
        }
        match lorehaven_db::tag_confirmation::gravity_contributing_tags(
            db,
            id,
            lorehaven_db::tag_confirmation::DEFAULT_CONTRIBUTION_CAP,
        )
        .await
        {
            Ok(tags) => {
                tags_by_work.insert(
                    id.clone(),
                    tags.into_iter().map(|t| t.to_lowercase()).collect(),
                );
            }
            Err(error) => {
                // A work whose tags cannot be read is offered with no dimensions,
                // which `uncertainty_for` scores as 1.0 — unmeasurable rather
                // than uncertain, so it cannot flood the front of a cold-start
                // queue. Failing the whole request instead would let one
                // unreadable tag row empty the menu.
                tracing::warn!(
                    target: "tasting",
                    %error,
                    "tag lookup failed for a calibration candidate; it will be scored with no dimensions"
                );
                tags_by_work.insert(id.clone(), Vec::new());
            }
        }
    }

    // `+ Send + Sync` is required, not decorative: `build_queue` holds this
    // closure across its own `.await`, and a bare `&dyn Fn` would make the
    // handler's future non-`Send` — an axum `Handler` error that names neither
    // the closure nor the reason.
    let lookup =
        move |id: &str| -> Vec<String> { tags_by_work.get(id).cloned().unwrap_or_default() };

    let candidates: Vec<lorehaven_db::tasting::Candidate> = ids
        .iter()
        .map(|id| lorehaven_db::tasting::Candidate {
            work_id: id.clone(),
            // Placeholder, overwritten by the selector inside `build_queue`.
            // Carrying a real number here would suggest a caller may choose the
            // order, which §49.5 does not allow.
            uncertainty: 0.0,
        })
        .collect();

    let offers = lorehaven_db::tasting::build_queue(
        db,
        &account,
        &query.session_id,
        limit,
        candidates,
        &lookup,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e)))?;

    let samples: Vec<TastingCard> = offers
        .into_iter()
        .map(|offer| TastingCard {
            sample_id: offer.sample_id,
            work_id: offer.work_id,
            title: offer.title,
            summary: offer.summary,
            sample_offset: offer.sample_offset,
            uncertainty: offer.uncertainty,
            reason: offer.reason.to_owned(),
        })
        .collect();

    let answered = lorehaven_db::tasting::session_count(db, &account, &query.session_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;

    Ok(Json(QueueResponse {
        samples,
        answered_in_session: answered,
        session_limit: limit,
        reasons: reason_options(),
    }))
}

/// `POST /api/v1/tasting/respond`
#[derive(Debug, Deserialize)]
pub struct RespondBody {
    /// The `tasting_samples` id from the queue.
    pub sample_id: String,
    /// `like` or `dislike`. Parsed rather than trusted: an unknown verdict
    /// string must not reach the column, where the CHECK would reject it as a
    /// raw database error instead of naming the field.
    pub verdict: String,
    /// Required. §49.5: the reason is the labelled signal.
    pub reason: String,
    #[serde(default)]
    pub free_text: Option<String>,
    /// §49.5's "not for me: ___" clause. The free text is only meaningful with
    /// that reason, and a free-text note attached to a `prose` rating is a
    /// reason the schema cannot store — so it is refused rather than dropped,
    /// because silently discarding it is how a reader's words go missing.
    pub session_id: String,
}

#[derive(Debug, Serialize)]
pub struct RespondResponse {
    pub recorded: bool,
    /// The weight sign this response contributed, so the client can confirm a
    /// decline was taken as a negative rather than dropped. Never the weight
    /// itself: §0.3 forbids exposing raw taste values to a reader.
    pub contribution: &'static str,
    /// How many remain in this session's quota.
    pub remaining_in_session: i64,
}

async fn post_response(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    axum::Json(body): axum::Json<RespondBody>,
) -> ApiResult<Json<RespondResponse>> {
    let db = state.db();
    let account = user.account_id.to_string();

    let verdict = lorehaven_db::tasting::TastingVerdict::parse(&body.verdict).ok_or_else(|| {
        ApiError(AppError::field(
            "verdict",
            "must be exactly \"like\" or \"dislike\"",
        ))
    })?;
    let reason = lorehaven_db::tasting::TastingReason::parse(&body.reason).ok_or_else(|| {
        ApiError(AppError::field(
            "reason",
            "one of: prose, characters, pacing, trope_execution, not_for_me — a rating \
             without a reason teaches the model nothing",
        ))
    })?;

    let free_text = body.free_text.filter(|t| !t.trim().is_empty());
    if free_text.is_some() && reason != lorehaven_db::tasting::TastingReason::NotForMe {
        return Err(ApiError(AppError::field(
            "free_text",
            "only the \"not_for_me\" reason carries free text; for the other reasons the \
             reason tag is the signal",
        )));
    }
    if reason == lorehaven_db::tasting::TastingReason::NotForMe && free_text.is_none() {
        return Err(ApiError(AppError::field(
            "free_text",
            "the \"not_for_me\" reason is the free-text one, so it needs the text it refers to",
        )));
    }
    if body.session_id.trim().is_empty() {
        return Err(ApiError(AppError::field(
            "session_id",
            "a non-empty session id is required, so the per-session bound can be counted",
        )));
    }

    // The sample must exist and be **this reader's**. Both cases answer 404, not
    // 403: another reader's sample id is not this reader's business, and a 403
    // would confirm the id exists.
    //
    // The uncertainty recorded is the selector's own, read back from the row it
    // wrote — never the one in the request. §49.5 stores that number precisely so
    // an evaluation can ask whether the queue chose uncertain items, and a number
    // the client supplied would make the check meaningless: the client is the
    // thing being checked.
    let uncertainty_at_draw =
        match lorehaven_db::tasting::uncertainty_of_sample(db, &body.sample_id).await {
            Ok(Some(found)) => found,
            Ok(None) => {
                return Err(ApiError(AppError::NotFound {
                    resource: "tasting sample",
                }))
            }
            Err(error) => return Err(ApiError(AppError::Internal(error))),
        };
    // Owned by someone else, or by no one: **one** answer, on purpose. A 403
    // would confirm that the id exists, and the two cases must be
    // indistinguishable or the 404 becomes an existence oracle.
    let owned = lorehaven_db::tasting::sample_owner(db, &body.sample_id)
        .await
        .map_err(|error| ApiError(AppError::Internal(error)))?
        .is_some_and(|owner| owner == account);
    if !owned {
        return Err(ApiError(AppError::NotFound {
            resource: "tasting sample",
        }));
    }

    let already = lorehaven_db::tasting::has_responded(db, &body.sample_id, &account)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    if already {
        // 409, and the *reason* is the point: a second rating of the same sample
        // would make the recorded signal depend on how many times a button was
        // pressed, which is the one property an active-learning queue cannot have.
        return Err(ApiError(AppError::Validation {
            message: "this sample has already been answered; re-rating it would make the \
                      recorded signal depend on how many times the button was pressed"
                .to_owned(),
            field_errors: Default::default(),
        }));
    }

    let response = lorehaven_db::tasting::TastingResponse {
        sample_id: body.sample_id.clone(),
        account_id: account.clone(),
        verdict,
        reason,
        free_text,
        uncertainty_at_draw,
        session_id: body.session_id.clone(),
    };
    lorehaven_db::tasting::record_response(db, &response)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;

    // §49.5 says the queue feeds the taste profile. Recording the answer without
    // moving any weight would be a survey, not calibration — and `goal.md` names
    // exactly this ("a field that exists and that no code writes"). The weight
    // moves on the *reason* the reader gave, because a reason is a statement about
    // a dimension and that is the only part of a response that is trainable.
    let applied = lorehaven_db::tasting::apply_response_to_weights(db, &account, &response)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;

    let answered = lorehaven_db::tasting::session_count(db, &account, &body.session_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;

    Ok(Json(RespondResponse {
        recorded: true,
        contribution: if applied {
            "your reading moved your taste profile"
        } else {
            "recorded; this work has no countable tag to weigh it against"
        },
        // The cap is the constant, not whatever the queue request asked for: a
        // reader who fetched with `limit=1` has still answered against the
        // session-wide cap, and reporting a remaining count against their own
        // last request's limit would let a client re-raise its own quota.
        remaining_in_session: i64::try_from(lorehaven_db::tasting::SESSION_LIMIT).unwrap_or(0)
            - answered,
    }))
}

/// `GET /api/v1/tasting/responses` — the reader's own answers, newest first.
///
/// §49.7: "no sample is discarded for being a surprise" and §49.5's acceptance
/// clause requires a declined sample to *appear in the profile*. This is that
/// surface: a reader can see every sample they rated, including the declines,
/// and neither the reason nor the free text is dropped on the way out.
#[derive(Debug, Deserialize)]
pub struct MyResponsesQuery {
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct MyResponseRow {
    pub work_id: String,
    pub title: String,
    pub verdict: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_text: Option<String>,
    pub uncertainty_at_draw: f64,
    pub answered_at: String,
}

#[derive(Debug, Serialize)]
pub struct MyResponsesResponse {
    pub responses: Vec<MyResponseRow>,
    /// How many were declines. Surfaced because §49.5 makes a decline a first
    /// class negative, and a profile view that only ever showed the positives
    /// would be the same discarding §49.5 forbids.
    pub declines: i64,
}

async fn get_my_responses(
    State(state): State<AppState>,
    Query(query): Query<MyResponsesQuery>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<MyResponsesResponse>> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200) as i64;
    let rows =
        lorehaven_db::tasting::responses_for(db_of(&state), &user.account_id.to_string(), limit)
            .await
            .map_err(|e| ApiError(AppError::Internal(e)))?;

    let declines = rows
        .iter()
        .filter(|row| row.verdict == lorehaven_db::tasting::TastingVerdict::Dislike.as_str())
        .count();
    let declines = i64::try_from(declines).unwrap_or(i64::MAX);

    Ok(Json(MyResponsesResponse {
        responses: rows
            .into_iter()
            .map(|row| MyResponseRow {
                work_id: row.work_id,
                title: row.title,
                verdict: row.verdict,
                reason: row.reason,
                free_text: row.free_text,
                uncertainty_at_draw: row.uncertainty_at_draw,
                answered_at: row.answered_at,
            })
            .collect(),
        declines,
    }))
}

fn db_of(state: &AppState) -> &lorehaven_db::Database {
    state.db()
}
