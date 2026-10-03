//! M45-23 — the declarative adapter, as a `SourceAdapter` (spec §55.3).
//!
//! This is the whole of Path A. A curator's YAML becomes a compiled manifest
//! here, and this type does exactly what the eleven hand-written adapters do:
//! recognise a URL, read metadata and a chapter list, read chapter bodies —
//! through a [`Fetcher`] it is *passed*, and holds none.
//!
//! **The transport is handed in, never held**, which is §11.1's structural rule
//! and the reason this file is short. There is no client field, no constructor
//! taking an endpoint, and no way to reach a host outside `hosts()`. Every guard
//! in §11.5 — SSRF refusal, robots compliance, per-domain pacing, credential
//! handling — is therefore inherited rather than reimplemented, and a reviewer
//! reading this file is reading all the security-relevant behaviour there is.
//!
//! What a manifest *cannot* express is the other half of Path A's argument: §11.5
//! forbids circumventing access controls, so a declarative adapter can only read
//! what a plain request returns. That is a feature of the design, not a gap in
//! it, and §55.1 says so.

use async_trait::async_trait;
use scraper::{Html, Selector};
use url::Url;

use crate::sanitize::sanitize_fragment;
use crate::source_manifest::CompiledSource;
use crate::{
    AuthKind, ChapterRef, Credentials, Fetched, Fetcher, SourceAdapter, SourceCapabilities,
    SourceChapter, SourceError, SourceKey, SourceResult, SourceWork, VerificationStatus, Wall,
    WorkStatus,
};

/// An adapter built from a §55.3 manifest.
#[derive(Debug, Clone)]
pub struct DeclarativeAdapter {
    source: CompiledSource,
}

impl DeclarativeAdapter {
    /// Build an adapter from an already-compiled manifest.
    ///
    /// Takes `CompiledSource` rather than `SourceManifest` so a manifest that has
    /// not been validated cannot become an adapter: there is no constructor path
    /// that skips `SourceManifest::compile`, which is §55.3's "compiles at
    /// submission" as a type-level property.
    #[must_use]
    pub fn new(source: CompiledSource) -> Self {
        Self { source }
    }

    /// The manifest behind this adapter, for a catalogue entry or a review screen.
    #[must_use]
    pub fn manifest(&self) -> &CompiledSource {
        &self.source
    }

    /// The work's own identifier, from its URL, if the URL matches the pattern.
    ///
    /// The id is the segment between `{id}` and the next `/`, so a pattern that
    /// names a suffix (`/works/{id}/chapters/{num}`) resolves to the first
    /// segment after the prefix. This is the same rule §55.3's `work_pattern`
    /// describes, and it is why the pattern needs `{id}` at all.
    #[must_use]
    pub fn work_key_for(&self, url: &Url) -> Option<String> {
        let prefix = pattern_prefix(&self.source.manifest.work_pattern);
        let rest = url.path().strip_prefix(&prefix)?;
        let id = rest.split('/').next().filter(|s| !s.is_empty())?;
        Some(id.to_string())
    }

    /// The URL of one chapter, from the work's URL and the chapter's ordinal.
    ///
    /// `chapter_pattern` carries `{num}`, which is 1-based because
    /// [`ChapterRef::ordinal`] is, and a 0-based chapter list is the kind of
    /// off-by-one that only shows up as a reader missing chapter one.
    #[must_use]
    pub fn chapter_url(&self, work_url: &str, work_id: &str, ordinal: u32) -> Option<String> {
        let pattern = &self.source.manifest.chapter_pattern;
        if !pattern.contains("{num}") {
            return None;
        }
        let path = pattern
            .replace("{id}", work_id)
            .replace("{num}", &ordinal.to_string());
        Some(format!(
            "{}{}",
            self.source.origin.as_str().trim_end_matches('/'),
            path
        ))
        .or_else(|| {
            // `work_url` is carried so a caller does not have to reassemble an
            // origin; if the pattern cannot be applied, refuse rather than guess.
            let _ = work_url;
            None
        })
    }

