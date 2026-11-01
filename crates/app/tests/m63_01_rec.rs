//! M63-01: the catalogue's fandom filter and `/api/v1/rec/run`, driven through
//! the real router.
//!
//! The store-level properties are in `crates/db/tests/catalogue_fandom.rs`. These
//! are the ones that are properties of the *routes*:
//!
//! - an unknown fandom is a `422` that names the field and suggests the nearest
//!   known one, because an empty page reads as "this fandom has nothing" and the
//!   two are different answers;
//! - a fandom-scoped page carries the coverage numbers, so a caller can say "265
//!   of 7,626 tagged" instead of implying the catalogue is fully tagged;
//! - `/rec/run` refuses an engine name that is not enabled, rather than falling
//!   back to the instance default without saying so;
//! - `engine=catalogue` runs no strategies, and says so by omitting the array —
//!   an empty array would read as "every strategy produced nothing";
//! - `engine=rrf` with `explain=1` reports every registered strategy, including
//!   the ones that contributed nothing. That is the finding an operator needs and
//!   the blended page cannot show.

use std::path::Path;

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

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "darkhold-m63-01-{tag}-{}-{:?}",
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
    // Pluggable, so the strategy registry is the engine under test. The filters
    // and the catalogue engine do not depend on this; `engine=` overrides it.
    config.discovery.rec_mode = darkhold_app::config::RecMode::Pluggable;
    config
}

struct Harness {
    _tdb: TestDb,
    _dir: std::path::PathBuf,
    config: Config,
    db: Database,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let config = config_for(&dir);
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

    async fn reader(&self, handle: &str) -> Client {
        let mut client = self.client();
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
        client
    }
}

struct Client {
    app: axum::Router,
    cookies: Vec<(String, String)>,
}

impl Client {
    fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn capture_cookies(&mut self, response: &axum::response::Response) {
        for value in response.headers().get_all(header::SET_COOKIE) {
            let Ok(raw) = value.to_str() else { continue };
            let pair = raw.split(';').next().unwrap_or(raw);
            let Some((name, val)) = pair.split_once('=') else {
                continue;
            };
            let (name, val) = (name.to_owned(), val.to_owned());
            self.cookies.retain(|(key, _)| key != &name);
            if !val.is_empty() {
                self.cookies.push((name, val));
            }
        }
    }

