//! Scribble Hub: a site that will hand over the list of chapters but keeps the
//! prose behind an interactive challenge.
//!
//! # Two doors, and they are not the same door
//!
//! `scribblehub.com` is behind Cloudflare, and the wall is not uniform:
//!
//! | Path | Plain | Fingerprint | Solver |
//! |---|---|---|---|
//! | `/series/{id}/{slug}/` | refused | refused | 33 KB |
//! | `/read/.../chapter/{id}/` | refused | refused | ~71 KB |
//! | `/wp-json/fictionapp/v1/stories/{id}` | **200** | — | — |
//! | `/wp-json/fictionapp/v1/stories/{id}/chapters` | **200** | — | — |
//!
//! The site's own mobile-app API answers a plain request while every reading
//! page needs a driven browser. So this adapter reads **metadata and the chapter
//! list from the API** and **prose from the reading pages**, and declares
//! [`Wall::Solver`] because every import ends up needing one — the work cannot be
//! imported without its text.
//!
//! # Why the API, and not the series page the reader sees
//!
//! The series page a reader loads shows the **newest fifteen chapters in
//! descending order** (`ol.toc_ol > li.toc_w`, fifteen per page) beside a header
//! reading *Table of Contents 113*. The rest is behind `?toc=N` pagination —
//! which this site's challenge refuses to clear: three attempts through the
//! solver on `/series/2357420/...?toc=1` and `?toc=2`, with and without the
//! site's own `#content1` fragment, timed out at 62–93 seconds each, while the
//! *same path without a query* solved in about seven.
//!
//! An adapter built on that page would therefore import fifteen of a hundred and
//! thirteen chapters and report the work as having a hundred and thirteen — a
//! partial import that looks complete, which is the one failure this codebase
//! treats as unacceptable. The API answers the whole list plainly, so that is
//! what this reads. The recorded series page is kept in the fixtures as the
//! evidence for why it is not used.
//!
//! # Fifty at a time, and the count is checked
//!
//! `/stories/{id}/chapters` returns **fifty** chapters and accepts `page=N`
//! (`per_page`, `offset` and `limit` are ignored — measured, not assumed). The
//! story object states `chapterCount`, so the adapter pages until it has that
//! many and refuses if the pages do not add up: `113` arrives as `50 + 50 + 13`,
//! and a work whose stated count and listed count disagree is a work this
//! adapter did not understand rather than a work to import in part.
//!
//! # The API's chapter number is not the ordinal
//!
//! Each chapter carries a `number`, and it **has gaps**: the recorded work runs
//! `1, 3, 4, 5, …` because chapter 2 was deleted. The import's [`ChapterRef`]
//! ordinal is dense and is what a reader's progress and notes are mapped onto,
//! so the ordinal is the chapter's position in the site's own order and the
//! site's `number` is left where it is — in the chapter title, which already
//! carries it.
//!
//! # What the API does not carry
//!
//! The chapter list's `content` field is **empty** on every chapter, so the prose
//! is only on the reading page — which is why the wall is declared at all. And
//! the story object has no language field, so no language is reported rather
//! than inferred from the prose.
//!
//! # Provenance
//!
//! Written against responses recorded from the live site on 2026-09-11; see
//! `tests/fixtures/scribblehub/` and the `## scribblehub` section of
//! `tests/fixtures/README.md`.

use async_trait::async_trait;
use scraper::Html;
use serde::Deserialize;
use url::Url;

use crate::sanitize::sanitize_fragment;
use crate::{
    attr_of, html_of, AuthKind, ChapterRef, Credentials, Fetcher, SourceAdapter,
    SourceCapabilities, SourceChapter, SourceError, SourceKey, SourceResult, SourceWork, Wall,
    WorkStatus,
};

/// The host, without `www.`.
pub const HOST: &str = "scribblehub.com";

/// The registry key.
pub const SOURCE_KEY: &str = "scribblehub";

/// The site's canonical origin.
///
/// Every address it states for itself — its `<link rel="canonical">`, its own
/// chapter links, its profile links — carries `www.`, so the addresses this
/// adapter builds carry it too. [`HOST`] stays bare because that is what the
/// fetcher's allow-list matches on, with or without the prefix.
const ORIGIN: &str = "https://www.scribblehub.com";

