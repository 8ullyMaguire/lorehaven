//! Royal Road.
//!
//! # Recognised URLs
//!
//! ```text
//! https://www.royalroad.com/fiction/{fiction_id}/{slug}
//! https://www.royalroad.com/fiction/{fiction_id}/{slug}/chapter/{chapter_id}/{slug}
//! ```
//!
//! A chapter URL previews the work it belongs to, because the work page is the
//! first three path segments of either shape.
//!
//! # Why this adapter does not look like the others
//!
//! Every other adapter in this crate reads a chapter body out of the same page
//! that lists the chapters. Royal Road does not work that way: the work page
//! carries the metadata and a table of chapter *links*, and each chapter is its
//! own document. So [`RoyalRoad::fetch_chapters`] is a loop, and a work of 109
//! chapters costs 110 requests. That is the site's shape, not a shortcut, and it
//! is why [`SourceCapabilities::per_chapter_fetch`] is true here — the import can
//! re-read one failed chapter without repeating the other 108.
//!
//! # What the markup actually is
//!
//! The selectors were derived from pages recorded on 2026-09-10 (see
//! `tests/fixtures/royalroad/`), and the earlier port of this adapter was wrong
//! about several of them. Written down because the difference is the point:
//!
//! * `h1.font-white` is the title, on both the work page and a chapter page.
//! * There is **no** `h2.chapter-title`, and no `div[property='description']`,
//!   and no `span[property='genre']` — all three appear in the ported code and
//!   match nothing on the live site. Metadata comes from the page's
//!   `application/ld+json` block instead, which is the site's own structured
//!   description of the work and is far less likely to drift than its CSS.
//! * The chapter list is `#chapters tbody tr.chapter-row`, each row carrying its
//!   chapter's `data-url`. The table's `data-chapters` attribute states the count;
//!   the fixture asserts the two agree, because a row list that silently drops
//!   rows is the failure mode this whole crate exists to avoid.
//! * `time[unixtime]` gives each chapter's real release date, so `published_at`
//!   and `updated_at` are the site's dates rather than the moment of the import.
//!
//! # What this adapter does *not* do
//!
//! It does not enumerate an author's works, so
//! [`SourceCapabilities::bibliography`] is false. Royal Road has author profiles
//! at `/profile/{id}` and they do list a writer's fiction, but that markup has
//! not been recorded, and an adapter written against a page nobody has looked at
//! is a guess. It is a small, known gap rather than an oversight.
//!
//! Tags are carried as display text (`Time Loop`, `Adventure`, `Fantasy`) and
//! deliberately not mapped onto anything: Lorehaven's taxonomy arrives in M9,
//! and `SourceWork::tags` exists so an adapter can pass them through without
//! inventing tag types.

use scraper::{Html, Selector};
use serde_json::Value;
use time::format_description::well_known::Iso8601;
use time::OffsetDateTime;
use url::Url;

use async_trait::async_trait;

use crate::sanitize::sanitize_fragment;
use crate::{
    collapse_whitespace, strip_tags, text_of, texts_of, ChapterRef, Credentials, Fetcher,
    SourceAdapter, SourceCapabilities, SourceChapter, SourceError, SourceKey, SourceResult,
    SourceWork, WorkStatus,
};

/// The host this adapter serves.
///
/// `www` is trimmed before comparison, so this one entry covers both
/// `royalroad.com` and `www.royalroad.com`.
const HOST: &str = "royalroad.com";

/// Royal Road.
#[derive(Debug, Clone)]
pub struct RoyalRoad {
    key: SourceKey,
    hosts: Vec<String>,
}

impl RoyalRoad {
    /// A new adapter.
    pub fn new() -> Self {
        Self {
            key: SourceKey::new("royalroad"),
            hosts: vec![HOST.to_owned()],
        }
    }

    fn host_matches(&self, host: &str) -> bool {
        let host = host.trim_start_matches("www.");
        self.hosts.iter().any(|ours| host == ours)
    }

