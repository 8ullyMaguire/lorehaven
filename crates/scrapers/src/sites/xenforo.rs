//! XenForo forums, and the one place in this project where our own user agent
//! is load-bearing.
//!
//! # Not one site: software
//!
//! A XenForo forum holds fanfiction as *threads* — a forum thread whose posts are
//! the chapters, with the ordered chapter list supplied by the **SV Threadmarks**
//! add-on. Three hosts matter, and they are three sources rather than one,
//! because the walls and the policies differ. Measured on 2026-09-11:
//!
//! | Host | `robots.txt` | Plain request | Declared wall |
//! |---|---|---|---|
//! | `forums.spacebattles.com` | 200, `Allow: /` for `*` | **200**, the real page | [`Wall::None`] |
//! | `forums.sufficientvelocity.com` | byte-identical to the above | **200**, the real page | [`Wall::None`] |
//! | `forum.questionablequesting.com` | **404, no file** | **200**, the real page | [`Wall::None`] |
//!
//! The three disagree about `robots.txt` — two publish one, one publishes none,
//! and the two that publish are byte-identical — which is why they are three
//! sources rather than one.
//!
//! # The wall that was measured wrong, twice
//!
//! SpaceBattles was first recorded as [`Wall::Solver`]: `403`, *Just a moment*, to
//! a plain request **and** to a browser fingerprint. The second measurement
//! disagreed, and the difference was the client's own consistency rather than the
//! host's policy:
//!
//! | Client | Result |
//! |---|---|
//! | `Lorehaven/{version} (+import)` — this fetcher's own agent | **200**, the real 128 KB page |
//! | `curl/8.0` | **200**, the real page |
//! | a Chrome `User-Agent` over plain TLS | **403**, *Just a moment* |
//!
//! A request that says what it is gets served; one that claims to be a browser
//! without behaving like one gets challenged. So the probe *caused* the wall it
//! reported: my reconnaissance sent a browser agent from a non-browser client, and
//! the 403 was Cloudflare noticing exactly that.
//!
//! Declaring [`Wall::Solver`] on that evidence would have been wrong in the
//! expensive direction — every instance without a solver refuses to import from
//! SpaceBattles before queueing, for a source that answers plainly. All three
//! hosts declare [`Wall::None`], a challenge is still escalated when the instance
//! has a solver, and [`Wall::None`] is documented as a *starting* point rather
//! than a claim that no wall exists.
//!
//! # Our own user agent decides whether we may read at all
//!
//! Both published files are `Allow: /` for `User-agent: *` followed by about
//! ninety lines that disallow individual crawlers **by name** — `GPTBot`,
//! `ClaudeBot`, `anthropic-ai`, `CCBot`, `Bytespider`, `PerplexityBot`,
//! `AhrefsBot` and the rest. `RobotsRules::parse` is called with
//! `product_token(&policy.user_agent)`, and this fetcher's default agent is
//! `Lorehaven/{version} (+import)`, whose token is `Lorehaven`. So the claim this
//! adapter rests on is that **our token is not one of those names**.
//!
//! Nothing enforces that. If the default user agent were ever changed to
//! something containing one of those strings — a well-meant "be explicit about
//! being an AI scraper" edit would do it — every XenForo import on that instance
//! would be refused by `robots.txt`, and the refusal would read as a wall rather
//! than as a mistake in our own configuration. This paragraph is here to be
//! found by whoever makes that change.
//!
//! # The three documents an import reads
//!
//! All three are addressed by the thread id alone — the slug is cosmetic and the
//! site resolves `/threads/{id}/...` directly, which was measured rather than
//! assumed, and is why a stored address never goes stale when an author renames
//! a thread:
//!
//! 1. **`/threads/{id}/threadmarks`** — the chapter list. Its header states
//!    `Created`, `Status` and, critically, **`Threadmarks: N`**, the total. Its
//!    items are `div.structItem--threadmark`, one per chapter, in reading order.
//! 2. **`/threads/{id}/`** — the title, the summary, the author's profile link
//!    and the thread's tags.
//! 3. **`/threads/{id}/post-{post}`** — one chapter's prose. The add-on has no
//!    per-post endpoint: this returns **the whole thread page** with that post
//!    anchored, which is why a post page is about 1.8 MB for a few kilobytes of
//!    prose. That cost is unavoidable and is stated here rather than discovered
//!    by an operator watching their traffic; `FetchPolicy::max_bytes` is the
//!    guard.
//!
//! # The chapter count is checkable, which is what makes this safe
//!
//! The header states the number of threadmarks, so a list that does not add up
//! to it is a list this adapter did not understand. The recorded work states 42
//! and arrives as 25 + 17 — and the 25 is not a coincidence: **`per_page` is
//! clamped**. Asking for `per_page=1000` returns **25** items, the site's default,
//! with no error and no warning. An adapter that asked for a large page and
//! trusted the answer would import 25 chapters of 42 and report success. This one
//! asks for 100 (which is honoured) and checks the total against the header
//! regardless, so a clamp in either direction is caught rather than silently
//! obeyed.
//!
//! # The status label differs between hosts
//!
//! The site enumerates its own vocabulary in its forum filters —
//! `incomplete`, `complete`, `hiatus`, `dropped` — and renders the **visible
//! label** in the header, which is not the machine value:
//!
//! * SpaceBattles and SufficientVelocity write **`Ongoing`**.
//! * QuestionableQuesting writes **`Incomplete`** for the same state.
//!
//! Both mean the work is being added to. `Dropped` is the site's word for what
//! the domain calls [`WorkStatus::Cancelled`]; the mapping is in [`status_of`],
//! and a label this build has never seen is [`WorkStatus::Unknown`] rather than a
//! guess — a guess would put an abandoned work in a reader's "in progress" list
//! or the reverse.
//!
//! # What is deliberately not read
//!
//! * **The per-chapter word count.** The threadmark list states one, but it is
//!   **abbreviated** — `1.7k`, `1.3k`, `960` — so it cannot be summed into a
//!   work's total, and reading `1.7k` as a number would be worse than not reading
//!   it. `word_count` is therefore `None` for this source: the forum does not
//!   state one and this adapter will not invent one.
//! * **The first post as a summary.** On the recorded work the first post is the
//!   author's index post; on many other threads the first post **is** the first
//!   chapter. Using it would put a chapter in the summary field on those. The
//!   site's own `og:description` is used instead, which is one field and means
//!   the same thing on every thread.
//! * **Tags as the theme renders them.** A tag anchor carries a category icon
//!   whose `<title>` is a label — `Setting battletech` is one tag, not two words.
//!   The category is stripped.
//!
//! # Provenance
//!
//! Written against pages recorded from the live sites on 2026-09-11; see
//! `tests/fixtures/xenforo/` and the `## xenforo` section of
//! `tests/fixtures/README.md`.

use async_trait::async_trait;
use scraper::{Html, Selector};
use url::Url;