/// The site's own API, under WordPress's REST prefix.
///
/// Reachable with a plain request, unlike every reading page.
const API: &str = "wp-json/fictionapp/v1";

/// How many chapters the API returns to one request.
///
/// Measured: `per_page`, `offset` and `limit` are all ignored and the page size
/// is fixed. Paging past the end returns an empty list rather than an error.
const PER_PAGE: usize = 50;

/// A ceiling on list pages, so a story object claiming an absurd
/// `chapterCount` cannot turn a preview into an unbounded number of requests.
const MAX_LIST_PAGES: usize = 400;

/// The site's pace.
///
/// Scribble Hub publishes no `Crawl-delay` — its `robots.txt` disallows only
/// `/wp-admin/` and allows everything else — so the fetcher's one-second floor
/// is the pace.
pub const PACING_MILLIS: u64 = 1_000;

/// Scribble Hub.
#[derive(Debug, Clone)]
pub struct ScribbleHub {
    key: SourceKey,
    hosts: Vec<String>,
}

impl Default for ScribbleHub {
    fn default() -> Self {
        Self::new()
    }
}

impl ScribbleHub {
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

    /// The story id in a series or reading address.
    ///
    /// `https://www.scribblehub.com/series/2357420/worlds-cutest-alchemist/`
    /// and `https://www.scribblehub.com/read/2357420-worlds-cutest-alchemist/
    /// chapter/2357479/` both name the same work, and the id is the first
    /// segment of the second path element in each case.
    pub fn story_id(url: &Url) -> Option<String> {
        let mut segments = url.path_segments()?;
        let first = segments.next()?;
        if first != "series" && first != "read" {
            return None;
        }
        let second = segments.next()?;
        let id: String = second.chars().take_while(char::is_ascii_digit).collect();
        (!id.is_empty()).then_some(id)
    }

    /// The chapter id in a reading address.
    pub fn chapter_id(url: &Url) -> Option<String> {
        let mut segments = url.path_segments()?;
        if segments.next()? != "read" {
            return None;
        }
        segments.next()?;
        if segments.next()? != "chapter" {
            return None;
        }
        let id = segments.next()?;
        let id: String = id.chars().take_while(char::is_ascii_digit).collect();
        (!id.is_empty()).then_some(id)
    }

    /// The story's own address, which is what [`SourceWork::source_url`] holds.
    ///
    /// Public with the three below so a test can check that every address this
    /// adapter builds is one the source's `robots.txt` allows.
    #[must_use]
    pub fn story_url(&self, id: &str) -> String {
        format!("{ORIGIN}/series/{id}/")
    }

    /// The address of a story object.
    #[must_use]
    pub fn api_story(&self, id: &str) -> String {
        format!("{ORIGIN}/{API}/stories/{id}")
    }

    /// The address of one page of a story's chapter list.
    #[must_use]
    pub fn api_chapters(&self, id: &str, page: usize) -> String {
        format!("{ORIGIN}/{API}/stories/{id}/chapters?page={page}")
    }

    /// A chapter's reading address.
    ///
    /// The site's own shape is `/read/{storyId}-{slug}/chapter/{chapterId}/`,
    /// and the slug comes from the story object. A chapter whose slug is unknown
    /// is still addressable: the site redirects `/read/{storyId}/chapter/{id}/`
    /// to the canonical form, so the fallback is a valid address rather than a
    /// guess at the slug.
    #[must_use]
    pub fn chapter_url(&self, story_id: &str, slug: &str, chapter_id: &str) -> String {
        if slug.is_empty() {
            format!("{ORIGIN}/read/{story_id}/chapter/{chapter_id}/")
        } else {
            format!("{ORIGIN}/read/{slug}/chapter/{chapter_id}/")
        }
    }

