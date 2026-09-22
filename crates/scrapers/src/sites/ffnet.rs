//! FanFiction.net and FictionPress: one script, two hosts, and two different
//! walls.
//!
//! # Why this is one module
//!
//! The two sites are the same software with different skins. `#profile_top`,
//! `#chap_select`, `#storytext`, the `/s/{id}/{ordinal}/{slug}` address and the
//! one-line metadata block are on both, and a parser written against one is a
//! parser written against both. They are still two adapters, because a source
//! key is stored on every row and "a work imported from FanFiction.net" is not
//! the same claim as "a work imported from FictionPress" — but they share the
//! reading and differ only in [`Site`], which is where the differences are
//! written down.
//!
//! # The differences are the whole point
//!
//! Measured on 2026-09-11, and each of these is a defect waiting for a parser
//! that assumed the first host it saw was the shape of the source:
//!
//! * **The walls differ, and they are not the same wall.** A plain request is
//!   refused by both. FanFiction.net then accepts a browser's TLS and HTTP/2
//!   fingerprint; FictionPress refuses that too and answers a challenge only a
//!   driven browser clears. So [`Site::wall`] returns [`Wall::Fingerprint`] for
//!   one and [`Wall::Solver`] for the other. A `Wall` is a property of *one
//!   host* — inheriting FanFiction.net's answer would have FictionPress imports
//!   refused with "a fingerprint is enough" and then fail on every page.
//! * **Attribute quoting differs.** FanFiction.net writes `id=chap_select` and
//!   `value=1 selected`; FictionPress writes `id="chap_select"` and
//!   `value="1" selected=""`. Invisible to an HTML parser, lethal to a regex —
//!   during reconnaissance it produced a false finding that FictionPress omits
//!   its first chapter, because `selected=""` did not match a pattern written
//!   for `selected>`.
//! * **The visible date format differs.** `3/14/2015` against `Jun 20, 2016`.
//!   Neither is parsed. Both hosts carry the real timestamp in a `data-xutime`
//!   attribute as epoch seconds, and that is the value.
//! * **The characters field is optional.** FictionPress omits it on both
//!   recorded works; FanFiction.net always has it. The metadata block is one
//!   ` - `-delimited run of labelled fields with an unlabeled run in the middle
//!   — language, genres, characters — so the unlabeled run is one to three long
//!   and nothing may be counted from the start of the line.
//!
//! # The chapter list is `#chap_select`, and it is complete
//!
//! Every chapter is an `<option>` of one `<select id=chap_select>`: 122 of them
//! on the recorded long work, 17 and 4 on the two FictionPress ones. There is no
//! pagination and no second table of contents. The list is rendered **twice**,
//! in the top and bottom navigation, identically; reading the first is what this
//! adapter does.
//!
//! The option's `value` is the ordinal and its text is `{ordinal}. {title}`.
//! That makes this the source where **the ordinal is the source's own chapter
//! key**, which is worth stating rather than being quietly grateful for: the
//! rule is to use the source's own key, and the site has nothing better to
//! offer. The chapter's address is `/s/{work id}/{ordinal}/{slug}` and the slug
//! is decorative — verified live on both hosts, where `/s/{id}/2/` and
//! `/s/{id}/` serve the same chapters as their slugged forms. A decorative slug
//! must not be part of a key.
//!
//! The `selected` option states which chapter a page *is*. This site's pages do
//! not otherwise say their own position, and that is how a single recorded
//! chapter page can be read without the work page — and how a page that answers
//! with the wrong chapter is caught instead of stored.
//!
//! # Not found is a `200`
//!
//! Both hosts answer a missing work with **HTTP 200** and a page carrying no
//! `#profile_top`, no `#chap_select` and no `#storytext`, so the status code
//! says nothing and the document has to be read. The page's heading is `Story
//! Not Found`.
//!
//! It *also* contains the sentence "Story is unavailable for reading." That
//! sentence is boilerplate for a work that simply does not exist, and it is
//! deliberately **not** read as a moderation hold: doing so would send an
//! operator looking for a takedown that never happened.
//!
//! # What these recordings do not cover
//!
//! A genuinely withheld work has not been recorded, so this adapter claims
//! [`SourceError::NotFound`] for the missing-work page and claims nothing about
//! a hold. Both hosts also serve every page — the missing-work page included —
//! with `NOARCHIVE` in a `robots` meta tag; that is a directive about
//! search-engine caches rather than about a reader keeping a copy of a work they
//! are reading, and an instance's operator is the one who decides what their
//! instance stores.
//!
//! # Provenance
//!
//! Ported from `fanfic-scrapers`' `fanfiction.net` and `fictionpress.com`
//! definitions, then checked against pages recorded from the live sites on
//! 2026-09-11 (see `tests/fixtures/README.md`). The port's three selectors
//! (`#profile_top b`, `#chap_select`, `#storytext`) all hold. What did not carry
//! across: a hardcoded `ongoing` status, set for every work whether the page
//! said so or not — on this site the `Status:` field is present or absent, and a
//! work without it is `unknown`.

