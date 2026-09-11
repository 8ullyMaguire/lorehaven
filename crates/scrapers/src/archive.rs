//! The third escalation tier: an archived copy.
//!
//! # What this is, and what it is not
//!
//! This tier exists for two situations, and it is worth being precise about which,
//! because the difference is the whole ethics of it:
//!
//! * **A work the source still has but will not serve us** — a managed challenge
//!   the solver could not pass either. The archive holds a copy somebody's earlier
//!   crawl made, and reading it is reading the Internet Archive, which is a public
//!   library doing the thing libraries do.
//! * **A work the source no longer has.** Fanfiction is deleted constantly —
//!   by authors withdrawing, by archives closing, by a takedown — and for a site
//!   whose job is preservation (spec §11.11) an archived copy is the difference
//!   between a dead link and a readable work. This is the case the tier is most
//!   valuable for, and the one it is most obviously right for.
//!
//! What it is **not** is a way around a source's stated wishes. The caller reaches
//! this tier only after the source's own `robots.txt` has already been consulted,
//! so a site that says "do not crawl us" is not crawled by this route either.
//! Reading somebody's archive.org copy of a page an author asked to have removed
//! from the live web would be a strange reading of that author's intent, and the
//! decision to route around a `Disallow` does not get made here by accident.
//!
//! # Provenance
//!
//! The body did not come from the source, so [`Fetched::provenance`] says where it
//! did come from. This is not decoration: an adapter that stored `final_url` as a
//! work's canonical URL would otherwise publish a `web.archive.org` address as the
//! work's home. The requested URL is kept as `final_url` and the snapshot is
//! recorded beside it, so a reader can be told the chapter came from an archived
//! copy — which for a preserved work is exactly what they should be told.

use crate::{Fetched, Provenance, SourceError, SourceResult};
use serde::Deserialize;
use std::time::Duration;

/// Where the archive's lookup API lives.
const AVAILABILITY_ENDPOINT: &str = "https://archive.org/wayback/available";
/// Where snapshots are served from.
const SNAPSHOT_BASE: &str = "https://web.archive.org";
/// How many hops the archive's own redirect is followed for. It answers an entry
/// URL with one `302`; more than a couple would mean something is wrong.
const MAX_REDIRECTS: usize = 4;

/// A client for the Internet Archive's Wayback Machine.
pub struct ArchiveClient {
    http: reqwest::Client,
    /// Hosts whose works may be looked up. The same allowlist the rest of the
    /// fetcher uses, re-checked here rather than assumed.
    allowed_hosts: Vec<String>,
    /// The lookup API. Overridable so an operator may use a mirror, and so a
    /// test can exercise this whole tier without reaching the Internet Archive.
    availability_endpoint: String,
    /// Where snapshots are served from. Overridable for the same two reasons.
    /// Whatever it is set to is the host a snapshot URL is checked against, so
    /// overriding it widens where this tier will read from — which is why it is
    /// a deliberate call rather than a constant.
    snapshot_base: String,
}