    /// Read a story object into a work, without its chapter list.
    ///
    /// # Errors
    ///
    /// [`SourceError::Parse`] when the document is not the story object this
    /// adapter expects, or when the site reports the story missing — the API
    /// answers a missing story with `404` and a `fa_story_not_found` envelope
    /// rather than an empty object.
    pub fn parse_story(&self, json: &str, story_id: &str) -> SourceResult<Story> {
        let what = format!("story {story_id}");
        let envelope: Envelope = serde_json::from_str(json).map_err(|error| {
            SourceError::Parse(format!(
                "scribblehub story {story_id} is not a story object: {error}"
            ))
        })?;
        let story: Story = envelope.payload(&what)?;
        if story.id.to_string() != story_id {
            return Err(SourceError::Parse(format!(
                "scribblehub served story {} when story {story_id} was asked for",
                story.id
            )));
        }
        Ok(story)
    }

    /// Read one page of the chapter list.
    ///
    /// # Errors
    ///
    /// [`SourceError::Parse`] when the page is not a list of chapters.
    pub fn parse_chapter_page(&self, json: &str) -> SourceResult<Vec<ApiChapter>> {
        let envelope: Envelope = serde_json::from_str(json).map_err(|error| {
            SourceError::Parse(format!("scribblehub chapter list is not a list: {error}"))
        })?;
        envelope.payload("a chapter list")
    }

    /// Assemble the chapter list from the API's pages.
    ///
    /// # Errors
    ///
    /// [`SourceError::Parse`] when the pages do not add up to the count the
    /// story object states. A work whose stated count and listed count disagree
    /// is a work this adapter did not understand: importing the pages it did get
    /// would import part of a work and report success.
    pub fn assemble_chapters(
        &self,
        pages: &[Vec<ApiChapter>],
        story_id: &str,
        declared: u32,
    ) -> SourceResult<Vec<ChapterRef>> {
        let listed: Vec<&ApiChapter> = pages.iter().flatten().collect();
        if declared != 0 && listed.len() != declared as usize {
            return Err(SourceError::Parse(format!(
                "scribblehub story {story_id} states {declared} chapters and lists {}",
                listed.len()
            )));
        }
        if listed.is_empty() {
            return Err(SourceError::Parse(format!(
                "scribblehub story {story_id} lists no chapters"
            )));
        }
        // The ordinal is the position in the site's own order — dense, and what
        // a reader's progress is mapped onto. The API's `number` has gaps and is
        // not it; see the module documentation.
        Ok(listed
            .iter()
            .enumerate()
            .map(|(index, chapter)| ChapterRef {
                ordinal: (index as u32) + 1,
                source_chapter_key: chapter.id.to_string(),
                title: chapter.title.clone(),
            })
            .collect())
    }

    /// Read one chapter's prose.
    ///
    /// # Errors
    ///
    /// [`SourceError::Parse`] when the page carries no `#chp_raw`, and when the
    /// page states an address for a different chapter than the one asked for —
    /// the prose would otherwise be stored under the wrong ordinal.
    pub fn parse_chapter(
        &self,
        html: &str,
        work: &SourceWork,
        chapter_id: &str,
    ) -> SourceResult<SourceChapter> {
        let document = Html::parse_document(html);

        if let Some(served) = canonical_chapter(&document) {
            if served != chapter_id {
                return Err(SourceError::Parse(format!(
                    "scribblehub served chapter {served} when chapter {chapter_id} \
                     of story {} was asked for",
                    work.source_work_key
                )));
            }
        }

        let body = html_of(&document, "div#chp_raw").ok_or_else(|| {
            SourceError::Parse(format!(
                "scribblehub chapter {chapter_id} of story {} has no #chp_raw",
                work.source_work_key
            ))
        })?;

        let entry = work
            .chapters
            .iter()
            .find(|entry| entry.source_chapter_key == chapter_id);

        Ok(SourceChapter {
            ordinal: entry.map_or(0, |entry| entry.ordinal),
            source_chapter_key: chapter_id.to_owned(),
            title: entry.map(|entry| entry.title.clone()).unwrap_or_default(),
            content_html: sanitize_fragment(&body, Url::parse(&work.source_url).ok().as_ref()),
            image_urls: crate::sanitize::extract_image_urls(
                &body,
                Url::parse(&work.source_url).ok().as_ref(),
            ),
        })
    }

