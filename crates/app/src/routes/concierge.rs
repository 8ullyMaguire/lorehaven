//! M45-22 — the personal concierge's HTTP surface (spec §54.5, §54.6, §54.7).
//!
//! ```text
//! GET    /me/concierge              render a queue (?mood= & ?minutes=)
//! GET    /me/concierge/sessions     the reader's own session history
//! GET    /me/watches                the reader's own WIP watches
//! PUT    /me/watches/{work_id}      watch a work's completion
//! DELETE /me/watches/{work_id}      withdraw, silently (§54.5)
//! ```
//!
//! Every route is behind `RequireSession`. Not because a mood is secret — it is
//! not — but because §54.3 records the session, §54.6 scopes the history to one
//! reader, and a route that may act on the caller's behalf must not be reachable
//! by someone who has not identified themselves.
//!
//! **The order of work in `render_queue` is the whole of §54.** It is:
//!
//! 1. Validate the selector against the moods this instance actually carries.
//!    An unknown mood is refused, naming the list — never a fallback to the
//!    unfiltered queue, because §54.6 makes "matched nothing" an answer with its
//!    own explanation.
//! 2. Run the §16 blend with `seen` populated and a **cap well above the budget**.
//!    The cut is a tail operation; asking the blend for exactly the budget's worth
//!    would make the budget decide *eligibility*, which §54.2 forbids.
//! 3. Narrow by mood — a candidate-set constraint, applied to the blend's order,
//!    before any time arithmetic.
//! 4. `apply_budget` — cuts the tail, and only the tail.
//! 5. `record_session`.
//!
//! Steps 2 and 3 are the two that are easy to swap, and swapping them produces a
//! feature that looks right and ranks wrongly: with the mood applied after the
//! budget, a mood filter would silently drop whatever the budget cut, and the
//! reader would be told their mood excluded things their time did.

use axum::extract::{Path, Query, State};
use axum::routing::{delete, get, put};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

use lorehaven_db::concierge_store as store;
use lorehaven_domain::concierge::{
    apply_budget, ConciergeQueue, QueueReason, RateSource, SessionSelector,
};

/// How many candidates the blend is asked for before any cutting.
///
/// Deliberately large and deliberately **not** derived from the budget. The blend
/// is allowed to overrun and then be cut; if this number tracked `minutes`, the
/// budget would be deciding which works were eligible before `apply_budget` ever
/// ran, and §54.2's table ("time constrains the tail, never the ranking") would be
/// false in the one place a reader could observe it.
const CANDIDATE_CAP: usize = 60;

/// `?mood=` and `?minutes=`.
///
/// Both optional, and `Option` for both, so "no mood" and "no budget" are
/// distinguishable from a mood of `""` and a budget of `0`. A reader who sends
/// `minutes=0` has said something — I have no time — and gets an explained empty
/// queue, not the whole blend.
#[derive(Debug, Default, Deserialize)]
pub struct ConciergeQuery {
    pub mood: Option<String>,
    pub minutes: Option<u32>,
}

impl ConciergeQuery {
    fn selector(&self) -> SessionSelector {
        SessionSelector {
            mood: self.mood.clone(),
            budget_minutes: self.minutes,
        }
    }
}

