//! Wattpad: metadata the site will serve plainly, prose it asks us not to read.
//!
//! # The two halves, and why they are different
//!
//! Wattpad answers two different questions on two different paths, and its
//! `robots.txt` treats them differently:
//!
//! * **What a work is** comes from `https://www.wattpad.com/api/v3/stories/{id}`,
//!   which returns JSON: the title, the author, the description, the language,
//!   the timestamps, the tags, whether the author marked it complete, and the
//!   full list of parts with their ids and titles. That path is **permitted** —
//!   the site's `robots.txt` disallows `/apiv2/*`, `/docs/*`, `/fonts/*`,
//!   `/dialog/*`, `/workers/*`, `/rss/*`, `/clubs/*` and the personal pages,
//!   and `/api/v3/` is on none of those — so a preview of a Wattpad work needs
//!   no allowance from anybody.
//! * **What a chapter says** comes from
//!   `https://www.wattpad.com/apiv2/?m=storytext&id={part}&page=`, which returns
//!   the prose. That path is **disallowed by name**: `Disallow: /apiv2/*`, read
//!   from the site's own file on 2026-09-11 and recorded at
//!   `tests/fixtures/wattpad/robots.txt`.
//!
//! So this adapter is one where an instance's default behaviour differs from its
//! reachable behaviour, and it says so rather than pretending otherwise. Under
//! the default configuration the chapter fetch is refused by the shared fetcher
//! with a message that names the rule, which is spec §11.5's requirement that a
//! restriction be reported as a restriction rather than as a parse failure.
//! Under `imports.honour_robots = false` — the operator's own decision, made on
//! their own instance — the same code reads the prose with no change to this
//! module. That is the whole point of putting the answer in configuration: the
//! adapter states what the source has, and the operator states what their
//! instance will do about it.
//!
//! # The prose endpoint, measured
//!
//! * **`page=` empty is the whole chapter.** Verified on 2026-09-11 against two
//!   parts: the empty form is byte-identical to the concatenation of the
//!   numbered pages (`page=1`, `page=2`, … until empty). A part that the reader
//!   app splits into five screens therefore costs one request here, and an
//!   adapter that walked `page=1..n` would spend five to get the same bytes.
//! * **The response is HTML fragment**: `<p>` elements, each carrying a
//!   `data-p-id` and sometimes `data-media-type`. The ids are the site's
//!   paragraph anchors, not content, and the sanitiser drops them.
//! * **A part may be media rather than prose.** The recorded work's first three
//!   parts are `Photo Gallery`, `Playlist` and `Cast` — 54, 865 and 998
//!   characters of the same `<p>` markup, one of them holding an `<img>`. They
//!   are parts the reader is shown, so they are chapters here for the same reason
//!   Royal Road's announcement posts are: the site lists them, and an adapter
//!   that silently dropped them would report a work as shorter than its author
//!   published it. The sanitizer removes the images, which is what makes them
//!   readable as the text they contain.
//! * **The endpoint may gzip regardless of what the request asked for.**
//!   Measured twice on 2026-09-11: `content-encoding: gzip` in answer to an
//!   explicit `Accept-Encoding: identity`, and plain bytes on a later request to
//!   the same URL — the answer varies by edge. It is therefore not something
//!   this adapter can predict, and it is not handled here: the shared fetcher
//!   undoes a declared compression before any adapter sees the body
//!   ([`crate::safety::decode_response_body`]). Without that, a
//!   gzipped chapter would be stored as mojibake and the import would report
//!   success.
//!
//! # What the site does not publish
//!
//! **A word count.** `length` on a story and on a part is a *character* count —
//! the recorded work's first chapter is `length: 18380` against a page that
//! states `wordCount: 3676`, a ratio of five, which is what a character count
//! looks like. Per-part word counts exist on the part's own page and are not in
//! the story document, so summing them would cost one request per part at
//! preview time. This adapter therefore reports **no word count at all**, which
//! is the honest answer: passing 291,689 off as a word count would be wrong by a
//! factor of five in the direction a reader would not check.
//!
//! **Category names.** `categories` is a list of integers (`[6, 0]`) with no
//! published table behind it. They are dropped rather than guessed at; the tags
//! the author chose are carried, because those are text.
//!
//! # Drafts
//!
//! A part carries `draft: false`. An unauthenticated read only ever sees
//! published parts, so this should never be `true` — but a part the author has
//! not published is not part of the public work, and including one would publish
//! it further. Drafts are excluded and the count they remove is reported rather
//! than absorbed.
//!
//! # Provenance
//!
//! Written against pages and JSON recorded from the live site on 2026-09-11; see
//! `tests/fixtures/wattpad/` and the `## wattpad` section of
//! `tests/fixtures/README.md`.

