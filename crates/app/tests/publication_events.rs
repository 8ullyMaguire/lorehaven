//! M4-03 — the publication event log drives freshness and sort.
//!
//! # What this proves
//!
//! * **A work's publication age and its update age are different questions.**
//!   A work published a year ago and edited today has a publication age of a
//!   year and an update age of a day.
//! * **`new` and `updated` use the right one each**, and both come from the
//!   **event log** rather than from the row's `updated_at`.
//! * **An update writes an event.** Before this, the update path moved the
//!   work's `updated_at` and wrote nothing, so the `updated` sort had nothing
//!   to sort by and the row's timestamp was the only answer — which is the
//!   answer the specification says is wrong.
//!
//! # Why the event log and not the row
//!
//! `works.updated_at` is a single column with two meanings: when the work was
//! last touched, and when it was last *published*. A work edited today but
//! published a year ago has both, and they disagree — so a surface that sorts
//! by the column cannot answer "what is new?" correctly, and the two questions
//! need two sources.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use darkhold_app::config::Config;
use darkhold_app::server::{self, set_trust_proxy};
use darkhold_app::state::AppState;
use darkhold_db::content::{list_public_works_by_freshness, FreshnessSort};
use darkhold_db::DatabaseConfig;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "darkhold-pubevents-{tag}-{}-{:?}",
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

    async fn register(&mut self, email: &str, handle: &str) {
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

/// Create a work and publish it, returning its id.
///
/// **A work needs a unit with a document before it may be published** — the
/// domain's own publication readiness, and not this test's rule.
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

    // Publish is optimistic and names the version it expects, which the unit
    // moved on. Read it back rather than guessing.
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

/// Backdate a work's publish event, so "published a year ago" is a fact and
/// not a story.
async fn backdate_publication(harness: &Harness, work_id: &str, when: &str) {
    let sql = harness.tdb.db().sql(
        "UPDATE publication_events SET occurred_at = ? WHERE work_id = ? AND action = 'publish'",
        "UPDATE publication_events SET occurred_at = $1 WHERE work_id = $2::uuid AND action = 'publish'",
    );
    match harness.tdb.db().backend() {
        darkhold_db::Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(when)
                .bind(work_id)
                .execute(harness.tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("backdate the publication");
        }
        darkhold_db::Backend::Postgres => {
            sqlx::query(&sql)
                .bind(when)
                .bind(work_id)
                .execute(harness.tdb.db().postgres_pool().expect("postgres"))
                .await
                .expect("backdate the publication");
        }
    }
}

#[tokio::test]
async fn a_work_published_a_year_ago_and_edited_today_sorts_differently() {
    let harness = Harness::new("sorts").await;
    let mut client = harness.client();
    client.register("author@example.test", "Author").await;

    // **The old work**: published a year ago, edited today.
    let old = published_work(&mut client, "Published a year ago").await;
    backdate_publication(&harness, &old, "2025-10-20T00:00:00Z").await;

    let (status, body) = client.get(&format!("/api/v1/works/{old}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let version = body["version"].as_i64().unwrap_or(1);
    let (status, body) = client
        .patch(
            &format!("/api/v1/works/{old}"),
            json!({ "expected_version": version, "summary": "Edited today." }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "edit the old work: {body}");

    // **The new work**: published today, not edited since.
    let new = published_work(&mut client, "Published today").await;

    // **`new` sorts by the publication age.** The old work is a year old, so it
    // comes after the new one.
    let by_new = list_public_works_by_freshness(harness.tdb.db(), FreshnessSort::New, 50)
        .await
        .expect("list by publication age");
    let new_position = by_new.iter().position(|id| id.to_string() == new);
    let old_position = by_new.iter().position(|id| id.to_string() == old);
    assert!(
        new_position.is_some() && old_position.is_some(),
        "both works are public: {by_new:?}"
    );
    assert!(
        new_position < old_position,
        "under 'new' the work published today comes first: {by_new:?}"
    );

    // **`updated` sorts by the update age.** The old work was edited today and
    // the new one was not, so the order *reverses* — and that reversal is the
    // whole point of the two questions being separate.
    let by_updated = list_public_works_by_freshness(harness.tdb.db(), FreshnessSort::Updated, 50)
        .await
        .expect("list by update age");
    let new_position = by_updated.iter().position(|id| id.to_string() == new);
    let old_position = by_updated.iter().position(|id| id.to_string() == old);
    assert!(
        new_position.is_some() && old_position.is_some(),
        "both works are public: {by_updated:?}"
    );
    assert!(
        old_position < new_position,
        "under 'updated' the work edited today comes first, and the order reverses: \
         {by_updated:?}"
    );
}

#[tokio::test]
async fn an_update_writes_an_event() {
    // **The event log is the answer to "when was this changed?"** Before this,
    // the update path moved `updated_at` and wrote nothing, so the log had no
    // update in it and the `updated` sort had nothing to sort by.
    let harness = Harness::new("event").await;
    let mut client = harness.client();
    client.register("author2@example.test", "Author2").await;

    let id = published_work(&mut client, "A Work").await;
    let (status, body) = client.get(&format!("/api/v1/works/{id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let version = body["version"].as_i64().unwrap_or(1);

    let (status, body) = client
        .patch(
            &format!("/api/v1/works/{id}"),
            json!({ "expected_version": version, "summary": "A blurb." }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // The event log carries the update.
    let freshness =
        darkhold_db::content::work_freshness(harness.tdb.db(), id.parse().expect("work id"))
            .await
            .expect("read the freshness");
    assert!(
        freshness.updated_at.is_some(),
        "the update wrote an event: {freshness:?}"
    );
    assert!(
        freshness.published_at.is_some(),
        "and the publication did too: {freshness:?}"
    );
}
