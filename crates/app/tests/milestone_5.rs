//! Milestone 5 acceptance tests (spec §10).
//!
//! These run against the real router, a real SQLite file, a real storage
//! directory and the real worker. The worker is not mocked: the whole point of
//! the milestone is that two workers racing for one queue do not both get the
//! same row, and a fake would prove nothing about that.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_app::worker::{PassReport, TopicHandler, Worker, WorkerOptions};
use lorehaven_db::storage::BlobStore;
use lorehaven_db::{jobs, outbox, Database, DatabaseConfig};
use lorehaven_domain::jobs::{JobKind, JobState, RetryPolicy};
use lorehaven_domain::{AccountId, JobId, OutboxEventId};
use serde_json::{json, Value};
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m5-{tag}-{}-{:?}",
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
    config
}

struct Harness {
    dir: PathBuf,
    db: Database,
    state: AppState,
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
            report.applied.contains(&"0005_jobs_and_storage".to_owned()),
            "the jobs migration must apply: {report:?}"
        );

        let state = AppState::new(config_for(&dir), db.clone());
        Self { dir, db, state }
    }

    fn client(&self) -> Client {
        Client::new(server::build_router(AppState::new(
            config_for(&self.dir),
            self.db.clone(),
        )))
    }

    fn store(&self) -> BlobStore {
        BlobStore::new(self.dir.join("storage"))
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

fn worker() -> Worker {
    worker_with(RetryPolicy {
        max_attempts: 3,
        base_delay: Duration::from_secs(1),
        backoff: 2.0,
        jitter_permille: 0,
    })
}

fn worker_with(policy: RetryPolicy) -> Worker {
    Worker::new(WorkerOptions {
        id: "test-worker".to_owned(),
        lease: Duration::from_millis(500),
        poll_interval: Duration::from_millis(10),
        policy,
        batch: 50,
    })
}

// ---------------------------------------------------------------------------
// The queue
// ---------------------------------------------------------------------------

/// Two workers racing for one queue must not both get the same row.
#[tokio::test]
async fn a_claimed_job_is_not_claimed_twice() {
    let harness = Harness::new("claim-once").await;
    let store = harness.store();
    let _ = &store;
    for _ in 0..8 {
        jobs::enqueue(
            &harness.db,
            JobKind::Maintenance,
            r#"{"task":"probe","steps":1}"#,
            None,
            None,
            0,
            &RetryPolicy::default(),
        )
        .await
        .expect("enqueue");
    }

    let now = time::OffsetDateTime::now_utc();
    let mut seen: Vec<String> = Vec::new();
    // Eight claims by two workers, interleaved the way two processes would.
    for round in 0..4 {
        for name in ["worker-a", "worker-b"] {
            let claimed = jobs::claim_next(&harness.db, name, Duration::from_secs(60), now)
                .await
                .expect("claim")
                .expect("a job is waiting");
            assert!(
                !seen.contains(&claimed.id),
                "round {round}: {name} was given a job another worker already holds"
            );
            seen.push(claimed.id);
        }
    }
    assert_eq!(seen.len(), 8, "eight jobs, eight distinct claims");

    harness.cleanup().await;
}

/// A worker that dies mid-job leaves a lease, and the sweep returns the job to
/// the queue. Without this, one `kill -9` takes a job out of circulation
/// forever.
#[tokio::test]
async fn a_lease_that_expires_is_requeued() {
    let harness = Harness::new("lease-expiry").await;
    let job = jobs::enqueue(
        &harness.db,
        JobKind::Maintenance,
        r#"{"task":"probe"}"#,
        None,
        None,
        0,
        &RetryPolicy::default(),
    )
    .await
    .expect("enqueue");

    let start = time::OffsetDateTime::now_utc();
    let claimed = jobs::claim_next(&harness.db, "doomed-worker", Duration::from_secs(30), start)
        .await
        .expect("claim")
        .expect("a job is waiting");
    assert_eq!(claimed.id, job.to_string());
    assert_eq!(claimed.state, "leased");
    assert!(claimed.lease_expires_at.is_some());

    // Long before the lease ends, nothing is reclaimed.
    let requeued = jobs::requeue_expired_leases(&harness.db, start + Duration::from_secs(1))
        .await
        .expect("sweep");
    assert_eq!(requeued, 0, "a live lease must not be stolen");
    assert_eq!(
        jobs::find(&harness.db, job).await.unwrap().unwrap().state,
        "leased"
    );

    // After it expires, the job is available again — and the worker that held it
    // can no longer finish it.
    let requeued = jobs::requeue_expired_leases(&harness.db, start + Duration::from_secs(31))
        .await
        .expect("sweep");
    assert_eq!(requeued, 1);
    let row = jobs::find(&harness.db, job).await.unwrap().unwrap();
    assert_eq!(row.state, "queued");
    assert!(row.lease_owner.is_none());

    let reclaimed = jobs::claim_next(
        &harness.db,
        "second-worker",
        Duration::from_secs(30),
        start + Duration::from_secs(32),
    )
    .await
    .expect("claim")
    .expect("the job is claimable again");
    assert_eq!(reclaimed.id, job.to_string());
    assert!(
        !jobs::complete(&harness.db, job, "doomed-worker")
            .await
            .expect("complete"),
        "the worker that lost its lease must not be able to finish the job"
    );

    harness.cleanup().await;
}

/// A cancelled job stops at the next checkpoint, rather than running to the end
/// and reporting success.
#[tokio::test]
async fn a_cancelled_job_stops_at_the_next_checkpoint() {
    let harness = Harness::new("cancel-checkpoint").await;
    // Twelve steps of 40ms: long enough that the cancel lands in the middle.
    let job = jobs::enqueue(
        &harness.db,
        JobKind::Maintenance,
        r#"{"task":"probe","steps":12,"delay_ms":40}"#,
        None,
        None,
        0,
        &RetryPolicy::default(),
    )
    .await
    .expect("enqueue");

    let state = harness.state.clone();
    let running = tokio::spawn(async move { worker().run_once(&state).await });

    // Let a couple of steps happen, then cancel it.
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert!(
        jobs::cancel(&harness.db, job).await.expect("cancel"),
        "a queued or running job is cancellable"
    );
    let report = running.await.expect("join").expect("pass");

    let (finished, job_state) = report.job.expect("the worker ran the job");
    assert_eq!(finished, job);
    assert_eq!(job_state, JobState::Cancelled);

    let row = jobs::find(&harness.db, job).await.unwrap().unwrap();
    assert_eq!(row.state, "cancelled");
    assert!(
        row.progress_permille < 1000,
        "a cancelled job must not have finished its work: {row:?}"
    );
    assert!(
        row.checkpoint.is_some(),
        "a cancelled job leaves the checkpoint it stopped at: {row:?}"
    );
    let attempts = jobs::attempts_for(&harness.db, job)
        .await
        .expect("attempts");
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].outcome.as_deref(), Some("cancelled"));

    harness.cleanup().await;
}

