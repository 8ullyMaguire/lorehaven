//! Ficbook: one work page, one page per part, and a `robots.txt` with one rule
//! that shapes everything.
//!
//! # The rule
//!
//! `Disallow: /*?*` — **every path carrying a query string**. The site writes
//! this itself and then breaks it: its own links to works are
//! `/readfic/{uuid}?source=premium&premiumVisit=1` and
//! `/readfic/{uuid}?from_promo=1` on the listing pages. Those queries are
//! decorative — the same work answers identically without them — so this adapter
//! **strips the query and the fragment** rather than accepting an address its
//! own front door would be refused. That is a normalisation, not a workaround:
//! the canonical form is the one the site's `<link rel="canonical">` names, and
//! that form has no query either.
//!
//! The fragment matters for the same reason and is easier to miss: the site's own
//! chapter links are `/readfic/{work}/{part}#part_content`, and a fragment is not
//! sent to the server at all, so storing it would store something the site never
//! saw.
//!
//! Two other rules are deliberately not worked around: `Disallow: *printfic*`
//! and `Disallow: *download*`. Those are the site's own views of the same
//! content, and reading them would be reading a path the site has asked
//! crawlers to leave alone rather than reading the page a reader sees. This
//! adapter reads the reading page.
//!
//! # One work page, one page per part
//!
//! * **The work page** (`/readfic/{work}`) carries the title, the author, the
//!   description, the labels, the tags, the size, and the complete part list —
//!   and **no prose at all**. There is no `#content` on it, which is the
//!   structural difference this adapter's chapter parse relies on: a work page
//!   fed to the chapter parser is refused rather than returning an empty
//!   chapter for every part.
//! * **A part page** (`/readfic/{work}/{part}`) carries that part's prose in
//!   `div#content`, and states its own address in `<link rel="canonical">`.
//!   That is how it identifies itself — this source is unlike Wattpad, whose
//!   prose endpoint is addressed by the chapter and therefore cannot say which
//!   chapter came back.
//!
//! # Labels are class names, not words
//!
//! The status and the rating are visible as Russian text — *В процессе*,
//! *Завершён*, *NC-17* — and reading that text would tie the parser to a
//! language. Both are also carried as machine-readable class names, which is
//! what this adapter reads:
//!
//! ```text
//! <div class="ds-label ds-label-rating-NC-17"> … <div class="ds-label ds-label-status-finished">
//! ```
//!
//! Only two statuses are recorded (`in-progress`, `finished`), so those are the
//! two that are mapped and anything else is `unknown` — a status this build has
//! never seen is not evidence of completion either way.
//!
//! # The size line is words, and its word forms vary
//!
//! Two shapes are recorded for the same field:
//!
//! ```text
//! планируется Макси, написано 158 страниц, 77 507 слов, 24 части
//! 24 страницы, 8 122 слова, 4 части
//! ```
//!
//! So the fields are found by their unit rather than by their position, the
//! optional plan prefix is not parsed, and **the thousands separator is a
//! non-breaking space** — `77\xa0507` — which is why the digits are collected
//! rather than the string parsed. The part count in that line is also the check
//! on the part list: a page that states 24 parts and lists a different number is
//! not a page this adapter understood.
//!
//! # Footnotes live outside the prose
//!
//! The prose is in `div#content`; the footnotes are **not**. Each reference is an
//! empty placeholder — `<span class="footnote" id="fn_35183469_0"></span>` — and
//! the text sits in a `textFootnotes` object in a script at the bottom of the
//! page. An adapter that read `#content` alone would drop every note the author
//! wrote, silently and with a chapter that looks complete. They are read out of
//! the script, marked with a superscript at the reference, and gathered at the
//! end of the chapter.
//!
//! # What the site does not say
//!
//! **The work's language.** There is an `itemprop="inLanguage"` and it is
//! `ru-Latn` on every work recorded — the site's own interface locale, written
//! into a machine-readable field. Reported as the work's language it would claim
//! every ficbook work is Russian-in-Latin-script, including the many in Cyrillic.
//! So no language is reported, which is the honest answer: this page does not
//! carry one.
//!
//! # Dates
//!
//! The work page has no date of its own. Each part does — `3 августа 2023 г.,
//! 12:37` — so the first part's date is the work's publication and the last
//! part's is its last change, which is what those dates mean on a serialised
//! work. They are read as **MSK (+03:00)**, the site's own zone, on the same
//! reasoning Syosetu's dates are read as JST: the page writes a local time with
//! no offset, and reading it as UTC would put every date three hours out.
//!
//! # Provenance
//!
//! Written against pages recorded from the live site on 2026-09-11; see
//! `tests/fixtures/ficbook/` and the `## ficbook` section of
//! `tests/fixtures/README.md`.

