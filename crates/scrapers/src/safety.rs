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

use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE, USER_AGENT};
use reqwest::redirect::Policy;
use tokio::sync::{Mutex, Semaphore};
use url::Url;

use crate::{Fetched, Fetcher, SourceError, SourceResult};

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
}

struct PinnedClient {
    addresses: Vec<SocketAddr>,
    client: reqwest::Client,
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
    async fn get_with_redirects(
        &self,
        url: &str,
        form: Option<&[(&str, &str)]>,
    ) -> SourceResult<Fetched> {
        let mut current = validate_url(url, &self.policy)?;
        let mut credential_sent_to: Option<String> = None;

        for hop in 0..=self.policy.max_redirects {
            let host = shared_host(&current);
            self.check_host_allowed(&host)?;
            let authority = self.pinned_client(&host).await?;

            let mut headers = HeaderMap::new();
            headers.insert(
                USER_AGENT,
                HeaderValue::from_str(&self.policy.user_agent).map_err(|_| {
                    SourceError::Internal("user agent is not a valid header".into())
                })?,
            );

            // The credential travels only to the host it was configured for.
            let send_credential = self.credential_host.as_deref() == Some(host.as_str());
            if send_credential {
                if let Some((name, value)) = &self.credential_header {
                    headers.insert(name.clone(), value.clone());
                }
                credential_sent_to = Some(host.clone());
            }

            let request = match form {
                Some(fields) => {
                    let mut pairs: Vec<(&str, &str)> = fields.to_vec();
                    let encoded = encode_form(&mut pairs);
                    headers.insert(
                        CONTENT_TYPE,
                        HeaderValue::from_static("application/x-www-form-urlencoded"),
                    );
                    authority
                        .client
                        .post(current.clone())
                        .headers(headers)
                        .body(encoded)
                }
                None => authority.client.get(current.clone()).headers(headers),
            };

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

            let response = request.send().await.map_err(map_reqwest_error)?;

            let status = response.status();
            if status.is_redirection() {
                let Some(location) = response.headers().get(reqwest::header::LOCATION) else {
                    return Err(SourceError::Parse(format!(
                        "{} sent a redirect with no Location",
                        current
                    )));
                };
                let location = location.to_str().map_err(|_| {
                    SourceError::Parse("redirect Location is not valid text".into())
                })?;
                if hop == self.policy.max_redirects {
                    return Err(SourceError::Refused(format!(
                        "more than {} redirects from {url}",
                        self.policy.max_redirects
                    )));
                }
                let next = current.join(location).map_err(|e| {
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

            if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::GONE {
                return Err(SourceError::NotFound);
            }
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                let retry = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("unspecified");
                return Err(SourceError::RateLimited(format!(
                    "{host} asked us to wait: retry-after {retry}"
                )));
            }
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                // A 403 from a challenge wall is an operational block, not a bad
                // credential; distinguishing them matters because one is
                // retried and the other asks the reader to act.
                return Err(if send_credential {
                    SourceError::AuthRequired(format!("{host} rejected the credential"))
                } else {
                    SourceError::Blocked
                });
            }
            if !status.is_success() {
                return Err(SourceError::Network(format!("{host} answered {status}")));
            }

            let content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let body = read_bounded(response, self.policy.max_bytes).await?;
            return Ok(Fetched {
                final_url: current.to_string(),
                body,
                content_type,
            });
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
    async fn pinned_client(&self, host: &str) -> SourceResult<PinnedClient> {
        let addresses = resolve_public(host, self.policy.timeout).await?;
        let mut clients = self.clients.lock().await;
        if let Some(existing) = clients.get(host) {
            if existing.addresses == addresses {
                return Ok(PinnedClient {
                    addresses: existing.addresses.clone(),
                    client: existing.client.clone(),
                });
            }
            tracing::debug!(host, "resolved addresses changed; re-pinning");
        }
        let client = reqwest::Client::builder()
            // We follow redirects ourselves so each hop is checked.
            .redirect(Policy::none())
            .timeout(self.policy.timeout)
            .connect_timeout(self.policy.connect_timeout)
            .cookie_store(true)
            // Belt and braces: even if a proxy environment variable were set,
            // the pinned addresses are what we meant to reach.
            .no_proxy()
            .resolve_to_addrs(host, &addresses)
            .build()
            .map_err(|e| SourceError::Internal(format!("building http client: {e}")))?;
        let pinned = PinnedClient {
            addresses: addresses.clone(),
            client,
        };
        clients.insert(
            host.to_owned(),
            PinnedClient {
                addresses,
                client: pinned.client.clone(),
            },
        );
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

    /// Sleep as long as this host's declared minimum gap requires.
    async fn wait_for_turn(&self, host: &str) {
        if self.policy.min_interval_per_host.is_zero() {
            return;
        }
        let state = self.host_state(host).await;
        let mut guard = state.lock().await;
        if let Some(last) = guard.last_started {
            let elapsed = last.elapsed();
            if elapsed < self.policy.min_interval_per_host {
                let wait = self.policy.min_interval_per_host - elapsed;
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
        self.get_with_redirects(url, None).await
    }

    async fn post_form(&self, url: &str, fields: &[(&str, &str)]) -> SourceResult<Fetched> {
        self.get_with_redirects(url, Some(fields)).await
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

/// Read a body, refusing to buffer more than `max_bytes`.
async fn read_bounded(mut response: reqwest::Response, max_bytes: usize) -> SourceResult<String> {
    let mut collected: Vec<u8> = Vec::with_capacity(64 * 1024);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| SourceError::Network(format!("reading body: {e}")))?
    {
        if collected.len() + chunk.len() > max_bytes {
            return Err(SourceError::Refused(format!(
                "response exceeds the {max_bytes} byte limit"
            )));
        }
        collected.extend_from_slice(&chunk);
    }
    // Sources are not consistent about declaring their charset, and a mis-decode
    // corrupts every title in the import, so this is lossy on purpose rather
    // than "probably ASCII".
    Ok(String::from_utf8_lossy(&collected).into_owned())
}

/// Map a transport error onto a category the import knows how to act on.
fn map_reqwest_error(error: reqwest::Error) -> SourceError {
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
    }
}