use async_trait::async_trait;
use scraper::{Html, Selector};
use time::OffsetDateTime;
use url::Url;

use crate::sanitize::sanitize_fragment;
use crate::{
    attr_of, collapse_whitespace, html_of, text_of, AuthKind, ChapterRef, Credentials, Fetcher,
    SourceAdapter, SourceCapabilities, SourceChapter, SourceError, SourceKey, SourceResult,
    SourceWork, Wall, WorkStatus,
};

/// The host FanFiction.net is read from, without `www.`.
///
/// The allow-list holds the bare host because `host_matches` trims the
/// `www.` — a URL that arrives with it and one that does not are the same site,
/// and listing both would claim a host twice.
pub const FANFICTION_NET_HOST: &str = "fanfiction.net";

/// The host FictionPress is read from, without `www.`.
pub const FICTIONPRESS_HOST: &str = "fictionpress.com";

/// The key FanFiction.net registers under.
pub const FANFICTION_NET_KEY: &str = "ffnet";

/// The key FictionPress registers under.
pub const FICTIONPRESS_KEY: &str = "fictionpress";

/// The `robots.txt` pace both hosts publish (`crawl-delay: 5`).
///
/// Read from each host's own file on 2026-09-11, where both also carry
/// `Allow: /` for `User-agent: *` and `Content-Signal: search=yes,
/// ai-train=no, use=reference`. The fetcher enforces the pace per host
/// regardless; this is the adapter telling an operator what to expect, and on a
/// solver-backed import it is not the dominant cost — a driven browser takes
/// about twelve seconds a page on its own.
pub const PACING_MILLIS: u64 = 5_000;

/// Which of the two sites an adapter is.
///
/// Every difference that is not the shared reading lives here, so the two
/// adapters cannot drift apart in the parser while claiming to be siblings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Site {
    /// FanFiction.net: refused plainly, served to a browser fingerprint.
    FanFictionNet,
    /// FictionPress: refused plainly **and** to a fingerprint; needs a solver.
    FictionPress,
}

impl Site {
    /// This site's registry key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::FanFictionNet => FANFICTION_NET_KEY,
            Self::FictionPress => FICTIONPRESS_KEY,
        }
    }

    /// What this site is called, for a reader.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::FanFictionNet => "FanFiction.net",
            Self::FictionPress => "FictionPress",
        }
    }

    /// The host this site is read from, bare, for the fetcher's allow-list.
    #[must_use]
    pub const fn host(self) -> &'static str {
        match self {
            Self::FanFictionNet => FANFICTION_NET_HOST,
            Self::FictionPress => FICTIONPRESS_HOST,
        }
    }

    /// The origin canonical URLs are built on.
    ///
    /// `www.`, because that is the host the site's own links use and the one
    /// both hosts redirect the bare name to. A stored `source_url` is fetched
    /// again on every update check, so it is worth being the address the site
    /// answers directly.
    #[must_use]
    pub const fn origin(self) -> &'static str {
        match self {
            Self::FanFictionNet => "https://www.fanfiction.net",
            Self::FictionPress => "https://www.fictionpress.com",
        }
    }

    /// The least this host needs before it will serve a page, as measured.
    ///
    /// The two differ, which is the clearest example in this repository of a
    /// wall belonging to a host rather than to the software a host runs.
    #[must_use]
    pub const fn wall(self) -> Wall {
        match self {
            Self::FanFictionNet => Wall::Fingerprint,
            Self::FictionPress => Wall::Solver,
        }
    }
}

/// One of the two sites, as a [`SourceAdapter`].
#[derive(Debug, Clone)]
pub struct FanFiction {
    site: Site,
    key: SourceKey,
    hosts: Vec<String>,
}

impl FanFiction {
    /// The FanFiction.net adapter.
    #[must_use]
    pub fn fanfiction_net() -> Self {
        Self::new(Site::FanFictionNet)
    }

    /// The FictionPress adapter.
    #[must_use]
    pub fn fiction_press() -> Self {
        Self::new(Site::FictionPress)
    }

    /// The adapter for one site.
    #[must_use]
    pub fn new(site: Site) -> Self {
        Self {
            site,
            key: SourceKey::new(site.key()),
            hosts: vec![site.host().to_owned()],
        }
    }

    /// Which site this is.
    #[must_use]
    pub const fn site(&self) -> Site {
        self.site
    }

    fn host_matches(&self, host: &str) -> bool {
        let host = host.trim_start_matches("www.");
        self.hosts.iter().any(|ours| host == ours)
    }