    /// The numeric fiction id in a `/fiction/{id}/...` path.
    ///
    /// Returns `None` for a path that is not a fiction — including the site's
    /// own `/fictions/search`, which shares the prefix and is not a work.
    fn fiction_id(url: &Url) -> Option<String> {
        let mut segments = url.path_segments()?;
        if segments.next()? != "fiction" {
            return None;
        }
        let id = segments.next()?;
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        Some(id.to_owned())
    }

    /// The work page for either URL shape.
    ///
    /// A chapter URL and a work URL differ only after the third path segment, so
    /// truncating there turns one into the other. A `/fiction/{id}` with no slug
    /// is left as it is and relies on the site's own redirect — the fetcher
    /// re-validates every hop, so following it is not a hole.
    fn work_url(url: &Url) -> Option<String> {
        let id = RoyalRoad::fiction_id(url)?;
        let segments: Vec<&str> = url.path_segments()?.collect();
        match segments.get(2) {
            Some(slug) if !slug.is_empty() => {
                Some(format!("https://www.{HOST}/fiction/{id}/{slug}"))
            }
            _ => Some(format!("https://www.{HOST}/fiction/{id}")),
        }
    }

    /// Parse a work page.
    fn parse_work(&self, html: &str, url: &Url) -> SourceResult<SourceWork> {
        let document = Html::parse_document(html);
        if is_not_found(&document) {
            return Err(SourceError::NotFound);
        }

        let fiction_id = RoyalRoad::fiction_id(url)
            .or_else(|| {
                // A redirect lands on the canonical URL, which always has an id.
                attr_of_fiction_id(&document)
            })
            .unwrap_or_default();

        let structured = json_ld(&document);

        let title = structured
            .as_ref()
            .and_then(|value| json_str(value, "name"))
            .or_else(|| text_of(&document, "h1.font-white"))
            .ok_or_else(|| {
                SourceError::Parse(
                    "the work page carried neither a JSON-LD name nor an `h1.font-white`"
                        .to_owned(),
                )
            })?;
        let title = collapse_whitespace(&title);
        if title.is_empty() {
            return Err(SourceError::Parse(
                "the work page's title is empty".to_owned(),
            ));
        }

        // The description is HTML in the structured block, so it is reduced to
        // text: `summary` is prose, and a reader's summary that still had
        // `<p>` in it would be rendered as literal angle brackets.
        let summary = structured
            .as_ref()
            .and_then(|value| json_str(value, "description"))
            .map(|raw| collapse_whitespace(&strip_tags(&raw)))
            .unwrap_or_default();

        let author_text = structured
            .as_ref()
            .and_then(|value| value.get("author"))
            .and_then(|author| {
                json_str(author, "name").or_else(|| {
                    // `author` may be a list on some works.
                    author
                        .as_array()?
                        .first()
                        .and_then(|first| json_str(first, "name"))
                })
            })
            .map(|name| collapse_whitespace(&name))
            .unwrap_or_default();

        let author_url = structured
            .as_ref()
            .and_then(|value| value.get("author"))
            .and_then(|author| json_str(author, "url"));

        let language = structured
            .as_ref()
            .and_then(|value| json_str(value, "inLanguage"))
            .map(|tag| collapse_whitespace(&tag))
            .filter(|tag| !tag.is_empty());

        let published_at = structured
            .as_ref()
            .and_then(|value| {
                json_str(value, "datePublished").or_else(|| json_str(value, "dateCreated"))
            })
            .and_then(|raw| parse_date(&raw));
        let updated_at = structured
            .as_ref()
            .and_then(|value| json_str(value, "dateModified"))
            .and_then(|raw| parse_date(&raw));

        let rows = parse_chapter_rows(&document);

        // A page that states a chapter count and a page whose rows disagree is
        // the exact failure this parser must not paper over: it means the table
        // shape moved and we are reading a subset. Said out loud rather than
        // returning a short list that looks like a short work.
        if let Some(stated) = stated_chapter_count(&document) {
            if rows.len() as i64 != stated {
                return Err(SourceError::Parse(format!(
                    "the work page lists {} chapter rows but states {stated} chapters",
                    rows.len()
                )));
            }
        }
        if rows.is_empty() {
            return Err(SourceError::Parse(
                "the work page listed no chapters at all".to_owned(),
            ));
        }

        let chapters = rows
            .iter()
            .enumerate()
            .map(|(index, row)| ChapterRef {
                ordinal: (index as u32) + 1,
                source_chapter_key: row.chapter_id.clone(),
                title: row.title.clone(),
            })
            .collect();

        // The site's own word count, from the Pages tooltip, which states it as
        // "calculated from 806,306 words". Absent rather than zero when the
        // tooltip is not there.
        let word_count = word_count(&document);

        let canonical = RoyalRoad::work_url(url).unwrap_or_else(|| url.to_string());

        Ok(SourceWork {
            source_key: self.key.clone(),
            source_work_key: fiction_id,
            source_url: canonical,
            title,
            author_text,
            author_url,
            summary,
            word_count,
            language,
            status: work_status(&document),
            published_at,
            updated_at,
            chapters,
            // Royal Road shows no content rating; its only related signal is
            // `isFamilyFriendly` in the structured block, and turning that into
            // one of Lorehaven's ratings is a classification decision (spec
            // §12.2) rather than something this crate should decide.
            rating_text: None,
            // No content warnings are published in a form a reader would
            // recognise as one.
            warning_texts: Vec::new(),
            tags: work_tags(&document),
        })
    }

