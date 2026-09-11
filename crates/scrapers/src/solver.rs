//! The second escalation tier: a solver service, when a fingerprint is not enough.
//!
//! # What this talks to
//!
//! Not a specific program. Popular tools in this space — FlareSolverr, its
//! maintained fork **Byparr**, and **obscura-solverr** (a Rust headless browser
//! behind the same API) — all expose the same HTTP contract on port 8191:
//! a `POST` of `{"cmd": "request.get", "url": …}` answered with
//! `{"status": "ok", "solution": {"response": "<html>", …}}`. Implementing the
//! *protocol* rather than any one tool is the only version of this that does not
//! need rewriting when a tool is archived, which is what happened to FlareSolverr
//! itself: it is unmaintained now and fails on current managed challenges, while
//! the API it defined is what everything else still speaks.
//!
//! # Why a service rather than a browser subprocess
//!
//! `chromium --dump-dom` is how `ficnexus` solves these, and it works. But a
//! browser subprocess does its own DNS resolution, follows its own redirects, and
//! loads subresources — images, stylesheets, scripts — so `SafeFetcher` cannot pin
//! it, and every one of those fetches is a request the guard never approved. A
//! service is a *process boundary*: the importer sends one HTTP request to an
//! operator-configured address, and what the service then chooses to fetch is the
//! service's business, on the service's own network position. The importer's guard
//! stays intact and the solver's behaviour is the operator's to configure.
//!
//! That boundary is what makes this tier acceptable at all, and it comes with a
//! condition the caller must keep: **a solver request is a request the guard did
//! not make**. So the solver is only ever handed a URL whose host the source
//! itself declared — never a URL that arrived from a user, a redirect, or a page.
//! [`SolverClient::get`] re-checks this rather than trusting the caller to have
//! done it, because this is the one place in the crate where the check is the
//! entire mitigation.
//!
//! # Sessions
//!
//! A chapter import is many requests to one host, and a managed challenge solved
//! per request would hammer both the source and the solver. The service supports
//! sessions — a cookie jar held across requests — so the challenge is solved once
//! and the rest of the import rides the cookies the solve earned. The session is
//! created lazily, reused, and recreated if the service says it has expired.

use crate::{Fetched, SourceError, SourceResult};
use serde::Deserialize;
use std::time::Duration;
use tokio::sync::Mutex;

/// How long a solver is given to work on a challenge.
///
/// A managed challenge that needs a JavaScript round trip takes a few seconds; one
/// that needs a second, interactive step never completes and holding a request
/// open for it only makes the import look hung. This is the ceiling on the whole
/// exchange, and the service applies its own internally too.
const DEFAULT_MAX_TIMEOUT_MS: u64 = 60_000;

/// The service's own ceiling, `maxTimeout` capped at two minutes.
const SERVICE_MAX_TIMEOUT_MS: u64 = 120_000;

/// Where the solver service is and how long it may take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolverConfig {
    /// The service's base URL, e.g. `http://127.0.0.1:8191`.
    ///
    /// Loopback and private-network addresses are expected and permitted: this is
    /// normally a container the operator runs beside the instance, and requiring
    /// it to be public would be requiring a worse deployment.
    pub endpoint: String,
    /// How long to let it work on a challenge.
    pub max_timeout: Duration,
}

impl SolverConfig {
    /// A configuration for a service on `endpoint`.
    #[must_use]
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into().trim_end_matches('/').to_owned(),
            max_timeout: Duration::from_millis(DEFAULT_MAX_TIMEOUT_MS),
        }
    }

    /// The `maxTimeout` to send, in milliseconds, within the service's own limit.
    #[must_use]
    fn max_timeout_ms(&self) -> u64 {
        u64::try_from(self.max_timeout.as_millis())
            .unwrap_or(SERVICE_MAX_TIMEOUT_MS)
            .clamp(1_000, SERVICE_MAX_TIMEOUT_MS)
    }
}

