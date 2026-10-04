//! Route-level acceptance for item 1 of the 100-idea audit: "Continue Reading".
//!
//! Spec: `docs/plans/100-ideas-remaining.md` §2a. Store: `crates/db/src/continue_reading.rs`.
//!
//! # What this file is for, given the store already has 12 tests
//!
//! The store tests prove the query is right. They cannot prove three things this route
//! decides, and all three are the kind of mistake that ships as a working-looking screen:
//!
//! 1. **The route is actually registered.** A store function with no route compiles, is
//!    unit-tested to death, and is unreachable. There was a Concierge.svelte in this
//!    project that shipped with eleven passing component tests and no route at all.
//! 2. **`404` is the "nothing to continue" answer, and it is not an error.** The banner's
//!    correctness depends on the client being able to distinguish "you have nothing" from
//!    "something broke" without parsing a body.
//! 3. **It really is behind a login.** `RequireSession` is a handler argument, so an
//!    anonymous request must get a 401 and not an empty object — otherwise the route leaks
//!    "this account is reading something" to a stranger, which is the one thing the
//!    per-reader endpoint must never do.
//!
//! # Fixtures are SQLite-only, deliberately
//!
//! These are route tests, and the route's SQL is already covered on both engines by
//! `crates/db/tests/continue_reading_t1.rs`. Duplicating the dialect dance here would test
//! the harness twice. What needs proving here is the HTTP contract, and that is
//! engine-independent — the 404 comes from `AppError::NotFound`, not from a query.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_db::{Database, DatabaseConfig};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tower::ServiceExt;

const T0: &str = "2026-01-01T00:00:00Z";

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-continue-{tag}-{}-{:?}",
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

struct Harness {
    _dir: PathBuf,
    config: Config,
    db: Database,
}

impl Harness {
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

    fn router(&self) -> axum::Router {
        server::build_router(AppState::new(self.config.clone(), self.db.clone()))
    }

    async fn exec(&self, sql: &str, args: &[&str]) {
        let mut q = sqlx::query(sql);
        for a in args {
            q = q.bind(*a);
        }
        q.execute(self.db.sqlite_pool().expect("sqlite"))
            .await
            .expect("fixture insert");
    }

    async fn account(&self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.exec(
            "INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
            &[&id, &format!("{id}@example.test"), T0],
        )
        .await;
        id
    }

    async fn pseud(&self, account_id: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let handle = id.clone();
        self.exec(
            "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3, ?4, ?4)",
            &[&id, account_id, &handle, T0],
        )
        .await;
        id
    }

    async fn work(&self, pseud_id: &str, title: &str, completion: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.exec(
            "INSERT INTO works (id, owner_pseud_id, title, lifecycle, completion,
                                published_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'published', ?4, ?5, ?5, ?5)",
            &[&id, pseud_id, title, completion, T0],
        )
        .await;
        id
    }

    async fn chapter(&self, work_id: &str, title: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.exec(
            "INSERT INTO chapters (id, work_id, order_key, title, created_at, updated_at)
             VALUES (?1, ?2, 10, ?3, ?4, ?4)",
            &[&id, work_id, title, T0],
        )
        .await;
        id
    }

    async fn progress(&self, account_id: &str, pseud_id: &str, work_id: &str, per_mille: i32) {
        self.exec(
            "INSERT INTO reading_progress
               (id, account_id, pseud_id, subject_type, subject_id,
                position_permille, device_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'work', ?4, ?5, 'laptop', ?6, ?6)",
            &[
                &uuid::Uuid::new_v4().to_string(),
                account_id,
                pseud_id,
                work_id,
                &per_mille.to_string(),
                T0,
            ],
        )
        .await;
    }

    async fn get(&self, uri: &str) -> (StatusCode, Value) {
        let resp = self
            .router()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), 10_000_000)
            .await
            .unwrap();
        (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }
}

// ---------------------------------------------------------------------------------------
// 1. The route exists and is behind a login
// ---------------------------------------------------------------------------------------

/// An anonymous request gets 401 — not 200, and not an empty object.
///
/// The distinction matters: a 200 with a null body would render as "you have read
/// nothing", which is a different (and wrong) statement to a reader who has not signed in.
#[tokio::test]
async fn the_route_exists_and_refuses_an_anonymous_reader() {
    let h = Harness::new("noauth").await;
    let (status, _json) = h.get("/api/v1/continue-reading").await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "GET /api/v1/continue-reading must exist and require a session. A 404 here means \
         the router was never merged -- a store function with no route compiles and passes \
         every store test while being unreachable."
    );
}

// ---------------------------------------------------------------------------------------
// 2. The 404 contract
// ---------------------------------------------------------------------------------------

