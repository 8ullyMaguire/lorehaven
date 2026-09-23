//! M18 Phase 3 — Engagement Layer (spec §9.7.1, §9.8, §9.9).
//!
//! Tests cover streaks (login tracking, milestone detection), lifecycle
//! incentives (completion bonus, resurrection reward), and taste-weighted
//! notification queueing.

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
        "lorehaven-taste-engage-{tag}-{}-{:?}",
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

async fn register(client: &mut Client, email: &str, handle: &str) -> String {
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
    me["account"]["id"].as_str().expect("account id").to_owned()
}

/// Insert a test account directly into the database and return its ID.
async fn insert_test_account(db: &lorehaven_db::Database, email: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let now = lorehaven_db::identity::now_rfc3339();
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO accounts (id, status, email, age_state, created_at, updated_at, version)
                 VALUES (?, 'active', ?, 'adult', ?, ?, 1)"
            )
            .bind(&id)
            .bind(email)
            .bind(&now)
            .bind(&now)
            .execute(db.sqlite_pool().unwrap())
            .await
            .expect("insert test account");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO accounts (id, status, email, age_state, created_at, updated_at, version)
                 VALUES ($1::uuid, 'active', $2, 'adult', $3, $4, 1)"
            )
            .bind(&id)
            .bind(email)
            .bind(&now)
            .bind(&now)
            .execute(db.postgres_pool().unwrap())
            .await
            .expect("insert test account");
        }
    }
    id
}

/// Insert a test account + pseud + work directly and return the work ID.
async fn insert_test_work(db: &lorehaven_db::Database, title: &str) -> String {
    let account_id = uuid::Uuid::new_v4().to_string();
    let pseud_id = uuid::Uuid::new_v4().to_string();
    let work_id = uuid::Uuid::new_v4().to_string();
    let now = lorehaven_db::identity::now_rfc3339();
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO accounts (id, status, email, age_state, created_at, updated_at, version)
                 VALUES (?, 'active', ?, 'adult', ?, ?, 1)"
            )
            .bind(&account_id)
            .bind(format!("{account_id}@test"))
            .bind(&now)
            .bind(&now)
            .execute(db.sqlite_pool().unwrap())
            .await
            .expect("insert account");
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at, version)
                 VALUES (?, ?, ?, ?, ?, ?, 1)"
            )
            .bind(&pseud_id)
            .bind(&account_id)
            .bind("testpseud")
            .bind("Test Pseud")
            .bind(&now)
            .bind(&now)
            .execute(db.sqlite_pool().unwrap())
            .await
            .expect("insert pseud");
            sqlx::query(
                "INSERT INTO works (id, owner_pseud_id, title, lifecycle, completion, created_at, updated_at, version)
                 VALUES (?, ?, ?, 'draft', 'in_progress', ?, ?, 1)"
            )
            .bind(&work_id)
            .bind(&pseud_id)
            .bind(title)
            .bind(&now)
            .bind(&now)
            .execute(db.sqlite_pool().unwrap())
            .await
            .expect("insert work");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO accounts (id, status, email, age_state, created_at, updated_at, version)
                 VALUES ($1::uuid, 'active', $2, 'adult', $3, $4, 1)"
            )
            .bind(&account_id)
            .bind(format!("{account_id}@test"))
            .bind(&now)
            .bind(&now)
            .execute(db.postgres_pool().unwrap())
            .await
            .expect("insert account");
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at, version)
                 VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6, 1)"
            )
            .bind(&pseud_id)
            .bind(&account_id)
            .bind("testpseud")
            .bind("Test Pseud")
            .bind(&now)
            .bind(&now)
            .execute(db.postgres_pool().unwrap())
            .await
            .expect("insert pseud");
            sqlx::query(
                "INSERT INTO works (id, owner_pseud_id, title, lifecycle, completion, created_at, updated_at, version)
                 VALUES ($1::uuid, $2::uuid, $3, 'draft', 'in_progress', $4, $5, 1)"
            )
            .bind(&work_id)
            .bind(&pseud_id)
            .bind(title)
            .bind(&now)
            .bind(&now)
            .execute(db.postgres_pool().unwrap())
            .await
            .expect("insert work");
        }
    }
    work_id
}

#[tokio::test]
async fn streak_first_login_starts_streak_at_1() {
    let harness = Harness::new("streak-first-login").await;
    let db = harness.tdb.db().clone();
    let account = insert_test_account(&db, "streak1@t.test").await;

    let state = lorehaven_db::engagement::record_login(&db, &account)
        .await
        .expect("record login");
    assert_eq!(state.current, 1);
    assert_eq!(state.longest, 1);
    assert!(state.last_login_at.is_some());
}