use async_trait::async_trait;
use scraper::{Html, Selector};
use time::{macros::offset, Date, Month, OffsetDateTime, Time};
use url::Url;

use crate::sanitize::sanitize_fragment;
use crate::{
    attr_of, collapse_whitespace, html_of, text_of, texts_of, AuthKind, ChapterRef, Credentials,
    Fetcher, SourceAdapter, SourceCapabilities, SourceChapter, SourceError, SourceKey,
    SourceResult, SourceWork, WorkStatus,
};

/// The host, without `www.`.
pub const HOST: &str = "ficbook.net";

/// The registry key.
pub const SOURCE_KEY: &str = "ficbook";

/// The site's pace.
///
/// Ficbook publishes no `Crawl-delay`, so the fetcher's one-second floor is the
/// pace. Left at the floor: a site that published nothing has asked for nothing.
pub const PACING_MILLIS: u64 = 1_000;

/// The reading path every address this adapter claims begins with.
const READ_PATH: &str = "readfic";

/// The zone the site's dates are written in.
///
/// The page writes `3 августа 2023 г., 12:37` — a local time with no offset —
/// and the site is a Russian one. Read as UTC every date would be three hours
/// out; read as MSK they are what a reader in the site's own zone sees.
const SITE_OFFSET: time::UtcOffset = offset!(+3);

/// Ficbook.
#[derive(Debug, Clone)]
pub struct Ficbook {
    key: SourceKey,
    hosts: Vec<String>,
}

impl Default for Ficbook {
    fn default() -> Self {
        Self::new()
    }
}

impl Ficbook {
    /// Build the adapter.
    #[must_use]
    pub fn new() -> Self {
        Self {
            key: SourceKey::new(SOURCE_KEY),
            hosts: vec![HOST.to_owned()],
        }
    }

    fn host_matches(&self, host: &str) -> bool {
        let host = host.trim_start_matches("www.");
        self.hosts.iter().any(|ours| host == ours)
    }

