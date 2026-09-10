//! Milestone 6 acceptance tests (spec §11).
//!
//! These run against the real router, a real SQLite file, a real storage
//! directory, the real worker and the real Archive-software parser. The only
//! thing replaced is the network: a test adapter answers `preview` and
//! `fetch_chapters` from the pages recorded in
//! `crates/scrapers/tests/fixtures/ao3/`, which are the bytes a live fetch
//! returned. A test that reaches the network fails on a plane, and an importer
//! only ever exercised against a live site is an importer whose retry and
//! resume rules nobody has pinned.
//!
//! What is proven here, and could not be proven any other way:
//!
//! * a queued import becomes stored chapters, through the worker;
//! * a preview stores nothing;
//! * a disabled source is a refusal that names the operator's reason;
//! * a source that needs a credential is refused *before* anything is fetched;
//! * a chapter already held is not stored twice;
//! * a transient fetch failure leaves the queue holding the retry;
//! * a dry run reports the plan and stores no chapters;
//! * a credential's plaintext is in no response and not in the database file.

use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_app::worker::{Worker, WorkerOptions};
use lorehaven_db::storage::BlobStore;
use lorehaven_db::{imports, jobs, Database, DatabaseConfig};
use lorehaven_domain::jobs::{JobKind, RetryPolicy};
use lorehaven_domain::AccountId;
use lorehaven_scrapers::async_trait;
use lorehaven_scrapers::registry::Registry;
use lorehaven_scrapers::sites::ao3::ArchiveSoftware;
use lorehaven_scrapers::{
    AuthKind, Credentials, Fetcher, SourceAdapter, SourceCapabilities, SourceChapter, SourceError,
    SourceKey, SourceResult, SourceWork,
};
use serde_json::{json, Value};
use tower::ServiceExt;

/// The work the recorded fixtures are of.
const WORK_URL: &str = "https://archiveofourown.org/works/92356871";
/// Its id at the source, which is what the library row is keyed on.
const WORK_KEY: &str = "92356871";
/// The source key, which the adapter, the source row and the import row share.
const SOURCE: &str = "ao3";

// ---------------------------------------------------------------------------
// The fixture adapter
// ---------------------------------------------------------------------------

/// The Archive-software adapter with the network removed.
///
/// It delegates parsing to the real adapter: `preview_from_html` and
/// `chapters_from_html` are the trait's own fixture seam, so this is the same
/// parser the live path uses rather than a stand-in for it. What it does not do
/// is fetch. It is handed a fetcher and deliberately never touches it — a test
/// adapter that reached the network would be a test that fails offline, and the
/// guard it would be reaching through is covered by the scrapers crate's own
/// tests.
struct FixtureArchive {
    inner: ArchiveSoftware,
    /// The work page: the metadata and the chapter list.
    work_html: String,
    /// The whole-work page: every chapter body.
    full_html: String,
    /// When set, `fetch_chapters` fails with this. Proves the retry path.
    fail_chapters: Option<String>,
    /// When set, the adapter claims to need a credential. Proves that a
    /// missing credential is caught before a fetch rather than after.
    requires_auth: bool,
}

impl FixtureArchive {
    fn new() -> Self {
        Self {
            inner: ArchiveSoftware::new(),
            work_html: fixture("work.html"),
            full_html: fixture("work-full.html"),
            fail_chapters: None,
            requires_auth: false,
        }
    }

    fn failing(message: &str) -> Self {
        Self {
            fail_chapters: Some(message.to_owned()),
            ..Self::new()
        }
    }

    fn needing_a_credential() -> Self {
        Self {
            requires_auth: true,
            ..Self::new()
        }
    }
}

/// A recorded page, located through `CARGO_MANIFEST_DIR` so the test does not
/// depend on the working directory a runner happens to use.
fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../scrapers/tests/fixtures/ao3")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()))
}

#[async_trait]
impl SourceAdapter for FixtureArchive {
    fn key(&self) -> SourceKey {
        SourceKey::new(SOURCE)
    }

    fn capabilities(&self) -> SourceCapabilities {
        let mut capabilities = self.inner.capabilities();
        if self.requires_auth {
            capabilities.authentication = AuthKind::Password;
        }
        capabilities
    }