use async_trait::async_trait;
use serde::Deserialize;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use url::Url;

use crate::sanitize::sanitize_fragment;
use crate::{
    AuthKind, ChapterRef, Credentials, Fetcher, SourceAdapter, SourceCapabilities, SourceChapter,
    SourceError, SourceKey, SourceResult, SourceWork, WorkStatus,
};

/// The host, without `www.`.
pub const HOST: &str = "wattpad.com";

/// The registry key.
pub const SOURCE_KEY: &str = "wattpad";

/// The site's own pace.
///
/// Wattpad's `robots.txt` publishes **no** `Crawl-delay`, so the fetcher's
/// one-second floor is the pace and this is the courtesy value beneath it. Left
/// at the floor rather than raised: a site that published nothing has asked for
/// nothing, and inventing a slower number here would be a guess that goes stale
/// in the same way an invented faster one would.
pub const PACING_MILLIS: u64 = 1_000;

/// One Wattpad work.
#[derive(Debug, Clone)]
pub struct Wattpad {
    key: SourceKey,
    hosts: Vec<String>,
}

impl Default for Wattpad {
    fn default() -> Self {
        Self::new()
    }
}

impl Wattpad {
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

    /// The numeric story id in a `/story/{id}-{slug}` path.
    ///
    /// The slug is decorative and the id is not: the site serves the same work
    /// with or without it, so it is deliberately not part of the key.
    fn story_id(url: &Url) -> Option<String> {
        let mut segments = url.path_segments()?;
        if segments.next()? != "story" {
            return None;
        }
        let first = segments.next()?;
        let id: String = first.chars().take_while(char::is_ascii_digit).collect();
        (!id.is_empty()).then_some(id)
    }

    /// The numeric part id in a `/{id}-{slug}` path.
    ///
    /// A part's address is the id at the site root, so this is the read URL for
    /// a chapter rather than for a work.
    fn part_id(url: &Url) -> Option<String> {
        let first = url.path_segments()?.next()?;
        let id: String = first.chars().take_while(char::is_ascii_digit).collect();
        (!id.is_empty()).then_some(id)
    }

    /// The canonical work address, which needs no slug.
    fn work_url(&self, story_id: &str) -> String {
        format!("https://www.{HOST}/story/{story_id}")
    }

    /// The site's own JSON for a work.
    fn story_api(&self, story_id: &str) -> String {
        format!("https://www.{HOST}/api/v3/stories/{story_id}")
    }

    /// The site's prose endpoint for one part.
    ///
    /// `page=` is left empty on purpose: measured, that is the whole chapter
    /// rather than its first screen, and it is byte-identical to the numbered
    /// pages joined.
    fn text_url(&self, part_id: &str) -> String {
        format!("https://www.{HOST}/apiv2/?m=storytext&id={part_id}&page=")
    }