#[tokio::test]
async fn streak_repeated_login_same_day_is_idempotent() {
    let harness = Harness::new("streak-same-day").await;
    let db = harness.tdb.db().clone();
    let account = insert_test_account(&db, "streak2@t.test").await;

    lorehaven_db::engagement::record_login(&db, &account)
        .await
        .expect("login 1");
    let state = lorehaven_db::engagement::record_login(&db, &account)
        .await
        .expect("login 2");
    assert_eq!(
        state.current, 1,
        "second login same day should not increment"
    );
}

#[tokio::test]
async fn lifecycle_event_fires_once_then_replays_as_false() {
    let harness = Harness::new("lifecycle-once").await;
    let db = harness.tdb.db().clone();
    let work_id = insert_test_work(&db, "lifecycle work").await;

    let first = lorehaven_db::engagement::record_lifecycle_event(&db, &work_id, "completion")
        .await
        .expect("record 1");
    assert!(first, "first completion should fire");
    let second = lorehaven_db::engagement::record_lifecycle_event(&db, &work_id, "completion")
        .await
        .expect("record 2");
    assert!(!second, "duplicate completion should not re-fire");
}

#[tokio::test]
async fn lifecycle_resurrection_distinct_from_completion() {
    let harness = Harness::new("lifecycle-resurrect").await;
    let db = harness.tdb.db().clone();
    let work_id = insert_test_work(&db, "rezz work").await;

    let c = lorehaven_db::engagement::record_lifecycle_event(&db, &work_id, "completion")
        .await
        .expect("completion");
    let r = lorehaven_db::engagement::record_lifecycle_event(&db, &work_id, "resurrection")
        .await
        .expect("resurrection");
    assert!(c && r, "distinct event types should both fire");
}

#[tokio::test]
async fn taste_notification_queue_and_drain() {
    let harness = Harness::new("taste-notif-queue").await;
    let db = harness.tdb.db().clone();
    let account = insert_test_account(&db, "notif@t.test").await;
    let work_id = insert_test_work(&db, "notif work").await;

    lorehaven_db::engagement::queue_taste_notification(&db, &work_id, &account, 0.75)
        .await
        .expect("queue");

    // Re-queue updates score but does not duplicate.
    lorehaven_db::engagement::queue_taste_notification(&db, &work_id, &account, 0.85)
        .await
        .expect("re-queue");

    let pending = lorehaven_db::engagement::pending_taste_notification_count(&db, &account)
        .await
        .expect("count");
    assert_eq!(pending, 1, "re-queue should not duplicate the row");

    let drained = lorehaven_db::engagement::drain_taste_notification_queue(&db, 10)
        .await
        .expect("drain");
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].0, work_id);
    assert_eq!(drained[0].1, account);
    assert!((drained[0].2 - 0.85).abs() < 1e-9);

    let pending_after = lorehaven_db::engagement::pending_taste_notification_count(&db, &account)
        .await
        .expect("count after");
    assert_eq!(pending_after, 0);
}

#[tokio::test]
async fn days_to_iso_date_known_dates() {
    // 1970-01-01 is day 0.
    assert_eq!(
        lorehaven_db::engagement::__days_to_iso_date_for_test(0),
        "1970-01-01"
    );
    assert_eq!(
        lorehaven_db::engagement::__days_to_iso_date_for_test(1),
        "1970-01-02"
    );
    // 2024-01-01 is day 19723.
    assert_eq!(
        lorehaven_db::engagement::__days_to_iso_date_for_test(19723),
        "2024-01-01"
    );
    // 2026-09-22 is day 20718.
    assert_eq!(
        lorehaven_db::engagement::__days_to_iso_date_for_test(20718),
        "2026-09-22"
    );
}

#[tokio::test]
async fn login_records_streak_and_endpoint_reports_it() {
    let harness = Harness::new("streak-endpoint").await;
    let mut client = harness.client();
    let account = register(&mut client, "streak3@t.test", "streak3").await;

    // Login again through the API to trigger record_login in the login route.
    let (status, _body) = client
        .post(
            "/api/v1/auth/login",
            json!({ "email": "streak3@t.test", "password": PASSWORD }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "login should succeed");

    let (status, body) = client.get("/api/v1/me/streak").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["current_streak"], 1,
        "one login should give a streak of 1"
    );
    assert!(body["longest_streak"].as_i64().unwrap() >= 1);
    assert!(
        body["last_login_at"].is_string(),
        "last login timestamp present: {body}"
    );
}

#[tokio::test]
async fn streak_endpoint_requires_session() {
    let harness = Harness::new("streak-anon").await;
    let mut client = harness.client();
    let (status, _body) = client.get("/api/v1/me/streak").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
