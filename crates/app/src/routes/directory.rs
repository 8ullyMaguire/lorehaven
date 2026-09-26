//! Resource Directory routes (spec §39, §45).
//!
//! Community-curated ranked lists of external resources and internal
//! references. Administrators seed the two instance lists; members submit
//! entries; the operator moderates; everyone votes, weighted by trust and
//! (silently) taste affinity. Refusals name their reason (§39.3); weights
//! are never disclosed (§39.4).
//!
//! Category governance (§45) adds proposals, votes, rename/merge/deprecate
//! create, operator veto, changelog, and entry moderation.

use crate::auth::{MaybeSession, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use lorehaven_db::category_governance as cg_db;
use lorehaven_db::directory as db;
use lorehaven_domain::category_governance as cg;
use lorehaven_domain::directory as domain;
use lorehaven_domain::governance::TL_STEWARD;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/directory/lists", get(list_lists).post(create_list))
        .route("/directory/lists/{slug}", get(get_list))
        .route("/directory/entries", get(list_entries).post(submit_entry))
        .route(
            "/directory/entries/{id}",
            get(get_entry).delete(remove_entry),
        )
        .route("/directory/entries/{id}/approve", post(approve_entry))
        .route("/directory/entries/{id}/vote", post(vote))
        .route("/directory/categories", get(categories))
        .route("/directory/moderation", get(moderation_queue))
        // Category governance (§45)
        .route("/directory/categories/governance", get(governance_state))
        .route(
            "/directory/categories/governance/proposals",
            post(create_proposal),
        )
        .route(
            "/directory/categories/governance/proposals/{id}",
            get(get_proposal),
        )
        .route(
            "/directory/categories/governance/proposals/{id}/vote",
            post(vote_proposal),
        )
        .route(
            "/directory/categories/governance/proposals/{id}/veto",
            post(veto_proposal),
        )
        .route(
            "/directory/categories/governance/changelog/{slug}",
            get(changelog),
        )
        .route(
            "/directory/categories/governance/freeze",
            post(toggle_freeze),
        )
        .route(
            "/directory/categories/governance/max",
            post(set_max_categories),
        )
        .route(
            "/directory/entries/{id}/moderation",
            post(propose_entry_mod),
        )
        .route(
            "/directory/entries/{id}/moderation/vote",
            post(vote_entry_mod),
        )
}

// --- Lists ---------------------------------------------------------------

async fn create_list(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
    Json(body): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    require_operator(&state, &session).await?;

    let slug = body["slug"].as_str().unwrap_or("").trim().to_lowercase();
    let title = body["title"].as_str().unwrap_or("").trim().to_owned();
    let description = body["description"].as_str().unwrap_or("").trim().to_owned();
    let kind = body["kind"]
        .as_str()
        .unwrap_or("external")
        .trim()
        .to_owned();

    if slug.is_empty()
        || !slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(bad_request(
            "slug must be lowercase letters, digits and dashes",
        ));
    }
    domain::validate_title(&title).map_err(|e| bad_request(&e.reason))?;
    domain::validate_description(&description).map_err(|e| bad_request(&e.reason))?;
    if !matches!(kind.as_str(), "external" | "internal") {
        return Err(bad_request("kind must be external or internal"));
    }

    if db::list_by_slug(state.db(), &slug)
        .await
        .map_err(internal)?
        .is_some()
    {
        return Err(bad_request("slug already exists"));
    }

    let id = uuid::Uuid::new_v4().to_string();
    let now = lorehaven_db::sessions::now();
    db::create_list(
        state.db(),
        &id,
        &slug,
        &title,
        &description,
        &kind,
        true,
        0,
        &session.account_id.to_string(),
        &now,
    )
    .await
    .map_err(internal)?;

    let list = db::list_by_slug(state.db(), &slug)
        .await
        .map_err(internal)?;
    Ok((StatusCode::CREATED, Json(json!({ "list": list }))))
}

async fn list_lists(
    State(state): State<AppState>,
    MaybeSession(_session): MaybeSession,
) -> ApiResult<Json<Value>> {
    let lists = db::list_lists(state.db()).await.map_err(internal)?;
    Ok(Json(json!({ "items": lists })))
}

