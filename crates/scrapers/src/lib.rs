//! Source adapters for Lorehaven's importer (spec §11.1, §11.5, §11.7).
//!
//! # What this crate is
//!
//! An adapter knows how to read one site and nothing else. It is handed a URL
//! and optional credentials, and it hands back **data**: a work's metadata, a
//! list of its chapters, chapter bodies. It never sees a database, never writes
//! a row, and never decides whether an import should happen — that decision is
//! the domain crate's (`plan_import`) and the repository's, exactly as
//! spec §11.1 intends ("Adapters use the shared safe fetcher") and the
//! implementation plan spells out ("Never let an adapter see the database").
//!
//! # The one structural rule
//!
//! **An adapter cannot make an HTTP request of its own.** It does not construct
//! a client, does not hold one, and has no way to reach the network except the
//! [`Fetcher`] it is passed for the call. This is deliberate and it is a
//! security boundary, not a style choice: spec §11.5 requires that user-supplied
//! URLs be defended against server-side request forgery, and a guard that every
//! adapter *may* call is a guard that some adapter will not. Here the only
//! constructor for a network-capable fetcher is [`safety::SafeFetcher::new`],
//! which validates every URL, every redirect, and the address it actually
//! connects to before a byte moves.
//!
//! The same seam is what makes the crate testable without a network: a
//! [`safety::FixtureFetcher`] serves recorded pages, and every adapter also
//! exposes `*_from_html` entry points so a parser can be tested against a
//! fixture file with no fetcher at all. See `docs/plans/milestone-06-imports.md`.
//!
//! # Provenance
//!
//! The adapters here were ported from `fanfic-scrapers` (FicNexus's Rust port of
//! FanFicFare's site definitions), which is AGPL-3.0-or-later like this
//! workspace. Two things were deliberately *not* carried across, because they
//! were defects rather than design: a field that stored a work id in the
//! author-id column, and adapters that set a work's published and updated
//! timestamps to the moment of the fetch. See the milestone record for the rest.

pub mod registry;
pub mod robots;
pub mod safety;
pub mod sanitize;
pub mod sites;

use std::fmt;

// Re-exported so a crate that implements `SourceAdapter` writes
// `#[lorehaven_scrapers::async_trait]` and gets the same macro this trait was
// declared with. Two versions of the macro in one build would produce impls
// that do not satisfy the trait.
pub use async_trait::async_trait;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub use registry::Registry;
pub use safety::{FetchPolicy, FixtureFetcher, SafeFetcher};

/// A stable, lowercase identifier for a source (`ao3`, `ffnet`, `xenforo`).
///
/// The key is the value stored in `sources.key`, joined from `library_items`,
/// and carried on an import job's payload. It is part of the stored schema, so
/// renaming one is a migration rather than a code change.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SourceKey(String);

impl SourceKey {
    /// Build a key, normalising it to lowercase.
    ///
    /// # Panics
    /// Never in practice: the assertion below is on a literal in this crate's
    /// own tests. Keys are constants written by hand, not user input — the one
    /// place a key could arrive from outside is `sources.key` read back from the
    /// database, and a malformed one is a schema fault rather than a request.
    #[must_use]
    pub fn new(raw: impl Into<String>) -> Self {
        let raw = raw.into().to_lowercase();
        debug_assert!(
            !raw.is_empty() && raw.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "source key {raw:?} must be non-empty lowercase alphanumeric or dash"
        );
        Self(raw)
    }

    /// The stored form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SourceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// How a source authenticates, when it does (spec §11.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    /// No account is needed to read public works.
    None,
    /// A source-issued API token or key.
    Token,
    /// A username and password, or a session cookie. Adapters that declare this
    /// must document what they store, because spec §11.6 requires explicit
    /// consent for it.
    Password,
    /// A cookie the reader pasted in. Only for sources that offer nothing else.
    SessionCookie,
}

impl AuthKind {
    /// The stored form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Token => "token",
            Self::Password => "password",
            Self::SessionCookie => "session_cookie",
        }
    }
}

