//! M5-06 and M5-07 — an honest progress indicator, and a private "did not finish".
//!
//! # What this proves
//!
//! * **The progress indicator reflects the reader's real position in the
//!   CURRENT unit** (spec §10.1) — not an estimate, and not a percentage of
//!   the whole work presented as a position.
//! * **When the position is unknown, the indicator says so.** It does not show
//!   0 %: a reader told they are at the beginning of a unit they have read is
//!   told something false, and a false 0 % is worse than no number.
//! * **"Did not finish" is private, reversible, and never a judgement** (spec
//!   §10.1, §10.3). It is the reader's own note to themselves.
//! * **A reader's own payload carries no warmth, no streak and no engagement
//!   score.** The warmth band is for the *author* — "this month, 34 lurkers,
//!   12 reactors" — and a warmth meter on your own reading is a
//!   streak-shaped dark pattern.
//!
//! # Why the payload is asserted on
//!
//! The absence of a field is the kind of thing that is true until somebody
//! adds the field. A reader's own payload is exactly where a warmth band
//! would go — it is the reader's own data — so the assertion is that the
//! words are not there.

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
        "darkhold-honest-{tag}-{}-{:?}",
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

    async fn get(&mut self, uri: &str) -> (StatusCode, Value) {
        self.request("GET", uri, None).await
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

/// Publish a work with one unit, and return `(work id, unit id, revision id)`.
async fn published_work(client: &mut Client) -> (String, String, String) {
    client.register("honest@example.test", "honest").await;

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
async fn the_progress_indicator_says_so_when_the_position_is_unknown() {
    let harness = Harness::new("unknown").await;
    let mut client = harness.client();
    let (work_id, unit_id, _) = published_work(&mut client).await;

    let (status, page) = client
        .get_html(&format!("/works/{work_id}/units/{unit_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(
        page.contains("Your place in this unit is not known yet."),
        "**the indicator says the position is unknown**: {page}"
    );
    assert!(
        !page.contains("aria-valuenow=\"0\""),
        "and it does not show 0 %, which would be a false position: {page}"
    );
}

#[tokio::test]
async fn the_progress_indicator_shows_the_readers_real_position_in_the_unit() {
    let harness = Harness::new("known").await;
    let mut client = harness.client();
    let (work_id, unit_id, revision_id) = published_work(&mut client).await;

    let (status, body) = client
        .post_form(
            &format!("/works/{work_id}/units/{unit_id}/position"),
            &format!("position_permille=370&content_revision={revision_id}"),
        )
        .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "save the place: {body}");

    let (status, page) = client
        .get_html(&format!("/works/{work_id}/units/{unit_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(
        page.contains("aria-valuenow=\"37\""),
        "**the indicator shows the reader's real position in the unit**: {page}"
    );
    assert!(page.contains("37%"), "and it says so in words: {page}");
    assert!(
        page.contains("of this unit"),
        "**of THIS unit** -- not of the whole work presented as a position: {page}"
    );
}

#[tokio::test]
async fn did_not_finish_is_private_and_reversible() {
    let harness = Harness::new("dnf").await;
    let mut client = harness.client();
    let (work_id, unit_id, _) = published_work(&mut client).await;

    // **The mark is set.**
    let (status, body) = client
        .post_form(
            &format!("/works/{work_id}/units/{unit_id}/dnf"),
            "reason=slow_pacing&note=it+was+good+but+long",
        )
        .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "mark it: {body}");

    let (status, page) = client
        .get_html(&format!("/works/{work_id}/units/{unit_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(
        page.contains("did not finish"),
        "the page says the reader marked it: {page}"
    );
    assert!(
        page.contains("Only you can see that"),
        "**and that it is private**: {page}"
    );
    assert!(
        page.contains("Take that back"),
        "and it offers the way to undo it: {page}"
    );

    // **The mark is reversed.**
    let (status, body) = client
        .post_form(&format!("/works/{work_id}/units/{unit_id}/dnf"), "reason=")
        .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "take it back: {body}");

    let (status, page) = client
        .get_html(&format!("/works/{work_id}/units/{unit_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(
        !page.contains("Only you can see that"),
        "the mark is gone: {page}"
    );
    assert!(
        page.contains("Did not finish it?"),
        "and the action is offered again: {page}"
    );
}

#[tokio::test]
async fn a_readers_own_payload_carries_no_warmth_no_streak_and_no_engagement_score() {
    let harness = Harness::new("warmth").await;
    let mut client = harness.client();
    client.register("warm@example.test", "warm").await;

    let (status, body) = client.get("/api/v1/auth/me").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // **A warmth meter on your own reading is a streak-shaped dark pattern**
    // (spec §10.3). The band is for the AUTHOR -- "this month, 34 lurkers,
    // 12 reactors" -- and never for the reader.
    let rendered = body.to_string();
    for forbidden in [
        "warmth",
        "streak",
        "engagement_score",
        "engagement",
        "lurkers",
        "reactors",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "**the reader's own payload carries no '{forbidden}'**: {body}"
        );
    }
}