    /// The chapter rows of a work page, in the page's order.
    fn parse_rows_or_error(&self, html: &str, work: &SourceWork) -> SourceResult<Vec<ChapterRow>> {
        let document = Html::parse_document(html);
        if is_not_found(&document) {
            return Err(SourceError::NotFound);
        }
        let rows = parse_chapter_rows(&document);
        if rows.is_empty() {
            return Err(SourceError::Parse(format!(
                "the work page for {} listed no chapters",
                work.source_work_key
            )));
        }
        Ok(rows)
    }

    /// Parse one chapter document.
    ///
    /// A chapter page holds exactly one chapter and does not state its position
    /// in the work, so the ordinal comes from the caller when it is known
    /// (a retry, or a walk of the chapter list) and is otherwise recovered by
    /// matching the chapter's title against the work's own list.
    fn parse_chapter_page(
        &self,
        html: &str,
        work: &SourceWork,
        ordinal: Option<u32>,
    ) -> SourceResult<SourceChapter> {
        let document = Html::parse_document(html);
        if is_not_found(&document) {
            return Err(SourceError::NotFound);
        }

        let title = text_of(&document, "h1.font-white")
            .map(|raw| collapse_whitespace(&raw))
            .unwrap_or_default();

        let body = chapter_body(&document).ok_or_else(|| {
            SourceError::Parse(
                "the chapter page carried no `div.chapter-content`, so its prose could not be \
                 found"
                    .to_owned(),
            )
        })?;

        // Relative links inside a chapter are resolved against the work page, so
        // a link the author wrote survives the import instead of becoming a
        // dead relative path on our domain.
        let base = Url::parse(&work.source_url).ok();
        let content_html = sanitize_fragment(&body, base.as_ref());

        // The ordinal, in order of decreasing trust: what the caller asked for,
        // then the work's own list matched by title, then the "17." prefix the
        // site puts on a numbered chapter's title. A chapter page whose position
        // cannot be established at all is a parse failure rather than chapter 1.
        let referenced = work
            .chapters
            .iter()
            .find(|candidate| !title.is_empty() && candidate.title == title);
        let (ordinal, source_chapter_key, title) = match (ordinal, referenced) {
            (Some(ordinal), Some(reference)) => (
                ordinal,
                reference.source_chapter_key.clone(),
                if title.is_empty() {
                    reference.title.clone()
                } else {
                    title
                },
            ),
            (Some(ordinal), None) => (ordinal, format!("ordinal:{ordinal}"), title),
            (None, Some(reference)) => (
                reference.ordinal,
                reference.source_chapter_key.clone(),
                if title.is_empty() {
                    reference.title.clone()
                } else {
                    title
                },
            ),
            (None, None) => {
                let ordinal = leading_ordinal(&title).ok_or_else(|| {
                    SourceError::Parse(format!(
                        "the chapter page's title {title:?} does not appear in the work's chapter \
                         list and does not begin with a chapter number, so there is no way to say \
                         which chapter it is"
                    ))
                })?;
                let key = work
                    .chapters
                    .iter()
                    .find(|candidate| candidate.ordinal == ordinal)
                    .map(|candidate| candidate.source_chapter_key.clone())
                    .unwrap_or_else(|| format!("ordinal:{ordinal}"));
                (ordinal, key, title)
            }
        };

        Ok(SourceChapter {
            ordinal,
            source_chapter_key,
            title,
            content_html,
        })
    }
}

