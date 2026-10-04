//! Route-level acceptance for the reader surface — items 14, 27 and 33 of the
//! 100-idea audit.
//!
//! Spec: `docs/spec-reader-surface-t1.md`. Store: `crates/db/src/reader_surface.rs`.
//! Plan: `docs/plan-reader-surface-t1.md` step S5.
//!
//! # What this file is for, given the store already has tests
//!
//! The store tests prove the queries are right. This file proves three things they
//! cannot:
//!
//! 1. **The empty response is `{"works": []}`, not `null` and not a 404.** A client
//!    iterating this result must not have to handle null, and "you have nothing new" is
//!    an answer rather than a missing route.
//! 2. **The two public routes really are public.** A login wall in front of data
//!    computed only from public rows would teach readers the data is not published — and
//!    would hide the store's privacy rule from exactly the reader who could check it.
//! 3. **The one per-reader route really does require a session**, so item 14 cannot
//!    silently become a shared route that leaks "what this account reads".

use axum::body::Body;
use axum::http::{Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_db::Database;
use lorehaven_db::DatabaseConfig;
use serde_json::Value;
use std::path::{Path, PathBuf};
use tower::ServiceExt;

const T0: &str = "2026-01-01T00:00:00Z";

/// A timestamp inside the leaderboard's own seven-day window, computed rather than
/// hard-coded.
///
/// This was `T0` in the first version and three tests failed for a reason worth
/// recording: **the store takes the window as a bound parameter so the store tests can
/// pin a clock, but the ROUTE computes the window from `Utc::now()`.** A fixture pinned to
/// a literal date is nine months outside a seven-day window measured from today, so the
/// leaderboard correctly returned nothing and the tests asserted a bug that was not
/// there. A store test can pin the clock because it owns the parameter; a route test
/// cannot, so it has to be relative to now — and anything that *asserts a window*
/// belongs at the store level, where the window is an argument.
fn recent() -> String {
    (chrono::Utc::now() - chrono::Duration::days(1))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-rs-routes-{tag}-{}-{:?}",
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

    /// One statement, dialect-agnostic enough for the SQLite-only fixtures below.
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

    async fn pseud(&self) -> String {
        let acct = self.account().await;
        let id = uuid::Uuid::new_v4().to_string();
        // `pseuds.handle` is UNIQUE, so the handle is the uuid string itself.
        let handle = id.clone();
        self.exec(
            "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3, ?4, ?4)",
            &[&id, &acct, &handle, T0],
        )
        .await;
        id
    }

    async fn work(&self, title: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let owner = self.pseud().await;
        let title = title.to_string();
        self.exec(
            "INSERT INTO works (id, owner_pseud_id, title, summary, lifecycle, visibility,
                                completion, published_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'a summary', 'published', 'public', 'complete', ?4, ?3, ?3)",
            &[&id, &owner, &title, T0],
        )
        .await;
        id
    }

    async fn node(&self, kind: &str, canonical: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let kind = kind.to_string();
        let canonical = canonical.to_string();
        self.exec(
            "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at)
             VALUES (?1, ?2, ?3, ?3, ?4)",
            &[&id, &kind, &canonical, T0],
        )
        .await;
        id
    }

    async fn tag(&self, work_id: &str, node_id: &str) {
        self.exec(
            "INSERT INTO work_tags (work_id, node_id, weight, added_at) VALUES (?1, ?2, 0, ?3)",
            &[work_id, node_id, T0],
        )
        .await;
    }

    async fn get_json(&self, uri: &str) -> (StatusCode, Value) {
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
// The shape, which is the contract every client depends on
// ---------------------------------------------------------------------------------------

/// Every route returns a `works` ARRAY even with nothing in it.
///
/// `null` here would force every client to branch on null-ness, and the branch would be
/// wrong the first time one component forgot it. This asserts the array-ness rather than
/// the emptiness, because "empty" is what an empty database produces by accident and
/// "is an array" is what the type contract promises.
#[tokio::test]
async fn most_bookmarked_returns_an_array_when_there_is_nothing() {
    let h = Harness::new("mb-empty-shape").await;
    let (status, json) = h.get_json("/api/v1/discovery/most-bookmarked").await;

    assert_eq!(status, StatusCode::OK, "an empty leaderboard is not a 404");
    assert!(
        json["works"].is_array(),
        "expected a `works` array, got {json}. A null here breaks every client."
    );
    assert_eq!(json["works"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn similar_returns_an_array_for_an_unknown_work() {
    let h = Harness::new("sim-unknown-shape").await;
    let missing = uuid::Uuid::new_v4();
    let (status, json) = h
        .get_json(&format!("/api/v1/works/{missing}/similar"))
        .await;

    assert_eq!(status, StatusCode::OK, "an empty rail is not a 404");
    assert!(
        json["works"].is_array(),
        "expected a `works` array, got {json}"
    );
}

#[tokio::test]
async fn new_in_your_fandoms_requires_a_session() {
    let h = Harness::new("nif-noauth").await;
    let (status, _) = h.get_json("/api/v1/discovery/new-in-your-fandoms").await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "item 14 answers 'what is NEW TO YOU', so an anonymous caller must not get an answer"
    );
}

// ---------------------------------------------------------------------------------------
// The public door, and the privacy rule it exposes
// ---------------------------------------------------------------------------------------

/// The leaderboard is readable **without a session**, and it counts only public bookmarks.
///
/// This is the route-level half of the store's privacy rule: the query's
/// `is_public = 1` predicate is what makes the door safe to leave open, so the two
/// belong in the same test. Ten private bookmarks on one work, one public on another —
/// the public one leads.
#[tokio::test]
async fn most_bookmarked_is_public_and_counts_only_public_bookmarks() {
    let h = Harness::new("mb-public-door").await;
    let hidden = h.work("Hidden").await;
    let shown = h.work("Shown").await;

    for _ in 0..10 {
        let a = h.account().await;
        h.exec(
            "INSERT INTO bookmarks (id, account_id, subject_type, subject_id, is_public,
                                    created_at, updated_at)
             VALUES (?1, ?2, 'work', ?3, 0, ?4, ?4)",
            &[&uuid::Uuid::new_v4().to_string(), &a, &hidden, &recent()],
        )
        .await;
    }
    let a = h.account().await;
    h.exec(
        "INSERT INTO bookmarks (id, account_id, subject_type, subject_id, is_public,
                                created_at, updated_at)
         VALUES (?1, ?2, 'work', ?3, 1, ?4, ?4)",
        &[&uuid::Uuid::new_v4().to_string(), &a, &shown, &recent()],
    )
    .await;

    // No Authorization header: this is the public door.
    let (status, json) = h.get_json("/api/v1/discovery/most-bookmarked").await;

    assert_eq!(
        status,
        StatusCode::OK,
        "the leaderboard must be readable logged out, or a login wall teaches readers the data is private"
    );
    let titles: Vec<&str> = json["works"]
        .as_array()
        .expect("array")
        .iter()
        .map(|w| w["title"].as_str().expect("title"))
        .collect();
    assert_eq!(
        titles,
        vec!["Shown"],
        "a work with only private bookmarks reached a PUBLIC leaderboard: {titles:?}"
    );
}

/// The count a reader sees is a count of public bookmarkers, and it is a distinct count.
///
/// Two public bookmarks by the same reader is one reader. This asserts the number the
/// API renders, not just the ordering the store test asserts.
#[tokio::test]
async fn most_bookmarked_reports_distinct_public_bookmarkers() {
    let h = Harness::new("mb-distinct-count").await;
    let w = h.work("Counted Once").await;

    let a = h.account().await;
    for _ in 0..3 {
        h.exec(
            "INSERT INTO bookmarks (id, account_id, subject_type, subject_id, is_public,
                                    created_at, updated_at)
             VALUES (?1, ?2, 'work', ?3, 1, ?4, ?4)",
            &[&uuid::Uuid::new_v4().to_string(), &a, &w, &recent()],
        )
        .await;
    }

    let (_, json) = h.get_json("/api/v1/discovery/most-bookmarked").await;
    assert_eq!(
        json["works"][0]["recent_bookmarks"], 1,
        "one reader bookmarking three times counted as three bookmarkers"
    );
}

/// A window of zero would return everything since the epoch under a heading that says
/// "this week" — a wrong answer with a correct-looking label. Refused, not clamped.
#[tokio::test]
async fn most_bookmarked_refuses_a_zero_window() {
    let h = Harness::new("mb-zero-window").await;
    let (status, _) = h
        .get_json("/api/v1/discovery/most-bookmarked?window_days=0")
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "window_days=0 must be refused, not silently clamped to a different question. \
         422 is this codebase's validation status (error.rs::status_code)."
    );
}

