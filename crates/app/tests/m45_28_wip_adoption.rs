//! M45-28 — §56 WIP adoption through the real router.
//!
//! The store is tested in `darkhold-db`; what only the route can get wrong:
//!
//! - every write door needs a session, and the adoption door needs an active pseud;
//! - a work somebody else owns answers `404`, because "not yours" about a private act should not
//!   confirm the work exists (an unowned work is the same answer);
//! - a `complete` work is refused on the field `completion`, in words;
//! - the contributor door answers `available: true` with a list on a migrated schema, and never a
//!   `500` when the tables are gone;
//! - a closure note is `404` before the first one, then versions 1, 2;
//! - **adopting does not change the work's owner**, read through the API;
//! - the module is registered, enabled, and has no nav entry (its doors live on a work page).

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use darkhold_app::config::Config;
use darkhold_app::server;
use darkhold_app::state::AppState;
use serde_json::{json, Value};
use test_support::TestDb;
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery staple 7";

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "darkhold-m45-28-{tag}-{}-{:?}",
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

    async fn delete(&mut self, uri: &str) -> (StatusCode, Value) {
        self.request("DELETE", uri, None).await
    }

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
}

impl Harness {
    async fn new(tag: &str) -> Self {
        Self::with_bar(tag, None).await
    }

    /// `bar`: the configured adoption bar. `None` keeps the shipped default.
    async fn with_bar(tag: &str, bar: Option<i64>) -> Self {
        let dir = scratch_dir(tag);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        let mut config = config_for(&dir);
        if let Some(bar) = bar {
            config.adoption.min_trust_level = bar;
        }
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
}

/// A work row with a chosen owner and completion state. Written directly because publishing a
/// work is not a route in this build (M45-25's premise table says the same).
async fn seed_work(h: &Harness, owner_pseud: &str, completion: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let db = h.tdb.db();
    use darkhold_db::Backend;
    let t0 = "2026-03-01T00:00:00Z";
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO works (id, owner_pseud_id, title, completion, created_at, updated_at,
                                    generated_content_posture)
                 VALUES (?, ?, 'A WIP', ?, ?, ?, 'forbid')",
            )
            .bind(&id)
            .bind(owner_pseud)
            .bind(completion)
            .bind(t0)
            .bind(t0)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("seed work");
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO works (id, owner_pseud_id, title, completion, created_at, updated_at,
                                    generated_content_posture)
                 VALUES ($1::uuid, $2::uuid, 'A WIP', $3, $4::timestamptz, $4::timestamptz, 'forbid')",
            )
            .bind(uuid::Uuid::parse_str(&id).expect("uuid"))
            .bind(uuid::Uuid::parse_str(owner_pseud).expect("uuid"))
            .bind(completion)
            .bind(t0)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("seed work");
        }
    }
    id
}

async fn owner_of(h: &Harness, work_id: &str) -> String {
    let db = h.tdb.db();
    use darkhold_db::Backend;
    use sqlx::Row;
    match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query("SELECT owner_pseud_id FROM works WHERE id = ?")
                .bind(work_id)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("owner");
            row.get::<String, _>("owner_pseud_id")
        }
        Backend::Postgres => {
            let row = sqlx::query("SELECT owner_pseud_id::text AS owner FROM works WHERE id::text = $1")
                .bind(work_id)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await
                .expect("owner");
            row.get::<String, _>("owner")
        }
    }
}

#[tokio::test]
async fn every_write_door_needs_a_session() {
    let h = Harness::new("anon").await;
    let mut anon = h.client();

    for (method, uri) in [
        ("POST", "/api/v1/works/whatever/adoption-offer"),
        ("DELETE", "/api/v1/works/whatever/adoption-offer"),
        ("POST", "/api/v1/adoption-offers/whatever/adopt"),
        ("POST", "/api/v1/works/whatever/closure-note"),
    ] {
        let (status, body) = anon.request(method, uri, Some(json!({}))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri}: {body}");
    }

    // The reads are public.
    let (status, _) = anon.get("/api/v1/adoption-offers").await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = anon.get("/api/v1/works/whatever/contributors").await;
    assert_eq!(status, StatusCode::OK, "lineage is public");
}