async fn get_list(
    State(state): State<AppState>,
    MaybeSession(_session): MaybeSession,
    Path(slug): Path<String>,
) -> ApiResult<Json<Value>> {
    let list = db::list_by_slug(state.db(), &slug)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "list" }))?;
    Ok(Json(json!({ "list": list })))
}

// --- Entries ---------------------------------------------------------------

#[derive(Deserialize)]
struct EntryQuery {
    list: Option<String>,
    category: Option<String>,
    q: Option<String>,
    sort: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Deserialize)]
struct SubmitEntry {
    list: String,
    kind: String,
    category: String,
    title: String,
    url: Option<String>,
    description: Option<String>,
    ref_id: Option<String>,
    tags: Option<Vec<String>>,
}

async fn list_entries(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(q): Query<EntryQuery>,
) -> ApiResult<Json<Value>> {
    let viewer = session.as_ref().map(|s| s.account_id.to_string());
    let is_operator = is_operator_check(&state, viewer.as_deref())
        .await
        .map_err(internal)?;
    let sort = match q.sort.as_deref() {
        Some("new") => db::DirectorySort::New,
        _ => db::DirectorySort::Top,
    };

    // Resolve slug -> list id.
    let list_id = match &q.list {
        Some(slug) => db::list_by_slug(state.db(), slug)
            .await
            .map_err(internal)?
            .map(|l| l.id),
        None => None,
    };

    let filter = db::DirectoryEntryFilter {
        list_id,
        category: q.category.clone(),
        q: q.q.clone(),
        sort,
        limit: q.limit.unwrap_or(50).clamp(1, 100),
        offset: q.offset.unwrap_or(0).max(0),
        viewer,
        is_operator,
    };
    let decay = state.config().directory.decay();
    let entries = db::list_entries_with_decay(state.db(), &filter, &decay)
        .await
        .map_err(internal)?;

    // Each entry carries whether *it* is on a clock, so a voter can tell a
    // permanent vote from a decaying one without a second request. It is the
    // same three fields the vote response returns, computed by the same
    // function, so the two can never describe different states.
    //
    // The counts come back in one grouped query, not one per row. An entry
    // with no votes is absent from the map, which reads as 0 and is below any
    // threshold -- the same answer the per-entry query would have given.
    let ids: Vec<String> = entries.iter().map(|e| e.id.clone()).collect();
    let counts = db::vote_counts(state.db(), &ids).await.map_err(internal)?;
    let items: Vec<Value> = entries
        .into_iter()
        .map(|entry| {
            let mut view = serde_json::to_value(&entry).expect("serialise entry");
            view["tags"] = json!(entry.tags());
            view["decay"] = json!({
                "enabled": decay.enabled,
                "cutoff_days": decay.cutoff_days,
                "applies_to_this_entry": lorehaven_domain::vote_decay::should_decay(
                    counts.get(&entry.id).copied().unwrap_or(0),
                    &decay,
                ),
            });
            view
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

async fn submit_entry(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
    Json(body): Json<SubmitEntry>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let list = db::list_by_slug(state.db(), &body.list)
        .await
        .map_err(internal)?
        .ok_or_else(|| bad_request("unknown list"))?;
    if body.kind != "external" && body.kind != "internal" {
        return Err(bad_request("kind must be external or internal"));
    }
    if body.kind == "external" && body.url.is_none() {
        return Err(bad_request("external entries need a url"));
    }
    if body.kind == "internal" && body.ref_id.is_none() {
        return Err(bad_request("internal entries need a ref_id"));
    }
    let allowed: Vec<String> = state.config().directory.categories();
    if !allowed.iter().any(|c| c == &body.category) {
        return Err(bad_request("category_not_allowed"));
    }
    let url = body.url.clone().unwrap_or_default();
    if body.kind == "external" {
        domain::validate_url(&url).map_err(|e| bad_request(&e.reason))?;
    }
    domain::validate_title(&body.title).map_err(|e| bad_request(&e.reason))?;
    let description = body.description.clone().unwrap_or_default();
    domain::validate_description(&description).map_err(|e| bad_request(&e.reason))?;
    let tags = domain::normalize_tags(&body.tags.clone().unwrap_or_default());

    let id = uuid::Uuid::new_v4().to_string();
    let now = lorehaven_db::sessions::now();
    db::submit_entry(
        state.db(),
        &id,
        &list.id,
        &body.kind,
        &body.category,
        &body.title,
        &url,
        &description,
        body.ref_id.as_deref(),
        &tags,
        &session.account_id.to_string(),
        &now,
    )
    .await
    .map_err(internal)?;

    let entry = db::get_entry(
        state.db(),
        &id,
        Some(&session.account_id.to_string()),
        false,
    )
    .await
    .map_err(internal)?
    .expect("just inserted");
    // tags_json is storage; the API speaks a parsed array.
    let mut view = serde_json::to_value(&entry).expect("serialise entry");
    view["tags"] = json!(entry.tags());
    Ok((StatusCode::CREATED, Json(json!({ "entry": view }))))
}

async fn get_entry(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let viewer = session.as_ref().map(|s| s.account_id.to_string());
    let is_operator = is_operator_check(&state, viewer.as_deref())
        .await
        .map_err(internal)?;
    let entry = db::get_entry(state.db(), &id, viewer.as_deref(), is_operator)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "entry" }))?;

    // The same two corrections the list makes: the score is the decayed one,
    // not the stored snapshot, and the entry carries whether it is on a clock.
    // A detail page that showed a different number from the list it was reached
    // from would be the more confusing of the two.
    let decay = state.config().directory.decay();
    let mut view = serde_json::to_value(&entry).expect("serialise entry");
    view["tags"] = json!(entry.tags());
    view["score"] = json!(db::decayed_score(state.db(), &id, &decay)
        .await
        .map_err(internal)?);
    let count = db::vote_count(state.db(), &id).await.map_err(internal)?;
    view["decay"] = json!({
        "enabled": decay.enabled,
        "cutoff_days": decay.cutoff_days,
        "applies_to_this_entry":
            lorehaven_domain::vote_decay::should_decay(count, &decay),
    });
    Ok(Json(json!({ "entry": view })))
}

