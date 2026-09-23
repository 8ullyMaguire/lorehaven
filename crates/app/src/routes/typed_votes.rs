//! Typed votes, vote budgets, meta-moderation and karma (spec §35.2, repo M32).
//!
//! The API the spec names:
//!
//! ```text
//! POST   /forum/posts/{id}/vote            cast or change a typed vote
//! DELETE /forum/posts/{id}/vote            retract it
//! GET    /forum/posts/{id}/votes           counts, and names per transparency tier
//! POST   /forum/votes/{id}/meta            a TL4+ steward's verdict (spec §35.2)
//! GET    /me/vote-budget                   the rolling-window allowance
//! GET    /forum/karma                      the caller's own karma
//! GET    /forum/karma/{pseud}              a public profile's karma
//! ```
//!
//! Two surfaces the spec names in prose rather than in its endpoint list are
//! served here too, because a client cannot use the feature without them:
//! `GET /forum/categories/{id}/vote-types` (the taxonomy is *data*, so it has to
//! be readable) and `PUT /forum/posts/{id}/vote-visibility` (the post author's
//! opt-in to revealing who voted).
//!
//! Every refusal a voter can act on — budget exhausted, a type this category
//! does not offer, the wrong trust level for meta-moderation — is a
//! `VALIDATION_FAILED` or `ACCESS_DENIED` with a sentence, never a silent drop.

use std::collections::BTreeMap;

use axum::extract::{Path, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use time::OffsetDateTime;

use lorehaven_domain::typed_votes::{
    budget_limit_for, can_meta_moderate, find_vote_type, is_moderator, vote_transparency,
    VoteBudget, VoteTransparency,
};

use crate::auth::{RequirePseud, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        // The typed vote itself: one per pseud, changeable, retractable.
        .route(
            "/forum/posts/{id}/vote",
            post(cast_vote).delete(retract_vote),
        )
        .route("/forum/posts/{id}/votes", get(get_post_votes))
        // The author's opt-in to revealing who voted (spec §35.2 tiers).
        .route(
            "/forum/posts/{id}/vote-visibility",
            put(put_vote_visibility),
        )
        // Meta-moderation: a TL4+ verdict on someone's vote.
        .route("/forum/votes/{id}/meta", post(post_meta_vote))
        // The taxonomy is data, so it is readable (spec §35.2).
        .route(
            "/forum/categories/{id}/vote-types",
            get(get_category_vote_types),
        )
        // The caller's own allowance, and karma (own and public).
        .route("/me/vote-budget", get(get_vote_budget))
        .route("/forum/karma", get(get_own_karma))
        .route("/forum/karma/{pseud}", get(get_karma))
}

fn internal(e: anyhow::Error) -> ApiError {
    ApiError(lorehaven_domain::AppError::Internal(e))
}

fn internal_sql(e: sqlx::Error) -> ApiError {
    ApiError(lorehaven_domain::AppError::Internal(e.into()))
}

/// Refuse with a sentence a voter can act on.
fn refuse(message: String) -> ApiError {
    ApiError(lorehaven_domain::AppError::Validation {
        message,
        field_errors: BTreeMap::new(),
    })
}

/// The caller's allowance and what the rolling window has consumed.
async fn budget_for(state: &AppState, account_id: &str) -> ApiResult<VoteBudget> {
    let trust = lorehaven_db::governance::trust_for(state.db(), account_id)
        .await
        .map_err(internal_sql)?;
    let limit = budget_limit_for(&state.config().forum.vote_budget, trust);
    let start = lorehaven_db::identity::format_rfc3339(
        lorehaven_domain::typed_votes::window_start(OffsetDateTime::now_utc()),
    );
    let spent = lorehaven_db::typed_votes::budget_spent(state.db(), account_id, &start)
        .await
        .map_err(internal)?;
    Ok(VoteBudget::new(limit, spent))
}

/// The taxonomy a category offers, so a client knows what it may cast.
async fn get_category_vote_types(
    State(state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let types = lorehaven_db::typed_votes::vote_types_for_category(state.db(), &id)
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "category_id": id, "items": types })))
}

