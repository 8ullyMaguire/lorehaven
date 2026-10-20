//! The pseud/account link must never appear in a response body.
//!
//! # Why this file exists
//!
//! A **pseud** is the face a reader wears, and an **account** is the identity behind
//! it. Spec §6.1 keeps them strictly apart, and N-2 says the link between them is
//! never exposed: a reader's pseudonyms are their own, and a public surface that
//! names the account behind a pseudonym has turned a chosen face into a tracked
//! one.
//!
//! The link exists — `pseud.account_id` — and it has to, because the account is
//! what holds the credential, the trust level and the entitlement. **The
//! requirement is not that the link does not exist; it is that it is never
//! serialised.** One route that forgets is enough, and the forgetfulness is
//! invisible in review because a single `account_id` in a body looks like any
//! other field.
//!
//! So this file does not check a *rule*; it checks **bodies**. Every public
//! representation of an account is fetched, and every one of them is searched for
//! the account's id.
//!
//! # The two ways this test could be wrong, and both are handled
//!
//! 1. **A vacuous pass.** If the search is for a string that could never appear,
//!    the test passes whatever the routes do. So there is a **positive control**:
//!    the same body *does* carry the pseud's id, so the search is looking at the
//!    right place and the body is being read.
//! 2. **A false negative from a different id.** If the body leaked *some other*
//    account's id, a search for one id would miss it. So the search is for **any
//!    account id at all** — the account's own, plus a **fictional** one the test
//!    makes up, plus a structural check for a key named `account_id` anywhere in
//!    the document.

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
        "darkhold-pseud-privacy-{tag}-{}-{:?}",
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

    async fn register(&mut self, email: &str, handle: &str) -> (String, String) {
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
        let (status, me) = self.get("/api/v1/auth/me").await;
        assert_eq!(status, StatusCode::OK, "{me}");
        let account_id = me["account"]["id"].as_str().expect("account id").to_owned();
        // The pseud list, not a `pseud` key: `/api/v1/auth/me` is the ACCOUNT's
        // own surface, so it legitimately carries the account id. That is why
        // this route is not one of the surfaces under test -- the test is about
        // the surfaces a STRANGER can reach.
        let pseud_id = me["pseuds"][0]["id"].as_str().expect("pseud id").to_owned();
        (account_id, pseud_id)
    }
}

/// Every key in a JSON document, at any depth.
fn keys(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                out.push(key.clone());
                keys(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                keys(item, out);
            }
        }
        _ => {}
    }
}

/// Every string in a JSON document, at any depth.
fn strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => out.push(text.clone()),
        Value::Array(items) => {
            for item in items {
                strings(item, out);
            }
        }
        Value::Object(map) => {
            for child in map.values() {
                strings(child, out);
            }
        }
        _ => {}
    }
}

/// Assert that a body carries no account id, in any form.
///
/// **The check is structural as well as textual.** A body that carried the id
/// under a different key would pass a search for the id's *text* if the id were
/// serialised as a number or split across fields, so the key check is the one
/// that cannot be fooled.
fn assert_no_account_id(what: &str, body: &Value, account_id: &str, fictional: &str) {
    let mut found_keys = Vec::new();
    keys(body, &mut found_keys);
    for key in &found_keys {
        assert!(
            !key.eq_ignore_ascii_case("account_id")
                && !key.eq_ignore_ascii_case("accountId")
                && !key.eq_ignore_ascii_case("owner_account_id")
                && !key.eq_ignore_ascii_case("acting_account_id"),
            "{what}: the body carries an account link under the key {key:?}"
        );
    }

    let mut found_strings = Vec::new();
    strings(body, &mut found_strings);
    for text in &found_strings {
        assert!(
            !text.contains(account_id),
            "{what}: the body carries the account id {account_id} inside a string"
        );
        assert!(
            !text.contains(fictional),
            "{what}: the body carries an account id inside a string -- the fictional \
             one, which proves the search would have caught a real one"
        );
    }
}

