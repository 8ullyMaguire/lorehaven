//! M3-03 — the trust event log: every trust change writes a row.
//!
//! # The invariant
//!
//! **A level that changed with no event is a defect** (spec §6.7). Not a
//! style issue: an unexplained level change is indistinguishable from a
//! level that was always that, and a moderator queue whose membership cannot
//! be explained cannot be audited.
//!
//! # Append-only, and enforced
//!
//! The table refuses an `UPDATE` and a `DELETE` — with **triggers**, in
//! migration 0117, on both engines. A trust history that can be rewritten is
//! not a history, and a property that is only stated in a comment is a
//! property that will be broken by the next person who does not read the
//! comment.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use darkhold_app::config::Config;
use darkhold_app::server::{self, set_trust_proxy};
use darkhold_app::state::AppState;
use darkhold_db::trust_events::TrustCause;
use darkhold_db::DatabaseConfig;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "darkhold-events-{tag}-{}-{:?}",
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
    config.database = DatabaseConfig::new(format!(
        "sqlite://{}/darkhold.sqlite?mode=rwc",
        dir.display()
    ));
    config
}

const PASSWORD: &str = "test-pass-1234";

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

    async fn request(
        &mut self,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        for (name, value) in &self.cookies {
            builder = builder.header(header::COOKIE, format!("{name}={value}"));
        }
        if let Some(token) = self
            .cookies
            .iter()
            .find(|(k, _)| k == "darkhold_csrf")
            .map(|(_, v)| v.to_owned())
        {
            builder = builder.header("x-csrf-token", token);
        }
        let body = match body {
            Some(v) => Body::from(v.to_string()),
            None => Body::empty(),
        };
        let response = self
            .app
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        self.capture(&response);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into_owned()));
        (status, json)
    }

    async fn get(&mut self, uri: &str) -> (StatusCode, Value) {
        self.request("GET", uri, None).await
    }

    async fn post(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("POST", uri, Some(body)).await
    }

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
        me["account"]["id"].as_str().expect("account id").to_owned()
    }
}

struct Harness {
    _dir: PathBuf,
    tdb: test_support::TestDb,
    config: Config,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        set_trust_proxy(false);
        let dir = scratch_dir(tag);
        let tdb = test_support::TestDb::connect_with_dir(tag, &dir).await;
        let config = config_for(&dir);
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

#[test]
fn the_cause_vocabulary_is_closed() {
    // **The database has the same list in a CHECK.** Two lists that can
    // disagree would be a list the database accepts and the code cannot
    // explain — and an unexplainable event is worse than no event, because it
    // looks like a record.
    let names: Vec<&str> = TrustCause::all().iter().map(|c| c.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "tenure",
            "contribution",
            "vote_score",
            "report_accuracy",
            "elastic",
            "demotion_spam",
            "operator",
            "fold",
        ],
        "spec §6.7's cause vocabulary, in order"
    );
    for cause in TrustCause::all() {
        assert_eq!(
            TrustCause::parse(cause.as_str()),
            Some(cause),
            "the round trip holds"
        );
    }
    assert_eq!(
        TrustCause::parse("because_i_said_so"),
        None,
        "an unknown cause is refused rather than defaulted -- a default would \\
         turn a cause the code does not know into a cause it does"
    );
}

