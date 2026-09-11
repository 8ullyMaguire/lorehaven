//! Which HTTP stack opens the socket, and how a bot wall is recognised.
//!
//! # Why there is a seam here at all
//!
//! `SafeFetcher`'s value is not that it makes requests — it is everything it does
//! *around* a request: resolving a host itself, discarding the answers that point
//! into private networks, pinning the survivors so a later DNS answer cannot move
//! the connection, walking redirects by hand so each hop is checked, bounding the
//! body while it is read, and pacing the whole thing against `robots.txt`. None
//! of that is transport-specific, and a second fetcher written beside the first
//! would drift out of step with it within a milestone.
//!
//! So the transport is the only thing that varies. `Engine::Plain` is the
//! `reqwest` client the crate has always used. `Engine::Impersonating` is the
//! same request sent by a client whose TLS and HTTP/2 fingerprints are a browser's
//! — which is the only thing that distinguishes a request Cloudflare challenges
//! from one it serves. Both are built from the *same* address list, so the guard
//! is not weakened by choosing the second; `resolve_to_addrs`, `redirect(none)`
//! and the bounded read are all still in force. The impersonating engine is not a
//! way around the guard, it is a different socket under the same guard.
//!
//! # Why impersonation is a feature
//!
//! It is off by default and pulls its own HTTP/TLS stack (`primp` is a fork of
//! `reqwest`, and brings `primp-hyper`, `primp-h2`, `primp-rustls` with it — some
//! twenty-five crates). An operator who never imports from a challenged source
//! should not carry that, and a crate consumer should not be made to. The binary
//! this workspace builds turns it on; the library does not assume it.

use crate::safety::FetchPolicy;
use crate::SourceError;
use std::net::SocketAddr;
#[cfg(feature = "cloudflare-impersonation")]
use std::sync::Arc;
use url::Url;

/// A browser to imitate when a source is behind a fingerprint wall.
///
/// A named browser rather than a set of knobs, because the thing being imitated
/// is a *coherent* fingerprint: the TLS ClientHello, the HTTP/2 settings frame
/// order, the header order and the `User-Agent` have to agree with each other, or
/// the result is a client that looks like no browser at all and is challenged
/// more readily than the plain one. `primp` holds the coherent sets; picking one
/// of them is the whole API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Impersonation {
    /// Chrome, at a version `primp` chooses.
    #[default]
    Chrome,
    /// Edge.
    Edge,
    /// Firefox.
    Firefox,
    /// Safari.
    Safari,
}

impl Impersonation {
    /// The name used in configuration and in the source catalogue.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::Edge => "edge",
            Self::Firefox => "firefox",
            Self::Safari => "safari",
        }
    }
}