/// `GET /api/v1/me/concierge` — render a queue for this reader (§54.4).
pub async fn render_queue(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Query(q): Query<ConciergeQuery>,
) -> ApiResult<Json<Value>> {
    let db = state.db();
    let account = user.account_id.to_string();
    let selector = q.selector();

    // 1. Validate. The available list is the moods a *published* work actually
    //    carries — a taxonomy node nobody has used is not something a reader can
    //    ask for and be satisfied by, and offering it would be a promise the
    //    instance cannot keep.
    let available = store::moods_in_use(db)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    if let Err(refusal) = selector.validate(&available) {
        // 422 with the message naming what DOES exist. `AppError::Validation` is
        // 422 in this codebase, and a refusal is a validation outcome: the request
        // was well-formed and the answer is no.
        return Err(ApiError(lorehaven_domain::AppError::field(
            "mood",
            refusal.message(),
        )));
    }

    // 2. The §16 blend, unmodified, with a cap that is NOT the budget. `seen`
    //    comes from inside `generate_with_registry`, which reads the reader's
    //    history — the same exclusion the discovery route applies, so a reader with
    //    no selector sees the same works in the same order (§54.7's invariant that
    //    the session layer did not make the default path worse).
    let registry = crate::rec_engine::build_registry(&state.config().discovery);
    let ranked = crate::rec_engine::generate_with_registry(db, &registry, &account, CANDIDATE_CAP)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    // 3. Narrow by mood, in blend order. §54.1: mood constrains the candidate set,
    //    and the ranking inside that set is untouched.
    let candidates: Vec<String> = match selector.mood.as_deref() {
        Some(mood) if !mood.trim().is_empty() => store::filter_by_mood(db, &ranked, mood)
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?,
        _ => ranked.clone(),
    };

    // §54.6: a selector that matched nothing is an ANSWER with an explanation, not
    // a fallback to the unfiltered queue. It is still recorded, so "I asked and
    // got nothing" is visible in the reader's own history.
    if candidates.is_empty() && selector.mood.is_some() {
        let why = match selector.mood.as_deref() {
            Some(mood) if !mood.trim().is_empty() => format!(
                "no work in this instance's current recommendations carries the mood {mood:?}"
            ),
            _ => "there is nothing to recommend yet".to_owned(),
        };
        let queue = ConciergeQueue::explained_empty("", why, RateSource::Default, selector.clone());
        let session_id = store::record_session(db, &account, &queue)
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
        return Ok(Json(json!({
            "session_id": session_id,
            "items": [],
            "estimated_minutes": 0.0,
            "truncated_at": Value::Null,
            "rate_source": "default",
            "explained_empty": queue.explained_empty,
        })));
    }

    // 4. Duration estimates, then the cut. A work with no chapters has no
    //    estimate and is kept and marked (§54.4), charged the queue's midpoint.
    let estimates = store::duration_estimates(db, &candidates)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    let with_estimates: Vec<(String, Option<f64>)> = candidates
        .iter()
        .map(|id| (id.clone(), estimates.get(id).copied()))
        .collect();

    let budget = selector.budget_minutes.map(f64::from);
    let (mut items, truncated_at, total) = apply_budget(&with_estimates, budget);

    // §50's `reason`, and §54.6's transparency about it: an item that survived
    // because no mood filter applied says `Blend`, one that survived because its
    // duration is unknown says so. A reader who asked for "comfort" and is shown an
    // item marked `Blend` can see that the mood did not do what they asked — which
    // is the point of carrying the reason at all.
    if selector.mood.is_some() {
        for item in &mut items {
            if item.reason == QueueReason::DurationUnknown {
                continue;
            }
            item.reason = QueueReason::Mood {
                mood: selector.mood.clone().unwrap_or_default(),
            };
        }
    }

    // §54.4: the rate is the reader's observed rate if they have one, and the
    // instance default if not. Guessing a personal rate without an observation
    // would make the same request mean different things on a reader's first day
    // and their fortieth.
    let rate_source = if has_observed_rate(db, &account).await {
        RateSource::Observed
    } else {
        RateSource::Default
    };

    let queue = ConciergeQueue {
        session_id: String::new(),
        items,
        estimated_minutes: total,
        truncated_at,
        rate_source,
        selector,
        explained_empty: None,
    };
    let session_id = store::record_session(db, &account, &queue)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    Ok(Json(json!({
        "session_id": session_id,
        "items": queue.items,
        "estimated_minutes": queue.estimated_minutes,
        "truncated_at": queue.truncated_at,
        "rate_source": match queue.rate_source {
            RateSource::Observed => "observed",
            RateSource::Default => "default",
        },
        "explained_empty": Value::Null,
    })))
}

/// Does this reader have an observed reading rate? (§54.4)
///
/// §36.11's progress journal is the proper home for this and is **not** built, so
/// the observation is read directly from `reading_progress`. It counts *advanced*
/// positions only: a stored position of 0 permille is an account that exists, not
/// a reader who has been observed, and treating it as one would make a brand-new
/// account's first request claim a personal rate it has never demonstrated.
///
/// Errors answer `false`, which means "use the instance default". A reader served
/// the default is a worse estimate than one served a guessed personal rate, and
/// §54.4 forbids the guess; the fallback is the honest side of that trade.
async fn has_observed_rate(db: &lorehaven_db::Database, account_id: &str) -> bool {
    let sql = db.sql(
        "SELECT COUNT(*) FROM reading_progress
          WHERE account_id = ? AND position_permille > 0",
        "SELECT COUNT(*) FROM reading_progress
          WHERE account_id = $1::uuid AND position_permille > 0",
    );
    // A 1-tuple, because `FromRow` is not implemented for a bare `i64`. Decoding a
    // COUNT needs one column, and `COUNT(*)` is the only way to ask this question
    // without pulling the rows.
    let count: (i64,) = match db.backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_as::<_, (i64,)>(&sql)
            .bind(account_id)
            .fetch_one(db.sqlite_pool().expect("sqlite handle"))
            .await
            .unwrap_or((0,)),
        lorehaven_db::Backend::Postgres => sqlx::query_as::<_, (i64,)>(&sql)
            .bind(account_id)
            .fetch_one(db.postgres_pool().expect("postgres handle"))
            .await
            .unwrap_or((0,)),
    };
    count.0 > 0
}

