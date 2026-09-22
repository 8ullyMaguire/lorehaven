//! The Archive-software family: Archive of Our Own and the archives that run
//! the same code.
//!
//! # Recognised URLs
//!
//! ```text
//! https://archiveofourown.org/works/{work_id}
//! https://archiveofourown.org/works/{work_id}/chapters/{chapter_id}
//! https://archiveofourown.org/works/{work_id}?view_full_work=true
//! ```
//!
//! The same parser serves the other Archive-software instances, which share this
//! markup: `adastrafanfic.com` and `squidgeworld.org` are configured here. They
//! are declared as *configured* rather than *verified* — the fixture tests that
//! pin this parser were recorded from archiveofourown.org, and the mirrors are
//! exercised by the live check in `docs/verification.md`, not by a fixture.
//!
//! # Capabilities
//!
//! Metadata, chapters, per-chapter fetch and bibliography are all available. The
//! chapter index on a work page carries each chapter's own id, so a single
//! chapter can be re-read without touching the others, and an author's works are
//! listed on their profile page.
//!
//! # What this adapter does *not* do
//!
//! It does not sign in. The adult-content interstitial is honoured — a work
//! behind it is reported as needing the reader's confirmation rather than
//! fetched — but account login, which some works require, is not implemented
//! here; a locked work reports `AuthRequired` and the import pauses rather than
//! retrying. That is an honest gap, recorded in the milestone notes.

use scraper::{Html, Selector};
use time::format_description::well_known::Iso8601;
use time::OffsetDateTime;
use url::Url;

use async_trait::async_trait;

use crate::sanitize::sanitize_fragment;
use crate::{
    attr_of, collapse_whitespace, html_of, strip_tags, text_of, texts_of, ChapterRef, Credentials,
    Fetcher, SourceAdapter, SourceCapabilities, SourceChapter, SourceError, SourceKey,
    SourceResult, SourceWork, Wall, WorkStatus,
};

/// Hosts this adapter serves.
const HOSTS: [&str; 3] = [
    "archiveofourown.org",
    "adastrafanfic.com",
    "squidgeworld.org",
];

/// The Archive-software adapter.
#[derive(Debug, Clone)]
pub struct ArchiveSoftware {
    key: SourceKey,
    hosts: Vec<String>,
}

impl Default for ArchiveSoftware {
    fn default() -> Self {
        Self::new()
    }
}

impl ArchiveSoftware {
    /// The family, with every host it serves.
    #[must_use]
    pub fn new() -> Self {
        Self {
            key: SourceKey::new("ao3"),
            hosts: HOSTS.iter().map(|host| (*host).to_owned()).collect(),
        }
    }

    /// The hosts, for the fetcher's allow-list.
    #[must_use]
    pub fn hosts(&self) -> Vec<String> {
        self.hosts.clone()
    }

    /// Whether a URL is one of this family's hosts.
    fn host_matches(&self, host: &str) -> bool {
        let host = host.trim_start_matches("www.");
        self.hosts.iter().any(|ours| host == ours)
    }

    /// The numeric work id in a `/works/{id}` path.
    ///
    /// Returns `None` for a URL that is not a work — including the site's own
    /// `/works/search` and `/works/123/navigate`, which share the prefix.
    fn work_id(url: &Url) -> Option<String> {
        let mut segments = url.path_segments()?;
        if segments.next()? != "works" {
            return None;
        }
        let id = segments.next()?;
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        Some(id.to_owned())
    }

    /// The chapter id in a `/works/{work}/chapters/{chapter}` path.
    fn chapter_id(url: &Url) -> Option<String> {
        let mut segments = url.path_segments()?;
        if segments.next()? != "works" || Self::work_id(url).is_none() {
            return None;
        }
        if segments.next()? != "chapters" {
            return None;
        }
        let id = segments.next()?;
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        Some(id.to_owned())
    }

    /// `https://host/works/{id}`
    fn work_url(&self, work_id: &str) -> String {
        format!("https://{}/works/{work_id}", self.hosts[0])
    }