/// What an adapter can actually do, declared rather than discovered.
///
/// Capability absence must be visible (spec §11.1), so this is reported to the
/// reader in the source catalogue and stored on the `sources` row. A source that
/// cannot list chapters says so, instead of returning an empty list that reads
/// like a work with no chapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceCapabilities {
    /// Metadata can be read without fetching any chapter body.
    pub metadata: bool,
    /// Chapter bodies can be read.
    pub chapters: bool,
    /// A single chapter can be fetched by ordinal, without re-reading the whole
    /// work. When this is false, a failed chapter cannot be retried on its own
    /// and the import must re-read the work — an honest limit, surfaced at
    /// planning time rather than discovered on retry.
    pub per_chapter_fetch: bool,
    /// The source can enumerate another author's works (spec §11.9).
    pub bibliography: bool,
    /// The source exposes a revision marker (an edit time, a version), so an
    /// update check can answer "unchanged" without reading every chapter.
    pub incremental: bool,
    /// How the source authenticates.
    pub authentication: AuthKind,
    /// A polite minimum gap between requests, when the source documents one.
    /// The fetcher honours it per host regardless of what an adapter declares;
    /// this is the adapter telling the operator what to expect.
    pub min_interval_millis: Option<u64>,
}

impl SourceCapabilities {
    /// A read-only, unauthenticated, whole-work source: the common case, and the
    /// honest default for an adapter that has not said otherwise.
    #[must_use]
    pub const fn public_read() -> Self {
        Self {
            metadata: true,
            chapters: true,
            per_chapter_fetch: false,
            bibliography: false,
            incremental: false,
            authentication: AuthKind::None,
            min_interval_millis: None,
        }
    }
}

/// The life state a source reports for a work.
///
/// `Unknown` is not a synonym for `Ongoing`: a site that does not publish a
/// completion marker must not have its works recorded as unfinished, because
/// the reader's "check for updates" would then keep asking for ever.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkStatus {
    /// Published and still being added to.
    Ongoing,
    /// The author marked it finished.
    Complete,
    /// The author marked it paused.
    Hiatus,
    /// The author abandoned it.
    Cancelled,
    /// The source does not say.
    Unknown,
}

impl WorkStatus {
    /// The stored form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ongoing => "ongoing",
            Self::Complete => "complete",
            Self::Hiatus => "hiatus",
            Self::Cancelled => "cancelled",
            Self::Unknown => "unknown",
        }
    }
}

/// One chapter's identity, without its body.
///
/// A preview returns these: enough to show the reader what they are about to
/// import and how many requests it will take, carrying no text at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChapterRef {
    /// 1-based position in the work. The ordinal is what a reader's progress and
    /// a reader's notes are mapped onto across a re-import, so it must be stable
    /// across runs for an unchanged work.
    pub ordinal: u32,
    /// The source's own chapter identifier, when it has one that is stable.
    /// Used as the dedupe key in `import_chapters`; falls back to the ordinal
    /// when the source has nothing better.
    pub source_chapter_key: String,
    /// The chapter's title as shown by the source. May be empty.
    pub title: String,
}

/// One chapter's body, with the identity of [`ChapterRef`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceChapter {
    /// Position in the work, 1-based.
    pub ordinal: u32,
    /// The source's stable identifier for this chapter.
    pub source_chapter_key: String,
    /// Title as shown by the source.
    pub title: String,
    /// The chapter body as HTML, sanitised on the way out of this crate.
    ///
    /// It is sanitised here rather than at the storage boundary because an
    /// adapter is the only code that knows which parts of a foreign page are
    /// content: a source's navigation chrome, script tags and tracking images
    /// are noise that no downstream sanitiser can distinguish from prose.
    pub content_html: String,
}