impl Default for RoyalRoad {
    fn default() -> Self {
        Self::new()
    }
}

/// One row of a work page's chapter table.
#[derive(Debug, Clone)]
struct ChapterRow {
    /// The site's chapter id, from the row's `data-url`.
    chapter_id: String,
    /// The row's `data-url`, a site-absolute path.
    path: String,
    /// The chapter's title as the table shows it.
    title: String,
    /// The release date the row states, when it states one.
    released: Option<OffsetDateTime>,
}

#[async_trait]
impl SourceAdapter for RoyalRoad {
    fn key(&self) -> SourceKey {
        self.key.clone()
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities {
            metadata: true,
            chapters: true,
            // Each chapter is its own document, so a retry can re-read one
            // without repeating the others. This is the capability the plan's
            // retry criterion needs, and it is genuinely available here.
            per_chapter_fetch: true,
            // Author profiles exist but their markup has not been recorded.
            bibliography: false,
            // The structured block carries `dateModified`, so "has this changed?"
            // costs one work-page request.
            incremental: true,
            authentication: crate::AuthKind::None,
            // Royal Road documents no rate limit. This is a courtesy value for a
            // site with no challenge wall: the fetcher enforces it per host, and
            // reading a 109-chapter work through it takes about three minutes.
            min_interval_millis: Some(1_500),
        }
    }

    fn can_handle(&self, url: &Url) -> bool {
        let Some(host) = url.host_str() else {
            return false;
        };
        self.host_matches(host) && RoyalRoad::fiction_id(url).is_some()
    }

    fn hosts(&self) -> Vec<String> {
        self.hosts.clone()
    }

    async fn preview(
        &self,
        fetch: &dyn Fetcher,
        url: &Url,
        _creds: Option<&Credentials>,
    ) -> SourceResult<SourceWork> {
        // Normalised first, so pasting a chapter URL previews the whole work.
        let target = RoyalRoad::work_url(url).unwrap_or_else(|| url.to_string());
        let page = fetch.get(&target).await?;
        let parsed = Url::parse(&page.final_url).unwrap_or_else(|_| url.clone());
        self.parse_work(&page.body, &parsed)
    }

    async fn fetch_chapters(
        &self,
        fetch: &dyn Fetcher,
        work: &SourceWork,
        _creds: Option<&Credentials>,
    ) -> SourceResult<Vec<SourceChapter>> {
        // The work page is re-read first so the chapter URLs are the site's
        // current ones rather than whatever a preview saw some time ago.
        let listing = fetch.get(&work.source_url).await?;
        let rows = self.parse_rows_or_error(&listing.body, work)?;

        // The ordinal is the page's order, and the chapter's own id is carried
        // in the row — so a retry can find this chapter again by id even if the
        // work has gained chapters since.
        let mut chapters = Vec::with_capacity(rows.len());
        for (index, row) in rows.iter().enumerate() {
            let ordinal = (index as u32) + 1;
            let page = fetch.get(&absolute(&row.path)).await?;
            let mut chapter = self.parse_chapter_page(&page.body, work, Some(ordinal))?;
            chapter.ordinal = ordinal;
            chapter.source_chapter_key = row.chapter_id.clone();
            if chapter.title.is_empty() {
                chapter.title = row.title.clone();
            }
            let _ = row.released;
            chapters.push(chapter);
        }
        Ok(chapters)
    }