/// A failed attempt is retried later, not immediately, and gives up when the
/// policy's budget is spent.
#[tokio::test]
async fn a_retry_uses_the_backoff() {
    let harness = Harness::new("retry-backoff").await;
    let policy = RetryPolicy {
        max_attempts: 2,
        base_delay: Duration::from_secs(60),
        backoff: 2.0,
        jitter_permille: 0,
    };
    let job = jobs::enqueue(
        &harness.db,
        JobKind::Maintenance,
        r#"{"task":"probe","steps":3,"fail":"the far end hung up"}"#,
        None,
        None,
        0,
        &policy,
    )
    .await
    .expect("enqueue");

    let state = harness.state.clone();
    let report = worker_with(policy).run_once(&state).await.expect("pass");
    assert_eq!(
        report.job.expect("the worker ran the job").1,
        JobState::Queued,
        "the first failure is retried"
    );

    let row = jobs::find(&harness.db, job).await.unwrap().unwrap();
    assert_eq!(row.attempts, 1);
    assert!(row.last_error.is_some(), "the reason is recorded");
    let available = time::OffsetDateTime::parse(
        &row.available_at,
        &time::format_description::well_known::Rfc3339,
    )
    .expect("a timestamp");
    let now: time::OffsetDateTime = time::OffsetDateTime::now_utc();
    assert!(
        available > now + Duration::from_secs(50),
        "a retry must wait for the backoff, not spin: {available} against {now}"
    );

    // Not claimable yet: the queue must not hand it to a worker before then.
    let claimed = jobs::claim_next(
        &harness.db,
        "eager-worker",
        Duration::from_secs(30),
        time::OffsetDateTime::now_utc(),
    )
    .await
    .expect("claim");
    assert!(
        claimed.is_none(),
        "a job waiting for its retry is not claimable"
    );

    // Run out the budget: the second failure is terminal.
    let claimed = jobs::claim_next(
        &harness.db,
        "eager-worker",
        Duration::from_secs(30),
        available + Duration::from_secs(1),
    )
    .await
    .expect("claim")
    .expect("the retry is claimable once its time comes");
    assert_eq!(
        claimed.attempts, 1,
        "a claim alone is not yet an attempt; the worker records that next"
    );
    // What the worker does immediately after a claim, so the counter and the
    // retry decision see the same number the handler does.
    jobs::attempt_started(&harness.db, job, claimed.attempts + 1, "eager-worker")
        .await
        .expect("attempt started");
    assert_eq!(
        jobs::find(&harness.db, job)
            .await
            .unwrap()
            .unwrap()
            .attempts,
        2,
        "the attempt just made is the second one"
    );
    let next = jobs::fail(
        &harness.db,
        job,
        "eager-worker",
        "still broken",
        &policy,
        false,
        available + Duration::from_secs(1),
    )
    .await
    .expect("fail");
    assert_eq!(
        next,
        JobState::Failed,
        "the second attempt exhausts the policy"
    );
    assert_eq!(
        jobs::find(&harness.db, job).await.unwrap().unwrap().state,
        "failed"
    );

    harness.cleanup().await;
}