impl ArchiveClient {
    /// Build an archive client permitted to look up `allowed_hosts`.
    ///
    /// An empty allowlist permits nothing, which is the safe reading: this tier
    /// exists as a fallback, and a fallback with no boundary is just an outbound
    /// request with extra steps.
    ///
    /// # Errors
    /// [`SourceError::Internal`] if the HTTP client cannot be built.
    pub fn new(allowed_hosts: Vec<String>) -> SourceResult<Self> {
        // Redirects are followed by hand, for the same reason the source fetcher
        // does it: the answer to a request is not authority to request wherever it
        // points. The archive's entry URL is a `302` to the snapshot's real
        // address, so a hop happens on the happy path — and a hop is exactly where
        // an unexamined redirect would turn this tier into the arbitrary-URL fetch
        // the rest of the crate is built to avoid.
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| SourceError::Internal(format!("building archive client: {e}")))?;
        Ok(Self {
            http,
            allowed_hosts: allowed_hosts
                .into_iter()
                .map(|host| host.trim_start_matches("www.").to_ascii_lowercase())
                .collect(),
            availability_endpoint: AVAILABILITY_ENDPOINT.to_owned(),
            snapshot_base: SNAPSHOT_BASE.to_owned(),
        })
    }

    /// Look snapshots up somewhere else — a mirror, or a test's stub.
    #[must_use]
    pub fn with_endpoints(
        mut self,
        availability_endpoint: impl Into<String>,
        snapshot_base: impl Into<String>,
    ) -> Self {
        self.availability_endpoint = availability_endpoint.into();
        self.snapshot_base = snapshot_base.into().trim_end_matches('/').to_owned();
        self
    }

    /// Read the archived copy of `url`, if the archive holds one.
    ///
    /// # Errors
    /// [`SourceError::Refused`] if `url` is not on a host this client may look up,
    /// [`SourceError::NotFound`] if the archive holds no snapshot, and
    /// [`SourceError::Network`] if the archive cannot be reached.
    pub async fn get(&self, url: &str) -> SourceResult<Fetched> {
        let parsed = url::Url::parse(url)
            .map_err(|e| SourceError::Internal(format!("archive given a non-URL: {e}")))?;
        self.check_host(&parsed)?;

        let snapshot = self.snapshot_for(url).await?;
        let snapshot_url = snapshot.url.ok_or_else(|| {
            tracing::debug!(url, "the Internet Archive holds no snapshot of this URL");
            SourceError::NotFound
        })?;

        // The snapshot URL comes from the archive's own API, but it is still
        // something we are about to fetch, so it is parsed and its host checked
        // rather than trusted. A reply that pointed anywhere but the archive would
        // otherwise turn this tier into the arbitrary-URL fetch the whole crate
        // is built to avoid.
        let parsed_snapshot = url::Url::parse(&snapshot_url).map_err(|e| {
            SourceError::Parse(format!("the archive named an unusable snapshot: {e}"))
        })?;
        let expected_host = url::Url::parse(&self.snapshot_base)
            .map_err(|e| SourceError::Internal(format!("archive snapshot base: {e}")))?
            .host_str()
            .map(str::to_owned);
        if parsed_snapshot.host_str().map(str::to_owned) != expected_host {
            return Err(SourceError::Refused(format!(
                "the archive pointed at {parsed_snapshot}, which is not {}",
                self.snapshot_base
            )));
        }
        // `id_` asks for the archived bytes themselves. Measured against the real
        // archive: the wrapped form of a page came back at 636 kB where its `id_`
        // form was 93 kB, the difference being the archive's own toolbar and the
        // rewritten URLs inside it — which an adapter's selectors would then be
        // matching against a page that is not the page.
        //
        // A missing timestamp is not a problem: the entry form without one resolves
        // to the *newest* snapshot, which was also verified rather than assumed.
        let raw = format!(
            "{}/web/{}id_/{}",
            self.snapshot_base,
            snapshot.timestamp.as_deref().unwrap_or("2"),
            url
        );

        // The archive answers its entry URL with a `302` to the snapshot's own
        // address, so the happy path is two requests — and every hop is checked
        // against the archive's host before it is made.
        let mut target = raw.clone();
        for _ in 0..=MAX_REDIRECTS {
            let hop = url::Url::parse(&target).map_err(|e| {
                SourceError::Parse(format!("the archive redirected to an unusable URL: {e}"))
            })?;
            if hop.host_str().map(str::to_owned) != expected_host {
                return Err(SourceError::Refused(format!(
                    "the archive redirected to {hop}, which is not {}",
                    self.snapshot_base
                )));
            }

            let response = self
                .http
                .get(hop.as_str())
                .send()
                .await
                .map_err(|e| SourceError::Network(format!("reading the archived copy: {e}")))?;
            let status = response.status();

            if status.is_redirection() {
                let Some(location) = response.headers().get(reqwest::header::LOCATION) else {
                    return Err(SourceError::Parse(
                        "the archive redirected with no Location".to_owned(),
                    ));
                };
                let location = location.to_str().map_err(|_| {
                    SourceError::Parse("the archive's Location is not valid text".to_owned())
                })?;
                target = hop
                    .join(location)
                    .map_err(|e| {
                        SourceError::Parse(format!("the archive's Location {location:?}: {e}"))
                    })?
                    .to_string();
                continue;
            }

            if status == reqwest::StatusCode::NOT_FOUND {
                tracing::debug!(url, "the Internet Archive holds no snapshot of this URL");
                return Err(SourceError::NotFound);
            }
            if !status.is_success() {
                return Err(SourceError::Network(format!(
                    "the archive answered {status} for {url}"
                )));
            }

            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let bytes = response
                .bytes()
                .await
                .map_err(|e| SourceError::Network(format!("reading the archived copy: {e}")))?;

            return Ok(Fetched {
                // The URL asked for, so an adapter storing a canonical URL stores
                // the work's real address and not the archive's.
                final_url: url.to_owned(),
                body: crate::safety::decode_body(&bytes, content_type.as_deref()),
                content_type,
                // An archived page carries the archive's validators, not the
                // source's. Reusing them for a conditional request against the live
                // source would be sending a validator for a different resource.
                etag: None,
                last_modified: None,
                provenance: Provenance::Archive {
                    // The address the bytes actually came from, which after a hop is
                    // the resolved snapshot rather than the entry point.
                    snapshot_url: target,
                    timestamp: snapshot.timestamp,
                },
            });
        }

        Err(SourceError::Refused(format!(
            "the archive redirected more than {MAX_REDIRECTS} times for {url}"
        )))
    }

    /// Ask the archive whether it holds `url`.
    async fn snapshot_for(&self, url: &str) -> SourceResult<Snapshot> {
        let response = self
            .http
            .get(&self.availability_endpoint)
            .query(&[("url", url)])
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("asking the Internet Archive: {e}")))?;
        // Read as text and parse here rather than with `Response::json`, so that a
        // rate-limited or offline archive — which answers with HTML, not JSON —
        // produces a message naming what happened instead of a deserialisation
        // error that reads like a bug in this file.
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| SourceError::Network(format!("reading the Internet Archive: {e}")))?;
        if !status.is_success() {
            return Err(SourceError::Network(format!(
                "the Internet Archive answered {status}: {}",
                text.chars().take(120).collect::<String>()
            )));
        }
        let reply: Availability = serde_json::from_str(&text).map_err(|e| {
            SourceError::Network(format!(
                "the Internet Archive's reply was not what it documents ({e}): {}",
                text.chars().take(120).collect::<String>()
            ))
        })?;
        Ok(reply.archived_snapshots.closest.unwrap_or_default())
    }

    /// Reject a URL that is not for a host this client may look up.
    fn check_host(&self, url: &url::Url) -> SourceResult<()> {
        let host = url
            .host_str()
            .ok_or_else(|| SourceError::Internal("archive given a URL with no host".to_owned()))?;
        let bare = host.trim_start_matches("www.").to_ascii_lowercase();
        let allowed = self
            .allowed_hosts
            .iter()
            .any(|allowed| bare == *allowed || bare.ends_with(&format!(".{allowed}")));
        if allowed {
            Ok(())
        } else {
            Err(SourceError::Refused(format!(
                "{host} is not a host this source may fetch from, so it is \
                 certainly not one to look up in an archive"
            )))
        }
    }
}

