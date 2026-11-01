//! M63-04: `seed`, `author` and `exclude_read` through the real router.
//!
//! The store properties are in `crates/db/tests/catalogue_similar.rs`. These are
//! the ones that belong to the route:
//!
//! - a seed page answers `sort: "similarity"` with `ranked`, `total` **and**
//!   `scanned`, because a page cannot say how much of the catalogue it looked at;
//! - an unknown seed is a `404` and an ambiguous one is a `422` that names the
//!   candidates — the two are different answers and an empty page is neither;
//! - `seed` and `sort` together are refused rather than ordered by one and
//!   silently ignoring the other;
//! - `exclude_read` without a session is a `422` that names the field, not a 200
//!   with nothing excluded.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use darkhold_app::config::Config;
use darkhold_app::server;
use darkhold_app::state::AppState;
use darkhold_db::Database;
use serde_json::Value;
use test_support::TestDb;
use tower::ServiceExt;

const T0: &str = "2026-01-01T00:00:00Z";

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "darkhold-m63-04-{tag}-{}-{:?}",
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
    config
}

struct Harness {
    _tdb: TestDb,
    _dir: PathBuf,
    app: axum::Router,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let config = config_for(&dir);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        let db = tdb.db().clone();
        seed(&db).await;
        let app = server::build_router(AppState::new(config, db));
        Self {
            _tdb: tdb,
            _dir: dir,
            app,
        }
    }

    async fn get(&self, uri: &str) -> (StatusCode, Value) {
        let request = Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .expect("request");
        let response = self.app.clone().oneshot(request).await.expect("response");
        let status = response.status();
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
}

/// `(source_work_key, title, summary, author)`.
const ROWS: &[(&str, &str, &str, &str)] = &[
    (
        "ghost",
        "Ghost in the City",
        "A cyberpunk gamer wakes up in Night City with a system",
        "ZFighter18",
    ),
    (
        "rewrite",
        "Ghost in the City (rewrite)",
        "The same Night City gamer story, retold",
        "Someone Else",
    ),
    (
        "mech",
        "Battletech Mercenary Tales",
        "A mechwarrior story about contracts and dropships",
        "Oshha",
    ),
    (
        "mech-two",
        "Aurigan Battleship",
        "Another mechwarrior contract story",
        "Oshha",
    ),
];

async fn seed(db: &Database) {
    let account = uuid::Uuid::new_v4().to_string();
    match db.backend() {
        darkhold_db::Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            sqlx::query(
                "INSERT INTO accounts (id, email, status, age_state, created_at, updated_at, permission_statement)
                 VALUES ($1::uuid, $2, 'active', 'adult', $3, $3, 'minimal')",
            )
            .bind(uuid::Uuid::parse_str(&account).expect("uuid"))
            .bind(format!("{account}@example.invalid"))
            .bind(T0)
            .execute(pool)
            .await
            .expect("account");
            for (index, (key, title, summary, author)) in ROWS.iter().enumerate() {
                sqlx::query(
                    "INSERT INTO library_items
                     (id, account_id, source_key, source_work_key, title, author_text, summary,
                      word_count, status, source_url, source_updated_at, last_synced_at, created_at,
                      updated_at, likes)
                     VALUES ($1::uuid, $2::uuid, 'sb', $3, $4, $5, $6, 20000, 'ongoing',
                             'https://example.invalid/x', $7, $7, $7, $7, $8)",
                )
                .bind(uuid::Uuid::new_v4())
                .bind(uuid::Uuid::parse_str(&account).expect("uuid"))
                .bind(*key)
                .bind(*title)
                .bind(*author)
                .bind(*summary)
                .bind(T0)
                .bind(10_000 - (index as i64 * 100))
                .execute(pool)
                .await
                .expect("library item");
            }
        }
        darkhold_db::Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            sqlx::query(
                "INSERT INTO accounts (id, email, status, age_state, created_at, updated_at, permission_statement)
                 VALUES (?, ?, 'active', 'adult', ?, ?, 'minimal')",
            )
            .bind(&account)
            .bind(format!("{account}@example.invalid"))
            .bind(T0)
            .bind(T0)
            .execute(pool)
            .await
            .expect("account");
            for (index, (key, title, summary, author)) in ROWS.iter().enumerate() {
                sqlx::query(
                    "INSERT INTO library_items
                     (id, account_id, source_key, source_work_key, title, author_text, summary,
                      word_count, status, source_url, source_updated_at, last_synced_at, created_at,
                      updated_at, likes)
                     VALUES (?, ?, 'sb', ?, ?, ?, ?, 20000, 'ongoing', 'https://example.invalid/x',
                             ?, ?, ?, ?, ?)",
                )
                .bind(uuid::Uuid::new_v4().to_string())
                .bind(&account)
                .bind(*key)
                .bind(*title)
                .bind(*author)
                .bind(*summary)
                .bind(T0)
                .bind(T0)
                .bind(T0)
                .bind(T0)
                .bind(10_000 - (index as i64 * 100))
                .execute(pool)
                .await
                .expect("library item");
            }
        }
    }
}