    /// Parse everything a work page offers.
    fn parse_work(&self, html: &str, url: &Url) -> SourceResult<SourceWork> {
        let document = Html::parse_document(html);

        // A 404 and an adult-content wall both arrive as `200` with a notice
        // page. Neither is a parse failure, and neither may become a work with
        // an empty chapter list (spec §11.6: a source that changes must fail
        // loudly, and a work that does not exist must not look like one that
        // does).
        if is_not_found(&document) {
            return Err(SourceError::NotFound);
        }
        let adult_gated = is_adult_gate(&document);

        let work_id = Self::work_id(url).ok_or_else(|| {
            SourceError::Unsupported(format!("{url} is not an Archive-software work URL"))
        })?;

        let title = text_of(&document, "h2.title.heading").ok_or_else(|| {
            SourceError::Parse(format!("no title on the work page for work {work_id}"))
        })?;

        if adult_gated && title.is_empty() {
            return Err(SourceError::AuthRequired(
                "this work is behind the adult-content confirmation".to_owned(),
            ));
        }

        let author_text = texts_of(&document, "h3.byline.heading a[rel=author]").join(", ");
        let author_text = if author_text.is_empty() {
            text_of(&document, "h3.byline.heading").unwrap_or_else(|| "Anonymous".to_owned())
        } else {
            author_text
        };
        let author_url = attr_of(&document, "h3.byline.heading a[rel=author]", "href")
            .and_then(|href| url.join(&href).ok())
            .map(|url| url.to_string());

        let summary = html_of(&document, "div.summary.module blockquote.userstuff")
            .or_else(|| html_of(&document, "#summary blockquote.userstuff"))
            .map(|raw| strip_tags(&raw))
            .unwrap_or_default();

        let (chapter_count, chapter_total) = parse_chapter_counts(&document);

        // The chapter index carries each chapter's own id, so the keys a reader's
        // notes and progress are mapped onto are the source's, not a positional
        // guess. It is absent on a single-chapter work's page, which is why the
        // count above is the fallback.
        let mut chapters = parse_chapter_index(&document);
        if chapters.is_empty() {
            if is_adult_gate(&document) {
                return Err(SourceError::AuthRequired(
                    "this work is behind the adult-content confirmation".to_owned(),
                ));
            }
            let total = chapter_total.or(chapter_count).unwrap_or(0);
            for ordinal in 1..=total {
                chapters.push(ChapterRef {
                    ordinal,
                    source_chapter_key: ordinal.to_string(),
                    title: format!("Chapter {ordinal}"),
                });
            }
        }

        let tags_of = |selector: &str| texts_of(&document, selector);
        let rating_text = tags_of("dd.rating.tags a.tag").into_iter().next();
        let warning_texts = tags_of("dd.warning.tags a.tag");
        let mut tags = Vec::new();
        for selector in [
            "dd.category.tags a.tag",
            "dd.fandom.tags a.tag",
            "dd.relationship.tags a.tag",
            "dd.character.tags a.tag",
            "dd.freeform.tags a.tag",
        ] {
            tags.extend(tags_of(selector));
        }

        let status = parse_status(&document);
        let (published_at, updated_at) = parse_dates(&document);
        let word_count = text_of(&document, "dl.stats > dd.words")
            .or_else(|| text_of(&document, "dd.words"))
            .and_then(|raw| parse_number(&raw));
        let language = text_of(&document, "dd.language");

        Ok(SourceWork {
            source_key: self.key.clone(),
            source_work_key: work_id.clone(),
            source_url: self.work_url(&work_id),
            title,
            author_text,
            author_url,
            summary,
            word_count,
            language,
            status,
            published_at,
            updated_at,
            chapters,
            rating_text,
            warning_texts,
            tags,
        })
    }

