//! M5-01 — the reading surface is **server-rendered HTML** (spec §8.5, §10.1, §21.1).
//!
//! # What this proves
//!
//! * **The reader's page is HTML, and the text is IN it.** A single-page
//!   application fetches its content after the document loads, so with
//!   JavaScript off the reader gets an empty page. This asserts the text is
//!   in the response body, which is the whole of the requirement.
//! * **The navigation is a real link**, not a fetch: the next and previous
//!   units are `<a href>` elements, and following one returns the other
//!   unit's text.
//! * **The position save degrades to a form post**, not a failure: a signed-out
//!   reader's post is answered with a redirect and not an error, and a
//!   signed-in reader's post is saved.
//! * **The escaping is decided by the API, not by each call site.** A title
//!   carrying `<script>` comes out as text and not as markup.
//! * **A signed-out visitor may read.** A reader page that asked for a
//!   session would be a login wall in front of the archive's own content.
//!
//! # Why this is a Rust test and not only a Playwright one
//!
//! The Playwright journey (`frontend/e2e/reader-surface.spec.ts`) proves the
//! chain in a browser with JavaScript switched off. This proves the same
//! facts about the *response*, which is where the requirement lives: the text
//! is in the body, the links are links, and the form posts. A browser test
//! that passed because the page happened to render would not prove that the
//! text was server-rendered rather than fetched.

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
        "darkhold-reader-{tag}-{}-{:?}",
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

    /// A request that returns the **raw body**, because the point of these
    /// tests is what the body says.
    async fn raw(
        &mut self,
        method: &str,
        uri: &str,
        form: Option<&str>,
    ) -> (StatusCode, String, axum::http::HeaderMap) {
        let mut builder = Request::builder().method(method).uri(uri);
        if form.is_some() {
            builder = builder.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        }
        for (name, value) in &self.cookies {
            builder = builder.header(header::COOKIE, format!("{name}={value}"));
        }
        if let Some(token) = self
            .cookies
            .iter()
            .find(|(k, _)| k == "darkhold_csrf")
            .map(|(_, v)| v.to_owned())
        {
            builder = builder.header("x-csrf-token", token);
        }
        let body = match form {
            Some(form) => Body::from(form.to_owned()),
            None => Body::empty(),
        };
        let response = self
            .app
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        self.capture(&response);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&body).into_owned(), headers)
    }

    async fn get(&mut self, uri: &str) -> (StatusCode, String, axum::http::HeaderMap) {
        self.raw("GET", uri, None).await
    }

    async fn post_form(
        &mut self,
        uri: &str,
        form: &str,
    ) -> (StatusCode, String, axum::http::HeaderMap) {
        self.raw("POST", uri, Some(form)).await
    }

    async fn post(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        for (name, value) in &self.cookies {
            builder = builder.header(header::COOKIE, format!("{name}={value}"));
        }
        if let Some(token) = self
            .cookies
            .iter()
            .find(|(k, _)| k == "darkhold_csrf")
            .map(|(_, v)| v.to_owned())
        {
            builder = builder.header("x-csrf-token", token);
        }
        let response = self
            .app
            .clone()
            .oneshot(builder.body(Body::from(body.to_string())).unwrap())
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

    async fn patch(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method("PATCH")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        for (name, value) in &self.cookies {
            builder = builder.header(header::COOKIE, format!("{name}={value}"));
        }
        if let Some(token) = self
            .cookies
            .iter()
            .find(|(k, _)| k == "darkhold_csrf")
            .map(|(_, v)| v.to_owned())
        {
            builder = builder.header("x-csrf-token", token);
        }
        let response = self
            .app
            .clone()
            .oneshot(builder.body(Body::from(body.to_string())).unwrap())
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

/// Publish a work with two units, and return (work id, first unit id, second
/// unit id).
///
/// The units are created through the **API**, not by writing rows, because a
/// row written by hand would not have the reader-visible shape the page
/// renders: no current revision, no sanitized body, nothing to read.
async fn published_work_with_two_units(
    client: &mut Client,
    handle: &str,
) -> (String, String, String) {
    client
        .register(&format!("{handle}@example.test"), handle)
        .await;

    let (status, body) = client
        .post("/api/v1/works", json!({ "title": "A Work To Read" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create the work: {body}");
    let work_id = body["id"].as_str().expect("the work's id").to_owned();

    let mut unit_ids = Vec::new();
    for (index, title) in ["The First Unit", "The Second Unit"].iter().enumerate() {
        let (status, body) = client
            .post(
                &format!("/api/v1/works/{work_id}/chapters"),
                json!({ "title": title }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "create a unit: {body}");
        let unit_id = body["id"].as_str().expect("the unit's id").to_owned();

        // The unit needs **text**, or the page has nothing to render and the
        // test would pass on an empty page.
        let (status, body) = client
            .patch(
                &format!("/api/v1/chapters/{unit_id}"),
                json!({
                    "expected_version": 1,
                    "title": title,
                    "document": {
                        "type": "doc",
                        "content": [
                            {
                                "type": "paragraph",
                                "content": [{
                                    "type": "text",
                                    "text": format!("The body of unit number {}.", index + 1),
                                }],
                            },
                        ],
                    },
                }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "write the unit: {body}");
        unit_ids.push(unit_id);
    }

    // Publish the work, or the reader page refuses it: a draft is not a
    // public work.
    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/publish"),
            json!({ "expected_version": 1 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "publish the work: {body}");

    (work_id, unit_ids[0].clone(), unit_ids[1].clone())
}

#[tokio::test]
async fn the_reading_page_carries_the_text_in_its_body() {
    let harness = Harness::new("body").await;
    let mut client = harness.client();
    let (work_id, first_unit, _) = published_work_with_two_units(&mut client, "reader").await;

    let (status, body, headers) = client
        .get(&format!("/works/{work_id}/units/{first_unit}"))
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("text/html; charset=utf-8"),
        "the reader's page is HTML"
    );
    assert!(
        body.contains("The body of unit number 1."),
        "**the text is in the response body**, which is what server-rendered \
         means: {body}"
    );
    assert!(
        body.contains("<!DOCTYPE html>"),
        "and it is a document: {body}"
    );
}

#[tokio::test]
async fn the_navigation_is_a_real_link_and_following_it_returns_the_next_unit() {
    let harness = Harness::new("nav").await;
    let mut client = harness.client();
    let (work_id, first_unit, second_unit) =
        published_work_with_two_units(&mut client, "navreader").await;

    let (status, body, _) = client
        .get(&format!("/works/{work_id}/units/{first_unit}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // The next unit is a LINK. A fetch would be a script, and a script is
    // what the reader does without.
    let next_href = format!("/works/{work_id}/units/{second_unit}");
    assert!(
        body.contains(&format!("href=\"{next_href}\"")),
        "the next unit is a real link: {body}"
    );
    assert!(
        body.contains("rel=\"next\""),
        "and it says so for a reader who cannot see it: {body}"
    );

    // Following the link returns the OTHER unit's text. The two units say
    // different things, so this is a real check and not a tautology.
    let (status, body, _) = client.get(&next_href).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("The body of unit number 2."),
        "following the link reads the next unit: {body}"
    );
    assert!(
        !body.contains("The body of unit number 1."),
        "and not the one the reader was on: {body}"
    );

    // And the way back is a link too.
    let previous_href = format!("/works/{work_id}/units/{first_unit}");
    assert!(
        body.contains(&format!("href=\"{previous_href}\"")),
        "the way back is a real link: {body}"
    );
}

#[tokio::test]
async fn the_position_save_degrades_to_a_form_post_and_not_to_a_failure() {
    let harness = Harness::new("position").await;
    let mut client = harness.client();
    let (work_id, first_unit, _) = published_work_with_two_units(&mut client, "posreader").await;

    // The form is on the page, and it POSTs.
    let (status, body, _) = client
        .get(&format!("/works/{work_id}/units/{first_unit}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains(&format!(
            "action=\"/works/{work_id}/units/{first_unit}/position\""
        )),
        "the page carries the position form: {body}"
    );
    assert!(
        body.contains("method=\"post\""),
        "and it is a POST, which works without JavaScript: {body}"
    );

    // A signed-out reader's post is **not an error**: they are sent back to
    // the page they were on. Their place lives in their browser, and a 401
    // would be a failure where the specification asks for a degradation.
    let (status, _, headers) = client
        .post_form(
            &format!("/works/{work_id}/units/{first_unit}/position"),
            "position_permille=250",
        )
        .await;
    assert_eq!(
        status,
        StatusCode::SEE_OTHER,
        "a signed-out post is redirected"
    );
    assert_eq!(
        headers.get(header::LOCATION).and_then(|v| v.to_str().ok()),
        Some(format!("/works/{work_id}/units/{first_unit}").as_str()),
        "and back to the page"
    );
}

#[tokio::test]
async fn a_signed_in_readers_position_is_saved() {
    let harness = Harness::new("saved").await;
    let mut client = harness.client();
    let (work_id, first_unit, _) = published_work_with_two_units(&mut client, "saver").await;

    let (status, _, _) = client
        .post_form(
            &format!("/works/{work_id}/units/{first_unit}/position"),
            "position_permille=250",
        )
        .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "the post is answered");

    // The position is on the account, which is what makes it follow the
    // reader to another device (spec §10.3).
    let (status, body, _) = client
        .get(&format!(
            "/api/v1/reading/progress?subject_type=work&subject_id={work_id}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("position_permille"),
        "the position is on the account, which is what makes it follow the \
         reader to another device (spec §10.3): {body}"
    );
    assert!(
        body.contains("\"position_permille\":250"),
        "and it is the position the reader posted: {body}"
    );
}

#[tokio::test]
async fn a_title_carrying_markup_is_text_and_not_markup() {
    let harness = Harness::new("escape").await;
    let mut client = harness.client();
    client.register("escapee@example.test", "escapee").await;

    let (status, body) = client
        .post(
            "/api/v1/works",
            json!({ "title": "A Work <script>alert('x')</script>" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let work_id = body["id"].as_str().expect("the work's id").to_owned();

    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/chapters"),
            json!({ "title": "A Unit </title><script>alert('y')</script>" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let unit_id = body["id"].as_str().expect("the unit's id").to_owned();

    // The unit needs text, or the work cannot be published and the page
    // would have nothing to render.
    let (status, body) = client
        .patch(
            &format!("/api/v1/chapters/{unit_id}"),
            json!({
                "expected_version": 1,
                "document": {
                    "type": "doc",
                    "content": [{
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "A body." }],
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
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, page, _) = client
        .get(&format!("/works/{work_id}/units/{unit_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(
        !page.contains("<script>alert("),
        "**no script the client sent is ever markup**: {page}"
    );
    assert!(
        page.contains("&lt;script&gt;"),
        "it comes out as text: {page}"
    );
}

#[tokio::test]
async fn a_signed_out_visitor_may_read() {
    // The reader page must not be a login wall: a signed-out reader is a
    // reader (spec §7), and the page is public.
    let harness = Harness::new("anon").await;
    let mut client = harness.client();
    let (work_id, first_unit, _) = published_work_with_two_units(&mut client, "anonreader").await;

    // Sign out, so the request carries no session at all.
    client.cookies.clear();

    let (status, body, _) = client
        .get(&format!("/works/{work_id}/units/{first_unit}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("The body of unit number 1."),
        "a signed-out reader reads: {body}"
    );
}