/// What a preview learned about a work.
///
/// Timestamps are optional because a source that does not publish them must not
/// have one invented. An earlier version of these adapters set `published` and
/// `updated` to the moment of the fetch, which made every imported work look as
/// though it had been written today; `None` is the honest answer and the
/// storage layer keeps its own `first_imported_at` for the question "when did
/// this arrive here?".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceWork {
    /// Which adapter produced this.
    pub source_key: SourceKey,
    /// The source's own identifier for the work (`21845264`, `12345/thread`).
    /// Together with `source_key` this is the natural key of an import.
    pub source_work_key: String,
    /// The canonical URL, after any normalisation the adapter applied.
    pub source_url: String,
    /// The work's title.
    pub title: String,
    /// The author as the source displays them. Free text, because most sources
    /// have no account concept to point at.
    pub author_text: String,
    /// The author's profile URL, when the page carried one.
    pub author_url: Option<String>,
    /// The summary or description, as plain text.
    pub summary: String,
    /// The source's word count, when it publishes one.
    pub word_count: Option<i64>,
    /// The source's language tag, when it publishes one.
    pub language: Option<String>,
    /// Whether the source says the work is finished.
    pub status: WorkStatus,
    /// When the source says the work was first published.
    pub published_at: Option<OffsetDateTime>,
    /// When the source says the work last changed.
    pub updated_at: Option<OffsetDateTime>,
    /// The chapter list. Empty when the source could not enumerate chapters even
    /// though [`SourceCapabilities::chapters`] is true — which is the signal the
    /// import treats as a parse failure, not as an empty work.
    pub chapters: Vec<ChapterRef>,
    /// The source's rating string, as it displays it (`Teen And Up Audiences`).
    /// Kept as text; mapping it onto Lorehaven's own rating scale is a
    /// classification decision (spec §12.2), not a scraping one.
    pub rating_text: Option<String>,
    /// The source's warnings or content labels, as displayed.
    pub warning_texts: Vec<String>,
    /// Tags as display text. Carried but not mapped: Lorehaven's taxonomy and
    /// its typed query language arrive in Milestone 9, and inventing tag types
    /// here would fix a schema this crate has no business fixing.
    pub tags: Vec<String>,
}

impl SourceWork {
    /// How many chapters a full import would have to read.
    #[must_use]
    pub fn chapter_count(&self) -> usize {
        self.chapters.len()
    }
}

/// Credentials for one source.
///
/// The `Debug` implementation prints no secret material. A credential that
/// reaches a log line is a credential leaked, and the cheapest way to prevent it
/// is to make the type refuse to describe itself — the same rule
/// `crates/app/src/secrets.rs` follows.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    /// The source this credential belongs to.
    pub source_key: SourceKey,
    /// The account name, when the source has one. Not secret.
    pub username: String,
    /// The secret: an API token, a password, or the value of a session cookie.
    pub secret: String,
    /// Whether the credential may be used against adult-rated works, when the
    /// source gates them behind a confirmation the credential itself satisfies.
    pub adult_allowed: bool,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("source_key", &self.source_key)
            .field("username", &self.username)
            .field("secret", &"<redacted>")
            .field("adult_allowed", &self.adult_allowed)
            .finish()
    }
}

impl Credentials {
    /// Build a credential.
    #[must_use]
    pub fn new(
        source_key: impl Into<String>,
        username: impl Into<String>,
        secret: impl Into<String>,
    ) -> Self {
        Self {
            source_key: SourceKey::new(source_key),
            username: username.into(),
            secret: secret.into(),
            adult_allowed: false,
        }
    }

    /// Allow this credential to reach adult-rated works.
    #[must_use]
    pub fn with_adult(mut self, allowed: bool) -> Self {
        self.adult_allowed = allowed;
        self
    }
}