    /// Read a work's metadata out of the site's JSON.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::NotFound`] for the site's own `1017 NotFound`
    /// answer, which arrives with HTTP `400` and is therefore not visible in the
    /// status code. Anything else that does not parse is a parse failure, kept
    /// distinct because a shape that changed must not be reported as a work that
    /// was deleted.
    pub fn parse_story(&self, json: &str, story_id: &str) -> SourceResult<SourceWork> {
        let payload: StoryPayload = serde_json::from_str(json).map_err(|error| {
            // The site reports a missing story as a JSON *error* body with a
            // 400, so the not-found case is here rather than in a status check.
            if let Ok(error) = serde_json::from_str::<ApiError>(json) {
                if error.error_type == "NotFound" {
                    return SourceError::NotFound;
                }
            }
            SourceError::Parse(format!("wattpad story {story_id}: {error}"))
        })?;

        // The document states its own id and its own part count. Both are
        // compared against what was read, because a story served under the wrong
        // id would attach a work to somebody else's row, and a part list that is
        // shorter than the count is a list that does not describe the work.
        if payload.id != story_id {
            return Err(SourceError::Parse(format!(
                "wattpad answered for story {} when {story_id} was asked for",
                payload.id
            )));
        }

        let published: Vec<&Part> = payload.parts.iter().filter(|part| !part.draft).collect();
        let drafts = payload.parts.len() - published.len();
        if drafts > 0 {
            tracing::warn!(
                story = story_id,
                drafts,
                "wattpad returned parts the author has not published; they are excluded"
            );
        }
        if published.len() != payload.num_parts {
            return Err(SourceError::Parse(format!(
                "wattpad story {story_id} states {} parts and lists {}",
                payload.num_parts,
                published.len()
            )));
        }

        let chapters: Vec<ChapterRef> = published
            .iter()
            .enumerate()
            .map(|(index, part)| ChapterRef {
                ordinal: (index as u32) + 1,
                // The site's own id for the part, which is what its prose
                // endpoint takes as a parameter. It is stable across edits, so
                // it is the key rather than the position.
                source_chapter_key: part.id.to_string(),
                title: part.title.clone(),
            })
            .collect();

        // Read before the struct so the construction below stays a list of
        // fields rather than a chain with a fix-up on the end.
        let published_at = payload.create_date.as_deref().and_then(rfc3339);
        let updated_at = payload.modify_date.as_deref().and_then(rfc3339);

        Ok(SourceWork {
            source_key: self.key.clone(),
            source_work_key: story_id.to_owned(),
            source_url: self.work_url(story_id),
            title: payload.title,
            author_text: payload.user.name.clone(),
            // Built rather than read: the story document names the author and
            // does not link them, and `/user/{name}` is the address the site's
            // own markup uses for the same author (read off the work page's
            // structured block on 2026-09-11).
            author_url: Some(format!("https://www.{HOST}/user/{}", payload.user.name)),
            summary: payload.description,
            // See the module documentation: the site publishes characters, not
            // words, and per-part word counts that would cost a request each.
            word_count: None,
            language: payload.language.map(|language| language.name),
            status: if payload.completed {
                WorkStatus::Complete
            } else {
                // The site's flag is "the author marked it complete", so its
                // absence is a work still being added to — and unlike
                // FanFiction.net's `Status:` field, this one is always present,
                // which is what makes reading the negative here the truth
                // rather than an assumption.
                WorkStatus::Ongoing
            },
            published_at,
            updated_at,
            chapters,
            rating_text: None,
            warning_texts: Vec::new(),
            tags: payload.tags,
        })
    }

    /// Read one part's prose from the fragment the text endpoint returns.
    ///
    /// # Why this is not [`SourceAdapter::chapters_from_html`]
    ///
    /// Because a Wattpad chapter document **cannot identify itself**. The prose
    /// endpoint is addressed by the *chapter's* id, so the response is a
    /// fragment with no marker saying which part it is — unlike a FanFiction.net
    /// chapter page, whose `selected` option states its own position. The caller
    /// is the only party that knows, so the ordinal is a parameter here and the
    /// trait's `chapters_from_html` is deliberately left at its default rather
    /// than given a guess. The fixture tests drive this method directly.
    #[must_use]
    pub fn chapter_from_text(
        &self,
        text: &str,
        work: &SourceWork,
        ordinal: u32,
        base: Option<&Url>,
    ) -> SourceChapter {
        let chapter = work.chapters.iter().find(|entry| entry.ordinal == ordinal);
        SourceChapter {
            ordinal,
            source_chapter_key: chapter
                .map(|entry| entry.source_chapter_key.clone())
                .unwrap_or_else(|| ordinal.to_string()),
            title: chapter.map(|entry| entry.title.clone()).unwrap_or_default(),
            content_html: sanitize_fragment(text, base),
            image_urls: crate::sanitize::extract_image_urls(text, base),
        }
    }
}

#[async_trait]
impl SourceAdapter for Wattpad {
    fn key(&self) -> SourceKey {
        self.key.clone()
    }