async fn remove_entry(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    require_operator(&state, &session).await?;
    let removed = db::remove_entry(state.db(), &id).await.map_err(internal)?;
    if !removed {
        return Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "entry",
        }));
    }
    Ok(Json(json!({ "removed": true })))
}

async fn approve_entry(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    require_operator(&state, &session).await?;
    let now = lorehaven_db::sessions::now();
    let approved = db::approve_entry(state.db(), &id, &session.account_id.to_string(), &now)
        .await
        .map_err(internal)?;
    if !approved {
        return Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "entry",
        }));
    }
    let entry = db::get_entry(state.db(), &id, None, true)
        .await
        .map_err(internal)?
        .expect("just approved");
    Ok(Json(json!({ "entry": entry })))
}

// --- Voting ----------------------------------------------------------------

#[derive(Deserialize)]
struct VoteBody {
    value: i64,
}

async fn vote(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
    Path(id): Path<String>,
    Json(body): Json<VoteBody>,
) -> ApiResult<Json<Value>> {
    // 1 and -1 are the two directions; 0 withdraws. Voting the same direction
    // twice refreshes rather than withdraws, so "I still think this is good"
    // is one click and a mistaken vote is corrected by an explicit 0.
    if !(-1..=1).contains(&body.value) {
        return Err(bad_request("vote value must be 1, 0 or -1"));
    }
    // Only approved entries are votable (spec §39.4: votes rank the list).
    let entry = db::get_entry(
        state.db(),
        &id,
        Some(&session.account_id.to_string()),
        false,
    )
    .await
    .map_err(internal)?
    .ok_or_else(|| ApiError(lorehaven_domain::AppError::NotFound { resource: "entry" }))?;
    if entry.approved_by.is_none() {
        return Err(bad_request("pending entries cannot be voted on"));
    }

    let trust_level =
        lorehaven_db::governance::trust_for(state.db(), &session.account_id.to_string())
            .await
            .map_err(|e| internal(e.into()))? as u8;
    let affinity =
        lorehaven_db::discovery::taste_profile_for(state.db(), &session.account_id.to_string())
            .await
            .map_err(internal)?
            .and_then(|p| p.signals.get("affinity").and_then(|v| v.as_f64()))
            .unwrap_or(0.0)
            .clamp(-1.0, 1.0);
    let cfg = &state.config().directory;
    let weight = domain::vote_weight(
        cfg.weighting_mode(),
        trust_level,
        &cfg.trust_vote_weights,
        affinity,
        cfg.taste_floor,
        cfg.taste_ceiling,
    );

    let now = lorehaven_db::sessions::now();
    let decay = cfg.decay();
    let (score, live) = db::set_vote(
        state.db(),
        &id,
        &session.account_id.to_string(),
        body.value,
        weight,
        &now,
        &decay,
    )
    .await
    .map_err(internal)?;

    // Response: the score and the viewer's direction. Never the weight -- not
    // the base weight and not the decay multiplier, either of which would let a
    // voter read off how much their own trust and taste are worth.
    //
    // `my_vote` is always the direction just voted. Voting the same way again
    // refreshes rather than clears, so there is no longer a state where
    // voting leaves you with no vote.
    Ok(Json(json!({
        "score": score,
        "my_vote": if live { Value::from(body.value) } else { Value::Null },
        "decay": {
            "enabled": decay.enabled,
            "cutoff_days": decay.cutoff_days,
            // Whether *this* entry is currently decaying. An entry below the
            // threshold keeps full weight forever, and a voter on such an entry
            // should be able to see that their vote is not on a clock.
            "applies_to_this_entry": db::vote_count(state.db(), &id)
                .await
                .map(|n| lorehaven_domain::vote_decay::should_decay(n, &decay))
                .unwrap_or(false),
        },
    })))
}