/// Why an adapter could not do what it was asked.
///
/// The variants are *operational categories*, because the caller's decision
/// depends on the category and not on the message: a `RateLimited` pauses and
/// retries later, an `AuthRequired` pauses and asks the reader, a `ParseError`
/// fails loudly because the source's markup changed, and a `NotFound` is
/// terminal. Collapsing these into one "scrape failed" is how a 429 becomes a
/// permanently failed import.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceError {
    /// The work does not exist on that source.
    #[error("the source has no work at that URL")]
    NotFound,
    /// The source holds the work and will not serve it: a moderation hold, a
    /// work withdrawn by its author, a takedown in progress.
    ///
    /// Distinct from [`SourceError::NotFound`] because the two send an operator
    /// to different places — one is a typo in a URL and one is a work the source
    /// has stopped publishing — and distinct from [`SourceError::Blocked`]
    /// because a hold is a state the *source* is in rather than a refusal aimed
    /// at us, so no amount of waiting or re-requesting resolves it. An import
    /// that reported a moderation hold as "no work at that URL" would send a
    /// reader looking for a mistake they did not make.
    #[error("the source holds that work but is not serving it: {0}")]
    Withheld(String),
    /// The source is refusing us: a challenge wall, a ban, an IP block.
    #[error("the source refused the request")]
    Blocked,
    /// The request never completed.
    #[error("network failure: {0}")]
    Network(String),
    /// The page arrived and did not contain what it must contain.
    ///
    /// Deliberately loud. A parser that returns zero chapters from a page it
    /// does not recognise produces an empty library item that looks like
    /// success (spec §11.6, plan "A source that changes its HTML must fail
    /// loudly").
    #[error("could not parse the source's page: {0}")]
    Parse(String),
    /// The URL is not one this adapter handles.
    #[error("unsupported source: {0}")]
    Unsupported(String),
    /// The source needs an account, or the credential we hold is not good.
    #[error("authentication required: {0}")]
    AuthRequired(String),
    /// The source asked us to slow down.
    #[error("rate limited by the source: {0}")]
    RateLimited(String),
    /// The URL was rejected before any request was made.
    #[error("refused to fetch: {0}")]
    Refused(String),
    /// An internal fault. Details are logged, not returned to a reader.
    #[error("internal fault: {0}")]
    Internal(String),
}

impl SourceError {
    /// Whether retrying the identical request later could plausibly succeed.
    ///
    /// This is the only question the import job asks of an error, so it is
    /// answered here rather than inferred from the message at the call site.
    #[must_use]
    pub const fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::Network(_) | Self::RateLimited(_) | Self::Blocked
        )
    }

    /// Whether the reader has something to fix before a retry can work.
    ///
    /// Drives the "paused, and here is what to do" state rather than a retry
    /// loop: spec §11.6 requires that an expired credential *pause* the job.
    #[must_use]
    pub const fn needs_the_reader(&self) -> bool {
        matches!(self, Self::AuthRequired(_))
    }

    /// A coarse category for the `import_jobs.report_json` and the job's
    /// `last_error`, so an operator can group failures without parsing prose.
    #[must_use]
    pub const fn category(&self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::Withheld(_) => "withheld",
            Self::Blocked => "blocked",
            Self::Network(_) => "network",
            Self::Parse(_) => "parse",
            Self::Unsupported(_) => "unsupported",
            Self::AuthRequired(_) => "auth_required",
            Self::RateLimited(_) => "rate_limited",
            Self::Refused(_) => "refused",
            Self::Internal(_) => "internal",
        }
    }
}

/// The result type adapters use.
pub type SourceResult<T> = Result<T, SourceError>;

/// What an adapter is given in order to read a page.
///
/// Implemented by [`safety::SafeFetcher`] for real traffic and by
/// [`safety::FixtureFetcher`] for recorded pages. An adapter cannot tell the
/// difference, which is the point: the same parsing code path is exercised by
/// the fixture tests as by production.
#[async_trait]
pub trait Fetcher: Send + Sync {
    /// Retrieve a URL as text.
    ///
    /// Implementations must reject a URL that is not http(s), must not follow a
    /// redirect into a private network, and must bound the response size.
    async fn get(&self, url: &str) -> SourceResult<Fetched>;

    /// Retrieve a URL, telling the source which revision we already hold.
    ///
    /// The default ignores the validators and performs an ordinary fetch, which
    /// is the honest behaviour for a fetcher that cannot do better: a conditional
    /// request is an optimisation, and a fetcher that pretended to make one
    /// would report `Fetched` every time while the caller believed a `304` was
    /// possible. [`safety::SafeFetcher`] overrides this and treats a real `304`
    /// as [`ConditionalFetch::NotModified`].
    async fn get_conditional(
        &self,
        url: &str,
        _known: Option<&RevisionValidators>,
    ) -> SourceResult<ConditionalFetch> {
        Ok(ConditionalFetch::Fetched(self.get(url).await?))
    }

