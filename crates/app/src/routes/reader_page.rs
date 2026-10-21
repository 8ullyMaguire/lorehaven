//! The reading surface: **server-rendered HTML** (spec §8.5, §10.1, §21.1).
//!
//! # Why the reader is rendered here and not in the browser
//!
//! Spec §8.5: *"The reader is server-rendered HTML — no JavaScript required to
//! read."* Spec §21.1: *"Accessibility is a floor: the reader works without
//! JavaScript."* And the plan's T-M5-01 asks for the whole chain — open a
//! work, read a unit, move to the next one, come back — with **the navigation
//! as a real link, not a fetch**.
//!
//! A single-page application cannot be that surface. Its content is fetched
//! after the document loads, so with JavaScript off the reader gets an empty
//! page and a spinner that never resolves. The reader is therefore rendered
//! here, by the server, from the same stores the API reads.
//!
//! # What JavaScript is still for
//!
//! The reader is a **document that JavaScript improves**, not a document that
//! JavaScript provides. Without it, the reader can read, navigate, save a
//! place and reveal a gated section. With it, the position save becomes
//! continuous rather than a button, and the notes panel appears. Nothing the
//! reader needs is on the other side of that line.
//!
//! # The URL keeps its shape
//!
//! The reader lives at `/works/{work_id}/units/{unit_id}` — the path the SPA
//! already used, so the site's links do not change and the specification's
//! reader URLs (§8.5) are honoured. The server answers that path, so the SPA
//! shell never sees it; the site links to it with a **real link**, which is
//! what "the chapter navigation is a real link, not a fetch" asks for.
//!
//! # The escaping
//!
//! Everything on this page is escaped by [`crate::html`], which makes the
//! decision in its API rather than at each call site. The single exception is
//! the unit's `sanitized_html`, which the **server** produced from the
//! validated editor document (spec §8.3, ADR 0002) — the one value here that
//! is markup rather than text.

use axum::extract::{Form, Path, State};
use axum::http::{header, StatusCode};
use axum::response::{Html as AxumHtml, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;

use serde::Deserialize;

use darkhold_db::content;
use darkhold_db::reading::{self, ProgressInput};
use darkhold_domain::AppError;

use crate::auth::MaybeSession;
use crate::html::Html;
use crate::http::{ApiError, ApiResult};
use crate::routes::works::{actor_for, parse_unit_id, parse_work_id, reading_decision, Reading};
use crate::state::AppState;

/// The cookie that carries a reader's reveal.
///
/// **The reveal is per browser, not per account.** It is a decision about
/// *this* reading -- "show me this part now" -- and it is deliberately not
/// stored on the account: a reader who reveals a spoiler on one device has
/// not said they want it revealed on every device, and a reader who has not
/// revealed it on one device has not said they never want to.
///
/// The value is a comma-separated list of scopes: a unit id, or `*` for the
/// whole work. A cookie is small and a reader's reveal list is short.
pub const REVEAL_COOKIE: &str = "darkhold_reveal";

/// What the reader has asked to see.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Reveals {
    /// The units the reader has asked to see.
    pub units: Vec<String>,
    /// Whether the reader has asked to see the whole work.
    pub whole_work: bool,
}

impl Reveals {
    /// Read the reveals out of a cookie's value.
    pub fn parse(raw: Option<&str>) -> Self {
        let Some(raw) = raw else {
            return Self::default();
        };
        let mut reveals = Self::default();
        for part in raw.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if part == "*" {
                reveals.whole_work = true;
            } else {
                reveals.units.push(part.to_owned());
            }
        }
        reveals
    }

    /// Whether this unit is among the reveals.
    pub fn covers(&self, unit_id: &str) -> bool {
        self.whole_work || self.units.iter().any(|unit| unit == unit_id)
    }

    /// The cookie's value with one more scope added.
    pub fn with(&self, scope: &str) -> String {
        let mut parts: Vec<String> = self.units.clone();
        if scope == "*" {
            parts.push("*".to_owned());
        } else if !parts.iter().any(|unit| unit == scope) {
            parts.push(scope.to_owned());
        }
        parts.join(",")
    }
}

/// The reveal cookie's value, out of the request's cookies.
fn reveal_cookie(headers: &axum::http::HeaderMap) -> Option<&str> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    cookies.split(';').find_map(|cookie| {
        let (name, value) = cookie.split_once('=')?;
        (name.trim() == REVEAL_COOKIE).then_some(value.trim())
    })
}

/// The reading surface's routes.
///
/// Mounted at the **root**, not under `/api/v1`: these are pages, and a page
/// under `/api/` would be an API client asking for HTML. The route inventory
/// records them with the same care as the rest.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/works/{work_id}/units/{unit_id}", get(read_unit_page))
        .route(
            "/works/{work_id}/units/{unit_id}/position",
            post(save_position),
        )
        .route(
            "/works/{work_id}/units/{unit_id}/typography",
            post(save_typography),
        )
        .route(
            "/works/{work_id}/units/{unit_id}/reveal",
            post(reveal_section),
        )
        .route(
            "/works/{work_id}/units/{unit_id}/dnf",
            post(set_did_not_finish),
        )
        .route("/works/{work_id}/units/{unit_id}/notes", post(save_note))
        .route(
            "/works/{work_id}/units/{unit_id}/notes/{note_id}/delete",
            post(delete_note),
        )
}

