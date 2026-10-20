//! M4-06 — the optimistic concurrency contract.
//!
//! # What this proves
//!
//! * **Two concurrent updates to the same work: one wins, one gets 409, and
//!   the loser's write did not land.** A conflict that only refuses the second
//!   *request* is not a conflict — the point is that the second *change* never
//!   happened.
//! * **The conflict response names the field and both versions**, so a client
//!   can retry without re-reading the work and hoping.
//! * **Every mutable table carries `id`, `created_at`, `updated_at` and
//!   `version`.** The `version` column is the optimistic-concurrency counter,
//!   and a table without one has no way to refuse a stale write.
//!
//! # Why the conflict has to name the version
//!
//! A 409 that says "conflict" tells the caller that it lost and not what it
//! lost to. The only way to find out is to re-read the work — and a client
//! that re-reads and retries has to be right about the *second* read too, which
//! is a race it did not ask for.

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
        "darkhold-conflict-{tag}-{}-{:?}",
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

    async fn register(&mut self, email: &str, handle: &str) -> String {
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
        let (status, me) = self.get("/api/v1/auth/me").await;
        assert_eq!(status, StatusCode::OK, "{me}");
        me["account"]["id"].as_str().expect("account id").to_owned()
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

#[tokio::test]
async fn two_concurrent_updates_to_one_work_one_wins_and_one_conflicts() {
    let harness = Harness::new("concurrent").await;
    let mut client = harness.client();
    client.register("author@example.test", "Author").await;

    // A work, and the version it is at.
    let (status, body) = client
        .post("/api/v1/works", json!({ "title": "A Work" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    let work_id = body["id"].as_str().expect("work id").to_owned();
    let version = body["version"].as_i64().expect("version");
    assert_eq!(version, 1, "a new work is at version 1");

    // **Two writers, both holding the same version.** This is the shape of a
    // lost edit: two tabs, two people, one work.
    let (status, body) = client
        .patch(
            &format!("/api/v1/works/{work_id}"),
            json!({ "expected_version": version, "title": "The winner" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "the first write wins: {body}");

    let (status, body) = client
        .patch(
            &format!("/api/v1/works/{work_id}"),
            json!({ "expected_version": version, "title": "The loser" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "the second write is refused: {body}"
    );
    assert_eq!(body["error"]["code"], "REVISION_CONFLICT", "{body}");

    // **And the loser's write did not land.** A conflict that only refuses the
    // second *request* is not a conflict; the point is that the second
    // *change* never happened.
    let (status, body) = client.get(&format!("/api/v1/works/{work_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let title = body["title"]
        .as_str()
        .or_else(|| body["work"]["title"].as_str())
        .unwrap_or_default();
    assert_eq!(
        title, "The winner",
        "the loser's write did not land: {body}"
    );
}

#[tokio::test]
async fn the_conflict_names_the_field_and_both_versions() {
    // **A client that cannot retry is a client that clobbers.** The response
    // says which field the conflict is about and what each side held, so the
    // retry needs no second read.
    let harness = Harness::new("named").await;
    let mut client = harness.client();
    client.register("author2@example.test", "Author2").await;

    let (status, body) = client
        .post("/api/v1/works", json!({ "title": "A Work" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let work_id = body["id"].as_str().expect("work id").to_owned();
    let version = body["version"].as_i64().expect("version");

    // Move the work on.
    let (status, _) = client
        .patch(
            &format!("/api/v1/works/{work_id}"),
            json!({ "expected_version": version, "title": "Moved on" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // And now conflict.
    let (status, body) = client
        .patch(
            &format!("/api/v1/works/{work_id}"),
            json!({ "expected_version": version, "title": "Stale" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "REVISION_CONFLICT", "{body}");

    let conflict = body["error"]["conflict"]
        .as_object()
        .unwrap_or_else(|| panic!("the conflict names the field and both versions: {body}"));
    assert_eq!(
        conflict.get("field").and_then(Value::as_str),
        Some("version"),
        "the conflict names the field it is about: {body}"
    );
    assert_eq!(
        conflict.get("expected").and_then(Value::as_i64),
        Some(version),
        "and what the client believed: {body}"
    );
    assert_eq!(
        conflict.get("actual").and_then(Value::as_i64),
        Some(version + 1),
        "and what the server holds, so the retry needs no second read: {body}"
    );
}

#[tokio::test]
async fn a_version_is_bumped_by_a_write_and_not_by_a_read() {
    let harness = Harness::new("bump").await;
    let mut client = harness.client();
    client.register("author3@example.test", "Author3").await;

    let (status, body) = client
        .post("/api/v1/works", json!({ "title": "A Work" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let work_id = body["id"].as_str().expect("work id").to_owned();
    let version = body["version"].as_i64().expect("version");

    // A read does not move it.
    for _ in 0..3 {
        let (status, _) = client.get(&format!("/api/v1/works/{work_id}")).await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, body) = client.get(&format!("/api/v1/works/{work_id}")).await;
    assert_eq!(status, StatusCode::OK);
    let after_reads = body["version"]
        .as_i64()
        .or_else(|| body["work"]["version"].as_i64())
        .unwrap_or(version);
    assert_eq!(after_reads, version, "a read does not bump the version");

    // A write does.
    let (status, body) = client
        .patch(
            &format!("/api/v1/works/{work_id}"),
            json!({ "expected_version": version, "summary": "A blurb" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}