    /// The work id and optional part id in a `/readfic/{work}[/{part}]` path.
    ///
    /// `None` for anything that is not a reading address, including the
    /// `/readfic/{work}/download` path the site links from a work page.
    fn read_path(url: &Url) -> Option<(String, Option<String>)> {
        let mut segments = url.path_segments()?;
        if segments.next()? != READ_PATH {
            return None;
        }
        let work = segments.next()?;
        if work.is_empty() || work.contains('.') {
            return None;
        }
        let part = match segments.next() {
            Some(part) if !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()) => {
                Some(part.to_owned())
            }
            // A trailing segment that is not a part id is another view of the
            // same work (`download`, `printfic`), which this adapter does not
            // read and does not claim.
            Some(_) => return None,
            None => None,
        };
        Some((work.to_owned(), part))
    }

    /// The canonical work address — the form this adapter asks for.
    ///
    /// Public so a test can check that what it would ask for is an address the
    /// source's own `robots.txt` allows: nothing here carries a query or a
    /// fragment, because this source disallows every query string.
    #[must_use]
    pub fn work_url(&self, work: &str) -> String {
        format!("https://{HOST}/{READ_PATH}/{work}")
    }

    /// The canonical address of one part.
    #[must_use]
    pub fn part_url(&self, work: &str, part: &str) -> String {
        format!("https://{HOST}/{READ_PATH}/{work}/{part}")
    }

    /// Read a work's metadata and part list out of its page.
    ///
    /// # Errors
    ///
    /// [`SourceError::Parse`] when the page is not a work page — distinguished
    /// from a missing work, which the site answers with a real `404`.
    pub fn parse_work(&self, html: &str, work: &str) -> SourceResult<SourceWork> {
        let document = Html::parse_document(html);

        let title = text_of(&document, "h1[itemprop='name']").ok_or_else(|| {
            SourceError::Parse(format!("ficbook work {work} has no title element"))
        })?;

        let author_text = text_of(&document, "a[itemprop='author']").unwrap_or_default();
        let author_url =
            attr_of(&document, "a[itemprop='author']", "href").map(|href| absolute(&href));

        let summary = text_of(&document, "[itemprop='description']").unwrap_or_default();

        let chapters = part_refs(&document);
        let size = size_line(&document);

        // The page states its own part count beside a list it renders in full.
        // A page where the two disagree is a page this adapter did not
        // understand, and importing the list would import a fraction of a work
        // and report success.
        if let Some(stated) = size.parts {
            if chapters.is_empty() {
                return Err(SourceError::Parse(format!(
                    "ficbook work {work} states {stated} parts and lists none"
                )));
            }
            if stated as usize != chapters.len() {
                return Err(SourceError::Parse(format!(
                    "ficbook work {work} states {stated} parts and lists {}",
                    chapters.len()
                )));
            }
        }

        // A publication date is the first part's and a last change is the
        // last part's. The part list is in publication order, so this is the
        // same reading whether or not the dates are monotonic.
        let part_dates: Vec<OffsetDateTime> = part_dates(&document).into_iter().flatten().collect();

        Ok(SourceWork {
            source_key: self.key.clone(),
            source_work_key: work.to_owned(),
            source_url: self.work_url(work),
            title,
            author_text,
            author_url,
            summary,
            word_count: size.words.map(i64::from),
            // See the module documentation: the only language field on the page
            // is the site's own interface locale, written into every work.
            language: None,
            status: status_of(&document),
            published_at: part_dates.first().copied(),
            updated_at: part_dates.last().copied(),
            chapters,
            rating_text: rating_of(&document),
            warning_texts: Vec::new(),
            tags: texts_of(&document, "div.tags a.tag"),
        })
    }

    /// Read one part's prose, and the notes that are not in it.
    ///
    /// # Errors
    ///
    /// [`SourceError::Parse`] when the page carries no `#content`. A *work*
    /// page reaches this and is refused, which is deliberate: returning an empty
    /// chapter for a page with no prose would store a work whose every chapter
    /// is blank and report success.
    pub fn parse_chapter(
        &self,
        html: &str,
        work: &SourceWork,
        part: &str,
    ) -> SourceResult<SourceChapter> {
        let document = Html::parse_document(html);

        let raw = html_of(&document, "div#content").ok_or_else(|| {
            SourceError::Parse(format!(
                "ficbook part {part} of {} has no #content",
                work.source_work_key
            ))
        })?;

        // The page states its own address, which is how it identifies itself.
        // Checked against the part that was asked for, because a page that
        // answers with another chapter would have its prose stored under the
        // wrong ordinal — and a reader's progress and notes are mapped onto the
        // ordinal across a re-import.
        if let Some(canonical) = canonical_part(&document) {
            if canonical != part {
                return Err(SourceError::Parse(format!(
                    "ficbook served part {canonical} when part {part} of {} was asked for",
                    work.source_work_key
                )));
            }
        }

        let body = inline_footnotes(&raw, &text_footnotes(&document));

        let entry = work
            .chapters
            .iter()
            .find(|entry| entry.source_chapter_key == part);
        let ordinal = entry.map_or(0, |entry| entry.ordinal);

        Ok(SourceChapter {
            ordinal,
            source_chapter_key: part.to_owned(),
            title: entry.map(|entry| entry.title.clone()).unwrap_or_default(),
            content_html: sanitize_fragment(&body, Url::parse(&work.source_url).ok().as_ref()),
            image_urls: crate::sanitize::extract_image_urls(&body, Url::parse(&work.source_url).ok().as_ref()),
        })
    }

    /// The ordinal of a part page, from the work's own list.
    fn ordinal_of(work: &SourceWork, part: &str) -> Option<u32> {
        work.chapters
            .iter()
            .find(|entry| entry.source_chapter_key == part)
            .map(|entry| entry.ordinal)
    }
}

#[async_trait]
impl SourceAdapter for Ficbook {
    fn key(&self) -> SourceKey {
        self.key.clone()
    }