    /// The work id in a `/s/{id}/...` path.
    ///
    /// Returns `None` for anything else on these hosts — including `/u/{id}`
    /// profiles and the site's own static pages.
    fn work_id(url: &Url) -> Option<String> {
        let mut segments = url.path_segments()?;
        if segments.next()? != "s" {
            return None;
        }
        let id = segments.next()?;
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        Some(id.to_owned())
    }

    /// The canonical work address, which the site serves chapter 1 at.
    fn work_url(&self, work_id: &str) -> String {
        format!("{}/s/{work_id}/", self.site.origin())
    }

    /// A chapter's address.
    ///
    /// Without the slug: both hosts serve it, and a slug is generated from the
    /// title, so including it would make the address change when an author
    /// renames a chapter.
    fn chapter_url(&self, work_id: &str, ordinal: u32) -> String {
        format!("{}/s/{work_id}/{ordinal}/", self.site.origin())
    }

    /// Read a work's metadata and chapter list out of its page.
    fn parse_work(&self, html: &str, work_id: &str) -> SourceResult<SourceWork> {
        let document = Html::parse_document(html);
        if !has_work_structure(&document) {
            return Err(absence(html, self.site));
        }

        let title = text_of(&document, "#profile_top b").ok_or_else(|| {
            SourceError::Parse(format!(
                "{} page for /s/{work_id}/ has a metadata block with no title",
                self.site.display_name()
            ))
        })?;

        let author_text = text_of(&document, "#profile_top a[href^='/u/']").unwrap_or_default();
        let author_url = attr_of(&document, "#profile_top a[href^='/u/']", "href")
            .map(|href| absolute(self.site, &href));

        let summary = text_of(&document, "#profile_top div.xcontrast_txt").unwrap_or_default();

        let metadata = text_of(&document, "#profile_top span.xgray")
            .map(|line| Metadata::parse(&line))
            .unwrap_or_default();

        let stamps = stamps(&document);
        let (updated_epoch, published_epoch) = pair_stamps(
            &stamps,
            metadata.updated_text.as_deref(),
            metadata.published_text.as_deref(),
        );

        let chapters = chapter_refs(&document);

        // The site states its own chapter count beside a list it renders in
        // full. They disagree only when the page is not what it appears to be —
        // a partial render, a cached page from a different revision — and an
        // import that trusted the list would then import a fraction of a work
        // and report success.
        if let Some(stated) = metadata.chapters {
            let listed = chapters.len();
            if listed > 0 && listed != stated as usize {
                return Err(SourceError::Parse(format!(
                    "{} page for /s/{work_id}/ states {stated} chapters and lists {listed}",
                    self.site.display_name()
                )));
            }
        }

        // The page states its own id, and it is the id the URL carries. A page
        // that disagrees means one of the two was read wrongly, and importing it
        // under the wrong key would attach a work to somebody else's row.
        if let Some(stated) = metadata.work_id.as_deref() {
            if stated != work_id {
                return Err(SourceError::Parse(format!(
                    "{} page for /s/{work_id}/ states id {stated}",
                    self.site.display_name()
                )));
            }
        }

        let tags: Vec<String> = metadata
            .genres
            .iter()
            .chain(metadata.characters.iter())
            .cloned()
            .collect();

        Ok(SourceWork {
            source_key: self.key.clone(),
            source_work_key: work_id.to_owned(),
            source_url: self.work_url(work_id),
            title,
            author_text,
            author_url,
            summary,
            word_count: metadata.words,
            language: metadata.language,
            status: metadata.status,
            published_at: published_epoch.and_then(utc),
            updated_at: updated_epoch.and_then(utc),
            chapters,
            rating_text: metadata.rating,
            warning_texts: Vec::new(),
            tags,
        })
    }

    /// Read one chapter's page.
    ///
    /// `ordinal` is what the caller asked for, and the page's own `selected`
    /// option is checked against it: a page that answers with a different
    /// chapter is a page whose prose would otherwise be stored under the wrong
    /// position, and a chapter stored at the wrong ordinal is a reader's
    /// progress and notes pointing at somebody else's text.
    async fn read_chapter(
        &self,
        fetch: &dyn Fetcher,
        work_id: &str,
        ordinal: u32,
        title: &str,
    ) -> SourceResult<SourceChapter> {
        let target = self.chapter_url(work_id, ordinal);
        let page = fetch.get(&target).await?;
        let base = Url::parse(&page.final_url)
            .or_else(|_| Url::parse(&target))
            .expect("a URL built from a constant origin must parse");

        let document = Html::parse_document(&page.body);
        if !has_work_structure(&document) {
            return Err(absence(&page.body, self.site));
        }

        if let Some(served) = selected_ordinal(&document) {
            if served != ordinal {
                return Err(SourceError::Parse(format!(
                    "{} served chapter {served} when chapter {ordinal} of /s/{work_id}/ was asked for",
                    self.site.display_name()
                )));
            }
        }

        let raw = html_of(&document, "div#storytext").ok_or_else(|| {
            SourceError::Parse(format!(
                "{} chapter page for /s/{work_id}/{ordinal}/ has no #storytext",
                self.site.display_name()
            ))
        })?;

        Ok(SourceChapter {
            ordinal,
            // The ordinal is the source's own key: the chapter list carries no
            // per-chapter id, and the slug is decorative.
            source_chapter_key: ordinal.to_string(),
            title: title.to_owned(),
            content_html: sanitize_fragment(&raw, Some(&base)),
            image_urls: crate::sanitize::extract_image_urls(&raw, Some(&base)),
        })
    }
}

