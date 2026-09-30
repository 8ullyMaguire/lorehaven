//! Acceptance: a reader's own copy of an external body (spec §11.15b,
//! amendment §6.2–6.4; plan `m59-10-reader-body-request.md`).
//!
//! The gate's ORDER is the design, and one test exists only to pin it — see
//! `an_aggregate_instance_refuses_by_name_and_never_reaches_the_trust_gate`.
//! The privacy test is verified by injection; see
//! `a_reader_cannot_read_another_readers_copy`.

use std::path::Path;

use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_db::Database;
use serde_json::{json, Value};
use test_support::TestDb;
use tower::ServiceExt;

const GOOD_PASSWORD: &str = "a-long-enough-passphrase";

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-body-request-{tag}-{}-{:?}",
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
    // Honoured, so this file runs on whichever backend the selector names.
    config.database = match std::env::var("LOREHAVEN_TEST_PG_URL") {
        Ok(url) => lorehaven_db::DatabaseConfig::new(url),
        Err(_) => lorehaven_db::DatabaseConfig::new(format!(
            "sqlite://{}/lorehaven.sqlite?mode=rwc",
            dir.display()
        )),
    };
    // The rate-limit buckets are process-global at 127.0.0.1, so the development
    // defaults are exhausted by neighbouring suites long before this file's own
    // requests finish.
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

struct Client {
    app: axum::Router,
    cookies: Vec<(String, String)>,
}

impl Client {
    fn cookie_header(&self) -> String {
        self.cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn capture(&mut self, response: &axum::response::Response) {
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

    async fn send(&mut self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(path);
        if !self.cookies.is_empty() {
            builder = builder.header(header::COOKIE, self.cookie_header());
        }
        let request = match body {
            Some(value) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(axum::body::Body::from(value.to_string()))
                .expect("request"),
            None => builder.body(axum::body::Body::empty()).expect("request"),
        };
        let response = self
            .app
            .clone()
            .oneshot(request)
            .await
            .expect("route response");
        let status = response.status();
        self.capture(&response);
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("response body");
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, value)
    }

    async fn get(&mut self, path: &str) -> (StatusCode, Value) {
        self.send("GET", path, None).await
    }

    async fn post(&mut self, path: &str, body: Value) -> (StatusCode, Value) {
        self.send("POST", path, Some(body)).await
    }
}

struct Harness {
    /// Held so the scratch database lives as long as the harness.
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

    /// A signed-in reader at `trust`.
    async fn reader_at(&self, handle: &str, trust: i64) -> Client {
        let mut client = self.client();
        // The register body's error text is only read on the login path below, so
        // the registration attempt's is discarded — a re-registration is expected
        // when two tests share a scratch directory.
        let (status, _body) = client
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
        if !status.is_success() {
            let (status, body) = client
                .post(
                    "/api/v1/auth/login",
                    json!({
                        "email": format!("{handle}@example.com"),
                        "password": GOOD_PASSWORD,
                    }),
                )
                .await;
            assert!(status.is_success(), "login {handle}: {status} {body}");
        }
        let account = account_of(&self.db, handle).await;
        lorehaven_db::governance::set_trust(&self.db, &account.to_string(), trust, "test")
            .await
            .expect("set trust");
        client
    }

    /// A reader above the default bar of 2.
    async fn reader(&self, handle: &str) -> Client {
        self.reader_at(handle, 3).await
    }