    /// The host this fetcher considers the source's own, for credential
    /// purposes. A credential is offered only to a request to this host, so a
    /// redirect to another origin cannot carry it (spec §11.5: "Avoid forwarding
    /// credentials to unrelated origins").
    fn credential_host(&self) -> Option<&str> {
        None
    }

    /// A POST of an urlencoded form, for the sources whose login is a form.
    /// Default: refused, because most adapters never authenticate.
    async fn post_form(&self, url: &str, _fields: &[(&str, &str)]) -> SourceResult<Fetched> {
        Err(SourceError::Refused(format!(
            "this fetcher does not POST forms ({url})"
        )))
    }
}

/// One page back from a fetch.
#[derive(Debug, Clone)]
pub struct Fetched {
    /// The URL the body actually came from, after redirects. An adapter that
    /// stores a canonical URL must use this rather than the URL it asked for.
    pub final_url: String,
    /// The body text.
    pub body: String,
    /// The `Content-Type` header, when the server sent one.
    pub content_type: Option<String>,
    /// The `ETag` the source gave for this revision, when it gave one.
    ///
    /// Kept so the *next* fetch of this URL can say `If-None-Match` and be
    /// answered `304` instead of re-sending a page nothing has changed on. It
    /// is a validator, not a secret, so it is safe to store beside the cached
    /// bytes (spec §10.4).
    pub etag: Option<String>,
    /// The `Last-Modified` the source gave, when it gave one. The weaker of the
    /// two validators, and useful precisely because a source that sends no ETag
    /// often sends this.
    pub last_modified: Option<String>,
}

/// The validators a page was last seen with.
///
/// A pair rather than a single value because HTTP has two mechanisms and the
/// header that applies depends on which one the source published. Sending both
/// is correct: a server that only understands one ignores the other.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RevisionValidators {
    /// The `ETag` from the previous fetch, when the source sent one.
    pub etag: Option<String>,
    /// The `Last-Modified` from the previous fetch, when the source sent one.
    pub last_modified: Option<String>,
}

impl RevisionValidators {
    /// Whether these validators are worth sending — at least one is set.
    #[must_use]
    pub fn is_usable(&self) -> bool {
        self.etag.is_some() || self.last_modified.is_some()
    }
}

/// What a conditional fetch produced.
///
/// `NotModified` is not a failure and not an empty page: it means the source
/// confirmed the revision we already hold is still current, so the caller should
/// serve what it has. Collapsing it into an error would make every cached read
/// look like a broken source.
#[derive(Debug, Clone)]
pub enum ConditionalFetch {
    /// The source says the revision we hold is still current. Nothing came back
    /// but the confirmation.
    NotModified,
    /// A page came back — either changed, or the source ignored the condition.
    Fetched(Fetched),
}

/// One site, one adapter (spec §11.1).
///
/// Every method takes the [`Fetcher`] rather than holding one, and takes
/// credentials per call rather than at construction, so an adapter is a value
/// with no state and no lifetime beyond the request.
#[async_trait]
pub trait SourceAdapter: Send + Sync {
    /// This adapter's key.
    fn key(&self) -> SourceKey;

    /// What this source is called, for a reader.
    ///
    /// Required rather than defaulted to the key, because the key is a
    /// machine's name for the source and every screen that shows it has to
    /// translate it anyway. A trait that defaulted to `royalroad` would put that
    /// string in front of a reader and leave nobody responsible for it, which is
    /// exactly what happened: the import page fell back to the key on a fresh
    /// instance and read "on royalroad".
    ///
    /// The instance's `sources` row carries its own copy, which an operator may
    /// change. This is what the build calls the source before anybody decides
    /// otherwise.
    fn display_name(&self) -> &'static str;

    /// What this adapter can do.
    fn capabilities(&self) -> SourceCapabilities;

