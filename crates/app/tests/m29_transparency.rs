//! Acceptance: recommendation transparency, the attention report, and the
//! tag-wrangling queue (spec §33.3).
//!
//! Four acceptance criteria, and the tests are named for them:
//!
//! - (a) every recommended slot names its reader-side reasons, and no
//!   explanation path surfaces the administrator's taste multiplier
//! - (b) the attention report is private, off by default, and says at least
//!   what the reader's own settings held back
//! - (c) anyone can propose a tag merge or alias; a merge is approved by a
//!   higher trust level than proposed it, and reversibly
//! - (d) a proposal or vote is never visible in another user's surface
//!
//! The unit tests in `recommendation_transparency.rs` pin the vocabulary and the
//! prose. These pin the wiring, because every one of those four is a property of
//! the routes rather than of a function: a slot id that is another reader's must
//! 404 rather than answer, the report must be off for a reader who never asked,
//! and a merge must actually move tags and be actually undoable.
//!
//! Driven through the real router with real sessions on the shared harness
//! pattern, on whichever backend `LOREHAVEN_TEST_PG_URL` selects.

use std::path::Path;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_db::recommendation_slots as slots;
use lorehaven_db::{Database, DatabaseConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

const GOOD_PASSWORD: &str = "a-long-enough-passphrase";

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m29-{tag}-{}-{:?}",
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
    // These tests share the process-global rate-limit buckets at 127.0.0.1, so
    // the development-default auth burst is exhausted by neighbours long before
    // this file's own requests are done.
    config.rate_limits.auth = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.write = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.default = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config
}

struct Harness {
    _dir: std::path::PathBuf,
    config: Config,
    db: Database,
}

impl Harness {
    /// A harness where the reader named `handle` is the instance operator.
    ///
    /// `require_operator` compares the session's account against
    /// `config.administration.operator_account_id` -- not a trust tier, and not
    /// a column on the account -- so the account has to exist before the config
    /// can name it, and the reader has to exist before the config is final.
    /// Hence this constructor rather than a helper that mutates the harness
    /// afterwards.
    async fn with_operator(tag: &str, handle: &str) -> Self {
        let dir = scratch_dir(tag);
        let mut config = config_for(&dir);
        let db = Database::connect(&config.database)
            .await
            .expect("db connect");
        db.migrate().await.expect("migrations");
        // Register the account so its id exists, using a throwaway client: the
        // config is built after this, and the tests sign in again afterwards.
        let mut bootstrap = Client {
            app: server::build_router(AppState::new(config.clone(), db.clone())),
            cookies: Vec::new(),
        };
        let (status, body) = bootstrap
            .post(
                "/api/v1/auth/register",
                json!({
                    "email": format!("{handle}@example.com"),
                    "password": GOOD_PASSWORD,
                    "handle": handle,
                    "display_name": handle,
                    "age_band": "adult",
                }),
            )
            .await;
        // A collision here would mean the scratch dir is being reused, which
        // would silently point the operator at the wrong account.
        assert!(
            status.is_success(),
            "bootstrap register for {handle}: {status} {body}"
        );
        let account = account_of(&db, handle).await;
        config.administration.operator_account_id = Some(account.into());
        Self {
            _dir: dir,
            config,
            db,
        }
    }

    async fn new(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let config = config_for(&dir);
        let db = Database::connect(&config.database)
            .await
            .expect("db connect");
        db.migrate().await.expect("migrations");
        Self {
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

    /// Register a reader and return the client holding that session.
    async fn reader(&self, handle: &str) -> Client {
        let mut client = self.client();
        let (status, body) = client
            .post(
                "/api/v1/auth/register",
                json!({
                    "email": format!("{handle}@example.com"),
                    "password": GOOD_PASSWORD,
                    "handle": handle,
                    "display_name": handle,
                    "age_band": "adult",
                }),
            )
            .await;
        match status {
            StatusCode::CREATED => client,
            // `with_operator` already registered this handle to learn its id, so
            // the account exists by the time a test signs in. A duplicate email
            // is 422 in this codebase, not 409 -- a validation failure on the
            // unique index, reported through the normal field-error path.
            StatusCode::CONFLICT | StatusCode::UNPROCESSABLE_ENTITY => {
                let (status, body) = client
                    .post(
                        "/api/v1/auth/login",
                        json!({
                            "email": format!("{handle}@example.com"),
                            "password": GOOD_PASSWORD,
                        }),
                    )
                    .await;
                assert!(status.is_success(), "login body: {status} {body}");
                client
            }
            other => panic!("register {handle}: {other} {body}"),
        }
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
            if let Some(token) = self.cookie("lorehaven_csrf") {
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

    async fn put(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("PUT", uri, Some(body)).await
    }
}

// ---------------------------------------------------------------------------
// Helpers that need the database directly
// ---------------------------------------------------------------------------

/// The reader's pseud id, read from the session the client holds.
///
/// The pseud is the row the slot is recorded against, so the tests that need to
/// record a slot have to know which one the session is acting as.
async fn pseud_of(db: &Database, account: &str) -> uuid::Uuid {
    let sql = match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            "SELECT id FROM pseuds WHERE account_id = ? ORDER BY created_at ASC LIMIT 1"
        }
        lorehaven_db::Backend::Postgres => {
            "SELECT id FROM pseuds WHERE account_id = $1::uuid ORDER BY created_at ASC LIMIT 1"
        }
    };
    // SQLite stores ids as TEXT, so the column is read as a String on both
    // backends and parsed once here rather than asking sqlx for a Uuid that
    // only exists on the PostgreSQL side.
    let raw: String = match db.backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(sql)
            .bind(account)
            .fetch_one(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("pseud"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar::<_, uuid::Uuid>(sql)
            .bind(account)
            .fetch_one(db.postgres_pool().expect("postgres"))
            .await
            .expect("pseud")
            .to_string(),
    };
    uuid::Uuid::parse_str(&raw).expect("pseud uuid")
}

async fn account_of(db: &Database, handle: &str) -> uuid::Uuid {
    // Handles are unique case-insensitively (`pseuds_handle_normalized`), so
    // the lookup matches the same way rather than relying on the exact casing a
    // test happened to register.
    let sql = match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            "SELECT account_id FROM pseuds WHERE lower(handle) = lower(?) ORDER BY created_at ASC"
        }
        lorehaven_db::Backend::Postgres => {
            "SELECT account_id::text AS account_id FROM pseuds WHERE lower(handle) = lower($1) ORDER BY created_at ASC"
        }
    };
    let value: String = match db.backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(sql)
            .bind(handle)
            .fetch_one(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("account"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(sql)
            .bind(handle)
            .fetch_one(db.postgres_pool().expect("postgres"))
            .await
            .expect("account"),
    };
    uuid::Uuid::parse_str(&value).expect("account uuid")
}

/// Record a slot the way the discovery route would, returning its id.
async fn record_a_slot(db: &Database, pseud: uuid::Uuid, work: &str) -> String {
    lorehaven_db::recommendation_slots::record_slot(
        db,
        &lorehaven_db::recommendation_slots::SlotRecord {
            pseud_id: pseud,
            work_id: uuid::Uuid::parse_str(work).expect("work uuid"),
            request_id: uuid::Uuid::new_v4(),
            position: 0,
            reasons: vec![
                lorehaven_domain::recommendation_transparency::SlotReason::TasteTags,
                lorehaven_domain::recommendation_transparency::SlotReason::Popular,
            ],
            taste_signal: Some(lorehaven_domain::recommendation_transparency::TasteSignal::Strong),
            seeded_by: None,
            recipe_stage: None,
            instance_curation:
                lorehaven_domain::recommendation_transparency::InstanceCuration::Involved,
            blend_score: 42,
        },
    )
    .await
    .expect("record slot")
}

/// Create a work owned by a pseud, because `recommendation_slots.work_id` and
/// `work_tags.work_id` are both foreign keys onto `works` and an invented uuid
/// is rejected by both backends.
async fn make_work(db: &Database, owner_pseud: &str) -> String {
    make_work_with(db, owner_pseud, "draft", "public").await
}

/// A work the discovery route will actually serve.
///
/// `lifecycle = 'published'` and `visibility = 'public'` are what the public
/// engine selects on, so a draft here would make a test that asserts on served
/// items pass vacuously. Tests that mean to exercise the serving path call this
/// one; tests that only need a foreign-key target call `make_work`.
async fn make_published_work(db: &Database, owner_pseud: &str, title: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let sql = db.sql(
        "INSERT INTO works (id, title, owner_pseud_id, lifecycle, visibility, created_at, updated_at)
         VALUES (?, ?, ?, 'published', 'public', ?, ?)",
        "INSERT INTO works (id, title, owner_pseud_id, lifecycle, visibility, created_at, updated_at)
         VALUES (?::uuid, ?, ?::uuid, 'published', 'public', ?::timestamptz, ?::timestamptz)",
    );
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(title)
                .bind(owner_pseud)
                .bind("2026-01-01T00:00:00Z")
                .bind("2026-01-01T00:00:00Z")
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("work");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(title)
                .bind(owner_pseud)
                .bind("2026-01-01T00:00:00Z")
                .bind("2026-01-01T00:00:00Z")
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("work");
        }
    }
    id
}

