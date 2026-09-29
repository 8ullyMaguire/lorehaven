//! Retention administration: the instance setting and its per-source overrides
//! (spec §11.15).
//!
//! ```text
//! GET   /api/v1/admin/retention/policy
//! PATCH /api/v1/admin/retention/policy
//! GET   /api/v1/admin/retention/sources
//! PUT   /api/v1/admin/retention/sources/:sourceKey
//! DELETE /api/v1/admin/retention/sources/:sourceKey
//! ```
//!
//! **The setting is the operator's, and it is stated once.** Nothing an
//! uploader, an adapter, a reader or a federated peer does may raise it, and
//! these four routes are the only place it can be changed. That is why the
//! widening check lives here *and* in the store: the route refuses with a 400
//! naming both settings, and the store refuses again for a caller that skipped
//! the route. §11.15 wants the operator told, and an operator told by an error
//! message is the only version of this that works.
//!
//! **`require_operator` 404s rather than 403s.** The setting says whether this
//! instance holds the text of every work it knows about, so confirming that an
//! operator surface *exists* to an unauthenticated reader is itself a small
//! disclosure about the instance. The rule is the repository's existing one and
//! it is reused rather than reinvented here.
//!
//! **The DELETE route is not in the spec's four.** §11.15's own refusal message
//! — the one a reader sees when a source is overridden — says "Remove the
//! source override in admin settings". An action named by a user-facing message
//! has to exist, or the message is worse than a generic error because it sends
//! the operator to look for something that is not there.

use axum::extract::{Path, State};
// `get(..).patch(..)` and `put(..).delete(..)` are `MethodRouter` builders, so the
// free `patch`/`delete` functions are not needed and importing them reads as
// though a separate route were registered.
use axum::routing::{get, post, put};
use axum::{Json, Router};
use lorehaven_db::retention;
use lorehaven_domain::retention::BodyMode;
use lorehaven_domain::AppError;
use serde::Deserialize;

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::routes::discovery::require_operator;
use crate::state::AppState;

/// Retention administration routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/retention/policy",
            get(get_policy).patch(patch_policy),
        )
        .route("/admin/retention/sources", get(get_sources))
        // §11.15 / amendment §4.2: the operator's preservation-debt number.
        // Operator-only by construction — `require_operator` 404s anybody else,
        // and the amendment says explicitly that this is not public and not
        // through `stats`.
        .route(
            "/admin/retention/works-past-saving",
            get(get_works_past_saving),
        )
        // `{source_key}`, not `:source_key`: axum 0.8's path syntax, and the
        // `:sourceKey` in §11.15's route table is prose naming the segment, not
        // a router pattern. Using the older spelling panics at *router build*
        // time, which is why every test in this file failed together rather than
        // one at a time.
        .route(
            "/admin/retention/sources/{source_key}",
            put(put_source_override).delete(delete_source_override),
        )
        // `post` is imported for the trailing `.post` on the sources route
        // below, which shares the path with `get` and answers a payload-less
        // POST as a 405 rather than a 404 — the distinction matters when an
        // operator's tooling guesses the verb.
        .route("/admin/retention/sources", post(unsupported_verb))
}

/// The request body for a policy change.
#[derive(Debug, Deserialize)]
struct PolicyPatch {
    /// The mode to store.
    ///
    /// Deserialised as a `String` and parsed by hand rather than as a
    /// `BodyMode`, because serde's derived error for an unknown variant names
    /// the *type* and the field rather than saying which two values are legal.
    /// §11.15's refusal requirement is about naming the policy, and an operator
    /// who typed `aggregte` deserves to be told that it is `aggregate` or
    /// `cache` — not that their JSON was invalid.
    body_mode: String,
}

/// The request body for a per-source override.
#[derive(Debug, Deserialize)]
struct SourceOverrideBody {
    body_mode: String,
}

/// The current policy, and what it means for a reader.
async fn get_policy(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    require_operator(&state, &user)?;

    let policy = retention::read_policy(state.db())
        .await
        .map_err(|error| ApiError(AppError::Internal(error)))?;
    let effective = retention::effective_instance_mode(state.db())
        .await
        .map_err(|error| ApiError(AppError::Internal(error)))?;

    Ok(Json(serde_json::json!({
        "body_mode": effective.as_str(),
        // `configured` is separate from `body_mode` on purpose: an instance
        // nobody has touched is *caching* because `cache` is the default, not
        // because anybody chose it, and a dashboard that cannot tell those two
        // apart will report a decision that was never made.
        "configured": policy.is_some(),
        "updated_by": policy.as_ref().and_then(|p| p.updated_by.clone()),
        "updated_at": policy.as_ref().map(|p| p.updated_at.clone()),
        "version": policy.as_ref().map(|p| p.version).unwrap_or(0),
        "available_modes": [BodyMode::Cache.as_str(), BodyMode::Aggregate.as_str()],
    })))
}

