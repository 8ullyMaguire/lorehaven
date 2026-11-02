//! M45-26: the welcome rota's doors, through the real router.
//!
//! The store properties live in `crates/db/tests/welcome_rota.rs`. These are the ones
//! that belong to the route:
//!
//! - **every reader door needs a session** — the anonymous answer is `401`, not an
//!   empty list that reads as "you owe nothing";
//! - **the report is operator-only**, and a reader gets the `404` a missing page gets;
//! - **a session with no active pseud is refused on the field `pseud`**, because the
//!   promise is made to a pseud and a pseud is what can write the comment;
//! - **a second claim returns the open welcome**, so the surface cannot be used to
//!   bank assignments;
//! - **the rota closes on the comment**: `completed: false` with a reason naming the
//!   work until a comment by that pseud exists, then `completed: true`;
//! - **the module is registered and carries its nav entry**, so the surface is
//!   reachable from the reader's sidebar and not merely mounted.

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
        "darkhold-m45-26-{tag}-{}-{:?}",
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

    /// Register, and return `(account id, pseud id)`. The pseud is created by
    /// registration but is NOT this session's active one until it is activated, which
    /// is what the "no pseud" door below relies on.
    async fn register(&mut self, email: &str, handle: &str) -> (String, String) {
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
        let account = me["account"]["id"].as_str().expect("account id").to_owned();
        let pseud = me["pseuds"][0]["id"].as_str().expect("pseud id").to_owned();
        (account, pseud)
    }

    async fn activate(&mut self, pseud: &str) {
        let (status, body) = self
            .post(&format!("/api/v1/pseuds/{pseud}/activate"), json!({}))
            .await;
        assert!(
            status == StatusCode::NO_CONTENT || status == StatusCode::OK,
            "activate {pseud}: {status} {body}"
        );
    }
}

struct Harness {
    _dir: PathBuf,
    tdb: TestDb,
    config: Config,
    /// The catalogue row the rota can offer: endorsed, with a work id.
    work_id: String,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        let config = config_for(&dir);
        let work_id = seed_work(tdb.db()).await;
        Self {
            _dir: dir,
            tdb,
            config,
            work_id,
        }
    }

    fn client(&self) -> Client {
        Client::new(server::build_router(AppState::new(
            self.config.clone(),
            self.tdb.db().clone(),
        )))
    }

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

/// One endorsed work that a welcome can be taken for. Returns its work id.
async fn seed_work(db: &darkhold_db::Database) -> String {
    let account = uuid::Uuid::new_v4().to_string();
    let pseud = uuid::Uuid::new_v4().to_string();
    let work = uuid::Uuid::new_v4().to_string();
    let item = uuid::Uuid::new_v4().to_string();
    let email = format!("{account}@example.invalid");
    let handle = format!("author-{}", &account[..8]);
    match db.backend() {
        darkhold_db::Backend::Postgres => {
            for sql in [
                "INSERT INTO accounts (id, email, status, age_state, created_at, updated_at, permission_statement)
                 VALUES ($1::uuid, $2, 'active', 'adult', $3, $3, 'minimal')".to_string(),
            ] {
                sqlx::query(&sql)
                    .bind(uuid::Uuid::parse_str(&account).expect("uuid"))
                    .bind(&email)
                    .bind(T0)
                    .execute(db.postgres_pool().expect("postgres"))
                    .await
                    .expect("fixture account");
            }
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES ($1::uuid, $2::uuid, $3, $3, $4::timestamptz, $4::timestamptz)",
            )
            .bind(uuid::Uuid::parse_str(&pseud).expect("uuid"))
            .bind(uuid::Uuid::parse_str(&account).expect("uuid"))
            .bind(&handle)
            .bind(T0)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("fixture pseud");
            sqlx::query(
                "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at,
                                    generated_content_posture)
                 VALUES ($1::uuid, $2::uuid, 'a finished story', $3::timestamptz, $3::timestamptz, 'forbid')",
            )
            .bind(uuid::Uuid::parse_str(&work).expect("uuid"))
            .bind(uuid::Uuid::parse_str(&pseud).expect("uuid"))
            .bind(T0)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("fixture work");
            sqlx::query(
                "INSERT INTO library_items
                 (id, account_id, source_key, source_work_key, title, author_text, summary,
                  status, source_url, created_at, updated_at, likes, work_id)
                 VALUES ($1::uuid, $2::uuid, 'sb', 'probe-work', 'a finished story', 'Ada Author',
                         '', 'ongoing', 'https://example.invalid/w', $3, $3, 40, $4::uuid)",
            )
            .bind(uuid::Uuid::parse_str(&item).expect("uuid"))
            .bind(uuid::Uuid::parse_str(&account).expect("uuid"))
            .bind(T0)
            .bind(uuid::Uuid::parse_str(&work).expect("uuid"))
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("fixture library item");
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
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&pseud)
            .bind(&account)
            .bind(&handle)
            .bind(&handle)
            .bind(T0)
            .bind(T0)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("fixture pseud");
            sqlx::query(
                "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at,
                                    generated_content_posture)
                 VALUES (?, ?, 'a finished story', ?, ?, 'forbid')",
            )
            .bind(&work)
            .bind(&pseud)
            .bind(T0)
            .bind(T0)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("fixture work");
            sqlx::query(
                "INSERT INTO library_items
                 (id, account_id, source_key, source_work_key, title, author_text, summary,
                  status, source_url, created_at, updated_at, likes, work_id)
                 VALUES (?, ?, 'sb', 'probe-work', 'a finished story', 'Ada Author',
                         '', 'ongoing', 'https://example.invalid/w', ?, ?, 40, ?)",
            )
            .bind(&item)
            .bind(&account)
            .bind(T0)
            .bind(T0)
            .bind(&work)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("fixture library item");
        }
    }
    work
}

