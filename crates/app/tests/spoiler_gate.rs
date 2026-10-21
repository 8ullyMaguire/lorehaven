//! M5-05 — the spoiler gate (spec §10.1, §10.2).
//!
//! # What this proves
//!
//! * **The gated text is NOT in the response body before the reader asks.**
//!   That is the whole of the card's fourth item, and it is the item that
//!   matters: *"a hidden element with the text in it is not a gate"*. A gate
//!   that leaves the text in the page and hides it with CSS is a gate a reader
//!   walks past with the source view, that a screen reader reads aloud, and
//!   that a search engine indexes.
//! * **The gate is a form post**, not a fetch — so it works without
//!   JavaScript, like everything else on the reading surface.
//! * **The reader configures it**: they can lift the gate for this section or
//!   for the whole work, and the choice is honoured.
//! * **The gate is the author's to set.** A reader cannot gate somebody
//!   else's text.
//!
//! # Why the response is the thing under test
//!
//! A gate is easy to fake. A `<div hidden>` with the text inside passes every
//! visual check and every screenshot, and the text is still in the response.
//! So the assertion is on the **body**, and the body is fetched with a plain
//! HTTP client rather than through a browser.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use darkhold_app::config::Config;
use darkhold_app::server::{self, set_trust_proxy};
use darkhold_app::state::AppState;
use darkhold_db::DatabaseConfig;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "darkhold-gate-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn config_for(dir: &Path) -> Config {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    config.database = DatabaseConfig::new(format!(
        "sqlite://{}/darkhold.sqlite?mode=rwc",
        dir.display()
    ));
    config
}

const PASSWORD: &str = "test-pass-1234";

/// The text the gate is hiding. It is distinctive on purpose: the assertion
/// is that this string is **absent** from the response.
const THE_SECRET: &str = "The-revealed-sentence-behind-the-gate";

struct Client {
    app: axum::Router,
    cookies: Vec<(String, String)>,
}

impl Client {
    fn new(app: axum::Router) -> Self {
        Self {
            app,
            cookies: Vec::new(),
        }
    }

    /// The `Cookie` header, the way a browser sends it: **one** header with
    /// every cookie in it.
    ///
    /// Sending one header per cookie is a mistake that hides itself: the
    /// server reads the *first* one, so the session cookie arrives and the
    /// rest do not. A browser joins them, and so does this.
    fn cookie_header(&self) -> Option<String> {
        if self.cookies.is_empty() {
            return None;
        }
        Some(
            self.cookies
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<String>>()
                .join("; "),
        )
    }

    fn capture(&mut self, response: &axum::response::Response) {
        for value in response.headers().get_all(header::SET_COOKIE) {
            let Ok(text) = value.to_str() else { continue };
            let Some((pair, _)) = text.split_once(';') else {
                continue;
            };
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            let name = name.trim().to_owned();
            let value = value.trim().to_owned();
            self.cookies.retain(|(k, _)| k != &name);
            if !value.is_empty() {
                self.cookies.push((name, value));
            }
        }
    }

