//! CHYOA: an interactive, branching fiction platform.
//!
//! # Recognised URLs
//!
//! ```text
//! https://chyoa.com/story/{slug}.{id}
//! https://chyoa.com/chapter/{slug}.{id}
//! ```
//!
//! Both preview the work, because the story id is the work and the chapter page
//! says which story it belongs to. A chapter URL is resolved to its parent story
//! by following the breadcrumb rather than by guessing the path.
//!
//! # Capabilities
//!
//! Metadata, chapters and per-chapter fetch are available. Bibliography is not:
//! an author's story list is behind a search that this adapter does not run.
//!
//! The chapter list is the story's **branches** page — a tree of reading paths.
//! It is rendered server-side, which is what makes this source scrapable where
//! an SPA is not. The prose is in `div.rd-story-prose.chapter-content` on the
//! chapter page, also server-rendered.
//!
//! # What this adapter does not do
//!
//! It does not authenticate. A reader must be signed in to see some works, and
//! a gated work reports [`SourceError::AuthRequired`] rather than guessing past
//! the gate.

use async_trait::async_trait;
use scraper::{Html, Selector};
use url::Url;

use crate::{
    attr_of, collapse_whitespace, text_of, AuthKind, ChapterRef, Credentials, Fetcher,
    SourceAdapter, SourceCapabilities, SourceChapter, SourceError, SourceKey, SourceResult,
    SourceWork, WorkStatus,
};

/// Hosts this adapter serves.
const HOSTS: [&str; 2] = ["chyoa.com", "www.chyoa.com"];

/// The CHYOA adapter.
#[derive(Debug, Clone)]
pub struct Chyoa {
    key: SourceKey,
    hosts: Vec<String>,
}

impl Default for Chyoa {
    fn default() -> Self {
        Self::new()
    }
}

impl Chyoa {
    /// The adapter and its hosts.
    #[must_use]
    pub fn new() -> Self {
        Self {
            key: SourceKey::new("chyoa"),
            hosts: HOSTS.iter().map(|h| (*h).to_owned()).collect(),
        }
    }

    /// The hosts, for the fetcher's allow-list.
    #[must_use]
    pub fn hosts(&self) -> Vec<String> {
        self.hosts.clone()
    }

    /// Whether a URL is one of this adapter's hosts.
    fn host_matches(&self, host: &str) -> bool {
        let host = host.trim_start_matches("www.");
        self.hosts.iter().any(|ours| host == ours)
    }

    /// The story id from a `/story/{slug}.{id}` path, or `None` if the URL is
    /// not a story page.
    fn story_id(url: &Url) -> Option<String> {
        let mut segments = url.path_segments()?;
        if segments.next()? != "story" {
            return None;
        }
        let slug_id = segments.next()?;
        // Must end with a numeric id after the last dot.
        let (slug, id) = slug_id.rsplit_once('.')?;
        if slug.is_empty() || id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        Some(slug_id.to_owned())
    }

    /// The chapter id from a `/chapter/{slug}.{id}` path, or `None` if the URL
    /// is not a chapter page.
    fn chapter_id(url: &Url) -> Option<String> {
        let mut segments = url.path_segments()?;
        if segments.next()? != "chapter" {
            return None;
        }
        let slug_id = segments.next()?;
        let (slug, id) = slug_id.rsplit_once('.')?;
        if slug.is_empty() || id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        Some(slug_id.to_owned())
    }

    /// The story URL for a given story id.
    fn story_url(id: &str) -> String {
        format!("https://chyoa.com/story/{id}")
    }

    /// The chapter URL for a given chapter id.
    fn chapter_url(id: &str) -> String {
        format!("https://chyoa.com/chapter/{id}")
    }
}

#[async_trait]
impl SourceAdapter for Chyoa {
    fn key(&self) -> SourceKey {
        self.key.clone()
    }