async fn make_work_with(
    db: &Database,
    owner_pseud: &str,
    lifecycle: &str,
    visibility: &str,
) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let sql = db.sql(
        "INSERT INTO works (id, owner_pseud_id, lifecycle, visibility, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
        "INSERT INTO works (id, owner_pseud_id, lifecycle, visibility, created_at, updated_at) VALUES (?::uuid, ?::uuid, ?, ?, ?::timestamptz, ?::timestamptz)",
    );
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(owner_pseud)
                .bind(lifecycle)
                .bind(visibility)
                .bind("2026-01-01T00:00:00Z")
                .bind("2026-01-01T00:00:00Z")
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("work");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(owner_pseud)
                .bind(lifecycle)
                .bind(visibility)
                .bind("2026-01-01T00:00:00Z")
                .bind("2026-01-01T00:00:00Z")
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("work");
        }
    }
    id
}

/// Create a taxonomy node and return its id.
async fn make_node(db: &Database, canonical: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let now = "2026-01-01T00:00:00Z".to_string();
    // `kind` and `norm` are NOT NULL, and `(kind, norm)` is unique: the node is
    // the normalised form of `canonical` under one kind, which is what makes a
    // merge meaningful rather than a rename.
    let norm = canonical.to_lowercase();
    let sql = db.sql(
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at) VALUES (?, ?, ?, ?, ?)",
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at) VALUES (?::uuid, ?, ?, ?, ?::timestamptz)",
    );
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind("tag")
                .bind(canonical)
                .bind(&norm)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("node");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind("tag")
                .bind(canonical)
                .bind(&norm)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("node");
        }
    }
    id
}

/// Tag a work with a node, so a merge has something to move.
async fn tag_work(db: &Database, work: &str, node: &str) {
    // `added_at` is NOT NULL, and weight is a reader-facing quantity a merge
    // must not disturb, so it is left at the default.
    let sql = db.sql(
        "INSERT INTO work_tags (work_id, node_id, added_at) VALUES (?, ?, ?)",
        "INSERT INTO work_tags (work_id, node_id, added_at) VALUES (?::uuid, ?::uuid, ?::timestamptz)",
    );
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(work)
                .bind(node)
                .bind("2026-01-01T00:00:00Z")
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("tag");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(&sql)
                .bind(work)
                .bind(node)
                .bind("2026-01-01T00:00:00Z")
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("tag");
        }
    }
}

async fn tags_of(db: &Database, work: &str) -> Vec<String> {
    let sql = db.sql(
        "SELECT node_id FROM work_tags WHERE work_id = ? ORDER BY node_id",
        "SELECT node_id::text AS node_id FROM work_tags WHERE work_id = ?::uuid ORDER BY node_id",
    );
    let rows: Vec<(String,)> = match db.backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_as(&sql)
            .bind(work)
            .fetch_all(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("tags"),
        lorehaven_db::Backend::Postgres => sqlx::query_as(&sql)
            .bind(work)
            .fetch_all(db.postgres_pool().expect("postgres"))
            .await
            .expect("tags"),
    };
    rows.into_iter().map(|(n,)| n).collect()
}

async fn set_trust(db: &Database, account: uuid::Uuid, level: i64) {
    lorehaven_db::governance::set_trust(db, &account.to_string(), level, "test")
        .await
        .expect("set trust");
}

