//! Contributor and invitation endpoints (spec §8).
//!
//! ```text
//! POST   /works/:id/contributors/invitations   invite a pseud
//! PATCH  /works/:id/contributors/:pseudId      change a role or attribution
//! DELETE /works/:id/contributors/:pseudId      remove a contributor
//! GET    /invitations                          invitations awaiting the acting pseud
//! POST   /invitations/:id/accept               accept one
//! POST   /invitations/:id/decline              decline one
//! ```
//!
//! Two properties this module exists to hold:
//!
//! * **An invitation identifies pseuds, never accounts** (spec §8 acceptance:
//!   "invitations identify the exposed pseud"). The response names the invited
//!   pseud and the pseud that invited them, and nothing about who owns either.
//! * **Only an owner or co-author may invite**, checked through
//!   `can_manage_contributors` rather than by comparing identifiers inline.
//!
//! There is no email transport yet (Milestone 16 owns it), so an invitation is
//! delivered in the interface: the invited pseud sees it under `/invitations`.
//! That is stated rather than papered over — the token hash is stored for the
//! link-based flow that will use it.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use lorehaven_db::collaboration;
use lorehaven_db::identity;
use lorehaven_domain::content::{can_manage_contributors, Contributor, ContributorRole};
use lorehaven_domain::policy::{Decision, DenyReason};
use lorehaven_domain::{AppError, CollaborationInviteId, PseudId, WorkId};
use serde::{Deserialize, Serialize};

use crate::auth::{RequireSession, SessionUser};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

/// Contributor and invitation routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/works/{id}/contributors/invitations",
            post(create_invitation),
        )
        .route(
            "/works/{id}/contributors/{pseud}",
            patch(update_contributor).delete(remove_contributor),
        )
        .route("/invitations", get(list_invitations))
        .route("/invitations/{id}/accept", post(accept_invitation))
        .route("/invitations/{id}/decline", post(decline_invitation))
        .route("/invitations/{id}/revoke", post(revoke_invitation))
}

/// An invitation, as either side sees it.
#[derive(Debug, Serialize)]
struct InvitationView {
    id: CollaborationInviteId,
    work_id: WorkId,
    work_title: String,
    /// The pseud that was invited.
    invited_handle: String,
    /// The pseud that issued the invitation.
    invited_by_handle: String,
    role: String,
    role_label: String,
    status: String,
    message: Option<String>,
    created_at: String,
    version: i64,
}

impl From<collaboration::Invite> for InvitationView {
    fn from(invite: collaboration::Invite) -> Self {
        Self {
            id: invite.id,
            work_id: invite.work_id,
            work_title: invite.work_title,
            invited_handle: invite.invited_handle,
            invited_by_handle: invite.invited_by_handle,
            role_label: invite.role.label().to_owned(),
            role: invite.role.as_str().to_owned(),
            status: invite.status,
            message: invite.message,
            created_at: invite.created_at,
            version: invite.version,
        }
    }
}