    /// Whether this adapter handles a URL.
    ///
    /// Called with a parsed, already-validated URL, so an adapter matches on
    /// host and path and never has to parse hostile text itself.
    fn can_handle(&self, url: &url::Url) -> bool;

    /// The hosts this adapter reads from, and the only hosts it may reach.
    ///
    /// The importer builds its fetcher's allow-list from this, so a link on a
    /// page cannot get an unrelated origin fetched. An adapter that lists a host
    /// it does not use is widening the importer's reach, not its own, which is
    /// why the registry test asserts every listed host is claimed by the adapter.
    fn hosts(&self) -> Vec<String>;

    /// Read a work's metadata and chapter list, and no chapter bodies.
    ///
    /// This is what `POST /imports/preview` calls, so it must not write
    /// anything anywhere — a preview that has already imported is a trap.
    async fn preview(
        &self,
        fetch: &dyn Fetcher,
        url: &url::Url,
        creds: Option<&Credentials>,
    ) -> SourceResult<SourceWork>;

    /// Read every chapter body of a work.
    async fn fetch_chapters(
        &self,
        fetch: &dyn Fetcher,
        work: &SourceWork,
        creds: Option<&Credentials>,
    ) -> SourceResult<Vec<SourceChapter>>;

    /// Read one chapter body by ordinal.
    ///
    /// Default: refused. Overridden only where the source genuinely addresses a
    /// chapter on its own, and advertised through
    /// [`SourceCapabilities::per_chapter_fetch`] so the import knows before it
    /// needs to. A default that silently re-read the whole work would make the
    /// capability flag a lie.
    async fn fetch_chapter(
        &self,
        _fetch: &dyn Fetcher,
        work: &SourceWork,
        ordinal: u32,
        _creds: Option<&Credentials>,
    ) -> SourceResult<SourceChapter> {
        Err(SourceError::Unsupported(format!(
            "{} cannot fetch chapter {ordinal} of {} on its own",
            self.key(),
            work.source_work_key
        )))
    }

    /// Parse a work's metadata from a page that has already been retrieved.
    ///
    /// The fixture seam. Without it every parser test would need a network, and
    /// a test that reaches the network fails on a plane (spec §11.7 requires
    /// recorded fixtures for each adapter).
    fn preview_from_html(&self, html: &str, url: &url::Url) -> SourceResult<SourceWork> {
        let _ = (html, url);
        Err(SourceError::Unsupported(format!(
            "{} has no fixture entry point",
            self.key()
        )))
    }

    /// Parse chapters from a page that has already been retrieved.
    fn chapters_from_html(
        &self,
        html: &str,
        work: &SourceWork,
    ) -> SourceResult<Vec<SourceChapter>> {
        let _ = (html, work);
        Err(SourceError::Unsupported(format!(
            "{} has no fixture entry point",
            self.key()
        )))
    }

    /// List an author's works, for a bibliography import (spec §11.9).
    /// Default: refused, and reported as such by
    /// [`SourceCapabilities::bibliography`] being false.
    async fn list_author_works(
        &self,
        _fetch: &dyn Fetcher,
        _profile_url: &url::Url,
        _creds: Option<&Credentials>,
    ) -> SourceResult<Vec<String>> {
        Err(SourceError::Unsupported(format!(
            "{} cannot enumerate an author's works",
            self.key()
        )))
    }
}

/// Extract the text of the first element matching `selector`, trimmed.
///
/// A helper rather than a habit: every adapter needs it, and a hundred
/// hand-written copies of `select(..).next().map(|e| e.text().collect::<String>())`
/// is a hundred places for a missing `.trim()` to hide.
#[must_use]
pub fn text_of(document: &scraper::Html, selector: &str) -> Option<String> {
    let selector = scraper::Selector::parse(selector).ok()?;
    document
        .select(&selector)
        .next()
        .map(|element| collapse_whitespace(&element.text().collect::<String>()))
}