    fn display_name(&self) -> &'static str {
        "Ficbook"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities {
            metadata: true,
            chapters: true,
            // A part has its own address and it is derivable from the work's
            // list, so a retry re-reads one part and touches nothing else.
            per_chapter_fetch: true,
            // Author pages exist (`/authors/{id}`); their markup has not been
            // recorded.
            bibliography: false,
            // The work page carries every part's date, so "has this changed?"
            // costs one request and compares the last of them.
            incremental: true,
            authentication: AuthKind::None,
            min_interval_millis: Some(PACING_MILLIS),
        }
    }

    fn can_handle(&self, url: &Url) -> bool {
        let Some(host) = url.host_str() else {
            return false;
        };
        self.host_matches(host) && Ficbook::read_path(url).is_some()
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
        // A part address is normalised to its work's, so pasting a chapter
        // previews the work. The query and the fragment are dropped first: the
        // site disallows every query string, and it writes the ones on its own
        // links itself.
        let (work, _) = Ficbook::read_path(url).ok_or_else(|| {
            SourceError::Unsupported(format!("{url} is not a Ficbook story address"))
        })?;
        let page = fetch.get(&self.work_url(&work)).await?;
        self.parse_work(&page.body, &work)
    }

    async fn fetch_chapters(
        &self,
        fetch: &dyn Fetcher,
        work: &SourceWork,
        _creds: Option<&Credentials>,
    ) -> SourceResult<Vec<SourceChapter>> {
        if work.chapters.is_empty() {
            return Err(SourceError::Parse(format!(
                "ficbook work {} lists no parts",
                work.source_work_key
            )));
        }

        let mut chapters = Vec::with_capacity(work.chapters.len());
        for entry in &work.chapters {
            let page = fetch
                .get(&self.part_url(&work.source_work_key, &entry.source_chapter_key))
                .await?;
            chapters.push(self.parse_chapter(&page.body, work, &entry.source_chapter_key)?);
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
        let entry = work
            .chapters
            .iter()
            .find(|entry| entry.ordinal == ordinal)
            .ok_or_else(|| {
                SourceError::Unsupported(format!(
                    "ficbook work {} has no part {ordinal}",
                    work.source_work_key
                ))
            })?;
        let page = fetch
            .get(&self.part_url(&work.source_work_key, &entry.source_chapter_key))
            .await?;
        self.parse_chapter(&page.body, work, &entry.source_chapter_key)
    }

    fn preview_from_html(&self, html: &str, url: &Url) -> SourceResult<SourceWork> {
        let (work, _) = Ficbook::read_path(url).ok_or_else(|| {
            SourceError::Unsupported(format!("{url} is not a Ficbook story address"))
        })?;
        self.parse_work(html, &work)
    }

    fn chapters_from_html(
        &self,
        html: &str,
        work: &SourceWork,
    ) -> SourceResult<Vec<SourceChapter>> {
        // A part page states its own address, so the ordinal comes from the
        // page rather than from the caller — see the module documentation.
        let document = Html::parse_document(html);
        let part = canonical_part(&document).ok_or_else(|| {
            SourceError::Parse(format!(
                "ficbook page for work {} does not state its own address",
                work.source_work_key
            ))
        })?;

        let mut chapter = self.parse_chapter(html, work, &part)?;
        chapter.ordinal = Ficbook::ordinal_of(work, &part).unwrap_or(chapter.ordinal);
        Ok(vec![chapter])
    }
}

// ---------------------------------------------------------------------------
// Parsing helpers.
// ---------------------------------------------------------------------------

/// The part list, in the page's order.
///
/// Scoped to `ul.list-of-fanfic-parts` rather than to every `a.part-link`,
/// because the same page carries "next chapter" navigation links of the same
/// shape: collecting every match would put a duplicate of one part in the list
/// and, on a one-part work, would double it.
fn part_refs(document: &Html) -> Vec<ChapterRef> {
    let Ok(selector) = Selector::parse("ul.list-of-fanfic-parts li.part a.part-link") else {
        return Vec::new();
    };
    let Ok(title_selector) = Selector::parse("div.part-title h3") else {
        return Vec::new();
    };

    document
        .select(&selector)
        .filter_map(|anchor| {
            let href = anchor.value().attr("href")?;
            let part = href
                .split('#')
                .next()
                .and_then(|path| path.rsplit('/').next())?
                .to_owned();
            if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let title = anchor
                .select(&title_selector)
                .next()
                .map(|heading| collapse_whitespace(&heading.text().collect::<String>()))
                .unwrap_or_default();
            Some((part, title))
        })
        .enumerate()
        .map(|(index, (part, title))| ChapterRef {
            ordinal: (index as u32) + 1,
            // The site's own id for the part, which is what its address carries.
            source_chapter_key: part,
            title,
        })
        .collect()
}

/// When each part was posted, in the page's order.
fn part_dates(document: &Html) -> Vec<Option<OffsetDateTime>> {
    let Ok(selector) = Selector::parse("ul.list-of-fanfic-parts li.part div.part-info span[title]")
    else {
        return Vec::new();
    };
    document
        .select(&selector)
        .map(|span| span.value().attr("title").and_then(parse_site_date))
        .collect()
}

