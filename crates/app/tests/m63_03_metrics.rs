//! M63-03: the quality metrics through the real router.
//!
//! The store is covered in `crates/db/tests/quality_metrics.rs` and the maths in
//! `darkhold-domain`. These are the route properties:
//!
//! - a metric page carries `metric`, `ranked` and `total`, and every item carries
//!   a `score` — the gate is the difference between "best of 1,247" and "best of
//!   141,317", and it has to be in the response, not the documentation;
//! - a blocked metric is refused by name, with the work item that unblocks it,
//!   rather than silently reordered;
//! - `limit` does not move the gate, because the gate is a property of the
//!   population and the page is a window on it;
//! - `/rec/run` carries the same fields, so `dh rec --sort source_pct --engine rrf`
//!   prints the same gate as the catalogue route.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use darkhold_app::config::Config;
use darkhold_app::server;
use darkhold_app::state::AppState;
use darkhold_db::Database;
use serde_json::Value;
use test_support::TestDb;
use tower::ServiceExt;

const GOOD_PASSWORD: &str = "a-long-enough-passphrase";
const T0: &str = "2026-01-01T00:00:00Z";

struct Harness {
    _tdb: TestDb,
    _dir: std::path::PathBuf,
    config: Config,
    db: Database,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "darkhold-m63-03-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        let mut config = Config::development_defaults();
        config.storage.root = dir.clone();
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        let db = tdb.db().clone();
        Self {
            _tdb: tdb,
            _dir: dir,
            config,
            db,
        }
    }

    fn client(&self) -> Client {
        Client {
            app: server::build_router(AppState::new(self.config.clone(), self.db.clone())),
            cookies: Vec::new(),
        }
    }
}

struct Client {
    app: axum::Router,
    cookies: Vec<(String, String)>,
}

impl Client {
    async fn request(&mut self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        let cookies = self
            .cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        if !cookies.is_empty() {
            builder = builder.header(header::COOKIE, cookies);
        }
        if !matches!(method, "GET" | "HEAD" | "OPTIONS") {
            if let Some((_, token)) = self.cookies.iter().find(|(k, _)| k == "darkhold_csrf") {
                builder = builder.header("x-csrf-token", token.clone());
            }
        }
        let request = match body {
            Some(ref value) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(value).expect("serialise")))
                .expect("request"),
            None => builder.body(Body::empty()).expect("request"),
        };
        let response = self.app.clone().oneshot(request).await.expect("response");
        let status = response.status();
        for value in response.headers().get_all(header::SET_COOKIE) {
            let Ok(raw) = value.to_str() else { continue };
            let pair = raw.split(';').next().unwrap_or(raw);
            let Some((name, val)) = pair.split_once('=') else {
                continue;
            };
            self.cookies.retain(|(key, _)| key != name);
            if !val.is_empty() {
                self.cookies.push((name.to_owned(), val.to_owned()));
            }
        }
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
}

/// The same population the store test uses, cut down: two archives, one story in
/// each at a similar *rank*, and one row with no endorsement.
const SEED: &[(&str, &str, i64)] = &[
    ("sb-1", "sb", 1_000),
    ("sb-2", "sb", 2_000),
    ("sb-3", "sb", 3_000),
    ("sb-4", "sb", 4_000),
    ("sb-marvel", "sb", 33_107),
    ("sb-first", "sb", 45_000),
    ("qq-1", "qq", 5),
    ("qq-2", "qq", 10),
    ("qq-marvel", "qq", 4_179),
    ("sb-silent", "sb", 0),
];

async fn seed(db: &Database, account: &str) {
    for (key, source, likes) in SEED {
        match db.backend() {
            darkhold_db::Backend::Postgres => {
                sqlx::query(
                    "INSERT INTO library_items
                     (id, account_id, source_key, source_work_key, title, author_text, summary,
                      word_count, status, source_url, source_updated_at, last_synced_at, created_at,
                      updated_at, likes)
                     VALUES ($1::uuid, $2::uuid, $3, $4, $4, 'A', '', 40000,
                             'ongoing', 'https://example.invalid/x', $5, $5, $5, $5, $6)",
                )
                .bind(uuid::Uuid::new_v4())
                .bind(uuid::Uuid::parse_str(account).expect("uuid"))
                .bind(source)
                .bind(key)
                .bind(T0)
                .bind(likes)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("library item");
            }
            darkhold_db::Backend::Sqlite => {
                sqlx::query(
                    "INSERT INTO library_items
                     (id, account_id, source_key, source_work_key, title, author_text, summary,
                      word_count, status, source_url, source_updated_at, last_synced_at, created_at,
                      updated_at, likes)
                     VALUES (?, ?, ?, ?, ?, 'A', '', 40000,
                             'ongoing', 'https://example.invalid/x', ?, ?, ?, ?, ?)",
                )
                .bind(uuid::Uuid::new_v4().to_string())
                .bind(account)
                .bind(source)
                .bind(key)
                .bind(key)
                .bind(T0)
                .bind(T0)
                .bind(T0)
                .bind(T0)
                .bind(likes)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("library item");
            }
        }
    }
}

