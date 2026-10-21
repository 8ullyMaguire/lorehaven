//! M5-03 — the position save survives everything (spec §10.1, §10.3).
//!
//! # What this proves
//!
//! * **The position lives on the account, not in the browser** (spec §10.3).
//!   A position set on one device is on the reader's *other* device's page,
//!   because it was saved against the account and the page reads the account.
//! * **A position set against revision N is not applied to revision N+1
//!   silently.** The unit has been revised since the reader's place was
//!   recorded, so the page says the place is stale and does not apply it: the
//!   text in between may have been rewritten, and putting a reader at a
//!   fraction of a text they have not read is worse than starting them at the
//!   top with an honest note.
//!
//! # Why the *page* is the thing under test
//!
//! The store's `progress_for` already reads every device's position for the
//! account — that is what makes §9.3's "two devices disagree, so the reader
//! is asked" work. What was missing was the **reading surface**: the
//! server-rendered page did not read the position at all, so a reader who had
//! saved their place arrived at the top of the unit every time.
//!
//! So the test reads the page, twice: once with the position saved, and once
//! after the unit has been revised underneath it.

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
        "darkhold-position-{tag}-{}-{:?}",
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
    /// every cookie in it. One header per cookie means the server reads only
    /// the first, and the rest silently never arrive.
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

    /// A GET that returns JSON, for the API's own answers.
    async fn get(&mut self, uri: &str) -> (StatusCode, Value) {
        self.request("GET", uri, None).await
    }

    async fn patch(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("PATCH", uri, Some(body)).await
    }

    /// A GET that returns the **raw body**: the page is a document, and the
    /// document is what the test reads.
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

/// Publish a work with one unit, and return `(work id, unit id, revision id)`.
async fn published_work(client: &mut Client) -> (String, String, String) {
    let (status, body) = client
        .post(
            "/api/v1/auth/register",
            json!({
                "email": "positioner@example.test",
                "password": PASSWORD,
                "handle": "positioner",
                "display_name": "Positioner",
                "age_band": "adult",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "register: {body}");

    let (status, body) = client
        .post("/api/v1/works", json!({ "title": "A Work With A Place" }))
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
                        "content": [{ "type": "text", "text": "The first text." }],
                    }],
                },
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "write the unit: {body}");
    let revision_id = body["current_revision_id"]
        .as_str()
        .expect("the revision's id")
        .to_owned();

    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/publish"),
            json!({ "expected_version": 1 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "publish the work: {body}");

    (work_id, unit_id, revision_id)
}

#[tokio::test]
async fn the_position_follows_the_reader_to_another_device() {
    let harness = Harness::new("device").await;
    let mut client = harness.client();
    let (work_id, unit_id, revision_id) = published_work(&mut client).await;

    // **Device A** saves a place. The device id is what makes it *a
    // device's* position rather than the account's only one.
    let (status, body) = client
        .post_form(
            &format!("/works/{work_id}/units/{unit_id}/position"),
            &format!("position_permille=420&content_revision={revision_id}&device_id=device-a"),
        )
        .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "save the place: {body}");

    // **Device B** reads the same work. It has no position of its own -- and
    // the position is on the *account*, so the page still shows where the
    // reader was.
    let (status, page) = client
        .get_html(&format!("/works/{work_id}/units/{unit_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(
        page.contains("value=\"420\""),
        "**the position is on the account, so it is on the other device too**: \
         {page}"
    );
    assert!(
        page.contains("42%"),
        "and the page says so in words: {page}"
    );
}

#[tokio::test]
async fn arriving_in_a_unit_puts_the_work_in_the_readers_history() {
    let harness = Harness::new("arrival").await;
    let mut client = harness.client();
    let (work_id, unit_id, _) = published_work(&mut client).await;

    // **Arriving is a reading.** The reader's page records the arrival, and it
    // is what puts an opened work in the reader's history.
    let (status, page) = client
        .get_html(&format!("/works/{work_id}/units/{unit_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");

    let (status, body) = client.get("/api/v1/library/history").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert!(
        items.iter().any(|item| {
            item["subject_id"].as_str() == Some(work_id.as_str())
                || item["work_id"].as_str() == Some(work_id.as_str())
        }),
        "the work the reader opened is in their history: {body}"
    );
}

#[tokio::test]
async fn a_position_against_an_older_revision_is_marked_stale_and_not_applied() {
    let harness = Harness::new("stale").await;
    let mut client = harness.client();
    let (work_id, unit_id, revision_id) = published_work(&mut client).await;

    let (status, body) = client
        .post_form(
            &format!("/works/{work_id}/units/{unit_id}/position"),
            &format!("position_permille=420&content_revision={revision_id}&device_id=device-a"),
        )
        .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "save the place: {body}");

    // **The unit is revised.** The revisions are append-only (ADR 0002), so
    // the old revision still exists -- and the reader's place points at text
    // that may have been rewritten.
    let (status, body) = client
        .patch(
            &format!("/api/v1/chapters/{unit_id}"),
            json!({
                "expected_version": 2,
                "document": {
                    "type": "doc",
                    "content": [{
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "The rewritten text." }],
                    }],
                },
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "revise the unit: {body}");

    // **The page says the place is stale**, and does not apply it.
    let (status, page) = client
        .get_html(&format!("/works/{work_id}/units/{unit_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(
        page.contains("reader-surface__position-stale"),
        "**the page says the saved place is stale**: {page}"
    );
    assert!(
        !page.contains("value=\"420\""),
        "and it does NOT apply the old position, because the text in between \
         may have been rewritten: {page}"
    );
}
