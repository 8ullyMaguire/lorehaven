//! Milestone 3 acceptance tests (spec §8).
//!
//! Spec §8 acceptance:
//!
//! * concurrent edits return conflicts;
//! * revision restoration creates a new revision;
//! * public readers never receive unpublished revisions;
//! * repeated publication with one idempotency key does not duplicate
//!   notifications;
//! * pseud switching does not change ownership;
//! * invitations identify the exposed pseud;
//! * withdrawn or newly restricted content disappears from public indexes and
//!   caches.
//!
//! Plus the properties this project adds on top of the spec's list, because
//! they are the ones a reader's trust depends on: a draft is a `404` to
//! everyone but its contributors, an anonymous reader never receives the editor
//! document, and a document outside the editor schema is refused rather than
//! stored.
//!
//! Everything runs against the real router, a real SQLite file and a cookie jar
//! that behaves like a browser's.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::{outbox, DatabaseConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m3-{tag}-{}-{:?}",
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
        "sqlite://{}/lorehaven.sqlite?mode=rwc",
        dir.display()
    ));
    // Raise rate limits for parallel test execution.
    config.rate_limits.auth = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.write = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.search = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.default = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config
}

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

    fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn capture_cookies(&mut self, response: &axum::response::Response) {
        for value in response.headers().get_all(header::SET_COOKIE) {
            let Ok(text) = value.to_str() else { continue };
            let Some((pair, _attributes)) = text.split_once(';') else {
                continue;
            };
            if let Some((name, value)) = pair.split_once('=') {
                let name = name.trim().to_owned();
                let value = value.trim().to_owned();
                self.cookies.retain(|(key, _)| key != &name);
                if !value.is_empty() {
                    self.cookies.push((name, value));
                }
            }
        }
    }

    async fn request(
        &mut self,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if !self.cookies.is_empty() {
            let header = self
                .cookies
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("; ");
            builder = builder.header(header::COOKIE, header);
        }
        if !matches!(method, "GET" | "HEAD" | "OPTIONS") {
            if let Some(token) = self.cookie("lorehaven_csrf") {
                builder = builder.header("x-csrf-token", token.to_owned());
            }
        }

        let request = match body {
            Some(value) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&value).expect("serialise")))
                .expect("request"),
            None => builder.body(Body::empty()).expect("request"),
        };

        let response = self.app.clone().oneshot(request).await.expect("response");
        let status = response.status();
        self.capture_cookies(&response);

        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .expect("body");
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        };
        (status, value)
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
}

struct Harness {
    dir: PathBuf,
    tdb: test_support::TestDb,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        set_trust_proxy(false);
        let _ = lorehaven_app::logging::init(&lorehaven_app::config::LoggingConfig {
            filter: "error".to_owned(),
            format: lorehaven_app::config::LogFormat::Pretty,
        });

        let dir = scratch_dir(tag);
        let tdb = test_support::TestDb::connect_with_dir(tag, &dir).await;
        let report: Vec<String> = tdb.applied_migrations().to_vec();
        assert!(
            report.contains(&"0003_works".to_owned()),
            "the works migration must apply: {report:?}"
        );

        Self { dir, tdb }
    }

    fn client(&self) -> Client {
        Client::new(server::build_router(AppState::new(
            config_for(&self.dir),
            self.tdb.db().clone(),
        )))
    }

    /// A second browser: same database, separate cookie jar.
    fn anonymous(&self) -> Client {
        self.client()
    }

    async fn cleanup(self) {
        self.tdb.cleanup().await;
        let _ = std::fs::remove_dir_all(self.dir);
    }
}

const PASSWORD: &str = "a-long-enough-passphrase";

async fn register(client: &mut Client, email: &str, handle: &str) -> String {
    let (status, body) = client
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
    body["account"]["id"]
        .as_str()
        .expect("account id")
        .to_owned()
}

async fn active_pseud(client: &mut Client) -> String {
    let (status, body) = client.get("/api/v1/auth/me").await;
    assert_eq!(status, StatusCode::OK, "me: {body}");
    body["active_pseud_id"]
        .as_str()
        .expect("active pseud id")
        .to_owned()
}

