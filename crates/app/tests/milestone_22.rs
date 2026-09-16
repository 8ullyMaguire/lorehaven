//! Milestone 22 — contract tests for the generalized media skeleton
//! (spec §32; migration 0024 + `/api/v1` media doors).
//!
//! The skeleton ships migration 0024, domain rule modules, and API contract
//! routes that return 501. These tests pin the CONTRACT so the implementing
//! agent can fill bodies without reshaping them: when a body is implemented,
//! its 501 assertion is replaced by the behavior test (the M21 pattern).
//!
//! Contract being pinned:
//! - migration 0024 creates the media entity tables on BOTH dialects and
//!   adds `works.format` defaulting to `prose`;
//! - every read door answers 501 on the Default rate class;
//! - every write door answers 501 on the Write rate class (sessions and
//!   scopes arrive with the bodies — the 501s move first).

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::{Backend, DatabaseConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness — the same shape as the other milestone tests.
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m22-{}-{:?}-{:?}",
        tag,
        std::process::id(),
        std::thread::current().id(),
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

struct Client {
    app: axum::Router,
}

impl Client {
    fn new(app: axum::Router) -> Self {
        Self { app }
    }
    async fn send(&mut self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let builder = Request::builder().method(method).uri(uri);
        let request = match body {
            Some(v) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&v).expect("serialise")))
                .expect("request"),
            None => builder.body(Body::empty()).expect("request"),
        };
        let response = self.app.clone().oneshot(request).await.expect("response");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024)
            .await
            .expect("body");
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, value)
    }
    async fn get(&mut self, uri: &str) -> (StatusCode, Value) {
        self.send("GET", uri, None).await
    }
    async fn post(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.send("POST", uri, Some(body)).await
    }
    async fn patch(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.send("PATCH", uri, Some(body)).await
    }
    async fn put(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.send("PUT", uri, Some(body)).await
    }
}

struct Fixture {
    dir: PathBuf,
    tdb: test_support::TestDb,
}

impl Fixture {
    async fn new(tag: &str) -> Self {
        set_trust_proxy(false);
        let _ = lorehaven_app::logging::init(&lorehaven_app::config::LoggingConfig {
            filter: "error".to_owned(),
            format: lorehaven_app::config::LogFormat::Pretty,
        });
        let dir = scratch_dir(tag);
        let tdb = test_support::TestDb::connect_with_dir(tag, &dir).await;
        assert!(
            tdb.applied_migrations()
                .iter()
                .any(|id| id.contains("media_generalization")),
            "the 0024 media-generalization migration must be part of the catalogue"
        );
        Self { dir, tdb }
    }
    fn client(&self) -> Client {
        Client::new(server::build_router(AppState::new(
            config_for(&self.dir),
            self.tdb.db().clone(),
        )))
    }
    async fn cleanup(self) {
        self.tdb.cleanup().await;
        let _ = std::fs::remove_dir_all(self.dir);
    }
}

// ---------------------------------------------------------------------------
// Migration 0024: the media entity tables exist on both dialects, and
// `works` carries `format` defaulting to `prose`.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn migration_0024_creates_the_media_entity_tables() {
    let fx = Fixture::new("tables").await;
    {
        let db = fx.tdb.db();
        let table_list: &[&str] = &[
            "creators",
            "media_creators",
            "distributors",
            "distributorships",
            "media_collections",
            "media_collection_items",
            "media_editions",
            "media_rights",
            "quality_signals",
        ];
        let table_exists_sql = db.sql(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?",
            "SELECT COUNT(*) FROM pg_tables WHERE tablename = ?",
        );
        for &table in table_list {
            let n: i64 = match db.backend() {
                Backend::Sqlite => sqlx::query_scalar(table_exists_sql.as_ref())
                    .bind(table)
                    .fetch_one(db.sqlite_pool().expect("sqlite"))
                    .await
                    .expect("table existence query"),
                Backend::Postgres => sqlx::query_scalar(table_exists_sql.as_ref())
                    .bind(table)
                    .fetch_one(db.postgres_pool().expect("pg"))
                    .await
                    .expect("table existence query"),
            };
            assert_eq!(n, 1, "table {table} must exist after 0024");
        }

        let cols: Vec<String> = match db.backend() {
            Backend::Sqlite => sqlx::query_scalar(
                db.sql(
                    "SELECT name FROM pragma_table_info('works')",
                    "SELECT column_name FROM information_schema.columns WHERE table_name = 'works' ORDER BY ordinal_position",
                )
                .as_ref(),
            )
            .fetch_all(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("works columns"),
            Backend::Postgres => sqlx::query_scalar(
                db.sql(
                    "SELECT name FROM pragma_table_info('works')",
                    "SELECT column_name FROM information_schema.columns WHERE table_name = 'works' ORDER BY ordinal_position",
                )
                .as_ref(),
            )
            .fetch_all(db.postgres_pool().expect("pg"))
            .await
            .expect("works columns"),
        };
        assert!(
            cols.iter().any(|name| name == "format"),
            "works must gain the format column after 0024"
        );
    }
    fx.cleanup().await;
}