    fn display_name(&self) -> &'static str {
        "Wattpad"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities {
            metadata: true,
            // True as a statement about the *source*, which is what this field
            // means: the site serves every part's prose and this adapter reads
            // it. Whether a given instance may fetch it is the instance's
            // `imports.honour_robots`, answered per instance rather than
            // compiled into an adapter — and reporting `false` here would tell a
            // compliant instance that chapters are impossible, which is a
            // different claim and a wrong one.
            chapters: true,
            // One part is one request to an endpoint addressed by the part's own
            // id, so a retry re-reads exactly the chapter that failed.
            per_chapter_fetch: true,
            // Author profiles exist; their markup has not been recorded.
            bibliography: false,
            // The story document carries `modifyDate`, so an update check costs
            // one request.
            incremental: true,
            authentication: AuthKind::None,
            min_interval_millis: Some(PACING_MILLIS),
        }
    }

    fn can_handle(&self, url: &Url) -> bool {
        let Some(host) = url.host_str() else {
            return false;
        };
        self.host_matches(host)
            && (Wattpad::story_id(url).is_some() || Wattpad::part_id(url).is_some())
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
        // A part URL is resolved to its work first, which needs the part's own
        // metadata for the `groupId` it belongs to. That costs one extra request
        // and is the only way a pasted chapter address can preview a work.
        let story_id = match Wattpad::story_id(url) {
            Some(id) => id,
            None => {
                let part_id = Wattpad::part_id(url).ok_or_else(|| {
                    SourceError::Unsupported(format!("{url} is not a Wattpad story or part"))
                })?;
                let page = fetch
                    .get(&format!("https://www.{HOST}/api/v3/story_parts/{part_id}"))
                    .await?;
                let part: PartDetail = serde_json::from_str(&page.body).map_err(|error| {
                    SourceError::Parse(format!("wattpad part {part_id}: {error}"))
                })?;
                part.group_id
            }
        };

        let page = fetch.get(&self.story_api(&story_id)).await?;
        self.parse_story(&page.body, &story_id)
    }

    async fn fetch_chapters(
        &self,
        fetch: &dyn Fetcher,
        work: &SourceWork,
        _creds: Option<&Credentials>,
    ) -> SourceResult<Vec<SourceChapter>> {
        if work.chapters.is_empty() {
            return Err(SourceError::Parse(format!(
                "wattpad work {} lists no parts",
                work.source_work_key
            )));
        }

        let mut chapters = Vec::with_capacity(work.chapters.len());
        for entry in &work.chapters {
            chapters.push(self.fetch_chapter(fetch, work, entry.ordinal, None).await?);
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
                    "wattpad work {} has no part {ordinal}",
                    work.source_work_key
                ))
            })?;

        let target = self.text_url(&entry.source_chapter_key);
        let page = fetch.get(&target).await?;
        // The fragment is relative-free but sanitised against the canonical work
        // URL anyway, so a link that does appear resolves to the site rather
        // than being stored as a bare path.
        let base = Url::parse(&work.source_url).ok();
        Ok(self.chapter_from_text(&page.body, work, ordinal, base.as_ref()))
    }

    fn preview_from_html(&self, html: &str, url: &Url) -> SourceResult<SourceWork> {
        // The fixture seam. `html` is the site's JSON, because that is what a
        // preview reads — see the module documentation for why the story
        // *document* rather than the story *page* is the source of metadata.
        let story_id = Wattpad::story_id(url).ok_or_else(|| {
            SourceError::Unsupported(format!("{url} is not a Wattpad story address"))
        })?;
        self.parse_story(html, &story_id)
    }
}

/// A work, as `/api/v3/stories/{id}` describes it.
///
/// Only the fields this adapter reads. The site returns more — a cover, vote and
/// comment counts, a read count, a copyright flag, a `readerBrowseEligibility`
/// block — and none of it is stored here, so none of it is deserialised. A
/// struct that named every field would break the day the site added one; this
/// one only breaks when a field it *uses* changes.
#[derive(Debug, Deserialize)]
struct StoryPayload {
    id: String,
    title: String,
    #[serde(default)]
    description: String,
    #[serde(rename = "createDate", default)]
    create_date: Option<String>,
    #[serde(rename = "modifyDate", default)]
    modify_date: Option<String>,
    #[serde(default)]
    completed: bool,
    #[serde(default)]
    language: Option<Language>,
    #[serde(default)]
    user: Author,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(rename = "numParts", default)]
    num_parts: usize,
    #[serde(default)]
    parts: Vec<Part>,
}