/// The form the position save posts.
///
/// Every field is a string, because a form post is strings — and the values
/// are parsed and **validated** below rather than trusted, because a form post
/// is whatever the client sent.
#[derive(Debug, Default, Deserialize)]
pub struct PositionForm {
    /// Where in the unit the reader was, in permille.
    #[serde(default)]
    pub position_permille: String,
    /// The paragraph the reader was at.
    #[serde(default)]
    pub paragraph_anchor: String,
    /// The revision the reader was reading.
    #[serde(default)]
    pub content_revision: String,
    /// The unit the reader was in. Defaults to the unit in the path.
    #[serde(default)]
    pub unit_id: String,
    /// The device the reader was on. Empty means "this one".
    #[serde(default)]
    pub device_id: String,
}

/// The form the typography control posts.
///
/// The reader's appearance is the reader's own (spec §10.1), and the control
/// is a **form** rather than a fetch: a reader without JavaScript must be able
/// to change how the page looks, exactly as they must be able to read it.
#[derive(Debug, Default, Deserialize)]
pub struct TypographyForm {
    #[serde(default)]
    pub font_scale: String,
    #[serde(default)]
    pub line_height: String,
    #[serde(default)]
    pub measure: String,
    #[serde(default)]
    pub reader_theme: String,
}

/// The form a reader's note posts.
#[derive(Debug, Default, Deserialize)]
pub struct NoteForm {
    /// The note's text.
    #[serde(default)]
    pub body: String,
}

/// Serve the reading page.
async fn read_unit_page(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path((work_id, unit_id)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
) -> ApiResult<Response> {
    let work_id = parse_work_id(&work_id)?;
    let unit_id = parse_unit_id(&unit_id)?;

    let work = content::find_work(state.db(), work_id)
        .await?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "work" }))?;
    let contributors =
        darkhold_db::collaboration::contributors_for_work(state.db(), work_id).await?;
    let actor = actor_for(session.as_ref());

    match reading_decision(&state, actor.as_ref(), &work, &contributors).await {
        Reading::Contributor | Reading::Public => {}
        Reading::Denied(error) => return Err(ApiError(error)),
    }

    let units = content::units_for_work(state.db(), work_id).await?;
    let unit = units
        .iter()
        .find(|candidate| candidate.id == unit_id)
        .cloned()
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "unit" }))?;

    let revision = match unit.current_revision_id {
        Some(id) => content::find_revision(state.db(), id).await?,
        None => None,
    };

    // The reader's own appearance, when there is a reader to have one. A
    // signed-out reader gets the defaults, and the page still reads.
    let typography = match session.as_ref() {
        Some(user) => reading::typography_for(state.db(), user.account_id).await?,
        None => reading::Typography::default(),
    };

    let position = units.iter().position(|candidate| candidate.id == unit_id);
    let previous = position
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| units.get(index));
    let next = position.and_then(|index| units.get(index + 1));

    // The reader's own notes, when there is a reader with a face to own them.
    // A note is private to its pseud (spec §10.3), so a signed-out reader
    // has none and sees nothing about notes at all.
    let notes = match session.as_ref().and_then(|user| user.pseud_id) {
        Some(pseud) => reading::notes_for(state.db(), pseud, "work", &work_id.to_string()).await?,
        None => Vec::new(),
    };

    // The narration, when the work has a published one. A draft edition is
    // the author's working copy, and saying it exists would leak the shape of
    // work nobody else can read yet.
    let narration = darkhold_db::media::list_media_editions(state.db(), &work_id.to_string())
        .await?
        .into_iter()
        .find(|edition| edition.edition_kind == "narration" && edition.published_at.is_some())
        .map(|edition| edition.id);

    // **The warnings are attached to the work** (spec §10.2). A warning is
    // information and never a gate on the archive: a reader who filters
    // nothing sees everything, with the warnings attached -- so they are on
    // the page, not behind a filter.
    let warnings =
        darkhold_db::taxonomy::list_work_warnings(state.db(), &work_id.to_string()).await?;

    // **The reader's place** (spec §10.3). The position lives on the
    // *account* and not in the browser, so it follows the reader to another
    // device -- which is the whole of "the position survives a device change".
    //
    // And a position recorded against an **older revision** is not applied
    // silently: the text in between may have been rewritten, so the page says
    // the place is stale rather than putting the reader at a fraction of a
    // text that is no longer there.
    let (position, stale_position) = match session.as_ref() {
        Some(user) => {
            let positions =
                reading::progress_for(state.db(), user.account_id, "work", &work_id.to_string())
                    .await?;
            let current = unit.current_revision_id;
            // A unit with no revision has no text to be lost in, so any stored
            // position is as good as another.
            let reliable = |position: &darkhold_domain::reading::ReadingPosition| match current {
                Some(current) => darkhold_domain::reading::position_is_reliable(position, current),
                None => true,
            };
            let position = positions
                .iter()
                .find(|position| reliable(position))
                .cloned();
            // **Stale** means: there is a stored position, and none of them can
            // be trusted against the unit's current revision.
            let stale =
                current.is_some() && !positions.is_empty() && !positions.iter().any(reliable);
            (position, stale)
        }
        None => (None, false),
    };

    // **The reader's own mark.** Private to their own face, and never a
    // judgement (spec §10.1, §10.3).
    let did_not_finish = match session.as_ref().and_then(|user| user.pseud_id) {
        Some(pseud) => darkhold_db::dnf::read_dnf(state.db(), pseud, work_id).await?,
        None => None,
    };

    // **The gate.** A work can require a gate on a section (spec §10.1), and
    // until the reader asks, **the text is not in the response** -- a hidden
    // element with the text in it is not a gate.
    let reveals = Reveals::parse(reveal_cookie(&headers));
    let gated = unit.spoiler_gated && !reveals.covers(&unit_id.to_string());

    let stylesheet = crate::assets::app_stylesheet(&state).await;
    let body = render_page(
        &work,
        &unit,
        revision
            .as_ref()
            .map(|revision| revision.sanitized_html.as_str()),
        &units,
        previous,
        next,
        &typography,
        stylesheet.as_deref(),
        &notes,
        narration.as_deref(),
        &warnings,
        session.is_some(),
        gated,
        position.as_ref(),
        stale_position,
        did_not_finish.as_ref(),
    );

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            // The reader's page is a document, and a document must not be
            // cached by a proxy as if it were the shell: the page carries the
            // reader's own preferences.
            (header::CACHE_CONTROL, "private, no-store"),
        ],
        AxumHtml(body),
    )
        .into_response())
}

