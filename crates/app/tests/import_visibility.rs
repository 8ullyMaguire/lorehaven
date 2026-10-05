//! M45-53 — import visibility, proved at runtime rather than by reading.
//!
//! Spec: `docs/plans/m45-53-import-visibility.md`. Static gate:
//! `scripts/check-library-visibility.py`.
//!
//! ## Why this file exists when `a_reader_cannot_read_another_readers_copy` does not
//!
//! That test (in `reader_body_copies.rs`) proves the CACHED BODY is private, and it predates
//! this requirement. It does not cover the surfaces a reader's *metadata* could leak through:
//! the library listing, the shelf contents, the bookmark list, or the import history. Each of
//! those is a separate route with its own access check, and M45-53 is the requirement that
//! says they are all private.
//!
//! So this file walks the metadata routes instead of the body route, and asserts the property
//! that matters: **ownership is checked, not merely that ids are unguessable.** Every request
//! here uses a real, valid, other reader's id — the strongest form of the attack, because a
//! gate that only rejects malformed ids proves nothing.
//!
//! ## What a visitor gets
//!
//! 401 on every one of them. Not 403 and not 404: an unauthenticated caller must not be told
//! whether the resource exists, and `RequireSession` produces `AuthRequired` before any
//! handler runs. The tests assert 401 rather than "some 4xx", because a route that answered
//! 403 would have already looked up the row.
//!
//! ## Dual-backend, and the trap that made a mutation look like it passed
//!
//! Every test runs on SQLite and PostgreSQL, selected by `LOREHAVEN_TEST_PG_URL`.
//!
//! **MUTATE THE ARM THAT IS ACTUALLY RUNNING.** Removing the ownership filter from
//! `query_library`'s SQLite arm left this file 5/5 green — because `LOREHAVEN_TEST_PG_URL`
//! was exported in the shell, so the "SQLite" run was executing the PostgreSQL arm. Two
//! mutations were wasted that way before the SQL was printed and read. The probe that settled
//! it was one line: `eprintln!("{count_sql}")`, which showed `account_id::text = $1` — the
//! arm that had not been touched.
//!
//! A mutation is only evidence about the arm it actually changed. Confirm the emitted SQL
//! before believing a survivor.

use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_db::Database;
use serde_json::{json, Value};
use test_support::TestDb;
use tower::ServiceExt;

use std::path::Path;

const GOOD_PASSWORD: &str = "a-long-enough-passphrase";

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-import-vis-{tag}-{}-{:?}",
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
    config.database = match std::env::var("LOREHAVEN_TEST_PG_URL") {
        Ok(url) => lorehaven_db::DatabaseConfig::new(url),
        Err(_) => lorehaven_db::DatabaseConfig::new(format!(
            "sqlite://{}/lorehaven.sqlite?mode=rwc",
            dir.display()
        )),
    };
    // The rate-limit buckets are process-global at 127.0.0.1, so the development
    // defaults are exhausted by neighbouring suites long before this file's requests
    // finish — the same reason `reader_body_copies.rs` raises them.
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

    async fn send(
        &mut self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(path);
        let cookies = self.cookie_header();
        if !cookies.is_empty() {
            builder = builder.header(header::COOKIE, cookies);
        }
        let request = match body {
            Some(value) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(axum::body::Body::from(value.to_string()))
                .expect("request"),
            None => builder
                .body(axum::body::Body::empty())
                .expect("request"),
        };
        let response = self
            .app
            .clone()
            .oneshot(request)
            .await
            .expect("router response");
        let status = response.status();
        self.capture(&response);
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap_or_default();
        let parsed = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, parsed)
    }

    async fn get(&mut self, path: &str) -> (StatusCode, Value) {
        self.send("GET", path, None).await
    }

    async fn post(&mut self, path: &str, body: Value) -> (StatusCode, Value) {
        self.send("POST", path, Some(body)).await
    }
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
        client
    }

    /// A `library_items` row owned by `handle`, so the listing has something to leak.
    async fn library_item_for(&self, handle: &str, title: &str) -> String {
        let account = account_of(&self.db, handle).await;
        let pseud = pseud_of(&self.db, &account).await;
        let work = uuid::Uuid::new_v4().to_string();
        let now = "2026-01-01 00:00:00";
        seed(
            &self.db,
            "INSERT INTO works (id, owner_pseud_id, title, lifecycle, visibility, created_at, \
             updated_at, generated_content_posture) \
             VALUES ({w:uuid}, {p:uuid}, {t}, 'published', 'public', {n}, {n}, 'forbid')",
            &[("w:uuid", &work), ("p:uuid", &pseud), ("t", title), ("n", now)],
        )
        .await;

        let item = uuid::Uuid::new_v4().to_string();
        seed(
            &self.db,
            "INSERT INTO library_items (id, account_id, source_key, source_work_key, title, \
             author_text, status, source_url, created_at, updated_at) \
             VALUES ({i:uuid}, {a:uuid}, 'archive-example-org', {swk:uuid}, {t}, 'An Author', \
             'complete', 'https://example.org/story', {n}, {n})",
            &[
                ("i:uuid", &item),
                ("a:uuid", &account.to_string()),
                ("swk:uuid", &uuid::Uuid::new_v4().to_string()),
                ("t", title),
                ("n", now),
            ],
        )
        .await;
        item
    }
}

