//! Imports, the source catalogue and source credentials (spec §11.3, §11.6).
//!
//! ```text
//! GET    /imports/sources                     the catalogue this build ships
//! POST   /imports/preview                     detect, fetch metadata, plan; writes nothing
//! POST   /imports                             enqueue an import
//! POST   /imports/batch                       enqueue one import per URL in a plain-text list
//! GET    /imports?cursor=…&state=…            the caller's own imports, envelope
//! GET    /imports/:id                         one import, with its chapters
//! POST   /imports/:id/cancel                  ask the worker to stop
//! POST   /imports/:id/retry-failed-chapters   re-fetch only what failed
//! GET    /library/items                       the caller's own imported items
//! GET    /source-credentials                  metadata only, never a secret
//! POST   /source-credentials                  store one, encrypted
//! DELETE /source-credentials/:id              revoke it
//! POST   /source-credentials/:id/test         check it against the source
//! POST   /admin/sources/health                operators only; sweep source health
//! ```
//!
//! Five rules this module exists to hold:
//!
//! * **A credential's plaintext never leaves the server.** Every response here
//!   is built from metadata: a label, a status, a date. There is no code path
//!   that serialises a secret, which is a stronger guarantee than remembering
//!   to skip a field.
//! * **A preview writes nothing.** It runs before a reader has confirmed
//!   anything, so it must not leave an import, an item or a chapter behind —
//!   otherwise a curious paste would fill a library.
//! * **An import's destination is the caller's private library** unless the
//!   request says otherwise, and only `library` is accepted today. Spec §11.2:
//!   a source login demonstrates access, not permission to republish.
//! * **A caller sees their own imports and nobody else's**, so the id on the
//!   path is never sufficient on its own.
//! * **A source the catalogue says is not working is refused before it is
//!   queued.** Spec §11.8's health states are a promise to the reader; a source
//!   that has failed three times with no success is unavailable, and an import
//!   into it is a queued job that is certain to fail.
//!
//! The preview is the one place a reader's URL reaches the network during a
//! request. That is deliberate — the alternative is a spinner around an
//! out-of-band poll, and the reader confirmed a plan they could not see — and it
//! is safe because the fetch goes through the same guard the worker uses, with a
//! tighter timeout because somebody is waiting for it.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

use lorehaven_db::{imports, library, revisions, secrets};
use lorehaven_domain::imports::{plan_import, ChapterIdentity, ImportedWork};
use lorehaven_domain::library::ReadingStatus;
use lorehaven_domain::{AppError, PseudId};
use lorehaven_scrapers::{SafeFetcher, SourceAdapter, SourceKey};

use crate::auth::{MaybeSession, RequirePseud, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

/// The catalogue and the preview.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/imports/sources", get(list_sources))
        .route("/imports/preview", post(preview_import))
        .route("/imports", get(list_imports).post(start_import))
        .route("/imports/batch", post(start_import_batch))
        .route("/imports/{id}", get(get_import))
        .route("/imports/{id}/cancel", post(cancel_import))
        .route("/library/imports/csv", post(import_shelf_csv))
        .route(
            "/imports/{id}/retry-failed-chapters",
            post(retry_failed_chapters),
        )
        .route(
            "/source-credentials",
            get(list_credentials).post(store_credential),
        )
        .route(
            "/source-credentials/{id}",
            axum::routing::delete(delete_credential),
        )
        .route("/source-credentials/{id}/test", post(test_credential))
}

/// The operator surface for the source catalogue.
///
/// Separate from [`router`] because it is gated on configuration rather than on
/// a session: `config.administration.operator_account_id` names one account, and
/// a non-operator is answered `404` rather than `403` — confirming that an
/// operator surface exists is itself a disclosure (spec §11.8, and the same rule
/// `/admin/jobs` follows).
pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/admin/sources/health", post(sweep_source_health))
        .route(
            "/admin/sources/revisions",
            get(revision_cache_stats).delete(clear_revision_cache),
        )
        .route("/admin/sources/revisions/purge", post(purge_revision_cache))
}

/// What the revision cache is holding.
///
/// Reported rather than inferred. The number is the only way to tell a cache
/// that is working from one that is silently empty, and an empty cache looks
/// exactly like a healthy one from every other endpoint: imports still succeed,
/// they just cost a request each.
async fn revision_cache_stats(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    require_operator(&state, &user)?;
    let total = revisions::count(state.db()).await?;
    Ok(Json(serde_json::json!({
        "entries": total,
        "ttl_seconds": state.config().revisions.ttl_secs,
    })))
}

/// Drop the entries whose expiry has passed.
///
/// Only the entries. The bytes they pointed at are left for the collector to
/// judge, because `content_blobs` is shared with the snapshots a reader is
/// actually reading — a cache that deleted its own blobs would be a cache that
/// could delete a chapter.
async fn purge_revision_cache(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    require_operator(&state, &user)?;
    let purged = revisions::purge_expired(state.db()).await?;
    Ok(Json(serde_json::json!({ "purged": purged })))
}

/// Empty the cache.
///
/// Safe by construction and still worth an operator's decision: it costs
/// requests, not correctness. Deliberately *not* wired into the maintenance
/// worker, so nothing empties the cache on its own.
async fn clear_revision_cache(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    require_operator(&state, &user)?;
    let cleared = revisions::clear(state.db()).await?;
    Ok(Json(serde_json::json!({ "cleared": cleared })))
}

/// Recompute every source's health from the import history.
///
/// The sweep runs automatically after each import; this route is how an
/// operator asks for it without waiting for traffic — after fixing a source,
/// say, or after a run of failures from a source nobody has retried.
async fn sweep_source_health(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    require_operator(&state, &user)?;
    let changes = imports::recompute_source_health(state.db(), imports::HEALTH_WINDOW_DAYS).await?;
    Ok(Json(serde_json::json!({
        "sources_considered": changes.len(),
        "changes": changes
            .into_iter()
            .map(|change| serde_json::json!({
                "key": change.key,
                "previous": change.previous,
                "current": change.current,
                "completed": change.completed,
                "failed": change.failed,
                "changed": change.previous != change.current,
            }))
            .collect::<Vec<_>>(),
    })))
}

/// Whether this account may use the operator surface.
///
/// Duplicated from `jobs.rs` rather than shared: it is nine lines, and a shared
/// helper would be one more thing to keep in step with the trust level that
/// replaces it in Milestone 13.
fn require_operator(state: &AppState, user: &crate::auth::SessionUser) -> ApiResult<()> {
    let configured = state.config().administration.operator_account_id;
    if configured == Some(user.account_id) {
        return Ok(());
    }
    tracing::debug!(
        operator_configured = configured.is_some(),
        "an operator route was reached by an account that is not the operator"
    );
    Err(ApiError(AppError::NotFound { resource: "page" }))
}

/// Refuse a source the catalogue says cannot be used (spec §11.8).
///
/// Two states are refused and they are refused differently, because the reader's
/// next move is different. A source an operator switched off is a policy the
/// reader cannot change, so the message carries the operator's own reason. A
/// source that has failed repeatedly is a fact about the source, so the message
/// says so and says that trying again later is reasonable.
async fn refuse_unusable_source(state: &AppState, source_key: &str) -> ApiResult<()> {
    let Some(source) = imports::find_source(state.db(), source_key).await? else {
        // No row is not a refusal: a source this build knows but the instance
        // has not synced yet is usable, and inventing a refusal for it would
        // make a fresh install unable to import at all.
        return Ok(());
    };

    if !source.enabled {
        let reason = source
            .disabled_reason
            .unwrap_or_else(|| "switched off by an operator, with no reason recorded".to_owned());
        return Err(ApiError(AppError::SourceUnavailable {
            domain: format!("the {source_key} source is switched off: {reason}"),
        }));
    }

    if source.health == "unavailable" {
        return Err(ApiError(AppError::SourceUnavailable {
            domain: format!(
                "the {source_key} source has failed {} times in the last {} days with no success, so it is \
                 being left alone for now; try again later, or check the source's own page",
                imports::FAILURES_TO_UNAVAILABLE,
                imports::HEALTH_WINDOW_DAYS
            ),
        }));
    }

    Ok(())
}