/// One reply, normalised across transports.
///
/// Deliberately not either stack's response type: the caller's job is to decide
/// whether this is a page, a redirect or a challenge, and that decision must not
/// be written twice.
#[derive(Debug)]
pub(crate) struct EngineReply {
    /// The status code.
    pub status: u16,
    /// The `Location` header, on a redirect.
    pub location: Option<String>,
    /// The `Content-Type` header, when the server sent one.
    pub content_type: Option<String>,
    /// The `ETag`, when the server sent one.
    pub etag: Option<String>,
    /// The `Last-Modified`, when the server sent one.
    pub last_modified: Option<String>,
    /// The `Retry-After` header, when the source sent one with a `429`.
    pub retry_after: Option<String>,
    /// The decoded body. Empty on a redirect, which is never read.
    pub body: String,
    /// The three headers [`is_bot_challenge`] reads, kept because the header map
    /// itself belongs to a response that has been consumed by the time the body
    /// is complete.
    diagnostics: Vec<(&'static str, String)>,
}

impl EngineReply {
    /// Whether this reply is a bot wall rather than the page that was asked for.
    ///
    /// The body has to have been read for this to be knowable, which is why the
    /// fetcher asks after reading rather than before.
    #[must_use]
    pub fn is_challenge(&self) -> bool {
        is_bot_challenge(self.status, &self.diagnostics, &self.body)
    }
}

/// Which stack opens the socket.
///
/// Cheap to clone — both stacks hold a handle to shared connection pools — so a
/// pinned client can be cached and handed out without reconnecting.
#[derive(Clone)]
pub(crate) enum Engine {
    /// `reqwest`, as rustls presents it.
    Plain(reqwest::Client),
    /// A browser's TLS and HTTP/2 fingerprint, via `primp`.
    #[cfg(feature = "cloudflare-impersonation")]
    Impersonating(Arc<primp::Client>),
}

impl Engine {
    /// Build an engine pinned to `addresses`.
    ///
    /// Every client this returns has automatic redirect following disabled and is
    /// bound to the addresses it was given. Those two are the whole guard, and
    /// they are set here rather than at the call site so a new transport cannot
    /// arrive without them.
    ///
    /// # Errors
    /// [`SourceError::Unsupported`] when impersonation was asked for by a build
    /// that does not carry it, and [`SourceError::Internal`] if a client cannot be
    /// constructed at all.
    pub(crate) fn build(
        impersonation: Option<Impersonation>,
        host: &str,
        addresses: &[SocketAddr],
        policy: &FetchPolicy,
    ) -> crate::SourceResult<Self> {
        match impersonation {
            None => {
                let client = reqwest::Client::builder()
                    // We follow redirects ourselves so each hop is checked.
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(policy.timeout)
                    .connect_timeout(policy.connect_timeout)
                    .cookie_store(true)
                    // Belt and braces: even if a proxy environment variable were
                    // set, the pinned addresses are what we meant to reach.
                    .no_proxy()
                    .resolve_to_addrs(host, addresses)
                    .build()
                    .map_err(|e| SourceError::Internal(format!("building http client: {e}")))?;
                Ok(Self::Plain(client))
            }
            Some(wanted) => Self::impersonating(wanted, host, addresses, policy),
        }
    }

    #[cfg(feature = "cloudflare-impersonation")]
    fn impersonating(
        wanted: Impersonation,
        host: &str,
        addresses: &[SocketAddr],
        policy: &FetchPolicy,
    ) -> crate::SourceResult<Self> {
        use primp::Impersonate;
        let profile = match wanted {
            Impersonation::Chrome => Impersonate::Chrome,
            Impersonation::Edge => Impersonate::Edge,
            Impersonation::Firefox => Impersonate::Firefox,
            Impersonation::Safari => Impersonate::Safari,
        };
        let client = primp::ClientBuilder::new()
            .impersonate(profile)
            // The fingerprint has to be one machine's, not a blend: a Chrome
            // ClientHello announcing a macOS platform from a Linux server is a
            // combination no real browser produces.
            .impersonate_os(impersonate_os())
            // Identical guarantees to the plain engine, and for the same reasons.
            .redirect(primp::redirect::Policy::none())
            .timeout(policy.timeout)
            .connect_timeout(policy.connect_timeout)
            .cookie_store(true)
            .no_proxy()
            .resolve_to_addrs(host, addresses)
            .build()
            .map_err(|e| SourceError::Internal(format!("building impersonating client: {e}")))?;
        Ok(Self::Impersonating(Arc::new(client)))
    }

    #[cfg(not(feature = "cloudflare-impersonation"))]
    fn impersonating(
        wanted: Impersonation,
        _host: &str,
        _addresses: &[SocketAddr],
        _policy: &FetchPolicy,
    ) -> crate::SourceResult<Self> {
        Err(SourceError::Unsupported(format!(
            "this source is behind a challenge wall and needs the {} fingerprint, \
             but this build was compiled without the `cloudflare-impersonation` feature",
            wanted.as_str()
        )))
    }