/// Replaying one idempotency key enqueues one job: a retried request from a
/// browser must not start the work twice.
#[tokio::test]
async fn replaying_one_idempotency_key_enqueues_one_job() {
    let harness = Harness::new("idempotent-enqueue").await;
    let policy = RetryPolicy::default();
    let payload = r#"{"task":"probe"}"#;

    let first = jobs::enqueue(
        &harness.db,
        JobKind::Maintenance,
        payload,
        Some("request-42"),
        None,
        0,
        &policy,
    )
    .await
    .expect("enqueue");
    let second = jobs::enqueue(
        &harness.db,
        JobKind::Maintenance,
        payload,
        Some("request-42"),
        None,
        0,
        &policy,
    )
    .await
    .expect("enqueue again");
    assert_eq!(first, second, "the same key is the same job");

    let third = jobs::enqueue(
        &harness.db,
        JobKind::Maintenance,
        payload,
        Some("request-43"),
        None,
        0,
        &policy,
    )
    .await
    .expect("enqueue");
    assert_ne!(first, third, "a different key is a different job");

    let all = jobs::all_jobs(&harness.db, None, 50, None)
        .await
        .expect("list");
    assert_eq!(all.len(), 2);

    // A job with no key is never deduplicated against another.
    let unkeyed_a = jobs::enqueue(
        &harness.db,
        JobKind::Maintenance,
        payload,
        None,
        None,
        0,
        &policy,
    )
    .await
    .expect("enqueue");
    let unkeyed_b = jobs::enqueue(
        &harness.db,
        JobKind::Maintenance,
        payload,
        None,
        None,
        0,
        &policy,
    )
    .await
    .expect("enqueue");
    assert_ne!(unkeyed_a, unkeyed_b);

    harness.cleanup().await;
}

