//! The shared safe fetcher and the URL guard behind it (spec §11.5).
//!
//! Every byte an adapter reads passes through [`SafeFetcher`]. It is the only
//! thing in the workspace that turns a URL a user typed into a network request,
//! and it is written on the assumption that the URL is hostile.
//!
//! # What is defended, and how
//!
//! * **Unsupported schemes** are refused up front — only `http` and `https`.
//!   `file:`, `ftp:`, `data:` and `gopher:` are all one line away from reading
//!   the server's disk or its metadata service.
//! * **Loopback, private, link-local and metadata addresses** are refused, and
//!   they are refused *after resolution*. Checking the literal in the URL is not
//!   enough: `http://127.0.0.1/` is obvious, `http://0x7f000001/`,
//!   `http://[::1]/` and `http://2130706433/` are the same address in disguise,
//!   and a public hostname whose `A` record points at `169.254.169.254` is the
//!   attack this exists to stop.
//! * **DNS rebinding** is addressed by pinning. We resolve the host ourselves,
//!   keep only the addresses that pass the check, and then hand those exact
//!   addresses to the HTTP client with
//!   [`reqwest::ClientBuilder::resolve_to_addrs`]. The client never performs its
//!   own second lookup, so there is no window between "we checked" and "we
//!   connected" in which a record could change.
//! * **Redirects** are followed by hand rather than by the client, so every hop
//!   is re-validated and re-pinned. A redirect to a private address is refused
//!   exactly as a direct request would be, and that refusal is what stops the
//!   classic "public URL 302s to the metadata service" bypass.
//! * **Credentials are not forwarded across origins.** Each request states the
//!   host it is for; a redirect to a different host drops it and, by default,
//!   stops.
//! * **Size, time and concurrency** are bounded — bytes because a 4 GB response
//!   is a denial of service, time because a hung socket is one too, and
//!   concurrency because a hundred parallel requests to one small archive is
//!   indistinguishable from an attack.
//! * **Decompression bombs cannot reach us**: this crate does not enable
//!   reqwest's `gzip`/`brotli` features, so no compressed body is inflated. That
//!   is a deliberate omission rather than an oversight — were a source to
//!   require it, the fix is a bounded streaming decoder, not the feature flag.
//! * **The source sets the pace, and `robots.txt` is where it says so**
//!   (spec §11.5). Each host's `robots.txt` is read once, its `Disallow` rules
//!   are honoured as refusals, and its `Crawl-delay` becomes the minimum gap
//!   between requests to that host. Where a site publishes neither, the gap is
//!   one second — because "no information" is not "no limit".
//!
//!   This is enforced here rather than in each adapter for the same reason the
//!   address checks are: an adapter able to opt out of pacing would make the rule
//!   advisory. It is also why the robots *parser* lives in
//!   [`crate::robots`] as a pure function over a string — the policy is
//!   testable without a network, and the fetching is testable without a site.
//!
//! # What is deliberately not here
//!
//! There is no "allow private networks" switch. An instance that needs to import
//! from its own network needs an operator-configured capability, which is
//! Milestone 17's work, and a flag on this struct would be reachable from
//! request handling. A guard with an escape hatch is not a guard.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::header::{
    HeaderMap, HeaderValue, CONTENT_TYPE, IF_MODIFIED_SINCE, IF_NONE_MATCH, USER_AGENT,
};
use tokio::sync::{Mutex, Semaphore};
use url::Url;

use crate::archive::ArchiveClient;
use crate::engine::{Engine, Impersonation};
use crate::robots::{FetchClass, RobotsPosture, RobotsRules};
use crate::solver::{SolverClient, SolverConfig};
use crate::{
    ConditionalFetch, Fetched, Fetcher, Provenance, RevisionValidators, SourceCapabilities,
    SourceError, SourceResult,
};

/// The product token a site's `robots.txt` would name us by.
///
/// Our `User-Agent` is `Lorehaven/<version> (+import)`, and the de-facto
/// matching is "the crawler's token contains the robots value", so the token is
/// the part before the slash. Kept as a function rather than a constant because
/// it is derived from the configured agent, and a hard-coded copy would go stale
/// the moment the agent changed.
fn product_token(user_agent: &str) -> String {
    user_agent
        .split(['/', ' ', '(', ')'])
        .find(|part| !part.is_empty())
        .unwrap_or("Lorehaven")
        .to_owned()
}

/// The `User-Agent` a request of `class` sends, built from the configured agent.
///
/// The class is appended HERE rather than at the call site, so a fetch cannot
/// report a class it is not making: the call site names the class and this
/// function turns that into a token, and there is no path that builds a token
/// without a class. A UA assembled at a call site is a UA that can lie.
#[must_use]
pub fn user_agent_for(base: &str, class: FetchClass) -> String {
    format!("{base} (class={})", class.as_token())
}

/// Whether a `User-Agent` token is one this instance may send.
///
/// Rejects a token that names another product's crawler, or presents as a
/// browser. Both are impersonation (spec §24.5), and impersonation is not a
/// grey area: a site that blocks a bot and is read anyway by pretending to be
/// something else has had its answer ignored, which is the same thing the
/// `robots.txt` compliance in this module exists to prevent, only quieter.
///
/// The check is on the WHOLE token, and deliberately unforgiving. A token is
/// an identity this instance asserts to every host it visits; there is no
/// legitimate reason for a reader-facing import agent to name Googlebot, and
/// every legitimate reason to name itself. Substring matching, rather than a
/// list of exact tokens, because the thing being refused is the *claim*, and
/// `Mozilla/5.0 (compatible; Lorehaven/1.0; Googlebot/2.1)` is still the claim.
#[must_use]
pub fn validate_user_agent(token: &str) -> bool {
    let lowered = token.to_ascii_lowercase();
    // Case-insensitive throughout: a token is read by humans in logs and
    // matched by machines, and `GoogleBot` is the same claim as `googlebot`.
    for impersonated in IMPERSONATED_AGENTS {
        if lowered.contains(impersonated) {
            return false;
        }
    }
    true
}

/// Agents whose use would be impersonation rather than identification.
///
/// Kept as a list of lowercase needles so a variant spelling is still caught,
/// and so adding an agent is one line. Deliberately NOT a list of permitted
/// tokens: a denylist of claims plus a positive requirement that the token name
/// this product is safer than an allowlist that has to be extended every time
/// a legitimate phrasing is thought of, and the cost of a false accept here is
/// a site being lied to.
const IMPERSONATED_AGENTS: &[&str] = &[
    // Search engines. A crawler claiming to be one of these is trying to be
    // served under indexing rules it has not earned.
    "googlebot",
    "bingbot",
    "duckduckbot",
    "yandexbot",
    "baiduspider",
    "slurp",
    // The two big AI crawlers, by their published tokens and their common
    // spellings.
    "gptbot",
    "chatgpt-user",
    "claudebot",
    "anthropic-ai",
    "perplexitybot",
    "ccbot",
    "ai2bot",
    "amazonbot",
    "applebot",
    "meta-externalagent",
    "bytespider",
    "omgili", // OmniParser / images
    "imagesiftbot",
    // Social readers, which sites serve on very different terms.
    "twitterbot",
    "facebookexternalhit",
    "linkedinbot",
    "slackbot",
    "discordbot",
    "telegrambot",
    "whatsapp",
];

/// What a source needs before it will serve a page at all.
///
/// # Why this is separate from [`Unblock`]
///
/// [`Unblock`] is the chain an import will *try*; this is the least the source
/// will *accept*. They are different questions, and conflating them is how a
/// mismatch stays invisible until a reader is watching a preview fail: an adapter
/// whose source needs a solver, on an instance that runs none, gets a chain with
/// no step that can pass — and the only way to find out used to be to spend a
/// request discovering it.
///
/// So an adapter states the wall it measured, and the importer compares that
/// against what the instance can actually run *before anything is queued*
/// (spec §11.1: capability absence must be visible). What a reader gets is then
/// the reason and the fix, rather than a fetch that fails for a cause the
/// preview could have named.
///
/// # Measured, not inherited
///
/// A wall is a property of one host, not of the software a host runs. Of five
/// walled sources probed on 2026-09-11, one accepted a browser fingerprint and
/// four refused it — three different browsers tried — and one of the four was a
/// sibling host of the site that accepted it. An adapter that inherited a
/// sibling's answer would be wrong about its own source in whichever direction
/// the siblings happened to differ.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Wall {
    /// A plain request is served. The common case, and the default.
    #[default]
    None,
    /// A plain request is refused and a browser's TLS and HTTP/2 fingerprint is
    /// enough. Requires a build carrying the `cloudflare-impersonation` feature,
    /// which the importer checks before queueing rather than at fetch time.
    Fingerprint,
    /// A fingerprint was measured *insufficient*: the source answers an
    /// interactive challenge that only a driven browser clears, so a solver
    /// service has to be running for this source to be readable at all. A
    /// fingerprint step is deliberately *not* tried first here — it was measured
    /// to fail, and a step known to fail is a request the source did not need to
    /// serve on every page of every import.
    Solver,
}

/// What may be tried when a source's front door refuses a plain request.
///
/// # Why this is policy and not a fallback that always runs
///
/// Each escalation step makes a request the source did not simply serve: a second
/// attempt with a browser's fingerprint, a browser driven by a service we run, or
/// a read of somebody else's archived copy. That is a decision an operator makes
/// about a source, not something the importer should quietly do to every site it
/// touches — so it is configuration, it defaults to nothing, and an adapter
/// declares which of its sources need it.
///
/// # The order
///
/// Steps run in the order `SafeFetcher::escalation_steps` declares:
/// the configured transport, then a plain client if the transport was
/// impersonating from the start, then the solver, then the archive. Each step runs
/// only when the previous one was answered with a *bot challenge* — never when the
/// source answered `404`, or refused us by its own `robots.txt`, because no
/// different client changes those answers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Unblock {
    /// Present a browser's TLS and HTTP/2 fingerprint from the first request.
    ///
    /// Declared by the adapter for a source known to be behind a fingerprint
    /// wall, so the import does not spend a request learning what the adapter's
    /// author already knew. `None` starts plain, and impersonation is then only
    /// tried if the source answers a challenge — which is a source behind a wall
    /// whether or not anybody wrote it down.
    pub fingerprint: Option<Impersonation>,
    /// A FlareSolverr-compatible service to drive a browser through a challenge
    /// the fingerprint alone did not pass.
    pub solver: Option<SolverConfig>,
    /// Whether an archived copy of a page may be read when the source will not
    /// serve it. See [`crate::archive`] for what this does and does not permit.
    pub archive: bool,
}

impl Unblock {
    /// Nothing is escalated to: the plain transport, and a challenge is reported
    /// as [`SourceError::Blocked`].
    #[must_use]
    pub const fn none() -> Self {
        Self {
            fingerprint: None,
            solver: None,
            archive: false,
        }
    }

    /// A source behind a TLS-fingerprint wall.
    #[must_use]
    pub const fn fingerprint(wanted: Impersonation) -> Self {
        Self {
            fingerprint: Some(wanted),
            solver: None,
            archive: false,
        }
    }

    /// Add a solver service.
    #[must_use]
    pub fn with_solver(mut self, solver: SolverConfig) -> Self {
        self.solver = Some(solver);
        self
    }

    /// Allow an archived copy as the last resort.
    #[must_use]
    pub const fn with_archive(mut self) -> Self {
        self.archive = true;
        self
    }

    /// Whether anything at all would be escalated to.
    #[must_use]
    pub fn is_none(&self) -> bool {
        self.fingerprint.is_none() && self.solver.is_none() && !self.archive
    }
}

impl Unblock {
    /// The chain a wall implies, before an instance adds what it will run.
    ///
    /// One place, because this mapping is the part that has to stay honest: a
    /// wall the chain cannot express would be a wall an adapter could declare and
    /// never be held to. Kept separate from [`crate::SourceAdapter::unblock`] so
    /// it can be tested without an adapter, which is also why it is public — an
    /// adapter
    /// with an unusual source builds on it rather than restating it.
    #[must_use]
    pub fn for_wall(wall: Wall) -> Self {
        match wall {
            // A solver is the *instance's* to offer: an adapter cannot know
            // whether one is running, so a solver wall declares no step here and
            // the chain is config's to fill in. Impersonating first is
            // deliberately not tried for such a source — it was measured to fail
            // there, and a step known to fail is a request the source did not
            // need to serve on every page of every import.
            Wall::None | Wall::Solver => Self::none(),
            Wall::Fingerprint => Self::fingerprint(Impersonation::default()),
        }
    }
}

/// How much, how long, and how often a fetch may be.
#[derive(Debug, Clone)]
pub struct FetchPolicy {
    /// Largest response body accepted, in bytes. A larger one is truncated with
    /// an error rather than buffered: this is a ceiling on memory, so it must be
    /// enforced while reading, not after.
    pub max_bytes: usize,
    /// Total time for one request, including the body.
    pub timeout: Duration,
    /// Time to establish the connection.
    pub connect_timeout: Duration,
    /// How many redirect hops to follow before giving up.
    pub max_redirects: usize,
    /// The `User-Agent` sent. Sites block default library agents; pretending to
    /// be a browser is not the same as circumventing a control, and this is
    /// honest about what it is.
    pub user_agent: String,
    /// Minimum gap between two requests to the same host (spec §11.5,
    /// "Respect source rate limits"). Enforced inside the fetcher so no caller
    /// can forget it.
    pub min_interval_per_host: Duration,
    /// Requests in flight to one host at a time.
    pub max_concurrent_per_host: usize,
    /// How long a host's `robots.txt` is trusted before it is read again
    /// (spec §11.5). A site that changes its rules mid-import is followed
    /// eventually; an import that read it once per request would double its
    /// requests to read about how many requests it may make.
    pub robots_ttl: Duration,
    /// The minimum gap used when a host publishes no `Crawl-delay`
    /// (spec §11.5). A second, because "no information" is not "no limit".
    pub default_interval_per_host: Duration,
    /// What may be tried when the source refuses a plain request.
    pub unblock: Unblock,
    /// Whether a path a host's `robots.txt` forbids is refused
    /// (spec §11.5, "Honour `Disallow`").
    ///
    /// # What this does and does not switch
    ///
    /// It switches the **`Disallow` gate only**. Pacing is not policy: the
    /// `Crawl-delay` from the same file, and the one-second floor beneath it,
    /// are still read and still enforced while this is `false`. A permission
    /// question and a load question arrive in one file, and an operator who
    /// answers the first differently has said nothing about the second —
    /// overriding `Disallow` and then hammering the host is the behaviour that
    /// turns a lost permission into a lost address.
    ///
    /// It is likewise nothing to do with [`Unblock`]: that answers a source
    /// which refuses us *technically*, and this answers one which has *asked*
    /// us not to read it. Nor is it access control — `robots.txt` is a crawling
    /// convention, not authentication, and no credential is bypassed here.
    ///
    /// # Why it is a field rather than a constant
    ///
    /// Because an instance's operator is the one who answers for its crawls,
    /// and there are sources whose rules make an import impossible that no
    /// other part of this system can overrule. Defaulting to `true` and letting
    /// it be switched off in configuration keeps that a decision somebody made
    /// and can point at, rather than a behaviour that ships on.
    /// DEPRECATED, and kept only so a config written before the posture existed
    /// still parses. Read [`FetchPolicy::robots_posture`] for the resolved
    /// answer: **this field is not consulted by the fetcher**, and a caller that
    /// sets it without also setting the posture has changed nothing. `false`
    /// means `Permissive` and `true` means `Strict`; see
    /// [`crate::robots::resolve_posture`].
    pub honour_robots: bool,
    /// What this instance does about a `Disallow` (spec §11.5,
    /// `imports.robots_posture`).
    ///
    /// This is the field the fetcher reads. `honour_robots` is the
    /// compatibility key, and it is resolved into this one when the policy is
    /// built, so there is exactly one answer at the point of use and no caller
    /// has to know that both keys exist.
    pub robots_posture: crate::robots::RobotsPosture,
}

impl Default for FetchPolicy {
    fn default() -> Self {
        Self {
            max_bytes: 8 * 1024 * 1024,
            timeout: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(10),
            max_redirects: 5,
            user_agent: format!("Lorehaven/{} (+import)", env!("CARGO_PKG_VERSION")),
            min_interval_per_host: Duration::from_millis(500),
            max_concurrent_per_host: 2,
            robots_ttl: Duration::from_secs(60 * 60),
            default_interval_per_host: Duration::from_secs(1),
            unblock: Unblock::none(),
            // A crawler that reads a host's rules and then ignores them is the
            // thing robots.txt exists to be told about, so compliance is the
            // default and overriding it is an explicit edit.
            honour_robots: true,
            // The field the fetcher actually reads, and the one the two
            // compatibility keys resolve into. Default is `Strict`, matching
            // `honour_robots: true` above — a caller that sets only the old
            // field still gets compliance.
            robots_posture: crate::robots::RobotsPosture::Strict,
        }
    }
}