/// The author, as the story document names them.
#[derive(Debug, Default, Deserialize)]
struct Author {
    #[serde(default)]
    name: String,
}

/// The language, as `{"id": 1, "name": "English"}`.
#[derive(Debug, Deserialize)]
struct Language {
    name: String,
}

/// One part of a work.
#[derive(Debug, Deserialize)]
struct Part {
    /// The site's id for the part, and the parameter its prose endpoint takes.
    id: u64,
    #[serde(default)]
    title: String,
    /// Whether the author has published this part. See the module docs.
    #[serde(default)]
    draft: bool,
}

/// A part, as `/api/v3/story_parts/{id}` describes it.
#[derive(Debug, Deserialize)]
struct PartDetail {
    /// The work this part belongs to.
    #[serde(rename = "groupId")]
    group_id: String,
}

/// The site's own error body.
#[derive(Debug, Deserialize)]
struct ApiError {
    #[serde(rename = "error_type", default)]
    error_type: String,
}

/// Parse an RFC 3339 timestamp.
///
/// The site writes `2026-04-22T02:39:18Z` — an explicit zone, which is why it is
/// read rather than guessed at. A shape that does not parse yields `None`, the
/// honest answer for a timestamp this build does not understand.
fn rfc3339(raw: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(raw, &Rfc3339).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter() -> Wattpad {
        Wattpad::new()
    }

    #[test]
    fn a_story_address_is_read_for_its_id_with_or_without_a_slug() {
        let with = Url::parse("https://www.wattpad.com/story/410445604-the-older-swan-paul-lahote")
            .unwrap();
        let without = Url::parse("https://www.wattpad.com/story/410445604").unwrap();
        assert_eq!(Wattpad::story_id(&with).as_deref(), Some("410445604"));
        assert_eq!(Wattpad::story_id(&without).as_deref(), Some("410445604"));
    }

    #[test]
    fn a_part_address_is_read_for_its_id() {
        let url =
            Url::parse("https://www.wattpad.com/1623966332-the-older-swan-paul-lahote-chapter-one")
                .unwrap();
        assert_eq!(Wattpad::part_id(&url).as_deref(), Some("1623966332"));
        assert!(adapter().can_handle(&url), "a part address is claimable");

        // A user profile is a bare slug and is not a part.
        let profile = Url::parse("https://www.wattpad.com/user/love-yourself-xoxo").unwrap();
        assert_eq!(Wattpad::part_id(&profile), None);
        assert!(!adapter().can_handle(&profile));
    }

    #[test]
    fn the_addresses_carry_no_slug_and_the_text_url_asks_for_the_whole_chapter() {
        let adapter = adapter();
        assert_eq!(
            adapter.work_url("410445604"),
            "https://www.wattpad.com/story/410445604"
        );
        // `page=` empty rather than `page=1`: measured, the empty form is the
        // whole chapter and the numbered pages are its screens.
        assert_eq!(
            adapter.text_url("1623966332"),
            "https://www.wattpad.com/apiv2/?m=storytext&id=1623966332&page="
        );
    }

    #[test]
    fn a_timestamp_is_read_as_the_zone_it_states() {
        assert_eq!(
            rfc3339("2026-04-22T02:39:18Z"),
            Some(time::macros::datetime!(2026-04-22 02:39:18 UTC))
        );
        assert_eq!(rfc3339("not a date"), None);
    }

    #[test]
    fn only_the_host_this_adapter_reads_is_claimed() {
        assert!(adapter().can_handle(&Url::parse("https://wattpad.com/story/1").unwrap()));
        assert!(adapter().can_handle(&Url::parse("https://www.wattpad.com/story/1").unwrap()));
        assert!(!adapter().can_handle(&Url::parse("https://example.com/story/1").unwrap()));
    }
}