/// A client for a FlareSolverr-compatible service.
pub struct SolverClient {
    config: SolverConfig,
    http: reqwest::Client,
    /// The session id, once one has been created. `None` until the first request,
    /// because creating a session for an import that never needs one would leave
    /// a browser context open on the service for nothing.
    session: Mutex<Option<String>>,
    /// Hosts this solver may be aimed at.
    allowed_hosts: Vec<String>,
}

impl SolverClient {
    /// Build a client for `config`, permitted to reach `allowed_hosts`.
    ///
    /// # Errors
    /// [`SourceError::Unsupported`] if the endpoint is not an absolute http(s)
    /// URL — a misconfiguration the operator should see at startup, not at the
    /// first challenged chapter.
    pub fn new(config: SolverConfig, allowed_hosts: Vec<String>) -> SourceResult<Self> {
        let parsed = url::Url::parse(&config.endpoint)
            .map_err(|e| SourceError::Unsupported(format!("solver endpoint is not a URL: {e}")))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(SourceError::Unsupported(format!(
                "solver endpoint scheme {} is not http or https",
                parsed.scheme()
            )));
        }
        // The solver is given a generous connect timeout but no overall client
        // timeout: the service does its own waiting, and the request to it must
        // outlast `maxTimeout` or it would time out while the solve is succeeding.
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(config.max_timeout + Duration::from_secs(30))
            .build()
            .map_err(|e| SourceError::Internal(format!("building solver client: {e}")))?;
        Ok(Self {
            config,
            http,
            session: Mutex::new(None),
            allowed_hosts: allowed_hosts
                .into_iter()
                .map(|host| host.trim_start_matches("www.").to_ascii_lowercase())
                .collect(),
        })
    }

    /// Where the service is, for a log line or a health check.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.config.endpoint
    }

    /// Ask the service for `url`.
    ///
    /// # Errors
    /// [`SourceError::Refused`] if `url` is not on a host this solver was built
    /// for — the check that is the whole point of the process boundary.
    /// [`SourceError::Blocked`] if the service could not get past the wall either,
    /// and [`SourceError::Network`] if the service could not be reached at all.
    pub async fn get(&self, url: &str) -> SourceResult<Fetched> {
        let parsed = url::Url::parse(url)
            .map_err(|e| SourceError::Internal(format!("solver given a non-URL: {e}")))?;
        self.check_host(&parsed)?;

        // A session is opened before the first request rather than after a
        // failure, because the whole reason to use a session is to pay for the
        // challenge once. `sessions.create` is the only command that can precede
        // `request.get`, and a service that will not open one is still usable
        // statelessly — so a failure here is logged and stepped over rather than
        // failing an import that could otherwise have read the page.
        let session = match self.session_id().await {
            Some(existing) => Some(existing),
            None => match self.create_session().await {
                Ok(created) => Some(created),
                Err(error) => {
                    tracing::debug!(
                        solver = %self.config.endpoint,
                        %error,
                        "the solver would not open a session; reading statelessly"
                    );
                    None
                }
            },
        };

        let outcome = match self.request(url, session.as_deref()).await {
            Attempted::Ok(reply) => reply,
            Attempted::Failed(error) => return Err(error),
            // A session can expire between requests, or the service can be
            // restarted mid-import. Both are fixed by opening a new one rather
            // than by failing the chapter — and this is decided from the
            // service's own sentence, not from a message we composed, because a
            // `SourceError` no longer carries that sentence.
            Attempted::SessionGone => {
                tracing::debug!(
                    solver = %self.config.endpoint,
                    "the solver forgot our session; opening a new one"
                );
                *self.session.lock().await = None;
                let fresh = self.create_session().await?;
                match self.request(url, Some(&fresh)).await {
                    Attempted::Ok(reply) => reply,
                    Attempted::Failed(error) => return Err(error),
                    Attempted::SessionGone => {
                        return Err(SourceError::Blocked);
                    }
                }
            }
        };

        let solution = outcome.solution.unwrap_or_default();
        let body = solution.response.unwrap_or_default();
        if body.is_empty() {
            return Err(SourceError::Parse("the solver returned no page".to_owned()));
        }
        let status = solution.status.unwrap_or(200);
        if !(200..400).contains(&status) {
            // The service got through the wall and the source said no. That is
            // the source's answer, not a solver failure, so it is reported as the
            // source's.
            return Err(match status {
                404 => SourceError::NotFound,
                403 | 401 => SourceError::Withheld(format!(
                    "the solver reached the page and the source answered {status}"
                )),
                _ => SourceError::Blocked,
            });
        }
        Ok(Fetched {
            final_url: solution.url.unwrap_or_else(|| url.to_owned()),
            body,
            content_type: "text/html; charset=UTF-8".to_owned().into(),
            // A solved page is a fresh read of the source through a browser the
            // service owns; it carries no validator we could honestly reuse, and
            // pretending otherwise would make the revision cache claim a
            // conditional request it never made.
            etag: None,
            last_modified: None,
            // A solved page is the source's own page, read through a browser the
            // service owns. It is not an archived copy, and saying it was would
            // tell a reader their chapter came from a snapshot.
            provenance: crate::Provenance::Source,
        })
    }

    /// Reject a URL that is not for a host this solver was built for.
    fn check_host(&self, url: &url::Url) -> SourceResult<()> {
        let host = url
            .host_str()
            .ok_or_else(|| SourceError::Internal("solver given a URL with no host".to_owned()))?;
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
                 certainly not one to hand to the solver"
            )))
        }
    }

    async fn session_id(&self) -> Option<String> {
        self.session.lock().await.clone()
    }

    /// Create a session, storing it for reuse.
    async fn create_session(&self) -> SourceResult<String> {
        let reply: SolverReply = self
            .post(&serde_json::json!({
                "cmd": "sessions.create",
            }))
            .await?;
        if reply.status != "ok" {
            tracing::debug!(
                reason = reply.message.as_deref().unwrap_or("no reason given"),
                "the solver could not open a session"
            );
            return Err(SourceError::Blocked);
        }
        let session = reply.session.ok_or_else(|| {
            SourceError::Parse("the solver opened a session without naming it".to_owned())
        })?;
        *self.session.lock().await = Some(session.clone());
        Ok(session)
    }

    /// One `request.get`, naming a session when there is one.
    ///
    /// A session is used when one exists because it is what makes a multi-chapter
    /// import one solve instead of many. The first call is session-less only if
    /// creating one failed, which is not fatal: a stateless solve still reads the
    /// page.
    async fn request(&self, url: &str, session: Option<&str>) -> Attempted {
        let mut payload = serde_json::json!({
            "cmd": "request.get",
            "url": url,
            "maxTimeout": self.config.max_timeout_ms(),
        });
        if let Some(session) = session {
            payload["session"] = serde_json::Value::String(session.to_owned());
        }
        let reply = match self.post(&payload).await {
            Ok(reply) => reply,
            Err(error) => return Attempted::Failed(error),
        };
        if reply.status != "ok" {
            let message = reply.message.unwrap_or_default();
            if session.is_some() && names_a_missing_session(&message) {
                return Attempted::SessionGone;
            }
            // A wall the solver could not pass is not a bug in this code, and
            // must not read like one: it is the source declining to serve a
            // browser too, so the answer is `Blocked` — the category that already
            // means "a challenge wall, a ban, an IP block". The service's own
            // sentence is logged rather than discarded, because it is the only
            // thing that distinguishes a wall from a misconfigured service.
            tracing::warn!(solver = %self.config.endpoint, %message, "the solver did not pass the wall");
            return Attempted::Failed(SourceError::Blocked);
        }
        Attempted::Ok(reply)
    }

    /// POST one command to the service.
    async fn post(&self, payload: &serde_json::Value) -> SourceResult<SolverReply> {
        // Serialised and sent by hand rather than with `RequestBuilder::json`,
        // which needs a `reqwest` feature this crate does not otherwise want: the
        // only JSON it ever sends is these three commands.
        let body = payload.to_string();
        let response = self
            .http
            .post(&self.config.endpoint)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| {
                SourceError::Network(format!(
                    "the solver at {} could not be reached: {e}",
                    self.config.endpoint
                ))
            })?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| SourceError::Network(format!("reading the solver's reply: {e}")))?;
        if !status.is_success() {
            return Err(SourceError::Network(format!(
                "the solver answered {status}: {}",
                text.chars().take(200).collect::<String>()
            )));
        }
        serde_json::from_str(&text).map_err(|e| {
            SourceError::Parse(format!(
                "the solver's reply was not JSON ({e}): {}",
                text.chars().take(200).collect::<String>()
            ))
        })
    }
}

