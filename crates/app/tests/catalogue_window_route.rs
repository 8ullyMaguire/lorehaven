//! `GET /api/v1/catalogue?updated=…` over real HTTP.
//!
//! The grammar is unit-tested in `crates/domain/src/window.rs` and the predicates
//! are proved against both engines in `crates/db/tests/catalogue_window.rs`. What
//! only the route can get wrong is the wiring: a handler that accepts the
//! parameter and forgets to pass it to the store passes every test above, because
//! the store is never asked. So this file asserts that the parameter **changes the
//! result set**, and that a malformed window is a 422 naming the field rather
//! than a silent "no filter".
//!
//! The fixture uses absolute dates, not relative ones: a route test that reads the
//! wall clock fails on a Sunday in a month with 31 days.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use darkhold_app::config::Config;
use darkhold_app::server::{self, set_trust_proxy};
use darkhold_app::state::AppState;
use darkhold_db::DatabaseConfig;
use serde_json::Value;
use test_support::scratch_dir;
use tower::ServiceExt;

fn config_for(dir: &Path) -> Config {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    config.database = DatabaseConfig::new(format!(
        "sqlite://{}/darkhold.sqlite?mode=rwc",
        dir.display()
    ));
    config.rate_limits.auth = darkhold_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.write = darkhold_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.search = darkhold_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.default = darkhold_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config
}

struct Harness {
    dir: PathBuf,
    tdb: test_support::TestDb,
    app: axum::Router,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        set_trust_proxy(false);
        let _ = darkhold_app::logging::init(&darkhold_app::config::LoggingConfig {
            filter: "error".to_owned(),
            format: darkhold_app::config::LogFormat::Pretty,
        });
        let dir = scratch_dir(tag);
        let tdb = test_support::TestDb::connect_with_dir(tag, &dir).await;
        let app = server::build_router(AppState::new(config_for(&dir), tdb.db().clone()));
        Self { dir, tdb, app }
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
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, value)
    }

    async fn cleanup(self) {
        self.tdb.cleanup().await;
        let _ = std::fs::remove_dir_all(self.dir);
    }
}

/// Two rows: one with a 2020 source clock, one with a 2026 one.
async fn seed(tdb: &test_support::TestDb) {
    let account = uuid::Uuid::new_v4().to_string();
    exec_with(
        tdb,
        "INSERT INTO accounts (id, email, created_at, updated_at) \
         VALUES (?1#u, 'window-route@example.com', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        &[&account],
    )
    .await;
    // Canonical shape, because migration 0132 makes the database refuse any other: the
    // fixtures used to carry a bare date here, which is exactly what 0132 backfilled away.
    for (key, source_updated) in [
        ("old-story", "2020-06-01T00:00:00Z"),
        ("new-story", "2026-10-30T00:00:00Z"),
    ] {
        exec_with(
            tdb,
            "INSERT INTO library_items \
             (id, account_id, source_key, source_work_key, title, author_text, summary, \
              word_count, status, source_url, source_updated_at, last_synced_at, created_at, \
              updated_at, likes) \
             VALUES (?1#u, ?2#u, 'sb', ?3, ?3, 'A', '', 1000, 'ongoing', \
                     'https://example.invalid/x', ?4, '2026-10-30T00:00:00Z', \
                     '2026-10-30T00:00:00Z', '2026-10-30T00:00:00Z', 5)",
            &[&uuid::Uuid::new_v4().to_string(), &account, key, source_updated],
        )
        .await;
    }
}

async fn keys(body: &Value) -> Vec<String> {
    let mut out: Vec<String> = body["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| item["source_work_key"].as_str().unwrap_or("?").to_owned())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

#[tokio::test]
async fn the_window_parameter_actually_filters() {
    let h = Harness::new("window-route").await;
    seed(&h.tdb).await;

    // No window: both rows. This is the control — without it, a handler that
    // dropped every row would also "filter".
    let (status, all) = h.get("/api/v1/catalogue").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(keys(&all).await, vec!["new-story", "old-story"]);

    // An absolute range selects the 2020 row alone.
    let (status, old) = h.get("/api/v1/catalogue?updated=2020-01-01..2021-01-01").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(keys(&old).await, vec!["old-story"], "{old}");

    // A single day selects the 2026 row alone: a bound is a day window, and the stored
    // instant at its midnight is inside it (half-open, so the next day's midnight is not).
    let (status, new) = h.get("/api/v1/catalogue?updated=@2026-10-30").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(keys(&new).await, vec!["new-story"], "{new}");

    // An open-ended range: everything from 2026 onward on the source clock.
    // (`>` is only defined as a standalone "older than" window; inside a range the
    // left side is the bound, not a comparison operator.)
    let (status, open) = h.get("/api/v1/catalogue?updated=2026-01-01..").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(keys(&open).await, vec!["new-story"], "{open}");

    h.cleanup().await;
}

#[tokio::test]
async fn a_malformed_window_is_a_422_naming_the_field() {
    let h = Harness::new("window-route-bad").await;
    seed(&h.tdb).await;

    for (query, field) in [
        ("updated=7q", "updated"),
        ("refreshed=soon", "refreshed"),
        ("added=1d..7d", "added"),
    ] {
        let (status, body) = h.get(&format!("/api/v1/catalogue?{query}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}: {body}");
        let message = body["error"]["message"]
            .as_str()
            .or_else(|| body["message"].as_str())
            .unwrap_or_else(|| panic!("no message in {body}"));
        assert!(
            message.contains(field),
            "the error must name the field it came from: {message}"
        );
    }

    h.cleanup().await;
}

async fn exec_with(tdb: &test_support::TestDb, sqlite: &str, args: &[&str]) {
    let db = tdb.db();
    match db.backend() {
        darkhold_db::Backend::Sqlite => {
            let sql = sqlite.replace("#u", "").replace("#i", "");
            let mut q = sqlx::query(&sql);
            for a in args {
                q = q.bind(*a);
            }
            q.execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("fixture insert");
        }
        darkhold_db::Backend::Postgres => {
            let pg = (1..=9).fold(sqlite.to_string(), |acc, n| {
                acc.replace(&format!("?{n}#u"), &format!("${n}::uuid"))
                    .replace(&format!("?{n}#i"), &format!("${n}::bigint"))
                    .replace(&format!("?{n}"), &format!("${n}"))
            });
            let mut q = sqlx::query(&pg);
            for a in args {
                match uuid::Uuid::parse_str(a) {
                    Ok(u) => q = q.bind(u),
                    Err(_) => q = q.bind(*a),
                }
            }
            q.execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("fixture insert");
        }
    }
}