impl Default for FanFiction {
    fn default() -> Self {
        Self::fanfiction_net()
    }
}

#[async_trait]
impl SourceAdapter for FanFiction {
    fn key(&self) -> SourceKey {
        self.key.clone()
    }

    fn display_name(&self) -> &'static str {
        self.site.display_name()
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities {
            metadata: true,
            chapters: true,
            // Every chapter has its own address, and the address is derivable
            // from the ordinal alone. So a retry re-reads one chapter and
            // touches nothing else.
            per_chapter_fetch: true,
            // Author pages exist (`/u/{id}`) and their markup has not been
            // recorded, so this claims nothing about them.
            bibliography: false,
            // The work page carries the last-change epoch, and it is also the
            // page the chapter list comes from: "has this changed?" costs one
            // request.
            incremental: true,
            authentication: AuthKind::None,
            min_interval_millis: Some(PACING_MILLIS),
        }
    }

    fn wall(&self) -> Wall {
        self.site.wall()
    }

    fn can_handle(&self, url: &Url) -> bool {
        let Some(host) = url.host_str() else {
            return false;
        };
        self.host_matches(host) && FanFiction::work_id(url).is_some()
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
        // A chapter URL is normalised to the work's own address first, so
        // pasting chapter 7 previews the work rather than the chapter.
        let work_id = FanFiction::work_id(url).ok_or_else(|| {
            SourceError::Unsupported(format!(
                "{url} is not a {} story address",
                self.site.display_name()
            ))
        })?;
        let page = fetch.get(&self.work_url(&work_id)).await?;
        self.parse_work(&page.body, &work_id)
    }

    async fn fetch_chapters(
        &self,
        fetch: &dyn Fetcher,
        work: &SourceWork,
        _creds: Option<&Credentials>,
    ) -> SourceResult<Vec<SourceChapter>> {
        // A work whose list is empty is a parse failure, not an empty work: the
        // adapter claims `chapters`, so it either enumerated them or it did not
        // understand the page. Returning an empty vector here would be an import
        // that reports success having stored no prose.
        if work.chapters.is_empty() {
            return Err(SourceError::Parse(format!(
                "{} work {} lists no chapters",
                self.site.display_name(),
                work.source_work_key
            )));
        }

        let mut chapters = Vec::with_capacity(work.chapters.len());
        for entry in &work.chapters {
            chapters.push(
                self.read_chapter(fetch, &work.source_work_key, entry.ordinal, &entry.title)
                    .await?,
            );
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
        let title = work
            .chapters
            .iter()
            .find(|entry| entry.ordinal == ordinal)
            .map(|entry| entry.title.clone())
            .ok_or_else(|| {
                SourceError::Unsupported(format!(
                    "work {} has no chapter {ordinal}",
                    work.source_work_key
                ))
            })?;
        self.read_chapter(fetch, &work.source_work_key, ordinal, &title)
            .await
    }

    fn preview_from_html(&self, html: &str, url: &Url) -> SourceResult<SourceWork> {
        let work_id = FanFiction::work_id(url).ok_or_else(|| {
            SourceError::Unsupported(format!(
                "{url} is not a {} story address",
                self.site.display_name()
            ))
        })?;
        self.parse_work(html, &work_id)
    }

    fn chapters_from_html(
        &self,
        html: &str,
        work: &SourceWork,
    ) -> SourceResult<Vec<SourceChapter>> {
        // A chapter document holds one chapter. Its position comes from the
        // option the page marks `selected` — the page does not otherwise state
        // it — and the title from the work's own list when that list has the
        // ordinal, so this agrees with what the live path stores.
        let document = Html::parse_document(html);
        if !has_work_structure(&document) {
            return Err(absence(html, self.site));
        }

        let ordinal = selected_ordinal(&document).ok_or_else(|| {
            SourceError::Parse(format!(
                "{} chapter page for work {} marks no selected chapter",
                self.site.display_name(),
                work.source_work_key
            ))
        })?;

        let title = work
            .chapters
            .iter()
            .find(|entry| entry.ordinal == ordinal)
            .map(|entry| entry.title.clone())
            .unwrap_or_default();

        let base = Url::parse(&work.source_url).ok();
        let raw = html_of(&document, "div#storytext").ok_or_else(|| {
            SourceError::Parse(format!(
                "{} chapter page for work {} has no #storytext",
                self.site.display_name(),
                work.source_work_key
            ))
        })?;

        Ok(vec![SourceChapter {
            ordinal,
            source_chapter_key: ordinal.to_string(),
            title,
            content_html: sanitize_fragment(&raw, base.as_ref()),
            image_urls: crate::sanitize::extract_image_urls(&raw, base.as_ref()),
        }])
    }
}

