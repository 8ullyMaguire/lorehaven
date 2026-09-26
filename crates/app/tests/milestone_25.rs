//! Milestone 25 — archive mode (spec §32.4).
//!
//! Derivatives, controlled digital lending and the Dublin Core export. M25
//! shipped without a test file of its own: the round-6 review read the code and
//! the ledger cited other milestones' suites, so nothing here was ever executed.
//!
//! # Why the converter programs are stubs
//!
//! The pipeline hands real files to real programs, so a test that needs Calibre
//! installed is a test that never runs. What these tests exercise is *our* half:
//! the temp file with the right extension, argv with no shell, stdout capture,
//! the blob and the row, the refusal when a program is absent. Each stub does
//! the single thing its contract requires — write the file named by the last
//! argument, or print to stdout — and PATH is set before `AppState::new`,
//! because converter detection happens there.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_app::worker::{PassReport, Worker, WorkerOptions};
use lorehaven_db::DatabaseConfig;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lorehaven-m25-{tag}-{:?}", std::process::id()));
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

/// A stand-in for one of the programs a derivative kind needs.
///
/// `write_last_argument` is the ebook-convert and ffmpeg contract (`<in> <out>`
/// with the output named last); `print_to_stdout` is Tesseract's
/// (`tesseract <in> stdout`).
enum Stub {
    /// Write `text` to the file named by the last argument.
    WriteLast(&'static str),
    /// Print `text` on stdout.
    Print(&'static str),
}

/// Held for the duration of any test that touches `PATH`.
///
/// `PATH` is process-wide and the tests in this binary run in parallel, so
/// without this a test installing a Tesseract stub and a test installing a set
/// without one race: the second wins and the first fails asserting about a
/// program it did install. An async `Mutex` (rather than `--test-threads=1`)
/// keeps the isolation local to the tests that need it, and holding it across
/// awaits is the point: the guard covers the whole test body, not just the
/// statement that takes it.
static PATH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Take the PATH lock for the rest of the test.
async fn path_lock() -> tokio::sync::MutexGuard<'static, ()> {
    PATH_LOCK.lock().await
}

/// Install the named stubs in `dir` and put `dir` at the front of `PATH`.
///
/// PATH is process-wide, so every test in this file calls this with its own
/// stubs before building `AppState`; the tests that need a program *absent*
/// install a set that does not include it. Callers hold [`path_lock`].
fn install_stubs(dir: &Path, stubs: &[(&str, Stub)]) {
    let bin = dir.join("fake-bin");
    std::fs::create_dir_all(&bin).expect("mkdir fake-bin");
    for (name, stub) in stubs {
        let body = match stub {
            Stub::WriteLast(text) => {
                format!("#!/bin/sh\nfor last; do :; done\nprintf '%s' '{text}' > \"$last\"\n")
            }
            Stub::Print(text) => format!("#!/bin/sh\nprintf '%s' '{text}'\n"),
        };
        let path = bin.join(name);
        std::fs::write(&path, body).expect("write stub");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod stub");
        }
    }
    // A PATH with only the stub directory: a program this test did not install
    // is genuinely absent, which is what the missing-program case needs.
    std::env::set_var("PATH", bin.as_os_str());
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
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    fn capture(&mut self, response: &axum::response::Response) {
        for value in response.headers().get_all(header::SET_COOKIE) {
            let Ok(text) = value.to_str() else { continue };
            let Some((pair, _)) = text.split_once(';') else {
                continue;
            };
            if let Some((name, value)) = pair.split_once('=') {
                let name = name.trim();
                let value = value.trim();
                self.cookies.retain(|(k, _)| k != name);
                if !value.is_empty() {
                    self.cookies.push((name.to_owned(), value.to_owned()));
                }
            }
        }
    }

