//! Pawchive — a Patreon fanfiction archive at <https://pawchive.pw>.
//!
//! # Recognised URLs
//!
//! ```text
//! https://pawchive.pw/patreon/user/{user_id}
//! ```
//!
//! # Capabilities
//!
//! Only `bibliography` (listing an author's posts) and `chapters` (reading
//! chapter text when the author pasted it) are available. Most Patreon
//! cross-posts on Pawchive are links-only (Deckreader, Google Docs); those
//! posts are skipped during a full import — they report `content_length: 0`
//! and no chapter text. The adapter cannot log into Patreon, so it reads
//! only what Pawchive's public JSON API returns.
//!
//! The adapter uses two endpoints:
//! * `GET /api/v1/patreon/user/{uid}/posts?limit=50&o={offset}` — list posts
//! * `GET /api/v1/patreon/user/{uid}/post/{pid}` — one post with navigation

use async_trait::async_trait;
use scraper::Html;
use url::Url;

use crate::{
    AuthKind, Credentials, Fetcher, SourceAdapter, SourceCapabilities, SourceChapter, SourceError,
    SourceKey, SourceResult, SourceWork, WorkStatus,
};

/// Hosts this adapter serves.
const HOSTS: [&str; 2] = ["pawchive.pw", "www.pawchive.pw"];

/// The Pawchive adapter.
#[derive(Debug, Clone)]
pub struct Pawchive {
    key: SourceKey,
    hosts: Vec<String>,
}

impl Default for Pawchive {
    fn default() -> Self {
        Self::new()
    }
}

impl Pawchive {
    /// The adapter and its hosts.
    #[must_use]
    pub fn new() -> Self {
        Self {
            key: SourceKey::new("pawchive"),
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

    /// The numeric user id in a `/patreon/user/{id}` path.
    fn user_id(url: &Url) -> Option<String> {
        let mut segments = url.path_segments()?;
        if segments.next()? != "patreon" {
            return None;
        }
        if segments.next()? != "user" {
            return None;
        }
        let id = segments.next()?;
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        Some(id.to_owned())
    }

    /// Build the JSON API URL for a user's posts page.
    fn api_user_posts_url(uid: &str, offset: u32) -> String {
        format!(
            "https://pawchive.pw/api/v1/patreon/user/{uid}/posts?limit=50&o={offset}"
        )
    }
}

/// One post from Pawchive's JSON API.
#[derive(Debug, serde::Deserialize)]
struct ApiPost {
    id: String,
    user: String,
    title: String,
    content: String,
    #[serde(default)]
    tags: serde_json::Value,
    #[serde(default)]
    published: Option<String>,
    #[serde(default)]
    attachments: serde_json::Value,
}

/// The JSON list response is just an array of posts.
type ApiPostList = Vec<ApiPost>;

#[async_trait]
impl SourceAdapter for Pawchive {
    fn key(&self) -> SourceKey {
        self.key.clone()
    }

    fn display_name(&self) -> &'static str {
        "Pawchive"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities {
            metadata: true,
            chapters: true,
            bibliography: true,
            per_chapter_fetch: false,
            incremental: false,
            authentication: AuthKind::None,
            min_interval_millis: None,
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
        let uid = Self::user_id(url).ok_or_else(|| {
            SourceError::Unsupported(format!("{url} is not a Pawchive user URL"))
        })?;

        // Fetch first page of posts to count them and pick up metadata.
        let api_url = Self::api_user_posts_url(&uid, 0);
        let page = fetch
            .get(&api_url)
            .await
            .map_err(|e| SourceError::Network(format!("listing posts: {e}")))?;
        let posts: ApiPostList = serde_json::from_str(&page.body).map_err(|e| {
            SourceError::Parse(format!("Pawchive API returned invalid JSON: {e}"))
        })?;

        if posts.is_empty() {
            return Err(SourceError::NotFound);
        }

        // Count posts with real content (chapters) vs links-only.
        let mut chapters = Vec::new();
        for (ordinal, post) in posts.iter().enumerate() {
            let text = strip_tags(&post.content);
            let text = text.trim().to_owned();
            // Only include posts that have meaningful chapter text.
            if text.len() > 50 {
                chapters.push(crate::ChapterRef {
                    ordinal: ordinal as u32 + 1,
                    source_chapter_key: post.id.clone(),
                    title: post.title.clone(),
                });
            }
        }

        Ok(SourceWork {
            source_key: self.key.clone(),
            source_work_key: uid.clone(),
            source_url: url.to_string(),
            title: format!("Pawchive user {uid}"),
            author_text: uid.clone(),
            author_url: Some(url.to_string()),
            summary: format!("{} posts on Pawchive", posts.len()),
            word_count: None,
            language: Some("en".to_owned()),
            status: WorkStatus::Unknown,
            published_at: None,
            updated_at: None,
            chapters,
            rating_text: None,
            warning_texts: Vec::new(),
            tags: Vec::new(),
        })
    }

    async fn fetch_chapters(
        &self,
        fetch: &dyn Fetcher,
        work: &SourceWork,
        _creds: Option<&Credentials>,
    ) -> SourceResult<Vec<SourceChapter>> {
        let uid = &work.source_work_key;

        // Paginate through all posts and yield those with chapter text.
        let mut chapters = Vec::new();
        let mut offset = 0;
        loop {
            let api_url = Self::api_user_posts_url(uid, offset);
            let page = fetch
                .get(&api_url)
                .await
                .map_err(|e| SourceError::Network(format!("listing posts: {e}")))?;
            let posts: ApiPostList = serde_json::from_str(&page.body).map_err(|e| {
                SourceError::Parse(format!("Pawchive API returned invalid JSON: {e}"))
            })?;

            if posts.is_empty() {
                break;
            }

            for post in &posts {
                let text = strip_tags(&post.content);
                let text = collapse_whitespace(&text);
                if text.len() > 50 {
                    chapters.push(SourceChapter {
                        ordinal: chapters.len() as u32 + 1,
                        source_chapter_key: post.id.clone(),
                        title: post.title.clone(),
                        content_html: text,
                    });
                }
            }

            if posts.len() < 50 {
                break;
            }
            offset += 50;
        }

        Ok(chapters)
    }

    async fn list_author_works(
        &self,
        _fetch: &dyn Fetcher,
        _profile_url: &Url,
        _creds: Option<&Credentials>,
    ) -> SourceResult<Vec<String>> {
        // Pawchive does not expose a "recommended" JSON API, so the caller
        // must use the HTML scraping path if they want similar authors.
        Err(SourceError::Unsupported(
            "Pawchive does not have a native similar-authors API".to_owned(),
        ))
    }

    fn preview_from_html(&self, html: &str, url: &Url) -> SourceResult<SourceWork> {
        let _ = (html, url);
        Err(SourceError::Unsupported(
            "Pawchive has no HTML preview entry point".to_owned(),
        ))
    }

    fn chapters_from_html(
        &self,
        html: &str,
        work: &SourceWork,
    ) -> SourceResult<Vec<SourceChapter>> {
        let _ = (html, work);
        Err(SourceError::Unsupported(
            "Pawchive has no HTML chapters entry point".to_owned(),
        ))
    }
}

/// Strip HTML tags from a fragment, returning plain text.
fn strip_tags(html: &str) -> String {
    let document = Html::parse_fragment(html);
    document.root_element().text().collect::<Vec<_>>().join(" ")
}

/// Collapse internal whitespace to a single string.
fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