use crate::sanitize::sanitize_fragment;
use crate::{
    attr_of, collapse_whitespace, html_of, text_of, AuthKind, ChapterRef, Credentials, Fetcher,
    SourceAdapter, SourceCapabilities, SourceChapter, SourceError, SourceKey, SourceResult,
    SourceWork, Wall, WorkStatus,
};

/// SpaceBattles.
pub const SPACEBATTLES_KEY: &str = "spacebattles";
/// Sufficient Velocity.
pub const SUFFICIENT_VELOCITY_KEY: &str = "sufficientvelocity";
/// Questionable Questing.
pub const QUESTIONABLE_QUESTING_KEY: &str = "questionablequesting";

/// The hosts, bare, for the fetcher's allow-list.
pub const SPACEBATTLES_HOST: &str = "forums.spacebattles.com";
/// Sufficient Velocity's host.
pub const SUFFICIENT_VELOCITY_HOST: &str = "forums.sufficientvelocity.com";
/// Questionable Questing's host.
pub const QUESTIONABLE_QUESTING_HOST: &str = "forum.questionablequesting.com";

/// The pace.
///
/// No `Crawl-delay` is published by any of the three hosts — one carries no
/// `robots.txt` at all — so the fetcher's one-second floor is the pace, stated
/// here because it is what an operator should expect.
pub const PACING_MILLIS: u64 = 1_000;

/// How many threadmarks to ask for in one request.
///
/// Measured: `per_page=50` and `per_page=100` are honoured; `per_page=1000` is
/// **silently clamped to 25**, the site's default. 100 is honoured on all three
/// hosts and is the largest value that was verified to be honoured, so it is
/// what this asks for.
const PER_PAGE: usize = 100;

/// A ceiling on list pages, so a thread whose stated count is nonsense cannot
/// turn a preview into an unbounded number of requests.
const MAX_LIST_PAGES: usize = 200;

/// Which forum an adapter is.
///
/// Every difference that is not the shared XenForo reading lives here, so the
/// three adapters cannot drift apart in the parser while claiming to be the same
/// software — and so that each host's wall and policy is stated once, where it
/// belongs, rather than inherited from a sibling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forum {
    /// SpaceBattles: refused plainly, served through a solver.
    SpaceBattles,
    /// Sufficient Velocity: served plainly.
    SufficientVelocity,
    /// Questionable Questing: served plainly, and publishes no `robots.txt`.
    QuestionableQuesting,
}

impl Forum {
    /// This forum's registry key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::SpaceBattles => SPACEBATTLES_KEY,
            Self::SufficientVelocity => SUFFICIENT_VELOCITY_KEY,
            Self::QuestionableQuesting => QUESTIONABLE_QUESTING_KEY,
        }
    }

    /// What this forum is called, for a reader.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::SpaceBattles => "SpaceBattles",
            Self::SufficientVelocity => "Sufficient Velocity",
            Self::QuestionableQuesting => "Questionable Questing",
        }
    }

    /// The host this forum is read from, bare.
    #[must_use]
    pub const fn host(self) -> &'static str {
        match self {
            Self::SpaceBattles => SPACEBATTLES_HOST,
            Self::SufficientVelocity => SUFFICIENT_VELOCITY_HOST,
            Self::QuestionableQuesting => QUESTIONABLE_QUESTING_HOST,
        }
    }

    /// The origin canonical URLs are built on.
    #[must_use]
    pub const fn origin(self) -> &'static str {
        match self {
            Self::SpaceBattles => "https://forums.spacebattles.com",
            Self::SufficientVelocity => "https://forums.sufficientvelocity.com",
            Self::QuestionableQuesting => "https://forum.questionablequesting.com",
        }
    }

    /// The least this host needs before it will serve a page, as measured.
    ///
    /// All three are [`Wall::None`], and the history of that value is worth
    /// keeping, because the first answer was wrong in a way that would have cost
    /// every import on an instance without a solver.
    ///
    /// # SpaceBattles: a wall that depended on the client's own consistency
    ///
    /// SpaceBattles was first measured as [`Wall::Solver`], and it *did* refuse:
    /// `403`, *Just a moment*, a Cloudflare interstitial, to both a plain request
    /// and a browser fingerprint. The measurement was made — and the mistake was
    /// in the request, not the reading. Those probes sent a **browser**
    /// `User-Agent` over non-browser TLS, and that incoherence is what Cloudflare
    /// challenged. Measured again with each client's own honest agent:
    ///
    /// | Client | Result |
    /// |---|---|
    /// | `Lorehaven/{version} (+import)` — this fetcher's own agent | **200**, the real 128 KB page |
    /// | `curl/8.0` | **200**, the real page |
    /// | a Chrome `User-Agent` over plain TLS | **403**, *Just a moment* |
    ///
    /// So this host serves a request that says what it is, and challenges one
    /// that claims to be a browser without behaving like one. Declaring a solver
    /// wall would have been wrong in the expensive direction: every instance
    /// without a solver configured refuses to import from SpaceBattles at all,
    /// before queueing, for a source that answers a plain request.
    ///
    /// Nothing is lost by starting cheap. A challenge encountered at fetch time
    /// is escalated when the instance has a solver configured, and reported as
    /// [`SourceError::Blocked`] when it does not — so if this host tightens, the
    /// import still works on an instance that can run a browser, and says so
    /// plainly on one that cannot, rather than being refused up front forever.
    ///
    /// # The other two
    ///
    /// SufficientVelocity and QuestionableQuesting serve every reading page to a
    /// plain request, measured the same way. Their `robots.txt` files differ from
    /// each other — SufficientVelocity publishes one byte-identical to
    /// SpaceBattles', QuestionableQuesting publishes none at all — which is the
    /// other half of why these are three sources and not one.
    #[must_use]
    pub const fn wall(self) -> Wall {
        match self {
            Self::SpaceBattles | Self::SufficientVelocity | Self::QuestionableQuesting => {
                Wall::None
            }
        }
    }

    /// All three, for the registry and for tests that must cover each host.
    #[must_use]
    pub const fn all() -> [Self; 3] {
        [
            Self::SpaceBattles,
            Self::SufficientVelocity,
            Self::QuestionableQuesting,
        ]
    }
}

/// One of the three forums, as a [`SourceAdapter`].
#[derive(Debug, Clone)]
pub struct XenForo {
    forum: Forum,
    key: SourceKey,
    hosts: Vec<String>,
}

impl XenForo {
    /// The SpaceBattles adapter.
    #[must_use]
    pub fn spacebattles() -> Self {
        Self::new(Forum::SpaceBattles)
    }

    /// The Sufficient Velocity adapter.
    #[must_use]
    pub fn sufficient_velocity() -> Self {
        Self::new(Forum::SufficientVelocity)
    }

    /// The Questionable Questing adapter.
    #[must_use]
    pub fn questionable_questing() -> Self {
        Self::new(Forum::QuestionableQuesting)
    }

    /// The adapter for one forum.
    #[must_use]
    pub fn new(forum: Forum) -> Self {
        Self {
            forum,
            key: SourceKey::new(forum.key()),
            hosts: vec![forum.host().to_owned()],
        }
    }