/// Whether the service is saying the session we named is gone.
///
/// Matched on its prose because that is all it offers: the protocol has no code
/// for an expired session, and the two phrasings below are FlareSolverr's
/// ("This session does not exist") and Byparr's. Read off the *reply* rather
/// than off an error we composed, because the sentence is the only place the
/// distinction survives.
fn names_a_missing_session(message: &str) -> bool {
    let text = message.to_ascii_lowercase();
    text.contains("session") && (text.contains("not exist") || text.contains("expired"))
}

/// What one `request.get` produced.
///
/// Three outcomes rather than a `Result`, because "the service does not know that
/// session" is neither a reply nor a failure: it is a retryable condition the
/// caller has to be able to see.
enum Attempted {
    /// The page, or the source's own refusal of it.
    Ok(SolverReply),
    /// The session we named is gone. Open another and ask again.
    SessionGone,
    /// The service could not be used at all.
    Failed(SourceError),
}

/// The service's reply, which is the same shape for every command.
#[derive(Debug, Deserialize)]
struct SolverReply {
    /// `"ok"` or `"error"`.
    status: String,
    /// Why, when it is not `"ok"`.
    #[serde(default)]
    message: Option<String>,
    /// The session id, from `sessions.create`.
    #[serde(default)]
    session: Option<String>,
    /// The page, from `request.get`.
    #[serde(default)]
    solution: Option<Solution>,
}

