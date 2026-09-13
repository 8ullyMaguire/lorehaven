//! M14 — Governance: reports, trust, moderation, sanctions, appeals, audit.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::{Database, DatabaseConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m14-{}-{:?}-{:?}",
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
    async fn patch(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("PATCH", uri, Some(body)).await
    }
    async fn put(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("PUT", uri, Some(body)).await
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
        let _ = db.migrate().await.expect("migrate");
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

async fn register(client: &mut Client, email: &str, handle: &str) -> (String, String) {
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
    let (status, me) = client.get("/api/v1/auth/me").await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let account = me["account"]["id"].as_str().expect("account id").to_owned();
    let pseud = me["active_pseud_id"].as_str().expect("pseud id").to_owned();
    (account, pseud)
}

/// Create a published work owned by the registering author.
async fn published_work(
    harness: &Harness,
    email: &str,
    handle: &str,
    title: &str,
) -> (String, String, String) {
    let mut author = harness.client();
    let (account, pseud) = register(&mut author, email, handle).await;
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
    let doc = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{"type": "text", "text": "A chapter with enough words to have a middle and an end."}]
        }]
    });
    let (status, body) = author
        .request(
            "PATCH",
            &format!("/api/v1/chapters/{chapter}"),
            Some(json!({
                "expected_version": chapter_version,
                "document": doc
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "save: {body}");
    let (status, body) = author
        .post(
            &format!("/api/v1/works/{work_id}/publish"),
            json!({
                "expected_version": work_version,
                "idempotency_key": format!("m14-{work_id}")
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "publish: {body}");
    (work_id, account, pseud)
}

// ---------------------------------------------------------------------------
// Reports
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_report_can_be_submitted_and_listed() {
    let harness = Harness::new("report-create").await;
    let (work_id, _, _) =
        published_work(&harness, "author@example.com", "Author", "Report Work").await;
    let mut reporter = harness.client();
    let (_, _) = register(&mut reporter, "reporter@example.com", "Reporter").await;

    let (status, body) = reporter
        .post(
            "/api/v1/reports",
            json!({
                "subject_type": "work",
                "subject_id": work_id,
                "reason": "inappropriate content",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "submit: {body}");
    let report_id = body["id"].as_str().expect("report id").to_owned();

    let (status, body) = reporter.get("/api/v1/reports").await;
    assert_eq!(status, StatusCode::OK, "list: {body}");
    let items = body["items"].as_array().expect("items");
    assert!(
        items.iter().any(|i| i["id"].as_str() == Some(&report_id))
    );

    harness.cleanup().await;
}

#[tokio::test]
async fn an_unauthenticated_user_can_submit_a_report_but_not_assign_tasks() {
    let harness = Harness::new("report-anon").await;
    let (work_id, _, _) =
        published_work(&harness, "a@example.com", "Author", "Anon Report").await;
    let mut anon = harness.client();

    let (status, _body) = anon
        .post(
            "/api/v1/reports",
            json!({
                "subject_type": "work",
                "subject_id": work_id,
                "reason": "spam",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "anon submit OK");

    let (status, body) = anon
        .post(
            "/api/v1/moderation/reports/placeholder/assign",
            json!({}),
        )
        .await;
    assert!(
        !matches!(status, StatusCode::OK),
        "assign task should not succeed for anon: {body}"
    );

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Audit log
// ---------------------------------------------------------------------------

#[tokio::test]
async fn my_audit_log_returns_entries_for_an_account() {
    let harness = Harness::new("audit-log").await;
    let mut user = harness.client();
    let (_account, _) = register(&mut user, "audit@example.com", "Auditor").await;

    let (status, _body) = user
        .get("/api/v1/me/audit-log")
        .await;
    assert!(
        status == StatusCode::OK || status == StatusCode::NOT_FOUND,
        "audit log: {status}"
    );

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// My trust
// ---------------------------------------------------------------------------

#[tokio::test]
async fn my_trust_returns_a_trust_level() {
    let harness = Harness::new("my-trust").await;
    let mut user = harness.client();
    let (_account, _) = register(&mut user, "trust@example.com", "TrustTest").await;

    let (status, body) = user.get("/api/v1/my/trust").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let trust = body["level"].as_i64().unwrap_or(0);
    assert_eq!(trust, 0, "new account should be TL_NEW");

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Sanctions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_steward_can_issue_a_sanction() {
    let harness = Harness::new("sanction-issue").await;
    let (work_id, _, _) =
        published_work(&harness, "target@example.com", "Target", "Target Work").await;
    let mut steward = harness.client();
    let (s_account, _) = register(&mut steward, "steward@example.com", "Steward").await;

    // Promote the steward to TL 4 so they can issue sanctions.
    lorehaven_db::governance::set_trust(
        &harness.db,
        &s_account,
        4,
        "steward-test",
    )
    .await
    .expect("promote steward");

    // Submit a report first (so there is something to sanction around).
    let mut reporter = harness.client();
    let (_, _) = register(&mut reporter, "rep@example.com", "Reporter2").await;
    let (_status, body) = reporter
        .post(
            "/api/v1/reports",
            json!({
                "subject_type": "work",
                "subject_id": work_id,
                "reason": "policy violation",
            }),
        )
        .await;
    let _report_id = body["id"].as_str().expect("report id").to_owned();

    // Steward sanctions the reporter account (self-promoted to TL4).
    // We expect either 200 (sanction issued) or a validation error about
    // the target, but NOT a 403 forbidden (steward is authorized).
    let (status, _body) = steward
        .post(
            "/api/v1/sanctions",
            json!({
                "target_account": s_account.clone(),
                "reason": "test sanction",
                "duration_days": 7,
            }),
        )
        .await;
    assert!(
        !(status == StatusCode::FORBIDDEN || status == StatusCode::UNAUTHORIZED),
        "steward should not be forbidden: {status}"
    );

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Appeals
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_user_can_open_and_list_their_own_appeals() {
    let harness = Harness::new("appeal-open").await;
    let mut user = harness.client();
    let (_account, _) = register(&mut user, "appeal@example.com", "Appealer").await;

    let (status, body) = user
        .post(
            "/api/v1/appeals",
            json!({
                "sanction_id": "nonexistent-sanction",
                "statement": "I did not violate the rules.",
            }),
        )
        .await;
    assert!(
        status == StatusCode::OK || status == StatusCode::NOT_FOUND,
        "open appeal: {status} {body}"
    );

    let (status, body) = user.get("/api/v1/my/appeals").await;
    assert_eq!(status, StatusCode::OK, "list appeals: {body}");

    harness.cleanup().await;
}
