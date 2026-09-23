//! Milestone 7 acceptance tests (spec §13).
//!
//! Spec §13's acceptance list, phrased as the properties rather than the
//! checklist: an export is a job and not a request; an export that would be
//! empty, or of a format this instance cannot produce, is refused before a job
//! exists; a converted file is really produced and really opens; a download link
//! is short-lived and single-use; one reader's export is not another's; and the
//! retention sweep removes the export without removing anything it shares bytes
//! with.
//!
//! These run against the real router, a real SQLite file, a real storage
//! directory and the real worker. The work is authored through the real API, so
//! the export renders the same document the reader's page renders. The one thing
//! replaced is the external converter, which is replaced *honestly*: a test that
//! claimed to exercise `ebook-convert` would fail on a machine without Calibre,
//! so the converter tests assert what this machine can actually do and say so.

use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_app::worker::{Worker, WorkerOptions};
use lorehaven_db::{Backend, DatabaseConfig};
use lorehaven_domain::exports::{epub, ExportFormat};
use lorehaven_domain::jobs::{JobKind, RetryPolicy};
use serde_json::{json, Value};
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m7-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn config_for(dir: &Path) -> Config {
    let mut config = Config::development_defaults();
    // Blobs live under a `storage` subdirectory, so an assertion about a file on
    // disk is looking where the store actually writes.
    config.storage.root = dir.join("storage");
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
    // Device delivery email for tests. The delivery endpoint accepts this and
    // returns "delivered" without actually sending mail (spec §13.4 / M7-03).
    config.device = Some(lorehaven_app::config::DeviceConfig {
        kindle_email: Some("test-kindle@lorehaven.example".into()),
        device_email: Some("test-device@lorehaven.example".into()),
    });
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
        let (status, _, value) = self.request_raw(method, uri, body).await;
        (status, value)
    }

    /// The same, keeping the bytes: a download is not JSON.
    async fn request_raw(
        &mut self,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Option<(String, Vec<u8>)>, Value) {
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
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_owned();
        self.capture_cookies(&response);

        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024 * 1024)
            .await
            .expect("body");
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        };
        let raw = if bytes.is_empty() {
            None
        } else {
            Some((content_type, bytes.to_vec()))
        };
        (status, raw, value)
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

    /// A download, returning the media type and the bytes.
    async fn download(&mut self, uri: &str) -> (StatusCode, String, Vec<u8>) {
        let (status, raw, _value) = self.request_raw("GET", uri, None).await;
        match raw {
            Some((content_type, bytes)) => (status, content_type, bytes),
            None => (status, String::new(), Vec::new()),
        }
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
            report.contains(&"0008_exports".to_owned()),
            "the exports migration must apply: {report:?}"
        );

        Self { dir, tdb }
    }

    fn state(&self) -> AppState {
        AppState::new(config_for(&self.dir), self.tdb.db().clone())
    }

    fn client(&self) -> Client {
        Client::new(server::build_router(self.state()))
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

fn document(text: &str) -> Value {
    json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": text }],
        }],
    })
}

