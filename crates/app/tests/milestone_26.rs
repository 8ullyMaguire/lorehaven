//! Milestone 26 — TTS narration edition CRUD (spec §32.5).
//!
//! Verifies that requesting a narration creates a `narration` edition in
//! draft state with the machine producer credited as narrator. Does NOT
//! test actual audio generation — that requires an AI provider integration
//! that doesn't exist yet in this build.

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

// Surface the app's internal error logs (http.rs drops them without a
// subscriber, which makes 500s undebuggable in tests).
#[allow(unused)]
fn init_logs() {
    let _ = tracing_subscriber::fmt().try_init();
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m26-narrate-{tag}-{:?}",
        std::process::id()
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
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
    fn capture(&mut self, response: &axum::response::Response) {
        for value in response.headers().get_all(header::SET_COOKIE) {
            let Ok(text) = value.to_str() else {
                continue;
            };
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
    async fn get(&mut self, uri: &str) -> (StatusCode, Value) {
        self.send("GET", uri, None).await
    }
}

async fn register(client: &mut Client, email: &str, handle: &str) {
    let (status, body) = client.post("/api/v1/auth/register", json!({ "email": email, "password": "a-long-enough-passphrase", "handle": handle, "display_name": handle, "age_band": "adult" })).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "register failed for {email}: {body}"
    );
}

async fn create_and_publish_work(client: &mut Client, title: &str) -> String {
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

    let (status, _) = client.post(&format!("/api/v1/works/{work_id}/publish"), json!({ "expected_version": work_version, "idempotency_key": format!("m26-narrate-{work_id}") })).await;
    assert_eq!(status, StatusCode::OK, "publish work");

    work_id
}

#[tokio::test]
async fn request_narration_creates_draft_edition_with_credited_narrator() {
    init_logs();
    let dir = scratch_dir("narrate-create");
    let tdb = test_support::TestDb::connect_with_dir("narrate-create", &dir).await;
    // The silent engine: this test is about the edition, not about a voice, and
    // the request door refuses to queue synthesis on a host that cannot narrate.
    let app = server::build_router(AppState::new(
        config_with_silent_tts(&dir),
        tdb.db().clone(),
    ));

    let mut client = Client::new(app.clone());
    register(&mut client, "author@example.com", "author").await;
    let work_id = create_and_publish_work(&mut client, "Narration Work").await;

    // Request a TTS narration
    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/editions"),
            json!({ "provider": "ai-provider" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "request narration: {body}");
    assert_eq!(body["edition_kind"].as_str().unwrap(), "narration");
    assert_eq!(body["state"].as_str().unwrap(), "draft");
    assert!(body["machine_produced"].as_bool().unwrap_or(false));

    let edition_id = body["edition_id"].as_str().unwrap().to_owned();

    // Read the edition back
    let (status, body) = client.get(&format!("/api/v1/editions/{edition_id}")).await;
    assert_eq!(status, StatusCode::OK, "get edition: {body}");
    assert_eq!(body["edition_kind"].as_str().unwrap(), "narration");
    assert_eq!(body["work_id"].as_str().unwrap(), work_id);
    assert!(body["label"].as_str().unwrap().contains("ai-provider"));

    println!("PASS: request_narration_creates_draft_edition_with_credited_narrator");
}

#[tokio::test]
async fn narration_editions_listable_after_creation() {
    let dir = scratch_dir("narrate-list");
    let tdb = test_support::TestDb::connect_with_dir("narrate-list", &dir).await;
    let app = server::build_router(AppState::new(
        config_with_silent_tts(&dir),
        tdb.db().clone(),
    ));

    let mut client = Client::new(app.clone());
    register(&mut client, "author@example.com", "author").await;
    let work_id = create_and_publish_work(&mut client, "Listable Work").await;

    // Create narration
    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/editions"),
            json!({ "provider": "ai-provider" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "request narration: {body}");

    // List editions
    let (status, body) = client
        .get(&format!("/api/v1/works/{work_id}/editions"))
        .await;
    assert_eq!(status, StatusCode::OK, "list editions: {body}");
    let editions = body["editions"].as_array().expect("editions array");
    assert_eq!(editions.len(), 1, "expected exactly one narration edition");
    assert_eq!(editions[0]["edition_kind"].as_str().unwrap(), "narration");

    println!("PASS: narration_editions_listable_after_creation");
}

// ---------------------------------------------------------------------------
// The narration pipeline: request -> job -> audio -> approval
// ---------------------------------------------------------------------------

/// A config whose narration engine needs no synthesizer.
///
/// `silent` is what makes the pipeline testable: it emits a well-formed WAV the
/// length of the text, so the assertions below are about the *pipeline* —
/// chunking, splicing, storage, the draft gate — rather than about a voice.
fn config_with_silent_tts(dir: &Path) -> Config {
    let mut config = config_for(dir);
    config.tts.engine = "silent".to_string();
    config
}

async fn run_worker_once(state: &AppState) -> PassReport {
    let worker = Worker::new(WorkerOptions::named("m26-narrate-worker"));
    worker.run_once(state).await.expect("worker pass")
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

/// Requesting a narration queues one synthesis job and nothing else.
#[tokio::test]
async fn requesting_a_narration_queues_the_synthesis_job() {
    let dir = scratch_dir("narrate-queue");
    let tdb = test_support::TestDb::connect_with_dir("narrate-queue", &dir).await;
    let config = config_with_silent_tts(&dir);
    let state = AppState::new(config.clone(), tdb.db().clone());
    let app = server::build_router(state.clone());

    let mut client = Client::new(app.clone());
    register(&mut client, "author@example.com", "author").await;
    let work_id = create_and_publish_work(&mut client, "Queued Work").await;

    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/editions"),
            json!({ "provider": "ai-provider" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "request narration: {body}");
    assert_eq!(body["state"].as_str().unwrap(), "draft");
    assert!(
        body["job_id"].as_str().is_some_and(|id| !id.is_empty()),
        "the request must name the queued job: {body}"
    );

    // Exactly one queued job, and it is a narration job whose payload names the
    // edition rather than the work: a queue row must not carry a work's text.
    let kind = scalar(tdb.db(), "SELECT kind FROM jobs WHERE state = 'queued'").await;
    assert_eq!(kind, "narration");
    let payload: String = match tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query_scalar("SELECT payload FROM jobs WHERE state = 'queued'")
                .fetch_one(tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("payload")
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query_scalar("SELECT payload FROM jobs WHERE state = 'queued'")
                .fetch_one(tdb.db().postgres_pool().expect("postgres"))
                .await
                .expect("payload")
        }
    };
    let parsed: Value = serde_json::from_str(&payload).expect("payload is JSON");
    assert_eq!(
        parsed["edition_id"].as_str().unwrap(),
        body["edition_id"].as_str().unwrap()
    );
    assert!(
        !payload.contains("Content."),
        "the payload must not carry the work's text: {payload}"
    );

    println!("PASS: requesting_a_narration_queues_the_synthesis_job");
}

/// The whole pipeline: the worker synthesizes, the audio is stored, the edition
/// stays a draft through all of it, and only the author's approval publishes it.
#[tokio::test]
async fn the_narration_pipeline_stores_audio_and_keeps_the_draft_gate() {
    let dir = scratch_dir("narrate-pipeline");
    let tdb = test_support::TestDb::connect_with_dir("narrate-pipeline", &dir).await;
    let config = config_with_silent_tts(&dir);
    let state = AppState::new(config.clone(), tdb.db().clone());
    let app = server::build_router(state.clone());

    let mut client = Client::new(app.clone());
    register(&mut client, "author@example.com", "author").await;
    let work_id = create_and_publish_work(&mut client, "Narrated Work").await;

    let (_, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/editions"),
            json!({ "provider": "ai-provider" }),
        )
        .await;
    let edition_id = body["edition_id"].as_str().unwrap().to_owned();

    // The job runs to completion.
    let report = run_worker_once(&state).await;
    assert_eq!(
        report.job.as_ref().map(|(_, state)| *state),
        Some(lorehaven_domain::jobs::JobState::Succeeded),
        "the narration job must succeed on a silent engine: {report:?}"
    );

    // Audio was recorded on the edition, and a media_file row points at it.
    let (status, body) = client.get(&format!("/api/v1/editions/{edition_id}")).await;
    assert_eq!(status, StatusCode::OK, "get edition: {body}");
    assert!(body["has_audio"].as_bool().unwrap_or(false), "{body}");
    assert_eq!(
        body["state"].as_str().unwrap(),
        "draft",
        "synthesis must not publish the edition: {body}"
    );
    let checksum = body["audio_checksum"].as_str().unwrap().to_owned();
    let file_count: i64 = match tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query_scalar("SELECT COUNT(*) FROM media_files WHERE checksum = ?")
                .bind(&checksum)
                .fetch_one(tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("media_files count")
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query_scalar("SELECT COUNT(*) FROM media_files WHERE checksum = $1")
                .bind(&checksum)
                .fetch_one(tdb.db().postgres_pool().expect("postgres"))
                .await
                .expect("media_files count")
        }
    };
    assert_eq!(
        file_count, 1,
        "the audio is attached to exactly one file row"
    );

    // Approving publishes it.
    let (status, body) = client
        .post(&format!("/api/v1/editions/{edition_id}/approve"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "approve: {body}");
    assert_eq!(body["state"].as_str().unwrap(), "published");
    assert!(body["changed"].as_bool().unwrap_or(false));

    // Publishing is idempotent: a second approval changes nothing.
    let (status, body) = client
        .post(&format!("/api/v1/editions/{edition_id}/approve"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "re-approve: {body}");
    assert!(
        !body["changed"].as_bool().unwrap_or(true),
        "a second approval must be a no-op: {body}"
    );

    println!("PASS: the_narration_pipeline_stores_audio_and_keeps_the_draft_gate");
}

/// The audio door: contributor-only while the edition is a draft, public once
/// it is published, and a 404 rather than a 403 for everyone else.
#[tokio::test]
async fn narration_audio_is_contributor_only_until_published() {
    let dir = scratch_dir("narrate-audio-door");
    let tdb = test_support::TestDb::connect_with_dir("narrate-audio-door", &dir).await;
    let config = config_with_silent_tts(&dir);
    let state = AppState::new(config.clone(), tdb.db().clone());
    let app = server::build_router(state.clone());

    let mut author = Client::new(app.clone());
    register(&mut author, "author@example.com", "author").await;
    let work_id = create_and_publish_work(&mut author, "Gated Work").await;
    let (_, body) = author
        .post(
            &format!("/api/v1/works/{work_id}/editions"),
            json!({ "provider": "ai-provider" }),
        )
        .await;
    let edition_id = body["edition_id"].as_str().unwrap().to_owned();
    run_worker_once(&state).await;

    // The author may fetch the draft's audio: it is how they check it.
    let response = author
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/editions/{edition_id}/audio"))
                .header(
                    header::COOKIE,
                    author
                        .cookies
                        .iter()
                        .map(|(n, v)| format!("{n}={v}"))
                        .collect::<Vec<_>>()
                        .join("; "),
                )
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "author fetches draft audio"
    );
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("audio/wav")
    );
    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .expect("body");
    assert!(
        bytes.len() > 44,
        "the WAV must have a header and some audio"
    );
    assert_eq!(&bytes[0..4], b"RIFF");

    // A signed-out reader gets the same answer as for an edition that is not
    // there: §3.3 — "use 404 rather than revealing inaccessible private
    // objects". The door is `MaybeSession` (published audio is public), so the
    // refusal is the draft branch's 404, not an `AUTH_REQUIRED` 401 that would
    // confirm a draft exists at this id.
    let mut anonymous = Client::new(app.clone());
    let (status, _) = anonymous
        .get(&format!("/api/v1/editions/{edition_id}/audio"))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "anonymous draft audio");

    // Another account with no connection to the work is refused, not told.
    let mut stranger = Client::new(app.clone());
    register(&mut stranger, "stranger@example.com", "stranger").await;
    let (status, _) = stranger
        .get(&format!("/api/v1/editions/{edition_id}/audio"))
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a stranger must not learn a draft narration exists"
    );
    // And the *metadata* door answers the same way: a stranger asking about a
    // draft edition is told it is not there, because the work itself is public
    // and this is only the author's unpublished narration.
    let (status, _) = stranger
        .get(&format!("/api/v1/editions/{edition_id}"))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "stranger reads the draft");

    // Once published, any reader may fetch it.
    let (status, _) = author
        .post(&format!("/api/v1/editions/{edition_id}/approve"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = stranger
        .get(&format!("/api/v1/editions/{edition_id}/audio"))
        .await;
    assert_eq!(status, StatusCode::OK, "a published narration downloads");

    println!("PASS: narration_audio_is_contributor_only_until_published");
}

/// Approving a narration whose audio does not exist is refused, and says why.
#[tokio::test]
async fn a_narration_without_audio_cannot_be_approved() {
    let dir = scratch_dir("narrate-no-audio");
    let tdb = test_support::TestDb::connect_with_dir("narrate-no-audio", &dir).await;
    let app = server::build_router(AppState::new(
        config_with_silent_tts(&dir),
        tdb.db().clone(),
    ));

    let mut client = Client::new(app.clone());
    register(&mut client, "author@example.com", "author").await;
    let work_id = create_and_publish_work(&mut client, "Silent Work").await;
    let (_, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/editions"),
            json!({ "provider": "ai-provider" }),
        )
        .await;
    let edition_id = body["edition_id"].as_str().unwrap().to_owned();

    // No worker pass: the job is still queued.
    let (status, body) = client
        .post(&format!("/api/v1/editions/{edition_id}/approve"), json!({}))
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "approve: {status} {body}"
    );
    assert!(
        body.to_string().contains("no audio"),
        "the refusal must name what is missing: {body}"
    );

    println!("PASS: a_narration_without_audio_cannot_be_approved");
}

/// An instance with no synthesizer fails the job with a sentence naming what to
/// install — it does not crash the worker, and it does not pretend to narrate.
#[tokio::test]
async fn a_missing_engine_fails_the_job_with_a_remedy() {
    let dir = scratch_dir("narrate-missing-engine");
    let tdb = test_support::TestDb::connect_with_dir("narrate-missing-engine", &dir).await;
    // The default engine is Piper, which is not installed in a test container.
    let config = config_for(&dir);
    assert_eq!(config.tts.engine, "piper");
    let state = AppState::new(config, tdb.db().clone());
    let app = server::build_router(state.clone());

    let mut client = Client::new(app.clone());
    register(&mut client, "author@example.com", "author").await;
    let work_id = create_and_publish_work(&mut client, "Unnarratable Work").await;

    // Requesting is refused up front when the instance has no engine at all:
    // the reader learns before a job is queued, not from a failed queue row.
    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/editions"),
            json!({ "provider": "ai-provider" }),
        )
        .await;
    if state.can_narrate() {
        assert_eq!(status, StatusCode::OK, "piper is installed here: {body}");
        println!("SKIP: a_missing_engine_fails_the_job_with_a_remedy (piper installed)");
        return;
    }
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{status} {body}");
    let message = body.to_string();
    assert!(
        message.contains("piper") || message.contains("narration engine"),
        "the refusal must name the engine: {message}"
    );

    println!("PASS: a_missing_engine_fails_the_job_with_a_remedy");
}

