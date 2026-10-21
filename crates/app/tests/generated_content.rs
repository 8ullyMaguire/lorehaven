//! M4-07 — the author's AI declaration.
//!
//! # What this proves
//!
//! * **A work with `ai_training = deny` and a declared-AI attribute is still
//!   indexed and recommended.** Spec §15.4 and N-4: the archive indexes and
//!   recommends AI-generated works, labelled as a declared attribute. The
//!   author's statement about *training* says nothing about whether their work
//!   is in the archive — a work is not hidden because its author said no to
//!   training.
//! * **The statement is shown as a statement** — *"this author's stated
//!   preference, recorded"* — and **never as "protected"**. A badge that
//!   promises something the archive does not do is a lie with a logo.
//! * **The label is present on the work page and in the list**, and it says the
//!   author declared it — the archive does not detect AI-generated text and
//!   does not claim to.
//! * **The instance's posture decides, and its refusal names the policy**
//!   (spec §6.9): under `forbid` a declared work is refused, and the refusal
//!   says *which* rule refused it.

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
        "darkhold-ai-{tag}-{}-{:?}",
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

/// The clock, in one place, so this file has a single dependency on "now".
fn crate_now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

/// A published work, with a unit of text in it.
async fn published_work(client: &mut Client, title: &str) -> String {
    let (status, body) = client
        .post("/api/v1/works", json!({ "title": title }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    let id = body["id"].as_str().expect("work id").to_owned();

    let (status, body) = client
        .post(
            &format!("/api/v1/works/{id}/chapters"),
            json!({ "title": "One" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "add a unit: {body}");
    let unit = body["id"].as_str().expect("unit id").to_owned();
    let unit_version = body["version"].as_i64().expect("unit version");

    let doc = json!({ "type": "doc", "content": [
        { "type": "paragraph", "content": [
            { "type": "text", "text": "A unit with enough words to have a middle." }] }] });
    let (status, body) = client
        .patch(
            &format!("/api/v1/chapters/{unit}"),
            json!({ "expected_version": unit_version, "document": doc }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "write the unit: {body}");

    let (status, body) = client.get(&format!("/api/v1/works/{id}")).await;
    assert_eq!(status, StatusCode::OK, "reload the work: {body}");
    let current = body["version"].as_i64().expect("current work version");

    let (status, body) = client
        .post(
            &format!("/api/v1/works/{id}/publish"),
            json!({ "expected_version": current }),
        )
        .await;
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "publish the work: {status} {body}"
    );
    id
}

#[tokio::test]
async fn a_declared_ai_work_is_still_indexed_and_recommended_and_labelled() {
    let harness = Harness::new("declared").await;
    let mut client = harness.client();
    client.register("author@example.test", "Author").await;

    // A published work, with the author declining AI training.
    let work_id = published_work(&mut client, "A declared-AI work").await;
    let (status, body) = client.get(&format!("/api/v1/works/{work_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let version = body["version"].as_i64().unwrap_or(1);

    let (status, body) = client
        .patch(
            &format!("/api/v1/works/{work_id}"),
            json!({ "expected_version": version, "ai_training": "deny" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "set the statement: {body}");
    assert_eq!(body["ai_training"], "deny", "{body}");

    // And the author declares that the work contains generated content.
    let (status, body) = client
        .patch(
            &format!("/api/v1/works/{work_id}/generated-content"),
            json!({ "declaration": "declared" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "declare the work: {status} {body}");
    assert_eq!(body["generated_content"]["declared"], true, "{body}");

    // **The statement is a statement, and never a protection.**
    let statement = body["ai_training_statement"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase();
    assert!(
        statement.contains("stated preference"),
        "the wording is the specification's: {statement}"
    );
    assert!(
        !statement.contains("protect"),
        "and it is never 'protected': a badge that promises something the archive \\
         does not do is a lie with a logo: {statement}"
    );

    // **The work's page carries the label.**
    let (status, page) = client.get(&format!("/api/v1/works/{work_id}")).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(
        page["generated_content"]["declared"], true,
        "the label is on the work's page: {page}"
    );
    assert_eq!(page["ai_training"], "deny", "{page}");

    // **And the list carries it too.** The work's page is where a reader meets
    // the work; the list is where a reader meets it *before* opening it, and a
    // label that only appears on the page is a label most readers never see.
    let (status, list) = client.get("/api/v1/works").await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let items = list.as_array().expect("the list is an array");
    let mine = items
        .iter()
        .find(|item| item["id"] == work_id.as_str())
        .unwrap_or_else(|| panic!("the work is in the author's list: {list}"));
    assert_eq!(mine["ai_training"], "deny", "{mine}");
    assert_eq!(
        mine["generated_declared"], true,
        "and the list says the work is a declared-AI work: {mine}"
    );

    // **And the work is still indexed and recommended.** N-4: the archive
    // indexes and recommends AI-generated works. A statement about *training*
    // says nothing about whether the work is in the archive, and a work that
    // vanished because its author said no to training would be the archive
    // refusing its author.
    let recommended = darkhold_db::discovery::public_recommendations(harness.tdb.db(), None, 50)
        .await
        .expect("the public recommendations");
    assert!(
        recommended.iter().any(|id| id.to_string() == work_id),
        "the work is still recommended: {recommended:?}"
    );
}

#[tokio::test]
async fn the_postures_refusal_names_the_policy() {
    // **A refusal that names the policy** (spec §6.9): "this instance forbids
    // generated content" — the rule, not a bare error.
    let harness = Harness::new("forbid").await;
    let mut client = harness.client();
    client.register("author2@example.test", "Author2").await;

    // The instance forbids generated content.
    darkhold_db::generated_content::set_generated_content_policy(
        harness.tdb.db(),
        darkhold_domain::generated_content::GeneratedContentPosture::Forbid,
        None,
        &crate_now(),
    )
    .await
    .expect("set the posture");

    let work_id = published_work(&mut client, "A work").await;
    let (status, body) = client
        .patch(
            &format!("/api/v1/works/{work_id}/generated-content"),
            json!({ "declaration": "declared" }),
        )
        .await;
    assert!(
        status == StatusCode::UNPROCESSABLE_ENTITY || status == StatusCode::FORBIDDEN,
        "the declaration is refused: {status} {body}"
    );
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("forbids generated content"),
        "and the refusal names the policy: {message}"
    );
}

#[tokio::test]
async fn the_default_posture_accepts_a_declaration() {
    // **The default is `allow`** (spec §6.9, D-4/N-4). The owner's corpus has
    // 8 of 54 works carrying `ai-generated text` and he writes them: an archive
    // whose default refuses the work its own author writes is an archive that
    // refuses its author.
    let harness = Harness::new("default").await;
    let mut client = harness.client();
    client.register("author3@example.test", "Author3").await;

    let posture = darkhold_db::generated_content::generated_content_policy(harness.tdb.db())
        .await
        .expect("read the posture");
    assert_eq!(
        posture,
        darkhold_domain::generated_content::GeneratedContentPosture::Allow,
        "an instance that has not decided accepts, and labels"
    );
}