/// Refuse a source this instance has no way of reaching.
///
/// Separate from [`refuse_unusable_source`] because the two are different
/// failures: that one is about the *source's* condition (switched off, or failing
/// for everybody), and this one is about this instance's own capability. Both are
/// decided before anything is queued, so neither turns into a job that fails a
/// page at a time while a reader waits.
fn refuse_unreachable_source(state: &AppState, adapter: &dyn SourceAdapter) -> ApiResult<()> {
    match state.config().imports.unreachable_reason(adapter) {
        Some(reason) => Err(ApiError(AppError::SourceUnavailable { domain: reason })),
        None => Ok(()),
    }
}

/// How many rows a page holds.
const PAGE: i64 = 50;

/// How long a reader waits for a preview.
///
/// Shorter than the worker's, because a person is watching this one. The point
/// of the shorter clock is that the answer "that source is slow" arrives while
/// the reader is still interested, rather than after they have given up and
/// reloaded, which would start a second fetch.
const PREVIEW_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

// ---------------------------------------------------------------------------
// The catalogue
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct SourceView {
    key: String,
    display_name: String,
    adapter_version: String,
    enabled: bool,
    disabled_reason: Option<String>,
    health: String,
    last_checked_at: Option<String>,
    /// What the adapter can do. Absence is visible here rather than discovered
    /// when an import fails (spec §11.1, "Capability absence must be visible").
    capabilities: serde_json::Value,
    /// The terms this instance reads sources on (spec §11.5).
    robots: RobotsView,
}

/// How this instance treats the `Disallow` rules in a source's `robots.txt`.
///
/// Carried in the catalogue because an override that lives only in a config file
/// and a log line is one a reader cannot see and an operator can forget. It is
/// instance-wide, so every entry repeats it — which is the point: the answer does
/// not depend on which source is being looked at, and an operator should not have
/// to work out whether it does.
#[derive(Debug, Serialize)]
struct RobotsView {
    /// Whether a path a source's `robots.txt` forbids is refused.
    honour_disallow: bool,
    /// Whether the pace the same file publishes is still enforced. Always true.
    ///
    /// A constant rather than a configuration value, and it is stated here so
    /// that switching `honour_disallow` off cannot read as switching off the
    /// politeness rules beside it.
    honour_crawl_delay: bool,
    /// What happens to a forbidden path while `honour_disallow` is false.
    note: &'static str,
}

impl RobotsView {
    fn of(config: &crate::config::Config) -> Self {
        Self {
            honour_disallow: config.imports.honour_robots,
            honour_crawl_delay: true,
            note: if config.imports.honour_robots {
                "paths a source's robots.txt forbids are refused, and the failure names the rule"
            } else {
                "this instance reads paths a source's robots.txt forbids, under                  `imports.honour_robots = false`; the source's own crawl delay is still enforced"
            },
        }
    }
}

/// The catalogue a reader is shown: every source this build can read, with this
/// instance's own state laid over it.
///
/// # Why it is driven by the registry and not by the rows
///
/// It used to be the other way round, and on a fresh instance it reported
/// nothing at all. The `sources` table is *instance* state — whether an operator
/// switched a source off, what its health is, when it was last checked — and
/// nothing creates a row for a source that has never been used. So a build with
/// three working adapters offered a reader an empty list, and the import page's
/// own "What this instance can read" section said nothing, while the preview for
/// the very same URLs would have worked. The page was wrong about the instance
/// in the direction that stops a reader trying.
///
/// The build knows what it can read. The row knows what the operator has since
/// decided. So the adapter list comes first and the row refines it, and a row
/// whose source this build cannot read is still reported — with `known: false`,
/// because a record that a source exists is not a claim that it can be read.
async fn list_sources(
    State(state): State<AppState>,
    MaybeSession(_user): MaybeSession,
) -> ApiResult<Json<serde_json::Value>> {
    let rows = imports::list_sources(state.db()).await?;
    let registry = state.registry();

    let mut items: Vec<SourceView> = registry
        .adapters()
        .iter()
        .map(|adapter| {
            let key = adapter.key();
            let row = rows.iter().find(|row| row.key == key.as_str());
            SourceView {
                key: key.as_str().to_owned(),
                // The row wins when it has a name: an operator who renamed a
                // source meant it, and the build's name is only the default.
                display_name: row.map_or_else(
                    || adapter.display_name().to_owned(),
                    |row| row.display_name.clone(),
                ),
                adapter_version: row.map_or_else(
                    || "unrecorded".to_owned(),
                    |row| row.adapter_version.clone(),
                ),
                // No row is not "disabled": it means nobody has ever switched
                // this source off, which is the only thing `enabled` records.
                enabled: row.is_none_or(|row| row.enabled),
                disabled_reason: row.and_then(|row| row.disabled_reason.clone()),
                health: row.map_or_else(|| "unknown".to_owned(), |row| row.health.clone()),
                last_checked_at: row.and_then(|row| row.last_checked_at.clone()),
                capabilities: capabilities_of(adapter.as_ref()),
                // The instance's own answer to every source's `robots.txt`,
                // carried on each entry because this is the page an operator
                // reads before wondering why one archive refuses. Instance-wide
                // and therefore identical everywhere, which is the point: a
                // per-source answer would be one an operator could set once and
                // later be wrong about (spec §11.5).
                robots: RobotsView::of(state.config()),
            }
        })
        .collect();

    // A row for a source this build cannot read. Reported rather than hidden:
    // somebody recorded it, and a catalogue that silently dropped it would make
    // an operator's own note about a source disappear.
    items.extend(
        rows.iter()
            .filter(|row| registry.by_key(&SourceKey::new(row.key.clone())).is_err())
            .map(|row| SourceView {
                key: row.key.clone(),
                display_name: row.display_name.clone(),
                adapter_version: row.adapter_version.clone(),
                enabled: row.enabled,
                disabled_reason: row.disabled_reason.clone(),
                health: row.health.clone(),
                last_checked_at: row.last_checked_at.clone(),
                capabilities: serde_json::json!({ "known": false }),
                robots: RobotsView::of(state.config()),
            }),
    );

    items.sort_by(|left, right| left.key.cmp(&right.key));
    Ok(Json(serde_json::json!({ "items": items })))
}

/// What an adapter can do, as the catalogue reports it.
fn capabilities_of(adapter: &dyn SourceAdapter) -> serde_json::Value {
    let capabilities = adapter.capabilities();
    serde_json::json!({
        "known": true,
        "metadata": capabilities.metadata,
        "chapters": capabilities.chapters,
        "per_chapter_fetch": capabilities.per_chapter_fetch,
        "bibliography": capabilities.bibliography,
        "incremental": capabilities.incremental,
        "authentication": capabilities.authentication.as_str(),
        // What the source requires of a client, so an operator can see that a
        // source needs a solver *before* a reader pastes a URL into it
        // (spec §11.1). A source whose wall this instance cannot satisfy is
        // refused at preview and at start with the reason.
        "wall": match adapter.wall() {
            lorehaven_scrapers::Wall::None => "none",
            lorehaven_scrapers::Wall::Fingerprint => "fingerprint",
            lorehaven_scrapers::Wall::Solver => "solver",
        },
    })
}

/// Wrap a fetcher so a preview reuses what the source says has not changed.
///
/// The scope mirrors the worker's: a read made with a stored credential is filed
/// under the pseud it belongs to, and an anonymous read under `public`. Two
/// readers' credentialed reads must not share entries, which is the whole reason
/// the scope is part of the cache key (spec §10.4).
async fn caching_fetcher<'a>(
    state: &'a AppState,
    fetcher: SafeFetcher,
    source_key: &str,
    scope: &str,
) -> crate::revisions::CachingFetcher<'a, SafeFetcher> {
    let adapter_version = imports::find_source(state.db(), source_key)
        .await
        .ok()
        .flatten()
        .map_or_else(|| "0".to_owned(), |record| record.adapter_version);
    crate::revisions::CachingFetcher::new(
        fetcher,
        state.db(),
        state.config().storage.root.clone(),
        source_key,
        &adapter_version,
        scope,
        state.config().revisions.ttl_secs,
    )
}