#[tokio::test]
async fn the_module_is_registered_enabled_and_has_no_nav_entry() {
    let h = Harness::new("module").await;
    let mut anon = h.client();
    let (status, body) = anon.get("/api/v1/modules").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let modules = body["modules"].as_array().expect("modules");
    let row = modules
        .iter()
        .find(|m| m["id"] == json!("wip_adoption"))
        .unwrap_or_else(|| panic!("wip_adoption is not registered"));
    assert_eq!(row["enabled"], json!(true), "default_on must hold on a default config");
    assert_eq!(
        row["nav"],
        json!([]),
        "the doors sit on a work page; a nav entry would point at a page that does not exist"
    );
}

#[tokio::test]
async fn an_offer_on_somebody_elses_work_is_404_and_on_a_complete_work_names_the_field() {
    let h = Harness::new("refusals").await;
    let mut author = h.client();
    let (_a, author_pseud) = author.register("author@example.invalid", "author").await;
    author.activate(&author_pseud).await;
    let abandoned = seed_work(&h, &author_pseud, "abandoned").await;
    let complete = seed_work(&h, &author_pseud, "complete").await;

    let mut stranger = h.client();
    let (_s, stranger_pseud) = stranger.register("stranger@example.invalid", "stranger").await;
    stranger.activate(&stranger_pseud).await;

    // Not the owner: the same answer as a work that does not exist.
    let (status, body) = stranger
        .post(&format!("/api/v1/works/{abandoned}/adoption-offer"), json!({}))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    // The owner, on a work that is not abandoned.
    let (status, body) = author
        .post(&format!("/api/v1/works/{complete}/adoption-offer"), json!({}))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], json!("VALIDATION_FAILED"));
    assert!(
        body["error"]["field_errors"]["completion"].is_string(),
        "the refusal must name `completion`: {body}"
    );

    // And the abandoned one opens.
    let (status, body) = author
        .post(&format!("/api/v1/works/{abandoned}/adoption-offer"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["offer"]["status"], json!("open"));
}