/// Save the reader's place, and return them to the page.
///
/// The **form post** is the no-JavaScript path: a reader without JavaScript
/// still gets a place saved, because arriving at a unit is itself a reading
/// and the page says so. With JavaScript, the same endpoint is what the
/// continuous save posts to.
///
/// A refusal is not a failure here. The reader is sent back to the page they
/// were on, because a position that could not be saved is not something a
/// reader can do anything about, and an error page would lose the text they
/// were reading.
async fn save_position(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path((work_id, unit_id)): Path<(String, String)>,
    Form(form): Form<PositionForm>,
) -> ApiResult<Response> {
    let work_id = parse_work_id(&work_id)?;
    let unit_id = parse_unit_id(&unit_id)?;
    let back = format!("/works/{work_id}/units/{unit_id}");

    let Some(user) = session else {
        // A signed-out reader has no place to save to: the position lives on
        // the account, not in the browser (spec §10.3). The browser keeps its
        // own copy, and the server says so by sending the reader back.
        return Ok(Redirect::to(&back).into_response());
    };

    // The permille is parsed, not trusted: a form post is whatever the client
    // sent, and a position of 4000 permille is not a position.
    let fraction: u16 = form
        .position_permille
        .trim()
        .parse::<i64>()
        .ok()
        .map(|value| value.clamp(0, 1000) as u16)
        .unwrap_or(0);

    let revision = if form.content_revision.trim().is_empty() {
        None
    } else {
        form.content_revision
            .trim()
            .parse::<darkhold_domain::RevisionId>()
            .ok()
    };

    let anchor = if form.paragraph_anchor.trim().is_empty() {
        None
    } else {
        Some(form.paragraph_anchor.trim())
    };

    let device = if form.device_id.trim().is_empty() {
        None
    } else {
        Some(form.device_id.trim())
    };

    let result = reading::save_progress(
        state.db(),
        ProgressInput {
            account: user.account_id,
            pseud: user.pseud_id,
            subject_type: "work",
            subject_id: &work_id.to_string(),
            unit_id: Some(&unit_id.to_string()),
            revision,
            anchor,
            fraction,
            device,
        },
    )
    .await;
    if let Err(error) = result {
        tracing::error!(%error, "the position save failed");
    }

    Ok(Redirect::to(&back).into_response())
}

/// Save the reader's appearance, and return them to the page.
async fn save_typography(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path((work_id, unit_id)): Path<(String, String)>,
    Form(form): Form<TypographyForm>,
) -> ApiResult<Response> {
    let work_id = parse_work_id(&work_id)?;
    let unit_id = parse_unit_id(&unit_id)?;
    let back = format!("/works/{work_id}/units/{unit_id}");

    let Some(user) = session else {
        return Ok(Redirect::to(&back).into_response());
    };

    // The numbers are parsed and **clamped**, not trusted: a form post is
    // whatever the client sent, and a font scale of 900 is not a font scale.
    let font_scale = form
        .font_scale
        .trim()
        .parse::<f64>()
        .ok()
        .map(|value| value.clamp(0.8, 2.0))
        .unwrap_or(1.0);
    let line_height = form
        .line_height
        .trim()
        .parse::<f64>()
        .ok()
        .map(|value| value.clamp(1.0, 3.0))
        .unwrap_or(1.6);
    let measure = form
        .measure
        .trim()
        .parse::<i64>()
        .ok()
        .map(|value| value.clamp(30, 120))
        .unwrap_or(66);
    // The theme is one of the four the stylesheet defines. A theme the
    // stylesheet does not have is a theme that renders as the default, so the
    // set is closed here rather than trusted.
    let reader_theme = match form.reader_theme.trim() {
        "paper" | "white" | "sepia" | "dark" => form.reader_theme.trim().to_owned(),
        _ => "sepia".to_owned(),
    };

    let current = reading::typography_for(state.db(), user.account_id).await?;
    let _ = reading::save_typography(
        state.db(),
        reading::TypographyInput {
            account: user.account_id,
            expected_version: current.version,
            font_scale,
            line_height,
            measure,
            reader_theme: &reader_theme,
            distraction_free: current.distraction_free,
        },
    )
    .await;

    Ok(Redirect::to(&back).into_response())
}

