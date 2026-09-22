//! Syosetu — 小説家になろう, "Shousetsuka ni Narou".
//!
//! # Recognised URLs
//!
//! ```text
//! https://ncode.syosetu.com/{ncode}/
//! https://ncode.syosetu.com/{ncode}/{episode}/
//! https://ncode.syosetu.com/novelview/infotop/ncode/{ncode}/
//! ```
//!
//! # Why this adapter reads three kinds of page
//!
//! Every other adapter in this crate reads one page and is done. Syosetu spreads
//! the same information over three, and knowing which page holds what is most of
//! the work:
//!
//! | Page | Holds |
//! |---|---|
//! | `/{ncode}/` | the episode list, **100 episodes at a time**, paginated by `?p=N` |
//! | `/novelview/infotop/ncode/{ncode}/` | title, author, summary, dates, status, word count, tags, and the *total* episode count |
//! | `/{ncode}/{episode}/` | one episode's prose |
//!
//! The consequence is the thing to understand before reading further: **the work
//! page does not state how many episodes the work has.** A work with 795 episodes
//! serves 100 rows and a pager; the number 795 appears only on the info page, as
//! 全795エピソード. So a preview that reads the work page and stops reports 100 of
//! 795 chapters, and — because the import trusts the preview — silently imports a
//! seventh of the work as though it were all of it.
//!
//! This adapter therefore reads the info page first, walks every page of the
//! episode list, and then checks the two against each other. If they disagree it
//! returns [`SourceError::Parse`] rather than a short list, because a short list
//! is the failure mode that looks like success.
//!
//! The cost is real and worth stating: previewing a 795-episode work is one info
//! request plus eight list requests, and importing it is 795 more. At the one
//! request per second Syosetu publishes in its `robots.txt` that is about
//! fourteen minutes, which is why the import is resumable.
//!
//! # Dates are Japanese local time
//!
//! The site shows `2012年 04月20日 21時58分` and `2012/04/20 21:58`, neither with
//! an offset. These are read as **JST (+09:00)** — the site is Japanese, its
//! `html` element declares `lang="ja"`, and treating a Japanese local time as UTC
//! would shift every imported date by nine hours. The offset is carried on the
//! value rather than converted away, so the stored instant is correct whatever a
//! reader's own timezone is. This is an inference, not something the page states;
//! it is recorded here because if it is wrong it is wrong for every work.
//!
//! # One-shots (短編)
//!
//! A 短編 is a single-installment work. Its work page carries the prose directly
//! and has no episode list at all, and its info page has no episode-count element
//! — the two are genuinely absent rather than zero. It imports as a work of
//! exactly one chapter, whose key is the work's own ncode, which is also how
//! [`Syosetu::fetch_chapter`] knows to re-read the work page rather than build an
//! episode URL that does not exist.
//!
//! # What this adapter does *not* do
//!
//! It does not handle `novel18.syosetu.com`, the adult sibling host. That is a
//! separate host behind an age-confirmation gate, and this adapter neither claims
//! its URLs nor lists it in `hosts()` — a source in the catalogue that always
//! refuses is worse than one that is absent, and no page from behind the gate
//! could be recorded to write a parser against. A novel18 URL routes to nothing
//! and is reported as unsupported.
//!
//! It does not enumerate an author's works either, so
//! [`SourceCapabilities::bibliography`] is false: the info page links to the
//! author's page, but its layout has not been recorded.
//!
//! Long works paginate their *episode list*; they do not split an episode across
//! pages. That was checked rather than assumed — a single episode is one document
//! with one `p-novel__text` body — because the alternative would have needed
//! joining logic, and silently importing the first page of every long chapter is
//! the kind of bug this crate exists to prevent.

use scraper::{ElementRef, Html, Selector};
use time::{OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};
use url::Url;

use async_trait::async_trait;

use crate::sanitize::sanitize_fragment;
use crate::{
    collapse_whitespace, strip_tags, text_of, ChapterRef, Credentials, Fetcher, SourceAdapter,
    SourceCapabilities, SourceChapter, SourceError, SourceKey, SourceResult, SourceWork,
    WorkStatus,
};

/// The host this adapter serves.
const HOST: &str = "ncode.syosetu.com";

/// How many episodes the site lists per page of a work's episode list.
const EPISODES_PER_PAGE: usize = 100;

/// A ceiling on how many list pages will be walked.
///
/// The site's own pager states the last page, so this is not how the walk knows
/// where to stop — it is a backstop against a malformed pager asking for ten
/// thousand pages. 200 pages is 20,000 episodes, comfortably beyond the longest
/// work on the site, so reaching it means the pager was misread.
const MAX_LIST_PAGES: u32 = 200;

/// Syosetu.
#[derive(Debug, Clone)]
pub struct Syosetu {
    key: SourceKey,
    hosts: Vec<String>,
}

/// What the info page says about a work.
///
/// Not `Default`: the status is a [`WorkStatus`], which deliberately has no
/// default, because a source that does not say must not have one invented for it.
#[derive(Debug, Clone)]
struct Info {
    title: String,
    author: String,
    author_url: Option<String>,
    summary: String,
    word_count: Option<i64>,
    language: Option<String>,
    status: WorkStatus,
    published_at: Option<OffsetDateTime>,
    updated_at: Option<OffsetDateTime>,
    tags: Vec<String>,
    /// 全795エピソード, as a number. `None` for a 短編, which has no such element.
    total_episodes: Option<u32>,
}

/// What one page of a work's episode list says.
#[derive(Debug, Clone)]
struct ListPage {
    /// `(ordinal, title)`, in page order.
    episodes: Vec<(u32, String)>,
    /// The last page the pager offers, when it offers one.
    last_page: Option<u32>,
    /// The range the page claims to be showing, as the site states it.
    stated_range: Option<(u32, u32)>,
    /// The latest episode date on the page.
    latest: Option<OffsetDateTime>,
}

