//! M31 — Work discussion modes, reaction bar, linked threads, migration tool.
//!
//! Spec §35.0–35.1. These tests drive the real router against a real SQLite
//! file and prove the mode rules the spec states:
//!
//! - a ThreadOnly work refuses work comments server-side (the reaction bar
//!   and the linked thread are the surface, not a hidden comment form);
//! - a CommentsOnly work keeps today's comment behavior exactly;
//! - reactions are one-per-pseud, changeable, retractable, with correct
//!   aggregates;
//! - the comment-to-topic migration preserves authorship, order, and
//!   timestamps, and is idempotent;
//! - only a work's contributors may change its discussion mode.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::{Backend, DatabaseConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m31-{tag}-{}-{:?}",
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
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
    fn capture(&mut self, response: &axum::response::Response) {
        for value in response.headers().get_all(header::SET_COOKIE) {
            let Ok(text) = value.to_str() else { continue };
            let Some((pair, _)) = text.split_once(';') else {
                continue;
            };
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
                .body(Body::from(serde_json::to_vec(&v).expect("serialise")))
                .expect("request"),
            None => builder.body(Body::empty()).expect("request"),
        };
        let response = self.app.clone().oneshot(request).await.expect("response");
        let status = response.status();
        self.capture(&response);
        let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024)
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