    async fn fetch_chapter(
        &self,
        fetch: &dyn Fetcher,
        work: &SourceWork,
        ordinal: u32,
        _creds: Option<&Credentials>,
    ) -> SourceResult<SourceChapter> {
        // The chapter id comes from the work's own list, but its URL needs the
        // slug the site generates from the chapter's title — and a slug-less
        // chapter URL is a 404. So the work page is read once to resolve id to
        // path, and then exactly one chapter body is fetched. The rest of the
        // work is not touched: that is the whole point of the capability.
        let wanted = work
            .chapters
            .iter()
            .find(|candidate| candidate.ordinal == ordinal)
            .ok_or_else(|| {
                SourceError::Unsupported(format!(
                    "work {} has no chapter {ordinal}",
                    work.source_work_key
                ))
            })?;

        let listing = fetch.get(&work.source_url).await?;
        let rows = self.parse_rows_or_error(&listing.body, work)?;
        let row = rows
            .iter()
            .find(|row| row.chapter_id == wanted.source_chapter_key)
            .ok_or(SourceError::NotFound)?;

        let page = fetch.get(&absolute(&row.path)).await?;
        let mut chapter = self.parse_chapter_page(&page.body, work, Some(ordinal))?;
        chapter.ordinal = ordinal;
        chapter.source_chapter_key = wanted.source_chapter_key.clone();
        if chapter.title.is_empty() {
            chapter.title = wanted.title.clone();
        }
        Ok(chapter)
    }

    fn preview_from_html(&self, html: &str, url: &Url) -> SourceResult<SourceWork> {
        self.parse_work(html, url)
    }

    fn chapters_from_html(
        &self,
        html: &str,
        work: &SourceWork,
    ) -> SourceResult<Vec<SourceChapter>> {
        // A chapter document holds one chapter, so this returns a list of one.
        // The ordinal is recovered by matching the page's title against the
        // work's chapter list, which is what makes the fixture test able to
        // assert a chapter's position without a network.
        self.parse_chapter_page(html, work, None)
            .map(|chapter| vec![chapter])
    }
}

/// The chapter body's inner HTML, with the site's own furniture excluded.
///
/// `div.chapter-content` and `div.chapter-inner` are the *same* element on this
/// site, both classes on one `div`, so either selector finds it. The author's
/// note is a *sibling* of that element rather than a child, which is why
/// selecting the element rather than its parent is what keeps the note out of
/// the prose — a distinction the ported adapter did not make.
///
/// The first paragraph of the body repeats the chapter's number and title, and
/// is left in place. It is the author's own text, and stripping content is a
/// worse error than echoing a heading the reader already saw.
fn chapter_body(document: &Html) -> Option<String> {
    let selector = Selector::parse("div.chapter-content").ok()?;
    document
        .select(&selector)
        .next()
        .map(|element| element.inner_html())
}

/// Every row of the chapter table, in document order.
fn parse_chapter_rows(document: &Html) -> Vec<ChapterRow> {
    let Ok(rows) = Selector::parse("#chapters tbody tr.chapter-row") else {
        return Vec::new();
    };
    let Ok(title_link) = Selector::parse("a[href*=\"/chapter/\"]") else {
        return Vec::new();
    };
    let Ok(time) = Selector::parse("time") else {
        return Vec::new();
    };

    let mut collected = Vec::new();
    for row in document.select(&rows) {
        let Some(path) = row.value().attr("data-url") else {
            continue;
        };
        let Some(chapter_id) = chapter_id_from_path(path) else {
            continue;
        };
        // The first link in the row is the chapter's title; the second is the
        // release date pointing at the same URL, so taking the first is what
        // keeps a title from being read as a date.
        let title = row
            .select(&title_link)
            .next()
            .map(|link| collapse_whitespace(&link.text().collect::<String>()))
            .unwrap_or_default();
        let released = row
            .select(&time)
            .next()
            .and_then(|element| element.value().attr("unixtime"))
            .and_then(|stamp| stamp.trim().parse::<i64>().ok())
            .and_then(|stamp| OffsetDateTime::from_unix_timestamp(stamp).ok());
        collected.push(ChapterRow {
            chapter_id,
            path: path.to_owned(),
            title,
            released,
        });
    }
    collected
}