/// The engine the instance names is the engine the edition credits.
#[tokio::test]
async fn the_credited_narrator_is_the_engine_that_ran() {
    let dir = scratch_dir("narrate-credit");
    let tdb = test_support::TestDb::connect_with_dir("narrate-credit", &dir).await;
    let app = server::build_router(AppState::new(
        config_with_silent_tts(&dir),
        tdb.db().clone(),
    ));

    let mut client = Client::new(app.clone());
    register(&mut client, "author@example.com", "author").await;
    let work_id = create_and_publish_work(&mut client, "Credited Work").await;
    let (_, body) = client
        .post(&format!("/api/v1/works/{work_id}/editions"), json!({}))
        .await;
    let edition_id = body["edition_id"].as_str().unwrap().to_owned();
    assert_eq!(
        body["narrator"].as_str().unwrap(),
        "silent",
        "an unnamed provider defaults to the engine that will actually run: {body}"
    );

    // The credit lands in media_edition_creators, which is what §32.5 reads.
    let credited = scalar(
        tdb.db(),
        &format!("SELECT creator_id FROM media_edition_creators WHERE edition_id = '{edition_id}'"),
    )
    .await;
    assert_eq!(credited, "silent");

    println!("PASS: the_credited_narrator_is_the_engine_that_ran");
}