    async fn request(
        &mut self,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(cookie) = self.cookie_header() {
            builder = builder.header(header::COOKIE, cookie);
        }
        if let Some(token) = self
            .cookies
            .iter()
            .find(|(k, _)| k == "darkhold_csrf")
            .map(|(_, v)| v.to_owned())
        {
            builder = builder.header("x-csrf-token", token);
        }
        let body = match body {
            Some(v) => Body::from(v.to_string()),
            None => Body::empty(),
        };
        let response = self
            .app
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        self.capture(&response);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into_owned()));
        (status, json)
    }

    async fn post(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("POST", uri, Some(body)).await
    }

    async fn patch(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("PATCH", uri, Some(body)).await
    }

    /// A GET that returns the **raw body**. The gate is about what the
    /// response says, so the response is what the test reads.
    async fn get_html(&mut self, uri: &str) -> (StatusCode, String) {
        let mut builder = Request::builder().method("GET").uri(uri);
        if let Some(cookie) = self.cookie_header() {
            builder = builder.header(header::COOKIE, cookie);
        }
        let response = self
            .app
            .clone()
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        self.capture(&response);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    /// A **form post**, the way a reader without JavaScript sends one.
    async fn post_form(
        &mut self,
        uri: &str,
        form: &str,
    ) -> (StatusCode, axum::http::HeaderMap, String) {
        let mut builder = Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        if let Some(cookie) = self.cookie_header() {
            builder = builder.header(header::COOKIE, cookie);
        }
        let response = self
            .app
            .clone()
            .oneshot(builder.body(Body::from(form.to_owned())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        self.capture(&response);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, headers, String::from_utf8_lossy(&body).into_owned())
    }

    async fn register(&mut self, email: &str, handle: &str) {
        let (status, body) = self
            .post(
                "/api/v1/auth/register",
                json!({
                    "email": email,
                    "password": PASSWORD,
                    "handle": handle,
                    "display_name": handle,
                    "age_band": "adult",
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "register {handle}: {body}");
    }
}

struct Harness {
    _dir: PathBuf,
    tdb: test_support::TestDb,
    config: Config,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        set_trust_proxy(false);
        let dir = scratch_dir(tag);
        let tdb = test_support::TestDb::connect_with_dir(tag, &dir).await;
        let config = config_for(&dir);
        Self {
            _dir: dir,
            tdb,
            config,
        }
    }

    fn client(&self) -> Client {
        Client::new(server::build_router(AppState::new(
            self.config.clone(),
            self.tdb.db().clone(),
        )))
    }
}

/// Publish a work with one unit whose body is [`THE_SECRET`], and return
/// `(work id, unit id)`.
async fn published_work_with_secret(client: &mut Client) -> (String, String) {
    client.register("gater@example.test", "gater").await;

    let (status, body) = client
        .post("/api/v1/works", json!({ "title": "A Gated Work" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create the work: {body}");
    let work_id = body["id"].as_str().expect("the work's id").to_owned();

    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/chapters"),
            json!({ "title": "The Gated Unit" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create a unit: {body}");
    let unit_id = body["id"].as_str().expect("the unit's id").to_owned();

    let (status, body) = client
        .patch(
            &format!("/api/v1/chapters/{unit_id}"),
            json!({
                "expected_version": 1,
                "document": {
                    "type": "doc",
                    "content": [{
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": THE_SECRET }],
                    }],
                },
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "write the unit: {body}");

    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/publish"),
            json!({ "expected_version": 1 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "publish the work: {body}");

    (work_id, unit_id)
}

#[tokio::test]
async fn the_gated_text_is_not_in_the_response_before_the_reader_asks() {
    let harness = Harness::new("gated").await;
    let mut client = harness.client();
    let (work_id, unit_id) = published_work_with_secret(&mut client).await;

    // **The author gates the section.**
    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/units/{unit_id}/gate"),
            json!({ "gated": true }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "gate the section: {body}");

    // **The text is NOT in the response.** Not hidden by CSS, not in a data
    // attribute, not in a script: **absent**.
    let (status, page) = client
        .get_html(&format!("/works/{work_id}/units/{unit_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(
        !page.contains(THE_SECRET),
        "**the gated text is not in the response body**: {page}"
    );
    assert!(
        page.contains("reader-surface__gate"),
        "and the page says there is a gate: {page}"
    );
    assert!(
        page.contains("Show me this part"),
        "with the way to ask for it: {page}"
    );
}

#[tokio::test]
async fn the_gate_is_a_form_post_and_the_revealed_text_arrives() {
    let harness = Harness::new("reveal").await;
    let mut client = harness.client();
    let (work_id, unit_id) = published_work_with_secret(&mut client).await;

    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/units/{unit_id}/gate"),
            json!({ "gated": true }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "gate the section: {body}");

    // **The reveal is a form post** -- the no-JavaScript path.
    let (status, headers, _) = client
        .post_form(
            &format!("/works/{work_id}/units/{unit_id}/reveal"),
            "scope=unit",
        )
        .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "the reveal is answered");
    assert_eq!(
        headers.get(header::LOCATION).and_then(|v| v.to_str().ok()),
        Some(format!("/works/{work_id}/units/{unit_id}").as_str()),
        "and the reader is sent back to the page"
    );

    // **And now the text is there.**
    let (status, page) = client
        .get_html(&format!("/works/{work_id}/units/{unit_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(
        page.contains(THE_SECRET),
        "the reader asked, and the text is there: {page}"
    );
}

#[tokio::test]
async fn the_reader_can_lift_the_gate_for_the_whole_work() {
    let harness = Harness::new("whole").await;
    let mut client = harness.client();
    let (work_id, unit_id) = published_work_with_secret(&mut client).await;

    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/units/{unit_id}/gate"),
            json!({ "gated": true }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "gate the section: {body}");

    let (status, _headers, _) = client
        .post_form(
            &format!("/works/{work_id}/units/{unit_id}/reveal"),
            "scope=work",
        )
        .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "the reveal is answered");

    let (status, page) = client
        .get_html(&format!("/works/{work_id}/units/{unit_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(
        page.contains(THE_SECRET),
        "the reader asked for the whole work, and the text is there: {page}"
    );
}

#[tokio::test]
async fn a_reader_cannot_gate_somebody_elses_text() {
    let harness = Harness::new("author").await;
    let mut client = harness.client();
    let (work_id, unit_id) = published_work_with_secret(&mut client).await;

    // **Another reader** tries to gate it. The author's door.
    client.register("stranger@example.test", "stranger").await;
    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/units/{unit_id}/gate"),
            json!({ "gated": true }),
        )
        .await;
    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
        "a reader cannot gate somebody else's text: {status} {body}"
    );

    // And the work reads exactly as it did.
    let (status, page) = client
        .get_html(&format!("/works/{work_id}/units/{unit_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(
        page.contains(THE_SECRET),
        "the gate was refused, so the text is still readable: {page}"
    );
}