/// The votes on a post: aggregates to anyone, names per the transparency tier.
async fn get_post_votes(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let post = lorehaven_db::typed_votes::post_context(state.db(), &id)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "post" }))?;

    let counts = lorehaven_db::typed_votes::vote_counts(state.db(), &id)
        .await
        .map_err(internal)?;
    let weighted_bp = lorehaven_db::typed_votes::weighted_total_bp(state.db(), &id)
        .await
        .map_err(internal)?;
    let total: i64 = counts.iter().map(|(_, count)| count).sum();

    let trust = lorehaven_db::governance::trust_for(state.db(), &user.account_id.to_string())
        .await
        .map_err(internal_sql)?;
    // "The post author" is the *account*, not the one pseud the session happens
    // to be acting as: an author with two pseuds is the same author.
    let viewer_is_author = lorehaven_db::identity::pseuds_for_account(state.db(), user.account_id)
        .await
        .map_err(internal)?
        .iter()
        .any(|pseud| pseud.id.to_string() == post.author_pseud);
    let author_opted_in = post.votes_visible != 0;
    let tier = vote_transparency(viewer_is_author, author_opted_in, is_moderator(trust));

    let mine = match user.pseud_id {
        Some(pseud_id) => {
            lorehaven_db::typed_votes::vote_for(state.db(), &id, &pseud_id.to_string())
                .await
                .map_err(internal)?
                .map(|vote| vote.vote_type)
        }
        None => None,
    };

    let votes = if tier == VoteTransparency::IndividualVotes {
        let rows = lorehaven_db::typed_votes::votes_on_post(state.db(), &id)
            .await
            .map_err(internal)?;
        Some(
            rows.iter()
                .map(|row| {
                    json!({
                        "id": row.id,
                        "pseud": row.pseud,
                        "vote_type": row.vote_type,
                        "created_at": row.created_at,
                    })
                })
                .collect::<Vec<_>>(),
        )
    } else {
        // The tier forbids it, so the key is absent rather than empty: a client
        // must not read "no names" as "no votes".
        None
    };

    let counts_json: Vec<Value> = counts
        .iter()
        .map(|(vote_type, count)| json!({ "vote_type": vote_type, "count": count }))
        .collect();

    Ok(Json(json!({
        "counts": counts_json,
        "total": total,
        "weighted_bp": weighted_bp,
        "transparency": tier.as_str(),
        "author_opted_in": author_opted_in,
        "mine": mine,
        "votes": votes,
    })))
}

/// A cast (or a change) of one typed vote.
#[derive(Debug, Deserialize)]
struct CastVoteBody {
    vote_type: String,
}

async fn cast_vote(
    State(state): State<AppState>,
    RequirePseud { user, pseud_id }: RequirePseud,
    Path(id): Path<String>,
    Json(body): Json<CastVoteBody>,
) -> ApiResult<Json<Value>> {
    let pseud = pseud_id.to_string();
    let account = user.account_id.to_string();

    let post = lorehaven_db::typed_votes::post_context(state.db(), &id)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "post" }))?;

    // The taxonomy is the category's: a Critique category that configures
    // `constructive | harsh_but_fair | needs_sources` refuses `insightful`
    // without any code change (spec §35.2).
    let taxonomy =
        lorehaven_db::typed_votes::vote_types_for_category(state.db(), &post.category_id)
            .await
            .map_err(internal)?;
    let vote_type = find_vote_type(&taxonomy, &body.vote_type)
        .cloned()
        .ok_or_else(|| {
            refuse(format!(
                "This category does not offer a {:?} vote.",
                body.vote_type
            ))
        })?;

    // Budget: the full cost for a new vote, only the difference for a change
    // (a change from a negative type to a positive one releases budget).
    let budget = budget_for(&state, &account).await?;
    let existing = lorehaven_db::typed_votes::vote_for(state.db(), &id, &pseud)
        .await
        .map_err(internal)?;
    let previous_cost = match &existing {
        Some(vote) => lorehaven_db::typed_votes::vote_type(state.db(), &vote.vote_type)
            .await
            .map_err(internal)?
            .map_or(0, |kind| kind.cost),
        None => 0,
    };
    let additional = (vote_type.cost - previous_cost).max(0);
    if !budget.can_afford(additional) {
        return Err(refuse(format!(
            "Your vote budget for this 24-hour window is spent ({} of {}, and this vote needs {}). \
             Unused votes do not roll over; they come back as votes leave the window.",
            budget.spent, budget.limit, additional
        )));
    }

    // Weight is read *now*, so a decayed caster's next vote counts less.
    let forum = &state.config().forum;
    let weight_bp = lorehaven_db::typed_votes::caster_weight_bp(
        state.db(),
        &pseud,
        forum.min_vote_weight_bp,
        forum.meta_mod_min_verdicts,
    )
    .await
    .map_err(internal)?;

    let now = OffsetDateTime::now_utc();
    let at = lorehaven_db::identity::format_rfc3339(now);
    lorehaven_db::typed_votes::upsert_vote(state.db(), &id, &pseud, &vote_type.id, weight_bp, &at)
        .await
        .map_err(internal)?;

    // Karma is the receiver's: swap this pseud's old contribution for the new
    // one. A change is a delta, not an addition, so repeated changes cannot
    // inflate it.
    let previous_weight = existing.as_ref().map_or(0, |vote| vote.weight_at_cast_bp);
    let delta = weight_bp - previous_weight;
    if delta != 0 {
        lorehaven_db::typed_votes::accrue_karma(
            state.db(),
            &post.author_pseud,
            delta,
            now,
            forum.karma_decay_percent,
        )
        .await
        .map_err(internal)?;
    }

    let spent_after = budget.spent + additional;
    let outcome = if existing.is_some() {
        "changed"
    } else {
        "cast"
    };
    Ok(Json(json!({
        "outcome": outcome,
        "vote_type": vote_type.id,
        "weight_bp": weight_bp,
        "budget": {
            "limit": budget.limit,
            "spent": spent_after,
            "remaining": (budget.limit - spent_after).max(0),
        },
    })))
}