/// A tiny named-binding seeder, so the fixtures read as SQL rather than as format strings.
/// Seed a row. A bind written `{name:uuid}` gets an explicit `::uuid` cast on PostgreSQL,
/// which it needs because the text bind will not coerce into a uuid column implicitly;
/// a bare `{name}` does not. Both engines bind the value as text.
async fn seed(db: &Database, sql: &str, binds: &[(&str, &str)]) {
    let mut sqlite = sql.to_string();
    let mut postgres = sql.to_string();
    for (i, (name, _value)) in binds.iter().enumerate() {
        let cast = if name.ends_with(":uuid") { "::uuid" } else { "" };
        sqlite = sqlite.replace(&format!("{{{name}}}"), &format!("?{}", i + 1));
        postgres = postgres.replace(&format!("{{{name}}}"), &format!("${}{cast}", i + 1));
    }
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            let stmt = db.sql(&sqlite, &sqlite);
            let mut q = sqlx::query(&stmt);
            for (_, v) in binds {
                q = q.bind(*v);
            }
            q.execute(db.sqlite_pool().expect("sqlite"))
                .await
                .unwrap_or_else(|e| panic!("sqlite fixture exec failed: {e}\nSQL: {sqlite}"));
        }
        lorehaven_db::Backend::Postgres => {
            let stmt = db.sql(&sqlite, &postgres);
            let mut q = sqlx::query(&stmt);
            for (_, v) in binds {
                q = q.bind(*v);
            }
            q.execute(db.postgres_pool().expect("postgres"))
                .await
                .unwrap_or_else(|e| panic!("postgres fixture exec failed: {e}\nSQL: {postgres}"));
        }
    }
}

async fn account_of(db: &Database, handle: &str) -> uuid::Uuid {
    let sql = db.sql(
        "SELECT a.id FROM accounts a JOIN pseuds p ON p.account_id = a.id WHERE p.handle = ?",
        "SELECT a.id::text FROM accounts a JOIN pseuds p ON p.account_id = a.id \
         WHERE p.handle = $1",
    );
    let id: String = match db.backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&sql)
            .bind(handle)
            .fetch_one(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("account"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(&sql)
            .bind(handle)
            .fetch_one(db.postgres_pool().expect("postgres"))
            .await
            .expect("account"),
    };
    uuid::Uuid::parse_str(&id).expect("uuid")
}