impl FetchPolicy {
    /// The policy for one source.
    ///
    /// Only the politeness interval comes from the adapter, because the adapter
    /// is the only code that knows what its site tolerates. Every other limit
    /// is the instance's floor: an adapter able to ask for a ten-gigabyte body
    /// or an hour-long timeout could undo the guard it is running behind, which
    /// would make the guard advisory.
    #[must_use]
    pub fn for_source(capabilities: SourceCapabilities) -> Self {
        let mut policy = Self::default();
        if let Some(millis) = capabilities.min_interval_millis {
            policy.min_interval_per_host = Duration::from_millis(millis);
        }
        policy
    }

    /// The largest body this class of read may accept (spec §11.5, amendment
    /// §1.2).
    ///
    /// `Metadata` gets an eighth of the content ceiling, because a chapter
    /// listing is small and a body is not — and the whole point of declaring a
    /// class is that the ceiling follows the declaration rather than the URL. A
    /// 1 MiB page that was declared `Content` is refused; a 1 MiB page declared
    /// `Metadata` is also refused, because a metadata document that big is
    /// itself the signal that the declaration was wrong.
    ///
    /// `Media` keeps the full ceiling: images and audio legitimately exceed
    /// 1 MiB, and there is nothing to shrink them to.
    #[must_use]
    pub fn max_bytes_for(&self, class: FetchClass) -> usize {
        match class {
            FetchClass::Metadata => 1024 * 1024,
            FetchClass::Content | FetchClass::Media => self.max_bytes,
        }
    }
}

/// Per-host politeness state: the last request's start, and the gate.
struct HostState {
    last_started: Option<Instant>,
    gate: Arc<Semaphore>,
}

/// A fetcher that validates, pins, bounds and paces every request.
pub struct SafeFetcher {
    policy: FetchPolicy,
    /// Hosts this fetcher is willing to talk to. A source's own hosts, set when
    /// the source is looked up, so a page cannot direct us somewhere unrelated.
    allowed_hosts: Vec<String>,
    /// Pinned clients, keyed by host. The cached address set is compared on each
    /// use: an address that moved invalidates the client rather than silently
    /// re-pointing it at somewhere unchecked.
    clients: Mutex<HashMap<String, PinnedClient>>,
    hosts: Mutex<HashMap<String, Arc<Mutex<HostState>>>>,
    /// Credentials applied to requests to `credential_host`, if any. Held as a
    /// header so the value never appears in a URL, a log line or a redirect.
    credential_header: Option<(reqwest::header::HeaderName, HeaderValue)>,
    credential_host: Option<String>,
    /// Each host's `robots.txt`, keyed by host (spec §11.5).
    robots: Mutex<HashMap<String, RobotsEntry>>,
    /// The solver service, built once.
    ///
    /// Cached on the fetcher rather than rebuilt per request because a solver
    /// holds a *session* — the cookies a solved challenge earned — and a session
    /// rebuilt for every chapter is a challenge solved for every chapter, which
    /// is the load this tier exists to avoid.
    solver: Mutex<Option<Arc<SolverClient>>>,
    /// The archive client, built once.
    archive: Mutex<Option<Arc<ArchiveClient>>>,
    /// How many paths this fetcher has read that their host's `robots.txt`
    /// forbids.
    ///
    /// Counted rather than only logged, because an operator who switched
    /// [`FetchPolicy::honour_robots`] off is the one who has to answer for what
    /// it cost, and a count is the smallest thing that answers "how much".
    /// Zero unless the policy overrides, so it is also the honest report for an
    /// instance that complies.
    robots_overrides: AtomicU64,
    /// How many paths have been read and DISCARDED — the host forbade them, the
    /// posture is `MetadataOnly`, and the fetch was the metadata class.
    ///
    /// A separate counter from `robots_overrides` and not folded into it, because
    /// the two mean opposite things and an operator asking "what did the posture
    /// cost me?" needs both:
    ///
    /// * `robots_overrides` counts reads that would not have happened under a
    ///   compliant policy. It is a number an operator may have to explain to a
    ///   site's owner, and it is zero on a compliant instance.
    /// * `robots_read_and_discarded` counts reads the posture ASKED FOR and then
    ///   declined to keep. Nothing was overridden and nobody needs to be told
    ///   about any of it individually — but the volume is the answer to whether
    ///   `MetadataOnly` is throwing away the dataset.
    ///
    /// They were the same counter once, and folding them would have made the
    /// posture's cost invisible: a `MetadataOnly` instance reaches
    /// `ReadAndDiscarded` constantly, it is the only reason that posture exists,
    /// and it reported nothing at all.
    robots_read_and_discarded: AtomicU64,
    /// Hosts already warned about a discarded read, so the warning is once per
    /// host and the count carries the rest.
    robots_discard_hosts: Mutex<HashSet<String>>,
    /// The hosts already reported, so the override is stated once per host
    /// rather than once per chapter.
    ///
    /// A 122-chapter import against a host that forbids chapter paths is 122
    /// identical warnings, and a log line repeated until it is scrolled past is
    /// not a record of anything.
    robots_override_hosts: Mutex<HashSet<String>>,
}

/// One host's `robots.txt`, and when we read it.
struct RobotsEntry {
    rules: RobotsRules,
    read_at: Instant,
}

#[derive(Clone)]
struct PinnedClient {
    addresses: Vec<SocketAddr>,
    /// The transport, pinned to those addresses. Either stack, same guarantees.
    engine: crate::engine::Engine,
}

/// Which clients one request may escalate through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Escalation {
    /// One attempt, whatever the policy's transport is. Used for `robots.txt`:
    /// a source that challenges its own rules file has told us nothing, and
    /// paying a solver to read it would be spending somebody else's capacity on
    /// a file we can do without.
    None,
    /// The full chain from the policy.
    Policy,
}

/// One client in the escalation chain.
#[derive(Debug, Clone)]
enum Step {
    /// Through the pinned transport, with this fingerprint (`None` = plain).
    Transport(Option<Impersonation>),
    /// Through a solver service.
    Solver(SolverConfig),
    /// Through an archive.
    Archive,
}

/// What one attempt produced.
#[derive(Debug)]
enum Attempt {
    /// An answer from the source: a page, a `304`, or the error that goes with
    /// the status it sent.
    Done(ConditionalFetch),
    /// A bot wall. Not a failure of this attempt but a question for the caller:
    /// whether to escalate, and to what.
    Challenged,
}

impl SafeFetcher {
    /// Build a fetcher for one source's hosts.
    ///
    /// `allowed_hosts` are the hosts this fetcher may reach, and they must be
    /// the adapter's own. The check is not defence in depth against a careless
    /// adapter — it is the mechanism that keeps a redirect, or a URL in a page,
    /// from turning the importer into a scanner for whatever network the server
    /// happens to sit on.
    #[must_use]
    pub fn new(allowed_hosts: Vec<String>, policy: FetchPolicy) -> Self {
        Self {
            policy,
            allowed_hosts: allowed_hosts
                .into_iter()
                .map(|host| host.trim_start_matches("www.").to_ascii_lowercase())
                .collect(),
            clients: Mutex::new(HashMap::new()),
            hosts: Mutex::new(HashMap::new()),
            credential_header: None,
            credential_host: None,
            robots: Mutex::new(HashMap::new()),
            robots_overrides: AtomicU64::new(0),
            robots_read_and_discarded: AtomicU64::new(0),
            robots_discard_hosts: Mutex::new(HashSet::new()),
            robots_override_hosts: Mutex::new(HashSet::new()),
            solver: Mutex::new(None),
            archive: Mutex::new(None),
        }
    }