/// The metadata block, split into its labelled fields and the unlabeled run.
///
/// The block is one line:
///
/// ```text
/// Rated: Fiction T - English - Humor/Romance - Harry P., Hermione G. -
/// Chapters: 122 - Words: 661,619 - Reviews: 37,687 - Favs: 32,477 -
/// Follows: 23,567 - Updated: 3/14/2015 - Published: 2/28/2010 -
/// Status: Complete - id: 5782108
/// ```
///
/// The labels are the anchors: everything between `Rated:` and `Chapters:` is
/// unlabeled, and that run is `[language] [genres] [characters]` — one to three
/// long, because the characters field is absent on a work that lists none. That
/// is why nothing here counts from the start of the line.
#[derive(Debug, PartialEq, Eq)]
struct Metadata {
    rating: Option<String>,
    language: Option<String>,
    genres: Vec<String>,
    characters: Vec<String>,
    chapters: Option<u32>,
    words: Option<i64>,
    updated_text: Option<String>,
    published_text: Option<String>,
    status: WorkStatus,
    work_id: Option<String>,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            rating: None,
            language: None,
            genres: Vec::new(),
            characters: Vec::new(),
            chapters: None,
            words: None,
            updated_text: None,
            published_text: None,
            // The site's own default: a page that did not say.
            status: WorkStatus::Unknown,
            work_id: None,
        }
    }
}

impl Metadata {
    /// Split a metadata line.
    fn parse(line: &str) -> Self {
        let mut fields = Self::default();
        let mut unlabeled: Vec<&str> = Vec::new();

        for raw in line.split(" - ") {
            let field = raw.trim();
            if field.is_empty() {
                continue;
            }
            if let Some(rest) = field.strip_prefix("Rated:") {
                fields.rating = non_empty(rest);
            } else if let Some(rest) = field.strip_prefix("Chapters:") {
                fields.chapters = count(rest);
            } else if let Some(rest) = field.strip_prefix("Words:") {
                fields.words = count(rest).map(i64::from);
            } else if let Some(rest) = field.strip_prefix("Updated:") {
                fields.updated_text = non_empty(rest);
            } else if let Some(rest) = field.strip_prefix("Published:") {
                fields.published_text = non_empty(rest);
            } else if let Some(rest) = field.strip_prefix("Status:") {
                fields.status = status_of(rest);
            } else if let Some(rest) = field.strip_prefix("id:") {
                fields.work_id = non_empty(rest);
            } else if is_known_label(field) {
                // `Reviews:`, `Favs:`, `Follows:` — counted by the site, read by
                // nobody here. Named rather than matched by "contains a colon"
                // so that a field this adapter has never seen is treated as part
                // of the unlabeled run instead of being silently dropped.
                continue;
            } else {
                unlabeled.push(field);
            }
        }

        // The run is [language] [genres] [characters]. A language is a single
        // word; genres are slash-separated; characters are comma-separated — so
        // the first field is checked before it is believed, and a page that
        // dropped its language is read as genres-and-characters rather than
        // filing its first genre as a language.
        match unlabeled.split_first() {
            Some((first, rest)) if looks_like_language(first) => {
                fields.language = Some((*first).to_owned());
                fields.genres = split_list(rest.first().copied());
                fields.characters = split_list(rest.get(1).copied());
            }
            Some((first, rest)) => {
                fields.genres = split_list(Some(*first));
                fields.characters = split_list(rest.first().copied());
            }
            None => {}
        }

        fields
    }
}

/// Whether a field is one of the site's own counters.
fn is_known_label(field: &str) -> bool {
    ["Reviews:", "Favs:", "Follows:", "Rated:", "Status:"]
        .iter()
        .any(|label| field.starts_with(label))
}