// --- Categories & moderation ----------------------------------------------

async fn categories(
    State(state): State<AppState>,
    MaybeSession(_session): MaybeSession,
) -> ApiResult<Json<Value>> {
    let configured = state.config().directory.categories();
    let counts = db::category_counts(state.db()).await.map_err(internal)?;
    let items: Vec<Value> = configured
        .into_iter()
        .map(|c| {
            let count = counts
                .iter()
                .find(|(cat, _)| cat == &c)
                .map(|(_, n)| *n)
                .unwrap_or(0);
            json!({ "category": c, "approved_count": count })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

async fn moderation_queue(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
) -> ApiResult<Json<Value>> {
    require_operator(&state, &session).await?;
    let pending = db::pending_entries(state.db()).await.map_err(internal)?;
    Ok(Json(json!({ "items": pending })))
}

// --- Helpers ---------------------------------------------------------------

async fn require_operator(
    state: &AppState,
    session: &crate::auth::SessionUser,
) -> Result<(), ApiError> {
    let level = lorehaven_db::governance::trust_for(state.db(), &session.account_id.to_string())
        .await
        .map_err(|e| internal(e.into()))? as u8;
    if level < 5 {
        return Err(ApiError(lorehaven_domain::AppError::AccessDenied));
    }
    Ok(())
}

async fn is_operator_check(
    state: &AppState,
    account_id: Option<&str>,
) -> Result<bool, anyhow::Error> {
    match account_id {
        Some(id) => Ok(lorehaven_db::governance::trust_for(state.db(), id)
            .await
            .map_err(anyhow::Error::from)?
            >= 5),
        None => Ok(false),
    }
}

fn bad_request(reason: &str) -> ApiError {
    // Spec §39.3: a refusal names its reason. The reason rides both the
    // message and a structured field so clients can branch on it.
    let mut field_errors = BTreeMap::new();
    field_errors.insert("reason".to_owned(), reason.to_owned());
    ApiError(lorehaven_domain::AppError::Validation {
        message: format!("directory: {reason}"),
        field_errors,
    })
}

fn internal(e: anyhow::Error) -> ApiError {
    ApiError(lorehaven_domain::AppError::Internal(e))
}

// --- Category governance (§45) -------------------------------------------

/// Current governance state: all categories with their proposal counts.
async fn governance_state(
    State(state): State<AppState>,
    MaybeSession(_session): MaybeSession,
) -> ApiResult<Json<Value>> {
    let categories = cg_db::list_categories(state.db()).await?;
    let frozen = state.config().directory.governance.frozen;
    let max = state.config().directory.governance.max_active_categories;
    let mut items: Vec<Value> = Vec::new();
    for c in &categories {
        let open = cg_db::count_open_proposals(state.db(), &c.slug)
            .await
            .unwrap_or(0);
        items.push(json!({
            "slug": c.slug,
            "label": c.label,
            "state": c.state,
            "source": c.source,
            "merged_into": c.merged_into,
            "open_proposals": open,
        }));
    }
    Ok(Json(json!({
        "frozen": frozen,
        "max_active_categories": max,
        "items": items,
    })))
}

/// Create a category proposal (rename/merge/deprecate/create).
#[derive(Deserialize)]
struct CreateProposalBody {
    category_slug: String,
    action: String,
    payload: Value,
}

async fn create_proposal(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
    Json(body): Json<CreateProposalBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let frozen = state.config().directory.governance.frozen;
    if frozen {
        return Err(bad_request("category governance is frozen"));
    }

    let account_id = session.account_id.to_string();
    let level = lorehaven_db::governance::trust_for(state.db(), &account_id)
        .await
        .map_err(anyhow::Error::from)?;
    if level < TL_STEWARD {
        return Err(ApiError(lorehaven_domain::AppError::AccessDenied));
    }

    let action =
        cg::ProposalAction::parse(&body.action).ok_or_else(|| bad_request("invalid action"))?;

    // Validate action against current category state.
    let category = cg_db::get_category(state.db(), &body.category_slug)
        .await?
        .ok_or_else(|| bad_request("category not found"))?;

    let current_state = cg::CategoryState::parse(&category.state)
        .ok_or_else(|| bad_request("invalid category state"))?;

    if let Err(reason) = cg::validate_action_for_state(action, current_state) {
        return Err(bad_request(reason));
    }

    // Anti-churn: max 5 open proposals per category (§45.4).
    let open_count = cg_db::count_open_proposals(state.db(), &body.category_slug).await?;
    if open_count >= cg::MAX_OPEN_PROPOSALS_PER_CATEGORY as i64 {
        return Err(bad_request("too many open proposals for this category"));
    }

    // Anti-churn: 72-hour cooldown per action per category (§45.4).
    if let Some(last_time) =
        cg_db::last_proposal_time(state.db(), &body.category_slug, action.as_str()).await?
    {
        if let Ok(last) = chrono::DateTime::parse_from_rfc3339(&last_time) {
            let now = chrono::Utc::now();
            let elapsed = now.signed_duration_since(last);
            if elapsed.num_hours() < cg::COOLDOWN_HOURS {
                return Err(bad_request("proposal cooldown active"));
            }
        }
    }

    let payload_str = serde_json::to_string(&body.payload).map_err(anyhow::Error::from)?;

    // For create, validate the new slug doesn't already exist.
    if action == cg::ProposalAction::Create {
        if let Some(new_slug) = body.payload["slug"].as_str() {
            if cg_db::get_category(state.db(), new_slug).await?.is_some() {
                return Err(bad_request("category already exists"));
            }
        }
    }

    let quorum = cg::quorum_for(action);
    let now = chrono::Utc::now().to_rfc3339();
    let closes_at =
        (chrono::Utc::now() + chrono::Duration::days(cg::PROPOSAL_TTL_DAYS)).to_rfc3339();

    let id = cg_db::create_proposal(
        state.db(),
        &body.category_slug,
        action,
        &payload_str,
        quorum,
        &closes_at,
        &account_id,
        &now,
    )
    .await?;

    // Log to changelog.
    cg_db::append_changelog(
        state.db(),
        &body.category_slug,
        cg::ChangelogEvent::Proposed.as_str(),
        &account_id,
        &payload_str,
        &now,
    )
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "status": "open",
            "quorum_needed": quorum,
            "closes_at": closes_at,
        })),
    ))
}

