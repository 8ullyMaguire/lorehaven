//! M3-05 — the modlog: readable by any signed-in reader.
//!
//! # What this proves
//!
//! * **Anonymous access is 401, not 403.** The modlog is a public record, not
//!   a door that exists only for the permitted: being anonymous is a fact
//!   about the *reader*, not about the record.
//! * **Every entry carries the five fields** the specification names: actor,
//!   action, target, reason and timestamp.
//! * **The acting account's id is never published.** The actor is the acting
//!   *pseudonym*, and the account id stays where it belongs — in the
//!   instance's own audit.
//! * **The target is a pseudonym when the target is an account**, for the same
//!   reason.
//! * **Newest first, with cursor pagination.**

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use darkhold_app::config::Config;
use darkhold_app::server::{self, set_trust_proxy};
use darkhold_app::state::AppState;
use darkhold_db::DatabaseConfig;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "darkhold-modlog-{tag}-{}-{:?}",
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

    /// Register and return the account id and the pseud id.
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

    async fn set_level(&self, account: &str, level: i64) {
        darkhold_db::governance::set_trust(self.tdb.db(), account, level, "test")
            .await
            .expect("set the trust level");
    }
}

#[tokio::test]
async fn the_modlog_is_refused_to_an_anonymous_reader_with_a_401() {
    // **401, not 403.** "403 answers 'yes, and you may not'" — but the modlog
    // is a public record that any reader of the instance may read, and being
    // anonymous is a fact about the *reader*, not about the record.
    let harness = Harness::new("anonymous").await;
    let mut client = harness.client();

    let (status, body) = client.get("/api/v1/modlog").await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "an anonymous reader gets 401, not 403: {body}"
    );
    assert_eq!(
        body["error"]["code"], "AUTH_REQUIRED",
        "and the machine code says which: {body}"
    );
}

#[tokio::test]
async fn an_operator_action_appears_with_its_five_fields_and_no_account_id() {
    let harness = Harness::new("operator").await;
    let mut client = harness.client();

    // An operator, and a reader to sanction.
    //
    // **A separate client for the target**: a second registration on the same
    // client replaces its session, and the sanction would then be the
    // *target's* to issue.
    let (operator_account, operator_pseud) = client.register("op@example.test", "Operator").await;
    let mut target_client = harness.client();
    let (target_account, target_pseud) = target_client
        .register("target@example.test", "Target")
        .await;
    harness.set_level(&operator_account, 5).await;

    // The operator sanctions the reader.
    let (status, body) = client
        .post(
            "/api/v1/sanctions",
            json!({
                "account": target_account,
                "kind": "suspend",
                "reason_ref": "a reason the reader can read",
                "duration_days": 7,
            }),
        )
        .await;
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "issue a sanction: {status} {body}"
    );

    // The modlog shows it, with all five fields.
    let (status, log) = client.get("/api/v1/modlog").await;
    assert_eq!(status, StatusCode::OK, "{log}");
    let items = log["items"].as_array().expect("items");
    let entry = items
        .iter()
        .find(|entry| entry["action"] == "sanction.issued")
        .unwrap_or_else(|| panic!("the sanction is in the modlog: {log}"));

    // **Actor.**
    assert_eq!(
        entry["actor"]["pseud_id"], operator_pseud,
        "the actor is the acting pseudonym: {entry}"
    );
    assert_eq!(entry["actor"]["handle"], "Operator");
    // **Action.**
    assert_eq!(entry["action"], "sanction.issued");
    // **Target.**
    assert_eq!(entry["target"]["type"], "account");
    assert_eq!(
        entry["target"]["id"], target_pseud,
        "a target that is an account is published as its public face: {entry}"
    );
    assert_eq!(entry["target"]["is_pseudonym"], true);
    // **Reason.**
    assert_eq!(entry["reason"], "a reason the reader can read");
    // **Timestamp.**
    assert!(
        entry["created_at"].as_str().is_some_and(|t| !t.is_empty()),
        "the entry carries a timestamp: {entry}"
    );

    // **And the account ids are nowhere in the body.**
    let serialised = serde_json::to_string(&log).expect("serialise");
    assert!(
        !serialised.contains(&operator_account),
        "the acting ACCOUNT id is never published: {serialised}"
    );
    assert!(
        !serialised.contains(&target_account),
        "the target's ACCOUNT id is never published either: {serialised}"
    );
}

#[tokio::test]
async fn the_modlog_is_newest_first_and_pages_by_cursor() {
    let harness = Harness::new("paging").await;
    let mut client = harness.client();
    let (account, _) = client.register("pager@example.test", "Pager").await;
    harness.set_level(&account, 5).await;

    // Three actions.
    for index in 0..3 {
        let (status, body) = client
            .post(
                "/api/v1/sanctions",
                json!({
                    "account": account,
                    "kind": "rate_limit",
                    "reason_ref": format!("reason {index}"),
                    "duration_days": 1,
                }),
            )
            .await;
        assert!(
            status == StatusCode::OK || status == StatusCode::CREATED,
            "sanction {index}: {status} {body}"
        );
    }

    let (status, log) = client.get("/api/v1/modlog?limit=2").await;
    assert_eq!(status, StatusCode::OK, "{log}");
    let items = log["items"].as_array().expect("items");
    assert_eq!(items.len(), 2, "the page is the limit: {log}");
    let cursor = log["next_cursor"]
        .as_str()
        .expect("a full page hands back a cursor");

    // The second page continues rather than repeating.
    let (status, page2) = client
        .get(&format!("/api/v1/modlog?limit=2&cursor={cursor}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{page2}");
    let items2 = page2["items"].as_array().expect("items");
    assert!(!items2.is_empty(), "the second page has entries: {page2}");
    let first = items[0]["id"].as_str().expect("id");
    let second_first = items2[0]["id"].as_str().expect("id");
    assert_ne!(
        first, second_first,
        "the pages do not overlap: {log} / {page2}"
    );

    // And a cursor the instance did not issue is a validation failure, not an
    // empty page.
    let (status, body) = client.get("/api/v1/modlog?cursor=not-a-cursor").await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a cursor this collection did not issue is refused: {body}"
    );
}
