//! Acceptance: the retention *proposal* surface for readers (spec §5,
//! amendment §5; the plan's E.4).
//!
//! `retention_proposals.rs` proves the store. `retention_routes.rs` proves the
//! operator's four admin routes. This file proves the four reader routes, and
//! two of its tests exist for reasons that have nothing to do with coverage:
//!
//! - **the ballots do not leak.** A governance vote that records who voted is a
//!   record of what a reader wanted, attached to their account, readable by
//!   everyone who can open a proposal. §45.2's argument — a reading habit must
//!   not set instance policy — applies to a reader's *choice* as much as to
//!   their weight, and it arrives by a different route: a weight changes what a
//!   ballot counts, a leak changes what it reveals. The store cannot leak,
//!   because its `tally` returns two integers and no row type. The routes could
//!   still leak by serialising a `Ballot` they do not have, so the test asserts
//!   the *absence* of every plausible field name in the bytes, which is the
//!   shape of assertion that survives a refactor. See
//!   `no_response_exposes_a_ballot_or_its_voter`.
//!
//! - **the refusal names the bar.**
//!   `a_reader_below_the_bar_cannot_open_or_file_a_proposal_and_is_told_the_bar`
//!   is the plan's own name and its second clause is the part that is easy to
//!   drop. "trust level too low" is true and useless; a reader who is told what
//!   would be enough can decide whether to wait, ask, or go elsewhere.

use std::path::Path;

use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_db::Database;
use serde_json::{json, Value};
use test_support::TestDb;
use tower::ServiceExt;

const GOOD_PASSWORD: &str = "a-long-enough-passphrase";

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-retention-proposals-{tag}-{}-{:?}",
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
    // Honoured, so this file runs on whichever backend the selector names — the
    // same note as `retention_routes.rs`, which found a hardcoded SQLite URL
    // here once already.
    config.database = match std::env::var("LOREHAVEN_TEST_PG_URL") {
        Ok(url) => lorehaven_db::DatabaseConfig::new(url),
        Err(_) => lorehaven_db::DatabaseConfig::new(format!(
            "sqlite://{}/lorehaven.sqlite?mode=rwc",
            dir.display()
        )),
    };
    // The rate-limit buckets are process-global at 127.0.0.1, so the
    // development defaults are exhausted by neighbouring suites long before
    // this file's own requests finish.
    config.rate_limits.auth = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.write = lorehaven_app::limiter::Quota {
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
    fn cookie_header(&self) -> String {
        self.cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn capture(&mut self, response: &axum::response::Response) {
        for value in response.headers().get_all(header::SET_COOKIE) {
            let Ok(raw) = value.to_str() else { continue };
            let pair = raw.split(';').next().unwrap_or(raw);
            let Some((name, val)) = pair.split_once('=') else {
                continue;
            };
            let (name, val) = (name.to_owned(), val.to_owned());
            self.cookies.retain(|(key, _)| key != &name);
            if !val.is_empty() {
                self.cookies.push((name, val));
            }
        }
    }

    async fn send(&mut self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(path);
        if !self.cookies.is_empty() {
            builder = builder.header(header::COOKIE, self.cookie_header());
        }
        let request = match body {
            Some(value) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(axum::body::Body::from(value.to_string()))
                .expect("request"),
            None => builder.body(axum::body::Body::empty()).expect("request"),
        };
        let response = self
            .app
            .clone()
            .oneshot(request)
            .await
            .expect("route response");
        let status = response.status();
        self.capture(&response);
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("response body");
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, value)
    }

    async fn get(&mut self, path: &str) -> (StatusCode, Value) {
        self.send("GET", path, None).await
    }

    async fn post(&mut self, path: &str, body: Value) -> (StatusCode, Value) {
        self.send("POST", path, Some(body)).await
    }
}

