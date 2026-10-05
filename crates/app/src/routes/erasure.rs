//! M45-51 — subject access and erasure.
//!
//! ```text
//! GET  /api/v1/me/data       — everything held about the calling reader
//! POST /api/v1/me/erasure    — erase the calling reader
//! ```
//!
//! Spec: `docs/plans/m45-51-subject-access-and-erasure.md`.
//!
//! ## Both routes are `RequireSession`, and there is no admin variant
//!
//! A subject-access request is by definition a request about *the caller*. An operator asking
//! "what do you hold on reader X" is a different request with a different legal basis, and it
//! belongs to the notice/DSAR tooling (M45-55), not here. Having an operator-only copy of this
//! module would put a second, unaudited path to a reader's data on the same router — so there
//! is one route and it reads the session, never a path parameter.
//!
//! ## The 409 the house `AppError` cannot express
//!
//! Erasing an account that has published works is refused without `force`, because
//! `works.owner_pseud_id` is `ON DELETE CASCADE` and the erasure would take the writing with
//! it. That refusal needs a **409**, which `AppError` has no variant for — adding one would
//! change the error contract for 200 other routes to serve this one case. So `erase` returns
//! `Response` and builds the status itself. The alternative, inventing a `Validation` with a
//! misleading message, is worse: a 422 says "your request was malformed" when the request was
//! well-formed and the answer is "not yet".
//!
//! ## Two confirmations, and why the second is not ceremony
//!
//! `confirm_handle` must match one of the caller's handles. A session cookie is a weak
//! confirmation for an irreversible action, and this is the only irreversible route in the
//! module. `confirm_published_works` is a separate flag rather than a field on the first,
//! because they answer different questions: the first is *are you sure this is you*, the
//! second is *do you know this destroys your writing*. A reader who typed their handle without
//! realising what would be lost should still be stopped, and one who set the second without
//! the first should not get through either.

use std::collections::BTreeMap;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

use lorehaven_db::erasure::{erase_account, plan_erasure, subject_data, SubjectData};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me/data", get(get_data))
        .route("/me/erasure", post(post_erasure))
}

/// Everything held about the calling reader.
///
/// 200 with a JSON object whose sections are always arrays — never null, never absent. A
/// client that has to branch on null-ness is a client with a bug, and an absent section reads
/// as "we do not hold that" when it may mean "we forgot to ask".
///
/// The disclosure set is assembled in `db::erasure::subject_data`, whose field names are
/// checked against `FORBIDDEN_SCOPE_NAMES` by a store test. That test walks the serialised
/// keys, so adding a forbidden field to `SubjectData` turns it red on both engines.
async fn get_data(
    State(state): State<AppState>,
    session: RequireSession,
) -> ApiResult<Json<SubjectData>> {
    let data = subject_data(state.db(), &session.0.account_id.to_string()).await?;
    Ok(Json(data))
}

/// The request body.
///
/// `deny_unknown_fields` so a typo in `force` is a refusal that names the field, rather than
/// an erasure that half-happened because an unrecognised key was ignored. That asymmetry is
/// the whole reason this struct exists: on an irreversible route, silently ignoring what it
/// did not understand is the dangerous default.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ErasureBody {
    /// One of the caller's handles, typed back. See the module docs.
    confirm_handle: Option<String>,
    /// Acknowledges that published works will be deleted.
    #[serde(default)]
    confirm_published_works: bool,
    /// Set by the client that read the plan and is proceeding anyway. With no published
    /// works this is not needed; with them, both flags are required.
    #[serde(default)]
    force: bool,
}

/// What the erasure did, or what it would do.
///
/// The same shape serves the dry run (`force` absent, works outstanding → 409) and the
/// completion (200), so a client renders one shape and the `deleted` flag distinguishes them.
/// That matters more than it looks: two response shapes means two renderers, and one of them
/// is the one a reader sees at the moment they are deciding.
async fn post_erasure(
    State(state): State<AppState>,
    session: RequireSession,
    body: Option<Json<ErasureBody>>,
) -> Result<Response, ApiError> {
    let body = body.map(|Json(b)| b).unwrap_or(ErasureBody {
        confirm_handle: None,
        confirm_published_works: false,
        force: false,
    });
    let account_id = session.0.account_id.to_string();
    let plan = plan_erasure(state.db(), &account_id).await?;

    // Confirmation of identity. Checked before anything is counted or reported, so a caller
    // who has not confirmed learns nothing about the account beyond what they already have.
    if let Some(confirm) = body.confirm_handle.as_deref() {
        if !plan
            .handle
            .as_deref()
            .is_some_and(|h| h.eq_ignore_ascii_case(confirm))
        {
            return Err(validation(
                "confirm_handle does not match any handle on this account",
                "confirm_handle",
            ));
        }
    }

    // The refusal that needs a 409: published work would be deleted with the account.
    //
    // Not a block and not a warning — a 409 that says exactly what would be lost. Silently
    // orphaning a published work is worse than a refusal, and silently refusing is worse than
    // being told, because the reader cannot then choose.
    if plan.published_work_count > 0 && !(body.force && body.confirm_published_works) {
        return Ok(Response {
            status: StatusCode::CONFLICT,
            body: Json(json!({
                "deleted": false,
                "reason": "published_works_would_be_deleted",
                "published_work_count": plan.published_work_count,
                "handle": plan.handle,
                "message": format!(
                    "Erasing this account deletes {} published work(s), because a work belongs to \\
                     the pseudonym that wrote it and the pseudonym belongs to you. Re-send with \\
                     confirm_published_works and force if that is what you want.",
                    plan.published_work_count,
                ),
            })),
        });
    }

    erase_account(state.db(), &account_id).await?;

    Ok(Response {
        status: StatusCode::OK,
        body: Json(json!({
            "deleted": true,
            "private_rows_erased": plan.total_private_rows(),
            "published_work_count": plan.published_work_count,
            "cancelled_open_exports": plan.open_export_count,
        })),
    })
}

/// `AppError` has no 409, so this handler builds its own `Response`. See the module docs.
struct Response {
    status: StatusCode,
    body: Json<Value>,
}

// `Response` is not axum's `IntoResponse`; this is the adapter, and it is the only place the
// status is chosen.
impl axum::response::IntoResponse for Response {
    fn into_response(self) -> axum::response::Response {
        (self.status, self.body).into_response()
    }
}

fn validation(message: &str, field: &str) -> ApiError {
    let mut field_errors = BTreeMap::new();
    field_errors.insert(field.to_string(), message.to_string());
    ApiError(lorehaven_domain::AppError::Validation {
        message: message.to_string(),
        field_errors,
    })
}