    /// Which forum this is.
    #[must_use]
    pub const fn forum(&self) -> Forum {
        self.forum
    }

    fn host_matches(&self, host: &str) -> bool {
        let host = host.trim_start_matches("www.");
        self.hosts.iter().any(|ours| host == ours)
    }

    /// The thread id in any of the addresses a thread has.
    ///
    /// `https://forums.spacebattles.com/threads/by-the-horns-story-only-thread.262832/`,
    /// the same with `/threadmarks`, `/page-7` or `/post-11149727` appended, and
    /// the bare `/threads/262832/` form all name the same thread. The id is the
    /// digits after the last `.` in the second path segment, or the whole
    /// segment when there is no slug.
    fn thread_id(url: &Url) -> Option<String> {
        let mut segments = url.path_segments()?;
        if segments.next()? != "threads" {
            return None;
        }
        let segment = segments.next()?;
        if segment.is_empty() {
            return None;
        }
        let id: String = match segment.rsplit_once('.') {
            Some((_, id)) => id.chars().take_while(char::is_ascii_digit).collect(),
            // No slug: the segment is the id itself, and it must be all digits —
            // `/threads/threadmarks` and `/threads/post-12` are not threads.
            None => {
                if segment.chars().all(|c| c.is_ascii_digit()) {
                    segment.to_owned()
                } else {
                    String::new()
                }
            }
        };
        (!id.is_empty()).then_some(id)
    }

    /// The post id in a `/posts/{id}/` address.
    ///
    /// The site's own "copy link to post" address, and a thing a reader does
    /// paste. It names a post rather than a thread, so it cannot say which work
    /// it belongs to without being fetched — the site redirects it to the thread
    /// page, and that page names the thread. [`XenForo::preview`] follows that up;
    /// [`thread_id_in_page`] does the reading.
    fn post_address(url: &Url) -> Option<String> {
        let mut segments = url.path_segments()?;
        if segments.next()? != "posts" {
            return None;
        }
        let id = segments.next()?;
        let id: String = id.chars().take_while(char::is_ascii_digit).collect();
        (!id.is_empty()).then_some(id)
    }

    /// A thread's address.
    ///
    /// The id-only form: the site resolves it directly, and it cannot go stale
    /// when an author renames a thread, which the slugged form would.
    #[must_use]
    pub fn thread_url(&self, id: &str) -> String {
        format!("{}/threads/{id}/", self.forum.origin())
    }

    /// One page of a thread's chapter list.
    #[must_use]
    pub fn threadmarks_url(&self, id: &str, page: usize) -> String {
        format!(
            "{}/threads/{id}/threadmarks?per_page={PER_PAGE}&page={page}",
            self.forum.origin()
        )
    }

    /// One chapter's address.
    ///
    /// The add-on serves the containing page with the post anchored; there is no
    /// address that returns only the post.
    #[must_use]
    pub fn post_url(&self, id: &str, post: &str) -> String {
        format!("{}/threads/{id}/post-{post}", self.forum.origin())
    }

    /// Read one page of a thread's chapter list.
    ///
    /// # Errors
    ///
    /// [`SourceError::Parse`] when the page carries no threadmark list at all,
    /// which is what a forum page or an error page would look like.
    pub fn parse_threadmark_page(&self, html: &str, id: &str) -> SourceResult<ThreadmarkPage> {
        let document = Html::parse_document(html);
        let items = threadmark_items(&document);
        let chapters: Vec<ChapterRef> = items
            .iter()
            .enumerate()
            .map(|(index, item)| ChapterRef {
                ordinal: (index as u32) + 1,
                source_chapter_key: item.post.clone(),
                title: item.title.clone(),
            })
            .collect();
        let dates = items.iter().map(|item| item.date).collect();
        let stated = stated_threadmarks(&document);

        // A page with a stated count and no items is not a thread with no
        // chapters: it is a page this adapter did not understand.
        if chapters.is_empty() && stated.is_none() {
            return Err(SourceError::Parse(format!(
                "xenforo thread {id} has no threadmark list on this page"
            )));
        }

        Ok(ThreadmarkPage {
            status: status_of(&status_label(&document)),
            created: header_created(&document),
            stated,
            author: item_author(&document),
            chapters,
            dates,
        })
    }

    /// Read a thread's own page for what the work is.
    ///
    /// # Errors
    ///
    /// [`SourceError::Parse`] when the page has no title, which is what an error
    /// page looks like.
    pub fn parse_thread(&self, html: &str, id: &str) -> SourceResult<ThreadSummary> {
        let document = Html::parse_document(html);
        let title = text_of(&document, "h1.p-title-value").ok_or_else(|| {
            SourceError::Parse(format!("xenforo thread {id} has no title element"))
        })?;

        let author_text = text_of(&document, "div.p-description a.username").unwrap_or_default();
        let author_url = attr_of(&document, "div.p-description a.username", "href")
            .map(|href| absolute(&self.forum, &href));

        // The site's own one-line summary. See the module documentation for why
        // the first post is not used instead.
        let summary =
            attr_of(&document, "meta[property='og:description']", "content").unwrap_or_default();

        Ok(ThreadSummary {
            title,
            author_text,
            author_url,
            summary,
            tags: tag_names(&document),
            canonical: attr_of(&document, "link[rel='canonical']", "href"),
        })
    }

    /// Read one chapter's prose out of a thread page.
    ///
    /// # Errors
    ///
    /// [`SourceError::Parse`] when the page does not carry the post asked for —
    /// the add-on serves the whole thread, so a page that is missing the post is
    /// a page this adapter should refuse rather than store empty.
    pub fn parse_post(
        &self,
        html: &str,
        work: &SourceWork,
        post: &str,
    ) -> SourceResult<SourceChapter> {
        let document = Html::parse_document(html);

        // Scoped to the article for this post: a thread page carries one
        // `.bbWrapper` per post on it — 26 of them on the recorded page — so an
        // unscoped selector returns the first post's prose for every chapter.
        let selector = format!("article[data-content='post-{post}'] .bbWrapper");
        let body = html_of(&document, &selector).ok_or_else(|| {
            SourceError::Parse(format!(
                "xenforo post {post} of thread {} is not on the page served for it",
                work.source_work_key
            ))
        })?;

        let entry = work
            .chapters
            .iter()
            .find(|entry| entry.source_chapter_key == post);

        Ok(SourceChapter {
            ordinal: entry.map_or(0, |entry| entry.ordinal),
            source_chapter_key: post.to_owned(),
            title: entry.map(|entry| entry.title.clone()).unwrap_or_default(),
            content_html: sanitize_fragment(&body, Url::parse(&work.source_url).ok().as_ref()),
            image_urls: crate::sanitize::extract_image_urls(&body, Url::parse(&work.source_url).ok().as_ref()),
        })
    }
}