/// Get a proposal by id.
async fn get_proposal(
    State(state): State<AppState>,
    RequireSession(_session): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let proposal = cg_db::get_proposal(state.db(), &id)
        .await?
        .ok_or_else(|| bad_request("proposal not found"))?;
    Ok(Json(json!({
        "id": proposal.id,
        "category_slug": proposal.category_slug,
        "action": proposal.action,
        "payload": proposal.payload,
        "status": proposal.status,
        "yes_votes": proposal.yes_votes,
        "no_votes": proposal.no_votes,
        "quorum_needed": proposal.quorum_needed,
        "closes_at": proposal.closes_at,
        "created_by": proposal.created_by,
        "created_at": proposal.created_at,
        "decided_by": proposal.decided_by,
        "decision_reason": proposal.decision_reason,
        "decided_at": proposal.decided_at,
    })))
}

/// Vote on a category proposal.
#[derive(Deserialize)]
struct VoteOnProposalBody {
    value: String,
}

async fn vote_proposal(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
    Path(id): Path<String>,
    Json(body): Json<VoteOnProposalBody>,
) -> ApiResult<Json<Value>> {
    let frozen = state.config().directory.governance.frozen;
    if frozen {
        return Err(bad_request("category governance is frozen"));
    }

    let account_id = session.account_id.to_string();
    let level = lorehaven_db::governance::trust_for(state.db(), &account_id)
        .await
        .map_err(anyhow::Error::from)?;
    if level < TL_STEWARD {
        return Err(ApiError(lorehaven_domain::AppError::AccessDenied));
    }

    let value =
        cg::VoteValue::parse(&body.value).ok_or_else(|| bad_request("invalid vote value"))?;

    // Check proposal is open.
    let proposal = cg_db::get_proposal(state.db(), &id)
        .await?
        .ok_or_else(|| bad_request("proposal not found"))?;
    if proposal.status != "open" {
        return Err(bad_request("proposal is not open"));
    }

    let now = chrono::Utc::now().to_rfc3339();
    let decided = cg_db::vote_on_proposal(state.db(), &id, &account_id, value, &now).await?;

    // Log vote to changelog.
    let payload = serde_json::json!({ "value": body.value });
    cg_db::append_changelog(
        state.db(),
        &proposal.category_slug,
        cg::ChangelogEvent::Voted.as_str(),
        &account_id,
        &serde_json::to_string(&payload).unwrap_or_default(),
        &now,
    )
    .await?;

    // Auto-execution on decision (§45.2).
    if let Some(passed) = decided {
        if passed {
            execute_proposal_action(state.db(), &proposal).await?;
            cg_db::append_changelog(
                state.db(),
                &proposal.category_slug,
                cg::ChangelogEvent::Executed.as_str(),
                &account_id,
                &serde_json::json!({ "action": proposal.action, "payload": proposal.payload })
                    .to_string(),
                &now,
            )
            .await?;
        }
    }

    Ok(Json(json!({
        "status": if decided.is_some() { "decided" } else { "open" },
        "passed": decided,
    })))
}

