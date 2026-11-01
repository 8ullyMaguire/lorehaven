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
//! The `enabled` flag on each row is the resolved instance decision. Per-account
//! overrides (§59.6, `extension_user_state`) land here next as a second field
//! rather than by changing this one: a client that only understands `enabled`
//! keeps working, and the distinction between "off for everyone" and "off for you"
//! is visible instead of inferred.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::http::ApiResult;
use crate::state::AppState;

/// Module catalogue routes.
pub fn router() -> Router<AppState> {
    Router::new().route("/modules", get(list))
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
async fn list(State(state): State<AppState>) -> ApiResult<Json<Catalogue>> {
    let modules = state.modules();
    let mut rows: Vec<ModuleView> = darkhold_domain::modules::all()
        .iter()
        .map(|ext| ModuleView {
            id: ext.id,
            name: ext.name,
            summary: ext.summary,
            enabled: modules.enabled(ext.id),
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
    }))
}
