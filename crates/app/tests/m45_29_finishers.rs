//! M45-29 — §60 finisher signals through the real router.
//!
//! The store is tested in `darkhold-db`; what only the route can get wrong:
//!
//! - **the module ships OFF** — a default config answers `404 MODULE_UNAVAILABLE` for both doors,
//!   and an operator enabling it (`[modules] enabled`) is what turns them on. That is the review's
//!   "off by default", and it has to survive being a compiled default;
//! - both doors are public once enabled;
//! - an unknown author is `404`, an author with nothing concluded is `200` with zeroes;
//! - **the payload has no negative key and no negative word** — the framing rule is asserted against
//!   the JSON that actually goes out, not against the function that built it;
//! - cadence for an import-shaped work is `measurable: false` with a reason and a `200`.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use darkhold_app::config::Config;
use darkhold_app::server;
use darkhold_app::state::AppState;
use serde_json::{json, Value};
use test_support::TestDb;
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "darkhold-m45-29-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn config_for(dir: &Path, enabled: bool) -> Config {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    if enabled {
        config.modules.enabled = vec!["finishers".to_owned()];
    }
    config
}

struct Harness {
    _dir: PathBuf,
    tdb: TestDb,
    config: Config,
}

impl Harness {
    async fn new(tag: &str, enabled: bool) -> Self {
        let dir = scratch_dir(tag);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        let config = config_for(&dir, enabled);
        Self {
            _dir: dir,
            tdb,
            config,
        }
    }

    async fn get(&self, uri: &str) -> (StatusCode, Value) {
        let request = Request::builder()
            .method("GET")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::empty())
            .expect("request");
        let app = server::build_router(AppState::new(
            self.config.clone(),
            self.tdb.db().clone(),
        ));
        let response = app.oneshot(request).await.expect("response");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body");
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, value)
    }
}

async fn author_with_works(h: &Harness, completion_counts: &[(i64, &str)]) -> String {
    // One account with one activated pseud, and `count` works in each completion state.
    let account = uuid::Uuid::new_v4().to_string();
    let pseud = uuid::Uuid::new_v4().to_string();
    let t0 = "2026-03-01T00:00:00Z";
    let db = h.tdb.db();
    use darkhold_db::Backend;
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?, ?, ?, ?)")
                .bind(&account)
                .bind(format!("{account}@example.invalid"))
                .bind(t0)
                .bind(t0)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("account");
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&pseud)
            .bind(&account)
            .bind("author")
            .bind("author")
            .bind(t0)
            .bind(t0)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("pseud");
            for (count, completion) in completion_counts {
                for _ in 0..*count {
                    sqlx::query(
                        "INSERT INTO works (id, owner_pseud_id, title, completion, created_at,
                                            updated_at, generated_content_posture)
                         VALUES (?, ?, 'A work', ?, ?, ?, 'forbid')",
                    )
                    .bind(uuid::Uuid::new_v4().to_string())
                    .bind(&pseud)
                    .bind(*completion)
                    .bind(t0)
                    .bind(t0)
                    .execute(db.sqlite_pool().expect("sqlite"))
                    .await
                    .expect("work");
                }
            }
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO accounts (id, email, created_at, updated_at)
                 VALUES ($1::uuid, $2, $3::timestamptz, $3::timestamptz)",
            )
            .bind(uuid::Uuid::parse_str(&account).expect("uuid"))
            .bind(format!("{account}@example.invalid"))
            .bind(t0)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("account");
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES ($1::uuid, $2::uuid, 'author', 'author', $3::timestamptz, $3::timestamptz)",
            )
            .bind(uuid::Uuid::parse_str(&pseud).expect("uuid"))
            .bind(uuid::Uuid::parse_str(&account).expect("uuid"))
            .bind(t0)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("pseud");
            for (count, completion) in completion_counts {
                for _ in 0..*count {
                    sqlx::query(
                        "INSERT INTO works (id, owner_pseud_id, title, completion, created_at,
                                            updated_at, generated_content_posture)
                         VALUES ($1::uuid, $2::uuid, 'A work', $3, $4::timestamptz, $4::timestamptz, 'forbid')",
                    )
                    .bind(uuid::Uuid::new_v4())
                    .bind(uuid::Uuid::parse_str(&pseud).expect("uuid"))
                    .bind(*completion)
                    .bind(t0)
                    .execute(db.postgres_pool().expect("postgres"))
                    .await
                    .expect("work");
                }
            }
        }
    }
    pseud
}