    /// Parse the chapter bodies out of a whole-work page.
    fn parse_chapters(&self, html: &str, work: &SourceWork) -> SourceResult<Vec<SourceChapter>> {
        let document = Html::parse_document(html);
        if is_not_found(&document) {
            return Err(SourceError::NotFound);
        }
        if is_adult_gate(&document) && html_of(&document, "div.chapter").is_none() {
            return Err(SourceError::AuthRequired(
                "this work is behind the adult-content confirmation".to_owned(),
            ));
        }

        // `div.chapter` alone is not enough: each chapter contains a
        // `<div class="chapter preface group">` for its heading, so a plain
        // class match returns two elements per chapter and half of them have no
        // body. The real containers are the ones with a generated `chapter-N`
        // id, which is what this selects.
        let chapter_selector =
            Selector::parse("div.chapter[id^='chapter-']").expect("static selector");
        let title_selector = Selector::parse("h3.title").expect("static selector");
        let body_selector =
            Selector::parse("div.userstuff.module[role=article]").expect("static selector");

        let mut chapters = Vec::new();
        for element in document.select(&chapter_selector) {
            // The ordinal comes from the id the site generated, not from the
            // position in the document, so an unexpected extra element cannot
            // shift every chapter's number.
            let ordinal = element
                .value()
                .attr("id")
                .and_then(|id| id.strip_prefix("chapter-"))
                .and_then(|n| n.parse::<u32>().ok())
                .unwrap_or((chapters.len() + 1) as u32);

            // The chapter's own link gives its id, which is the stable key. A
            // chapter with no link keeps the ordinal, which is stable across a
            // re-import of an unchanged work.
            let source_chapter_key = element
                .select(&title_selector)
                .next()
                .and_then(|title| {
                    title
                        .select(&Selector::parse("a[href]").expect("static selector"))
                        .next()
                        .and_then(|anchor| anchor.value().attr("href"))
                        .and_then(|href| {
                            Url::parse(href).ok().or_else(|| {
                                self.work_url(&work.source_work_key)
                                    .parse::<Url>()
                                    .ok()?
                                    .join(href)
                                    .ok()
                            })
                        })
                })
                .and_then(|link| Self::chapter_id(&link))
                .unwrap_or_else(|| {
                    work.chapters
                        .iter()
                        .find(|candidate| candidate.ordinal == ordinal)
                        .map_or_else(|| ordinal.to_string(), |c| c.source_chapter_key.clone())
                });

            let raw_title = element
                .select(&title_selector)
                .next()
                .map(|title| collapse_whitespace(&title.text().collect::<String>()))
                .unwrap_or_default();
            let title = clean_chapter_title(&raw_title, ordinal);

            let Some(body) = element.select(&body_selector).next() else {
                // A chapter element with no article body is the shape a markup
                // change takes. Recording it as an empty chapter would store a
                // blank reading experience that looks like a successful import.
                return Err(SourceError::Parse(format!(
                    "chapter {ordinal} of work {} has no body element",
                    work.source_work_key
                )));
            };

            // The site's own chrome lives inside the article element — a
            // landmark heading that says "Chapter Text" and nothing else. It is
            // the adapter's job to remove it, because no generic sanitiser can
            // tell a site's furniture from its prose.
            let raw_body = html_without_landmarks(body);

            let work_url = self
                .work_url(&work.source_work_key)
                .parse::<Url>()
                .expect("our own url");
            let content_html = sanitize_fragment(&raw_body, Some(&work_url));
            let image_urls = crate::sanitize::extract_image_urls(&raw_body, Some(&work_url));
            if crate::sanitize::is_blank(&content_html) {
                return Err(SourceError::Parse(format!(
                    "chapter {ordinal} of work {} sanitised to nothing",
                    work.source_work_key
                )));
            }

            chapters.push(SourceChapter {
                ordinal,
                source_chapter_key,
                title,
                content_html,
                image_urls,
            });
        }

        if chapters.is_empty() {
            return Err(SourceError::Parse(format!(
                "no chapters found on the page for work {}; the source's markup has changed",
                work.source_work_key
            )));
        }
        Ok(chapters)
    }
}

#[async_trait]
impl SourceAdapter for ArchiveSoftware {
    fn key(&self) -> SourceKey {
        self.key.clone()
    }