// ---------------------------------------------------------------------------
// The read doors: every one answers 501 with the house error shape.
// ---------------------------------------------------------------------------

const READ_DOORS: &[&str] = &[
    "/api/v1/media",
    "/api/v1/media/00000000-0000-0000-0000-000000000001",
    "/api/v1/media/00000000-0000-0000-0000-000000000001/files",
    "/api/v1/media/00000000-0000-0000-0000-000000000001/editions",
    "/api/v1/creators",
    "/api/v1/creators/00000000-0000-0000-0000-000000000002",
    "/api/v1/creators/00000000-0000-0000-0000-000000000002/media",
    "/api/v1/distributors",
    "/api/v1/distributors/00000000-0000-0000-0000-000000000003",
    "/api/v1/distributors/00000000-0000-0000-0000-000000000003/media",
    "/api/v1/media-collections",
    "/api/v1/media-collections/00000000-0000-0000-0000-000000000004",
    "/api/v1/media-collections/00000000-0000-0000-0000-000000000004/media",
    "/api/v1/canons/00000000-0000-0000-0000-000000000005/media",
    "/api/v1/spaces/00000000-0000-0000-0000-000000000006/media",
];

#[tokio::test]
async fn every_media_read_door_is_a_contract_stub() {
    let fx = Fixture::new("read-doors").await;
    let mut client = fx.client();
    for door in READ_DOORS {
        let (status, body) = client.get(door).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "GET {door}: {body}");
        assert_eq!(
            body["error"]["code"],
            json!("NOT_IMPLEMENTED"),
            "GET {door} must carry the house error shape"
        );
    }
    fx.cleanup().await;
}

// ---------------------------------------------------------------------------
// The write doors: 501 on the Write rate class; sessions and scopes arrive
// with the bodies, and the 501 assertions move then.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn every_media_write_door_is_a_contract_stub() {
    let fx = Fixture::new("write-doors").await;
    let mut client = fx.client();
    let empty = json!({});
    let doors: &[(&str, &str)] = &[
        ("POST", "/api/v1/media/query"),
        ("POST", "/api/v1/creators"),
        (
            "PATCH",
            "/api/v1/creators/00000000-0000-0000-0000-000000000002",
        ),
        ("POST", "/api/v1/distributors"),
        ("POST", "/api/v1/media-collections"),
        (
            "PUT",
            "/api/v1/media-collections/00000000-0000-0000-0000-000000000004",
        ),
    ];
    for (method, door) in doors {
        let (status, body) = match *method {
            "POST" => client.post(door, empty.clone()).await,
            "PATCH" => client.patch(door, empty.clone()).await,
            "PUT" => client.put(door, empty.clone()).await,
            _ => unreachable!("table only carries POST, PATCH and PUT"),
        };
        assert_eq!(
            status,
            StatusCode::NOT_IMPLEMENTED,
            "{method} {door}: {body}"
        );
        assert_eq!(
            body["error"]["code"],
            json!("NOT_IMPLEMENTED"),
            "{method} {door} must carry the house error shape"
        );
    }
    fx.cleanup().await;
}