#[tokio::test]
async fn the_module_ships_off_and_an_operator_enables_it() {
    let off = Harness::new("off", false).await;
    let (status, body) = off
        .get(&format!("/api/v1/works/{}/cadence", uuid::Uuid::new_v4()))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"]["code"], json!("MODULE_UNAVAILABLE"), "{body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .expect("a message")
            .contains("finishers"),
        "the refusal names the module: {body}"
    );

    let on = Harness::new("on", true).await;
    let (status, body) = on
        .get(&format!("/api/v1/works/{}/cadence", uuid::Uuid::new_v4()))
        .await;
    assert_eq!(status, StatusCode::OK, "enabled, both doors are public: {body}");
}

#[tokio::test]
async fn the_registry_row_is_present_off_by_default_and_carries_no_nav() {
    let h = Harness::new("registry", false).await;
    let (status, body) = h.get("/api/v1/modules").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let row = body["modules"]
        .as_array()
        .expect("modules")
        .iter()
        .find(|m| m["id"] == json!("finishers"))
        .unwrap_or_else(|| panic!("finishers is not registered"));
    assert_eq!(row["enabled"], json!(false), "the review says off by default");
    assert_eq!(row["nav"], json!([]));
}

#[tokio::test]
async fn an_unknown_author_is_404_and_an_author_with_nothing_is_zeroes() {
    let h = Harness::new("author", true).await;
    let (status, body) = h
        .get(&format!("/api/v1/authors/{}/finisher-signals", uuid::Uuid::new_v4()))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let pseud = author_with_works(&h, &[]).await;
    let (status, body) = h
        .get(&format!("/api/v1/authors/{pseud}/finisher-signals"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["finished"], json!(0));
    assert_eq!(body["concluded"], json!(0));
    assert_eq!(body["summary"], json!("no concluded works yet"));
}

#[tokio::test]
async fn the_payload_carries_counts_but_never_a_label() {
    let h = Harness::new("framing", true).await;
    let pseud = author_with_works(&h, &[(9, "complete"), (1, "abandoned"), (4, "in_progress")]).await;
    let (status, body) = h
        .get(&format!("/api/v1/authors/{pseud}/finisher-signals"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    assert_eq!(body["finished"], json!(9));
    assert_eq!(body["concluded"], json!(10));
    assert_eq!(body["summary"], json!("finished 9 of 10 concluded works"));

    // §60's rule, asserted against the JSON that actually goes out.
    let text = body.to_string().to_lowercase();
    for word in darkhold_domain::finishers::NEGATIVE_WORDS {
        assert!(
            !text.contains(word),
            "the payload carries the negative word {word:?}: {body}"
        );
    }
    for key in ["abandoned", "unfinished", "incomplete", "failed", "stalled"] {
        assert!(
            body.get(key).is_none(),
            "a top-level key named {key:?} is a label: {body}"
        );
    }
}

#[tokio::test]
async fn cadence_for_a_work_with_no_revisions_is_unmeasured_with_a_reason() {
    let h = Harness::new("cadence", true).await;
    let (status, body) = h
        .get(&format!("/api/v1/works/{}/cadence", uuid::Uuid::new_v4()))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["measurable"], json!(false), "{body}");
    let reason = body["reason"].as_str().expect("a reason");
    assert!(
        darkhold_domain::finishers::is_positive(reason),
        "even a refusal is phrased positively: {reason:?}"
    );
}

#[tokio::test]
async fn an_authors_cadence_is_unmeasured_until_a_work_spans_two_days() {
    let h = Harness::new("authorcadence", true).await;
    let pseud = author_with_works(&h, &[(2, "complete")]).await;
    let (status, body) = h
        .get(&format!("/api/v1/authors/{pseud}/finisher-signals"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["cadence"]["measurable"],
        json!(false),
        "works with no revisions cannot have a cadence: {body}"
    );
}