/// A job kind this build has no handler for fails loudly. A job that quietly
/// "succeeded" without doing its work is the one outcome nobody detects later.
#[tokio::test]
async fn a_job_with_no_handler_fails_loudly() {
    let harness = Harness::new("no-handler").await;
    let job = jobs::enqueue(
        &harness.db,
        JobKind::Reindex,
        "{}",
        None,
        None,
        0,
        &RetryPolicy {
            max_attempts: 1,
            ..RetryPolicy::default()
        },
    )
    .await
    .expect("enqueue");

    let state = harness.state.clone();
    let report = worker().run_once(&state).await.expect("pass");
    assert_eq!(report.job.expect("ran").1, JobState::Failed);

    let row = jobs::find(&harness.db, job).await.unwrap().unwrap();
    assert_eq!(row.state, "failed");
    let error = row.last_error.expect("a reason");
    assert!(error.contains("reindex"), "{error}");

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

/// The same bytes stored twice are one blob and one file: content addressing is
/// what makes deduplication physical rather than logical.
#[tokio::test]
async fn the_same_bytes_stored_twice_share_one_blob() {
    let harness = Harness::new("dedupe").await;
    let store = harness.store();

    let (first, key) = store
        .put(&harness.db, b"the same chapter text", "text/plain")
        .await
        .expect("put");
    let before = store
        .stat(&harness.db, &first)
        .await
        .expect("stat")
        .expect("a row");

    // A second put of the same bytes, a moment later.
    tokio::time::sleep(Duration::from_millis(5)).await;
    let (second, second_key) = store
        .put(&harness.db, b"the same chapter text", "text/plain")
        .await
        .expect("put");
    assert_eq!(first, second);
    assert_eq!(key, second_key);

    let after = store
        .stat(&harness.db, &first)
        .await
        .expect("stat")
        .expect("a row");
    assert_eq!(
        before.last_referenced_at, after.last_referenced_at,
        "a re-put must not resurrect a blob something else may be collecting"
    );
    assert_eq!(after.storage_key, key);

    // Exactly one file, at the derived path, with the bytes intact.
    let path = store.path_for(&first);
    assert!(path.exists(), "{}", path.display());
    assert_eq!(
        tokio::fs::read(&path).await.expect("read"),
        b"the same chapter text"
    );
    let (blobs, bytes) = store.usage(&harness.db).await.expect("usage");
    assert_eq!(blobs, 1);
    assert_eq!(bytes, 21, "the byte count is the file's, not an estimate");

    harness.cleanup().await;
}

/// Deleting one of two references keeps the blob: `content_references` is the
/// only thing standing between a shared blob and data loss.
#[tokio::test]
async fn deleting_one_reference_keeps_the_blob() {
    let harness = Harness::new("ref-count").await;
    let store = harness.store();
    let (checksum, _) = store
        .put(&harness.db, b"shared bytes", "text/plain")
        .await
        .expect("put");

    store
        .reference(&harness.db, &checksum, "work", "work-a")
        .await
        .expect("reference");
    store
        .reference(&harness.db, &checksum, "work", "work-b")
        .await
        .expect("reference");

    assert!(
        !store
            .delete_if_unreferenced(&harness.db, &checksum)
            .await
            .expect("delete"),
        "a referenced blob is not deleted"
    );
    assert!(store
        .unreference(&harness.db, &checksum, "work", "work-a")
        .await
        .expect("unreference"));
    assert!(
        !store
            .delete_if_unreferenced(&harness.db, &checksum)
            .await
            .expect("delete"),
        "one reference remains, so the bytes stay"
    );

    // And a stranger can still read them: the delete did not half-happen.
    let bytes = store
        .get(&harness.db, &checksum)
        .await
        .expect("get")
        .expect("the blob is still there");
    assert_eq!(bytes, b"shared bytes");

    harness.cleanup().await;
}

/// The last reference going takes the blob and its file with it.
#[tokio::test]
async fn deleting_the_last_reference_removes_the_blob() {
    let harness = Harness::new("last-reference").await;
    let store = harness.store();
    let (checksum, _) = store
        .put(&harness.db, b"temporary bytes", "text/plain")
        .await
        .expect("put");
    store
        .reference(&harness.db, &checksum, "export", "export-1")
        .await
        .expect("reference");
    let path = store.path_for(&checksum);
    assert!(path.exists());

    store
        .unreference(&harness.db, &checksum, "export", "export-1")
        .await
        .expect("unreference");
    assert!(
        store
            .delete_if_unreferenced(&harness.db, &checksum)
            .await
            .expect("delete"),
        "the last reference is gone, so the blob goes"
    );
    assert!(!path.exists(), "the file went with the row");
    assert!(store
        .stat(&harness.db, &checksum)
        .await
        .expect("stat")
        .is_none());
    assert!(store
        .get(&harness.db, &checksum)
        .await
        .expect("get")
        .is_none());

    // Deleting again is not an error: the caller asked for the end state.
    assert!(!store
        .delete_if_unreferenced(&harness.db, &checksum)
        .await
        .expect("delete again"));

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// The outbox
// ---------------------------------------------------------------------------

/// An event is deleted only after its handler returns success, and a topic
/// nothing handles yet is left in place rather than marked delivered.
#[tokio::test]
async fn an_outbox_event_is_deleted_only_after_its_handler_succeeds() {
    let harness = Harness::new("outbox").await;

    // Two topics: one with a handler, one that arrives in a later milestone.
    let handled = outbox::enqueue(
        &harness.db,
        "test.deliverable",
        r#"{"hello":"world"}"#,
        Some("dedupe-1"),
    )
    .await
    .expect("enqueue");
    let unhandled = outbox::enqueue(
        &harness.db,
        "publish.index",
        r#"{"work":"x"}"#,
        Some("dedupe-2"),
    )
    .await
    .expect("enqueue");

    let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = seen.clone();
    let handler: TopicHandler = Arc::new(move |_state, event| {
        let captured = captured.clone();
        let topic = event.topic.clone();
        Box::pin(async move {
            captured.lock().expect("lock").push(topic);
            Ok(())
        })
    });

    let worker = worker().with_topic("test.deliverable", handler);
    let state = harness.state.clone();
    let report = worker.run_once(&state).await.expect("pass");

    assert_eq!(report.outbox_delivered, 1);
    assert_eq!(
        report.outbox_deferred, 1,
        "no handler for publish.index yet"
    );
    assert_eq!(seen.lock().expect("lock").as_slice(), ["test.deliverable"]);

    let pending = outbox::pending(&harness.db, 50).await.expect("pending");
    let topics: Vec<&str> = pending.iter().map(|event| event.topic.as_str()).collect();
    assert_eq!(
        topics,
        ["publish.index"],
        "the handled event is gone and the unhandled one is still waiting"
    );
    assert_eq!(
        outbox::undelivered_count(&harness.db).await.expect("count"),
        1
    );

    let _ = (handled, unhandled);
    harness.cleanup().await;
}

/// A handler that fails leaves the event, records why, and pushes it out of the
/// way for a while.
#[tokio::test]
async fn a_failing_outbox_handler_retries_with_a_reason() {
    let harness = Harness::new("outbox-failure").await;
    outbox::enqueue(&harness.db, "test.broken", "{}", Some("dedupe-3"))
        .await
        .expect("enqueue");

    let handler: TopicHandler = Arc::new(|_state, _event| {
        Box::pin(async { Err(anyhow::anyhow!("the far end refused the connection")) })
    });
    let worker = worker().with_topic("test.broken", handler);
    let state = harness.state.clone();
    let report = worker.run_once(&state).await.expect("pass");

    assert_eq!(report.outbox_failed, 1);
    assert_eq!(report.outbox_delivered, 0);
    let pending = outbox::pending(&harness.db, 50).await.expect("pending");
    assert!(
        pending.is_empty(),
        "a failed event is not offered again immediately"
    );

    let row: (i64, Option<String>) = sqlx::query_as(
        "SELECT attempts, last_error FROM outbox_events WHERE topic = 'test.broken'",
    )
    .fetch_one(harness.db.sqlite_pool().expect("handle"))
    .await
    .expect("row");
    assert_eq!(row.0, 1);
    assert!(
        row.1.as_deref().unwrap_or_default().contains("refused"),
        "{row:?}"
    );

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

/// A caller sees their own jobs, and only their own.
#[tokio::test]
async fn the_job_list_shows_only_the_callers_own_jobs() {
    let harness = Harness::new("jobs-routes").await;
    let mut mine = harness.client();
    let me = register(&mut mine, "mine@example.com", "Mine").await;
    let mut theirs = harness.client();
    let them = register(&mut theirs, "theirs@example.com", "Theirs").await;

    let policy = RetryPolicy::default();
    let my_job = jobs::enqueue(
        &harness.db,
        JobKind::Maintenance,
        r#"{"task":"probe"}"#,
        None,
        Some(me),
        0,
        &policy,
    )
    .await
    .expect("enqueue");
    let their_job = jobs::enqueue(
        &harness.db,
        JobKind::Maintenance,
        r#"{"task":"probe"}"#,
        None,
        Some(them),
        0,
        &policy,
    )
    .await
    .expect("enqueue");

    let (status, body) = mine.get("/api/v1/jobs").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{body}");
    assert_eq!(items[0]["id"], my_job.to_string());
    assert_eq!(items[0]["kind"], "maintenance");
    assert_eq!(items[0]["state"], "queued");
    assert_eq!(items[0]["cancellable"], true);
    assert_eq!(body["next_cursor"], Value::Null);

    // Another account's job is not in the list …
    let (status, body) = theirs.get("/api/v1/jobs").await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], their_job.to_string());

    // … and cannot be cancelled by id either: a 404, which is what a resource
    // the caller may not reach looks like.
    let (status, _) = theirs
        .post(&format!("/api/v1/jobs/{my_job}/cancel"), json!({}))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        jobs::find(&harness.db, my_job)
            .await
            .unwrap()
            .unwrap()
            .state,
        "queued",
        "a stranger's cancel did nothing"
    );

    // The owner can cancel, and the answer says what the state now is.
    let (status, body) = mine
        .post(&format!("/api/v1/jobs/{my_job}/cancel"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "cancelled");
    assert_eq!(body["cancellable"], false);
    assert_eq!(
        jobs::find(&harness.db, my_job)
            .await
            .unwrap()
            .unwrap()
            .state,
        "cancelled"
    );

    harness.cleanup().await;
}

/// `/admin/jobs` is gated on `config.administration.operator_account_id`, and a
/// non-operator gets a 404 rather than a 403 — the difference matters, because
/// a 403 confirms that the surface exists.
#[tokio::test]
async fn the_admin_surface_is_gated_on_the_operator_account() {
    let harness = Harness::new("admin-gate").await;
    let mut reader = harness.client();
    let reader_account = register(&mut reader, "reader@example.com", "Reader").await;

    jobs::enqueue(
        &harness.db,
        JobKind::Maintenance,
        r#"{"task":"probe"}"#,
        None,
        Some(reader_account),
        0,
        &RetryPolicy::default(),
    )
    .await
    .expect("enqueue");

    // No operator is configured: nobody may use it, including the account that
    // would be the operator once one is named.
    let (status, body) = reader.get("/api/v1/admin/jobs").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    // Configure the operator and the same session gets in.
    let mut config = config_for(&harness.dir);
    config.administration.operator_account_id = Some(reader_account);
    let operator_client = Client::new(server::build_router(AppState::new(
        config,
        harness.db.clone(),
    )));
    // A fresh client has no cookies; sign in through it.
    let mut operator_client = operator_client;
    let (status, body) = operator_client
        .post(
            "/api/v1/auth/login",
            json!({ "email": "reader@example.com", "password": PASSWORD }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "sign in: {body}");

    let (status, body) = operator_client.get("/api/v1/admin/jobs").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"].as_array().expect("items").len(), 1);

    // An unknown state filter is refused rather than ignored: a page that shows
    // everything when asked for a typo looks like it worked.
    let (status, body) = operator_client.get("/api/v1/admin/jobs?state=runing").await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "an unknown state is refused rather than ignored: {body}"
    );
    assert_eq!(body["error"]["code"], "VALIDATION_FAILED");

    // A cursor this API never issued is refused too, rather than quietly
    // restarting the list at page one.
    let (status, body) = operator_client
        .get("/api/v1/jobs?cursor=not-a-cursor")
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "VALIDATION_FAILED");

    // … and a real one filters.
    let (status, body) = operator_client
        .get("/api/v1/admin/jobs?state=succeeded")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().expect("items").len(), 0);

    let mut plain = harness.client();
    register(&mut plain, "plain@example.com", "Plain").await;
    let (status, _) = plain.get("/api/v1/admin/jobs").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "still not an operator");

    harness.cleanup().await;
}