    fn display_name(&self) -> &'static str {
        "Archive of Our Own"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities {
            metadata: true,
            chapters: true,
            per_chapter_fetch: true,
            bibliography: true,
            // The stats block carries a published date but not a revision
            // marker, so "has this changed?" still needs the work page. Reading
            // it is one request, which is what an update check costs.
            incremental: true,
            authentication: crate::AuthKind::None,
            min_interval_millis: Some(1_000),
        }
    }
    fn wall(&self) -> Wall {
        // AO3 is behind a Cloudflare challenge that rejects non-browser TLS fingerprints.
        Wall::Fingerprint
    }

    fn can_handle(&self, url: &Url) -> bool {
        let Some(host) = url.host_str() else {
            return false;
        };
        self.host_matches(host) && ArchiveSoftware::work_id(url).is_some()
    }

    fn hosts(&self) -> Vec<String> {
        ArchiveSoftware::hosts(self)
    }

    async fn preview(
        &self,
        fetch: &dyn Fetcher,
        url: &Url,
        creds: Option<&Credentials>,
    ) -> SourceResult<SourceWork> {
        let work_id = ArchiveSoftware::work_id(url).ok_or_else(|| {
            SourceError::Unsupported(format!("{url} is not an Archive-software work URL"))
        })?;
        let mut target = self.work_url(&work_id);
        if creds.is_some_and(|creds| creds.adult_allowed) {
            // The interstitial's own "Proceed" link carries this parameter.
            target.push_str("?view_adult=true");
        }
        let page = fetch.get(&target).await?;
        let parsed = Url::parse(&page.final_url).unwrap_or_else(|_| url.clone());
        self.parse_work(&page.body, &parsed)
    }

    async fn fetch_chapters(
        &self,
        fetch: &dyn Fetcher,
        work: &SourceWork,
        creds: Option<&Credentials>,
    ) -> SourceResult<Vec<SourceChapter>> {
        let mut target = format!(
            "{}?view_full_work=true",
            self.work_url(&work.source_work_key)
        );
        if creds.is_some_and(|creds| creds.adult_allowed) {
            target.push_str("&view_adult=true");
        }
        let page = fetch.get(&target).await?;
        self.parse_chapters(&page.body, work)
    }

    async fn fetch_chapter(
        &self,
        fetch: &dyn Fetcher,
        work: &SourceWork,
        ordinal: u32,
        creds: Option<&Credentials>,
    ) -> SourceResult<SourceChapter> {
        // The chapter id comes from the work's own index, so a retry re-reads
        // exactly one chapter and the source needs no extra request to work it
        // out.
        let chapter = work
            .chapters
            .iter()
            .find(|candidate| candidate.ordinal == ordinal)
            .ok_or_else(|| {
                SourceError::Unsupported(format!(
                    "work {} has no chapter {ordinal}",
                    work.source_work_key
                ))
            })?;
        if !chapter
            .source_chapter_key
            .chars()
            .all(|c| c.is_ascii_digit())
        {
            return Err(SourceError::Unsupported(
                "this work's chapter ids are not known, so a single chapter cannot be re-read"
                    .to_owned(),
            ));
        }
        let mut target = format!(
            "https://{}/works/{}/chapters/{}",
            self.hosts[0], work.source_work_key, chapter.source_chapter_key
        );
        if creds.is_some_and(|creds| creds.adult_allowed) {
            target.push_str("?view_adult=true");
        }
        let page = fetch.get(&target).await?;
        let mut chapters = self.parse_chapters(&page.body, work)?;
        // A chapter page holds one `div.chapter`; its position on the page is
        // not its position in the work, so the ordinal is restored from the
        // request rather than trusted from the parse.
        chapters
            .pop()
            .map(|mut found| {
                found.ordinal = ordinal;
                found.source_chapter_key = chapter.source_chapter_key.clone();
                if found.title.is_empty() {
                    found.title = chapter.title.clone();
                }
                found
            })
            .ok_or(SourceError::NotFound)
    }

    async fn list_author_works(
        &self,
        fetch: &dyn Fetcher,
        profile_url: &Url,
        creds: Option<&Credentials>,
    ) -> SourceResult<Vec<String>> {
        let mut all = Vec::new();
        let mut page_url = profile_url.to_string();
        // Bounded: an author with thousands of works must not turn one request
        // into an unbounded walk (spec §11.9, "pagination and enumeration
        // limits"). The caller sees a truncated list rather than a hung job.
        for _ in 0..20 {
            let page = fetch.get(&page_url).await?;
            let document = Html::parse_document(&page.body);
            let selector =
                Selector::parse("h4.heading a[href^='/works/']").expect("static selector");
            let mut found_any = false;
            for element in document.select(&selector) {
                if let Some(href) = element.value().attr("href") {
                    if let Ok(url) = profile_url.join(href) {
                        if ArchiveSoftware::work_id(&url).is_some() {
                            all.push(url.to_string());
                            found_any = true;
                        }
                    }
                }
            }
            let next = attr_of(&document, "ol.pagination li.next a[href]", "href")
                .or_else(|| attr_of(&document, "a.next_page", "href"));
            match next.and_then(|href| profile_url.join(&href).ok()) {
                Some(next) if found_any => page_url = next.to_string(),
                _ => break,
            }
        }
        let _ = creds;
        all.sort();
        all.dedup();
        Ok(all)
    }

    fn preview_from_html(&self, html: &str, url: &Url) -> SourceResult<SourceWork> {
        self.parse_work(html, url)
    }

    fn chapters_from_html(
        &self,
        html: &str,
        work: &SourceWork,
    ) -> SourceResult<Vec<SourceChapter>> {
        self.parse_chapters(html, work)
    }
}