    /// The chapter a reading page states as its own address.
    fn chapter_of(&self, work: &SourceWork, document: &Html) -> Option<String> {
        canonical_chapter(document).filter(|id| {
            work.chapters
                .iter()
                .any(|entry| &entry.source_chapter_key == id)
        })
    }
}

// ---------------------------------------------------------------------------
// The trait.
// ---------------------------------------------------------------------------

#[async_trait]
impl SourceAdapter for ScribbleHub {
    fn key(&self) -> SourceKey {
        self.key.clone()
    }

    fn display_name(&self) -> &'static str {
        "Scribble Hub"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities {
            metadata: true,
            // True, and gated on a solver: the text is only on the reading
            // pages, which is why `wall` below is declared.
            chapters: true,
            per_chapter_fetch: true,
            // The API has `/authors/{id}` and `/users/{id}/stories`; their
            // shapes have not been recorded.
            bibliography: false,
            // `lastUpdated` is on the story object, so "has this changed?" costs
            // one request and compares one field.
            incremental: true,
            authentication: AuthKind::None,
            min_interval_millis: Some(PACING_MILLIS),
        }
    }

    /// Measured: a plain request and a browser fingerprint are both refused on
    /// every reading page, and the solver clears the challenge in about seven
    /// seconds. The API is reachable without any of it, but a work cannot be
    /// imported without its text.
    fn wall(&self) -> Wall {
        Wall::Solver
    }

    fn can_handle(&self, url: &Url) -> bool {
        let Some(host) = url.host_str() else {
            return false;
        };
        self.host_matches(host) && ScribbleHub::story_id(url).is_some()
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
        let story_id = ScribbleHub::story_id(url).ok_or_else(|| {
            SourceError::Unsupported(format!("{url} is not a Scribble Hub story address"))
        })?;

        let page = fetch.get(&self.api_story(&story_id)).await?;
        let story = self.parse_story(&page.body, &story_id)?;
        if !story.is_accessible {
            return Err(SourceError::Withheld(format!(
                "scribblehub story {story_id} is not being served"
            )));
        }

        let pages = self
            .read_chapter_pages(fetch, &story_id, story.chapter_count)
            .await?;
        let chapters = self.assemble_chapters(&pages, &story_id, story.chapter_count)?;

        let url = self.story_url(&story_id);
        Ok(story.into_work(self.key.clone(), url, chapters))
    }

    async fn fetch_chapters(
        &self,
        fetch: &dyn Fetcher,
        work: &SourceWork,
        _creds: Option<&Credentials>,
    ) -> SourceResult<Vec<SourceChapter>> {
        if work.chapters.is_empty() {
            return Err(SourceError::Parse(format!(
                "scribblehub story {} lists no chapters",
                work.source_work_key
            )));
        }
        let mut chapters = Vec::with_capacity(work.chapters.len());
        for entry in &work.chapters {
            let page = fetch
                .get(&self.read_url(work, &entry.source_chapter_key))
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
                    "scribblehub story {} has no chapter {ordinal}",
                    work.source_work_key
                ))
            })?;
        let page = fetch
            .get(&self.read_url(work, &entry.source_chapter_key))
            .await?;
        self.parse_chapter(&page.body, work, &entry.source_chapter_key)
    }

    fn preview_from_html(&self, html: &str, url: &Url) -> SourceResult<SourceWork> {
        // The document this seam is handed for this source is the API's story
        // object, not a page — the site's own structured answer for the same
        // work, and the one the adapter actually reads. The chapter list is
        // assembled from a separate set of documents and is left empty here.
        let story_id = ScribbleHub::story_id(url).ok_or_else(|| {
            SourceError::Unsupported(format!("{url} is not a Scribble Hub story address"))
        })?;
        let story = self.parse_story(html, &story_id)?;
        Ok(story.into_work(self.key.clone(), self.story_url(&story_id), Vec::new()))
    }

    fn chapters_from_html(
        &self,
        html: &str,
        work: &SourceWork,
    ) -> SourceResult<Vec<SourceChapter>> {
        let document = Html::parse_document(html);
        let chapter_id = self.chapter_of(work, &document).ok_or_else(|| {
            SourceError::Parse(format!(
                "scribblehub page for story {} does not state a chapter in its list",
                work.source_work_key
            ))
        })?;
        Ok(vec![self.parse_chapter(html, work, &chapter_id)?])
    }
}