    /// A work imported from `source`, so the route's `library_items` lookup finds
    /// a source. Without this the POST refuses with `NO_IMPORT_RECORD`, which is
    /// itself tested — so every other test needs one of these.
    async fn imported_work(&self, source: &str, title: &str) -> String {
        let work_id = uuid::Uuid::new_v4().to_string();
        let now = lorehaven_db::identity::now_rfc3339();
        let sql = match self.db.backend() {
            lorehaven_db::Backend::Sqlite => {
                // `owner_pseud_id`, NOT `owner_account_id`: ownership belongs to
                // the pseud (ADR 0003 — "pseud switching does not change
                // ownership"), so the subquery reaches `pseuds`.
                "INSERT INTO works (id, owner_pseud_id, title, summary, language,
                                    rating, visibility, lifecycle, completion,
                                    created_at, updated_at)
                 VALUES (?1, (SELECT id FROM pseuds ORDER BY created_at ASC LIMIT 1),
                         ?2, 'a summary', 'en', 'general', 'public', 'active',
                         'in_progress', ?3, ?3)"
            }
            lorehaven_db::Backend::Postgres => {
                "INSERT INTO works (id, owner_pseud_id, title, summary, language,
                                    rating, visibility, lifecycle, completion,
                                    created_at, updated_at)
                 VALUES ($1::uuid, (SELECT id FROM pseuds ORDER BY created_at ASC LIMIT 1),
                         $2, 'a summary', 'en', 'general', 'public', 'active',
                         'in_progress', $3, $3)"
            }
        };
        let item_id = uuid::Uuid::new_v4().to_string();
        let item_sql = match self.db.backend() {
            lorehaven_db::Backend::Sqlite => {
                // `status` is a CHECK over (ongoing, complete, hiatus, cancelled,
                // unknown) — 'imported' is not one of them, and `account_id`,
                // `source_work_key` and `source_url` are NOT NULL with no
                // default. All four were wrong in the first draft.
                "INSERT INTO library_items (id, account_id, source_key, source_work_key,
                                            title, source_url, status, work_id,
                                            created_at, updated_at)
                 VALUES (?1, (SELECT id FROM accounts ORDER BY created_at ASC LIMIT 1),
                         ?2, 'src:ext-1', ?3, 'https://example.org/ext-1',
                         'complete', ?4, ?5, ?5)"
            }
            lorehaven_db::Backend::Postgres => {
                "INSERT INTO library_items (id, account_id, source_key, source_work_key,
                                            title, source_url, status, work_id,
                                            created_at, updated_at)
                 VALUES ($1::uuid, (SELECT id FROM accounts ORDER BY created_at ASC LIMIT 1),
                         $2, 'src:ext-1', $3, 'https://example.org/ext-1',
                         'complete', $4::uuid, $5, $5)"
            }
        };
        // The two arms are spelled out rather than funnelled through a helper:
        // `db.sql()` is exactly the thing that goes wrong when a `?1` ends up
        // on the Postgres arm, so these INSERTs keep their placeholders visible
        // next to the backend they belong to.
        match self.db.backend() {
            lorehaven_db::Backend::Sqlite => {
                sqlx::query(sql)
                    .bind(&work_id)
                    .bind(title)
                    .bind(&now)
                    .execute(self.db.sqlite_pool().expect("sqlite"))
                    .await
                    .expect("insert work");
                sqlx::query(item_sql)
                    .bind(&item_id)
                    .bind(source)
                    .bind(title)
                    .bind(&work_id)
                    .bind(&now)
                    .execute(self.db.sqlite_pool().expect("sqlite"))
                    .await
                    .expect("insert library item");
            }
            lorehaven_db::Backend::Postgres => {
                sqlx::query(sql)
                    .bind(&work_id)
                    .bind(title)
                    .bind(&now)
                    .execute(self.db.postgres_pool().expect("postgres"))
                    .await
                    .expect("insert work");
                sqlx::query(item_sql)
                    .bind(&item_id)
                    .bind(source)
                    .bind(title)
                    .bind(&work_id)
                    .bind(&now)
                    .execute(self.db.postgres_pool().expect("postgres"))
                    .await
                    .expect("insert library item");
            }
        }
        work_id
    }

    /// Set the INSTANCE's retention mode, then a source override.
    ///
    /// Instance first, because `write_source_override` refuses to widen: a source
    /// can only be `cache` when the instance already is. The reverse order would
    /// fail on every test here, and a failure that reads as "the feature is
    /// broken" is really about the order of two setup calls.
    ///
    /// `write_policy` and `write_source_override` both take a concrete `Uuid`,
    /// because `instance_retention_policy.updated_by` is
    /// `NOT NULL REFERENCES accounts(id)`. This harness has a real reader, so it
    /// passes theirs — no system account needed.
    ///
    /// The actor is NAMED, never assumed. The first draft hardcoded `"alice"`,
    /// which does not exist in a test whose only reader is `lowtrust` — the same
    /// fault the route holds itself to avoid under §6.4.4: name the row rather
    /// than guess at which row you meant.
    async fn set_source_mode_as(&self, actor_handle: &str, source: &str, mode: &str) {
        let mode = lorehaven_domain::retention::BodyMode::parse_stored(Some(mode))
            .unwrap_or_else(|| panic!("{mode} is a BodyMode spelling"));
        let actor = account_of(&self.db, actor_handle).await;
        lorehaven_db::retention::write_policy(&self.db, mode, actor)
            .await
            .expect("set the instance mode");
        if mode.stores_bodies() {
            lorehaven_db::retention::write_source_override(&self.db, source, mode, actor)
                .await
                .expect("set source override");
        }
    }

    /// The common case: the reader this test registered is the actor.
    async fn set_source_mode(&self, source: &str, mode: &str) {
        self.set_source_mode_as("alice", source, mode).await
    }

    async fn copies_for(&self, work_id: &str) -> i64 {
        lorehaven_db::reader_body_copies::count_copies(&self.db, work_id)
            .await
            .expect("count copies")
    }
}

