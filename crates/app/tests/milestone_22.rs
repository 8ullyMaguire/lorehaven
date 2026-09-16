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
    cookies: Vec<(String, String)>,
}

impl Client {
    fn new(app: axum::Router) -> Self {
        Self {
            app,
            cookies: Vec::new(),
        }
    }
    fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
    fn capture(&mut self, response: &axum::response::Response) {
        for value in response.headers().get_all(header::SET_COOKIE) {
            let Ok(text) = value.to_str() else {
                continue;
            };
            let Some((pair, _)) = text.split_once(';') else {
                continue;
            };
            if let Some((name, value)) = pair.split_once('=') {
                let name = name.trim().to_owned();
                let value = value.trim().to_owned();
                self.cookies.retain(|(k, _)| k != &name);
                if !value.is_empty() {
                    self.cookies.push((name, value));
                }
            }
        }
    }
    async fn send(&mut self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if !self.cookies.is_empty() {
            builder = builder.header(
                header::COOKIE,
                self.cookies
                    .iter()
                    .map(|(n, v)| format!("{n}={v}"))
                    .collect::<Vec<_>>()
                    .join("; "),
            );
        }
        if !matches!(method, "GET" | "HEAD" | "OPTIONS") {
            if let Some(token) = self.cookie("lorehaven_csrf").map(str::to_owned) {
                builder = builder.header("x-csrf-token", token);
            }
        }
        let request = match body {
            Some(v) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&v).expect("serialise")))
                .expect("request"),
            None => builder.body(Body::empty()).expect("request"),
        };
        let response = self.app.clone().oneshot(request).await.expect("response");
        self.capture(&response);
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
    /// Raw-body GET for non-JSON doors (the Atom feed).
    async fn get_raw(&mut self, uri: &str) -> (StatusCode, String) {
        let request = Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .expect("request");
        let response = self.app.clone().oneshot(request).await.expect("response");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024)
            .await
            .expect("body");
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }
    async fn conditional_get(&self, uri: &str, etag: Option<&str>) -> axum::response::Response {
        let mut request = Request::builder().uri(uri);
        if let Some(etag) = etag {
            request = request.header(header::IF_NONE_MATCH, etag);
        }
        self.app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap()
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
        assert!(
            tdb.applied_migrations()
                .iter()
                .any(|id| id.contains("canon_space")),
            "the 0026 canon_space migration must be part of the catalogue"
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

// Implemented read doors now have behavior tests below.
// These remain as 501 contract stubs:
const READ_DOORS_STILL_501: &[&str] = &[];

#[tokio::test]
async fn unimplemented_read_doors_still_return_501() {
    let fx = Fixture::new("read-doors").await;
    let mut client = fx.client();
    for door in READ_DOORS_STILL_501 {
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
// The write doors: sessions are required; API-scope enforcement is the
// M23 remainder.
// ---------------------------------------------------------------------------

// The write doors all require a session (RequireSession). No write door
// has API-scope enforcement yet — that is the M23 remainder.
#[tokio::test]
async fn write_doors_require_a_session() {
    let fx = Fixture::new("write-auth").await;
    let mut client = fx.client();
    let empty = json!({});
    // POST /api/v1/media/query is a read (complex-query listing) and is
    // anonymous-allowed, so it is not in this list.
    let doors: &[(&str, &str)] = &[
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
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {door}: {body}");
    }
    fx.cleanup().await;
}

// ---------------------------------------------------------------------------
// Behavior: eligibility, pagination, and feed escaping, exercised through
// the doors with seeded works (ADR 0002 vocabulary, ADR 0003 ownership).
// ---------------------------------------------------------------------------

const PASSWORD: &str = "a-long-enough-passphrase";

async fn register(client: &mut Client, email: &str, handle: &str) {
    let (status, body) = client
        .post(
            "/api/v1/auth/register",
            json!({
                "email": email,
                "password": PASSWORD,
                "handle": handle,
                "display_name": handle,
                "age_band": "adult"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "register {handle}: {body}");
}

/// One scalar (first row, first column) for test seeding/lookups.
async fn scalar(fx: &Fixture, sql: &str, bind: &str) -> String {
    let sql = fx.tdb.sql(sql);
    let db = fx.tdb.db();
    match db.backend() {
        Backend::Sqlite => sqlx::query_scalar(&sql)
            .bind(bind)
            .fetch_one(db.sqlite_pool().expect("sqlite pool"))
            .await
            .expect("scalar"),
        Backend::Postgres => sqlx::query_scalar(&sql)
            .bind(bind)
            .fetch_one(db.postgres_pool().expect("postgres pool"))
            .await
            .expect("scalar"),
    }
}

async fn author_pseud_id(fx: &Fixture, email: &str) -> String {
    scalar(
        fx,
        "SELECT CAST(p.id AS TEXT) FROM pseuds p \
         JOIN accounts a ON a.id = p.account_id WHERE a.email = ?",
        email,
    )
    .await
}

/// Seed one work row directly. Deterministic ids; RFC3339 with a Z offset
/// so a cursor never carries a character that URL-decoding would mangle.
async fn seed_work(
    fx: &Fixture,
    n: u32,
    pseud_id: &str,
    title: &str,
    visibility: &str,
    lifecycle: &str,
    created_at: &str,
) -> String {
    let id = format!("00000000-0000-0000-0000-{n:012}");
    let cast = if fx.tdb.is_postgres() { "::uuid" } else { "" };
    let sql = fx.tdb.sql(&format!(
        "INSERT INTO works (id, owner_pseud_id, title, summary, visibility, lifecycle, \
         created_at, updated_at, version) VALUES (?{cast}, ?{cast}, ?, '', ?, ?, ?, ?, 1)"
    ));
    let db = fx.tdb.db();
    let result = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(&id)
            .bind(pseud_id)
            .bind(title)
            .bind(visibility)
            .bind(lifecycle)
            .bind(created_at)
            .bind(created_at)
            .execute(db.sqlite_pool().expect("sqlite pool"))
            .await
            .map(|_| ()),
        Backend::Postgres => sqlx::query(&sql)
            .bind(&id)
            .bind(pseud_id)
            .bind(title)
            .bind(visibility)
            .bind(lifecycle)
            .bind(created_at)
            .bind(created_at)
            .execute(db.postgres_pool().expect("postgres pool"))
            .await
            .map(|_| ()),
    };
    result.expect("seed work");
    id
}

/// Seed one media_files row for a work (migration 0025).
async fn seed_file(fx: &Fixture, work_id: &str, mime_type: &str) {
    let cast = if fx.tdb.is_postgres() { "::uuid" } else { "" };
    let sql = fx.tdb.sql(&format!(
        "INSERT INTO media_files (id, work_id, edition_kind, url, mime_type, created_at, updated_at, version) \
         VALUES (?{cast}, ?{cast}, 'prose', 'https://files.example.test/a.epub', ?, \
                 '2026-09-01T00:03:00Z', '2026-09-01T00:03:00Z', 1)"
    ));
    let db = fx.tdb.db();
    let result = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(derived_id(work_id, "10000000"))
            .bind(work_id)
            .bind(mime_type)
            .execute(db.sqlite_pool().expect("sqlite pool"))
            .await
            .map(|_| ()),
        Backend::Postgres => sqlx::query(&sql)
            .bind(derived_id(work_id, "10000000"))
            .bind(work_id)
            .bind(mime_type)
            .execute(db.postgres_pool().expect("postgres pool"))
            .await
            .map(|_| ()),
    };
    result.expect("seed media file");
}

/// Seed one media_editions row for a work (migration 0024).
async fn seed_edition(fx: &Fixture, work_id: &str) {
    let cast = if fx.tdb.is_postgres() { "::uuid" } else { "" };
    let sql = fx.tdb.sql(&format!(
        "INSERT INTO media_editions (id, work_id, edition_kind, label, created_at, updated_at, version) \
         VALUES (?{cast}, ?{cast}, 'revised', 'Second edition', \
                 '2026-09-01T00:04:00Z', '2026-09-01T00:04:00Z', 1)"
    ));
    let db = fx.tdb.db();
    let result = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(derived_id(work_id, "20000000"))
            .bind(work_id)
            .execute(db.sqlite_pool().expect("sqlite pool"))
            .await
            .map(|_| ()),
        Backend::Postgres => sqlx::query(&sql)
            .bind(derived_id(work_id, "20000000"))
            .bind(work_id)
            .execute(db.postgres_pool().expect("postgres pool"))
            .await
            .map(|_| ()),
    };
    result.expect("seed media edition");
}

/// Deterministic file/edition ids derived from the seeded work id: swap
/// the work uuid's first group for `10…`/`20…`, keeping valid uuid shape
/// on both dialects and avoiding collisions.
fn derived_id(work_id: &str, group: &str) -> String {
    format!("{group}-{}", &work_id[9..])
}

#[tokio::test]
async fn attributed_media_doors_decode_seeded_rows() {
    let fx = Fixture::new("attributed-media").await;
    let mut owner = fx.client();
    register(&mut owner, "attributed@example.com", "attributedauthor").await;
    let pseud = author_pseud_id(&fx, "attributed@example.com").await;
    let work = seed_work(
        &fx,
        61,
        &pseud,
        "Attributed",
        "public",
        "published",
        "2026-09-01T00:00:00Z",
    )
    .await;
    let mut anon = fx.client();
    let creator = "00000000-0000-0000-0000-000000000062";
    let distributor = "00000000-0000-0000-0000-000000000063";
    let statements = [
        ("INSERT INTO creators (id, kind, display_name, created_at, updated_at) VALUES (?, 'external', 'External Author', '2026-09-01', '2026-09-01')", vec![creator.to_owned()]),
        ("INSERT INTO distributors (id, name, kind, created_at, updated_at) VALUES (?, 'Archive', 'archive', '2026-09-01', '2026-09-01')", vec![distributor.to_owned()]),
        ("INSERT INTO media_creators (id, work_id, creator_id, role, created_at) VALUES (?, ?, ?, 'author', '2026-09-01')", vec![derived_id(&work, "40000000"), work.clone(), creator.to_owned()]),
        ("INSERT INTO distributorships (id, work_id, distributor_id, role, created_at) VALUES (?, ?, ?, 'hosted', '2026-09-01')", vec![derived_id(&work, "50000000"), work.clone(), distributor.to_owned()]),
    ];
    for (statement, binds) in statements {
        let sql = fx.tdb.sql(&if fx.tdb.is_postgres() {
            statement.replace("?", "?::uuid")
        } else {
            statement.to_owned()
        });
        match fx.tdb.db().backend() {
            Backend::Sqlite => {
                let mut q = sqlx::query(&sql);
                for v in &binds {
                    q = q.bind(v);
                }
                q.execute(fx.tdb.db().sqlite_pool().unwrap()).await.unwrap();
            }
            Backend::Postgres => {
                let mut q = sqlx::query(&sql);
                for v in &binds {
                    q = q.bind(v);
                }
                q.execute(fx.tdb.db().postgres_pool().unwrap())
                    .await
                    .unwrap();
            }
        }
    }
    for uri in [
        "/api/v1/creators".to_owned(),
        format!("/api/v1/creators/{creator}"),
        "/api/v1/distributors".to_owned(),
        format!("/api/v1/distributors/{distributor}"),
    ] {
        let (status, body) = anon.get(&uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        assert!(!body.to_string().contains("api_key"));
    }
    let (status, created) = owner.post("/api/v1/creators", json!({"kind":"external", "display_name":"Created Author", "source_key":"fixture", "source_creator_id":"external-1"})).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let (status, created) = owner.post("/api/v1/distributors", json!({"kind":"archive", "name":"Created Archive", "source_key":"fixture-archive", "canonical_url":"https://example.com"})).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let (status, data) = anon
        .get(&format!(
            "/api/v1/distributors/{}",
            created["id"].as_str().unwrap()
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(data["source_key"], "fixture-archive");
    let (status, body) = owner
        .patch(
            &format!("/api/v1/creators/{creator}"),
            json!({"name":"Hijacked"}),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "untrusted actor changed shared attribution: {body}"
    );
    let account = lorehaven_db::media::find_media(fx.tdb.db(), &work)
        .await
        .unwrap()
        .unwrap()
        .owning_account_id;
    lorehaven_db::governance::set_trust(fx.tdb.db(), &account, 6, "{}")
        .await
        .unwrap();
    assert_eq!(
        owner
            .patch(
                &format!("/api/v1/creators/{creator}"),
                json!({"name":"Curated Author"})
            )
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        anon.get(&format!("/api/v1/creators/{creator}")).await.1["display_name"],
        "Curated Author"
    );
    let (status, collection) = owner
        .post(
            "/api/v1/media-collections",
            json!({"kind":"reading_list", "title":"Reading List", "visibility":"public"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{collection}");
    let collection = collection["id"].as_str().unwrap();
    let (status, body) = anon
        .get(&format!("/api/v1/media-collections/{collection}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["title"], "Reading List");
    for visibility in ["private", "unlisted", "restricted"] {
        let (status, created) = owner
            .post(
                "/api/v1/media-collections",
                json!({"kind":"reading_list", "title":visibility, "visibility":visibility}),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let id = created["id"].as_str().unwrap();
        assert_eq!(
            owner
                .get(&format!("/api/v1/media-collections/{id}"))
                .await
                .0,
            StatusCode::OK
        );
        let expected = if visibility == "unlisted" {
            StatusCode::OK
        } else {
            StatusCode::NOT_FOUND
        };
        assert_eq!(
            anon.get(&format!("/api/v1/media-collections/{id}")).await.0,
            expected
        );
    }
    let (status, body) = anon.get("/api/v1/media-collections").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    for uri in [
        format!("/api/v1/creators/{creator}/media"),
        format!("/api/v1/distributors/{distributor}/media"),
    ] {
        let (status, body) = anon.get(&uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        assert_eq!(body["items"][0]["id"], work, "{body}");
    }
    fx.cleanup().await;
}

#[tokio::test]
async fn media_tokens_enforce_scopes_and_revocation() {
    use lorehaven_domain::api_scopes::Scope;
    let fx = Fixture::new("media-scopes").await;
    let mut owner = fx.client();
    register(&mut owner, "scopes@example.com", "scopes").await;
    let pseud = author_pseud_id(&fx, "scopes@example.com").await;
    let draft = seed_work(
        &fx,
        11,
        &pseud,
        "Token draft",
        "private",
        "draft",
        "2026-09-01T00:00:00Z",
    )
    .await;
    let account = lorehaven_db::media::find_media(fx.tdb.db(), &draft)
        .await
        .unwrap()
        .unwrap()
        .owning_account_id;
    let raw = uuid::Uuid::new_v4().to_string();
    let read_token_id = lorehaven_db::external::issue_token(
        fx.tdb.db(),
        &account,
        "personal",
        "Reader",
        &lorehaven_app::crypto::hash_token(&raw),
        &[Scope::ContentRead],
    )
    .await
    .unwrap();

    // Direct-door read of a private draft via bearer token — owner's own work.
    let status = owner
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/media/{draft}"))
                .header(header::AUTHORIZATION, format!("Bearer {raw}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status();
    assert_eq!(status, StatusCode::OK, "bearer token reads own draft");

    // Same draft without a token — 404 (not public, do not reveal existence).
    let status = owner
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/media/{draft}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status();
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "anonymous cannot reach private draft"
    );

    // Write-class routes still require a session (CSRF-bound). A token alone is 401.
    let status = owner
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/media-collections")
                .header(header::AUTHORIZATION, format!("Bearer {raw}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"kind":"reading_list","title":"No write"}"#))
                .unwrap(),
        )
        .await
        .unwrap()
        .status();
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "write routes require a session"
    );

    // Revoked token loses access to the draft.
    lorehaven_db::external::revoke_token(fx.tdb.db(), &read_token_id)
        .await
        .unwrap();
    let status = owner
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/media/{draft}"))
                .header(header::AUTHORIZATION, format!("Bearer {raw}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status();
    assert_eq!(status, StatusCode::NOT_FOUND, "revoked token loses access");
}

#[tokio::test]
async fn anonymous_media_doors_exclude_explicit_content() {
    let fx = Fixture::new("media-anonymous-explicit").await;
    let mut owner = fx.client();
    register(&mut owner, "explicit-owner@example.com", "explicitowner").await;
    let pseud = author_pseud_id(&fx, "explicit-owner@example.com").await;
    let id = seed_work(
        &fx,
        71,
        &pseud,
        "Explicit private-to-adults title",
        "public",
        "published",
        "2026-09-01T00:00:00Z",
    )
    .await;
    let sql = fx
        .tdb
        .sql("UPDATE works SET rating = 'explicit' WHERE CAST(id AS TEXT) = ?");
    match fx.tdb.db().backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .execute(fx.tdb.db().sqlite_pool().unwrap())
                .await
                .unwrap();
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .execute(fx.tdb.db().postgres_pool().unwrap())
                .await
                .unwrap();
        }
    }
    let mut anon = fx.client();
    let (status, body) = anon.get(&format!("/api/v1/media/{id}")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "anonymous direct door leaked explicit content: {body}"
    );
    let (status, body) = anon.get("/api/v1/media").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["total"], 0,
        "anonymous list leaked explicit content: {body}"
    );
    for format in ["atom", "rss"] {
        let (status, body) = anon
            .get_raw(&format!("/api/v1/media/feed?format={format}"))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            !body.contains("Explicit private-to-adults title"),
            "anonymous {format} leaked: {body}"
        );
    }
    for door in ["files", "editions"] {
        assert_eq!(
            anon.get(&format!("/api/v1/media/{id}/{door}")).await.0,
            StatusCode::NOT_FOUND
        );
    }
    let mut reader = fx.client();
    register(&mut reader, "explicit-reader@example.com", "explicitreader").await;
    // Declaring adulthood does not opt into explicit discovery.
    assert_eq!(reader.get("/api/v1/media").await.1["total"], 0);
    assert_eq!(
        reader.get(&format!("/api/v1/media/{id}")).await.0,
        StatusCode::NOT_FOUND
    );
    // Existing ownership semantics must survive the age gate.
    assert_eq!(
        owner.get(&format!("/api/v1/media/{id}")).await.0,
        StatusCode::OK
    );
    fx.cleanup().await;
}

#[tokio::test]
async fn media_etags_validate_eligible_representations() {
    let fx = Fixture::new("media-etags").await;
    let mut owner = fx.client();
    register(&mut owner, "etags@example.com", "etagsauthor").await;
    let pseud = author_pseud_id(&fx, "etags@example.com").await;
    let id = seed_work(
        &fx,
        81,
        &pseud,
        "Cache me",
        "public",
        "published",
        "2026-09-01T00:00:00Z",
    )
    .await;
    let anon = fx.client();
    for uri in [
        "/api/v1/media".to_owned(),
        format!("/api/v1/media/{id}"),
        "/api/v1/media/feed".to_owned(),
    ] {
        let response = anon.conditional_get(&uri, None).await;
        assert_eq!(response.status(), StatusCode::OK);
        let etag = response
            .headers()
            .get(header::ETAG)
            .expect("ETag")
            .to_str()
            .unwrap()
            .to_owned();
        assert!(response.headers()[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("private"));
        let response = anon
            .conditional_get(&uri, Some(&format!("W/{etag}, \"other\"")))
            .await;
        assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
        assert!(axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            anon.conditional_get(&uri, Some("\"stale\"")).await.status(),
            StatusCode::OK
        );
    }
    let first = anon.conditional_get("/api/v1/media", None).await;
    let tag = first.headers()[header::ETAG].to_str().unwrap().to_owned();
    seed_work(
        &fx,
        82,
        &pseud,
        "Hidden",
        "restricted",
        "published",
        "2026-09-02T00:00:00Z",
    )
    .await;
    assert_eq!(
        anon.conditional_get("/api/v1/media", Some(&tag))
            .await
            .status(),
        StatusCode::NOT_MODIFIED
    );
    seed_work(
        &fx,
        83,
        &pseud,
        "New public",
        "public",
        "published",
        "2026-09-03T00:00:00Z",
    )
    .await;
    assert_eq!(
        anon.conditional_get("/api/v1/media", Some(&tag))
            .await
            .status(),
        StatusCode::OK
    );
    let missing = "/api/v1/media/00000000-0000-0000-0000-000000000082";
    assert_eq!(
        anon.conditional_get(missing, Some("*")).await.status(),
        StatusCode::NOT_FOUND
    );
    fx.cleanup().await;
}

#[tokio::test]
async fn media_query_fields_filter_get_post_and_feed() {
    let fx = Fixture::new("query-fields").await;
    let mut owner = fx.client();
    register(&mut owner, "fields@example.com", "fieldsauthor").await;
    let pseud = author_pseud_id(&fx, "fields@example.com").await;
    let wanted = seed_work(
        &fx,
        91,
        &pseud,
        "Needle",
        "public",
        "published",
        "2026-09-01T00:00:00Z",
    )
    .await;
    seed_work(
        &fx,
        92,
        &pseud,
        "Other",
        "public",
        "published",
        "2026-09-02T00:00:00Z",
    )
    .await;
    seed_work(
        &fx,
        93,
        &pseud,
        "Needle secret",
        "restricted",
        "published",
        "2026-09-03T00:00:00Z",
    )
    .await;
    seed_edition(&fx, &wanted).await;
    let statement = fx.tdb.sql("UPDATE works SET completion = 'complete', published_at = '2026-08-01T00:00:00Z' WHERE CAST(id AS TEXT) = ?");
    match fx.tdb.db().backend() {
        Backend::Sqlite => {
            sqlx::query(&statement)
                .bind(&wanted)
                .execute(fx.tdb.db().sqlite_pool().unwrap())
                .await
                .unwrap();
        }
        Backend::Postgres => {
            sqlx::query(&statement)
                .bind(&wanted)
                .execute(fx.tdb.db().postgres_pool().unwrap())
                .await
                .unwrap();
        }
    }
    let sql = fx.tdb.sql("INSERT INTO quality_signals (id, work_id, signal_kind, value, weight, source, computed_at) VALUES (CAST(? AS UUID), CAST(? AS UUID), 'editorial_review', 800, 1, 'test', '2026-09-01')");
    let sqlite = "INSERT INTO quality_signals (id, work_id, signal_kind, value, weight, source, computed_at) VALUES (?, ?, 'editorial_review', 800, 1, 'test', '2026-09-01')";
    match fx.tdb.db().backend() {
        Backend::Sqlite => {
            sqlx::query(sqlite)
                .bind(derived_id(&wanted, "30000000"))
                .bind(&wanted)
                .execute(fx.tdb.db().sqlite_pool().unwrap())
                .await
                .unwrap();
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(derived_id(&wanted, "30000000"))
                .bind(&wanted)
                .execute(fx.tdb.db().postgres_pool().unwrap())
                .await
                .unwrap();
        }
    }
    let mut anon = fx.client();
    for query in [
        "min_quality:750",
        "rating:general%20AND%20title:Needle",
        "completion:complete",
        "published:2026-08-01..2026-08-01",
        "quality:editorial_review:750",
        "format:prose%20AND%20updated:2026-09-01..2026-09-01",
        "edition:revised",
        "title:Needle",
        "author:fieldsauthor%20AND%20title:Needle",
        "title:Needle%20OR%20title:absent",
    ] {
        let (status, body) = anon.get(&format!("/api/v1/media?q={query}")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["total"], 1, "{query}: {body}");
        assert_eq!(body["items"][0]["id"], wanted);
    }
    let (status, body) = owner
        .post("/api/v1/media/query", json!({"q":"title:Other"}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 1, "{body}");
    let (status, feed) = anon.get_raw("/api/v1/media/feed?q=title:Needle").await;
    assert_eq!(status, StatusCode::OK, "{feed}");
    assert!(feed.contains("<title>Needle</title>"), "{feed}");
    assert!(!feed.contains("Needle secret") && !feed.contains("<title>Other</title>"));
    assert!(feed.contains("rel=\"self\""), "{feed}");
    assert!(feed.contains("q=title%3ANeedle"), "{feed}");
    assert!(feed.contains("urn:uuid:"), "{feed}");
    let (status, rss) = anon
        .get_raw("/api/v1/media/feed?q=title:Needle&format=rss")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(rss.contains("<rss version=\"2.0\""), "{rss}");
    assert!(rss.contains("<title>Needle</title>"));
    assert!(!rss.contains("Needle secret"));
    assert_eq!(
        anon.get_raw("/api/v1/media/feed?format=invalid").await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (status, _) = anon.get("/api/v1/media?q=%28title:Needle").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    for suffix in [
        "limit=0",
        "limit=-1",
        "cursor=broken",
        "cursor=2026-09-01%7Cnot-a-uuid",
        "q=min_quality:1001",
        "q=updated:2026-02-30..2026-03-01",
        "q=format:bogus",
    ] {
        let (status, body) = anon.get(&format!("/api/v1/media?{suffix}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{suffix}: {body}");
    }
    fx.cleanup().await;
}

#[tokio::test]
async fn media_listings_and_direct_doors_apply_the_visibility_matrix() {
    let fx = Fixture::new("media-visibility").await;
    let mut anon = fx.client();
    let mut owner = fx.client();
    register(&mut owner, "m22-author@example.com", "m22author").await;
    let pseud_id = author_pseud_id(&fx, "m22-author@example.com").await;

    let public1 = seed_work(
        &fx,
        1,
        &pseud_id,
        "Public one",
        "public",
        "published",
        "2026-09-01T00:01:00Z",
    )
    .await;
    seed_work(
        &fx,
        2,
        &pseud_id,
        "Public two",
        "public",
        "published",
        "2026-09-01T00:02:00Z",
    )
    .await;
    let unlisted = seed_work(
        &fx,
        3,
        &pseud_id,
        "Unlisted one",
        "unlisted",
        "published",
        "2026-09-01T00:03:00Z",
    )
    .await;
    let restricted = seed_work(
        &fx,
        4,
        &pseud_id,
        "Restricted one",
        "restricted",
        "published",
        "2026-09-01T00:04:00Z",
    )
    .await;
    let draft = seed_work(
        &fx,
        5,
        &pseud_id,
        "Draft one",
        "public",
        "draft",
        "2026-09-01T00:05:00Z",
    )
    .await;

    // Anonymous listing: public published works only, newest first.
    let (status, body) = anon.get("/api/v1/media").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let titles: Vec<&str> = body["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|i| i["title"].as_str().expect("title"))
        .collect();
    assert_eq!(titles, vec!["Public two", "Public one"]);
    assert_eq!(body["total"], 2);

    // The author's listing adds restricted and their own unlisted, but
    // drafts still never list.
    let (status, body) = owner.get("/api/v1/media").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let titles: Vec<&str> = body["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|i| i["title"].as_str().expect("title"))
        .collect();
    assert_eq!(
        titles,
        vec!["Restricted one", "Unlisted one", "Public two", "Public one"]
    );
    assert_eq!(body["total"], 4);

    // Direct doors: unlisted is link-reachable by anyone; restricted needs
    // a session; drafts stay owner-only. Ineligible callers get 404, not
    // 403 — existence is not revealed.
    let (status, _) = anon.get(&format!("/api/v1/media/{public1}")).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = anon.get(&format!("/api/v1/media/{unlisted}")).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = anon.get(&format!("/api/v1/media/{restricted}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = owner.get(&format!("/api/v1/media/{restricted}")).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = anon.get(&format!("/api/v1/media/{draft}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = owner.get(&format!("/api/v1/media/{draft}")).await;
    assert_eq!(status, StatusCode::OK);

    fx.cleanup().await;
}

#[tokio::test]
async fn media_pagination_walks_every_row_exactly_once() {
    let fx = Fixture::new("media-cursor").await;
    let mut client = fx.client();
    register(&mut client, "m22-cursor@example.com", "m22cursor").await;
    let pseud_id = author_pseud_id(&fx, "m22-cursor@example.com").await;

    // Five public works; rows 3 and 4 SHARE a created_at so the id
    // tiebreak inside the compound cursor is exercised.
    let mut seeded = Vec::new();
    for n in 1..=5u32 {
        let minute = match n {
            1 => 1,
            2 => 2,
            3 | 4 => 3,
            _ => 4,
        };
        let ts = format!("2026-09-01T00:{minute:02}:00Z");
        seeded.push(
            seed_work(
                &fx,
                n,
                &pseud_id,
                &format!("Work {n}"),
                "public",
                "published",
                &ts,
            )
            .await,
        );
    }

    let mut seen: Vec<String> = Vec::new();
    let mut uri = "/api/v1/media?limit=2".to_string();
    let mut pages = 0;
    loop {
        let (status, body) = client.get(&uri).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        for item in body["items"].as_array().expect("items") {
            let id = item["id"].as_str().expect("id").to_string();
            assert!(!seen.contains(&id), "row repeated across pages: {id}");
            seen.push(id);
        }
        pages += 1;
        match body["next_cursor"].as_str() {
            Some(cursor) => uri = format!("/api/v1/media?limit=2&cursor={cursor}"),
            None => break,
        }
    }
    assert_eq!(pages, 3, "5 rows at limit 2 must walk in 3 pages");
    seen.sort();
    seeded.sort();
    assert_eq!(seen, seeded, "the walk missed or invented rows");

    fx.cleanup().await;
}

#[tokio::test]
async fn the_media_feed_escapes_user_text() {
    let fx = Fixture::new("media-feed-escape").await;
    let mut client = fx.client();
    register(&mut client, "m22-feed@example.com", "m22feed").await;
    let pseud_id = author_pseud_id(&fx, "m22-feed@example.com").await;
    seed_work(
        &fx,
        1,
        &pseud_id,
        "<script>alert(\"x\")</script> & more",
        "public",
        "published",
        "2026-09-01T00:01:00Z",
    )
    .await;

    let (status, xml) = client.get_raw("/api/v1/media/feed").await;
    assert_eq!(status, StatusCode::OK, "{xml}");
    assert!(xml.contains("&lt;script&gt;"), "title not escaped: {xml}");
    assert!(xml.contains("&amp;"), "ampersand not escaped: {xml}");
    assert!(
        !xml.contains("<script>"),
        "raw script tag leaked into the feed: {xml}"
    );

    fx.cleanup().await;
}

#[tokio::test]
async fn media_files_and_editions_doors_return_data() {
    let fx = Fixture::new("media_files_editions").await;
    let mut owner = fx.client();
    register(&mut owner, "editions@example.com", "editions-handle").await;
    let pseud_id = author_pseud_id(&fx, "editions@example.com").await;

    // A published public work with one file and one edition seeded
    // directly — the doors must decode real rows, not empty vectors.
    let work_id = seed_work(
        &fx,
        1,
        &pseud_id,
        "Work with files",
        "public",
        "published",
        "2026-09-01T00:01:00Z",
    )
    .await;
    seed_file(&fx, &work_id, "application/epub+zip").await;
    seed_edition(&fx, &work_id).await;

    // A draft work with a file: its files door must 404 for anonymous
    // callers (the direct-door rule hides drafts) even though rows exist.
    let draft_id = seed_work(
        &fx,
        2,
        &pseud_id,
        "Draft with files",
        "public",
        "draft",
        "2026-09-01T00:02:00Z",
    )
    .await;
    seed_file(&fx, &draft_id, "text/plain").await;

    // Owner sees the file and the edition, fully decoded.
    let (status, body) = owner.get(&format!("/api/v1/media/{work_id}/files")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let files = body["files"].as_array().expect("files array");
    assert_eq!(files.len(), 1, "expected the seeded file: {body}");
    assert_eq!(files[0]["mime_type"], "application/epub+zip");
    assert_eq!(files[0]["version"], 1);

    let (status, body) = owner
        .get(&format!("/api/v1/media/{work_id}/editions"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let editions = body["editions"].as_array().expect("editions array");
    assert_eq!(editions.len(), 1, "expected the seeded edition: {body}");
    assert_eq!(editions[0]["edition_kind"], "revised");

    // Anonymous callers get 404 for the draft's files — the direct-door
    // rule hides drafts; existence is not leaked.
    let mut anon = fx.client();
    let (status, _) = anon.get(&format!("/api/v1/media/{draft_id}/files")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    fx.cleanup().await;
}

/// Seed one canon row (migration 0026).
async fn seed_canon(fx: &Fixture, n: u32, name: &str) -> String {
    let id = format!("00000000-0000-0000-0000-{n:012}");
    let cast = if fx.tdb.is_postgres() { "::uuid" } else { "" };
    let sql = fx.tdb.sql(&format!(
        "INSERT INTO canons (id, name, created_at, updated_at, version) \
         VALUES (?{cast}, ?, '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z', 1)"
    ));
    let db = fx.tdb.db();
    let result = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(&id)
            .bind(name)
            .execute(db.sqlite_pool().expect("sqlite pool"))
            .await
            .map(|_| ()),
        Backend::Postgres => sqlx::query(&sql)
            .bind(&id)
            .bind(name)
            .execute(db.postgres_pool().expect("pg pool"))
            .await
            .map(|_| ()),
    };
    result.expect("seed canon");
    id
}

/// Seed a canon_works association row (migration 0026).
async fn seed_canon_work(fx: &Fixture, canon_id: &str, work_id: &str, position: i32) {
    let cast = if fx.tdb.is_postgres() { "::uuid" } else { "" };
    let sql = fx.tdb.sql(&format!(
        "INSERT INTO canon_works (canon_id, work_id, position, created_at) \
         VALUES (?{cast}, ?{cast}, ?, '2026-09-01T00:00:00Z')"
    ));
    let db = fx.tdb.db();
    let result = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(canon_id)
            .bind(work_id)
            .bind(position)
            .execute(db.sqlite_pool().expect("sqlite pool"))
            .await
            .map(|_| ()),
        Backend::Postgres => sqlx::query(&sql)
            .bind(canon_id)
            .bind(work_id)
            .bind(position)
            .execute(db.postgres_pool().expect("pg pool"))
            .await
            .map(|_| ()),
    };
    result.expect("seed canon_work");
}

/// Seed one space row (migration 0026).
async fn seed_space(fx: &Fixture, n: u32, name: &str) -> String {
    let id = format!("00000000-0000-0000-0000-{n:012}");
    let cast = if fx.tdb.is_postgres() { "::uuid" } else { "" };
    let sql = fx.tdb.sql(&format!(
        "INSERT INTO spaces (id, name, created_at, updated_at, version) \
         VALUES (?{cast}, ?, '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z', 1)"
    ));
    let db = fx.tdb.db();
    let result = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(&id)
            .bind(name)
            .execute(db.sqlite_pool().expect("sqlite pool"))
            .await
            .map(|_| ()),
        Backend::Postgres => sqlx::query(&sql)
            .bind(&id)
            .bind(name)
            .execute(db.postgres_pool().expect("pg pool"))
            .await
            .map(|_| ()),
    };
    result.expect("seed space");
    id
}

/// Seed a space_works association row (migration 0026).
async fn seed_space_work(fx: &Fixture, space_id: &str, work_id: &str, position: i32) {
    let cast = if fx.tdb.is_postgres() { "::uuid" } else { "" };
    let sql = fx.tdb.sql(&format!(
        "INSERT INTO space_works (space_id, work_id, position, created_at) \
         VALUES (?{cast}, ?{cast}, ?, '2026-09-01T00:00:00Z')"
    ));
    let db = fx.tdb.db();
    let result = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(space_id)
            .bind(work_id)
            .bind(position)
            .execute(db.sqlite_pool().expect("sqlite pool"))
            .await
            .map(|_| ()),
        Backend::Postgres => sqlx::query(&sql)
            .bind(space_id)
            .bind(work_id)
            .bind(position)
            .execute(db.postgres_pool().expect("pg pool"))
            .await
            .map(|_| ()),
    };
    result.expect("seed space_work");
}

#[tokio::test]
async fn canon_and_space_doors_return_scoped_media() {
    let fx = Fixture::new("canon-space").await;
    let mut owner = fx.client();
    register(&mut owner, "canon@example.com", "canon-handle").await;
    let pseud_id = author_pseud_id(&fx, "canon@example.com").await;

    // Seed a published public work.
    let work_id = seed_work(
        &fx,
        1,
        &pseud_id,
        "Canon Work",
        "public",
        "published",
        "2026-09-01T00:01:00Z",
    )
    .await;

    // Seed a draft work (should NOT appear for anonymous callers).
    let draft_id = seed_work(
        &fx,
        2,
        &pseud_id,
        "Draft Work",
        "public",
        "draft",
        "2026-09-01T00:02:00Z",
    )
    .await;

    // Seed canon and associate the published work.
    let canon_id = seed_canon(&fx, 100, "Mainline Canon").await;
    seed_canon_work(&fx, &canon_id, &work_id, 1).await;
    seed_canon_work(&fx, &canon_id, &draft_id, 2).await;

    // Anonymous: only the published work appears.
    let mut anon = fx.client();
    let canon_uri = format!("/api/v1/canons/{canon_id}/media");
    let (status, body) = anon.get(&canon_uri).await;
    assert_eq!(status, StatusCode::OK, "canon door failed: {body}");
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1, "only published work expected: {body}");
    assert_eq!(items[0]["id"], work_id);
    assert_eq!(body["name"], "Mainline Canon");

    // Seed space and associate the published work.
    let space_id = seed_space(&fx, 200, "Fandom Space").await;
    seed_space_work(&fx, &space_id, &work_id, 1).await;

    let space_uri = format!("/api/v1/spaces/{space_id}/media");
    let (status, body) = anon.get(&space_uri).await;
    assert_eq!(status, StatusCode::OK, "space door failed: {body}");
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1, "only published work expected: {body}");
    assert_eq!(items[0]["id"], work_id);
    assert_eq!(body["name"], "Fandom Space");

    let hidden = seed_work(
        &fx,
        3,
        &pseud_id,
        "Unlisted",
        "unlisted",
        "published",
        "2026-09-01T00:03:00Z",
    )
    .await;
    let restricted = seed_work(
        &fx,
        4,
        &pseud_id,
        "Restricted",
        "restricted",
        "published",
        "2026-09-01T00:04:00Z",
    )
    .await;
    for id in [&hidden, &restricted] {
        seed_canon_work(&fx, &canon_id, id, 3).await;
        seed_space_work(&fx, &space_id, id, 3).await;
    }
    let mut reader = fx.client();
    register(&mut reader, "scope-reader@example.com", "scopereader").await;
    for uri in [&canon_uri, &space_uri] {
        let (_, body) = anon.get(uri).await;
        assert_eq!(
            body["items"].as_array().unwrap().len(),
            1,
            "anonymous scope leaked: {body}"
        );
        let (_, body) = reader.get(uri).await;
        let items = body["items"].as_array().unwrap();
        assert_eq!(items.len(), 2, "signed-in scope: {body}");
        assert!(items.iter().any(|item| item["id"] == restricted));
        assert!(!items.iter().any(|item| item["id"] == hidden));
    }

    // 404 for non-existent canon.
    let bad_canon = "/api/v1/canons/00000000-0000-0000-0000-999999999999/media";
    let (status, _) = anon.get(bad_canon).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // 404 for non-existent space.
    let bad_space = "/api/v1/spaces/00000000-0000-0000-0000-999999999999/media";
    let (status, _) = anon.get(bad_space).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    fx.cleanup().await;
}