/// Retract a vote. Idempotent: retracting nothing is a success that reports
/// `removed: false` rather than a 404 a client has to special-case.
async fn retract_vote(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let pseud = pseud_id.to_string();
    let removed = lorehaven_db::typed_votes::delete_vote(state.db(), &id, &pseud)
        .await
        .map_err(internal)?;

    if let Some(vote) = &removed {
        // The vote contributed its cast weight to the author's karma; taking
        // the vote back takes the contribution with it.
        if let Ok(Some(post)) = lorehaven_db::typed_votes::post_context(state.db(), &id).await {
            let _ = lorehaven_db::typed_votes::accrue_karma(
                state.db(),
                &post.author_pseud,
                -vote.weight_at_cast_bp,
                OffsetDateTime::now_utc(),
                state.config().forum.karma_decay_percent,
            )
            .await;
        }
    }

    Ok(Json(json!({
        "outcome": "retracted",
        "removed": removed.is_some(),
    })))
}

/// The post author's opt-in to revealing who voted (spec §35.2 tiers).
#[derive(Debug, Deserialize)]
struct VisibilityBody {
    visible: bool,
}

async fn put_vote_visibility(
    State(state): State<AppState>,
    RequirePseud { user, .. }: RequirePseud,
    Path(id): Path<String>,
    Json(body): Json<VisibilityBody>,
) -> ApiResult<Json<Value>> {
    let post = lorehaven_db::typed_votes::post_context(state.db(), &id)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "post" }))?;

    // Only the post's author may open or close their own vote record. Anyone
    // else gets the same 404 a missing post gets (spec §3.3).
    let is_author = lorehaven_db::identity::pseuds_for_account(state.db(), user.account_id)
        .await
        .map_err(internal)?
        .iter()
        .any(|pseud| pseud.id.to_string() == post.author_pseud);
    if !is_author {
        return Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "post",
        }));
    }

    let updated = lorehaven_db::typed_votes::set_votes_visible(state.db(), &id, body.visible)
        .await
        .map_err(internal)?;
    if !updated {
        return Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "post",
        }));
    }
    Ok(Json(json!({
        "post_id": id,
        "votes_visible": body.visible,
    })))
}

/// A steward's verdict on someone else's vote.
#[derive(Debug, Deserialize)]
struct MetaVoteBody {
    fair: bool,
}