/// The chapter id in a `/fiction/{id}/{slug}/chapter/{chapter_id}/{slug}` path.
fn chapter_id_from_path(path: &str) -> Option<String> {
    let mut segments = path.trim_start_matches('/').split('/');
    while let Some(segment) = segments.next() {
        if segment == "chapter" {
            let id = segments.next()?;
            if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) {
                return Some(id.to_owned());
            }
            return None;
        }
    }
    None
}

/// A site-absolute path as an absolute URL.
fn absolute(path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_owned();
    }
    let separator = if path.starts_with('/') { "" } else { "/" };
    format!("https://www.{HOST}{separator}{path}")
}

/// The `application/ld+json` block's object.
///
/// Royal Road publishes a `Book` here — name, description, author, language and
/// dates — which is the site describing the work in a machine-readable form
/// rather than through its own class names. A page with no block, or with a
/// block that is not JSON, yields `None` and the caller falls back to the
/// document's markup.
fn json_ld(document: &Html) -> Option<Value> {
    let selector = Selector::parse("script[type=\"application/ld+json\"]").ok()?;
    for element in document.select(&selector) {
        let raw: String = element.text().collect();
        let Ok(value) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        // Some pages wrap the block in an array, and some carry several blocks
        // (a breadcrumb list, a book). The one describing the work is the one
        // with a name.
        match value {
            Value::Object(_) => {
                if value.get("name").is_some() {
                    return Some(value);
                }
            }
            Value::Array(items) => {
                if let Some(found) = items
                    .into_iter()
                    .find(|item| item.get("name").and_then(Value::as_str).is_some())
                {
                    return Some(found);
                }
            }
            _ => {}
        }
    }
    None
}

/// A string field of a JSON object, ignored when it is not a non-empty string.
fn json_str(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(|raw| raw.trim().to_owned())
        .filter(|raw| !raw.is_empty())
}

/// The chapter count the table states in its `data-chapters` attribute.
fn stated_chapter_count(document: &Html) -> Option<i64> {
    let selector = Selector::parse("#chapters").ok()?;
    document
        .select(&selector)
        .next()
        .and_then(|table| table.value().attr("data-chapters"))
        .and_then(|raw| raw.trim().parse::<i64>().ok())
}

/// The fiction id, read from the canonical link the page declares.
///
/// Only a fallback: the id comes from the URL in the ordinary case, and this
/// exists so a redirect that dropped the id still leaves the work addressable
/// rather than producing a work with an empty key.
fn attr_of_fiction_id(document: &Html) -> Option<String> {
    let selector = Selector::parse("link[rel=\"canonical\"]").ok()?;
    let href = document
        .select(&selector)
        .next()
        .and_then(|link| link.value().attr("href"))?;
    let url = Url::parse(href).ok()?;
    RoyalRoad::fiction_id(&url)
}

/// The word count the Pages tooltip states.
///
/// The tooltip reads "This story would be 2,932 pages long as a published
/// paperback book. This is an estimate based on an average of 275 words per
/// page, calculated from 806,306 words." The word count is the only number in
/// that sentence that is a word count, and the page count is not it.
fn word_count(document: &Html) -> Option<i64> {
    let selector = Selector::parse("i.popovers[title=\"Story Length\"]").ok()?;
    let content = document
        .select(&selector)
        .next()
        .and_then(|element| element.value().attr("data-content"))?;
    let (_, tail) = content.split_once("calculated from")?;
    let digits: String = tail
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit() || *c == ',')
        .filter(|c| c.is_ascii_digit())
        .collect();
    digits.parse::<i64>().ok().filter(|count| *count > 0)
}

