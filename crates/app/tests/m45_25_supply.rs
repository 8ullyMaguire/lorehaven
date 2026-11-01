//! M45-25: the supply doors, through the real router.
//!
//! The store properties are in `crates/db/tests/supply_keystones.rs`. These are
//! the ones that belong to the route:
//!
//! - **every supply door needs a session**, and the anonymous answer is `401`
//!   rather than an empty list that reads as "no keystone authors";
//! - **a non-operator gets the same `404` a missing page gets**, because
//!   `require_operator` does not confirm the existence of an operator surface to
//!   somebody who is not one;
//! - **the operator's recompute is the only way a keystone list appears**, and the
//!   list it produces is the one the drought report reads;
//! - **the drought report refuses in words** on a catalogue with no comments,
//!   instead of an empty array a caller would read as "no droughts";
//! - **a reader's routing is theirs**, and somebody else's id is a `404`.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use darkhold_app::config::Config;
use darkhold_app::server;
use darkhold_app::state::AppState;
use darkhold_domain::ids::AccountId;
use serde_json::{json, Value};
use test_support::TestDb;
use tower::ServiceExt;

const T0: &str = "2026-01-01T00:00:00Z";
const PASSWORD: &str = "correct horse battery staple 7";

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "darkhold-m45-25-{tag}-{}-{:?}",
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

/// A cookie-carrying client: the session routes need one, and a client that does
/// not carry cookies back is testing the anonymous path and passing by accident.
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

    fn capture(&mut self, response: &axum::response::Response) {
        for value in response.headers().get_all(header::SET_COOKIE) {
            let Ok(text) = value.to_str() else { continue };
            let Some((pair, _)) = text.split_once(';') else {
                continue;
            };
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            let name = name.trim().to_owned();
            let value = value.trim().to_owned();
            self.cookies.retain(|(k, _)| k != &name);
            if !value.is_empty() {
                self.cookies.push((name, value));
            }
        }
    }

    async fn request(&mut self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        if !self.cookies.is_empty() {
            let jar = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            builder = builder.header(header::COOKIE, jar);
            if let Some((_, token)) = self.cookies.iter().find(|(k, _)| k == "darkhold_csrf") {
                builder = builder.header("x-csrf-token", token);
            }
        }
        let body = match body {
            Some(v) => Body::from(v.to_string()),
            None => Body::empty(),
        };
        let response = self
            .app
            .clone()
            .oneshot(builder.body(body).expect("request"))
            .await
            .expect("response");
        let status = response.status();
        self.capture(&response);
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

    async fn get(&mut self, uri: &str) -> (StatusCode, Value) {
        self.request("GET", uri, None).await
    }

    async fn post(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("POST", uri, Some(body)).await
    }

    /// Register, and return the account id the session now acts for.
    async fn register(&mut self, email: &str, handle: &str) -> String {
        let (status, body) = self
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
        let (status, me) = self.get("/api/v1/auth/me").await;
        assert_eq!(status, StatusCode::OK, "{me}");
        me["account"]["id"]
            .as_str()
            .expect("account id")
            .to_owned()
    }
}

struct Harness {
    _dir: PathBuf,
    tdb: TestDb,
    config: Config,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        let config = config_for(&dir);
        seed(db_of(&tdb)).await;
        Self {
            _dir: dir,
            tdb,
            config,
        }
    }

    fn client(&self) -> Client {
        Client::new(server::build_router(AppState::new(
            self.config.clone(),
            self.tdb.db().clone(),
        )))
    }

    /// A client whose session is the operator, by naming that account in config —
    /// which is exactly how `require_operator` decides — **carrying the cookies
    /// of the client that registered**, because a router without the session
    /// cookie is an anonymous caller and every operator assert would fail at 401
    /// for a reason that has nothing to do with the operator check.
    fn operator(&self, from: &Client, account: &str) -> Client {
        let mut config = self.config.clone();
        config.administration.operator_account_id =
            Some(AccountId::from_str(account).expect("account id"));
        let mut client = Client::new(server::build_router(AppState::new(
            config,
            self.tdb.db().clone(),
        )));
        client.cookies = from.cookies.clone();
        client
    }
}

