//! Settings endpoints (spec §7).
//!
//! ```text
//! GET   /settings/privacy    PATCH /settings/privacy
//! GET   /settings/content    PATCH /settings/content
//! ```
//!
//! Privacy and content are separate endpoints because they answer different
//! questions — "who may see what of mine" versus "what do I want to be shown" —
//! and because they have different ceilings. A privacy change can narrow
//! exposure on its own; a content change is always intersected with what the
//! instance's policy allows, so setting `max_rating: explicit` as a restricted
//! account widens nothing.

use std::collections::BTreeMap;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use lorehaven_db::identity::{self, PrivacyScope};
use lorehaven_db::sessions;
use lorehaven_domain::policy::ContentRating;
use lorehaven_domain::{AppError, PseudId};
use serde::{Deserialize, Serialize};

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::privacy;
use crate::state::AppState;

/// Settings routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/settings/privacy", get(get_privacy).patch(patch_privacy))
        .route("/settings/content", get(get_content).patch(patch_content))
}

// ---------------------------------------------------------------------------
// Privacy
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct PrivacyView {
    /// Account-level settings.
    account: BTreeMap<String, String>,
    /// Pseud-level settings, keyed by pseud identifier.
    pseuds: BTreeMap<String, BTreeMap<String, String>>,
    /// Every recognised key, its permitted values and its description, so the
    /// interface renders what the server accepts rather than maintaining its
    /// own copy that can drift.
    schema: Vec<KeyDescription>,
}

#[derive(Debug, Serialize)]
struct KeyDescription {
    key: &'static str,
    summary: &'static str,
    values: &'static [&'static str],
}

#[derive(Debug, Deserialize)]
struct PatchPrivacy {
    /// Pseud-level setting to change. Absent means account-level.
    #[serde(default)]
    pseud_id: Option<PseudId>,
    /// The settings to change.
    changes: BTreeMap<String, String>,
}