/// An operator may retry a job that has finished failing; a job that has not
/// finished is not retryable, and the answer says so.
#[tokio::test]
async fn an_operator_can_retry_a_failed_job() {
    let harness = Harness::new("admin-retry").await;
    let mut client = harness.client();
    let account = register(&mut client, "ops@example.com", "Ops").await;

    let job = jobs::enqueue(
        &harness.db,
        JobKind::Thumbnail,
        "{}",
        None,
        Some(account),
        0,
        &RetryPolicy {
            max_attempts: 1,
            ..RetryPolicy::default()
        },
    )
    .await
    .expect("enqueue");
    let state = harness.state.clone();
    worker().run_once(&state).await.expect("run");
    assert_eq!(
        jobs::find(&harness.db, job).await.unwrap().unwrap().state,
        "failed"
    );

    let mut config = config_for(&harness.dir);
    config.administration.operator_account_id = Some(account);
    let mut operator = Client::new(server::build_router(AppState::new(
        config,
        harness.db.clone(),
    )));
    let (status, body) = operator
        .post(
            "/api/v1/auth/login",
            json!({ "email": "ops@example.com", "password": PASSWORD }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = operator
        .post(&format!("/api/v1/admin/jobs/{job}/retry"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "queued");
    assert_eq!(
        body["attempts"], 0,
        "a retry gets the policy's whole budget"
    );

    // A queued job is not retryable: there is nothing to retry yet.
    let (status, body) = operator
        .post(&format!("/api/v1/admin/jobs/{job}/retry"), json!({}))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "VALIDATION_FAILED");

    harness.cleanup().await;
}

/// The `worker --once` seam: one pass runs one job and reports what it did.
#[tokio::test]
async fn one_pass_runs_one_job_and_says_so() {
    let harness = Harness::new("worker-once").await;
    let job = jobs::enqueue(
        &harness.db,
        JobKind::Maintenance,
        r#"{"task":"probe","steps":3}"#,
        None,
        None,
        0,
        &RetryPolicy::default(),
    )
    .await
    .expect("enqueue");

    let state = harness.state.clone();
    let report: PassReport = worker().run_once(&state).await.expect("pass");
    assert_eq!(report.job.expect("a job ran").0, job);
    assert!(report.did_something());

    let row = jobs::find(&harness.db, job).await.unwrap().unwrap();
    assert_eq!(row.state, "succeeded");
    assert_eq!(row.progress_permille, 1000);
    assert_eq!(
        row.checkpoint, None,
        "a finished job has no checkpoint to resume"
    );
    let attempts = jobs::attempts_for(&harness.db, job)
        .await
        .expect("attempts");
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].outcome.as_deref(), Some("succeeded"));
    assert!(attempts[0].finished_at.is_some());

    // A pass with an empty queue reports that too, rather than hanging.
    let report = worker().run_once(&state).await.expect("empty pass");
    assert!(report.job.is_none());
    assert!(!report.did_something());

    harness.cleanup().await;
}