// ---------------------------------------------------------------------------
// Adult taxonomy behind every door (spec §32.5, §7.6)
// ---------------------------------------------------------------------------
//
// These three tests are the M26 rating-gate verification. They were written in
// this session and then dropped when the narration tests replaced this file
// wholesale; restored here, and the all-doors case now walks every door that
// can answer with a work — list, direct, files, editions, canon, space, search
// — plus the narration audio door, which was serving a published narration of
// an explicit work to anyone who knew its id.

/// Run one seed statement against whichever backend the fixture opened.
///
/// The query is built inside each arm on purpose: `sqlx::query`'s database
/// parameter is inferred from the executor it is handed, and one `Query` value
/// shared across both arms cannot be executed against two database types.
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
    result.expect("seed statement");
}

/// Seed a canon (migration 0026).
async fn seed_canon(tdb: &test_support::TestDb, n: u32, name: &str) -> String {
    let id = format!("00000000-0000-0000-0000-{n:012}");
    let cast = if tdb.is_postgres() { "::uuid" } else { "" };
    exec(
        tdb,
        &format!(
            "INSERT INTO canons (id, name, created_at, updated_at, version) \
             VALUES (?{cast}, ?, '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z', 1)"
        ),
        &[&id, name],
    )
    .await;
    id
}