#[derive(Debug, Deserialize)]
struct InviteRequest {
    /// The handle of the pseud to invite.
    handle: String,
    role: String,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateContributorRequest {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    public_attribution: Option<bool>,
}

async fn create_invitation(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Json(request): Json<InviteRequest>,
) -> ApiResult<(StatusCode, Json<InvitationView>)> {
    let actor_pseud = acting_pseud(&user)?;
    let (work, contributors) = owned_work(&state, &user, &id).await?;

    if let Decision::Deny(reason) = manage_decision(&user, &contributors) {
        return Err(refusal(reason));
    }

    let role = ContributorRole::parse(&request.role).ok_or_else(|| {
        ApiError(AppError::field(
            "role",
            "A role is one of owner, coauthor, editor or beta_reader.",
        ))
    })?;

    // Inviting an owner would hand the work away; ownership moves by an
    // explicit transfer, which does not exist yet and is not invented here.
    if matches!(role, ContributorRole::Owner) {
        return Err(ApiError(AppError::field(
            "role",
            "Ownership cannot be handed over by invitation.",
        )));
    }

    let invited = identity::find_pseud_by_handle(state.db(), request.handle.trim())
        .await?
        .ok_or_else(|| {
            ApiError(AppError::field(
                "handle",
                "No pseud with that handle. Handles are the public handle, not an email address.",
            ))
        })?;

    if contributors.iter().any(|c| c.pseud_id == invited.id) {
        return Err(ApiError(AppError::field(
            "handle",
            "That pseud already contributes to this work.",
        )));
    }

    if invited.id == actor_pseud {
        return Err(ApiError(AppError::field(
            "handle",
            "You already contribute to this work.",
        )));
    }

    let message = request
        .message
        .as_deref()
        .map(str::trim)
        .filter(|message| !message.is_empty());
    if message.is_some_and(|message| message.chars().count() > 2000) {
        return Err(ApiError(AppError::field(
            "message",
            "A message may be at most 2000 characters.",
        )));
    }

    // The token is minted so that the link-based delivery Milestone 16 adds has
    // something to carry; only its hash is stored.
    let token = crate::crypto::generate_token();
    let id = collaboration::create_invite(
        state.db(),
        work.id,
        invited.id,
        actor_pseud,
        role,
        &crate::crypto::hash_token(&token),
        message,
    )
    .await?;

    let invite = collaboration::find_invite(state.db(), id)
        .await?
        .ok_or_else(|| {
            ApiError(AppError::internal(
                "invite",
                anyhow::anyhow!("invite vanished"),
            ))
        })?;

    Ok((StatusCode::CREATED, Json(invite.into())))
}

async fn update_contributor(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path((id, pseud)): Path<(String, String)>,
    Json(request): Json<UpdateContributorRequest>,
) -> ApiResult<StatusCode> {
    let (work, contributors) = owned_work(&state, &user, &id).await?;

    if let Decision::Deny(reason) = manage_decision(&user, &contributors) {
        return Err(refusal(reason));
    }

    let pseud_id = parse_pseud_id(&pseud)?;
    let role = match request.role.as_deref() {
        Some(raw) => Some(ContributorRole::parse(raw).ok_or_else(|| {
            ApiError(AppError::field(
                "role",
                "A role is one of owner, coauthor, editor or beta_reader.",
            ))
        })?),
        None => None,
    };

    if matches!(role, Some(ContributorRole::Owner)) {
        return Err(ApiError(AppError::field(
            "role",
            "Ownership cannot be handed over. Transfer is not implemented.",
        )));
    }

    let changed = collaboration::update_contributor(
        state.db(),
        work.id,
        pseud_id,
        role,
        request.public_attribution,
    )
    .await?;

    if !changed {
        // Either the pseud does not contribute, or it is the owner — which this
        // endpoint deliberately refuses to change.
        return Err(ApiError(AppError::field(
            "pseud",
            "That pseud is not a changeable contributor of this work.",
        )));
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn remove_contributor(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path((id, pseud)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let (work, contributors) = owned_work(&state, &user, &id).await?;

    if let Decision::Deny(reason) = manage_decision(&user, &contributors) {
        return Err(refusal(reason));
    }

    let pseud_id = parse_pseud_id(&pseud)?;
    let removed = collaboration::remove_contributor(state.db(), work.id, pseud_id).await?;

    if !removed {
        return Err(ApiError(AppError::field(
            "pseud",
            "That pseud is not a removable contributor of this work.",
        )));
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn list_invitations(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Vec<InvitationView>>> {
    let pseud = acting_pseud(&user)?;
    let invites = collaboration::pending_invites_for_pseud(state.db(), pseud).await?;
    Ok(Json(
        invites.into_iter().map(InvitationView::from).collect(),
    ))
}

async fn accept_invitation(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<InvitationView>> {
    respond(&state, &user, &id, true).await
}

async fn decline_invitation(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<InvitationView>> {
    respond(&state, &user, &id, false).await
}

async fn respond(
    state: &AppState,
    user: &SessionUser,
    raw_id: &str,
    accept: bool,
) -> ApiResult<Json<InvitationView>> {
    let pseud = acting_pseud(user)?;
    let invite_id: CollaborationInviteId = raw_id.parse().map_err(|_| {
        ApiError(AppError::NotFound {
            resource: "invitation",
        })
    })?;

    let invite = collaboration::find_invite(state.db(), invite_id)
        .await?
        .ok_or_else(|| {
            ApiError(AppError::NotFound {
                resource: "invitation",
            })
        })?;

    // Only the pseud that was invited may answer, and it may answer as that
    // pseud only: accepting under a different face would grant rights to a
    // pseud that was never offered them.
    if invite.invited_pseud_id != pseud {
        return Err(ApiError(AppError::NotFound {
            resource: "invitation",
        }));
    }

    let answered = collaboration::respond_to_invite(state.db(), &invite, accept).await?;
    if !answered {
        return Err(ApiError(AppError::field(
            "invitation",
            "That invitation has already been answered.",
        )));
    }

    let updated = collaboration::find_invite(state.db(), invite_id)
        .await?
        .ok_or_else(|| {
            ApiError(AppError::NotFound {
                resource: "invitation",
            })
        })?;

    Ok(Json(updated.into()))
}

async fn revoke_invitation(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let invite_id: CollaborationInviteId = id.parse().map_err(|_| {
        ApiError(AppError::NotFound {
            resource: "invitation",
        })
    })?;

    let invite = collaboration::find_invite(state.db(), invite_id)
        .await?
        .ok_or_else(|| {
            ApiError(AppError::NotFound {
                resource: "invitation",
            })
        })?;

    let (_work, contributors) = owned_work(&state, &user, &invite.work_id.to_string()).await?;

    if let Decision::Deny(reason) = manage_decision(&user, &contributors) {
        return Err(refusal(reason));
    }

    let revoked = collaboration::revoke_invite(state.db(), invite_id).await?;
    if !revoked {
        return Err(ApiError(AppError::field(
            "invitation",
            "That invitation has already been answered.",
        )));
    }

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn acting_pseud(user: &SessionUser) -> ApiResult<PseudId> {
    user.pseud_id.ok_or(ApiError(AppError::AccessDenied))
}

fn parse_pseud_id(raw: &str) -> ApiResult<PseudId> {
    raw.parse()
        .map_err(|_| ApiError(AppError::NotFound { resource: "pseud" }))
}

/// Load a work *through* the acting pseud's contributions.
async fn owned_work(
    state: &AppState,
    user: &SessionUser,
    raw_id: &str,
) -> ApiResult<(lorehaven_db::content::Work, Vec<Contributor>)> {
    let work_id: WorkId = raw_id
        .parse()
        .map_err(|_| ApiError(AppError::NotFound { resource: "work" }))?;

    let work = lorehaven_db::content::find_work(state.db(), work_id)
        .await?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "work" }))?;

    let contributors = collaboration::contributors_for_work(state.db(), work_id).await?;
    let pseud = acting_pseud(user)?;

    if !contributors.iter().any(|c| c.pseud_id == pseud) {
        return Err(ApiError(AppError::NotFound { resource: "work" }));
    }

    Ok((work, contributors))
}

fn manage_decision(user: &SessionUser, contributors: &[Contributor]) -> Decision {
    match user.pseud_id {
        Some(pseud_id) => can_manage_contributors(
            &lorehaven_domain::policy::Actor {
                account_id: user.account_id,
                pseud_id,
                age_state: user.age_state,
                trusted_reviewer: false,
            },
            contributors,
        ),
        None => Decision::Deny(DenyReason::NotAuthenticated),
    }
}

fn refusal(reason: DenyReason) -> ApiError {
    tracing::debug!(reason = reason.as_str(), "contributor management denied");
    match reason {
        DenyReason::NotAContributor | DenyReason::NotAuthenticated => {
            ApiError(AppError::NotFound { resource: "work" })
        }
        _ => ApiError(AppError::AccessDenied),
    }
}
