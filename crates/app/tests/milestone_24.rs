//! Milestone 24 — anchored comments, orphaning, and CSV imports (spec §12, §24.8, §32.3).

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_db::DatabaseConfig;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lorehaven-m24-{tag}-{:?}", std::process::id()));
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
    async fn get(&mut self, uri: &str) -> (StatusCode, Value) {
        self.send("GET", uri, None).await
    }
    async fn post(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.send("POST", uri, Some(body)).await
    }
    async fn patch(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.send("PATCH", uri, Some(body)).await
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

async fn create_work_with_chapter(client: &mut Client, title: &str) -> (String, String) {
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

    let doc = json!({ "type": "doc", "content": [
        { "type": "paragraph", "content": [{ "type": "text", "text": "First paragraph." }] },
        { "type": "paragraph", "content": [{ "type": "text", "text": "Second paragraph." }] },
        { "type": "paragraph", "content": [{ "type": "text", "text": "Third paragraph." }] },
    ] });
    let (status, _) = client
        .patch(
            &format!("/api/v1/chapters/{chapter_id}"),
            json!({ "expected_version": chapter_version, "document": doc }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "save chapter");

    let (status, _) = client.post(&format!("/api/v1/works/{work_id}/publish"), json!({ "expected_version": work_version, "idempotency_key": format!("m24-{work_id}") })).await;
    assert_eq!(status, StatusCode::OK, "publish work");

    (work_id, chapter_id)
}

#[tokio::test]
async fn anchored_comments_round_trip() {
    let dir = scratch_dir("anchored-round-trip");
    let tdb = test_support::TestDb::connect_with_dir("anchored-round-trip", &dir).await;
    let app = server::build_router(AppState::new(config_for(&dir), tdb.db().clone()));
    let mut client = Client::new(app);

    register(&mut client, "anchored@example.com", "anchored").await;
    let (work_id, chapter_id) = create_work_with_chapter(&mut client, "Anchored Work").await;

    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/comments"),
            json!({ "body": "Whole work comment" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "whole-work comment: {body}");
    let whole_comment_id = body["id"].as_str().unwrap().to_owned();

    let (status, body) = client.post(&format!("/api/v1/works/{work_id}/comments"), json!({ "body": "Anchored to paragraph 1", "anchor_kind": "paragraph", "anchor_value": "1", "anchor_chapter_id": chapter_id })).await;
    assert_eq!(status, StatusCode::OK, "anchored comment: {body}");
    let anchored_comment_id = body["id"].as_str().unwrap().to_owned();

    let (status, body) = client
        .get(&format!("/api/v1/works/{work_id}/comments"))
        .await;
    assert_eq!(status, StatusCode::OK, "list comments");
    let items = body["items"].as_array().expect("items array");
    assert!(items.len() >= 2);

    let anchored = items
        .iter()
        .find(|c| c["id"].as_str().unwrap() == anchored_comment_id)
        .expect("anchored");
    assert_eq!(anchored["anchor_kind"].as_str().unwrap(), "paragraph");
    assert_eq!(anchored["anchor_value"].as_str().unwrap(), "1");

    let whole = items
        .iter()
        .find(|c| c["id"].as_str().unwrap() == whole_comment_id)
        .expect("whole");
    assert!(whole["anchor_kind"].is_null());

    println!("PASS: anchored_comments_round_trip");
}

#[tokio::test]
async fn anchored_comment_validation() {
    let dir = scratch_dir("anchored-validation");
    let tdb = test_support::TestDb::connect_with_dir("anchored-validation", &dir).await;
    let app = server::build_router(AppState::new(config_for(&dir), tdb.db().clone()));
    let mut client = Client::new(app);

    register(&mut client, "anchored-val@example.com", "anchoredval").await;
    let (work_id, chapter_id) = create_work_with_chapter(&mut client, "Validation Work").await;

    let (status, _) = client
        .post(
            &format!("/api/v1/works/{work_id}/comments"),
            json!({ "body": "bad", "anchor_kind": "paragraph", "anchor_value": "1" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "paragraph needs chapter_id"
    );

    let (status, _) = client.post(&format!("/api/v1/works/{work_id}/comments"), json!({ "body": "bad", "anchor_kind": "timestamp", "anchor_value": "42", "anchor_chapter_id": chapter_id })).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "timestamp rejects chapter_id"
    );

    let (status, _) = client
        .post(
            &format!("/api/v1/works/{work_id}/comments"),
            json!({ "body": "bad", "anchor_kind": "timestamp", "anchor_value": "abc" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "invalid timestamp rejected"
    );

    let (status, _) = client
        .post(
            &format!("/api/v1/works/{work_id}/comments"),
            json!({ "body": "bad", "anchor_kind": "offset", "anchor_value": "1" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "unknown kind rejected"
    );

    let (status, _) = client
        .post(
            &format!("/api/v1/works/{work_id}/comments"),
            json!({ "body": "bad", "anchor_kind": "paragraph" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "incomplete anchor rejected"
    );

    let (status, _) = client
        .post(
            &format!("/api/v1/works/{work_id}/comments"),
            json!({ "body": "ok", "anchor_kind": "timestamp", "anchor_value": "01:23:45" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "valid timestamp accepted");

    println!("PASS: anchored_comment_validation");
}

#[test]
fn goodreads_csv_parses() {
    let csv = r#"Title,Author,ISBN,My Rating,Average Rating,Shelves,Date Read
The Left Hand of Darkness,Ursula K. Le Guin,9780060500249,5,3.94,sci-fi;classic,2024-01-15"#;

    // The CSV import module from lorehaven_scrapers should parse this into a structured record.
    let parsed =
        lorehaven_scrapers::csv::import_shelf(csv, "goodreads").expect("parse goodreads csv");
    assert_eq!(parsed.rows.len(), 1, "one row parsed");
    let row = &parsed.rows[0];
    assert_eq!(row.title, "The Left Hand of Darkness");
    assert_eq!(row.author, "Ursula K. Le Guin");
    assert_eq!(row.my_rating, Some(5));
    assert!(row.shelves.contains(&"sci-fi".to_string()));
    assert!(row.shelves.contains(&"classic".to_string()));

    println!("PASS: goodreads_csv_parses");
}

// ---------------------------------------------------------------------------
// Creator dashboard (spec §32.3, §24.3)
// ---------------------------------------------------------------------------

/// The dashboard reports the caller's own works, and every small count arrives
/// as a band rather than as a number.
#[tokio::test]
async fn the_creator_dashboard_reports_banded_aggregates() {
    let dir = scratch_dir("dashboard");
    let tdb = test_support::TestDb::connect_with_dir("dashboard", &dir).await;
    let app = server::build_router(AppState::new(config_for(&dir), tdb.db().clone()));

    let mut author = Client::new(app.clone());
    register(&mut author, "author@example.com", "author").await;
    let (work_id, _) = create_work_with_chapter(&mut author, "Dashboarded Work").await;

    // A second work, left as a draft, so the published/unpublished split is
    // visible rather than inferred.
    let (status, body) = author
        .post("/api/v1/works", json!({ "title": "Unfinished" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "draft work: {body}");

    // One comment the positivity filter delivered.
    let (status, body) = author
        .post(
            &format!("/api/v1/works/{work_id}/comments"),
            json!({ "body": "A comment on my own work" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "comment: {body}");

    let (status, body) = author.get("/api/v1/me/dashboard").await;
    assert_eq!(status, StatusCode::OK, "dashboard: {body}");

    let floor = body["floor"].as_i64().expect("floor");
    assert!(floor >= 1, "the dashboard must state its floor: {body}");

    // Works and words are the author's own facts about their own text, so they
    // are exact: banding a chapter count would make the dashboard useless.
    assert_eq!(body["works"]["total"].as_i64(), Some(2), "{body}");
    assert_eq!(body["works"]["published"].as_i64(), Some(1), "{body}");
    assert_eq!(body["works"]["unpublished"].as_i64(), Some(1), "{body}");
    assert!(
        body["works"]["chapters"].as_i64().unwrap_or(0) >= 1,
        "{body}"
    );
    assert!(body["works"]["words"].as_i64().unwrap_or(0) > 0, "{body}");

    // Reader-facing counts are below the floor here, so they are bands — a
    // string, not a number, because a number is what a client would render as
    // an exact figure.
    let band = format!("fewer_than_{floor}");
    assert_eq!(
        body["readers"]["bookmarks"].as_str(),
        Some(band.as_str()),
        "{body}"
    );
    assert_eq!(
        body["readers"]["reviews"].as_str(),
        Some(band.as_str()),
        "{body}"
    );
    assert_eq!(
        body["readers"]["ratings"].as_str(),
        Some(band.as_str()),
        "{body}"
    );
    // A mean is only present when its count is.
    assert!(body["readers"]["mean_stars"].is_null(), "{body}");
    assert_eq!(
        body["positivity"]["comments_delivered"].as_str(),
        Some(band.as_str()),
        "one delivered comment is a band, not a number: {body}"
    );

    // No reader, pseud or account is named anywhere in the payload.
    let rendered = body.to_string();
    for forbidden in ["reader@", "pseud_id", "account_id", "author@"] {
        assert!(
            !rendered.contains(forbidden),
            "the dashboard leaked {forbidden}: {rendered}"
        );
    }

    // Another account's works are not in this dashboard.
    let mut stranger = Client::new(app.clone());
    register(&mut stranger, "stranger@example.com", "stranger").await;
    let (status, body) = stranger.get("/api/v1/me/dashboard").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["works"]["total"].as_i64(), Some(0), "{body}");
    assert_eq!(body["works"]["published"].as_i64(), Some(0), "{body}");

    // And a signed-out caller is refused rather than shown zeros that could be
    // mistaken for another account's.
    let mut anonymous = Client::new(app.clone());
    let (status, _) = anonymous.get("/api/v1/me/dashboard").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    println!("PASS: the_creator_dashboard_reports_banded_aggregates");
}

/// A StoryGraph export produces library states that respect the reader's own,
/// and rows it cannot map are refused by name. §32.3's acceptance criterion.
#[tokio::test]
async fn a_shelf_export_imports_states_and_names_what_it_refuses() {
    let dir = scratch_dir("shelf-import");
    let tdb = test_support::TestDb::connect_with_dir("shelf-import", &dir).await;
    let app = server::build_router(AppState::new(config_for(&dir), tdb.db().clone()));

    let mut reader = Client::new(app.clone());
    register(&mut reader, "reader@example.com", "reader").await;

    // A StoryGraph-shaped export: one finished with a date, one unread, and one
    // row with no title that nothing can map.
    let csv = "Title,Authors,ISBN,My Rating,Date Read,Review\n\
Reading Book,Ada Writer,9780000000001,4,2019/03/09,Loved it.\n\
Unread Book,Bo Writer,9780000000002,,,\n\
,Cy Writer,9780000000003,3,2019/04/01,\n";
    let (status, body) = reader
        .post(
            "/api/v1/library/imports/csv",
            json!({ "format": "storygraph", "csv": csv }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "shelf import: {status} {body}");

    assert_eq!(body["imported"].as_i64(), Some(2), "{body}");
    assert_eq!(body["kept_existing_state"].as_i64(), Some(0), "{body}");
    let refused = body["refused"].as_array().expect("refused array");
    assert_eq!(refused.len(), 1, "the untitled row is refused: {body}");
    assert!(
        refused[0]["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("no title"),
        "{body}"
    );
    // Named by its line: a row with no title has no other identity in the file
    // the reader is looking at.
    assert!(
        refused[0]["title"]
            .as_str()
            .unwrap_or_default()
            .starts_with("line "),
        "the refusal must name where the row is: {body}"
    );
    assert_eq!(body["skipped_rows_in_file"].as_i64(), Some(1), "{body}");

    // The finished row carries the reader's own date, not the import's.
    let (status, body) = reader.get("/api/v1/library/items").await;
    assert_eq!(status, StatusCode::OK, "library items: {body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 2, "two library rows: {body}");
    let reading = items
        .iter()
        .find(|item| item["title"] == "Reading Book")
        .expect("the finished row");
    assert_eq!(
        reading["source_url"].as_str().unwrap(),
        "import://storygraph/isbn:9780000000001"
    );

    let item_id = reading["id"].as_str().unwrap().to_owned();
    let (status, body) = reader
        .get(&format!("/api/v1/library/items/{item_id}/status"))
        .await;
    assert_eq!(status, StatusCode::OK, "reading status: {status} {body}");
    assert_eq!(body["status"].as_str().unwrap(), "finished", "{body}");
    assert!(
        body["finished_at"]
            .as_str()
            .unwrap_or_default()
            .starts_with("2019-03-09"),
        "the date read is the reader's, not the import's: {body}"
    );

    // Re-importing the same file updates the same two rows and adds none: the
    // unique key is what makes that true rather than a check that could race.
    let (status, body) = reader
        .post(
            "/api/v1/library/imports/csv",
            json!({ "format": "storygraph", "csv": csv }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "re-import: {status} {body}");
    assert_eq!(body["imported"].as_i64(), Some(0), "{body}");
    assert_eq!(
        body["kept_existing_state"].as_i64(),
        Some(2),
        "a re-import must not overwrite the reader's own state: {body}"
    );
    let (_, body) = reader.get("/api/v1/library/items").await;
    assert_eq!(body["items"].as_array().unwrap().len(), 2, "{body}");

    // An unknown format is refused before anything is written.
    let (status, body) = reader
        .post(
            "/api/v1/library/imports/csv",
            json!({ "format": "booklikes", "csv": csv }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");

    println!("PASS: a_shelf_export_imports_states_and_names_what_it_refuses");
}
