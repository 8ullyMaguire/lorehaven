//! Milestone 4 acceptance tests (spec §9).
//!
//! These run against the real router, a real SQLite file and a cookie jar.
//! The harness is copied from `milestone_3.rs` and extended for the reader's
//! private data: a second pseud on the same account, a second browser's cookie
//! jar, and signed-out visitors.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::{reading, DatabaseConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m4-{tag}-{}-{:?}",
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

    async fn put(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("PUT", uri, Some(body)).await
    }

    async fn patch(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("PATCH", uri, Some(body)).await
    }

    async fn delete(&mut self, uri: &str) -> (StatusCode, Value) {
        self.request("DELETE", uri, None).await
    }
}

struct Harness {
    dir: PathBuf,
    tdb: test_support::TestDb,
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
        let report: Vec<String> = tdb.applied_migrations().to_vec();
        assert!(
            report.contains(&"0004_reading".to_owned()),
            "the reading migration must apply: {report:?}"
        );

        Self { dir, tdb }
    }

    fn client(&self) -> Client {
        Client::new(server::build_router(AppState::new(
            config_for(&self.dir),
            self.tdb.db().clone(),
        )))
    }

    async fn cleanup(self) {
        self.tdb.cleanup().await;
        let _ = std::fs::remove_dir_all(self.dir);
    }
}

const PASSWORD: &str = "a-long-enough-passphrase";

async fn register(client: &mut Client, email: &str, handle: &str) -> String {
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
        .to_owned()
}

async fn active_pseud(client: &mut Client) -> String {
    let (status, body) = client.get("/api/v1/auth/me").await;
    assert_eq!(status, StatusCode::OK, "me: {body}");
    body["active_pseud_id"]
        .as_str()
        .expect("active pseud id")
        .to_owned()
}

async fn create_work(client: &mut Client, title: &str) -> Value {
    let (status, body) = client
        .post("/api/v1/works", json!({ "title": title }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    body
}

async fn add_chapter(client: &mut Client, work: &str, title: &str) -> (String, i64) {
    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work}/chapters"),
            json!({ "title": title }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "add chapter: {body}");
    (
        body["id"].as_str().expect("chapter id").to_owned(),
        body["version"].as_i64().expect("version"),
    )
}

fn document(text: &str) -> Value {
    json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": text }],
        }],
    })
}

async fn save_chapter(
    client: &mut Client,
    chapter: &str,
    version: i64,
    text: &str,
) -> (StatusCode, Value) {
    client
        .patch(
            &format!("/api/v1/chapters/{chapter}"),
            json!({ "expected_version": version, "document": document(text) }),
        )
        .await
}

async fn publish(client: &mut Client, work: &str, version: i64) -> (StatusCode, Value) {
    client
        .post(
            &format!("/api/v1/works/{work}/publish"),
            json!({ "expected_version": version, "idempotency_key": format!("m4-publish-{work}-{version}") }),
        )
        .await
}