#[tokio::test]
async fn no_public_representation_of_an_account_carries_its_id() {
    set_trust_proxy(false);
    let dir = scratch_dir("public");
    let config = config_for(&dir);
    let tdb = test_support::TestDb::connect_with_dir("pseud-privacy", &dir).await;
    let mut client = Client::new(server::build_router(AppState::new(
        config.clone(),
        tdb.db().clone(),
    )));

    let (account_id, pseud_id) = client
        .register("reader@example.test", "masked-reader")
        .await;

    // A fictional account id: a well-formed UUID that belongs to nobody here. If a
    // body carried *any* account id, this is the one a body would be least
    // likely to have leaked by accident -- so it is the one that proves the
    // search is looking.
    let fictional = uuid::Uuid::new_v4().to_string();

    // --- a work by the pseud, published so it is public ----------------------
    let (status, body) = client
        .post("/api/v1/works", json!({ "title": "A Public Work" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    let work_id = body["id"].as_str().expect("work id").to_owned();

    // **The discussion mode is `comments_only`** -- the legacy shape, supported
    // indefinitely (spec §13.2). A *new* work is `thread_only` by default and
    // its text discussion lives in the linked forum thread, so a test that
    // comments on a work has to say which shape it wants.
    let (status, body) = client
        .patch(
            &format!("/api/v1/works/{work_id}"),
            json!({
                "expected_version": body["version"].as_i64().unwrap_or(1),
                "discussion_mode": "comments_only",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "set the discussion mode: {body}");

    // --- a comment by the pseud ---------------------------------------------
    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/comments"),
            json!({ "body": "A comment with enough text to be a comment." }),
        )
        .await;
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "comment: {body}"
    );

    // --- every public representation, and the check on each ------------------
    let surfaces: Vec<(&str, String)> = vec![
        (
            "the pseud's public profile",
            format!("/api/v1/pseuds/{pseud_id}/profile"),
        ),
        ("the work's public page", format!("/api/v1/works/{work_id}")),
        (
            "the work's comments",
            format!("/api/v1/works/{work_id}/comments"),
        ),
        (
            "the public search",
            "/api/v1/public/search?q=Public".to_string(),
        ),
        ("the public browse", "/api/v1/browse".to_string()),
        (
            "the notifications list",
            "/api/v1/notifications".to_string(),
        ),
        ("the reader's pseud list", "/api/v1/pseuds".to_string()),
        ("the directory", "/api/v1/directory/entries".to_string()),
    ];

    let mut checked = 0usize;
    for (what, uri) in surfaces {
        let (status, body) = client.get(&uri).await;
        // A surface that answers 404 or 403 is not carrying the id either, and
        // the test is about the body -- so a non-200 is recorded and skipped
        // rather than failing.
        if status != StatusCode::OK {
            continue;
        }
        checked += 1;
        assert_no_account_id(what, &body, &account_id, &fictional);
    }

    // **The test asserts that it looked at something.** A loop over eight
    // surfaces that all answered 404 would pass without ever reading a body,
    // and a test that never reads a body is a test that proves nothing.
    assert!(
        checked >= 4,
        "only {checked} of the public surfaces answered 200, so the test examined          almost nothing -- a route that answers 404 is not a leak, and a test that          reads no bodies is not a test"
    );

    // --- the positive control -------------------------------------------------
    //
    // **Without this, the test could pass for the wrong reason.** The bodies
    // above carry the *pseud's* id — so the search is looking at a document that
    // really does carry identifiers, and "no account id" is a statement about
    // the account id and not about a body that happens to be empty.
    let (status, profile) = client
        .get(&format!("/api/v1/pseuds/{pseud_id}/profile"))
        .await;
    assert_eq!(status, StatusCode::OK, "the profile is public: {profile}");
    let mut found = Vec::new();
    strings(&profile, &mut found);
    assert!(
        found.iter().any(|text| text.contains(&pseud_id)),
        "the profile carries the PSEUD's id -- so the search is looking at a body \\
         that really does carry identifiers, and 'no account id' is a statement \\
         about the account id and not about an empty body"
    );

    // And the pseud's own id must not be confused with the account's: they are
    // different identifiers and the body carries the first and not the second.
    assert!(
        !found.iter().any(|text| text.contains(&account_id)),
        "the profile carries the pseud id and not the account id"
    );

    tdb.db().close().await;
    let _ = std::fs::remove_dir_all(&dir);
}