/// A chapter body's HTML with the site's landmark headings removed.
///
/// The landmark heading is navigation furniture that AO3 renders inside the
/// article element that holds the prose. Removing it by its exact outer HTML
/// rather than by a regular expression means the removal cannot over-match a
/// reader's own `<h3>` in their text.
fn html_without_landmarks(body: scraper::ElementRef<'_>) -> String {
    let mut html = body.inner_html();
    let Ok(selector) = Selector::parse("h3.landmark, div.landmark") else {
        return html;
    };
    for element in body.select(&selector) {
        let outer = element.html();
        if !outer.is_empty() {
            html = html.replace(&outer, "");
        }
    }
    html
}

/// Whether a page is the site's error page rather than a work.
///
/// The signal is structural — the error page's main region carries its own class
/// — with the heading text as a fallback. The `404` page has no `h2.title.heading`
/// at all, so a title-based check would report it as a parse failure rather than
/// as a missing work, and "this work does not exist" would read to a reader as
/// "something is wrong with us".
fn is_not_found(document: &Html) -> bool {
    let class = attr_of(document, "#main", "class")
        .unwrap_or_default()
        .to_lowercase();
    if class.contains("error") || class.contains("not-found") {
        return true;
    }
    text_of(document, "#main h2.heading")
        .map(|text| {
            let text = text.to_lowercase();
            text.contains("error") || text.contains("not found")
        })
        .unwrap_or(false)
}

/// Whether a page is the adult-content confirmation.
fn is_adult_gate(document: &Html) -> bool {
    text_of(document, "#main p.caution")
        .or_else(|| text_of(document, "#main"))
        .map(|text| {
            let lower = text.to_lowercase();
            lower.contains("could have adult content") || lower.contains("this work could have")
        })
        .unwrap_or(false)
}

/// `dd.chapters` reads `3/3` or `10/30`, or just `12` for a single-chapter work.
///
/// Returns the number posted and, when the source states one, the number
/// planned. The distinction matters: a work that says `10/30` is ongoing even if
/// its author has not touched the status field, and a work that says `3/3` may
/// still be marked updated.
fn parse_chapter_counts(document: &Html) -> (Option<u32>, Option<u32>) {
    let Some(raw) = text_of(document, "dd.chapters") else {
        return (None, None);
    };
    let mut parts = raw.split('/');
    let posted = parts.next().and_then(parse_count);
    let planned = parts.next().and_then(parse_count);
    (posted, planned)
}

fn parse_count(raw: &str) -> Option<u32> {
    let trimmed = raw.trim();
    if trimmed == "?" {
        return None;
    }
    trimmed.replace(',', "").parse().ok()
}

/// The chapter index's options, in order.
fn parse_chapter_index(document: &Html) -> Vec<ChapterRef> {
    let Ok(selector) = Selector::parse("select#selected_id option") else {
        return Vec::new();
    };
    document
        .select(&selector)
        .enumerate()
        .filter_map(|(index, option)| {
            let value = option.value().attr("value")?;
            if value.is_empty() {
                return None;
            }
            let ordinal = (index + 1) as u32;
            let label = collapse_whitespace(&option.text().collect::<String>());
            let title = clean_chapter_title(&label, ordinal);
            Some(ChapterRef {
                ordinal,
                source_chapter_key: value.to_owned(),
                title,
            })
        })
        .collect()
}

