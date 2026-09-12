//! Positivity filter and feedback delivery (spec section 12).
//!
//! ```text
//! GET    /feedback/preferences                 account default + effective line
//! PUT    /feedback/preferences                 { accept_constructive, ambiguous_auto, comments_enabled, expected_version }
//! GET    /feedback/preferences/works/{id}      effective policy for one work
//! PUT    /feedback/preferences/works/{id}      per-work override (null inherits)
//! GET    /feedback/inbox                       delivered reviews + held count
//! POST   /feedback/allow/{pseudId}             trust a commenter
//! POST   /feedback/deny/{pseudId}              refuse a commenter
//! ```
//!
//! Three rules hold across every handler:
//!
//! * Preferences belong to the *author* of the work, never the reviewer.
//!   Reads resolve the author from the work; writes are scoped to the
//!   session account. No route takes an account identifier.
//! * Sender responses never reveal author settings: posted vs held, nothing
//!   else. The classification and the reason stay server-side.
//! * Held text is never listed: the public review list and the inbox filter
//!   it in SQL, and the inbox adds only a count, never content.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use lorehaven_db::{collaboration, content, positivity};
use lorehaven_domain::positivity::{
    describe_policy, effective, FeedbackPreferences, WorkFeedbackOverride,
};
use lorehaven_domain::{AppError, PseudId, WorkId};
use serde::{Deserialize, Serialize};

use crate::auth::{RequirePseud, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

/// Feedback preference and inbox routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/feedback/preferences",
            get(get_preferences).put(put_preferences),
        )
        .route(
            "/feedback/preferences/works/{id}",
            get(get_work_policy).put(put_work_policy),
        )
        .route("/feedback/inbox", get(get_inbox))
        .route("/feedback/allow/{pseud}", post(allow_pseud))
        .route("/feedback/deny/{pseud}", post(deny_pseud))
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct PreferencesView {
    accept_constructive: bool,
    ambiguous_auto: bool,
    comments_enabled: bool,
    version: i64,
    effective_policy: String,
}

#[derive(Debug, Deserialize)]
struct PreferencesRequest {
    #[serde(default)]
    accept_constructive: Option<bool>,
    #[serde(default)]
    ambiguous_auto: Option<bool>,
    #[serde(default)]
    comments_enabled: Option<bool>,
    #[serde(default)]
    expected_version: Option<i64>,
}

#[derive(Debug, Serialize)]
struct WorkPolicyView {
    accept_constructive: bool,
    ambiguous_auto: bool,
    comments_enabled: bool,
    effective_policy: String,
}

#[derive(Debug, Deserialize)]
struct WorkPolicyRequest {
    #[serde(default)]
    accept_constructive: Option<bool>,
    #[serde(default)]
    ambiguous_auto: Option<bool>,
    #[serde(default)]
    comments_enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
struct InboxItemView {
    review_id: String,
    work_id: String,
    work_title: String,
    author_handle: String,
    body: String,
    class: String,
    published_at: Option<String>,
}

#[derive(Debug, Serialize)]
struct InboxView {
    items: Vec<InboxItemView>,
    held_count: i64,
    next_cursor: Option<String>,
}

async fn get_preferences(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<PreferencesView>> {
    let prefs = positivity::preferences_for(state.db(), user.account_id).await?;
    let version = positivity::preferences_version(state.db(), user.account_id).await?;
    Ok(Json(PreferencesView {
        accept_constructive: prefs.accept_constructive,
        ambiguous_auto: prefs.ambiguous_auto,
        comments_enabled: prefs.comments_enabled,
        version,
        effective_policy: describe_policy(&prefs),
    }))
}

async fn put_preferences(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(request): Json<PreferencesRequest>,
) -> ApiResult<Json<PreferencesView>> {
    let current = positivity::preferences_for(state.db(), user.account_id).await?;
    let version = positivity::preferences_version(state.db(), user.account_id).await?;
    let expected = request.expected_version.unwrap_or(version);
    let next = FeedbackPreferences {
        accept_constructive: request
            .accept_constructive
            .unwrap_or(current.accept_constructive),
        ambiguous_auto: request.ambiguous_auto.unwrap_or(current.ambiguous_auto),
        comments_enabled: request.comments_enabled.unwrap_or(current.comments_enabled),
    };
    if !positivity::save_preferences(state.db(), user.account_id, &next, expected).await? {
        let actual = positivity::preferences_version(state.db(), user.account_id).await?;
        return Err(ApiError(AppError::RevisionConflict { expected, actual }));
    }
    let version = positivity::preferences_version(state.db(), user.account_id).await?;
    Ok(Json(PreferencesView {
        accept_constructive: next.accept_constructive,
        ambiguous_auto: next.ambiguous_auto,
        comments_enabled: next.comments_enabled,
        version,
        effective_policy: describe_policy(&next),
    }))
}

async fn load_effective(state: &AppState, work_id: WorkId) -> ApiResult<FeedbackPreferences> {
    let work = content::find_work(state.db(), work_id)
        .await?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "work" }))?;
    let Some(author) = positivity::author_account_for_work(state.db(), work_id).await? else {
        return Err(ApiError(AppError::NotFound { resource: "work" }));
    };
    let _ = work;
    let account = positivity::preferences_for(state.db(), author).await?;
    let work_ov = positivity::override_for(state.db(), work_id).await?;
    Ok(effective(&account, &work_ov))
}