/// Extract the text of every element matching `selector`, trimmed.
#[must_use]
pub fn texts_of(document: &scraper::Html, selector: &str) -> Vec<String> {
    let Ok(selector) = scraper::Selector::parse(selector) else {
        return Vec::new();
    };
    document
        .select(&selector)
        .map(|element| collapse_whitespace(&element.text().collect::<String>()))
        .filter(|text| !text.is_empty())
        .collect()
}

/// Extract the inner HTML of the first element matching `selector`.
#[must_use]
pub fn html_of(document: &scraper::Html, selector: &str) -> Option<String> {
    let selector = scraper::Selector::parse(selector).ok()?;
    document.select(&selector).next().map(|e| e.inner_html())
}

/// Extract an attribute of the first element matching `selector`.
#[must_use]
pub fn attr_of(document: &scraper::Html, selector: &str, attribute: &str) -> Option<String> {
    let selector = scraper::Selector::parse(selector).ok()?;
    document
        .select(&selector)
        .next()
        .and_then(|element| element.value().attr(attribute))
        .map(str::to_owned)
}

/// Collapse runs of whitespace and trim.
///
/// Source pages are full of newline-and-indent whitespace inside text nodes;
/// without this every stored title carries the page's formatting.
#[must_use]
pub fn collapse_whitespace(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut pending_space = false;
    for ch in raw.chars() {
        if ch.is_whitespace() {
            pending_space = !out.is_empty();
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(ch);
        }
    }
    out
}

/// Strip every tag from a fragment and collapse its whitespace.
///
/// Used for summaries, which most sources deliver with markup around them and
/// which Lorehaven stores as plain text.
#[must_use]
pub fn strip_tags(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut depth = 0usize;
    for ch in raw.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    collapse_whitespace(&html_unescape(&out))
}