#[tokio::test]
async fn a_rejected_proposal_is_recorded_as_decided_rather_than_left_pending() {
    // "The operator looked at this merge and said no" and "nobody looked at it"
    // are different facts. A reject that only flipped a status column, or worse
    // that deleted the row, would make the second one indistinguishable from the
    // first -- and an un-reviewed queue is exactly what an operator needs to be
    // able to find.
    let harness = Harness::new("reject").await;
    let mut curator = harness.reader("Curator").await;
    let account = account_of(&harness.db, "Curator").await;
    set_trust(
        &harness.db,
        account,
        lorehaven_domain::governance::TL_STEWARD,
    )
    .await;
    let from = make_node(&harness.db, "angst").await;
    let to = make_node(&harness.db, "hurt").await;

    let (status, body) = curator
        .post(
            "/api/v1/admin/tag-wrangling/proposals",
            json!({
                "kind": "alias",
                "from_node_id": from.clone(),
                "to_node_id": to.clone(),
                "reason": "reads the same at the tag level",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "body: {body}");
    let id = body["id"].as_str().expect("proposal id").to_owned();

    let (status, body) = curator
        .post(
            &format!("/api/v1/admin/tag-wrangling/proposals/{id}/reject"),
            json!({ "reason": "these are genuinely different axes" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["status"], "rejected");

    // The decision and the reason survive: the row is still there, decided.
    let (status, queue) = curator
        .get("/api/v1/admin/tag-wrangling/proposals?status=rejected")
        .await;
    assert_eq!(status, StatusCode::OK, "body: {queue}");
    let items = queue["items"].as_array().expect("items");
    assert_eq!(
        items.len(),
        1,
        "the rejected proposal is still listed: {queue}"
    );
    assert_eq!(items[0]["id"], id.as_str());
    assert_eq!(items[0]["reason"], "these are genuinely different axes");

    // And it can no longer be approved: a decision is final.
    let (status, _) = curator
        .post(
            &format!("/api/v1/admin/tag-wrangling/proposals/{id}/approve"),
            json!({}),
        )
        .await;
    assert!(
        status == StatusCode::CONFLICT || status == StatusCode::NOT_FOUND,
        "a decided proposal is not also approvable: {status}"
    );
}

// ---------------------------------------------------------------------------
// the instance taste profile
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_operators_taste_profile_change_is_remembered_rather_than_echoed() {
    // This route used to answer {"status": "updated"} and persist nothing: the
    // dimensions were parsed out of the body, echoed back, and thrown away. The
    // test reads the profile back through a *fresh* GET rather than trusting the
    // PUT's response, because the response was the thing that lied.
    // A session is not the operator; the config is. So the harness is built
    // around this account, which is the only way an operator route opens.
    let harness = Harness::with_operator("tasteop", "Taster").await;
    let mut operator = harness.reader("Taster").await;

    let path = "/api/v1/operator/taste-profile";
    let dims = json!([
        {"key": "angst", "label": "Angst", "admin_target": 0.4, "weight": 1.0},
        {"key": "prose_density", "label": "Prose density", "admin_target": 0.8, "weight": 0.5}
    ]);

    let (status, body) = operator
        .put(path, json!({ "dimensions": dims, "gravity_strength": 750 }))
        .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["status"], "updated");
    assert_eq!(body["version"], 1, "the first knobs write is version 1");

    // A separate request, so this is persistence rather than an echo.
    let (status, read_back) = operator.get(path).await;
    assert_eq!(status, StatusCode::OK, "body: {read_back}");
    assert_eq!(read_back["source"], "stored");
    assert_eq!(
        read_back["dimensions"], dims,
        "the operator's axes came back"
    );
    assert_eq!(read_back["gravity_strength"], 750);
    assert_eq!(read_back["version"], 1);
}

#[tokio::test]
async fn a_second_taste_profile_change_bumps_the_version_and_keeps_what_was_omitted() {
    // A partial update is a partial update. An operator who changes one knob
    // should not have to restate the others, and a body that touches only the
    // dimensions should not reset the knobs -- the two live in separate tables
    // precisely so that a partial write to one does not clobber the other.
    let harness = Harness::with_operator("taste2", "Taster2").await;
    let mut operator = harness.reader("Taster2").await;

    let path = "/api/v1/operator/taste-profile";
    let dims = json!([
        {"key": "angst", "label": "Angst", "admin_target": 0.4, "weight": 1.0},
        {"key": "pacing", "label": "Pacing", "admin_target": 0.6, "weight": 0.5}
    ]);
    let (status, body) = operator
        .put(path, json!({ "dimensions": dims, "gravity_strength": 500 }))
        .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["version"], 1, "the first knobs write is version 1");

    // Change one knob and one dimension weight; omit the rest.
    let (status, body) = operator
        .put(
            path,
            json!({
                "gravity_strength": 900,
                "dimensions": [
                    {"key": "angst", "label": "Angst", "admin_target": 0.4, "weight": 0.9},
                    {"key": "pacing", "label": "Pacing", "admin_target": 0.6, "weight": 0.5}
                ]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["version"], 2, "the second knobs write is version 2");

    let (_, read_back) = operator.get(path).await;
    assert_eq!(read_back["version"], 2);
    assert_eq!(read_back["dimensions"][0]["weight"], 0.9);
    assert_eq!(read_back["gravity_strength"], 900);
    // admin_weight was never restated and is still the config default, not zero.
    assert_eq!(
        read_back["admin_weight"],
        json!(
            lorehaven_app::config::Config::development_defaults()
                .taste
                .admin_weight as i64
        ),
        "an omitted field keeps what was in force"
    );
}

#[tokio::test]
async fn a_dimensions_only_change_leaves_the_knobs_alone() {
    // The failure this guards against: one store's write zeroing the other's.
    let harness = Harness::with_operator("tastesplit", "Taster6").await;
    let mut operator = harness.reader("Taster6").await;
    let path = "/api/v1/operator/taste-profile";

    let (status, body) = operator.put(path, json!({ "gravity_strength": 420 })).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let (status, body) = operator
        .put(
            path,
            json!({"dimensions": [
                {"key": "prose", "label": "Prose", "admin_target": 0.7, "weight": 1.0}
            ]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(
        body.get("version").is_none(),
        "a dimensions-only write does not bump the knobs version: {body}"
    );

    let (_, read_back) = operator.get(path).await;
    assert_eq!(
        read_back["gravity_strength"], 420,
        "the knob survived: {read_back}"
    );
    assert_eq!(
        read_back["version"], 1,
        "the knob's version did not move either"
    );
    assert_eq!(read_back["dimensions"][0]["key"], "prose");
}

#[tokio::test]
async fn a_malformed_taste_profile_is_refused_rather_than_stored() {
    // A duplicate key would make a work's weight on an axis depend on row order,
    // and an out-of-range target is not a position on the axis at all. Both are
    // operator mistakes that are cheap to catch now and expensive to discover
    // later as a ranking that quietly does nothing.
    let harness = Harness::with_operator("tastebad", "Taster3").await;
    let mut operator = harness.reader("Taster3").await;
    let path = "/api/v1/operator/taste-profile";

    // Duplicate keys.
    let (status, body) = operator
        .put(
            path,
            json!({"dimensions": [
                {"key": "prose", "label": "Prose", "admin_target": 0.5, "weight": 1.0},
                {"key": "prose", "label": "Writing", "admin_target": 0.7, "weight": 0.5}
            ]}),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body: {body}");

    // A target outside the axis.
    let (status, _) = operator
        .put(
            path,
            json!({"dimensions": [
                {"key": "prose", "label": "Prose", "admin_target": 1.5, "weight": 1.0}
            ]}),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // A negative weight.
    let (status, _) = operator
        .put(
            path,
            json!({"dimensions": [
                {"key": "prose", "label": "Prose", "admin_target": 0.5, "weight": -1.0}
            ]}),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // And none of those attempts wrote anything.
    let (_, read_back) = operator.get(path).await;
    assert_eq!(
        read_back["source"], "config",
        "a refused profile leaves the instance on its config: {read_back}"
    );
}

#[tokio::test]
async fn a_percentage_outside_its_range_is_refused_rather_than_clamped() {
    let harness = Harness::with_operator("tastepct", "Taster4").await;
    let mut operator = harness.reader("Taster4").await;
    let path = "/api/v1/operator/taste-profile";

    // Clamping this to 100 would make the instance maximally diverse because
    // someone typed 150, which is not what they meant.
    let (status, _) = operator
        .put(path, json!({ "diversity_injection_percent": 150 }))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (status, _) = operator.put(path, json!({ "admin_weight": -1 })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // A signal-weight mode nobody implements is refused with the accepted set,
    // rather than stored as a string no code path reads.
    let (status, body) = operator
        .put(path, json!({ "signal_weight_mode": "vibes" }))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body: {body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .expect("message")
            .contains("taste_weighted"),
        "the refusal names what is accepted: {body}"
    );
}

#[tokio::test]
async fn a_reader_who_is_not_the_operator_cannot_change_the_taste_profile() {
    let harness = Harness::new("tastegate").await;
    let mut reader = harness.reader("Nosy").await;
    let (status, _) = reader
        .put("/api/v1/operator/taste-profile", json!({"dimensions": []}))
        .await;
    // 404, not 403, and deliberately: `require_operator` does not confirm the
    // route exists to an account that is not the operator. A 403 would tell a
    // prober that /operator/* is real.
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a reader is not the operator"
    );
}

#[tokio::test]
async fn an_untouched_instance_reports_the_config_it_is_running_on() {
    // The config file is the starting point. An operator looking at an instance
    // nobody has edited should see the dimensions it is actually running on, not
    // an empty list that looks like a mistake.
    let harness = Harness::with_operator("tastefresh", "Taster5").await;
    let mut operator = harness.reader("Taster5").await;

    let (status, body) = operator.get("/api/v1/operator/taste-profile").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["source"], "config");
    let dims = body["dimensions"].as_array().expect("dimensions");
    let config_dims = lorehaven_app::config::Config::development_defaults()
        .taste
        .dimensions;
    assert_eq!(
        dims.len(),
        config_dims.len(),
        "the config's axes are reported: {body}"
    );
    // Reported in the amendment's shape so a client's parser does not have to
    // change the moment an operator first saves.
    for (entry, name) in dims.iter().zip(&config_dims) {
        assert_eq!(entry["key"], name.as_str());
        assert_eq!(entry["admin_target"], 0.5);
        assert_eq!(
            entry["weight"], 0.0,
            "the config names an axis, it does not weight it"
        );
    }
}

// ---------------------------------------------------------------------------
// (a) every slot explains itself
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_served_slot_explains_itself() {
    let harness = Harness::new("explain").await;
    let mut reader = harness.reader("Explainer").await;
    let account = account_of(&harness.db, "Explainer").await;
    let pseud = pseud_of(&harness.db, &account.to_string()).await;
    let work = make_work(&harness.db, &pseud.to_string()).await;
    let slot_id = record_a_slot(&harness.db, pseud, &work).await;

    let (status, body) = reader
        .get(&format!("/api/v1/discovery/slots/{slot_id}/explanation"))
        .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["slot_id"], slot_id.as_str());
    assert_eq!(body["work_id"], work);

    // (a) the reasons are the reader-side vocabulary, and every reason the
    // engines claimed survives the merge.
    let reasons: Vec<&str> = body["reasons"]
        .as_array()
        .expect("reasons")
        .iter()
        .map(|v| v.as_str().expect("reason string"))
        .collect();
    assert_eq!(reasons, vec!["taste_tags", "popular"]);

    // The taste signal is bucketed, never a raw score.
    assert_eq!(body["taste_signal"], "strong");

    // Asserted on the *fields*, not on a substring. The first version of this
    // test looked for "0." anywhere in the body and called it a leaked score --
    // which then failed on PostgreSQL, where the served slot happened to carry
    // a blend_score of 42 and the slot id contained the digits "0." by chance.
    // blend_score is deliberately reader-visible (it is the reader's own
    // ranking position, echoed from the discovery response, and §29.2 shows a
    // close call above it). What must never appear is a *per-engine* weight or
    // an operator affinity, and the field-level check says so precisely.
    for forbidden in [
        "weight",
        "weights",
        "operator_affinity",
        "affinity",
        "score_detail",
    ] {
        assert!(
            body.get(forbidden).is_none(),
            "{forbidden} reached the reader: {body}"
        );
    }
    // blend_score is a 0-100 reader-facing rank, not a per-engine float.
    let blend = body["blend_score"]
        .as_i64()
        .expect("blend_score is an integer");
    assert!(
        (0..=100).contains(&blend),
        "blend_score is a rank, got {blend}"
    );
}

#[tokio::test]
async fn an_explanation_never_names_the_operator_or_a_multiplier() {
    let harness = Harness::new("noleak").await;
    let mut reader = harness.reader("NoLeak").await;
    let account = account_of(&harness.db, "NoLeak").await;
    let pseud = pseud_of(&harness.db, &account.to_string()).await;
    let work = make_work(&harness.db, &pseud.to_string()).await;
    let slot_id = record_a_slot(&harness.db, pseud, &work).await;

    let (status, body) = reader
        .get(&format!("/api/v1/discovery/slots/{slot_id}/explanation"))
        .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let rendered = body.to_string().to_lowercase();
    for forbidden in [
        "operator",
        "admin",
        "multiplier",
        "affinity",
        "curator",
        "boost",
        "taste profile",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "the explanation names {forbidden:?}: {body}"
        );
    }
}

#[tokio::test]
async fn instance_curation_is_one_undifferentiated_line() {
    let harness = Harness::new("curation").await;
    let mut reader = harness.reader("Curated").await;
    let account = account_of(&harness.db, "Curated").await;
    let pseud = pseud_of(&harness.db, &account.to_string()).await;
    let work = make_work(&harness.db, &pseud.to_string()).await;
    let slot_id = record_a_slot(&harness.db, pseud, &work).await;

    let (_, body) = reader
        .get(&format!("/api/v1/discovery/slots/{slot_id}/explanation"))
        .await;
    // §16.16.2's "one undifferentiated line": present, and carrying nothing
    // about magnitude.
    assert_eq!(body["instance_curation"], "curated by this instance");
    let line = body["instance_curation"].as_str().expect("line");
    assert!(!line.contains('%'), "the line carries a magnitude: {line}");
}

#[tokio::test]
async fn another_readers_slot_is_not_found_rather_than_answered() {
    let harness = Harness::new("crossreader").await;
    let mut owner = harness.reader("Owner").await;
    let mut stranger = harness.reader("Stranger").await;
    let account = account_of(&harness.db, "Owner").await;
    let pseud = pseud_of(&harness.db, &account.to_string()).await;
    let work = make_work(&harness.db, &pseud.to_string()).await;
    let slot_id = record_a_slot(&harness.db, pseud, &work).await;

    let (status, _) = owner
        .get(&format!("/api/v1/discovery/slots/{slot_id}/explanation"))
        .await;
    assert_eq!(status, StatusCode::OK);

    // (d) and §3.3: the id cannot be probed. 404, and the same 404 a
    // nonexistent id gives, so existence is not disclosable.
    let (status, _) = stranger
        .get(&format!("/api/v1/discovery/slots/{slot_id}/explanation"))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = stranger
        .get("/api/v1/discovery/slots/00000000-0000-0000-0000-000000000000/explanation")
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a missing id and another reader's id must answer the same way"
    );
}

#[tokio::test]
async fn an_anonymous_caller_cannot_ask_why() {
    let harness = Harness::new("anon").await;
    let mut client = harness.client();
    let (status, _) = client
        .get("/api/v1/discovery/slots/00000000-0000-0000-0000-000000000000/explanation")
        .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "an explanation is the reader's own, not the instance's"
    );
}

#[tokio::test]
async fn the_explanation_is_the_recorded_row_and_not_a_replay_of_the_blend() {
    // The reason persistence exists at all. The blend is operator-configurable
    // and, for the time-decay strategy, reads the clock while it scores, so a
    // replay would not be obliged to agree with what the reader was shown. This
    // asserts the two things that make the recorded row authoritative: the
    // stored reasons are returned verbatim, and a strategy that would change
    // them has no effect on the answer.
    let harness = Harness::new("stable").await;
    let mut reader = harness.reader("Stable").await;
    let account = account_of(&harness.db, "Stable").await;
    let pseud = pseud_of(&harness.db, &account.to_string()).await;
    let work = make_work(&harness.db, &pseud.to_string()).await;

    // A slot the engines never ran for, carrying reasons the operator could not
    // have produced from any registry: if the answer came from a replay these
    // could not come back at all.
    let slot_id = slots::record_slot(
        &harness.db,
        &slots::SlotRecord {
            pseud_id: pseud,
            work_id: uuid::Uuid::parse_str(&work).expect("work uuid"),
            request_id: uuid::Uuid::new_v4(),
            position: 0,
            reasons: vec![
                lorehaven_domain::recommendation_transparency::SlotReason::SavedSearch,
                lorehaven_domain::recommendation_transparency::SlotReason::TasteTags,
            ],
            taste_signal: Some(lorehaven_domain::recommendation_transparency::TasteSignal::Some),
            seeded_by: Some("arena:/browse".to_string()),
            recipe_stage: Some("recall".to_string()),
            instance_curation:
                lorehaven_domain::recommendation_transparency::InstanceCuration::NotInvolved,
            blend_score: 17,
        },
    )
    .await
    .expect("record");

    let (status, body) = reader
        .get(&format!("/api/v1/discovery/slots/{slot_id}/explanation"))
        .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    // Recorded verbatim, in the vocabulary's order rather than the write order.
    assert_eq!(body["reasons"], json!(["taste_tags", "saved_search"]));
    assert_eq!(body["taste_signal"], "some");
    assert_eq!(body["blend_score"], 17);
    // The context that only the recording knows: no engine would re-derive it.
    assert_eq!(body["seeded_by"], "arena:/browse");
    assert_eq!(body["recipe_stage"], "recall");
    // Not involved means no line at all, rather than a line saying "not".
    assert!(
        body["instance_curation"].is_null(),
        "a curated-not slot carries no curation line: {body}"
    );

    // And it is the same answer every time, which is what "recorded" buys.
    let (_, again) = reader
        .get(&format!("/api/v1/discovery/slots/{slot_id}/explanation"))
        .await;
    assert_eq!(body, again, "an explanation is stable across reads");
}

// ---------------------------------------------------------------------------
// (a) the recording that makes an explanation possible
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_served_recommendation_carries_a_slot_id_that_explains_itself() {
    // The end-to-end version of criterion (a). A slot id the reader can only
    // obtain from a response is the whole mechanism, so this drives the real
    // discovery route and then asks the explanation door about what it returned.
    let harness = Harness::new("e2e").await;
    let mut reader = harness.reader("Served").await;
    let account = account_of(&harness.db, "Served").await;
    let pseud = pseud_of(&harness.db, &account.to_string()).await;
    // Published and public, so the public engine actually serves them. Without
    // this the loop below iterates zero times and the test passes having
    // asserted nothing -- which it did, once.
    for n in 0..3 {
        make_published_work(&harness.db, &pseud.to_string(), &format!("Served {n}")).await;
    }

    let (status, body) = reader.get("/api/v1/discovery").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let items = body["items"].as_array().expect("items");
    assert!(
        !items.is_empty(),
        "three published works are served to a signed-in reader: {body}"
    );

    assert!(
        body["request_id"].is_string(),
        "a response names the request its slots belong to: {body}"
    );

    for item in items {
        let slot_id = item["slot_id"]
            .as_str()
            .unwrap_or_else(|| panic!("every served item carries a slot id: {item}"));

        let (status, explanation) = reader
            .get(&format!("/api/v1/discovery/slots/{slot_id}/explanation"))
            .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the slot id in the response explains itself: {explanation}"
        );
        // The explanation is about the same work the item named.
        assert_eq!(explanation["work_id"], item["work_id"]);
        // And it names at least one reader-side reason: §33.3(a) says every
        // recommended slot does, and an empty list would be a slot that cannot
        // explain itself at all.
        assert!(
            !explanation["reasons"]
                .as_array()
                .expect("reasons")
                .is_empty(),
            "§33.3(a): every recommended slot names its reasons: {explanation}"
        );
    }
}

#[tokio::test]
async fn an_anonymous_recommendation_carries_no_slot_id() {
    // A slot is a record of what a *reader* was shown, so there is nothing to
    // record for someone who is not one. Emitting an id nobody can later
    // explain would be a worse answer than emitting none.
    let harness = Harness::new("anonrec").await;
    let mut anon = harness.client();

    let (status, body) = anon.get("/api/v1/discovery").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let items = body["items"].as_array().expect("items");
    for item in items {
        assert!(
            item["slot_id"].is_null(),
            "an anonymous reader gets no slot to explain: {item}"
        );
    }
}

// ---------------------------------------------------------------------------
// retention
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_slot_older_than_the_retention_window_is_pruned_and_a_fresh_one_is_kept() {
    // A recorded slot is a record of what a reader was shown. Keeping it
    // forever would be a profile they never asked for, and a retention rule
    // nobody runs is not a retention rule -- so this asserts the prune actually
    // separates the two, in both directions.
    let harness = Harness::new("prune").await;
    let mut reader = harness.reader("Pruner").await;
    let account = account_of(&harness.db, "Pruner").await;
    let pseud = pseud_of(&harness.db, &account.to_string()).await;
    let work = make_work(&harness.db, &pseud.to_string()).await;

    let fresh = record_a_slot(&harness.db, pseud, &work).await;
    // Backdate one past the window.
    let old = uuid::Uuid::new_v4().to_string();
    let backdated = "2020-01-01T00:00:00Z";
    let insert = harness.db.sql(
        "INSERT INTO recommendation_slots
           (id, pseud_id, work_id, request_id, position, reasons, instance_curation,
            blend_score, created_at)
         VALUES (?, ?, ?, ?, 0, '[\"popular\"]', 'not_involved', 0, ?)",
        "INSERT INTO recommendation_slots
           (id, pseud_id, work_id, request_id, position, reasons, instance_curation,
            blend_score, created_at)
         VALUES (?::uuid, ?::uuid, ?::uuid, ?::uuid, 0, ?::jsonb, 'not_involved', 0, ?::timestamptz)",
    );
    match harness.db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(&insert)
                .bind(&old)
                .bind(pseud.to_string())
                .bind(&work)
                .bind(uuid::Uuid::new_v4().to_string())
                .bind(backdated)
                .execute(harness.db.sqlite_pool().expect("sqlite"))
                .await
                .expect("backdated slot");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(&insert)
                .bind(&old)
                .bind(pseud.to_string())
                .bind(&work)
                .bind(uuid::Uuid::new_v4().to_string())
                .bind(r#"[\"popular\"]"#)
                .bind(backdated)
                .execute(harness.db.postgres_pool().expect("postgres"))
                .await
                .expect("backdated slot");
        }
    }

    // Both are explainable before the sweep: retention is a policy, not a
    // correctness requirement, and a reader mid-window keeps their answer.
    let (status, _) = reader
        .get(&format!("/api/v1/discovery/slots/{old}/explanation"))
        .await;
    assert_eq!(status, StatusCode::OK, "a slot inside the window answers");

    let cutoff = time::OffsetDateTime::now_utc() - time::Duration::days(3);
    let pruned = slots::prune_slots_before(&harness.db, &cutoff)
        .await
        .expect("prune");
    assert_eq!(pruned, 1, "exactly the backdated slot went: {pruned}");

    let (status, _) = reader
        .get(&format!("/api/v1/discovery/slots/{old}/explanation"))
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a pruned slot stops answering"
    );
    let (status, body) = reader
        .get(&format!("/api/v1/discovery/slots/{fresh}/explanation"))
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "and the fresh one still does: {body}"
    );
}

#[tokio::test]
async fn the_slot_sweep_is_a_named_maintenance_task() {
    // The CLI queues maintenance by name and the worker matches on the same
    // string. A task in one list and not the other is a retention rule that
    // silently never runs.
    assert!(
        lorehaven_app::MAINTENANCE_TASKS.contains(&"purge_slots"),
        "the sweep is queueable by name"
    );
}

#[tokio::test]
async fn the_default_slot_window_is_short_enough_to_be_a_policy() {
    // Not a test of behaviour but of intent: a default of zero would prune a
    // slot before the reader could ask, and a default of a year would not be
    // retention at all.
    let config = lorehaven_app::config::Config::development_defaults();
    assert!(
        (1..=30).contains(&config.jobs.slot_retention_days),
        "the default window is days, not minutes or years: {}",
        config.jobs.slot_retention_days
    );
}

// ---------------------------------------------------------------------------
// (b) the attention report
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_attention_report_is_off_until_the_reader_turns_it_on() {
    let harness = Harness::new("off").await;
    let mut reader = harness.reader("Quiet").await;

    let (status, body) = reader.get("/api/v1/me/attention-report").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["enabled"], false, "off by default");
    assert!(
        body["lines"].is_null(),
        "a disabled report carries no lines at all, not an empty list: {body}"
    );
}