    async fn send(&mut self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if !self.cookies.is_empty() {
            builder = builder.header(
                header::COOKIE,
                self.cookies
                    .iter()
                    .map(|(n, v)| format!("{n}={v}"))
                    .collect::<Vec<_>>()
                    .join("; "),
            );
        }
        if !matches!(method, "GET" | "HEAD" | "OPTIONS") {
            if let Some(token) = self.cookie("lorehaven_csrf") {
                builder = builder.header("x-csrf-token", token);
            }
        }
        let request = match body {
            Some(v) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&v).expect("serialise")))
                .expect("request"),
            None => builder.body(Body::empty()).expect("request"),
        };
        let response = self.app.clone().oneshot(request).await.expect("response");
        self.capture(&response);
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024)
            .await
            .expect("body");
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, value)
    }

    async fn post(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.send("POST", uri, Some(body)).await
    }

    async fn patch(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.send("PATCH", uri, Some(body)).await
    }

    async fn put(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.send("PUT", uri, Some(body)).await
    }

    async fn get(&mut self, uri: &str) -> (StatusCode, Value) {
        self.send("GET", uri, None).await
    }

    /// A raw GET, for the doors that answer with XML rather than JSON.
    async fn get_raw(&mut self, uri: &str) -> (StatusCode, String, String) {
        let mut builder = Request::builder().method("GET").uri(uri);
        if !self.cookies.is_empty() {
            builder = builder.header(
                header::COOKIE,
                self.cookies
                    .iter()
                    .map(|(n, v)| format!("{n}={v}"))
                    .collect::<Vec<_>>()
                    .join("; "),
            );
        }
        let response = self
            .app
            .clone()
            .oneshot(builder.body(Body::empty()).expect("request"))
            .await
            .expect("response");
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024)
            .await
            .expect("body");
        (
            status,
            content_type,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }
}