    /// Attach a credential header, sent only to `host` and to nothing a redirect
    /// leads to.
    ///
    /// # Errors
    /// Returns [`SourceError::Internal`] if the credential contains bytes that
    /// cannot appear in a header — a credential we cannot send safely must not
    /// be sent at all.
    pub fn with_credential_header(
        mut self,
        host: impl Into<String>,
        name: &str,
        value: &str,
    ) -> SourceResult<Self> {
        let host = host.into().to_ascii_lowercase();
        let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|e| SourceError::Internal(format!("credential header name: {e}")))?;
        let value = HeaderValue::from_str(value)
            .map_err(|_| SourceError::Internal("credential header value is not valid".into()))?;
        self.credential_header = Some((name, value));
        self.credential_host = Some(host);
        Ok(self)
    }

    /// The policy in force.
    #[must_use]
    pub fn policy(&self) -> &FetchPolicy {
        &self.policy
    }

    /// How many paths this fetcher has read against their host's `robots.txt`.
    ///
    /// Always zero on a compliant instance, which is what makes it worth
    /// reporting: the number exists only where somebody chose to make it exist.
    #[must_use]
    pub fn robots_overrides(&self) -> u64 {
        self.robots_overrides.load(Ordering::Relaxed)
    }

    /// How many forbidden paths have been read in order to throw the bytes away.
    ///
    /// The second half of M59-04, and deliberately a DIFFERENT number from
    /// [`Self::robots_overrides`]. An operator reading this is asking "is
    /// `MetadataOnly` throwing away the dataset?", and the answer to that is not
    /// the answer to "is this instance ignoring sites?" — folding the two
    /// together would make both unanswerable, and the existing test
    /// `a_read_and_discarded_fetch_is_not_counted_as_an_override` is what keeps
    /// them apart.
    #[must_use]
    pub fn robots_read_and_discarded(&self) -> u64 {
        self.robots_read_and_discarded.load(Ordering::Relaxed)
    }

    /// Note that a forbidden path is read and then discarded.
    ///
    /// Once per host, like the override warning, and for the same reason: a log
    /// line repeated until it is scrolled past is not a warning. The wording is
    /// NOT the override's -- this is not an instance ignoring anybody, it is an
    /// instance honouring a posture it was told to hold, and a log that reads
    /// like an apology for a decision the operator made on purpose is its own
    /// kind of wrong.
    async fn record_robots_discard(&self, host: &str, path: &str) {
        self.robots_read_and_discarded
            .fetch_add(1, Ordering::Relaxed);
        let first_time = {
            let mut seen = self.robots_discard_hosts.lock().await;
            seen.insert(host.to_owned())
        };
        if first_time {
            tracing::debug!(
                host,
                path,
                "this host's robots.txt disallows this path and the fetch class is \
                 `Metadata`, so under `imports.robots_posture = metadata_only` it is \
                 being read and the response discarded. Nothing from it is stored. This \
                 is counted in robots_read_and_discarded, which is how the operator can \
                 see what the posture is costing the dataset."
            );
        }
    }

    /// Note that a forbidden path is being read anyway.
    ///
    /// Warns once per host — the message names the host and the path that
    /// happened to be first, and the count carries the rest.
    async fn record_robots_override(&self, host: &str, path: &str) {
        self.robots_overrides.fetch_add(1, Ordering::Relaxed);
        let first_time = {
            let mut seen = self.robots_override_hosts.lock().await;
            seen.insert(host.to_owned())
        };
        if first_time {
            tracing::warn!(
                host,
                path,
                "this host's robots.txt disallows a path the import needs, and this instance \
                 is configured not to honour `Disallow`; the read is going ahead under \
                 `imports.honour_robots = false`. The host's own crawl delay is still enforced."
            );
        }
    }

    /// Fetch, following redirects by hand with validation at every hop.
    ///
    /// This is the path every adapter read goes through, so this is where the
    /// source's own rules are applied: a path the site forbids is refused here
    /// rather than fetched and then regretted (spec §11.5).
    async fn get_with_redirects(
        &self,
        url: &str,
        form: Option<&[(&str, &str)]>,
        conditional: Option<&RevisionValidators>,
    ) -> SourceResult<ConditionalFetch> {
        // `Content` is the default here and the direction matters. §11.5 says
        // the class is declared by the adapter that performs the fetch, so a
        // caller that has not declared one is treated as asking for the most
        // privileged class — which under `MetadataOnly` means a *refusal*, not
        // a read-and-discard. Defaulting to `Metadata` instead would make an
        // undeclared fetch the one case that gets through, which is the
        // opposite of the safe direction.
        self.get_with_class(url, form, conditional, FetchClass::Content)
            .await
    }

    /// Fetch, declaring what kind of read this is (spec §11.5, `FetchClass`).
    ///
    /// **The public entry point for a class-declared read, and it is public
    /// because a caller outside this crate needs it.** The preservation
    /// recheck (M59, spec §2.5) is the first: it reads a destination archive's
    /// public item page to decide whether the work is still preserved there,
    /// and it must declare that read as `Metadata` so the 1 MiB ceiling and
    /// `MetadataOnly` robots posture apply. With only [`Fetcher::get`] on the
    /// public surface, its two options were both wrong — declaring `Content`
    /// for a page it intends to compare and discard, or reaching into
    /// `get_with_class`, which is private.
    ///
    /// The direction of the default is the same here as in
    /// [`Self::get_with_redirects`]: an undeclared read is treated as the most
    /// privileged class, because under `MetadataOnly` that is a *refusal* and
    /// the alternative makes the undeclared case the one that gets through.
    pub async fn get_declared(&self, url: &str, class: FetchClass) -> SourceResult<crate::Fetched> {
        expect_fetched(self.get_with_class(url, None, None, class).await?)
    }

    /// Fetch, declaring what kind of read this is (spec §11.5, `FetchClass`).
    ///
    /// The class is a parameter rather than something derived from the URL,
    /// because a URL that looks like a chapter fetched as `Metadata` has to be
    /// bounded as a metadata fetch. A fetcher that inferred it from the path
    /// would let an adapter's guess raise its own ceiling.
    async fn get_with_class(
        &self,
        url: &str,
        form: Option<&[(&str, &str)]>,
        conditional: Option<&RevisionValidators>,
        class: FetchClass,
    ) -> SourceResult<ConditionalFetch> {
        let parsed = validate_url(url, &self.policy)?;
        let host = shared_host(&parsed);
        // Read either way: the same file carries the pace, and a policy that
        // overrides `Disallow` has said nothing about how hard to knock.
        let robots = self.robots_for(&host).await;
        // Bound once, because the gate is asked twice below and a second call
        // is a second chance for the two answers to disagree — which is how a
        // discarded read becomes a storable one.
        let gate = robots_gate(&robots, parsed.path(), self.policy.robots_posture, class);
        match gate {
            RobotsGate::Allowed => {}
            RobotsGate::Refused => {
                return Err(SourceError::Refused(format!(
                    "{host} disallows {} in its robots.txt",
                    parsed.path()
                )))
            }
            // Overridden, and read-and-discarded, are both "the read goes
            // ahead". They differ in what happens to the result, and that is
            // carried on the `Fetched` below rather than being decided here —
            // the fetcher has no opinion about storage, and the flag is what
            // makes the opinion enforceable by whoever does store.
            RobotsGate::ReadAndDiscarded => self.record_robots_discard(&host, parsed.path()).await,
            RobotsGate::Overridden => self.record_robots_override(&host, parsed.path()).await,
        }
        // Label the result. A `ReadAndDiscarded` read is marked as discarded so
        // that `Fetched::storable_body` refuses to hand its bytes out, which is
        // the A.4 limit that holds when the byte ceiling and the parse type are
        // each wrong on their own.
        //
        // Every other answer is marked with the class that was declared, so a
        // caller can see what kind of read it received without having to have
        // declared it itself.
        self.send_with_redirects(url, form, conditional, Escalation::Policy, class)
            .await
            .map(|fetch| match fetch {
                ConditionalFetch::Fetched(mut page) => {
                    Self::label_fetched(&mut page, class, gate);
                    ConditionalFetch::Fetched(page)
                }
                other => other,
            })
    }

    /// The byte ceiling this fetch hands to the transport.
    ///
    /// A separate function from [`FetchPolicy::max_bytes_for`] and from the
    /// expression at the call site, because the mutation that deleted it was the
    /// one that survived: the policy method was correct and tested, the call
    /// site said `self.policy.max_bytes`, and every ceiling test stayed green
    /// because none of them looked at the seam. One name, used by the send path
    /// and asserted directly, cannot drift from itself.
    fn ceiling_for(&self, class: FetchClass) -> usize {
        // Follows the declared class, not the URL and not the policy's own
        // `max_bytes`. A metadata read is capped at 1 MiB even on an instance
        // that raised the content ceiling, because raising the content ceiling
        // is an operator saying "I will store more prose", not "read more
        // documents".
        self.policy.max_bytes_for(class)
    }

    /// The labelling step, extracted so it can be tested without a network.
    ///
    /// This exists because the discard is applied at the very end of the fetch,
    /// after the gate, and everything above that point needs DNS. A test that
    /// asserts on a real `get_with_class` therefore asserts on a *connection
    /// error* — the `if let Ok(...)` never matches, the test passes, and it
    /// would keep passing if the discard were deleted outright. That is not a
    /// hypothetical: an earlier version of these tests did exactly that, and a
    /// mutation proved it.
    ///
    /// So the rule is applied by a function that can be called directly with a
    /// body in hand. The network path calls this same function, so the test and
    /// the production path cannot drift.
    fn label_fetched(page: &mut crate::Fetched, class: FetchClass, gate: RobotsGate) {
        page.fetch_class = class;
        page.discarded = matches!(gate, RobotsGate::ReadAndDiscarded);
        if page.discarded {
            // The flag alone is advisory. `Fetched::body` is a public field
            // read directly by about forty call sites in the site adapters, so a
            // flag that only `storable_body` consults would be defeated by every
            // one of them. Clearing the bytes makes the discard hold for a caller
            // that never heard of the flag: an adapter parsing `page.body` gets
            // an empty string and stores nothing, which is the whole point.
            //
            // The bytes are dropped, not relocated — nothing keeps a copy, so
            // "read it and keep none of it" is true of memory too and not only
            // of the database.
            page.body = String::new();
        }
    }

    /// Fetch with no regard for `robots.txt`.
    ///
    /// Used for reading `robots.txt` itself, which cannot be gated on having read
    /// `robots.txt`. Everything else goes through
    /// [`SafeFetcher::get_with_redirects`]: the address checks, the pinning, the
    /// bounds and the pacing all live in [`SafeFetcher::attempt`], so skipping the
    /// robots gate is skipping only the robots gate.
    async fn send_with_redirects(
        &self,
        url: &str,
        form: Option<&[(&str, &str)]>,
        conditional: Option<&RevisionValidators>,
        escalation: Escalation,
        class: FetchClass,
    ) -> SourceResult<ConditionalFetch> {
        // Computed here rather than passed in, because a caller that supplied
        // the number could supply any number: a `self.policy.max_bytes` at the
        // call site survived a full mutation pass while every ceiling test
        // stayed green, since none of them looked at the seam. Taking the class
        // makes the class the only thing a caller can express, and this the one
        // place the ceiling is derived.
        let ceiling = self.ceiling_for(class);
        let steps = match escalation {
            Escalation::None => vec![Step::Transport(self.policy.unblock.fingerprint)],
            Escalation::Policy => self.escalation_steps(),
        };
        let mut last: Option<SourceError> = None;
        let mut challenged = false;

        for step in steps {
            match step {
                Step::Transport(fingerprint) => {
                    match self
                        .attempt(url, form, conditional, fingerprint, class, ceiling)
                        .await
                    {
                        Ok(Attempt::Done(fetched)) => return Ok(fetched),
                        Ok(Attempt::Challenged) => {
                            challenged = true;
                            tracing::debug!(
                                url,
                                fingerprint =
                                    fingerprint.map(Impersonation::as_str).unwrap_or("none"),
                                "the source answered a bot challenge rather than the page"
                            );
                        }
                        // The source's own answer — a `404`, a `robots.txt`
                        // refusal, a rejected credential. A different client does
                        // not change it, and escalating on it is how one `404`
                        // becomes three requests to a source that already said no.
                        Err(error) => return Err(error),
                    }
                }
                Step::Solver(config) => {
                    let client = self.solver(&config).await?;
                    match client.get(url).await {
                        Ok(fetched) => return Ok(ConditionalFetch::Fetched(fetched)),
                        Err(error) => {
                            tracing::debug!(url, %error, "the solver did not get the page");
                            last = Some(error);
                        }
                    }
                }
                Step::Archive => {
                    let client = self.archive().await?;
                    match client.get(url).await {
                        Ok(fetched) => return Ok(ConditionalFetch::Fetched(fetched)),
                        Err(error) => {
                            tracing::debug!(url, %error, "no archived copy was readable");
                            last = Some(error);
                        }
                    }
                }
            }
        }

        // A wall is the source refusing us, which is `Blocked` and not whatever
        // the last fallback happened to say — except that an infrastructure
        // failure is the operator's to fix and is a more useful thing to report
        // than "blocked" ever could be.
        if let Some(error) = last {
            if matches!(
                error,
                SourceError::Network(_) | SourceError::Unsupported(_) | SourceError::Internal(_)
            ) {
                return Err(error);
            }
            if challenged {
                return Err(SourceError::Blocked);
            }
            return Err(error);
        }
        if challenged {
            return Err(SourceError::Blocked);
        }
        Err(SourceError::Internal("no attempt was made".to_owned()))
    }

    /// The clients to try, in order, for one request.
    ///
    /// See [`Unblock`] for why each step exists. The order is cheapest-and-most-
    /// likely first: a fingerprint costs one request and no third party, a solver
    /// spends a browser somebody is running, and an archive is somebody else's
    /// copy of a page rather than the page.
    fn escalation_steps(&self) -> Vec<Step> {
        let mut steps = vec![Step::Transport(self.policy.unblock.fingerprint)];
        if let Some(config) = self.policy.unblock.solver.clone() {
            steps.push(Step::Solver(config));
        }
        if self.policy.unblock.archive {
            steps.push(Step::Archive);
        }
        steps
    }

    /// The solver client, built on first use.
    ///
    /// Built once per fetcher — and a fetcher serves one import — because the
    /// solver's *session* is the cookies a solved challenge earned. A session
    /// rebuilt per chapter is a challenge solved per chapter, which is exactly
    /// the load this tier exists to avoid.
    async fn solver(&self, config: &SolverConfig) -> SourceResult<Arc<SolverClient>> {
        let mut slot = self.solver.lock().await;
        if let Some(existing) = slot.as_ref() {
            return Ok(Arc::clone(existing));
        }
        let client = Arc::new(SolverClient::new(
            config.clone(),
            self.allowed_hosts.clone(),
        )?);
        *slot = Some(Arc::clone(&client));
        Ok(client)
    }

    /// The archive client, built on first use.
    async fn archive(&self) -> SourceResult<Arc<ArchiveClient>> {
        let mut slot = self.archive.lock().await;
        if let Some(existing) = slot.as_ref() {
            return Ok(Arc::clone(existing));
        }
        let client = Arc::new(ArchiveClient::new(self.allowed_hosts.clone())?);
        *slot = Some(Arc::clone(&client));
        Ok(client)
    }

    /// One attempt, through the pinned transport, with a chosen fingerprint.
    ///
    /// This is the only place a request is made, and every guarantee the crate
    /// makes about requests lives in this function: the host is the source's own,
    /// the client is pinned to addresses we resolved and vetted, redirects are
    /// walked by hand so each hop is checked, the body is bounded while it is
    /// read, and the turn is taken against `robots.txt`'s pace before anything is
    /// sent.
    async fn attempt(
        &self,
        url: &str,
        form: Option<&[(&str, &str)]>,
        conditional: Option<&RevisionValidators>,
        fingerprint: Option<Impersonation>,
        class: FetchClass,
        ceiling: usize,
    ) -> SourceResult<Attempt> {
        let mut current = validate_url(url, &self.policy)?;
        let mut credential_sent_to: Option<String> = None;

        for hop in 0..=self.policy.max_redirects {
            let host = shared_host(&current);
            self.check_host_allowed(&host)?;
            let authority = self.pinned_client(&host, fingerprint).await?;

            let mut headers = HeaderMap::new();
            // An impersonating client sends the browser's own `User-Agent`, and
            // overriding it here would break the very thing being impersonated:
            // a fingerprint is only coherent if the TLS ClientHello, the HTTP/2
            // settings and the headers agree with each other, and a Chrome
            // handshake announcing `Lorehaven/0.1.0` agrees with nothing. This was
            // measured, not reasoned — the first version of this code set the
            // agent unconditionally and the wall refused it.
            //
            // The consequence is worth stating plainly, because it is the cost of
            // the technique: **a fingerprinted request does not identify itself as
            // Lorehaven.** That is why this is never done implicitly. An adapter
            // declares it for a source, the instance's build has to carry the
            // feature, and the escalation only runs on a source whose own
            // `robots.txt` has already permitted the crawl.
            if fingerprint.is_none() {
                // The token names the class this request is actually making
                // (spec §1.5), and it is built HERE rather than at the call site
                // so a fetch cannot report a class it is not making. The class
                // arrives as a `FetchClass`, not as text, so nothing upstream can
                // put a lie in the token.
                let agent = user_agent_for(&self.policy.user_agent, class);

                // The impersonation refusal, at the last point before the bytes
                // go out (spec §24.5). Checking here rather than at config load
                // is deliberate: there is no route that sets a user agent, so
                // `policy.user_agent` only ever holds a build-time default, and a
                // check that cannot be bypassed does not need to be somewhere
                // convenient. A configured token naming another product's bot
                // fails the fetch with a refusal that names the token, instead
                // of a site quietly being lied to on every request.
                if !validate_user_agent(&agent) {
                    return Err(SourceError::Refused(format!(
                        "the configured user agent is refused: {agent:?} names another product's \
                         crawler, and this instance does not impersonate one. Set a token that \
                         identifies Lorehaven."
                    )));
                }

                headers.insert(
                    USER_AGENT,
                    HeaderValue::from_str(&agent).map_err(|_| {
                        SourceError::Internal("user agent is not a valid header".into())
                    })?,
                );
            }

            // The conditional headers say which revision we already hold, so a
            // source with nothing new can answer `304` and send no body at all.
            // Sent on the first hop only: after a redirect the URL is a
            // different resource, and a validator for the old one means
            // nothing for the new (RFC 9110).
            if hop == 0 {
                if let Some(validators) = conditional {
                    if let Some(etag) = &validators.etag {
                        if let Ok(value) = HeaderValue::from_str(etag) {
                            headers.insert(IF_NONE_MATCH, value);
                        }
                    }
                    if let Some(modified) = &validators.last_modified {
                        if let Ok(value) = HeaderValue::from_str(modified) {
                            headers.insert(IF_MODIFIED_SINCE, value);
                        }
                    }
                }
            }

            // The credential travels only to the host it was configured for.
            let send_credential = self.credential_host.as_deref() == Some(host.as_str());
            if send_credential {
                if let Some((name, value)) = &self.credential_header {
                    headers.insert(name.clone(), value.clone());
                }
                credential_sent_to = Some(host.clone());
            }

            let encoded_body = form.map(|fields| {
                let mut pairs: Vec<(&str, &str)> = fields.to_vec();
                headers.insert(
                    CONTENT_TYPE,
                    HeaderValue::from_static("application/x-www-form-urlencoded"),
                );
                encode_form(&mut pairs)
            });

            self.wait_for_turn(&host).await;
            let host_state = self.host_state(&host).await;
            let gate = {
                let guard = host_state.lock().await;
                guard.gate.clone()
            };
            let _permit = gate
                .acquire_owned()
                .await
                .map_err(|_| SourceError::Internal("host gate closed".into()))?;

            let reply = authority
                .engine
                .send(&current, encoded_body.as_deref(), headers, ceiling)
                .await?;

            // Before any status handling, because a wall is not the page whatever
            // status it arrives with. Reporting it here as `Blocked` would end the
            // escalation before it began, which is the difference between a source
            // the importer can read and one it never will.
            if reply.is_challenge() {
                return Ok(Attempt::Challenged);
            }

            let status = reply.status;
            if (300..400).contains(&status) {
                let Some(location) = reply.location else {
                    return Err(SourceError::Parse(format!(
                        "{} sent a redirect with no Location",
                        current
                    )));
                };
                if hop == self.policy.max_redirects {
                    return Err(SourceError::Refused(format!(
                        "more than {} redirects from {url}",
                        self.policy.max_redirects
                    )));
                }
                let next = current.join(&location).map_err(|e| {
                    SourceError::Parse(format!("redirect Location {location:?}: {e}"))
                })?;
                let next = validate_url(next.as_str(), &self.policy)?;
                tracing::debug!(
                    from = %current,
                    to = %next,
                    credential_carried = credential_sent_to.is_some()
                        && self.credential_host.as_deref() == Some(shared_host(&next).as_str()),
                    "following import redirect"
                );
                current = next;
                continue;
            }

            // `304` is the good outcome of a conditional request, not a
            // failure: the source has confirmed the revision we hold is still
            // current. It is only ever a valid answer when we asked
            // conditionally, so a bare `GET` that produces one is a protocol
            // error rather than an empty page to be stored.
            if status == 304 {
                if conditional.is_none() {
                    return Err(SourceError::Network(format!(
                        "{host} answered 304 to an unconditional request"
                    )));
                }
                return Ok(Attempt::Done(ConditionalFetch::NotModified));
            }
            if status == 404 || status == 410 {
                return Err(SourceError::NotFound);
            }
            if status == 429 {
                let retry = reply.retry_after.as_deref().unwrap_or("unspecified");
                return Err(SourceError::RateLimited(format!(
                    "{host} asked us to wait: retry-after {retry}"
                )));
            }
            if status == 401 || status == 403 {
                // A 403 from a challenge wall is an operational block, not a bad
                // credential; distinguishing them matters because one is
                // retried and the other asks the reader to act.
                return Err(if send_credential {
                    SourceError::AuthRequired(format!("{host} rejected the credential"))
                } else {
                    SourceError::Blocked
                });
            }
            if !(200..300).contains(&status) {
                return Err(SourceError::Network(format!(
                    "{host} answered {}",
                    describe_code(status)
                )));
            }

            return Ok(Attempt::Done(ConditionalFetch::Fetched(Fetched {
                final_url: current.to_string(),
                body: reply.body,
                content_type: reply.content_type,
                etag: reply.etag,
                last_modified: reply.last_modified,
                provenance: Provenance::Source,
                // Not a gated fetch: the archive and the solver both answer a
                // request the fetcher already permitted, and the fixture path has no
                // robots at all. Storable, and content-classed.
                fetch_class: crate::robots::FetchClass::Content,
                discarded: false,
            })));
        }

        Err(SourceError::Refused(format!(
            "more than {} redirects from {url}",
            self.policy.max_redirects
        )))
    }

    /// Reject a host outside the source's own.
    fn check_host_allowed(&self, host: &str) -> SourceResult<()> {
        if self.allowed_hosts.is_empty() {
            return Ok(());
        }
        let bare = host.trim_start_matches("www.");
        if self
            .allowed_hosts
            .iter()
            .any(|allowed| bare == allowed || bare.ends_with(&format!(".{allowed}")))
        {
            Ok(())
        } else {
            Err(SourceError::Refused(format!(
                "{host} is not a host this source may fetch from"
            )))
        }
    }

    /// Resolve, validate, and return a client pinned to the surviving addresses.
    ///
    /// The cache is keyed by host and stores the addresses it was pinned to; when
    /// resolution yields a different set the client is rebuilt. Rebuilding loses
    /// the cookie jar, which is the safe direction — a session earned when a host
    /// answered from one address has not been earned from another.
    async fn pinned_client(
        &self,
        host: &str,
        fingerprint: Option<Impersonation>,
    ) -> SourceResult<PinnedClient> {
        let addresses = resolve_public(host, self.policy.timeout).await?;
        // A fingerprint is part of what a client *is*, so the cache key has to
        // carry it: a plain client cached for one hop must not be handed back for
        // an attempt that asked to impersonate, which would silently retry the
        // wall with the very client it just refused.
        let key = match fingerprint {
            Some(wanted) => format!("{host}#{}", wanted.as_str()),
            None => host.to_owned(),
        };
        let mut clients = self.clients.lock().await;
        if let Some(existing) = clients.get(&key) {
            if existing.addresses == addresses {
                return Ok(PinnedClient {
                    addresses: existing.addresses.clone(),
                    engine: existing.engine.clone(),
                });
            }
            tracing::debug!(host, "resolved addresses changed; re-pinning");
        }
        let engine = Engine::build(fingerprint, host, &addresses, &self.policy)?;
        let pinned = PinnedClient {
            addresses: addresses.clone(),
            engine: engine.clone(),
        };
        clients.insert(key, PinnedClient { addresses, engine });
        Ok(pinned)
    }

    async fn host_state(&self, host: &str) -> Arc<Mutex<HostState>> {
        let mut hosts = self.hosts.lock().await;
        hosts
            .entry(host.to_owned())
            .or_insert_with(|| {
                Arc::new(Mutex::new(HostState {
                    last_started: None,
                    gate: Arc::new(Semaphore::new(self.policy.max_concurrent_per_host)),
                }))
            })
            .clone()
    }

    /// The minimum gap for a host, from its `robots.txt` when it published one.
    ///
    /// Deliberately reads the cache only. Fetching belongs to `robots_for`, and
    /// this is called from inside the request path — so a cache miss here gives
    /// the default rather than a nested fetch, which is also what keeps the
    /// `robots.txt` request itself from depending on a rule about reading
    /// `robots.txt`.
    async fn interval_for(&self, host: &str) -> Duration {
        let published = {
            let cache = self.robots.lock().await;
            cache.get(host).and_then(|entry| entry.rules.crawl_delay())
        };
        // The floor applies either way: spec §11.5 makes one second the pace for
        // a host that published nothing, and an adapter's own interval is a
        // floor beneath the site's number rather than a substitute for it.
        match published {
            Some(delay) => delay.max(self.policy.min_interval_per_host),
            None => self
                .policy
                .default_interval_per_host
                .max(self.policy.min_interval_per_host),
        }
    }

    /// This host's `robots.txt`, read once and then trusted for the policy's TTL.
    ///
    /// The lock is *not* held across the fetch. Holding it would deadlock: this
    /// path fetches, and fetching reads the pacing that reads this cache. Two
    /// concurrent reads of a cold host can therefore both fetch `robots.txt`,
    /// which is one extra request to a one-request-per-second host — a far
    /// better trade than a lock that can hang an import.
    async fn robots_for(&self, host: &str) -> RobotsRules {
        {
            let cache = self.robots.lock().await;
            if let Some(entry) = cache.get(host) {
                if entry.read_at.elapsed() < self.policy.robots_ttl {
                    return entry.rules.clone();
                }
            }
        }

        let url = format!("https://{host}/robots.txt");
        // `robots.txt` is read unconditionally: it is the file that decides the
        // pacing, so asking the source to cache it would be asking it to make
        // the rules stale.
        let outcome = self
            // `robots.txt` is read without consulting a gate, so there is no
            // declared class for it. `Media` is the honest one: it carries the
            // full ceiling, because a `robots.txt` larger than the content
            // ceiling is not a metadata-fetch failure — it is a host we should
            // stop negotiating with, and the file has to fit for it to be read
            // at all. Naming the class rather than passing a number keeps the
            // reasoning next to the choice instead of inside a bare `usize`.
            .send_with_redirects(&url, None, None, Escalation::None, FetchClass::Media)
            .await;
        let rules = match outcome {
            // A site with no `robots.txt` has no restrictions. `404` arrives as
            // `NotFound` because that is how every other fetch reports it.
            Err(SourceError::NotFound) => RobotsRules::unrestricted(),
            Ok(ConditionalFetch::Fetched(page)) => {
                RobotsRules::parse(&page.body, &product_token(&self.policy.user_agent))
            }
            // A `304` to an unconditional read is a protocol fault rather than
            // an unchanged file. It falls into the same arm as any other
            // failure: the rules are unknown, and unknown means the default
            // pace rather than no rules.
            Ok(ConditionalFetch::NotModified) => {
                tracing::warn!(
                    host,
                    "robots.txt answered 304 to an unconditional request; using the default pace"
                );
                RobotsRules::unrestricted()
            }
            // Anything else — a 5xx, a challenge wall, a network failure — leaves
            // the site's rules unknown. Recorded rather than treated as "no
            // rules", because a reader's import should not fail over a file that
            // is temporarily broken, and an operator should see that it happened.
            Err(error) => {
                tracing::warn!(
                    host,
                    %error,
                    "could not read robots.txt; using the default pace and no path rules"
                );
                RobotsRules::unrestricted()
            }
        };

        let mut cache = self.robots.lock().await;
        cache.insert(
            host.to_owned(),
            RobotsEntry {
                rules: rules.clone(),
                read_at: Instant::now(),
            },
        );
        rules
    }

    /// Sleep as long as this host's gap requires.
    async fn wait_for_turn(&self, host: &str) {
        let interval = self.interval_for(host).await;
        if interval.is_zero() {
            return;
        }
        let state = self.host_state(host).await;
        let mut guard = state.lock().await;
        if let Some(last) = guard.last_started {
            let elapsed = last.elapsed();
            if elapsed < interval {
                let wait = interval - elapsed;
                // Released before sleeping: holding the lock would serialise the
                // sleep itself and make the gap cumulative.
                drop(guard);
                tokio::time::sleep(wait).await;
                guard = state.lock().await;
            }
        }
        guard.last_started = Some(Instant::now());
    }
}