/// Seed a canon_works association row (migration 0026).
async fn seed_canon_work(tdb: &test_support::TestDb, canon_id: &str, work_id: &str) {
    let cast = if tdb.is_postgres() { "::uuid" } else { "" };
    exec(
        tdb,
        &format!(
            "INSERT INTO canon_works (canon_id, work_id, position, created_at) \
             VALUES (?{cast}, ?{cast}, 1, '2026-09-01T00:00:00Z')"
        ),
        &[canon_id, work_id],
    )
    .await;
}

/// Seed a space (migration 0026).
async fn seed_space(tdb: &test_support::TestDb, n: u32, name: &str) -> String {
    let id = format!("00000000-0000-0000-0000-{n:012}");
    let cast = if tdb.is_postgres() { "::uuid" } else { "" };
    exec(
        tdb,
        &format!(
            "INSERT INTO spaces (id, name, created_at, updated_at, version) \
             VALUES (?{cast}, ?, '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z', 1)"
        ),
        &[&id, name],
    )
    .await;
    id
}

/// Seed a space_works association row (migration 0026).
async fn seed_space_work(tdb: &test_support::TestDb, space_id: &str, work_id: &str) {
    let cast = if tdb.is_postgres() { "::uuid" } else { "" };
    exec(
        tdb,
        &format!(
            "INSERT INTO space_works (space_id, work_id, position, created_at) \
             VALUES (?{cast}, ?{cast}, 1, '2026-09-01T00:00:00Z')"
        ),
        &[space_id, work_id],
    )
    .await;
}