// ---------------------------------------------------------------------------
// Preview
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct PreviewRequest {
    url: String,
    /// The pseud the import would run as. Absent means the session's active
    /// pseud, so the common case does not have to repeat it.
    #[serde(default)]
    pseud_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct PlanView {
    /// `create`, `update` or `no_change`.
    plan: String,
    added: usize,
    removed: usize,
    reordered: usize,
    retitled: usize,
    /// The individual changes, so the reader can see *what* would change and
    /// not only how many things would.
    changes: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct PreviewView {
    source_key: String,
    source_work_key: String,
    source_url: String,
    title: String,
    author_text: String,
    author_url: Option<String>,
    summary: String,
    language: Option<String>,
    word_count: Option<i64>,
    status: String,
    chapter_count: usize,
    chapters: Vec<ChapterRefView>,
    /// What the import would do, so the confirmation screen is not a guess.
    plan: PlanView,
    /// Whether this destination would create something new.
    is_new: bool,
    /// Whether an identical work appears to be held already under another
    /// source. A warning, not a refusal: the reader decides.
    duplicate_warning: Option<String>,
}

#[derive(Debug, Serialize)]
struct ChapterRefView {
    ordinal: u32,
    source_chapter_key: String,
    title: String,
}

async fn preview_import(
    State(state): State<AppState>,
    RequirePseud { user, pseud_id }: RequirePseud,
    Json(request): Json<PreviewRequest>,
) -> ApiResult<Json<PreviewView>> {
    let pseud = match &request.pseud_id {
        Some(raw) => parse_pseud(raw)?,
        None => pseud_id,
    };
    let url = parse_import_url(&request.url)?;
    let adapter = state.registry().route(&url).map_err(|_| {
        ApiError(AppError::Validation {
            message: "no source in this build handles that address".to_owned(),
            field_errors: Default::default(),
        })
    })?;
    // A preview is cheap for us and not for the source. Asking one the
    // catalogue says is unavailable spends a request to learn what the
    // catalogue already knew.
    refuse_unusable_source(&state, adapter.key().as_str()).await?;
    refuse_unreachable_source(&state, adapter)?;

    // The same guard the worker uses. A preview is not a lesser fetch: if this
    // is safe to run later then it is safe to run now, and if it is not, the
    // reader finds out before anything is queued.
    let mut policy = crate::imports::policy_for(adapter, state.config());
    policy.timeout = PREVIEW_TIMEOUT;
    let fetcher = SafeFetcher::new(adapter.hosts(), policy);

    // A preview uses a stored credential when the source needs one. It never
    // writes one, and it never reports one.
    let credential = match adapter.capabilities().authentication {
        lorehaven_scrapers::AuthKind::None => None,
        _ => load_credential_for(&state, &pseud, adapter.key().as_str()).await?,
    };
    let scope = if credential.is_some() {
        pseud.to_string()
    } else {
        crate::revisions::PUBLIC_SCOPE.to_owned()
    };
    let fetcher = caching_fetcher(&state, fetcher, adapter.key().as_str(), &scope).await;

    let work = adapter
        .preview(&fetcher, &url, credential.as_ref())
        .await
        .map_err(preview_error)?;

    let existing_item = imports::find_library_item(
        state.db(),
        &user.account_id.to_string(),
        adapter.key().as_str(),
        &work.source_work_key,
    )
    .await?;
    let existing_chapters = imports::previous_chapters_for(
        state.db(),
        &user.account_id.to_string(),
        adapter.key().as_str(),
        &work.source_work_key,
    )
    .await?;

    let held = existing_item.as_ref().map(|item| ImportedWork {
        title: item.title.clone(),
        author_text: item.author_text.clone(),
        chapters: existing_chapters
            .clone()
            .unwrap_or_default()
            .iter()
            .map(|chapter| ChapterIdentity {
                source_chapter_key: chapter.source_chapter_key.clone(),
                ordinal: u32::try_from(chapter.ordinal).unwrap_or(u32::MAX),
                title: chapter.title.clone(),
                word_count: None,
            })
            .collect(),
        word_count: None,
    });

    let plan = plan_import(
        held.as_ref(),
        &ImportedWork {
            title: work.title.clone(),
            author_text: work.author_text.clone(),
            chapters: work
                .chapters
                .iter()
                .map(|chapter| ChapterIdentity {
                    word_count: None,
                    source_chapter_key: chapter.source_chapter_key.clone(),
                    ordinal: chapter.ordinal,
                    title: chapter.title.clone(),
                })
                .collect(),
            word_count: None,
        },
    );

    let duplicate_warning = find_duplicate(&state, &user.account_id.to_string(), &work).await?;

    Ok(Json(PreviewView {
        source_key: work.source_key.to_string(),
        source_work_key: work.source_work_key.clone(),
        source_url: work.source_url.clone(),
        title: work.title.clone(),
        author_text: work.author_text.clone(),
        author_url: work.author_url.clone(),
        summary: work.summary.clone(),
        language: work.language.clone(),
        word_count: work.word_count,
        status: work.status.as_str().to_owned(),
        chapter_count: work.chapters.len(),
        chapters: work
            .chapters
            .iter()
            .map(|chapter| ChapterRefView {
                ordinal: chapter.ordinal,
                source_chapter_key: chapter.source_chapter_key.clone(),
                title: chapter.title.clone(),
            })
            .collect(),
        plan: describe_plan(&plan),
        is_new: plan.is_creation(),
        duplicate_warning,
    }))
}

/// Warn when an apparent copy of this work is already in the library.
///
/// Matched on normalised title and author, across every source, because the
/// duplicate a reader cares about is the one they already have — not the one
/// under the same source key, which the plan already knows about.
async fn find_duplicate(
    state: &AppState,
    account_id: &str,
    work: &lorehaven_scrapers::SourceWork,
) -> ApiResult<Option<String>> {
    let items = imports::list_library_items(state.db(), account_id, 200, None).await?;
    let candidate = ImportedWork {
        title: work.title.clone(),
        author_text: work.author_text.clone(),
        chapters: Vec::new(),
        word_count: None,
    };
    for item in items {
        if item.source_key == work.source_key.as_str()
            && item.source_work_key == work.source_work_key
        {
            continue;
        }
        let held = ImportedWork {
            title: item.title.clone(),
            author_text: item.author_text.clone(),
            chapters: Vec::new(),
            word_count: None,
        };
        if lorehaven_domain::imports::looks_like_a_duplicate(&candidate, &held) {
            return Ok(Some(format!(
                "you already have {:?} from {}, which looks like the same work",
                item.title, item.source_key
            )));
        }
    }
    Ok(None)
}

/// Render a plan for the interface.
fn describe_plan(plan: &lorehaven_domain::imports::ImportPlan) -> PlanView {
    use lorehaven_domain::imports::{ChapterChange, ImportPlan};
    let (name, changes) = match plan {
        ImportPlan::Create => ("create", Vec::new()),
        ImportPlan::NoChange => ("no_change", Vec::new()),
        ImportPlan::Update { changes } => (
            "update",
            changes
                .iter()
                .map(|change| match change {
                    ChapterChange::Added { chapter } => serde_json::json!({
                        "kind": "added",
                        "ordinal": chapter.ordinal,
                        "title": chapter.title,
                    }),
                    ChapterChange::Removed { chapter } => serde_json::json!({
                        "kind": "removed",
                        "ordinal": chapter.ordinal,
                        "title": chapter.title,
                    }),
                    ChapterChange::Reordered {
                        source_chapter_key,
                        from,
                        to,
                    } => serde_json::json!({
                        "kind": "reordered",
                        "source_chapter_key": source_chapter_key,
                        "from": from,
                        "to": to,
                    }),
                    ChapterChange::Retitled {
                        source_chapter_key,
                        was,
                        now,
                    } => serde_json::json!({
                        "kind": "retitled",
                        "source_chapter_key": source_chapter_key,
                        "was": was,
                        "now": now,
                    }),
                })
                .collect(),
        ),
    };
    PlanView {
        plan: name.to_owned(),
        added: plan.added(),
        removed: plan.removed(),
        reordered: plan.reordered(),
        retitled: plan.retitled(),
        changes,
    }
}

/// Turn a fetch failure into something a reader can act on.
///
/// The status is chosen by whose problem it is. A work that does not exist is
/// the reader's typo and gets a validation error; a source that is refusing
/// requests is nobody's fault and gets `SourceUnavailable`, because a reader
/// who is told "invalid" will edit a URL that was correct.
fn preview_error(error: lorehaven_scrapers::SourceError) -> ApiError {
    use lorehaven_scrapers::SourceError as E;
    match error {
        E::NotFound => ApiError(AppError::NotFound { resource: "work" }),
        E::AuthRequired(detail) => ApiError(AppError::Validation {
            message: format!(
                "that work needs a signed-in session at the source; save a credential first ({detail})"
            ),
            field_errors: Default::default(),
        }),
        E::Unsupported(detail) => ApiError(AppError::Validation {
            message: format!("this build cannot read that page: {detail}"),
            field_errors: Default::default(),
        }),
        E::Parse(detail) => ApiError(AppError::Validation {
            message: format!("the page did not look like a work this build can read: {detail}"),
            field_errors: Default::default(),
        }),
        // Rate limiting, blocking and network faults are all "the source is not
        // answering us right now", which is what `SourceUnavailable` means.
        E::RateLimited(_) => ApiError(AppError::SourceUnavailable {
            domain: "the source".to_owned(),
        }),
        E::Blocked => ApiError(AppError::SourceUnavailable {
            domain: "the source (it refused the request as automated traffic)".to_owned(),
        }),
        // The source answered and the answer was no: a moderation hold, a work
        // withdrawn by its author, a takedown in progress. Not `NotFound`, which
        // would send a reader to edit a URL that is correct, and not a
        // `Validation` error for the same reason. `SourceUnavailable` is the
        // closest existing shape — the work is not coming from that source today
        // — and the detail says whose decision it was.
        E::Withheld(detail) => ApiError(AppError::SourceUnavailable {
            domain: format!("the source ({detail})"),
        }),
        E::Network(detail) => ApiError(AppError::SourceUnavailable {
            domain: format!("the source ({detail})"),
        }),
        E::Refused(detail) => ApiError(AppError::Validation {
            message: format!("the source refused the request: {detail}"),
            field_errors: Default::default(),
        }),
        E::Internal(detail) => ApiError(AppError::Internal(anyhow::anyhow!(detail))),
    }
}

// ---------------------------------------------------------------------------
// Starting an import
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct StartImportRequest {
    url: String,
    #[serde(default)]
    pseud_id: Option<String>,
    /// Only `library` is accepted. Spec §11.2 requires an explicit destination,
    /// and the other three (own draft, republication with permission, approved
    /// preservation batch) belong to milestones that can enforce them.
    #[serde(default = "default_destination")]
    destination: String,
    /// Import the metadata and report, without storing chapters.
    #[serde(default)]
    dry_run: bool,
    /// The preview's own plan, echoed back. When present and different from the
    /// plan the server computes, the import is refused: the reader confirmed
    /// something that has since changed, and importing anyway would act on a
    /// consent that no longer applies to what would happen.
    #[serde(default)]
    confirmed_plan: Option<String>,
}

fn default_destination() -> String {
    "library".to_owned()
}

async fn start_import(
    State(state): State<AppState>,
    RequirePseud { user, pseud_id }: RequirePseud,
    Json(request): Json<StartImportRequest>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let pseud = match &request.pseud_id {
        Some(raw) => parse_pseud(raw)?,
        None => pseud_id,
    };
    if request.destination != "library" {
        return Err(ApiError(AppError::Validation {
            message: format!(
                "{} is not a destination this build can import into; only \"library\" is",
                request.destination
            ),
            field_errors: Default::default(),
        }));
    }
    let url = parse_import_url(&request.url)?;
    let adapter = state.registry().route(&url).map_err(|_| {
        ApiError(AppError::Validation {
            message: "no source in this build handles that address".to_owned(),
            field_errors: Default::default(),
        })
    })?;
    let source_key = adapter.key().as_str().to_owned();
    refuse_unusable_source(&state, &source_key).await?;
    refuse_unreachable_source(&state, adapter)?;

    // A source that cannot do this is refused here rather than queued and failed
    // later, because a queued job is a promise that the work will be attempted.
    if !adapter.capabilities().metadata {
        return Err(ApiError(AppError::Validation {
            message: format!("the {source_key} adapter cannot read a work's metadata"),
            field_errors: Default::default(),
        }));
    }

    // The plan the reader confirmed, re-derived. This is the cheap half of the
    // consent check: the metadata fetch that produced the preview is repeated,
    // but nothing is stored until both agree.
    if let Some(expected) = &request.confirmed_plan {
        let mut policy = crate::imports::policy_for(adapter, state.config());
        policy.timeout = PREVIEW_TIMEOUT;
        let fetcher = SafeFetcher::new(adapter.hosts(), policy);
        let credential = match adapter.capabilities().authentication {
            lorehaven_scrapers::AuthKind::None => None,
            _ => load_credential_for(&state, &pseud, &source_key).await?,
        };
        let scope = if credential.is_some() {
            pseud.to_string()
        } else {
            crate::revisions::PUBLIC_SCOPE.to_owned()
        };
        let fetcher = caching_fetcher(&state, fetcher, &source_key, &scope).await;
        let work = adapter
            .preview(&fetcher, &url, credential.as_ref())
            .await
            .map_err(preview_error)?;

        let held = held_work(&state, &user.account_id.to_string(), &source_key, &work).await?;
        let plan = plan_import(
            held.as_ref(),
            &ImportedWork {
                title: work.title.clone(),
                author_text: work.author_text.clone(),
                chapters: work
                    .chapters
                    .iter()
                    .map(|chapter| ChapterIdentity {
                        source_chapter_key: chapter.source_chapter_key.clone(),
                        ordinal: chapter.ordinal,
                        word_count: None,
                        title: chapter.title.clone(),
                    })
                    .collect(),
                word_count: None,
            },
        );
        if describe_plan(&plan).plan != *expected {
            return Err(ApiError(AppError::RevisionConflict {
                expected: 0,
                actual: 0,
            }));
        }
    }

    // Order matters and is not free to change. The import's id exists first, so
    // the queue row can carry it in its payload from the moment the row is
    // visible. Queueing first and patching the payload afterwards leaves a job
    // any worker may claim while it points at an import that is not there.
    let id = lorehaven_domain::ImportJobId::new().to_string();
    let job_id = lorehaven_db::jobs::enqueue(
        state.db(),
        lorehaven_domain::jobs::JobKind::Import,
        // The payload names the import and nothing else. Spec §11.6: no
        // credentials in job payloads — and the URL, which can carry a private
        // token, stays in the import row rather than here.
        &serde_json::json!({ "import_job_id": id }).to_string(),
        None,
        Some(user.account_id),
        0,
        &lorehaven_domain::jobs::RetryPolicy::default(),
    )
    .await?;

    let import = imports::create_import_job(
        state.db(),
        &id,
        &job_id.to_string(),
        &user.account_id.to_string(),
        &pseud.to_string(),
        &source_key,
        url.as_str(),
        &request.destination,
        request.dry_run,
    )
    .await?;

    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "import_id": import.id,
            "job_id": job_id.to_string(),
            "source_key": source_key,
            "destination": request.destination,
            "dry_run": request.dry_run,
            // The import row's own word, not a guess: `queued` is what the
            // worker will pick up, and `pending` was never a state it has.
            "state": import.state,
            "created_at": import.created_at,
        })),
    ))
}