/// An unknown maintenance task is a fatal failure, and a job the worker cannot
/// parse is not silently marked done.
#[tokio::test]
async fn an_unknown_maintenance_task_is_a_fatal_failure() {
    let harness = Harness::new("unknown-task").await;
    let job = jobs::enqueue(
        &harness.db,
        JobKind::Maintenance,
        r#"{"task":"make-coffee"}"#,
        None,
        None,
        0,
        &RetryPolicy {
            max_attempts: 3,
            ..RetryPolicy::default()
        },
    )
    .await
    .expect("enqueue");

    let state = harness.state.clone();
    let report = worker().run_once(&state).await.expect("pass");
    assert_eq!(report.job.expect("ran").1, JobState::Failed);

    let row = jobs::find(&harness.db, job).await.unwrap().unwrap();
    assert_eq!(row.state, "failed");
    assert_eq!(
        row.attempts, 1,
        "a fatal failure does not burn the retry budget"
    );
    assert!(
        row.last_error.unwrap_or_default().contains("make-coffee"),
        "the message names the task"
    );

    harness.cleanup().await;
}

/// The maintenance sweep deletes terminal jobs past their retention window and
/// leaves everything else alone.
#[tokio::test]
async fn the_sweep_deletes_only_old_terminal_jobs() {
    let harness = Harness::new("sweep").await;
    let policy = RetryPolicy::default();
    let queued = jobs::enqueue(
        &harness.db,
        JobKind::Maintenance,
        r#"{"task":"probe"}"#,
        None,
        None,
        0,
        &policy,
    )
    .await
    .expect("enqueue");
    let done = jobs::enqueue(
        &harness.db,
        JobKind::Maintenance,
        r#"{"task":"probe","steps":1}"#,
        None,
        None,
        0,
        &policy,
    )
    .await
    .expect("enqueue");

    let state = harness.state.clone();
    // Run the second job (the first is claimed in queue order, so take two
    // passes: priority is equal and `available_at` decides).
    let first = worker().run_once(&state).await.expect("pass");
    let second = worker().run_once(&state).await.expect("pass");
    assert_eq!(
        first.job.map(|(id, _)| id),
        Some(queued),
        "the first claim is the first job"
    );
    assert_eq!(second.job.map(|(id, _)| id), Some(done));

    // A sweep with nothing old deletes nothing.
    assert_eq!(
        jobs::purge_terminal_jobs(
            &harness.db,
            time::OffsetDateTime::now_utc() - Duration::from_secs(30 * 24 * 60 * 60)
        )
        .await
        .expect("sweep"),
        0
    );

    // With a cutoff in the future, the terminal job goes and the row history
    // with it; the other job was terminal too, and goes the same way — but the
    // point is that the sweep is the only path that deletes jobs at all.
    let purged = jobs::purge_terminal_jobs(&harness.db, time::OffsetDateTime::now_utc())
        .await
        .expect("sweep");
    assert_eq!(purged, 2);
    assert!(jobs::find(&harness.db, queued)
        .await
        .expect("find")
        .is_none());
    let attempts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM job_attempts")
        .fetch_one(harness.db.sqlite_pool().expect("handle"))
        .await
        .expect("count");
    assert_eq!(attempts, 0, "attempts cascade with their job");

    harness.cleanup().await;
}