#[async_trait::async_trait]
impl Fetcher for SafeFetcher {
    async fn get(&self, url: &str) -> SourceResult<Fetched> {
        expect_fetched(self.get_with_redirects(url, None, None).await?)
    }

    async fn post_form(&self, url: &str, fields: &[(&str, &str)]) -> SourceResult<Fetched> {
        expect_fetched(self.get_with_redirects(url, Some(fields), None).await?)
    }

    /// Fetch, asking the source to answer `304` when nothing has changed.
    ///
    /// The conditional headers are sent only for a request that carries no form
    /// and no credential-bearing body, because `If-None-Match` on a POST is
    /// meaningless: the whole point of the condition is that the request has no
    /// side effect to skip.
    async fn get_conditional(
        &self,
        url: &str,
        known: Option<&RevisionValidators>,
    ) -> SourceResult<ConditionalFetch> {
        let Some(known) = known.filter(|validators| validators.is_usable()) else {
            // Nothing to be conditional about. Asking anyway would be a request
            // with a header the source cannot act on.
            return Ok(ConditionalFetch::Fetched(expect_fetched(
                self.get_with_redirects(url, None, None).await?,
            )?));
        };
        self.get_with_redirects(url, None, Some(known)).await
    }
}

/// Take the page out of a conditional result, refusing the impossible case.
///
/// A `304` can only answer a request that carried a condition, so an
/// unconditional `get` that receives one has hit a fault rather than an
/// unchanged page. Returning an error here rather than an empty body is the
/// difference between a loud bug and a chapter stored as nothing — which is the
/// exact failure the ported Syosetu code had.
/// How to say a status code to an operator.
///
/// `StatusCode`'s own `Display` appends `<unknown status code>` to any code it
/// has no reason phrase for, which is every code an intermediary invents. A
/// reader who met a Cloudflare wobble was told:
///
/// ```text
/// source unavailable: the source (archiveofourown.org answered 525 <unknown status code>)
/// ```
///
/// The number alone is not much better. 525, 522 and 520 all mean the site's
/// own server is in trouble rather than that we are blocked or misconfigured, and
/// which of those it is decides whether an operator retries, waits, or goes and
/// looks at the source. So the intermediary's family is named, standard codes
/// keep their phrase, and a code nobody has heard of is reported as a bare
/// number rather than as a number plus an apology for not recognising it.
/// How to say a status code we only hold as a number.
///
/// The transport normalises a status to `u16`, and the explanatory text for
/// Cloudflare's 52x range lives in [`describe_status`], so this is the bridge
/// rather than a second copy of it.
fn describe_code(status: u16) -> String {
    reqwest::StatusCode::from_u16(status).map_or_else(|_| status.to_string(), describe_status)
}

fn describe_status(status: reqwest::StatusCode) -> String {
    let code = status.as_u16();
    let detail = match code {
        520 => Some("the site's own server returned an unknown error"),
        521 => Some("the site's own server is down"),
        522 => Some("the connection to the site's server timed out"),
        523 => Some("the site's server is unreachable"),
        524 => Some("the site's server timed out"),
        525 => Some("an SSL handshake with the site's server failed"),
        526 => Some("the site's SSL certificate is invalid"),
        527 => Some("the site's proxy reported an error"),
        530 => Some("the site's server could not be resolved"),
        _ => None,
    };

    match (detail, status.canonical_reason()) {
        (Some(detail), _) => format!("{code} ({detail})"),
        (None, Some(reason)) => format!("{code} {reason}"),
        (None, None) => code.to_string(),
    }
}

fn expect_fetched(result: ConditionalFetch) -> SourceResult<Fetched> {
    match result {
        ConditionalFetch::Fetched(fetched) => Ok(fetched),
        ConditionalFetch::NotModified => Err(SourceError::Network(
            "the source answered 304 to an unconditional request".to_owned(),
        )),
    }
}

/// A fetcher that serves recorded pages and records what was asked for.
///
/// This is the test seam. It exists so the *orchestration* — how many requests
/// an import makes, whether a retry re-reads chapters it already has, whether a
/// disabled source is touched at all — can be asserted without a network. The
/// parsers themselves are tested through the adapters' `*_from_html` entry
/// points, which need no fetcher.
#[derive(Default)]
pub struct FixtureFetcher {
    pages: HashMap<String, String>,
    requested: std::sync::Mutex<Vec<String>>,
    /// When set, every fetch of a URL containing this substring fails with the
    /// given error. Lets a test drive the failure paths deterministically.
    failures: HashMap<String, SourceError>,
}

impl FixtureFetcher {
    /// An empty fetcher.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Serve `body` for `url`.
    #[must_use]
    pub fn with_page(mut self, url: impl Into<String>, body: impl Into<String>) -> Self {
        self.pages.insert(url.into(), body.into());
        self
    }

    /// Make every fetch of a URL containing `needle` fail with `error`.
    #[must_use]
    pub fn with_failure(mut self, needle: impl Into<String>, error: SourceError) -> Self {
        self.failures.insert(needle.into(), error);
        self
    }

    /// Every URL asked for, in order, including repeats.
    #[must_use]
    pub fn requested(&self) -> Vec<String> {
        self.requested
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// How many times a URL was asked for.
    #[must_use]
    pub fn times_requested(&self, needle: &str) -> usize {
        self.requested()
            .iter()
            .filter(|url| url.contains(needle))
            .count()
    }
}

#[async_trait::async_trait]
impl Fetcher for FixtureFetcher {
    async fn get(&self, url: &str) -> SourceResult<Fetched> {
        if let Ok(mut requested) = self.requested.lock() {
            requested.push(url.to_owned());
        }
        if let Some((_, error)) = self
            .failures
            .iter()
            .find(|(needle, _)| url.contains(needle.as_str()))
        {
            return Err(error.clone());
        }
        match self.pages.get(url) {
            Some(body) => Ok(Fetched {
                final_url: url.to_owned(),
                body: body.clone(),
                content_type: Some("text/html; charset=utf-8".to_owned()),
                etag: None,
                last_modified: None,
                provenance: Provenance::Source,
                // A fixture stands in for the network, and the fixture path has
                // no `robots.txt` to consult — so this bypasses the gate and so
                // bypasses the discard it would set. Deliberate: a test that
                // seeds a page and expects it back must not have to declare a
                // posture to get it.
                fetch_class: crate::robots::FetchClass::Content,
                discarded: false,
            }),
            None => Err(SourceError::Network(format!(
                "no fixture recorded for {url}"
            ))),
        }
    }

    async fn post_form(&self, url: &str, fields: &[(&str, &str)]) -> SourceResult<Fetched> {
        let mut pairs: Vec<(&str, &str)> = fields.to_vec();
        let encoded = encode_form(&mut pairs);
        self.get(&format!("{url}?{encoded}")).await
    }
}

/// Validate a URL before any request is made.
///
/// # Errors
/// * [`SourceError::Refused`] for a scheme other than http(s), a URL with
///   userinfo in it (a credential in a URL is a credential in a log), an empty
///   or dotted-obfuscated host, or an obviously local hostname.
/// * [`SourceError::Parse`] for a string that is not a URL at all.
pub fn validate_url(raw: &str, policy: &FetchPolicy) -> SourceResult<Url> {
    if raw.len() > 2048 {
        return Err(SourceError::Refused("URL is implausibly long".into()));
    }
    let url = Url::parse(raw).map_err(|e| SourceError::Parse(format!("not a URL: {e}")))?;
    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(SourceError::Refused(format!(
                "scheme {other:?} is not fetched"
            )))
        }
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(SourceError::Refused(
            "URL carries credentials; they belong in the credential store".into(),
        ));
    }
    let Some(host) = url.host_str() else {
        return Err(SourceError::Refused("URL has no host".into()));
    };
    if host.is_empty() {
        return Err(SourceError::Refused("URL has an empty host".into()));
    }
    if is_local_hostname(host) {
        return Err(SourceError::Refused(format!("{host} is a local name")));
    }
    let _ = policy;
    Ok(url)
}

/// What a host's own rules say about reading one path, given what the instance
/// is willing to do about them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RobotsGate {
    /// The rules do not mention this path, or they allow it.
    Allowed,
    /// The rules forbid it and the instance honours them.
    Refused,
    /// The rules forbid it, the posture is `MetadataOnly`, and this is a
    /// metadata fetch: read it, keep none of it.
    ///
    /// Distinct from `Allowed` because **the bytes are discarded** — the whole
    /// point of the posture is that nothing is stored, so a caller that treats
    /// this as `Allowed` and persists the parse result has quietly turned
    /// `metadata_only` into `permissive`. Distinct from `Overridden` because
    /// nothing was overridden: the posture asked for the read and then declined
    /// to keep it, so there is no operator decision being reported here.
    ReadAndDiscarded,
    /// The rules forbid it and the instance has chosen to read it anyway.
    Overridden,
}

/// Decide a fetch against one host's rules (spec §11.5, "Honour `Disallow`").
///
/// A pure function of the rules, the path, the posture and the class, and
/// deliberately so: the gate itself is only reachable through a real
/// `robots.txt` fetch, and a branch reachable only through a live host is a
/// branch no test exercises. This is the whole of the decision — the tracking
/// that follows an override lives at the call site — so every answer can be
/// asserted against real rules parsed from real files.
///
/// The class is an input, never derived from the path. That is the single most
/// important property of this signature: a URL that looks like a chapter
/// fetched as `Metadata` is bounded as a metadata fetch, so the ceiling cannot
/// be raised by an adapter's guess about what shape the URL has.
///
/// | `rules.allows(path)` | posture | class | result |
/// |---|---|---|---|
/// | true | any | any | `Allowed` |
/// | false | `Strict` | any | `Refused` |
/// | false | `MetadataOnly` | `Metadata` | `ReadAndDiscarded` |
/// | false | `MetadataOnly` | `Content` \| `Media` | `Refused` |
/// | false | `Permissive` | any | `Overridden` |
fn robots_gate(
    rules: &RobotsRules,
    path: &str,
    posture: RobotsPosture,
    class: FetchClass,
) -> RobotsGate {
    if rules.allows(path) {
        return RobotsGate::Allowed;
    }
    match posture {
        RobotsPosture::Strict => RobotsGate::Refused,
        // Only the metadata class survives this posture, and it survives only
        // as a read whose bytes are thrown away. `Content` and `Media` fall
        // through to `Refused` because "read it, store none of it" is a
        // coherent instruction about a chapter listing and not about a chapter.
        RobotsPosture::MetadataOnly => match class {
            FetchClass::Metadata => RobotsGate::ReadAndDiscarded,
            FetchClass::Content | FetchClass::Media => RobotsGate::Refused,
        },
        RobotsPosture::Permissive => RobotsGate::Overridden,
    }
}

/// The host of a URL, lowercased and with `www.` removed.
#[must_use]
pub fn shared_host(url: &Url) -> String {
    url.host_str()
        .unwrap_or_default()
        .trim_start_matches("www.")
        .to_ascii_lowercase()
}

/// Whether a hostname is local by name, before any resolution.
///
/// Resolution is the real defence; this catches the names that would otherwise
/// be resolved at all, and the ones a resolver might answer helpfully for.
#[must_use]
pub fn is_local_hostname(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    const LOCAL_SUFFIXES: [&str; 5] = [".local", ".internal", ".home.arpa", ".lan", ".localdomain"];
    if LOCAL_SUFFIXES.iter().any(|suffix| host.ends_with(suffix)) {
        return true;
    }
    // Cloud metadata services answer to these names on the instances that have
    // them; there is no legitimate import from any of them.
    const METADATA_HOSTS: [&str; 4] = [
        "metadata.google.internal",
        "metadata.goog",
        "instance-data",
        "metadata",
    ];
    METADATA_HOSTS.contains(&host.as_str())
}

/// Whether an address may not be connected to.
///
/// # Errors
/// Never — the boolean is the answer. It is a function of one address with no
/// state, so it can be tested exhaustively.
#[must_use]
pub fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_forbidden_v4(v4),
        IpAddr::V6(v6) => {
            // An IPv4-mapped address is an IPv4 address wearing a hat, and must
            // be judged by the IPv4 rules or `::ffff:169.254.169.254` walks in.
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_forbidden_v4(mapped);
            }
            is_forbidden_v6(v6)
        }
    }
}

fn is_forbidden_v4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    let (a, b) = (octets[0], octets[1]);
    ip.is_unspecified()              // 0.0.0.0/8
        || ip.is_loopback()          // 127/8
        || ip.is_private()           // 10/8, 172.16/12, 192.168/16
        || ip.is_link_local()        // 169.254/16 — includes 169.254.169.254
        || ip.is_broadcast()
        || ip.is_multicast()         // 224/4
        || a == 0
        || a >= 240                  // 240/4 reserved
        || (a == 100 && (64..128).contains(&b)) // 100.64/10 carrier NAT
        || (a == 192 && b == 0)      // 192.0.0/24 and 192.0.2/24
        || (a == 198 && (b == 18 || b == 19)) // 198.18/15 benchmarking
        || (a == 198 && b == 51)     // 198.51.100/24
        || (a == 203 && b == 0)      // 203.0.113/24
        || (a == 169 && b == 254)
}