// ---------------------------------------------------------------------------
// Batch import
// ---------------------------------------------------------------------------

/// The ceiling on a batch, so one request cannot fill the queue past what a
/// worker running alongside it could ever drain. 500 URLs is already a large
/// deliberate migration; anything bigger belongs in several requests.
const BATCH_MAX_URLS: usize = 500;

#[derive(Debug, Deserialize)]
struct BatchImportRequest {
    /// One URL per line, plain text. Blank lines and `#` comments are ignored,
    /// so the file a reader keeps their links in can be posted as it is.
    urls: String,
    #[serde(default)]
    pseud_id: Option<String>,
    /// Only `library`, for the same reason [`StartImportRequest`] allows it.
    #[serde(default = "default_destination")]
    destination: String,
    #[serde(default)]
    dry_run: bool,
}

/// Split a plain-text URL list into lines, dropping blanks, comments and
/// duplicates. A batch that names the same work twice queues it twice, and the
/// second import would arrive as an update that changed nothing — so the
/// duplicates are reported rather than silently queued.
fn split_url_list(raw: &str) -> (Vec<String>, usize) {
    let mut seen = std::collections::HashSet::new();
    let mut urls = Vec::new();
    let mut duplicates = 0usize;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if seen.insert(trimmed.to_owned()) {
            urls.push(trimmed.to_owned());
        } else {
            duplicates += 1;
        }
    }
    (urls, duplicates)
}

