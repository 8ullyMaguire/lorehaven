//! M2-03: the 404-not-403 doctrine, proved on a private work.
//!
//! # The rule
//!
//! **"403 answers 'yes, and you may not'."** So a resource the caller may not
//! know about is **404** — and so is a malformed id, and so is an operator
//! surface, because all three must be indistinguishable from the outside (spec
//! §20.1, N-13).
//!
//! A **capability** check is not one of them: "you may not moderate" is a 403,
//! because the door is public knowledge and the permission is not. The two are
//! different questions — *may I do this?* and *may I know this exists?* — and
//! only the second one is answered with 404.
//!
//! # Why the body matters as much as the status
//!
//! A 404 whose body carries the work's title, its id or its author is a 404
//! with a footnote, and the footnote is the leak. **So the test asserts the body
//! says nothing**, and the positive control asserts the body *would* have said
//! something — otherwise "the body is empty" is a statement about a body that
//! was empty anyway.

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
        "darkhold-not-found-{tag}-{}-{:?}",
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

    async fn create_work(&mut self, title: &str) -> String {
        let (status, body) = self.post("/api/v1/works", json!({ "title": title })).await;
        assert_eq!(status, StatusCode::CREATED, "create work {title}: {body}");
        body["id"].as_str().expect("work id").to_owned()
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

/// A title nobody would guess, so a leak of it is unmistakable.
const SECRET_TITLE: &str = "A Work Nobody May Know About";

#[tokio::test]
async fn a_private_work_is_404_to_a_stranger_and_says_nothing_about_itself() {
    let harness = Harness::new("stranger").await;

    // The owner, and the private work.
    let mut owner = harness.client();
    owner.register("owner@example.test", "Owner").await;
    let work_id = owner.create_work(SECRET_TITLE).await;

    // A stranger, with a session of their own -- so the refusal cannot be
    // "you are not signed in". **The stranger is a real reader**, which is what
    // makes the 404 a disclosure decision rather than an authentication one.
    let mut stranger = harness.client();
    stranger.register("stranger@example.test", "Stranger").await;

    let (status, body) = stranger.get(&format!("/api/v1/works/{work_id}")).await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a private work is 404 to a stranger, not 403: {body}"
    );

    // **The body says nothing.** Not the title, not the id, not the author.
    let serialised = serde_json::to_string(&body).expect("serialise");
    assert!(
        !serialised.contains(SECRET_TITLE),
        "the 404 body carries the work's title: {serialised}"
    );
    assert!(
        !serialised.contains(&work_id),
        "the 404 body carries the work's id: {serialised}"
    );
    assert!(
        !serialised.contains("Owner"),
        "the 404 body carries the work's author: {serialised}"
    );
    assert_eq!(
        body["error"]["code"], "NOT_FOUND",
        "the machine code is the one the specification names: {body}"
    );

    harness.tdb.db().close().await;
}

#[tokio::test]
async fn the_owner_receives_their_own_private_work() {
    // **The positive control.** Without it, "the body is empty" would be a
    // statement about a body that was empty anyway, and the test above would
    // pass without ever proving that a 200 carries the work.
    let harness = Harness::new("owner").await;

    let mut owner = harness.client();
    owner.register("owner2@example.test", "Owner").await;
    let work_id = owner.create_work(SECRET_TITLE).await;

    let (status, body) = owner.get(&format!("/api/v1/works/{work_id}")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the owner sees their own private work: {body}"
    );
    let serialised = serde_json::to_string(&body).expect("serialise");
    assert!(
        serialised.contains(SECRET_TITLE),
        "the owner's own body DOES carry the title -- so the stranger's body not \
         carrying it is a decision and not an accident: {serialised}"
    );

    harness.tdb.db().close().await;
}

#[tokio::test]
async fn a_malformed_id_is_404_and_says_no_more_than_a_private_one_does() {
    // **A malformed id and a private id must be indistinguishable.** A caller
    // that can tell them apart is an oracle for what exists, and the oracle is
    // worth more than the 404.
    let harness = Harness::new("malformed").await;

    let mut owner = harness.client();
    owner.register("owner3@example.test", "Owner").await;
    let work_id = owner.create_work(SECRET_TITLE).await;

    let mut stranger = harness.client();
    stranger
        .register("stranger3@example.test", "Stranger")
        .await;

    let (malformed_status, malformed_body) = stranger.get("/api/v1/works/not-a-uuid-at-all").await;
    let (private_status, private_body) = stranger.get(&format!("/api/v1/works/{work_id}")).await;

    assert_eq!(malformed_status, StatusCode::NOT_FOUND, "{malformed_body}");
    assert_eq!(private_status, StatusCode::NOT_FOUND, "{private_body}");
    assert_eq!(
        malformed_body["error"]["code"], private_body["error"]["code"],
        "a malformed id and a private id answer with the same code"
    );
    assert_eq!(
        malformed_body["error"]["message"], private_body["error"]["message"],
        "a malformed id and a private id answer with the same message"
    );

    harness.tdb.db().close().await;
}

#[tokio::test]
async fn the_doctrine_has_one_helper_and_the_domain_owns_the_wording() {
    // **The doctrine is named in one place**, and the wording is the domain's.
    // A second place would be a second wording, and a second wording is how a
    // caller learns which of two answers to expect.
    assert!(
        darkhold_domain::error::not_found_if_inaccessible("work", true).is_ok(),
        "an accessible resource is not an error"
    );
    let error = darkhold_domain::error::not_found_if_inaccessible("work", false)
        .expect_err("an inaccessible resource is an error");
    match error {
        darkhold_domain::AppError::NotFound { resource } => {
            assert_eq!(resource, "work", "the resource noun is the coarse one")
        }
        other => panic!("the helper must answer with NotFound, got {other:?}"),
    }
}
