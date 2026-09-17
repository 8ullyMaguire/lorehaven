//! Pseud endpoints (spec §7).
//!
//! ```text
//! GET    /pseuds              list the account's own pseuds
//! POST   /pseuds              create one
//! PATCH  /pseuds/:id          edit one (own only)
//! POST   /pseuds/:id/activate make it this session's active pseud
//! GET    /pseuds/:handle      public profile
//! ```
//!
//! The security property this module exists to hold: **no response here ever
//! contains `account_id`**, and every mutation is scoped by the authenticated
//! account in the query rather than checked afterwards. Spec §7 acceptance:
//! "An account cannot edit another account's pseud" and "Pseud linkage is
//! absent from public API responses."
//!
//! The distinction that keeps this honest: `Pseud::account_id` is loaded and
//! used for authorization, then dropped. It is never serialised.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use lorehaven_db::identity::{self, PrivacyScope};
use lorehaven_domain::policy::AgeState;
use lorehaven_domain::{AppError, PseudId};
use serde::{Deserialize, Serialize};

use crate::auth::{MaybeSession, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::privacy;
use crate::state::AppState;

/// Pseud routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/pseuds", get(list_pseuds).post(create_pseud))
        .route("/pseuds/{id}", patch(update_pseud))
        .route("/pseuds/{id}/activate", post(activate_pseud))
        .route("/pseuds/{id}/profile", get(public_profile))
}

/// A pseud as its owner sees it.
#[derive(Debug, Serialize)]
pub struct OwnPseudView {
    /// Identifier.
    pub id: PseudId,
    /// Unique handle.
    pub handle: String,
    /// Display name.
    pub display_name: String,
    /// Biography.
    pub bio: Option<String>,
    /// `listed` or `hidden`.
    pub discoverability: String,
    /// Optimistic-concurrency version, required by the next `PATCH`.
    pub version: i64,
    /// Creation time, RFC 3339.
    pub created_at: String,
    /// Whether this is the session's active pseud.
    pub active: bool,
}

/// A pseud as the public sees it.
///
/// Note what is absent: no account, no email, no session, no version, and no
/// indication of whether this pseud shares an owner with any other.
#[derive(Debug, Serialize)]
pub struct PublicPseudView {
    /// Identifier.
    pub id: PseudId,
    /// Handle.
    pub handle: String,
    /// Display name.
    pub display_name: String,
    /// Biography.
    pub bio: Option<String>,
    /// Creation time, RFC 3339.
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
struct CreatePseudRequest {
    handle: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    bio: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdatePseudRequest {
    /// The version the client believes it is editing (spec §3.4).
    expected_version: i64,
    /// The fields to change. Absent fields are left alone.
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    bio: Option<String>,
    #[serde(default)]
    discoverability: Option<String>,
}

async fn list_pseuds(
    State(state): State<AppState>,
    MaybeSession(_user): MaybeSession,
    crate::auth::RequireSession(user): crate::auth::RequireSession,
) -> ApiResult<Json<Vec<OwnPseudView>>> {
    let pseuds = identity::pseuds_for_account(state.db(), user.account_id).await?;

    Ok(Json(
        pseuds
            .into_iter()
            .map(|pseud| OwnPseudView {
                active: user.pseud_id == Some(pseud.id),
                id: pseud.id,
                handle: pseud.handle,
                display_name: pseud.display_name,
                bio: pseud.bio,
                discoverability: pseud.discoverability.as_str().to_owned(),
                version: pseud.version,
                created_at: pseud.created_at,
            })
            .collect(),
    ))
}

async fn create_pseud(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(request): Json<CreatePseudRequest>,
) -> ApiResult<(StatusCode, Json<OwnPseudView>)> {
    // The same constraint the age policy places on writing applies here: an
    // account that cannot publish does not need more public faces.
    if !may_hold_pseuds(user.age_state) {
        return Err(ApiError(AppError::AccessDenied));
    }

    let handle = crate::routes::auth::validate_handle(&request.handle)?;

    if identity::find_pseud_by_handle(state.db(), &handle)
        .await?
        .is_some()
    {
        return Err(ApiError(AppError::field(
            "handle",
            "That handle is already taken.",
        )));
    }

    let display_name = request
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&handle)
        .to_owned();
    crate::routes::auth::validate_display_name(&display_name)?;
    let bio = sanitise_bio(request.bio.as_deref())?;