/// The work's status, from the label the site puts beside the title.
///
/// The labels there are a mix of fiction *type* ("Original", "Fanfiction") and
/// status ("ONGOING", "COMPLETED", "HIATUS", "DROPPED", "STUB"). Only the ones
/// that are unambiguously a status are read; anything else — including `STUB`,
/// whose meaning on this site is about the work having been withdrawn and which
/// this adapter does not interpret — leaves the status `Unknown`.
fn work_status(document: &Html) -> WorkStatus {
    let labels = texts_of(document, ".fiction-info span.label");
    for label in labels {
        match label.trim().to_ascii_uppercase().as_str() {
            "COMPLETED" | "COMPLETE" => return WorkStatus::Complete,
            "ONGOING" => return WorkStatus::Ongoing,
            "HIATUS" | "ON HIATUS" => return WorkStatus::Hiatus,
            "DROPPED" | "CANCELLED" | "CANCELED" | "ABANDONED" => return WorkStatus::Cancelled,
            _ => {}
        }
    }
    WorkStatus::Unknown
}

/// The work's tags, as the site displays them.
///
/// Carried as display text and not mapped onto anything: Lorehaven's taxonomy
/// and its typed query language arrive in M9, and the `tags` field of
/// [`SourceWork`] exists precisely so an adapter can pass them through without
/// inventing tag types this crate has no business fixing.
fn work_tags(document: &Html) -> Vec<String> {
    let Ok(selector) = Selector::parse(".fiction-info span.tags a") else {
        return Vec::new();
    };
    let mut tags: Vec<String> = document
        .select(&selector)
        .map(|element| collapse_whitespace(&element.text().collect::<String>()))
        .filter(|tag| !tag.is_empty())
        .collect();
    tags.dedup();
    tags
}

/// Whether a page is the site's error page rather than a work.
///
/// Structural first: a work page has a title heading and a chapter table, and
/// the error page has neither. The `<title>` text is a fallback for a page that
/// has no landmarks at all — and a page with no landmarks and no error title is
/// left to fail as a parse error, which is louder and more honest than calling
/// an unrecognised page a missing work.
fn is_not_found(document: &Html) -> bool {
    if text_of(document, "h1.font-white").is_some() {
        return false;
    }
    if stated_chapter_count(document).is_some() {
        return false;
    }
    let title = text_of(document, "title")
        .unwrap_or_default()
        .to_lowercase();
    title.contains("not found") || title.contains("404")
}

/// The chapter number a title begins with, as `17. Title` or `17 - Title`.
///
/// Only the number is taken. The title is kept exactly as the site shows it,
/// number and all, because that is the chapter's title — so the rest of the
/// string is not something this function has any use for.
fn leading_ordinal(title: &str) -> Option<u32> {
    let trimmed = title.trim_start();
    let digits: String = trimmed.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let ordinal = digits.parse::<u32>().ok()?;
    if ordinal == 0 {
        return None;
    }
    // The number has to be a chapter number rather than the start of a title.
    // `17. Title`, `3 - Title` and `109) Title` qualify; `1984 part one` does
    // not, and the separator is what tells them apart — a bare space is not
    // enough, because a space follows the digits in both.
    let rest = trimmed[digits.len()..].trim_start();
    let separated = match rest.chars().next() {
        None => true,
        Some(separator) => matches!(separator, '.' | ')' | '-' | ':' | '\u{2014}' | '\u{2013}'),
    };
    separated.then_some(ordinal)
}