fn db_of(tdb: &TestDb) -> &darkhold_db::Database {
    tdb.db()
}

/// Two authors, two engaged works each, in one source. `ada` holds the top two
/// ranks of four, so with `min_pct = 0.5` she is a keystone and `bea` is not.
async fn seed(db: &darkhold_db::Database) {
    let account = uuid::Uuid::new_v4().to_string();
    let email = format!("{account}@example.invalid");
    match db.backend() {
        darkhold_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO accounts (id, email, status, age_state, created_at, updated_at, permission_statement)
                 VALUES ($1::uuid, $2, 'active', 'adult', $3, $3, 'minimal')",
            )
            .bind(uuid::Uuid::parse_str(&account).expect("uuid"))
            .bind(&email)
            .bind(T0)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("fixture account");
        }
        darkhold_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO accounts (id, email, status, age_state, created_at, updated_at, permission_statement)
                 VALUES (?, ?, 'active', 'adult', ?, ?, 'minimal')",
            )
            .bind(&account)
            .bind(&email)
            .bind(T0)
            .bind(T0)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("fixture account");
        }
    }

    for (key, author, likes) in [
        ("ada-1", "ada", 900),
        ("ada-2", "ada", 901),
        ("bea-1", "bea", 10),
        ("bea-2", "bea", 11),
    ] {
        match db.backend() {
            darkhold_db::Backend::Postgres => {
                sqlx::query(
                    "INSERT INTO library_items
                     (id, account_id, source_key, source_work_key, title, author_text, summary,
                      status, source_url, created_at, updated_at, likes)
                     VALUES ($1::uuid, $2::uuid, 'sb', $3, $4, $5, '', 'ongoing',
                             'https://example.invalid/w', $6, $6, $7)",
                )
                .bind(uuid::Uuid::new_v4())
                .bind(uuid::Uuid::parse_str(&account).expect("uuid"))
                .bind(key)
                .bind(format!("{key} title"))
                .bind(author)
                .bind(T0)
                .bind(likes)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("library item");
            }
            darkhold_db::Backend::Sqlite => {
                sqlx::query(
                    "INSERT INTO library_items
                     (id, account_id, source_key, source_work_key, title, author_text, summary,
                      status, source_url, created_at, updated_at, likes)
                     VALUES (?, ?, 'sb', ?, ?, ?, '', 'ongoing', 'https://example.invalid/w', ?, ?, ?)",
                )
                .bind(uuid::Uuid::new_v4().to_string())
                .bind(&account)
                .bind(key)
                .bind(format!("{key} title"))
                .bind(author)
                .bind(T0)
                .bind(T0)
                .bind(likes)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("library item");
            }
        }
    }
}

#[tokio::test]
async fn an_anonymous_caller_gets_401_on_every_supply_door() {
    let harness = Harness::new("anon").await;
    let mut client = harness.client();
    for uri in [
        "/api/v1/supply/keystones",
        "/api/v1/supply/droughts",
        "/api/v1/supply/routing",
    ] {
        let (status, body) = client.get(uri).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}: {body}");
    }
    let (status, body) = client.post("/api/v1/supply/route", json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "route: {body}");
}