/// One page of a thread's chapter list.
#[derive(Debug, Clone)]
pub struct ThreadmarkPage {
    /// The status the header states, as a [`WorkStatus`].
    ///
    /// [`WorkStatus::Unknown`] when the header states no status, or one this
    /// build does not recognise — never a default that would claim the work is
    /// being added to.
    pub status: WorkStatus,
    /// When the thread was opened.
    pub created: Option<time::OffsetDateTime>,
    /// How many threadmarks the header says the thread has.
    pub stated: Option<u32>,
    /// The author the items name.
    pub author: Option<String>,
    /// This page's chapters, in reading order.
    pub chapters: Vec<ChapterRef>,
    /// When each chapter was posted, aligned with `chapters`. Carried beside
    /// them rather than on them because the domain's chapter reference has no
    /// date — a chapter's date is not part of a chapter's identity — while a
    /// work's last change is.
    pub dates: Vec<Option<time::OffsetDateTime>>,
}

impl Default for ThreadmarkPage {
    fn default() -> Self {
        Self {
            status: WorkStatus::Unknown,
            created: None,
            stated: None,
            author: None,
            chapters: Vec::new(),
            dates: Vec::new(),
        }
    }
}

impl ThreadmarkPage {
    /// When the work last changed.
    ///
    /// The newest chapter's date: this site's threadmarks are in reading order
    /// and a story thread is appended to as the work continues, so the last
    /// entry is the latest change. The maximum is taken rather than the last
    /// element because a list imported page by page is assembled in order and a
    /// thread whose dates are out of order should still report the newest.
    #[must_use]
    pub fn updated(&self) -> Option<time::OffsetDateTime> {
        self.dates.iter().flatten().max().copied()
    }

    /// When the work was first posted.
    ///
    /// The header's `Created`, falling back to the earliest chapter date — a
    /// thread opened before its first threadmark existed states one, and a thread
    /// whose header is absent has only the chapters.
    #[must_use]
    pub fn published(&self) -> Option<time::OffsetDateTime> {
        self.created
            .or_else(|| self.dates.iter().flatten().min().copied())
    }
}

/// What a thread's own page says about the work.
#[derive(Debug, Clone, Default)]
pub struct ThreadSummary {
    /// The thread's title.
    pub title: String,
    /// The thread starter.
    pub author_text: String,
    /// The thread starter's profile.
    pub author_url: Option<String>,
    /// The site's own summary.
    pub summary: String,
    /// The thread's tags.
    pub tags: Vec<String>,
    /// The address the site states for the thread.
    pub canonical: Option<String>,
}

/// The status a header label means.
///
/// The site's own vocabulary is `incomplete`, `complete`, `hiatus` and `dropped`
/// (read from its forum filters); the label it renders is not the machine value,
/// and two hosts render different words for the same state:
///
/// * `Ongoing` (SpaceBattles, SufficientVelocity) and `Incomplete`
///   (QuestionableQuesting) are the same thing: being added to.
/// * `Dropped` is the site's word for what the domain calls *cancelled*.
///
/// Anything else is [`WorkStatus::Unknown`]: a label this build has never seen is
/// not evidence of any particular state, and guessing would put an abandoned work
/// in a reader's "in progress" list or a finished one in their "reading" list.
#[must_use]
pub fn status_of(label: &str) -> WorkStatus {
    match label.trim().to_lowercase().as_str() {
        "ongoing" | "incomplete" | "in progress" | "in-progress" => WorkStatus::Ongoing,
        "complete" | "completed" | "finished" => WorkStatus::Complete,
        "hiatus" | "on hiatus" => WorkStatus::Hiatus,
        "dropped" | "abandoned" | "cancelled" | "canceled" => WorkStatus::Cancelled,
        _ => WorkStatus::Unknown,
    }
}

/// One item of a threadmark list.
#[derive(Debug, Clone)]
struct ThreadmarkItem {
    /// The post id, which is the chapter's key.
    post: String,
    /// The threadmark label the author wrote.
    title: String,
    /// When the post was made.
    date: Option<time::OffsetDateTime>,
}

/// The thread's chapters, in the page's order.
///
/// The ordinal is the item's position: the add-on renders the list in reading
/// order, and a reader's progress and bookmarks are mapped onto the ordinal.
fn threadmark_items(document: &Html) -> Vec<ThreadmarkItem> {
    let Ok(item_selector) = Selector::parse("div.structItem--threadmark") else {
        return Vec::new();
    };
    let Ok(anchor_selector) = Selector::parse("a[data-tp-primary='on']") else {
        return Vec::new();
    };
    let Ok(time_selector) = Selector::parse("time[datetime]") else {
        return Vec::new();
    };

    document
        .select(&item_selector)
        .filter_map(|item| {
            let anchor = item.select(&anchor_selector).next()?;
            let post = post_id_in_href(anchor.value().attr("href")?)?;
            let date = item
                .select(&time_selector)
                .next()
                .and_then(|time| time.value().attr("datetime"))
                .and_then(parse_forum_datetime);
            Some(ThreadmarkItem {
                post,
                title: collapse_whitespace(&anchor.text().collect::<String>()),
                date,
            })
        })
        .collect()
}

/// The post id in a threadmark item's link.
///
/// The recorded forums disagree about how they write it, and the difference is
/// not cosmetic — reading only one shape loses every chapter on the other host:
///
/// * SpaceBattles and SufficientVelocity write a site-relative path with a
///   fragment: `/threads/by-the-horns-story-only-thread.262832/#post-11149727`.
/// * QuestionableQuesting writes an absolute URL whose id is a path segment:
///   `https://forum.questionablequesting.com/threads/margin-of-error.39359/post-13124301`.
///
/// Both carry the id, so both are read. The fragment is tried first because a
/// path segment could also be something else the theme links (`/post-123/preview`
/// would still be this post, so taking the digits that follow is right either
/// way).
fn post_id_in_href(href: &str) -> Option<String> {
    let tail = href
        .rsplit_once("#post-")
        .map(|(_, id)| id)
        .or_else(|| href.rsplit_once("/post-").map(|(_, id)| id))?;
    let id: String = tail.chars().take_while(char::is_ascii_digit).collect();
    (!id.is_empty()).then_some(id)
}

/// How many threadmarks the header states, if it states one.
fn stated_threadmarks(document: &Html) -> Option<u32> {
    pair_value(document, "Threadmarks").and_then(|value| value.trim().parse().ok())
}

/// When the thread was opened.
fn header_created(document: &Html) -> Option<time::OffsetDateTime> {
    let Ok(selector) = Selector::parse("dl.pairs") else {
        return None;
    };
    let Ok(time_selector) = Selector::parse("time[datetime]") else {
        return None;
    };
    document.select(&selector).find_map(|pair| {
        let label = pair
            .select(&Selector::parse("dt").ok()?)
            .next()
            .map(|dt| collapse_whitespace(&dt.text().collect::<String>()))?;
        if label != "Created" {
            return None;
        }
        pair.select(&time_selector)
            .next()
            .and_then(|time| time.value().attr("datetime"))
            .and_then(parse_forum_datetime)
    })
}

/// The status label the header states.
fn status_label(document: &Html) -> String {
    pair_value(document, "Status").unwrap_or_default()
}

