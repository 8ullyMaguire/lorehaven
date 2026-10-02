//! Gap B's HTTP surface: `GET /api/v1/discovery/blind-date`.
//!
//! The store-level behaviour is proved in `crates/app/tests/blind_date.rs`. What is proved
//! here is what only the route can get wrong, and both items are invisible to a store
//! test:
//!
//!   * **the date comes from the server, not the request.** A `?date=` parameter would let
//!     a client enumerate forward and turn "one work per reader per day" into a catalogue
//!     browser -- the exact thing the deterministic design exists to prevent.
//!   * **"no eligible work today" is a 200 with a null body, not a 404.** An empty
//!     catalogue is a quiet surface, not a broken one, and a 404 conflates it with "no
//!     such endpoint".
//!
//! The `Client`/`Harness` pair is copied from `milestone_16.rs` rather than extracted into
//! a shared module: a shared HTTP harness is a refactor of eight test binaries, and this is
//! three assertions.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::DatabaseConfig;
use serde_json::{json, Value};
use test_support::scratch_dir;
use tower::ServiceExt;

const PASSWORD: &str = "a-long-enough-passphrase";

fn config_for(dir: &Path) -> Config {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    config.database = DatabaseConfig::new(format!(
        "sqlite://{}/lorehaven.sqlite?mode=rwc",
        dir.display()
    ));
    // Raise rate limits for parallel test execution.
    config.rate_limits.auth = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.write = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.search = lorehaven_app::limiter::Quota {
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
    async fn request(
        &mut self,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
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
        let status = response.status();
        self.capture(&response);
        let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024)
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
    async fn get(&mut self, uri: &str) -> (StatusCode, Value) {
        self.request("GET", uri, None).await
    }
    async fn post(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("POST", uri, Some(body)).await
    }
}

struct Harness {
    dir: PathBuf,
    tdb: test_support::TestDb,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        set_trust_proxy(false);
        let _ = lorehaven_app::logging::init(&lorehaven_app::config::LoggingConfig {
            filter: "error".to_owned(),
            format: lorehaven_app::config::LogFormat::Pretty,
        });
        let dir = scratch_dir(tag);
        let tdb = test_support::TestDb::connect_with_dir(tag, &dir).await;
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

/// Create `count` published, public works so the surface has something to pick.
///
/// Each work gets its own author, via the same `exec_with` dialect helper the other suites
/// in this directory use: hand-rolling `RETURNING` plus a `?N`/`$N` rewrite is three more
/// ways to be subtly wrong on one engine, and a correct helper already exists.
async fn seed_works(tdb: &test_support::TestDb, count: usize) {
    for n in 0..count {
        let email = format!(
            "bd-author-{n}-{}@example.com",
            uuid::Uuid::new_v4().simple()
        );
        let account = uuid::Uuid::new_v4().to_string();
        exec_with(
            tdb,
            "INSERT INTO accounts (id, email, created_at, updated_at) \
             VALUES (?1#u, ?2, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            &[&account, &email],
        )
        .await;
        exec_with(
            tdb,
            "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
             VALUES (?1#u, ?2#u, ?3, ?3, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            &[
                &uuid::Uuid::new_v4().to_string(),
                &account,
                &format!("bda{}", &uuid::Uuid::new_v4().to_string()[..8]),
            ],
        )
        .await;
        exec_with(
            tdb,
            "INSERT INTO works (id, owner_pseud_id, title, lifecycle, visibility, published_at, \
                 created_at, updated_at, generated_content_posture) \
             SELECT ?1#u, id, ?3, 'published', 'public', '2026-01-01T00:00:00Z', \
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'forbid' \
             FROM pseuds WHERE account_id = ?2#u",
            &[
                &uuid::Uuid::new_v4().to_string(),
                &account,
                &format!("Blind Date candidate {n}"),
            ],
        )
        .await;
    }
}

/// `?N#u` binds a native uuid on PostgreSQL and text on SQLite; `?N#i` an integer; a bare
/// `?N` a string.
async fn exec_with(tdb: &test_support::TestDb, sqlite: &str, args: &[&String]) {
    let db = tdb.db();
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            let sql = sqlite.replace("#u", "").replace("#i", "");
            let mut q = sqlx::query(&sql);
            for a in args {
                q = q.bind(a.as_str());
            }
            q.execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("fixture insert");
        }
        lorehaven_db::Backend::Postgres => {
            let pg = (1..=8).fold(sqlite.to_string(), |acc, n| {
                acc.replace(&format!("?{n}#u"), &format!("${n}::uuid"))
                    .replace(&format!("?{n}#i"), &format!("${n}::bigint"))
                    .replace(&format!("?{n}"), &format!("${n}"))
            });
            let mut q = sqlx::query(&pg);
            for a in args {
                match uuid::Uuid::parse_str(a) {
                    Ok(u) => q = q.bind(u),
                    Err(_) => q = q.bind(a.as_str()),
                }
            }
            q.execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("fixture insert");
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

async fn register(client: &mut Client, email: &str, handle: &str) {
    let (status, body) = client
        .post(
            "/api/v1/auth/register",
            json!({
                "email": email,
                "password": PASSWORD,
                "handle": handle,
                "display_name": handle,
                "age_band": "adult",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "register {handle}: {body}");
}

#[tokio::test]
async fn the_endpoint_returns_a_work_id_and_ignores_any_date_parameter() {
    let harness = Harness::new("blind-date-route").await;
    let mut client = harness.client();
    register(&mut client, "reader@example.com", "BdReader").await;
    seed_works(&harness.tdb, 5).await;

    let (status, body) = client.get("/api/v1/discovery/blind-date").await;
    assert_eq!(status, StatusCode::OK, "blind date: {body}");
    assert!(
        body["work_id"].is_string(),
        "five eligible works, so there is a pick: {body}"
    );

    // The date parameter must not exist. Even if the server ignored it today, a client
    // that sends one believes it can enumerate, so the response shape is what matters: the
    // server always reports its own date and always picks for it.
    let (status, dated) = client
        .get("/api/v1/discovery/blind-date?date=2020-01-01")
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an unknown parameter is not an error: {dated}"
    );
    assert_eq!(
        dated["work_id"], body["work_id"],
        "the pick must not change because the client asked for another day"
    );

    harness.cleanup().await;
}

#[tokio::test]
async fn an_empty_catalogue_is_a_200_with_a_null_work_not_a_404() {
    // Nothing to offer today is a normal answer. A 404 would conflate it with "no such
    // endpoint", and the frontend would render an error instead of an empty state.
    let harness = Harness::new("blind-date-empty").await;
    let mut client = harness.client();
    register(&mut client, "empty@example.com", "BdEmpty").await;

    let (status, body) = client.get("/api/v1/discovery/blind-date").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an empty catalogue is quiet, not broken: {body}"
    );
    assert_eq!(
        body["work_id"],
        Value::Null,
        "no eligible work, and the body says so: {body}"
    );
    assert!(
        body["date"].is_string(),
        "the date is still reported, so a client can cache per day: {body}"
    );

    harness.cleanup().await;
}

#[tokio::test]
async fn the_endpoint_is_stable_across_reloads() {
    // The whole point of the deterministic seed, at the layer a user can observe.
    let harness = Harness::new("blind-date-stable").await;
    let mut client = harness.client();
    register(&mut client, "stable@example.com", "BdStable").await;
    seed_works(&harness.tdb, 8).await;

    let (_, first) = client.get("/api/v1/discovery/blind-date").await;
    for _ in 0..5 {
        let (_, again) = client.get("/api/v1/discovery/blind-date").await;
        assert_eq!(
            again["work_id"], first["work_id"],
            "five reloads later it is the same work"
        );
    }

    harness.cleanup().await;
}