/// Decode the entities a source page is likely to carry in an attribute or a
/// text node. Not a complete HTML5 entity table: the long tail of named
/// entities does not appear in the fields this is used on, and a partial table
/// that is honest is better than a dependency.
#[must_use]
pub fn html_unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(index) = rest.find('&') {
        out.push_str(&rest[..index]);
        let tail = &rest[index..];
        let Some(semi) = tail.find(';').filter(|semi| *semi <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let entity = &tail[1..semi];
        match decode_entity(entity) {
            Some(decoded) => {
                out.push(decoded);
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// One entity body (the text between `&` and `;`).
fn decode_entity(entity: &str) -> Option<char> {
    let named = match entity {
        "amp" => return Some('&'),
        "lt" => return Some('<'),
        "gt" => return Some('>'),
        "quot" => return Some('"'),
        "apos" => return Some('\''),
        "nbsp" => return Some(' '),
        "hellip" => return Some('…'),
        "mdash" => return Some('—'),
        "ndash" => return Some('–'),
        "lsquo" => return Some('‘'),
        "rsquo" => return Some('’'),
        "ldquo" => return Some('“'),
        "rdquo" => return Some('”'),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    let numeric = entity.strip_prefix('#')?;
    let code = match numeric.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => numeric.parse::<u32>().ok()?,
    };
    char::from_u32(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_keys_normalise_to_lowercase() {
        assert_eq!(SourceKey::new("AO3").as_str(), "ao3");
        assert_eq!(SourceKey::new("xen-foro").as_str(), "xen-foro");
        assert_eq!(SourceKey::new("ao3").to_string(), "ao3");
    }

    #[test]
    fn credentials_never_describe_themselves() {
        let creds = Credentials::new("ao3", "reader", "hunter2-the-password");
        let rendered = format!("{creds:?}");
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
        // The username is not secret and is useful in a log line.
        assert!(rendered.contains("reader"), "{rendered}");
    }

    #[test]
    fn error_categories_drive_the_right_decision() {
        // Transient: worth the retry.
        assert!(SourceError::Network("reset".into()).is_transient());
        assert!(SourceError::RateLimited("slow down".into()).is_transient());
        assert!(SourceError::Blocked.is_transient());
        // Not transient: a retry would fail identically.
        assert!(!SourceError::NotFound.is_transient());
        assert!(!SourceError::Parse("no chapter div".into()).is_transient());
        assert!(!SourceError::AuthRequired("expired".into()).is_transient());
        // A hold is the source's own state, not a refusal aimed at us: waiting
        // does not lift it.
        assert!(!SourceError::Withheld("not validated".into()).is_transient());
        // Only one category is the reader's to fix.
        assert!(SourceError::AuthRequired("expired".into()).needs_the_reader());
        assert!(!SourceError::Network("reset".into()).needs_the_reader());
        assert!(!SourceError::Blocked.needs_the_reader());
    }

    #[test]
    fn error_categories_are_stable_strings() {
        assert_eq!(SourceError::NotFound.category(), "not_found");
        assert_eq!(SourceError::Blocked.category(), "blocked");
        assert_eq!(
            SourceError::AuthRequired("x".into()).category(),
            "auth_required"
        );
        assert_eq!(SourceError::Refused("x".into()).category(), "refused");
        assert_eq!(SourceError::Withheld("x".into()).category(), "withheld");
    }

    #[test]
    fn whitespace_collapses_without_eating_words() {
        assert_eq!(collapse_whitespace("  a\n\t b  "), "a b");
        assert_eq!(collapse_whitespace("\n\n"), "");
        assert_eq!(collapse_whitespace("one"), "one");
        // A space between words survives; leading and trailing do not.
        assert_eq!(collapse_whitespace("a  b"), "a b");
    }

    #[test]
    fn tags_are_stripped_from_summaries() {
        assert_eq!(strip_tags("<p>Hello <b>there</b></p>"), "Hello there");
        assert_eq!(strip_tags("no markup"), "no markup");
        assert_eq!(strip_tags("<br/>"), "");
    }

    #[test]
    fn entities_decode_including_numeric() {
        assert_eq!(html_unescape("a &amp; b"), "a & b");
        assert_eq!(html_unescape("&lt;tag&gt;"), "<tag>");
        assert_eq!(html_unescape("caf&#233;"), "café");
        assert_eq!(html_unescape("caf&#xe9;"), "café");
        assert_eq!(html_unescape("100 &euro;"), "100 &euro;");
        // A bare ampersand is not an entity and must survive.
        assert_eq!(html_unescape("Q&A"), "Q&A");
    }

    #[test]
    fn helpers_read_a_document() {
        let document = scraper::Html::parse_document(
            r#"<h1 class="t">  A   Title </h1><div id="b"><p>body</p></div><a href="/x">l</a>"#,
        );
        assert_eq!(text_of(&document, "h1.t").as_deref(), Some("A Title"));
        assert_eq!(html_of(&document, "div#b").as_deref(), Some("<p>body</p>"));
        assert_eq!(attr_of(&document, "a", "href").as_deref(), Some("/x"));
        assert_eq!(text_of(&document, "h2"), None);
        assert_eq!(attr_of(&document, "a", "rel"), None);
    }

    #[test]
    fn work_status_has_a_stored_form() {
        assert_eq!(WorkStatus::Complete.as_str(), "complete");
        assert_eq!(WorkStatus::Unknown.as_str(), "unknown");
        assert_eq!(AuthKind::SessionCookie.as_str(), "session_cookie");
    }

    #[test]
    fn a_work_reports_its_chapter_count() {
        let work = SourceWork {
            source_key: SourceKey::new("ao3"),
            source_work_key: "1".into(),
            source_url: "https://archiveofourown.org/works/1".into(),
            title: "T".into(),
            author_text: "A".into(),
            author_url: None,
            summary: String::new(),
            word_count: None,
            language: None,
            status: WorkStatus::Unknown,
            published_at: None,
            updated_at: None,
            chapters: vec![
                ChapterRef {
                    ordinal: 1,
                    source_chapter_key: "1".into(),
                    title: "One".into(),
                },
                ChapterRef {
                    ordinal: 2,
                    source_chapter_key: "2".into(),
                    title: "Two".into(),
                },
            ],
            rating_text: None,
            warning_texts: Vec::new(),
            tags: Vec::new(),
        };
        assert_eq!(work.chapter_count(), 2);
    }
}
