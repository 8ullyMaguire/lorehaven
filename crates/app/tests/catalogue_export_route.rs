//! `GET /api/v1/catalogue/export.sqlite` over real HTTP.
//!
//! The builder is proved in `crates/db/tests/catalogue_export.rs`; this file
//! proves only what the route can get wrong, and each of the four is invisible to
//! a store test:
//!
//!   * **the file is actually served**, with a `content-type` a download manager
//!     respects and a `content-disposition` that names it, rather than a JSON body
//!     the browser renders as text;
//!   * **the manifest and the download agree** — the fingerprint in the ETag is
//!     the manifest's, so a client can tell a stale copy from a current one;
//!   * **the snapshot is cached, not rebuilt per request** — the second GET must
//!     not produce a second file;
//!   * **nothing from the private side travels.** The seeded account id is the
//!     sharpest possible probe: it appears in `library_items.account_id`, so if
//!     the export ever selected the whole table its bytes would contain it. This
//!     asserts against the downloaded *bytes*, which is what a reader would
//!     actually receive.
//!
//! The `Client`/`Harness` pair is copied from `blind_date_route.rs`, which copied
//! it from `milestone_16.rs`: a shared HTTP harness would be a refactor of ten
//! test binaries for four assertions.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
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
        let app = server::build_router(AppState::new(
            config_for(&dir),
            tdb.db().clone(),
        ));
        Self { dir, tdb, app }
    }

    async fn raw(&self, uri: &str) -> axum::response::Response {
        let request = Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .expect("request");
        self.app.clone().oneshot(request).await.expect("response")
    }

    async fn json(&self, uri: &str) -> (StatusCode, Value) {
        let response = self.raw(uri).await;
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .expect("body");
        let value = serde_json::from_slice(&bytes).expect("json body");
        (status, value)
    }

    async fn cleanup(self) {
        self.tdb.cleanup().await;
        let _ = std::fs::remove_dir_all(self.dir);
    }
}

/// One account holding three catalogue rows across two sources.
async fn seed(tdb: &test_support::TestDb) -> String {
    let account = uuid::Uuid::new_v4().to_string();
    exec_with(
        tdb,
        "INSERT INTO accounts (id, email, created_at, updated_at) \
         VALUES (?1#u, ?2, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        &[&account, "catalogue-export-route@example.com"],
    )
    .await;

    for (source, key, title, likes) in [
        ("sb", "sf-1", "Winter's Wake", "7"),
        ("sv", "sv-1", "Winter's Wake", "11"),
        ("qq", "qq-1", "A Different Story", "3"),
    ] {
        exec_with(
            tdb,
            "INSERT INTO library_items \
             (id, account_id, source_key, source_work_key, title, author_text, summary, \
              word_count, status, source_url, provenance_json, created_at, updated_at, likes) \
             VALUES (?1#u, ?2#u, ?3, ?4, ?5, 'Vesna', '', 42000, 'ongoing', \
                     'https://example.invalid/x', '{}', '2026-01-01T00:00:00Z', \
                     '2026-10-30T12:00:00Z', ?6)",
            &[
                &uuid::Uuid::new_v4().to_string(),
                &account,
                source,
                key,
                title,
                likes,
            ],
        )
        .await;
    }
    account
}

#[tokio::test]
async fn the_manifest_describes_the_file_before_anyone_downloads_it() {
    let h = Harness::new("cexport-manifest").await;
    seed(&h.tdb).await;

    let (status, body) = h.json("/api/v1/catalogue/export").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["rows"], 3, "three catalogue entries");
    assert_eq!(body["works"], 2, "two distinct stories");
    assert_eq!(body["sources"], 3, "sb, sv, qq");
    assert_eq!(body["url"], "/api/v1/catalogue/export.sqlite");
    assert_eq!(body["tables"][0]["name"], "works");
    assert!(
        body["privacy"].as_str().unwrap_or_default().contains("No account"),
        "the manifest states the boundary: {}",
        body["privacy"]
    );
    assert!(
        body["examples"].as_array().map(|a| a.len()).unwrap_or(0) >= 3,
        "the manifest ships example queries, which is the point of the download"
    );
    assert_eq!(
        body["fingerprint"].as_str().map(str::len),
        Some(64),
        "a hex sha256 the client can compare against the ETag"
    );

    h.cleanup().await;
}

#[tokio::test]
async fn the_download_is_a_cached_sqlite_file_that_carries_no_account() {
    let h = Harness::new("cexport-file").await;
    let account = seed(&h.tdb).await;

    let (_, manifest) = h.json("/api/v1/catalogue/export").await;
    let fingerprint = manifest["fingerprint"].as_str().expect("fingerprint").to_owned();

    let response = h.raw("/api/v1/catalogue/export.sqlite").await;
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers().clone();
    assert_eq!(
        headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()),
        Some("application/vnd.sqlite3"),
        "a download manager must see a database, not octet-stream"
    );
    let disposition = headers
        .get(header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(
        disposition.starts_with("attachment;") && disposition.contains(".sqlite"),
        "disposition names the file: {disposition}"
    );
    assert_eq!(
        headers.get(header::ETAG).and_then(|v| v.to_str().ok()),
        Some(fingerprint.as_str()),
        "the ETag is the catalogue fingerprint the manifest reported"
    );
    assert_eq!(
        headers.get("x-catalogue-rows").and_then(|v| v.to_str().ok()),
        Some("3")
    );

    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024 * 1024)
        .await
        .expect("body");
    assert_eq!(&bytes[..16], b"SQLite format 3\0", "a real sqlite file");

    // The sharpest probe available: the account id is in library_items, so if the
    // export ever copied the table wholesale, these bytes would contain it.
    let haystack = String::from_utf8_lossy(&bytes);
    assert!(
        !haystack.contains(&account),
        "the downloaded file contains the seeded account id"
    );
    assert!(
        !haystack.contains("catalogue-export-route@example.com"),
        "the downloaded file contains the seeded account's email"
    );

    // Cached: the second GET writes no second snapshot.
    let store = h.dir.join("catalogue-export");
    let count_sqlite = || {
        std::fs::read_dir(&store)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| {
                        e.path().extension().and_then(|x| x.to_str()) == Some("sqlite")
                    })
                    .count()
            })
            .unwrap_or(0)
    };
    assert_eq!(count_sqlite(), 1, "one snapshot for one fingerprint");
    let again = h.raw("/api/v1/catalogue/export.sqlite").await;
    assert_eq!(again.status(), StatusCode::OK);
    let _ = axum::body::to_bytes(again.into_body(), 64 * 1024 * 1024)
        .await
        .expect("body");
    assert_eq!(count_sqlite(), 1, "the second request reused the snapshot");

    h.cleanup().await;
}

/// Insert with the same dialect helper the other route suites use: `?N#u` marks a
/// uuid slot and `?N#i` a bigint, so one statement serves both engines.
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
            let pg = (1..=8).fold(sqlite.to_string(), |acc, n| {
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