/// Create a work and return its identifier.
async fn create_work(client: &mut Client, title: &str) -> Value {
    let (status, body) = client
        .post("/api/v1/works", json!({ "title": title }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    body
}

/// Add a chapter and return its identifier and version.
async fn add_chapter(client: &mut Client, work: &str, title: &str) -> (String, i64) {
    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work}/chapters"),
            json!({ "title": title }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "add chapter: {body}");
    (
        body["id"].as_str().expect("chapter id").to_owned(),
        body["version"].as_i64().expect("version"),
    )
}

/// A one-paragraph document the editor schema accepts.
fn document(text: &str) -> Value {
    json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": text }],
        }],
    })
}

async fn save_chapter(
    client: &mut Client,
    chapter: &str,
    expected_version: i64,
    text: &str,
) -> (StatusCode, Value) {
    client
        .patch(
            &format!("/api/v1/chapters/{chapter}"),
            json!({ "expected_version": expected_version, "document": document(text) }),
        )
        .await
}

async fn publish(
    client: &mut Client,
    work: &str,
    version: i64,
    key: Option<&str>,
) -> (StatusCode, Value) {
    let mut body = json!({ "expected_version": version });
    if let Some(key) = key {
        body["idempotency_key"] = json!(key);
    }
    client
        .post(&format!("/api/v1/works/{work}/publish"), body)
        .await
}