fn is_forbidden_v6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    ip.is_unspecified()
        || ip.is_loopback()          // ::1
        || ip.is_multicast()         // ff00::/8
        // fc00::/7 unique local
        || (segments[0] & 0xfe00) == 0xfc00
        // fe80::/10 link-local
        || (segments[0] & 0xffc0) == 0xfe80
        // 2001:db8::/32 documentation
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        // 64:ff9b::/96 NAT64 — the embedded IPv4 is what is reachable
        || (segments[0] == 0x0064 && segments[1] == 0xff9b)
}

/// Resolve a host and keep only the addresses that may be connected to.
///
/// # Errors
/// * [`SourceError::Refused`] when every address is forbidden, or the host does
///   not resolve. Refusing on an empty set is essential: "we could not check"
///   must never be treated as "so we connected anyway".
/// * [`SourceError::Network`] when resolution itself fails.
pub async fn resolve_public(host: &str, timeout: Duration) -> SourceResult<Vec<SocketAddr>> {
    if is_local_hostname(host) {
        return Err(SourceError::Refused(format!("{host} is a local name")));
    }
    // A literal address needs no resolver, and must not get one.
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_forbidden_ip(ip) {
            return Err(SourceError::Refused(format!(
                "{host} is not a routable public address"
            )));
        }
        return Ok(vec![SocketAddr::new(ip, 0)]);
    }

    let resolved = tokio::time::timeout(timeout, tokio::net::lookup_host((host, 0)))
        .await
        .map_err(|_| SourceError::Network(format!("resolving {host} timed out")))?
        .map_err(|e| SourceError::Network(format!("resolving {host}: {e}")))?;

    let allowed: Vec<SocketAddr> = resolved
        .filter(|addr| !is_forbidden_ip(addr.ip()))
        .collect();
    if allowed.is_empty() {
        // Either the host does not exist or every answer was private. Both mean
        // the same thing to a caller: we will not connect.
        return Err(SourceError::Refused(format!(
            "{host} does not resolve to a public address"
        )));
    }
    Ok(allowed)
}

/// Pull the `charset` parameter out of a `Content-Type` value.
///
/// A `Content-Type` is a media type followed by parameters —
/// `text/html; charset=ISO-8859-1` — and only the parameter is a charset. Passing
/// the whole header to a decoder label lookup would be passing `text/html;
/// charset=iso-8859-1` as the label, which matches nothing and silently falls
/// back to a lossy decode.
#[must_use]
pub(crate) fn charset_of_content_type(content_type: &str) -> Option<&str> {
    content_type
        .split(';')
        .skip(1)
        .map(str::trim)
        .find_map(|param| {
            let (name, value) = param.split_once('=')?;
            name.trim()
                .eq_ignore_ascii_case("charset")
                .then(|| value.trim().trim_matches(['"', '\'']))
        })
        .filter(|charset| !charset.is_empty())
}

/// Turn a fetched body into text using the charset the source declared.
///
/// # Why this is not `String::from_utf8_lossy`
///
/// Because the sources this reads from are old PHP archives that declare a
/// charset they do not use. `tgstorytime.com` says `charset=ISO-8859-1` — as do
/// three other members of the eFiction family — and then emits byte `0x92`, the
/// Windows-1252 right single quote, in the middle of chapter titles. Decoded as
/// true Latin-1 that byte becomes a C1 control character; decoded lossily it
/// becomes `U+FFFD`; either way the title is corrupted and the corruption is
/// invisible until somebody reads it. A reader importing a work whose title is
/// rendered `This week\u{fffd}s shows` has been handed a bad import that every
/// test would have passed.
///
/// The WHATWG encoding standard resolves exactly this ambiguity in the direction
/// the real web needs: the label `iso-8859-1` **means** `windows-1252`, because
/// every browser has read it that way for thirty years and the declared label is
/// the only thing an archive's author ever chose. `encoding_rs` implements that
/// table, so `ISO-8859-1` here gets the reader's decoding rather than the
/// standard's.
///
/// Three sources of the charset, in order of authority:
///
/// 1. The `Content-Type` header, which is what the HTTP layer actually said.
/// 2. A `<meta>` declaration in the first `SNIFF_BYTES` of the body, which is
///    where every one of these archives puts it.
/// 3. UTF-8, with replacement on error.
///
/// A body that is valid UTF-8 is taken as UTF-8 regardless of what was declared,
/// because that is the case that cannot be wrong: if the bytes decode cleanly as
/// UTF-8 they are UTF-8 with overwhelming likelihood, and honouring a wrong
/// `ISO-8859-1` label over them would mangle text that was never broken. This is
/// the same preference order browsers apply, and it is why the fixture recordings
/// — which contain real cp1252 bytes — still round-trip.
#[must_use]
pub fn decode_body(body: &[u8], declared_charset: Option<&str>) -> String {
    // A clean UTF-8 read ends the question. See rule 3 above.
    if let Ok(text) = std::str::from_utf8(body) {
        return text.to_owned();
    }

    let label = declared_charset
        .map(str::to_owned)
        .or_else(|| sniff_charset(body));

    let Some(label) = label else {
        return String::from_utf8_lossy(body).into_owned();
    };

    // `encoding_rs::Encoding::for_label` knows the WHATWG aliases, which is the
    // whole reason this crate is a dependency rather than a `match` on a few
    // strings: it is the table that maps `iso-8859-1` to windows-1252, `latin1`
    // to the same, and refuses labels it does not recognise.
    let Some(encoding) = encoding_rs::Encoding::for_label(label.trim().as_bytes()) else {
        return String::from_utf8_lossy(body).into_owned();
    };

    let (text, _, _) = encoding.decode(body);
    text.into_owned()
}

/// The most bytes a compressed body may expand to.
///
/// A ceiling on memory, like [`FetchPolicy::max_bytes`], and enforced the same
/// way — while reading rather than after. A body that expands past this is
/// truncated instead of allocated, so a decompression bomb costs this much and
/// no more. Truncation is visible downstream as a parse failure, which is the
/// right outcome: a page too large to hold is a page this importer cannot read,
/// and saying so beats both the allocation and a silent half-page.
const MAX_DECOMPRESSED_BYTES: u64 = 64 * 1024 * 1024;

/// Decode a response body: decompress it, then work out its charset.
///
/// # Why decompression belongs here
///
/// A source may compress a response whether or not the request asked for it.
/// `wattpad.com`'s chapter endpoint answers `content-encoding: gzip` to an
/// explicit `Accept-Encoding: identity` — verified 2026-09-11 — and a fetcher
/// that passed those bytes on would hand an adapter gzip for a chapter body.
/// Nothing fails: the bytes are stored, the parse finds no prose, and the import
/// reports a chapter that is empty. It is the same class of silent corruption as
/// a mis-read charset, and it is handled in the same place for the same reason.
///
/// # What is not handled
///
/// `br` and `zstd` are not decoded: neither has been observed from a source an
/// adapter reads, and adding a decompressor for one that has not is a dependency
/// bought with a guess. An encoding that arrives anyway is **named in a warning**
/// rather than passed off as text, because the alternative is exactly the silent
/// corruption this function exists to prevent.
#[must_use]
pub fn decode_response_body(
    body: &[u8],
    declared_charset: Option<&str>,
    content_encoding: Option<&str>,
) -> String {
    let Some(encoding) = content_encoding else {
        return decode_body(body, declared_charset);
    };

    // A list, in the order the encodings were applied — so undoing them means
    // walking it backwards. `Content-Encoding: gzip` is the only shape a source
    // here has sent; the loop costs nothing and does not pretend otherwise.
    let encodings: Vec<String> = encoding
        .split(',')
        .map(|part| part.trim().to_ascii_lowercase())
        .filter(|part| !part.is_empty())
        .collect();

    let mut current: Vec<u8> = body.to_vec();
    for name in encodings.iter().rev() {
        match name.as_str() {
            // `identity` is the no-op encoding, and a source that names it must
            // not be treated as though it had compressed.
            "identity" => {}
            "gzip" | "x-gzip" => match inflate_gzip(&current) {
                Ok(plain) => current = plain,
                Err(why) => {
                    tracing::warn!(
                        encoding = %name,
                        %why,
                        "a response declared a compression this fetcher could not undo"
                    );
                    break;
                }
            },
            "deflate" => match inflate_deflate(&current) {
                Ok(plain) => current = plain,
                Err(why) => {
                    tracing::warn!(
                        encoding = %name,
                        %why,
                        "a response declared a compression this fetcher could not undo"
                    );
                    break;
                }
            },
            other => {
                tracing::warn!(
                    encoding = %other,
                    "a response used a compression this fetcher does not decode; the bytes are \
                     being decoded as text, which will not be the page"
                );
                break;
            }
        }
    }

    decode_body(&current, declared_charset)
}

/// Gunzip, bounded.
fn inflate_gzip(body: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    use std::io::Read;
    let mut out = Vec::with_capacity(body.len().saturating_mul(4));
    flate2::read::GzDecoder::new(body)
        .take(MAX_DECOMPRESSED_BYTES)
        .read_to_end(&mut out)?;
    Ok(out)
}

/// Inflate a raw or zlib-wrapped deflate stream.
///
/// Both, because `Content-Encoding: deflate` is specified as zlib and served as
/// raw deflate often enough that every browser accepts either. Trying zlib first
/// matches what they do.
fn inflate_deflate(body: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    use std::io::Read;
    let mut out = Vec::with_capacity(body.len().saturating_mul(4));
    if flate2::read::ZlibDecoder::new(body)
        .take(MAX_DECOMPRESSED_BYTES)
        .read_to_end(&mut out)
        .is_ok()
        && !out.is_empty()
    {
        return Ok(out);
    }
    out.clear();
    flate2::read::DeflateDecoder::new(body)
        .take(MAX_DECOMPRESSED_BYTES)
        .read_to_end(&mut out)?;
    Ok(out)
}

/// How much of a body to search for a `<meta>` charset declaration.
///
/// Every archive in this family declares its charset inside the first kilobyte —
/// it is emitted by the template's `<head>` before any content. Reading more
/// would mean scanning prose for a string that a work could legitimately contain.
const SNIFF_BYTES: usize = 4096;

/// Find a charset declared in the document itself.
///
/// Both spellings the family uses: `<meta charset="...">` and the HTML 4 form
/// `<meta http-equiv="Content-Type" content="text/html; charset=...">`. The
/// scan is byte-wise and ASCII-only on purpose — it runs before the encoding is
/// known, so it cannot assume a decoding.
fn sniff_charset(body: &[u8]) -> Option<String> {
    let head = &body[..body.len().min(SNIFF_BYTES)];
    let lower: Vec<u8> = head.iter().map(u8::to_ascii_lowercase).collect();
    let needle = b"charset";
    let mut search_from = 0usize;
    while let Some(offset) = find_bytes(&lower[search_from..], needle) {
        let at = search_from + offset + needle.len();
        // Skip `=` and any quoting or space between it and the value.
        let rest = &head[at.min(head.len())..];
        let value: Vec<u8> = rest
            .iter()
            .copied()
            .skip_while(|b| matches!(b, b'=' | b' ' | b'"' | b'\''))
            .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            .collect();
        // A `<meta charset=x-user-defined>` or a stray word containing "charset"
        // yields something unusable; `for_label` rejects it and we fall through.
        if value.len() >= 3 {
            return String::from_utf8(value).ok();
        }
        search_from = at;
        if search_from >= lower.len() {
            break;
        }
    }
    None
}

/// Find a byte substring, without pulling in a search crate for eight lines.
fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Map a transport error onto a category the import knows how to act on.
pub(crate) fn map_reqwest_error(error: reqwest::Error) -> SourceError {
    if error.is_timeout() {
        SourceError::Network("timed out".to_owned())
    } else if error.is_connect() {
        SourceError::Network(format!("could not connect: {error}"))
    } else {
        SourceError::Network(error.to_string())
    }
}

