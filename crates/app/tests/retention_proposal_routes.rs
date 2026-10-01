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
            !body.to_string().contains("aggregate"),
            "a refusal must not echo a proposal's contents: {body}"
        );
    }
}

// ---- E.2 additions: an operator, the setting, and the settlement clock ----

impl Harness {
    /// A harness whose operator account is already named, so `require_operator`
    /// passes. The account is registered before the config is finalised,
    /// because `require_operator` compares the session's account against
    /// `config.administration.operator_account_id` — not a trust tier — so the
    /// id has to exist before anything can name it.
    async fn with_operator(tag: &str) -> Self {
        let mut harness = Self::new(tag).await;
        let mut bootstrap = harness.client();
        let (status, body) = bootstrap
            .post(
                "/api/v1/auth/register",
                json!({
                    "email": "operator@example.com",
                    "password": GOOD_PASSWORD,
                    "handle": "operator",
                    "display_name": "operator",
                    "age_band": "adult",
                }),
            )
            .await;
        assert!(
            status == StatusCode::CREATED,
            "bootstrap register: {status} {body}"
        );
        let account = account_of(&harness.db, "operator").await;
        harness.config.administration.operator_account_id = Some(account.into());
        harness
    }

    /// The same, with a zero-day cooling window.
    ///
    /// `settle` only acts on proposals whose window has *closed*, and the default
    /// `proposal_cooling_days` is 7 — so a proposal opened moments ago is
    /// correctly not yet due, and a test about governance would end up
    /// asserting about the clock instead. A 0-day window is a supported
    /// configuration (an instance that wants a ballot settleable as soon as it
    /// closes can say so), so this is the test *using* a setting rather than
    /// bending the code. It also keeps these tests on the ordinary `settle`
    /// route, which is the same path the operator's button and the scheduled job
    /// both take.
    ///
    /// The one test that is *about* the delay does not use this; it keeps the
    /// 7-day default and drives `run_at` with a clock on either side of
    /// `closes_at`.
    async fn with_operator_now(tag: &str) -> Self {
        let mut harness = Self::with_operator(tag).await;
        harness.config.retention_governance.proposal_cooling_days = 0;
        harness
    }

    /// The operator, signed in.
    async fn operator(&self) -> Client {
        let mut client = self.client();
        let (status, body) = client
            .post(
                "/api/v1/auth/login",
                json!({ "email": "operator@example.com", "password": GOOD_PASSWORD }),
            )
            .await;
        assert!(status.is_success(), "operator login: {status} {body}");
        client
    }

    /// The mode in force for a scope, read straight from the store.
    ///
    /// Read from the store rather than from `GET /admin/retention/policy`
    /// because the tests are about what the settlement pass *wrote*, and a
    /// round trip through a route that has its own defaulting would make a
    /// "the setting did not change" assertion pass for the wrong reason.
    async fn mode_for(&self, source_key: Option<&str>) -> String {
        let resolved =
            lorehaven_db::retention::resolve_for_source(&self.db, source_key, false, false)
                .await
                .expect("resolve");
        lorehaven_domain::retention::narrowest_mode(resolved.instance, resolved.source)
            .as_str()
            .to_owned()
    }
}

/// The instance-wide setting.
async fn instance_mode(harness: &Harness) -> String {
    harness.mode_for(None).await
}

// ============================ E.2 governance ============================

/// Three supporters, which is the ordinary bar for a *narrowing* proposal.
const NARROWING_SUPPORTERS: [&str; 3] = ["bob", "carol", "dave"];