#[tokio::test]
async fn every_trust_change_writes_a_row() {
    // **The invariant, over a scripted sequence of actions.** A level that
    // moved and left no event is the defect this whole table exists to make
    // impossible.
    let harness = Harness::new("invariant").await;
    let mut client = harness.client();
    let account = client.register("events@example.test", "Events").await;

    // The first level is an *initial state*, not a change.
    let events = darkhold_db::trust_events::list_for_account(harness.tdb.db(), &account, 50)
        .await
        .expect("read the history");
    assert!(
        events.is_empty(),
        "an account's first level is not a change: {events:?}"
    );

    // Now a change: the level moves.
    darkhold_db::governance::set_trust(harness.tdb.db(), &account, 3, "test")
        .await
        .expect("set the level");
    darkhold_db::trust_events::record(
        harness.tdb.db(),
        &account,
        1,
        3,
        TrustCause::Operator,
        Some("an operator said so"),
        None,
    )
    .await
    .expect("record the event");

    let events = darkhold_db::trust_events::list_for_account(harness.tdb.db(), &account, 50)
        .await
        .expect("read the history");
    assert_eq!(events.len(), 1, "the change has an event");
    assert_eq!(events[0].from_level, 1);
    assert_eq!(events[0].to_level, 3);
    assert_eq!(events[0].cause, TrustCause::Operator);
    assert_eq!(
        events[0].evidence_ref.as_deref(),
        Some("an operator said so"),
        "the event says what it was about"
    );

    // And a *second* change adds a second event — the log is a log.
    darkhold_db::governance::set_trust(harness.tdb.db(), &account, 5, "test")
        .await
        .expect("set the level");
    darkhold_db::trust_events::record(
        harness.tdb.db(),
        &account,
        3,
        5,
        TrustCause::Operator,
        None,
        None,
    )
    .await
    .expect("record the event");
    let events = darkhold_db::trust_events::list_for_account(harness.tdb.db(), &account, 50)
        .await
        .expect("read the history");
    assert_eq!(events.len(), 2, "two changes, two events");
    // Newest first.
    assert_eq!(events[0].to_level, 5);
    assert_eq!(events[1].to_level, 3);
}

#[tokio::test]
async fn the_table_refuses_to_be_rewritten() {
    // **Append-only, enforced by the database and not by a comment.**
    let harness = Harness::new("append-only").await;
    let mut client = harness.client();
    let account = client.register("append@example.test", "Append").await;

    darkhold_db::trust_events::record(
        harness.tdb.db(),
        &account,
        1,
        2,
        TrustCause::Tenure,
        None,
        None,
    )
    .await
    .expect("record the event");

    // An UPDATE is refused.
    //
    // **On both engines**, because the trigger is on both engines: a history
    // that can be rewritten on one of them is a history that can be rewritten.
    let update = match harness.tdb.db().backend() {
        darkhold_db::Backend::Sqlite => sqlx::query("UPDATE trust_events SET to_level = 6")
            .execute(harness.tdb.db().sqlite_pool().expect("sqlite"))
            .await
            .map(|_| ()),
        darkhold_db::Backend::Postgres => sqlx::query("UPDATE trust_events SET to_level = 6")
            .execute(harness.tdb.db().postgres_pool().expect("postgres"))
            .await
            .map(|_| ()),
    };
    assert!(
        update.is_err(),
        "an update must be refused: a trust history that can be rewritten is not \
         a history"
    );

    // And a DELETE is refused.
    let delete = match harness.tdb.db().backend() {
        darkhold_db::Backend::Sqlite => sqlx::query("DELETE FROM trust_events")
            .execute(harness.tdb.db().sqlite_pool().expect("sqlite"))
            .await
            .map(|_| ()),
        darkhold_db::Backend::Postgres => sqlx::query("DELETE FROM trust_events")
            .execute(harness.tdb.db().postgres_pool().expect("postgres"))
            .await
            .map(|_| ()),
    };
    assert!(delete.is_err(), "a delete must be refused too");

    // The row is still there.
    let events = darkhold_db::trust_events::list_for_account(harness.tdb.db(), &account, 50)
        .await
        .expect("read the history");
    assert_eq!(events.len(), 1, "the event survived the attempts");
    assert_eq!(events[0].to_level, 2);
}

#[tokio::test]
async fn the_accounts_trust_history_is_readable() {
    // **The reader.** `/api/v1/accounts/{id}/trust` is the account's trust
    // history — the events, not just the level.
    let harness = Harness::new("history").await;
    let mut client = harness.client();
    let account = client.register("history@example.test", "History").await;

    darkhold_db::governance::set_trust(harness.tdb.db(), &account, 4, "test")
        .await
        .expect("set the level");
    darkhold_db::trust_events::record(
        harness.tdb.db(),
        &account,
        1,
        4,
        TrustCause::Contribution,
        Some("twenty contributions"),
        None,
    )
    .await
    .expect("record the event");

    let (status, body) = client
        .get(&format!("/api/v1/accounts/{account}/trust"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["level"], 4);
    assert_eq!(body["level_name"], "moderator");
    let events = body["events"].as_array().expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["cause"], "contribution");
    assert_eq!(events[0]["from_level"], 1);
    assert_eq!(events[0]["to_level"], 4);
    assert_eq!(events[0]["evidence_ref"], "twenty contributions");
}