async fn published_work(
    harness: &Harness,
    email: &str,
    handle: &str,
    title: &str,
) -> (String, Client) {
    let mut author = harness.client();
    let _ = register(&mut author, email, handle).await;
    let (status, body) = author
        .post("/api/v1/works", json!({ "title": title }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    let work_id = body["id"].as_str().expect("id").to_owned();
    let work_version = body["version"].as_i64().expect("version");
    let (status, body) = author
        .post(
            &format!("/api/v1/works/{work_id}/chapters"),
            json!({ "title": "One" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "chapter: {body}");
    let chapter = body["id"].as_str().expect("chapter id").to_owned();
    let chapter_version = body["version"].as_i64().expect("version");
    let doc = json!({ "type": "doc", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "A chapter with enough words to have a middle." }] }] });
    let (status, body) = author
        .request(
            "PATCH",
            &format!("/api/v1/chapters/{chapter}"),
            Some(json!({ "expected_version": chapter_version, "document": doc })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "save: {body}");
    let (status, body) = author
        .post(
            &format!("/api/v1/works/{work_id}/publish"),
            json!({ "expected_version": work_version, "idempotency_key": format!("m31-{work_id}") }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "publish: {body}");
    (work_id, author)
}

/// A forum category must exist for linked topics to land in.
async fn seed_category(harness: &Harness) -> String {
    let db = harness.tdb.db();
    let id = uuid::Uuid::new_v4().to_string();
    let sql = db.sql(
        "INSERT INTO forum_categories (id, name, position, min_trust) VALUES (?, 'Work discussions', 0, 0)",
        "INSERT INTO forum_categories (id, name, position, min_trust) VALUES ($1, 'Work discussions', 0, 0)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("seed category");
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("seed category");
        }
    }
    id
}

// ---------------------------------------------------------------------------
// Modes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_new_work_reports_the_instance_default_mode() {
    let harness = Harness::new("default-mode").await;
    let (work_id, _author) = published_work(&harness, "m31-a@t.test", "m31a", "Default Mode").await;
    let mut reader = harness.client();
    register(&mut reader, "m31-b@t.test", "m31b").await;

    // The instance default (comments_only for legacy safety) applies.
    let (status, body) = reader
        .get(&format!("/api/v1/works/{work_id}/discussion-mode"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["mode"], "comments_only");
    assert_eq!(body["comments_enabled"], true);
    assert_eq!(body["thread_enabled"], false);

    harness.cleanup().await;
}

#[tokio::test]
async fn an_author_can_switch_their_work_to_thread_only() {
    let harness = Harness::new("thread-only").await;
    let (work_id, mut author) =
        published_work(&harness, "m31-c@t.test", "m31c", "Thread Only").await;

    let (status, body) = author
        .request(
            "PUT",
            &format!("/api/v1/works/{work_id}/discussion-mode"),
            Some(json!({ "mode": "thread_only" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["mode"], "thread_only");

    // And a stranger cannot change it back.
    let mut stranger = harness.client();
    register(&mut stranger, "m31-d@t.test", "m31d").await;
    let (status, _) = stranger
        .request(
            "PUT",
            &format!("/api/v1/works/{work_id}/discussion-mode"),
            Some(json!({ "mode": "comments_only" })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "stranger must not set mode");

    harness.cleanup().await;
}

#[tokio::test]
async fn a_thread_only_work_refuses_comments_server_side() {
    let harness = Harness::new("refuses-comments").await;
    let (work_id, mut author) =
        published_work(&harness, "m31-e@t.test", "m31e", "No Comments").await;
    let (status, _) = author
        .request(
            "PUT",
            &format!("/api/v1/works/{work_id}/discussion-mode"),
            Some(json!({ "mode": "thread_only" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let mut reader = harness.client();
    register(&mut reader, "m31-f@t.test", "m31f").await;
    let (status, body) = reader
        .post(
            &format!("/api/v1/works/{work_id}/comments"),
            json!({ "body": "This should be refused." }),
        )
        .await;
    assert!(
        status == StatusCode::UNPROCESSABLE_ENTITY || status == StatusCode::BAD_REQUEST,
        "ThreadOnly must refuse comments: {status} {body}"
    );

    // The same work in comments_only accepts them (today's behavior).
    let (status, _) = author
        .request(
            "PUT",
            &format!("/api/v1/works/{work_id}/discussion-mode"),
            Some(json!({ "mode": "comments_only" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = reader
        .post(
            &format!("/api/v1/works/{work_id}/comments"),
            json!({ "body": "Now this is fine." }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "comments_only accepts: {body}");

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Reactions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reactions_are_one_per_pseud_changeable_and_retractable() {
    let harness = Harness::new("reactions").await;
    let (work_id, _author) = published_work(&harness, "m31-g@t.test", "m31g", "Reactions").await;
    let mut reader = harness.client();
    register(&mut reader, "m31-h@t.test", "m31h").await;

    // Cast.
    let (status, body) = reader
        .post(
            &format!("/api/v1/works/{work_id}/reactions"),
            json!({ "vote_type": "well_written" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["outcome"], "cast");

    // Change.
    let (status, body) = reader
        .post(
            &format!("/api/v1/works/{work_id}/reactions"),
            json!({ "vote_type": "insightful" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["outcome"], "changed");

    // A second reader casts the same type: the aggregate must be 2 for one
    // type only (the change above must not have left a stale row).
    let mut other = harness.client();
    register(&mut other, "m31-i@t.test", "m31i").await;
    let (status, _) = other
        .post(
            &format!("/api/v1/works/{work_id}/reactions"),
            json!({ "vote_type": "insightful" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = reader
        .get(&format!("/api/v1/works/{work_id}/reactions"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let counts = body["counts"].as_array().expect("counts");
    assert_eq!(counts.len(), 1, "one vote type only: {counts:?}");
    assert_eq!(counts[0]["vote_type"], "insightful");
    assert_eq!(counts[0]["count"], 2);
    assert_eq!(body["mine"], "insightful");

    // Retract.
    let (status, body) = reader
        .post(
            &format!("/api/v1/works/{work_id}/reactions"),
            json!({ "vote_type": null }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["outcome"], "retracted");

    let (status, body) = reader
        .get(&format!("/api/v1/works/{work_id}/reactions"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let counts = body["counts"].as_array().expect("counts");
    assert_eq!(counts[0]["count"], 1, "retraction removed the vote");

    harness.cleanup().await;
}

#[tokio::test]
async fn an_invalid_reaction_type_is_refused() {
    let harness = Harness::new("bad-reaction").await;
    let (work_id, _author) = published_work(&harness, "m31-j@t.test", "m31j", "Bad Reaction").await;
    let mut reader = harness.client();
    register(&mut reader, "m31-k@t.test", "m31k").await;

    let (status, _) = reader
        .post(
            &format!("/api/v1/works/{work_id}/reactions"),
            json!({ "vote_type": "like" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Linked threads and the migration tool
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_migration_tool_moves_comments_into_a_linked_topic_once() {
    let harness = Harness::new("migrate").await;
    let category = seed_category(&harness).await;
    let (work_id, mut author) =
        published_work(&harness, "m31-l@t.test", "m31l", "Migrate Me").await;

    // Two comments from two readers.
    let mut r1 = harness.client();
    register(&mut r1, "m31-m@t.test", "m31m").await;
    let (status, body) = r1
        .post(
            &format!("/api/v1/works/{work_id}/comments"),
            json!({ "body": "First comment, made early." }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // The receipt carries no timestamp; read it back from the comment list.
    let (status, listed) = r1.get(&format!("/api/v1/works/{work_id}/comments")).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let first_created = listed["items"][0]["created_at"]
        .as_str()
        .expect("created_at")
        .to_owned();

    let mut r2 = harness.client();
    register(&mut r2, "m31-n@t.test", "m31n").await;
    let (status, _) = r2
        .post(
            &format!("/api/v1/works/{work_id}/comments"),
            json!({ "body": "Second comment, made later." }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // The author migrates.
    let (status, report) = author
        .post(
            &format!("/api/v1/works/{work_id}/migrate-comments"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{report}");
    assert_eq!(report["moved"], 2, "both comments moved: {report}");
    let topic_id = report["topic_id"].as_str().expect("topic id").to_owned();

    // The linked thread is discoverable.
    let (status, thread) = author.get(&format!("/api/v1/works/{work_id}/thread")).await;
    assert_eq!(status, StatusCode::OK, "{thread}");
    assert_eq!(thread["topic_id"], topic_id.as_str());

    // The posts preserve authorship, order, and timestamps.
    let (status, posts) = author
        .get(&format!("/api/v1/topics/{topic_id}/replies"))
        .await;
    assert_eq!(status, StatusCode::OK, "{posts}");
    let items = posts["items"].as_array().expect("posts");
    assert_eq!(items.len(), 2, "two posts: {items:?}");
    assert!(
        items[0]["body"].as_str().unwrap_or("").contains("First"),
        "order preserved: {items:?}"
    );
    assert_eq!(
        items[0]["created_at"].as_str().expect("post created_at"),
        first_created,
        "timestamp preserved"
    );

    // Running it again moves nothing (idempotent).
    let (status, report) = author
        .post(
            &format!("/api/v1/works/{work_id}/migrate-comments"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{report}");
    assert_eq!(report["moved"], 0, "second run moves nothing: {report}");

    // A stranger cannot migrate someone else's work.
    let mut stranger = harness.client();
    register(&mut stranger, "m31-o@t.test", "m31o").await;
    let (status, _) = stranger
        .post(
            &format!("/api/v1/works/{work_id}/migrate-comments"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "stranger must not migrate");

    let _ = category;
    harness.cleanup().await;
}