struct Harness {
    /// Held so the scratch database lives as long as the harness.
    _tdb: TestDb,
    _dir: std::path::PathBuf,
    config: Config,
    db: Database,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let config = config_for(&dir);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        let db = tdb.db().clone();
        Self {
            _tdb: tdb,
            _dir: dir,
            config,
            db,
        }
    }

    fn client(&self) -> Client {
        Client {
            app: server::build_router(AppState::new(self.config.clone(), self.db.clone())),
            cookies: Vec::new(),
        }
    }

    /// A signed-in reader at `trust` on the retention gate.
    ///
    /// Registered first and trusted afterwards, because a registration cannot
    /// set a trust level and a test that wanted "trust 0" would otherwise be
    /// testing an account that does not exist.
    async fn reader_at(&self, handle: &str, trust: i64) -> Client {
        let mut client = self.client();
        let (status, _body) = client
            .post(
                "/api/v1/auth/register",
                json!({
                    "email": format!("{handle}@example.com"),
                    "password": GOOD_PASSWORD,
                    "handle": handle,
                    "display_name": handle,
                    "age_band": "adult",
                }),
            )
            .await;
        if status != StatusCode::CREATED {
            let (status, body) = client
                .post(
                    "/api/v1/auth/login",
                    json!({
                        "email": format!("{handle}@example.com"),
                        "password": GOOD_PASSWORD,
                    }),
                )
                .await;
            assert!(status.is_success(), "login {handle}: {status} {body}");
        }
        let account = account_of(&self.db, handle).await;
        lorehaven_db::governance::set_trust(&self.db, &account.to_string(), trust, "test")
            .await
            .expect("set trust");
        client
    }

    /// A reader above the default bar.
    async fn reader(&self, handle: &str) -> Client {
        self.reader_at(handle, 3).await
    }
}

async fn account_of(db: &Database, handle: &str) -> uuid::Uuid {
    let sql = match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            "SELECT account_id FROM pseuds WHERE lower(handle) = lower(?) ORDER BY created_at ASC"
        }
        lorehaven_db::Backend::Postgres => {
            "SELECT account_id::text AS account_id FROM pseuds
              WHERE lower(handle) = lower($1) ORDER BY created_at ASC"
        }
    };
    let value: String = match db.backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(sql)
            .bind(handle)
            .fetch_one(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("account"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(sql)
            .bind(handle)
            .fetch_one(db.postgres_pool().expect("postgres"))
            .await
            .expect("account"),
    };
    uuid::Uuid::parse_str(&value).expect("account uuid")
}

/// A proposal body naming an archive and a reason.
fn proposal_body() -> Value {
    json!({
        "proposed_mode": "aggregate",
        "source_key": "archive:example.org",
        "rationale": "the full text is never read and the disk is finite",
    })
}

#[tokio::test]
async fn a_reader_at_the_bar_can_open_a_proposal_and_see_it_in_the_list() {
    let harness = Harness::new("at-bar").await;
    let mut alice = harness.reader("alice").await;

    let (status, body) = alice
        .post("/api/v1/retention/proposals", proposal_body())
        .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "a reader at trust 3 with a bar of 1 may open: {body}"
    );
    assert_eq!(body["proposed_mode"], "aggregate");
    assert_eq!(body["source_key"], "archive:example.org");

    // The list shows it, so the ballot can be joined rather than only guessed at.
    let (status, list) = harness
        .reader("watcher")
        .await
        .get("/api/v1/retention/proposals")
        .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let items = list["items"].as_array().expect("items array");
    assert_eq!(
        items.len(),
        1,
        "the proposal a reader just opened is listed"
    );
    assert_eq!(items[0]["id"], body["id"]);
}