// ---------------------------------------------------------------------------
// A visitor can read a published chapter, and a draft is a 404 to a stranger
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_visitor_can_read_a_published_chapter() {
    let harness = Harness::new("visitor-reads").await;
    let mut author = harness.client();
    register(&mut author, "author@example.com", "Quill").await;

    let work = create_work(&mut author, "The Open Road").await;
    let work_id = work["id"].as_str().expect("id").to_owned();
    let (chapter, version) = add_chapter(&mut author, &work_id, "One").await;
    save_chapter(
        &mut author,
        &chapter,
        version,
        "The road was open and the sky was wide.",
    )
    .await;
    publish(&mut author, &work_id, 1).await;

    let mut visitor = harness.client();
    let (status, body) = visitor
        .get(&format!("/api/v1/works/{work_id}/chapters/{chapter}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["sanitized_html"],
        "<p>The road was open and the sky was wide.</p>"
    );
    assert_eq!(body["editable"], false);

    harness.cleanup().await;
}

#[tokio::test]
async fn a_draft_is_a_404_to_a_stranger() {
    let harness = Harness::new("draft-404").await;
    let mut author = harness.client();
    register(&mut author, "author@example.com", "Quill").await;

    let work = create_work(&mut author, "Secret Draft").await;
    let work_id = work["id"].as_str().expect("id").to_owned();
    let (chapter, version) = add_chapter(&mut author, &work_id, "One").await;
    save_chapter(&mut author, &chapter, version, "Not for you.").await;

    let mut stranger = harness.client();
    let (status, _) = stranger.get(&format!("/api/v1/works/{work_id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = stranger
        .get(&format!("/api/v1/works/{work_id}/chapters/{chapter}"))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Reading progress: two devices keep two positions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn progress_saved_by_one_device_does_not_overwrite_another() {
    let harness = Harness::new("two-devices").await;
    let mut reader = harness.client();
    register(&mut reader, "reader@example.com", "Bookworm").await;

    let work = create_work(&mut reader, "Long Work").await;
    let work_id = work["id"].as_str().expect("id").to_owned();
    let (chapter, version) = add_chapter(&mut reader, &work_id, "One").await;
    save_chapter(
        &mut reader,
        &chapter,
        version,
        "A chapter with enough words to have a middle.",
    )
    .await;
    publish(&mut reader, &work_id, 1).await;

    // Device A saves position 100.
    let (status, _) = reader
        .put(
            "/api/v1/reading/progress",
            json!({
                "subject_type": "work",
                "subject_id": work_id,
                "chapter_id": chapter,
                "position_permille": 100,
                "device_id": "device-a",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Device B saves position 900.
    let (status, _) = reader
        .put(
            "/api/v1/reading/progress",
            json!({
                "subject_type": "work",
                "subject_id": work_id,
                "chapter_id": chapter,
                "position_permille": 900,
                "device_id": "device-b",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Both positions are stored.
    let account_id: lorehaven_domain::AccountId = reader.get("/api/v1/auth/me").await.1["account"]
        ["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let positions = reading::progress_for(harness.tdb.db(), account_id, "work", &work_id)
        .await
        .expect("progress");
    assert_eq!(positions.len(), 2, "expected one position per device");

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Ratings: private by default, and a private rating never moves the aggregate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_private_rating_changes_no_public_number() {
    let harness = Harness::new("private-rating").await;
    let mut reader = harness.client();
    register(&mut reader, "rater@example.com", "Judge").await;

    // Publish a work to rate.
    let mut author = harness.client();
    register(&mut author, "author@example.com", "Quill").await;
    let work = create_work(&mut author, "Rated Work").await;
    let work_id = work["id"].as_str().expect("id").to_owned();
    let (chapter, version) = add_chapter(&mut author, &work_id, "One").await;
    save_chapter(&mut author, &chapter, version, "Something to read.").await;
    publish(&mut author, &work_id, 1).await;

    // A private rating of 5.
    let (status, _) = reader
        .put(
            &format!("/api/v1/works/{work_id}/rating"),
            json!({ "stars": 5, "is_public": false }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // The public aggregate is still absent.
    let summary =
        reading::public_rating_summary(harness.tdb.db(), work_id.parse().expect("work id"))
            .await
            .expect("summary");
    assert!(
        summary.is_none(),
        "a private rating must not move the aggregate"
    );

    harness.cleanup().await;
}

#[tokio::test]
async fn the_public_aggregate_is_absent_below_the_minimum_count() {
    let harness = Harness::new("aggregate-minimum").await;

    // Create a published work to rate (FK requires a real work).
    let mut author = harness.client();
    register(&mut author, "author@example.com", "Quill").await;
    let work = create_work(&mut author, "Aggregate Test").await;
    let work_id = work["id"].as_str().expect("id").to_owned();
    let (chapter, version) = add_chapter(&mut author, &work_id, "One").await;
    save_chapter(&mut author, &chapter, version, "Something.").await;
    publish(&mut author, &work_id, 1).await;
    let work_uuid: lorehaven_domain::WorkId = work_id.parse().unwrap();

    // Insert four public ratings directly; below the minimum of five.
    for i in 0..4 {
        let mut rater = harness.client();
        register(
            &mut rater,
            &format!("rater{i}@example.com"),
            &format!("Rater{i}"),
        )
        .await;
        let pseud = active_pseud(&mut rater).await;
        reading::upsert_rating(
            harness.tdb.db(),
            rater.get("/api/v1/auth/me").await.1["account"]["id"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap(),
            pseud.parse().expect("pseud"),
            work_uuid,
            4,
            true,
        )
        .await
        .expect("upsert");
    }

    let summary = reading::public_rating_summary(harness.tdb.db(), work_uuid)
        .await
        .expect("summary");
    assert!(
        summary.is_none(),
        "below the minimum count the aggregate is hidden"
    );

    harness.cleanup().await;
}

#[tokio::test]
async fn the_aggregate_reports_its_count_and_method() {
    let harness = Harness::new("aggregate-reports").await;

    // Create a published work to rate (FK requires a real work).
    let mut author = harness.client();
    register(&mut author, "author@example.com", "Quill").await;
    let work = create_work(&mut author, "Rated Work").await;
    let work_id = work["id"].as_str().expect("id").to_owned();
    let (chapter, version) = add_chapter(&mut author, &work_id, "One").await;
    save_chapter(&mut author, &chapter, version, "Something.").await;
    publish(&mut author, &work_id, 1).await;
    let work_uuid: lorehaven_domain::WorkId = work_id.parse().unwrap();

    // Five public ratings: 3, 4, 5, 4, 4 → mean 4.0 → 4000 permille.
    for (i, stars) in [3, 4, 5, 4, 4].iter().enumerate() {
        let mut rater = harness.client();
        register(
            &mut rater,
            &format!("rater{i}@example.com"),
            &format!("Rater{i}"),
        )
        .await;
        let pseud = active_pseud(&mut rater).await;
        reading::upsert_rating(
            harness.tdb.db(),
            rater.get("/api/v1/auth/me").await.1["account"]["id"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap(),
            pseud.parse().expect("pseud"),
            work_uuid,
            *stars,
            true,
        )
        .await
        .expect("upsert");
    }

    let summary = reading::public_rating_summary(harness.tdb.db(), work_uuid)
        .await
        .expect("summary");
    let summary = summary.expect("aggregate should be present at five ratings");
    assert_eq!(summary.count, 5);
    assert_eq!(summary.mean_permille, 4000);

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// History is per-pseud and can be cleared
// ---------------------------------------------------------------------------

#[tokio::test]
async fn switching_pseud_shows_a_different_history() {
    let harness = Harness::new("history-per-pseud").await;
    let mut reader = harness.client();
    register(&mut reader, "reader@example.com", "First").await;

    // Create a second pseud.
    let (status, body) = reader
        .post("/api/v1/pseuds", json!({ "handle": "Second" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let second = body["id"].as_str().expect("pseud id").to_owned();

    // Capture the first pseud BEFORE activating the second.
    let first = active_pseud(&mut reader).await;

    let (status, _) = reader
        .post(&format!("/api/v1/pseuds/{second}/activate"), json!({}))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Publish a work and touch history as the second pseud.
    let mut author = harness.client();
    register(&mut author, "author@example.com", "Quill").await;
    let work = create_work(&mut author, "Read Work").await;
    let work_id = work["id"].as_str().expect("id").to_owned();
    let (chapter, version) = add_chapter(&mut author, &work_id, "One").await;
    save_chapter(&mut author, &chapter, version, "Something to read.").await;
    publish(&mut author, &work_id, 1).await;

    let pseud_id: lorehaven_domain::PseudId = second.parse().expect("pseud");
    reading::touch_history(
        harness.tdb.db(),
        reader.get("/api/v1/auth/me").await.1["account"]["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
        pseud_id,
        "work",
        &work_id,
        Some(chapter.parse().expect("chapter")),
    )
    .await
    .expect("touch history");

    // The second pseud's history shows the work.
    let (status, body) = reader.get("/api/v1/library/history").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().expect("items").len(), 1);

    // Switch back to the first pseud: its history is empty.
    let (status, _) = reader
        .post(&format!("/api/v1/pseuds/{first}/activate"), json!({}))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = reader.get("/api/v1/library/history").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["items"].as_array().expect("items").len(),
        0,
        "the first pseud has not read it"
    );

    let _ = first;

    harness.cleanup().await;
}

/// Reading a chapter through the reader's own route records it in history.
///
/// `touch_history` had no caller in the application: the reading surface
/// reported a position and nothing recorded that the work had been opened, so
/// `/library/history` was empty for every real reader while these tests — which
/// called the repository directly — stayed green.
#[tokio::test]
async fn reading_a_chapter_records_it_in_history() {
    let harness = Harness::new("history-from-reading").await;
    let mut reader = harness.client();
    register(&mut reader, "reader@example.com", "Reader").await;

    let mut author = harness.client();
    register(&mut author, "author@example.com", "Quill").await;
    let work = create_work(&mut author, "The Long Road").await;
    let work_id = work["id"].as_str().expect("id").to_owned();
    let (chapter, version) = add_chapter(&mut author, &work_id, "One").await;
    save_chapter(&mut author, &chapter, version, "Text to read.").await;
    publish(&mut author, &work_id, 1).await;

    // Nothing has been read yet, so nothing is in the history.
    let (status, body) = reader.get("/api/v1/library/history").await;
    assert_eq!(status, StatusCode::OK, "history: {body}");
    assert_eq!(body["items"].as_array().expect("items").len(), 0);

    // The two requests the reading surface makes: open the chapter, then say
    // where the reader is.
    let (status, chapter_body) = reader
        .get(&format!("/api/v1/works/{work_id}/chapters/{chapter}"))
        .await;
    assert_eq!(status, StatusCode::OK, "read chapter: {chapter_body}");
    let revision_id = chapter_body["revision_id"]
        .as_str()
        .expect("revision id")
        .to_owned();
    let (status, body) = reader
        .put(
            "/api/v1/reading/progress",
            json!({
                "subject_type": "work",
                "subject_id": work_id,
                "chapter_id": chapter,
                "content_revision": revision_id,
                "position_permille": 400,
                "device_id": "device-1"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "save progress: {body}");

    let (status, body) = reader.get("/api/v1/library/history").await;
    assert_eq!(status, StatusCode::OK, "history: {body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "the work the reader opened: {body}");
    assert_eq!(items[0]["subject_id"], work_id);
    assert_eq!(items[0]["title"], "The Long Road");
    assert!(
        items[0]["last_read_at"]
            .as_str()
            .is_some_and(|at| !at.is_empty()),
        "a history row says when: {body}"
    );

    harness.cleanup().await;
}

#[tokio::test]
async fn clearing_history_removes_only_the_callers_rows() {
    let harness = Harness::new("clear-history").await;

    let mut reader_a = harness.client();
    register(&mut reader_a, "a@example.com", "AAA").await;
    let account_a: lorehaven_domain::AccountId = reader_a.get("/api/v1/auth/me").await.1["account"]
        ["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let mut reader_b = harness.client();
    register(&mut reader_b, "b@example.com", "BBB").await;
    let account_b: lorehaven_domain::AccountId = reader_b.get("/api/v1/auth/me").await.1["account"]
        ["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    // Both touch history for the same subject.
    let work_id = "550e8400-e29b-41d4-a716-446655440000";
    reading::touch_history(
        harness.tdb.db(),
        account_a,
        active_pseud(&mut reader_a).await.parse().expect("pseud"),
        "work",
        work_id,
        None,
    )
    .await
    .expect("touch a");
    reading::touch_history(
        harness.tdb.db(),
        account_b,
        active_pseud(&mut reader_b).await.parse().expect("pseud"),
        "work",
        work_id,
        None,
    )
    .await
    .expect("touch b");

    // A clears their history.
    let (status, _) = reader_a
        .post("/api/v1/library/history/clear", json!({}))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // A's history is empty; B's still has its row.
    let a_history = reading::history_for(
        harness.tdb.db(),
        account_a,
        active_pseud(&mut reader_a).await.parse().expect("pseud"),
        50,
    )
    .await
    .expect("history a");
    let b_history = reading::history_for(
        harness.tdb.db(),
        account_b,
        active_pseud(&mut reader_b).await.parse().expect("pseud"),
        50,
    )
    .await
    .expect("history b");
    assert_eq!(a_history.len(), 0);
    assert_eq!(b_history.len(), 1);

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Typography is account-scoped and optimistic-concurrency protected
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_stale_typography_patch_returns_conflict() {
    let harness = Harness::new("typography-conflict").await;
    let mut reader = harness.client();
    register(&mut reader, "reader@example.com", "Reader").await;

    // Write a first typography row so a real version exists.
    let (status, _) = reader
        .patch(
            "/api/v1/settings/typography",
            json!({ "expected_version": 0, "font_scale": 1.2 }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Read the current version.
    let (status, body) = reader.get("/api/v1/settings/typography").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let version = body["version"].as_i64().expect("version");

    // A stale patch with the wrong version is refused.
    let (status, body) = reader
        .patch(
            "/api/v1/settings/typography",
            json!({ "expected_version": version + 1, "font_scale": 1.5 }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "REVISION_CONFLICT");

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Two devices: the reader is asked which position to resume from
// ---------------------------------------------------------------------------

#[tokio::test]
async fn two_devices_that_disagree_produce_a_choice() {
    let harness = Harness::new("devices-disagree").await;
    let mut reader = harness.client();
    register(&mut reader, "reader@example.com", "Bookworm").await;
    let (work_id, chapter) = published_chapter(&harness, "Disagreeing Devices").await;

    let (status, body) = reader
        .get(&format!("/api/v1/works/{work_id}/chapters/{chapter}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let revision = body["revision_id"].as_str().expect("revision id");

    // Two devices read the same revision and stopped in different places.
    for (device, permille) in [("device-a", 100), ("device-b", 900)] {
        let (status, body) = reader
            .put(
                "/api/v1/reading/progress",
                json!({
                    "subject_type": "work",
                    "subject_id": work_id,
                    "chapter_id": chapter,
                    "content_revision": revision,
                    "position_permille": permille,
                    "device_id": device,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    }

    let (status, body) = reader
        .get(&format!(
            "/api/v1/reading/progress?subject_type=work&subject_id={work_id}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["positions"].as_array().expect("positions").len(),
        2,
        "one position per device: {body}"
    );

    // Spec §9.3: differing devices present a choice rather than silently
    // taking the furthest position.
    assert_eq!(body["resolution"]["kind"], "ask_the_reader", "{body}");
    let mine = &body["resolution"]["mine"];
    let other = &body["resolution"]["other"];
    assert!(mine.is_object(), "the reader's own position: {body}");
    assert!(other.is_object(), "the other device's position: {body}");
    assert_ne!(
        mine["position_permille"], other["position_permille"],
        "the two options must be different places: {body}"
    );

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Resuming after an edit: the anchor is what is kept
// ---------------------------------------------------------------------------

#[tokio::test]
async fn resuming_after_an_edit_uses_the_anchor_not_the_offset() {
    let harness = Harness::new("anchor-after-edit").await;
    let mut reader = harness.client();
    register(&mut reader, "reader@example.com", "Bookworm").await;

    let mut author = harness.client();
    register(&mut author, "author@example.com", "Quill").await;
    let work = create_work(&mut author, "Revised Work").await;
    let work_id = work["id"].as_str().expect("id").to_owned();
    let (chapter, chapter_version) = add_chapter(&mut author, &work_id, "One").await;
    let (status, body) = save_chapter(
        &mut author,
        &chapter,
        chapter_version,
        "The first draft of the scene.",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let version = body["version"].as_i64().expect("chapter version");
    publish(&mut author, &work_id, 1).await;

    // The reader stops a third of the way into the published revision, at a
    // paragraph anchor.
    let (status, body) = reader
        .get(&format!("/api/v1/works/{work_id}/chapters/{chapter}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let read_revision = body["revision_id"]
        .as_str()
        .expect("revision id")
        .to_owned();

    let (status, body) = reader
        .put(
            "/api/v1/reading/progress",
            json!({
                "subject_type": "work",
                "subject_id": work_id,
                "chapter_id": chapter,
                "content_revision": read_revision,
                "paragraph_anchor": "p-3",
                "position_permille": 330,
                "device_id": "device-a",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    // The author rewrites the chapter. The words at 33% are not the words the
    // reader was looking at any more.
    let (status, body) = save_chapter(
        &mut author,
        &chapter,
        version,
        "The second draft, rewritten from the top and with a different middle.",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = reader
        .get(&format!("/api/v1/works/{work_id}/chapters/{chapter}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let current_revision: lorehaven_domain::RevisionId = body["revision_id"]
        .as_str()
        .expect("revision id")
        .parse()
        .expect("revision id");
    assert_ne!(
        current_revision.to_string(),
        read_revision,
        "an edit must create a new revision for this test to mean anything"
    );

    // The stored position kept the anchor and the revision it was recorded
    // against, so a client can put the reader back at the paragraph and must
    // not treat the offset as a position in the new text.
    let account_id: lorehaven_domain::AccountId = reader.get("/api/v1/auth/me").await.1["account"]
        ["id"]
        .as_str()
        .expect("account id")
        .parse()
        .expect("account id");
    let positions = reading::progress_for(harness.tdb.db(), account_id, "work", &work_id)
        .await
        .expect("progress");
    assert_eq!(
        positions.len(),
        1,
        "the edit must not duplicate the position"
    );
    let stored = &positions[0];
    assert_eq!(stored.anchor.as_deref(), Some("p-3"), "the anchor survives");
    assert_eq!(
        stored
            .revision
            .map(|revision| revision.to_string())
            .as_deref(),
        Some(read_revision.as_str()),
        "the position still names the revision it was taken against"
    );
    assert_eq!(stored.fraction, 330);
    assert!(
        !lorehaven_domain::reading::position_is_reliable(stored, current_revision),
        "a position taken against removed text is not reliable"
    );

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// A rating belongs to the pseud that gave it
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_rating_is_invisible_to_the_accounts_other_pseud() {
    let harness = Harness::new("rating-per-pseud").await;
    let mut reader = harness.client();
    register(&mut reader, "reader@example.com", "First").await;
    let first: lorehaven_domain::PseudId = active_pseud(&mut reader).await.parse().expect("pseud");

    let (status, body) = reader
        .post("/api/v1/pseuds", json!({ "handle": "Second" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let second: lorehaven_domain::PseudId = body["id"].as_str().expect("pseud id").parse().unwrap();
    let (status, _) = reader
        .post(&format!("/api/v1/pseuds/{second}/activate"), json!({}))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (work_id, _chapter) = published_chapter(&harness, "Rated Work").await;
    let work: lorehaven_domain::WorkId = work_id.parse().expect("work id");

    // The second face rates it, privately.
    let (status, body) = reader
        .put(
            &format!("/api/v1/works/{work_id}/rating"),
            json!({ "stars": 5, "is_public": false }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    assert!(
        reading::rating_for(harness.tdb.db(), first, work)
            .await
            .expect("rating")
            .is_none(),
        "the first pseud gave no rating and must see none"
    );
    let rated = reading::rating_for(harness.tdb.db(), second, work)
        .await
        .expect("rating")
        .expect("the rating the second face gave");
    assert_eq!(rated.stars, 5);

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Typography belongs to the account, not to a pseud
// ---------------------------------------------------------------------------

#[tokio::test]
async fn typography_follows_the_account_not_the_pseud() {
    let harness = Harness::new("typography-account").await;
    let mut reader = harness.client();
    register(&mut reader, "reader@example.com", "First").await;

    let (status, body) = reader
        .patch(
            "/api/v1/settings/typography",
            json!({ "expected_version": 0, "font_scale": 1.3, "reader_theme": "dark" }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let (status, body) = reader.get("/api/v1/settings/typography").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let version = body["version"].as_i64().expect("version");
    assert_eq!(version, 1);
    assert_eq!(body["reader_theme"], "dark");

    // Another face of the same account sees the same settings and the same
    // version — typography is a property of the reader, not of a face.
    let (status, body) = reader
        .post("/api/v1/pseuds", json!({ "handle": "Second" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let second = body["id"].as_str().expect("pseud id").to_owned();
    let (status, _) = reader
        .post(&format!("/api/v1/pseuds/{second}/activate"), json!({}))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = reader.get("/api/v1/settings/typography").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["reader_theme"], "dark");
    assert_eq!(body["version"].as_i64().expect("version"), version);

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Reviews: private until published, public identity is the active pseud
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_review_stays_private_until_it_is_published() {
    let harness = Harness::new("review-private").await;
    let mut reader = harness.client();
    register(&mut reader, "reader@example.com", "Bookworm").await;
    let (work_id, _chapter) = published_chapter(&harness, "Reviewed Work").await;

    let (status, body) = reader
        .put(
            &format!("/api/v1/works/{work_id}/reviews"),
            json!({ "body": "A quiet, careful story." }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["is_public"], false, "a review is private by default");
    assert!(body["published_at"].is_null(), "{body}");
    let version = body["version"].as_i64().expect("version");

    let mut visitor = harness.client();
    let (status, body) = visitor
        .get(&format!("/api/v1/works/{work_id}/reviews"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["items"].as_array().expect("items").len(),
        0,
        "an unpublished review is not readable by anyone else: {body}"
    );

    // Publishing it makes it visible, attributed to the active pseud.
    let (status, body) = reader
        .put(
            &format!("/api/v1/works/{work_id}/reviews"),
            json!({
                "body": "A quiet, careful story.",
                "is_public": true,
                "contains_spoilers": true,
                "expected_version": version,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["is_public"], true);
    assert!(body["published_at"].is_string(), "{body}");

    let (status, body) = visitor
        .get(&format!("/api/v1/works/{work_id}/reviews"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{body}");
    assert_eq!(items[0]["author_handle"], "Bookworm");
    assert_eq!(items[0]["body"], "A quiet, careful story.");
    assert_eq!(items[0]["contains_spoilers"], true);

    harness.cleanup().await;
}

#[tokio::test]
async fn a_withdrawn_review_leaves_the_public_list() {
    let harness = Harness::new("review-withdrawn").await;
    let mut reader = harness.client();
    register(&mut reader, "reader@example.com", "Bookworm").await;
    let (work_id, _chapter) = published_chapter(&harness, "Withdrawn Review").await;

    let (status, body) = reader
        .put(
            &format!("/api/v1/works/{work_id}/reviews"),
            json!({ "body": "Worth your evening.", "is_public": true }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // A stranger deleting the same work's review removes nothing: the
    // statement is scoped to the caller's own pseud.
    let mut stranger = harness.client();
    register(&mut stranger, "stranger@example.com", "Passerby").await;
    let (status, _) = stranger
        .delete(&format!("/api/v1/works/{work_id}/reviews"))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let mut visitor = harness.client();
    let (status, body) = visitor
        .get(&format!("/api/v1/works/{work_id}/reviews"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["items"].as_array().expect("items").len(),
        1,
        "another account's withdrawal must not withdraw this review: {body}"
    );

    // The author withdrawing it does.
    let (status, _) = reader
        .delete(&format!("/api/v1/works/{work_id}/reviews"))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = visitor
        .get(&format!("/api/v1/works/{work_id}/reviews"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["items"].as_array().expect("items").len(),
        0,
        "a withdrawn review leaves the public list: {body}"
    );

    harness.cleanup().await;
}

#[tokio::test]
async fn a_stale_review_write_returns_conflict() {
    let harness = Harness::new("review-conflict").await;
    let mut reader = harness.client();
    register(&mut reader, "reader@example.com", "Bookworm").await;
    let (work_id, _chapter) = published_chapter(&harness, "Contested Review").await;

    let (status, body) = reader
        .put(
            &format!("/api/v1/works/{work_id}/reviews"),
            json!({ "body": "First thoughts." }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["version"].as_i64().expect("version"), 1);

    // A second tab still holding version 0 loses the race.
    let (status, body) = reader
        .put(
            &format!("/api/v1/works/{work_id}/reviews"),
            json!({ "body": "Written from an older tab.", "expected_version": 0 }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "REVISION_CONFLICT");

    // The version it read is accepted, and the write is not duplicated.
    let (status, body) = reader
        .put(
            &format!("/api/v1/works/{work_id}/reviews"),
            json!({ "body": "Second thoughts.", "expected_version": 1 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["version"].as_i64().expect("version"), 2);
    assert_eq!(body["body"], "Second thoughts.");

    // A review with no words is refused rather than stored empty.
    let (status, body) = reader
        .put(
            &format!("/api/v1/works/{work_id}/reviews"),
            json!({ "body": "   " }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "VALIDATION_FAILED");

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// A helper for the tests that need a published work to point a reader at
// ---------------------------------------------------------------------------

/// Publish a one-chapter work with a different account and return its ids.
async fn published_chapter(harness: &Harness, title: &str) -> (String, String) {
    let mut author = harness.client();
    let email = format!(
        "author-{}@example.com",
        title.to_lowercase().replace(' ', "-")
    );
    register(&mut author, &email, "Quill").await;
    let work = create_work(&mut author, title).await;
    let work_id = work["id"].as_str().expect("id").to_owned();
    let (chapter, version) = add_chapter(&mut author, &work_id, "One").await;
    let (status, body) = save_chapter(
        &mut author,
        &chapter,
        version,
        "A chapter with enough words to have a middle.",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "save chapter: {body}");
    let (status, body) = publish(&mut author, &work_id, 1).await;
    assert_eq!(status, StatusCode::OK, "publish: {body}");
    (work_id, chapter)
}

// ---------------------------------------------------------------------------
// Private notes are per pseud and never leave their writer
// ---------------------------------------------------------------------------

/// A note belongs to the writer's pseud and is invisible to a stranger.
#[tokio::test]
async fn a_note_is_private_to_its_writer_and_visible_only_to_them() {
    let harness = Harness::new("note-private").await;
    let mut author = harness.client();
    register(&mut author, "author@example.com", "Quill").await;
    let first = active_pseud(&mut author).await;

    let work = create_work(&mut author, "The Open Road").await;
    let work_id = work["id"].as_str().expect("id").to_owned();
    let (chapter, version) = add_chapter(&mut author, &work_id, "One").await;
    save_chapter(&mut author, &chapter, version, "One").await;
    publish(&mut author, &work_id, 1).await;

    // The author writes a note anchored to the chapter.
    let (status, body) = author
        .put(
            "/api/v1/notes",
            json!({
                "subject_type": "work",
                "subject_id": work_id,
                "anchor": format!("p-{chapter}"),
                "body": "a private observation"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "write note: {body}");

    // The note is visible to the same pseud on this account.
    let (status, body) = author
        .get(&format!(
            "/api/v1/notes?subject_type=work&subject_id={work_id}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "list notes: {body}");
    let notes = body.as_array().expect("notes");
    assert_eq!(notes.len(), 1, "the writer sees their own note");
    assert_eq!(notes[0]["body"], "a private observation");

    // The note id is captured while its writer can still see it.
    let note_id = notes[0]["id"].as_str().expect("note id").to_owned();

    // A second face on the same account keeps its own notes: adding a pseud and
    // switching to it hides the first face's note.
    let (status, body) = author
        .post("/api/v1/pseuds", json!({ "handle": "Second" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "second pseud: {body}");
    let second = body["id"].as_str().expect("pseud id").to_owned();
    let (status, _) = author
        .post(&format!("/api/v1/pseuds/{second}/activate"), json!({}))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "activate the second face");
    let (status, body) = author
        .get(&format!(
            "/api/v1/notes?subject_type=work&subject_id={work_id}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "other pseud list notes: {body}");
    let notes = body.as_array().expect("notes");
    assert_eq!(
        notes.len(),
        0,
        "a pseud cannot see another's notes on the same account"
    );

    // A stranger cannot see it either.
    let mut stranger = harness.client();
    register(&mut stranger, "stranger@example.com", "Read").await;
    let (status, body) = stranger
        .get(&format!(
            "/api/v1/notes?subject_type=work&subject_id={work_id}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "stranger list notes: {body}");
    let notes = body.as_array().expect("notes");
    assert_eq!(
        notes.len(),
        0,
        "a stranger cannot see another writer's notes"
    );

    // Another face cannot delete the writer's note, even holding its id: the
    // delete is scoped by the acting pseud and quietly does nothing.
    let (status, _) = author.delete(&format!("/api/v1/notes/{note_id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Back as the writer: the note survived the other face's delete.
    let (status, _) = author
        .post(&format!("/api/v1/pseuds/{first}/activate"), json!({}))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "activate the writer again");
    let (status, body) = author
        .get(&format!(
            "/api/v1/notes?subject_type=work&subject_id={work_id}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "writer list notes: {body}");
    assert_eq!(
        body.as_array().expect("notes").len(),
        1,
        "another face's delete does not remove the writer's note"
    );

    // Deleting removes the writer's note and nothing else.
    let (status, _) = author.delete(&format!("/api/v1/notes/{note_id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = author
        .get(&format!(
            "/api/v1/notes?subject_type=work&subject_id={work_id}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().expect("notes").len(), 0);

    harness.cleanup().await;
}

/// The note repository honours the subject, so a note on a work does not
/// surface against a library item with the same id.
#[tokio::test]
async fn notes_filter_to_the_exact_subject() {
    let harness = Harness::new("note-subject").await;
    let mut reader = harness.client();
    register(&mut reader, "reader@example.com", "Reader").await;

    let work = create_work(&mut reader, "A Work").await;
    let work_id = work["id"].as_str().expect("id").to_owned();
    let (chapter, version) = add_chapter(&mut reader, &work_id, "One").await;
    save_chapter(&mut reader, &chapter, version, "One").await;
    publish(&mut reader, &work_id, 1).await;

    // One note on the work.
    let (status, _) = reader
        .put(
            "/api/v1/notes",
            json!({
                "subject_type": "work",
                "subject_id": work_id,
                "body": "on the work"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Listing the work returns it; listing by a different subject does not.
    let (status, body) = reader
        .get(&format!(
            "/api/v1/notes?subject_type=work&subject_id={work_id}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().expect("notes").len(), 1);

    let (status, body) = reader
        .get("/api/v1/notes?subject_type=work&subject_id=other")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().expect("notes").len(), 0);

    harness.cleanup().await;
}

/// Saving the same anchor twice edits the one note instead of adding a second.
///
/// The schema carries no unique index on `(pseud, subject, anchor)` — an anchor
/// may be NULL and both engines treat NULLs as distinct, so a constraint would
/// not express the key. The repository therefore has to do the upsert itself,
/// and this test is what notices when it stops.
#[tokio::test]
async fn saving_the_same_note_twice_updates_it_in_place() {
    let harness = Harness::new("note-upsert").await;
    let mut reader = harness.client();
    register(&mut reader, "reader@example.com", "Reader").await;

    let work = create_work(&mut reader, "A Work").await;
    let work_id = work["id"].as_str().expect("id").to_owned();
    let (chapter, version) = add_chapter(&mut reader, &work_id, "One").await;
    save_chapter(&mut reader, &chapter, version, "One").await;
    publish(&mut reader, &work_id, 1).await;

    for body in ["first", "second"] {
        let (status, response) = reader
            .put(
                "/api/v1/notes",
                json!({
                    "subject_type": "work",
                    "subject_id": work_id,
                    "anchor": "p-1",
                    "body": body
                }),
            )
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "write note: {response}");
    }

    let (status, body) = reader
        .get(&format!(
            "/api/v1/notes?subject_type=work&subject_id={work_id}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    let notes = body.as_array().expect("notes");
    assert_eq!(
        notes.len(),
        1,
        "one anchor holds one note, not one per save"
    );
    assert_eq!(
        notes[0]["body"], "second",
        "the second save replaces the body"
    );
    assert_eq!(notes[0]["version"], 2, "the edit bumps the version");

    harness.cleanup().await;
}