/// Parse a date the way Royal Road writes it.
///
/// The structured block uses an offset-qualified timestamp
/// (`2018-10-28T21:34:43+00:00`), but the chapter table's own `datetime`
/// attribute carries seven fractional digits, so both shapes are accepted. A
/// date in a shape this does not know becomes `None` rather than a guess,
/// because a wrong date on an imported work is worse than an absent one.
fn parse_date(raw: &str) -> Option<OffsetDateTime> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(parsed) = OffsetDateTime::parse(text, &Iso8601::DEFAULT) {
        return Some(parsed);
    }
    let date =
        time::Date::parse(text, &time::format_description::well_known::Iso8601::DATE).ok()?;
    Some(date.midnight().assume_utc())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(raw: &str) -> Url {
        Url::parse(raw).expect("a test URL must parse")
    }

    fn adapter() -> RoyalRoad {
        RoyalRoad::new()
    }

    #[test]
    fn a_work_url_is_claimed() {
        assert!(adapter().can_handle(&url(
            "https://www.royalroad.com/fiction/21220/mother-of-learning"
        )));
    }

    #[test]
    fn a_chapter_url_is_claimed_as_its_work() {
        assert!(adapter().can_handle(&url(
            "https://www.royalroad.com/fiction/21220/mother-of-learning/chapter/301778/1-good-morning-brother"
        )));
    }

    #[test]
    fn the_bare_host_is_the_same_site() {
        assert!(adapter().can_handle(&url("https://royalroad.com/fiction/21220/slug")));
    }

    #[test]
    fn the_search_page_is_not_a_work() {
        // `/fictions/search` shares the prefix but is not a work, and claiming
        // it would make the importer try to import a search results page.
        assert!(!adapter().can_handle(&url("https://www.royalroad.com/fictions/search?title=x")));
        assert!(!adapter().can_handle(&url("https://www.royalroad.com/fiction/abc/slug")));
    }

    #[test]
    fn another_site_is_not_claimed() {
        assert!(!adapter().can_handle(&url("https://www.example.com/fiction/1/slug")));
    }

    #[test]
    fn a_work_url_is_built_from_a_chapter_url() {
        assert_eq!(
            RoyalRoad::work_url(&url(
                "https://www.royalroad.com/fiction/21220/mother-of-learning/chapter/301778/1-good"
            )),
            Some("https://www.royalroad.com/fiction/21220/mother-of-learning".to_owned())
        );
    }

    #[test]
    fn a_work_url_is_built_without_a_slug() {
        assert_eq!(
            RoyalRoad::work_url(&url("https://www.royalroad.com/fiction/21220")),
            Some("https://www.royalroad.com/fiction/21220".to_owned())
        );
    }

    #[test]
    fn a_chapter_id_is_read_out_of_a_row_path() {
        assert_eq!(
            chapter_id_from_path("/fiction/21220/mother-of-learning/chapter/301778/1-good"),
            Some("301778".to_owned())
        );
        assert_eq!(
            chapter_id_from_path("/fiction/21220/mother-of-learning"),
            None
        );
        assert_eq!(
            chapter_id_from_path("/fiction/21220/x/chapter/not-a-number/y"),
            None
        );
    }

    #[test]
    fn a_chapter_number_is_read_out_of_a_title() {
        assert_eq!(leading_ordinal("17. The Bitter Truth"), Some(17));
        assert_eq!(leading_ordinal("3 - Something"), Some(3));
        assert_eq!(leading_ordinal("109) Coming Home"), Some(109));
        assert_eq!(leading_ordinal("Afterword"), None);
        assert_eq!(leading_ordinal("0. Nothing"), None);
        // A title that merely begins with a number is not a numbered chapter.
        assert_eq!(leading_ordinal("1984 part one"), None);
    }

    #[test]
    fn a_stated_date_is_parsed_with_its_offset() {
        let parsed = parse_date("2018-10-28T21:34:43+00:00").expect("the fixture date must parse");
        assert_eq!(parsed.year(), 2018);
        assert_eq!(parsed.month() as u8, 10);
        assert_eq!(parsed.day(), 28);
    }

    #[test]
    fn a_date_in_an_unknown_shape_is_absent_rather_than_guessed() {
        assert!(parse_date("sometime last Tuesday").is_none());
        assert!(parse_date("").is_none());
    }

    #[test]
    fn the_capability_that_a_retry_needs_is_advertised() {
        let capabilities = adapter().capabilities();
        assert!(capabilities.metadata);
        assert!(capabilities.chapters);
        assert!(capabilities.per_chapter_fetch);
    }

    #[test]
    fn only_the_one_host_is_listed() {
        assert_eq!(adapter().hosts(), vec![HOST.to_owned()]);
    }
}