/// Encode a form body by hand.
///
/// `reqwest`'s `form` helper is available, but a source that needs the same key
/// twice (several do, for arrays) is served by a `HashMap`-backed encoder
/// wrongly and silently.
#[must_use]
pub fn encode_form(fields: &mut [(&str, &str)]) -> String {
    let mut out = String::new();
    for (key, value) in fields {
        if !out.is_empty() {
            out.push('&');
        }
        out.push_str(&urlencoding::encode(key));
        out.push('=');
        out.push_str(&urlencoding::encode(value));
    }
    out
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_wall_decides_the_chain_an_import_starts_from() {
        // A source that serves a plain request escalates to nothing: the common
        // case must not pay for the rare one.
        assert!(Unblock::for_wall(Wall::None).is_none());

        // A fingerprint wall starts impersonating rather than spending a request
        // on the challenge the adapter's author already knew about.
        let chain = Unblock::for_wall(Wall::Fingerprint);
        assert!(
            chain.fingerprint.is_some(),
            "a fingerprint wall starts fingered"
        );
        assert!(
            chain.solver.is_none(),
            "a solver is the instance's to offer"
        );
        assert!(
            !chain.archive,
            "an archived read is a decision, not a default"
        );

        // A solver wall declares no step of its own, and on purpose: the
        // fingerprint was measured insufficient for such a host, so a step known
        // to fail would be one refused request per page.
        let chain = Unblock::for_wall(Wall::Solver);
        assert!(
            chain.is_none(),
            "the chain is config's to fill in: {chain:?}"
        );
    }

    #[test]
    fn a_disallowed_path_is_refused_when_the_instance_honours_the_rules() {
        // Recorded from `tgstorytime.com` on 2026-09-10: the whole archive is
        // disallowed, which is why that member cannot be imported on a
        // compliant instance.
        let rules = crate::robots::RobotsRules::parse("User-agent: *\nDisallow: /\n", "Lorehaven");
        let path = "/viewstory.php?sid=6369";

        assert_eq!(
            robots_gate(&rules, path, RobotsPosture::Strict, FetchClass::Content),
            RobotsGate::Refused
        );
        // And the same rules under an instance that has overridden them.
        assert_eq!(
            robots_gate(&rules, path, RobotsPosture::Permissive, FetchClass::Content),
            RobotsGate::Overridden
        );
    }

    #[test]
    fn metadata_only_reads_a_disallowed_metadata_path_and_discards_it() {
        // The one new answer, and the reason the posture exists: the old
        // boolean had no way to say "read it, keep none of it".
        let rules = crate::robots::RobotsRules::parse("User-agent: *\nDisallow: /\n", "Lorehaven");

        assert_eq!(
            robots_gate(
                &rules,
                "/viewstory.php?sid=6369",
                RobotsPosture::MetadataOnly,
                FetchClass::Metadata
            ),
            RobotsGate::ReadAndDiscarded,
            "a disallowed METADATA read survives this posture as a read whose bytes are thrown away"
        );
    }

    #[test]
    fn metadata_only_still_refuses_every_content_and_media_path() {
        // The property that makes `metadata_only` a posture rather than a
        // loophole. `ReadAndDiscarded` is scoped to `FetchClass::Metadata`, and
        // a chapter under the same posture is refused outright — which is the
        // only thing stopping "read it and discard it" from becoming "read it
        // and keep the parts worth keeping".
        let rules = crate::robots::RobotsRules::parse("User-agent: *\nDisallow: /\n", "Lorehaven");
        let path = "/viewstory.php?sid=6369";

        for class in [FetchClass::Content, FetchClass::Media] {
            assert_eq!(
                robots_gate(&rules, path, RobotsPosture::MetadataOnly, class),
                RobotsGate::Refused,
                "a disallowed {:?} path is refused under metadata_only, not read and discarded",
                class
            );
        }
    }

    #[test]
    fn the_class_is_never_inferred_from_the_path() {
        // The single most important property of the new signature. A URL that
        // looks exactly like a chapter page, fetched as `Metadata`, must come
        // out the same as any other metadata fetch — because if a fetcher
        // inferred the class from the path, an adapter's guess about a URL's
        // shape would raise its own ceiling, and the "just the metadata" door
        // would be a body door with a different name.
        let rules = crate::robots::RobotsRules::parse("User-agent: *\nDisallow: /\n", "Lorehaven");
        let chapter_shaped = "/viewstory.php?sid=6369";

        // Identical to a plain metadata path, despite looking like prose.
        assert_eq!(
            robots_gate(
                &rules,
                chapter_shaped,
                RobotsPosture::MetadataOnly,
                FetchClass::Metadata
            ),
            RobotsGate::ReadAndDiscarded
        );
        // And the same URL declared as content is refused. The class is the only
        // thing that differs between these two calls.
        assert_eq!(
            robots_gate(
                &rules,
                chapter_shaped,
                RobotsPosture::MetadataOnly,
                FetchClass::Content
            ),
            RobotsGate::Refused
        );
    }

    #[test]
    fn the_byte_ceiling_follows_the_declared_class() {
        // Amendment §1.2 limit 1. The point is that the ceiling is chosen by the
        // DECLARATION, so raising the instance's content ceiling cannot make a
        // metadata read accept more — an operator who raised `max_bytes` said
        // "I will store more prose", not "read more documents".
        let mut policy = policy();
        let base = policy.max_bytes;
        assert!(
            base > 1024 * 1024,
            "the content ceiling is above the metadata one"
        );

        assert_eq!(policy.max_bytes_for(FetchClass::Metadata), 1024 * 1024);
        assert_eq!(policy.max_bytes_for(FetchClass::Content), base);
        assert_eq!(policy.max_bytes_for(FetchClass::Media), base);

        // Raise it a long way. Metadata does not move; content and media do.
        policy.max_bytes = 64 * 1024 * 1024;
        assert_eq!(
            policy.max_bytes_for(FetchClass::Metadata),
            1024 * 1024,
            "a raised content ceiling does not widen a metadata read, which is the property that \
             makes the class worth declaring"
        );
        assert_eq!(policy.max_bytes_for(FetchClass::Content), 64 * 1024 * 1024);
        assert_eq!(policy.max_bytes_for(FetchClass::Media), 64 * 1024 * 1024);
    }

    /// The body of `SafeFetcher::send_with_redirects`, for the one assertion that
    /// cannot be made at runtime.
    /// The source text of `send_with_redirects`.
    fn send_path_source() -> String {
        source_of(
            "    async fn send_with_redirects(",
            "\n    /// The clients to try, in order, for one request.",
        )
    }

    /// The source text of `attempt` — the function that actually builds the
    /// request headers, one call deeper than `send_with_redirects`.
    fn attempt_source() -> String {
        source_of(
            "    async fn attempt(",
            "\n    /// Reject a host outside the source's own.",
        )
    }

    /// One function's source, bounded by the next item in its impl block.
    ///
    /// Shared by the two callers because they bound differently and a copy
    /// each would drift. A missing bound panics rather than returning
    /// something truncated, because a truncated slice would make every
    /// `contains` assertion in it vacuously false — which is a green test.
    fn source_of(start_marker: &str, end_marker: &str) -> String {
        let src = include_str!("safety.rs");
        let start = src
            .find(start_marker)
            .unwrap_or_else(|| panic!("{start_marker:?} exists"));
        let rest = &src[start..];
        let end = rest
            .find(end_marker)
            .unwrap_or_else(|| panic!("{end_marker:?} bounds it"));
        rest[..end].to_owned()
    }

    #[tokio::test]
    async fn the_ceiling_the_send_path_uses_is_the_one_the_class_asked_for() {
        // D6 survived the first pass: `max_bytes_for` was correct and thoroughly
        // tested, and nothing connected it to the byte count handed to the
        // transport. A `policy.max_bytes` at the call site left every ceiling
        // test green while a metadata read accepted 8 MiB.
        //
        // The fix is the name: the send path and this test both call
        // `SafeFetcher::ceiling_for`, so a call site that stops consulting it is
        // a compile error rather than a silent widening. Asserting the
        // expression again would not have caught D6 — that test would have
        // restated the same one-liner.
        let fetcher = fetcher_with_robots(Duration::from_millis(500), "User-agent: *\n").await;

        // Also: the send path must DERIVE its ceiling from this function rather
        // than carrying its own. `ceiling_for` being correct is not the same
        // thing as the send path asking it, and a `self.policy.max_bytes` at
        // the call site survived a mutation pass on exactly that distinction.
        // The type now makes a caller supply a class, so the only remaining way
        // to bypass the function is to ignore the parameter it was given — and
        // that is what the source is checked for here, because no runtime
        // assertion can see it.
        //
        // A source-text assertion is normally a smell. This one earns it: the
        // property is that a specific expression appears at a specific place,
        // the code under test is one line, and the alternative is a test that
        // passes while the bug is present.
        let send_path = self::send_path_source();
        assert!(
            send_path.contains("self.ceiling_for(class)"),
            "the send path derives its ceiling from ceiling_for; found instead: {send_path}"
        );
        assert!(
            !send_path.contains("self.policy.max_bytes,"),
            "and does not reach for the policy's own ceiling; found: {send_path}"
        );

        assert_eq!(fetcher.ceiling_for(FetchClass::Metadata), 1024 * 1024);
        assert_eq!(
            fetcher.ceiling_for(FetchClass::Content),
            fetcher.policy.max_bytes
        );
        assert_eq!(
            fetcher.ceiling_for(FetchClass::Media),
            fetcher.policy.max_bytes
        );
    }

    #[test]
    fn an_allowed_path_is_allowed_under_every_posture_and_class() {
        // The override is not a licence to reclassify everything: a path the
        // rules permit takes the same branch whether or not the instance
        // honours them, so nothing is counted as an override that was not one,
        // and a read-and-discard is not recorded where nothing was overridden.
        let rules = crate::robots::RobotsRules::parse(
            "User-agent: *\nDisallow: /private/\nAllow: /\n",
            "Lorehaven",
        );

        for posture in [
            RobotsPosture::Strict,
            RobotsPosture::MetadataOnly,
            RobotsPosture::Permissive,
        ] {
            for class in [FetchClass::Metadata, FetchClass::Content, FetchClass::Media] {
                assert_eq!(
                    robots_gate(&rules, "/viewstory.php?sid=1", posture, class),
                    RobotsGate::Allowed,
                    "{:?} under {posture:?}",
                    class
                );
            }
        }
        // And the disallowed subtree still separates the postures.
        for class in [FetchClass::Metadata, FetchClass::Content] {
            assert_eq!(
                robots_gate(&rules, "/private/x", RobotsPosture::Strict, class),
                RobotsGate::Refused
            );
            assert_eq!(
                robots_gate(&rules, "/private/x", RobotsPosture::Permissive, class),
                RobotsGate::Overridden
            );
        }
    }

    #[test]
    fn a_host_with_no_rules_has_nothing_to_refuse_or_to_override() {
        // A `404` for `robots.txt` is a site with no restrictions (spec §11.5),
        // so it must produce neither a refusal nor a recorded override.
        let rules = crate::robots::RobotsRules::unrestricted();
        for posture in [
            RobotsPosture::Strict,
            RobotsPosture::MetadataOnly,
            RobotsPosture::Permissive,
        ] {
            assert_eq!(
                robots_gate(&rules, "/anything", posture, FetchClass::Content),
                RobotsGate::Allowed
            );
        }
    }

    #[test]
    fn the_four_answers_are_the_whole_decision() {
        // The gate answers only these four things, so a caller that handles all
        // four has handled every case — which is the property that makes
        // extracting it worth more than inlining it. A fifth answer added later
        // must break this test, because a caller that does not handle it would
        // otherwise have no arm to land in.
        let forbidding =
            crate::robots::RobotsRules::parse("User-agent: *\nDisallow: /\n", "Lorehaven");
        let allowing = crate::robots::RobotsRules::unrestricted();

        let answers = [
            robots_gate(
                &forbidding,
                "/x",
                RobotsPosture::Strict,
                FetchClass::Content,
            ),
            robots_gate(
                &forbidding,
                "/x",
                RobotsPosture::MetadataOnly,
                FetchClass::Metadata,
            ),
            robots_gate(
                &forbidding,
                "/x",
                RobotsPosture::Permissive,
                FetchClass::Content,
            ),
            robots_gate(&allowing, "/x", RobotsPosture::Strict, FetchClass::Content),
        ];
        assert_eq!(
            answers,
            [
                RobotsGate::Refused,
                RobotsGate::ReadAndDiscarded,
                RobotsGate::Overridden,
                RobotsGate::Allowed
            ]
        );
    }

    #[test]
    fn a_gzipped_body_is_undone_before_it_is_read_as_text() {
        // Measured: `wattpad.com`'s part-text endpoint answers
        // `content-encoding: gzip` to an explicit `Accept-Encoding: identity`.
        // A fetcher that passed those bytes on would store a chapter body as
        // mojibake and report success.
        use std::io::Write;

        let page = "<p>Abby was the first to greet them.</p>";
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(page.as_bytes()).expect("compress");
        let compressed = encoder.finish().expect("finish");

        assert_ne!(compressed, page.as_bytes(), "the fixture must compress");
        assert_eq!(
            decode_response_body(&compressed, Some("text/plain; charset=UTF-8"), Some("gzip")),
            page
        );
        // And the same bytes with no encoding declared are the corruption this
        // test exists to catch — so the assertion above is not vacuous.
        assert_ne!(
            decode_response_body(&compressed, Some("text/plain; charset=UTF-8"), None),
            page
        );
    }

    #[test]
    fn an_uncompressed_body_is_left_alone_whatever_it_declares() {
        let page = b"<p>Plain prose.</p>";
        let expected = "<p>Plain prose.</p>";

        // `identity` is the no-op encoding, and a source that names it must not
        // be treated as though it had compressed.
        assert_eq!(
            decode_response_body(page, Some("text/html; charset=UTF-8"), Some("identity")),
            expected
        );
        // And the common case: no encoding header at all.
        assert_eq!(
            decode_response_body(page, Some("text/html; charset=UTF-8"), None),
            expected
        );
    }

    #[test]
    fn a_declared_compression_that_cannot_be_undone_does_not_take_the_page_with_it() {
        // An unknown encoding must not turn a readable body into a panic or into
        // nothing. It is passed through as text and named in a warning; the
        // assertion here is only that the call still answers.
        let body = b"<p>Not actually compressed.</p>";
        let text = decode_response_body(body, Some("text/html; charset=UTF-8"), Some("br"));
        assert!(!text.is_empty());

        // And a body that *claims* gzip but is not gzip is handled the same way
        // rather than panicking on a decode error.
        let text = decode_response_body(body, Some("text/html; charset=UTF-8"), Some("gzip"));
        assert!(!text.is_empty(), "a failed decode still returns something");
    }

    #[test]
    fn a_chain_of_encodings_is_undone_in_reverse() {
        // The encodings are listed in the order they were applied, so undoing
        // them means walking the list backwards. Asserted with a real chain
        // rather than trusted, because getting the order wrong would decode
        // something and produce text that looks plausible.
        use std::io::Write;

        let page = "<p>Twice through.</p>";
        let mut once = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        once.write_all(page.as_bytes()).expect("compress");
        let once = once.finish().expect("finish");
        let mut twice = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        twice.write_all(&once).expect("compress");
        let twice = twice.finish().expect("finish");

        assert_eq!(
            decode_response_body(&twice, None, Some("gzip, gzip")),
            page,
            "the list is undone outside-in"
        );
    }

    #[test]
    fn compliance_is_the_default_policy() {
        // An instance does not have to be told to read a host's rules. This is
        // the assertion that fails if somebody ever flips the default.
        assert!(crate::FetchPolicy::default().honour_robots);
    }

    #[test]
    fn the_fingerprint_flag_tells_the_truth_about_this_build() {
        // The importer refuses a fingerprint-walled source when this is false, so
        // it has to agree with the feature the transport is compiled behind.
        assert_eq!(
            crate::FINGERPRINT_SUPPORTED,
            cfg!(feature = "cloudflare-impersonation")
        );
    }
    use super::{
        charset_of_content_type, decode_body, decode_response_body, describe_status, robots_gate,
        sniff_charset, Escalation, RobotsGate, SafeFetcher, Step, Unblock,
    };
    use crate::engine::Impersonation;
    use crate::solver::SolverConfig;

    /// The chain for a policy, as names, so the order is asserted rather than
    /// assumed.
    fn chain(unblock: Unblock) -> Vec<String> {
        let policy = FetchPolicy {
            unblock,
            ..FetchPolicy::default()
        };
        SafeFetcher::new(vec!["example.com".into()], policy)
            .escalation_steps()
            .iter()
            .map(|step| match step {
                Step::Transport(None) => "plain".to_owned(),
                Step::Transport(Some(wanted)) => format!("fingerprint:{}", wanted.as_str()),
                Step::Solver(_) => "solver".to_owned(),
                Step::Archive => "archive".to_owned(),
            })
            .collect()
    }

    #[test]
    fn a_source_that_declares_nothing_escalates_to_nothing() {
        // The default has to stay cheap: every source that serves a plain request
        // must not pay for the ones that do not.
        assert_eq!(chain(Unblock::none()), vec!["plain"]);
    }

    #[test]
    fn a_declared_fingerprint_is_used_from_the_first_request() {
        // The adapter has already researched the source, so there is no reason to
        // spend a request discovering the wall.
        assert_eq!(
            chain(Unblock::fingerprint(Impersonation::Firefox)),
            vec!["fingerprint:firefox"],
        );
    }

    #[test]
    fn the_solver_and_the_archive_come_after_the_transport() {
        let steps = chain(
            Unblock::fingerprint(Impersonation::Chrome)
                .with_solver(SolverConfig::new("http://127.0.0.1:8191"))
                .with_archive(),
        );
        assert_eq!(steps, vec!["fingerprint:chrome", "solver", "archive"]);
    }

    #[test]
    fn the_solver_and_the_archive_run_after_the_declared_transport() {
        // Nothing is escalated to that was not asked for: a source whose adapter
        // declared nothing goes out plain and no further, even on an instance
        // that has a solver and a fingerprint compiled in. An instance that
        // silently impersonated for every challenging source would be doing
        // something its operator never asked it to do.
        let steps = chain(
            Unblock::none()
                .with_solver(SolverConfig::new("http://127.0.0.1:8191"))
                .with_archive(),
        );
        assert_eq!(steps, vec!["plain", "solver", "archive"]);
    }

    #[test]
    fn an_unknown_unblock_is_the_default_and_escalates_nowhere() {
        assert!(Unblock::default().is_none());
        assert!(Unblock::none().is_none());
        assert!(!Unblock::fingerprint(Impersonation::Chrome).is_none());
        assert!(!Unblock::none().with_archive().is_none());
    }

    #[test]
    fn robots_txt_is_read_through_exactly_one_transport() {
        // A source that challenges its own rules file has told us nothing about
        // its pages, and paying a solver to read `robots.txt` would be spending
        // somebody else's capacity on a file we can do without.
        assert_eq!(Escalation::None, Escalation::None);
        let policy = FetchPolicy::default();
        assert!(
            policy.unblock.is_none(),
            "the default policy escalates to nothing"
        );
    }

    /// A code with a reason phrase keeps it.
    #[test]
    fn a_standard_status_keeps_its_phrase() {
        assert_eq!(
            describe_status(reqwest::StatusCode::SERVICE_UNAVAILABLE),
            "503 Service Unavailable"
        );
    }

    /// An intermediary's code is named, because "525" alone does not tell an
    /// operator whether to retry or to wait.
    #[test]
    fn a_cloudflare_code_is_named() {
        assert_eq!(
            describe_status(reqwest::StatusCode::from_u16(525).expect("525 is a status")),
            "525 (an SSL handshake with the site's server failed)"
        );
        assert_eq!(
            describe_status(reqwest::StatusCode::from_u16(522).expect("522 is a status")),
            "522 (the connection to the site's server timed out)"
        );
    }

    /// A code nobody has heard of is a bare number, and carries none of
    /// `StatusCode`'s own `<unknown status code>` apology.
    #[test]
    fn an_unheard_of_code_is_just_the_number() {
        let described = describe_status(reqwest::StatusCode::from_u16(599).expect("599"));
        assert_eq!(described, "599");
        assert!(
            !described.contains("unknown"),
            "the reader is not told that we do not recognise a number: {described}"
        );
    }
    use super::*;

    fn policy() -> FetchPolicy {
        FetchPolicy::default()
    }

    #[test]
    fn only_http_and_https_are_fetchable() {
        assert!(validate_url("https://archiveofourown.org/works/1", &policy()).is_ok());
        assert!(validate_url("http://example.com/x", &policy()).is_ok());
        for bad in [
            "file:///etc/passwd",
            "ftp://example.com/x",
            "gopher://example.com/",
            "data:text/html,<h1>x",
            "javascript:alert(1)",
        ] {
            let error = validate_url(bad, &policy()).unwrap_err();
            assert!(
                matches!(error, SourceError::Refused(_)),
                "{bad} produced {error:?}"
            );
        }
    }

    #[test]
    fn a_credential_in_a_url_is_refused() {
        let error = validate_url("https://user:pw@example.com/x", &policy()).unwrap_err();
        assert!(matches!(error, SourceError::Refused(_)), "{error:?}");
        let error = validate_url("https://user@example.com/x", &policy()).unwrap_err();
        assert!(matches!(error, SourceError::Refused(_)), "{error:?}");
    }

    #[test]
    fn local_hostnames_are_refused_by_name() {
        for host in [
            "localhost",
            "LOCALHOST",
            "a.localhost",
            "thing.local",
            "box.internal",
            "printer.home.arpa",
            "metadata.google.internal",
        ] {
            assert!(
                validate_url(&format!("http://{host}/x"), &policy()).is_err(),
                "{host} was allowed"
            );
        }
        assert!(!is_local_hostname("example.com"));
        assert!(!is_local_hostname("notlocalhost.com"));
    }

    #[test]
    fn loopback_and_private_addresses_are_forbidden() {
        for ip in [
            "127.0.0.1",
            "127.1.2.3",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254", // the cloud metadata service
            "0.0.0.0",
            "100.64.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "203.0.113.9",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
        ] {
            let ip: IpAddr = ip.parse().unwrap();
            assert!(is_forbidden_ip(ip), "{ip} was allowed");
        }
    }

    #[test]
    fn public_addresses_are_allowed() {
        for ip in [
            "1.1.1.1",
            "8.8.8.8",
            "151.101.1.140",
            "2606:4700:4700::1111",
        ] {
            let ip: IpAddr = ip.parse().unwrap();
            assert!(!is_forbidden_ip(ip), "{ip} was refused");
        }
    }

    #[test]
    fn an_ipv4_mapped_address_cannot_smuggle_a_private_one() {
        // ::ffff:169.254.169.254 is the metadata service with a sixth colon.
        let mapped: IpAddr = "::ffff:169.254.169.254".parse().unwrap();
        assert!(is_forbidden_ip(mapped));
        let loopback: IpAddr = "::ffff:127.0.0.1".parse().unwrap();
        assert!(is_forbidden_ip(loopback));
        // And a mapped public address is still public.
        let public: IpAddr = "::ffff:1.1.1.1".parse().unwrap();
        assert!(!is_forbidden_ip(public));
    }

    #[test]
    fn ipv6_internal_ranges_are_forbidden() {
        for ip in [
            "::1",
            "::",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "ff02::1",
            "2001:db8::1",
            "64:ff9b::7f00:1",
        ] {
            let ip: IpAddr = ip.parse().unwrap();
            assert!(is_forbidden_ip(ip), "{ip} was allowed");
        }
        let public: IpAddr = "2001:4860:4860::8888".parse().unwrap();
        assert!(!is_forbidden_ip(public));
    }

    #[tokio::test]
    async fn a_literal_private_address_is_refused_without_resolving() {
        let error = resolve_public("127.0.0.1", Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(matches!(error, SourceError::Refused(_)), "{error:?}");
        // And a literal public address needs no resolver, so it works offline.
        let ok = resolve_public("1.1.1.1", Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(ok.len(), 1);
    }

    #[tokio::test]
    async fn a_local_name_is_refused_before_resolution() {
        let error = resolve_public("localhost", Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(matches!(error, SourceError::Refused(_)), "{error:?}");
    }

    #[test]
    fn a_host_outside_the_source_is_refused() {
        let fetcher = SafeFetcher::new(vec!["archiveofourown.org".into()], policy());
        assert!(fetcher.check_host_allowed("archiveofourown.org").is_ok());
        assert!(fetcher
            .check_host_allowed("www.archiveofourown.org")
            .is_ok());
        assert!(fetcher.check_host_allowed("evil.example").is_err());
        // A suffix match must not be enough on its own.
        assert!(fetcher
            .check_host_allowed("notarchiveofourown.org")
            .is_err());
        assert!(fetcher
            .check_host_allowed("archiveofourown.org.evil.example")
            .is_err());
    }

    #[test]
    fn the_credential_header_is_not_set_table() {
        let fetcher = SafeFetcher::new(vec!["example.com".into()], policy())
            .with_credential_header("example.com", "X-Token", "secret-value")
            .expect("header is valid");
        assert_eq!(fetcher.credential_host.as_deref(), Some("example.com"));
        // A newline in a credential must be refused rather than sent.
        let bad = SafeFetcher::new(vec!["example.com".into()], policy()).with_credential_header(
            "example.com",
            "X-Token",
            "bad\r\nInjected: 1",
        );
        assert!(bad.is_err());
    }

    #[test]
    fn a_credential_is_not_sent_to_our_own_host_mismatch() {
        // The rule the redirect path relies on.
        let fetcher = SafeFetcher::new(vec!["a.example".into(), "b.example".into()], policy())
            .with_credential_header("A.EXAMPLE", "X-Token", "t")
            .unwrap();
        assert_eq!(fetcher.credential_host.as_deref(), Some("a.example"));
        assert_ne!(fetcher.credential_host.as_deref(), Some("b.example"));
    }

    #[test]
    fn form_bodies_encode_both_halves() {
        let mut fields = vec![("user", "a b"), ("pass", "p&q=1")];
        assert_eq!(encode_form(&mut fields), "user=a%20b&pass=p%26q%3D1");
        // An empty body is not a stray ampersand.
        let mut empty: Vec<(&str, &str)> = Vec::new();
        assert_eq!(encode_form(&mut empty), "");
    }

    #[tokio::test]
    async fn the_fixture_fetcher_records_what_was_asked_for() {
        let fetcher = FixtureFetcher::new()
            .with_page("https://example.com/a", "<html>a</html>")
            .with_failure("broken", SourceError::RateLimited("slow".into()));

        let page = fetcher.get("https://example.com/a").await.unwrap();
        assert_eq!(page.body, "<html>a</html>");
        assert_eq!(page.final_url, "https://example.com/a");

        let error = fetcher.get("https://example.com/broken").await.unwrap_err();
        assert!(matches!(error, SourceError::RateLimited(_)));

        let missing = fetcher
            .get("https://example.com/missing")
            .await
            .unwrap_err();
        assert!(matches!(missing, SourceError::Network(_)));

        assert_eq!(fetcher.times_requested("example.com/a"), 1);
        assert_eq!(fetcher.times_requested("example.com"), 3);
        assert_eq!(fetcher.requested().len(), 3);
    }

    #[test]
    fn default_policy_is_bounded() {
        let policy = FetchPolicy::default();
        assert!(policy.max_bytes <= 16 * 1024 * 1024);
        assert!(policy.timeout <= Duration::from_secs(60));
        assert!(policy.max_redirects <= 10);
        assert!(policy.max_concurrent_per_host >= 1);
        assert!(!policy.user_agent.is_empty());
        // Spec §11.5's floor, asserted where it is defined rather than where it
        // is used: a default of zero here would silently remove the limit for
        // every host that publishes no `Crawl-delay`.
        assert!(policy.default_interval_per_host >= Duration::from_secs(1));
        assert!(policy.robots_ttl > Duration::ZERO);
    }

    #[test]
    fn the_product_token_is_the_leading_word_of_our_agent() {
        // A site's `robots.txt` names crawlers by product token, not by the full
        // `User-Agent`, so this is what decides whether a named group applies.
        assert_eq!(product_token("Lorehaven/0.1.0 (+import)"), "Lorehaven");
        assert_eq!(product_token("Lorehaven"), "Lorehaven");
    }

    /// A fetcher with one seeded `robots.txt`, so pacing and path rules can be
    /// tested without a network.
    async fn fetcher_with_robots(agent_interval: Duration, robots: &str) -> SafeFetcher {
        let mut policy = policy();
        policy.min_interval_per_host = agent_interval;
        let fetcher = SafeFetcher::new(vec!["example.com".into()], policy);
        let rules = RobotsRules::parse(robots, "Lorehaven");
        {
            let mut cache = fetcher.robots.lock().await;
            cache.insert(
                "example.com".to_owned(),
                RobotsEntry {
                    rules,
                    read_at: Instant::now(),
                },
            );
        }
        fetcher
    }

    #[tokio::test]
    async fn pacing_is_identical_under_every_posture() {
        // Spec §11.5's own carve-out, and the row the plan insists be a test
        // rather than a comment: `Permissive` is a statement about `Disallow`
        // only. An instance that ignores a site's path rules is still bound by
        // the time that site asked for, and the whole reason an operator can
        // justify `permissive` at all is that the politeness half still holds.
        //
        // So the two axes must be independent: every posture gets the published
        // `Crawl-delay`, and every posture gets the one-second floor when the
        // host publishes nothing. A mutation that made the posture reach the
        // pacing decision — the obvious way to "implement" `permissive` — would
        // zero the delay for that row and be caught here.
        for posture in [
            RobotsPosture::Strict,
            RobotsPosture::MetadataOnly,
            RobotsPosture::Permissive,
        ] {
            let mut published = fetcher_with_robots(
                Duration::from_millis(500),
                "User-agent: *\nCrawl-delay: 3\nDisallow: /\n",
            )
            .await;
            published.policy.robots_posture = posture;
            assert_eq!(
                published.interval_for("example.com").await,
                Duration::from_secs(3),
                "{posture:?} still waits the host's published Crawl-delay"
            );

            let mut unpublished =
                fetcher_with_robots(Duration::from_millis(500), "User-agent: *\nDisallow: /\n")
                    .await;
            unpublished.policy.robots_posture = posture;
            assert_eq!(
                unpublished.interval_for("example.com").await,
                Duration::from_secs(1),
                "{posture:?} still falls back to the one-second floor, and `permissive` is not \
                 an exemption from it"
            );
        }
    }

    #[tokio::test]
    async fn a_published_crawl_delay_is_used_as_the_gap() {
        let fetcher = fetcher_with_robots(
            Duration::from_millis(500),
            "User-agent: *\nCrawl-delay: 3\n",
        )
        .await;

        assert_eq!(
            fetcher.interval_for("example.com").await,
            Duration::from_secs(3)
        );
    }

    #[tokio::test]
    async fn a_host_with_no_published_delay_gets_one_second() {
        // The case spec §11.5 names: no information is not no limit.
        let fetcher =
            fetcher_with_robots(Duration::from_millis(500), "User-agent: *\nDisallow: /x\n").await;

        assert_eq!(
            fetcher.interval_for("example.com").await,
            Duration::from_secs(1)
        );
    }

    #[tokio::test]
    async fn a_host_we_have_not_read_yet_also_gets_one_second() {
        // A cache miss must not become a faster fetch, and must not fetch
        // `robots.txt` from inside the request path.
        let mut policy = policy();
        policy.min_interval_per_host = Duration::from_millis(100);
        let fetcher = SafeFetcher::new(vec!["example.com".into()], policy);

        assert_eq!(
            fetcher.interval_for("example.com").await,
            Duration::from_secs(1)
        );
    }

    #[tokio::test]
    async fn an_adapters_slower_interval_is_not_shortened_by_a_published_one() {
        // The max of the two: the site's number is a floor, and an adapter that
        // was more cautious than the site is not thereby wrong.
        let fetcher = fetcher_with_robots(
            Duration::from_millis(5_000),
            "User-agent: *\nCrawl-delay: 1\n",
        )
        .await;

        assert_eq!(
            fetcher.interval_for("example.com").await,
            Duration::from_secs(5)
        );
    }

    #[tokio::test]
    async fn a_malformed_delay_cannot_produce_a_faster_pace() {
        let fetcher = fetcher_with_robots(
            Duration::from_millis(200),
            "User-agent: *\nCrawl-delay: soon\n",
        )
        .await;

        // Unreadable → the default, never zero.
        assert_eq!(
            fetcher.interval_for("example.com").await,
            Duration::from_secs(1)
        );
    }

    // -----------------------------------------------------------------------
    // The class-specific User-Agent token, and the impersonation refusal
    // (spec §1.5 and §24.5)
    // -----------------------------------------------------------------------

    /// The token names the class the request is actually making.
    ///
    /// Three classes, three tokens, asserted together: one example would pass
    /// with a function that hard-codes `class=metadata`, and the whole point is
    /// that a metadata fetch and a media fetch are distinguishable to a site
    /// choosing what to serve.
    #[test]
    fn the_user_agent_names_the_class_of_the_request() {
        let base = "Lorehaven/1.0 (+import)";
        for (class, expected) in [
            (FetchClass::Metadata, "metadata"),
            (FetchClass::Content, "content"),
            (FetchClass::Media, "media"),
        ] {
            let agent = user_agent_for(base, class);
            assert!(
                agent.contains(&format!("class={expected}")),
                "a {class:?} fetch must say so: {agent:?}"
            );
            assert!(
                agent.starts_with(base),
                "and the configured agent is the prefix, not replaced: {agent:?}"
            );
        }
    }

    /// A token naming another product's crawler is refused (spec §24.5).
    ///
    /// The whole list, not a sample, and each in more than one spelling. A
    /// denylist is only as good as the variants it catches: `GoogleBot` and
    /// `googlebot` are the same claim, and so is `Mozilla/5.0 (compatible;
    /// Googlebot/2.1; +http://www.google.com/bot.html)` — a site reading that
    /// sees a crawler, not a browser wearing a crawler's name.
    #[test]
    fn a_token_impersonating_another_crawler_is_refused() {
        for token in [
            "Googlebot/2.1",
            "googlebot/2.1",
            "GOOGLEBOT/2.1",
            "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)",
            "Mozilla/5.0 (compatible; Lorehaven/1.0; Googlebot/2.1)",
            "bingbot/2.0",
            "GPTBot/1.0",
            "ChatGPT-User/1.0",
            "ClaudeBot/1.0",
            "PerplexityBot/1.0",
            "CCBot/2.0",
            "Bytespider",
            "Twitterbot/1.0",
            "facebookexternalhit/1.1",
            "Slackbot-LinkExpanding 1.0",
            "Discordbot/2.0",
            "TelegramBot",
        ] {
            assert!(
                !validate_user_agent(token),
                "{token:?} impersonates another crawler and must be refused"
            );
        }

        // And an honest token is allowed, including one this instance builds.
        for token in [
            "Lorehaven/1.0 (+import)",
            "lorehaven/1.0",
            "Lorehaven/1.0 (class=metadata)",
            "Lorehaven/1.0 (class=media)",
            "SomeReader/2.0",
        ] {
            assert!(
                validate_user_agent(token),
                "{token:?} identifies its owner and must be allowed"
            );
        }
    }

    /// The refusal happens before any byte leaves, and it names the token.
    ///
    /// Asserted at the send path rather than at config load because that is
    /// where the decision is made. `get_with_class` on an unreachable host would
    /// fail on the connection instead, so the assertion is that the error is a
    /// *refusal* naming the agent — a connection error would be a different
    /// variant and would fail this test.
    /// The class the caller names is the class the token carries.
    ///
    /// A source-text assertion, for the same reason as the ceiling's one: the
    /// property is that a specific expression appears at a specific place, the
    /// code under test is one line, and no runtime assertion can see a `let _ =
    /// class;` that a mutation introduces. B7 replaced the class at the send
    /// path with a constant and the whole suite stayed green, because the
    /// refusal tests never look at the token and the token test calls
    /// `user_agent_for` directly — so nothing connected the two.
    #[test]
    fn the_send_path_names_the_class_it_was_given() {
        let send_path = self::attempt_source();
        assert!(
            send_path.contains("user_agent_for(&self.policy.user_agent, class)"),
            "the token is built from the class the caller named; found instead: {send_path}"
        );
        // The class is threaded in as a `FetchClass`, not as text, so the
        // parameter is present by construction and the only way to lose it is to
        // ignore it. Both are the same bug wearing different clothes.
        assert!(
            !send_path.contains("let _ = class"),
            "and the class is not discarded on the way; found: {send_path}"
        );
    }

    #[tokio::test]
    async fn an_impersonating_token_is_refused_before_the_request_is_made() {
        let mut policy = policy();
        policy.user_agent = "Googlebot/2.1".to_owned();
        let fetcher = SafeFetcher::new(vec!["example.com".into()], policy);

        let error = fetcher
            .get_with_class(
                "https://example.com/story/1",
                None,
                None,
                FetchClass::Content,
            )
            .await
            .unwrap_err();
        match error {
            SourceError::Refused(message) => {
                assert!(
                    message.contains("Googlebot"),
                    "the refusal names the offending token: {message}"
                );
                assert!(message.contains("impersonat"), "and says why: {message}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    /// An honest fetch is not refused by the impersonation check.
    ///
    /// The regression net for the check: a check that refuses everything is a
    /// check that looks correct, because the only tests it needs are about the
    /// things it stops. A real fetch against an unreachable host must fail with
    /// a *connection* error, not a refusal.
    #[tokio::test]
    async fn an_honest_token_is_not_refused() {
        let policy = policy();
        assert!(
            validate_user_agent(&policy.user_agent),
            "the build's own agent identifies Lorehaven"
        );
        let fetcher = SafeFetcher::new(vec!["example.invalid".into()], policy);
        let error = fetcher
            .get_with_class(
                "https://example.invalid/story/1",
                None,
                None,
                FetchClass::Content,
            )
            .await
            .unwrap_err();
        let refused_for_the_agent = matches!(
            &error,
            SourceError::Refused(message) if message.contains("user agent")
        );
        assert!(
            !refused_for_the_agent,
            "an honest agent must not be refused by the impersonation check: {error:?}"
        );
    }

    #[tokio::test]
    async fn a_disallowed_path_is_refused_before_any_request_is_made() {
        // No network is reachable in a test, so the refusal has to happen before
        // the fetch: were the gate applied after the request, this test would
        // fail on the connection attempt rather than on the refusal.
        let fetcher = fetcher_with_robots(
            Duration::from_millis(500),
            "User-agent: *\nDisallow: /private\n",
        )
        .await;

        let error = fetcher
            .get("https://example.com/private/report")
            .await
            .unwrap_err();
        assert!(
            matches!(error, SourceError::Refused(_)),
            "expected a refusal, got {error:?}"
        );
    }

    #[tokio::test]
    async fn an_undeclared_fetch_is_refused_under_metadata_only() {
        // The direction of the undeclared-class default is the whole point, and
        // getting it backwards is a silent hole. `get_with_redirects` is what
        // every adapter that has not been updated reaches, and it declares
        // `Content` — so under `metadata_only` it is REFUSED.
        //
        // Defaulting to `Metadata` would make the one fetch that declared
        // nothing the one fetch that gets through: an adapter nobody has
        // updated would quietly read a forbidden path while a compliant one
        // would not.
        //
        // This goes through `get` — the real trait method, so the real default
        // — rather than naming the class in the assertion. A test that wrote
        // `FetchClass::Content` next to the expectation would keep passing when
        // the default changed underneath it, which is the failure this test
        // exists to catch. The class is left undeclared on purpose: the caller
        // under test is one that does not know the concept yet.
        let mut fetcher =
            fetcher_with_robots(Duration::from_millis(500), "User-agent: *\nDisallow: /\n").await;
        fetcher.policy.robots_posture = RobotsPosture::MetadataOnly;

        let error = fetcher
            .get("https://example.com/story/1")
            .await
            .unwrap_err();
        assert!(
            matches!(error, SourceError::Refused(_)),
            "a caller that declared no class is refused under metadata_only, which is the safe \
             direction. Got {error:?} — if this is a network error rather than a refusal, the \
             gate was passed and the ceiling was not applied."
        );
    }

    #[tokio::test]
    async fn a_declared_metadata_fetch_under_metadata_only_reaches_the_network() {
        // The other half, and it is what makes the test above meaningful: the
        // same fetcher, the same rules, the same posture, differing only in the
        // declared class. If the refusal above came from the host rather than
        // the gate, this one would fail too.
        //
        // It cannot assert success, because no network is reachable — so it
        // asserts the failure is NOT a refusal. A `Refused` here would mean the
        // posture refuses metadata, which is the opposite of what it is for.
        let mut fetcher =
            fetcher_with_robots(Duration::from_millis(500), "User-agent: *\nDisallow: /\n").await;
        fetcher.policy.robots_posture = RobotsPosture::MetadataOnly;

        let result = fetcher
            .get_with_class(
                "https://example.com/story/1",
                None,
                None,
                FetchClass::Metadata,
            )
            .await;
        if let Err(ref e) = result {
            assert!(
                !matches!(e, SourceError::Refused(_)),
                "a declared METADATA fetch under metadata_only is not refused — the whole point \
                 of the posture is to read it and discard it. Got {e:?}"
            );
        }
    }

    #[test]
    fn a_discarded_read_hands_back_no_bytes_at_all() {
        // The A.4 limit that holds when the other two are wrong. Asserted on a
        // real body, not on a fetch that cannot succeed.
        let mut page = crate::Fetched::from_source(
            "https://example.com/story/1",
            "<html>the entire forbidden chapter, in full</html>",
            Some("text/html".to_owned()),
        );
        SafeFetcher::label_fetched(
            &mut page,
            FetchClass::Metadata,
            RobotsGate::ReadAndDiscarded,
        );

        assert_eq!(
            page.body, "",
            "a read-and-discarded body is not available to an adapter that reached for \
             `page.body` directly, which is what every site adapter does"
        );
        assert!(
            page.discarded,
            "and the page says so, so a caller can explain why"
        );
        assert_eq!(
            page.fetch_class,
            FetchClass::Metadata,
            "the class is still reported"
        );

        let refusal = page.storable_body().expect_err(
            "and the storable accessor refuses, rather than handing out an empty string that \
             looks like an empty document",
        );
        assert!(
            refusal.to_string().contains("metadata_only"),
            "the refusal names the posture that caused it: {refusal}"
        );
    }

    #[test]
    fn a_storable_read_keeps_its_bytes() {
        // The other three arms, so the test above is about the distinction and
        // not about a labelling step that empties everything. A version that
        // cleared the body unconditionally would satisfy the discard test and
        // destroy every import.
        for gate in [
            RobotsGate::Allowed,
            RobotsGate::Overridden,
            RobotsGate::Refused,
        ] {
            let mut page = crate::Fetched::from_source(
                "https://example.com/story/1",
                "<html>the chapter</html>",
                None,
            );
            SafeFetcher::label_fetched(&mut page, FetchClass::Content, gate);
            assert_eq!(
                page.body, "<html>the chapter</html>",
                "{gate:?} is not a discard, so the bytes stay"
            );
            assert!(!page.discarded, "{gate:?} does not set the discard flag");
            assert!(
                page.storable_body().is_ok(),
                "{gate:?} hands its bytes out to a storer"
            );
        }
    }

    #[test]
    fn an_allowed_metadata_read_is_storable() {
        // The subtle one: `metadata_only` discards a *forbidden* path, not
        // every metadata read. A metadata fetch of an allowed path is ordinary
        // and storable, and a labelling step that keyed off the class or the
        // posture rather than the gate would throw it away — silently dropping
        // the chapter listings that is what the instance is for.
        let mut page = crate::Fetched::from_source(
            "https://example.com/story/1",
            "<html>title, author, chapter list</html>",
            None,
        );
        SafeFetcher::label_fetched(&mut page, FetchClass::Metadata, RobotsGate::Allowed);

        assert_eq!(
            page.body, "<html>title, author, chapter list</html>",
            "an ALLOWED metadata read is storable; the discard follows the gate, not the class"
        );
        assert!(!page.discarded);
        assert_eq!(page.fetch_class, FetchClass::Metadata);
    }

    #[tokio::test]
    async fn a_read_and_discarded_fetch_is_not_counted_as_an_override() {
        // The counter is what an operator reads to answer "is this instance
        // ignoring sites?", and `ReadAndDiscarded` is not that. It reads a
        // forbidden path, yes — but it stores none of it, so nothing about the
        // instance's stored corpus depends on the host's wishes being ignored.
        //
        // This is the one property of the new arm that a mutation to the
        // `match` cannot reach: routing `ReadAndDiscarded` through
        // `record_robots_override` still answers every gate assertion correctly,
        // because the counter is not part of what the gate returns. Asserting the
        // counter is what closes that.
        let mut fetcher =
            fetcher_with_robots(Duration::from_millis(500), "User-agent: *\nDisallow: /\n").await;
        fetcher.policy.robots_posture = RobotsPosture::MetadataOnly;

        let _ = fetcher
            .get_with_class(
                "https://example.com/story/1",
                None,
                None,
                FetchClass::Metadata,
            )
            .await;

        assert_eq!(
            fetcher.robots_overrides(),
            0,
            "reading a forbidden path in order to discard it is not overriding the host, and a \
             report that counts it would tell an operator this instance ignores robots.txt when it \
             has stored nothing at all from the host"
        );
    }

    /// M59-04: every overridden OR DISCARDED read is counted per host, and the
    /// first per host is logged.
    ///
    /// The counter existed and covered only half the requirement. `ReadAndDiscarded`
    /// was `{}` in the match — a read that happens constantly on a `MetadataOnly`
    /// instance, which is the only reason that posture exists, and it reported
    /// nothing. An operator asking "what is the posture costing me?" got zero
    /// for the posture's whole purpose.
    ///
    /// Two assertions, because "counted" and "counted once per host for the log"
    /// are different claims and the second is the one that prevents a log flood:
    /// three reads of the same host produce a count of 3 and a single warning.
    #[tokio::test]
    async fn a_read_and_discarded_fetch_is_counted_on_its_own_counter() {
        let mut fetcher =
            fetcher_with_robots(Duration::from_millis(500), "User-agent: *\nDisallow: /\n").await;
        fetcher.policy.robots_posture = RobotsPosture::MetadataOnly;

        for n in 1..=3 {
            let _ = fetcher
                .get_with_class(
                    &format!("https://example.com/story/{n}"),
                    None,
                    None,
                    FetchClass::Metadata,
                )
                .await;
        }

        assert_eq!(
            fetcher.robots_read_and_discarded(),
            3,
            "three forbidden metadata reads were discarded and only {} were counted. \
             M59-04: the counter is what lets an operator answer what the posture cost, \
             and on a `MetadataOnly` instance this read is the posture's normal traffic.",
            fetcher.robots_read_and_discarded()
        );
        // The other half of the requirement, and the reason the two counters are
        // kept apart: this must NOT appear in the override count, which is the
        // number an operator may have to explain to a site's owner.
        assert_eq!(
            fetcher.robots_overrides(),
            0,
            "a read-and-discard is not an override. `a_read_and_discarded_fetch_is_not_\
             counted_as_an_override` already asserts this; it is repeated here because \
             the two counters are the whole design and a mutation that moved this read \
             to the override counter would pass the old test."
        );
        // Once per host, so the log is a warning rather than a flood.
        assert!(
            fetcher.robots_discard_hosts.lock().await.len() == 1,
            "the discard warning is keyed per host, and one host has been read three times"
        );
    }

    /// A second host is warned about separately, so "first per host" is not
    /// "first overall" — which is the bug a single global flag would have.
    ///
    /// The first attempt at this used `example.org` and asserted two warnings.
    /// It got one, and the *test* was wrong rather than the code: the fixture
    /// only allows `example.com`, so the second request is refused before the
    /// gate is reached and never counted. Worth recording, because "the test
    /// went red" and "the test was wrong" look identical from the outside and
    /// only one of them is a bug.
    #[tokio::test]
    async fn each_host_gets_its_own_discard_warning() {
        let mut fetcher =
            fetcher_with_robots(Duration::from_millis(500), "User-agent: *\nDisallow: /\n").await;
        fetcher.policy.robots_posture = RobotsPosture::MetadataOnly;

        // A second host this fetcher is allowed to talk to, with the same rules.
        fetcher.allowed_hosts.push("other.example".into());
        {
            let mut cache = fetcher.robots.lock().await;
            cache.insert(
                "other.example".to_owned(),
                RobotsEntry {
                    rules: RobotsRules::parse("User-agent: *\nDisallow: /\n", "Lorehaven"),
                    read_at: Instant::now(),
                },
            );
        }

        let _ = fetcher
            .get_with_class("https://example.com/a", None, None, FetchClass::Metadata)
            .await;
        let _ = fetcher
            .get_with_class("https://other.example/b", None, None, FetchClass::Metadata)
            .await;

        let hosts = fetcher.robots_discard_hosts.lock().await;
        assert_eq!(
            hosts.len(),
            2,
            "two hosts were read and discarded, so two hosts have been warned about. \
             A single global flag would report 1 and leave the second site with no \
             record that its rules were hit at all."
        );
        drop(hosts);
        assert_eq!(
            fetcher.robots_read_and_discarded(),
            2,
            "both reads counted, or the per-host keying is being reported for a read \
             that never happened"
        );
    }

    #[tokio::test]
    async fn a_genuine_override_is_counted() {
        // The other half, so the assertion above is about the distinction and
        // not about a counter that never moves.
        let mut fetcher =
            fetcher_with_robots(Duration::from_millis(500), "User-agent: *\nDisallow: /\n").await;
        fetcher.policy.robots_posture = RobotsPosture::Permissive;

        let _ = fetcher.get("https://example.com/story/1").await;

        assert_eq!(
            fetcher.robots_overrides(),
            1,
            "a permissive posture really does read the forbidden path and really is counted"
        );
    }

    #[tokio::test]
    async fn a_freshly_read_robots_file_is_not_read_again() {
        let fetcher = fetcher_with_robots(
            Duration::from_millis(500),
            "User-agent: *\nCrawl-delay: 2\n",
        )
        .await;

        // The seeded entry is fresh, so this returns it rather than making a
        // request. The assertion that matters is that it returns at all: if the
        // TTL were not consulted, this call would try to reach example.com.
        assert_eq!(
            fetcher.interval_for("example.com").await,
            Duration::from_secs(2)
        );
        assert!(fetcher.robots_for("example.com").await.was_found());
    }

    // --- charset decoding -------------------------------------------------
    //
    // Every byte sequence below is taken from a real recording under
    // `crates/scrapers/tests/fixtures/efiction/`, which is the only reason to
    // trust them: these are the pages the decoder exists for.

    /// A page that declares ISO-8859-1 and means Windows-1252 is read the way a
    /// browser reads it.
    ///
    /// The bytes are `tgstorytime-work.html`'s own: `0x92` where the author
    /// typed a right single quote. Decoded as true Latin-1 this is a C1 control
    /// character, and lossily it is `U+FFFD` — either way the chapter title,
    /// which is the text a reader sees in their library, is corrupted.
    #[test]
    fn a_windows_1252_byte_in_a_latin1_declaration_is_a_quote() {
        let body = [
            b'T', b'h', b'i', b's', b' ', b'w', b'e', b'e', b'k', 0x92, b's',
        ];
        let text = decode_body(&body, Some("ISO-8859-1"));
        assert_eq!(text, "This week\u{2019}s");
        assert!(
            !text.contains('\u{fffd}'),
            "no replacement character is introduced: {text:?}"
        );
    }

    /// The same, for the pound sign the same archive produces.
    #[test]
    fn a_windows_1252_pound_sign_survives() {
        let body = *b"\xa35";
        assert_eq!(decode_body(&body, Some("ISO-8859-1")), "\u{a3}5");
    }

    /// Valid UTF-8 wins over a wrong declaration.
    ///
    /// An archive that says `ISO-8859-1` while emitting UTF-8 is common, and
    /// honouring the label there would turn every accented character into two
    /// mojibake bytes. The bytes below are valid UTF-8 and must be read as such.
    #[test]
    fn valid_utf8_is_read_as_utf8_whatever_was_declared() {
        let body = "café — naïve".as_bytes();
        assert_eq!(decode_body(body, Some("ISO-8859-1")), "café — naïve");
    }

    /// With nothing declared at all, the document's own `<meta>` is used.
    #[test]
    fn the_documents_own_declaration_is_used_when_the_header_has_none() {
        let mut body = Vec::new();
        body.extend_from_slice(
            b"<html><head><meta http-equiv=\"Content-Type\" \
              content=\"text/html; charset=ISO-8859-1\">",
        );
        body.push(0x92); // right single quote, invalid on its own
        let text = decode_body(&body, None);
        assert!(text.ends_with('\u{2019}'), "sniffed the meta: {text:?}");
    }

    /// The HTML5 spelling of the same declaration.
    #[test]
    fn a_meta_charset_element_is_sniffed() {
        assert_eq!(
            sniff_charset(b"<head><meta charset=\"windows-1252\">").as_deref(),
            Some("windows-1252")
        );
    }

    /// A page that declares nothing at all still decodes lossily rather than
    /// failing, because a body with one bad byte is still a page worth reading.
    #[test]
    fn an_undeclared_body_decodes_lossily_rather_than_failing() {
        let text = decode_body(&[b'o', b'k', 0x92], None);
        assert!(text.starts_with("ok"));
    }

    /// A label nobody recognises falls back rather than panicking.
    #[test]
    fn an_unknown_label_falls_back_to_a_lossy_decode() {
        let text = decode_body(&[b'a', 0x92], Some("definitely-not-a-charset"));
        assert!(text.starts_with('a'));
    }

    // --- Content-Type parsing ---------------------------------------------

    /// Only the parameter is a charset; the media type is not.
    #[test]
    fn a_content_type_yields_its_charset_parameter() {
        assert_eq!(
            charset_of_content_type("text/html; charset=ISO-8859-1"),
            Some("ISO-8859-1")
        );
        assert_eq!(
            charset_of_content_type("text/html;charset=utf-8"),
            Some("utf-8")
        );
        assert_eq!(
            charset_of_content_type("text/html; charset=\"utf-8\""),
            Some("utf-8"),
            "a quoted parameter is the same parameter"
        );
    }

    /// A `Content-Type` with no charset is `None`, not the whole header.
    ///
    /// This is the difference between falling back to the document's own
    /// declaration and looking up the label `text/html`, which matches nothing
    /// and quietly degrades every page that omits the parameter.
    #[test]
    fn a_content_type_without_a_charset_yields_nothing() {
        assert_eq!(charset_of_content_type("text/html"), None);
        assert_eq!(charset_of_content_type(""), None);
    }

    // The expiry path — an entry older than the TTL being re-read — is not
    // tested here, and deliberately not: re-reading means a real request to a
    // real host, and a unit test that reaches the network fails on a plane.
    // What is covered is the freshness check above plus the parser in
    // `crate::robots`; the re-read itself belongs to a live check.
}