/// The page the service read.
#[derive(Debug, Default, Deserialize)]
struct Solution {
    /// The URL the browser ended on, after any client-side navigation.
    #[serde(default)]
    url: Option<String>,
    /// The status the source answered with.
    #[serde(default)]
    status: Option<u16>,
    /// The page's HTML.
    #[serde(default)]
    response: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// A stub standing in for the service, so the protocol is verified against
    /// real bytes on a real socket rather than against a mock of our own client.
    async fn stub(replies: Vec<String>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let handle = tokio::spawn(async move {
            let mut seen = Vec::new();
            for reply in replies {
                let (mut socket, _) = listener.accept().await.expect("accept");
                let mut buffer = vec![0_u8; 8192];
                let read = socket.read(&mut buffer).await.expect("read");
                seen.push(String::from_utf8_lossy(&buffer[..read]).into_owned());
                let body = reply;
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.expect("write");
                socket.flush().await.expect("flush");
            }
            seen
        });
        (format!("http://{addr}"), handle)
    }

    fn ok_solution(html: &str) -> String {
        serde_json::json!({
            "status": "ok",
            "message": "Challenge solved!",
            "solution": { "url": "https://www.fimfiction.net/story/373233/", "status": 200, "response": html }
        })
        .to_string()
    }

    #[tokio::test]
    async fn a_solved_page_comes_back_with_its_url() {
        let (endpoint, _seen) = stub(vec![
            serde_json::json!({"status": "ok", "session": "abc123"}).to_string(),
            ok_solution("<html><body><div class=\"story\">prose</div></body></html>"),
        ])
        .await;
        let client = SolverClient::new(SolverConfig::new(&endpoint), vec!["fimfiction.net".into()])
            .expect("client");
        let fetched = client
            .get("https://www.fimfiction.net/story/373233/")
            .await
            .expect("a page");
        assert!(fetched.body.contains("prose"));
        assert_eq!(
            fetched.final_url,
            "https://www.fimfiction.net/story/373233/"
        );
        assert!(fetched.etag.is_none(), "a solved page carries no validator");
    }