impl ScribbleHub {
    /// Page the API until the story's own count is reached.
    async fn read_chapter_pages(
        &self,
        fetch: &dyn Fetcher,
        story_id: &str,
        declared: u32,
    ) -> SourceResult<Vec<Vec<ApiChapter>>> {
        let mut pages = Vec::new();
        let mut collected = 0usize;
        let mut page = 1usize;

        loop {
            let response = fetch.get(&self.api_chapters(story_id, page)).await?;
            let chapters = self.parse_chapter_page(&response.body)?;
            let short = chapters.len() < PER_PAGE;
            collected += chapters.len();
            pages.push(chapters);

            // Stop on a short page — the API's way of saying it has no more —
            // or as soon as the story's own count is met, whichever is first.
            if short || collected >= declared as usize {
                break;
            }
            page += 1;
            if page > MAX_LIST_PAGES {
                return Err(SourceError::Internal(format!(
                    "scribblehub story {story_id} claims {declared} chapters and is still \
                     listing more after {MAX_LIST_PAGES} pages"
                )));
            }
        }
        Ok(pages)
    }

    /// The address of one chapter of a work.
    fn read_url(&self, work: &SourceWork, chapter_id: &str) -> String {
        // The work's address is `/series/{id}/`; the reading address needs the
        // slug, which the work's own URL does not carry. The site redirects
        // `/read/{id}/chapter/{c}/` to the canonical form, so the id alone is a
        // valid address and this needs no slug.
        self.chapter_url(&work.source_work_key, "", chapter_id)
    }
}

// ---------------------------------------------------------------------------
// The site's own shapes.
// ---------------------------------------------------------------------------

/// The API's envelope: `{"data": …, "code": …, "message": …}`.
///
/// # Why `data` is untyped here
///
/// The two recorded shapes disagree about what `data` holds. A good answer puts
/// the payload there; a fault puts a **status object** there and names itself in
/// `code`:
///
/// ```text
/// {"data":{"id":2357420,"title":"…"}}                                  // a story
/// {"code":"fa_story_not_found","message":"Story not found.",
///  "data":{"status":404}}                                             // no story
/// ```
///
/// So `data` is read as an untyped value and converted once `code` has said what
/// the envelope is. Typing it as the payload would make the missing-story case a
/// deserialisation failure about an integer where a string was expected — a
/// *parse* error for a work that simply does not exist, sending a reader looking
/// for a bug in the parser.
#[derive(Debug, Deserialize)]
struct Envelope {
    data: Option<serde_json::Value>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

impl Envelope {
    /// Read the payload, or say why there is not one.
    ///
    /// # Errors
    ///
    /// [`SourceError::NotFound`] for the one fault code recorded, and
    /// [`SourceError::Parse`] for a fault code this build has never seen or an
    /// envelope whose payload is not the shape the caller expects.
    fn payload<T: serde::de::DeserializeOwned>(self, what: &str) -> SourceResult<T> {
        if let Some(code) = self.code {
            let message = self.message.unwrap_or_default();
            match code.as_str() {
                // The only fault recorded. The site's own sentence is logged
                // rather than returned: a reader sees the same words for every
                // source that has no work at a URL.
                "fa_story_not_found" => {
                    tracing::debug!(what, message, "scribblehub has no such story");
                    return Err(SourceError::NotFound);
                }
                // A fault this build has never seen is not evidence that the
                // work is missing, so it is loud.
                _ => {
                    return Err(SourceError::Parse(format!(
                        "scribblehub answered {what} with code {code}: {message}"
                    )))
                }
            }
        }
        let data = self.data.ok_or_else(|| {
            SourceError::Parse(format!("scribblehub answered {what} with no data"))
        })?;
        serde_json::from_value(data).map_err(|error| {
            SourceError::Parse(format!(
                "scribblehub answered {what} with a shape it does not have: {error}"
            ))
        })
    }
}

/// A story, as `/stories/{id}` states it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Story {
    /// The story id.
    pub id: u64,
    /// The title, as the site writes it.
    pub title: String,
    /// `{id}-{slug}`, which is what the reading addresses are built from.
    #[serde(default)]
    pub slug: String,
    /// The site's own summary.
    #[serde(default)]
    pub description: String,
    /// The author.
    pub author: StoryAuthor,
    /// `ongoing` or `completed`, in the two recorded cases.
    #[serde(default)]
    pub status: String,
    /// How many chapters the site says the story has.
    #[serde(default)]
    pub chapter_count: u32,
    /// The site's own word count for the whole story.
    #[serde(default)]
    pub word_count: i64,
    /// Whether the site marks the story mature.
    #[serde(default)]
    pub is_mature: bool,
    /// Whether the site is serving the story at all.
    #[serde(default = "default_true")]
    pub is_accessible: bool,
    /// The site's last change to the story, RFC 3339 in UTC.
    #[serde(default, rename = "lastUpdated")]
    pub last_updated: Option<String>,
    /// The site's genres, which it keeps apart from its tags.
    #[serde(default)]
    pub genres: Vec<Term>,
    /// The site's tags.
    #[serde(default)]
    pub tags: Vec<Term>,
}