/// A field's value, or `None` when it is empty.
fn non_empty(raw: &str) -> Option<String> {
    let value = raw.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// A count written with thousands separators (`661,619`).
fn count(raw: &str) -> Option<u32> {
    raw.trim().replace(',', "").parse().ok()
}

/// Whether a field could be the language rather than a genre or a character.
///
/// Deliberately a shape test rather than a list of languages: a list would be a
/// guess about a site that supports more of them than anybody here has seen, and
/// the wrong guess is worse than the shape test. It only has to answer "is this
/// one bare word", because that is what distinguishes it from the two fields
/// that can follow it.
fn looks_like_language(field: &str) -> bool {
    !field.is_empty()
        && !field.contains('/')
        && !field.contains(',')
        && !field.contains(':')
        && field.split_whitespace().count() == 1
}

/// Split a comma- or slash-separated field into its values.
///
/// Both separators, because the two fields that use it use one each: genres are
/// `Humor/Romance` and characters are `Harry P., Hermione G.`. Neither value
/// vocabulary uses the other's separator, so one splitter reads both.
fn split_list(field: Option<&str>) -> Vec<String> {
    field
        .map(|raw| {
            raw.split(['/', ','])
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Read the status field's value.
///
/// The field is present or absent, and that is the whole of it: the two-chapter
/// recorded work carries no `Status:` at all, and the 122-chapter one carries
/// `Status: Complete`. A work without the field is `unknown` — *not* `ongoing`,
/// which is what the ported adapter hardcoded, and which is wrong half the time
/// on this site with nothing on the page to indicate it.
fn status_of(raw: &str) -> WorkStatus {
    let value = raw.to_lowercase();
    if value.contains("complete") {
        WorkStatus::Complete
    } else if value.contains("in-progress") || value.contains("in progress") {
        WorkStatus::Ongoing
    } else {
        WorkStatus::Unknown
    }
}

/// Whether a page carries the structure a work or chapter page has.
///
/// Both hosts answer a missing work with HTTP `200`, so nothing may be read from
/// the status code; this is the check that a document is what it claims to be.
fn has_work_structure(document: &Html) -> bool {
    text_of(document, "#profile_top b").is_some()
}

/// Why a page that is not a work page is not one.
///
/// Two outcomes only, and they are deliberately distinguishable: the site's own
/// missing-work page, and a page this adapter does not understand. Folding the
/// second into the first would report a markup change as a deleted work.
fn absence(html: &str, site: Site) -> SourceError {
    let document = Html::parse_document(html);
    if let Some(text) = text_of(&document, "body") {
        if text.contains("Story Not Found") {
            return SourceError::NotFound;
        }
    }
    SourceError::Parse(format!(
        "{} page is neither a work nor the site's not-found page",
        site.display_name()
    ))
}

/// Every `data-xutime` stamp on the page, in document order.
///
/// The visible text is carried alongside the epoch because the metadata line
/// quotes it — that is what pairs a stamp with the field it belongs to.
fn stamps(document: &Html) -> Vec<(String, i64)> {
    let Ok(selector) = Selector::parse("span[data-xutime]") else {
        return Vec::new();
    };
    document
        .select(&selector)
        .filter_map(|element| {
            let epoch = element.value().attr("data-xutime")?.trim().parse().ok()?;
            Some((
                collapse_whitespace(&element.text().collect::<String>()),
                epoch,
            ))
        })
        .collect()
}

/// Pair the page's stamps with `Updated:` and `Published:`.
///
/// By the visible text the metadata line quotes, not by position, because the
/// label is on the page and the position is a property of the template. The two
/// are the same answer on every recorded page; they stop being the same answer
/// the moment a host adds a third stamp, and the one that reads the label
/// survives that.
///
/// When both labels quote the same day — a work published and never edited —
/// the stamps are assigned in order, which is the order the labels are written
/// in, so the pairing stays right rather than giving both fields one epoch.
fn pair_stamps(
    stamps: &[(String, i64)],
    updated_text: Option<&str>,
    published_text: Option<&str>,
) -> (Option<i64>, Option<i64>) {
    let (mut updated, mut published) = (None, None);
    for (visible, epoch) in stamps {
        if updated.is_none() && Some(visible.as_str()) == updated_text {
            updated = Some(*epoch);
            continue;
        }
        if published.is_none() && Some(visible.as_str()) == published_text {
            published = Some(*epoch);
        }
    }
    (updated, published)
}

/// An epoch as a UTC timestamp.
///
/// UTC because the attribute is epoch seconds and has no zone of its own; the
/// site displays it in the reader's local time, which is not a fact about the
/// work.
fn utc(epoch: i64) -> Option<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp(epoch).ok()
}

/// The chapter list, in the page's order.
///
/// The list is rendered twice, in the top and bottom navigation, and the two are
/// identical. This reads the first; a page where they disagreed would be a page
/// whose shape has changed, and the fixture set is where that shows up.
fn chapter_refs(document: &Html) -> Vec<ChapterRef> {
    let Some(select) = first_select(document) else {
        return Vec::new();
    };
    let Ok(option_selector) = Selector::parse("option") else {
        return Vec::new();
    };

    select
        .select(&option_selector)
        .filter_map(|option| {
            let ordinal: u32 = option.value().attr("value")?.trim().parse().ok()?;
            if ordinal == 0 {
                return None;
            }
            let label = collapse_whitespace(&option.text().collect::<String>());
            Some(ChapterRef {
                ordinal,
                // The site carries no per-chapter id in this list. The ordinal
                // is what it addresses a chapter by, so it is the key.
                source_chapter_key: ordinal.to_string(),
                title: strip_ordinal_prefix(&label, ordinal),
            })
        })
        .collect()
}

/// The ordinal of the option the page marks `selected`.
///
/// This is how a chapter page states its own position, and it is the only thing
/// on the page that does — which is `chapters_from_html`'s whole problem, and
/// how a page that answers with the wrong chapter is caught.
fn selected_ordinal(document: &Html) -> Option<u32> {
    let select = first_select(document)?;
    let selector = Selector::parse("option[selected]").ok()?;
    select
        .select(&selector)
        .find_map(|option| option.value().attr("value")?.trim().parse().ok())
}

/// The first `select#chap_select` on the page.
fn first_select(document: &Html) -> Option<scraper::ElementRef<'_>> {
    let selector = Selector::parse("select#chap_select").ok()?;
    document.select(&selector).next()
}

/// A chapter's title, with the site's own `{ordinal}. ` prefix removed.
///
/// The prefix is removed only when it is the ordinal the option itself claims,
/// so a chapter an author named "2. Something" under a different ordinal keeps
/// its name.
fn strip_ordinal_prefix(label: &str, ordinal: u32) -> String {
    let prefix = format!("{ordinal}. ");
    label
        .strip_prefix(&prefix)
        .map_or_else(|| label.to_owned(), str::to_owned)
}

/// Make a site-relative link absolute.
fn absolute(site: Site, href: &str) -> String {
    if href.starts_with('/') {
        format!("{}{href}", site.origin())
    } else {
        href.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(line: &str) -> Metadata {
        Metadata::parse(line)
    }

    #[test]
    fn the_metadata_line_is_read_through_its_labels() {
        // The recorded two-chapter FanFiction.net work.
        let fields = meta(
            "Rated: Fiction T - English - Humor/Romance - J. Holtzmann, Patty T., Erin G., Abby Y. \
             - Chapters: 2 - Words: 3,781 - Reviews: 3 - Favs: 2 - Follows: 5 - Updated: 2/7/2017 \
             - Published: 1/31/2017 - id: 12345678",
        );

        assert_eq!(fields.rating.as_deref(), Some("Fiction T"));
        assert_eq!(fields.language.as_deref(), Some("English"));
        assert_eq!(fields.genres, vec!["Humor", "Romance"]);
        assert_eq!(
            fields.characters,
            vec!["J. Holtzmann", "Patty T.", "Erin G.", "Abby Y."]
        );
        assert_eq!(fields.chapters, Some(2));
        assert_eq!(fields.words, Some(3_781));
        assert_eq!(fields.updated_text.as_deref(), Some("2/7/2017"));
        assert_eq!(fields.published_text.as_deref(), Some("1/31/2017"));
        assert_eq!(fields.work_id.as_deref(), Some("12345678"));
        assert_eq!(fields.status, WorkStatus::Unknown);
    }

    #[test]
    fn a_completed_work_carries_a_status_the_two_chapter_one_does_not() {
        let fields = meta(
            "Rated: Fiction T - English - Drama/Humor - Harry P., Hermione G. - Chapters: 122 \
             - Words: 661,619 - Reviews: 37,687 - Favs: 32,477 - Follows: 23,567 \
             - Updated: 3/14/2015 - Published: 2/28/2010 - Status: Complete - id: 5782108",
        );
        assert_eq!(fields.status, WorkStatus::Complete);
        assert_eq!(fields.words, Some(661_619));
    }

    #[test]
    fn a_run_of_two_unlabeled_fields_is_language_and_genres() {
        // The recorded FictionPress work: no characters, and no `Follows:`.
        let fields = meta(
            "Rated: Fiction M - English - Romance/Angst - Chapters: 17 - Words: 31,021 \
             - Reviews: 7 - Favs: 9 - Follows: 13 - Updated: Jun 20, 2016 \
             - Published: Mar 13, 2016 - id: 3280165",
        );

        assert_eq!(fields.language.as_deref(), Some("English"));
        assert_eq!(fields.genres, vec!["Romance", "Angst"]);
        assert!(fields.characters.is_empty());
        assert_eq!(fields.updated_text.as_deref(), Some("Jun 20, 2016"));
    }

    #[test]
    fn a_missing_language_does_not_make_a_genre_the_language() {
        // Not a recorded page: the guard this exists for is a page that dropped
        // the language field, where counting from the left would file the genre
        // as a language and shift everything after it.
        let fields =
            meta("Rated: Fiction T - Drama/Humor - Harry P. - Chapters: 3 - Words: 900 - id: 5");

        assert_eq!(fields.language, None);
        assert_eq!(fields.genres, vec!["Drama", "Humor"]);
        assert_eq!(fields.characters, vec!["Harry P."]);
    }

    #[test]
    fn a_count_is_read_through_its_thousands_separators() {
        assert_eq!(count(" 661,619 "), Some(661_619));
        assert_eq!(count("2"), Some(2));
        assert_eq!(count("not a number"), None);
    }

    #[test]
    fn a_status_is_read_from_its_value_and_defaults_to_unknown() {
        assert_eq!(status_of(" Complete"), WorkStatus::Complete);
        assert_eq!(status_of(" In-Progress"), WorkStatus::Ongoing);
        assert_eq!(status_of(" Wrapped Up"), WorkStatus::Unknown);
    }

    #[test]
    fn the_ordinal_prefix_is_removed_only_when_it_is_the_ordinal() {
        assert_eq!(strip_ordinal_prefix("2. Chapter 2", 2), "Chapter 2");
        assert_eq!(
            strip_ordinal_prefix("2. Vampiric Desires Chapter 2", 2),
            "Vampiric Desires Chapter 2"
        );
        // A title that happens to start with another number keeps it.
        assert_eq!(strip_ordinal_prefix("7. Chapter 2", 2), "7. Chapter 2");
        assert_eq!(strip_ordinal_prefix("Prologue", 1), "Prologue");
    }

    #[test]
    fn a_decoration_free_address_is_read_for_its_id() {
        let url = Url::parse("https://www.fanfiction.net/s/12345678/2/Jillian-Holtzmann").unwrap();
        assert_eq!(FanFiction::work_id(&url).as_deref(), Some("12345678"));

        let url = Url::parse("https://www.fanfiction.net/s/12345678/").unwrap();
        assert_eq!(FanFiction::work_id(&url).as_deref(), Some("12345678"));
    }

    #[test]
    fn a_profile_is_not_a_work() {
        let url = Url::parse("https://www.fanfiction.net/u/3631163/Pieland24").unwrap();
        assert_eq!(FanFiction::work_id(&url), None);
        assert!(!FanFiction::fanfiction_net().can_handle(&url));
    }

    #[test]
    fn each_adapter_claims_its_own_host_and_not_its_siblings() {
        let ffnet = FanFiction::fanfiction_net();
        let fictionpress = FanFiction::fiction_press();

        let on_ffnet = Url::parse("https://www.fanfiction.net/s/1/1/").unwrap();
        let on_fictionpress = Url::parse("https://www.fictionpress.com/s/1/1/").unwrap();

        assert!(ffnet.can_handle(&on_ffnet));
        assert!(fictionpress.can_handle(&on_fictionpress));
        assert!(!ffnet.can_handle(&on_fictionpress));
        assert!(!fictionpress.can_handle(&on_ffnet));
    }

    #[test]
    fn the_two_hosts_declare_different_walls() {
        // The measured fact this module exists to keep: a fingerprint passes one
        // and not the other.
        assert_eq!(Site::FanFictionNet.wall(), Wall::Fingerprint);
        assert_eq!(Site::FictionPress.wall(), Wall::Solver);
    }

    #[test]
    fn the_addresses_are_built_without_a_slug() {
        let adapter = FanFiction::fanfiction_net();
        assert_eq!(
            adapter.work_url("12345678"),
            "https://www.fanfiction.net/s/12345678/"
        );
        assert_eq!(
            adapter.chapter_url("12345678", 2),
            "https://www.fanfiction.net/s/12345678/2/"
        );
        // A relative author link is stored absolute, because it is meaningless
        // once the row is away from the page it came from.
        assert_eq!(
            absolute(Site::FictionPress, "/u/1057028/OdderThings"),
            "https://www.fictionpress.com/u/1057028/OdderThings"
        );
    }

    #[test]
    fn a_stamp_is_paired_by_the_label_that_quotes_it() {
        let stamps = vec![
            ("2/7/2017".to_owned(), 1_486_488_197),
            ("1/31/2017".to_owned(), 1_485_879_374),
        ];
        let (updated, published) = pair_stamps(&stamps, Some("2/7/2017"), Some("1/31/2017"));
        assert_eq!(updated, Some(1_486_488_197));
        assert_eq!(published, Some(1_485_879_374));
    }

    #[test]
    fn two_stamps_quoting_one_day_are_still_told_apart() {
        let stamps = vec![("3/14/2015".to_owned(), 2), ("3/14/2015".to_owned(), 1)];
        let (updated, published) = pair_stamps(&stamps, Some("3/14/2015"), Some("3/14/2015"));
        assert_eq!(updated, Some(2), "the first stamp is the one written first");
        assert_eq!(published, Some(1));
    }
}
