//! §59 — the module catalogue.
//!
//! ```text
//! GET  /api/v1/modules         what this instance runs, and what it could run
//! ```
//!
//! This is the surface §6.3's slots need: a page asks once, and hides the links to
//! what is not on. It is deliberately **not** an operator surface — it answers with
//! ids, names and states, never with the operator's reasoning, and it is core
//! (§59.2) because a catalogue that could itself be switched off would be a page
//! that cannot find out why it is empty.
//!
//! The `enabled` flag on each row is the resolved **instance** decision. A caller's
//! own narrowing is a second field, `enabled_for_caller`, rather than a change to
//! the first: a client that only understands `enabled` keeps working, and the
//! difference between "off for everyone" and "off for you" stays visible instead of
//! inferred. `overrides` names the rows the caller changed, so a settings page
//! renders what was changed without diffing two lists.
//!
//! `PATCH /api/v1/modules/{id}` is the only way a reader writes one of those rows,
//! and it can only narrow (§59.3): `enabled: false` writes, `enabled: true` deletes.
//! The gate in `server.rs` reads the written row back on the way into a `per_user`
//! surface — one primary-key read, and only for a caller with a session.

use axum::extract::{Path, State};
use axum::routing::{get, patch};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::auth::{MaybeSession, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use darkhold_domain::AppError;

/// Module catalogue routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/modules", get(list))
        // A reader's own override (§59.6). `per_user` rows only.
        .route("/modules/{id}", patch(set_one))
}

/// One row of the catalogue.
#[derive(Debug, Serialize)]
struct ModuleView {
    id: &'static str,
    name: &'static str,
    summary: &'static str,
    enabled: bool,
    stability: &'static str,
    toggle: &'static str,
    /// Whether **this caller** may reach the surface: `enabled` narrowed by their
    /// own row. Equal to `enabled` for an anonymous caller — there is nobody to
    /// hold a preference.
    enabled_for_caller: bool,
    milestone: &'static str,
    spec: &'static str,
    nav: Vec<NavView>,
}

/// A navigation entry belonging to an extension.
#[derive(Debug, Serialize)]
struct NavView {
    label: &'static str,
    href: &'static str,
    slot: &'static str,
}

/// The catalogue payload.
#[derive(Debug, Serialize)]
struct Catalogue {
    /// Whether the instance lets readers hold their own overrides (§59.3).
    allow_user_overrides: bool,
    core: &'static [&'static str],
    /// Every registered surface, ordered: what this instance runs first, then by
    /// id. A disabled row is still listed — that is the point, an operator reading
    /// the payload should find the surface they meant to switch on.
    modules: Vec<ModuleView>,
    /// Every idea the project has had and not built (§59.7), most relevant first.
    ///
    /// Separate from `modules` because the two answer different questions: a
    /// module is a surface this build can switch, an idea is work that would
    /// create one. A client renders a module list; an operator reads the ideas to
    /// decide what to build next, so each row carries the documents it came from
    /// and the score that ordered it.
    ideas: Vec<IdeaView>,
    /// Counts a page can render without walking both arrays.
    counts: Counts,
    /// The module ids this caller has switched off (§59.6). Empty for an anonymous
    /// caller and for the reader who changed nothing, which is most of them.
    overrides: Vec<String>,
}

/// What a `PATCH` answers: the row, and where its state came from.
#[derive(Debug, Serialize)]
struct OverrideView {
    id: &'static str,
    /// The caller's resolved state after the write.
    enabled_for_caller: bool,
    /// The instance's decision, unchanged by the write — said out loud so a reader
    /// who switched something on cannot believe they switched on a surface the
    /// operator switched off.
    instance_enabled: bool,
    /// Whether the caller now holds a row for this module.
    overridden: bool,
}

/// The `PATCH` body.
#[derive(Debug, Deserialize)]
struct OverrideRequest {
    /// `false` narrows; `true` clears the caller's row and inherits again.
    enabled: bool,
}

/// One unbuilt idea.
#[derive(Debug, Serialize)]
struct IdeaView {
    id: &'static str,
    name: &'static str,
    summary: &'static str,
    relevance: u8,
    source: &'static str,
    milestone: &'static str,
}

/// How many of each.
#[derive(Debug, Serialize)]
struct Counts {
    modules: usize,
    enabled: usize,
    ideas: usize,
}