/// The archive's answer about one URL.
#[derive(Debug, Deserialize)]
struct Availability {
    #[serde(default)]
    archived_snapshots: ArchivedSnapshots,
}

#[derive(Debug, Default, Deserialize)]
struct ArchivedSnapshots {
    #[serde(default)]
    closest: Option<Snapshot>,
}

/// One snapshot of a URL.
#[derive(Debug, Default, Deserialize)]
struct Snapshot {
    /// Whether the snapshot exists. Present but false when the URL is unknown,
    /// in which case `url` is absent.
    #[serde(default)]
    #[allow(dead_code)]
    available: bool,
    /// The snapshot's own address.
    #[serde(default)]
    url: Option<String>,
    /// When it was taken, `YYYYMMDDhhmmss`.
    #[serde(default)]
    timestamp: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn an_empty_allowlist_permits_nothing() {
        let client = ArchiveClient::new(vec![]).expect("client");
        let error = client
            .check_host(&url::Url::parse("https://www.fanfiction.net/s/1/1/").unwrap())
            .expect_err("must refuse");
        assert!(matches!(error, SourceError::Refused(_)), "got {error:?}");
    }

    #[test]
    fn a_host_the_source_did_not_declare_is_refused() {
        let client = ArchiveClient::new(vec!["fanfiction.net".into()]).expect("client");
        for hostile in ["https://169.254.169.254/x", "https://notfanfiction.net/x"] {
            let parsed = url::Url::parse(hostile).unwrap();
            assert!(
                client.check_host(&parsed).is_err(),
                "{hostile} was permitted"
            );
        }
        let ok = url::Url::parse("https://m.fanfiction.net/s/1/1/").unwrap();
        assert!(
            client.check_host(&ok).is_ok(),
            "a subdomain is the same source"
        );
    }