/// Author a work with `chapters` chapters, each a single paragraph.
///
/// Through the API rather than through SQL, so the export renders the document a
/// reader's page would render — a repository insert that bypassed the editor's
/// schema would test the exporter against a shape nothing produces.
async fn author(client: &mut Client, title: &str, chapters: &[&str]) -> String {
    let (status, body) = client
        .post("/api/v1/works", json!({ "title": title }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    let work = body["id"].as_str().expect("work id").to_owned();

    for (index, text) in chapters.iter().enumerate() {
        let (status, body) = client
            .post(
                &format!("/api/v1/works/{work}/chapters"),
                json!({ "title": format!("Chapter {}", index + 1) }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "add chapter: {body}");
        let chapter = body["id"].as_str().expect("chapter id").to_owned();
        let version = body["version"].as_i64().expect("version");

        let (status, body) = client
            .patch(
                &format!("/api/v1/chapters/{chapter}"),
                json!({ "expected_version": version, "document": document(text) }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "save chapter: {body}");
    }

    work
}

fn worker() -> Worker {
    Worker::new(WorkerOptions {
        id: "m7-test-worker".to_owned(),
        lease: Duration::from_secs(30),
        poll_interval: Duration::from_millis(10),
        policy: RetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_millis(1),
            backoff: 2.0,
            jitter_permille: 0,
        },
        batch: 50,
        resource_classes: None,
        max_bulk_concurrent: 1,
        fairness: true,
    })
}

/// Let the worker drain the queue.
async fn drain(state: &AppState, passes: usize) {
    let worker = worker();
    for _ in 0..passes {
        let report = worker.run_once(state).await.expect("worker pass");
        if report.job.is_none() {
            return;
        }
    }
}

/// Ask for an export and return its id.
async fn request_export(
    client: &mut Client,
    subject_id: &str,
    format: &str,
    acknowledge: bool,
) -> (StatusCode, Value) {
    client
        .post(
            "/api/v1/exports",
            json!({
                "subject_type": "work",
                "subject_id": subject_id,
                "format": format,
                "acknowledge_privacy": acknowledge,
            }),
        )
        .await
}

// ---------------------------------------------------------------------------
// An export is a job
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_export_is_a_job_not_a_request() {
    let harness = Harness::new("job").await;
    let state = harness.state();
    let mut client = harness.client();
    register(&mut client, "exporter@example.org", "exporter").await;
    let work = author(&mut client, "A Finished Work", &["One.", "Two."]).await;

    let (status, body) = request_export(&mut client, &work, "epub", true).await;
    // 202: accepted, not done. A 122-chapter work rendered inside a request is
    // the thing this avoids.
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let export_id = body["id"].as_str().expect("export id").to_owned();
    assert_ne!(
        body["state"],
        json!("ready"),
        "not immediately ready: {body}"
    );
    assert_eq!(body["downloadable"], json!(false), "{body}");
    assert!(
        body["job_id"].as_str().is_some_and(|id| !id.is_empty()),
        "the answer names the job: {body}"
    );

    // The file appears only once the worker has run.
    drain(&state, 8).await;

    let (status, body) = client.get(&format!("/api/v1/exports/{export_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], json!("ready"), "{body}");
    assert_eq!(body["downloadable"], json!(true), "{body}");
    assert!(body["output_bytes"].as_i64().unwrap_or(0) > 0, "{body}");
    harness.cleanup().await;
}

#[tokio::test]
async fn an_epub_export_opens_and_contains_every_chapter() {
    let harness = Harness::new("epub").await;
    let state = harness.state();
    let mut client = harness.client();
    register(&mut client, "reader@example.org", "reader").await;
    let work = author(
        &mut client,
        "The Long Way Round",
        &[
            "The first paragraph.",
            "The second paragraph.",
            "The third paragraph.",
        ],
    )
    .await;

    let (status, body) = request_export(&mut client, &work, "epub", true).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let export_id = body["id"].as_str().expect("export id").to_owned();
    drain(&state, 8).await;

    let (status, content_type, bytes) = client
        .download(&format!("/api/v1/exports/{export_id}/download"))
        .await;
    assert_eq!(status, StatusCode::OK, "download");
    assert_eq!(content_type, "application/epub+zip");
    assert!(
        bytes.starts_with(b"PK\x03\x04"),
        "the container is a zip: {:?}",
        &bytes[..4.min(bytes.len())]
    );

    // Validated with the container's own validator, which is a structural read of
    // the bytes: the mimetype entry first and stored, the package's metadata, the
    // navigation, and the spine naming every chapter file.
    let facts = epub::validate(&bytes).expect("a valid container");
    assert_eq!(facts.title, "The Long Way Round");
    assert_eq!(
        facts.chapter_titles,
        ["Chapter 1", "Chapter 2", "Chapter 3"],
        "every chapter is present and in order"
    );
    assert!(
        !facts.language.is_empty(),
        "the package states a language: {facts:?}"
    );
    assert!(facts.has_navigation, "the package has a table of contents");
    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Refusals happen before the job
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_privacy_notice_must_be_acknowledged() {
    let harness = Harness::new("privacy").await;
    let mut client = harness.client();
    register(&mut client, "uninformed@example.org", "uninformed").await;
    let work = author(&mut client, "Some Work", &["Words."]).await;

    let (status, body) = request_export(&mut client, &work, "epub", false).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let message = body["error"]["message"].as_str().unwrap_or_default();
    // The refusal carries the notice itself, so the interface has one source for
    // the text and cannot drift from what the server enforces.
    assert!(
        message.contains("leaves the server"),
        "the refusal states the notice: {message}"
    );

    // …and nothing was queued, so there is no half-made export to find later.
    let (status, body) = client.get("/api/v1/exports").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["exports"], json!([]), "{body}");
    harness.cleanup().await;
}

#[tokio::test]
async fn an_unknown_format_is_refused_before_a_job_is_created() {
    let harness = Harness::new("unknown-format").await;
    let mut client = harness.client();
    register(&mut client, "unknown@example.org", "unknown").await;
    let work = author(&mut client, "Some Work", &["Words."]).await;

    let (status, body) = request_export(&mut client, &work, "docx", true).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("docx"),
        "the refusal names what was asked for: {body}"
    );

    let (status, body) = client.get("/api/v1/exports").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["exports"], json!([]), "{body}");
    harness.cleanup().await;
}

#[tokio::test]
async fn a_format_with_no_converter_is_refused_with_what_to_install() {
    let harness = Harness::new("no-converter").await;
    let state = harness.state();
    let mut client = harness.client();
    register(&mut client, "noconv@example.org", "noconv").await;
    let work = author(&mut client, "Some Work", &["Words."]).await;

    // What this machine can do is a fact about this machine, so the test asserts
    // the *consistency* of the two answers rather than one of them.
    let can = state.converters().can_produce(ExportFormat::Pdf);
    let (status, body) = request_export(&mut client, &work, "pdf", true).await;

    if can {
        assert_eq!(
            status,
            StatusCode::ACCEPTED,
            "this machine has a converter: {body}"
        );
    } else {
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        let message = body["error"]["message"].as_str().unwrap_or_default();
        assert!(
            message.contains("install"),
            "the refusal says what would fix it: {message}"
        );

        let (status, body) = client.get("/api/v1/exports").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["exports"], json!([]), "no job was created: {body}");
    }

    // The catalogue agrees with the refusal, which is the property that matters:
    // an interface offering a format the request refuses is worse than either
    // answer alone.
    let (status, body) = client.get("/api/v1/exports/formats").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let pdf = body["formats"]
        .as_array()
        .expect("formats")
        .iter()
        .find(|entry| entry["format"] == json!("pdf"))
        .expect("pdf is listed");
    assert_eq!(pdf["available"], json!(can), "{pdf}");
    if !can {
        assert!(
            pdf["requires"]
                .as_str()
                .is_some_and(|hint| hint.contains("install")),
            "an unavailable format says what to install: {pdf}"
        );
    }
    harness.cleanup().await;
}

#[tokio::test]
async fn exporting_a_work_with_no_chapters_is_refused() {
    let harness = Harness::new("empty").await;
    let mut client = harness.client();
    register(&mut client, "empty@example.org", "empty").await;
    let work = author(&mut client, "Nothing Yet", &[]).await;

    let (status, body) = request_export(&mut client, &work, "epub", true).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("no chapters"),
        "an empty export is refused rather than produced: {body}"
    );
    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// The download is a capability
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_download_grant_expires_and_is_single_use() {
    let harness = Harness::new("grant").await;
    let state = harness.state();
    let mut client = harness.client();
    register(&mut client, "grant@example.org", "grant").await;
    let work = author(&mut client, "A Work", &["Words."]).await;

    let (status, body) = request_export(&mut client, &work, "plain_text", true).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let export_id = body["id"].as_str().expect("export id").to_owned();
    drain(&state, 8).await;

    let (status, body) = client
        .post(&format!("/api/v1/exports/{export_id}/grant"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let url = body["url"].as_str().expect("url").to_owned();
    assert!(
        body["expires_in_seconds"].as_i64().unwrap_or(0) > 0,
        "the link states its own life: {body}"
    );

    // A fresh browser: the token is the whole credential.
    let mut stranger = harness.anonymous();
    let (status, content_type, bytes) = stranger.download(&format!("/api/v1{url}")).await;
    assert_eq!(status, StatusCode::OK, "the grant opens it once");
    assert!(content_type.starts_with("text/plain"), "{content_type}");
    let text = String::from_utf8(bytes).expect("utf-8");
    assert!(text.contains("Words."), "{text}");

    // …and not twice. One answer for spent and expired alike: telling them apart
    // would tell someone holding a guess which guesses were close.
    let (status, body) = stranger.get(&format!("/api/v1{url}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    // An export with no file cannot mint one.
    let (status, body) = request_export(&mut client, &work, "epub", true).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let pending = body["id"].as_str().expect("export id").to_owned();
    let (status, body) = client
        .post(&format!("/api/v1/exports/{pending}/grant"), json!({}))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    harness.cleanup().await;
}

#[tokio::test]
async fn one_readers_export_is_not_anothers() {
    let harness = Harness::new("scoped").await;
    let state = harness.state();
    let mut owner = harness.client();
    register(&mut owner, "owner@example.org", "owner").await;
    let work = author(&mut owner, "Private Reading", &["Words."]).await;

    let (status, body) = request_export(&mut owner, &work, "plain_text", true).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let export_id = body["id"].as_str().expect("export id").to_owned();
    drain(&state, 8).await;

    // The export borrows the work's *content*, and an export of a private work is
    // the reader's own copy — not a second way to read the work.
    let mut stranger = harness.anonymous();
    register(&mut stranger, "stranger@example.org", "stranger").await;

    let (status, body) = stranger.get(&format!("/api/v1/exports/{export_id}")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a stranger sees nothing: {body}"
    );

    let (status, _content_type, _bytes) = stranger
        .download(&format!("/api/v1/exports/{export_id}/download"))
        .await;
    // Either a 404 (not theirs) or a 403 (sign-in required): never the file.
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::FORBIDDEN,
        "a stranger is refused, got {status}"
    );

    // The owner still can.
    let (status, _content_type, bytes) = owner
        .download(&format!("/api/v1/exports/{export_id}/download"))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!bytes.is_empty());
    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Retention
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_retention_sweep_removes_the_export_and_its_output() {
    let harness = Harness::new("retention").await;
    // The development default keeps exports forever (retention_days: 0), so
    // the sweep would find nothing to do. This test is about the sweep itself,
    // so it gives the harness the spec §13.2 window.
    let mut config = config_for(&harness.dir);
    config.exports.retention_days = 7;
    let state = AppState::new(config, harness.tdb.db().clone());
    let mut client = Client::new(server::build_router(state.clone()));
    register(&mut client, "aging@example.org", "aging").await;
    let work = author(&mut client, "An Old Export", &["Words."]).await;

    let (status, body) = request_export(&mut client, &work, "plain_text", true).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let export_id = body["id"].as_str().expect("export id").to_owned();
    drain(&state, 8).await;

    // Where the output is on disk, before anything removes it.
    let row = lorehaven_db::exports::find_export(harness.tdb.db(), &export_id)
        .await
        .expect("find")
        .expect("the export exists");
    let checksum = row
        .output_blob_checksum
        .clone()
        .expect("a ready export has an output");
    let store = lorehaven_db::storage::BlobStore::new(config_for(&harness.dir).storage.root);
    let path = store.path_for(&checksum);
    assert!(path.exists(), "the output is on disk at {}", path.display());

    // The same bytes, held by someone else too. Content is shared by checksum
    // across the whole instance, so the sweep must not be able to delete a
    // reader's reading copy on its way past.
    store
        .reference(harness.tdb.db(), &checksum, "test_other_owner", "elsewhere")
        .await
        .expect("a second reference");

    // Age the export past its window. The window is seven days and this test is
    // not going to wait, so the row's own clock is moved.
    let sql = harness.tdb.db().sql(
        "UPDATE export_jobs SET created_at = '2020-01-01T00:00:00Z' WHERE id = ?",
        "UPDATE export_jobs SET created_at = '2020-01-01T00:00:00Z' WHERE id::text = $1",
    );
    match harness.tdb.db().backend() {
        Backend::Sqlite => {
            sqlx::query(sql.as_ref())
                .bind(&export_id)
                .execute(harness.tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("age the export");
        }
        Backend::Postgres => {
            sqlx::query(sql.as_ref())
                .bind(&export_id)
                .execute(harness.tdb.db().postgres_pool().expect("postgres"))
                .await
                .expect("age the export");
        }
    }

    // Queued through the queue itself, as the CLI does, so this asserts the
    // worker's own behaviour rather than the operator route's permissions.
    lorehaven_db::jobs::enqueue(
        harness.tdb.db(),
        JobKind::Maintenance,
        &json!({ "task": "purge_exports" }).to_string(),
        None,
        None,
        0,
        &RetryPolicy::default(),
    )
    .await
    .expect("queue the sweep");
    drain(&state, 8).await;

    assert!(
        lorehaven_db::exports::find_export(harness.tdb.db(), &export_id)
            .await
            .expect("find")
            .is_none(),
        "the export is gone"
    );
    assert!(
        path.exists(),
        "…but the bytes are still there for the other owner"
    );

    // With the other reference gone too, the collector is entitled to remove it —
    // and this is the only place that deletion happens.
    store
        .unreference(harness.tdb.db(), &checksum, "test_other_owner", "elsewhere")
        .await
        .expect("drop the second reference");
    assert!(
        store
            .delete_if_unreferenced(harness.tdb.db(), &checksum)
            .await
            .expect("collect"),
        "an unreferenced blob is deleted"
    );
    assert!(!path.exists());
    harness.cleanup().await;
}

#[tokio::test]
async fn the_sweep_task_is_one_the_worker_knows() {
    // The CLI queues `purge_exports` by name and the worker matches on it, so a
    // rename on either side is a task that silently never runs — and retention is
    // a promise, so a sweep nobody runs is a promise nobody keeps.
    assert!(
        lorehaven_app::MAINTENANCE_TASKS.contains(&"purge_exports"),
        "the sweep is queued by name: {:?}",
        lorehaven_app::MAINTENANCE_TASKS
    );
    assert!(
        matches!(JobKind::parse("export"), Some(JobKind::Export)),
        "the export job kind round-trips"
    );
}

// ---------------------------------------------------------------------------
// Device delivery (spec §13.4 / M7-03)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn device_delivery_with_configured_email_returns_delivered() {
    let harness = Harness::new("with-device").await;
    let mut client = harness.client();
    register(&mut client, "withdevice@example.org", "withdevice").await;
    let work = author(&mut client, "Some Work", &["Words."]).await;

    let (status, body) = request_export(&mut client, &work, "epub", true).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let export_id = body["id"].as_str().expect("export id").to_owned();
    drain(&harness.state(), 8).await;

    let (status, body) = client
        .post(
            &format!("/api/v1/exports/{export_id}/deliver"),
            json!({ "device": "kindle" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "delivered");
    assert_eq!(body["device"], "kindle");
    assert_eq!(body["target_email"], "test-kindle@lorehaven.example");
    harness.cleanup().await;
}

#[tokio::test]
async fn device_delivery_refuses_unknown_device() {
    let harness = Harness::new("unknown-device").await;
    let mut client = harness.client();
    register(&mut client, "unknowndevice@example.org", "unknowndevice").await;
    let work = author(&mut client, "Some Work", &["Words."]).await;

    let (status, body) = request_export(&mut client, &work, "epub", true).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let export_id = body["id"].as_str().expect("export id").to_owned();
    drain(&harness.state(), 8).await;

    let (status, body) = client
        .post(
            &format!("/api/v1/exports/{export_id}/deliver"),
            json!({ "device": "smart_fridge" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("smart_fridge"),
        "the refusal names the unknown device: {message}"
    );
    harness.cleanup().await;
}