    /// Send one request and read the reply.
    ///
    /// Automatic redirect following is off, so a `3xx` comes back as a `3xx` and
    /// the caller decides what to do with it. The body is not read on a redirect:
    /// it is not the resource, and reading it would be spending a source's
    /// bandwidth on bytes nobody will look at.
    ///
    /// # Errors
    /// [`SourceError::Network`] if the request cannot be completed, and
    /// [`SourceError::Refused`] if the body exceeds `max_bytes`.
    pub(crate) async fn send(
        &self,
        url: &Url,
        form: Option<&str>,
        headers: reqwest::header::HeaderMap,
        max_bytes: usize,
    ) -> crate::SourceResult<EngineReply> {
        match self {
            Self::Plain(client) => {
                let request = match form {
                    Some(encoded) => client.post(url.clone()).body(encoded.to_owned()),
                    None => client.get(url.clone()),
                }
                .headers(headers);
                let mut response = request
                    .send()
                    .await
                    .map_err(crate::safety::map_reqwest_error)?;
                let parts = ReplyParts::from_plain(response.status().as_u16(), response.headers());
                if parts.is_redirect() {
                    return Ok(parts.into_reply(String::new()));
                }
                let charset = parts.charset.clone();
                let content_encoding = parts.content_encoding.clone();
                let mut guard = BodyGuard::new(max_bytes);
                while let Some(chunk) = response
                    .chunk()
                    .await
                    .map_err(|e| SourceError::Network(format!("reading body: {e}")))?
                {
                    guard.push(&chunk)?;
                }
                Ok(parts.into_reply(guard.finish(charset.as_deref(), content_encoding.as_deref())))
            }
            #[cfg(feature = "cloudflare-impersonation")]
            Self::Impersonating(client) => {
                let request = match form {
                    Some(encoded) => client.post(url.clone()).body(encoded.to_owned()),
                    None => client.get(url.clone()),
                }
                .headers(headers);
                let mut response = request
                    .send()
                    .await
                    .map_err(|e| SourceError::Network(format!("{url}: {e}")))?;
                let parts = ReplyParts::from_plain(response.status().as_u16(), response.headers());
                if parts.is_redirect() {
                    return Ok(parts.into_reply(String::new()));
                }
                let charset = parts.charset.clone();
                let content_encoding = parts.content_encoding.clone();
                let mut guard = BodyGuard::new(max_bytes);
                while let Some(chunk) = response
                    .chunk()
                    .await
                    .map_err(|e| SourceError::Network(format!("reading body: {e}")))?
                {
                    guard.push(&chunk)?;
                }
                Ok(parts.into_reply(guard.finish(charset.as_deref(), content_encoding.as_deref())))
            }
        }
    }
}

/// The OS a fingerprint should claim.
///
/// The server runs Linux, and saying so is both true and the least surprising
/// choice: claiming Windows or macOS would be a second thing that is not what it
/// appears to be, for no gain — Cloudflare challenges Linux browsers and serves
/// them perfectly well.
#[cfg(feature = "cloudflare-impersonation")]
fn impersonate_os() -> primp::ImpersonateOS {
    primp::ImpersonateOS::Linux
}

/// The response parts the fetcher reads, lifted out of either stack's types.
struct ReplyParts {
    status: u16,
    location: Option<String>,
    content_type: Option<String>,
    etag: Option<String>,
    last_modified: Option<String>,
    retry_after: Option<String>,
    charset: Option<String>,
    /// The `Content-Encoding` the source sent, if any.
    ///
    /// Collected because a source may compress whether or not the request asked,
    /// and the body must be undone before it is read as text. See
    /// [`crate::safety::decode_response_body`].
    content_encoding: Option<String>,
    diagnostics: Vec<(&'static str, String)>,
}

impl ReplyParts {
    fn from_plain(status: u16, headers: &reqwest::header::HeaderMap) -> Self {
        let get = |name: &str| -> Option<String> {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let content_type = get("content-type");
        let charset = content_type
            .as_deref()
            .and_then(crate::safety::charset_of_content_type)
            .map(str::to_owned);
        // The three headers the challenge test reads. Collected here because the
        // body may be read long after the response is consumed.
        let diagnostics = vec![
            ("cf-mitigated", get("cf-mitigated").unwrap_or_default()),
            ("server", get("server").unwrap_or_default()),
            ("server-timing", get("server-timing").unwrap_or_default()),
        ];
        Self {
            status,
            location: get("location"),
            content_type,
            etag: get("etag"),
            last_modified: get("last-modified"),
            retry_after: get("retry-after"),
            charset,
            content_encoding: get("content-encoding"),
            diagnostics,
        }
    }

    fn is_redirect(&self) -> bool {
        (300..400).contains(&self.status)
    }

    fn into_reply(self, body: String) -> EngineReply {
        EngineReply {
            status: self.status,
            location: self.location,
            content_type: self.content_type,
            etag: self.etag,
            last_modified: self.last_modified,
            retry_after: self.retry_after,
            body,
            diagnostics: self.diagnostics,
        }
    }
}

/// A body being read under a ceiling.
///
/// A `Vec` with a limit rather than a `Content-Length` check, because
/// `Content-Length` is a claim: a source that understates it, or omits it, or
/// sends a compressed body, would otherwise be able to make the importer allocate
/// without bound. The count is of the bytes actually received.
struct BodyGuard {
    max: usize,
    collected: Vec<u8>,
}

impl BodyGuard {
    fn new(max: usize) -> Self {
        Self {
            max,
            collected: Vec::with_capacity(64 * 1024),
        }
    }