/// The `Размер:` line, split into the numbers it states.
#[derive(Debug, Default, PartialEq, Eq)]
struct Size {
    words: Option<u32>,
    parts: Option<u32>,
    pages: Option<u32>,
}

/// Read the size line.
///
/// By unit rather than by position, because the line has two recorded shapes and
/// the first field is optional. `77\xa0507` is why the digits are collected
/// rather than the string parsed: the thousands separator is a non-breaking
/// space, not a comma.
fn size_line(document: &Html) -> Size {
    let Some(line) = size_text(document) else {
        return Size::default();
    };
    Size {
        words: number_before(&line, "слов"),
        parts: number_before(&line, "част"),
        pages: number_before(&line, "страниц"),
    }
}

/// The text of the `Размер:` block.
fn size_text(document: &Html) -> Option<String> {
    let Ok(selector) = Selector::parse("strong") else {
        return None;
    };
    let label = document.select(&selector).find(|element| {
        collapse_whitespace(&element.text().collect::<String>()).starts_with("Размер")
    })?;
    // The value is the label's next sibling element, which is the `<div>` the
    // site writes beside it.
    let value = label.next_siblings().find_map(|node| {
        let element = scraper::ElementRef::wrap(node)?;
        Some(collapse_whitespace(&element.text().collect::<String>()))
    })?;
    (!value.is_empty()).then_some(value)
}

/// The number written immediately before a unit word.
///
/// `number_before("… 77 507 слов …", "слов")` is 77507. The scan takes the run
/// of digits and separators immediately before the unit and strips everything
/// that is not a digit, so the separator may be a space, a non-breaking space, a
/// comma or nothing at all.
fn number_before(line: &str, unit: &str) -> Option<u32> {
    let at = line.find(unit)?;
    let before = &line[..at];
    let digits: String = before
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit() || c.is_whitespace() || *c == ',' || *c == '\u{a0}')
        .filter(char::is_ascii_digit)
        .collect();
    let digits: String = digits.chars().rev().collect();
    digits.parse().ok()
}

/// The status the page's own badge class states.
fn status_of(document: &Html) -> WorkStatus {
    let Some(class) = badge_class(document, "ds-label-status-") else {
        return WorkStatus::Unknown;
    };
    match class.as_str() {
        "finished" => WorkStatus::Complete,
        "in-progress" => WorkStatus::Ongoing,
        // Only these two are recorded. A status this build has never seen is
        // not evidence of completion either way, so it is reported as unknown
        // rather than guessed at.
        _ => WorkStatus::Unknown,
    }
}

/// The rating the page's own badge class states (`NC-17`, `R`, `PG-13`, `G`).
fn rating_of(document: &Html) -> Option<String> {
    badge_class(document, "ds-label-rating-")
}

/// Find a badge's class suffix.
///
/// The badges are read as class names rather than as their Russian text, so
/// that the parser is not tied to the interface language the page happens to be
/// served in.
fn badge_class(document: &Html, prefix: &str) -> Option<String> {
    let Ok(selector) = Selector::parse("div[class*='ds-label']") else {
        return None;
    };
    document
        .select(&selector)
        .find_map(|element| {
            element
                .value()
                .classes()
                .find_map(|class| class.strip_prefix(prefix).map(str::to_owned))
        })
        .filter(|suffix| !suffix.is_empty())
}

/// The part id a page states as its own address.
fn canonical_part(document: &Html) -> Option<String> {
    let href = attr_of(document, "link[rel='canonical']", "href")?;
    let url = Url::parse(&href).ok()?;
    Ficbook::read_path(&url)?.1
}