async fn account_of(db: &Database, handle: &str) -> uuid::Uuid {
    let sql = match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            "SELECT account_id FROM pseuds WHERE lower(handle) = lower(?) ORDER BY created_at ASC"
        }
        lorehaven_db::Backend::Postgres => {
            "SELECT account_id::text AS account_id FROM pseuds
              WHERE lower(handle) = lower($1) ORDER BY created_at ASC"
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

fn err_code(body: &Value) -> &str {
    body.pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn err_message(body: &Value) -> &str {
    body.pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or("")
}

// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_reader_below_the_bar_is_refused_with_the_bar_stated_and_no_row_is_written() {
    // §6.4.1. Trust 1 against the default bar of 2.
    let harness = Harness::new("below-bar").await;
    let mut low = harness.reader_at("lowtrust", 1).await;
    let work = harness.imported_work("archive:example.org", "A Work").await;
    harness
        .set_source_mode_as("lowtrust", "archive:example.org", "cache")
        .await;

    let (status, body) = low
        .post(&format!("/api/v1/works/{work}/body-request"), json!({}))
        .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(err_code(&body), "TRUST_TOO_LOW", "{body}");

    // The second clause, which is the easy one to drop: a refusal that does not
    // say the bar is true and useless — a reader told what would be enough can
    // decide whether to wait, ask, or go elsewhere.
    let message = err_message(&body);
    assert!(
        message.contains("2"),
        "the refusal states the bar of 2: {message}"
    );
    assert!(
        message.contains('1'),
        "the refusal states the caller's own level of 1: {message}"
    );

    assert_eq!(
        harness.copies_for(&work).await,
        0,
        "a refused request writes no row: a row saying this reader asked and was \
         refused is a record of a reader wanting bytes this instance does not hold"
    );
}

#[tokio::test]
async fn an_aggregate_instance_refuses_by_name_and_never_reaches_the_trust_gate() {
    // §6.4.3, AND the order in the plan's step 4.1.
    //
    // A trust-0 reader on an aggregate source. Checking trust first would refuse
    // with TRUST_TOO_LOW — which is true, would pass every other test in this
    // file, and would tell a reader not entitled to it that the instance is
    // aggregate. So the assertion is specifically on the CODE, not merely on the
    // status: this fails on a reordering and on nothing else.
    let harness = Harness::new("aggregate").await;
    let mut nobody = harness.reader_at("nobody", 0).await;
    let work = harness
        .imported_work("private:example.org", "An Aggregate Work")
        .await;
    harness
        .set_source_mode_as("nobody", "private:example.org", "aggregate")
        .await;

    let (status, body) = nobody
        .post(&format!("/api/v1/works/{work}/body-request"), json!({}))
        .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(
        err_code(&body),
        "RETENTION_AGGREGATE",
        "the mode is refused BEFORE the trust bar, so a reader learns the mode \
         rather than only their own standing: {body}"
    );
    assert!(
        err_message(&body).contains("private:example.org"),
        "the refusal names the source it is about: {body}"
    );
    assert_eq!(
        harness.copies_for(&work).await,
        0,
        "a refusal writes no row"
    );
}

#[tokio::test]
async fn a_cache_request_creates_a_pending_copy_the_reader_can_read() {
    // §6.4.2. The copy exists, it is the reader's own, and its state is pending —
    // because the fetch is a job and `202` promises work that has not happened.
    let harness = Harness::new("cache-request").await;
    let mut alice = harness.reader("alice").await;
    let work = harness
        .imported_work("archive:example.org", "A Cacheable Work")
        .await;
    harness
        .set_source_mode("archive:example.org", "cache")
        .await;

    let (status, body) = alice
        .post(&format!("/api/v1/works/{work}/body-request"), json!({}))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["state"], "pending", "{body}");

    // The GET answers with the same story.
    let (status, seen) = alice
        .get(&format!("/api/v1/works/{work}/body-request"))
        .await;
    assert_eq!(status, StatusCode::OK, "{seen}");
    assert_eq!(seen["state"], "pending", "{seen}");
    assert_eq!(seen["work_id"], work.as_str(), "{seen}");

    // And the audit row §6.2 asks for exists, recording the trust the reader
    // held AT THE TIME rather than leaving it to be re-derived.
    let requests: i64 = {
        let sql = match harness.db.backend() {
            lorehaven_db::Backend::Sqlite => {
                "SELECT COUNT(*) FROM retention_body_requests WHERE work_id = ?1"
            }
            lorehaven_db::Backend::Postgres => {
                "SELECT COUNT(*) FROM retention_body_requests WHERE work_id = $1::uuid"
            }
        };
        match harness.db.backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query_scalar(sql)
                .bind(&work)
                .fetch_one(harness.db.sqlite_pool().expect("sqlite"))
                .await
                .expect("audit rows"),
            lorehaven_db::Backend::Postgres => sqlx::query_scalar(sql)
                .bind(&work)
                .fetch_one(harness.db.postgres_pool().expect("postgres"))
                .await
                .expect("audit rows"),
        }
    };
    assert_eq!(requests, 1, "the request is recorded in the audit surface");
}