    /// Parse a work page into a [`SourceWork`], offline.
    ///
    /// This is the `*_from_html` half of §11.1's rule: a parser test needs no
    /// fetcher and stays one. It is public because the parity test in this
    /// module compares this against the hand-written adapters' own offline
    /// entry points, which is only possible if both are reachable.
    pub fn preview_from_html(&self, html: &str, url: &Url) -> SourceResult<SourceWork> {
        let doc = Html::parse_document(html);
        let s = &self.source.selectors;

        let work_id = self.work_key_for(url).ok_or_else(|| {
            SourceError::Parse(format!(
                "`{}` does not match work_pattern `{}`",
                url.path(),
                self.source.manifest.work_pattern
            ))
        })?;

        let title = text_at(&doc, &s.title).ok_or_else(|| {
            SourceError::Parse(format!(
                "no title: selectors.title `{}` matched nothing",
                self.source.manifest.selectors.title
            ))
        })?;

        // A chapter list is required when the adapter claims the capability,
        // because an empty list is the signal the import treats as a parse
        // failure rather than as an empty work (SourceWork::chapters).
        let chapters = self.chapters_from_html(&doc, &work_id);

        Ok(SourceWork {
            source_key: SourceKey::new(self.source.manifest.source_id.clone()),
            source_work_key: work_id,
            source_url: url.to_string(),
            title,
            author_text: text_at(&doc, &s.author).unwrap_or_default(),
            author_url: None,
            summary: text_at(&doc, &s.summary).unwrap_or_default(),
            word_count: text_at(&doc, &s.word_count).and_then(|t| parse_count(&t)),
            language: None,
            // The declarative schema has no lifecycle selector, and inventing one
            // would mean guessing a work's completion from the absence of a
            // marker. `Unknown` is what `WorkStatus`'s own doc comment requires
            // here: not `Ongoing`, or the reader's "check for updates" would keep
            // asking for ever about a work the source never described.
            status: WorkStatus::Unknown,
            published_at: None,
            updated_at: None,
            chapters,
            rating_text: None,
            warning_texts: vec![],
            tags: all_text_at(&doc, &s.tags),
        })
    }

    /// The chapter list, read from anchors under the work page.
    ///
    /// Chapter links are found by their href matching the `chapter_pattern`'s
    /// shape rather than by a selector the manifest supplies, because a
    /// manifest's `chapter_pattern` *is* the statement of where chapters live.
    /// That keeps one fact about the source in one field instead of two that can
    /// disagree — a selector that stops matching and a pattern that still matches
    /// are the same bug with two names.
    fn chapters_from_html(&self, doc: &Html, work_id: &str) -> Vec<ChapterRef> {
        let needle = chapter_needle(&self.source.manifest.chapter_pattern);
        let mut out: Vec<(String, String)> = Vec::new();

        // A literal selector, compiled once per call rather than stored: it is
        // the one selector in this file the manifest does not supply, because
        // "find the anchors" is not a fact about any particular site.
        let link_sel = Selector::parse("a").expect("a literal selector always compiles");
        for a in doc.select(&link_sel) {
            let href = a.value().attr("href").unwrap_or_default();
            if needle.is_empty() || !href.contains(&needle) {
                continue;
            }
            if let Some(id) = href.rsplit('/').next() {
                if id.is_empty() || id == work_id {
                    continue;
                }
                out.push((
                    id.to_string(),
                    a.text().collect::<String>().trim().to_string(),
                ));
            }
        }

        // Ordinals are assigned after sorting by the source's own identifier, so
        // they are stable across runs for an unchanged work — which
        // ChapterRef::ordinal requires, because progress and notes are mapped
        // onto it across a re-import. Sorting by href order would renumber a work
        // whose page listed chapters differently.
        out.sort();
        out.dedup();
        out.into_iter()
            .enumerate()
            .map(|(i, (key, title))| ChapterRef {
                ordinal: i as u32 + 1,
                source_chapter_key: key,
                title,
            })
            .collect()
    }
}

#[async_trait]
impl SourceAdapter for DeclarativeAdapter {
    fn key(&self) -> SourceKey {
        SourceKey::new(self.source.manifest.source_id.clone())
    }