/// Enqueue one import per URL in a plain-text list.
///
/// This is a queueing loop around [`start_import`]'s own checks, not a bypass
/// of them: every URL is parsed, routed to an adapter, and refused against the
/// same catalogue rules before its job is created. What a batch adds is the
/// per-URL answer — one bad line does not fail the file, it is reported as
/// `failed` alongside the `queued` — and a single response that says what
/// happened to each line.
async fn start_import_batch(
    State(state): State<AppState>,
    RequirePseud { user, pseud_id }: RequirePseud,
    Json(request): Json<BatchImportRequest>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let pseud = match &request.pseud_id {
        Some(raw) => parse_pseud(raw)?,
        None => pseud_id,
    };
    if request.destination != "library" {
        return Err(ApiError(AppError::Validation {
            message: format!(
                "{} is not a destination this build can import into; only \"library\" is",
                request.destination
            ),
            field_errors: Default::default(),
        }));
    }

    let (urls, duplicates) = split_url_list(&request.urls);
    if urls.is_empty() {
        return Err(ApiError(AppError::Validation {
            message: "the batch lists no URLs: give it one web address per line".to_owned(),
            field_errors: Default::default(),
        }));
    }
    if urls.len() > BATCH_MAX_URLS {
        return Err(ApiError(AppError::Validation {
            message: format!(
                "the batch lists {} URLs; split it into files of at most {BATCH_MAX_URLS}",
                urls.len()
            ),
            field_errors: Default::default(),
        }));
    }

    let mut queued = Vec::new();
    let mut failed = Vec::new();

    for url in &urls {
        // The same parse and routing a single import goes through, so a batch
        // cannot queue a URL the single endpoint would have refused.
        let parsed = match parse_import_url(url) {
            Ok(parsed) => parsed,
            Err(_) => {
                failed.push(serde_json::json!({
                    "url": url,
                    "code": "invalid_url",
                    "message": "must be an absolute http(s) address",
                }));
                continue;
            }
        };
        let adapter = match state.registry().route(&parsed) {
            Ok(adapter) => adapter,
            Err(_) => {
                failed.push(serde_json::json!({
                    "url": url,
                    "code": "no_source",
                    "message": "no source in this build handles that address",
                }));
                continue;
            }
        };
        let source_key = adapter.key().as_str().to_owned();
        if let Err(reason) = refuse_unusable_source(&state, &source_key).await {
            failed.push(serde_json::json!({
                "url": url,
                "code": "source_disabled",
                "message": format!("{reason}"),
            }));
            continue;
        }
        if !adapter.capabilities().metadata {
            failed.push(serde_json::json!({
                "url": url,
                "code": "no_metadata",
                "message": format!("the {source_key} adapter cannot read a work's metadata"),
            }));
            continue;
        }

        // The same order [`start_import`] keeps: the import's id exists before
        // the queue row that points at it, and the payload names the import and
        // nothing else.
        let id = lorehaven_domain::ImportJobId::new().to_string();
        let job = lorehaven_db::jobs::enqueue(
            state.db(),
            lorehaven_domain::jobs::JobKind::Import,
            &serde_json::json!({ "import_job_id": id }).to_string(),
            None,
            Some(user.account_id),
            0,
            &lorehaven_domain::jobs::RetryPolicy::default(),
        )
        .await;
        let job_id = match job {
            Ok(job_id) => job_id,
            Err(error) => {
                failed.push(serde_json::json!({
                    "url": url,
                    "code": "queue_error",
                    "message": format!("{error}"),
                }));
                continue;
            }
        };

        let created = imports::create_import_job(
            state.db(),
            &id,
            &job_id.to_string(),
            &user.account_id.to_string(),
            &pseud.to_string(),
            &source_key,
            parsed.as_str(),
            &request.destination,
            request.dry_run,
        )
        .await;

        match created {
            Ok(import) => queued.push(serde_json::json!({
                "url": url,
                "import_id": import.id,
                "job_id": job_id.to_string(),
                "source_key": source_key,
                "state": import.state,
            })),
            Err(error) => {
                failed.push(serde_json::json!({
                    "url": url,
                    "code": "create_error",
                    "message": format!("{error}"),
                }));
            }
        }
    }

    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "queued": queued,
            "failed": failed,
            "duplicates_skipped": duplicates,
            "queued_count": queued.len(),
            "failed_count": failed.len(),
        })),
    ))
}

/// What is already held for this work, in the planner's shape.
async fn held_work(
    state: &AppState,
    account_id: &str,
    source_key: &str,
    work: &lorehaven_scrapers::SourceWork,
) -> ApiResult<Option<ImportedWork>> {
    let item =
        imports::find_library_item(state.db(), account_id, source_key, &work.source_work_key)
            .await?;
    let chapters =
        imports::previous_chapters_for(state.db(), account_id, source_key, &work.source_work_key)
            .await?;
    Ok(item.map(|item| ImportedWork {
        title: item.title.clone(),
        author_text: item.author_text.clone(),
        chapters: chapters
            .clone()
            .unwrap_or_default()
            .iter()
            .map(|chapter| ChapterIdentity {
                source_chapter_key: chapter.source_chapter_key.clone(),
                ordinal: u32::try_from(chapter.ordinal).unwrap_or(u32::MAX),
                title: chapter.title.clone(),
                word_count: None,
            })
            .collect(),
        word_count: None,
    }))
}

// ---------------------------------------------------------------------------
// The caller's imports
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ImportsQuery {
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

#[derive(Debug, Serialize)]
struct ImportJobView {
    id: String,
    source_key: String,
    source_url: String,
    destination: String,
    state: String,
    dry_run: bool,
    library_item_id: Option<String>,
    /// The report, parsed, so a client does not have to parse a string field.
    report: Option<serde_json::Value>,
    created_at: String,
    updated_at: String,
    /// Whether the caller may still cancel it, so the button is offered exactly
    /// where the server would accept it.
    cancellable: bool,
}

fn import_view(row: imports::ImportJob) -> ImportJobView {
    ImportJobView {
        id: row.id,
        source_key: row.source_key,
        source_url: row.source_url,
        destination: row.destination_type,
        dry_run: row.dry_run,
        state: row.state.clone(),
        library_item_id: row.library_item_id,
        report: row
            .report_json
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok()),
        created_at: row.created_at,
        updated_at: row.updated_at,
        cancellable: matches!(row.state.as_str(), "pending" | "running"),
    }
}