    #[tokio::test]
    async fn the_second_request_reuses_the_session() {
        let (endpoint, seen) = stub(vec![
            serde_json::json!({"status": "ok", "session": "abc123"}).to_string(),
            ok_solution("<p>one</p>"),
            ok_solution("<p>two</p>"),
        ])
        .await;
        let client = SolverClient::new(SolverConfig::new(&endpoint), vec!["fimfiction.net".into()])
            .expect("client");
        client
            .get("https://www.fimfiction.net/story/1/")
            .await
            .expect("first");
        client
            .get("https://www.fimfiction.net/story/2/")
            .await
            .expect("second");
        let seen = seen.await.expect("stub finished");
        assert_eq!(seen.len(), 3, "one create, two requests");
        assert!(seen[0].contains("sessions.create"));
        // Without this the whole point of sessions — one solve for a whole work —
        // is lost, and every chapter pays for a browser again.
        assert!(
            seen[1].contains("\"session\":\"abc123\""),
            "first request: {}",
            seen[1]
        );
        assert!(
            seen[2].contains("\"session\":\"abc123\""),
            "second request: {}",
            seen[2]
        );
    }

    #[tokio::test]
    async fn an_expired_session_is_replaced_rather_than_failing_the_chapter() {
        let (endpoint, seen) = stub(vec![
            serde_json::json!({"status": "ok", "session": "old"}).to_string(),
            serde_json::json!({"status": "error", "message": "This session does not exist."})
                .to_string(),
            serde_json::json!({"status": "ok", "session": "new"}).to_string(),
            ok_solution("<p>recovered</p>"),
        ])
        .await;
        let client = SolverClient::new(SolverConfig::new(&endpoint), vec!["fimfiction.net".into()])
            .expect("client");
        let fetched = client
            .get("https://www.fimfiction.net/story/1/")
            .await
            .expect("a page");
        assert!(fetched.body.contains("recovered"));
        let seen = seen.await.expect("stub finished");
        assert_eq!(seen.len(), 4, "create, request, re-create, request");
    }

    #[tokio::test]
    async fn a_url_outside_the_sources_hosts_is_refused_before_any_request() {
        // The mitigation this tier rests on. No stub is started: reaching one
        // would itself be the failure.
        let client = SolverClient::new(
            SolverConfig::new("http://127.0.0.1:1"),
            vec!["fimfiction.net".into()],
        )
        .expect("client");
        let error = client
            .get("http://169.254.169.254/latest/meta-data/")
            .await
            .expect_err("must refuse");
        assert!(matches!(error, SourceError::Refused(_)), "got {error:?}");
        assert!(error
            .to_string()
            .contains("not a host this source may fetch from"));
    }

    #[tokio::test]
    async fn a_url_the_solver_was_not_built_for_is_refused_even_with_a_prefix() {
        let client = SolverClient::new(
            SolverConfig::new("http://127.0.0.1:1"),
            vec!["fimfiction.net".into()],
        )
        .expect("client");
        // `notfimfiction.net` must not pass a suffix comparison, and neither may
        // an unrelated host that merely ends in the allowed one as a substring.
        for hostile in [
            "https://notfimfiction.net/x",
            "https://fimfiction.net.evil.test/x",
        ] {
            let error = client.get(hostile).await.expect_err("must refuse");
            assert!(
                matches!(error, SourceError::Refused(_)),
                "{hostile} gave {error:?}"
            );
        }
        // A real subdomain is the same source.
        assert!(client
            .check_host(&url::Url::parse("https://www.fimfiction.net/s/1/").unwrap())
            .is_ok());
    }