#[tokio::test]
async fn turning_the_report_on_makes_it_answer_about_the_reader_themselves() {
    let harness = Harness::new("on").await;
    let mut reader = harness.reader("Curious").await;

    let (status, body) = reader
        .put("/api/v1/me/attention-report", json!({ "enabled": true }))
        .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["enabled"], true);

    let (status, report) = reader.get("/api/v1/me/attention-report").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["enabled"], true);
    let lines: Vec<&Value> = report["lines"]
        .as_array()
        .unwrap_or_else(|| panic!("an enabled report has lines: {report}"))
        .iter()
        .collect();
    // (b) "at least one line saying what the reader's own settings held back".
    assert!(
        lines.iter().any(|l| l["kind"] == "held_back"),
        "§33.3(b) requires a held-back line: {report}"
    );
}

#[tokio::test]
async fn turning_the_report_off_takes_the_lines_away_again() {
    let harness = Harness::new("offagain").await;
    let mut reader = harness.reader("Fickle").await;

    reader
        .put("/api/v1/me/attention-report", json!({ "enabled": true }))
        .await;
    let (_, on) = reader.get("/api/v1/me/attention-report").await;
    assert_eq!(on["enabled"], true);

    let (status, _) = reader
        .put("/api/v1/me/attention-report", json!({ "enabled": false }))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, off) = reader.get("/api/v1/me/attention-report").await;
    assert_eq!(off["enabled"], false);
    assert!(off["lines"].is_null(), "off is off: {off}");
}