async fn list_imports(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Query(query): Query<ImportsQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let after = query.cursor.as_deref().map(decode_cursor).transpose()?;
    let rows = imports::list_import_jobs(
        state.db(),
        &user.account_id.to_string(),
        query.state.as_deref(),
        PAGE,
        after.as_ref().map(|(at, id)| (at.as_str(), id.as_str())),
    )
    .await?;
    let next_cursor = if i64::try_from(rows.len()).unwrap_or(i64::MAX) >= PAGE {
        rows.last()
            .map(|row| encode_cursor(&row.created_at, &row.id))
    } else {
        None
    };
    Ok(Json(serde_json::json!({
        "items": rows.into_iter().map(import_view).collect::<Vec<_>>(),
        "next_cursor": next_cursor,
    })))
}

async fn get_import(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let row = imports::get_import_job(state.db(), &id)
        .await?
        .filter(|row| row.account_id == user.account_id.to_string())
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "import" }))?;

    let chapters = imports::list_import_chapters(state.db(), &row.id).await?;
    let mut view = serde_json::to_value(import_view(row)).unwrap_or(serde_json::Value::Null);
    if let Some(object) = view.as_object_mut() {
        object.insert(
            "chapters".to_owned(),
            serde_json::json!(chapters
                .into_iter()
                .map(|chapter| serde_json::json!({
                    "ordinal": chapter.ordinal,
                    "source_chapter_key": chapter.source_chapter_key,
                    "title": chapter.title,
                    "state": chapter.state,
                    "checksum": chapter.content_blob_checksum,
                    "note": chapter.note,
                }))
                .collect::<Vec<_>>()),
        );
    }
    Ok(Json(view))
}

async fn cancel_import(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let row = imports::get_import_job(state.db(), &id)
        .await?
        .filter(|row| row.account_id == user.account_id.to_string())
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "import" }))?;

    // Cancelling something that already finished is not an error: the caller
    // asked for the end state and it holds. The answer says what the state is.
    if matches!(row.state.as_str(), "completed" | "failed" | "cancelled") {
        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({ "import_id": row.id, "state": row.state })),
        ));
    }

    imports::set_import_state(state.db(), &row.id, "cancelled", None, None).await?;

    // The queue row is cancelled too, so the worker stops between chapters
    // rather than at the end of the work.
    if let Some(job_id) = imports::job_for_import(state.db(), &row.id).await? {
        if let Ok(job_id) = job_id.parse::<lorehaven_domain::JobId>() {
            lorehaven_db::jobs::cancel(state.db(), job_id).await?;
        }
    }

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({ "import_id": row.id, "state": "cancelled" })),
    ))
}

// Retired in M8: `GET /library/items` now lives in `routes::library`, which
// serves the same projection with filtering, sorting and paging. Kept here,
// unused, only long enough to be sure nothing referenced it — and removed
// outright rather than left as dead code.
#[allow(dead_code)]
async fn list_library(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Query(query): Query<ImportsQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let after = query.cursor.as_deref().map(decode_cursor).transpose()?;
    let rows = imports::list_library_items(
        state.db(),
        &user.account_id.to_string(),
        PAGE,
        after.as_ref().map(|(at, id)| (at.as_str(), id.as_str())),
    )
    .await?;
    let next_cursor = if i64::try_from(rows.len()).unwrap_or(i64::MAX) >= PAGE {
        rows.last()
            .map(|row| encode_cursor(&row.created_at, &row.id))
    } else {
        None
    };
    Ok(Json(serde_json::json!({
        "items": rows
            .into_iter()
            .map(|row| {
                let source_display_name = state
                    .registry()
                    .by_key(&SourceKey::new(row.source_key.clone()))
                    .map_or_else(
                        |_| row.source_key.clone(),
                        |adapter| adapter.display_name().to_owned(),
                    );
                serde_json::json!({
                "id": row.id,
                "source_key": row.source_key,
                "source_work_key": row.source_work_key,
                "source_url": row.source_url,
                "title": row.title,
                "author_text": row.author_text,
                "author_url": row.author_url,
                "summary": row.summary,
                "language": row.language,
                "word_count": row.word_count,
                "chapter_count": row.chapter_count,
                "source_display_name": source_display_name,
                "status": row.status,
                "source_updated_at": row.source_updated_at,
                "last_synced_at": row.last_synced_at,
                "created_at": row.created_at,
                "updated_at": row.updated_at,
                })
            })
            .collect::<Vec<_>>(),
        "next_cursor": next_cursor,
    })))
}

/// Queue another attempt at the chapters that failed, and only those.
///
/// This is the plan's "re-fetch only what failed" route, and it is a *new job
/// on the same import* rather than a new import: the chapters already stored are
/// already stored, so the second attempt reads the record, skips them, and asks
/// the source only for the ordinals that failed. For a source that cannot
/// address a single chapter the retry is a bulk fetch and the skip rule is what
/// keeps it from re-storing anything (spec §11.7, and the capability is reported
/// rather than assumed).
async fn retry_failed_chapters(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let row = imports::get_import_job(state.db(), &id)
        .await?
        .filter(|row| row.account_id == user.account_id.to_string())
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "import" }))?;

    if !matches!(row.state.as_str(), "completed" | "failed" | "cancelled") {
        return Err(ApiError(AppError::Validation {
            message: "this import has not finished, so there is nothing to retry yet".to_owned(),
            field_errors: Default::default(),
        }));
    }

    let chapters = imports::list_import_chapters(state.db(), &row.id).await?;
    let failed = chapters
        .iter()
        .filter(|chapter| chapter.state == "failed")
        .count();
    if failed == 0 {
        // Nothing to do, and saying so is better than queueing a job that will
        // find nothing and look like it did something.
        return Err(ApiError(AppError::Validation {
            message: "this import has no failed chapters to retry".to_owned(),
            field_errors: Default::default(),
        }));
    }

    let job_id = lorehaven_db::jobs::enqueue(
        state.db(),
        lorehaven_domain::jobs::JobKind::Import,
        &serde_json::json!({ "import_job_id": row.id }).to_string(),
        None,
        Some(user.account_id),
        0,
        &lorehaven_domain::jobs::RetryPolicy::default(),
    )
    .await?;
    imports::set_import_state(state.db(), &row.id, "queued", None, None).await?;

    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "import_id": row.id,
            "job_id": job_id.to_string(),
            "state": "queued",
            "failed_chapters": failed,
        })),
    ))
}

// ---------------------------------------------------------------------------
// Source credentials
// ---------------------------------------------------------------------------

/// How long a stored credential lives by default (spec §11.6: "Default expiry
/// of 30 days or the source's earlier expiry").
const DEFAULT_CREDENTIAL_DAYS: i64 = 30;

#[derive(Debug, Deserialize)]
struct CredentialRequest {
    source_key: String,
    /// What the source calls the account. Not secret, and what the reader sees
    /// in a list, so it is required: a list of credentials with no labels is a
    /// list nobody can choose from.
    label: String,
    /// The secret itself: a password, an API token, or the value of a session
    /// cookie. Write-only. It is never echoed, never logged and never returned.
    secret: String,
    /// Accepted and validated, but not yet stored: no adapter in this build
    /// gates content on a credential-side flag (see `load_credential_for`).
    /// It is refused rather than ignored, because silently dropping a reader's
    /// statement about what they may see is worse than not offering it.
    #[serde(default)]
    adult_allowed: bool,
    /// Days until this must be re-consented. Clamped to the default ceiling: a
    /// credential that outlives the policy is not this request's to grant.
    #[serde(default)]
    expires_in_days: Option<i64>,
}

#[derive(Debug, Serialize)]
struct CredentialView {
    id: String,
    source_key: String,
    label: String,
    status: String,
    expires_at: Option<String>,
    /// When the source last accepted or rejected it. Nullable because a
    /// credential nobody has tested yet is neither.
    last_checked_at: Option<String>,
    created_at: String,
    /// Present only on create: what happens next, in words.
    guidance: Option<String>,
}

fn credential_view(row: imports::SourceCredential) -> CredentialView {
    CredentialView {
        id: row.id,
        source_key: row.source_key,
        label: row.label,
        status: row.status,
        expires_at: row.expires_at,
        last_checked_at: row.last_checked_at,
        created_at: row.created_at,
        guidance: None,
    }
}