#[tokio::test]
async fn a_reader_below_the_bar_cannot_open_or_file_a_proposal_and_is_told_the_bar() {
    let harness = Harness::new("below-bar").await;
    // The bar is the default 1 and this reader is at 0.
    let mut newcomer = harness.reader_at("newcomer", 0).await;

    let (status, body) = newcomer
        .post("/api/v1/retention/proposals", proposal_body())
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "open refused: {body}"
    );
    let message = body["error"]["message"].as_str().unwrap_or_default();
    // The bar, and the reader's own level. Either alone would be a dead end.
    assert!(
        message.contains("trust level 1"),
        "the refusal names the bar: {message}"
    );
    assert!(
        message.contains("yours is 0"),
        "the refusal names the caller's level: {message}"
    );

    // Reading is gated the same way, and the same message shape.
    let (status, body) = newcomer.get("/api/v1/retention/proposals").await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "list refused: {body}"
    );
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("trust level 1"),
        "the list refusal names the bar too: {body}"
    );
}

#[tokio::test]
async fn a_raised_bar_excludes_a_reader_who_was_under_the_previous_one() {
    let harness = Harness::new("raised-bar").await;
    // Trust 1 is enough by default, so this reader may act — until the
    // operator raises the bar above them. This is the test that makes
    // `proposal_min_trust` a setting rather than a constant: without the
    // config, the only way to fail this is to edit code.
    let mut harness = harness;
    harness.config.retention_governance.proposal_min_trust = 2;
    let mut reader = harness.reader_at("trust-one", 1).await;

    let (status, body) = reader
        .post("/api/v1/retention/proposals", proposal_body())
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "trust 1 is refused once the bar is 2: {body}"
    );
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("trust level 2"), "{message}");
    assert!(message.contains("yours is 1"), "{message}");
}