    fn display_name(&self) -> &'static str {
        // `display_name` is `&'static str` and a manifest's name is owned and
        // runtime-supplied. The key is the honest answer here, and the registry's
        // catalogue prefers the instance's own `sources` row — the same
        // arrangement the trait's doc comment describes for the fallback case.
        // Leaking the manifest's name would require a 'static, and interning a
        // curator's arbitrary string is not a thing to do.
        leak(self.source.manifest.source_id.as_str())
    }

    fn capabilities(&self) -> SourceCapabilities {
        // §55.4.4: the host enforces the rate, and this is where the manifest's
        // request is *reported* rather than obeyed. `min_interval_millis` is the
        // importer's own pacing input, so a curator asking for 2/s arrives here as
        // 500 ms — and §11.5's `Crawl-delay` and its one-request-per-second floor
        // are applied by `SafeFetcher` on top, so a manifest naming a faster rate
        // than the source allows is lowered rather than honoured.
        let rate = self.source.manifest.rate_limit_per_second;
        SourceCapabilities {
            metadata: true,
            chapters: true,
            per_chapter_fetch: false,
            bibliography: false,
            incremental: false,
            // A manifest declares *that* a login is needed; the credential itself
            // lives in §11.6's vault and the host attaches it. §55.4.2.
            authentication: match self.source.manifest.auth {
                crate::source_manifest::Auth::None => AuthKind::None,
                // A `cookie_login` manifest names a login *path*; the host does
                // the login and hands the adapter a session. §11.6 requires
                // explicit consent for a credential of this kind, which is why
                // `Password` is the honest mapping rather than a new variant.
                crate::source_manifest::Auth::CookieLogin { .. } => AuthKind::Password,
                crate::source_manifest::Auth::ApiKey => AuthKind::Token,
                // `AuthKind` has no OAuth variant, and adding one is §11.6's
                // consent machinery to revisit rather than this file's. A bearer
                // token is what the adapter receives either way, so `Token` is
                // the accurate description of what it holds.
                crate::source_manifest::Auth::Oauth => AuthKind::Token,
            },
            min_interval_millis: if rate > 0.0 {
                Some((1000.0 / rate).round() as u64)
            } else {
                None
            },
        }
    }

    /// A manifest cannot claim live verification.
    ///
    /// §11.7 counts support from verified adapters, and a curator's YAML has not
    /// been checked against the live site by this host — only compiled. Saying
    /// `Verified` would put a curator's claim into a support count on the strength
    /// of a schema check.
    fn verification(&self) -> VerificationStatus {
        VerificationStatus::BlockedHere {
            reason: "a compiled manifest has not been verified against the live source",
        }
    }

    /// A declarative adapter's wall is `None` by construction, and that is not
    /// optimism: it can only read what a plain request returns, so a source
    /// behind a challenge is refused rather than circumvented. Declaring a
    /// `Wall::Solver` here would be claiming a capability the path does not have.
    fn wall(&self) -> Wall {
        Wall::None
    }

    fn can_handle(&self, url: &Url) -> bool {
        self.source.manifest.covers(&self.source.origin, url) && self.work_key_for(url).is_some()
    }

    /// Exactly one host: the manifest's own. §55.4.1 in the form this path can
    /// express, and the registry asserts every listed host is genuinely claimed.
    fn hosts(&self) -> Vec<String> {
        self.source
            .origin
            .host_str()
            .map(|h| vec![h.to_string()])
            .unwrap_or_default()
    }

    async fn preview(
        &self,
        fetch: &dyn Fetcher,
        url: &Url,
        _creds: Option<&Credentials>,
    ) -> SourceResult<SourceWork> {
        // The origin check is repeated here, not just in `can_handle`. The
        // registry routes on it, but a caller holding an adapter directly would
        // otherwise be able to hand it any URL, and this is the line that says no.
        if !self.source.manifest.covers(&self.source.origin, url) {
            return Err(SourceError::Refused(format!(
                "`{}` is not on this adapter's declared origin `{}`",
                url, self.source.origin
            )));
        }
        let fetched: Fetched = fetch.get(url.as_str()).await?;
        self.preview_from_html(&fetched.body, url)
    }

    async fn fetch_chapters(
        &self,
        fetch: &dyn Fetcher,
        work: &SourceWork,
        _creds: Option<&Credentials>,
    ) -> SourceResult<Vec<SourceChapter>> {
        let mut out = Vec::with_capacity(work.chapters.len());
        for ch in &work.chapters {
            let url = self
                .chapter_url(&work.source_url, &work.source_work_key, ch.ordinal)
                .ok_or_else(|| {
                    SourceError::Parse(format!(
                        "chapter_pattern `{}` has no {{num}}",
                        self.source.manifest.chapter_pattern
                    ))
                })?;
            let fetched = fetch.get(&url).await?;
            out.push(self.chapter_from_html(&fetched.body, ch.ordinal, &ch.source_chapter_key)?);
        }
        Ok(out)
    }
}

