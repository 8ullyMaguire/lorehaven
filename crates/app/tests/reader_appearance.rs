//! M5-02 — the reader's appearance is the reader's alone (spec §10.1, §21.1).
//!
//! # What this proves
//!
//! * **The reader's preference is per ACCOUNT**, so it follows the reader to
//!   another device. The test signs in a **second time** — a new session, the
//!   same account — and the reader's page carries the preference. A preference
//!   stored on the session would be gone, and a preference stored in the
//!   browser would be gone too, and both are the same failure wearing
//!   different clothes.
//! * **The page applies the reader's own appearance**: the reader's scale
//!   (`--reader-*`) is written inline on the document root, from the account's
//!   preference, and the **site** scale is not touched.
//! * **The reader's scale and the site's scale are separate variables**, so
//!   changing one cannot change the other. The frontend test proves the
//!   client-side half of that; this proves the server half.
//!
//! # Why the second session and not a second browser
//!
//! The claim is that the preference lives on the *account*. A second session
//! for the same account is the smallest thing that can prove it: the session
//! is new, the account is the same, and the preference is still there.

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
        "darkhold-appearance-{tag}-{}-{:?}",
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

    /// The `Cookie` header, the way a browser sends it: **one** header with
    /// every cookie in it.
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

    async fn post_form(&mut self, uri: &str, form: &str) -> (StatusCode, String) {
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
        self.capture(&response);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    /// A **new session for the same account**: sign in again.
    ///
    /// The session cookie is dropped first, so the sign-in issues a fresh one
    /// and the request that follows carries a session the reader has never
    /// used. The *account* is the same, and that is the point.
    async fn sign_in_again(&mut self, email: &str) {
        self.cookies.clear();
        let (status, body) = self
            .post(
                "/api/v1/auth/login",
                json!({ "email": email, "password": PASSWORD }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "sign in again: {body}");
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

/// Register, publish a work with one unit, and return `(work id, unit id)`.
async fn published_work(client: &mut Client) -> (String, String) {
    let (status, body) = client
        .post(
            "/api/v1/auth/register",
            json!({
                "email": "appearance@example.test",
                "password": PASSWORD,
                "handle": "appearance",
                "display_name": "Appearance",
                "age_band": "adult",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "register: {body}");

    let (status, body) = client
        .post("/api/v1/works", json!({ "title": "A Work To Read" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create the work: {body}");
    let work_id = body["id"].as_str().expect("the work's id").to_owned();

    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/chapters"),
            json!({ "title": "The Unit" }),
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
                        "content": [{ "type": "text", "text": "Some text." }],
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
async fn the_readers_appearance_follows_the_account_to_another_session() {
    let harness = Harness::new("account").await;
    let mut client = harness.client();
    let (work_id, unit_id) = published_work(&mut client).await;

    // **The reader chooses their appearance**, on the page.
    let (status, body) = client
        .post_form(
            &format!("/works/{work_id}/units/{unit_id}/typography"),
            "font_scale=1.35&line_height=1.9&measure=54&reader_theme=dark",
        )
        .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "save the appearance: {body}");

    // **A new session for the same account** -- another device.
    client.sign_in_again("appearance@example.test").await;

    let (status, page) = client
        .get_html(&format!("/works/{work_id}/units/{unit_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");

    // **The preference is there**, because it lives on the account.
    assert!(
        page.contains("--reader-font-scale:1.35"),
        "**the reader's appearance follows the account**: {page}"
    );
    assert!(
        page.contains("--reader-line-height:1.90"),
        "and so does the line spacing: {page}"
    );
    assert!(
        page.contains("--reader-measure:54ch"),
        "and the measure: {page}"
    );
    assert!(
        page.contains("data-reader=\"dark\""),
        "and the theme, on the attribute the stylesheet selects on: {page}"
    );
}

#[tokio::test]
async fn the_readers_scale_and_the_sites_scale_are_separate_variables() {
    let harness = Harness::new("scales").await;
    let mut client = harness.client();
    let (work_id, unit_id) = published_work(&mut client).await;

    let (status, body) = client
        .post_form(
            &format!("/works/{work_id}/units/{unit_id}/typography"),
            "font_scale=1.35&line_height=1.9&measure=54&reader_theme=dark",
        )
        .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "save the appearance: {body}");

    let (status, page) = client
        .get_html(&format!("/works/{work_id}/units/{unit_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");

    // **The reader's scale is `--reader-*`** and the site's is `--color-*`, and
    // the page writes only the reader's. The site's scale is not restated on
    // the page: restating it would be the reader's appearance deciding what the
    // site looks like.
    assert!(
        page.contains("--reader-font-scale"),
        "the reader's scale is written: {page}"
    );
    assert!(
        !page.contains("--color-bg"),
        "**and no site token is written**, because the two scales are \\
         independent: {page}"
    );
    assert!(!page.contains("--color-text"), "nor another: {page}");
}
