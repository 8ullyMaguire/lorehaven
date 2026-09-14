//! M17 — Translation pipeline: jobs, units, memory, glossaries, reviews, publications.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::{Database, DatabaseConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m17-{}-{:?}-{:?}",
        tag,
        std::process::id(),
        std::thread::current().id(),
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

struct Client {
    app: axum::Router,
    cookies: Vec<(String, String)>,
}

impl Client {
    fn new(app: axum::Router) -> Self {
        Self { app, cookies: Vec::new() }
    }
    fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    fn capture(&mut self, response: &axum::response::Response) {
        for value in response.headers().get_all(header::SET_COOKIE) {
            let Ok(text) = value.to_str() else { continue; };
            let Some((pair, _)) = text.split_once(';') else { continue; };
            if let Some((name, value)) = pair.split_once('=') {
                let name = name.trim().to_owned();
                let value = value.trim().to_owned();
                self.cookies.retain(|(k, _)| k != &name);
                if !value.is_empty() { self.cookies.push((name, value)); }
            }
        }
    }
    async fn request(&mut self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if !self.cookies.is_empty() {
            builder = builder.header(header::COOKIE, self.cookies.iter().map(|(n, v)| format!("{n}={v}")).collect::<Vec<_>>().join("; "));
        }
        if !matches!(method, "GET" | "HEAD" | "OPTIONS") {
            if let Some(token) = self.cookie("lorehaven_csrf").map(str::to_owned) {
                builder = builder.header("x-csrf-token", token);
            }
        }
        let request = match body {
            Some(v) => builder.header(header::CONTENT_TYPE, "application/json").body(Body::from(serde_json::to_vec(&v).expect("serialise"))).expect("request"),
            None => builder.body(Body::empty()).expect("request"),
        };
        let response = self.app.clone().oneshot(request).await.expect("response");
        let status = response.status();
        self.capture(&response);
        let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024).await.expect("body");
        let value = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into_owned())) };
        (status, value)
    }
    async fn get(&mut self, uri: &str) -> (StatusCode, Value) { self.request("GET", uri, None).await }
    async fn post(&mut self, uri: &str, body: Value) -> (StatusCode, Value) { self.request("POST", uri, Some(body)).await }
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
        let db = Database::connect(&DatabaseConfig::new(format!("sqlite://{}/lorehaven.sqlite?mode=rwc", dir.display()))).await.expect("connect");
        let _ = db.migrate().await.expect("migrate");
        Self { dir, db }
    }
    fn db(&self) -> &Database { &self.db }
    fn client(&self) -> Client { Client::new(server::build_router(AppState::new(config_for(&self.dir), self.db.clone()))) }
    async fn cleanup(self) { self.db.close().await; let _ = std::fs::remove_dir_all(self.dir); }
}

const PASSWORD: &str = "a-long-enough-passphrase";