#[tokio::test]
async fn a_non_operator_gets_a_404_rather_than_a_403() {
    let harness = Harness::new("not-operator").await;
    let mut client = harness.client();
    client.register("ordinary@example.test", "Ordinary").await;
    // The operator door answers as a missing page: confirming "this exists but is
    // not yours" is a map of the instance, and require_operator does not draw one.
    let (status, body) = client.get("/api/v1/supply/keystones").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn the_operator_recomputes_and_reads_the_keystones() {
    let harness = Harness::new("operator").await;
    let mut client = harness.client();
    let account = client.register("operator@example.test", "Operator").await;
    let mut operator = harness.operator(&client, &account);

    // Before a run there is no snapshot, and the empty list says why.
    let (status, body) = operator.get("/api/v1/supply/keystones").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["measured"], false);
    assert!(body["reason"].as_str().unwrap_or("").contains("recompute"));

    let (status, body) = operator
        .post("/api/v1/supply/recompute", json!({ "min_pct": 0.5 }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["engaged_rows"], 4);
    assert_eq!(body["keystones"], 1, "ada clears 0.5, bea does not: {body}");
    assert_eq!(body["taste_profile_rows"], 0, "no operator weights exist yet");

    let (status, body) = operator.get("/api/v1/supply/keystones?limit=5").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["measured"], true);
    assert_eq!(body["keystones"][0]["author_key"], "ada");
    assert_eq!(body["keystones"][0]["engaged_works"], 2);
}

#[tokio::test]
async fn droughts_name_the_reason_when_nothing_can_be_measured() {
    let harness = Harness::new("drought").await;
    let mut client = harness.client();
    let account = client.register("operator2@example.test", "Operator2").await;
    let mut operator = harness.operator(&client, &account);
    operator
        .post("/api/v1/supply/recompute", json!({ "min_pct": 0.5 }))
        .await;

    let (status, body) = operator.get("/api/v1/supply/droughts").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["measured"], false, "{body}");
    let reason = body["reason"].as_str().unwrap_or_default();
    assert!(
        reason.contains("cannot be told"),
        "the reason names the ambiguity rather than returning []: {reason}"
    );
    assert_eq!(body["comments_total"], 0);
    assert_eq!(
        body["authors"].as_array().map(Vec::len),
        Some(0),
        "an empty array plus a reason, never a list of every engaged work"
    );
}

#[tokio::test]
async fn a_reader_is_routed_and_completing_pays_once() {
    let harness = Harness::new("reader").await;
    let mut client = harness.client();
    let account = client.register("reader@example.test", "Reader").await;
    let mut operator = harness.operator(&client, &account);
    operator
        .post("/api/v1/supply/recompute", json!({ "min_pct": 0.5 }))
        .await;

    let (status, body) = client.post("/api/v1/supply/route", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let first = body["route"].clone();
    assert_eq!(first["author_key"], "ada", "{body}");
    let id = first["id"].as_str().expect("route id").to_owned();

    let (status, body) = client.post("/api/v1/supply/route", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let second = body["route"].clone();
    assert_ne!(first["work_id"], second["work_id"], "the exclusion moved on");

    // ada's two works are now both offered, so the next ask is a result, not an error.
    let (status, body) = client.post("/api/v1/supply/route", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["route"].is_null(), "{body}");
    assert!(body["reason"].as_str().unwrap_or("").contains("already been offered"));

    let (status, body) = client
        .post(&format!("/api/v1/supply/routing/{id}/complete"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["credit_bp"], 500, "patronage pays in basis points");

    let (status, body) = client.get("/api/v1/supply/routing").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["routing"].as_array().map(Vec::len), Some(2));
}

#[tokio::test]
async fn completing_another_readers_routing_is_a_404() {
    let harness = Harness::new("steal").await;
    let mut owner = harness.client();
    let account = owner.register("owner@example.test", "Owner").await;
    let mut operator = harness.operator(&owner, &account);
    operator
        .post("/api/v1/supply/recompute", json!({ "min_pct": 0.5 }))
        .await;
    let (_, body) = owner.post("/api/v1/supply/route", json!({})).await;
    let id = body["route"]["id"].as_str().expect("route id").to_owned();

    let mut stranger = harness.client();
    stranger.register("stranger@example.test", "Stranger").await;
    let (status, body) = stranger
        .post(&format!("/api/v1/supply/routing/{id}/complete"), json!({}))
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "somebody else's routing does not exist for you: {body}"
    );
}
