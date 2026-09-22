//! M17 Phase 2 — Health Layer (spec §0.4.1, §0.4.2, §16.19).
//!
//! Tests cover the admin taste profile, per-work taste vector derivation from
//! tags, the onboarding quiz (pick → vector → centroid distance), and taste
//! probe engagement recording.

use std::path::PathBuf;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::DatabaseConfig;
use serde_json::{json, Value};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-taste-health-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn config_for(dir: &PathBuf) -> Config {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    config.database = DatabaseConfig::new(format!(
        "sqlite://{}/lorehaven.sqlite?mode=rwc",
        dir.display()
    ));
    config
}

struct Harness {
    _dir: PathBuf,
    tdb: test_support::TestDb,
    config: Config,
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
        let config = config_for(&dir);
        Self { _dir: dir, tdb, config }
    }

    fn client(&self) -> Client {
        Client::new(server::build_router(AppState::new(
            self.config.clone(),
            self.tdb.db().clone(),
        )))
    }
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
        self.cookies
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
    fn capture(&mut self, response: &axum::response::Response) {
        for value in response.headers().get_all(header::SET_COOKIE) {
            let Ok(text) = value.to_str() else { continue };
            let Some((pair, _)) = text.split_once(';') else { continue };
            if let Some((name, value)) = pair.split_once('=') {
                let name = name.trim().to_owned();
                let value = value.trim().to_owned();
                self.cookies.retain(|(k, _)| k != &name);
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
            if let Some(token) = self.cookie("lorehaven_csrf").map(str::to_owned) {
                builder = builder.header("x-csrf-token", token);
            }
        }
        let request = match body {
            Some(v) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&v).unwrap()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        let response = self.app.clone().oneshot(request).await.unwrap();
        self.capture(&response);
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(json!({}));
        (status, value)
    }
    async fn get(&mut self, uri: impl AsRef<str>) -> (StatusCode, Value) {
        self.request("GET", uri.as_ref(), None).await
    }
    async fn post(&mut self, uri: impl AsRef<str>, body: Value) -> (StatusCode, Value) {
        self.request("POST", uri.as_ref(), Some(body)).await
    }
}

const PASSWORD: &str = "a-long-enough-passphrase";