    fn display_name(&self) -> &'static str {
        "CHYOA"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities {
            metadata: true,
            chapters: true,
            bibliography: false,
            per_chapter_fetch: true,
            incremental: false,
            authentication: AuthKind::None,
            min_interval_millis: Some(1500),
        }
    }

    fn can_handle(&self, url: &Url) -> bool {
        url.host_str()
            .map(|h| self.host_matches(h))
            .unwrap_or(false)
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
        // If given a chapter URL, resolve it to the story via the breadcrumb.
        let story_slug_id = if let Some(id) = Self::story_id(url) {
            id
        } else if Self::chapter_id(url).is_some() {
            let page = fetch
                .get(url.as_str())
                .await
                .map_err(|e| SourceError::Network(format!("reading chapter page: {e}")))?;
            let document = Html::parse_document(&page.body);
            // The breadcrumb on a chapter page links back to the story.
            let breadcrumb_sel =
                Selector::parse("a[href^='/story/']").expect("static selector");
            document
                .select(&breadcrumb_sel)
                .next()
                .and_then(|a| a.value().attr("href"))
                .and_then(|href| href.strip_prefix("/story/"))
                .map(|s| s.to_owned())
                .ok_or_else(|| {
                    SourceError::Parse("chapter page has no breadcrumb to its story".to_owned())
                })?
        } else {
            return Err(SourceError::Unsupported(format!(
                "{url} is not a CHYOA story or chapter URL"
            )));
        };

        let story_html = fetch
            .get(&Self::story_url(&story_slug_id))
            .await
            .map_err(|e| SourceError::Network(format!("reading story page: {e}")))?;
        self.preview_from_html(&story_html.body, url)
    }

    async fn fetch_chapters(
        &self,
        fetch: &dyn Fetcher,
        work: &SourceWork,
        _creds: Option<&Credentials>,
    ) -> SourceResult<Vec<SourceChapter>> {
        if work.chapters.is_empty() {
            return Err(SourceError::Parse(format!(
                "chyoa story {} lists no chapters",
                work.source_work_key
            )));
        }
        let mut chapters = Vec::with_capacity(work.chapters.len());
        for entry in &work.chapters {
            let page = fetch
                .get(&entry.source_chapter_key)
                .await
                .map_err(|e| SourceError::Network(format!("reading chapter {}: {e}", entry.ordinal)))?;
            chapters.push(self.parse_chapter(&page.body, work, entry.ordinal)?);
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
            .find(|e| e.ordinal == ordinal)
            .ok_or_else(|| {
                SourceError::Unsupported(format!(
                    "chyoa story {} has no chapter {ordinal}",
                    work.source_work_key
                ))
            })?;
        let page = fetch
            .get(&entry.source_chapter_key)
            .await
            .map_err(|e| SourceError::Network(format!("reading chapter {ordinal}: {e}")))?;
        self.parse_chapter(&page.body, work, ordinal)
    }

    fn preview_from_html(&self, html: &str, url: &Url) -> SourceResult<SourceWork> {
        let document = Html::parse_document(html);

        let title = text_of(&document, "h1.rd-story-reading-title").unwrap_or_default();
        if title.is_empty() {
            return Err(SourceError::NotFound);
        }

        let summary = attr_of(&document, "meta[name='description']", "content")
            .or_else(|| attr_of(&document, "meta[property='og:description']", "content"))
            .unwrap_or_default();

        // Author: the story owner link on the contributors section.
        let (author_text, author_url) = {
            let owner_sel = Selector::parse(
                ".story-contributors__group:first-of-type .story-contributors__avatar[href]",
            )
            .expect("static selector");
            document
                .select(&owner_sel)
                .next()
                .map(|a| {
                    let name = a
                        .value()
                        .attr("title")
                        .unwrap_or("")
                        .split(' ')
                        .next()
                        .unwrap_or("")
                        .to_owned();
                    let profile = format!(
                        "https://chyoa.com{}",
                        a.value().attr("href").unwrap_or("")
                    );
                    (name, Some(profile))
                })
                .unwrap_or((String::new(), None))
        };

        // Tags: anchors whose href matches `/story/{id}/tag/{tag}`.
        let tag_sel = Selector::parse("a[href*='/tag/']").expect("static selector");
        let tags = document
            .select(&tag_sel)
            .filter_map(|a| {
                let href = a.value().attr("href").unwrap_or("");
                href.rsplit_once("/tag/")
                    .map(|(_, tag)| urlencoding::decode(tag).unwrap_or_default().into_owned())
            })
            .collect::<Vec<_>>();

        // Chapters: the branch list. Each is an anchor in `.rd-story-branches
        // .rd-story-branch-link`. The href is `/chapter/{slug}.{id}` and the
        // text is the chapter title.
        let chapter_sel =
            Selector::parse(".rd-story-branches .rd-story-branch-link[href*='/chapter/']")
                .expect("static selector");
        let mut chapters = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (_i, a) in document.select(&chapter_sel).enumerate() {
            let href = a.value().attr("href").unwrap_or("");
            let Some((_base, chap_id)) = href.rsplit_once("/chapter/") else {
                continue;
            };
            // Dedupe: the same chapter may appear in multiple branch paths.
            if !seen.insert(chap_id.to_owned()) {
                continue;
            }
            let title = collapse_whitespace(&a.text().collect::<String>());
            chapters.push(ChapterRef {
                ordinal: (chapters.len() + 1) as u32,
                source_chapter_key: Self::chapter_url(chap_id),
                title,
            });
        }

        // If no branch links found, check if the URL is itself a chapter page
        // and create a single-chapter list for it.
        if chapters.is_empty() {
            if let Some(chap_id) = Self::chapter_id(url) {
                chapters.push(ChapterRef {
                    ordinal: 1,
                    source_chapter_key: Self::chapter_url(&chap_id),
                    title: title.clone(),
                });
            }
        }

        Ok(SourceWork {
            source_key: self.key.clone(),
            source_work_key: Self::story_id(url).unwrap_or_default(),
            source_url: url.to_string(),
            title,
            author_text,
            author_url,
            summary,
            word_count: None,
            language: Some("en".to_owned()),
            status: WorkStatus::Unknown,
            published_at: None,
            updated_at: None,
            chapters,
            rating_text: None,
            warning_texts: Vec::new(),
            tags,
        })
    }

    fn chapters_from_html(
        &self,
        html: &str,
        work: &SourceWork,
    ) -> SourceResult<Vec<SourceChapter>> {
        // When given a chapter page, parse its body.
        if work.source_url.contains("/chapter/") {
            let chapter = self.parse_chapter(html, work, 1)?;
            return Ok(vec![chapter]);
        }
        Err(SourceError::Unsupported(
            "chyoa chapters_from_html requires a chapter fixture (use the chapter URL as the work URL)".to_owned(),
        ))
    }
}

impl Chyoa {
    /// Parse a chapter body out of a chapter page's HTML.
    fn parse_chapter(
        &self,
        html: &str,
        work: &SourceWork,
        ordinal: u32,
    ) -> SourceResult<SourceChapter> {
        let document = Html::parse_document(html);

        let body_sel = Selector::parse("div.rd-story-prose.chapter-content")
            .expect("static selector");
        let body = document
            .select(&body_sel)
            .next()
            .ok_or_else(|| SourceError::Parse("chyoa chapter page has no prose container".to_owned()))?;

        let content_html = body.inner_html();
        let title = text_of(&document, "h1.rd-story-reading-title").unwrap_or_default();

        Ok(SourceChapter {
            ordinal,
            source_chapter_key: Self::chapter_url(&work.source_work_key),
            title,
            content_html,
        })
    }
}