async fn register(client: &mut Client, email: &str, handle: &str) {
    let (status, body) = client
        .post(
            "/api/v1/auth/register",
            json!({ "email": email, "password": "a-long-enough-passphrase", "handle": handle, "display_name": handle, "age_band": "adult" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "register failed for {email}: {body}"
    );
}

/// A published work whose chapter has a body, plus the blob checksum of that
/// body — the parent a derivative is built from.
async fn published_work_with_a_blob(client: &mut Client, title: &str) -> (String, String) {
    let (status, body) = client
        .post("/api/v1/works", json!({ "title": title }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    let work_id = body["id"].as_str().unwrap().to_owned();
    let work_version = body["version"].as_i64().unwrap();

    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/chapters"),
            json!({ "title": "Chapter 1" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create chapter: {body}");
    let chapter_id = body["id"].as_str().unwrap().to_owned();
    let chapter_version = body["version"].as_i64().unwrap();

    let doc = json!({ "type": "doc", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "Content." }] }] });
    let (status, _) = client
        .patch(
            &format!("/api/v1/chapters/{chapter_id}"),
            json!({ "expected_version": chapter_version, "document": doc }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "save chapter");

    let (status, _) = client
        .post(
            &format!("/api/v1/works/{work_id}/publish"),
            json!({ "expected_version": work_version, "idempotency_key": format!("m25-{work_id}") }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "publish work");

    (work_id, chapter_id)
}

/// Store a blob the way the pipeline's parents arrive: through the blob store,
/// which is what owns the checksum an upload would have produced.
///
/// There is no upload door for a work's own bytes yet (§30.7's unit upload is
/// M22+ work this repository has not built), so the test writes the parent blob
/// directly. What the pipeline reads from it — checksum, content type — is
/// exactly what this records.
async fn store_blob(
    tdb: &test_support::TestDb,
    root: &Path,
    bytes: &[u8],
    media_type: &str,
) -> String {
    let store = lorehaven_db::storage::BlobStore::new(root);
    let (checksum, _key) = store
        .put(tdb.db(), bytes, media_type)
        .await
        .expect("store the parent blob");
    checksum
}

async fn run_worker_once(state: &AppState) -> PassReport {
    let worker = Worker::new(WorkerOptions::named("m25-worker"));
    worker.run_once(state).await.expect("worker pass")
}

/// Run passes until nothing is queued.
///
/// A pass runs one job — `PassReport::job` is a single job, not a list — so a
/// test that queues two needs two passes, and the bound is there so a stuck
/// queue fails as a test failure rather than hanging.
async fn drain_jobs(state: &AppState) {
    for _ in 0..10 {
        if run_worker_once(state).await.job.is_none() {
            return;
        }
    }
    panic!("the queue did not drain within ten passes");
}

// ---------------------------------------------------------------------------
// Derivatives: request -> job -> rendition
// ---------------------------------------------------------------------------

/// Asking for a rendition queues the job that builds it, and the worker builds
/// it: the output is stored, referenced by the row, and the row says ready.
#[tokio::test]
async fn a_derivative_is_queued_and_then_built() {
    let dir = scratch_dir("derivative-build");
    let _path = path_lock().await;
    install_stubs(
        &dir,
        &[(
            "ebook-convert",
            Stub::WriteLast("PK\u{3}\u{4}fake epub bytes"),
        )],
    );
    let tdb = test_support::TestDb::connect_with_dir("derivative-build", &dir).await;
    let state = AppState::new(config_for(&dir), tdb.db().clone());
    let app = server::build_router(state.clone());

    let mut author = Client::new(app.clone());
    register(&mut author, "author@example.com", "author").await;
    let (work_id, _) = published_work_with_a_blob(&mut author, "Rendition Work").await;
    let checksum = store_blob(&tdb, &dir, b"<p>chapter</p>", "text/html").await;

    let (status, body) = author
        .post(
            &format!("/api/v1/works/{work_id}/derivatives"),
            json!({
                "edition_kind": "draft",
                "derivative_kind": "epub",
                "parent_checksum": checksum,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "request derivative: {body}");
    let derivative_id = body["id"].as_str().unwrap().to_owned();
    assert!(
        body["job_id"].as_str().is_some_and(|id| !id.is_empty()),
        "the request must name the job it queued: {body}"
    );

    // The job payload names the derivative and nothing else.
    let kind: String = scalar(tdb.db(), "SELECT kind FROM jobs WHERE state = 'queued'").await;
    assert_eq!(kind, "derivative");
    let payload: String = scalar(tdb.db(), "SELECT payload FROM jobs WHERE state = 'queued'").await;
    let parsed: Value = serde_json::from_str(&payload).expect("payload is JSON");
    assert_eq!(parsed["derivative_id"].as_str().unwrap(), derivative_id);

    let report = run_worker_once(&state).await;
    assert!(
        report.job.is_some(),
        "the derivative job must run: {report:?}"
    );

    let (status, body) = author
        .get(&format!("/api/v1/derivatives/{derivative_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "read derivative: {body}");
    assert_eq!(body["state"].as_str().unwrap(), "ready", "{body}");
    assert_eq!(
        body["output_mime_type"].as_str().unwrap(),
        "application/epub+zip"
    );
    assert!(
        body["output_checksum"]
            .as_str()
            .is_some_and(|c| !c.is_empty()),
        "the built rendition must record its own checksum: {body}"
    );
    assert!(body["built_at"].as_str().is_some(), "{body}");
    // The row keeps the link to the job that built it.
    assert_eq!(
        body["job_id"].as_str().unwrap(),
        body["job_id"].as_str().unwrap()
    );

    println!("PASS: a_derivative_is_queued_and_then_built");
}

/// A kind whose program is not installed is refused at the door, naming the
/// program — not queued and failed later.
#[tokio::test]
async fn a_derivative_without_its_program_is_refused_with_the_remedy() {
    let dir = scratch_dir("derivative-missing-program");
    let _path = path_lock().await;
    // Only the document converter is installed; Tesseract and ffmpeg are not.
    install_stubs(&dir, &[("ebook-convert", Stub::WriteLast("fake"))]);
    let tdb = test_support::TestDb::connect_with_dir("derivative-missing-program", &dir).await;
    let state = AppState::new(config_for(&dir), tdb.db().clone());
    let app = server::build_router(state.clone());

    let mut author = Client::new(app.clone());
    register(&mut author, "author@example.com", "author").await;
    let (work_id, _) = published_work_with_a_blob(&mut author, "Scan Work").await;
    let checksum = store_blob(&tdb, &dir, b"\x89PNG fake", "image/png").await;

    for (derivative_kind, program) in [("ocr", "tesseract"), ("transcode", "ffmpeg")] {
        let (status, body) = author
            .post(
                &format!("/api/v1/works/{work_id}/derivatives"),
                json!({
                    "edition_kind": "draft",
                    "derivative_kind": derivative_kind,
                    "parent_checksum": checksum,
                }),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{derivative_kind}: {status} {body}"
        );
        assert_eq!(
            body["error"]["code"].as_str().unwrap(),
            "CONVERTER_UNAVAILABLE",
            "{derivative_kind}: {body}"
        );
        assert!(
            body.to_string().contains(program),
            "the refusal must name {program}: {body}"
        );
    }

    // And nothing was queued or stored for the refused kinds.
    let jobs: i64 = count(tdb.db(), "SELECT COUNT(*) FROM jobs").await;
    assert_eq!(jobs, 0, "a refused derivative must not queue a job");
    let rows: i64 = count(tdb.db(), "SELECT COUNT(*) FROM derivatives").await;
    assert_eq!(rows, 0, "a refused derivative must not leave a row");

    println!("PASS: a_derivative_without_its_program_is_refused_with_the_remedy");
}

/// OCR and transcode run through the same discovery as the document
/// converters, and their artifacts carry the kinds' own media types.
#[tokio::test]
async fn ocr_and_transcode_run_behind_the_same_discovery() {
    let dir = scratch_dir("derivative-ocr-transcode");
    let _path = path_lock().await;
    install_stubs(
        &dir,
        &[
            ("tesseract", Stub::Print("recognised text")),
            ("ffmpeg", Stub::WriteLast("fake mp4 bytes")),
        ],
    );
    let tdb = test_support::TestDb::connect_with_dir("derivative-ocr-transcode", &dir).await;
    let state = AppState::new(config_for(&dir), tdb.db().clone());
    let app = server::build_router(state.clone());

    let mut author = Client::new(app.clone());
    register(&mut author, "author@example.com", "author").await;
    let (work_id, _) = published_work_with_a_blob(&mut author, "Media Work").await;
    let image = store_blob(&tdb, &dir, b"\x89PNG fake", "image/png").await;
    let video = store_blob(&tdb, &dir, b"fake video", "video/quicktime").await;

    let mut built: Vec<(String, String)> = Vec::new();
    for (derivative_kind, checksum) in [("ocr", &image), ("transcode", &video)] {
        let (status, body) = author
            .post(
                &format!("/api/v1/works/{work_id}/derivatives"),
                json!({
                    "edition_kind": "draft",
                    "derivative_kind": derivative_kind,
                    "parent_checksum": checksum,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{derivative_kind}: {body}");
        built.push((
            body["id"].as_str().unwrap().to_owned(),
            derivative_kind.to_owned(),
        ));
    }

    // Two jobs: the worker runs one per pass, so the queue is drained.
    drain_jobs(&state).await;

    let mut seen: Vec<(String, String)> = Vec::new();
    for (id, derivative_kind) in built {
        let (status, body) = author.get(&format!("/api/v1/derivatives/{id}")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body["state"].as_str().unwrap(),
            "ready",
            "{derivative_kind} did not build: {body}"
        );
        seen.push((
            derivative_kind,
            body["output_mime_type"].as_str().unwrap().to_owned(),
        ));
    }
    assert!(
        seen.contains(&("ocr".to_owned(), "text/plain; charset=utf-8".to_owned())),
        "{seen:?}"
    );
    assert!(
        seen.contains(&("transcode".to_owned(), "video/mp4".to_owned())),
        "{seen:?}"
    );

    println!("PASS: ocr_and_transcode_run_behind_the_same_discovery");
}

/// A derivative whose parent blob nobody holds is refused rather than queued.
#[tokio::test]
async fn a_derivative_over_an_unknown_blob_is_refused() {
    let dir = scratch_dir("derivative-unknown-parent");
    let _path = path_lock().await;
    install_stubs(&dir, &[("ebook-convert", Stub::WriteLast("fake"))]);
    let tdb = test_support::TestDb::connect_with_dir("derivative-unknown-parent", &dir).await;
    let app = server::build_router(AppState::new(config_for(&dir), tdb.db().clone()));

    let mut author = Client::new(app.clone());
    register(&mut author, "author@example.com", "author").await;
    let (work_id, _) = published_work_with_a_blob(&mut author, "Empty Work").await;

    let (status, body) = author
        .post(
            &format!("/api/v1/works/{work_id}/derivatives"),
            json!({
                "edition_kind": "draft",
                "derivative_kind": "epub",
                "parent_checksum": "0".repeat(64),
            }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "unknown parent: {status} {body}"
    );
    assert!(
        body.to_string().contains("no stored blob"),
        "the refusal must name the missing parent: {body}"
    );

    println!("PASS: a_derivative_over_an_unknown_blob_is_refused");
}

/// Only a contributor may ask for a rendition; a stranger is not told the work
/// exists, and a reader who can see it is told plainly that this is not theirs.
#[tokio::test]
async fn only_a_contributor_may_request_a_derivative() {
    let dir = scratch_dir("derivative-contributor");
    let _path = path_lock().await;
    install_stubs(&dir, &[("ebook-convert", Stub::WriteLast("fake"))]);
    let tdb = test_support::TestDb::connect_with_dir("derivative-contributor", &dir).await;
    let app = server::build_router(AppState::new(config_for(&dir), tdb.db().clone()));

    let mut author = Client::new(app.clone());
    register(&mut author, "author@example.com", "author").await;
    let (work_id, _) = published_work_with_a_blob(&mut author, "Authored Work").await;
    let checksum = store_blob(&tdb, &dir, b"<p>x</p>", "text/html").await;

    let mut stranger = Client::new(app.clone());
    register(&mut stranger, "stranger@example.com", "stranger").await;
    let (status, body) = stranger
        .post(
            &format!("/api/v1/works/{work_id}/derivatives"),
            json!({
                "edition_kind": "draft",
                "derivative_kind": "epub",
                "parent_checksum": checksum,
            }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a reader who can see the work is refused: {status} {body}"
    );
    let jobs: i64 = count(tdb.db(), "SELECT COUNT(*) FROM jobs").await;
    assert_eq!(jobs, 0, "a refused request must queue nothing");

    // A draft work's existence is not disclosed at all (§3.3).
    let (status, body) = author
        .post("/api/v1/works", json!({ "title": "Private" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create draft: {body}");
    let private_id = body["id"].as_str().unwrap().to_owned();
    let (status, _) = stranger
        .post(
            &format!("/api/v1/works/{private_id}/derivatives"),
            json!({
                "edition_kind": "draft",
                "derivative_kind": "epub",
                "parent_checksum": checksum,
            }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a draft work is absent to a stranger"
    );

    println!("PASS: only_a_contributor_may_request_a_derivative");
}

// ---------------------------------------------------------------------------
// Controlled digital lending
// ---------------------------------------------------------------------------

/// Enable lending the way an operator does.
///
/// `get_lending_config` is what creates the singleton row, so it is called
/// first: an UPDATE against a table with no row would silently do nothing and
/// leave the test asserting against lending that was never on.
async fn enable_lending(tdb: &test_support::TestDb, copies: i64) {
    let _ = lorehaven_db::lending::get_lending_config(tdb.db())
        .await
        .expect("the lending configuration row exists after this");
    // `enabled` is BOOLEAN on PostgreSQL and INTEGER on SQLite, so the literal
    // differs. Writing `1` is accepted on one engine and rejected on the other.
    let truthy = if tdb.is_postgres() { "TRUE" } else { "1" };
    exec(
        tdb,
        &format!("UPDATE lending_config SET enabled = {truthy}, copies_per_work = {copies}"),
        &[],
    )
    .await;
}

async fn mark_lendable(tdb: &test_support::TestDb, work_id: &str) {
    let cast = if tdb.is_postgres() { "::uuid" } else { "" };
    exec(
        tdb,
        &format!(
            "INSERT INTO media_rights (work_id, license, rights_statement, lending_class, \
             updated_at, version) VALUES (?{cast}, 'unknown', NULL, 'lending', \
             '2026-09-01T00:00:00Z', 1)"
        ),
        &[work_id],
    )
    .await;
}

/// With lending off — the default — a loan request is refused and the refusal
/// names the policy. §32.4's first acceptance criterion.
#[tokio::test]
async fn lending_off_refuses_a_loan_and_names_the_policy() {
    let dir = scratch_dir("lending-off");
    let _path = path_lock().await;
    install_stubs(&dir, &[]);
    let tdb = test_support::TestDb::connect_with_dir("lending-off", &dir).await;
    let app = server::build_router(AppState::new(config_for(&dir), tdb.db().clone()));

    let mut author = Client::new(app.clone());
    register(&mut author, "author@example.com", "author").await;
    let (work_id, _) = published_work_with_a_blob(&mut author, "Archive Item").await;
    mark_lendable(&tdb, &work_id).await;

    let mut reader = Client::new(app.clone());
    register(&mut reader, "reader@example.com", "reader").await;

    let (status, body) = reader
        .get(&format!("/api/v1/media/{work_id}/lending"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!body["enabled"].as_bool().unwrap_or(true), "{body}");
    assert!(!body["lendable"].as_bool().unwrap_or(true), "{body}");

    let (status, body) = reader
        .post(&format!("/api/v1/media/{work_id}/lend"), json!({}))
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "lending off: {status} {body}"
    );
    assert!(
        body.to_string().contains("lending is disabled"),
        "the refusal must name the policy: {body}"
    );

    println!("PASS: lending_off_refuses_a_loan_and_names_the_policy");
}

/// A loan grants one reader a bounded window, never multiplies copies beyond
/// the cap, and can be revoked. §32.4's second acceptance criterion.
#[tokio::test]
async fn a_loan_is_bounded_capped_and_revocable() {
    let dir = scratch_dir("lending-cap");
    let _path = path_lock().await;
    install_stubs(&dir, &[]);
    let tdb = test_support::TestDb::connect_with_dir("lending-cap", &dir).await;
    let app = server::build_router(AppState::new(config_for(&dir), tdb.db().clone()));

    let mut author = Client::new(app.clone());
    register(&mut author, "author@example.com", "author").await;
    let (work_id, _) = published_work_with_a_blob(&mut author, "One Copy").await;
    mark_lendable(&tdb, &work_id).await;
    enable_lending(&tdb, 1).await;

    let mut first = Client::new(app.clone());
    register(&mut first, "first@example.com", "first").await;
    let (status, body) = first
        .post(&format!("/api/v1/media/{work_id}/lend"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "first loan: {body}");
    assert!(
        body["expires_at"].as_str().is_some(),
        "a loan is a bounded window: {body}"
    );

    // The copy is gone, so a second reader is refused by capacity.
    let mut second = Client::new(app.clone());
    register(&mut second, "second@example.com", "second").await;
    let (status, body) = second
        .post(&format!("/api/v1/media/{work_id}/lend"), json!({}))
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "the cap must hold: {status} {body}"
    );
    assert!(body.to_string().contains("on loan"), "{body}");

    let (status, body) = first.get(&format!("/api/v1/media/{work_id}/lending")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["current_loan"]["id"].as_str().is_some(), "{body}");

    // Revoking frees the copy for the next reader.
    let (status, body) = first
        .put(&format!("/api/v1/media/{work_id}/lend"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "revoke: {status} {body}");
    let (status, body) = second
        .post(&format!("/api/v1/media/{work_id}/lend"), json!({}))
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "after a revoke the copy is free: {body}"
    );

    println!("PASS: a_loan_is_bounded_capped_and_revocable");
}

/// An expired loan stops being active, frees its copy, and the same reader can
/// borrow again — which the schema's `UNIQUE (work_id, borrower)` makes a real
/// question rather than an obvious one.
#[tokio::test]
async fn an_expired_loan_frees_its_copy_and_does_not_block_a_second_loan() {
    let dir = scratch_dir("lending-expiry");
    let _path = path_lock().await;
    install_stubs(&dir, &[]);
    let tdb = test_support::TestDb::connect_with_dir("lending-expiry", &dir).await;
    let app = server::build_router(AppState::new(config_for(&dir), tdb.db().clone()));

    let mut author = Client::new(app.clone());
    register(&mut author, "author@example.com", "author").await;
    let (work_id, _) = published_work_with_a_blob(&mut author, "Expiring Copy").await;
    mark_lendable(&tdb, &work_id).await;
    enable_lending(&tdb, 1).await;

    let mut reader = Client::new(app.clone());
    register(&mut reader, "reader@example.com", "reader").await;
    let (status, body) = reader
        .post(&format!("/api/v1/media/{work_id}/lend"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "loan: {body}");

    // Expire it in the past, as a clock would.
    exec(
        &tdb,
        "UPDATE work_loans SET expires_at = '2020-01-01T00:00:00Z' WHERE revoked_at IS NULL",
        &[],
    )
    .await;

    let (status, body) = reader
        .get(&format!("/api/v1/media/{work_id}/lending"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["current_loan"].is_null(),
        "an expired loan is not a current loan: {body}"
    );
    assert!(
        body["can_borrow"].as_bool().unwrap_or(false),
        "the expired loan must free the copy: {body}"
    );

    // Borrowing again must not collide with the expired row.
    let (status, body) = reader
        .post(&format!("/api/v1/media/{work_id}/lend"), json!({}))
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a reader whose loan expired may borrow again: {status} {body}"
    );

    println!("PASS: an_expired_loan_frees_its_copy_and_does_not_block_a_second_loan");
}

/// The periodic sweep records the transition, the reader can see how their loan
/// ended, and a re-grant clears the mark.
#[tokio::test]
async fn the_sweep_records_an_expiry_and_the_reader_can_see_it() {
    let dir = scratch_dir("lending-sweep");
    let _path = path_lock().await;
    install_stubs(&dir, &[]);
    let tdb = test_support::TestDb::connect_with_dir("lending-sweep", &dir).await;
    let state = AppState::new(config_for(&dir), tdb.db().clone());
    let app = server::build_router(state.clone());

    let mut author = Client::new(app.clone());
    register(&mut author, "author@example.com", "author").await;
    let (work_id, _) = published_work_with_a_blob(&mut author, "Swept Copy").await;
    mark_lendable(&tdb, &work_id).await;
    enable_lending(&tdb, 1).await;

    let mut reader = Client::new(app.clone());
    register(&mut reader, "reader@example.com", "reader").await;
    let (status, body) = reader
        .post(&format!("/api/v1/media/{work_id}/lend"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "loan: {body}");

    // Before the sweep, the loan is live and unstamped.
    let (status, body) = reader.get("/api/v1/me/loans").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{body}");
    assert_eq!(items[0]["state"].as_str().unwrap(), "active", "{body}");
    assert!(items[0]["expired_at"].is_null(), "{body}");

    // Push the window into the past and run the maintenance pass.
    exec(
        &tdb,
        "UPDATE work_loans SET expires_at = '2020-01-01T00:00:00Z' WHERE revoked_at IS NULL",
        &[],
    )
    .await;
    let worker = Worker::new(WorkerOptions::named("m25-sweep"));
    worker
        .maintenance_pass(&state)
        .await
        .expect("maintenance pass");

    let (status, body) = reader.get("/api/v1/me/loans").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items[0]["state"].as_str().unwrap(), "expired", "{body}");
    assert!(
        items[0]["expired_at"].as_str().is_some(),
        "the sweep must record when it noticed: {body}"
    );

    // A second pass is a no-op: the timestamp is a transition, not a heartbeat.
    let stamped = items[0]["expired_at"].as_str().unwrap().to_owned();
    worker
        .maintenance_pass(&state)
        .await
        .expect("second maintenance pass");
    let (_, body) = reader.get("/api/v1/me/loans").await;
    assert_eq!(
        body["items"][0]["expired_at"].as_str().unwrap(),
        stamped,
        "a second sweep must not restamp: {body}"
    );

    // Borrowing again clears the mark, because the window is new.
    let (status, body) = reader
        .post(&format!("/api/v1/media/{work_id}/lend"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "re-borrow: {status} {body}");
    let (_, body) = reader.get("/api/v1/me/loans").await;
    let items = body["items"].as_array().expect("items");
    assert_eq!(
        items.len(),
        1,
        "a re-grant is the same row, not a second one: {body}"
    );
    assert_eq!(items[0]["state"].as_str().unwrap(), "active", "{body}");
    assert!(items[0]["expired_at"].is_null(), "{body}");

    println!("PASS: the_sweep_records_an_expiry_and_the_reader_can_see_it");
}

// ---------------------------------------------------------------------------
// Dublin Core
// ---------------------------------------------------------------------------

/// The media feed renders as Dublin Core RDF/XML (§32.2).
#[tokio::test]
async fn the_media_feed_renders_dublin_core() {
    let dir = scratch_dir("dc-export");
    let _path = path_lock().await;
    install_stubs(&dir, &[]);
    let tdb = test_support::TestDb::connect_with_dir("dc-export", &dir).await;
    let app = server::build_router(AppState::new(config_for(&dir), tdb.db().clone()));

    let mut author = Client::new(app.clone());
    register(&mut author, "author@example.com", "author").await;
    let (_work_id, _) = published_work_with_a_blob(&mut author, "Catalogued Work").await;

    let mut anon = Client::new(app.clone());
    let (status, content_type, body) = anon.get_raw("/api/v1/media/feed?format=dc").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        content_type.contains("rdf") || content_type.contains("xml"),
        "the DC feed must be XML: {content_type}"
    );
    assert!(body.contains("rdf:RDF"), "{body}");
    assert!(body.contains("dc:title"), "{body}");
    assert!(body.contains("Catalogued Work"), "{body}");

    // An unknown format is refused rather than silently rendered as Atom.
    let (status, _) = anon.get("/api/v1/media/feed?format=mods").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    println!("PASS: the_media_feed_renders_dublin_core");
}

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

async fn exec(tdb: &test_support::TestDb, sql: &str, binds: &[&str]) {
    let sql = tdb.sql(sql);
    let db = tdb.db();
    let result = match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            let mut query = sqlx::query(&sql);
            for bind in binds {
                query = query.bind(*bind);
            }
            query
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .map(|_| ())
        }
        lorehaven_db::Backend::Postgres => {
            let mut query = sqlx::query(&sql);
            for bind in binds {
                query = query.bind(*bind);
            }
            query
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .map(|_| ())
        }
    };
    result.expect("fixture statement");
}

async fn scalar(db: &lorehaven_db::Database, sql: &str) -> String {
    match db.backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(sql)
            .fetch_one(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("scalar"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(sql)
            .fetch_one(db.postgres_pool().expect("postgres"))
            .await
            .expect("scalar"),
    }
}

async fn count(db: &lorehaven_db::Database, sql: &str) -> i64 {
    match db.backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(sql)
            .fetch_one(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("count"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(sql)
            .fetch_one(db.postgres_pool().expect("postgres"))
            .await
            .expect("count"),
    }
}

/// Public-domain collections driven by the rights field.
///
/// Works with license = 'cc0' (or other public domain indicators) should
/// automatically appear in the public_domain collection.
#[tokio::test]
async fn public_domain_collections_driven_by_rights_field() {
    let dir = scratch_dir("public-domain-collection");
    let _path = path_lock().await;
    install_stubs(&dir, &[]);
    let tdb = test_support::TestDb::connect_with_dir("public-domain-collection", &dir).await;
    let app = server::build_router(AppState::new(config_for(&dir), tdb.db().clone()));
    let mut client = Client::new(app.clone());

    // Register and login a user
    register(&mut client, "user@example.com", "user").await;

    // Create a work with CC0 license (public domain)
    let (work_id, _) = published_work_with_a_blob(&mut client, "Public Domain Work").await;
    // Create media_rights row and set to CC0
    insert_rights(&tdb, &work_id, "cc0").await;

    // Create a work with standard license (not public domain)
    let (work_id2, _) = published_work_with_a_blob(&mut client, "Copyrighted Work").await;
    insert_rights(&tdb, &work_id2, "all rights reserved").await;

    // List public domain collection - should contain the CC0 work
    let (status, body) = client
        .get("/api/v1/media-collections/kind/public_domain/media")
        .await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"].as_str().unwrap(), work_id);

    // Direct feed test: /api/v1/media-collections/kind/public_domain/media/feed
    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/media-collections/kind/public_domain/media/feed")
        .body(Body::empty())
        .expect("request");
    let response = app.clone().oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024)
        .await
        .expect("body");
    let feed_content = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(
        feed_content.contains("Public Domain Work"),
        "feed should contain CC0 work: {feed_content}"
    );
    assert!(
        !feed_content.contains("Copyrighted Work"),
        "feed should not contain standard work: {feed_content}"
    );

    println!("PASS: public_domain_collections_driven_by_rights_field");
}

/// Helper: insert media_rights row for a work
async fn insert_rights(tdb: &test_support::TestDb, work_id: &str, license: &str) {
    let cast = if tdb.is_postgres() { "::uuid" } else { "" };
    exec(
        tdb,
        &format!(
            "INSERT INTO media_rights (work_id, license, updated_at, version) VALUES (?{cast}, ?, '2026-09-01T00:00:00Z', 1)",
        ),
        &[work_id, license],
    )
    .await;
}