    let pseud_id =
        identity::create_pseud(state.db(), user.account_id, &handle, &display_name).await?;

    if let Some(bio) = &bio {
        set_bio(&state, pseud_id, Some(bio.as_str())).await?;
    }

    // A new pseud starts from the same stored defaults as the first one, so a
    // second pseud is never accidentally more public than the first.
    identity::set_privacy(
        state.db(),
        PrivacyScope::Pseud(&pseud_id),
        privacy::PUBLIC_BOOKMARKS.name,
        privacy::default_for(privacy::PUBLIC_BOOKMARKS.name, user.age_state),
    )
    .await?;
    identity::set_privacy(
        state.db(),
        PrivacyScope::Pseud(&pseud_id),
        privacy::PUBLIC_FOLLOWS.name,
        privacy::default_for(privacy::PUBLIC_FOLLOWS.name, user.age_state),
    )
    .await?;

    let created = identity::find_pseud_by_handle(state.db(), &handle)
        .await?
        .ok_or_else(|| {
            ApiError(AppError::Internal(anyhow::anyhow!(
                "pseud vanished after creation"
            )))
        })?;

    Ok((
        StatusCode::CREATED,
        Json(OwnPseudView {
            active: false,
            id: created.id,
            handle: created.handle,
            display_name: created.display_name,
            bio,
            discoverability: created.discoverability.as_str().to_owned(),
            version: created.version,
            created_at: created.created_at,
        }),
    ))
}

async fn update_pseud(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Json(request): Json<UpdatePseudRequest>,
) -> ApiResult<Json<OwnPseudView>> {
    let pseud_id: PseudId = id
        .parse()
        .map_err(|_| ApiError(AppError::NotFound { resource: "pseud" }))?;

    // **The ownership check.** Loading through the account means a pseud
    // belonging to someone else is not found, rather than found-and-refused:
    // the caller learns nothing about whether it exists.
    let pseud = owned_pseud(&state, user.account_id, pseud_id)
        .await?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "pseud" }))?;

    if pseud.version != request.expected_version {
        return Err(ApiError(AppError::RevisionConflict {
            expected: request.expected_version,
            actual: pseud.version,
        }));
    }

    if let Some(discoverability) = &request.discoverability {
        if discoverability != "listed" && discoverability != "hidden" {
            return Err(ApiError(AppError::field(
                "discoverability",
                "Choose either listed or hidden.",
            )));
        }
        // An account that may not be listed cannot list itself through this
        // endpoint either — the age policy is not bypassable by a second route.
        if discoverability == "listed" && !may_be_listed(user.age_state) {
            return Err(ApiError(AppError::AccessDenied));
        }
    }

    let display_name = match &request.display_name {
        Some(name) => {
            let trimmed = name.trim();
            crate::routes::auth::validate_display_name(trimmed)?;
            Some(trimmed.to_owned())
        }
        None => None,
    };
    let bio = sanitise_bio(request.bio.as_deref())?;

    let affected = identity::update_pseud(
        state.db(),
        user.account_id,
        pseud_id,
        request.expected_version,
        display_name.as_deref(),
        bio.as_deref(),
    )
    .await?;

    if !affected {
        // The version changed between the load above and the write: another
        // device got there first.
        let current = owned_pseud(&state, user.account_id, pseud_id).await?;
        return Err(ApiError(AppError::RevisionConflict {
            expected: request.expected_version,
            actual: current.map_or(0, |p| p.version),
        }));
    }

    if let Some(discoverability) = &request.discoverability {
        identity::set_pseud_discoverability(state.db(), pseud_id, discoverability).await?;
    }

    let updated = owned_pseud(&state, user.account_id, pseud_id)
        .await?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "pseud" }))?;

    Ok(Json(OwnPseudView {
        active: user.pseud_id == Some(updated.id),
        id: updated.id,
        handle: updated.handle,
        display_name: updated.display_name,
        bio: updated.bio,
        discoverability: updated.discoverability.as_str().to_owned(),
        version: updated.version,
        created_at: updated.created_at,
    }))
}