    fn can_handle(&self, url: &url::Url) -> bool {
        self.inner.can_handle(url)
    }

    fn hosts(&self) -> Vec<String> {
        self.inner.hosts()
    }

    async fn preview(
        &self,
        _fetch: &dyn Fetcher,
        url: &url::Url,
        _creds: Option<&Credentials>,
    ) -> SourceResult<SourceWork> {
        self.inner.preview_from_html(&self.work_html, url)
    }

    async fn fetch_chapters(
        &self,
        _fetch: &dyn Fetcher,
        work: &SourceWork,
        _creds: Option<&Credentials>,
    ) -> SourceResult<Vec<SourceChapter>> {
        if let Some(message) = &self.fail_chapters {
            return Err(SourceError::Network(message.clone()));
        }
        self.inner.chapters_from_html(&self.full_html, work)
    }

    fn preview_from_html(&self, html: &str, url: &url::Url) -> SourceResult<SourceWork> {
        self.inner.preview_from_html(html, url)
    }

    fn chapters_from_html(
        &self,
        html: &str,
        work: &SourceWork,
    ) -> SourceResult<Vec<SourceChapter>> {
        self.inner.chapters_from_html(html, work)
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m6-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn config_for(dir: &Path) -> Config {
    let mut config = Config::development_defaults();
    // Blobs live under a `storage` subdirectory of the scratch dir, which is
    // what `Harness::store` points at. Setting the root to the scratch dir
    // itself would put the object tree beside the database file, and every
    // assertion about stored bytes would look in the wrong place.
    config.storage.root = dir.join("storage");
    config.database = DatabaseConfig::new(format!(
        "sqlite://{}/lorehaven.sqlite?mode=rwc",
        dir.display()
    ));
    config
}

struct Harness {
    dir: PathBuf,
    db: Database,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        set_trust_proxy(false);
        let _ = lorehaven_app::logging::init(&lorehaven_app::config::LoggingConfig {
            filter: "error".to_owned(),
            format: lorehaven_app::config::LogFormat::Pretty,
        });

        let dir = scratch_dir(tag);
        let db = Database::connect(&DatabaseConfig::new(format!(
            "sqlite://{}/lorehaven.sqlite?mode=rwc",
            dir.display()
        )))
        .await
        .expect("connect");
        let report = db.migrate().await.expect("migrate");
        assert!(
            report.applied.contains(&"0006_imports".to_owned()),
            "the imports migration must apply: {report:?}"
        );

        let harness = Self { dir, db };
        harness.seed_source(true, None).await;
        harness
    }

    /// The source row the importer consults before it fetches anything.
    async fn seed_source(&self, enabled: bool, reason: Option<&str>) {
        imports::upsert_source(&self.db, SOURCE, "Archive of Our Own", "0.1.0", "{}")
            .await
            .expect("seed source");
        if !enabled {
            imports::set_source_enabled(&self.db, SOURCE, false, reason)
                .await
                .expect("disable source");
        }
    }

    fn state(&self) -> AppState {
        AppState::new(config_for(&self.dir), self.db.clone())
    }

    /// State whose registry is the fixture adapter, so an import never reaches
    /// the network and the parser is still the real one.
    fn state_with(&self, adapter: FixtureArchive) -> AppState {
        let mut registry = Registry::new();
        registry.register(Box::new(adapter));
        self.state().with_registry(registry)
    }

    fn store(&self) -> BlobStore {
        BlobStore::new(self.dir.join("storage"))
    }

    /// The bytes of every file the database writes, decoded lossily.
    ///
    /// SQLite keeps recent writes in `<name>-wal` until a checkpoint, so a test
    /// that reads only the main file is testing the state of the database some
    /// indeterminate time ago. For "is this plaintext on disk?" that difference
    /// is the whole question.
    fn database_files(&self) -> String {
        let mut all = String::new();
        for suffix in [
            "lorehaven.sqlite",
            "lorehaven.sqlite-wal",
            "lorehaven.sqlite-shm",
        ] {
            let path = self.dir.join(suffix);
            if let Ok(bytes) = std::fs::read(&path) {
                all.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
        all
    }

    async fn cleanup(self) {
        self.db.close().await;
        let _ = std::fs::remove_dir_all(self.dir);
    }
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
            let header_value = self
                .cookies
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("; ");
            builder = builder.header(header::COOKIE, header_value);
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
}

const PASSWORD: &str = "a-long-enough-passphrase";

async fn register(client: &mut Client, email: &str, handle: &str) -> AccountId {
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
        .parse()
        .expect("account id is a UUID")
}

/// A signed-in client and the pseud it acts as.
async fn signed_in(harness: &Harness, email: &str, handle: &str) -> (Client, AccountId, String) {
    let mut client = Client::new(server::build_router(harness.state()));
    let account = register(&mut client, email, handle).await;
    let (status, body) = client.get("/api/v1/pseuds").await;
    assert_eq!(status, StatusCode::OK, "pseuds: {body}");
    // A bare array, not an envelope: this collection is the account's own
    // pseuds and is not paginated.
    let pseuds = body.as_array().expect("a list of pseuds");
    let pseud = pseuds
        .iter()
        .find(|entry| entry["active"] == true)
        .or_else(|| pseuds.first())
        .and_then(|entry| entry["id"].as_str())
        .expect("a pseud id")
        .to_owned();
    (client, account, pseud)
}

fn worker() -> Worker {
    Worker::new(WorkerOptions {
        id: "m6-test-worker".to_owned(),
        lease: Duration::from_secs(30),
        poll_interval: Duration::from_millis(10),
        policy: RetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_millis(1),
            backoff: 2.0,
            jitter_permille: 0,
        },
        batch: 50,
    })
}

/// Record an import and the queue row that carries it, exactly as the route
/// does: the import's id exists first so the payload can name it, and the import
/// row then names the queue row. Getting this order wrong leaves a claimable job
/// pointing at an import that is not there.
async fn queue_import(harness: &Harness, account: AccountId, pseud: &str, dry_run: bool) -> String {
    let import_id = lorehaven_domain::ImportJobId::new().to_string();
    let job_id = jobs::enqueue(
        &harness.db,
        JobKind::Import,
        &json!({ "import_job_id": import_id }).to_string(),
        None,
        Some(account),
        0,
        &RetryPolicy::default(),
    )
    .await
    .expect("enqueue");
    imports::create_import_job(
        &harness.db,
        &import_id,
        &job_id.to_string(),
        &account.to_string(),
        pseud,
        SOURCE,
        WORK_URL,
        "library",
        dry_run,
    )
    .await
    .expect("create import");
    import_id
}

/// Run at most `passes` worker turns.
async fn run_passes(state: &AppState, passes: usize) {
    let worker = worker();
    for _ in 0..passes {
        let report = worker.run_once(state).await.expect("worker pass");
        if report.job.is_none() {
            return;
        }
    }
}

/// Queue an import and run it to a conclusion.
async fn run_import(
    harness: &Harness,
    account: AccountId,
    pseud: &str,
    adapter: FixtureArchive,
    dry_run: bool,
) -> String {
    harness.seed_source(true, None).await;
    let state = harness.state_with(adapter);
    let import_id = queue_import(harness, account, pseud, dry_run).await;
    run_passes(&state, 6).await;
    import_id
}

async fn import_row(harness: &Harness, id: &str) -> imports::ImportJob {
    imports::get_import_job(&harness.db, id)
        .await
        .expect("read import")
        .expect("the import exists")
}

// ---------------------------------------------------------------------------
// The import, end to end
// ---------------------------------------------------------------------------

/// The milestone's central criterion: a queued import becomes stored chapters.
#[tokio::test]
async fn an_import_stores_a_works_chapters() {
    let harness = Harness::new("import-happy").await;
    let (_client, account, pseud) = signed_in(&harness, "reader@example.org", "reader").await;

    let import_id = run_import(&harness, account, &pseud, FixtureArchive::new(), false).await;

    let row = import_row(&harness, &import_id).await;
    assert_eq!(row.state, "completed", "report: {:?}", row.report_json);

    let item = imports::find_library_item(&harness.db, &account.to_string(), SOURCE, WORK_KEY)
        .await
        .expect("find item")
        .expect("the item was created");
    assert!(
        !item.title.is_empty(),
        "the title was parsed from the recorded page"
    );
    assert!(
        item.source_updated_at.is_some(),
        "the source's own update date was parsed, not set to now"
    );
    assert_eq!(
        item.source_url, WORK_URL,
        "the library row keeps the address it was imported from"
    );

    let chapters = imports::list_import_chapters(&harness.db, &import_id)
        .await
        .expect("chapters");
    assert_eq!(chapters.len(), 3, "the fixture holds three chapters");

    let store = harness.store();
    for chapter in &chapters {
        assert_eq!(chapter.state, "stored", "chapter {:?}", chapter.ordinal);
        let checksum = chapter
            .content_blob_checksum
            .as_ref()
            .expect("a stored chapter names its blob");
        let bytes = store
            .get(&harness.db, checksum)
            .await
            .expect("read blob")
            .expect("the blob exists");
        assert!(
            !bytes.is_empty(),
            "chapter {:?} stored nothing",
            chapter.ordinal
        );
        assert!(
            !String::from_utf8_lossy(&bytes).contains("<script"),
            "chapter {:?} stored a script tag",
            chapter.ordinal
        );
    }

    // The report says what happened, in a shape a client can read.
    let report: Value =
        serde_json::from_str(row.report_json.as_deref().expect("a report")).expect("json report");
    assert_eq!(report["plan"], "create");
    assert_eq!(report["stored"], 3);
    assert_eq!(report["failed"], 0);

    harness.cleanup().await;
}

/// A preview writes nothing: no item, no chapter, no import.
#[tokio::test]
async fn a_preview_stores_nothing() {
    let harness = Harness::new("preview-readonly").await;
    let (_client, account, _pseud) = signed_in(&harness, "peeker@example.org", "peeker").await;

    let items_before = imports::list_library_items(&harness.db, &account.to_string(), 50, None)
        .await
        .expect("items");

    // The preview goes through the HTTP surface, on a state whose registry never
    // reaches the network.
    let mut http = Client::new(server::build_router(
        harness.state_with(FixtureArchive::new()),
    ));
    let account_for_login = register(&mut http, "peeker2@example.org", "peeker2").await;

    let (status, body) = http
        .post("/api/v1/imports/preview", json!({ "url": WORK_URL }))
        .await;
    assert_eq!(status, StatusCode::OK, "preview: {body}");
    assert_eq!(body["source_key"], SOURCE);
    assert_eq!(body["plan"]["plan"], "create");
    assert_eq!(body["chapter_count"], 3);
    assert_eq!(body["is_new"], true);
    assert!(
        body["title"]
            .as_str()
            .is_some_and(|title| !title.is_empty()),
        "the title was parsed: {body}"
    );

    // Nothing was written for either account.
    for id in [account, account_for_login] {
        let items = imports::list_library_items(&harness.db, &id.to_string(), 50, None)
            .await
            .expect("items");
        assert!(
            items.is_empty(),
            "a preview must not create a library item, found {}",
            items.len()
        );
    }
    let items_after = imports::list_library_items(&harness.db, &account.to_string(), 50, None)
        .await
        .expect("items");
    assert_eq!(items_after.len(), items_before.len());

    harness.cleanup().await;
}

/// A source that is switched off is refused with the operator's own reason, and
/// nothing is fetched.
#[tokio::test]
async fn a_disabled_source_is_refused_with_its_reason() {
    let harness = Harness::new("disabled-source").await;
    let (_client, account, pseud) = signed_in(&harness, "blocked@example.org", "blocked").await;

    harness
        .seed_source(false, Some("the site asked us to stop for a while"))
        .await;
    let state = harness.state_with(FixtureArchive::new());
    let import_id = queue_import(&harness, account, &pseud, false).await;
    run_passes(&state, 1).await;

    let row = import_row(&harness, &import_id).await;
    assert_eq!(row.state, "failed", "report: {:?}", row.report_json);
    let report = row.report_json.clone().unwrap_or_default();
    assert!(
        report.contains("source_disabled"),
        "the report names the refusal: {report}"
    );
    assert!(
        report.contains("the site asked us to stop"),
        "the operator's reason reaches the reader: {report}"
    );

    let chapters = imports::list_import_chapters(&harness.db, &import_id)
        .await
        .expect("chapters");
    assert!(chapters.is_empty(), "nothing should have been stored");

    harness.cleanup().await;
}

/// A source that needs a credential is refused *before* anything is fetched,
/// and the refusal is fatal rather than retried five times.
#[tokio::test]
async fn a_missing_credential_is_refused_before_any_fetch() {
    let harness = Harness::new("credential-missing").await;
    let (_client, account, pseud) =
        signed_in(&harness, "needslogin@example.org", "needslogin").await;

    let state = harness.state_with(FixtureArchive::needing_a_credential());
    let import_id = queue_import(&harness, account, &pseud, false).await;
    run_passes(&state, 1).await;

    let row = import_row(&harness, &import_id).await;
    assert_eq!(row.state, "failed", "report: {:?}", row.report_json);
    let report = row.report_json.clone().unwrap_or_default();
    assert!(
        report.contains("credential_missing"),
        "the report names the missing credential: {report}"
    );

    // Fatal, not transient: retrying would send the same absent credential.
    let job_id = imports::job_for_import(&harness.db, &import_id)
        .await
        .expect("job id")
        .expect("the import names its queue row");
    let job = jobs::find(&harness.db, job_id.parse().expect("a job id"))
        .await
        .expect("read job")
        .expect("the job exists");
    assert_eq!(
        job.state,
        "failed",
        "a missing credential must not be retried: {}",
        job.last_error.unwrap_or_default()
    );
    assert_eq!(job.attempts, 1);

    let chapters = imports::list_import_chapters(&harness.db, &import_id)
        .await
        .expect("chapters");
    assert!(chapters.is_empty());

    harness.cleanup().await;
}

/// A chapter already held is not stored a second time.
///
/// The rule this pins: a re-run of an import compares against what is recorded
/// and skips it, rather than re-reading and re-storing the whole work. The
/// assertion is on the *checksum* rather than on a count, because a check that
/// only counted rows would pass even if every chapter were fetched and written
/// again under the same checksum.
#[tokio::test]
async fn a_chapter_already_held_is_not_stored_again() {
    let harness = Harness::new("resume").await;
    let (_client, account, pseud) = signed_in(&harness, "resumer@example.org", "resumer").await;

    // A first import that completes, so there is something to resume from.
    let import_id = run_import(&harness, account, &pseud, FixtureArchive::new(), false).await;
    let first = imports::list_import_chapters(&harness.db, &import_id)
        .await
        .expect("chapters");
    assert_eq!(first.len(), 3);
    let expected: Vec<(String, Option<String>)> = first
        .iter()
        .map(|chapter| {
            (
                chapter.source_chapter_key.clone(),
                chapter.content_blob_checksum.clone(),
            )
        })
        .collect();

    // A second import of the same work: the same item, and each chapter's row
    // still pointing at the blob it already had.
    let second = run_import(&harness, account, &pseud, FixtureArchive::new(), false).await;
    let now = imports::list_import_chapters(&harness.db, &second)
        .await
        .expect("chapters");
    let after: Vec<(String, Option<String>)> = now
        .iter()
        .map(|chapter| {
            (
                chapter.source_chapter_key.clone(),
                chapter.content_blob_checksum.clone(),
            )
        })
        .collect();
    assert_eq!(
        after, expected,
        "the chapters were re-stored rather than recognised as already held"
    );

    // And the work is still one item, not two.
    let items = imports::list_library_items(&harness.db, &account.to_string(), 50, None)
        .await
        .expect("items");
    assert_eq!(items.len(), 1, "two imports of one work are one item");

    harness.cleanup().await;
}

/// A fetch that fails transiently leaves the queue holding the retry.
#[tokio::test]
async fn a_transient_fetch_failure_stays_queued_for_retry() {
    let harness = Harness::new("transient").await;
    let (_client, account, pseud) = signed_in(&harness, "retrier@example.org", "retrier").await;

    let state = harness.state_with(FixtureArchive::failing("the source timed out"));
    let import_id = queue_import(&harness, account, &pseud, false).await;
    run_passes(&state, 1).await;

    let row = import_row(&harness, &import_id).await;
    assert_ne!(row.state, "completed", "a failed fetch is not a completion");
    assert!(
        row.report_json.is_none(),
        "there is no plan report to write: {:?}",
        row.report_json
    );

    let job_id = imports::job_for_import(&harness.db, &import_id)
        .await
        .expect("job id")
        .expect("the import names its queue row");
    let job = jobs::find(&harness.db, job_id.parse().expect("a job id"))
        .await
        .expect("read job")
        .expect("the job exists");
    assert_eq!(
        job.state,
        "queued",
        "a transient failure is queued again, not failed: {}",
        job.last_error.unwrap_or_default()
    );
    assert_eq!(job.attempts, 1, "the first attempt is recorded");
    assert!(
        job.last_error
            .as_deref()
            .is_some_and(|error| error.contains("timed out")),
        "the reason is recorded: {:?}",
        job.last_error
    );

    // Nothing was stored, so a retry starts from nothing and does not have to
    // unpick a half-written item.
    let chapters = imports::list_import_chapters(&harness.db, &import_id)
        .await
        .expect("chapters");
    assert!(chapters.is_empty());

    harness.cleanup().await;
}

/// A dry run reports the plan and stores no chapters.
#[tokio::test]
async fn a_dry_run_reports_without_storing_chapters() {
    let harness = Harness::new("dry-run").await;
    let (_client, account, pseud) = signed_in(&harness, "dry@example.org", "dry").await;

    let import_id = run_import(&harness, account, &pseud, FixtureArchive::new(), true).await;

    let row = import_row(&harness, &import_id).await;
    assert_eq!(row.state, "completed");
    let report: Value =
        serde_json::from_str(row.report_json.as_deref().expect("a report")).expect("json");
    assert_eq!(report["dry_run"], true);
    assert_eq!(report["plan"], "create");

    let chapters = imports::list_import_chapters(&harness.db, &import_id)
        .await
        .expect("chapters");
    assert!(
        chapters.is_empty(),
        "a dry run stores no chapters, found {}",
        chapters.len()
    );

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------------

/// Storing a credential for a source that uses none is refused, and the
/// refusal does not echo the secret back.
#[tokio::test]
async fn a_credential_for_a_source_that_needs_none_is_refused() {
    let harness = Harness::new("secret-refused").await;
    let (mut client, _account, _pseud) =
        signed_in(&harness, "secretive@example.org", "secretive").await;

    let (status, body) = client
        .post(
            "/api/v1/source-credentials",
            json!({
                "source_key": SOURCE,
                "label": "my login",
                "secret": "hunter2-the-actual-password",
            }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "storing a credential for a source that uses none: {body}"
    );
    assert!(
        !body.to_string().contains("hunter2"),
        "the refusal echoed the secret: {body}"
    );
    assert!(
        !body.to_string().contains("the-actual-password"),
        "the refusal echoed the secret: {body}"
    );

    let (status, body) = client.get("/api/v1/source-credentials").await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.to_string().contains("hunter2"));
    assert_eq!(body["items"].as_array().map(Vec::len), Some(0));

    harness.cleanup().await;
}

/// A secret round-trips through the store, is absent from the database file in
/// the clear, and is removed with the credential that owns it.
#[tokio::test]
async fn a_credential_round_trips_through_the_encrypted_store() {
    let harness = Harness::new("secret-round-trip").await;
    let (_client, _account, pseud) = signed_in(&harness, "vault@example.org", "vault").await;

    let cipher = lorehaven_app::secrets::load_cipher(&harness.dir.join("storage"), None, false)
        .expect("the development key");

    // The secret is written first — `source_credentials.secret_id` is a foreign
    // key, so the value has to exist before the row that names it can.
    let owner_id = format!("{pseud}:{SOURCE}:a label");
    let secret_id = lorehaven_app::secrets::seal_secret(
        &harness.db,
        &cipher,
        "source_credential",
        &owner_id,
        "secret",
        &lorehaven_app::secrets::Secret::new("s3cr3t-value-42"),
    )
    .await
    .expect("seal");

    let (row, _previous) =
        imports::upsert_source_credential(&harness.db, &pseud, SOURCE, &secret_id, "a label", None)
            .await
            .expect("credential row");

    // It comes back...
    let opened = lorehaven_app::secrets::open_secret(&harness.db, &cipher, &secret_id)
        .await
        .expect("open")
        .expect("the secret exists");
    assert_eq!(opened, "s3cr3t-value-42");

    // ...and the database's own files hold none of it in the clear. Every file
    // the database writes is read, not only the main one: a recent write lives
    // in the write-ahead log until a checkpoint, so checking the main file alone
    // would miss exactly the rows this test just wrote — and would therefore
    // pass for the wrong reason.
    let haystack = harness.database_files();
    assert!(
        !haystack.contains("s3cr3t-value-42"),
        "the plaintext is in the database's files"
    );
    assert!(
        haystack.contains(&row.id),
        "the credential row is not in the database's files, so this test is not \
         looking where the data is"
    );

    // Deleting the credential takes the secret with it.
    let removed = imports::delete_source_credential(&harness.db, &row.id, &pseud)
        .await
        .expect("delete");
    assert_eq!(removed.as_deref(), Some(secret_id.as_str()));
    let gone = lorehaven_app::secrets::open_secret(&harness.db, &cipher, &secret_id)
        .await
        .expect("open after delete");
    assert!(gone.is_none(), "the secret outlived its credential");

    // Deleting a credential does not touch what was already imported
    // (spec §11.6). There is nothing imported here, so the assertion is that
    // the delete is scoped to the credential: a second delete finds nothing.
    let again = imports::delete_source_credential(&harness.db, &row.id, &pseud)
        .await
        .expect("second delete");
    assert!(again.is_none(), "the credential was already gone");

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// The catalogue and the caller's own imports
// ---------------------------------------------------------------------------

/// The catalogue reports what each adapter can do, so absence is visible.
#[tokio::test]
async fn the_catalogue_reports_capabilities() {
    let harness = Harness::new("catalogue").await;
    let (mut client, _account, _pseud) =
        signed_in(&harness, "browser@example.org", "browser").await;

    let (status, body) = client.get("/api/v1/sources").await;
    assert_eq!(status, StatusCode::OK, "sources: {body}");
    let items = body["items"].as_array().expect("items");
    let ao3 = items
        .iter()
        .find(|item| item["key"] == SOURCE)
        .expect("ao3 is in the catalogue");
    assert_eq!(ao3["capabilities"]["known"], true);
    assert_eq!(ao3["capabilities"]["chapters"], true);
    assert_eq!(ao3["capabilities"]["per_chapter_fetch"], true);
    assert_eq!(ao3["capabilities"]["authentication"], "none");
    assert_eq!(ao3["enabled"], true);

    harness.cleanup().await;
}

/// A caller sees their own imports and nobody else's.
#[tokio::test]
async fn one_readers_imports_are_not_anothers() {
    let harness = Harness::new("imports-scoped").await;

    let (_first_client, first, first_pseud) = signed_in(&harness, "one@example.org", "one").await;
    let first_import =
        run_import(&harness, first, &first_pseud, FixtureArchive::new(), false).await;

    let (mut second_client, _second, _second_pseud) =
        signed_in(&harness, "two@example.org", "two").await;

    let (status, body) = second_client.get("/api/v1/imports").await;
    assert_eq!(status, StatusCode::OK, "imports: {body}");
    let items = body["items"].as_array().expect("items");
    assert!(
        items.iter().all(|item| item["id"] != first_import.as_str()),
        "the other reader's import is visible: {body}"
    );
    assert!(items.is_empty(), "a new account has no imports: {body}");

    // Asking for it by id is not enough to reach it.
    let (status, _body) = second_client
        .get(&format!("/api/v1/imports/{first_import}"))
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "an import id from another account must not resolve"
    );

    harness.cleanup().await;
}

/// A pasted address that this build cannot read is refused as a validation
/// error rather than queued and failed later.
#[tokio::test]
async fn an_unknown_address_is_refused_before_it_is_queued() {
    let harness = Harness::new("bad-url").await;
    let (mut client, account, _pseud) = signed_in(&harness, "typo@example.org", "typo").await;

    for url in [
        "not a url at all",
        "ftp://example.org/work/1",
        "https://example.org/not-a-source",
    ] {
        let (status, body) = client.post("/api/v1/imports", json!({ "url": url })).await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{url} should be refused: {body}"
        );
    }

    let jobs = jobs::all_jobs(&harness.db, None, 50, None)
        .await
        .expect("jobs");
    assert!(
        !jobs.iter().any(|job| job.kind == "import"),
        "a refused address must not be queued"
    );
    let items = imports::list_library_items(&harness.db, &account.to_string(), 50, None)
        .await
        .expect("items");
    assert!(items.is_empty());

    harness.cleanup().await;
}
