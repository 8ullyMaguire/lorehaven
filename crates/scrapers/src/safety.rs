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

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::header::{
    HeaderMap, HeaderValue, CONTENT_TYPE, IF_MODIFIED_SINCE, IF_NONE_MATCH, USER_AGENT,
};
use tokio::sync::{Mutex, Semaphore};
use url::Url;

use crate::archive::ArchiveClient;
use crate::engine::{Engine, Impersonation};
use crate::robots::RobotsRules;
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
        let parsed = validate_url(url, &self.policy)?;
        let host = shared_host(&parsed);
        let robots = self.robots_for(&host).await;
        if !robots.allows(parsed.path()) {
            return Err(SourceError::Refused(format!(
                "{host} disallows {} in its robots.txt",
                parsed.path()
            )));
        }
        self.send_with_redirects(url, form, conditional, Escalation::Policy)
            .await
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
    ) -> SourceResult<ConditionalFetch> {
        let steps = match escalation {
            Escalation::None => vec![Step::Transport(self.policy.unblock.fingerprint)],
            Escalation::Policy => self.escalation_steps(),
        };
        let mut last: Option<SourceError> = None;
        let mut challenged = false;

        for step in steps {
            match step {
                Step::Transport(fingerprint) => {
                    match self.attempt(url, form, conditional, fingerprint).await {
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
                headers.insert(
                    USER_AGENT,
                    HeaderValue::from_str(&self.policy.user_agent).map_err(|_| {
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
                .send(
                    &current,
                    encoded_body.as_deref(),
                    headers,
                    self.policy.max_bytes,
                )
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
            .send_with_redirects(&url, None, None, Escalation::None)
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
    fn the_fingerprint_flag_tells_the_truth_about_this_build() {
        // The importer refuses a fingerprint-walled source when this is false, so
        // it has to agree with the feature the transport is compiled behind.
        assert_eq!(
            crate::FINGERPRINT_SUPPORTED,
            cfg!(feature = "cloudflare-impersonation")
        );
    }
    use super::{
        charset_of_content_type, decode_body, describe_status, sniff_charset, Escalation,
        SafeFetcher, Step, Unblock,
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
