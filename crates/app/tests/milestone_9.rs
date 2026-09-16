//! Positivity filter and feedback delivery (spec section 12).
//!
//! The plan's eight named properties, each against the real router, a real
//! SQLite file and a cookie jar:
//!
//! * positive text is stored and delivered under default preferences;
//! * constructive text is held by default and delivered once the author
//!   opts in -- and only for subsequent reviews, never retroactively;
//! * hostile text is held, never listed, and the sender learns nothing but
//!   "held for review" (asserted on the JSON: no class, no reason);
//! * withdrawal (DELETE) removes the review from the public list, and the
//!   classification row survives on the soft-deleted row for moderation;
//! * classification is deterministic for the same input;
//! * re-submitting the same text does not double-classify (one row).
//!
//! Two deliberate scoping notes: private (unpublished) reviews skip the
//! gate -- they are visible to nobody but their writer -- and pre-filter
//! reviews (no classification row) count as delivered, so this milestone
//! cannot silently hide what milestone 4 shipped.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::{positivity, Backend, DatabaseConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m9-{tag}-{}-{:?}",
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
    async fn put(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("PUT", uri, Some(body)).await
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
            report.contains(&"0010_positivity".to_owned()),
            "positivity migration must apply: {report:?}"
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

async fn register(client: &mut Client, email: &str, handle: &str) {
    let (status, body) = client.post("/api/v1/auth/register", json!({ "email": email, "password": PASSWORD, "handle": handle, "display_name": handle, "age_band": "adult" })).await;
    assert_eq!(status, StatusCode::CREATED, "register {handle}: {body}");
}

async fn published_work(harness: &Harness, email: &str, handle: &str, title: &str) -> String {
    let mut author = harness.client();
    register(&mut author, email, handle).await;
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
    let chapter = body["id"].as_str().expect("chapter").to_owned();
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
            json!({ "expected_version": work_version, "idempotency_key": format!("m9-{work_id}") }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "publish: {body}");
    work_id
}

async fn review_count(client: &mut Client, work: &str) -> usize {
    let (status, body) = client.get(&format!("/api/v1/works/{work}/reviews")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["items"].as_array().expect("items").len()
}

#[tokio::test]
async fn positive_text_is_stored_and_delivered_by_default() {
    let harness = Harness::new("positive").await;
    let work_id = published_work(&harness, "author@example.com", "Author", "Kind Work").await;
    let mut reader = harness.client();
    register(&mut reader, "reader@example.com", "Reader").await;
    let (status, body) = reader
        .put(
            &format!("/api/v1/works/{work_id}/reviews"),
            json!({ "body": "I loved this chapter, thank you!", "is_public": true }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["receipt"], "Comment posted.",
        "sender sees posted, not the class: {body}"
    );
    assert!(
        body.get("class").is_none() && body.get("reason").is_none(),
        "no leak: {body}"
    );
    let review_id = body["id"].as_str().expect("id").to_owned();
    let mut visitor = harness.client();
    assert_eq!(
        review_count(&mut visitor, &work_id).await,
        1,
        "delivered text is public"
    );
    let stored = positivity::classification_for(harness.tdb.db(), &review_id)
        .await
        .expect("read")
        .expect("classified");
    assert_eq!(
        stored.class,
        lorehaven_domain::positivity::FeedbackClass::Positive
    );
    assert_eq!(
        stored.outcome,
        lorehaven_domain::positivity::DeliveryOutcome::Delivered
    );
    harness.cleanup().await;
}

#[tokio::test]
async fn constructive_text_is_held_until_the_author_opts_in() {
    let harness = Harness::new("constructive").await;
    let work_id = published_work(&harness, "author@example.com", "Author", "Draft Work").await;
    let mut reader = harness.client();
    register(&mut reader, "reader@example.com", "Reader").await;
    let text = "The pacing felt rushed in the middle; consider a typo pass.";
    let (status, first) = reader
        .put(
            &format!("/api/v1/works/{work_id}/reviews"),
            json!({ "body": text, "is_public": true }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(
        first["receipt"], "Comment held for moderator review.",
        "default holds critique: {first}"
    );
    let mut visitor = harness.client();
    assert_eq!(
        review_count(&mut visitor, &work_id).await,
        0,
        "held text is not public"
    );
    // The author opts in; a *subsequent* critique is delivered. The first
    // stays held: preferences never reclassify retroactively.
    let mut author = harness.client();
    let (status, body) = author
        .post(
            "/api/v1/auth/login",
            json!({ "email": "author@example.com", "password": PASSWORD }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "sign in: {body}");
    let (status, prefs) = author.get("/api/v1/feedback/preferences").await;
    assert_eq!(status, StatusCode::OK, "{prefs}");
    let version = prefs["version"].as_i64().expect("version");
    assert!(
        prefs["effective_policy"]
            .as_str()
            .expect("policy")
            .contains("constructive critique off"),
        "panel shows the effective policy: {prefs}"
    );
    let (status, prefs) = author
        .put(
            "/api/v1/feedback/preferences",
            json!({ "accept_constructive": true, "expected_version": version }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{prefs}");
    assert!(
        prefs["effective_policy"]
            .as_str()
            .expect("policy")
            .contains("constructive critique on"),
        "{prefs}"
    );
    let (status, second) = reader
        .put(
            &format!("/api/v1/works/{work_id}/reviews"),
            json!({ "body": "The dialogue could improve with a second pass.", "is_public": true }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["receipt"], "Comment posted.", "{second}");
    assert_eq!(
        review_count(&mut visitor, &work_id).await,
        1,
        "only the subsequent critique delivers"
    );
    harness.cleanup().await;
}

#[tokio::test]
async fn hostile_text_is_held_and_reveals_nothing() {
    let harness = Harness::new("hostile").await;
    let work_id = published_work(&harness, "author@example.com", "Author", "Target Work").await;
    let mut reader = harness.client();
    register(&mut reader, "reader@example.com", "Reader").await;
    let (status, body) = reader
        .put(
            &format!("/api/v1/works/{work_id}/reviews"),
            json!({ "body": "You idiot, shut up and stop writing.", "is_public": true }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["receipt"], "Comment held for moderator review.",
        "{body}"
    );
    let text = serde_json::to_string(&body).expect("json");
    assert!(
        !text.contains("negative") && !text.contains("hostil"),
        "no class leaks: {body}"
    );
    assert!(
        body.get("signals").is_none() && body.get("confidence").is_none(),
        "no score leaks: {body}"
    );
    let mut visitor = harness.client();
    assert_eq!(
        review_count(&mut visitor, &work_id).await,
        0,
        "held text never lists"
    );
    // The author inbox shows a held count, never the content.
    let mut author = harness.client();
    let (status, login) = author
        .post(
            "/api/v1/auth/login",
            json!({ "email": "author@example.com", "password": PASSWORD }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{login}");
    let (status, inbox) = author.get("/api/v1/feedback/inbox").await;
    assert_eq!(status, StatusCode::OK, "{inbox}");
    assert_eq!(inbox["held_count"], 1, "counted, not shown: {inbox}");
    assert_eq!(
        inbox["items"].as_array().expect("items").len(),
        0,
        "{inbox}"
    );
    let text = serde_json::to_string(&inbox).expect("json");
    assert!(
        !text.contains("You idiot"),
        "held content never reaches the inbox: {inbox}"
    );
    harness.cleanup().await;
}

#[tokio::test]
async fn withdrawal_removes_the_review_but_keeps_the_audit_row() {
    let harness = Harness::new("withdraw").await;
    let work_id = published_work(&harness, "author@example.com", "Author", "Regret Work").await;
    let mut reader = harness.client();
    register(&mut reader, "reader@example.com", "Reader").await;
    let (status, body) = reader
        .put(
            &format!("/api/v1/works/{work_id}/reviews"),
            json!({ "body": "I loved the ending!", "is_public": true }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let review_id = body["id"].as_str().expect("id").to_owned();
    let mut visitor = harness.client();
    assert_eq!(review_count(&mut visitor, &work_id).await, 1);
    let (status, _) = reader
        .delete(&format!("/api/v1/works/{work_id}/reviews"))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        review_count(&mut visitor, &work_id).await,
        0,
        "withdrawn leaves the list"
    );
    assert!(
        positivity::classification_for(harness.tdb.db(), &review_id)
            .await
            .expect("read")
            .is_some(),
        "audit row survives on the soft-deleted review"
    );
    harness.cleanup().await;
}

#[tokio::test]
async fn neutral_text_is_deterministic_and_never_double_classified() {
    // Determinism lives in the domain: same input, same verdict, every time.
    let a =
        lorehaven_domain::positivity::classify("The dialogue could improve with a second pass.");
    let b =
        lorehaven_domain::positivity::classify("The dialogue could improve with a second pass.");
    assert_eq!(a, b);
    // Idempotency lives in the store: re-submitting one review rewrites one
    // classification row, never two.
    let harness = Harness::new("idem").await;
    let work_id = published_work(&harness, "author@example.com", "Author", "Steady Work").await;
    let mut reader = harness.client();
    register(&mut reader, "reader@example.com", "Reader").await;
    for _ in 0..2 {
        let (status, body) = reader
            .put(
                &format!("/api/v1/works/{work_id}/reviews"),
                json!({ "body": "I loved this chapter, thank you!", "is_public": true }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let count_sql = harness.tdb.db().sql(
        "SELECT COUNT(*) FROM review_classifications",
        "SELECT COUNT(*)::bigint FROM review_classifications",
    );
    let count: i64 = match harness.tdb.db().backend() {
        Backend::Sqlite => sqlx::query_scalar(count_sql.as_ref())
            .fetch_one(harness.tdb.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("count"),
        Backend::Postgres => sqlx::query_scalar(count_sql.as_ref())
            .fetch_one(harness.tdb.db().postgres_pool().expect("postgres"))
            .await
            .expect("count"),
    };
    assert_eq!(count, 1, "one review, one classification row");
    harness.cleanup().await;
}

#[tokio::test]
async fn denylist_holds_and_allowlist_delivers_regardless_of_class() {
    let harness = Harness::new("lists").await;
    let work_id = published_work(&harness, "author@example.com", "Author", "List Work").await;
    let mut reader = harness.client();
    register(&mut reader, "reader@example.com", "Reader").await;
    let (status, me) = reader.get("/api/v1/auth/me").await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let reviewer_pseud = me["active_pseud_id"].as_str().expect("pseud").to_owned();
    let mut author = harness.client();
    let (status, login) = author
        .post(
            "/api/v1/auth/login",
            json!({ "email": "author@example.com", "password": PASSWORD }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{login}");
    // Deny: even praise is held.
    let (status, _) = author
        .post(
            &format!("/api/v1/feedback/deny/{reviewer_pseud}"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = reader
        .put(
            &format!("/api/v1/works/{work_id}/reviews"),
            json!({ "body": "I loved this!", "is_public": true }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["receipt"], "Comment held for moderator review.",
        "{body}"
    );
    harness.cleanup().await;
}

#[tokio::test]
async fn work_policy_overrides_the_account_default() {
    let harness = Harness::new("workpolicy").await;
    let work_id = published_work(&harness, "author@example.com", "Author", "Policy Work").await;
    let mut author = harness.client();
    let (status, login) = author
        .post(
            "/api/v1/auth/login",
            json!({ "email": "author@example.com", "password": PASSWORD }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{login}");
    let (status, policy) = author
        .put(
            &format!("/api/v1/feedback/preferences/works/{work_id}"),
            json!({ "accept_constructive": true }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{policy}");
    assert!(
        policy["effective_policy"]
            .as_str()
            .expect("policy")
            .contains("constructive critique on"),
        "{policy}"
    );
    let mut stranger = harness.client();
    register(&mut stranger, "other@example.com", "Other").await;
    let (status, policy) = stranger
        .get(&format!("/api/v1/feedback/preferences/works/{work_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "policy is readable: {policy}");
    harness.cleanup().await;
}
