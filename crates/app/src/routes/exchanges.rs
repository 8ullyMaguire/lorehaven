//! §18.11 — gift exchanges, through the doors a participant and an organiser use (M45-27).
//!
//! The pairing itself is [`darkhold_domain::exchanges`] (pure) and the record is
//! [`darkhold_db::exchanges`]; this file is only the translation between them and HTTP.
//!
//! ## Participants are pseuds, because that is what §18 already stores
//!
//! `events.created_by` and `event_participation.account` are both written with a **pseud
//! id** (`events.rs`: `post_event` and `join_event` pass `pseud_id`). The column is named
//! `account`, and calling that a naming bug does not change what is in it. So an exchange
//! pairs pseuds, and the door that decides "may I run the matching" compares the caller's
//! pseud with `events.created_by`.
//!
//! ## `mine` answers before reveal, it does not refuse
//!
//! A participant who has been paired but not told gets `{recipient: null, revealed: false,
//! own terms}` — the honest answer to "whom am I writing for", which is *you have not been
//! told yet*. A `403` would say they did something wrong.

use axum::extract::{Path, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use darkhold_domain::exchanges::may_see_recipient;
use darkhold_domain::AppError;

use crate::auth::{RequirePseud, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/exchanges/{event}", get(get_exchange))
        .route("/exchanges/{event}/join", post(post_join))
        .route("/exchanges/{event}/terms", put(put_terms))
        .route("/exchanges/{event}/match", post(post_match))
        .route("/exchanges/{event}/reveal", post(post_reveal))
        .route("/exchanges/{event}/mine", get(get_mine))
}

#[derive(Debug, Deserialize, Default)]
struct TermsBody {
    #[serde(default)]
    offers: Vec<String>,
    #[serde(default)]
    wants: Vec<String>,
    #[serde(default)]
    excludes: Vec<String>,
}

fn terms_json(t: &darkhold_db::exchanges::Terms) -> Value {
    json!({ "offers": t.offers, "wants": t.wants, "excludes": t.excludes })
}

fn assignment_json(a: &darkhold_db::exchanges::Assignment) -> Value {
    json!({
        "giver": a.giver,
        "recipient": a.recipient,
        "assigned_at": a.assigned_at,
        "revealed_at": a.revealed_at,
        "matched_by": a.matched_by,
    })
}

/// Whether `pseud_id` organises this event. `created_by` is the pseud that created it.
async fn is_organiser(state: &AppState, event_id: &str, pseud_id: &str) -> Result<bool, ApiError> {
    let event = darkhold_db::events::get_event(state.db(), event_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    Ok(event.map(|e| e.created_by == pseud_id).unwrap_or(false))
}

async fn get_exchange(
    State(state): State<AppState>,
    Path(event_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let event = darkhold_db::events::get_event(state.db(), &event_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    if event.is_none() {
        return Err(ApiError(AppError::NotFound {
            resource: "event",
        }));
    }
    let summary = darkhold_db::exchanges::summary(state.db(), &event_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    // Counts only: who is paired with whom is not in this payload even when revealed,
    // because a public page that lists every pairing turns a gift exchange into a list.
    Ok(Json(json!({
        "event_id": event_id,
        "participants": summary.participants,
        "matched": summary.matched,
        "revealed": summary.revealed,
    })))
}

async fn post_join(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(event_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let event = darkhold_db::events::get_event(state.db(), &event_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    if event.is_none() {
        return Err(ApiError(AppError::NotFound {
            resource: "event",
        }));
    }
    let me = pseud_id.to_string();
    let joined = darkhold_db::events::join_event(state.db(), &event_id, &me)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    let terms = darkhold_db::exchanges::terms(state.db(), &event_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?
        .into_iter()
        .find(|t| t.account == me);
    // Joining twice is not an error: the answer is the same participation, and saying so
    // is what lets a client call this on every page load.
    Ok(Json(json!({
        "joined": true,
        "new": joined,
        "terms": terms.as_ref().map(terms_json),
    })))
}

async fn put_terms(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(event_id): Path<String>,
    Json(body): Json<TermsBody>,
) -> ApiResult<Json<Value>> {
    let me = pseud_id.to_string();
    match darkhold_db::exchanges::save_terms(
        state.db(),
        &event_id,
        &me,
        &body.offers,
        &body.wants,
        &body.excludes,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e)))?
    {
        darkhold_db::exchanges::SaveTerms::Saved(terms) => Ok(Json(json!({
            "saved": true,
            "terms": terms_json(&terms),
        }))),
        darkhold_db::exchanges::SaveTerms::NotAParticipant => Err(ApiError(AppError::field(
            "event_id",
            "join the exchange before declaring terms",
        ))),
        darkhold_db::exchanges::SaveTerms::SelfExclusion => Err(ApiError(AppError::field(
            "excludes",
            "you cannot exclude yourself: the matcher never pairs a participant with themselves",
        ))),
    }
}

async fn post_match(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(event_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let me = pseud_id.to_string();
    if !is_organiser(&state, &event_id, &me).await? {
        // 403 is the right status and `AccessDenied` is the only variant that carries it;
        // the vocabulary has no 403-with-a-message, and inventing a variant to say "you are
        // not the organiser" would change the error contract for one sentence.
        return Err(ApiError(AppError::AccessDenied));
    }
    match darkhold_db::exchanges::run_match(state.db(), &event_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?
    {
        darkhold_db::exchanges::MatchOutcome::Matched {
            assignments,
            method,
        } => Ok(Json(json!({
            "matched": assignments.len(),
            "method": method.as_str(),
            "assignments": assignments.iter().map(assignment_json).collect::<Vec<_>>(),
        }))),
        darkhold_db::exchanges::MatchOutcome::AlreadyMatched { assigned_at } => {
            // A precondition, answered the way `join_event` answers one: 422 naming the
            // field. 409 would read better and the vocabulary has no generic Conflict, so
            // the sentence carries the meaning and the status stays consistent with its
            // neighbour rather than inventing a code.
            Err(ApiError(AppError::field(
                "event_id",
                format!("this exchange was already matched at {assigned_at}; the pairing is the record"),
            )))
        }
        darkhold_db::exchanges::MatchOutcome::Impossible { unmatched } => {
            // Naming the people is the point: a partial pairing would promise gifts the
            // exchange cannot keep, so nothing was written.
            Err(ApiError(AppError::field(
                "participants",
                format!(
                    "no pairing exists that places everybody; unmatched: {}",
                    unmatched.join(", ")
                ),
            )))
        }
        darkhold_db::exchanges::MatchOutcome::TooFew { participants } => Err(ApiError(
            AppError::field(
                "participants",
                format!("an exchange needs at least two participants, this one has {participants}"),
            ),
        )),
        darkhold_db::exchanges::MatchOutcome::NoEvent => Err(ApiError(AppError::NotFound {
            resource: "event",
        })),
    }
}

async fn post_reveal(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(event_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let me = pseud_id.to_string();
    if !is_organiser(&state, &event_id, &me).await? {
        return Err(ApiError(AppError::AccessDenied));
    }
    match darkhold_db::exchanges::reveal(state.db(), &event_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?
    {
        darkhold_db::exchanges::Reveal::Revealed { revealed_at } => {
            Ok(Json(json!({ "revealed": true, "revealed_at": revealed_at })))
        }
        darkhold_db::exchanges::Reveal::NotMatched => Err(ApiError(AppError::field(
            "event_id",
            "there is nothing to reveal: the matching has not been run",
        ))),
        darkhold_db::exchanges::Reveal::NoEvent => Err(ApiError(AppError::NotFound {
            resource: "event",
        })),
    }
}

async fn get_mine(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(event_id): Path<String>,
) -> ApiResult<Json<Value>> {
    // The pseud is the participant, and a session without one cannot be in an exchange.
    let Some(pseud) = user.pseud_id else {
        return Err(ApiError(AppError::field(
            "pseud",
            "choose a pseudonym before joining an exchange",
        )));
    };
    let me = pseud.to_string();
    let terms = darkhold_db::exchanges::terms(state.db(), &event_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?
        .into_iter()
        .find(|t| t.account == me);
    let assignment = darkhold_db::exchanges::mine(state.db(), &event_id, &me)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;

    match assignment {
        None => Ok(Json(json!({
            "joined": terms.is_some(),
            "terms": terms.as_ref().map(terms_json),
            "recipient": Value::Null,
            "revealed": false,
            "reason": "this pseud is not in the pairing for this exchange",
        }))),
        Some(a) => {
            let revealed = a.revealed_at.is_some();
            let may_see = may_see_recipient(revealed, true);
            Ok(Json(json!({
                "joined": true,
                "terms": terms.as_ref().map(terms_json),
                "recipient": if may_see { json!(a.recipient) } else { Value::Null },
                "revealed": revealed,
                "assigned_at": a.assigned_at,
                "reason": if may_see {
                    Value::Null
                } else {
                    json!("the pairing is secret until the organiser reveals it")
                },
            })))
        }
    }
}