/// Publish a work and then give it a content rating.
///
/// The rating is set after publication on purpose: it is the published state
/// that the doors filter on, and a rating applied before it would leave the
/// test unable to tell "filtered" from "not published yet".
async fn published_work_with_rating(client: &mut Client, title: &str, rating: &str) -> String {
    let (status, body) = client
        .post("/api/v1/works", json!({ "title": title }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    let work_id = body["id"].as_str().unwrap().to_owned();
    let version = body["version"].as_i64().unwrap();

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
    let (status, body) = client
        .patch(
            &format!("/api/v1/chapters/{chapter_id}"),
            json!({ "expected_version": chapter_version, "document": doc }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "save chapter: {body}");

    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/publish"),
            json!({ "expected_version": version, "idempotency_key": format!("m26-rating-{work_id}") }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "publish work: {body}");

    let (status, body) = client
        .patch(
            &format!("/api/v1/works/{work_id}"),
            json!({ "expected_version": version + 1, "rating": rating }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "set rating {rating}: {body}");

    work_id
}

/// An anonymous reader meets no adult item on the list or direct door.
#[tokio::test]
async fn anonymous_readers_never_see_explicit() {
    let dir = scratch_dir("anon-explicit");
    let tdb = test_support::TestDb::connect_with_dir("anon-explicit", &dir).await;
    let app = server::build_router(AppState::new(config_for(&dir), tdb.db().clone()));

    let mut seed = Client::new(app.clone());
    register(&mut seed, "author@example.com", "author").await;
    let explicit_id = published_work_with_rating(&mut seed, "Explicit Work", "explicit").await;
    let _general_id = published_work_with_rating(&mut seed, "General Work", "general").await;

    let mut anon = Client::new(app.clone());
    let (status, body) = anon.get("/api/v1/media").await;
    assert_eq!(status, StatusCode::OK, "media list: {body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "anonymous sees only general: {body}");
    assert_eq!(items[0]["title"].as_str().unwrap(), "General Work");

    // A direct door hides rather than refuses: an adult item is, to this
    // reader, an item that is not there (§7.6, §3.3).
    let (status, _) = anon.get(&format!("/api/v1/media/{explicit_id}")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "explicit work should be hidden from anon"
    );

    println!("PASS: anonymous_readers_never_see_explicit");
}

/// A reader whose content ceiling is below an item's rating meets no such item.
#[tokio::test]
async fn opted_out_readers_never_see_explicit() {
    let dir = scratch_dir("optout-explicit");
    let tdb = test_support::TestDb::connect_with_dir("optout-explicit", &dir).await;
    let app = server::build_router(AppState::new(config_for(&dir), tdb.db().clone()));

    let mut seed = Client::new(app.clone());
    register(&mut seed, "author@example.com", "author").await;
    let explicit_id = published_work_with_rating(&mut seed, "Explicit Work", "explicit").await;
    let _mature_id = published_work_with_rating(&mut seed, "Mature Work", "mature").await;
    let _general_id = published_work_with_rating(&mut seed, "General Work", "general").await;

    let mut reader = Client::new(app.clone());
    register(&mut reader, "reader@example.com", "reader").await;
    let (status, body) = reader
        .post(
            "/api/v1/settings/content-preferences",
            json!({ "max_rating": "teen" }),
        )
        .await;
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "set prefs: {body}"
    );

    let (status, body) = reader.get("/api/v1/media").await;
    assert_eq!(status, StatusCode::OK, "media list: {body}");
    let items = body["items"].as_array().expect("items");
    let has_explicit = items
        .iter()
        .any(|i| i["id"].as_str().unwrap() == explicit_id);
    assert!(
        !has_explicit,
        "teen-pref reader must not see explicit: {body}"
    );
    let has_general = items
        .iter()
        .any(|i| i["title"].as_str().unwrap() == "General Work");
    assert!(has_general, "teen-pref reader still sees general: {body}");

    // And the direct door answers for this reader as it does for anyone who
    // may not see the item.
    let (status, _) = reader.get(&format!("/api/v1/media/{explicit_id}")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "direct door for a teen ceiling"
    );

    println!("PASS: opted_out_readers_never_see_explicit");
}

/// Every door that can answer with a work — including the narration audio
/// door — applies the same eligibility rule, so no door is the hole the rest
/// of them are closed around.
#[tokio::test]
async fn media_rating_filters_apply_to_all_doors() {
    let dir = scratch_dir("rating-doors");
    let tdb = test_support::TestDb::connect_with_dir("rating-doors", &dir).await;
    // The silent engine so the narration half of this test can actually
    // produce audio: the point is which door serves it, not how it sounds.
    let state = AppState::new(config_with_silent_tts(&dir), tdb.db().clone());
    let app = server::build_router(state.clone());

    let mut seed = Client::new(app.clone());
    register(&mut seed, "author@example.com", "author").await;
    let explicit_id = published_work_with_rating(&mut seed, "Explicit", "explicit").await;
    let _mature_id = published_work_with_rating(&mut seed, "Mature", "mature").await;
    let _general_id = published_work_with_rating(&mut seed, "General", "general").await;

    let canon_id = seed_canon(&tdb, 100, "Mainline Canon").await;
    seed_canon_work(&tdb, &canon_id, &explicit_id).await;
    let space_id = seed_space(&tdb, 200, "Quiet Space").await;
    seed_space_work(&tdb, &space_id, &explicit_id).await;

    // A published narration of the explicit work. This is the door that used to
    // serve the audio to anyone who knew the edition id.
    let (status, body) = seed
        .post(&format!("/api/v1/works/{explicit_id}/editions"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "request narration: {body}");
    let edition_id = body["edition_id"].as_str().unwrap().to_owned();
    let report = run_worker_once(&state).await;
    assert!(
        report.job.is_some(),
        "the narration job must run: {report:?}"
    );
    let (status, body) = seed
        .post(&format!("/api/v1/editions/{edition_id}/approve"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "approve narration: {body}");

    let mut anon = Client::new(app.clone());

    // The list door: only the general work.
    let (status, body) = anon.get("/api/v1/media").await;
    assert_eq!(status, StatusCode::OK, "media list: {body}");
    let items = body["items"].as_array().expect("media items");
    assert_eq!(items.len(), 1, "list: only general: {body}");
    let has_explicit = items
        .iter()
        .any(|i| i["id"].as_str().unwrap() == explicit_id);
    assert!(!has_explicit, "anonymous must not see explicit in list");

    // The search door.
    let (status, body) = anon.get("/api/v1/search?q=work").await;
    assert_eq!(status, StatusCode::OK, "search: {body}");
    let results = body["items"].as_array().expect("search results");
    let has_explicit = results
        .iter()
        .any(|i| i["id"].as_str().unwrap() == explicit_id);
    assert!(!has_explicit, "anonymous must not see explicit in search");

    // The direct doors: media, files and editions all hide it.
    for door in [
        format!("/api/v1/media/{explicit_id}"),
        format!("/api/v1/media/{explicit_id}/files"),
        format!("/api/v1/media/{explicit_id}/editions"),
    ] {
        let (status, body) = anon.get(&door).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{door} leaked explicit: {body}"
        );
    }

    // The scoped doors filter the item out of the collection rather than out of
    // the response body (`canon_media` answers 200 with what the caller may see).
    for door in [
        format!("/api/v1/canons/{canon_id}/media"),
        format!("/api/v1/spaces/{space_id}/media"),
    ] {
        let (status, body) = anon.get(&door).await;
        assert_eq!(status, StatusCode::OK, "{door}: {body}");
        let items = body["items"].as_array().expect("scoped items");
        let leaked = items
            .iter()
            .any(|item| item["id"].as_str() == Some(explicit_id.as_str()));
        assert!(!leaked, "{door} leaked explicit: {body}");
    }

    // The narration doors: a published narration of an adult work is not a
    // public object. The audio door answers with the eligibility refusal rather
    // than the bytes (the same `CONTENT_RESTRICTED` the work's own page gives
    // this reader), and the metadata doors hide it entirely.
    let (status, body) = anon
        .get(&format!("/api/v1/editions/{edition_id}/audio"))
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the audio door served an explicit narration to an anonymous reader: {body}"
    );
    assert!(
        body.to_string().contains("CONTENT_RESTRICTED"),
        "the refusal must say why: {body}"
    );
    let (status, body) = anon.get(&format!("/api/v1/editions/{edition_id}")).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the edition metadata door served an explicit work: {body}"
    );
    let (status, body) = anon
        .get(&format!("/api/v1/works/{explicit_id}/editions"))
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the editions list served an explicit work: {body}"
    );

    // The author still sees and serves their own work on every one of them.
    let (status, _) = seed
        .get(&format!("/api/v1/editions/{edition_id}/audio"))
        .await;
    assert_eq!(status, StatusCode::OK, "the author fetches their own audio");
    let (status, body) = seed
        .get(&format!("/api/v1/works/{explicit_id}/editions"))
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the author lists their editions: {body}"
    );
    assert_eq!(body["editions"].as_array().unwrap().len(), 1, "{body}");

    println!("PASS: media_rating_filters_apply_to_all_doors");
}