/// Turn a chapter label into a title.
///
/// The two shapes the site uses are `1. The Title` in the index and
/// `Chapter 1: The Title` in a chapter heading. Both are reduced to `The Title`,
/// and a chapter the author left unnamed keeps its number.
fn clean_chapter_title(raw: &str, ordinal: u32) -> String {
    let text = collapse_whitespace(raw);
    let without_number = text
        .strip_prefix(&format!("{ordinal}. "))
        .or_else(|| text.strip_prefix(&format!("{ordinal}.")))
        .unwrap_or(&text);
    let without_chapter = without_number
        .strip_prefix(&format!("Chapter {ordinal}"))
        .or_else(|| without_number.strip_prefix(&format!("chapter {ordinal}")))
        .unwrap_or(without_number);
    let cleaned = without_chapter
        .trim_start_matches(':')
        .trim()
        .trim_start_matches("—")
        .trim();
    if cleaned.is_empty() {
        format!("Chapter {ordinal}")
    } else {
        cleaned.to_owned()
    }
}

/// Whether the work is finished, from the stats block's own label.
///
/// The label is the only signal: `<dt class="status">Completed:</dt>` versus
/// `Updated:`. Anything else is `Unknown` — AO3 has no "abandoned" marker, and
/// inferring one from an old revision date would invent a fact.
fn parse_status(document: &Html) -> WorkStatus {
    let Some(label) = text_of(document, "dl.stats > dt.status") else {
        return WorkStatus::Unknown;
    };
    let label = label.to_lowercase();
    if label.starts_with("completed") {
        WorkStatus::Complete
    } else if label.starts_with("updated") {
        WorkStatus::Ongoing
    } else {
        WorkStatus::Unknown
    }
}

/// The published and last-changed dates, when the page states them.
///
/// The stats block carries one dated row besides `Published:` and labels it by
/// the work's own state: `Updated:` while it is being written, `Completed:` once
/// it is finished. Both are the source's statement of when the work last
/// changed, so both answer `updated_at`.
///
/// Reading only `Updated:` would leave every completed work with no last-change
/// date at all — which is worse than a null, because it looks like the source
/// never said, and an update check that trusts it would decide there is nothing
/// to compare against.
fn parse_dates(document: &Html) -> (Option<OffsetDateTime>, Option<OffsetDateTime>) {
    let published = text_of(document, "dl.stats > dd.published").and_then(|raw| parse_date(&raw));
    let status_text = text_of(document, "dl.stats > dt.status").unwrap_or_default();
    let status_date = text_of(document, "dl.stats > dd.status").and_then(|raw| parse_date(&raw));
    let status_label = status_text.to_lowercase();
    let updated = if status_label.starts_with("updated") || status_label.starts_with("completed") {
        status_date
    } else {
        None
    };
    (published, updated)
}

/// Parse the site's date format, which the fixtures show as `2026-09-10`.
///
/// A date the site states with a time is accepted too; a date in a shape this
/// does not know becomes `None` rather than a guess, because a wrong date on an
/// imported work is worse than an absent one.
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