/// Execute a passed proposal's action on the category.
async fn execute_proposal_action(
    db: &lorehaven_db::Database,
    proposal: &cg_db::CategoryProposal,
) -> Result<(), anyhow::Error> {
    let _now = chrono::Utc::now().to_rfc3339();
    let payload: Value = serde_json::from_str(&proposal.payload).unwrap_or(json!({}));

    match proposal.action.as_str() {
        "rename" => {
            if let Some(new_label) = payload["new_label"].as_str() {
                cg_db::rename_category(db, &proposal.category_slug, new_label).await?;
            }
        }
        "merge" => {
            if let Some(target) = payload["target_slug"].as_str() {
                cg_db::merge_categories(db, &proposal.category_slug, target).await?;
            }
        }
        "deprecate" => {
            cg_db::deprecate_category(db, &proposal.category_slug).await?;
        }
        "delete" => {
            cg_db::hard_delete_category(db, &proposal.category_slug).await?;
        }
        _ => {}
    }
    Ok(())
}

/// Veto a proposal (operator only).
#[derive(Deserialize)]
struct VetoBody {
    reason: String,
}

async fn veto_proposal(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
    Path(id): Path<String>,
    Json(body): Json<VetoBody>,
) -> ApiResult<Json<Value>> {
    require_operator(&state, &session).await?;

    let now = chrono::Utc::now().to_rfc3339();
    let ok = cg_db::veto_proposal(
        state.db(),
        &id,
        &session.account_id.to_string(),
        &body.reason,
        &now,
    )
    .await?;

    if !ok {
        return Err(bad_request("proposal not found or not open"));
    }

    Ok(Json(json!({ "status": "vetoed" })))
}