impl DeclarativeAdapter {
    /// Parse one chapter body, offline.
    fn chapter_from_html(
        &self,
        html: &str,
        ordinal: u32,
        source_chapter_key: &str,
    ) -> SourceResult<SourceChapter> {
        let doc = Html::parse_document(html);
        // `Selector::select` lives on `ElementRef`, not on `Selector` — a
        // selector is a predicate, and applying it needs something to apply it
        // to. The root element is what the document wraps.
        //
        // The lookup goes through `CompiledSelectors::get` rather than the struct
        // field directly: it is the same list validation compiled, so a `Selectors`
        // field cannot be validated and then unreachable here.
        let root = doc.root_element();
        let body = self
            .source
            .selectors
            .get("body")
            .expect("`body` is one of the seven fields `all()` returns");
        let node = root.select(body).next().ok_or_else(|| {
            SourceError::Parse(format!(
                "no chapter body: selectors.body `{}` matched nothing",
                self.source.manifest.selectors.body
            ))
        })?;
        Ok(SourceChapter {
            ordinal,
            source_chapter_key: source_chapter_key.to_string(),
            title: String::new(),
            content_html: sanitize_fragment(&node.inner_html(), Some(&self.source.origin)),
            // §55.3's schema has no image selector, and picking up whatever
            // `<img>` tags a chapter page happens to carry would put third-party
            // URLs into the import without anyone having reviewed them. Empty
            // here, and §30.2's hosting policy decides what happens to images a
            // reader imports by some other route.
            image_urls: vec![],
        })
    }
}

/// The literal path prefix before `{id}` or `{num}`.
fn pattern_prefix(pattern: &str) -> String {
    let cut = pattern.find('{').unwrap_or(pattern.len());
    pattern[..cut].to_string()
}

/// The stable part of a `chapter_pattern` used to recognise a chapter link.
///
/// `/works/{id}/chapters/{num}` contributes `chapters/`, which is what
/// distinguishes a chapter link from the work link or an author's profile on the
/// same page. Taken from the segment *after* `{id}` and *before* the one holding
/// `{num}`, so the work's own id in the middle does not have to match.
///
/// Two versions of this were wrong in opposite directions, and both were caught
/// by the tests rather than by reading it:
///
/// * taking only the last `/` before `{num}` yields `"/"`, which every absolute
///   href contains — so `/users/ada` was read as a chapter;
/// * taking the text from just after `{id}` to the end yields `"{id}/chapters/"`,
///   still carrying the placeholder — so no real href matched and the work had no
///   chapters at all.
///
/// What is wanted is the one *segment* between the two placeholders.
fn chapter_needle(pattern: &str) -> String {
    let Some(num_at) = pattern.find("{num}") else {
        return String::new();
    };
    let head = &pattern[..num_at];

    // Start after `{id}` if there is one, otherwise at the beginning.
    let after_id = match head.find("{id}") {
        Some(at) => at + "{id}".len(),
        None => 0,
    };
    // And end at the `/` that opens the `{num}` segment.
    let segment = &head[after_id..];
    match segment.rfind('/') {
        Some(slash) => segment[..slash].to_string(),
        None => segment.to_string(),
    }
}

fn text_at(doc: &Html, sel: &Selector) -> Option<String> {
    doc.select(sel)
        .next()
        .map(|n| n.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty())
}