    fn cookie_header(&self) -> String {
        self.cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    async fn request(
        &mut self,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        let cookies = self.cookie_header();
        if !cookies.is_empty() {
            builder = builder.header(header::COOKIE, cookies);
        }
        if !matches!(method, "GET" | "HEAD" | "OPTIONS") {
            if let Some(token) = self.cookie("darkhold_csrf") {
                builder = builder.header("x-csrf-token", token.to_owned());
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
}

/// Two catalogue rows — one tagged `Naruto`, one untagged — plus the fandom node.
///
/// The untagged row is the witness for the filter; the tagged one is the witness
/// for the coverage line, which must say `2 rows, 1 tagged`.
async fn seed_catalogue(db: &Database, account: &str) -> (String, String) {
    let pseud = uuid::Uuid::new_v4().to_string();
    let node = uuid::Uuid::new_v4().to_string();
    let tagged_work = uuid::Uuid::new_v4().to_string();
    let bare_work = uuid::Uuid::new_v4().to_string();
    let t0 = "2026-01-01T00:00:00Z";
    let tagged_id = uuid::Uuid::new_v4().to_string();
    let bare_id = uuid::Uuid::new_v4().to_string();

    match db.backend() {
        darkhold_db::Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES ($1::uuid, $2::uuid, 'tagger', 'tagger', $3, $3)",
            )
            .bind(uuid::Uuid::parse_str(&pseud).expect("uuid"))
            .bind(uuid::Uuid::parse_str(account).expect("uuid"))
            .bind(t0)
            .execute(pool)
            .await
            .expect("pseud");
            sqlx::query(
                "INSERT INTO canonical_entities (id, kind, canonical, norm, created_at, review_status, signal_count)
                 VALUES ($1::uuid, 'fandom', 'Naruto', 'naruto', $2, 'confirmed', 0)",
            )
            .bind(uuid::Uuid::parse_str(&node).expect("uuid"))
            .bind(t0)
            .execute(pool)
            .await
            .expect("fandom node");
            for (work, title) in [(&tagged_work, "Tagged"), (&bare_work, "Bare")] {
                sqlx::query(
                    "INSERT INTO works (id, owner_pseud_id, title, summary, lifecycle, visibility, completion, created_at, updated_at, generated_content_posture)
                     VALUES ($1::uuid, $2::uuid, $3, '', 'published', 'public', 'complete', $4, $4, 'forbid')",
                )
                .bind(uuid::Uuid::parse_str(work).expect("uuid"))
                .bind(uuid::Uuid::parse_str(&pseud).expect("uuid"))
                .bind(title)
                .bind(t0)
                .execute(pool)
                .await
                .expect("work");
            }
            sqlx::query(
                "INSERT INTO work_tags (work_id, node_id, prominence, added_at)
                 VALUES ($1::uuid, $2::uuid, 1, $3)",
            )
            .bind(uuid::Uuid::parse_str(&tagged_work).expect("uuid"))
            .bind(uuid::Uuid::parse_str(&node).expect("uuid"))
            .bind(t0)
            .execute(pool)
            .await
            .expect("tag");
            for (id, key, work, likes) in [
                (&tagged_id, "naruto-fic", Some(&tagged_work), 50),
                (&bare_id, "bare-fic", Some(&bare_work), 900),
            ] {
                sqlx::query(
                    "INSERT INTO library_items
                     (id, account_id, work_id, source_key, source_work_key, title, author_text, summary,
                      word_count, status, source_url, source_updated_at, last_synced_at, created_at, updated_at, likes)
                     VALUES ($1::uuid, $2::uuid, $3::uuid, 'sb', $4, $4, 'A', '', 1000,
                             'ongoing', 'https://example.invalid/x', $5, $5, $5, $5, $6)",
                )
                .bind(uuid::Uuid::parse_str(id).expect("uuid"))
                .bind(uuid::Uuid::parse_str(account).expect("uuid"))
                .bind(work.map(|w| uuid::Uuid::parse_str(w).expect("uuid")))
                .bind(key)
                .bind(t0)
                .bind(likes)
                .execute(pool)
                .await
                .expect("library item");
            }
        }
        darkhold_db::Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES (?, ?, 'tagger', 'tagger', ?, ?)",
            )
            .bind(&pseud)
            .bind(account)
            .bind(t0)
            .bind(t0)
            .execute(pool)
            .await
            .expect("pseud");
            sqlx::query(
                "INSERT INTO canonical_entities (id, kind, canonical, norm, created_at, review_status, signal_count)
                 VALUES (?, 'fandom', 'Naruto', 'naruto', ?, 'confirmed', 0)",
            )
            .bind(&node)
            .bind(t0)
            .execute(pool)
            .await
            .expect("fandom node");
            for (work, title) in [(&tagged_work, "Tagged"), (&bare_work, "Bare")] {
                sqlx::query(
                    "INSERT INTO works (id, owner_pseud_id, title, summary, lifecycle, visibility, completion, created_at, updated_at, generated_content_posture)
                     VALUES (?, ?, ?, '', 'published', 'public', 'complete', ?, ?, 'forbid')",
                )
                .bind(work)
                .bind(&pseud)
                .bind(title)
                .bind(t0)
                .bind(t0)
                .execute(pool)
                .await
                .expect("work");
            }
            sqlx::query("INSERT INTO work_tags (work_id, node_id, prominence, added_at) VALUES (?, ?, 1, ?)")
                .bind(&tagged_work)
                .bind(&node)
                .bind(t0)
                .execute(pool)
                .await
                .expect("tag");
            for (id, key, work, likes) in [
                (&tagged_id, "naruto-fic", Some(&tagged_work), 50),
                (&bare_id, "bare-fic", Some(&bare_work), 900),
            ] {
                sqlx::query(
                    "INSERT INTO library_items
                     (id, account_id, work_id, source_key, source_work_key, title, author_text, summary,
                      word_count, status, source_url, source_updated_at, last_synced_at, created_at, updated_at, likes)
                     VALUES (?, ?, ?, 'sb', ?, ?, 'A', '', 1000,
                             'ongoing', 'https://example.invalid/x', ?, ?, ?, ?, ?)",
                )
                .bind(id)
                .bind(account)
                .bind(work)
                .bind(key)
                .bind(key)
                .bind(t0)
                .bind(t0)
                .bind(t0)
                .bind(t0)
                .bind(likes)
                .execute(pool)
                .await
                .expect("library item");
            }
        }
    }