/// `GET /api/v1/me/concierge/sessions` — the reader's own history (§54.3).
pub async fn list_sessions(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    let rows = store::sessions_for(state.db(), &user.account_id.to_string(), 50)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    let sessions: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.id,
                "mood": r.mood,
                "budget_minutes": r.budget_minutes,
                "work_ids": store::decode_work_ids(&r.work_ids),
                "estimated_minutes": r.estimated_minutes,
                "truncated_at": r.truncated_at,
                "rate_source": r.rate_source,
                "created_at": r.created_at,
            })
        })
        .collect();
    Ok(Json(json!({ "sessions": sessions })))
}

/// `GET /api/v1/me/watches` — the reader's own watches (§54.5).
pub async fn list_watches(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    let rows = store::watches_for(state.db(), &user.account_id.to_string())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    let watches: Vec<Value> = rows
        .iter()
        .map(|(work_id, id, notified_at)| {
            json!({
                "work_id": work_id,
                "id": id,
                // Null means "still waiting". A reader can tell a watch that has
                // fired from one that has not without a second field saying so.
                "notified_at": notified_at,
                "pending": notified_at.is_none(),
            })
        })
        .collect();
    Ok(Json(json!({ "watches": watches })))
}

/// `PUT /api/v1/me/watches/{work_id}` — watch a work's completion (§54.5).
///
/// Idempotent: watching a work already watched returns the first watch's id, so a
/// double-tap cannot produce two notifications.
pub async fn put_watch(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(work_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let db = state.db();
    let account = user.account_id.to_string();
    let watch_id = store::add_watch(db, &account, &work_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;

    // §54.5's immediate-notify case: watching a work that is already complete
    // notifies now rather than waiting for a transition that will not come. Best
    // effort — a watch that exists is the durable part, and a failed notification
    // must not fail the request.
    let mut notified = false;
    if store::is_complete(db, &work_id).await.unwrap_or(false) {
        notified = crate::wip_watch::notify_completion(db, &work_id)
            .await
            .map(|n| n > 0)
            .unwrap_or_else(|e| {
                tracing::warn!(%work_id, %e, "immediate completion notify failed");
                false
            });
    }

    Ok(Json(json!({ "watch_id": watch_id, "notified": notified })))
}

/// `DELETE /api/v1/me/watches/{work_id}` — withdraw a watch (§54.5).
///
/// §54.5: silent, no notification, no tombstone. Idempotent too — withdrawing a
/// watch that is not there is a success, because the state the caller asked for is
/// the state they now have.
pub async fn delete_watch(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(work_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let removed = store::remove_watch(state.db(), &user.account_id.to_string(), &work_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "removed": removed })))
}

/// The router. Mounted under `/api/v1` by `server.rs`, so these paths are relative
/// to that — spelling out `/api/v1` here registered them at `/api/v1/api/v1/...`,
/// which compiled fine, passed the route-inventory test (table and module agreeing
/// on the same wrong string) and answered every real call with a bare 405.
pub fn router() -> axum::Router<AppState> {
    // One `.route(...)` per line, and `put`/`delete` as separate calls rather than
    // chained. `route_inventory.rs`'s extractor reads the path and its handlers off
    // a single line and recovers a bare method name — `get` — as the handler when
    // the call is chained or wrapped. The wrapped form made the inventory test fail
    // with a claim about `/me/permissions`, four files from anything it changed,
    // which is exactly the confusing failure this comment exists to prevent.
    axum::Router::new()
        .route("/me/concierge", get(render_queue))
        .route("/me/concierge/sessions", get(list_sessions))
        .route("/me/watches", get(list_watches))
        .route("/me/watches/{work_id}", put(put_watch))
        .route("/me/watches/{work_id}", delete(delete_watch))
}