/// Open a proposal and return its id. `mode` is what is being proposed.
async fn open(harness: &Harness, handle: &str, mode: &str, source: Option<&str>) -> String {
    let mut client = harness.reader(handle).await;
    let (status, body) = client
        .post(
            "/api/v1/retention/proposals",
            json!({
                "proposed_mode": mode,
                "source_key": source,
                "rationale": "the full text is never read and the disk is finite",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "open {mode}: {body}");
    body["id"].as_str().expect("id").to_owned()
}

/// Cast `support` from each of `voters`.
async fn votes(harness: &Harness, id: &str, voters: &[&str], support: bool) {
    for handle in voters {
        let mut client = harness.reader(handle).await;
        let (status, body) = client
            .post(
                &format!("/api/v1/retention/proposals/{id}/vote"),
                json!({ "support": support }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{handle} vote: {body}");
    }
}

#[tokio::test]
async fn in_advisory_mode_a_passed_proposal_does_not_change_the_setting() {
    let harness = Harness::with_operator_now("advisory-no-move").await;
    // `cache` is the default instance mode, so proposing `aggregate` narrows.
    assert_eq!(
        instance_mode(&harness).await,
        "cache",
        "the default is cache"
    );

    let id = open(&harness, "alice", "aggregate", None).await;
    votes(&harness, &id, &NARROWING_SUPPORTERS, true).await;

    // Quorum is met — the ballot has the readers' majority.
    let (status, body) = harness
        .reader("watcher")
        .await
        .get(&format!("/api/v1/retention/proposals/{id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["tally"]["quorum"]["reached"], true,
        "three supporters is the ordinary bar: {body}"
    );

    // Settle it, in advisory mode.
    let mut operator = harness.operator().await;
    let (status, settle) = operator
        .post("/api/v1/admin/retention/proposals/settle", json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "settle: {settle}");
    assert_eq!(settle["passed"], 1, "the ballot was recorded: {settle}");
    assert_eq!(
        settle["committed"], 0,
        "advisory mode commits nothing: {settle}"
    );
    assert_eq!(settle["binding_mode"], false, "{settle}");

    // **And the setting is untouched.** This is the assertion the test exists
    // for: the ballot reached quorum, the pass ran, and the mode is what it
    // was. An implementation that commits in advisory mode passes every
    // assertion above this line.
    assert_eq!(
        instance_mode(&harness).await,
        "cache",
        "a passing ballot in advisory mode moves no setting"
    );
}

#[tokio::test]
async fn in_advisory_mode_the_operator_response_is_required_before_anything_moves() {
    let harness = Harness::with_operator_now("advisory-respond").await;
    let id = open(&harness, "alice", "aggregate", None).await;
    votes(&harness, &id, &NARROWING_SUPPORTERS, true).await;

    let mut operator = harness.operator().await;
    let (status, settle) = operator
        .post("/api/v1/admin/retention/proposals/settle", json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{settle}");
    assert_eq!(instance_mode(&harness).await, "cache", "still unmoved");

    // Now the operator accepts. This is the only thing that moves it.
    let (status, body) = operator
        .post(
            &format!("/api/v1/admin/retention/proposals/{id}/respond"),
            json!({ "decision": "accept" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "respond accept: {body}");
    assert_eq!(body["state"], "passed", "{body}");
    assert_eq!(
        body["body_mode_now"], "aggregate",
        "the response reports what landed, so the operator does not need a \
         second request to confirm it: {body}"
    );
    assert_eq!(
        instance_mode(&harness).await,
        "aggregate",
        "the operator's acceptance is what moved the setting"
    );
}

#[tokio::test]
async fn in_advisory_mode_a_decline_needs_a_reason_and_records_the_disagreement() {
    let harness = Harness::with_operator_now("advisory-decline").await;
    let id = open(&harness, "alice", "aggregate", None).await;
    votes(&harness, &id, &NARROWING_SUPPORTERS, true).await;

    let mut operator = harness.operator().await;
    // A decline with no reason is refused: the operator has to be able to
    // explain it to the three readers who voted for it.
    let (status, body) = operator
        .post(
            &format!("/api/v1/admin/retention/proposals/{id}/respond"),
            json!({ "decision": "decline" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a decline without a reason: {body}"
    );
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("reason"),
        "{body}"
    );
    assert_eq!(instance_mode(&harness).await, "cache", "still unmoved");

    let (status, body) = operator
        .post(
            &format!("/api/v1/admin/retention/proposals/{id}/respond"),
            json!({
                "decision": "decline",
                "reason": "the aggregate index does not survive a reindex here, \
                           and a lost index is worse than a full disk",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "decline with a reason: {body}");
    assert_eq!(
        body["state"], "overridden",
        "a declined proposal says so, rather than quietly expiring: {body}"
    );
    assert_eq!(
        instance_mode(&harness).await,
        "cache",
        "declining moves nothing"
    );
}

/// Narrowing: the ordinary bar, and it commits at the ordinary bar.
///
/// Narrowing is `cache -> aggregate`. The instance is put into `cache` first so
/// the direction is a narrowing and the *ordinary* quorum (three) applies, which
/// is the whole point of the test — the higher bar is for widening only.
#[tokio::test]
async fn a_narrowing_proposal_commits_at_the_ordinary_bar() {
    let mut harness = Harness::with_operator_now("narrowing").await;
    harness.config.retention_governance.binding_mode = true;
    // Make the instance `cache` so `aggregate` is a narrowing and the *ordinary*
    // bar applies — which is the whole point: the higher bar is for widening
    // only, and a test that never narrows cannot tell the two apart.
    let actor = account_of(&harness.db, "operator").await;
    lorehaven_db::retention::write_policy(
        &harness.db,
        lorehaven_domain::retention::BodyMode::Cache,
        actor,
    )
    .await
    .expect("set cache");
    assert_eq!(instance_mode(&harness).await, "cache");

    let id = open(&harness, "alice", "aggregate", None).await;

    // Two supporters. **The tally is what proves two is not enough** — and
    // asking the tally is not the same as settling, which is what the first
    // version of this test did and which expired the proposal out from under
    // the third supporter's ballot. A closed ballot takes no more votes, and the
    // store says so by name, which is correct and not what this test is about.
    votes(&harness, &id, &["bob", "carol"], true).await;
    let (status, body) = harness
        .reader("watcher")
        .await
        .get(&format!("/api/v1/retention/proposals/{id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["tally"]["supporters"], 2, "{body}");
    assert_eq!(
        body["tally"]["quorum"]["reached"], false,
        "two is not the ordinary bar of three: {body}"
    );
    assert_eq!(body["tally"]["quorum"]["required"], 3, "{body}");
    assert_eq!(
        instance_mode(&harness).await,
        "cache",
        "and nothing moved yet"
    );

    // The third supporter, then one settle.
    votes(&harness, &id, &["dave"], true).await;
    let mut operator = harness.operator().await;
    let (status, settle) = operator
        .post("/api/v1/admin/retention/proposals/settle", json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{settle}");
    assert_eq!(
        settle["committed"], 1,
        "three is the ordinary bar for a narrowing: {settle}"
    );
    assert_eq!(
        instance_mode(&harness).await,
        "aggregate",
        "a narrowing commits at the ordinary bar"
    );
}

#[tokio::test]
async fn a_ballot_that_misses_the_bar_and_its_window_is_closed_as_expired() {
    // Its own test, because it is a behaviour rather than a footnote: a
    // proposal that did not reach its bar and whose window has shut is
    // finished, and leaving it `open` would advertise something nobody can vote
    // on. This is also the case that expires rather than committing, so it
    // needs to be visible on its own.
    let mut harness = Harness::with_operator_now("expired").await;
    harness.config.retention_governance.binding_mode = true;
    let actor = account_of(&harness.db, "operator").await;
    lorehaven_db::retention::write_policy(
        &harness.db,
        lorehaven_domain::retention::BodyMode::Cache,
        actor,
    )
    .await
    .expect("set cache");

    let id = open(&harness, "alice", "aggregate", None).await;
    votes(&harness, &id, &["bob", "carol"], true).await;

    let mut operator = harness.operator().await;
    let (status, settle) = operator
        .post("/api/v1/admin/retention/proposals/settle", json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{settle}");
    assert_eq!(settle["expired"], 1, "two is not the bar: {settle}");
    assert_eq!(settle["committed"], 0, "{settle}");
    assert_eq!(
        instance_mode(&harness).await,
        "cache",
        "a ballot short of its bar changes nothing"
    );

    // And it is closed, so a late voter is told the ballot has finished rather
    // than being counted against a decision that already went the other way.
    let (status, body) = harness
        .reader("watcher")
        .await
        .get(&format!("/api/v1/retention/proposals/{id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "expired", "{body}");

    let mut dave = harness.reader("dave").await;
    let (status, body) = dave
        .post(
            &format!("/api/v1/retention/proposals/{id}/vote"),
            json!({ "support": true }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a closed ballot takes no more votes: {body}"
    );
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("expired"),
        "the refusal names why: {body}"
    );
}

#[tokio::test]
async fn in_binding_mode_a_widening_proposal_at_the_ordinary_quorum_does_not_commit() {
    let mut harness = Harness::with_operator_now("widen-ordinary").await;
    harness.config.retention_governance.binding_mode = true;
    // **4, deliberately.** The first version left the harness default of 3 and
    // asserted `required > 3` — which is false, because with the config at 3 a
    // widening is judged at 3 and there is no difference to observe. The claim
    // worth testing is that the *configured* high-impact bar is the one a
    // widening is judged against, so the config has to differ from the ordinary
    // bar for the assertion to mean anything.
    harness.config.retention_governance.widen_quorum = 4;
    // The instance is `aggregate`, so proposing `cache` is a WIDENING and the
    // higher bar applies. The ordinary quorum of three is not enough.
    // A real account, not a sentinel: the operator's, so the seeded row is
    // indistinguishable from one `PATCH /admin/retention/policy` wrote.
    let actor = account_of(&harness.db, "operator").await;
    lorehaven_db::retention::write_policy(
        &harness.db,
        lorehaven_domain::retention::BodyMode::Aggregate,
        actor,
    )
    .await
    .expect("set aggregate");
    assert_eq!(instance_mode(&harness).await, "aggregate");

    let id = open(&harness, "alice", "cache", None).await;
    votes(&harness, &id, &NARROWING_SUPPORTERS, true).await;

    // At the ordinary quorum, settle finds the ballot has NOT reached the bar
    // that applies to it.
    let mut operator = harness.operator().await;
    let (status, settle) = operator
        .post("/api/v1/admin/retention/proposals/settle", json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{settle}");

    // The proposal's own tally says which bar it is judged against.
    let (_status, body) = harness
        .reader("watcher")
        .await
        .get(&format!("/api/v1/retention/proposals/{id}"))
        .await;
    let required = body["tally"]["quorum"]["required"]
        .as_i64()
        .expect("required");
    assert_eq!(
        required, 4,
        "a widening is judged at the configured high-impact bar, not the \
         ordinary three: {body}"
    );
    assert_eq!(
        body["tally"]["quorum"]["reached"], false,
        "three supporters do not meet a bar of four: {body}"
    );
    assert_eq!(
        body["tally"]["further_needed"], 1,
        "and the shortfall is named, so a reader knows one more would do it: {body}"
    );

    // And with three supporters it has not been reached, so nothing commits.
    assert_eq!(
        settle["committed"], 0,
        "the ordinary quorum does not commit a widening: {settle}"
    );
    assert_eq!(
        instance_mode(&harness).await,
        "aggregate",
        "the setting is unchanged at the ordinary quorum"
    );
}

#[tokio::test]
async fn in_binding_mode_a_widening_proposal_commits_after_quorum_and_cooling_days() {
    let mut harness = Harness::with_operator("widen-cooling").await;
    harness.config.retention_governance.binding_mode = true;
    // **2, deliberately**, and it does not lower the bar: `quorum_for` clamps a
    // widening up to `MINIMUM_QUORUM` (3) because a bar below three is "a
    // proposal decided by one person's second tap". Three supporters below, so
    // the test proves the clamp as well as the delay — a rule the code claims
    // and this demonstrates. The first version of this test set 2 and used two
    // supporters, expecting the bar to come down; the clamp refused, correctly.
    harness.config.retention_governance.widen_quorum = 2;
    harness.config.retention_governance.proposal_cooling_days = 7;
    // A real account, not a sentinel: the operator's, so the seeded row is
    // indistinguishable from one `PATCH /admin/retention/policy` wrote.
    let actor = account_of(&harness.db, "operator").await;
    lorehaven_db::retention::write_policy(
        &harness.db,
        lorehaven_domain::retention::BodyMode::Aggregate,
        actor,
    )
    .await
    .expect("set aggregate");

    let id = open(&harness, "alice", "cache", None).await;
    votes(&harness, &id, &NARROWING_SUPPORTERS, true).await;

    // The clock is a parameter, so this is not a sleep. `run_at` is exercised
    // directly with a `now` on either side of the proposal's `closes_at`.
    let app_state = lorehaven_app::state::AppState::new(harness.config.clone(), harness.db.clone());
    let closes_at = {
        let proposal = lorehaven_db::retention_proposals::proposal(&harness.db, &id)
            .await
            .expect("load")
            .expect("some");
        proposal.closes_at
    };

    // **One second before the window closes**: quorum is met, the window is
    // still open, and the setting must not have moved. This is the assertion
    // that separates binding mode from "commits at quorum", which is the most
    // likely way to get §5.3's delay wrong.
    let just_before = shift(&closes_at, -1);
    let before = lorehaven_app::routes::retention_settle::run_at(&app_state, &just_before)
        .await
        .expect("pass");
    assert_eq!(before.considered, 0, "the window is still open: {before:?}");
    assert_eq!(
        instance_mode(&harness).await,
        "aggregate",
        "a binding proposal does not commit the instant quorum is reached"
    );

    // **One second after**: the pass settles it.
    let just_after = shift(&closes_at, 1);
    let after = lorehaven_app::routes::retention_settle::run_at(&app_state, &just_after)
        .await
        .expect("pass");
    assert_eq!(after.considered, 1, "the window has closed: {after:?}");
    assert_eq!(after.committed, 1, "and it commits: {after:?}");
    assert_eq!(
        instance_mode(&harness).await,
        "cache",
        "a binding widening commits once the window closes"
    );

    // **The actor on the change row is not a reader.** The readers decided this;
    // putting one of their names on the row would be the ballot leak arriving
    // through the audit trail instead of the response body.
    let changes = lorehaven_db::retention_proposals::list_changes(&harness.db, None)
        .await
        .expect("changes");
    let change = changes
        .iter()
        .find(|c| c.to_mode.as_str() == "cache")
        .expect("the change was recorded");
    // **The actor is the instance's own account — a named non-person.**
    //
    // `actor` is `NOT NULL REFERENCES accounts (id)`, so "nobody" is not
    // representable and three candidates were wrong: NULL is refused, the nil
    // UUID is refused by the foreign key, and a reader's id would put one of
    // the three voters' names on the row their own ballot produced — the ballot
    // leak arriving through the audit trail rather than the response body.
    //
    // A fixed id, so it is identifiable in a database dump: an operator seeing
    // it knows at a glance that a row was written by the instance and not by a
    // person. A per-install random id would satisfy the constraint and make
    // audit rows uncorrelatable, which this assertion would catch.
    assert_eq!(
        change.actor,
        lorehaven_db::system_account(),
        "the instance acted, and the row says so by name: {change:?}"
    );
    for handle in ["alice", "bob", "carol"] {
        let account = account_of(&harness.db, handle).await.to_string();
        assert_ne!(
            change.actor, account,
            "{handle}'s name is not on a row their ballot caused"
        );
    }
}

/// Move an RFC3339 timestamp by `seconds`. Fixed-width UTC in, fixed-width UTC
/// out, because `overdue_proposals` compares `closes_at` **lexicographically**:
/// a `now` in any other shape would compare wrongly rather than fail, which is
/// the sort of thing that makes a test pass for the wrong reason.
fn shift(rfc3339: &str, seconds: i64) -> String {
    let parsed =
        time::OffsetDateTime::parse(rfc3339, &time::format_description::well_known::Rfc3339)
            .expect("parse closes_at");
    (parsed + time::Duration::seconds(seconds))
        .format(&time::format_description::well_known::Rfc3339)
        .expect("format")
}

#[tokio::test]
async fn an_operator_override_sets_the_setting_back_and_leaves_the_tally_visible() {
    let mut harness = Harness::with_operator_now("override").await;
    harness.config.retention_governance.binding_mode = true;
    // A real account, not a sentinel: the operator's, so the seeded row is
    // indistinguishable from one `PATCH /admin/retention/policy` wrote.
    let actor = account_of(&harness.db, "operator").await;
    lorehaven_db::retention::write_policy(
        &harness.db,
        lorehaven_domain::retention::BodyMode::Aggregate,
        actor,
    )
    .await
    .expect("set aggregate");

    let id = open(&harness, "alice", "cache", None).await;
    votes(&harness, &id, &["bob", "carol"], true).await;

    let mut operator = harness.operator().await;

    // An override with no reason is refused: it is the only record of why the
    // readers' decision was not followed.
    let (status, body) = operator
        .post(
            &format!("/api/v1/admin/retention/proposals/{id}/override"),
            json!({ "body_mode": "aggregate", "reason": "  " }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "an override needs a reason: {body}"
    );
    assert_eq!(instance_mode(&harness).await, "aggregate", "nothing moved");

    let (status, body) = operator
        .post(
            &format!("/api/v1/admin/retention/proposals/{id}/override"),
            json!({
                "body_mode": "aggregate",
                "reason": "the archive is 400GB and the readers did not know that",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "override: {body}");
    assert_eq!(body["body_mode_now"], "aggregate", "{body}");

    // The override is available in *both* modes, and it is available even
    // though a binding instance would have committed on its own — an instance
    // that has handed policy to its readers still needs a way to act when the
    // committed setting turns out to be wrong.
    assert_eq!(
        instance_mode(&harness).await,
        "aggregate",
        "the override landed"
    );

    // **And the tally survives it.** An override that erased the ballot would
    // make the readers' decision invisible, and the next reader to ask "did
    // anyone want this?" would be told no. The state is `overridden`, which is
    // the transition that makes the disagreement visible.
    let (status, after) = harness
        .reader("watcher")
        .await
        .get(&format!("/api/v1/retention/proposals/{id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{after}");
    assert_eq!(
        after["state"], "overridden",
        "the disagreement is recorded: {after}"
    );
    assert_eq!(
        after["tally"]["supporters"], 2,
        "the tally is still visible after an override: {after}"
    );
}

#[tokio::test]
async fn a_proposal_payload_carrying_a_non_retention_field_is_refused_by_name() {
    let harness = Harness::new("extra-field").await;
    let mut alice = harness.reader("alice").await;

    // `deny_unknown_fields`: a payload carrying a field this feature does not
    // act on is refused rather than silently accepted with the field dropped.
    // The dangerous version is a client sending `body_mode` (the *admin* route's
    // spelling) and having it ignored while `proposed_mode` is honoured — two
    // spellings of the same decision, one of which is a no-op.
    let (status, body) = alice
        .post(
            "/api/v1/retention/proposals",
            json!({
                "proposed_mode": "aggregate",
                "source_key": "archive:example.org",
                "rationale": "a reason",
                "body_mode": "cache",
            }),
        )
        .await;
    assert!(
        status == StatusCode::UNPROCESSABLE_ENTITY || status == StatusCode::BAD_REQUEST,
        "an unknown field is refused, not ignored: {status} {body}"
    );
    let message = body.to_string();
    assert!(
        message.contains("body_mode"),
        "the refusal names the offending field: {message}"
    );
}

#[tokio::test]
async fn opening_a_proposal_grants_no_trust_credit_badge_or_placement() {
    let harness = Harness::new("no-credit").await;

    // Register the reader, *then* read their trust. The first version read
    // `alice` here in a harness that never registered her, so `RowNotFound` was
    // the correct answer — and the duplicate "read a trust level, act, read it
    // again" shape is what hid it.
    let mut client = harness.client();
    let (status, _) = client
        .post(
            "/api/v1/auth/register",
            json!({
                "email": "newcomer@example.com",
                "password": GOOD_PASSWORD,
                "handle": "newcomer",
                "display_name": "newcomer",
                "age_band": "adult",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "register newcomer: {status}");
    let account = account_of(&harness.db, "newcomer").await.to_string();

    // A fresh account sits at 0 — which is *below* the default
    // `proposal_min_trust` of 1, so this reader is refused at the gate. That is
    // correct, and it is why the test raises them: "governance activity grants
    // no credit" needs a reader who can actually act, since a reader refused at
    // the gate cannot demonstrate that the gate grants nothing.
    let fresh = lorehaven_db::governance::trust_for(&harness.db, &account)
        .await
        .expect("trust before");
    assert_eq!(fresh, 0, "a new reader starts at 0, below the bar of 1");

    lorehaven_db::governance::set_trust(&harness.db, &account, 1, "test")
        .await
        .expect("raise to the bar");
    let before = lorehaven_db::governance::trust_for(&harness.db, &account)
        .await
        .expect("trust at the bar");
    assert_eq!(before, 1, "the reader is now exactly at the bar");

    // Open a proposal, and vote on somebody else's.
    let (status, body) = client
        .post(
            "/api/v1/retention/proposals",
            json!({
                "proposed_mode": "aggregate",
                "source_key": "archive:newcomer.example",
                "rationale": "nobody reads the bodies",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "open: {body}");
    let id = body["id"].as_str().expect("id").to_owned();
    let (status, body) = client
        .post(
            &format!("/api/v1/retention/proposals/{id}/vote"),
            json!({ "support": true }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "vote: {body}");

    // **Unchanged.** A preference poll that made its participants more visible
    // would become a status ladder within two releases, and §19.15's "a vote
    // never grants the proposer anything personal" is cheap to state and
    // expensive to retrofit. Opening a proposal and voting are the two actions a
    // reader could farm, so those are the two checked.
    let after = lorehaven_db::governance::trust_for(&harness.db, &account)
        .await
        .expect("trust after");
    assert_eq!(
        after, before,
        "opening a proposal and voting on it changed this reader's trust from \
         {before} to {after}"
    );
}

#[tokio::test]
async fn no_reader_can_read_another_readers_retention_ballot() {
    let harness = Harness::new("no-cross-read").await;
    let id = open(&harness, "alice", "aggregate", Some("archive:example.org")).await;
    votes(&harness, &id, &NARROWING_SUPPORTERS, true).await;

    // Every reader-facing surface, with a real ballot behind it.
    let mut reader = harness.reader("watcher").await;
    let (list_status, list) = reader.get("/api/v1/retention/proposals").await;
    assert_eq!(list_status, StatusCode::OK, "{list}");
    let (one_status, one) = reader
        .get(&format!("/api/v1/retention/proposals/{id}"))
        .await;
    assert_eq!(one_status, StatusCode::OK, "{one}");

    // A voter trying to read *their own* ballot back, which is the benign
    // version of the same request and the one a future `GET .../my-vote` route
    // would answer. It must not be answerable from a tally.
    assert_eq!(one["tally"]["supporters"], 3, "the count is there: {one}");
    assert!(
        one.get("my_vote").is_none() && one.get("ballot").is_none(),
        "a reader cannot read back even their own ballot: {one}"
    );

    // And the operator's routes are invisible to a reader — `require_operator`
    // 404s rather than 403s, so a 403 would confirm the surface exists.
    let (status, _) = reader
        .get(&format!("/api/v1/admin/retention/proposals/{id}"))
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the operator surface does not confirm itself to a reader: {status}"
    );
}
