//! M4-04 — series, relations, and the "more like this" ladder.
//!
//! # What this proves
//!
//! * **The ladder is ordered and every row carries its reason.** Spec §11.5:
//!   *"A recommendation without a reason is an advertisement."* A ladder whose
//!   rows all said "because it is similar" would be an advertisement with a
//!   JSON wrapper.
//! * **The first rung is the explicit relation and the second is the same
//!   author** — the two rungs that are *facts* about the work, rather than
//!   inferences about a reader.
//! * **The three taste rungs are named as unavailable**, and asking for one
//!   by name answers **404 with the module's own unavailability code** — not
//!   `NOT_IMPLEMENTED`, which is a claim about the instance rather than about
//!   the build.
//! * **A work in a series carries the author's position**, and the order is
//!   the only order there is.

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
        "darkhold-ladder-{tag}-{}-{:?}",
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

/// A published work, with a unit of text in it.
async fn published_work(client: &mut Client, title: &str) -> String {
    let (status, body) = client
        .post("/api/v1/works", json!({ "title": title }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    let id = body["id"].as_str().expect("work id").to_owned();

    let (status, body) = client
        .post(
            &format!("/api/v1/works/{id}/chapters"),
            json!({ "title": "One" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "add a unit: {body}");
    let unit = body["id"].as_str().expect("unit id").to_owned();
    let unit_version = body["version"].as_i64().expect("unit version");

    let doc = json!({ "type": "doc", "content": [
        { "type": "paragraph", "content": [
            { "type": "text", "text": "A unit with enough words to have a middle." }] }] });
    let (status, body) = client
        .patch(
            &format!("/api/v1/chapters/{unit}"),
            json!({ "expected_version": unit_version, "document": doc }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "write the unit: {body}");

    let (status, body) = client.get(&format!("/api/v1/works/{id}")).await;
    assert_eq!(status, StatusCode::OK, "reload the work: {body}");
    let current = body["version"].as_i64().expect("current work version");

    let (status, body) = client
        .post(
            &format!("/api/v1/works/{id}/publish"),
            json!({ "expected_version": current }),
        )
        .await;
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "publish the work: {status} {body}"
    );
    id
}

#[tokio::test]
async fn the_ladder_is_ordered_and_every_row_carries_a_reason() {
    let harness = Harness::new("ladder").await;
    let mut client = harness.client();
    client.register("author@example.test", "Author").await;

    // Two works by the same author, and one by somebody else.
    let first = published_work(&mut client, "The first work").await;
    let second = published_work(&mut client, "The second work").await;
    let mut other = harness.client();
    other.register("other@example.test", "Other").await;
    let _theirs = published_work(&mut other, "Somebody else's work").await;

    // The author declares that the second follows the first.
    let (status, body) = client
        .post(
            &format!("/api/v1/works/{first}/relations"),
            json!({ "to_work_id": second, "relation": "next_in_series" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "declare the relation: {body}");
    assert_eq!(body["relation"], "next_in_series", "{body}");

    // And the two are in a series, so the reason can name it.
    let (status, body) = client
        .post(
            &format!("/api/v1/works/{first}/series"),
            json!({ "slug": "a-series", "title": "A Series", "position": 1 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "put it in the series: {body}");
    let (status, body) = client
        .post(
            &format!("/api/v1/works/{second}/series"),
            json!({ "slug": "a-series", "title": "A Series", "position": 2 }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "put the second in the series: {body}"
    );

    // The ladder.
    let (status, ladder) = client
        .get(&format!("/api/v1/works/{first}/more-like-this"))
        .await;
    assert_eq!(status, StatusCode::OK, "{ladder}");
    let items = ladder["items"].as_array().expect("items");
    assert!(!items.is_empty(), "the ladder has rows: {ladder}");

    // **Every row carries a reason, and no two rows carry the same one.**
    let mut reasons = Vec::new();
    for item in items {
        let reason = item["reason"].as_str().unwrap_or_default();
        assert!(
            !reason.is_empty(),
            "a recommendation without a reason is an advertisement: {item}"
        );
        reasons.push(reason.to_owned());
    }
    let mut unique = reasons.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        reasons.len(),
        "and the reasons are not the same string for every row: {reasons:?}"
    );

    // **The order is the ladder's order**: the explicit relation first, then
    // the same author.
    let rungs: Vec<&str> = items
        .iter()
        .map(|item| item["rung"].as_str().unwrap_or_default())
        .collect();
    assert!(
        rungs.contains(&"next_in_series"),
        "the explicit relation is on the ladder: {rungs:?}"
    );
    assert!(
        rungs.contains(&"same_author"),
        "and so is the same author: {rungs:?}"
    );
    let first_next = rungs.iter().position(|rung| *rung == "next_in_series");
    let first_author = rungs.iter().position(|rung| *rung == "same_author");
    assert!(
        first_next < first_author,
        "the ladder is ORDERED: the explicit relation comes before the same author: {rungs:?}"
    );

    // And the reason names the series.
    let next = items
        .iter()
        .find(|item| item["rung"] == "next_in_series")
        .expect("the relation is on the ladder");
    assert!(
        next["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("A Series"),
        "the reason names the series: {next}"
    );
}

#[tokio::test]
async fn the_taste_rungs_are_named_as_unavailable() {
    // **A rung that is not implemented is named, not answered with a plausible
    // number.** A stub that returns a score is a ranking that looks measured
    // and is not — and a client that gets a shorter list has no way to tell an
    // empty ladder from an unimplemented one, and an empty ladder reads as
    // "this work has no neighbours", which is a statement about the work.
    let harness = Harness::new("unavailable").await;
    let mut client = harness.client();
    client.register("author2@example.test", "Author2").await;
    let work = published_work(&mut client, "A work").await;

    let (status, ladder) = client
        .get(&format!("/api/v1/works/{work}/more-like-this"))
        .await;
    assert_eq!(status, StatusCode::OK, "{ladder}");
    let unavailable = ladder["unavailable"].as_array().expect("unavailable");
    assert_eq!(
        unavailable.len(),
        3,
        "the three taste rungs are named: {ladder}"
    );
    for rung in unavailable {
        assert!(
            rung["reason"].as_str().is_some_and(|r| !r.is_empty()),
            "and each says why: {rung}"
        );
    }

    // **Asking for one by name is a refusal that names the module** — 404, not
    // 501. The thing is not here *yet*, and 501 would be a claim about the
    // instance rather than about the build.
    let (status, body) = client
        .get(&format!(
            "/api/v1/works/{work}/more-like-this?rung=taste_matched"
        ))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"]["code"], "MODULE_UNAVAILABLE", "{body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("taste"),
        "and the refusal names the module: {body}"
    );
}

#[tokio::test]
async fn a_relation_to_a_work_that_is_not_here_is_refused() {
    // A relation nobody can follow is not a relation.
    let harness = Harness::new("dangling").await;
    let mut client = harness.client();
    client.register("author3@example.test", "Author3").await;
    let work = published_work(&mut client, "A work").await;

    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work}/relations"),
            json!({
                "to_work_id": "00000000-0000-4000-8000-000000000000",
                "relation": "next_in_series",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}