fn default_true() -> bool {
    true
}

/// A story's author, as the API states it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoryAuthor {
    /// The author's numeric id, which their profile address carries.
    pub id: u64,
    /// The name the site displays.
    #[serde(default, rename = "displayName")]
    pub display_name: String,
    /// The handle, which their profile address also carries.
    #[serde(default)]
    pub username: String,
}

/// A named term: a genre or a tag.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Term {
    /// The display name.
    pub name: String,
}

/// One chapter, as the list endpoint states it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiChapter {
    /// The chapter id, which its reading address carries.
    pub id: u64,
    /// The title.
    #[serde(default)]
    pub title: String,
    /// The site's own chapter number, which **has gaps** where chapters were
    /// deleted and is therefore not the import's ordinal.
    #[serde(default)]
    pub number: u32,
    /// When the chapter was posted, RFC 3339 in UTC.
    #[serde(default, rename = "publishedAt")]
    pub published_at: Option<String>,
    /// How long the site says the chapter is.
    ///
    /// Not part of the domain's chapter shape, so the import does not carry it —
    /// but it is what makes the chapter list checkable against the work's own
    /// word count, which is the evidence that a list is complete rather than
    /// capped. See the fixture suite.
    #[serde(default)]
    pub word_count: u32,
    /// Always empty on the list endpoint: the prose is only on the reading
    /// page, which is why this adapter declares a wall.
    #[serde(default)]
    pub content: String,
}

impl Story {
    /// Turn a story object and its chapter list into the domain's shape.
    #[must_use]
    pub fn into_work(self, key: SourceKey, url: String, chapters: Vec<ChapterRef>) -> SourceWork {
        SourceWork {
            source_key: key,
            source_work_key: self.id.to_string(),
            source_url: url,
            title: self.title.clone(),
            author_text: self.author.display_name.clone(),
            author_url: Some(format!(
                "https://www.scribblehub.com/profile/{}/{}/",
                self.author.id, self.author.username
            )),
            summary: self.description.clone(),
            // The site's own count for the whole story, not a sum of the
            // chapters' — the list's counts cover only the chapters listed.
            word_count: Some(self.word_count),
            // The story object carries no language field, so none is reported
            // rather than inferred from the prose.
            language: None,
            status: status_of(&self.status),
            published_at: None,
            updated_at: parse_rfc3339(self.last_updated.as_deref()),
            chapters,
            // The site publishes a boolean rather than a scale, and `Mature` is
            // its own word for it — it is the name of its own account setting.
            rating_text: self.is_mature.then(|| "Mature".to_owned()),
            warning_texts: Vec::new(),
            // The site keeps genres and tags apart and the domain keeps one
            // list, so both are carried, genres first: dropping either would
            // lose something the site published.
            tags: self
                .genres
                .iter()
                .chain(self.tags.iter())
                .map(|term| term.name.clone())
                .collect(),
        }
    }
}