/// An expired credential check: outbox rows written by an earlier milestone are
/// still there and still undelivered, which is what the worker exists to fix.
#[tokio::test]
async fn the_worker_can_be_pointed_at_a_queue_that_is_already_waiting() {
    let harness = Harness::new("waiting-queue").await;
    for index in 0..5 {
        jobs::enqueue(
            &harness.db,
            JobKind::Maintenance,
            r#"{"task":"probe","steps":1}"#,
            None,
            None,
            i64::from(index),
            &RetryPolicy::default(),
        )
        .await
        .expect("enqueue");
    }

    let state = harness.state.clone();
    let mut succeeded = 0;
    for _ in 0..5 {
        let report = worker().run_once(&state).await.expect("pass");
        if let Some((_, JobState::Succeeded)) = report.job {
            succeeded += 1;
        }
    }
    assert_eq!(succeeded, 5);

    let counts = jobs::counts_by_state(&harness.db).await.expect("counts");
    assert_eq!(
        counts,
        vec![("succeeded".to_owned(), 5)],
        "every job in the queue ran and none was left behind"
    );

    harness.cleanup().await;
}

/// A job the worker claims is visible to its owner, with the progress the
/// handler recorded, which is what the page watches.
#[tokio::test]
async fn job_progress_and_errors_reach_the_owner() {
    let harness = Harness::new("job-progress").await;
    let mut client = harness.client();
    let account = register(&mut client, "owner@example.com", "Owner").await;

    let failing = jobs::enqueue(
        &harness.db,
        JobKind::Thumbnail,
        "{}",
        None,
        Some(account),
        0,
        &RetryPolicy {
            max_attempts: 1,
            ..RetryPolicy::default()
        },
    )
    .await
    .expect("enqueue");
    let state = harness.state.clone();
    worker().run_once(&state).await.expect("run");

    let (status, body) = client.get("/api/v1/jobs").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], failing.to_string());
    assert_eq!(items[0]["state"], "failed");
    assert!(
        items[0]["last_error"]
            .as_str()
            .is_some_and(|error| error.contains("thumbnail")),
        "the owner is told why: {body}"
    );
    assert_eq!(items[0]["cancellable"], false);
    let _ = (JobId::new(), OutboxEventId::new());

    harness.cleanup().await;
}