async fn register(client: &mut Client, email: &str, handle: &str) -> (String, String) {
    let (status, body) = client
        .post(
            "/api/v1/auth/register",
            json!({
                "email": email,
                "password": PASSWORD,
                "handle": handle,
                "display_name": handle,
                "age_band": "adult"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "register {handle}: {body}");
    let (status, me) = client.get("/api/v1/auth/me").await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let account = me["account"]["id"].as_str().expect("account id").to_owned();
    let pseud = me["active_pseud_id"].as_str().expect("pseud id").to_owned();
    (account, pseud)
}

/// Create a work via the API, returning its work_id.
async fn create_work(client: &mut Client, title: &str) -> String {
    let (status, body) = client
        .post(
            "/api/v1/works",
            json!({ "title": title, "summary": "test work", "rating": "general" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    body["id"].as_str().expect("work id").to_owned()
}

#[tokio::test]
async fn admin_taste_profile_save_and_read_roundtrip() {
    let harness = Harness::new("taste-profile-roundtrip").await;
    let db = harness.tdb.db().clone();

    let dimensions = vec![
        ("angst".to_string(), "Angst".to_string(), 0.8, 1.0),
        ("pacing".to_string(), "Pacing".to_string(), 0.3, 0.5),
    ];
    lorehaven_db::taste_health::save_admin_taste_profile(&db, &dimensions)
        .await
        .expect("save");

    let stored = lorehaven_db::taste_health::get_admin_taste_profile(&db)
        .await
        .expect("read");
    assert_eq!(stored.len(), 2);
    assert_eq!(stored[0].0, "angst");
    assert!((stored[0].2 - 0.8).abs() < 1e-9);
    assert_eq!(stored[1].0, "pacing");
    assert!((stored[1].3 - 0.5).abs() < 1e-9);
}

#[tokio::test]
async fn work_vector_derived_from_matching_tag() {
    let harness = Harness::new("work-vector-tags").await;
    let db = harness.tdb.db().clone();

    let dimensions = vec![
        ("angst".to_string(), "Angst".to_string(), 0.5, 1.0),
        ("pacing".to_string(), "Pacing".to_string(), 0.5, 1.0),
    ];
    lorehaven_db::taste_health::save_admin_taste_profile(&db, &dimensions)
        .await
        .expect("save profile");

    // Create a work and tag it "angst" with weight 50.
    let mut client = harness.client();
    let (_account, _pseud) = register(&mut client, "tag-user@t.test", "taguser").await;
    let work_id = create_work(&mut client, "tagged fic").await;

    // Create the taxonomy node first, then tag the work with its node_id.
    let (status, body) = client
        .post(
            "/api/v1/taxonomy",
            json!({ "kind": "tag", "canonical": "angst" }),
        )
        .await;
    assert!(status == StatusCode::CREATED || status == StatusCode::OK, "create taxonomy node: {body}");
    let node_id = body["node"]["id"].as_str().expect("node id").to_owned();

    let (status, _body) = client
        .post(
            format!("/api/v1/works/{work_id}/tags"),
            json!({ "node_id": node_id, "weight": 50 }),
        )
        .await;
    assert!(status == StatusCode::CREATED || status == StatusCode::OK, "tag work");

    let vector = lorehaven_db::taste_health::compute_and_store_work_vector(&db, &work_id)
        .await
        .expect("compute vector");
    // angst dimension should rise from neutral 0.5 by weight/100 = 0.5 → 1.0 clamped.
    assert_eq!(vector.len(), 2);
    assert!((vector[0] - 1.0).abs() < 1e-9, "angst dim: {vector:?}");
    // pacing stays neutral.
    assert!((vector[1] - 0.5).abs() < 1e-9, "pacing dim: {vector:?}");
}

#[tokio::test]
async fn quiz_picks_compute_initial_vector() {
    let harness = Harness::new("quiz-picks").await;
    let db = harness.tdb.db().clone();
    let mut client = harness.client();

    // Seed dimensions and a tagged work.
    let dimensions = vec![
        ("angst".to_string(), "Angst".to_string(), 0.9, 1.0),
        ("pacing".to_string(), "Pacing".to_string(), 0.1, 1.0),
    ];
    lorehaven_db::taste_health::save_admin_taste_profile(&db, &dimensions)
        .await
        .expect("save profile");
    let (account, _pseud) = register(&mut client, "quiz-user@t.test", "quizuser").await;
    let work_id = create_work(&mut client, "quiz seed").await;

    let (status, body) = client
        .post(
            "/api/v1/taxonomy",
            json!({ "kind": "tag", "canonical": "angst" }),
        )
        .await;
    assert!(status == StatusCode::CREATED || status == StatusCode::OK, "create taxonomy node: {body}");
    let node_id = body["node"]["id"].as_str().expect("node id").to_owned();
    let (status, _) = client
        .post(
            format!("/api/v1/works/{work_id}/tags"),
            json!({ "node_id": node_id, "weight": 100 }),
        )
        .await;
    assert!(status.is_success() || status == StatusCode::OK);

    // Compute the work's vector so the quiz can pick it.
    lorehaven_db::taste_health::compute_and_store_work_vector(&db, &work_id)
        .await
        .expect("compute work vector for quiz");

    // Pick it.
    let (status, body) = client
        .post(
            "/api/v1/quiz/answers",
            json!({ "picked": [work_id], "rejected": [] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "quiz answers: {body}");
    assert_eq!(body["vector_dimensions"], 2);

    // The account now has a stored taste vector with a finite centroid distance.
    let stored = lorehaven_db::taste_vectors::get_taste_vector(&db, &account)
        .await
        .expect("get vector");
    let (vector, distance, _at) = stored.expect("vector present");
    assert_eq!(vector.len(), 2);
    assert!(distance.is_finite());
    assert!(distance >= 0.0);

    // Answers are readable back.
    let (status, body) = client.get("/api/v1/quiz/answers").await;
    assert_eq!(status, StatusCode::OK, "quiz answers read: {body}");
    assert_eq!(body["picked"][0], work_id);
}

#[tokio::test]
async fn quiz_skip_leaves_account_without_vector() {
    let harness = Harness::new("quiz-skip").await;
    let db = harness.tdb.db().clone();
    let mut client = harness.client();
    let (account, _pseud) = register(&mut client, "skip-user@t.test", "skipuser").await;

    let (status, body) = client.post("/api/v1/quiz/skip", json!({})).await;
    assert_eq!(status, StatusCode::OK, "skip: {body}");
    assert_eq!(body["status"], "skipped");

    // No vector stored: the account row exists but taste_vector is empty and
    // taste_vector_computed_at is NULL.
    let stored = lorehaven_db::taste_vectors::get_taste_vector(&db, &account)
        .await
        .expect("get vector");
    match stored {
        Some((ref v, _d, ref at)) => {
            assert!(v.is_empty(), "skipped quiz should not store a non-empty vector");
            assert!(at.is_empty(), "skipped quiz should not store a computed_at");
        }
        None => {}
    }
}

#[tokio::test]
async fn probe_engagement_roundtrip() {
    let harness = Harness::new("probe-engagement").await;
    let db = harness.tdb.db().clone();
    let mut client = harness.client();
    let (account, _pseud) = register(&mut client, "probe-user@t.test", "probeuser").await;
    let work_id = create_work(&mut client, "probe target").await;

    lorehaven_db::taste_health::record_probe_engagement(&db, &account, &work_id, "positive")
        .await
        .expect("record");
    // Re-recording updates engagement (upsert).
    lorehaven_db::taste_health::record_probe_engagement(&db, &account, &work_id, "negative")
        .await
        .expect("update");

    let engagements = lorehaven_db::taste_health::get_probe_engagements(&db, &account)
        .await
        .expect("read");
    assert_eq!(engagements.len(), 1);
    assert_eq!(engagements[0].0, work_id);
    assert_eq!(engagements[0].1, "negative");
}