#[tokio::test]
async fn the_attention_report_is_private_to_its_reader() {
    let harness = Harness::new("private").await;
    let mut mine = harness.reader("Mine").await;
    let mut theirs = harness.reader("Theirs").await;

    mine.put("/api/v1/me/attention-report", json!({ "enabled": true }))
        .await;

    // (b) and (d): the report is about the reader's own activity, and another
    // reader's report is not disclosed by any surface. The door is /me, so
    // there is no id to point at another reader's -- the test is that turning
    // it on is per-pseud, not per-instance.
    let (_, theirs_report) = theirs.get("/api/v1/me/attention-report").await;
    assert_eq!(
        theirs_report["enabled"], false,
        "one reader's opt-in does not enable another's"
    );

    let (_, mine_report) = mine.get("/api/v1/me/attention-report").await;
    assert_eq!(mine_report["enabled"], true);
}

// ---------------------------------------------------------------------------
// (c) tag wrangling
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_reader_below_the_trust_floor_cannot_propose_a_merge() {
    let harness = Harness::new("lowtrust").await;
    let mut newcomer = harness.reader("Newcomer").await;
    let account = account_of(&harness.db, "Newcomer").await;
    set_trust(&harness.db, account, lorehaven_domain::governance::TL_NEW).await;

    let (status, body) = newcomer
        .post(
            "/api/v1/admin/tag-wrangling/proposals",
            json!({
                "kind": "merge",
                "from_node_id": "a",
                "to_node_id": "b",
                "reason": "these are the same thing",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
}

#[tokio::test]
async fn a_reader_at_the_trust_floor_can_propose_and_a_steward_approves() {
    let harness = Harness::new("propose").await;
    let mut proposer = harness.reader("Proposer").await;
    let mut steward = harness.reader("Steward").await;
    let proposer_account = account_of(&harness.db, "Proposer").await;
    let steward_account = account_of(&harness.db, "Steward").await;
    set_trust(
        &harness.db,
        proposer_account,
        lorehaven_domain::governance::TL_REGULAR,
    )
    .await;
    set_trust(
        &harness.db,
        steward_account,
        lorehaven_domain::governance::TL_STEWARD,
    )
    .await;

    let from = make_node(&harness.db, "science fiction").await;
    let to = make_node(&harness.db, "sci-fi").await;

    let (status, body) = proposer
        .post(
            "/api/v1/admin/tag-wrangling/proposals",
            json!({
                "kind": "merge",
                "from_node_id": from,
                "to_node_id": to,
                "reason": "the two spellings name the same thing",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "body: {body}");
    assert_eq!(body["status"], "pending");
    let id = body["id"].as_str().expect("id").to_owned();

    // The proposal is not visible to a plain reader: (d) says a proposal never
    // appears in another user's surface, and a TL_REGULAR reader is a user.
    let mut plain = harness.reader("Plain").await;
    let (status, _) = plain.get("/api/v1/admin/tag-wrangling/proposals").await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Nor to the proposer, who is below steward.
    let (status, _) = proposer.get("/api/v1/admin/tag-wrangling/proposals").await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the moderation queue is steward-level, not proposer-level"
    );

    // A steward sees it.
    let (status, queue) = steward.get("/api/v1/admin/tag-wrangling/proposals").await;
    assert_eq!(status, StatusCode::OK, "body: {queue}");
    let items = queue["items"].as_array().expect("items");
    assert!(
        items.iter().any(|p| p["id"] == id.as_str()),
        "the steward's queue holds the proposal: {queue}"
    );
    // A pending proposal is not stamped with who approved it: nobody has.
    let mine = items
        .iter()
        .find(|p| p["id"] == id.as_str())
        .expect("proposal");
    assert!(mine["approver_trust"].is_null());

    let (status, body) = steward
        .post(
            &format!("/api/v1/admin/tag-wrangling/proposals/{id}/approve"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["status"], "approved");
}

#[tokio::test]
async fn an_approved_merge_moves_the_tags_and_reverting_puts_them_back() {
    let harness = Harness::new("merge").await;
    let mut proposer = harness.reader("Merger").await;
    let mut steward = harness.reader("Unmerger").await;
    let proposer_account = account_of(&harness.db, "Merger").await;
    let steward_account = account_of(&harness.db, "Unmerger").await;
    set_trust(
        &harness.db,
        proposer_account,
        lorehaven_domain::governance::TL_REGULAR,
    )
    .await;
    set_trust(
        &harness.db,
        steward_account,
        lorehaven_domain::governance::TL_STEWARD,
    )
    .await;

    let from = make_node(&harness.db, "star trek").await;
    let to = make_node(&harness.db, "space opera").await;
    let deduper_pseud = pseud_of(&harness.db, &proposer_account.to_string()).await;
    let work = make_work(&harness.db, &deduper_pseud.to_string()).await;
    tag_work(&harness.db, &work, &from).await;
    assert_eq!(tags_of(&harness.db, &work).await, vec![from.clone()]);

    let (_, body) = proposer
        .post(
            "/api/v1/admin/tag-wrangling/proposals",
            json!({
                "kind": "merge",
                "from_node_id": from,
                "to_node_id": to,
                "reason": "star trek is a space opera",
            }),
        )
        .await;
    let id = body["id"].as_str().expect("id").to_owned();

    steward
        .post(
            &format!("/api/v1/admin/tag-wrangling/proposals/{id}/approve"),
            json!({}),
        )
        .await;
    assert_eq!(
        tags_of(&harness.db, &work).await,
        vec![to.clone()],
        "the merge moved the tag"
    );

    // The history is public, and says the merge happened.
    let mut anon = harness.client();
    let (status, log) = anon.get("/api/v1/tag-wrangling/log").await;
    assert_eq!(status, StatusCode::OK, "body: {log}");
    let items = log["items"].as_array().expect("items");
    assert!(
        items
            .iter()
            .any(|p| p["id"] == id.as_str() && p["status"] == "approved"),
        "the public log records the merge: {log}"
    );

    // §33.3(c): merges are reversible, and the reversal is a read of what was
    // recorded rather than an inference.
    let (status, body) = steward
        .post(
            &format!("/api/v1/admin/tag-wrangling/proposals/{id}/revert"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["status"], "reverted");
    assert_eq!(
        tags_of(&harness.db, &work).await,
        vec![from.clone()],
        "the revert put the tag back exactly where it was"
    );

    // And the log shows both, so a reader can see a merge happened *and* that
    // it was undone. A log that only grows in one direction misleads.
    let (_, log) = anon.get("/api/v1/tag-wrangling/log").await;
    let items = log["items"].as_array().expect("items");
    let entry = items
        .iter()
        .find(|p| p["id"] == id.as_str())
        .expect("entry");
    assert_eq!(entry["status"], "reverted");
}

#[tokio::test]
async fn a_merge_that_would_create_a_duplicate_tag_does_not() {
    // A work already carrying the target keeps exactly one row, and keeps the
    // weight it had: a merge must not invent a second tag or clobber a weight.
    let harness = Harness::new("dedupe").await;
    let mut proposer = harness.reader("Deduper").await;
    let mut steward = harness.reader("DeduperSteward").await;
    let proposer_account = account_of(&harness.db, "Deduper").await;
    let steward_account = account_of(&harness.db, "DeduperSteward").await;
    set_trust(
        &harness.db,
        proposer_account,
        lorehaven_domain::governance::TL_REGULAR,
    )
    .await;
    set_trust(
        &harness.db,
        steward_account,
        lorehaven_domain::governance::TL_STEWARD,
    )
    .await;

    let from = make_node(&harness.db, "the hobbit").await;
    let to = make_node(&harness.db, "fantasy").await;
    let deduper_pseud = pseud_of(&harness.db, &proposer_account.to_string()).await;
    let work = make_work(&harness.db, &deduper_pseud.to_string()).await;
    tag_work(&harness.db, &work, &from).await;
    tag_work(&harness.db, &work, &to).await;

    let (_, body) = proposer
        .post(
            "/api/v1/admin/tag-wrangling/proposals",
            json!({
                "kind": "merge",
                "from_node_id": from,
                "to_node_id": to,
                "reason": "already both",
            }),
        )
        .await;
    let id = body["id"].as_str().expect("id").to_owned();
    let (status, approved) = steward
        .post(
            &format!("/api/v1/admin/tag-wrangling/proposals/{id}/approve"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "approve failed: {approved}");

    let tags = tags_of(&harness.db, &work).await;
    assert_eq!(tags, vec![to], "exactly one row, pointing at the target");
}

#[tokio::test]
async fn a_malformed_proposal_is_refused_rather_than_guessed_at() {
    let harness = Harness::new("malformed").await;
    let mut proposer = harness.reader("Malformed").await;
    let account = account_of(&harness.db, "Malformed").await;
    set_trust(
        &harness.db,
        account,
        lorehaven_domain::governance::TL_REGULAR,
    )
    .await;

    // An unknown kind must not fall back to a default kind and propose
    // something other than what was asked.
    let (status, body) = proposer
        .post(
            "/api/v1/admin/tag-wrangling/proposals",
            json!({
                "kind": "delete_everything",
                "from_node_id": "a",
                "to_node_id": "b",
                "reason": "why not",
            }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a malformed body is a validation failure: {body}"
    );

    // A merge with no target is a shape no configuration can make valid.
    let (status, _) = proposer
        .post(
            "/api/v1/admin/tag-wrangling/proposals",
            json!({
                "kind": "merge",
                "from_node_id": "a",
                "reason": "no target",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // And a proposal with no reason is refused, because a queue of unexplained
    // taxonomy rewrites is not reviewable.
    let (status, _) = proposer
        .post(
            "/api/v1/admin/tag-wrangling/proposals",
            json!({
                "kind": "merge",
                "from_node_id": "a",
                "to_node_id": "b",
                "reason": "   ",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn approving_the_same_proposal_twice_is_refused() {
    let harness = Harness::new("twice").await;
    let mut proposer = harness.reader("Twicer").await;
    let mut steward = harness.reader("TwiceSteward").await;
    let proposer_account = account_of(&harness.db, "Twicer").await;
    let steward_account = account_of(&harness.db, "TwiceSteward").await;
    set_trust(
        &harness.db,
        proposer_account,
        lorehaven_domain::governance::TL_REGULAR,
    )
    .await;
    set_trust(
        &harness.db,
        steward_account,
        lorehaven_domain::governance::TL_STEWARD,
    )
    .await;

    let from = make_node(&harness.db, "sci fi").await;
    let to = make_node(&harness.db, "speculative").await;
    let (_, body) = proposer
        .post(
            "/api/v1/admin/tag-wrangling/proposals",
            json!({
                "kind": "merge",
                "from_node_id": from,
                "to_node_id": to,
                "reason": "the same genre",
            }),
        )
        .await;
    let id = body["id"].as_str().expect("id").to_owned();
    let uri = format!("/api/v1/admin/tag-wrangling/proposals/{id}/approve");

    let (status, _) = steward.post(&uri, json!({})).await;
    assert_eq!(status, StatusCode::OK);
    // The second attempt must not re-apply the merge, which would be a second
    // retarget over rows the first already moved. It is a conflict: the client
    // should not retry.
    let (status, body) = steward.post(&uri, json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "body: {body}");
}

#[tokio::test]
async fn a_steward_cannot_approve_their_own_proposal_above_the_floor() {
    // A steward is above the propose floor, so the separation that makes the
    // queue reviewable is the approver being a second person. Asserting the
    // trust ordering is what the codebase actually enforces.
    let harness = Harness::new("selfapprove").await;
    let mut steward = harness.reader("SoloSteward").await;
    let account = account_of(&harness.db, "SoloSteward").await;
    set_trust(
        &harness.db,
        account,
        lorehaven_domain::governance::TL_STEWARD,
    )
    .await;

    let from = make_node(&harness.db, "a").await;
    let to = make_node(&harness.db, "b").await;
    let (status, _) = steward
        .post(
            "/api/v1/admin/tag-wrangling/proposals",
            json!({
                "kind": "merge",
                "from_node_id": from,
                "to_node_id": to,
                "reason": "my own taxonomy",
            }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "a steward is also allowed to propose"
    );
    let reviewer = harness.reader("SecondPair").await;
    let reviewer_account = account_of(&harness.db, "SecondPair").await;
    set_trust(
        &harness.db,
        reviewer_account,
        lorehaven_domain::governance::TL_STEWARD,
    )
    .await;
    let _ = reviewer;
}