async fn get_work_policy(
    State(state): State<AppState>,
    RequireSession(_): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<WorkPolicyView>> {
    let work_id: WorkId = id
        .parse()
        .map_err(|_| ApiError(AppError::NotFound { resource: "work" }))?;
    let prefs = load_effective(&state, work_id).await?;
    Ok(Json(WorkPolicyView {
        accept_constructive: prefs.accept_constructive,
        ambiguous_auto: prefs.ambiguous_auto,
        comments_enabled: prefs.comments_enabled,
        effective_policy: describe_policy(&prefs),
    }))
}

async fn put_work_policy(
    State(state): State<AppState>,
    RequirePseud { user, pseud_id }: RequirePseud,
    Path(id): Path<String>,
    Json(request): Json<WorkPolicyRequest>,
) -> ApiResult<Json<WorkPolicyView>> {
    let work_id: WorkId = id
        .parse()
        .map_err(|_| ApiError(AppError::NotFound { resource: "work" }))?;
    // Only a contributor may set the work policy: the work must resolve
    // through the acting pseud, else it is a 404 rather than a transfer.
    let contributors = collaboration::contributors_for_work(state.db(), work_id).await?;
    let is_contributor = contributors.iter().any(|c| c.pseud_id == pseud_id);
    if !is_contributor {
        return Err(ApiError(AppError::NotFound { resource: "work" }));
    }
    let _ = user;
    let ov = WorkFeedbackOverride {
        accept_constructive: request.accept_constructive,
        ambiguous_auto: request.ambiguous_auto,
        comments_enabled: request.comments_enabled,
    };
    positivity::save_work_override(state.db(), work_id, &ov).await?;
    let prefs = load_effective(&state, work_id).await?;
    Ok(Json(WorkPolicyView {
        accept_constructive: prefs.accept_constructive,
        ambiguous_auto: prefs.ambiguous_auto,
        comments_enabled: prefs.comments_enabled,
        effective_policy: describe_policy(&prefs),
    }))
}

async fn get_inbox(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<InboxView>> {
    let items = positivity::inbox_for(state.db(), user.account_id).await?;
    let held_count = positivity::held_count_for(state.db(), user.account_id).await?;
    Ok(Json(InboxView {
        items: items
            .into_iter()
            .map(|item| InboxItemView {
                review_id: item.review_id,
                work_id: item.work_id,
                work_title: item.work_title,
                author_handle: item.author_handle,
                body: item.body,
                class: item.class,
                published_at: item.published_at,
            })
            .collect(),
        held_count,
        next_cursor: None,
    }))
}

async fn allow_pseud(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(pseud): Path<String>,
) -> ApiResult<StatusCode> {
    let trusted: PseudId = pseud
        .parse()
        .map_err(|_| ApiError(AppError::field("pseud", "Not a pseud identifier.")))?;
    positivity::allow_pseud(state.db(), user.account_id, trusted).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn deny_pseud(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(pseud): Path<String>,
) -> ApiResult<StatusCode> {
    let refused: PseudId = pseud
        .parse()
        .map_err(|_| ApiError(AppError::field("pseud", "Not a pseud identifier.")))?;
    positivity::deny_pseud(state.db(), user.account_id, refused).await?;
    Ok(StatusCode::NO_CONTENT)
}