/// The author an item names.
///
/// The threadmark list does not carry a profile link, only the name; the thread
/// page carries both, which is why an import reads it as well.
fn item_author(document: &Html) -> Option<String> {
    let selector = Selector::parse("div.structItem--threadmark").ok()?;
    document
        .select(&selector)
        .next()
        .and_then(|item| item.value().attr("data-content-author"))
        .filter(|author| !author.is_empty())
        .map(str::to_owned)
}

/// The value beside a `<dt>` label in any `dl.pairs` on the page.
fn pair_value(document: &Html, label: &str) -> Option<String> {
    let Ok(selector) = Selector::parse("dl.pairs") else {
        return None;
    };
    let Ok(dt_selector) = Selector::parse("dt") else {
        return None;
    };
    let Ok(dd_selector) = Selector::parse("dd") else {
        return None;
    };
    document.select(&selector).find_map(|pair| {
        let name = pair
            .select(&dt_selector)
            .next()
            .map(|dt| collapse_whitespace(&dt.text().collect::<String>()))?;
        if name != label {
            return None;
        }
        pair.select(&dd_selector)
            .next()
            .map(|dd| collapse_whitespace(&dd.text().collect::<String>()))
    })
}

/// A thread's tags.
///
/// The theme puts a category icon inside each tag's anchor, and the icon's
/// `<title>` is a label — so the anchor's text reads `Setting battletech` for one
/// tag called `battletech`. The category is the anchor's first child element and
/// is stripped from the front, which is what makes this a tag name rather than
/// two words.
fn tag_names(document: &Html) -> Vec<String> {
    let Ok(selector) = Selector::parse("a.tagItem") else {
        return Vec::new();
    };
    document
        .select(&selector)
        .filter_map(|anchor| {
            let full = collapse_whitespace(&anchor.text().collect::<String>());
            if full.is_empty() {
                return None;
            }
            let category = anchor
                .child_elements()
                .next()
                .map(|child| collapse_whitespace(&child.text().collect::<String>()))
                .unwrap_or_default();
            let name = match full.strip_prefix(&category) {
                Some(rest) => rest.trim().to_owned(),
                None => full,
            };
            (!name.is_empty()).then_some(name)
        })
        .collect()
}

/// Parse the timestamp a forum writes.
///
/// The recorded pages write `2013-06-24T23:28:23-0400` — an offset of `-0400`,
/// which RFC 3339 does not allow, so the well-known parser refuses it and the
/// date would be lost rather than wrong. Both forms are tried.
fn parse_forum_datetime(raw: &str) -> Option<time::OffsetDateTime> {
    if let Ok(at) = time::OffsetDateTime::parse(raw, &time::format_description::well_known::Rfc3339)
    {
        return Some(at.to_offset(time::UtcOffset::UTC));
    }
    let format = time::format_description::parse_owned::<2>(
        "[year]-[month]-[day]T[hour]:[minute]:[second][offset_hour sign:mandatory][offset_minute]",
    )
    .ok()?;
    time::OffsetDateTime::parse(raw, &format)
        .ok()
        .map(|at| at.to_offset(time::UtcOffset::UTC))
}

/// The thread a page says it is.
///
/// The forum software puts the container's identity in an attribute on the
/// document element: `data-content-key="thread-148769"`. That is what makes a
/// post-only address resolvable — the page a post redirects to names the thread
/// it was posted in, so no part of the site's URL shape has to be guessed at.
fn thread_id_in_page(html: &str) -> Option<String> {
    let document = Html::parse_document(html);
    let key = attr_of(&document, "html[data-content-key]", "data-content-key")?;
    let id = key.strip_prefix("thread-")?;
    (!id.is_empty() && id.chars().all(|c| c.is_ascii_digit())).then(|| id.to_owned())
}

/// Make a site-relative link absolute.
fn absolute(forum: &Forum, href: &str) -> String {
    if href.starts_with('/') {
        format!("{}{href}", forum.origin())
    } else {
        href.to_owned()
    }
}

impl XenForo {
    /// Put a work together out of the two documents that describe it.
    ///
    /// Public because the live path and the fixture tests must assemble a work
    /// the same way: a test that reassembled these fields itself would be
    /// checking its own arithmetic rather than this adapter's.
    #[must_use]
    pub fn assemble_work(
        &self,
        id: &str,
        thread: ThreadSummary,
        marks: ThreadmarkPage,
    ) -> SourceWork {
        // The list names the author by display name only; the thread page names
        // it with a profile link. The thread page wins when both are present,
        // because it is the one that can name the author's page.
        let author_text = if thread.author_text.is_empty() {
            marks.author.clone().unwrap_or_default()
        } else {
            thread.author_text
        };

        SourceWork {
            source_key: self.key.clone(),
            source_work_key: id.to_owned(),
            source_url: self.thread_url(id),
            title: thread.title,
            author_text,
            author_url: thread.author_url,
            summary: thread.summary,
            // Not stated anywhere the adapter can read: the list's per-chapter
            // counts are abbreviated (`1.7k`) and cannot be summed. See the
            // module documentation.
            word_count: None,
            // The forum states no language for a thread.
            language: None,
            status: marks.status,
            published_at: marks.published(),
            updated_at: marks.updated(),
            chapters: marks.chapters,
            // A forum thread carries no content rating.
            rating_text: None,
            warning_texts: Vec::new(),
            tags: thread.tags,
        }
    }

    /// Read a thread's whole chapter list, paging until it is complete.
    ///
    /// # Errors
    ///
    /// [`SourceError::Parse`] when the pages do not add up to the count the
    /// header states. That check is the whole reason this is safe: the site
    /// **silently clamps** an over-large `per_page` back to its own default, so a
    /// request that looked like it asked for everything can answer with a
    /// fraction, and importing the fraction would report success.
    pub async fn read_threadmark_pages(
        &self,
        fetch: &dyn Fetcher,
        id: &str,
    ) -> SourceResult<ThreadmarkPage> {
        let mut merged: Option<ThreadmarkPage> = None;
        let mut page = 1usize;

        loop {
            let response = fetch.get(&self.threadmarks_url(id, page)).await?;
            let parsed = self.parse_threadmark_page(&response.body, id)?;
            let page_is_short = parsed.chapters.is_empty() || parsed.chapters.len() < PER_PAGE;

            match merged.as_mut() {
                Some(collected) => collected.append(parsed),
                None => merged = Some(parsed),
            }

            let collected = merged.as_ref().map_or(0, |marks| marks.chapters.len());
            let stated = merged.as_ref().and_then(|marks| marks.stated);

            // Stop as soon as the stated count is reached, which is one request
            // for every thread recorded so far.
            if stated.is_some_and(|stated| collected >= stated as usize) {
                break;
            }
            // Without a stated count — a thread whose header carries no
            // `Threadmarks` pair, which one recorded thread does — a short page
            // is the only end condition the site offers.
            if page_is_short {
                break;
            }
            page += 1;
            if page > MAX_LIST_PAGES {
                return Err(SourceError::Internal(format!(
                    "xenforo thread {id} is still listing chapters after {MAX_LIST_PAGES} pages"
                )));
            }
        }

        let marks = merged.unwrap_or_default();

        // The check the module documentation is about.
        if let Some(stated) = marks.stated {
            if marks.chapters.len() != stated as usize {
                return Err(SourceError::Parse(format!(
                    "xenforo thread {id} states {stated} threadmarks and lists {}",
                    marks.chapters.len()
                )));
            }
        }
        if marks.chapters.is_empty() {
            return Err(SourceError::Parse(format!(
                "xenforo thread {id} lists no threadmarks"
            )));
        }
        Ok(marks)
    }
}