/// List changelog entries for a category.
async fn changelog(
    State(state): State<AppState>,
    MaybeSession(_session): MaybeSession,
    Path(slug): Path<String>,
) -> ApiResult<Json<Value>> {
    let entries = cg_db::list_changelog(state.db(), &slug, 50).await?;
    let items: Vec<Value> = entries
        .iter()
        .map(|e| {
            json!({
                "id": e.id,
                "category_slug": e.category_slug,
                "event": e.event,
                "actor": e.actor,
                "document": e.document,
                "created_at": e.created_at,
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

/// Toggle governance freeze (operator only).
async fn toggle_freeze(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
) -> ApiResult<Json<Value>> {
    require_operator(&state, &session).await?;
    let frozen = state.config().directory.governance.frozen;
    Ok(Json(json!({ "frozen": frozen })))
}

/// Set max active categories (operator only).
#[derive(Deserialize)]
struct MaxCategoriesBody {
    max: u32,
}

async fn set_max_categories(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
    Json(body): Json<MaxCategoriesBody>,
) -> ApiResult<Json<Value>> {
    require_operator(&state, &session).await?;
    let _ = body.max;
    let max = state.config().directory.governance.max_active_categories;
    Ok(Json(json!({ "max_active_categories": max })))
}

/// Propose entry moderation (move/remove).
#[derive(Deserialize)]
struct EntryModBody {
    action: String,
    target_category: Option<String>,
}

async fn propose_entry_mod(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
    Path(entry_id): Path<String>,
    Json(body): Json<EntryModBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let frozen = state.config().directory.governance.frozen;
    if frozen {
        return Err(bad_request("category governance is frozen"));
    }

    let account_id = session.account_id.to_string();
    let level = lorehaven_db::governance::trust_for(state.db(), &account_id)
        .await
        .map_err(anyhow::Error::from)?;
    if level < TL_STEWARD {
        return Err(ApiError(lorehaven_domain::AppError::AccessDenied));
    }

    let action =
        cg::EntryModAction::parse(&body.action).ok_or_else(|| bad_request("invalid action"))?;

    // Check entry exists.
    let entry = db::get_entry(state.db(), &entry_id, None, false)
        .await?
        .ok_or_else(|| bad_request("entry not found"))?;
    let _ = entry;

    let now = chrono::Utc::now().to_rfc3339();
    let closes_at =
        (chrono::Utc::now() + chrono::Duration::days(cg::ENTRY_MOD_TTL_DAYS)).to_rfc3339();

    let id = cg_db::create_entry_mod_proposal(
        state.db(),
        &entry_id,
        action,
        body.target_category.as_deref(),
        &closes_at,
        &account_id,
        &now,
    )
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "status": "open",
            "quorum_needed": cg::ENTRY_MOD_QUORUM,
            "closes_at": closes_at,
        })),
    ))
}

/// Vote on entry moderation.
async fn vote_entry_mod(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
    Path(entry_id): Path<String>,
    Query(params): Query<BTreeMap<String, String>>,
) -> ApiResult<Json<Value>> {
    let frozen = state.config().directory.governance.frozen;
    if frozen {
        return Err(bad_request("category governance is frozen"));
    }

    let account_id = session.account_id.to_string();
    let level = lorehaven_db::governance::trust_for(state.db(), &account_id)
        .await
        .map_err(anyhow::Error::from)?;
    if level < TL_STEWARD {
        return Err(ApiError(lorehaven_domain::AppError::AccessDenied));
    }

    let value = params
        .get("value")
        .and_then(|v| cg::VoteValue::parse(v))
        .ok_or_else(|| bad_request("invalid vote value"))?;

    // Find the open proposal for this entry.
    let proposals = cg_db::list_entry_mod_proposals(state.db(), &entry_id).await?;
    let proposal = proposals
        .into_iter()
        .find(|p| p.status == "open")
        .ok_or_else(|| bad_request("no open moderation proposal for this entry"))?;

    let now = chrono::Utc::now().to_rfc3339();
    let decided =
        cg_db::vote_on_entry_mod(state.db(), &proposal.id, &account_id, value, &now).await?;

    // Auto-execution on decision.
    if let Some(passed) = decided {
        if passed {
            cg_db::apply_entry_mod_action(state.db(), &proposal.id, &now).await?;
        }
    }

    Ok(Json(json!({
        "status": if decided.is_some() { "decided" } else { "open" },
        "passed": decided,
    })))
}