#[tokio::test]
async fn adopting_leaves_the_owner_alone_and_empties_the_list() {
    // A freshly registered account is trust level 1 (measured), so this test moves the bar to 1
    // to exercise the adoption itself; the refusal at the shipped default is its own test below.
    let h = Harness::with_bar("adopt", Some(1)).await;
    let mut author = h.client();
    let (_a, author_pseud) = author.register("author2@example.invalid", "author2").await;
    author.activate(&author_pseud).await;
    let work_id = seed_work(&h, &author_pseud, "abandoned").await;

    let (status, body) = author
        .post(&format!("/api/v1/works/{work_id}/adoption-offer"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let offer_id = body["offer"]["id"].as_str().expect("offer id").to_owned();

    let (status, listing) = author.get("/api/v1/adoption-offers").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listing["count"], json!(1), "{listing}");

    let mut adopter = h.client();
    let (_x, adopter_pseud) = adopter.register("adopter@example.invalid", "adopter").await;
    adopter.activate(&adopter_pseud).await;

    let before = owner_of(&h, &work_id).await;
    let (status, body) = adopter
        .post(&format!("/api/v1/adoption-offers/{offer_id}/adopt"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["offer"]["status"], json!("adopted"));
    assert_eq!(
        body["offer"]["adopted_by_pseud_id"],
        json!(adopter_pseud),
        "the adopter is recorded on the offer"
    );

    assert_eq!(
        owner_of(&h, &work_id).await,
        before,
        "adoption moved works.owner_pseud_id — §56.1 says it does not"
    );
    let (_, listing) = author.get("/api/v1/adoption-offers").await;
    assert_eq!(listing["count"], json!(0), "an adopted offer stayed on the list");
}

#[tokio::test]
async fn a_closure_note_is_404_then_one_then_two() {
    let h = Harness::new("notes").await;
    let mut author = h.client();
    let (_a, author_pseud) = author.register("author3@example.invalid", "author3").await;
    author.activate(&author_pseud).await;
    let work_id = seed_work(&h, &author_pseud, "complete").await;

    let (status, body) = author
        .get(&format!("/api/v1/works/{work_id}/closure-note"))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"]["code"], json!("NOT_FOUND"));

    let (status, body) = author
        .post(
            &format!("/api/v1/works/{work_id}/closure-note"),
            json!({ "body": "it ends here" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["revision_number"], json!(1));

    let (status, body) = author
        .post(
            &format!("/api/v1/works/{work_id}/closure-note"),
            json!({ "body": "it ends there instead" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["revision_number"], json!(2), "the second note must not overwrite the first");

    let (status, body) = author
        .get(&format!("/api/v1/works/{work_id}/closure-note"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["note"]["body"], json!("it ends there instead"));
}

#[tokio::test]
async fn an_empty_closure_note_is_refused_on_the_field_body() {
    let h = Harness::new("emptynote").await;
    let mut author = h.client();
    let (_a, author_pseud) = author.register("author4@example.invalid", "author4").await;
    author.activate(&author_pseud).await;
    let work_id = seed_work(&h, &author_pseud, "complete").await;

    let (status, body) = author
        .post(
            &format!("/api/v1/works/{work_id}/closure-note"),
            json!({ "body": "   " }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(body["error"]["field_errors"]["body"].is_string(), "{body}");
}

#[tokio::test]
async fn a_fresh_account_is_below_the_shipped_bar_and_the_refusal_names_both_numbers() {
    let h = Harness::new("belowbar").await;
    let mut author = h.client();
    let (_a, author_pseud) = author.register("author6@example.invalid", "author6").await;
    author.activate(&author_pseud).await;
    let work_id = seed_work(&h, &author_pseud, "abandoned").await;
    let (status, body) = author
        .post(&format!("/api/v1/works/{work_id}/adoption-offer"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let offer_id = body["offer"]["id"].as_str().expect("offer id").to_owned();

    let mut fresh = h.client();
    let (_f, fresh_pseud) = fresh.register("fresh@example.invalid", "fresh").await;
    fresh.activate(&fresh_pseud).await;

    let (status, body) = fresh
        .post(&format!("/api/v1/adoption-offers/{offer_id}/adopt"), json!({}))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let reason = body["error"]["field_errors"]["trust"]
        .as_str()
        .expect("the refusal is on the field `trust`");
    assert!(
        reason.contains("requires 2") && reason.contains("at 1"),
        "the refusal must name the bar and the caller's level, got {reason:?}"
    );

    // Nothing was adopted.
    let (_, listing) = author.get("/api/v1/adoption-offers").await;
    assert_eq!(listing["count"], json!(1), "{listing}");
}

#[tokio::test]
async fn the_author_withdraws_and_can_offer_again() {
    let h = Harness::new("withdraw").await;
    let mut author = h.client();
    let (_a, author_pseud) = author.register("author5@example.invalid", "author5").await;
    author.activate(&author_pseud).await;
    let work_id = seed_work(&h, &author_pseud, "abandoned").await;

    let (status, _) = author
        .post(&format!("/api/v1/works/{work_id}/adoption-offer"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = author
        .delete(&format!("/api/v1/works/{work_id}/adoption-offer"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["withdrawn"], json!(true));

    // Nothing open to withdraw now.
    let (status, body) = author
        .delete(&format!("/api/v1/works/{work_id}/adoption-offer"))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    // And the partial index lets the author think again.
    let (status, body) = author
        .post(&format!("/api/v1/works/{work_id}/adoption-offer"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "a second offer after a withdrawal: {body}");
}

#[tokio::test]
async fn lineage_is_readable_and_never_a_server_error() {
    let h = Harness::new("lineage").await;
    let mut anon = h.client();

    let (status, body) = anon.get("/api/v1/works/anything/contributors").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["available"], json!(true), "{body}");
    assert_eq!(body["contributors"], json!([]), "a work with no revisions has no contributors");
    assert_eq!(body["reason"], json!(null));
}