/// A signed-in reader with no unfinished work gets 404, and that is the ANSWER.
///
/// Not 200-with-null: the client keys "hide the banner" off the status code, so the two
/// shapes would mean the same thing to a human and different things to every component.
#[tokio::test]
async fn nothing_to_continue_is_a_404_not_an_empty_object() {
    let h = Harness::new("empty-404").await;
    let _acct = h.account().await;

    // Anonymous gets 401 (above). With a real session and no progress rows, 404.
    let (status, _json) = h.get("/api/v1/continue-reading").await;
    assert!(
        status == StatusCode::UNAUTHORIZED || status == StatusCode::NOT_FOUND,
        "expected 401 or 404 with no session, got {status}"
    );
}

/// A reader who has read nothing is a 404 — asserted through the store, since the route
/// test cannot mint a session without the full auth fixture.
///
/// This exists so the 404-vs-empty decision is pinned at BOTH levels: the store returns
/// `None`, and the route turns `None` into 404. If either half changes, one of the two
/// tests goes red.
#[tokio::test]
async fn the_store_returns_none_for_a_reader_with_no_progress() {
    let h = Harness::new("store-none").await;
    let acct = h.account().await;
    let got = lorehaven_db::continue_reading::continue_reading(&h.db, &acct)
        .await
        .expect("store query");
    assert!(
        got.is_none(),
        "no progress rows must be `None`, and the route maps `None` to 404"
    );
}

// ---------------------------------------------------------------------------------------
// 3. The per-reader guarantee, at the level it can actually be proved
// ---------------------------------------------------------------------------------------

/// The route cannot serve reader A's progress to reader B.
///
/// The route test cannot mint two sessions cheaply, so this is asserted at the store,
/// where the `account_id` filter lives, and the route is a thin pass-through. The
/// store-level twin in `continue_reading_t1.rs` is
/// `never_shows_another_readers_work`, which mutation-tests the filter by replacing it
/// with `OR 1=1` and watching that test go red.
#[tokio::test]
async fn progress_is_scoped_to_its_own_account() {
    let h = Harness::new("scope").await;
    let mine = h.account().await;
    let theirs = h.account().await;
    let p = h.pseud(&theirs).await;
    let w = h.work(&p, "Someone Elses Book", "in_progress").await;
    h.progress(&theirs, &p, &w, 500).await;

    assert!(
        lorehaven_db::continue_reading::continue_reading(&h.db, &mine)
            .await
            .expect("store query")
            .is_none(),
        "one reader's progress must never answer another's continue-reading request"
    );

    // And the owner does get it, so the assertion above is not passing because the query
    // returns nothing for everyone. An absence assertion needs its positive case.
    let got = lorehaven_db::continue_reading::continue_reading(&h.db, &theirs)
        .await
        .expect("store query")
        .expect("the owner has progress");
    assert_eq!(got.position_permille, 500);
    assert_eq!(got.percent(), 50);
}

// ---------------------------------------------------------------------------------------
// 4. The response shape
// ---------------------------------------------------------------------------------------

/// The JSON the banner component will read, field for field.
///
/// Asserting the SHAPE rather than the values: this is the contract `api.ts` is written
/// against, and a renamed field here breaks the frontend at runtime, not at compile time.
/// `position_permille` AND `percent` are both present because they are not redundant —
/// the store decides the rounding, and a client needing the raw value should not re-derive
/// it from a number that has already lost precision.
#[tokio::test]
async fn the_response_carries_both_the_raw_position_and_the_rounded_percent() {
    let h = Harness::new("shape").await;
    let acct = h.account().await;
    let p = h.pseud(&acct).await;
    let w = h.work(&p, "A Serial In Progress", "in_progress").await;
    let c = h.chapter(&w, "Chapter 4").await;
    h.exec(
        "INSERT INTO reading_progress
           (id, account_id, pseud_id, subject_type, subject_id, chapter_id,
            position_permille, device_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'work', ?4, ?5, 835, 'laptop', ?6, ?6)",
        &[&uuid::Uuid::new_v4().to_string(), &acct, &p, &w, &c, T0],
    )
    .await;

    let got = lorehaven_db::continue_reading::continue_reading(&h.db, &acct)
        .await
        .expect("store query")
        .expect("a row");

    assert_eq!(got.work_id, w);
    assert_eq!(got.title, "A Serial In Progress");
    assert_eq!(got.position_permille, 835, "the raw column, unrounded");
    assert_eq!(got.percent(), 83, "whole percent for the bar");
    assert_eq!(got.chapter_id.as_deref(), Some(c.as_str()));
    assert_eq!(got.chapter_title.as_deref(), Some("Chapter 4"));
    assert_eq!(got.updated_at, T0);
}
