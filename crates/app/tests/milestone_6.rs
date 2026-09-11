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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_app::worker::{Worker, WorkerOptions};
use lorehaven_db::storage::BlobStore;
use lorehaven_db::{imports, jobs, revisions, Database, DatabaseConfig};
use lorehaven_domain::jobs::{JobKind, RetryPolicy};
use lorehaven_domain::AccountId;
use lorehaven_scrapers::async_trait;
use lorehaven_scrapers::registry::Registry;
use lorehaven_scrapers::sites::ao3::ArchiveSoftware;
use lorehaven_scrapers::{
    AuthKind, Credentials, Fetcher, SourceAdapter, SourceCapabilities, SourceChapter, SourceError,
    SourceKey, SourceResult, SourceWork, Wall,
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
    fail_chapters: Option<SourceError>,
    /// When set, the adapter claims to need a credential. Proves that a
    /// missing credential is caught before a fetch rather than after.
    requires_auth: bool,
    /// What the adapter says its source needs of a client. Proves that a wall
    /// this instance cannot pass is refused before anything is queued.
    wall: Wall,
    /// How many times each read entry point was entered.
    calls: Arc<Calls>,
}

/// What the adapter was asked to do, counted.
#[derive(Debug, Default)]
struct Calls {
    previews: AtomicUsize,
    bulk_fetches: AtomicUsize,
    /// Every ordinal passed to `fetch_chapter`, in order.
    single_fetches: std::sync::Mutex<Vec<u32>>,
}

impl Calls {
    fn previews(&self) -> usize {
        self.previews.load(Ordering::SeqCst)
    }

    fn bulk_fetches(&self) -> usize {
        self.bulk_fetches.load(Ordering::SeqCst)
    }

    fn single_fetches(&self) -> Vec<u32> {
        self.single_fetches
            .lock()
            .expect("the call log is not poisoned")
            .clone()
    }
}

impl FixtureArchive {
    fn new() -> Self {
        Self {
            inner: ArchiveSoftware::new(),
            work_html: fixture("work.html"),
            full_html: fixture("work-full.html"),
            fail_chapters: None,
            requires_auth: false,
            wall: Wall::None,
            calls: Arc::new(Calls::default()),
        }
    }

    /// An adapter whose source answers an interactive challenge, so a solver
    /// service is the only client that can read it.
    ///
    /// The wall is FimFiction's rather than a made-up one: measured on
    /// 2026-09-11, that host refused a plain client and a browser fingerprint
    /// alike and was read only through a solver, which is the pair of facts that
    /// makes the refusal right. A test adapter with an invented wall would pass
    /// whatever the code did with it.
    fn behind_a_solver_wall() -> Self {
        Self {
            wall: Wall::Solver,
            ..Self::new()
        }
    }

    /// A handle on the call log, taken before the adapter is moved into the
    /// registry.
    fn calls(&self) -> Arc<Calls> {
        Arc::clone(&self.calls)
    }

    /// An adapter whose chapter read fails with a network fault, which is the
    /// transient case: the queue's answer is to try again.
    fn failing(message: &str) -> Self {
        Self {
            fail_chapters: Some(SourceError::Network(message.to_owned())),
            ..Self::new()
        }
    }