/// The notes the page keeps outside the prose, and the prose they belong to.
///
/// Each reference in the prose is an empty placeholder; the text is in a
/// `textFootnotes` object in a script at the bottom of the page. The notes are
/// gathered into a block at the end and the references are numbered, which is
/// what a reader of the printed page would see.
///
/// # Why the numbering follows the prose and not the object
///
/// The object's key order is not the order a reader meets the notes in, and it
/// is not even a stable sort: keys are `fn_{part}_{n}`, so a chapter with ten or
/// more notes sorts `…_10` before `…_2` in any map. Numbering by the object
/// would therefore misnumber every note from the tenth on. Numbering by the
/// prose is also simply the right answer — the marker a reader sees is the one
/// the order must follow — so the object is used only as a lookup.
fn inline_footnotes(body: &str, notes: &NoteMap) -> String {
    if notes.is_empty() {
        return body.to_owned();
    }

    // The placeholders in the order the prose writes them, and the ids found.
    let mut marked = String::with_capacity(body.len());
    let mut numbered: Vec<String> = Vec::new();
    let mut rest = body;

    while let Some(at) = rest.find(FOOTNOTE_MARK) {
        marked.push_str(&rest[..at]);
        let after = &rest[at + FOOTNOTE_MARK.len()..];
        let Some(end) = after.find('"') else {
            break;
        };
        let id = &after[..end];
        // Only a placeholder or an opening tag; a `</span>` is neither.
        let Some(close) = after.find("</span>") else {
            break;
        };
        let Some(note) = notes.get(id) else {
            // A reference with no note in the object: the marker stays as it is
            // rather than being replaced with nothing, so the omission is
            // visible rather than silent.
            marked.push_str(&rest[at..at + FOOTNOTE_MARK.len() + end + 1 + close + 7]);
            rest = &after[close + 7..];
            continue;
        };
        numbered.push(note.clone());
        let number = numbered.len();
        marked.push_str(&format!("<sup>[{number}]</sup>"));
        rest = &after[close + 7..];
    }
    marked.push_str(rest);

    // A note the prose never refers to is still the author's words. It is
    // listed after the others, unnumbered, rather than dropped.
    let orphaned: Vec<&String> = notes
        .iter()
        .filter(|(id, _)| !body.contains(&format!("{FOOTNOTE_MARK}{id}")))
        .map(|(_, text)| text)
        .collect();

    if numbered.is_empty() && orphaned.is_empty() {
        return body.to_owned();
    }

    let mut gathered = String::from("<hr><p><strong>Notes</strong></p><ol>");
    for note in &numbered {
        gathered.push_str(&format!("<li>{}</li>", note.trim()));
    }
    for note in orphaned {
        gathered.push_str(&format!("<li>{}</li>", note.trim()));
    }
    gathered.push_str("</ol>");

    format!("{marked}{gathered}")
}

/// The opening of a footnote reference in the prose.
const FOOTNOTE_MARK: &str = "<span class=\"footnote\" id=\"";

/// The notes a page carries, keyed by the id its prose refers to them by.
type NoteMap = std::collections::HashMap<String, String>;

/// Read the `textFootnotes` object.
///
/// `serde_json` rather than a hand-rolled scan, because the object is JSON with
/// a `\u` escape for every Cyrillic character: a parser that took the escapes
/// literally would store `\u0441` as five characters of text.
fn text_footnotes(document: &Html) -> NoteMap {
    let Ok(selector) = Selector::parse("script") else {
        return NoteMap::new();
    };
    for script in document.select(&selector) {
        let source = script.text().collect::<String>();
        let Some(at) = source.find("textFootnotes") else {
            continue;
        };
        let rest = &source[at..];
        let Some(open) = rest.find('{') else {
            continue;
        };
        let Some(close) = rest.rfind('}') else {
            continue;
        };
        if close <= open {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&rest[open..=close]) else {
            continue;
        };
        let Some(object) = value.as_object() else {
            continue;
        };
        return object
            .iter()
            .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
            .collect();
    }
    NoteMap::new()
}

/// Parse `3 августа 2023 г., 12:37`.
///
/// The day, a genitive Russian month name, a year, and a time. The zone is the
/// site's own and is applied by the caller; the `г.` is the site's abbreviation
/// for *year* and is not part of the number.
fn parse_site_date(raw: &str) -> Option<OffsetDateTime> {
    let cleaned = raw.replace("г.", " ").replace(',', " ");
    let mut words = cleaned.split_whitespace();
    let day: u8 = words.next()?.parse().ok()?;
    let month = month_of(words.next()?)?;
    let year: i32 = words.next()?.parse().ok()?;
    let time = words.next().unwrap_or("00:00");
    let (hour, minute) = time.split_once(':')?;
    let date = Date::from_calendar_date(year, month, day).ok()?;
    let clock = Time::from_hms(hour.parse().ok()?, minute.parse().ok()?, 0).ok()?;
    Some(OffsetDateTime::new_in_offset(date, clock, SITE_OFFSET).to_offset(time::UtcOffset::UTC))
}

/// A Russian month name in its genitive form, which is what a date uses.
fn month_of(name: &str) -> Option<Month> {
    let name = name.to_lowercase();
    let index = match name.as_str() {
        "января" => 1,
        "февраля" => 2,
        "марта" => 3,
        "апреля" => 4,
        "мая" => 5,
        "июня" => 6,
        "июля" => 7,
        "августа" => 8,
        "сентября" => 9,
        "октября" => 10,
        "ноября" => 11,
        "декабря" => 12,
        _ => return None,
    };
    Month::try_from(index).ok()
}