#[tokio::test]
async fn a_ballot_counts_up_and_changes_a_readers_mind_without_double_counting() {
    let harness = Harness::new("ballot-counts").await;
    let mut alice = harness.reader("alice").await;
    let (_status, proposal) = alice
        .post("/api/v1/retention/proposals", proposal_body())
        .await;
    let id = proposal["id"].as_str().expect("id").to_owned();

    let mut bob = harness.reader("bob").await;
    let (status, body) = bob
        .post(
            &format!("/api/v1/retention/proposals/{id}/vote"),
            json!({ "support": true }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["tally"]["supporters"], 1, "{body}");
    assert_eq!(body["tally"]["opposed"], 0, "{body}");

    // Bob changes his mind. He must still count once, and be counted as opposed.
    let (status, body) = bob
        .post(
            &format!("/api/v1/retention/proposals/{id}/vote"),
            json!({ "support": false }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["tally"]["supporters"], 0,
        "a changed ballot does not leave the old one behind: {body}"
    );
    assert_eq!(body["tally"]["opposed"], 1, "{body}");
}

#[tokio::test]
async fn no_response_exposes_a_ballot_or_its_voter() {
    let harness = Harness::new("no-leak").await;
    let mut alice = harness.reader("alice").await;
    let (_status, proposal) = alice
        .post("/api/v1/retention/proposals", proposal_body())
        .await;
    let id = proposal["id"].as_str().expect("id").to_owned();
    let alice_account = account_of(&harness.db, "alice").await.to_string();

    let mut bob = harness.reader("bob").await;
    let mut carol = harness.reader("carol").await;
    for client in [&mut bob, &mut carol] {
        let (status, body) = client
            .post(
                &format!("/api/v1/retention/proposals/{id}/vote"),
                json!({ "support": true }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    // Every surface a reader can reach, with three real ballots behind it.
    let mut reader = harness.reader("watcher").await;
    let (list_status, list) = reader.get("/api/v1/retention/proposals").await;
    assert_eq!(list_status, StatusCode::OK, "{list}");
    let (one_status, one) = reader
        .get(&format!("/api/v1/retention/proposals/{id}"))
        .await;
    assert_eq!(one_status, StatusCode::OK, "{one}");

    // The counts are there — that is what a reader needs in order to decide
    // whether to join, and a proposal whose support is invisible is useless.
    let tally = &one["tally"];
    assert_eq!(tally["supporters"], 2, "two supporters counted: {one}");
    assert_eq!(tally["opposed"], 0, "{one}");

    // And nothing that names a voter. Checked on the *serialised text*, not on
    // a whitelist of fields, because a leak would arrive as a field this file
    // has never heard of: a `voters` array, a `supporters` list, a `ballots`
    // key. Every key a leak would plausibly use is refused here, and the
    // accounts themselves are refused, so a leak under a name nobody guessed
    // still fails on the account ids.
    for (label, body) in [("list", &list), ("one", &one)] {
        let text = body.to_string();
        for forbidden in [
            "ballot",
            "voter",
            "voters",
            "voted",
            "supporters_by",
            "account",
            "account_id",
            "opened_by",
            "handle",
        ] {
            assert!(
                !text.contains(forbidden),
                "the {label} response names `{forbidden}`, which would reveal a ballot \
                 or its voter: {text}"
            );
        }
        for account in [
            alice_account.as_str(),
            &account_of(&harness.db, "bob").await.to_string(),
            &account_of(&harness.db, "carol").await.to_string(),
        ] {
            assert!(
                !text.contains(account),
                "the {label} response contains a voter's account id: {text}"
            );
        }
    }
}

#[tokio::test]
async fn a_second_proposal_on_the_same_setting_is_a_conflict_and_the_first_is_untouched() {
    let harness = Harness::new("one-open").await;
    let mut alice = harness.reader("alice").await;
    let (status, first) = alice
        .post("/api/v1/retention/proposals", proposal_body())
        .await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    let first_id = first["id"].as_str().expect("id").to_owned();

    let mut bob = harness.reader("bob").await;
    let (status, body) = bob
        .post("/api/v1/retention/proposals", proposal_body())
        .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "one open ballot per setting, and the status says so: {body}"
    );
    assert_eq!(body["error"]["code"], "CONFLICT", "{body}");

    // A 409 that quietly created a second row would be worse than no 409, so
    // the count is asserted rather than assumed.
    let mut watcher = harness.reader("watcher").await;
    let (_status, list) = watcher.get("/api/v1/retention/proposals").await;
    assert_eq!(
        list["items"].as_array().expect("items").len(),
        1,
        "the refused proposal left nothing behind: {list}"
    );
    let (_status, original) = watcher
        .get(&format!("/api/v1/retention/proposals/{first_id}"))
        .await;
    assert_eq!(
        original["rationale"], first["rationale"],
        "the first proposal was not rewritten by the refused second: {original}"
    );
}

#[tokio::test]
async fn a_bad_mode_and_an_empty_rationale_are_refused_by_name() {
    let harness = Harness::new("bad-input").await;
    let mut alice = harness.reader("alice").await;

    let (status, body) = alice
        .post(
            "/api/v1/retention/proposals",
            json!({
                "proposed_mode": "aggregte",
                "source_key": "archive:example.org",
                "rationale": "a typo",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("aggregte")
            && (message.contains("cache") || message.contains("aggregate")),
        "the refusal echoes the bad value and names the good ones: {message}"
    );

    let (status, body) = alice
        .post(
            "/api/v1/retention/proposals",
            json!({
                "proposed_mode": "aggregate",
                "source_key": "archive:example.org",
                "rationale": "   ",
            }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a reason is required: {body}"
    );
}

#[tokio::test]
async fn an_anonymous_caller_is_refused_and_learns_nothing_about_the_proposals() {
    let harness = Harness::new("anon").await;
    let mut alice = harness.reader("alice").await;
    let (_status, _) = alice
        .post("/api/v1/retention/proposals", proposal_body())
        .await;

    let mut anon = harness.client();
    for (method, path) in [
        ("GET", "/api/v1/retention/proposals"),
        ("POST", "/api/v1/retention/proposals"),
    ] {
        let (status, body) = anon
            .send(method, path, (method == "POST").then(proposal_body))
            .await;
        assert!(
            status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
            "{method} {path} without a session: {status} {body}"
        );
        assert!(
            body.to_string().contains("aggregate") == false,
            "a refusal must not echo a proposal's contents: {body}"
        );
    }
}