async fn get_privacy(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<PrivacyView>> {
    let account = identity::find_account(state.db(), user.account_id)
        .await?
        .ok_or_else(|| ApiError(AppError::AuthRequired))?;

    let mut account_settings = BTreeMap::new();
    for key in privacy::keys_in(privacy::Scope::Account) {
        // A stored value is used; otherwise the default for this account's age
        // state is reported. The two are distinguished nowhere in the response
        // on purpose: a client should not be able to tell the difference
        // between "chose the default" and "never chose", because that
        // difference is not actionable.
        let stored = identity::privacy_value(
            state.db(),
            PrivacyScope::Account(&user.account_id),
            key.name,
        )
        .await?;
        account_settings.insert(
            key.name.to_owned(),
            stored.unwrap_or_else(|| privacy::default_for(key.name, account.age_state).to_owned()),
        );
    }

    let mut pseud_settings = BTreeMap::new();
    for pseud in identity::pseuds_for_account(state.db(), user.account_id).await? {
        let mut values = BTreeMap::new();
        for key in privacy::keys_in(privacy::Scope::Pseud) {
            let stored =
                identity::privacy_value(state.db(), PrivacyScope::Pseud(&pseud.id), key.name)
                    .await?;
            values.insert(
                key.name.to_owned(),
                stored.unwrap_or_else(|| {
                    privacy::default_for(key.name, account.age_state).to_owned()
                }),
            );
        }
        pseud_settings.insert(pseud.id.to_string(), values);
    }

    Ok(Json(PrivacyView {
        account: account_settings,
        pseuds: pseud_settings,
        schema: privacy::ALL
            .iter()
            .map(|key| KeyDescription {
                key: key.name,
                summary: key.summary,
                values: key.values,
            })
            .collect(),
    }))
}

async fn patch_privacy(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(request): Json<PatchPrivacy>,
) -> ApiResult<Json<PrivacyView>> {
    if request.changes.is_empty() {
        return Err(ApiError(AppError::field(
            "changes",
            "Supply at least one setting to change.",
        )));
    }

    // Validate everything before writing anything: a request that names one
    // good key and one bad one must change nothing.
    for (key, value) in &request.changes {
        let Some(description) = privacy::find(key) else {
            return Err(ApiError(AppError::field(
                key,
                "That is not a recognised privacy setting.",
            )));
        };
        if !privacy::is_valid_value(description, value) {
            return Err(ApiError(AppError::field(
                key,
                format!("{} must be one of: {}.", key, description.values.join(", ")),
            )));
        }
    }

    // Each key belongs to exactly one scope, declared once in `privacy`. A
    // request that names a key at the wrong scope is refused rather than
    // reinterpreted, because guessing would let the same key mean two things.
    let target_scope = if request.pseud_id.is_some() {
        privacy::Scope::Pseud
    } else {
        privacy::Scope::Account
    };

    for key in request.changes.keys() {
        let declared = privacy::find(key).expect("validated above");
        if declared.scope != target_scope {
            let hint = match declared.scope {
                privacy::Scope::Pseud => "This setting applies to one pseud; supply pseud_id.",
                privacy::Scope::Account => "This setting applies to the account, not to one pseud.",
            };
            return Err(ApiError(AppError::field(key, hint)));
        }
    }

    match request.pseud_id {
        Some(pseud_id) => {
            // Ownership check: the pseud must belong to the caller.
            let owned = identity::find_pseud(state.db(), pseud_id)
                .await?
                .is_some_and(|pseud| pseud.account_id == user.account_id);
            if !owned {
                return Err(ApiError(AppError::NotFound { resource: "pseud" }));
            }

            for (key, value) in &request.changes {
                identity::set_privacy(state.db(), PrivacyScope::Pseud(&pseud_id), key, value)
                    .await?;
            }
        }
        None => {
            for (key, value) in &request.changes {
                identity::set_privacy(
                    state.db(),
                    PrivacyScope::Account(&user.account_id),
                    key,
                    value,
                )
                .await?;
            }
        }
    }

    // Return the resulting state, so the client does not have to guess what a
    // partial update produced.
    get_privacy(State(state), RequireSession(user)).await
}

// ---------------------------------------------------------------------------
// Content
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct ContentView {
    /// The rating the reader has chosen.
    max_rating: &'static str,
    /// Warnings the reader never wants to see.
    excluded_warnings: Vec<String>,
    /// The highest rating the instance's policy will show this account,
    /// whatever they choose. Reported so the interface can explain why a
    /// higher preference had no effect.
    policy_ceiling: &'static str,
    /// The rating actually in force: the lower of the two.
    effective_max_rating: &'static str,
    /// Optimistic-concurrency version, required by the next `PATCH`.
    version: i64,
}

#[derive(Debug, Deserialize)]
struct PatchContent {
    /// The version the client believes it is editing (spec §3.4).
    expected_version: i64,
    #[serde(default)]
    max_rating: Option<String>,
    #[serde(default)]
    excluded_warnings: Option<Vec<String>>,
}

async fn get_content(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<ContentView>> {
    let account = identity::find_account(state.db(), user.account_id)
        .await?
        .ok_or_else(|| ApiError(AppError::AuthRequired))?;
    let settings = sessions::content_settings(state.db(), user.account_id).await?;

    Ok(Json(content_view(account.age_state, &settings)))
}

fn content_view(
    age_state: lorehaven_domain::policy::AgeState,
    settings: &sessions::ContentSettings,
) -> ContentView {
    let policy = lorehaven_domain::policy::AccessPolicy::default();
    let ceiling = policy_ceiling(age_state, &policy);

    ContentView {
        max_rating: sessions::rating_name(settings.max_rating),
        excluded_warnings: settings.excluded_warnings.clone(),
        policy_ceiling: sessions::rating_name(ceiling),
        effective_max_rating: sessions::rating_name(ceiling.min(settings.max_rating)),
        version: settings.version,
    }
}

fn policy_ceiling(
    age_state: lorehaven_domain::policy::AgeState,
    policy: &lorehaven_domain::policy::AccessPolicy,
) -> ContentRating {
    match age_state {
        lorehaven_domain::policy::AgeState::DeclaredAdult => policy.adult_max_rating,
        _ => policy.minor_max_rating,
    }
}

async fn patch_content(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(request): Json<PatchContent>,
) -> ApiResult<Json<ContentView>> {
    let account = identity::find_account(state.db(), user.account_id)
        .await?
        .ok_or_else(|| ApiError(AppError::AuthRequired))?;
    let mut settings = sessions::content_settings(state.db(), user.account_id).await?;

    if settings.version != request.expected_version {
        return Err(ApiError(AppError::RevisionConflict {
            expected: request.expected_version,
            actual: settings.version,
        }));
    }

    if let Some(rating) = &request.max_rating {
        if !matches!(rating.as_str(), "general" | "teen" | "mature" | "explicit") {
            return Err(ApiError(AppError::field(
                "max_rating",
                "Choose one of: general, teen, mature, explicit.",
            )));
        }
        // Stored as chosen, not clamped: the ceiling is applied on read, so a
        // reader who later becomes eligible for more does not have to come back
        // and change this again.
        settings.max_rating = sessions::parse_rating(rating);
    }

    if let Some(warnings) = request.excluded_warnings {
        if warnings.len() > 200 {
            return Err(ApiError(AppError::field(
                "excluded_warnings",
                "At most 200 warnings may be excluded.",
            )));
        }
        let mut cleaned = Vec::with_capacity(warnings.len());
        for warning in warnings {
            let trimmed = warning.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.chars().count() > 80 {
                return Err(ApiError(AppError::field(
                    "excluded_warnings",
                    "Each warning may be at most 80 characters.",
                )));
            }
            if trimmed.chars().any(char::is_control) {
                return Err(ApiError(AppError::field(
                    "excluded_warnings",
                    "A warning cannot contain control characters.",
                )));
            }
            cleaned.push(trimmed.to_owned());
        }
        cleaned.sort_unstable();
        cleaned.dedup();
        settings.excluded_warnings = cleaned;
    }

    sessions::save_content_settings(
        state.db(),
        user.account_id,
        &settings,
        Some(request.expected_version),
    )
    .await
    .map_err(|error| {
        // The repository refuses when the version moved between our read and
        // our write; surface that as the documented conflict rather than a
        // generic fault.
        ApiError(AppError::RevisionConflict {
            expected: request.expected_version,
            actual: request.expected_version + 1,
        })
        .tap(error)
    })?;

    let updated = sessions::content_settings(state.db(), user.account_id).await?;
    Ok(Json(content_view(account.age_state, &updated)))
}

/// Small helper so a mapped error can still be logged with its cause.
trait TapError<T> {
    fn tap(self, cause: anyhow::Error) -> T;
}

impl TapError<ApiError> for ApiError {
    fn tap(self, cause: anyhow::Error) -> ApiError {
        tracing::warn!(%cause, "content settings write refused");
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lorehaven_domain::policy::AgeState;

    #[test]
    fn the_policy_ceiling_follows_the_age_state() {
        let policy = lorehaven_domain::policy::AccessPolicy::default();
        assert_eq!(
            policy_ceiling(AgeState::DeclaredAdult, &policy),
            ContentRating::Explicit
        );
        assert_eq!(
            policy_ceiling(AgeState::DeclaredMinor, &policy),
            ContentRating::General
        );
        assert_eq!(
            policy_ceiling(AgeState::Unknown, &policy),
            ContentRating::General
        );
    }

    #[test]
    fn a_preference_above_the_ceiling_has_no_effect() {
        // The reader asks for explicit; the policy says general. The effective
        // value must be the policy's, or the ceiling is decoration.
        let settings = sessions::ContentSettings {
            max_rating: ContentRating::Explicit,
            excluded_warnings: Vec::new(),
            version: 1,
        };
        let view = content_view(AgeState::DeclaredMinor, &settings);

        assert_eq!(view.max_rating, "explicit", "the stored choice is reported");
        assert_eq!(view.policy_ceiling, "general");
        assert_eq!(
            view.effective_max_rating, "general",
            "the effective value must not exceed the policy ceiling"
        );
    }

    #[test]
    fn an_adult_may_widen_up_to_the_policy() {
        let settings = sessions::ContentSettings {
            max_rating: ContentRating::Mature,
            excluded_warnings: vec!["character death".to_owned()],
            version: 3,
        };
        let view = content_view(AgeState::DeclaredAdult, &settings);

        assert_eq!(view.effective_max_rating, "mature");
        assert_eq!(view.version, 3);
        assert_eq!(view.excluded_warnings, vec!["character death".to_owned()]);
    }
}
