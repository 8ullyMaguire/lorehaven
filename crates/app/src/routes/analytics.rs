//! Trust-gated analytics over HTTP (`docs/spec-amendments/trust-gated-analytics.md`).
//!
//! Two endpoints, and the reason there are only two is the point:
//!
//! ```text
//! GET /api/v1/me/analytics                 what this viewer may see
//! GET /api/v1/me/analytics/{capability}    one of them
//! ```
//!
//! # The list endpoint is the design
//!
//! The obvious shape is one route per metric, each with its own `if trust >=
//! 3` check. That works right up until a metric is added and the check is
//! forgotten, and then the failure is a `200`. There are 57 capabilities in
//! the registry; 57 hand-written checks is 57 opportunities to be the one that
//! forgot, and the forgot is invisible in review because the check looks
//! right.
//!
//! So there is no per-metric authorisation anywhere in this file. A route asks
//! [`allowed_under`] and renders what it is told. The list endpoint and the
//! single-metric endpoint read the *same* function, which is why a capability
//! can never appear in one and not the other.
//!
//! # 403 versus 404
//!
//! A denied capability is `403`, and an unknown one is `404`. That distinction
//! is deliberate and it cuts against convenience. Collapsing them would mean a
//! prober could not tell a name this instance has not implemented from one it
//! has but withheld — and the set of names an instance has implemented is
//! itself information. The never-shown list stays structural because those
//! names are not scopes, so they parse to `None` and are indistinguishable
//! from any other typo.
//!
//! # What the response carries
//!
//! Every response states the capability's definition, its freshness, its
//! approximation, the trust level it needs, and whether it is about the viewer
//! or about other people (§24.2). The floor is not a server-side secret: a
//! reader told "fewer than 10" can see the rule, and a client can render it
//! without hardcoding a number.

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::auth::RequirePseud;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use lorehaven_domain::analytics::{allowed_under, Preset, Role, Scope, Subject};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me/analytics", get(my_analytics))
        .route("/me/analytics/{capability}", get(one_capability))
}

fn internal(e: impl Into<anyhow::Error>) -> ApiError {
    ApiError(lorehaven_domain::AppError::Internal(e.into()))
}

fn not_found() -> ApiError {
    ApiError(lorehaven_domain::AppError::NotFound {
        resource: "analytics capability",
    })
}

fn denied(detail: String) -> ApiError {
    let _ = detail;
    ApiError(lorehaven_domain::AppError::AccessDenied)
}

/// The §24.2 metadata block, from the registry rather than from a string here.
///
/// The floor is included deliberately. A client that has to hardcode 5 or 10 to
/// render "fewer than N" will get it wrong on one surface, and the wrong one
/// will be the surface nobody tested.
fn meta(scope: Scope) -> Value {
    let m = scope.method();
    json!({
        "name": scope.as_str(),
        "definition": m.definition,
        "freshness": m.freshness.as_str(),
        "approximation": m.approximation,
        "minimum_trust_level": scope.minimum_trust(),
        "subject": match scope.subject() {
            Subject::Self_ => "self",
            Subject::Other => "other",
        },
        "floor": scope.floor(),
    })
}

/// The viewer's trust level, role and instance preset.
///
/// Three separate things, and conflating them is what the registry exists to
/// prevent:
///
/// * the level comes from the governance table,
/// * the role is not a level at all — an operator is an operator because
///   `operator_role` says so, and that is what keeps the admin panel a grant
///   rather than a score,
/// * the preset is an operator's upper bound and can only remove capabilities.
async fn viewer(state: &AppState, account: &str) -> ApiResult<(i64, Role, Preset)> {
    let trust = lorehaven_db::governance::trust_for(state.db(), account)
        .await
        .map_err(internal)? as i64;
    let role = if lorehaven_db::governance::has_operator_role(state.db(), account)
        .await
        .map_err(internal)?
    {
        Role::Admin
    } else {
        Role::Reader
    };
    Ok((trust, role, preset_for(state)))
}