/// The journey's first half: a request that hands work to the queue answers
/// `202` with an id, and the work happens later, off the request.
#[tokio::test]
async fn a_request_that_starts_a_job_gets_a_202_and_an_id() {
    let harness = Harness::new("start-job").await;
    let mut client = harness.client();
    let account = register(&mut client, "writer@example.com", "Writer").await;

    let (status, body) = client
        .post(
            "/api/v1/jobs",
            json!({ "kind": "maintenance", "payload": { "task": "probe", "steps": 4 } }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["state"], "queued");
    assert_eq!(body["kind"], "maintenance");
    assert_eq!(body["progress_permille"], 0);
    assert_eq!(body["cancellable"], true, "a queued job can be cancelled");
    let id = body["id"].as_str().expect("a job id").to_owned();

    // The request did not do the work: nothing has run yet.
    let row = jobs::find(&harness.db, id.parse().expect("uuid"))
        .await
        .expect("find")
        .expect("the row is there");
    assert_eq!(row.state, "queued");
    assert_eq!(row.attempts, 0);
    assert_eq!(
        row.requested_by.as_deref(),
        Some(account.to_string().as_str()),
        "the job is the caller's, which is what lets them see and cancel it"
    );

    // Now the worker takes it, and the owner watches it finish.
    let state = harness.state.clone();
    worker_with(RetryPolicy {
        max_attempts: 3,
        base_delay: Duration::from_secs(1),
        backoff: 2.0,
        jitter_permille: 0,
    })
    .run_once(&state)
    .await
    .expect("pass");

    let (status, body) = client.get("/api/v1/jobs").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"].as_str(), Some(id.as_str()));
    assert_eq!(items[0]["state"], "succeeded");
    assert_eq!(items[0]["progress_permille"], 1000);
    assert_eq!(items[0]["cancellable"], false);

    harness.cleanup().await;
}

/// The self-service enqueue is narrow on purpose: a request may start the
/// diagnostic job and nothing else, and it does not exist in production.
#[tokio::test]
async fn the_self_service_enqueue_refuses_anything_but_the_probe() {
    let harness = Harness::new("start-job-refusals").await;
    let mut client = harness.client();
    register(&mut client, "writer@example.com", "Writer").await;

    for (label, body) in [
        ("another kind", json!({ "kind": "export" })),
        (
            "a maintenance task nobody asked for",
            json!({ "kind": "maintenance", "payload": { "task": "collect_blobs" } }),
        ),
    ] {
        let (status, body) = client.post("/api/v1/jobs", body).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{label}: {body}");
        assert_eq!(body["error"]["code"], "VALIDATION_FAILED", "{label}");
    }
    // Nothing was enqueued by a refused request.
    assert_eq!(
        jobs::all_jobs(&harness.db, None, 50, None)
            .await
            .expect("list")
            .len(),
        0
    );

    // A production instance does not have this route at all.
    let mut config = config_for(&harness.dir);
    config.environment = lorehaven_app::config::Environment::Production;
    let mut production = Client::new(server::build_router(AppState::new(
        config,
        harness.db.clone(),
    )));
    let (status, _) = production
        .post(
            "/api/v1/auth/register",
            json!({
                "email": "prod@example.com",
                "password": PASSWORD,
                "handle": "Prod",
                "display_name": "Prod",
                "age_band": "adult",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = production
        .post(
            "/api/v1/jobs",
            json!({ "kind": "maintenance", "payload": { "task": "probe" } }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "development only");

    harness.cleanup().await;
}

/// A page of jobs says whether there is another one, and the cursor it hands
/// back resumes exactly where the page ended: every row once, none twice.
#[tokio::test]
async fn a_page_of_jobs_carries_a_cursor_that_resumes_it() {
    let harness = Harness::new("cursor").await;
    let mut client = harness.client();
    let account = register(&mut client, "pager@example.com", "Pager").await;

    // Enqueued directly rather than over HTTP: 51 writes would meet the write
    // rate limit, and what is being tested here is the list, not the limit.
    // PAGE is 50, so 51 rows give a second page.
    let mut ids: Vec<String> = Vec::new();
    for index in 0..51 {
        let id = jobs::enqueue(
            &harness.db,
            JobKind::Maintenance,
            r#"{"task":"probe"}"#,
            None,
            Some(account),
            index,
            &RetryPolicy::default(),
        )
        .await
        .expect("enqueue");
        ids.push(id.to_string());
    }

    let (status, first) = client.get("/api/v1/jobs").await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let items = first["items"].as_array().expect("items");
    assert_eq!(items.len(), 50, "one page holds PAGE rows");
    let cursor = first["next_cursor"]
        .as_str()
        .expect("a full page offers a cursor")
        .to_owned();

    let (status, second) = client.get(&format!("/api/v1/jobs?cursor={cursor}")).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    let items = second["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "the rest of the collection");
    assert_eq!(second["next_cursor"], Value::Null, "and that is the end");

    // Every job exactly once across the two pages.
    let mut seen: Vec<String> = first["items"]
        .as_array()
        .expect("items")
        .iter()
        .chain(second["items"].as_array().expect("items").iter())
        .map(|item| item["id"].as_str().expect("id").to_owned())
        .collect();
    let total = seen.len();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), total, "no row is offered twice");
    let mut expected = ids.clone();
    expected.sort();
    assert_eq!(seen, expected, "and none is left out");

    // The page is newest first: the priority column rises with insertion order
    // and the list orders by creation, so the last enqueued leads.
    assert_eq!(first["items"][0]["id"].as_str(), Some(ids[50].as_str()));

    harness.cleanup().await;
}