async fn list_credentials(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
) -> ApiResult<Json<serde_json::Value>> {
    // Metadata only. There is no branch here that could include a secret,
    // because `CredentialView` has no field for one.
    let rows = imports::list_source_credentials(state.db(), &pseud_id.to_string(), None).await?;
    Ok(Json(serde_json::json!({
        "items": rows.into_iter().map(credential_view).collect::<Vec<_>>(),
    })))
}

async fn store_credential(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Json(request): Json<CredentialRequest>,
) -> ApiResult<(StatusCode, Json<CredentialView>)> {
    if request.secret.trim().is_empty() {
        return Err(ApiError(AppError::field("secret", "must not be empty")));
    }
    if request.label.trim().is_empty() {
        return Err(ApiError(AppError::field("label", "must not be empty")));
    }
    if request.adult_allowed {
        // Refused, not ignored: accepting a setting that changes nothing would
        // tell a reader their choice was recorded when it was discarded.
        return Err(ApiError(AppError::field(
            "adult_allowed",
            "no source in this build gates content on a credential-side flag, so this cannot be set",
        )));
    }

    let key = SourceKey::new(request.source_key.clone());
    let adapter = state.registry().by_key(&key).map_err(|_| {
        ApiError(AppError::Validation {
            message: format!("{} is not a source this build knows", request.source_key),
            field_errors: Default::default(),
        })
    })?;
    if adapter.capabilities().authentication == lorehaven_scrapers::AuthKind::None {
        return Err(ApiError(AppError::Validation {
            message: format!(
                "the {} source does not use credentials, so there is nothing to store",
                request.source_key
            ),
            field_errors: Default::default(),
        }));
    }

    let days = request
        .expires_in_days
        .unwrap_or(DEFAULT_CREDENTIAL_DAYS)
        .clamp(1, DEFAULT_CREDENTIAL_DAYS);
    let expires_at = times::now_utc()
        .checked_add(time::Duration::days(days))
        .map(format_time);

    let cipher = crate::secrets::load_cipher(
        &state.config().storage.root,
        state.config().security.secret_key_file.as_deref(),
        !state.config().environment.is_development(),
    )
    .map_err(|error| ApiError(AppError::Internal(anyhow::anyhow!(error))))?;

    // The row is written first, because the secret is encrypted *for* it: the
    // associated data includes the owner id, so the ciphertext cannot be moved
    // to another credential by editing a column.
    // The secret is written *first*, and the credential row then references it.
    // The other order cannot work: `source_credentials.secret_id` is a foreign
    // key, so a credential row that names a secret nobody has written yet is
    // refused by the database — correctly, because a credential whose value is
    // missing is not a credential.
    //
    // The encryption is bound to the credential's *natural key* — this pseud,
    // this source, this label — rather than to a row id that does not exist yet.
    // That is the better binding anyway: rotating the secret for the same label
    // replaces one `secrets` row rather than accumulating orphaned ones, because
    // the natural key is what `secrets` is unique on.
    let owner_id = credential_owner_id(&pseud_id.to_string(), &request.source_key, &request.label);
    let secret_id = crate::secrets::seal_secret(
        state.db(),
        &cipher,
        "source_credential",
        &owner_id,
        "secret",
        &crate::secrets::Secret::new(request.secret.clone()),
    )
    .await
    .map_err(ApiError::from)?;

    let (row, replaced) = imports::upsert_source_credential(
        state.db(),
        &pseud_id.to_string(),
        &request.source_key,
        &secret_id,
        &request.label,
        expires_at.as_deref(),
    )
    .await?;

    // A label that already had a *different* secret row leaves that row behind
    // when the natural key changed (a rotated label, say). Removing it keeps the
    // store honest: an unreferenced secret is a secret nobody can explain.
    if let Some(previous) = replaced {
        if previous != secret_id {
            secrets::delete_secret(state.db(), &previous).await?;
        }
    }

    let stored = imports::get_source_credential(state.db(), &row.id)
        .await?
        .ok_or_else(|| {
            ApiError(AppError::NotFound {
                resource: "credential",
            })
        })?;
    let mut view = credential_view(stored);
    view.guidance = Some(format!(
        "stored for the {} source as {}; expires {}. Imports started as this pseud will use it.",
        request.source_key,
        request.label,
        expires_at.unwrap_or_else(|| "never".to_owned())
    ));
    Ok((StatusCode::CREATED, Json(view)))
}

async fn delete_credential(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    // One call, because the deletion is one act: the repository removes the
    // ciphertext and the schema cascades the row that named it. A route that
    // had to delete two things in the right order would eventually delete one.
    let removed = imports::delete_source_credential(state.db(), &id, &pseud_id.to_string()).await?;
    if removed.is_none() {
        return Err(ApiError(AppError::NotFound {
            resource: "credential",
        }));
    }
    // Already-imported copies stay: spec §11.6, "Deleting a credential does not
    // automatically delete already imported copies."
    Ok(StatusCode::NO_CONTENT)
}

/// Check a stored credential against its source.
///
/// The check is a real request through the real guard, because a credential that
/// only works in a unit test is a credential that does not work. What it must
/// not do is leave anything behind: no import, no item, no chapter.
async fn test_credential(
    State(state): State<AppState>,
    RequirePseud { pseud_id, .. }: RequirePseud,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let row = imports::get_source_credential(state.db(), &id)
        .await?
        .filter(|row| row.pseud_id == pseud_id.to_string())
        .ok_or_else(|| {
            ApiError(AppError::NotFound {
                resource: "credential",
            })
        })?;

    let key = SourceKey::new(row.source_key.clone());
    let adapter = state.registry().by_key(&key).map_err(|_| {
        ApiError(AppError::Validation {
            message: format!("{} is not a source this build knows", row.source_key),
            field_errors: Default::default(),
        })
    })?;

    let landed = match load_stored_secret(&state, &row.secret_id).await {
        Ok(Some(secret)) => secret,
        Ok(None) => {
            imports::set_credential_status(state.db(), &row.id, "invalid", true).await?;
            return Ok(Json(serde_json::json!({
                "credential_id": row.id,
                "status": "invalid",
                "detail": "the stored secret is gone, so this credential cannot be used",
            })));
        }
        Err(error) => return Err(error),
    };

    // The source's own homepage is the cheapest authenticated address there is:
    // it is the page a signed-out visitor is redirected from, so a rejection
    // here means the credential is wrong rather than that a path was.
    let Some(probe) = adapter.hosts().into_iter().next() else {
        return Ok(Json(serde_json::json!({
            "credential_id": row.id,
            "status": row.status,
            "detail": "this adapter declares no hosts to probe",
        })));
    };
    let url = format!("https://{probe}/")
        .parse::<url::Url>()
        .map_err(|error| ApiError(AppError::Internal(anyhow::anyhow!(error))))?;

    let mut policy = crate::imports::policy_for(adapter, state.config());
    policy.timeout = PREVIEW_TIMEOUT;
    let fetcher = SafeFetcher::new(adapter.hosts(), policy)
        .with_credential_header(probe.clone(), crate::imports::CREDENTIAL_HEADER, &landed)
        .map_err(|error| {
            ApiError(AppError::Validation {
                message: format!("the stored credential cannot be sent: {error}"),
                field_errors: Default::default(),
            })
        })?;

    let response = lorehaven_scrapers::Fetcher::get(&fetcher, url.as_str()).await;

    match response {
        Ok(_) => {
            imports::set_credential_status(state.db(), &row.id, "active", true).await?;
            Ok(Json(serde_json::json!({
                "credential_id": row.id,
                "status": "active",
                "detail": format!("the {probe} source accepted the request"),
            })))
        }
        Err(lorehaven_scrapers::SourceError::AuthRequired(detail)) => {
            imports::set_credential_status(state.db(), &row.id, "invalid", true).await?;
            Ok(Json(serde_json::json!({
                "credential_id": row.id,
                "status": "invalid",
                "detail": format!("the source rejected the credential: {detail}"),
            })))
        }
        Err(error) => Ok(Json(serde_json::json!({
            "credential_id": row.id,
            "status": row.status,
            "detail": format!("the check could not be completed: {error}"),
        }))),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Load a stored secret for whichever pseud is asking.
async fn load_credential_for(
    state: &AppState,
    pseud: &PseudId,
    source_key: &str,
) -> ApiResult<Option<lorehaven_scrapers::Credentials>> {
    let rows =
        imports::list_source_credentials(state.db(), &pseud.to_string(), Some(source_key)).await?;
    let Some(row) = rows
        .into_iter()
        .find(|row| row.status != "revoked" && !row.secret_id.is_empty())
    else {
        return Ok(None);
    };
    let Some(secret) = load_stored_secret(state, &row.secret_id).await? else {
        return Ok(None);
    };
    Ok(Some(lorehaven_scrapers::Credentials {
        source_key: SourceKey::new(source_key.to_owned()),
        username: row.label,
        secret,
        // Whether an adult gate is satisfied is the credential's own business
        // and is not stored: the sources this build reads do not gate content
        // behind a flag we can assert on the reader's behalf, and pretending
        // otherwise would be a setting that does nothing.
        adult_allowed: false,
    }))
}

/// Decrypt one secret, or `None` when its row is gone.
async fn load_stored_secret(state: &AppState, secret_id: &str) -> ApiResult<Option<String>> {
    if secret_id.is_empty() {
        return Ok(None);
    }
    let cipher = crate::secrets::load_cipher(
        &state.config().storage.root,
        state.config().security.secret_key_file.as_deref(),
        !state.config().environment.is_development(),
    )
    .map_err(|error| ApiError(AppError::Internal(anyhow::anyhow!(error))))?;
    crate::secrets::open_secret(state.db(), &cipher, secret_id)
        .await
        .map_err(|error| ApiError(AppError::Internal(error)))
}

/// Parse the URL a reader pasted.
///
/// Only `http` and `https` are accepted. A `file:` URL here would be a way to
/// read the server's disk, and the guard would not be the thing that stopped it
/// — it would never get that far, because the scheme check lives here as well as
/// there. Two checks for the one mistake that has no safe recovery.
fn parse_import_url(raw: &str) -> ApiResult<url::Url> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ApiError(AppError::field("url", "must not be empty")));
    }
    let url = trimmed.parse::<url::Url>().map_err(|_| {
        ApiError(AppError::field(
            "url",
            "must be an absolute web address, like https://example.org/work/123",
        ))
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ApiError(AppError::field(
            "url",
            "must be an http or https address",
        )));
    }
    Ok(url)
}

