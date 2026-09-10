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
use lorehaven_db::{reading, Database, DatabaseConfig};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

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
            report.applied.contains(&"0004_reading".to_owned()),
            "the reading migration must apply: {report:?}"
        );

        Self { dir, db }
    }

    fn client(&self) -> Client {
        Client::new(server::build_router(AppState::new(
            config_for(&self.dir),
            self.db.clone(),
        )))
    }

    async fn cleanup(self) {
        self.db.close().await;
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
    let positions = reading::progress_for(&harness.db, account_id, "work", &work_id)
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
    let summary = reading::public_rating_summary(&harness.db, work_id.parse().expect("work id"))
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
            &harness.db,
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

    let summary = reading::public_rating_summary(&harness.db, work_uuid)
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
            &harness.db,
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

    let summary = reading::public_rating_summary(&harness.db, work_uuid)
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
        &harness.db,
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
    reading::touch_history(
        &harness.db,
        account_a,
        active_pseud(&mut reader_a).await.parse().expect("pseud"),
        "work",
        "work-1",
        None,
    )
    .await
    .expect("touch a");
    reading::touch_history(
        &harness.db,
        account_b,
        active_pseud(&mut reader_b).await.parse().expect("pseud"),
        "work",
        "work-1",
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
        &harness.db,
        account_a,
        active_pseud(&mut reader_a).await.parse().expect("pseud"),
        50,
    )
    .await
    .expect("history a");
    let b_history = reading::history_for(
        &harness.db,
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