/// Record the operator's decision.
async fn patch_policy(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<PolicyPatch>,
) -> ApiResult<Json<serde_json::Value>> {
    require_operator(&state, &user)?;

    let mode = parse_mode(&body.body_mode).map_err(|message| {
        ApiError(AppError::Validation {
            message,
            field_errors: Default::default(),
        })
    })?;

    // `AccountId` is a newtype over `Uuid` with a `From` in that direction, so
    // this is a conversion rather than a parse. Parsing the string form would be
    // a round trip through a representation the session already validated.
    let actor: uuid::Uuid = user.account_id.into();

    let before = retention::effective_instance_mode(state.db())
        .await
        .map_err(|error| ApiError(AppError::Internal(error)))?;
    let policy = retention::write_policy(state.db(), mode, actor)
        .await
        .map_err(|error| ApiError(AppError::Internal(error)))?;

    // The modlog entry is a courtesy *after* the write, not part of it: a
    // failure to record must not undo a decision the operator was shown as
    // saved, and §11.15 requires the change to be recorded — so it is recorded,
    // and a failure to record is logged rather than swallowed silently.
    if let Err(error) = lorehaven_db::governance::audit_append(
        state.db(),
        &user.account_id.to_string(),
        "retention.policy.set",
        "instance_retention_policy",
        "default",
        &serde_json::json!({
            "from": before.as_str(),
            "to": policy.body_mode.as_str(),
        })
        .to_string(),
    )
    .await
    {
        tracing::warn!(
            %error,
            "the retention policy was changed but could not be written to the modlog"
        );
    }

    Ok(Json(serde_json::json!({
        "body_mode": policy.body_mode.as_str(),
        "updated_by": policy.updated_by,
        "updated_at": policy.updated_at,
        "version": policy.version,
    })))
}

/// Every per-source override, and the instance's own mode beside them.
///
/// The instance mode is repeated in this response because the list is the thing
/// an operator reads to decide which override to remove, and a list that does
/// not say what the instance is set to makes "may only narrow" unenforceable
/// from the page.
async fn get_sources(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    require_operator(&state, &user)?;

    let instance = retention::effective_instance_mode(state.db())
        .await
        .map_err(|error| ApiError(AppError::Internal(error)))?;
    let overrides = retention::list_source_overrides(state.db())
        .await
        .map_err(|error| ApiError(AppError::Internal(error)))?;

    Ok(Json(serde_json::json!({
        "instance_body_mode": instance.as_str(),
        "overrides": overrides
            .iter()
            .map(|row| serde_json::json!({
                "source_key": row.source_key,
                "body_mode": row.body_mode.as_str(),
                "updated_by": row.updated_by,
                "updated_at": row.updated_at,
                "version": row.version,
            }))
            .collect::<Vec<_>>(),
    })))
}

/// Narrow one source family.
async fn put_source_override(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(source_key): Path<String>,
    Json(body): Json<SourceOverrideBody>,
) -> ApiResult<Json<serde_json::Value>> {
    require_operator(&state, &user)?;

    let mode = parse_mode(&body.body_mode).map_err(|message| {
        ApiError(AppError::Validation {
            message,
            field_errors: Default::default(),
        })
    })?;

    // `AccountId` is a newtype over `Uuid` with a `From` in that direction, so
    // this is a conversion rather than a parse. Parsing the string form would be
    // a round trip through a representation the session already validated.
    let actor: uuid::Uuid = user.account_id.into();

    match retention::write_source_override(state.db(), &source_key, mode, actor).await {
        Ok(row) => {
            let _ = lorehaven_db::governance::audit_append(
                state.db(),
                &user.account_id.to_string(),
                "retention.source.set",
                "instance_retention_source_override",
                &source_key,
                &serde_json::json!({ "body_mode": row.body_mode.as_str() }).to_string(),
            )
            .await;
            Ok(Json(serde_json::json!({
                "source_key": row.source_key,
                "body_mode": row.body_mode.as_str(),
                "updated_by": row.updated_by,
                "updated_at": row.updated_at,
                "version": row.version,
            })))
        }
        // **400, not 403 and not 500.** The operator's own rule refused their
        // input: they asked to widen a source past the instance setting, and
        // §11.15 says the fix is to change the instance setting — which this
        // response names. A 500 would send them to the logs, and a 403 would say
        // they are not allowed, which is false: they are allowed, the value
        // they sent is the part that is wrong.
        Err(retention::OverrideWriteError::Widening(refused)) => {
            Err(ApiError(AppError::Validation {
                // Built from the refused *fields*, not from the refusal's
                // `Display`. The first version interpolated `refused` and then
                // appended this sentence, which produced the message twice:
                // "a per-source override may only narrow: ao3 cannot be set to
                // cache while the instance is set to aggregate A per-source
                // override may only narrow: ...". The `Display` impl is for
                // `anyhow` chains and logs; a user-facing message that wants to
                // end with an instruction has to be composed from the fields.
                message: format!(
                    "{} cannot be set to {} while the instance is set to {}. A per-source \
                     override may only narrow; change the instance policy at \
                     /api/v1/admin/retention/policy instead.",
                    refused.source_key,
                    refused.attempted.as_str(),
                    refused.instance.as_str()
                ),
                field_errors: Default::default(),
            }))
        }
        Err(retention::OverrideWriteError::Storage(message)) => Err(ApiError(AppError::Internal(
            anyhow::anyhow!("could not write the retention override: {message}"),
        ))),
    }
}