async fn register(client: &mut Client, email: &str, handle: &str) {
    let (status, body) = client.post("/api/v1/auth/register", json!({ "email": email, "password": PASSWORD, "handle": handle, "display_name": handle, "age_band": "adult" })).await;
    assert_eq!(status, StatusCode::CREATED, "register {handle}: {body}");
    let _account = body["account"].as_object().expect("account object");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_translation_job_can_be_created_and_transitioned() {
    let harness = Harness::new("job-create").await;
    let _client = harness.client();

    let job_id = lorehaven_db::translation::create_job(
        harness.db(), "work-123", "en", "es", "machine:dev:1.0", None, "author-1",
    ).await.expect("create job");

    assert!(!job_id.is_empty());

    // Verify initial state is "quoted"
    let job = lorehaven_db::translation::get_job(harness.db(), &job_id).await.expect("get job").expect("job exists");
    assert_eq!(job["state"].as_str().unwrap(), "quoted");

    // Transition: quoted -> reserved
    lorehaven_db::translation::transition_job(
        harness.db(), &job_id,
        &lorehaven_domain::translation::TranslationJobState::Quoted,
        &lorehaven_domain::translation::TranslationJobState::Reserved,
    ).await.expect("transition to reserved");

    let job = lorehaven_db::translation::get_job(harness.db(), &job_id).await.expect("get job").expect("job exists");
    assert_eq!(job["state"].as_str().unwrap(), "reserved");

    harness.cleanup().await;
}

#[tokio::test]
async fn translation_units_can_be_upserted_and_listed() {
    let harness = Harness::new("units").await;
    let _client = harness.client();

    let job_id = lorehaven_db::translation::create_job(
        harness.db(), "work-456", "en", "fr", "human", None, "author-2",
    ).await.expect("create job");

    // Upsert a unit
    lorehaven_db::translation::upsert_unit(
        harness.db(), &job_id, "ch-1", 0, "Hello world", Some("Bonjour le monde"),
        &lorehaven_domain::translation::TranslationUnitState::Translated, None,
    ).await.expect("upsert unit");

    // Upsert another unit
    lorehaven_db::translation::upsert_unit(
        harness.db(), &job_id, "ch-1", 1, "How are you?", Some("Comment allez-vous?"),
        &lorehaven_domain::translation::TranslationUnitState::Pending, None,
    ).await.expect("upsert unit 2");

    let units = lorehaven_db::translation::units_for_job(harness.db(), &job_id).await.expect("units");
    assert_eq!(units.len(), 2);
    assert_eq!(units[0]["paragraph_index"].as_i64().unwrap(), 0);
    assert_eq!(units[1]["paragraph_index"].as_i64().unwrap(), 1);

    harness.cleanup().await;
}

#[tokio::test]
async fn translation_memory_can_be_added_and_looked_up() {
    let harness = Harness::new("memory").await;
    let _client = harness.client();

    let hash = lorehaven_domain::translation::paragraph_hash("The quick brown fox");

    lorehaven_db::translation::add_memory(
        harness.db(), "author-1", "en", "es", &hash, "The quick brown fox", "El rápido zorro marrón", 8000, false,
    ).await.expect("add memory");

    let result = lorehaven_db::translation::lookup_memory(
        harness.db(), "author-1", "en", "es", &hash,
    ).await.expect("lookup memory");

    assert!(result.is_some());
    let (target, quality, shared) = result.unwrap();
    assert_eq!(target, "El rápido zorro marrón");
    assert_eq!(quality, 8000);
    assert!(!shared);

    harness.cleanup().await;
}

#[tokio::test]
async fn glossary_term_can_be_added() {
    let harness = Harness::new("glossary").await;
    let _client = harness.client();

    let id = lorehaven_db::translation::add_glossary_term(
        harness.db(), "author-1", None, "en", "es", "dragon", "dragón", false,
    ).await.expect("add glossary term");

    assert!(!id.is_empty());

    // Test glossary application
    let text = "The dragon flew over the dragon mountain";
    let glossary = vec![("dragon".to_string(), "dragón".to_string())];
    let result = lorehaven_domain::translation::apply_glossary(text, &glossary, true);
    assert_eq!(result, "The dragón flew over the dragón mountain");

    harness.cleanup().await;
}

#[tokio::test]
async fn review_gate_can_be_opened_and_decided() {
    let harness = Harness::new("review").await;
    let _client = harness.client();

    let job_id = lorehaven_db::translation::create_job(
        harness.db(), "work-789", "en", "de", "machine:dev:1.0", None, "author-3",
    ).await.expect("create job");

    let review_id = lorehaven_db::translation::open_review_gate(
        harness.db(), &job_id, "reviewer-1", &lorehaven_domain::translation::ReviewGate::Linguistic,
    ).await.expect("open review gate");

    assert!(!review_id.is_empty());

    // Decide the review
    lorehaven_db::translation::decide_review_gate(
        harness.db(), &review_id, "approved", Some("Looks good!"),
    ).await.expect("decide review");

    harness.cleanup().await;
}

#[tokio::test]
async fn publication_can_be_recorded() {
    let harness = Harness::new("publication").await;
    let _client = harness.client();

    let job_id = lorehaven_db::translation::create_job(
        harness.db(), "work-abc", "en", "ja", "human", None, "author-4",
    ).await.expect("create job");

    let pub_id = lorehaven_db::translation::record_publication(
        harness.db(), &job_id, "work-abc-ja",
    ).await.expect("record publication");

    assert!(!pub_id.is_empty());

    harness.cleanup().await;
}