/// A comment by `pseud` on `work`, at the canonical second-precision shape the
/// column accepts.
async fn comment_as(db: &darkhold_db::Database, work: &str, pseud: &str) {
    let id = uuid::Uuid::new_v4().to_string();
    let at = darkhold_db::identity::now_rfc3339();
    match db.backend() {
        darkhold_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO comments (id, subject_type, subject_id, author_pseud, body,
                                       body_version, created_at)
                 VALUES ($1::uuid, 'work', $2, $3::uuid, 'a welcome', 'v1', $4)",
            )
            .bind(uuid::Uuid::parse_str(&id).expect("uuid"))
            .bind(work)
            .bind(uuid::Uuid::parse_str(pseud).expect("uuid"))
            .bind(&at)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("comment");
        }
        darkhold_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO comments (id, subject_type, subject_id, author_pseud, body,
                                       body_version, created_at)
                 VALUES (?, 'work', ?, ?, 'a welcome', 'v1', ?)",
            )
            .bind(&id)
            .bind(work)
            .bind(pseud)
            .bind(&at)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("comment");
        }
    }
}

#[tokio::test]
async fn every_reader_door_needs_a_session() {
    let h = Harness::new("anon").await;
    let mut anon = h.client();
    for (method, uri) in [
        ("GET", "/api/v1/welcome/rota"),
        ("POST", "/api/v1/welcome/rota/claim"),
        ("POST", "/api/v1/welcome/rota/whatever/complete"),
    ] {
        let (status, _) = anon.request(method, uri, Some(json!({}))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri} anonymously");
    }
}

#[tokio::test]
async fn the_report_is_operator_only() {
    let h = Harness::new("report").await;
    let mut reader = h.client();
    let (account, _pseud) = reader.register("reader@example.invalid", "reader").await;

    let (status, _) = reader.get("/api/v1/welcome/report").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a non-operator must not learn that this surface exists"
    );

    let mut op = h.operator(&reader, &account);
    let (status, body) = op.get("/api/v1/welcome/report").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["measured"], json!(true));
    assert_eq!(body["eligible"], json!(1), "one endorsed work needs a welcome");
}

#[tokio::test]
async fn a_session_with_no_active_pseud_is_refused_on_that_field() {
    let h = Harness::new("nopseud").await;
    let mut reader = h.client();
    let (account, _pseud) = reader.register("nopseud@example.invalid", "nopseud").await;
    // Registration activates a pseud, so the door is not reachable by registering and
    // not activating. It is reachable the way it actually happens: the pseud the
    // session chose was deleted, which `me` documents and this reproduces by clearing
    // the session's choice the way the delete path does.
    let db = db_of(&h.tdb);
    match db.backend() {
        darkhold_db::Backend::Postgres => {
            sqlx::query("UPDATE sessions SET active_pseud_id = NULL WHERE account_id = $1::uuid")
                .bind(uuid::Uuid::parse_str(&account).expect("uuid"))
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("clear active pseud");
        }
        darkhold_db::Backend::Sqlite => {
            sqlx::query("UPDATE sessions SET active_pseud_id = NULL WHERE account_id = ?")
                .bind(&account)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("clear active pseud");
        }
    }
    // The session cannot write a comment, so it cannot be given a welcome either.
    let (status, body) = reader.post("/api/v1/welcome/rota/claim", json!({})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], json!("VALIDATION_FAILED"));
    assert!(
        body["error"]["field_errors"]["pseud"].is_string(),
        "the refusal names the field: {body}"
    );
}