/// Register a reader and return the account id the catalogue rows hang off.
async fn reader(harness: &Harness, handle: &str) -> (Client, String) {
    let mut client = harness.client();
    let (status, body) = client
        .post(
            "/api/v1/auth/register",
            serde_json::json!({
                "email": format!("{handle}@example.com"),
                "password": GOOD_PASSWORD,
                "handle": handle,
                "display_name": handle,
                "age_band": "adult",
            }),
        )
        .await;
    assert!(status.is_success(), "register {handle}: {status} {body}");
    let account = match harness.db.backend() {
        darkhold_db::Backend::Postgres => sqlx::query_scalar::<_, String>(
            "SELECT id::text FROM accounts WHERE email = $1",
        )
        .bind(format!("{handle}@example.com"))
        .fetch_one(harness.db.postgres_pool().expect("postgres"))
        .await
        .expect("account id"),
        darkhold_db::Backend::Sqlite => {
            sqlx::query_scalar::<_, String>("SELECT id FROM accounts WHERE email = ?")
                .bind(format!("{handle}@example.com"))
                .fetch_one(harness.db.sqlite_pool().expect("sqlite"))
                .await
                .expect("account id")
        }
    };
    (client, account)
}

#[tokio::test]
async fn a_metric_page_carries_scores_the_gate_and_a_marked_unmeasured_block() {
    let harness = Harness::new("page").await;
    let (mut client, account) = reader(&harness, "m6303page").await;
    seed(&harness.db, &account).await;

    let (status, body) = client.get("/api/v1/catalogue?sort=source_pct&limit=20").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["metric"], "source_pct");
    assert_eq!(body["sort"], "source_pct");
    assert_eq!(body["ranked"], 9, "nine rows carry an endorsement: {body}");
    assert_eq!(body["total"], 10);

    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 10);
    assert!(
        items[0]["score"].as_f64().is_some(),
        "every ranked row carries a score: {}",
        items[0]
    );
    let last = items.last().expect("last");
    assert_eq!(last["source_work_key"], "sb-silent");
    assert_eq!(last["measured"], false, "a zero-like row is unmeasured");
    assert!(
        items[..9].iter().all(|item| item["measured"] == true),
        "measured rows precede the unmeasured block"
    );
    // The percentile itself: the top of qq and a strong sb row are both high,
    // even though raw likes are 8x apart.
    let sb = items
        .iter()
        .find(|item| item["source_work_key"] == "sb-marvel")
        .expect("sb copy")
        .clone();
    let qq = items
        .iter()
        .find(|item| item["source_work_key"] == "qq-marvel")
        .expect("qq copy")
        .clone();
    let (sb_score, qq_score) = (
        sb["score"].as_f64().expect("score"),
        qq["score"].as_f64().expect("score"),
    );
    assert!(
        (sb_score - qq_score).abs() < 0.2,
        "archive size must not decide the ranking: sb {sb_score}, qq {qq_score}"
    );
}

#[tokio::test]
async fn the_gate_does_not_move_with_the_page_size() {
    let harness = Harness::new("gate").await;
    let (mut client, account) = reader(&harness, "m6303gate").await;
    seed(&harness.db, &account).await;

    let (_, full) = client.get("/api/v1/catalogue?sort=quality").await;
    let (_, one) = client.get("/api/v1/catalogue?sort=quality&limit=1").await;
    assert_eq!(one["items"].as_array().expect("items").len(), 1);
    assert_eq!(one["ranked"], full["ranked"]);
    assert_eq!(one["total"], full["total"]);
}

#[tokio::test]
async fn a_blocked_metric_names_the_work_that_unblocks_it() {
    let harness = Harness::new("blocked").await;
    let mut client = harness.client();
    let (status, body) = client.get("/api/v1/catalogue?sort=velocity").await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a metric without data is a refusal, not a reordering: {body}"
    );
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("sort:"), "names the field: {message}");
    assert!(
        message.contains("M63-32"),
        "names the work that unblocks it: {message}"
    );
}

#[tokio::test]
async fn an_unknown_metric_is_refused_rather_than_silently_reordered() {
    let harness = Harness::new("unknown").await;
    let mut client = harness.client();
    let (status, body) = client.get("/api/v1/catalogue?sort=kudos").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body: {body}");
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("unknown sort"), "{message}");
}

#[tokio::test]
async fn rec_run_carries_the_metric_and_its_gate_too() {
    let harness = Harness::new("recrun").await;
    let (mut client, account) = reader(&harness, "m6303rec").await;
    seed(&harness.db, &account).await;

    let (status, body) = client
        .get("/api/v1/rec/run?engine=catalogue&sort=source_pct&limit=5")
        .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["engine"], "catalogue");
    assert_eq!(body["metric"], "source_pct");
    assert_eq!(body["sort"], "source_pct");
    assert_eq!(body["ranked"], 9);
    assert_eq!(body["total"], 10);
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 5);
    assert!(
        items[0]["rec_score"].as_f64().unwrap_or(0.0) > 0.0,
        "the metric score travels as rec_score: {}",
        items[0]
    );
    assert_eq!(items[0]["rec_rank"], 1);
}