/// Remove one source's override, returning to the instance's setting.
async fn delete_source_override(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(source_key): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    require_operator(&state, &user)?;

    let removed = retention::clear_source_override(state.db(), &source_key)
        .await
        .map_err(|error| ApiError(AppError::Internal(error)))?;

    // 404 rather than an idempotent 200, because the spec is a resource and a
    // DELETE of something absent has not found it. An operator's tooling that
    // treats 404 as "already absent" is correct; one that treats 200 as "done"
    // when nothing changed is not, and the difference shows up in a migration
    // script that believes it removed an override it never set.
    if !removed {
        return Err(ApiError(AppError::NotFound {
            resource: "retention source override",
        }));
    }

    let _ = lorehaven_db::governance::audit_append(
        state.db(),
        &user.account_id.to_string(),
        "retention.source.clear",
        "instance_retention_source_override",
        &source_key,
        "{}",
    )
    .await;

    Ok(Json(
        serde_json::json!({ "source_key": source_key, "removed": true }),
    ))
}

/// A POST to a route that only answers GET.
///
/// Registered so that the shared path returns `405 Method Not Allowed` rather
/// than the router's own `404`, which would tell a client's author that the
/// surface does not exist when it does. The body says which verbs are real.
async fn unsupported_verb() -> ApiError {
    ApiError(AppError::Validation {
        message: "this endpoint answers GET; use PUT to set a source override".to_owned(),
        field_errors: Default::default(),
    })
}

/// How many works this instance is currently past saving (§11.15, §4.2).
///
/// **Operator only, and that is the amendment's own instruction** rather than a
/// habit: "It changes no behaviour and it is not public." The count is also
/// deliberately NOT added to `/admin/stats`, which answers anonymous callers
/// with a stub — a preservation-debt figure on a surface an unauthenticated
/// reader can reach would be a disclosure about what this instance has failed
/// to keep.
///
/// The response carries the instance's mode beside the count, because a zero on
/// a `cache` instance and a zero on an `aggregate` instance are different facts:
/// the first means nothing is at risk, the second means nothing has been at risk
/// yet. Reporting both as `0` makes the number uninterpretable, which is how a
/// metric nobody can read becomes a metric nobody reads.
async fn get_works_past_saving(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    require_operator(&state, &user)?;

    let past = lorehaven_db::retention::works_past_saving(state.db())
        .await
        .map_err(|error| ApiError(AppError::Internal(error)))?;

    Ok(Json(serde_json::json!({
        "works_past_saving": past.count,
        "instance_body_mode": past.instance.as_str(),
    })))
}

/// Parse a mode, naming the two legal values in the refusal.
///
/// `BodyMode::parse_stored` is what the *store* uses, and it returns `None` for
/// anything unrecognised — correct for a row read at startup, where the caller's
/// fallback is a policy decision. Here the caller is an operator who typed
/// something, and the useful error names the two words they could have typed.
/// Reusing the parser and adding the message keeps the two in step: a third
/// value added to the enum appears in this list without anybody editing it.
fn parse_mode(value: &str) -> Result<BodyMode, String> {
    BodyMode::parse_stored(Some(value)).ok_or_else(|| {
        format!(
            "body_mode must be one of {}, not {value:?}",
            [BodyMode::Cache.as_str(), BodyMode::Aggregate.as_str()].join(" or ")
        )
    })
}