#[tokio::test]
async fn claim_then_a_second_claim_returns_the_same_welcome() {
    let h = Harness::new("same").await;
    let mut reader = h.client();
    let (_account, pseud) = reader.register("same@example.invalid", "same").await;
    reader.activate(&pseud).await;

    let (status, first) = reader.post("/api/v1/welcome/rota/claim", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["taken"], json!("new"));
    assert_eq!(first["assignment"]["work_id"], json!(h.work_id));
    let id = first["assignment"]["id"].as_str().expect("id").to_owned();

    let (status, second) = reader.post("/api/v1/welcome/rota/claim", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["taken"], json!("already-open"));
    assert_eq!(second["assignment"]["id"], json!(id));
}

#[tokio::test]
async fn the_rota_closes_on_the_comment_and_not_on_the_button() {
    let h = Harness::new("closes").await;
    let mut reader = h.client();
    let (_account, pseud) = reader.register("closes@example.invalid", "closes").await;
    reader.activate(&pseud).await;

    let (status, claimed) = reader.post("/api/v1/welcome/rota/claim", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{claimed}");
    let id = claimed["assignment"]["id"].as_str().expect("id").to_owned();
    let work = claimed["assignment"]["work_id"]
        .as_str()
        .expect("work")
        .to_owned();

    let (status, refused) = reader
        .post(&format!("/api/v1/welcome/rota/{id}/complete"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{refused}");
    assert_eq!(refused["completed"], json!(false));
    assert!(
        refused["reason"]
            .as_str()
            .is_some_and(|r| r.contains(&work)),
        "the refusal names the work: {refused}"
    );

    // The reader writes the comment the rota asked for; now the same door closes it.
    comment_as(db_of(&h.tdb), &work, &pseud).await;
    let (status, done) = reader
        .post(&format!("/api/v1/welcome/rota/{id}/complete"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["completed"], json!(true));
    assert!(done["assignment"]["completed_at"].is_string());

    let (status, mine) = reader.get("/api/v1/welcome/rota").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mine["open"], json!(0), "{mine}");
    assert_eq!(mine["assignments"][0]["completed_at"].is_string(), true);
}

#[tokio::test]
async fn another_readers_welcome_is_a_404() {
    let h = Harness::new("notyours").await;
    let mut first = h.client();
    let (_a, pseud) = first.register("owner@example.invalid", "owner").await;
    first.activate(&pseud).await;
    let (status, claimed) = first.post("/api/v1/welcome/rota/claim", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{claimed}");
    let id = claimed["assignment"]["id"].as_str().expect("id").to_owned();

    let mut other = h.client();
    let (_b, other_pseud) = other.register("other@example.invalid", "other").await;
    other.activate(&other_pseud).await;
    let (status, body) = other
        .post(&format!("/api/v1/welcome/rota/{id}/complete"), json!({}))
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "somebody else's welcome does not exist for this caller: {body}"
    );
}

#[tokio::test]
async fn the_module_is_registered_and_carries_its_nav_entry() {
    let h = Harness::new("module").await;
    let mut anon = h.client();
    let (status, body) = anon.get("/api/v1/modules").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let welcome = body["modules"]
        .as_array()
        .expect("modules")
        .iter()
        .find(|m| m["id"] == json!("welcome"))
        .unwrap_or_else(|| panic!("welcome is not in the registry payload: {body}"));
    assert_eq!(welcome["enabled"], json!(true));
    assert_eq!(welcome["toggle"], json!("per_user"));
    let nav = welcome["nav"].as_array().expect("nav entries");
    assert!(
        nav.iter().any(|n| n["href"] == json!("/welcome")),
        "the sidebar entry is what makes the surface reachable: {welcome}"
    );
}