// ---------------------------------------------------------------------------
// The vertical slice: draft → chapter → save → publish → read
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_draft_is_reachable_to_its_author_and_is_a_404_to_everyone_else() {
    let harness = Harness::new("draft").await;
    let mut author = harness.client();
    register(&mut author, "author@example.com", "Quill").await;

    let work = create_work(&mut author, "The Long Road").await;
    let work_id = work["id"].as_str().expect("id").to_owned();
    assert_eq!(work["lifecycle"], "draft");
    assert_eq!(work["role"], "owner");

    let (chapter, version) = add_chapter(&mut author, &work_id, "One").await;
    let (status, body) = save_chapter(&mut author, &chapter, version, "It began.").await;
    assert_eq!(status, StatusCode::OK, "save: {body}");
    assert_eq!(body["word_count"], 2);
    assert_eq!(body["has_content"], true);

    // The author sees it, with its chapter.
    let (status, body) = author.get(&format!("/api/v1/works/{work_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["title"], "The Long Road");
    assert_eq!(body["chapters"].as_array().expect("chapters").len(), 1);
    assert!(body["publication_blockers"]
        .as_array()
        .expect("blockers")
        .is_empty());

    // A stranger sees a 404, not a 403: the draft's existence is not disclosed.
    let mut stranger = harness.anonymous();
    let (status, _) = stranger.get(&format!("/api/v1/works/{work_id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = stranger
        .get(&format!("/api/v1/works/{work_id}/chapters/{chapter}"))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    harness.cleanup().await;
}

#[tokio::test]
async fn publishing_needs_a_title_a_chapter_and_content_and_then_reads_publicly() {
    let harness = Harness::new("publish").await;
    let mut author = harness.client();
    register(&mut author, "author@example.com", "Quill").await;

    // An empty work cannot be published, and the error names the field.
    let work = create_work(&mut author, "Untitled").await;
    let work_id = work["id"].as_str().expect("id").to_owned();
    let (status, body) = publish(&mut author, &work_id, 1, None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "VALIDATION_FAILED");
    assert!(
        body["error"]["field_errors"]["chapters"].is_string(),
        "{body}"
    );

    // A work with a chapter but no text is still not publishable.
    let (chapter, version) = add_chapter(&mut author, &work_id, "One").await;
    let (status, body) = publish(&mut author, &work_id, 1, None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(
        body["error"]["field_errors"]["chapters"].is_string(),
        "{body}"
    );

    // With text, it is.
    let (status, body) = save_chapter(&mut author, &chapter, version, "It began here.").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = publish(&mut author, &work_id, 1, None).await;
    assert_eq!(status, StatusCode::OK, "publish: {body}");
    assert_eq!(body["lifecycle"], "published");
    assert!(body["published_at"].is_string());

    // A stranger can now read the work and the chapter.
    let mut stranger = harness.anonymous();
    let (status, body) = stranger.get(&format!("/api/v1/works/{work_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["title"], "Untitled");
    assert_eq!(body["authors"][0]["handle"], "Quill");
    // A public view never carries the owner or a version to write back with.
    assert!(body.get("owner_pseud_id").is_none());
    assert!(body.get("version").is_none());

    let (status, body) = stranger
        .get(&format!("/api/v1/works/{work_id}/chapters/{chapter}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["sanitized_html"], "<p>It began here.</p>");
    assert_eq!(body["editable"], false);
    // The editor document is a contributor-only field.
    assert!(body["document"].is_null(), "{body}");

    harness.cleanup().await;
}

#[tokio::test]
async fn an_anonymous_reader_never_receives_a_draft_revision() {
    let harness = Harness::new("unpublished").await;
    let mut author = harness.client();
    register(&mut author, "author@example.com", "Quill").await;

    let work = create_work(&mut author, "Work").await;
    let work_id = work["id"].as_str().expect("id").to_owned();
    let (chapter, version) = add_chapter(&mut author, &work_id, "One").await;
    save_chapter(&mut author, &chapter, version, "Secret draft text.").await;
    publish(&mut author, &work_id, 1, None).await;

    let mut stranger = harness.anonymous();
    let (status, body) = stranger
        .get(&format!("/api/v1/works/{work_id}/chapters/{chapter}"))
        .await;
    assert_eq!(status, StatusCode::OK);

    // A second revision is written after publication. A reader must get the
    // current one, and must never be handed an arbitrary earlier revision.
    let (status, body) = save_chapter(
        &mut author,
        &chapter,
        body["chapter"]["version"].as_i64().unwrap(),
        "Second text.",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = stranger
        .get(&format!("/api/v1/works/{work_id}/chapters/{chapter}"))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["sanitized_html"], "<p>Second text.</p>");
    assert_eq!(body["revision_number"], 2);

    // Withdrawing takes it off the public surface entirely.
    let (status, work_body) = author.get(&format!("/api/v1/works/{work_id}")).await;
    assert_eq!(status, StatusCode::OK);
    let version = work_body["version"].as_i64().expect("version");

    let (status, body) = author
        .post(
            &format!("/api/v1/works/{work_id}/withdraw"),
            json!({ "expected_version": version }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "withdraw: {body}");
    assert_eq!(body["lifecycle"], "withdrawn");

    let (status, _) = stranger.get(&format!("/api/v1/works/{work_id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = stranger
        .get(&format!("/api/v1/works/{work_id}/chapters/{chapter}"))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // And a de-indexing side effect was queued in the same transaction.
    let topics = outbox::topics_for_work(harness.tdb.db(), work_id.parse().expect("work id"))
        .await
        .expect("topics");
    assert!(
        topics.iter().any(|topic| topic == "withdraw.deindex"),
        "expected a deindex event, got {topics:?}"
    );

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Concurrency and revisions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_stale_edit_is_refused_with_a_conflict_that_names_both_versions() {
    let harness = Harness::new("conflict").await;
    let mut first = harness.client();
    register(&mut first, "writer@example.com", "Quill").await;

    let work = create_work(&mut first, "Draft").await;
    let work_id = work["id"].as_str().expect("id").to_owned();
    let (chapter, _) = add_chapter(&mut first, &work_id, "One").await;

    // The same account in a second browser: it reads version 1 and edits.
    let mut second = harness.client();
    let (status, _) = second
        .post(
            "/api/v1/auth/login",
            json!({ "email": "writer@example.com", "password": PASSWORD }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = first
        .patch(
            &format!("/api/v1/works/{work_id}"),
            json!({ "expected_version": 1, "summary": "First writer." }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["version"], 2);

    // The second browser still believes version 1.
    let (status, body) = second
        .patch(
            &format!("/api/v1/works/{work_id}"),
            json!({ "expected_version": 1, "summary": "Second writer." }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "REVISION_CONFLICT");

    // The first writer's text survived: nothing was silently overwritten.
    let (_, body) = first.get(&format!("/api/v1/works/{work_id}")).await;
    assert_eq!(body["summary"], "First writer.");

    // The same rule holds for chapter text.
    let (_, chapter_body) = first
        .get(&format!("/api/v1/works/{work_id}/chapters/{chapter}"))
        .await;
    let version = chapter_body["chapter"]["version"]
        .as_i64()
        .expect("version");
    let (status, _) = save_chapter(&mut first, &chapter, version, "One.").await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = save_chapter(&mut second, &chapter, version, "Two.").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "REVISION_CONFLICT");

    harness.cleanup().await;
}

#[tokio::test]
async fn restoring_a_revision_creates_a_new_one_rather_than_rewriting_history() {
    let harness = Harness::new("restore").await;
    let mut author = harness.client();
    register(&mut author, "author@example.com", "Quill").await;

    let work = create_work(&mut author, "Work").await;
    let work_id = work["id"].as_str().expect("id").to_owned();
    let (chapter, version) = add_chapter(&mut author, &work_id, "One").await;

    let (_, body) = save_chapter(&mut author, &chapter, version, "First version.").await;
    let first_version_number = body["version"].as_i64().expect("version");

    let (_, body) = save_chapter(
        &mut author,
        &chapter,
        first_version_number,
        "Second version.",
    )
    .await;
    let second_version_number = body["version"].as_i64().expect("version");

    let (status, body) = author
        .get(&format!("/api/v1/chapters/{chapter}/revisions"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let revisions = body.as_array().expect("revisions");
    assert_eq!(revisions.len(), 2);
    // Newest first, and the newest is the current one.
    assert_eq!(revisions[0]["revision_number"], 2);
    assert_eq!(revisions[0]["current"], true);
    let oldest_id = revisions[1]["id"].as_str().expect("id").to_owned();

    let (status, body) = author
        .post(
            &format!("/api/v1/chapters/{chapter}/restore-revision"),
            json!({ "revision_id": oldest_id }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "restore: {body}");

    let (status, body) = author
        .get(&format!("/api/v1/chapters/{chapter}/revisions"))
        .await;
    assert_eq!(status, StatusCode::OK);
    let revisions = body.as_array().expect("revisions");
    // Three revisions: nothing was rewritten, a third was appended.
    assert_eq!(revisions.len(), 3);
    assert_eq!(revisions[0]["revision_number"], 3);
    assert_eq!(revisions[0]["restored_from_id"], json!(oldest_id));
    assert!(revisions[0]["note"]
        .as_str()
        .is_some_and(|note| note.contains("Restored from revision 1")));
    assert_eq!(revisions[2]["id"], json!(oldest_id), "history is intact");

    // The current text is the restored text.
    let (status, body) = author
        .get(&format!("/api/v1/works/{work_id}/chapters/{chapter}"))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["sanitized_html"], "<p>First version.</p>");

    let _ = second_version_number;

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Idempotent publication
// ---------------------------------------------------------------------------

#[tokio::test]
async fn replaying_one_idempotency_key_does_not_publish_or_notify_twice() {
    let harness = Harness::new("idempotent").await;
    let mut author = harness.client();
    register(&mut author, "author@example.com", "Quill").await;

    let work = create_work(&mut author, "Work").await;
    let work_id = work["id"].as_str().expect("id").to_owned();
    let (chapter, version) = add_chapter(&mut author, &work_id, "One").await;
    save_chapter(&mut author, &chapter, version, "Something to read.").await;

    let (status, body) = publish(&mut author, &work_id, 1, Some("publish-attempt-1")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["lifecycle"], "published");

    let topics_after_first = outbox::topics_for_work(harness.tdb.db(), work_id.parse().unwrap())
        .await
        .expect("topics");
    assert_eq!(
        topics_after_first
            .iter()
            .filter(|topic| topic.starts_with("publish."))
            .count(),
        2,
        "a publication queues a notification and an index update: {topics_after_first:?}"
    );

    // The same key again: accepted, but nothing happens.
    let (status, body) = publish(&mut author, &work_id, 2, Some("publish-attempt-1")).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let topics_after_replay = outbox::topics_for_work(harness.tdb.db(), work_id.parse().unwrap())
        .await
        .expect("topics");
    assert_eq!(
        topics_after_replay, topics_after_first,
        "a replayed publication must not enqueue anything"
    );

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Pseud isolation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn switching_pseuds_does_not_hand_over_a_work() {
    let harness = Harness::new("pseud-switch").await;
    let mut author = harness.client();
    register(&mut author, "author@example.com", "Quill").await;

    let work = create_work(&mut author, "Mine").await;
    let work_id = work["id"].as_str().expect("id").to_owned();

    // A second pseud on the same account.
    let (status, body) = author
        .post("/api/v1/pseuds", json!({ "handle": "Ink" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let ink = body["id"].as_str().expect("pseud id").to_owned();

    let (status, _) = author
        .post(&format!("/api/v1/pseuds/{ink}/activate"), json!({}))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Same account, different face: the work is not reachable, and is reported
    // as absent rather than as forbidden.
    let (status, _) = author.get(&format!("/api/v1/works/{work_id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = author
        .patch(
            &format!("/api/v1/works/{work_id}"),
            json!({ "expected_version": 1, "title": "Stolen" }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Its own list is empty, too: the work is not "also mine".
    let (status, body) = author.get("/api/v1/works").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.as_array().expect("works").is_empty(), "{body}");

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Collaboration
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_invitation_names_pseuds_and_grants_edit_rights_only_after_acceptance() {
    let harness = Harness::new("invitations").await;
    let mut owner = harness.client();
    register(&mut owner, "owner@example.com", "Quill").await;

    let mut other = harness.client();
    register(&mut other, "other@example.com", "Ink").await;
    let ink_pseud = active_pseud(&mut other).await;

    let work = create_work(&mut owner, "Shared").await;
    let work_id = work["id"].as_str().expect("id").to_owned();

    let (status, body) = owner
        .post(
            &format!("/api/v1/works/{work_id}/contributors/invitations"),
            json!({ "handle": "Ink", "role": "coauthor", "message": "Come and help." }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    // The invitation identifies the exposed pseud on both sides, and neither
    // account.
    assert_eq!(body["invited_handle"], "Ink");
    assert_eq!(body["invited_by_handle"], "Quill");
    assert_eq!(body["status"], "pending");
    assert!(body.get("account_id").is_none());
    assert!(!body.to_string().contains("other@example.com"));
    let invite_id = body["id"].as_str().expect("invite id").to_owned();

    // Before accepting, Ink cannot see or edit the work.
    let (status, _) = other.get(&format!("/api/v1/works/{work_id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The invitation is waiting for the invited pseud.
    let (status, body) = other.get("/api/v1/invitations").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body.as_array().expect("invitations").len(), 1);

    let (status, body) = other
        .post(
            &format!("/api/v1/invitations/{invite_id}/accept"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "accepted");

    // Now the work is reachable, with the role that was offered.
    let (status, body) = other.get(&format!("/api/v1/works/{work_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["role"], "coauthor");

    // The invited pseud is activated *before* the check, because the invitation
    // names a pseud: were Ink to switch faces, the work would disappear again.
    assert_eq!(active_pseud(&mut other).await, ink_pseud);

    // A second invitation for the same pseud is refused.
    let (status, body) = owner
        .post(
            &format!("/api/v1/works/{work_id}/contributors/invitations"),
            json!({ "handle": "Ink", "role": "editor" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(body["error"]["field_errors"]["handle"].is_string());

    // Ownership was never on offer.
    let (status, body) = owner
        .post(
            &format!("/api/v1/works/{work_id}/contributors/invitations"),
            json!({ "handle": "Ink", "role": "owner" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let _ = body;

    harness.cleanup().await;
}

#[tokio::test]
async fn an_editor_may_change_text_but_not_publish() {
    let harness = Harness::new("roles").await;
    let mut owner = harness.client();
    register(&mut owner, "owner@example.com", "Quill").await;
    let mut editor = harness.client();
    register(&mut editor, "editor@example.com", "Ink").await;

    let work = create_work(&mut owner, "Shared").await;
    let work_id = work["id"].as_str().expect("id").to_owned();
    let (chapter, version) = add_chapter(&mut owner, &work_id, "One").await;

    let (status, body) = owner
        .post(
            &format!("/api/v1/works/{work_id}/contributors/invitations"),
            json!({ "handle": "Ink", "role": "editor" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let invite = body["id"].as_str().expect("invite").to_owned();
    editor
        .post(&format!("/api/v1/invitations/{invite}/accept"), json!({}))
        .await;

    // The editor may edit text.
    let (status, body) = save_chapter(&mut editor, &chapter, version, "Edited by Ink.").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // But may not publish, nor invite.
    let (_, work_body) = owner.get(&format!("/api/v1/works/{work_id}")).await;
    let version = work_body["version"].as_i64().expect("version");
    let (status, body) = publish(&mut editor, &work_id, version, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "ACCESS_DENIED");

    let (status, body) = editor
        .post(
            &format!("/api/v1/works/{work_id}/contributors/invitations"),
            json!({ "handle": "Quill", "role": "coauthor" }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// The editor schema is the boundary
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_document_outside_the_editor_schema_is_refused_and_changes_nothing() {
    let harness = Harness::new("schema").await;
    let mut author = harness.client();
    register(&mut author, "author@example.com", "Quill").await;

    let work = create_work(&mut author, "Work").await;
    let work_id = work["id"].as_str().expect("id").to_owned();
    let (chapter, version) = add_chapter(&mut author, &work_id, "One").await;

    let (status, body) = save_chapter(&mut author, &chapter, version, "Legitimate text.").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let version = body["version"].as_i64().expect("version");

    // A script node, an unknown mark and a javascript: link are each refused.
    for hostile in [
        json!({ "type": "doc", "content": [{ "type": "script", "attrs": { "src": "x" } }] }),
        json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "hi", "marks": [{ "type": "onclick" }] }],
            }],
        }),
        json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text", "text": "click",
                    "marks": [{ "type": "link", "attrs": { "href": "javascript:alert(1)" } }],
                }],
            }],
        }),
    ] {
        let (status, body) = author
            .patch(
                &format!("/api/v1/chapters/{chapter}"),
                json!({ "expected_version": version, "document": hostile }),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["error"]["code"], "VALIDATION_FAILED");
        assert!(
            body["error"]["field_errors"]["document"].is_string(),
            "{body}"
        );
    }

    // Nothing was written: the stored revision is unchanged and there is still
    // exactly one of them.
    let (status, body) = author
        .get(&format!("/api/v1/chapters/{chapter}/revisions"))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().expect("revisions").len(), 1, "{body}");

    let (_, body) = author
        .get(&format!("/api/v1/works/{work_id}/chapters/{chapter}"))
        .await;
    assert_eq!(body["sanitized_html"], "<p>Legitimate text.</p>");

    harness.cleanup().await;
}

#[tokio::test]
async fn markup_is_escaped_in_the_reading_view() {
    let harness = Harness::new("escaping").await;
    let mut author = harness.client();
    register(&mut author, "author@example.com", "Quill").await;

    let work = create_work(&mut author, "Work").await;
    let work_id = work["id"].as_str().expect("id").to_owned();
    let (chapter, version) = add_chapter(&mut author, &work_id, "One").await;
    let (status, _) = save_chapter(
        &mut author,
        &chapter,
        version,
        "<script>alert('x')</script> & \"quotes\"",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    publish(&mut author, &work_id, 1, None).await;

    let mut stranger = harness.anonymous();
    let (status, body) = stranger
        .get(&format!("/api/v1/works/{work_id}/chapters/{chapter}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let html = body["sanitized_html"].as_str().expect("html");
    assert!(!html.contains("<script>"), "{html}");
    assert!(html.contains("&lt;script&gt;"), "{html}");
    assert!(html.contains("&amp;"), "{html}");

    harness.cleanup().await;
}