/// The status string the API states.
///
/// Two values are recorded — `ongoing` and `completed`. A string this build has
/// not seen is reported as unknown: it is not evidence of completion either way,
/// and guessing would put a finished work in a reader's "in progress" list.
fn status_of(status: &str) -> WorkStatus {
    match status {
        "ongoing" => WorkStatus::Ongoing,
        "completed" => WorkStatus::Complete,
        _ => WorkStatus::Unknown,
    }
}

/// Read an RFC 3339 timestamp, which is what the API writes.
fn parse_rfc3339(raw: Option<&str>) -> Option<time::OffsetDateTime> {
    let raw = raw?;
    time::OffsetDateTime::parse(raw, &time::format_description::well_known::Rfc3339)
        .ok()
        .map(|at| at.to_offset(time::UtcOffset::UTC))
}

/// The chapter id a reading page states as its own address.
fn canonical_chapter(document: &Html) -> Option<String> {
    let href = attr_of(document, "link[rel='canonical']", "href")?;
    ScribbleHub::chapter_id(&Url::parse(&href).ok()?)
}

/// The published date of a chapter, if the list stated one.
///
/// Kept for the fixture tests, which assert that the first chapter's date is
/// the work's publication — the story object carries no created date, so this is
/// the only place it exists.
#[must_use]
pub fn first_published(chapters: &[ApiChapter]) -> Option<time::OffsetDateTime> {
    chapters
        .iter()
        .filter_map(|chapter| parse_rfc3339(chapter.published_at.as_deref()))
        .min()
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn adapter() -> ScribbleHub {
        ScribbleHub::new()
    }

    #[test]
    fn both_of_a_storys_addresses_name_the_same_id() {
        // A series address and a reading address are two ways in to one work.
        for raw in [
            "https://www.scribblehub.com/series/2357420/worlds-cutest-alchemist/",
            "https://www.scribblehub.com/read/2357420-worlds-cutest-alchemist/chapter/2357479/",
        ] {
            let url = Url::parse(raw).unwrap();
            assert_eq!(
                ScribbleHub::story_id(&url).as_deref(),
                Some("2357420"),
                "{raw}"
            );
            assert!(adapter().can_handle(&url), "{raw} must be claimed");
        }

        // Addresses that name no story.
        for raw in [
            "https://www.scribblehub.com/",
            "https://www.scribblehub.com/series-ranking/",
            "https://www.scribblehub.com/profile/108709/drava/",
        ] {
            let url = Url::parse(raw).unwrap();
            assert!(!adapter().can_handle(&url), "{raw} must not be claimed");
        }
    }

    #[test]
    fn a_chapter_address_is_read_for_its_chapter_id() {
        let url = Url::parse(
            "https://www.scribblehub.com/read/2357420-worlds-cutest-alchemist/chapter/2357479/",
        )
        .unwrap();
        assert_eq!(ScribbleHub::chapter_id(&url).as_deref(), Some("2357479"));

        let series = Url::parse("https://www.scribblehub.com/series/2357420/x/").unwrap();
        assert_eq!(ScribbleHub::chapter_id(&series), None);
    }

    #[test]
    fn the_api_addresses_are_the_sites_own() {
        let adapter = adapter();
        assert_eq!(
            adapter.api_story("2357420"),
            "https://www.scribblehub.com/wp-json/fictionapp/v1/stories/2357420"
        );
        assert_eq!(
            adapter.api_chapters("2357420", 3),
            "https://www.scribblehub.com/wp-json/fictionapp/v1/stories/2357420/chapters?page=3"
        );
        assert_eq!(
            adapter.chapter_url("2357420", "2357420-worlds-cutest-alchemist", "2357479"),
            "https://www.scribblehub.com/read/2357420-worlds-cutest-alchemist/chapter/2357479/"
        );
        // Without a slug the id alone still addresses the chapter: the site
        // redirects the short form to the canonical one.
        assert_eq!(
            adapter.chapter_url("2357420", "", "2357479"),
            "https://www.scribblehub.com/read/2357420/chapter/2357479/"
        );
    }

    #[test]
    fn the_ordinal_is_the_sites_order_and_not_its_gapped_chapter_number() {
        // The recorded work runs 1, 3, 4, 5 … because chapter 2 was deleted.
        // The import's ordinal is dense; a gap would put a chapter in the wrong
        // place in a reader's progress.
        let chapters: Vec<ApiChapter> = (0..4)
            .map(|index| ApiChapter {
                id: 100 + index,
                title: format!("Chapter {}", index + 1),
                number: if index == 0 { 1 } else { (index as u32) + 2 },
                word_count: 0,
                published_at: None,
                content: String::new(),
            })
            .collect();

        let refs = adapter()
            .assemble_chapters(&[chapters], "2357420", 4)
            .expect("four chapters and a stated count of four");
        assert_eq!(
            refs.iter().map(|entry| entry.ordinal).collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
        assert_eq!(refs[1].source_chapter_key, "101");
        // The site's own number is not carried as the ordinal.
        assert_ne!(refs[1].ordinal, 3);
    }

    #[test]
    fn a_list_that_does_not_add_up_to_the_stated_count_is_refused() {
        let chapters: Vec<ApiChapter> = (0..3)
            .map(|index| ApiChapter {
                id: index,
                title: String::new(),
                number: 0,
                word_count: 0,
                published_at: None,
                content: String::new(),
            })
            .collect();

        // The story says 113; the pages produced 3. Importing the three would
        // import part of a work and report success.
        let error = adapter()
            .assemble_chapters(std::slice::from_ref(&chapters), "2357420", 113)
            .expect_err("a short list must not be imported");
        assert!(format!("{error}").contains("113"), "{error}");

        // An exact list of the stated size is fine.
        assert!(adapter()
            .assemble_chapters(&[chapters], "2357420", 3)
            .is_ok());
    }

    #[test]
    fn a_story_with_no_chapters_is_refused() {
        let error = adapter()
            .assemble_chapters(&[Vec::new()], "2357420", 0)
            .expect_err("a work with no chapters is not importable");
        assert!(format!("{error}").contains("no chapters"), "{error}");
    }

    #[test]
    fn the_status_strings_are_mapped_and_an_unseen_one_is_unknown() {
        assert_eq!(status_of("ongoing"), WorkStatus::Ongoing);
        assert_eq!(status_of("completed"), WorkStatus::Complete);
        // Neither value is evidence of the other, and a guess would put a
        // finished work in a reader's "in progress" list.
        for unseen in ["hiatus", "dropped", "cancelled", ""] {
            assert_eq!(status_of(unseen), WorkStatus::Unknown, "{unseen}");
        }
    }

    #[test]
    fn the_api_timestamps_are_utc_instants() {
        assert_eq!(
            parse_rfc3339(Some("2026-09-10T23:30:13+00:00")),
            Some(datetime!(2026-09-10 23:30:13 UTC))
        );
        // An offset is converted rather than kept.
        assert_eq!(
            parse_rfc3339(Some("2026-05-25T13:17:00+03:00")),
            Some(datetime!(2026-05-25 10:17:00 UTC))
        );
        assert_eq!(parse_rfc3339(None), None);
        assert_eq!(parse_rfc3339(Some("not a time")), None);
    }

    #[test]
    fn a_missing_story_is_not_found_rather_than_a_parse_failure() {
        // The API answers `404` with a `fa_story_not_found` envelope, which is a
        // different message from a page that arrived and was not understood.
        let error = adapter()
            .parse_story(
                r#"{"code":"fa_story_not_found","message":"Story not found.","data":{"status":404}}"#,
                "999999999",
            )
            .expect_err("a missing story is not a work");
        assert!(matches!(error, SourceError::NotFound), "{error}");
    }

    #[test]
    fn a_story_answered_for_another_id_is_refused() {
        // The prose and the metadata would otherwise be stored under the wrong
        // work, and a reader's library entry would follow it.
        let document = r#"{"data":{"id":1,"title":"x","author":{"id":2,"displayName":"a"}}}"#;
        let error = adapter()
            .parse_story(document, "2357420")
            .expect_err("a story for another id is not this story");
        assert!(format!("{error}").contains("2357420"), "{error}");
    }
}