#[tokio::test]
async fn a_reader_cannot_read_another_readers_copy() {
    // §6.2's "referenced by that reader's copy". Settle a copy for alice, then
    // have bob ask for his own and read the GET: he must get HIS state, not
    // hers, and the bytes must not appear in his response.
    //
    // VERIFIED BY INJECTION before this work was called done — the plan requires
    // it, and a privacy test that has never failed is not evidence.
    let harness = Harness::new("privacy").await;
    let mut alice = harness.reader("alice").await;
    let mut bob = harness.reader("bob").await;
    let work = harness
        .imported_work("archive:example.org", "A Private Copy")
        .await;
    harness
        .set_source_mode("archive:example.org", "cache")
        .await;

    let (status, _) = alice
        .post(&format!("/api/v1/works/{work}/body-request"), json!({}))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    // Settle alice's copy with bytes, as the job would.
    let alice_account = account_of(&harness.db, "alice").await;
    let copy = lorehaven_db::reader_body_copies::request_copy(
        &harness.db,
        &work,
        &alice_account,
        "archive:example.org",
        "chapters:0",
        3,
    )
    .await
    .expect("copy");
    assert!(lorehaven_db::reader_body_copies::settle_ready(
        &harness.db,
        &copy.id,
        "ALICE'S EXCLUSIVE TEXT",
        None,
    )
    .await
    .expect("settle"));

    // Alice can read her own.
    let (status, mine) = alice
        .get(&format!("/api/v1/works/{work}/body-request"))
        .await;
    assert_eq!(status, StatusCode::OK, "{mine}");
    assert_eq!(mine["plain_text"], "ALICE'S EXCLUSIVE TEXT", "{mine}");

    // Bob has never asked, so he gets 404 — and never her bytes.
    let (status, theirs) = bob.get(&format!("/api/v1/works/{work}/body-request")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{theirs}");
    assert!(
        !serde_json::to_string(&theirs)
            .unwrap_or_default()
            .contains("EXCLUSIVE"),
        "another reader's bytes must not appear in a response at all: {theirs}"
    );

    // And no response anywhere names whose copy it is.
    let rendered = serde_json::to_string(&mine).unwrap_or_default();
    for field in ["account_id", "reader_id", "requested_by", "owner"] {
        assert!(
            !rendered.contains(field),
            "the response must not expose {field}: {rendered}"
        );
    }
    assert!(
        !rendered.contains(&alice_account.to_string()),
        "the response must not carry the reader's account id: {rendered}"
    );
}

#[tokio::test]
async fn asking_twice_updates_the_copy_rather_than_adding_one() {
    // The unique index on (work_id, account_id). Counted as ROWS, not as
    // responses: a count of the response body would pass under two blobs.
    let harness = Harness::new("twice").await;
    let mut alice = harness.reader("alice").await;
    let work = harness
        .imported_work("archive:example.org", "Twice Asked")
        .await;
    harness
        .set_source_mode("archive:example.org", "cache")
        .await;

    let path = format!("/api/v1/works/{work}/body-request");
    let (first, first_body) = alice.post(&path, json!({})).await;
    assert_eq!(first, StatusCode::ACCEPTED, "{first_body}");
    let (second, second_body) = alice.post(&path, json!({})).await;
    assert_eq!(second, StatusCode::ACCEPTED, "{second_body}");

    assert_eq!(
        second_body["id"], first_body["id"],
        "a second request is the same copy, not a new one"
    );
    assert_eq!(
        harness.copies_for(&work).await,
        1,
        "exactly one row: without the unique index a reader who asks twice has two \
         blobs and the read path has to choose"
    );
}

#[tokio::test]
async fn a_work_with_no_import_record_is_refused_by_name() {
    // The plan's step 4.1 step 0. There is no source, so there is no retention
    // decision to apply — and defaulting to the instance's mode would be a guess
    // about where a work came from, deciding whether a body may be stored.
    let harness = Harness::new("no-import").await;
    let mut alice = harness.reader("alice").await;
    let orphan = uuid::Uuid::new_v4().to_string();
    let now = lorehaven_db::identity::now_rfc3339();
    let sql = match harness.db.backend() {
        lorehaven_db::Backend::Sqlite => {
            "INSERT INTO works (id, owner_pseud_id, title, summary, language, rating,
                                visibility, lifecycle, completion, created_at, updated_at)
             VALUES (?1, (SELECT id FROM pseuds ORDER BY created_at ASC LIMIT 1),
                     'Never Imported', 's', 'en', 'general', 'public', 'active',
                     'in_progress', ?2, ?2)"
        }
        lorehaven_db::Backend::Postgres => {
            "INSERT INTO works (id, owner_pseud_id, title, summary, language, rating,
                                visibility, lifecycle, completion, created_at, updated_at)
             VALUES ($1::uuid, (SELECT id FROM pseuds ORDER BY created_at ASC LIMIT 1),
                     'Never Imported', 's', 'en', 'general', 'public', 'active',
                     'in_progress', $2, $2)"
        }
    };
    match harness.db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(sql)
                .bind(&orphan)
                .bind(&now)
                .execute(harness.db.sqlite_pool().expect("sqlite"))
                .await
                .expect("insert work");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(sql)
                .bind(&orphan)
                .bind(&now)
                .execute(harness.db.postgres_pool().expect("postgres"))
                .await
                .expect("insert work");
        }
    }

    let (status, body) = alice
        .post(&format!("/api/v1/works/{orphan}/body-request"), json!({}))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(err_code(&body), "NO_IMPORT_RECORD", "{body}");
}

#[tokio::test]
async fn an_unsigned_in_reader_cannot_request_or_read_a_copy() {
    // Both routes are `RequireSession`, and the route inventory asserts the
    // audience. A copy is the requesting reader's, so an unauthenticated GET
    // would hand every copy to every caller.
    let harness = Harness::new("anon").await;
    // No work, and deliberately so: `imported_work` needs a pseud to own it, and
    // this test registers nobody, so the subquery that finds one would return
    // NULL against `works.owner_pseud_id`, which is NOT NULL. A bare
    // well-formed id is enough, because both routes are refused before the
    // handler looks anything up — which is the point being tested.
    let work = uuid::Uuid::new_v4().to_string();
    let mut anon = harness.client();

    let (status, body) = anon
        .post(&format!("/api/v1/works/{work}/body-request"), json!({}))
        .await;
    assert!(
        status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
        "an anonymous POST is refused, got {status}: {body}"
    );

    let (status, body) = anon
        .get(&format!("/api/v1/works/{work}/body-request"))
        .await;
    assert!(
        status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
        "an anonymous GET is refused, got {status}: {body}"
    );
}