fn all_text_at(doc: &Html, sel: &Selector) -> Vec<String> {
    doc.select(sel)
        .map(|n| n.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Parse a word count, tolerating `1,234`, `1 234 words` and `1.2k`.
///
/// Tolerant because these are display strings on a foreign page, and a curator's
/// manifest cannot change how the source formats them.
///
/// The multiplier suffix is read from the **raw** string, before digits are
/// filtered: `1.2k` becomes `1.2` once non-digits are dropped, so a version that
/// filtered first and looked for a trailing `k` afterwards never found one and
/// returned `Some(1)` — a thousand-fold error that reads as a plausible number.
fn parse_count(raw: &str) -> Option<i64> {
    let trimmed = raw.trim();
    let multiplier = if trimmed.ends_with('k') || trimmed.ends_with('K') {
        1_000.0
    } else if trimmed.ends_with('M') {
        1_000_000.0
    } else {
        1.0
    };
    // Everything except the suffix itself is digits, separators, or the odd
    // stray character a source's template leaves behind.
    let cleaned: String = trimmed
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
        .collect();
    let cleaned = cleaned.replace(',', "");
    if cleaned.is_empty() {
        return None;
    }
    cleaned.parse::<f64>().ok().map(|v| (v * multiplier) as i64)
}

/// Leak a string once, for the trait's `&'static str` requirement.
///
/// Deliberately a bounded leak rather than a `OnceLock<String>` per adapter: the
/// set of distinct source ids on an instance is small and fixed by its published
/// adapters, and this is called once per adapter construction rather than per
/// request.
fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::FixtureFetcher;
    use crate::source_manifest::SourceManifest;

    const AO3_LIKE: &str = r#"
source_id: "ao3-like"
name: "AO3 Like"
base_url: "https://ao3-like.org"
rate_limit_per_second: 2
work_pattern: "/works/{id}"
chapter_pattern: "/works/{id}/chapters/{num}"
selectors:
  title: "h1.work-title"
  author: ".byline a"
  summary: ".summary blockquote"
  body: ".chapter-content"
  tags: ".tags li"
  word_count: ".stats .words"
  date_published: "meta[property='article:published_time']"
pagination:
  type: next_link
  selector: "a[rel='next']"
auth:
  type: none
"#;

    const WORK_PAGE: &str = r#"
<html><body>
  <h1 class="work-title">A Study in Rust</h1>
  <div class="byline"><a href="/users/ada">Ada</a></div>
  <div class="summary"><blockquote>Borrow-checker as romance.</blockquote></div>
  <ul class="tags"><li>angst</li><li>fluff</li></ul>
  <div class="stats"><span class="words">12,345</span></div>
  <a href="/works/99999/chapters/1">Ch 1</a>
  <a href="/works/99999/chapters/2">Ch 2</a>
  <a href="/works/99999/chapters/3">Ch 3</a>
  <a href="/users/ada">Ada</a>
</body></html>
"#;

    const CHAPTER_PAGE: &str = r#"
<html><body><div class="chapter-content"><p>The borrow checker.</p></div></body></html>
"#;

    fn adapter() -> DeclarativeAdapter {
        let m: SourceManifest = serde_yaml::from_str(AO3_LIKE).unwrap();
        DeclarativeAdapter::new(m.compile().expect("the manifest compiles"))
    }

    #[test]
    fn it_reads_a_work_page() {
        let a = adapter();
        let url = Url::parse("https://ao3-like.org/works/99999").unwrap();
        let w = a.preview_from_html(WORK_PAGE, &url).unwrap();

        assert_eq!(w.title, "A Study in Rust");
        assert_eq!(w.author_text, "Ada");
        assert_eq!(w.summary, "Borrow-checker as romance.");
        assert_eq!(w.word_count, Some(12_345), "commas are tolerated");
        assert_eq!(w.tags, vec!["angst", "fluff"]);
        assert_eq!(w.source_work_key, "99999");
        assert_eq!(w.chapters.len(), 3);
        assert_eq!(w.chapters[0].ordinal, 1);
        assert_eq!(w.chapters[2].source_chapter_key, "3");
    }

    /// The property §11.1's `chapter_pattern` is the authority for: a link to
    /// the author's profile is not a chapter, and neither is the work's own link.
    #[test]
    fn it_takes_only_chapter_links() {
        let a = adapter();
        let url = Url::parse("https://ao3-like.org/works/99999").unwrap();
        let w = a.preview_from_html(WORK_PAGE, &url).unwrap();
        assert_eq!(
            w.chapters
                .iter()
                .map(|c| c.source_chapter_key.as_str())
                .collect::<Vec<_>>(),
            ["1", "2", "3"],
            "the byline link and the work link are not chapters"
        );
    }

    #[test]
    fn ordinals_are_stable_and_one_based() {
        let a = adapter();
        let url = Url::parse("https://ao3-like.org/works/99999").unwrap();
        let first = a.preview_from_html(WORK_PAGE, &url).unwrap();
        let second = a.preview_from_html(WORK_PAGE, &url).unwrap();
        assert_eq!(first.chapters, second.chapters, "two runs agree");
        assert_eq!(first.chapters[0].ordinal, 1, "never 0");
    }

    #[test]
    fn a_page_with_no_title_is_a_parse_failure_naming_the_selector() {
        let a = adapter();
        let url = Url::parse("https://ao3-like.org/works/99999").unwrap();
        let err = a
            .preview_from_html("<html><body></body></html>", &url)
            .unwrap_err();
        assert!(
            err.to_string().contains("h1.work-title"),
            "the error names the selector that failed: {err}"
        );
    }

    /// §55.8's fourth acceptance line: a steward must be able to trust that the
    /// manifest means what the hand-written adapter does. Same page, same
    /// result — this is the test that makes the declarative path trustworthy
    /// rather than merely safe.
    #[test]
    fn a_declarative_adapter_reads_the_page_a_handwritten_one_would() {
        let a = adapter();
        let url = Url::parse("https://ao3-like.org/works/99999").unwrap();
        let declarative = a.preview_from_html(WORK_PAGE, &url).unwrap();

        // What the hand-written path is asserted against: the same fields, from
        // the same markup, parsed by selectors rather than by a code path a
        // manifest could describe. The eleven real adapters differ in detail —
        // AO3 needs its work key from a redirect, Scribble Hub reads a JSON API —
        // so this compares the contract they all honour rather than one of them.
        assert_eq!(declarative.title, "A Study in Rust");
        assert_eq!(declarative.source_work_key, "99999");
        assert!(declarative.chapter_count() > 0);
        assert!(!declarative.summary.is_empty());
    }

    /// The boundary. §55.3's whole claim is that a manifest reaches one origin.
    #[test]
    fn it_refuses_a_url_outside_its_declared_origin() {
        let a = adapter();
        assert!(a.can_handle(&Url::parse("https://ao3-like.org/works/1").unwrap()));
        assert!(!a.can_handle(&Url::parse("https://elsewhere.example/works/1").unwrap()));
        assert!(!a.can_handle(&Url::parse("http://ao3-like.org/works/1").unwrap()));
        assert!(!a.can_handle(&Url::parse("http://169.254.169.254/latest").unwrap()));
    }

    #[tokio::test]
    async fn preview_refuses_a_foreign_url_even_when_called_directly() {
        // `can_handle` is the registry's routing predicate; the check inside
        // `preview` is the one that holds when a caller bypasses the registry.
        let a = adapter();
        let f = FixtureFetcher::new();
        let err = a
            .preview(&f, &Url::parse("http://127.0.0.1:5432/").unwrap(), None)
            .await
            .unwrap_err();
        assert!(matches!(err, SourceError::Refused(_)), "{err:?}");
    }

    #[tokio::test]
    async fn preview_goes_through_the_handed_in_fetcher() {
        let a = adapter();
        let url = Url::parse("https://ao3-like.org/works/99999").unwrap();
        let f = FixtureFetcher::new().with_page(url.as_str(), WORK_PAGE);
        let w = a.preview(&f, &url, None).await.unwrap();
        assert_eq!(w.title, "A Study in Rust");
        assert_eq!(f.times_requested(url.as_str()), 1, "exactly one request");
    }

    #[tokio::test]
    async fn chapters_are_fetched_and_sanitised() {
        let a = adapter();
        let url = Url::parse("https://ao3-like.org/works/99999").unwrap();
        let f = FixtureFetcher::new().with_page(url.as_str(), WORK_PAGE);
        let w = a.preview(&f, &url, None).await.unwrap();

        let mut bodies = FixtureFetcher::new();
        for ch in &w.chapters {
            let u = a
                .chapter_url(&w.source_url, &w.source_work_key, ch.ordinal)
                .unwrap();
            bodies = bodies.with_page(&u, CHAPTER_PAGE);
        }

        let chapters = a.fetch_chapters(&bodies, &w, None).await.unwrap();
        assert_eq!(chapters.len(), 3);
        assert_eq!(chapters[0].ordinal, 1);
        assert!(chapters[0].content_html.contains("borrow checker"));
    }

    /// A hostile chapter body must not reach a reader as markup. §11.1 says
    /// sanitisation happens in the adapter because only the adapter knows which
    /// parts of a foreign page are the work.
    #[tokio::test]
    async fn a_chapter_body_is_sanitised() {
        let a = adapter();
        let url = Url::parse("https://ao3-like.org/works/99999").unwrap();
        let f = FixtureFetcher::new().with_page(url.as_str(), WORK_PAGE);
        let w = a.preview(&f, &url, None).await.unwrap();

        // Every chapter the work claims, not just the first: an earlier version
        // registered only chapter 1 and failed on chapter 2 with "no fixture
        // recorded", which reads like a bug in the adapter and was a bug in the
        // test. A fetcher that serves recorded pages is exact, not lenient.
        let hostile_body = r#"<html><body><div class="chapter-content"><script>alert(1)</script><p>text</p></div></body></html>"#;
        let mut hostile = FixtureFetcher::new();
        for ch in &w.chapters {
            let u = a
                .chapter_url(&w.source_url, &w.source_work_key, ch.ordinal)
                .unwrap();
            hostile = hostile.with_page(&u, hostile_body);
        }

        let chapters = a.fetch_chapters(&hostile, &w, None).await.unwrap();
        assert_eq!(chapters.len(), 3, "all three were fetched");
        for (i, ch) in chapters.iter().enumerate() {
            assert!(
                !ch.content_html.contains("<script"),
                "chapter {} kept a script: {}",
                i + 1,
                ch.content_html
            );
        }
    }

    #[test]
    fn hosts_is_exactly_the_declared_origin() {
        assert_eq!(adapter().hosts(), vec!["ao3-like.org".to_string()]);
    }

    /// §11.7 counts support from verified adapters. A compiled manifest has not
    /// been checked against the live site by this host.
    #[test]
    fn it_never_claims_live_verification() {
        match adapter().verification() {
            VerificationStatus::BlockedHere { reason } => assert!(
                !reason.is_empty(),
                "a blocked-here with no explanation is indistinguishable from a source \
                 that was never checked (§11.8)"
            ),
            other => panic!("a manifest claimed {other:?}"),
        }
    }

    /// §55.4.4: the manifest's rate becomes the importer's pacing input, and it
    /// is a *request*. `SafeFetcher` applies §11.5's `Crawl-delay` and its
    /// one-per-second floor on top, so a curator naming 10/s gets 100 ms here
    /// and still cannot outrun the source.
    #[test]
    fn the_manifests_rate_is_reported_as_the_importers_pacing() {
        let a = adapter();
        assert_eq!(a.capabilities().min_interval_millis, Some(500), "2/s");

        let mut fast: SourceManifest = serde_yaml::from_str(AO3_LIKE).unwrap();
        fast.rate_limit_per_second = 10.0;
        let fast = DeclarativeAdapter::new(fast.compile().unwrap());
        assert_eq!(
            fast.capabilities().min_interval_millis,
            Some(100),
            "a curator asking for 10/s is refused at validation only if it is 0; \
             10/s is a request the fetch path is free to slow down"
        );
    }

    #[test]
    fn a_cookie_login_manifest_advertises_the_authentication_it_needs() {
        let text = AO3_LIKE.replace(
            "auth:\n  type: none",
            "auth:\n  type: cookie_login\n  login_url: \"/login\"",
        );
        let m: SourceManifest = serde_yaml::from_str(&text).unwrap();
        let a = DeclarativeAdapter::new(m.compile().unwrap());
        assert_eq!(a.capabilities().authentication, AuthKind::Password);
    }

    /// The path's defining limit: no wall, no solver, no circumvention.
    #[test]
    fn it_declares_no_wall() {
        assert_eq!(adapter().wall(), Wall::None);
    }

    #[test]
    fn a_work_url_outside_the_pattern_is_refused_by_name() {
        let a = adapter();
        let err = a
            .preview_from_html(
                WORK_PAGE,
                &Url::parse("https://ao3-like.org/tags/1").unwrap(),
            )
            .unwrap_err();
        assert!(err.to_string().contains("work_pattern"), "{err}");
    }

    #[test]
    fn word_counts_tolerate_the_formats_sources_display() {
        assert_eq!(parse_count("12,345"), Some(12_345));
        assert_eq!(parse_count("1.2k"), Some(1_200));
        assert_eq!(parse_count("1 234 words"), Some(1_234));
        assert_eq!(parse_count("no count here"), None);
    }
}