    fn push(&mut self, chunk: &[u8]) -> crate::SourceResult<()> {
        if self.collected.len() + chunk.len() > self.max {
            return Err(SourceError::Refused(format!(
                "response exceeds the {} byte limit",
                self.max
            )));
        }
        self.collected.extend_from_slice(chunk);
        Ok(())
    }

    fn finish(self, charset: Option<&str>, content_encoding: Option<&str>) -> String {
        crate::safety::decode_response_body(&self.collected, charset, content_encoding)
    }
}

/// Whether a reply is a bot wall rather than the page.
///
/// # What this has to get right
///
/// Both directions are costly. A false positive sends a perfectly good page to a
/// solver service — slow, and it spends somebody else's capacity to read a page we
/// already had. A false negative is worse: the importer parses the challenge page
/// as though it were the work, finds none of an adapter's selectors, and records a
/// work with no chapters. That is the failure mode this crate exists to avoid, so
/// the test is written to be *specific* rather than eager, and every signal in it
/// was read off a real challenge response on 2026-09-11:
///
/// | Source | What its challenge actually sent |
/// |---|---|
/// | `fimfiction.net` | `403`, `cf-mitigated: challenge`, `server: cloudflare`, `server-timing: chlray;desc="…"`, title `Just a moment...`, body containing `__cf_chl` |
/// | `scribblehub.com` | `403`, `server: cloudflare`, **no** `cf-mitigated`, title `Attention Required! \| Cloudflare` |
/// | `fictionpress.com` | `403`, `cf-mitigated: challenge`, title `Just a moment...` |
///
/// # The signal that is deliberately absent
///
/// `challenge-platform` — the URL of Cloudflare's challenge script — appears in the
/// **`<script>` tag of ordinary served pages**. A real `fanfiction.net` chapter page
/// contains it, as does every `royalroad.com` page. Matching on it would mark the
/// successful reads as failures, which is exactly the mistake an earlier draft of
/// the plan made and a live probe caught. The signals below are the ones that only
/// appear on a wall.
///
/// A marker is only trusted on a status that indicates refusal. A `200` containing
/// the phrase `Just a moment` is a page about a moment, not a wall.
#[must_use]
pub fn is_bot_challenge(status: u16, headers: &[(&str, String)], body: &str) -> bool {
    let refused = matches!(status, 403 | 429 | 503);
    if !refused {
        return false;
    }
    let header = |name: &str| -> Option<&str> {
        headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .filter(|v| !v.is_empty())
    };
    // Cloudflare sets this on a challenge it served itself. It is the most
    // precise signal available and needs no body at all.
    if let Some(mitigated) = header("cf-mitigated") {
        if mitigated.eq_ignore_ascii_case("challenge") {
            return true;
        }
    }
    // The challenge ray's identifier, in the timing header of a challenge only.
    if let Some(timing) = header("server-timing") {
        if timing.to_ascii_lowercase().contains("chlray") {
            return true;
        }
    }
    // Body markers. These are the challenge page's own words, not a script URL.
    const MARKERS: [&str; 3] = [
        "Just a moment",
        "__cf_chl",
        "Attention Required! | Cloudflare",
    ];
    MARKERS.iter().any(|marker| body.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
        pairs.iter().map(|(k, v)| (*k, (*v).to_owned())).collect()
    }

    #[test]
    fn a_cloudflare_challenge_header_is_a_wall() {
        // Exactly what fimfiction.net answered on 2026-09-11.
        assert!(is_bot_challenge(
            403,
            &headers(&[
                ("cf-mitigated", "challenge"),
                ("server", "cloudflare"),
                ("server-timing", "chlray;desc=\"a3950389885a104b\""),
            ]),
            "irrelevant",
        ));
    }

    #[test]
    fn scribblehubs_block_page_is_a_wall_without_the_header() {
        // scribblehub.com sends no `cf-mitigated`; the title is the signal.
        assert!(is_bot_challenge(
            403,
            &headers(&[("server", "cloudflare")]),
            "<title>Attention Required! | Cloudflare</title>",
        ));
    }

    #[test]
    fn the_challenge_ray_alone_is_enough() {
        assert!(is_bot_challenge(
            503,
            &headers(&[("server-timing", "chlray;desc=\"abc\"")]),
            "",
        ));
    }

    #[test]
    fn a_real_page_that_merely_loads_the_challenge_script_is_not_a_wall() {
        // The bug this test exists for: `challenge-platform` is the URL of
        // Cloudflare's script and appears in the <script> tag of ordinary served
        // pages — including every fanfiction.net chapter and every royalroad.com
        // page. Matching on it marks successful reads as failures.
        let real_page = "<html><head><title>Jillian Holtzmann: Ace Attorney Chapter 1</title>\
             <script src=\"/cdn-cgi/challenge-platform/h/b/scripts/jsd/main.js\"></script>\
             </head><body><div id=\"storytext\">prose</div></body></html>";
        assert!(!is_bot_challenge(
            200,
            &headers(&[("server", "cloudflare"), ("cf-mitigated", "")]),
            real_page,
        ));
    }

    #[test]
    fn a_marker_on_a_success_status_is_not_a_wall() {
        // A 200 that happens to contain the phrase is a page about a moment.
        assert!(!is_bot_challenge(
            200,
            &headers(&[]),
            "<h1>Just a moment while we load</h1>",
        ));
    }

    #[test]
    fn an_ordinary_page_mentioning_cloudflare_is_not_a_wall() {
        assert!(!is_bot_challenge(
            200,
            &headers(&[("server", "cloudflare")]),
            "<p>Hosted behind Cloudflare. Attention Required! | Cloudflare is their \
             error page.</p>",
        ));
    }

    #[test]
    fn a_404_with_no_markers_is_not_a_wall() {
        assert!(!is_bot_challenge(
            404,
            &headers(&[("server", "cloudflare")]),
            "not found"
        ));
    }

    #[test]
    fn an_empty_403_is_still_not_a_wall_without_a_marker() {
        // A bare 403 from a server that is not Cloudflare must not be escalated:
        // the solver cannot help, and asking it wastes a solve.
        assert!(!is_bot_challenge(
            403,
            &headers(&[("server", "nginx")]),
            "Forbidden"
        ));
    }
}