async fn post_meta_vote(
    State(state): State<AppState>,
    RequirePseud { user, pseud_id }: RequirePseud,
    Path(id): Path<String>,
    Json(body): Json<MetaVoteBody>,
) -> ApiResult<Json<Value>> {
    let account = user.account_id.to_string();
    let steward = pseud_id.to_string();

    let trust = lorehaven_db::governance::trust_for(state.db(), &account)
        .await
        .map_err(internal_sql)?;
    if !can_meta_moderate(trust) {
        // Moderate the moderators: weigh the caster's *future* weight down,
        // never take away their voice. Whose verdict is allowed to do that is
        // the trust ladder's business, and here it says no.
        return Err(ApiError(lorehaven_domain::AppError::AccessDenied));
    }

    let vote = lorehaven_db::typed_votes::vote_by_id(state.db(), &id)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "vote" }))?;
    if vote.pseud == steward {
        return Err(refuse("You cannot meta-moderate your own vote.".to_owned()));
    }

    let forum = &state.config().forum;
    let start = lorehaven_db::identity::format_rfc3339(
        lorehaven_domain::typed_votes::window_start(OffsetDateTime::now_utc()),
    );
    let spent = lorehaven_db::typed_votes::meta_mods_cast(state.db(), &account, &start)
        .await
        .map_err(internal)?;
    let already = lorehaven_db::typed_votes::meta_vote_for(state.db(), &id, &steward)
        .await
        .map_err(internal)?;
    let additional = i64::from(already.is_none());
    if spent + additional > forum.meta_mod_points {
        return Err(refuse(format!(
            "You have spent all {} meta-mod points for this 24-hour window.",
            forum.meta_mod_points
        )));
    }

    let now = OffsetDateTime::now_utc();
    let at = lorehaven_db::identity::format_rfc3339(now);
    lorehaven_db::typed_votes::upsert_meta_vote(state.db(), &id, &steward, body.fair, &at)
        .await
        .map_err(internal)?;

    // The consequence is on the *caster's future* votes, never on the vote that
    // was flagged: history keeps the weight it was cast at.
    let weight_bp = lorehaven_db::typed_votes::caster_weight_bp(
        state.db(),
        &vote.pseud,
        forum.min_vote_weight_bp,
        forum.meta_mod_min_verdicts,
    )
    .await
    .map_err(internal)?;
    let (fair, unfair) = lorehaven_db::typed_votes::meta_verdicts(state.db(), &vote.pseud)
        .await
        .map_err(internal)?;

    Ok(Json(json!({
        "vote_id": id,
        "fair": body.fair,
        "caster": vote.pseud,
        "caster_weight_bp": weight_bp,
        "verdicts": { "fair": fair, "unfair": unfair },
        "meta_mod_points": {
            "limit": forum.meta_mod_points,
            "spent": spent + additional,
        },
    })))
}

/// The caller's rolling vote allowance, and when it starts filling again.
async fn get_vote_budget(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    let account = user.account_id.to_string();
    let budget = budget_for(&state, &account).await?;
    let trust = lorehaven_db::governance::trust_for(state.db(), &account)
        .await
        .map_err(internal_sql)?;
    let now = OffsetDateTime::now_utc();
    let start =
        lorehaven_db::identity::format_rfc3339(lorehaven_domain::typed_votes::window_start(now));
    let oldest = lorehaven_db::typed_votes::oldest_charge(state.db(), &account, &start)
        .await
        .map_err(internal)?;
    let resets_at = oldest.and_then(|at| {
        OffsetDateTime::parse(&at, &time::format_description::well_known::Rfc3339)
            .ok()
            .map(|charge| {
                lorehaven_db::identity::format_rfc3339(lorehaven_domain::typed_votes::window_end(
                    charge,
                ))
            })
    });

    Ok(Json(json!({
        "trust": trust,
        "window_hours": lorehaven_domain::typed_votes::BUDGET_WINDOW_HOURS,
        "limit": budget.limit,
        "spent": budget.spent,
        "remaining": budget.remaining(),
        "exhausted": budget.exhausted(),
        "resets_at": resets_at,
    })))
}

/// The caller's own karma.
async fn get_own_karma(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
) -> ApiResult<Json<Value>> {
    let summary = decayed_karma(&state, &pseud_id.to_string()).await?;
    Ok(Json(karma_json(summary)))
}

/// A pseud's public karma. Karma is a display signal, so there is nothing
/// private to gate here beyond the profile itself.
async fn get_karma(
    State(state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(pseud): Path<String>,
) -> ApiResult<Json<Value>> {
    if pseud.trim().is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "pseud",
        }));
    }
    let summary = decayed_karma(&state, &pseud).await?;
    Ok(Json(karma_json(summary)))
}

/// Read karma after applying any inactivity decay that is due.
///
/// The decay is applied on read as well as by a sweep, so a dormant pseud's
/// number is never a stale high-water mark (spec §35.2).
async fn decayed_karma(
    state: &AppState,
    pseud: &str,
) -> ApiResult<lorehaven_db::typed_votes::KarmaSummary> {
    lorehaven_db::typed_votes::decay_karma(
        state.db(),
        pseud,
        OffsetDateTime::now_utc(),
        state.config().forum.karma_decay_percent,
    )
    .await
    .map_err(internal)?;
    lorehaven_db::typed_votes::karma_summary(state.db(), pseud)
        .await
        .map_err(internal)
}

/// Karma in the wire shape: basis points for arithmetic, a number for display.
fn karma_json(summary: lorehaven_db::typed_votes::KarmaSummary) -> Value {
    json!({
        "pseud": summary.pseud,
        "karma_bp": summary.karma_bp,
        "karma": summary.karma_bp as f64 / 1000.0,
        "votes_received": summary.votes_received,
        "weighted_received_bp": summary.weighted_received_bp,
        "updated_at": if summary.updated_at.is_empty() { Value::Null } else { json!(summary.updated_at) },
    })
}