    /// An adapter whose chapter read fails because the source will not serve the
    /// work at all. The error is carried rather than described, because the
    /// queue's decision is made from the *category* and a test that could only
    /// inject one category could not tell the categories apart.
    fn refusing(error: SourceError) -> Self {
        Self {
            fail_chapters: Some(error),
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

    fn display_name(&self) -> &'static str {
        "Archive of Our Own"
    }

    fn capabilities(&self) -> SourceCapabilities {
        let mut capabilities = self.inner.capabilities();
        if self.requires_auth {
            capabilities.authentication = AuthKind::Password;
        }
        capabilities
    }

    fn wall(&self) -> Wall {
        self.wall
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
        self.calls.previews.fetch_add(1, Ordering::SeqCst);
        self.inner.preview_from_html(&self.work_html, url)
    }

    async fn fetch_chapters(
        &self,
        _fetch: &dyn Fetcher,
        work: &SourceWork,
        _creds: Option<&Credentials>,
    ) -> SourceResult<Vec<SourceChapter>> {
        self.calls.bulk_fetches.fetch_add(1, Ordering::SeqCst);
        if let Some(error) = &self.fail_chapters {
            return Err(error.clone());
        }
        self.inner.chapters_from_html(&self.full_html, work)
    }

    /// One chapter, read from the recorded whole-work page.
    ///
    /// This is what a retry uses, so it is also where a chapter that the source
    /// serves badly is simulated: the ordinal is counted whatever happens, so a
    /// test can assert that a retry asked for the failed chapter and no other.
    async fn fetch_chapter(
        &self,
        _fetch: &dyn Fetcher,
        work: &SourceWork,
        ordinal: u32,
        _creds: Option<&Credentials>,
    ) -> SourceResult<SourceChapter> {
        self.calls
            .single_fetches
            .lock()
            .expect("the call log is not poisoned")
            .push(ordinal);

        let chapters = self.inner.chapters_from_html(&self.full_html, work)?;
        chapters
            .into_iter()
            .find(|chapter| chapter.ordinal == ordinal)
            .ok_or(SourceError::NotFound)
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

    /// A harness with migrations applied and no source rows at all.
    ///
    /// What a real instance looks like the moment it is set up: the `sources`
    /// table is instance state and nothing writes to it for a source nobody has
    /// used. Tests that need a row call [`Harness::new`]; tests about what an
    /// instance reports *before* any row exists need this, because the seeding in
    /// the other constructor is exactly the state under test.
    async fn empty(tag: &str) -> Self {
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

        Self { dir, db }
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
        self.state_with_config(adapter, config_for(&self.dir))
    }

    /// The same, on a config the test has adjusted — which is how an instance
    /// with a solver configured is told apart from one without.
    fn state_with_config(&self, adapter: FixtureArchive, config: Config) -> AppState {
        let mut registry = Registry::new();
        registry.register(Box::new(adapter));
        AppState::new(config, self.db.clone()).with_registry(registry)
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

    async fn delete(&mut self, uri: &str) -> (StatusCode, Value) {
        self.request("DELETE", uri, None).await
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

/// A source this instance has no way of reaching is refused before anything is
/// queued, and the refusal names the fix.
///
/// The alternative — and what this replaced — was discovering it one page at a
/// time: every fetch comes back a challenge, and the reader waits for an import
/// that was never able to work. On a self-hosted instance the reader is also the
/// operator, so the fix is theirs to apply and the message has to carry it.
#[tokio::test]
async fn a_source_this_instance_cannot_reach_is_refused_before_it_is_queued() {
    let harness = Harness::new("wall-no-solver").await;
    let adapter = FixtureArchive::behind_a_solver_wall();
    let calls = adapter.calls();
    let mut http = Client::new(server::build_router(harness.state_with(adapter)));
    let account = register(&mut http, "walled@example.org", "walled").await;
    let jobs_before = jobs::counts_by_state(&harness.db)
        .await
        .expect("job counts");

    let (status, body) = http
        .post("/api/v1/imports/preview", json!({ "url": WORK_URL }))
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_GATEWAY,
        "a source with no way to reach it is not previewable: {body}"
    );
    let message = body.to_string();
    assert!(
        message.contains("imports.solver_url"),
        "the refusal names the setting that fixes it: {message}"
    );
    assert!(
        message.contains("Byparr") || message.contains("FlareSolverr"),
        "and a service that speaks the protocol: {message}"
    );
    assert_eq!(
        calls.previews(),
        0,
        "no request may be made to a source this instance cannot read"
    );

    // The start route refuses too, and does so *before* a job exists: a queued
    // job is a promise that the work will be attempted.
    let (status, body) = http
        .post("/api/v1/imports", json!({ "url": WORK_URL }))
        .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "start: {body}");

    let jobs_after = jobs::counts_by_state(&harness.db)
        .await
        .expect("job counts");
    assert_eq!(
        jobs_before, jobs_after,
        "a refused import must not leave a job behind"
    );
    let refused = imports::list_import_jobs(&harness.db, &account.to_string(), None, 50, None)
        .await
        .expect("imports");
    assert!(
        refused.is_empty(),
        "no import row should exist: {refused:?}"
    );

    harness.cleanup().await;
}

/// The same source is readable as soon as a solver is configured.
///
/// The pair matters: a test that only asserted the refusal would pass just as
/// well if the adapter could never be used at all.
#[tokio::test]
async fn the_same_source_is_readable_once_a_solver_is_configured() {
    let harness = Harness::new("wall-with-solver").await;
    let mut config = config_for(&harness.dir);
    config.imports.solver_url = Some("http://127.0.0.1:8191".to_owned());

    let adapter = FixtureArchive::behind_a_solver_wall();
    let calls = adapter.calls();
    let mut http = Client::new(server::build_router(
        harness.state_with_config(adapter, config),
    ));
    register(&mut http, "hassolver@example.org", "hassolver").await;

    let (status, body) = http
        .post("/api/v1/imports/preview", json!({ "url": WORK_URL }))
        .await;
    assert_eq!(status, StatusCode::OK, "preview with a solver: {body}");
    assert_eq!(body["chapter_count"], 3);
    assert_eq!(
        calls.previews(),
        1,
        "the adapter is what answers the preview"
    );

    harness.cleanup().await;
}

/// The catalogue says which sources need a solver, so the requirement is
/// visible before a reader pastes a URL rather than after (spec §11.1).
#[tokio::test]
async fn the_catalogue_says_which_sources_need_a_solver() {
    let harness = Harness::new("wall-catalogue").await;
    let mut http = Client::new(server::build_router(
        harness.state_with(FixtureArchive::behind_a_solver_wall()),
    ));
    register(&mut http, "listing@example.org", "listing").await;

    let (status, body) = http.get("/api/v1/imports/sources").await;
    assert_eq!(status, StatusCode::OK, "sources: {body}");
    let entry = body["items"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["key"] == SOURCE))
        .expect("the source is in the catalogue");
    assert_eq!(entry["capabilities"]["wall"], "solver");

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

/// A source that holds the work fails the job rather than retrying it.
///
/// The mirror of the transient case above, and the reason `SourceError` has
/// categories rather than one variant: a moderation hold, a work withdrawn by its
/// author and a takedown in progress are all states the *source* is in. Retrying
/// asks the same question and receives the same answer, so a retry budget spent
/// on one is a reader waiting for an import that cannot arrive. What must hold is
/// that the job ends `failed` on its first attempt, with the source's own words
/// recorded, and that nothing was stored.
#[tokio::test]
async fn a_source_that_withholds_a_work_fails_the_job_instead_of_retrying() {
    let harness = Harness::new("withheld").await;
    let (_client, account, pseud) = signed_in(&harness, "withheld@example.org", "withheld").await;

    let state = harness.state_with(FixtureArchive::refusing(SourceError::Withheld(
        "this story has not been validated by its administrators".to_owned(),
    )));
    let import_id = queue_import(&harness, account, &pseud, false).await;
    run_passes(&state, 1).await;

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
        "a hold is terminal, not something to retry: {}",
        job.last_error.unwrap_or_default()
    );
    assert_eq!(job.attempts, 1, "one attempt, and no second");
    assert!(
        job.last_error.as_deref().is_some_and(
            |error| error.contains("will not serve it") && error.contains("not been validated")
        ),
        "the source's own words are recorded: {:?}",
        job.last_error
    );

    let row = import_row(&harness, &import_id).await;
    assert_ne!(row.state, "completed");
    let chapters = imports::list_import_chapters(&harness.db, &import_id)
        .await
        .expect("chapters");
    assert!(chapters.is_empty(), "nothing was stored");

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

    let (status, body) = client.get("/api/v1/imports/sources").await;
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
    // A source that serves a plain request says so, rather than leaving the
    // field absent: "needs nothing" and "the build forgot to say" must not look
    // the same to an operator reading the catalogue.
    assert_eq!(ao3["capabilities"]["wall"], "none");
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

/// The adapter is not called at all when the source is switched off.
///
/// "Nothing was stored" is not the same claim: a source that is paused must cost
/// no requests, and the only way to know is to count them.
#[tokio::test]
async fn the_adapter_is_not_called_when_the_source_is_disabled() {
    let harness = Harness::new("disabled-calls").await;
    let (_client, account, pseud) = signed_in(&harness, "quiet@example.org", "quiet").await;

    harness
        .seed_source(false, Some("paused by an operator"))
        .await;
    let adapter = FixtureArchive::new();
    let calls = adapter.calls();
    let state = harness.state_with(adapter);
    let import_id = queue_import(&harness, account, &pseud, false).await;
    run_passes(&state, 2).await;

    assert_eq!(calls.previews(), 0, "the preview must not have run");
    assert_eq!(calls.bulk_fetches(), 0, "no chapter fetch must have run");
    assert!(
        calls.single_fetches().is_empty(),
        "no single-chapter fetch must have run"
    );
    assert_eq!(import_row(&harness, &import_id).await.state, "failed");

    harness.cleanup().await;
}

/// A retry re-reads the chapters that failed and nothing else.
///
/// The record is seeded the way an abandoned attempt would leave it — one
/// chapter failed, the rest already stored — because that is the state the rule
/// is about. The assertion is on which ordinals the adapter was asked for: a
/// retry that re-read the whole work would still end with every chapter stored,
/// so counting rows would prove nothing.
#[tokio::test]
async fn a_failed_chapter_is_retried_without_refetching_the_rest() {
    let harness = Harness::new("retry-failed-chapters").await;
    let (_client, account, pseud) = signed_in(&harness, "resume2@example.org", "resume2").await;

    // A completed import, to learn the source's own chapter keys and the
    // checksums the good chapters ended up with.
    let done = run_import(&harness, account, &pseud, FixtureArchive::new(), false).await;
    let stored = imports::list_import_chapters(&harness.db, &done)
        .await
        .expect("chapters");
    assert_eq!(stored.len(), 3);
    let keys: Vec<String> = stored
        .iter()
        .map(|chapter| chapter.source_chapter_key.clone())
        .collect();
    let checksums: HashMap<String, Option<String>> = stored
        .iter()
        .map(|chapter| {
            (
                chapter.source_chapter_key.clone(),
                chapter.content_blob_checksum.clone(),
            )
        })
        .collect();

    // A second import of the same work, left the way an attempt that lost one
    // chapter would leave it: two stored, the middle one failed.
    let retry_id = lorehaven_domain::ImportJobId::new().to_string();
    let job_id = jobs::enqueue(
        &harness.db,
        JobKind::Import,
        &json!({ "import_job_id": retry_id }).to_string(),
        None,
        Some(account),
        0,
        &RetryPolicy::default(),
    )
    .await
    .expect("enqueue");
    imports::create_import_job(
        &harness.db,
        &retry_id,
        &job_id.to_string(),
        &account.to_string(),
        &pseud,
        SOURCE,
        WORK_URL,
        "library",
        false,
    )
    .await
    .expect("create import");
    let item = imports::find_library_item(&harness.db, &account.to_string(), SOURCE, WORK_KEY)
        .await
        .expect("find")
        .expect("the item exists");
    for (index, key) in keys.iter().enumerate() {
        let (state, checksum) = if index == 1 {
            ("failed".to_owned(), None)
        } else {
            ("stored".to_owned(), checksums.get(key).cloned().flatten())
        };
        imports::upsert_import_chapter(
            &harness.db,
            &retry_id,
            Some(&item.id),
            &imports::ImportChapterInput {
                source_chapter_key: key.clone(),
                ordinal: i64::try_from(index + 1).expect("ordinal"),
                title: String::new(),
                state,
                content_blob_checksum: checksum,
                note: None,
            },
        )
        .await
        .expect("seed chapter");
    }

    // The retry.
    let adapter = FixtureArchive::new();
    let calls = adapter.calls();
    let state = harness.state_with(adapter);
    run_passes(&state, 2).await;

    assert_eq!(
        calls.single_fetches(),
        vec![2],
        "the retry asked for the failed chapter and nothing else"
    );
    assert_eq!(
        calls.bulk_fetches(),
        0,
        "the retry must not re-read the whole work"
    );

    let after = imports::list_import_chapters(&harness.db, &retry_id)
        .await
        .expect("chapters");
    assert_eq!(after.len(), 3);
    for chapter in &after {
        assert_eq!(chapter.state, "stored", "chapter {}", chapter.ordinal);
        assert_eq!(
            chapter.content_blob_checksum,
            checksums
                .get(&chapter.source_chapter_key)
                .cloned()
                .flatten(),
            "chapter {} was re-stored rather than left alone",
            chapter.ordinal
        );
    }

    harness.cleanup().await;
}

/// An import is queued and answered at once; the request does not do the work.
#[tokio::test]
async fn an_import_is_queued_and_does_not_block_the_request() {
    let harness = Harness::new("queued").await;
    let (mut client, account, _pseud) = signed_in(&harness, "queuer@example.org", "queuer").await;

    let (status, body) = client
        .post(
            "/api/v1/imports",
            json!({ "url": WORK_URL, "destination": "library" }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "start import: {body}");
    let import_id = body["import_id"].as_str().expect("an import id").to_owned();
    let job_id = body["job_id"].as_str().expect("a job id");
    assert_eq!(body["state"], "queued");

    // The queue holds it, and the request did not claim to have done anything.
    let job = jobs::find(&harness.db, job_id.parse().expect("a job id"))
        .await
        .expect("read job")
        .expect("the job exists");
    assert_eq!(job.kind, JobKind::Import.as_str());
    // The payload names the import and nothing else: no credential, no URL.
    assert_eq!(
        job.payload,
        json!({ "import_job_id": import_id }).to_string()
    );
    assert!(
        !job.payload.contains("archiveofourown"),
        "the URL is not in the payload"
    );

    // The import row is the queue row's partner.
    let row = import_row(&harness, &import_id).await;
    assert_eq!(row.state, "queued");
    assert_eq!(row.destination_type, "library");
    assert_eq!(
        imports::job_for_import(&harness.db, &import_id)
            .await
            .expect("job id")
            .as_deref(),
        Some(job_id)
    );

    // And nothing has been fetched or stored yet.
    let chapters = imports::list_import_chapters(&harness.db, &import_id)
        .await
        .expect("chapters");
    assert!(chapters.is_empty(), "the request must not fetch chapters");
    let items = imports::list_library_items(&harness.db, &account.to_string(), 50, None)
        .await
        .expect("items");
    assert!(items.is_empty(), "the request must not create the item");

    harness.cleanup().await;
}

/// An expired credential is reported before the import starts, and is not
/// retried: the same expired credential would fail the same way.
#[tokio::test]
async fn an_expired_credential_is_reported_before_the_import_starts() {
    let harness = Harness::new("credential-expired").await;
    let (_client, account, pseud) = signed_in(&harness, "expired@example.org", "expired").await;

    // A credential whose expiry has passed, seeded directly: this is the state
    // the sweep would leave behind, not something a request can create.
    //
    // Its secret row has to be real, because `source_credentials.secret_id` is
    // a foreign key — which is the property that stops a credential existing
    // without a value to authenticate with.
    let cipher = lorehaven_app::secrets::load_cipher(&harness.dir.join("storage"), None, false)
        .expect("the development key");
    let secret_id = lorehaven_app::secrets::seal_secret(
        &harness.db,
        &cipher,
        "source_credential",
        &format!("{pseud}:{SOURCE}:an old login"),
        "secret",
        &lorehaven_app::secrets::Secret::new("a-stale-password"),
    )
    .await
    .expect("seed secret");

    let (row, _previous) = imports::upsert_source_credential(
        &harness.db,
        &pseud,
        SOURCE,
        &secret_id,
        "an old login",
        Some("2001-01-01T00:00:00Z"),
    )
    .await
    .expect("seed credential");
    assert_eq!(row.status, "active", "it was stored as active");

    let adapter = FixtureArchive::needing_a_credential();
    let calls = adapter.calls();
    let state = harness.state_with(adapter);
    let import_id = queue_import(&harness, account, &pseud, false).await;
    run_passes(&state, 1).await;

    let row = import_row(&harness, &import_id).await;
    assert_eq!(row.state, "failed", "report: {:?}", row.report_json);
    let report = row.report_json.clone().unwrap_or_default();
    assert!(
        report.contains("credential_expired"),
        "the report names the expiry: {report}"
    );
    assert!(
        report.contains("2001-01-01"),
        "the report says when it expired: {report}"
    );
    assert_eq!(
        calls.previews(),
        0,
        "nothing may be fetched with a dead credential"
    );

    // And the credential itself is marked, so the reader can see why.
    let listed = imports::list_source_credentials(&harness.db, &pseud, Some(SOURCE))
        .await
        .expect("credentials");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].status, "expired");

    harness.cleanup().await;
}

/// The retry route queues an attempt at the chapters that failed, and refuses
/// when there are none.
#[tokio::test]
async fn the_retry_route_queues_only_when_something_failed() {
    let harness = Harness::new("retry-route").await;
    let (mut client, account, pseud) =
        signed_in(&harness, "retryroute@example.org", "retryroute").await;

    let import_id = run_import(&harness, account, &pseud, FixtureArchive::new(), false).await;

    // Nothing failed, so there is nothing to retry.
    let (status, body) = client
        .post(
            &format!("/api/v1/imports/{import_id}/retry-failed-chapters"),
            json!({}),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a retry with nothing failed: {body}"
    );

    // Now record a failure, as an abandoned attempt would.
    let chapters = imports::list_import_chapters(&harness.db, &import_id)
        .await
        .expect("chapters");
    let second = chapters
        .iter()
        .find(|chapter| chapter.ordinal == 2)
        .expect("chapter two");
    imports::upsert_import_chapter(
        &harness.db,
        &import_id,
        None,
        &imports::ImportChapterInput {
            source_chapter_key: second.source_chapter_key.clone(),
            ordinal: 2,
            title: second.title.clone(),
            state: "failed".to_owned(),
            content_blob_checksum: None,
            note: Some("the source dropped it".to_owned()),
        },
    )
    .await
    .expect("mark failed");

    let (status, body) = client
        .post(
            &format!("/api/v1/imports/{import_id}/retry-failed-chapters"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "retry: {body}");
    assert_eq!(body["failed_chapters"], 1);
    assert_eq!(import_row(&harness, &import_id).await.state, "queued");

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Runtime source health (spec §11.8)
//
// The catalogue's health column is a promise to the reader: a source that has
// stopped working should not be offered, and an import into one should not be
// queued to fail. These tests drive the sweep the worker runs after every
// import, and the refusal the routes apply.
// ---------------------------------------------------------------------------

/// Record a finished import for the source, in the state given.
///
/// It goes through the same two rows the route creates, because the sweep reads
/// the import history and a test that fabricated history the routes cannot
/// produce would prove nothing about the sweep.
async fn finished_import(
    harness: &Harness,
    account: AccountId,
    pseud: &str,
    state: &str,
) -> String {
    let id = queue_import(harness, account, pseud, false).await;
    imports::set_import_state(&harness.db, &id, state, None, None)
        .await
        .expect("finish the import");
    id
}

async fn source_health(harness: &Harness) -> String {
    imports::find_source(&harness.db, SOURCE)
        .await
        .expect("find the source")
        .expect("the source has a row")
        .health
}

/// Three failures with no success is `unavailable`, and an import into it is
/// refused at the API rather than queued and failed.
#[tokio::test]
async fn repeated_failures_make_a_source_unavailable_and_imports_are_refused() {
    let harness = Harness::new("health-unavailable").await;
    let (_client, account, pseud) = signed_in(&harness, "unlucky@example.org", "unlucky").await;

    for _ in 0..imports::FAILURES_TO_UNAVAILABLE {
        finished_import(&harness, account, &pseud, "failed").await;
    }

    let changes = imports::recompute_source_health(&harness.db, imports::HEALTH_WINDOW_DAYS)
        .await
        .expect("sweep");
    let change = changes
        .iter()
        .find(|change| change.key == SOURCE)
        .expect("the source was considered");
    assert_eq!(change.previous, "unknown");
    assert_eq!(change.current, "unavailable");
    assert_eq!(change.failed, imports::FAILURES_TO_UNAVAILABLE);
    assert_eq!(
        change.completed, 0,
        "no import of this source has succeeded"
    );
    assert_eq!(source_health(&harness).await, "unavailable");

    // The refusal is a refusal, not a queued job: `preview` is where a reader
    // would have found out anyway, and the answer arrives without a request to
    // a source the catalogue says is not working.
    let mut http = Client::new(server::build_router(
        harness.state_with(FixtureArchive::new()),
    ));
    register(&mut http, "unlucky2@example.org", "unlucky2").await;
    let (status, body) = http
        .post("/api/v1/imports/preview", json!({ "url": WORK_URL }))
        .await;
    assert_ne!(
        status,
        StatusCode::OK,
        "an unavailable source was previewed: {body}"
    );
    assert!(
        body.to_string().contains("unavailable") || body.to_string().contains("failed"),
        "the refusal says why: {body}"
    );

    harness.cleanup().await;
}

/// A source that works and sometimes does not is `degraded`, not unavailable.
///
/// The distinction is the reader's: unavailable means "do not bother today",
/// and a source with recent successes does not deserve that.
#[tokio::test]
async fn one_success_among_failures_is_degraded_not_unavailable() {
    let harness = Harness::new("health-degraded").await;
    let (_client, account, pseud) = signed_in(&harness, "mixed@example.org", "mixed").await;

    finished_import(&harness, account, &pseud, "completed").await;
    for _ in 0..4 {
        finished_import(&harness, account, &pseud, "failed").await;
    }

    imports::recompute_source_health(&harness.db, imports::HEALTH_WINDOW_DAYS)
        .await
        .expect("sweep");
    assert_eq!(
        source_health(&harness).await,
        "degraded",
        "four failures and a success is a source that mostly works"
    );

    harness.cleanup().await;
}

/// A cancellation says something about the reader, not about the source.
#[tokio::test]
async fn a_cancelled_import_is_neither_a_success_nor_a_failure() {
    let harness = Harness::new("health-cancelled").await;
    let (_client, account, pseud) = signed_in(&harness, "fickle@example.org", "fickle").await;

    for _ in 0..5 {
        finished_import(&harness, account, &pseud, "cancelled").await;
    }

    let changes = imports::recompute_source_health(&harness.db, imports::HEALTH_WINDOW_DAYS)
        .await
        .expect("sweep");
    assert!(
        changes.iter().all(|change| change.key != SOURCE),
        "five cancellations are not evidence about the source: {changes:?}"
    );
    assert_eq!(source_health(&harness).await, "unknown");

    harness.cleanup().await;
}

/// A source nobody has tried keeps whatever it had. Silence is not evidence.
#[tokio::test]
async fn a_source_with_no_finished_imports_is_left_alone() {
    let harness = Harness::new("health-untried").await;

    let changes = imports::recompute_source_health(&harness.db, imports::HEALTH_WINDOW_DAYS)
        .await
        .expect("sweep");
    assert!(changes.is_empty(), "nothing to report: {changes:?}");
    assert_eq!(source_health(&harness).await, "unknown");

    harness.cleanup().await;
}

/// A pause is an operator's decision, and the sweep is not allowed to undo it.
///
/// This is the test that matters most in this group: a sweep that could clear a
/// pause would silently re-enable a source somebody switched off on purpose,
/// and the operator's own reason would still be sitting on the row looking
/// current.
#[tokio::test]
async fn a_sweep_never_clears_an_operators_pause() {
    let harness = Harness::new("health-paused").await;
    let (_client, account, pseud) = signed_in(&harness, "paused@example.org", "paused").await;

    imports::set_source_health(&harness.db, SOURCE, "paused")
        .await
        .expect("pause the source");
    for _ in 0..5 {
        finished_import(&harness, account, &pseud, "completed").await;
    }

    let changes = imports::recompute_source_health(&harness.db, imports::HEALTH_WINDOW_DAYS)
        .await
        .expect("sweep");
    assert!(
        changes.iter().all(|change| change.key != SOURCE),
        "a paused source is not the sweep's to change: {changes:?}"
    );
    assert_eq!(
        source_health(&harness).await,
        "paused",
        "five successes must not un-pause a source an operator switched off"
    );

    harness.cleanup().await;
}

/// The worker sweeps after it runs an import, so health keeps itself current
/// without anybody asking.
#[tokio::test]
async fn running_an_import_updates_the_sources_health() {
    let harness = Harness::new("health-after-run").await;
    let (_client, account, pseud) = signed_in(&harness, "runner@example.org", "runner").await;

    let import_id = run_import(&harness, account, &pseud, FixtureArchive::new(), false).await;
    assert_eq!(import_row(&harness, &import_id).await.state, "completed");
    assert_eq!(
        source_health(&harness).await,
        "healthy",
        "a successful import is what makes a source healthy"
    );

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// The revision cache (spec §10.4)
//
// The cache's own behaviour is proven in `revision_cache.rs`, against a
// scripted fetcher. What is proven here is the part that needs the real
// server: that the surface exists, and that it is an operator's and nobody
// else's.
// ---------------------------------------------------------------------------

/// A client for an operator, built over an existing account.
///
/// A separate state rather than a flag on the harness, because being an
/// operator is a property of the *configuration* — which account
/// `config.administration.operator_account_id` names — and not of the session.
/// Building it here is what proves the gate reads the configuration rather than
/// something the caller passed in.
async fn operator_client(harness: &Harness, account: AccountId, email: &str) -> Client {
    let mut config = config_for(&harness.dir);
    config.administration.operator_account_id = Some(account);
    let mut client = Client::new(server::build_router(AppState::new(
        config,
        harness.db.clone(),
    )));
    let (status, body) = client
        .post(
            "/api/v1/auth/login",
            json!({ "email": email, "password": PASSWORD }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "operator login: {body}");
    client
}

/// The cache's surface is an operator's, and its absence is a 404 to everybody
/// else.
///
/// `404` rather than `403`: confirming that an operator surface exists is itself
/// a disclosure, which is the rule `/admin/jobs` already follows.
#[tokio::test]
async fn the_revision_cache_surface_belongs_to_the_operator() {
    let harness = Harness::new("revisions-route").await;
    let (mut reader, account, _pseud) = signed_in(&harness, "curious@example.org", "curious").await;

    // A reader is told the endpoint does not exist.
    let (status, body) = reader.get("/api/v1/admin/sources/revisions").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (status, body) = reader
        .post("/api/v1/admin/sources/revisions/purge", json!({}))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    // The operator sees what the cache holds.
    let mut operator = operator_client(&harness, account, "curious@example.org").await;
    let (status, body) = operator.get("/api/v1/admin/sources/revisions").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["entries"], 0, "nothing has been cached yet: {body}");
    assert!(
        body["ttl_seconds"].as_i64().is_some_and(|ttl| ttl > 0),
        "the operator can see how long entries last: {body}"
    );

    harness.cleanup().await;
}

/// An operator can empty the cache, and doing so leaves the stored bytes alone.
///
/// The bytes are the point of the test. `content_blobs` is shared with the
/// snapshots a reader is reading, so a cache that deleted its own blobs would be
/// a cache that could delete somebody's chapter.
#[tokio::test]
async fn clearing_the_revision_cache_does_not_delete_stored_bytes() {
    let harness = Harness::new("revisions-clear").await;
    let (_reader, account, _pseud) = signed_in(&harness, "keeper@example.org", "keeper").await;

    // A chapter's bytes, stored the way an import stores them.
    let store = harness.store();
    let (checksum, _key) = store
        .put(&harness.db, b"<html>a chapter</html>", "text/html")
        .await
        .expect("store the snapshot");

    // And a cache entry pointing at the same bytes, as a real read would leave.
    let entry = revisions::RevisionEntry {
        checksum: checksum.clone(),
        etag: Some("\"v1\"".to_owned()),
        last_modified: None,
        expires_at: lorehaven_db::identity::in_seconds(3600),
    };
    revisions::upsert(
        &harness.db,
        &revisions::RevisionKey {
            source_key: SOURCE,
            revision_key: WORK_URL,
            adapter_version: "0.1.0",
            security_scope: "public",
        },
        &entry,
    )
    .await
    .expect("record the revision");
    assert_eq!(revisions::count(&harness.db).await.expect("count"), 1);

    let mut operator = operator_client(&harness, account, "keeper@example.org").await;
    let (status, body) = operator.delete("/api/v1/admin/sources/revisions").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["cleared"], 1, "{body}");
    assert_eq!(revisions::count(&harness.db).await.expect("count"), 0);

    // The bytes are still there. A cache is an optimisation; losing it costs
    // requests, and losing a snapshot costs a reader their chapter.
    assert!(
        store
            .get(&harness.db, &checksum)
            .await
            .expect("read the blob")
            .is_some(),
        "clearing the revision cache must not delete content_blobs"
    );

    harness.cleanup().await;
}

/// The catalogue lists what the build can read, even before anything has been
/// imported and before any `sources` row exists.
///
/// This is the bug the browser journey found: `GET /imports/sources` used to be
/// driven by the `sources` table, which holds *instance* state — enabled, health,
/// last check — and which nothing creates for a source nobody has used. A fresh
/// instance with three working adapters therefore answered `{"items": []}`, and
/// the import page's own "What this instance can read" section said nothing while
/// a preview of the same URL would have worked. The page was wrong about the
/// instance in the direction that stops a reader trying.
///
/// The harness deliberately starts from a database with no `sources` rows at all,
/// which is what a real instance looks like before its first import.
#[tokio::test]
async fn the_catalogue_lists_the_builds_sources_before_any_row_exists() {
    let harness = Harness::empty("catalogue-fresh").await;
    assert!(
        imports::list_sources(&harness.db)
            .await
            .expect("list sources")
            .is_empty(),
        "the fixture is only meaningful with no rows"
    );

    let (mut client, _account, _pseud) = signed_in(&harness, "fresh@example.org", "fresh").await;
    let (status, body) = client.get("/api/v1/imports/sources").await;
    assert_eq!(status, StatusCode::OK, "sources: {body}");

    let items = body["items"].as_array().expect("items");
    assert!(
        !items.is_empty(),
        "a build with adapters must not report an empty catalogue: {body}"
    );

    // The one the fixture registry carries, named as a reader should see it and
    // not as its key.
    let source = items
        .iter()
        .find(|item| item["key"] == SOURCE)
        .expect("the fixture adapter is in the catalogue");
    assert_eq!(source["display_name"], "Archive of Our Own");
    assert_eq!(
        source["enabled"], true,
        "no row means nobody has switched it off"
    );
    assert_eq!(
        source["health"], "unknown",
        "nothing has been tried, so nothing is claimed about it"
    );
    assert_eq!(source["capabilities"]["known"], true);
    assert_eq!(source["capabilities"]["chapters"], true);

    harness.cleanup().await;
}

/// The library says how many chapters of a work it actually holds.
///
/// Counted from what was stored, not copied from the source's own number. The
/// two answer different questions and diverge the moment an import is partial:
/// a card showing the source's count would describe a complete copy of a work
/// the reader holds a third of. The browser journey is where this was noticed —
/// the card carried the author, the status, the word count and both dates, and
/// never said how many chapters had arrived.
#[tokio::test]
async fn the_library_reports_how_many_chapters_are_stored() {
    let harness = Harness::new("library-count").await;
    let (_client, account, pseud) = signed_in(&harness, "counted@example.org", "counted").await;

    // The count is read before the import, when there is no item at all: a
    // library that reported a count only after a second visit would be reporting
    // on the wrong thing.
    let before = imports::list_library_items(&harness.db, &account.to_string(), 50, None)
        .await
        .expect("list the empty library");
    assert!(before.is_empty(), "nothing is imported yet");

    let import_id = run_import(&harness, account, &pseud, FixtureArchive::new(), false).await;
    let stored = imports::list_import_chapters(&harness.db, &import_id)
        .await
        .expect("the import's chapters")
        .iter()
        .filter(|chapter| chapter.state == "stored")
        .count();
    assert_eq!(stored, 3, "the fixture holds three chapters");

    let items = imports::list_library_items(&harness.db, &account.to_string(), 50, None)
        .await
        .expect("list the library");
    let item = items.first().expect("the imported item");
    assert_eq!(
        item.chapter_count, 3,
        "the copy reports the chapters it holds, not the source's claim"
    );

    harness.cleanup().await;
}