/// Parse `2,116` or `2116`.
fn parse_number(raw: &str) -> Option<i64> {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '-')
        .collect();
    cleaned.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter() -> ArchiveSoftware {
        ArchiveSoftware::new()
    }

    fn url(raw: &str) -> Url {
        Url::parse(raw).unwrap()
    }

    #[test]
    fn it_recognises_work_urls_and_refuses_everything_else() {
        let adapter = adapter();
        assert!(adapter.can_handle(&url("https://archiveofourown.org/works/123")));
        assert!(adapter.can_handle(&url("https://archiveofourown.org/works/123/chapters/456")));
        assert!(adapter.can_handle(&url("https://www.archiveofourown.org/works/123")));
        assert!(adapter.can_handle(&url("https://squidgeworld.org/works/123")));
        assert!(adapter.can_handle(&url(
            "https://archiveofourown.org/works/123?view_full_work=true"
        )));

        // Not a work.
        assert!(!adapter.can_handle(&url("https://archiveofourown.org/works/search")));
        assert!(!adapter.can_handle(&url("https://archiveofourown.org/users/someone")));
        assert!(!adapter.can_handle(&url("https://archiveofourown.org/tags/X/works")));
        // Not our host.
        assert!(!adapter.can_handle(&url("https://fanfiction.net/s/123")));
        // A lookalike host must not match.
        assert!(!adapter.can_handle(&url("https://archiveofourown.org.evil.example/works/1")));
        assert!(!adapter.can_handle(&url("https://notarchiveofourown.org/works/1")));
    }

    #[test]
    fn chapter_titles_are_reduced_from_both_shapes() {
        assert_eq!(
            clean_chapter_title("1. 2 Minutes to Midnight", 1),
            "2 Minutes to Midnight"
        );
        assert_eq!(clean_chapter_title("Chapter 2: Stratego", 2), "Stratego");
        assert_eq!(clean_chapter_title("Chapter 3", 3), "Chapter 3");
        assert_eq!(clean_chapter_title("", 4), "Chapter 4");
        // A number that is not this chapter's is part of the title.
        assert_eq!(clean_chapter_title("2. 1984", 2), "1984");
    }

    #[test]
    fn counts_parse_from_dd_chapters() {
        let document = Html::parse_document(r#"<dd class="chapters">3/3</dd>"#);
        assert_eq!(parse_chapter_counts(&document), (Some(3), Some(3)));
        let document = Html::parse_document(r#"<dd class="chapters">10/30</dd>"#);
        assert_eq!(parse_chapter_counts(&document), (Some(10), Some(30)));
        // A planned count of "?" is not a number and must not become zero.
        let document = Html::parse_document(r#"<dd class="chapters">7/?</dd>"#);
        assert_eq!(parse_chapter_counts(&document), (Some(7), None));
        let document = Html::parse_document(r#"<dd class="chapters">1,024</dd>"#);
        assert_eq!(parse_chapter_counts(&document), (Some(1024), None));
        let document = Html::parse_document("<p>nothing</p>");
        assert_eq!(parse_chapter_counts(&document), (None, None));
    }

    #[test]
    fn status_comes_from_the_labels_own_word() {
        let completed = Html::parse_document(
            r#"<dl class="stats"><dt class="status">Completed:</dt><dd class="status">2026-09-10</dd></dl>"#,
        );
        assert_eq!(parse_status(&completed), WorkStatus::Complete);
        let updated = Html::parse_document(
            r#"<dl class="stats"><dt class="status">Updated:</dt><dd class="status">2026-09-10</dd></dl>"#,
        );
        assert_eq!(parse_status(&updated), WorkStatus::Ongoing);
        // No status field at all is unknown, not ongoing.
        let none = Html::parse_document(r#"<dl class="stats"><dt class="words">Words:</dt></dl>"#);
        assert_eq!(parse_status(&none), WorkStatus::Unknown);
    }

    #[test]
    fn a_date_is_parsed_when_stated_and_absent_when_not() {
        let parsed = parse_date("2026-09-10").expect("iso date");
        assert_eq!(parsed.year(), 2026);
        assert_eq!(parsed.month() as u8, 9);
        assert_eq!(parsed.day(), 10);
        assert!(parse_date("").is_none());
        assert!(parse_date("last Tuesday").is_none());
    }

    #[test]
    fn a_completed_works_last_change_is_its_completion_date() {
        // The two fetches differ only in the label AO3 uses for the work's own
        // state, which is how the site says "still being written" versus
        // "finished". Both are a last-change date and both must reach
        // `updated_at`; reading only `Updated:` loses the date for every
        // completed work.
        let completed = Html::parse_document(
            r#"<dl class="stats">
                 <dt class="published">Published:</dt><dd class="published">2026-09-01</dd>
                 <dt class="status">Completed:</dt><dd class="status">2026-09-10</dd>
               </dl>"#,
        );
        let ongoing = Html::parse_document(
            r#"<dl class="stats">
                 <dt class="published">Published:</dt><dd class="published">2026-09-01</dd>
                 <dt class="status">Updated:</dt><dd class="status">2026-09-10</dd>
               </dl>"#,
        );

        for document in [&completed, &ongoing] {
            let (published, updated) = parse_dates(document);
            assert!(published.is_some());
            assert_eq!(
                updated.map(|at| at.date().to_string()),
                Some("2026-09-10".to_owned()),
                "the label is how the site spells the same fact"
            );
        }

        // A work with no dated row at all reports neither rather than guessing.
        let bare = Html::parse_document(r#"<dl class="stats"></dl>"#);
        let (published, updated) = parse_dates(&bare);
        assert!(published.is_none() && updated.is_none());
    }

    #[test]
    fn numbers_ignore_thousands_separators() {
        assert_eq!(parse_number("2,116"), Some(2116));
        assert_eq!(parse_number("2116"), Some(2116));
        assert_eq!(parse_number("words: 12"), Some(12));
    }
}