    #[tokio::test]
    async fn a_solver_that_is_not_running_says_so() {
        let client = SolverClient::new(
            SolverConfig::new("http://127.0.0.1:1"),
            vec!["fimfiction.net".into()],
        )
        .expect("client");
        let error = client
            .get("https://www.fimfiction.net/story/1/")
            .await
            .expect_err("must fail");
        assert!(matches!(error, SourceError::Network(_)), "got {error:?}");
        assert!(
            error.to_string().contains("could not be reached"),
            "got {error}"
        );
    }

    #[tokio::test]
    async fn a_source_that_refuses_the_solver_too_is_reported_as_withheld() {
        let (endpoint, _seen) = stub(vec![
            serde_json::json!({"status": "ok", "session": "s"}).to_string(),
            serde_json::json!({
                "status": "ok",
                "solution": { "url": "https://www.fimfiction.net/x/", "status": 403, "response": "<html>no</html>" }
            })
            .to_string(),
        ])
        .await;
        let client = SolverClient::new(SolverConfig::new(&endpoint), vec!["fimfiction.net".into()])
            .expect("client");
        let error = client
            .get("https://www.fimfiction.net/story/1/")
            .await
            .expect_err("must fail");
        assert!(matches!(error, SourceError::Withheld(_)), "got {error:?}");
    }

    #[tokio::test]
    async fn a_garbled_reply_is_a_parse_error_not_a_panic() {
        let (endpoint, _seen) = stub(vec![
            serde_json::json!({"status": "ok", "session": "s"}).to_string(),
            "not json at all".to_owned(),
        ])
        .await;
        let client = SolverClient::new(SolverConfig::new(&endpoint), vec!["fimfiction.net".into()])
            .expect("client");
        let error = client
            .get("https://www.fimfiction.net/story/1/")
            .await
            .expect_err("must fail");
        assert!(matches!(error, SourceError::Parse(_)), "got {error:?}");
    }

    #[test]
    fn a_non_http_endpoint_is_rejected_at_construction() {
        // A misconfiguration the operator should see at startup rather than at
        // the first challenged chapter.
        for broken in ["ftp://solver", "not a url", "file:///etc/passwd", ""] {
            let error = SolverClient::new(SolverConfig::new(broken), vec![])
                .err()
                .unwrap_or_else(|| panic!("{broken:?} should not have been accepted"));
            assert!(
                matches!(error, SourceError::Unsupported(_)),
                "{broken:?} gave {error:?}"
            );
        }
    }

    #[test]
    fn a_session_the_service_forgot_is_recognised_in_its_own_words() {
        for message in [
            "This session does not exist.",
            "The session has expired",
            "session abc123 does not exist",
        ] {
            assert!(
                names_a_missing_session(message),
                "{message:?} not recognised"
            );
        }
        // A wall is not a forgotten session, and must not be retried as one.
        for message in [
            "Challenge not solved",
            "Error: timeout",
            "the source returned 503",
        ] {
            assert!(!names_a_missing_session(message), "{message:?} misread");
        }
    }

    #[test]
    fn max_timeout_is_clamped_to_the_services_own_ceiling() {
        let mut config = SolverConfig::new("http://127.0.0.1:8191");
        config.max_timeout = Duration::from_secs(600);
        assert_eq!(config.max_timeout_ms(), SERVICE_MAX_TIMEOUT_MS);
        config.max_timeout = Duration::from_millis(1);
        assert_eq!(config.max_timeout_ms(), 1_000);
    }
}