// ---------------------------------------------------------------------------------------
// Similar works, end to end
// ---------------------------------------------------------------------------------------

/// The rail is public and it carries the score that produced the ordering.
///
/// The score is in the response on purpose: it is what lets the UI say WHY a work is on
/// the rail ("shares the fandom Harry Potter") instead of asserting "similar", which is
/// the thing spec §5 says must not be an unsubstantiated claim.
#[tokio::test]
async fn similar_is_public_and_returns_scored_matches() {
    let h = Harness::new("sim-public-scored").await;
    let hp = h.node("fandom", "Harry Potter").await;
    let angst = h.node("mood", "angst").await;

    let subject = h.work("Subject").await;
    h.tag(&subject, &hp).await;
    h.tag(&subject, &angst).await;

    let match_work = h.work("Match").await;
    h.tag(&match_work, &hp).await;
    h.tag(&match_work, &angst).await;

    let (status, json) = h
        .get_json(&format!("/api/v1/works/{subject}/similar"))
        .await;

    assert_eq!(status, StatusCode::OK);
    let works = json["works"].as_array().expect("array");
    assert_eq!(
        works.len(),
        1,
        "expected exactly the one genuine match, got {json}"
    );
    assert_eq!(works[0]["id"], match_work.as_str());
    let score = works[0]["similarity"].as_f64().expect("a numeric score");
    assert!(
        (0.0..=1.0).contains(&score),
        "score {score} escaped [0,1] — a client cannot compare scores across works"
    );
}

/// The work itself never appears on its own rail, even over HTTP.
#[tokio::test]
async fn similar_never_recommends_the_work_itself() {
    let h = Harness::new("sim-not-self").await;
    let hp = h.node("fandom", "Harry Potter").await;
    let angst = h.node("mood", "angst").await;

    let subject = h.work("Subject").await;
    h.tag(&subject, &hp).await;
    h.tag(&subject, &angst).await;

    let (_, json) = h
        .get_json(&format!("/api/v1/works/{subject}/similar"))
        .await;
    let ids: Vec<&str> = json["works"]
        .as_array()
        .expect("array")
        .iter()
        .map(|w| w["id"].as_str().expect("id"))
        .collect();
    assert!(
        !ids.contains(&subject.as_str()),
        "a work appeared on its own similar-works rail"
    );
}