impl ThreadmarkPage {
    /// Append another page's worth of chapters, keeping the ordinals dense.
    ///
    /// Each page numbers its own items from one, so a merged list has to be
    /// renumbered: an ordinal is what a reader's progress and notes are keyed
    /// to, and two chapters sharing ordinal 1 would put one of them under the
    /// other's bookmark.
    ///
    /// Public because a caller assembling a list from several recorded pages —
    /// a test, or a retry that resumes from page two — must merge them the same
    /// way [`XenForo::read_threadmark_pages`] does, rather than reimplementing
    /// the renumbering and checking its own arithmetic.
    pub fn append(&mut self, other: ThreadmarkPage) {
        let base = self.chapters.len() as u32;
        for (mut chapter, date) in other.chapters.into_iter().zip(other.dates) {
            chapter.ordinal += base;
            self.chapters.push(chapter);
            self.dates.push(date);
        }
    }
}

#[async_trait]
impl SourceAdapter for XenForo {
    fn key(&self) -> SourceKey {
        self.key.clone()
    }

    fn display_name(&self) -> &'static str {
        self.forum.display_name()
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities {
            metadata: true,
            // Gated on the wall the forum states: SpaceBattles needs a solver
            // for every reading page, the other two need nothing.
            chapters: true,
            // A chapter has its own address — the post — so a retry re-reads one
            // page and touches nothing else. The page is the whole thread, which
            // is the add-on's doing and not something this adapter can narrow.
            per_chapter_fetch: true,
            // Member pages exist and their markup has not been recorded.
            bibliography: false,
            // The chapter list states both the total and each chapter's date, so
            // "has this changed?" costs one request and compares two fields.
            incremental: true,
            authentication: AuthKind::None,
            min_interval_millis: Some(PACING_MILLIS),
        }
    }

    fn wall(&self) -> Wall {
        self.forum.wall()
    }

    fn can_handle(&self, url: &Url) -> bool {
        let Some(host) = url.host_str() else {
            return false;
        };
        // Both the thread addresses and the site's own post-only address, which
        // names a post and is resolved to its thread when it is fetched.
        self.host_matches(host)
            && (XenForo::thread_id(url).is_some() || XenForo::post_address(url).is_some())
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
        // A post-only address names a post, not a thread. The site redirects it
        // to the containing thread page, and that page says which thread it is,
        // so the work is resolved by reading rather than by guessing.
        let (id, already_fetched) = match XenForo::thread_id(url) {
            Some(id) => (id, None),
            None => {
                let post = XenForo::post_address(url).ok_or_else(|| {
                    SourceError::Unsupported(format!("{url} is not a XenForo thread address"))
                })?;
                let response = fetch.get(url.as_str()).await?;
                let id = thread_id_in_page(&response.body).ok_or_else(|| {
                    SourceError::Parse(format!(
                        "xenforo post {post} was served a page that names no thread"
                    ))
                })?;
                (id, Some(response.body))
            }
        };

        // The chapter list first: it is the document that can fail with an
        // answer worth reading — a count that does not add up — and there is no
        // point fetching a thread page for a work that is about to be refused.
        let marks = self.read_threadmark_pages(fetch, &id).await?;
        let thread_html = match already_fetched {
            Some(body) => body,
            None => fetch.get(&self.thread_url(&id)).await?.body,
        };
        let thread = self.parse_thread(&thread_html, &id)?;

        Ok(self.assemble_work(&id, thread, marks))
    }

    async fn fetch_chapters(
        &self,
        fetch: &dyn Fetcher,
        work: &SourceWork,
        _creds: Option<&Credentials>,
    ) -> SourceResult<Vec<SourceChapter>> {
        if work.chapters.is_empty() {
            return Err(SourceError::Parse(format!(
                "xenforo thread {} lists no chapters",
                work.source_work_key
            )));
        }
        let mut chapters = Vec::with_capacity(work.chapters.len());
        for entry in &work.chapters {
            let page = fetch
                .get(&self.post_url(&work.source_work_key, &entry.source_chapter_key))
                .await?;
            chapters.push(self.parse_post(&page.body, work, &entry.source_chapter_key)?);
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
                    "xenforo thread {} has no chapter {ordinal}",
                    work.source_work_key
                ))
            })?;
        let page = fetch
            .get(&self.post_url(&work.source_work_key, &entry.source_chapter_key))
            .await?;
        self.parse_post(&page.body, work, &entry.source_chapter_key)
    }

    fn preview_from_html(&self, html: &str, url: &Url) -> SourceResult<SourceWork> {
        // The fixture seam, and for this source it is half a preview by
        // construction: a XenForo work is described by two documents — the thread
        // page for what the work *is* and the threadmark list for what is *in*
        // it — and the seam is handed one string. This parses the thread page and
        // returns a work with no chapters, which is the honest answer to "what
        // does this document say the work is". A test that wants the chapters
        // calls `parse_threadmark_page` and then `assemble_work`, which is
        // exactly what `preview` does with the two live responses.
        let id = XenForo::thread_id(url).ok_or_else(|| {
            SourceError::Unsupported(format!("{url} is not a XenForo thread address"))
        })?;
        let thread = self.parse_thread(html, &id)?;
        Ok(self.assemble_work(&id, thread, ThreadmarkPage::default()))
    }

    fn chapters_from_html(
        &self,
        html: &str,
        work: &SourceWork,
    ) -> SourceResult<Vec<SourceChapter>> {
        // Every chapter of this work that this document carries, in the work's
        // order. A XenForo post address serves the containing page — the recorded
        // one holds 26 posts — and nothing on the page says which of them was
        // asked for, so "the chapter this page is" would be a guess. What the
        // page indisputably contains is a set of this work's chapters, which is
        // what this returns.
        let document = Html::parse_document(html);
        let Ok(selector) = Selector::parse("article[data-content^='post-']") else {
            return Err(SourceError::Internal("the post selector is invalid".into()));
        };
        let present: std::collections::HashSet<String> = document
            .select(&selector)
            .filter_map(|article| article.value().attr("data-content"))
            .filter_map(|value| value.strip_prefix("post-"))
            .map(str::to_owned)
            .collect();

        let mut chapters = Vec::new();
        for entry in &work.chapters {
            if present.contains(&entry.source_chapter_key) {
                chapters.push(self.parse_post(html, work, &entry.source_chapter_key)?);
            }
        }
        if chapters.is_empty() {
            return Err(SourceError::Parse(format!(
                "xenforo page for thread {} carries none of its chapters",
                work.source_work_key
            )));
        }
        Ok(chapters)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn every_address_a_thread_has_names_the_same_thread() {
        let url = |raw: &str| Url::parse(raw).unwrap();
        // Every form the site links, including the ones a reader is most likely
        // to paste: a threadmark list, a chapter, and a later page.
        for raw in [
            "https://forums.spacebattles.com/threads/by-the-horns-story-only-thread.262832/",
            "https://forums.spacebattles.com/threads/by-the-horns-story-only-thread.262832/threadmarks",
            "https://forums.spacebattles.com/threads/262832/",
            "https://forums.spacebattles.com/threads/262832/threadmarks?per_page=100&page=2",
            "https://forums.spacebattles.com/threads/262832/post-11149727",
            "https://forums.spacebattles.com/threads/262832/page-7#post-113679890",
        ] {
            assert_eq!(XenForo::thread_id(&url(raw)).as_deref(), Some("262832"), "{raw}");
        }

        // A slug that itself contains a dot: the id is what follows the *last*
        // one, which is the site's own shape.
        assert_eq!(
            XenForo::thread_id(&url(
                "https://forums.spacebattles.com/threads/a.title.with.dots.1333472/"
            ))
            .as_deref(),
            Some("1333472")
        );
    }

    #[test]
    fn an_address_that_names_no_thread_is_not_claimed() {
        let url = |raw: &str| Url::parse(raw).unwrap();
        for raw in [
            "https://forums.spacebattles.com/",
            "https://forums.spacebattles.com/forums/creative-writing.18/",
            "https://forums.spacebattles.com/members/master-arminas.28195/",
            "https://forums.spacebattles.com/threads/",
            // Under /threads/ but not a thread: no digits after the dot.
            "https://forums.spacebattles.com/threads/by-the-horns/",
        ] {
            assert!(
                XenForo::thread_id(&url(raw)).is_none(),
                "{raw} must not name a thread"
            );
        }
        // And an address on another host is not this adapter's, even when it
        // looks the same: the three forums are three sources.
        let elsewhere = url("https://forums.sufficientvelocity.com/threads/x.123/");
        assert!(!XenForo::spacebattles().can_handle(&elsewhere));
        assert!(XenForo::sufficient_velocity().can_handle(&elsewhere));
    }

    #[test]
    fn the_sites_own_post_address_is_claimed_and_resolves_to_its_thread() {
        // `Copy link to post` gives `/posts/{id}/`, which is a real thing a
        // reader pastes. It names a post rather than a thread, so the adapter
        // claims it and resolves it by reading the page it redirects to.
        let url = Url::parse("https://forums.sufficientvelocity.com/posts/39100025/").unwrap();
        let adapter = XenForo::sufficient_velocity();
        assert!(adapter.can_handle(&url));
        assert_eq!(XenForo::post_address(&url).as_deref(), Some("39100025"));
        // It names no thread on its own, which is why it has to be fetched.
        assert_eq!(XenForo::thread_id(&url), None);

        // A recorded page states which thread it is.
        assert_eq!(
            thread_id_in_page(
                "<html data-content-key=\"thread-148769\" data-template=\"thread_view\">"
            )
            .as_deref(),
            Some("148769")
        );
        // A page that is not a thread is not one.
        assert_eq!(
            thread_id_in_page("<html data-content-key=\"forum-19\">"),
            None
        );
        assert_eq!(thread_id_in_page("<html>"), None);
    }

    #[test]
    fn a_threadmark_link_is_read_in_both_of_the_shapes_the_forums_write() {
        // SpaceBattles and SufficientVelocity: a path with a fragment.
        assert_eq!(
            post_id_in_href("/threads/by-the-horns-story-only-thread.262832/#post-11149727")
                .as_deref(),
            Some("11149727")
        );
        // QuestionableQuesting: an absolute URL with the id as a path segment.
        // Reading only the fragment shape loses every chapter on that host.
        assert_eq!(
            post_id_in_href(
                "https://forum.questionablequesting.com/threads/margin-of-error.39359/post-13124301"
            )
            .as_deref(),
            Some("13124301")
        );
        // The theme's other links are not chapters.
        assert_eq!(post_id_in_href("/threads/margin-of-error.39359/"), None);
        assert_eq!(post_id_in_href("/forums/creative-writing.18/"), None);
        assert_eq!(post_id_in_href("#post-"), None);
    }

    #[test]
    fn every_host_starts_plain_and_the_expensive_wall_is_not_declared() {
        // All three were measured to serve a plain request under this fetcher's
        // own user agent, including SpaceBattles — which was *first* recorded as
        // `Wall::Solver` on evidence produced by sending a browser agent from a
        // non-browser client. See the module documentation.
        //
        // The assertion is deliberately `None` and not "whatever it currently is":
        // declaring a solver wall on a source that answers plainly makes every
        // instance without a solver refuse to import from it at all, before
        // queueing, so a host must not be declared expensive without a
        // measurement that stands on its own.
        for forum in Forum::all() {
            assert_eq!(forum.wall(), Wall::None, "{forum:?}");
        }

        // Each forum is its own source, with its own key and host.
        let mut keys: Vec<&str> = Forum::all().iter().map(|forum| forum.key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), 3);
        let mut hosts: Vec<&str> = Forum::all().iter().map(|forum| forum.host()).collect();
        hosts.sort_unstable();
        hosts.dedup();
        assert_eq!(hosts.len(), 3);

        // And an address built by one is not claimed by another.
        let sb = XenForo::spacebattles();
        let sv = XenForo::sufficient_velocity();
        assert!(sb.can_handle(&Url::parse(&sb.thread_url("262832")).unwrap()));
        assert!(!sv.can_handle(&Url::parse(&sb.thread_url("262832")).unwrap()));
    }

    #[test]
    fn the_status_labels_two_hosts_render_differently_mean_the_same_thing() {
        // SpaceBattles and SufficientVelocity write `Ongoing`; Questionable
        // Questing writes `Incomplete` for the same state. Both are read from
        // the recorded pages.
        assert_eq!(status_of("Ongoing"), WorkStatus::Ongoing);
        assert_eq!(status_of("Incomplete"), WorkStatus::Ongoing);
        assert_eq!(status_of("  Ongoing "), WorkStatus::Ongoing);

        assert_eq!(status_of("Complete"), WorkStatus::Complete);
        assert_eq!(status_of("Completed"), WorkStatus::Complete);
        assert_eq!(status_of("Hiatus"), WorkStatus::Hiatus);
        // The site's word for an abandoned work is `Dropped`; the domain's is
        // `Cancelled`.
        assert_eq!(status_of("Dropped"), WorkStatus::Cancelled);
        assert_eq!(status_of("Abandoned"), WorkStatus::Cancelled);

        // A label this build has never seen is not evidence of any state.
        for unseen in ["", "Locked", "Archived", "Moved"] {
            assert_eq!(status_of(unseen), WorkStatus::Unknown, "{unseen}");
        }
    }

    #[test]
    fn a_forum_timestamp_is_read_with_its_own_offset() {
        // The recorded pages write `-0400`, which RFC 3339 does not allow, so the
        // well-known parser refuses it and the date would be lost rather than
        // wrong. Both forms are accepted.
        assert_eq!(
            parse_forum_datetime("2013-06-24T23:28:23-0400"),
            Some(datetime!(2013-06-25 03:28:23 UTC))
        );
        // The colon form is RFC 3339 and takes the same path.
        assert_eq!(
            parse_forum_datetime("2013-06-25T03:28:23+00:00"),
            Some(datetime!(2013-06-25 03:28:23 UTC))
        );
        // An eastern offset moves the other way.
        assert_eq!(
            parse_forum_datetime("2026-09-11T12:00:00+0300"),
            Some(datetime!(2026-09-11 09:00:00 UTC))
        );
        assert_eq!(parse_forum_datetime("not a timestamp"), None);
    }

    #[test]
    fn a_tag_is_read_without_the_category_the_theme_prefixes_to_it() {
        // The theme puts a category icon inside the anchor and its <title> is a
        // label, so the anchor's text reads `Setting battletech` for one tag
        // called `battletech`.
        let document = Html::parse_document(
            "<a class=\"tagItem tagItem--tag_battletech\" href=\"/forums/x.18/?tags[0]=battletech\">\
               <i class=\"fa--xf fal fa-globe\"><svg><title>Setting</title></svg></i> battletech\
             </a>\
             <a class=\"tagItem tagItem--tag_alt-power\" href=\"/forums/x.18/?tags[0]=alt-power\">\
               <i class=\"fa--xf fal fa-globe\"><svg><title>Setting</title></svg></i> Alt-Power\
             </a>",
        );
        assert_eq!(tag_names(&document), vec!["battletech", "Alt-Power"]);
    }

    #[test]
    fn the_threadmarks_addresses_are_built_on_the_forums_own_origin() {
        let sb = XenForo::spacebattles();
        assert_eq!(
            sb.thread_url("262832"),
            "https://forums.spacebattles.com/threads/262832/"
        );
        assert_eq!(
            sb.threadmarks_url("262832", 2),
            "https://forums.spacebattles.com/threads/262832/threadmarks?per_page=100&page=2"
        );
        assert_eq!(
            sb.post_url("262832", "11149727"),
            "https://forums.spacebattles.com/threads/262832/post-11149727"
        );
        // The same paths on the other two hosts.
        assert!(XenForo::sufficient_velocity()
            .thread_url("148769")
            .starts_with("https://forums.sufficientvelocity.com/"));
        assert!(XenForo::questionable_questing()
            .thread_url("39359")
            .starts_with("https://forum.questionablequesting.com/"));

        // Everything built is an address the adapter claims.
        for forum in Forum::all() {
            let adapter = XenForo::new(forum);
            let url = Url::parse(&adapter.threadmarks_url("1", 1)).unwrap();
            assert!(adapter.can_handle(&url), "{url} must be claimed");
        }
    }

    #[test]
    fn a_second_page_of_chapters_is_renumbered_rather_than_restarting_at_one() {
        // Each page numbers its own items from one. An ordinal is what a
        // reader's progress and notes are keyed to, so two chapters sharing
        // ordinal 1 would put one under the other's bookmark.
        let page = |posts: &[&str]| ThreadmarkPage {
            chapters: posts
                .iter()
                .enumerate()
                .map(|(index, post)| ChapterRef {
                    ordinal: (index as u32) + 1,
                    source_chapter_key: (*post).to_owned(),
                    title: String::new(),
                })
                .collect(),
            dates: vec![None; posts.len()],
            ..ThreadmarkPage::default()
        };

        let mut first = page(&["a", "b"]);
        first.append(page(&["c"]));
        assert_eq!(
            first
                .chapters
                .iter()
                .map(|c| (c.ordinal, c.source_chapter_key.as_str()))
                .collect::<Vec<_>>(),
            vec![(1, "a"), (2, "b"), (3, "c")]
        );
        assert_eq!(first.dates.len(), first.chapters.len());
    }

    #[test]
    fn a_works_dates_come_from_the_header_and_from_the_last_chapter() {
        let marks = ThreadmarkPage {
            created: Some(datetime!(2013-06-24 23:27:50 UTC)),
            chapters: vec![ChapterRef {
                ordinal: 1,
                source_chapter_key: "a".into(),
                title: String::new(),
            }],
            dates: vec![Some(datetime!(2013-06-25 03:28:23 UTC))],
            ..ThreadmarkPage::default()
        };
        assert_eq!(marks.published(), Some(datetime!(2013-06-24 23:27:50 UTC)));
        assert_eq!(marks.updated(), Some(datetime!(2013-06-25 03:28:23 UTC)));

        // With no header date the earliest chapter is the publication.
        let no_header = ThreadmarkPage {
            created: None,
            dates: vec![
                Some(datetime!(2021-11-30 22:06:09 UTC)),
                Some(datetime!(2013-07-02 04:57:24 UTC)),
            ],
            ..ThreadmarkPage::default()
        };
        assert_eq!(
            no_header.published(),
            Some(datetime!(2013-07-02 04:57:24 UTC))
        );
        assert_eq!(
            no_header.updated(),
            Some(datetime!(2021-11-30 22:06:09 UTC))
        );
    }

    #[test]
    fn the_word_count_is_not_invented_from_abbreviated_chapter_counts() {
        // The list states `1.7k` for a chapter. Read as a number that is 1, and
        // the work's total would be nonsense; the forum states no total at all.
        let adapter = XenForo::spacebattles();
        let work = adapter.assemble_work(
            "262832",
            ThreadSummary {
                title: "By The Horns".into(),
                ..ThreadSummary::default()
            },
            ThreadmarkPage::default(),
        );
        assert_eq!(work.word_count, None);
        assert_eq!(work.language, None);
        assert_eq!(work.rating_text, None);
    }

    #[test]
    fn the_thread_pages_author_wins_over_the_lists_name() {
        // The list names the author by display name only; the thread page names
        // it with a profile link, so it is the better record when both are there.
        let adapter = XenForo::spacebattles();
        let marks = ThreadmarkPage {
            author: Some("master arminas".into()),
            ..ThreadmarkPage::default()
        };

        let with_thread = adapter.assemble_work(
            "262832",
            ThreadSummary {
                title: "By The Horns".into(),
                author_text: "master arminas".into(),
                author_url: Some(
                    "https://forums.spacebattles.com/members/master-arminas.28195/".into(),
                ),
                ..ThreadSummary::default()
            },
            marks.clone(),
        );
        assert_eq!(with_thread.author_text, "master arminas");
        assert_eq!(
            with_thread.author_url.as_deref(),
            Some("https://forums.spacebattles.com/members/master-arminas.28195/")
        );

        // A thread page with no author falls back to the list's name rather
        // than storing an empty author.
        let without_thread = adapter.assemble_work("262832", ThreadSummary::default(), marks);
        assert_eq!(without_thread.author_text, "master arminas");
    }
}