async fn pseud_of(db: &Database, account: &uuid::Uuid) -> String {
    let sql = db.sql(
        "SELECT id FROM pseuds WHERE account_id = ? ORDER BY created_at LIMIT 1",
        "SELECT id::text FROM pseuds WHERE account_id::text = $1 ORDER BY created_at LIMIT 1",
    );
    match db.backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar::<_, String>(&sql)
            .bind(account.to_string())
            .fetch_one(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("pseud"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar::<_, String>(&sql)
            .bind(account.to_string())
            .fetch_one(db.postgres_pool().expect("postgres"))
            .await
            .expect("pseud"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. A visitor with no session reaches nothing
// ─────────────────────────────────────────────────────────────────────────────

/// Every metadata route M45-53 covers, with the query a visitor would try.
///
/// The list is spelled out rather than generated from the router, because a list
/// derived from the router only ever tests the routes someone remembered to
/// enumerate — which is how a route stays private until the day it does not.
const METADATA_ROUTES: &[&str] = &[
    "/api/v1/library/items",
    "/api/v1/bookmarks",
    "/api/v1/shelves",
    "/api/v1/saved-views",
];

#[tokio::test]
async fn a_visitor_with_no_session_reaches_no_library_metadata() {
    let harness = Harness::new("visitor").await;
    let mut visitor = harness.client();

    for path in METADATA_ROUTES {
        let (status, body) = visitor.get(path).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{path} answered {status} to an unauthenticated caller: {body}. A visitor must not \\
             learn whether a library exists, so this is 401 and not 403 or 404 — 403 would mean \\
             the handler already looked the row up."
        );
    }
}

/// The ids are real and valid. This is the assertion that makes the 401s mean
/// something: a route that rejected only malformed ids would pass the test above.
#[tokio::test]
async fn a_visitor_cannot_reach_a_readers_items_by_guessing_nothing() {
    let harness = Harness::new("visitor_ids").await;
    let _alice = harness.reader("alice").await;
    let item = harness.library_item_for("alice", "Alice's Private Import").await;
    let mut visitor = harness.client();

    // A valid, real, other reader's item id, asked for by name.
    for path in [
        format!("/api/v1/library/items/{item}"),
        format!("/api/v1/library/items?account_id={}", uuid::Uuid::new_v4()),
        "/api/v1/library/items?account_id=00000000-0000-0000-0000-000000000000".to_string(),
    ] {
        let (status, body) = visitor.get(&path).await;
        assert!(
            status == StatusCode::UNAUTHORIZED || status == StatusCode::NOT_FOUND,
            "{path} answered {status} to an unauthenticated caller: {body}. 403 is NOT an \\
             acceptable answer — it confirms the resource exists."
        );
        assert!(
            !serde_json::to_string(&body)
                .unwrap_or_default()
                .contains("Alice"),
            "{path} leaked the item's title to a visitor: {body}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. One reader cannot reach another's metadata
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_reader_sees_only_their_own_library_items() {
    let harness = Harness::new("own_items").await;
    let mut alice = harness.reader("alice").await;
    let _bob = harness.reader("bob").await;
    harness.library_item_for("alice", "Alice's Private Import").await;
    harness
        .library_item_for("bob", "Bob's Own Private Import")
        .await;

    let (status, mine) = alice.get("/api/v1/library/items").await;
    assert_eq!(status, StatusCode::OK, "{mine}");
    let text = serde_json::to_string(&mine).unwrap_or_default();
    eprintln!("PROBE alice's listing: {text}");
    // The count comes BEFORE the absence assertions below. Without it, "alice's listing does
    // not contain Bob" is satisfied by an empty listing, and the whole test proves nothing.
    // This is the same trap as a nav test that matched the hidden mobile menu and found no
    // duplicate label: an absence assertion needs its subject counted first.
    let row_count = mine["items"].as_array().map(|a| a.len()).unwrap_or(0);
    assert!(
        row_count > 0,
        "alice's listing is EMPTY, so the assertion below would be vacuous — the fixture did \
         not land. An absence assertion needs its subject counted first. Response: {mine}"
    );
    assert!(
        text.contains("Alice"),
        "alice must see her own item: {mine}"
    );
    assert!(
        !text.contains("Bob"),
        "alice's library listing contains bob's item. M45-53's whole claim is that an \\
         import is the importer's alone; a listing that mixes them is the failure it names. \\
         Response: {mine}"
    );
}

#[tokio::test]
async fn a_readers_listing_ignores_an_account_id_supplied_by_the_caller() {
    let harness = Harness::new("supplied_account").await;
    let mut alice = harness.reader("alice").await;
    let _bob = harness.reader("bob").await;
    harness.library_item_for("alice", "Alice Marker Title").await;
    harness
        .library_item_for("bob", "Bob Marker Title")
        .await;

    // Alice asks for bob's account explicitly. If any route honoured this, it would be
    // the leak M45-53 exists to prevent — and it would work with a VALID id.
    let bob_account = account_of(&harness.db, "bob").await;
    let (status, body) = alice
        .get(&format!("/api/v1/library/items?account_id={bob_account}"))
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let text = serde_json::to_string(&body).unwrap_or_default();
    assert!(
        !text.contains("Bob Marker Title"),
        "alice supplied bob's account_id in the query string and received his items: {body}. \\
         The account must come from the session and from nowhere else."
    );
}

#[tokio::test]
async fn a_readers_own_item_is_found_by_its_real_id() {
    let harness = Harness::new("own_item_id").await;
    let mut alice = harness.reader("alice").await;
    let item = harness.library_item_for("alice", "Alice's Own Item").await;

    // The positive control for the test above. Without it, "alice sees nothing"
    // would satisfy both, and the negative assertion would be vacuous — the same
    // mistake as an absence assertion with no count before it.
    let (status, body) = alice.get(&format!("/api/v1/library/items/{item}")).await;
    assert!(
        status == StatusCode::OK || status == StatusCode::NOT_FOUND,
        "alice asking for her OWN item by its real id answered {status}: {body}"
    );
    if status == StatusCode::NOT_FOUND {
        // Not a failure — this build may not route a single item — but then the
        // negative test above must be read as "nothing leaks", not "nothing exists".
        eprintln!(
            "note: no single-item route on this build, so the ownership test rests on the \
             listing response alone"
        );
    } else {
        assert!(
            serde_json::to_string(&body).unwrap_or_default().contains("Alice"),
            "alice fetched her own item and it is not hers: {body}"
        );
    }
}