impl Syosetu {
    /// A new adapter.
    pub fn new() -> Self {
        Self {
            key: SourceKey::new("syosetu"),
            hosts: vec![HOST.to_owned()],
        }
    }

    fn host_matches(&self, host: &str) -> bool {
        let host = host.trim_start_matches("www.");
        self.hosts.iter().any(|ours| host == ours)
    }

    /// The work's ncode in a path, as `n2267be`.
    ///
    /// Exactly three URL shapes carry one: `/{ncode}/`, `/{ncode}/{episode}/` and
    /// `/novelview/infotop/ncode/{ncode}/`. Anything else on the host is not a
    /// work — the site also serves help, ranking, search and static pages under
    /// the same origin, and a matcher that read the first path segment and
    /// ignored the rest would claim every one of them.
    ///
    /// Being narrow is the safe direction: a page this adapter cannot recognise
    /// is reported as unsupported, which is visible, where a page it claimed and
    /// mis-parsed would import as an empty work.
    fn ncode(url: &Url) -> Option<String> {
        let segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
        match segments.as_slice() {
            ["novelview", "infotop", "ncode", ncode] if is_ncode(ncode) => {
                Some((*ncode).to_owned())
            }
            [ncode] if is_ncode(ncode) => Some((*ncode).to_owned()),
            [ncode, episode] if is_ncode(ncode) && episode.parse::<u32>().is_ok() => {
                Some((*ncode).to_owned())
            }
            _ => None,
        }
    }

    /// Whether this is the info page rather than the work itself.
    fn is_info_url(url: &Url) -> bool {
        matches!(
            url.path_segments().map(|s| s.collect::<Vec<_>>()),
            Some(segments) if segments.starts_with(&["novelview", "infotop", "ncode"])
        )
    }

    fn work_url(ncode: &str) -> String {
        format!("https://{HOST}/{ncode}/")
    }

    fn info_url(ncode: &str) -> String {
        format!("https://{HOST}/novelview/infotop/ncode/{ncode}/")
    }

    /// A work's episode list, one page at a time.
    fn list_url(ncode: &str, page: u32) -> String {
        if page <= 1 {
            format!("https://{HOST}/{ncode}/")
        } else {
            format!("https://{HOST}/{ncode}/?p={page}")
        }
    }

    /// One episode's own URL.
    ///
    /// The episode number in the URL *is* the episode's position in the work —
    /// checked against the recorded pages, where `/{ncode}/1/` states `1/795` and
    /// `/{ncode}/2/` states `2/795`. That is what lets a single chapter be
    /// re-read without touching the work page.
    fn episode_url(ncode: &str, ordinal: u32) -> String {
        format!("https://{HOST}/{ncode}/{ordinal}/")
    }