#[tokio::test]
async fn a_seed_page_answers_similarity_with_all_three_counts() {
    let harness = Harness::new("seed").await;
    let (status, body) = harness
        .get("/api/v1/catalogue?seed=Ghost%20in%20the%20City&limit=2")
        .await;
    assert_eq!(status, StatusCode::OK, "seed page: {body}");
    assert_eq!(body["sort"], "similarity");
    assert_eq!(body["metric"], "similarity");
    assert_eq!(body["seed"]["title"], "Ghost in the City");
    for field in ["ranked", "total", "scanned"] {
        assert!(
            body[field].as_i64().is_some(),
            "the gate needs {field}: {body}"
        );
    }
    let items = body["items"].as_array().expect("items");
    // The preselection matched the seed and the rewrite; the seed never ranks
    // itself, so one neighbour. `limit=2` asks for up to two.
    assert_eq!(items.len(), 1, "one neighbour exists: {body}");
    assert_eq!(items[0]["source_work_key"], "rewrite", "the rewrite leads: {body}");
    assert_eq!(body["ranked"], 1, "ranked counts neighbours: {body}");
    assert_eq!(body["total"], 2, "total counts what the preselection matched: {body}");
    assert!(
        !items[0]["shared"].as_array().expect("shared").is_empty(),
        "a similarity page explains itself: {body}"
    );
    assert!(
        items.iter().all(|item| item["source_work_key"] != "mech"),
        "a mechwarrior story is not a neighbour: {body}"
    );
}

#[tokio::test]
async fn an_unknown_seed_is_a_404_not_an_empty_page() {
    let harness = Harness::new("missing").await;
    let (status, body) = harness.get("/api/v1/catalogue?seed=nothing%20called%20this").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unknown seed: {body}");
}

#[tokio::test]
async fn an_ambiguous_seed_names_the_candidates() {
    let harness = Harness::new("ambiguous").await;
    let (status, body) = harness.get("/api/v1/catalogue?seed=Ghost").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "ambiguous seed: {body}");
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("rows match") && message.contains("Ghost in the City"),
        "the refusal names the rows, was {message:?}"
    );
}

#[tokio::test]
async fn a_seed_refuses_a_second_ordering() {
    let harness = Harness::new("two-orders").await;
    let (status, body) = harness
        .get("/api/v1/catalogue?seed=Ghost%20in%20the%20City&sort=source_pct")
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "seed + sort: {body}");
}

#[tokio::test]
async fn excluding_read_without_a_session_is_refused_by_name() {
    let harness = Harness::new("exclude").await;
    let (status, body) = harness.get("/api/v1/catalogue?exclude_read=1").await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "exclude_read without a session: {body}"
    );
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("exclude_read"),
        "the refusal names the field, was {message:?}"
    );
}

#[tokio::test]
async fn the_author_filter_reaches_the_page() {
    let harness = Harness::new("author").await;
    let (status, body) = harness.get("/api/v1/catalogue?author=oshha").await;
    assert_eq!(status, StatusCode::OK, "author page: {body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 2, "two rows by that author, got {body}");
    assert!(
        items.iter().all(|item| item["author_text"] == "Oshha"),
        "every row carries the pseud: {body}"
    );
}