/// The instance preset (§0.6), mapped onto the analytics vocabulary.
///
/// The configured presets are a different axis from the analytics ones: they
/// describe what kind of library this is, not how much of its analytics is
/// visible. Two of them constrain analytics and the rest do not:
///
/// * `curated_boutique` is a gallery — public counters and personal stats, no
///   community dashboards,
/// * `open_library` is the standard archive set.
///
/// A preset that this function does not recognise falls back to `Archive`
/// rather than to the most restrictive value, because an operator upgrading
/// with a new preset name should not silently lose their analytics — and the
/// trust ladder still applies either way. Presets can only *remove*
/// capabilities; they can never raise a trust level's ceiling.
fn preset_for(state: &AppState) -> Preset {
    match state.config().instance.preset.as_str() {
        "curated_boutique" | "gallery" => Preset::Gallery,
        _ => Preset::Archive,
    }
}

/// `GET /api/v1/me/analytics` — what this viewer may see.
///
/// A dashboard renders this list. It is the only enumeration of the surface and
/// it comes from the same function the single-metric route consults, so the two
/// can never disagree.
pub async fn my_analytics(
    State(state): State<AppState>,
    RequirePseud { user, pseud_id }: RequirePseud,
) -> ApiResult<Json<Value>> {
    let (trust, role, preset) = viewer(&state, &user.account_id.to_string()).await?;

    let capabilities: Vec<Value> = lorehaven_domain::analytics::visible_to(trust, role, preset)
        .into_iter()
        .map(meta)
        .collect();

    Ok(Json(json!({
        "viewer": {
            "trust_level": trust,
            "role": match role {
                Role::Reader => "reader",
                Role::Trustee => "trustee",
                Role::Admin => "admin",
            },
            "preset": preset.as_str(),
        },
        "pseud": pseud_id.to_string(),
        "capabilities": capabilities,
    })))
}

/// The viewer's own reading totals, for `own.reading.basic`.
///
/// The counts are the caller's own, so there is no floor to apply and no band
/// to report: `Subject::Self_` means banding these would be telling somebody
/// something false about themselves.
async fn reading_value(state: &AppState, account_id: &str) -> ApiResult<Value> {
    // `AppError: From<anyhow::Error>` already exists, so the `?` converts and
    // the cause survives into the 500's log line. A hand-written `map_err`
    // that discarded `e` would throw away the only clue the reader's server has
    // about which statement failed.
    let totals = lorehaven_db::analytics::reading_totals(state.db(), account_id).await?;
    // The named fields are nested under the capability's own key rather than
    // laid out flat. Two capabilities with different value shapes share one
    // `value` object, and flat fields would collide by name -- the second
    // metric to land would have to be called `chapters_read_weekly` or the
    // client would have to know which shape it is looking at.
    Ok(json!({
        "status": "ok",
        "reading": {
            "finished_works": totals.finished_works,
            "chapters_read": totals.chapters_read,
            "words_read": totals.words_read,
            "reading_seconds": totals.reading_seconds,
        },
    }))
}

/// `GET /api/v1/me/analytics/{capability}` — one capability.
pub async fn one_capability(
    State(state): State<AppState>,
    RequirePseud { user, .. }: RequirePseud,
    Path(capability): Path<String>,
) -> ApiResult<Json<Value>> {
    // An unknown name is a client bug, and it is a 404 — including for the
    // never-shown names, which are not scopes and so do not parse. A prober
    // cannot tell `ab.signal_weights` from a typo, which is the point.
    let scope = Scope::parse(&capability).ok_or_else(not_found)?;

    let (trust, role, preset) = viewer(&state, &user.account_id.to_string()).await?;

    // The one authorisation decision in this file.
    if !allowed_under(scope, trust, role, preset) {
        return Err(denied(format!(
            "{} needs trust level {}",
            scope.as_str(),
            scope.minimum_trust()
        )));
    }

    // `implemented` is derived from the scope, never hardcoded per route. A
    // hardcoded `false` is a claim that has to be edited in two places when a
    // query lands, and the failure is a dashboard that reports a number as
    // "not implemented" for a capability the instance can answer.
    let value = match scope {
        Scope::OwnReadingBasic => reading_value(&state, &user.account_id.to_string()).await?,
        _ => json!({
            "status": "not_implemented",
            "note": "registered and gated; no query behind it yet",
        }),
    };
    let implemented = !value["status"].is_null();

    Ok(Json(json!({
        "capability": capability,
        "implemented": implemented,
        "meta": meta(scope),
        "value": value,
    })))
}