    /// Parse the info page.
    fn parse_info(&self, html: &str) -> SourceResult<Info> {
        let document = Html::parse_document(html);
        if is_not_found(&document) {
            return Err(SourceError::NotFound);
        }

        let title = text_of(&document, "h1.p-infotop-title a")
            .or_else(|| text_of(&document, "h1.p-infotop-title"))
            .map(|raw| collapse_whitespace(&raw))
            .filter(|title| !title.is_empty())
            .ok_or_else(|| {
                SourceError::Parse(
                    "the info page carried no title at `h1.p-infotop-title`".to_owned(),
                )
            })?;

        let author_entry = entry(&document, "作者名");
        let author = author_entry
            .as_ref()
            .map(|element| collapse_whitespace(&element.text().collect::<String>()))
            .unwrap_or_default();
        let author_url = author_entry.as_ref().and_then(|element| {
            Selector::parse("a")
                .ok()
                .and_then(|selector| element.select(&selector).next())
                .and_then(|link| link.value().attr("href"))
                .map(|href| href.to_owned())
        });

        // The summary is HTML on the page and prose in `SourceWork`, so it is
        // reduced to text: a reader's summary still carrying `<br />` would be
        // rendered as literal angle brackets.
        let summary = entry(&document, "あらすじ")
            .map(|element| collapse_whitespace(&strip_tags(&element.inner_html())))
            .unwrap_or_default();

        let word_count = entry(&document, "文字数")
            .and_then(|element| digits_in(&element.text().collect::<String>()))
            .filter(|count| *count > 0);

        // The page declares its own language, which is the source publishing one
        // rather than us assuming it because the text looks Japanese.
        let language = document
            .root_element()
            .value()
            .attr("lang")
            .map(collapse_whitespace)
            .filter(|tag| !tag.is_empty());

        let published_at = entry(&document, "掲載日")
            .and_then(|element| parse_date(&element.text().collect::<String>()));
        let updated_at = entry(&document, "最新掲載日")
            .and_then(|element| parse_date(&element.text().collect::<String>()));

        // キーワード mixes full-width and half-width separators and uses `&nbsp;`
        // between some tags, so it is split on any whitespace after the entities
        // have been resolved.
        let tags = entry(&document, "キーワード")
            .map(|element| {
                element
                    .text()
                    .collect::<String>()
                    .split_whitespace()
                    .map(|tag| tag.trim().to_owned())
                    .filter(|tag| !tag.is_empty())
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();

        let status = text_of(&document, "span.p-infotop-type__type")
            .map(|label| work_status(&label))
            .unwrap_or(WorkStatus::Unknown);

        // 全795エピソード. Absent on a 短編, which is why this is an Option rather
        // than a number that defaults to zero.
        let total_episodes = text_of(&document, "span.p-infotop-type__allep")
            .and_then(|raw| digits_in(&raw))
            .and_then(|value| u32::try_from(value).ok())
            .filter(|total| *total > 0);

        Ok(Info {
            title,
            author,
            author_url,
            summary,
            word_count,
            language,
            status,
            published_at,
            updated_at,
            tags,
            total_episodes,
        })
    }

    /// Parse one page of a work's episode list.
    fn parse_list_page(&self, html: &str) -> SourceResult<ListPage> {
        let document = Html::parse_document(html);
        if is_not_found(&document) {
            return Err(SourceError::NotFound);
        }

        let rows = Selector::parse("div.p-eplist__sublist")
            .map_err(|e| SourceError::Internal(format!("a fixed selector failed: {e}")))?;
        let link = Selector::parse("a.p-eplist__subtitle")
            .map_err(|e| SourceError::Internal(format!("a fixed selector failed: {e}")))?;
        let update = Selector::parse("div.p-eplist__update")
            .map_err(|e| SourceError::Internal(format!("a fixed selector failed: {e}")))?;

        let mut episodes = Vec::new();
        let mut latest: Option<OffsetDateTime> = None;

        for row in document.select(&rows) {
            let Some(anchor) = row.select(&link).next() else {
                continue;
            };
            let Some(href) = anchor.value().attr("href") else {
                continue;
            };
            // The ordinal comes from the URL rather than from counting rows, so a
            // page that starts at episode 101 is numbered correctly and a row this
            // parser failed to understand cannot silently renumber the rest.
            let Some(ordinal) = href
                .trim_start_matches('/')
                .split('/')
                .nth(1)
                .and_then(|n| n.parse::<u32>().ok())
            else {
                continue;
            };
            let title = collapse_whitespace(&anchor.text().collect::<String>());

            if let Some(update_element) = row.select(&update).next() {
                // The element also carries the revision marker, whose date is in
                // a `title` attribute; the visible text is the original posting
                // date, which is the one that describes the episode.
                if let Some(date) = parse_date(&update_element.text().collect::<String>()) {
                    latest = Some(match latest {
                        Some(existing) if existing > date => existing,
                        _ => date,
                    });
                }
            }

            episodes.push((ordinal, title));
        }

        // The pager states the last page, which is how the walk knows where to
        // stop. Read from the link's own href rather than by counting the pager's
        // items, because the pager omits pages in the middle on a long work.
        let last_page = Selector::parse("a.c-pager__item--last")
            .ok()
            .and_then(|selector| document.select(&selector).next())
            .and_then(|anchor| anchor.value().attr("href"))
            .and_then(|href| Url::parse(&format!("https://{HOST}{href}")).ok())
            .and_then(|url| {
                url.query_pairs()
                    .find(|(key, _)| key == "p")
                    .and_then(|(_, value)| value.parse::<u32>().ok())
            });

        // エピソード 1 ～ 100 を表示中, as the page states it.
        let stated_range = text_of(&document, "div.c-pager__result-stats").and_then(|raw| {
            let numbers: Vec<u32> = raw
                .split(|c: char| !c.is_ascii_digit())
                .filter(|part| !part.is_empty())
                .filter_map(|part| part.parse::<u32>().ok())
                .collect();
            match numbers.as_slice() {
                [first, last, ..] => Some((*first, *last)),
                _ => None,
            }
        });

        Ok(ListPage {
            episodes,
            last_page,
            stated_range,
            latest,
        })
    }

    /// Assemble a work from the pages that describe it.
    ///
    /// Takes the documents rather than fetching them, so the fixture test can
    /// drive the whole assembly — including the disagreement check that the
    /// multi-page shape exists to catch.
    ///
    /// `list_pages` must be every page of the episode list. The caller is
    /// responsible for walking them; this function is what refuses to accept a
    /// partial walk.
    pub fn assemble(
        &self,
        ncode: &str,
        work_html: &str,
        info_html: &str,
        list_pages: &[String],
    ) -> SourceResult<SourceWork> {
        let info = self.parse_info(info_html)?;

        // A 短編 has no episode list and no total, and its work page holds the
        // prose itself.
        if info.total_episodes.is_none() {
            let document = Html::parse_document(work_html);
            if is_not_found(&document) {
                return Err(SourceError::NotFound);
            }
            if !has_episode_body(&document) {
                return Err(SourceError::Parse(format!(
                    "the info page for {ncode} states no episode count and the work page carries \
                     no episode body, so this work is neither serialized nor a one-shot"
                )));
            }
            let title = if info.title.is_empty() {
                text_of(&document, "h1.p-novel__title").unwrap_or_default()
            } else {
                info.title.clone()
            };
            return Ok(SourceWork {
                source_key: self.key.clone(),
                source_work_key: ncode.to_owned(),
                source_url: Syosetu::work_url(ncode),
                title: collapse_whitespace(&title),
                author_text: info.author,
                author_url: info.author_url,
                summary: info.summary,
                word_count: info.word_count,
                language: info.language,
                status: info.status,
                published_at: info.published_at,
                updated_at: info.updated_at,
                chapters: vec![ChapterRef {
                    ordinal: 1,
                    // The work's own ncode, not `1`: there is no episode 1 to
                    // name, and `fetch_chapter` reads this to know it must fetch
                    // the work page rather than build an episode URL.
                    source_chapter_key: ncode.to_owned(),
                    title: collapse_whitespace(&title),
                }],
                rating_text: None,
                warning_texts: Vec::new(),
                tags: info.tags,
            });
        }

        let total = info.total_episodes.expect("checked above");

        // Every page, in order, with each page's own claims checked before its
        // rows are believed.
        let mut episodes: Vec<(u32, String)> = Vec::new();
        let mut expected_total: Option<u32> = None;
        for (index, page_html) in list_pages.iter().enumerate() {
            let page = self.parse_list_page(page_html)?;
            let page_number = (index as u32) + 1;

            if page.episodes.is_empty() {
                return Err(SourceError::Parse(format!(
                    "page {page_number} of {ncode}'s episode list listed no episodes"
                )));
            }

            // The pager's own last-page number, cross-checked on page 1 against
            // the info page's count. A work whose pager wants more pages than we
            // were given is an incomplete walk, and this is where that is caught.
            if page_number == 1 {
                if let Some(last) = page.last_page {
                    if last > MAX_LIST_PAGES {
                        return Err(SourceError::Parse(format!(
                            "{ncode}'s episode list asks for {last} pages, beyond the {MAX_LIST_PAGES} \
                             this adapter will walk"
                        )));
                    }
                    expected_total = Some(match page.last_page {
                        Some(_) => last,
                        None => 0,
                    });
                }
            }

            // A page that is not the last one has to be full. A short page in the
            // middle is a truncated response, and because the totals would still
            // add up the count check at the end cannot see it.
            let is_last_page = page_number as usize == list_pages.len();
            if !is_last_page && page.episodes.len() != EPISODES_PER_PAGE {
                return Err(SourceError::Parse(format!(
                    "page {page_number} of {ncode} is not the last page but lists {} episodes, \
                     not {EPISODES_PER_PAGE}",
                    page.episodes.len()
                )));
            }

            // The page states which episodes it is showing. A page whose rows
            // disagree with its own statement is a page whose markup moved, and
            // reading a subset of it would be silent.
            if let Some((first, last)) = page.stated_range {
                let rows_match = page.episodes.first().map(|(n, _)| *n) == Some(first)
                    && page.episodes.last().map(|(n, _)| *n) == Some(last);
                if !rows_match {
                    return Err(SourceError::Parse(format!(
                        "page {page_number} of {ncode} states it shows episodes {first}-{last} but \
                         lists {:?}-{:?}",
                        page.episodes.first().map(|(n, _)| *n),
                        page.episodes.last().map(|(n, _)| *n)
                    )));
                }
            }

            // Pages after the first must continue where the previous one stopped.
            if let (Some((previous_last, _)), Some((first, _))) =
                (episodes.last(), page.episodes.first())
            {
                if *first != previous_last + 1 {
                    return Err(SourceError::Parse(format!(
                        "page {page_number} of {ncode} starts at episode {first} but the previous \
                         page ended at {previous_last}"
                    )));
                }
            }

            episodes.extend(page.episodes);
        }

        // The check this whole three-page arrangement exists for. The episode list
        // says how many rows it has; the info page says how many episodes the work
        // has. If they disagree — a page we did not read, a row we did not
        // understand — the honest answer is an error, because the alternative is
        // an import that quietly stores part of a work and reports success.
        if episodes.len() as u32 != total {
            return Err(SourceError::Parse(format!(
                "{ncode}'s info page states {total} episodes but its episode list yielded {} \
                 across {} page(s)",
                episodes.len(),
                list_pages.len()
            )));
        }

        // A walk that stopped before the pager's own last page is short by
        // definition, and the count check above cannot catch it when both numbers
        // happen to agree.
        if let Some(expected) = expected_total {
            if expected as usize != list_pages.len() {
                return Err(SourceError::Parse(format!(
                    "{ncode}'s pager states {expected} pages but {} were read",
                    list_pages.len()
                )));
            }
        }

        let updated_at = info.updated_at.or_else(|| {
            list_pages
                .iter()
                .filter_map(|html| self.parse_list_page(html).ok())
                .filter_map(|page| page.latest)
                .max()
        });

        let chapters = episodes
            .into_iter()
            .map(|(ordinal, title)| ChapterRef {
                ordinal,
                // The episode number is the site's own stable identifier for the
                // chapter: it is in the URL, and it is what the site itself
                // displays as `2/795`.
                source_chapter_key: ordinal.to_string(),
                title,
            })
            .collect();

        Ok(SourceWork {
            source_key: self.key.clone(),
            source_work_key: ncode.to_owned(),
            // Normalised to the work page even when the reader pasted an info or
            // episode URL: `fetch_chapters` reads this, and an episode URL there
            // would import one chapter as if it were the work.
            source_url: Syosetu::work_url(ncode),
            title: info.title,
            author_text: info.author,
            author_url: info.author_url,
            summary: info.summary,
            word_count: info.word_count,
            language: info.language,
            status: info.status,
            published_at: info.published_at,
            updated_at,
            chapters,
            rating_text: None,
            warning_texts: Vec::new(),
            tags: info.tags,
        })
    }

    /// Parse one episode document into a chapter.
    fn parse_episode(
        &self,
        html: &str,
        work: &SourceWork,
        ordinal: u32,
    ) -> SourceResult<SourceChapter> {
        let document = Html::parse_document(html);
        if is_not_found(&document) {
            return Err(SourceError::NotFound);
        }

        let body = episode_body(&document).ok_or_else(|| {
            SourceError::Parse(format!(
                "episode {ordinal} of {} carried no `div.p-novel__text`, so its prose could not be \
                 found",
                work.source_work_key
            ))
        })?;

        let title = text_of(&document, "h1.p-novel__title")
            .map(|raw| collapse_whitespace(&raw))
            .filter(|title| !title.is_empty())
            .or_else(|| {
                work.chapters
                    .iter()
                    .find(|chapter| chapter.ordinal == ordinal)
                    .map(|chapter| chapter.title.clone())
            })
            .unwrap_or_default();

        // Relative links inside the prose are resolved against the work page, so
        // a link the author wrote survives the import instead of becoming a dead
        // relative path on our domain.
        let base = Url::parse(&work.source_url).ok();
        let content_html = sanitize_fragment(&body, base.as_ref());
        let image_urls = crate::sanitize::extract_image_urls(&body, base.as_ref());

        // The key is the work's own key for this ordinal, not a number invented
        // here: a one-shot's chapter is keyed by its ncode, and returning "1"
        // instead would give the same chapter two identities — the work's and the
        // fetched chapter's — and the import dedupes on that key.
        let source_chapter_key = work
            .chapters
            .iter()
            .find(|chapter| chapter.ordinal == ordinal)
            .map(|chapter| chapter.source_chapter_key.clone())
            .unwrap_or_else(|| ordinal.to_string());

        Ok(SourceChapter {
            ordinal,
            source_chapter_key,
            title,
            content_html,
            image_urls,
        })
    }

    /// Walk every page of a work's episode list.
    async fn fetch_list_pages(
        &self,
        fetch: &dyn Fetcher,
        ncode: &str,
    ) -> SourceResult<Vec<String>> {
        let first = fetch.get(&Syosetu::list_url(ncode, 1)).await?;
        let first_page = self.parse_list_page(&first.body)?;

        let pages = first_page.last_page.unwrap_or(1).max(1);
        if pages > MAX_LIST_PAGES {
            return Err(SourceError::Parse(format!(
                "{ncode}'s episode list asks for {pages} pages, beyond the {MAX_LIST_PAGES} this \
                 adapter will walk"
            )));
        }
        // The count is checked against what the pager promised: a page that
        // served 100 rows and no pager is a single-page work, and a page that
        // served a full 100 rows with no pager is the shape a truncated response
        // has — the caller's own count check will catch the second.
        let mut bodies = vec![first.body];

        for page in 2..=pages {
            let response = fetch.get(&Syosetu::list_url(ncode, page)).await?;
            bodies.push(response.body);
        }
        Ok(bodies)
    }
}

impl Default for Syosetu {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SourceAdapter for Syosetu {
    fn key(&self) -> SourceKey {
        self.key.clone()
    }

    fn display_name(&self) -> &'static str {
        "Syosetu"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities {
            metadata: true,
            chapters: true,
            // An episode number is its own URL, so a retry re-reads exactly one
            // chapter without touching the work page or the other episodes.
            per_chapter_fetch: true,
            // The info page links to the author's page, but that page's layout
            // has not been recorded.
            bibliography: false,
            // The info page carries 最新掲載日, so an update check is one request.
            incremental: true,
            authentication: crate::AuthKind::None,
            // Syosetu publishes `Crawl-delay: 1` in its robots.txt, and the
            // fetcher takes the site's number over this one. Stated here so a
            // reader of the adapter sees the expected pace; the fetcher's own
            // floor of one second applies if the file ever stops saying so.
            min_interval_millis: Some(1_000),
        }
    }

    fn can_handle(&self, url: &Url) -> bool {
        let Some(host) = url.host_str() else {
            return false;
        };
        // Host *and* a real ncode in the path. The host match alone would claim
        // the site's search, ranking and help pages, none of which is a work.
        self.host_matches(host) && Syosetu::ncode(url).is_some()
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
        let ncode = Syosetu::ncode(url)
            .ok_or_else(|| SourceError::Unsupported(format!("{url} is not a Syosetu work URL")))?;

        // The work page and the info page are both needed, and the work page says
        // which shape this is: a 短編 carries its prose, a serialized work
        // carries an episode list.
        let work = fetch.get(&Syosetu::work_url(&ncode)).await?;
        let info = fetch.get(&Syosetu::info_url(&ncode)).await?;

        // The two documents are reduced to the three facts the rest of this needs
        // and then dropped, in a scope that ends before the next request. `Html`
        // is not `Send`, so holding one across an `.await` would make this future
        // non-`Send` and unusable from the worker that runs imports.
        let (work_lists_episodes, work_missing, info_missing) = {
            let work_document = Html::parse_document(&work.body);
            let info_document = Html::parse_document(&info.body);
            (
                has_episode_list(&work_document),
                is_not_found(&work_document),
                is_not_found(&info_document),
            )
        };
        if work_missing || info_missing {
            return Err(SourceError::NotFound);
        }

        // Only a serialized work has an episode list to walk. Reading the pages
        // is what makes the chapter count in the preview the real one rather than
        // the 100 the first page shows.
        let list_pages = if work_lists_episodes {
            self.fetch_list_pages(fetch, &ncode).await?
        } else {
            Vec::new()
        };

        self.assemble(&ncode, &work.body, &info.body, &list_pages)
    }

    async fn fetch_chapters(
        &self,
        fetch: &dyn Fetcher,
        work: &SourceWork,
        _creds: Option<&Credentials>,
    ) -> SourceResult<Vec<SourceChapter>> {
        let ncode = &work.source_work_key;

        // A one-shot's single chapter is the work page itself, and there is no
        // episode list to walk.
        if work.chapters.len() == 1
            && work
                .chapters
                .first()
                .is_some_and(|chapter| chapter.source_chapter_key == *ncode)
        {
            let page = fetch.get(&Syosetu::work_url(ncode)).await?;
            return self
                .parse_episode(&page.body, work, 1)
                .map(|chapter| vec![chapter]);
        }

        let mut chapters = Vec::with_capacity(work.chapters.len());
        for chapter in &work.chapters {
            let page = fetch
                .get(&Syosetu::episode_url(ncode, chapter.ordinal))
                .await?;
            chapters.push(self.parse_episode(&page.body, work, chapter.ordinal)?);
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
        let ncode = &work.source_work_key;
        let known = work
            .chapters
            .iter()
            .find(|chapter| chapter.ordinal == ordinal)
            .ok_or_else(|| {
                SourceError::Unsupported(format!("work {ncode} has no episode {ordinal}"))
            })?;

        // A one-shot has no episode URLs at all: its single chapter is the work
        // page, which is what the chapter's key being the ncode means.
        let target = if known.source_chapter_key == *ncode {
            Syosetu::work_url(ncode)
        } else {
            Syosetu::episode_url(ncode, ordinal)
        };

        let page = fetch.get(&target).await?;
        self.parse_episode(&page.body, work, ordinal)
    }

    fn preview_from_html(&self, html: &str, url: &Url) -> SourceResult<SourceWork> {
        // The single-document seam. An info page yields the metadata and the
        // total episode count; a work page yields the episodes on its first page.
        // Neither alone is a complete work for a serialized title, and the
        // assembly that combines them is `assemble`, which the fixture test drives
        // with every recorded page at once.
        let ncode = Syosetu::ncode(url)
            .ok_or_else(|| SourceError::Unsupported(format!("{url} is not a Syosetu work URL")))?;

        if Syosetu::is_info_url(url) {
            let info = self.parse_info(html)?;
            return Ok(SourceWork {
                source_key: self.key.clone(),
                source_work_key: ncode.clone(),
                source_url: Syosetu::work_url(&ncode),
                title: info.title,
                author_text: info.author,
                author_url: info.author_url,
                summary: info.summary,
                word_count: info.word_count,
                language: info.language,
                status: info.status,
                published_at: info.published_at,
                updated_at: info.updated_at,
                // The chapters themselves are not on the info page, and inventing
                // placeholder refs for `total` of them would put a chapter count
                // into a preview with nothing behind it. Empty here means "this
                // document does not carry the list", and the caller that combines
                // documents fills it in.
                chapters: Vec::new(),
                rating_text: None,
                warning_texts: Vec::new(),
                tags: info.tags,
            });
        }

        let document = Html::parse_document(html);
        if is_not_found(&document) {
            return Err(SourceError::NotFound);
        }
        if has_episode_list(&document) {
            let page = self.parse_list_page(html)?;
            let title = text_of(&document, "h1.p-novel__title")
                .map(|raw| collapse_whitespace(&raw))
                .unwrap_or_default();
            return Ok(SourceWork {
                source_key: self.key.clone(),
                source_work_key: ncode.clone(),
                source_url: Syosetu::work_url(&ncode),
                title,
                author_text: String::new(),
                author_url: None,
                summary: String::new(),
                word_count: None,
                language: None,
                status: WorkStatus::Unknown,
                published_at: None,
                updated_at: page.latest,
                chapters: page
                    .episodes
                    .into_iter()
                    .map(|(ordinal, title)| ChapterRef {
                        ordinal,
                        source_chapter_key: ordinal.to_string(),
                        title,
                    })
                    .collect(),
                rating_text: None,
                warning_texts: Vec::new(),
                tags: Vec::new(),
            });
        }

        // A one-shot's work page: the whole work is one chapter.
        let title = text_of(&document, "h1.p-novel__title")
            .map(|raw| collapse_whitespace(&raw))
            .filter(|title| !title.is_empty())
            .ok_or_else(|| {
                SourceError::Parse(
                    "the work page carried neither an episode list nor an `h1.p-novel__title`"
                        .to_owned(),
                )
            })?;
        Ok(SourceWork {
            source_key: self.key.clone(),
            source_work_key: ncode.clone(),
            source_url: Syosetu::work_url(&ncode),
            title: title.clone(),
            author_text: String::new(),
            author_url: None,
            summary: String::new(),
            word_count: None,
            language: None,
            status: WorkStatus::Complete,
            published_at: None,
            updated_at: None,
            chapters: vec![ChapterRef {
                ordinal: 1,
                source_chapter_key: ncode,
                title,
            }],
            rating_text: None,
            warning_texts: Vec::new(),
            tags: Vec::new(),
        })
    }

    fn chapters_from_html(
        &self,
        html: &str,
        work: &SourceWork,
    ) -> SourceResult<Vec<SourceChapter>> {
        // A one-shot's work page holds its single chapter's prose; an episode page
        // holds one episode's. The ordinal is taken from the caller's list by
        // matching the chapter key, and a page that matches nothing is reported
        // rather than filed under chapter 1.
        let document = Html::parse_document(html);
        if is_not_found(&document) {
            return Err(SourceError::NotFound);
        }
        if !has_episode_body(&document) {
            return Err(SourceError::Parse(
                "the document carried no `div.p-novel__text`, so there is no prose in it"
                    .to_owned(),
            ));
        }

        // Which chapter is this? The site states it, as `2/795`, and that is the
        // only self-description on the page. Falling back to the work's own list
        // by title, and only then refusing, keeps a page without the marker from
        // being filed as chapter 1.
        let ordinal = text_of(&document, "div.p-novel__number")
            .and_then(|raw| {
                raw.trim()
                    .split('/')
                    .next()
                    .and_then(|n| n.trim().parse::<u32>().ok())
            })
            .or_else(|| {
                let title =
                    text_of(&document, "h1.p-novel__title").map(|raw| collapse_whitespace(&raw))?;
                work.chapters
                    .iter()
                    .find(|chapter| chapter.title == title)
                    .map(|chapter| chapter.ordinal)
            })
            .ok_or_else(|| {
                SourceError::Parse(
                    "the episode page states no `N/total` position and its title matches no \
                     chapter of the work, so there is no way to say which episode it is"
                        .to_owned(),
                )
            })?;

        self.parse_episode(html, work, ordinal)
            .map(|chapter| vec![chapter])
    }
}

/// Whether a path segment looks like an ncode: `n` and then digits and letters.
fn is_ncode(candidate: &str) -> bool {
    let Some(rest) = candidate.strip_prefix('n') else {
        return false;
    };
    !rest.is_empty()
        && rest
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && rest.chars().any(|c| c.is_ascii_digit())
}

/// Whether a document is the site's error page.
///
/// Structural first: a work page has an episode list, a title heading or an
/// episode body, and the error page has none. The title text is the fallback for
/// a page with no landmarks at all.
fn is_not_found(document: &Html) -> bool {
    if has_episode_list(document) || has_episode_body(document) {
        return false;
    }
    if text_of(document, "h1.p-infotop-title").is_some()
        || text_of(document, "span.p-infotop-type__type").is_some()
    {
        return false;
    }
    let title = text_of(document, "title").unwrap_or_default();
    let body = text_of(document, "body").unwrap_or_default();
    let haystack = format!("{title}\n{body}");
    haystack.contains("見つかりません") || haystack.contains("エラー")
}

/// Whether the document lists episodes.
fn has_episode_list(document: &Html) -> bool {
    Selector::parse("div.p-eplist__sublist")
        .ok()
        .is_some_and(|selector| document.select(&selector).next().is_some())
}

/// Whether the document carries episode prose.
fn has_episode_body(document: &Html) -> bool {
    Selector::parse("div.p-novel__text")
        .ok()
        .is_some_and(|selector| document.select(&selector).next().is_some())
}

/// The prose of an episode, with the author's notes kept but set apart.
///
/// The body is one or more `div.p-novel__text` elements inside `div.p-novel__body`.
/// The plain one is the story; one carrying `--preface` or `--afterword` is the
/// author speaking outside it, which the recorded pages show both of on a work
/// with a preface and an afterword around a single chapter's prose.
///
/// The notes are wrapped in `blockquote` rather than a `div` with a class. That is
/// a rendering choice, and the reason is concrete: the crate's sanitiser allows a
/// fixed list of tags and attributes in which `div` and `class` do not appear, so a
/// `<div class="notes">` wrapper would be stripped and the note would become
/// indistinguishable from the story. `blockquote` is allowed, and is the closest
/// available tag to "the author, outside the narrative". Changing this means
/// changing the sanitiser's allow-list, which is a larger decision than this
/// adapter should make on its own.
fn episode_body(document: &Html) -> Option<String> {
    let selector = Selector::parse("div.p-novel__body div.p-novel__text").ok()?;
    let mut out = String::new();
    let mut found_any = false;

    for element in document.select(&selector) {
        found_any = true;
        let classes = element.value().attr("class").unwrap_or_default();
        let is_note = classes.contains("preface") || classes.contains("afterword");
        let inner = element.inner_html();
        if is_note {
            out.push_str("<blockquote>");
            out.push_str(&inner);
            out.push_str("</blockquote>\n");
        } else {
            out.push_str(&inner);
            out.push('\n');
        }
    }

    // Some pages carry the text elements without the `p-novel__body` wrapper;
    // falling back rather than returning nothing, because the prose is the one
    // thing this must not lose.
    if !found_any {
        let loose = Selector::parse("div.p-novel__text").ok()?;
        for element in document.select(&loose) {
            out.push_str(&element.inner_html());
            out.push('\n');
        }
    }
    Some(out).filter(|body| !body.trim().is_empty())
}

/// The `<dd>` that follows a `<dt class="p-infotop-data__title">label</dt>`.
///
/// The info page is a definition list, so the value is the label's next `dd`
/// sibling rather than something addressable by its own selector. Anything that
/// cannot be addressed directly — a `dt`'s value, its order — has to be walked.
fn entry<'a>(document: &'a Html, label: &str) -> Option<ElementRef<'a>> {
    let selector = Selector::parse("dt.p-infotop-data__title").ok()?;
    for term in document.select(&selector) {
        if collapse_whitespace(&term.text().collect::<String>()) != label {
            continue;
        }
        let mut node = term.next_sibling();
        while let Some(current) = node {
            if let Some(element) = ElementRef::wrap(current) {
                if element.value().name() == "dd" {
                    return Some(element);
                }
            }
            node = current.next_sibling();
        }
    }
    None
}

/// Every digit in a string, as a number. `9,666,529文字` becomes `9666529`.
fn digits_in(raw: &str) -> Option<i64> {
    let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
    digits.parse::<i64>().ok()
}

/// The site's status label, as a [`WorkStatus`].
///
/// `連載中` (serialized, in progress) and `短編` (one-shot) are the two labels the
/// recorded fixtures exercise. `完結済` (serialization finished) and `休載中` (on
/// hiatus) are the site's other labels, taken from its vocabulary rather than from
/// a recorded page — a fixture for a finished serial has not been recorded, so
/// those two are unverified and said so here rather than implied by silence.
/// Anything unrecognised is `Unknown`, which is the honest answer and is what the
/// import shows.
fn work_status(label: &str) -> WorkStatus {
    let label = label.trim();
    if label.contains("連載中") {
        WorkStatus::Ongoing
    } else if label.contains("短編") || label.contains("完結") {
        WorkStatus::Complete
    } else if label.contains("休載") {
        WorkStatus::Hiatus
    } else if label.contains("削除") {
        WorkStatus::Cancelled
    } else {
        WorkStatus::Unknown
    }
}

/// Japan Standard Time, the timezone every date on this site is written in.
const JST: UtcOffset = match UtcOffset::from_hms(9, 0, 0) {
    Ok(offset) => offset,
    Err(_) => UtcOffset::UTC,
};

/// Parse either of the two date shapes the site uses.
///
/// The info page writes `2012年 04月20日 21時58分`; an episode list row writes
/// `2012/04/20 21:58`. Neither carries an offset, and both are JST — see the
/// module documentation for why, and for the cost of being wrong about it. A date
/// in a shape this does not recognise becomes `None` rather than a guess, because
/// a wrong date on an imported work is worse than an absent one.
fn parse_date(raw: &str) -> Option<OffsetDateTime> {
    let text = collapse_whitespace(raw);
    if text.is_empty() {
        return None;
    }

    let (year, month, day, hour, minute) = if text.contains('年') {
        let (year, rest) = text.split_once('年')?;
        let (month, rest) = rest.split_once('月')?;
        let (day, rest) = rest.split_once('日')?;
        let (hour, rest) = rest.split_once('時')?;
        let (minute, _) = rest.split_once('分')?;
        (
            number_in(year)?,
            number_in(month)?,
            number_in(day)?,
            number_in(hour)?,
            number_in(minute)?,
        )
    } else {
        let (date, time) = text.split_once(char::is_whitespace)?;
        let mut date_parts = date.split('/');
        let year = number_in(date_parts.next()?)?;
        let month = number_in(date_parts.next()?)?;
        let day = number_in(date_parts.next()?)?;
        let mut time_parts = time.split(':');
        let hour = number_in(time_parts.next()?)?;
        let minute = number_in(time_parts.next()?)?;
        (year, month, day, hour, minute)
    };

    let date = time::Date::from_calendar_date(
        i32::try_from(year).ok()?,
        time::Month::try_from(u8::try_from(month).ok()?).ok()?,
        u8::try_from(day).ok()?,
    )
    .ok()?;
    let time = Time::from_hms(u8::try_from(hour).ok()?, u8::try_from(minute).ok()?, 0).ok()?;
    Some(PrimitiveDateTime::new(date, time).assume_offset(JST))
}

/// The leading run of digits in a string, as a number.
fn number_in(raw: &str) -> Option<i64> {
    let digits: String = raw
        .trim()
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse::<i64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(raw: &str) -> Url {
        Url::parse(raw).expect("a test URL must parse")
    }

    fn adapter() -> Syosetu {
        Syosetu::new()
    }

    #[test]
    fn a_work_url_is_claimed() {
        assert!(adapter().can_handle(&url("https://ncode.syosetu.com/n2267be/")));
    }

    #[test]
    fn an_episode_url_is_claimed_as_its_work() {
        assert!(adapter().can_handle(&url("https://ncode.syosetu.com/n2267be/795/")));
    }

    #[test]
    fn an_info_url_is_claimed() {
        assert!(adapter().can_handle(&url(
            "https://ncode.syosetu.com/novelview/infotop/ncode/n2267be/"
        )));
    }

    #[test]
    fn the_adult_host_is_not_claimed() {
        // Deliberately out of scope: the host is behind an age gate, so no page
        // could be recorded to write a parser against, and a catalogue entry that
        // always refuses is worse than an absent one.
        assert!(!adapter().can_handle(&url("https://novel18.syosetu.com/n2267be/")));
    }

    #[test]
    fn a_page_that_is_not_a_work_is_not_claimed() {
        // The bare host match would claim all of these.
        assert!(!adapter().can_handle(&url("https://ncode.syosetu.com/")));
        assert!(!adapter().can_handle(&url("https://ncode.syosetu.com/novelview/")));
        assert!(!adapter().can_handle(&url("https://ncode.syosetu.com/novelview/infotop/ncode/")));
        assert!(!adapter().can_handle(&url("https://ncode.syosetu.com/n2267be/static/")));
    }

    #[test]
    fn another_site_is_not_claimed() {
        assert!(!adapter().can_handle(&url("https://www.example.com/n2267be/")));
    }

    #[test]
    fn an_ncode_is_recognised_in_both_shapes() {
        assert_eq!(
            Syosetu::ncode(&url("https://ncode.syosetu.com/n2267be/")).as_deref(),
            Some("n2267be")
        );
        assert_eq!(
            Syosetu::ncode(&url(
                "https://ncode.syosetu.com/novelview/infotop/ncode/n9525ii/"
            ))
            .as_deref(),
            Some("n9525ii")
        );
        // An older work, whose ncode is all digits.
        assert_eq!(
            Syosetu::ncode(&url("https://ncode.syosetu.com/n9636x/")).as_deref(),
            Some("n9636x")
        );
        assert_eq!(
            Syosetu::ncode(&url("https://ncode.syosetu.com/robots.txt")),
            None
        );
    }

    #[test]
    fn an_episode_url_is_built_from_the_ordinal() {
        assert_eq!(
            Syosetu::episode_url("n2267be", 42),
            "https://ncode.syosetu.com/n2267be/42/"
        );
        assert_eq!(
            Syosetu::list_url("n2267be", 1),
            "https://ncode.syosetu.com/n2267be/"
        );
        assert_eq!(
            Syosetu::list_url("n2267be", 3),
            "https://ncode.syosetu.com/n2267be/?p=3"
        );
    }

    #[test]
    fn the_date_the_info_page_writes_is_parsed_as_jst() {
        let parsed = parse_date("2012年 04月20日 21時58分").expect("the fixture date must parse");

        // The site's times are Japanese local time. Ten in the evening JST is
        // 13:58 the same day in UTC, so the instant is what has to be right — and
        // the offset is carried rather than discarded.
        assert_eq!(parsed.offset(), JST);
        assert_eq!(parsed.hour(), 21);
        assert_eq!(parsed.to_offset(UtcOffset::UTC).hour(), 12);
    }

    #[test]
    fn the_date_an_episode_row_writes_is_parsed_too() {
        let parsed = parse_date("2012/04/20 21:58").expect("the fixture date must parse");
        assert_eq!(
            (parsed.year(), parsed.month() as u8, parsed.day()),
            (2012, 4, 20)
        );
        assert_eq!(parsed.offset(), JST);
    }

    #[test]
    fn a_date_in_an_unknown_shape_is_absent_rather_than_guessed() {
        assert!(parse_date("sometime").is_none());
        assert!(parse_date("").is_none());
        // A month that is not a month must not become one.
        assert!(parse_date("2012年 13月20日 21時58分").is_none());
    }

    #[test]
    fn a_word_count_is_read_out_of_its_label() {
        assert_eq!(digits_in("9,666,529文字"), Some(9_666_529));
        assert_eq!(digits_in("全795エピソード"), Some(795));
        assert_eq!(digits_in("no digits"), None);
    }

    #[test]
    fn the_status_labels_are_read() {
        // The two the fixtures exercise.
        assert_eq!(work_status("連載中"), WorkStatus::Ongoing);
        assert_eq!(work_status("短編"), WorkStatus::Complete);
        // The site's other labels. Unverified by a fixture — see the function.
        assert_eq!(work_status("完結済"), WorkStatus::Complete);
        assert_eq!(work_status("休載中"), WorkStatus::Hiatus);
        // And an unrecognised label is not guessed at.
        assert_eq!(work_status("なにか"), WorkStatus::Unknown);
        assert_eq!(work_status(""), WorkStatus::Unknown);
    }

    #[test]
    fn the_capability_a_retry_needs_is_advertised() {
        let capabilities = adapter().capabilities();
        assert!(capabilities.metadata);
        assert!(capabilities.chapters);
        assert!(capabilities.per_chapter_fetch);
        // The author page's layout has not been recorded.
        assert!(!capabilities.bibliography);
    }

    #[test]
    fn only_the_one_host_is_listed() {
        assert_eq!(adapter().hosts(), vec![HOST.to_owned()]);
    }

    #[test]
    fn the_pace_is_the_one_the_site_publishes() {
        // `Crawl-delay: 1` in Syosetu's robots.txt. The fetcher prefers the
        // file's number over this one; this is the adapter stating what it
        // expects, and it matches.
        assert_eq!(adapter().capabilities().min_interval_millis, Some(1_000));
    }
}