/// The identity a stored credential is encrypted against.
///
/// The natural key rather than a row id, because the secret is written before
/// the row exists (see `store_credential`), and because this is what `secrets`
/// is unique on — so rotating a password replaces one row instead of leaving the
/// old ciphertext behind under an id nothing references.
///
/// The separator is `\u{1f}` (unit separator), which cannot appear in a UUID, a
/// source key or a label that has been through validation, so two different
/// credentials cannot produce the same owner id by colliding on a delimiter.
fn credential_owner_id(pseud_id: &str, source_key: &str, label: &str) -> String {
    format!("{pseud_id}\u{1f}{source_key}\u{1f}{label}")
}

fn parse_pseud(raw: &str) -> ApiResult<PseudId> {
    raw.parse()
        .map_err(|_| ApiError(AppError::NotFound { resource: "pseud" }))
}

/// Decode `created_at|id`, the pair the next page starts after.
fn decode_cursor(raw: &str) -> ApiResult<(String, String)> {
    let decoded = BASE64
        .decode(raw)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .ok_or_else(|| {
            ApiError(AppError::field(
                "cursor",
                "is not one this collection issued",
            ))
        })?;
    let (created_at, id) = decoded.split_once('|').ok_or_else(|| {
        ApiError(AppError::field(
            "cursor",
            "is not one this collection issued",
        ))
    })?;
    Ok((created_at.to_owned(), id.to_owned()))
}

fn encode_cursor(created_at: &str, id: &str) -> String {
    BASE64.encode(format!("{created_at}|{id}"))
}

/// The clock, in one place, so this module has a single dependency on "now".
mod times {
    pub fn now_utc() -> time::OffsetDateTime {
        time::OffsetDateTime::now_utc()
    }
}

/// RFC 3339 UTC, which is the only timestamp format this codebase stores.
fn format_time(at: time::OffsetDateTime) -> String {
    crate::format_rfc3339(at)
}

// ---------------------------------------------------------------------------
// Shelf exports (spec §32.3, M24)
// ---------------------------------------------------------------------------

/// A library export to import, and which site it came from.
#[derive(Debug, Deserialize)]
pub struct ShelfImportRequest {
    /// `goodreads` or `storygraph`.
    pub format: String,
    /// The export file's contents, as text.
    pub csv: String,
}

/// Import a shelf export from Goodreads or StoryGraph.
///
/// Ingestion only, and metadata only: a shelf export records what the reader
/// read elsewhere, so this creates the reader's own library rows and the state
/// each row implies — never chapter bodies, and never a claim that this instance
/// holds the work.
///
/// # What "respects the reader's existing ratings and dates" means here
///
/// A row's date read becomes the item's `finished_at`, not the time of the
/// import: the reader finished the book in 2019 and imported it today. A library
/// state the reader has already set for an item is theirs and is left alone; the
/// import fills in what is missing and reports how many it left. Rows it cannot
/// map are refused by name, with the reason, rather than imported as a guess.
pub async fn import_shelf_csv(
    State(state): State<AppState>,
    RequirePseud { user, pseud_id: _ }: RequirePseud,
    Json(request): Json<ShelfImportRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let db = state.db();
    let format = request.format.trim().to_ascii_lowercase();
    let shelf = lorehaven_scrapers::csv::import_shelf(&request.csv, &format).map_err(|error| {
        ApiError::from(AppError::Validation {
            message: format!("this file is not a {format} library export: {error}"),
            field_errors: Default::default(),
        })
    })?;

    let plan = lorehaven_scrapers::csv::plan_shelf_import(&shelf);
    let account_id = user.account_id.to_string();
    let mut imported = 0i64;
    let mut kept_existing_state = 0i64;

    for item in &plan.items {
        let input = shelf_import_input(&format, item);
        let row =
            imports::upsert_library_item(db, &account_id, &format, &item.source_work_key, &input)
                .await?;

        let status = if item.state == "finished" {
            ReadingStatus::Finished
        } else {
            ReadingStatus::WantToRead
        };
        let written = library::set_imported_reading_status(
            db,
            &account_id,
            "library_item",
            &row.id,
            status,
            item.finished_at.as_deref(),
        )
        .await?;
        if written {
            imported += 1;
        } else {
            kept_existing_state += 1;
        }
    }

    Ok(Json(serde_json::json!({
        "format": format,
        "imported": imported,
        "kept_existing_state": kept_existing_state,
        "refused": plan
            .refused
            .iter()
            .map(|refusal| serde_json::json!({
                "title": refusal.title,
                "reason": refusal.reason,
            }))
            .collect::<Vec<_>>(),
        "skipped_rows_in_file": shelf.skipped,
    })))
}

/// The library item an imported shelf row becomes.
///
/// `source_url` is an `import://` URI on purpose: a shelf export carries no URL
/// for the book, and a fabricated `https://` one would be a link somebody could
/// later follow and act on. The scheme says where the row came from and is not
/// fetchable by anything.
fn shelf_import_input(
    format: &str,
    item: &lorehaven_scrapers::csv::PlannedItem,
) -> imports::LibraryItemInput {
    imports::LibraryItemInput {
        title: item.title.clone(),
        author_text: item.author_text.clone(),
        author_url: None,
        summary: String::new(),
        language: None,
        word_count: None,
        // A shelf export says nothing about where the book's text is, so the
        // honest publication status is "unknown".
        status: "unknown".to_owned(),
        source_url: format!("import://{format}/{}", item.source_work_key),
        source_updated_at: None,
        provenance_json: item.provenance_json.clone(),
    }
}