/// Make a site-relative link absolute.
fn absolute(href: &str) -> String {
    if href.starts_with('/') {
        format!("https://{HOST}{href}")
    } else {
        href.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter() -> Ficbook {
        Ficbook::new()
    }

    #[test]
    fn a_reading_address_is_read_for_its_work_and_part() {
        let work =
            Url::parse("https://ficbook.net/readfic/01899919-f575-76ed-8476-cec2348b02bf").unwrap();
        let part =
            Url::parse("https://ficbook.net/readfic/01899919-f575-76ed-8476-cec2348b02bf/35183469")
                .unwrap();

        assert_eq!(
            Ficbook::read_path(&work),
            Some(("01899919-f575-76ed-8476-cec2348b02bf".to_owned(), None))
        );
        assert_eq!(
            Ficbook::read_path(&part),
            Some((
                "01899919-f575-76ed-8476-cec2348b02bf".to_owned(),
                Some("35183469".to_owned())
            ))
        );
    }

    #[test]
    fn a_view_of_a_work_that_is_not_the_reading_page_is_not_claimed() {
        // `/readfic/{work}/download` is what the site's own work page links, and
        // its own `robots.txt` disallows `*download*`. Claiming it would mean
        // accepting an address the fetcher would then refuse.
        for raw in [
            "https://ficbook.net/readfic/01899919-f575-76ed-8476-cec2348b02bf/download",
            "https://ficbook.net/readfic/01899919-f575-76ed-8476-cec2348b02bf/printfic",
            "https://ficbook.net/authors/1878391",
            "https://ficbook.net/fanfiction/books",
        ] {
            let url = Url::parse(raw).unwrap();
            assert!(!adapter().can_handle(&url), "{raw} must not be claimed");
        }
    }

    #[test]
    fn the_query_and_the_fragment_are_not_part_of_the_address() {
        // The site disallows every query string and writes decorative ones on
        // its own links, and a fragment never reaches the server. Both are
        // dropped, which is why the canonical addresses below carry neither.
        let decorated = Url::parse(
            "https://ficbook.net/readfic/01899919-f575-76ed-8476-cec2348b02bf/35183469\
             ?from_promo=1#part_content",
        )
        .unwrap();
        let (work, part) = Ficbook::read_path(&decorated).expect("a decorated address is readable");

        assert_eq!(work, "01899919-f575-76ed-8476-cec2348b02bf");
        assert_eq!(part.as_deref(), Some("35183469"));
        assert_eq!(
            adapter().part_url(&work, "35183469"),
            "https://ficbook.net/readfic/01899919-f575-76ed-8476-cec2348b02bf/35183469"
        );
    }

    #[test]
    fn the_size_line_is_read_by_unit_because_its_shapes_differ() {
        // Both shapes are recorded, from two real works. The plan prefix is
        // present on one and absent on the other, and the word forms differ.
        let running = size_line(&Html::parse_document(
            "<strong>Размер:</strong><div>планируется Макси, написано 158 страниц, 77\u{a0}507 \
             слов, 24 части</div>",
        ));
        assert_eq!(
            running,
            Size {
                words: Some(77_507),
                parts: Some(24),
                pages: Some(158),
            }
        );

        let finished = size_line(&Html::parse_document(
            "<strong>Размер:</strong><div>24 страницы, 8 122 слова, 4 части</div>",
        ));
        assert_eq!(
            finished,
            Size {
                words: Some(8_122),
                parts: Some(4),
                pages: Some(24),
            }
        );
    }

    #[test]
    fn a_number_is_read_through_whatever_separates_its_digits() {
        assert_eq!(number_before("77 507 слов", "слов"), Some(77_507));
        assert_eq!(number_before("77\u{a0}507 слов", "слов"), Some(77_507));
        assert_eq!(number_before("8 122 слова", "слов"), Some(8_122));
        assert_eq!(number_before("24 части", "част"), Some(24));
        assert_eq!(number_before("158 страниц", "страниц"), Some(158));
        assert_eq!(number_before("nothing here", "слов"), None);
    }

    #[test]
    fn the_status_and_the_rating_are_read_from_class_names_not_from_prose() {
        // The visible text is Russian. Reading it would tie the parser to the
        // interface language the page happens to be served in.
        let finished = Html::parse_document(
            "<div class=\"ds-label ds-label-rating-R\">R</div>\
             <div class=\"ds-label ds-label-status-finished\">Завершён</div>",
        );
        assert_eq!(status_of(&finished), WorkStatus::Complete);
        assert_eq!(rating_of(&finished).as_deref(), Some("R"));

        let running = Html::parse_document(
            "<div class=\"ds-label ds-label-rating-NC-17\">NC-17</div>\
             <div class=\"ds-label ds-label-status-in-progress\">В процессе</div>",
        );
        assert_eq!(status_of(&running), WorkStatus::Ongoing);
        assert_eq!(rating_of(&running).as_deref(), Some("NC-17"));

        // A status this build has never seen is not evidence of completion.
        let other = Html::parse_document("<div class=\"ds-label ds-label-status-frozen\">x</div>");
        assert_eq!(status_of(&other), WorkStatus::Unknown);
    }

    #[test]
    fn a_site_date_is_read_in_the_sites_own_zone() {
        // `3 августа 2023 г., 12:37` is 12:37 in Moscow, which is 09:37 UTC.
        // Read as UTC it would be three hours out.
        assert_eq!(
            parse_site_date("3 августа 2023 г., 12:37"),
            Some(time::macros::datetime!(2023-08-03 09:37:00 UTC))
        );
        assert_eq!(
            parse_site_date("11 сентября 2026 г., 12:36"),
            Some(time::macros::datetime!(2026-09-11 09:36:00 UTC))
        );
        assert_eq!(parse_site_date("not a date"), None);
        // Every month name the site can write is known.
        for name in [
            "января",
            "февраля",
            "марта",
            "апреля",
            "мая",
            "июня",
            "июля",
            "августа",
            "сентября",
            "октября",
            "ноября",
            "декабря",
        ] {
            assert!(month_of(name).is_some(), "{name} must be a month");
        }
    }

    #[test]
    fn notes_are_gathered_out_of_the_prose_they_were_written_in() {
        let document = Html::parse_document(
            "<script>const textFootnotes = {\"fn_1_0\":\"(\\u0441) \\u00ab\\u0414\\u043d\\u043e\\u00bb\"};</script>",
        );
        let notes = text_footnotes(&document);
        assert_eq!(notes.len(), 1);
        // The escapes are decoded, not stored as text.
        assert_eq!(notes.get("fn_1_0").map(String::as_str), Some("(с) «Дно»"));

        let body = "<p>Text<span class=\"footnote\" id=\"fn_1_0\"></span></p>";
        let marked = inline_footnotes(body, &notes);
        assert!(marked.contains("<sup>[1]</sup>"), "{marked}");
        assert!(marked.contains("(с) «Дно»"), "{marked}");
        assert!(!marked.contains("class=\"footnote\""), "{marked}");
    }

    #[test]
    fn notes_are_numbered_by_where_the_prose_refers_to_them() {
        // The object is written in an order that is not the reading order, and
        // not even a stable one: `fn_1_10` sorts before `fn_1_2`. Numbering by
        // the object would misnumber every note from the tenth on.
        let document = Html::parse_document(
            "<script>const textFootnotes = {\"fn_1_10\":\"tenth\",\"fn_1_2\":\"second\",\"fn_1_1\":\"first\"};</script>",
        );
        let notes = text_footnotes(&document);
        let body = "<p>a<span class=\"footnote\" id=\"fn_1_1\"></span>\
                   b<span class=\"footnote\" id=\"fn_1_2\"></span>\
                   c<span class=\"footnote\" id=\"fn_1_10\"></span></p>";

        let marked = inline_footnotes(body, &notes);
        let first = marked.find("first").expect("the first note is listed");
        let second = marked.find("second").expect("the second note is listed");
        let tenth = marked.find("tenth").expect("the tenth note is listed");
        assert!(first < second && second < tenth, "{marked}");

        let positions: Vec<usize> = ["<sup>[1]</sup>", "<sup>[2]</sup>", "<sup>[3]</sup>"]
            .iter()
            .map(|marker| {
                marked
                    .find(marker)
                    .unwrap_or_else(|| panic!("{marker} missing"))
            })
            .collect();
        assert!(positions[0] < positions[1] && positions[1] < positions[2]);
    }

    #[test]
    fn a_page_with_no_notes_is_left_alone() {
        let document = Html::parse_document("<div id=\"content\"><p>Text</p></div>");
        let body = "<p>Text</p>";
        assert_eq!(inline_footnotes(body, &text_footnotes(&document)), body);
    }
}