    (tagged_id, bare_id)
}

async fn account_of(db: &Database, handle: &str) -> String {
    let email = format!("{handle}@example.com");
    let sql = db.sql(
        "SELECT id FROM accounts WHERE email = ?",
        "SELECT id::text FROM accounts WHERE email = $1",
    );
    let row: (String,) = match db.backend() {
        darkhold_db::Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(&email)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("account")
        }
        darkhold_db::Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(&email)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await
                .expect("account")
        }
    };
    row.0
}

fn item_keys(page: &Value) -> Vec<String> {
    page["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["source_work_key"].as_str().unwrap_or("?").to_owned())
        .collect()
}

#[tokio::test]
async fn the_catalogue_route_filters_by_fandom_and_reports_coverage() {
    let harness = Harness::new("filter").await;
    let mut reader = harness.reader("m6301reader").await;
    let account = account_of(&harness.db, "m6301reader").await;
    let (tagged, _bare) = seed_catalogue(&harness.db, &account).await;

    let (status, body) = reader.get("/api/v1/catalogue?fandom=naruto").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(item_keys(&body), vec!["naruto-fic"], "only the tagged row");
    assert_eq!(body["fandom"], "Naruto", "the resolved spelling is echoed");
    assert_eq!(body["fandom_tagged_rows"], 1);
    assert_eq!(body["catalogue_rows"], 2);
    assert!(!tagged.is_empty());

    // Case-insensitive on the way in, as the CLI's `--fandom NaRuTo` will be.
    let (status, body) = reader.get("/api/v1/catalogue?fandom=NaRuTo").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(item_keys(&body), vec!["naruto-fic"]);

    // Without the filter both rows are there — the filter is not hiding the
    // untagged row from every query.
    let (status, body) = reader.get("/api/v1/catalogue").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().expect("items").len(), 2);
    assert!(
        body.get("fandom").is_none(),
        "no fandom filter, no fandom echo"
    );
}

#[tokio::test]
async fn an_unknown_fandom_names_the_field_and_suggests() {
    let harness = Harness::new("suggest").await;
    let mut reader = harness.reader("m6301typo").await;
    let account = account_of(&harness.db, "m6301typo").await;
    seed_catalogue(&harness.db, &account).await;

    let (status, body) = reader.get("/api/v1/catalogue?fandom=narut").await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "an unknown fandom is a validation failure, not an empty page: {body}"
    );
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("did you mean \"Naruto\""),
        "the suggestion is the useful half: {message}"
    );
    assert!(
        message.contains("fandom:"),
        "the message names the field: {message}"
    );
}

#[tokio::test]
async fn the_fandom_list_carries_counts() {
    let harness = Harness::new("list").await;
    let mut reader = harness.reader("m6301list").await;
    let account = account_of(&harness.db, "m6301list").await;
    seed_catalogue(&harness.db, &account).await;

    let (status, body) = reader.get("/api/v1/catalogue/fandoms").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let listed = body.as_array().expect("array");
    assert_eq!(listed.len(), 1, "one fandom is tagged: {body}");
    assert_eq!(listed[0]["name"], "Naruto");
    assert_eq!(listed[0]["count"], 1);
}

#[tokio::test]
async fn rec_run_needs_a_session() {
    let harness = Harness::new("anon").await;
    let mut client = harness.client();
    let (status, _) = client.get("/api/v1/rec/run?engine=catalogue").await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "the engines read the caller's history; they are not a public surface"
    );
}