/// What the reader asked to see.
#[derive(Debug, Default, Deserialize)]
pub struct RevealForm {
    /// `unit` for this section, or `work` for every section of the work.
    #[serde(default)]
    pub scope: String,
}

/// Reveal a gated section, and return the reader to it.
///
/// **A form post, not a fetch** (spec §10.1): the gate has to work without
/// JavaScript, and a gate that only opens with a script is a gate that only
/// opens for some readers.
///
/// The scope is the reader's choice and it is honoured exactly: `unit` reveals
/// this section, `work` reveals every section of the work. **A work can
/// require a gate, and the reader can lift it for the reading** -- which is
/// the difference between a gate and a wall.
async fn reveal_section(
    State(_state): State<AppState>,
    MaybeSession(_session): MaybeSession,
    Path((work_id, unit_id)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
    Form(form): Form<RevealForm>,
) -> ApiResult<Response> {
    let work_id = parse_work_id(&work_id)?;
    let unit_id = parse_unit_id(&unit_id)?;
    let back = format!("/works/{work_id}/units/{unit_id}");

    let reveals = Reveals::parse(
        headers
            .get(header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .and_then(|cookies| {
                cookies.split(';').find_map(|cookie| {
                    let (name, value) = cookie.split_once('=')?;
                    (name.trim() == REVEAL_COOKIE).then_some(value.trim())
                })
            }),
    );
    let scope = if form.scope.trim() == "work" {
        "*".to_owned()
    } else {
        unit_id.to_string()
    };
    let value = reveals.with(&scope);

    let cookie = format!("{REVEAL_COOKIE}={value}; Path=/; SameSite=Lax; Max-Age=86400");
    Ok((
        StatusCode::SEE_OTHER,
        [
            (header::LOCATION, back.as_str()),
            (header::SET_COOKIE, cookie.as_str()),
        ],
    )
        .into_response())
}

/// What the reader said about why they did not finish.
#[derive(Debug, Default, Deserialize)]
pub struct DnfForm {
    /// The reason, as the domain spells it.
    #[serde(default)]
    pub reason: String,
    /// A free-text note for themselves.
    #[serde(default)]
    pub note: String,
}

/// Mark the work as "did not finish", or take the mark back (spec §10.1).
///
/// **It is the reader's own note to themselves.** It is private, it is
/// reversible, and it is **never a judgement**: nothing here is sent to the
/// author, and the reader is not asked to justify themselves to anybody.
///
/// The mark is per **work**, not per unit: a reader who did not finish a work
/// did not finish the work, and marking half of it would be a distinction
/// nobody asked for.
async fn set_did_not_finish(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path((work_id, unit_id)): Path<(String, String)>,
    Form(form): Form<DnfForm>,
) -> ApiResult<Response> {
    let work_id = parse_work_id(&work_id)?;
    let unit_id = parse_unit_id(&unit_id)?;
    let back = format!("/works/{work_id}/units/{unit_id}");

    let Some(user) = session else {
        return Ok(Redirect::to(&back).into_response());
    };
    let Some(pseud) = user.pseud_id else {
        return Ok(Redirect::to(&back).into_response());
    };

    // **Reversible.** An empty reason means "take the mark back", because the
    // reader's way of undoing it should not be a second button that says so.
    let reason = form.reason.trim();
    if reason.is_empty() {
        let _ = darkhold_db::dnf::delete_dnf(
            state.db(),
            pseud,
            work_id,
            &darkhold_db::identity::now_rfc3339(),
        )
        .await;
        return Ok(Redirect::to(&back).into_response());
    }

    // The reason is one of the domain's, or the reader's own words. An
    // unknown reason is **not** stored as an unknown reason: it is stored as
    // "other", because the reader said something and the vocabulary did not
    // have a word for it.
    let reason = match reason.parse::<darkhold_domain::dnf::DnfReason>() {
        Ok(reason) => reason.as_str().to_owned(),
        Err(_) => darkhold_domain::dnf::DnfReason::Other.as_str().to_owned(),
    };

    // **The error is logged, not swallowed.** A swallowed error here is a
    // silent failure: the reader presses the button, the page comes back
    // saying they have not marked it, and nothing says why.
    if let Err(error) = darkhold_db::dnf::upsert_dnf(
        state.db(),
        user.account_id,
        pseud,
        work_id,
        &reason,
        if form.note.trim().is_empty() {
            None
        } else {
            Some(form.note.trim())
        },
        // **Private.** The mark is the reader's own; the author is told about
        // the *aggregate*, never about the reader.
        false,
        &darkhold_db::identity::now_rfc3339(),
    )
    .await
    {
        tracing::error!(%error, "the did-not-finish mark was not saved");
    }

    Ok(Redirect::to(&back).into_response())
}

/// Save a reader's note, and return them to the page.
async fn save_note(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path((work_id, unit_id)): Path<(String, String)>,
    Form(form): Form<NoteForm>,
) -> ApiResult<Response> {
    let work_id = parse_work_id(&work_id)?;
    let unit_id = parse_unit_id(&unit_id)?;
    let back = format!("/works/{work_id}/units/{unit_id}");

    let Some(user) = session else {
        return Ok(Redirect::to(&back).into_response());
    };
    let Some(pseud) = user.pseud_id else {
        return Ok(Redirect::to(&back).into_response());
    };

    let body = form.body.trim();
    if !body.is_empty() {
        let _ = reading::save_note(
            state.db(),
            user.account_id,
            pseud,
            "work",
            &work_id.to_string(),
            None,
            body,
        )
        .await;
    }

    Ok(Redirect::to(&back).into_response())
}

/// Delete a reader's note, and return them to the page.
async fn delete_note(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path((work_id, unit_id, note_id)): Path<(String, String, String)>,
) -> ApiResult<Response> {
    let work_id = parse_work_id(&work_id)?;
    let unit_id = parse_unit_id(&unit_id)?;
    let back = format!("/works/{work_id}/units/{unit_id}");

    if let Some(user) = session {
        if let Some(pseud) = user.pseud_id {
            let _ = reading::delete_note(state.db(), pseud, &note_id).await;
        }
    }

    Ok(Redirect::to(&back).into_response())
}

/// Render the reading page.
#[allow(clippy::too_many_arguments)]
fn render_page(
    work: &content::Work,
    unit: &content::Unit,
    sanitized_html: Option<&str>,
    units: &[content::Unit],
    previous: Option<&content::Unit>,
    next: Option<&content::Unit>,
    typography: &reading::Typography,
    stylesheet: Option<&str>,
    notes: &[reading::Note],
    narration: Option<&str>,
    warnings: &[darkhold_db::taxonomy::WorkWarning],
    signed_in: bool,
    gated: bool,
    position: Option<&darkhold_domain::reading::ReadingPosition>,
    stale_position: bool,
    did_not_finish: Option<&darkhold_db::dnf::DnfRow>,
) -> String {
    let mut page = Html::new();

    page.raw("<!DOCTYPE html>");
    // The reader's theme is the reader's own: the `data-reader` attribute
    // selects the reader scale, and the site scale is a different set of
    // variables entirely (spec §10.1, §21.1).
    page.open(
        "html",
        &[("lang", "en"), ("data-reader", &typography.reader_theme)],
    );
    page.open("head", &[]);
    page.void("meta", &[("charset", "utf-8")]);
    page.void(
        "meta",
        &[
            ("name", "viewport"),
            ("content", "width=device-width, initial-scale=1"),
        ],
    );
    let title = if unit.title.trim().is_empty() {
        work.title.clone()
    } else {
        format!("{} — {}", unit.title, work.title)
    };
    page.open("title", &[]).text(&title).close("title");
    // The stylesheet is the one the application already ships, so the reader's
    // scale and the site's scale are literally the same source of truth.
    if let Some(href) = stylesheet {
        page.void("link", &[("rel", "stylesheet"), ("href", href)]);
    }
    // The reader's own appearance, written inline so it overrides the
    // stylesheet's reader scale without touching the site's scale.
    let properties = crate::html::style_block(&[
        (
            "--reader-font-scale",
            &format!("{:.2}", typography.font_scale),
        ),
        (
            "--reader-line-height",
            &format!("{:.2}", typography.line_height),
        ),
        ("--reader-measure", &format!("{}ch", typography.measure)),
    ]);
    page.open("style", &[]).raw(&properties).close("style");
    page.close("head");

    page.open("body", &[("class", "reader-surface")]);

    // ── the bar: the work, and the way back ────────────────────────────────
    page.open("header", &[("class", "reader-surface__bar")]);
    page.open(
        "a",
        &[
            ("class", "reader-surface__work"),
            ("href", &work_href(work)),
        ],
    );
    page.text(&work.title);
    page.close("a");
    page.close("header");

    // ── the table of contents ─────────────────────────────────────────────
    if units.len() > 1 {
        page.open(
            "nav",
            &[("class", "reader-surface__toc"), ("aria-label", "Contents")],
        );
        page.open("ol", &[]);
        for candidate in units {
            let is_current = candidate.id == unit.id;
            page.open("li", &[]);
            page.open_opt(
                "a",
                &[
                    ("href", Some(&unit_href(work, candidate))),
                    ("class", Some(if is_current { "is-current" } else { "" })),
                    ("aria-current", if is_current { Some("page") } else { None }),
                ],
            );
            let label = if candidate.title.trim().is_empty() {
                format!("Unit {}", candidate.order_key / 10)
            } else {
                candidate.title.clone()
            };
            page.text(&label);
            page.close("a");
            page.close("li");
        }
        page.close("ol");
        page.close("nav");
    }

    // ── the warnings ──────────────────────────────────────────────────────
    //
    // **Above the text, and visible without asking.** A warning is
    // information (spec §10.2): a reader who filters nothing sees everything,
    // *with the warnings attached*. A warning that a reader has to go looking
    // for is a warning that protects nobody.
    if !warnings.is_empty() {
        page.open("section", &[("class", "reader-surface__warnings")]);
        page.open("h2", &[("class", "reader-surface__warnings-title")]);
        page.text("Content warnings");
        page.close("h2");
        page.open("ul", &[]);
        for warning in warnings {
            page.open("li", &[("class", "reader-surface__warning")]);
            page.open("span", &[("class", "reader-surface__warning-name")]);
            page.text(&warning.canonical);
            page.close("span");
            // The severity, in words. A number is a scale nobody can read.
            page.text(if warning.severity >= 2 {
                " (heavy)"
            } else {
                " (light)"
            });
            if let Some(note) = warning.spoiler_safe_note.as_deref() {
                page.open("span", &[("class", "reader-surface__warning-note")]);
                page.text(note);
                page.close("span");
            }
            page.close("li");
        }
        page.close("ul");
        page.close("section");
    }

    // ── the unit ──────────────────────────────────────────────────────────
    page.open("main", &[("class", "reader-surface__main")]);
    page.open("article", &[("class", "reader-surface__unit")]);
    if !unit.title.trim().is_empty() {
        page.open("h1", &[("class", "reader-surface__title")]);
        page.text(&unit.title);
        page.close("h1");
    }
    page.open("div", &[("class", "reader-surface__body")]);
    if gated {
        // **The text is NOT here.** A gate that leaves the text in the page
        // and hides it with CSS is a gate that a reader can walk past with
        // the source view, and it is a gate that a screen reader reads aloud
        // and a search engine indexes.
        page.open("section", &[("class", "reader-surface__gate")]);
        page.open("h2", &[])
            .text("This part is behind a gate")
            .close("h2");
        page.open("p", &[]);
        page.text(
            "The author has marked this part of the work as a spoiler. It is not \
             on this page until you ask for it.",
        );
        page.close("p");
        page.open(
            "form",
            &[
                ("class", "reader-surface__gate-form"),
                ("method", "post"),
                (
                    "action",
                    &format!("/works/{}/units/{}/reveal", work.id, unit.id),
                ),
            ],
        );
        page.void(
            "input",
            &[("type", "hidden"), ("name", "scope"), ("value", "unit")],
        );
        page.open("button", &[("type", "submit")]);
        page.text("Show me this part");
        page.close("button");
        page.close("form");
        page.open(
            "form",
            &[
                ("class", "reader-surface__gate-form"),
                ("method", "post"),
                (
                    "action",
                    &format!("/works/{}/units/{}/reveal", work.id, unit.id),
                ),
            ],
        );
        page.void(
            "input",
            &[("type", "hidden"), ("name", "scope"), ("value", "work")],
        );
        page.open("button", &[("type", "submit")]);
        page.text("Show me the whole work from now on");
        page.close("button");
        page.close("form");
        page.close("section");
    } else {
        match sanitized_html {
            // The one raw insertion on this page, and it is markup the SERVER
            // produced (spec §8.3, ADR 0002).
            Some(html) => {
                page.raw(html);
            }
            // A unit with no revision yet has no body. The empty state explains
            // itself (spec §21.1): "every empty state explains itself".
            None => {
                page.open("p", &[("class", "reader-surface__empty")]);
                page.text("This unit has no text yet.");
                page.close("p");
            }
        }
    }
    page.close("div");
    page.close("article");

    // ── the way on ────────────────────────────────────────────────────────
    page.open("nav", &[("class", "reader-surface__nav")]);
    match previous {
        Some(previous) => {
            page.open(
                "a",
                &[
                    ("class", "reader-surface__prev"),
                    ("rel", "prev"),
                    ("href", &unit_href(work, previous)),
                ],
            );
            page.text("Previous");
            page.close("a");
        }
        None => {
            page.open("span", &[("class", "reader-surface__prev is-absent")]);
            page.text("Previous");
            page.close("span");
        }
    }
    match next {
        Some(next) => {
            page.open(
                "a",
                &[
                    ("class", "reader-surface__next"),
                    ("rel", "next"),
                    ("href", &unit_href(work, next)),
                ],
            );
            page.text("Next");
            page.close("a");
        }
        None => {
            page.open("span", &[("class", "reader-surface__next is-absent")]);
            page.text("Next");
            page.close("span");
        }
    }
    page.close("nav");

    // ── the narration ─────────────────────────────────────────────────────
    //
    // An `<audio>` element and nothing else. The edition's audio is served by
    // the instance's own media route, so the player works without JavaScript.
    if let Some(edition) = narration {
        page.open("section", &[("class", "reader-surface__narration")]);
        page.open(
            "audio",
            &[
                ("controls", "controls"),
                ("src", &format!("/editions/{edition}/audio")),
                ("preload", "metadata"),
            ],
        );
        page.close("audio");
        page.close("section");
    }

    // ── "did not finish" ──────────────────────────────────────────────────
    //
    // **The reader's own note to themselves** (spec §10.1). It is private,
    // it is reversible, and it is never a judgement: the page says what it
    // is, and it offers the way to take it back.
    if signed_in {
        page.open("section", &[("class", "reader-surface__dnf")]);
        match did_not_finish {
            Some(mark) => {
                page.open("p", &[("class", "reader-surface__dnf-note")]);
                // The reason is shown to the reader, because a note they cannot
                // read back is not a note. It is **private**: only they see it.
                let label = mark
                    .reason
                    .parse::<darkhold_domain::dnf::DnfReason>()
                    .map(|reason| reason.label())
                    .unwrap_or("For a reason of your own");
                page.text(&format!(
                    "You marked this as did not finish ({label}). Only you can see that."
                ));
                page.close("p");
                page.open(
                    "form",
                    &[
                        ("class", "reader-surface__dnf-form"),
                        ("method", "post"),
                        (
                            "action",
                            &format!("/works/{}/units/{}/dnf", work.id, unit.id),
                        ),
                    ],
                );
                page.void(
                    "input",
                    &[("type", "hidden"), ("name", "reason"), ("value", "")],
                );
                page.open("button", &[("type", "submit")]);
                page.text("Take that back");
                page.close("button");
                page.close("form");
            }
            None => {
                page.open("p", &[("class", "reader-surface__dnf-note")]);
                page.text("Did not finish it? That is between you and the work.");
                page.close("p");
                page.open(
                    "form",
                    &[
                        ("class", "reader-surface__dnf-form"),
                        ("method", "post"),
                        (
                            "action",
                            &format!("/works/{}/units/{}/dnf", work.id, unit.id),
                        ),
                    ],
                );
                page.open("label", &[("for", "reader-dnf-reason")]);
                page.text("Why not?");
                page.close("label");
                page.open("select", &[("id", "reader-dnf-reason"), ("name", "reason")]);
                for reason in [
                    darkhold_domain::dnf::DnfReason::NotMyTaste,
                    darkhold_domain::dnf::DnfReason::Triggering,
                    darkhold_domain::dnf::DnfReason::SlowPacing,
                    darkhold_domain::dnf::DnfReason::AbandonedByAuthor,
                    darkhold_domain::dnf::DnfReason::DroppedOther,
                ] {
                    page.open("option", &[("value", reason.as_str())]);
                    page.text(reason.label());
                    page.close("option");
                }
                page.close("select");
                page.open("label", &[("for", "reader-dnf-note")]);
                page.text("Anything to add? (nobody else sees this)");
                page.close("label");
                page.open(
                    "input",
                    &[
                        ("type", "text"),
                        ("id", "reader-dnf-note"),
                        ("name", "note"),
                    ],
                );
                page.close("input");
                page.open("button", &[("type", "submit")]);
                page.text("Mark it");
                page.close("button");
                page.close("form");
            }
        }
        page.close("section");
    }

    // ── the reader's notes ────────────────────────────────────────────────
    //
    // Private to the reader's own face (spec §10.3), and a **form**: a note
    // is written without JavaScript, and a note that could only be written by
    // a fetch would be a note a reader without JavaScript could not keep.
    if signed_in {
        page.open("section", &[("class", "reader-surface__notes")]);
        page.open("h2", &[]).text("Your notes").close("h2");
        if notes.is_empty() {
            page.open("p", &[("class", "reader-surface__empty")]);
            page.text("You have not written a note on this work.");
            page.close("p");
        } else {
            page.open("ul", &[("class", "reader-surface__note-list")]);
            for note in notes {
                page.open("li", &[]);
                page.open("p", &[]).text(&note.body).close("p");
                page.open(
                    "form",
                    &[
                        ("class", "reader-surface__note-delete"),
                        ("method", "post"),
                        (
                            "action",
                            &format!(
                                "/works/{}/units/{}/notes/{}/delete",
                                work.id, unit.id, note.id
                            ),
                        ),
                    ],
                );
                page.open("button", &[("type", "submit")]);
                page.text("Delete this note");
                page.close("button");
                page.close("form");
                page.close("li");
            }
            page.close("ul");
        }
        page.open(
            "form",
            &[
                ("class", "reader-surface__note-form"),
                ("method", "post"),
                (
                    "action",
                    &format!("/works/{}/units/{}/notes", work.id, unit.id),
                ),
            ],
        );
        page.open("label", &[("for", "reader-note")]);
        page.text("Write a note to yourself about this work.");
        page.close("label");
        page.open(
            "textarea",
            &[("id", "reader-note"), ("name", "body"), ("rows", "3")],
        );
        page.close("textarea");
        page.open("button", &[("type", "submit")]);
        page.text("Save the note");
        page.close("button");
        page.close("form");
        page.close("section");
    }

    // ── the typography control ────────────────────────────────────────────
    //
    // The reader's appearance is the reader's own (spec §10.1, §21.1), and the
    // control is a **form**: a reader without JavaScript must be able to
    // change how the page looks, exactly as they must be able to read it.
    if signed_in {
        page.open("section", &[("class", "reader-surface__typography")]);
        page.open("h2", &[]).text("How this page looks").close("h2");
        page.open(
            "form",
            &[
                ("method", "post"),
                (
                    "action",
                    &format!("/works/{}/units/{}/typography", work.id, unit.id),
                ),
            ],
        );
        page.open("label", &[("for", "reader-font-scale")]);
        page.text("Text size");
        page.close("label");
        page.open(
            "input",
            &[
                ("type", "range"),
                ("id", "reader-font-scale"),
                ("name", "font_scale"),
                ("min", "0.8"),
                ("max", "2"),
                ("step", "0.05"),
                ("value", &format!("{:.2}", typography.font_scale)),
            ],
        );
        page.close("input");
        page.open("label", &[("for", "reader-line-height")]);
        page.text("Line spacing");
        page.close("label");
        page.open(
            "input",
            &[
                ("type", "range"),
                ("id", "reader-line-height"),
                ("name", "line_height"),
                ("min", "1"),
                ("max", "3"),
                ("step", "0.05"),
                ("value", &format!("{:.2}", typography.line_height)),
            ],
        );
        page.close("input");
        page.open("label", &[("for", "reader-measure")]);
        page.text("Line length");
        page.close("label");
        page.open(
            "input",
            &[
                ("type", "number"),
                ("id", "reader-measure"),
                ("name", "measure"),
                ("min", "30"),
                ("max", "120"),
                ("value", &typography.measure.to_string()),
            ],
        );
        page.close("input");
        page.open("label", &[("for", "reader-theme")]);
        page.text("Page colour");
        page.close("label");
        page.open(
            "select",
            &[("id", "reader-theme"), ("name", "reader_theme")],
        );
        for theme in ["paper", "white", "sepia", "dark"] {
            let selected = theme == typography.reader_theme;
            page.open_opt(
                "option",
                &[
                    ("value", Some(theme)),
                    ("selected", if selected { Some("selected") } else { None }),
                ],
            );
            page.text(theme);
            page.close("option");
        }
        page.close("select");
        page.open("button", &[("type", "submit")]);
        page.text("Save");
        page.close("button");
        page.close("form");
        page.close("section");
    }

    // ── the progress indicator ────────────────────────────────────────────
    //
    // **The reader's real position in the CURRENT UNIT** (spec §10.1) -- not
    // an estimate, and not a percentage of the whole work presented as a
    // position.
    //
    // And when the position is unknown the indicator **says so**. It does not
    // show 0 %: a reader told they are at the beginning of a unit they have
    // read is told something false, and a false 0 % is worse than no number.
    page.open("div", &[("class", "reader-surface__progress")]);
    match position {
        Some(position) => {
            let percent = position.fraction / 10;
            page.open(
                "div",
                &[
                    ("class", "reader-surface__progress-bar"),
                    ("role", "progressbar"),
                    ("aria-valuenow", &percent.to_string()),
                    ("aria-valuemin", "0"),
                    ("aria-valuemax", "100"),
                    ("aria-label", "Your place in this unit"),
                    // The bar's width, as a custom property. The number is
                    // also in `aria-valuenow`, so the value is in the
                    // accessibility tree and not only in the picture.
                    ("style", &format!("--reader-progress: {percent}")),
                ],
            );
            page.open("span", &[("class", "reader-surface__progress-value")]);
            page.text(&format!("{percent}%"));
            page.close("span");
            page.close("div");
            page.open("span", &[("class", "reader-surface__progress-note")]);
            page.text("of this unit");
            page.close("span");
        }
        None => {
            page.open("span", &[("class", "reader-surface__progress-unknown")]);
            page.text("Your place in this unit is not known yet.");
            page.close("span");
        }
    }
    page.close("div");

    // ── the position form ─────────────────────────────────────────────────
    //
    // The form is the **no-JavaScript path**, and it is always present: with
    // JavaScript the same form carries the live position, and without it the
    // reader presses the button and their place is recorded. A form that
    // appears only when JavaScript runs is a form that fails when it does not.
    page.open(
        "form",
        &[
            ("class", "reader-surface__position"),
            ("method", "post"),
            (
                "action",
                &format!("/works/{}/units/{}/position", work.id, unit.id),
            ),
        ],
    );
    page.open("label", &[("for", "reader-position")]);
    page.text("Where did you stop?");
    page.close("label");
    // The saved position, when there is one that can be trusted. The reader's
    // place follows them across devices (spec §10.3), so the form opens where
    // they left off and not at the top.
    let saved = position.map(|position| position.fraction.to_string());
    page.open(
        "input",
        &[
            ("type", "range"),
            ("id", "reader-position"),
            ("name", "position_permille"),
            ("min", "0"),
            ("max", "1000"),
            ("value", saved.as_deref().unwrap_or("0")),
            ("step", "10"),
        ],
    );
    page.close("input");
    if let Some(position) = position {
        let percent = position.fraction / 10;
        page.open("p", &[("class", "reader-surface__position-saved")]);
        page.text(&format!(
            "You were {percent}% of the way through this unit last time."
        ));
        page.close("p");
    }
    if stale_position {
        // **A stale position is said to be stale.** The unit has been revised
        // since the reader's place was recorded, so the fraction would point at
        // text that is no longer there. Silently applying it would put a
        // reader at a place in a text they have not read.
        page.open("p", &[("class", "reader-surface__position-stale")]);
        page.text(
            "Your saved place was recorded against an earlier version of this \
             unit, so it has not been applied. The text may have changed since \
             you were there.",
        );
        page.close("p");
    }
    page.void(
        "input",
        &[
            ("type", "hidden"),
            ("name", "unit_id"),
            ("value", &unit.id.to_string()),
        ],
    );
    if let Some(revision) = unit.current_revision_id {
        page.void(
            "input",
            &[
                ("type", "hidden"),
                ("name", "content_revision"),
                ("value", &revision.to_string()),
            ],
        );
    }
    page.open("button", &[("type", "submit")]);
    page.text("Save my place");
    page.close("button");
    page.close("form");

    page.close("main");
    page.close("body");
    page.close("html");

    page.finish()
}

/// The page URL for a work.
fn work_href(work: &content::Work) -> String {
    format!("/works/{}", work.id)
}

/// The page URL for a unit of a work.
fn unit_href(work: &content::Work, unit: &content::Unit) -> String {
    format!("/works/{}/units/{}", work.id, unit.id)
}