    #[test]
    fn the_default_snapshot_base_is_the_archive_itself() {
        // The property that keeps an adapter from publishing an archive.org URL
        // as a work's canonical address is asserted against a live stub in
        // `an_archived_copy_is_read_and_marked_as_one`. This pins the default, so
        // that pointing the tier somewhere else stays a deliberate act.
        let client = ArchiveClient::new(vec!["fanfiction.net".into()]).expect("client");
        assert_eq!(client.snapshot_base, SNAPSHOT_BASE);
        assert_eq!(client.availability_endpoint, AVAILABILITY_ENDPOINT);
    }

    /// A stub archive that answers the lookup and serves a snapshot.
    ///
    /// A socket rather than a mock, so the tier is exercised through its real
    /// HTTP path — the query it sends, the JSON it has to parse, the `id_` URL it
    /// builds, and the provenance it attaches to what comes back. `{BASE}` in the
    /// availability body is replaced with the stub's own address, because the
    /// snapshot URL it names has to be on the host the client will read from.
    async fn stub_archive(availability: &str, snapshot: Option<&str>) -> String {
        stub_archive_with_redirect(availability, snapshot, None).await
    }

    /// The same stub, optionally answering the snapshot's entry path with a `302`
    /// to `redirect_to` — which is what the real archive does.
    async fn stub_archive_with_redirect(
        availability: &str,
        snapshot: Option<&str>,
        redirect_to: Option<&str>,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let base = format!("http://{addr}");
        let availability = availability.replace("{BASE}", &base);
        let snapshot = snapshot.map(str::to_owned);
        let redirect_to = redirect_to.map(|target| target.replace("{BASE}", &base));
        // One hop, then the page: the archive answers its entry URL with a single
        // `302`, and the entry URL already carries the `id_` modifier — verified
        // against the real archive, where the entry path redirects *and* contains
        // it, so the two addresses cannot be told apart by shape.
        let hopped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let mut buffer = vec![0_u8; 8192];
                let Ok(read) = socket.read(&mut buffer).await else {
                    continue;
                };
                let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let (status, body, location) = if path.starts_with("/web/") {
                    let already_hopped = hopped.swap(true, std::sync::atomic::Ordering::SeqCst);
                    match (&redirect_to, &snapshot) {
                        (Some(target), _) if !already_hopped => {
                            ("302 Found", String::new(), Some(target.clone()))
                        }
                        (_, Some(html)) => ("200 OK", html.clone(), None),
                        (_, None) => ("404 Not Found", "<html>no</html>".to_owned(), None),
                    }
                } else if path.starts_with("/broken") {
                    (
                        "500 Internal Server Error",
                        "<html>down</html>".to_owned(),
                        None,
                    )
                } else {
                    ("200 OK", availability.clone(), None)
                };
                let mut response = format!("HTTP/1.1 {status}\r\n");
                if let Some(location) = location {
                    response.push_str(&format!("location: {location}\r\n"));
                }
                response.push_str(&format!(
                    "content-type: text/html; charset=UTF-8\r\ncontent-length: {}\r\n\r\n{body}",
                    body.len()
                ));
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            }
        });
        base
    }

    /// A client for `fanfiction.net` pointed at a stub archive.
    async fn client_for_stub(availability: &str, snapshot: Option<&str>) -> ArchiveClient {
        let base = stub_archive(availability, snapshot).await;
        ArchiveClient::new(vec!["fanfiction.net".into()])
            .expect("client")
            .with_endpoints(format!("{base}/available"), base)
    }

    /// A lookup answer naming `{BASE}`, so the stub's own address is used.
    fn found_snapshot(timestamp: &str) -> String {
        serde_json::json!({
            "archived_snapshots": {
                "closest": {
                    "available": true,
                    "status": "200",
                    "timestamp": timestamp,
                    "url": format!(
                        "{{BASE}}/web/{timestamp}/https://www.fanfiction.net/s/12345678/1/"
                    ),
                }
            }
        })
        .to_string()
    }

    const WORK: &str = "https://www.fanfiction.net/s/12345678/1/";

    #[tokio::test]
    async fn an_archived_copy_is_read_and_marked_as_one() {
        let client = client_for_stub(
            &found_snapshot("20240101120000"),
            Some("<html><body><div id=\"storytext\">archived prose</div></body></html>"),
        )
        .await;
        let fetched = client.get(WORK).await.expect("an archived copy");
        assert!(fetched.body.contains("archived prose"));
        // The URL asked for, so an adapter storing a canonical URL stores the
        // work's address and not the archive's.
        assert_eq!(fetched.final_url, WORK);
        assert!(
            fetched.provenance.is_archived(),
            "an archived read must say so"
        );
        match fetched.provenance {
            crate::Provenance::Archive {
                snapshot_url,
                timestamp,
            } => {
                assert!(
                    snapshot_url.contains("id_"),
                    "the identity flag is what returns the real bytes: {snapshot_url}"
                );
                assert_eq!(timestamp.as_deref(), Some("20240101120000"));
            }
            crate::Provenance::Source => panic!("an archived read was recorded as a source read"),
        }
        // An archived page carries the archive's validators, not the source's.
        assert!(fetched.etag.is_none());
        assert!(fetched.last_modified.is_none());
    }

    #[tokio::test]
    async fn a_url_the_archive_never_saw_is_not_found() {
        let client = client_for_stub(r#"{"archived_snapshots":{}}"#, Some("<html>x</html>")).await;
        let error = client.get(WORK).await.expect_err("no snapshot");
        assert!(matches!(error, SourceError::NotFound), "got {error:?}");
    }

    #[tokio::test]
    async fn a_snapshot_pointing_off_the_archive_is_refused() {
        // The lookup's own answer is not trusted: a reply naming a URL anywhere
        // but the archive would otherwise make this tier an arbitrary-URL fetch,
        // which is the thing the whole crate exists to avoid.
        let client = client_for_stub(
            &serde_json::json!({
                "archived_snapshots": {
                    "closest": {
                        "available": true,
                        "url": "http://169.254.169.254/latest/meta-data/",
                        "timestamp": "20240101120000"
                    }
                }
            })
            .to_string(),
            Some("<html>x</html>"),
        )
        .await;
        let error = client.get(WORK).await.expect_err("must refuse");
        assert!(matches!(error, SourceError::Refused(_)), "got {error:?}");
        assert!(error.to_string().contains("is not"), "got {error}");
    }

    #[tokio::test]
    async fn an_archive_that_is_down_says_so_rather_than_parsing_html_as_json() {
        // The Internet Archive answers a 500 with an HTML page. Reporting that as
        // a deserialisation error would read like a bug in this file.
        let base = stub_archive("{}", Some("<html>x</html>")).await;
        let client = ArchiveClient::new(vec!["fanfiction.net".into()])
            .expect("client")
            .with_endpoints(format!("{base}/broken"), base);
        let error = client.get(WORK).await.expect_err("must fail");
        assert!(matches!(error, SourceError::Network(_)), "got {error:?}");
        assert!(error.to_string().contains("500"), "got {error}");
    }

    #[tokio::test]
    async fn a_snapshot_that_is_missing_at_the_archive_is_not_found() {
        let client = client_for_stub(&found_snapshot("20240101120000"), None).await;
        let error = client.get(WORK).await.expect_err("no snapshot");
        assert!(matches!(error, SourceError::NotFound), "got {error:?}");
    }

    #[tokio::test]
    async fn the_archives_own_redirect_is_followed_and_the_final_address_recorded() {
        // Verified against the real archive: its entry URL answers `302` to the
        // snapshot's own address, so the happy path is two requests. The redirect
        // must be followed — and the address actually read must be what provenance
        // records, rather than the entry point that merely pointed at it.
        const RESOLVED: &str =
            "{BASE}/web/20240101120000id_/https://www.fanfiction.net/s/12345678/1/";
        let base = stub_archive_with_redirect(
            &found_snapshot("20240101120000"),
            Some("<html><body><div id=\"storytext\">archived prose</div></body></html>"),
            Some(RESOLVED),
        )
        .await;
        let client = ArchiveClient::new(vec!["fanfiction.net".into()])
            .expect("client")
            .with_endpoints(format!("{base}/available"), base.clone());
        let fetched = client.get(WORK).await.expect("an archived copy");
        assert!(fetched.body.contains("archived prose"));
        match fetched.provenance {
            crate::Provenance::Archive { snapshot_url, .. } => {
                assert!(
                    snapshot_url.contains("20240101120000id_"),
                    "provenance should name the resolved snapshot, not the entry point: {snapshot_url}"
                );
            }
            crate::Provenance::Source => panic!("an archived read was recorded as a source read"),
        }
    }

    #[tokio::test]
    async fn a_redirect_off_the_archive_is_refused() {
        // The archive's answer is not authority to fetch wherever it points. Without
        // the per-hop check an off-host redirect would make this tier the
        // arbitrary-URL fetch the whole crate exists to avoid.
        let base = stub_archive_with_redirect(
            &found_snapshot("20240101120000"),
            Some("<html>elsewhere</html>"),
            Some("http://169.254.169.254/latest/meta-data/"),
        )
        .await;
        let client = ArchiveClient::new(vec!["fanfiction.net".into()])
            .expect("client")
            .with_endpoints(format!("{base}/available"), base);
        let error = client.get(WORK).await.expect_err("must refuse");
        assert!(matches!(error, SourceError::Refused(_)), "got {error:?}");
        assert!(
            error.to_string().contains("is not "),
            "the refusal should name where it was pointed: {error}"
        );
    }

    #[tokio::test]
    async fn a_missing_host_is_refused_before_the_network_is_touched() {
        let client = ArchiveClient::new(vec!["fanfiction.net".into()]).expect("client");
        let error = client
            .get("http://127.0.0.1:9/secret")
            .await
            .expect_err("must refuse");
        assert!(matches!(error, SourceError::Refused(_)), "got {error:?}");
    }

    #[tokio::test]
    async fn a_url_with_no_host_is_an_internal_error() {
        let client = ArchiveClient::new(vec!["fanfiction.net".into()]).expect("client");
        // `file:` parses but has no host; it must not reach the network either.
        let error = client
            .get("file:///etc/passwd")
            .await
            .expect_err("must refuse");
        assert!(
            matches!(error, SourceError::Refused(_) | SourceError::Internal(_)),
            "got {error:?}"
        );
    }
}