async fn activate_pseud(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let pseud_id: PseudId = id
        .parse()
        .map_err(|_| ApiError(AppError::NotFound { resource: "pseud" }))?;

    // Same ownership check as above. Without it, this endpoint would let anyone
    // act as anyone.
    owned_pseud(&state, user.account_id, pseud_id)
        .await?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "pseud" }))?;

    // Session-scoped: another device signed into the same account keeps the
    // pseud it chose.
    lorehaven_db::sessions::set_active_pseud(state.db(), user.session_id, pseud_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

async fn public_profile(
    State(state): State<AppState>,
    MaybeSession(_user): MaybeSession,
    Path(id): Path<String>,
) -> ApiResult<Json<PublicPseudView>> {
    // A handle that looks like a UUID still resolves by handle first, so the
    // identifier path is only taken when the handle lookup finds nothing.
    let pseud = identity::find_pseud_by_handle(state.db(), &id).await?;

    let pseud = match pseud {
        Some(pseud) => pseud,
        None => {
            let pseud_id: PseudId = id
                .parse()
                .map_err(|_| ApiError(AppError::NotFound { resource: "pseud" }))?;
            identity::find_pseud(state.db(), pseud_id)
                .await?
                .ok_or_else(|| ApiError(AppError::NotFound { resource: "pseud" }))?
        }
    };

    // A hidden pseud is a 404, not a 403: the site does not confirm that a
    // profile exists but is withheld (spec §3.3).
    if !pseud.discoverability.is_listed() {
        return Err(ApiError(AppError::NotFound { resource: "pseud" }));
    }

    Ok(Json(PublicPseudView {
        id: pseud.id,
        handle: pseud.handle,
        display_name: pseud.display_name,
        bio: pseud.bio,
        created_at: pseud.created_at,
    }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Whether this age state permits holding pseuds at all.
///
/// Reading is available to everyone; a public identity is a form of
/// participation, and spec §7 only permits that for an account we may
/// lawfully serve.
#[must_use]
pub fn may_hold_pseuds(age_state: AgeState) -> bool {
    age_state.may_participate()
}

/// Whether this age state permits appearing in the directory.
#[must_use]
pub fn may_be_listed(age_state: AgeState) -> bool {
    age_state.may_participate()
}

/// Load a pseud, but only if the account owns it.
async fn owned_pseud(
    state: &AppState,
    account_id: lorehaven_domain::AccountId,
    pseud_id: PseudId,
) -> ApiResult<Option<identity::Pseud>> {
    let pseud = identity::find_pseud(state.db(), pseud_id).await?;
    Ok(pseud.filter(|pseud| pseud.account_id == account_id))
}

/// Reject a bio that would be unusable or dangerous downstream.
fn sanitise_bio(bio: Option<&str>) -> ApiResult<Option<String>> {
    let Some(bio) = bio else {
        return Ok(None);
    };
    let trimmed = bio.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > 2000 {
        return Err(ApiError(AppError::field(
            "bio",
            "A bio may be at most 2000 characters.",
        )));
    }
    if trimmed
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(ApiError(AppError::field(
            "bio",
            "A bio cannot contain control characters.",
        )));
    }
    Ok(Some(trimmed.to_owned()))
}

async fn set_bio(state: &AppState, pseud_id: PseudId, bio: Option<&str>) -> ApiResult<()> {
    identity::set_pseud_bio(state.db(), pseud_id, bio).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restricted_account_may_not_hold_pseuds() {
        assert!(!may_hold_pseuds(AgeState::Restricted));
        assert!(!may_hold_pseuds(AgeState::DeclaredMinor));
        assert!(!may_hold_pseuds(AgeState::Unknown));
        assert!(may_hold_pseuds(AgeState::DeclaredAdult));
        assert!(may_hold_pseuds(AgeState::AuthorizedUnderPolicy));
    }

    #[test]
    fn bios_are_length_and_control_checked() {
        assert_eq!(sanitise_bio(None).expect("none"), None);
        assert_eq!(sanitise_bio(Some("  ")).expect("blank"), None);
        assert_eq!(
            sanitise_bio(Some("  Writes things.  ")).expect("ok"),
            Some("Writes things.".to_owned())
        );
        // Newlines and tabs are fine in prose.
        assert!(sanitise_bio(Some("line one\nline two")).is_ok());
        // Other control characters are not.
        assert!(sanitise_bio(Some("bell\u{7}")).is_err());
        assert!(sanitise_bio(Some(&"x".repeat(2001))).is_err());
    }
}