/// `GET /api/v1/modules`.
async fn list(State(state): State<AppState>, MaybeSession(session): MaybeSession) -> ApiResult<Json<Catalogue>> {
    let modules = state.modules();
    // One query for the whole page rather than one per row: the caller's changes are
    // a short list, and an empty list is the common case.
    let overrides = match &session {
        Some(user) if modules.allow_user_overrides() => {
            darkhold_db::module_state::overrides_for(state.db(), &user.account_id.to_string())
                .await
                .map_err(|error| ApiError(AppError::Internal(error)))?
        }
        _ => Vec::new(),
    };
    let mut rows: Vec<ModuleView> = darkhold_domain::modules::all()
        .iter()
        .map(|ext| ModuleView {
            id: ext.id,
            name: ext.name,
            summary: ext.summary,
            enabled: modules.enabled(ext.id),
            // The gate's rule, not a second one: only a `per_user` row narrows
            // anything, so a stray row written directly into the table must not
            // make the catalogue report a surface as gone for its caller.
            enabled_for_caller: modules.enabled(ext.id)
                && !(ext.toggle.is_per_user() && overrides.iter().any(|off| off == ext.id)),
            stability: ext.stability.as_str(),
            toggle: match ext.toggle {
                darkhold_domain::modules::Toggle::Fixed => "fixed",
                darkhold_domain::modules::Toggle::Operator => "operator",
                darkhold_domain::modules::Toggle::PerUser => "per_user",
            },
            milestone: ext.milestone,
            spec: ext.spec,
            nav: ext
                .nav
                .iter()
                .map(|n| NavView {
                    label: n.label,
                    href: n.href,
                    slot: n.slot,
                })
                .collect(),
        })
        .collect();
    // Enabled first, then id: the common question is "what does this instance do",
    // and the alphabetical answer buries `works` among seventy off-by-default rows.
    rows.sort_by(|a, b| b.enabled.cmp(&a.enabled).then_with(|| a.id.cmp(b.id)));

    let ideas: Vec<IdeaView> = darkhold_domain::modules::ideas()
        .iter()
        .map(|idea| IdeaView {
            id: idea.id,
            name: idea.name,
            summary: idea.summary,
            relevance: idea.relevance,
            source: idea.source,
            milestone: idea.milestone,
        })
        .collect();
    let counts = Counts {
        modules: rows.len(),
        enabled: rows.iter().filter(|row| row.enabled).count(),
        ideas: ideas.len(),
    };

    Ok(Json(Catalogue {
        allow_user_overrides: modules.allow_user_overrides(),
        core: darkhold_domain::modules::core(),
        modules: rows,
        ideas,
        counts,
        overrides,
    }))
}

/// `PATCH /api/v1/modules/{id}` — a reader's own override (§59.6).
///
/// Three refusals, each for a different reason and each with its own code: unknown id
/// is `404` (nobody's mistake, the thing is not here), a row whose switch belongs to
/// the operator or the build is `403` naming who decides, and a planned row is `422`
/// because the question is about work that does not exist yet.
async fn set_one(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Json(body): Json<OverrideRequest>,
) -> ApiResult<Json<OverrideView>> {
    let module = darkhold_domain::modules::find(&id)
        .ok_or(ApiError(AppError::NotFound { resource: "module" }))?;
    module.may_user_toggle().map_err(ApiError)?;

    let instance_enabled = state.modules().enabled(module.id);
    if !state.modules().allow_user_overrides() {
        // The instance has switched the whole layer off. 403 rather than silently
        // doing nothing: a settings page that saves a value the server discards is
        // worse than one that says it cannot.
        return Err(ApiError(AppError::AccessDenied));
    }

    let account = user.account_id.to_string();
    // `overridden` is the reader's state AFTER the write, not what the write did:
    // the first version returned `clear`'s "was there a row" here, so switching a
    // surface back on answered `overridden: true` and `enabled_for_caller: false` —
    // the opposite of the truth, and exactly what the test caught.
    let overridden = if body.enabled {
        // Widening is not a thing a reader can do (§59.3): `true` means "my row goes
        // away", and the instance's answer applies again — which may still be off.
        darkhold_db::module_state::clear(state.db(), &account, module.id)
            .await
            .map_err(|error| ApiError(AppError::Internal(error)))?;
        false
    } else {
        let now = darkhold_db::identity::now_rfc3339();
        darkhold_db::module_state::set_off(state.db(), &account, module.id, &now)
            .await
            .map_err(|error| ApiError(AppError::Internal(error)))?;
        true
    };

    Ok(Json(OverrideView {
        id: module.id,
        enabled_for_caller: instance_enabled && !overridden,
        instance_enabled,
        overridden,
    }))
}