#[tokio::test]
async fn engine_catalogue_is_the_metric_order_and_runs_no_strategies() {
    let harness = Harness::new("cat").await;
    let mut reader = harness.reader("m6301cat").await;
    let account = account_of(&harness.db, "m6301cat").await;
    seed_catalogue(&harness.db, &account).await;

    let (status, body) = reader
        .get("/api/v1/rec/run?fandom=naruto&engine=catalogue&explain=1")
        .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["engine"], "catalogue");
    assert_eq!(item_keys(&body), vec!["naruto-fic"]);
    assert!(
        body.get("strategies").is_none(),
        "no strategies ran, so there is nothing to explain — an empty array would \
         read as 'every strategy produced nothing': {body}"
    );
    assert!(
        body["items"][0].get("rec_score").is_none(),
        "the catalogue engine ranks nothing, so there is no score to show"
    );
}

#[tokio::test]
async fn engine_rrf_ranks_the_head_and_covers_every_strategy() {
    let harness = Harness::new("rrf").await;
    let mut reader = harness.reader("m6301rrf").await;
    let account = account_of(&harness.db, "m6301rrf").await;
    seed_catalogue(&harness.db, &account).await;

    let (status, body) = reader
        .get("/api/v1/rec/run?fandom=naruto&engine=rrf&explain=1")
        .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["engine"], "rrf");
    assert_eq!(body["candidates"], 1);

    let strategies = body["strategies"].as_array().expect("strategies");
    assert!(
        !strategies.is_empty(),
        "every registered strategy is reported, including the ones with nothing to say"
    );
    for strategy in strategies {
        let produced = strategy["produced"].as_u64().unwrap_or(0) as usize;
        let matched = strategy["matched"].as_u64().unwrap_or(0) as usize;
        assert!(
            matched <= produced,
            "a strategy cannot match more than it produced: {strategy}"
        );
        assert!(
            matched <= 1,
            "there is one candidate, so no strategy can match two: {strategy}"
        );
    }
    // At least one strategy produced nothing at all — a reader with no history
    // gives co-occurrence nothing to work from — and it is still listed.
    assert!(
        strategies
            .iter()
            .any(|s| s["produced"].as_u64().unwrap_or(1) == 0),
        "a strategy that produced nothing must still be reported: {body}"
    );
}

#[tokio::test]
async fn engine_legacy_is_the_feed_blend_restricted_to_the_filter() {
    let harness = Harness::new("legacy").await;
    let mut reader = harness.reader("m6301legacy").await;
    let account = account_of(&harness.db, "m6301legacy").await;
    seed_catalogue(&harness.db, &account).await;

    let (status, body) = reader
        .get("/api/v1/rec/run?fandom=naruto&engine=legacy")
        .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["engine"], "legacy");
    // Whatever the feed's own engines return, the answer cannot contain a work
    // the filter excluded — that is what "restricted to the filter" means.
    for key in item_keys(&body) {
        assert_eq!(key, "naruto-fic");
    }
    assert!(
        body.get("strategies").is_none(),
        "legacy is the feed's blend, not the registry: no strategy table"
    );
}

#[tokio::test]
async fn an_engine_that_is_not_enabled_is_refused_by_name() {
    let harness = Harness::new("engine").await;
    let mut reader = harness.reader("m6301engine").await;

    let (status, body) = reader
        .get("/api/v1/rec/run?engine=not_a_strategy")
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body: {body}");
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("not_a_strategy"),
        "the refusal names what was asked for: {message}"
    );
    assert!(
        message.contains("enabled strategies"),
        "and what is available instead: {message}"
    );
}

#[tokio::test]
async fn a_bad_window_is_the_same_error_on_both_routes() {
    // One parser, two routes: a window that means nothing must be refused the
    // same way whether it came with `engine=` or without.
    let harness = Harness::new("window").await;
    let mut reader = harness.reader("m6301window").await;

    let (catalogue, _) = reader.get("/api/v1/catalogue?updated=nonsense").await;
    let (rec, _) = reader
        .get("/api/v1/rec/run?engine=catalogue&updated=nonsense")
        .await;
    assert_eq!(catalogue, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(rec, catalogue, "the same grammar refuses the same text");
}
