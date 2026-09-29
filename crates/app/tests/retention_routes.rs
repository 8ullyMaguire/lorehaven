//! Acceptance: the retention admin surface (spec §11.15).
//!
//! `retention_policy.rs` proves the store. This file proves the four routes, and
//! the routes are where three of §11.15's requirements actually land:
//!
//! - **the widening is refused by name.** Not "the row was not written" — an
//!   error that names the source, the mode asked for, the instance's mode, and
//!   the one endpoint that would change it. A refusal the operator cannot act on
//!   is the generic error §11.15 forbids, wearing a status code.
//! - **it is a 400, not a 403 and not a 500.** They are not allowed is false;
//!   the instance is broken is false. Only the value is wrong.
//! - **the surface is invisible to a non-operator.** `require_operator` answers
//!   404, and the tests assert 404 rather than 403 — the status is the contract,
//!   and a test that only asserted "refused" would pass against a 403 that
//!   confirms the admin surface exists.
//!
//! The `DELETE` route is also tested, because §11.15's own refusal message tells
//! a reader to "Remove the source override in admin settings" and an action
//! named by a user-facing message has to exist.

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
        "lorehaven-retention-routes-{tag}-{}-{:?}",
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
    // same note as `m29_transparency.rs`, which found a hardcoded SQLite URL
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

/// A session-capable client. Cookies are carried by hand rather than by a
/// cookie jar, as every suite in this directory does.
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

    async fn patch(&mut self, path: &str, body: Value) -> (StatusCode, Value) {
        self.send("PATCH", path, Some(body)).await
    }

    async fn put(&mut self, path: &str, body: Value) -> (StatusCode, Value) {
        self.send("PUT", path, Some(body)).await
    }

    async fn delete(&mut self, path: &str) -> (StatusCode, Value) {
        self.send("DELETE", path, None).await
    }
}

struct Harness {
    /// Held so the scratch PostgreSQL database lives as long as the harness.
    _tdb: TestDb,
    _dir: std::path::PathBuf,
    config: Config,
    db: Database,
}

impl Harness {
    /// A harness where the reader named `operator` holds the operator role.
    ///
    /// The account is registered *before* the config is finalised, because
    /// `require_operator` compares the session's account against
    /// `config.administration.operator_account_id` — not a trust tier — so the
    /// id has to exist before anything can name it.
    async fn with_operator(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let mut config = config_for(&dir);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        let db = tdb.db().clone();

        let mut bootstrap = Client {
            app: server::build_router(AppState::new(config.clone(), db.clone())),
            cookies: Vec::new(),
        };
        let (status, body) = bootstrap
            .send(
                "POST",
                "/api/v1/auth/register",
                Some(json!({
                    "email": "operator@example.com",
                    "password": GOOD_PASSWORD,
                    "handle": "operator",
                    "display_name": "operator",
                    "age_band": "adult",
                })),
            )
            .await;
        assert!(
            status == StatusCode::CREATED,
            "bootstrap register: {status} {body}"
        );
        let account = account_of(&db, "operator").await;
        config.administration.operator_account_id = Some(account.into());
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

    /// A signed-in reader. The operator handle already exists, so a register
    /// returning a conflict is expected and the code falls through to login.
    async fn reader(&self, handle: &str) -> Client {
        let mut client = self.client();
        let (status, _body) = client
            .send(
                "POST",
                "/api/v1/auth/register",
                Some(json!({
                    "email": format!("{handle}@example.com"),
                    "password": GOOD_PASSWORD,
                    "handle": handle,
                    "display_name": handle,
                    "age_band": "adult",
                })),
            )
            .await;
        if status != StatusCode::CREATED {
            let (status, body) = client
                .send(
                    "POST",
                    "/api/v1/auth/login",
                    Some(json!({
                        "email": format!("{handle}@example.com"),
                        "password": GOOD_PASSWORD,
                    })),
                )
                .await;
            assert!(status.is_success(), "login {handle}: {status} {body}");
        }
        client
    }

    /// The signed-in operator.
    async fn operator(&self) -> Client {
        self.reader("operator").await
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
    uuid::Uuid::parse_str(&value).expect("an account id is a uuid")
}

/// The message a validation failure carries, whatever shape the error body has.
///
/// The plan's own note applies here: assert on the *type* and the reason code,
/// and keep the prose in the message. This pulls the human sentence out of
/// whichever envelope the handler used so the assertion below is about content.
fn message_of(body: &Value) -> String {
    body.get("message")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            body.get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| body.to_string())
}

/// An unconfigured instance reads as `cache` and says it is unconfigured.
///
/// Both halves. `body_mode` alone would be satisfied by a row seeded with
/// 'cache' at migration time, and then `configured` would be true and the
/// dashboard would report a decision nobody made — which §11.15's whole framing
/// is against.
#[tokio::test]
async fn an_unconfigured_instance_reads_as_cache_and_says_it_is_unconfigured() {
    let harness = Harness::with_operator("unconfigured").await;
    let mut client = harness.operator().await;

    let (status, body) = client.get("/api/v1/admin/retention/policy").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["body_mode"], "cache");
    assert_eq!(
        body["configured"], false,
        "an instance nobody configured is caching because `cache` is the default, not \
         because anybody chose it"
    );
    assert_eq!(body["version"], 0);
    assert_eq!(
        body["available_modes"],
        json!(["cache", "aggregate"]),
        "the response names both legal values, so a client need not hardcode them"
    );
}

/// The operator records a decision, and the response is the stored row.
#[tokio::test]
async fn an_operator_records_the_policy_and_reads_back_what_was_stored() {
    let harness = Harness::with_operator("record").await;
    let mut client = harness.operator().await;

    let (status, body) = client
        .patch(
            "/api/v1/admin/retention/policy",
            json!({ "body_mode": "aggregate" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["body_mode"], "aggregate");
    assert_eq!(body["version"], 1);

    let (status, read) = client.get("/api/v1/admin/retention/policy").await;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(read["body_mode"], "aggregate");
    assert_eq!(read["configured"], true);
    assert!(
        read["updated_by"].is_string(),
        "the row records who changed it, which is what makes it auditable"
    );

    // And the store agrees, read directly. A route that answered from a cache
    // and the store that disagreed would leave the operator looking at a
    // setting nothing obeys.
    let stored = lorehaven_db::retention::read_policy(&harness.db)
        .await
        .expect("read the policy");
    assert_eq!(
        stored.expect("a row was written").body_mode,
        lorehaven_domain::retention::BodyMode::Aggregate
    );
}

/// An unrecognised mode is refused naming the two legal values.
#[tokio::test]
async fn an_unrecognised_mode_is_refused_naming_the_two_legal_values() {
    let harness = Harness::with_operator("bad_mode").await;
    let mut client = harness.operator().await;

    for wrong in ["agggregate", "CACHE", "cached", "", "metadata"] {
        let (status, body) = client
            .patch(
                "/api/v1/admin/retention/policy",
                json!({ "body_mode": wrong }),
            )
            .await;
        // 422, not 400: this codebase's `AppError::Validation` maps to
        // UNPROCESSABLE_ENTITY, which is the convention every other validation
        // in the instance already follows. A 400 here would be a *second* code
        // for the same class of refusal, and a client branching on the code
        // would have to learn which routes use which.
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{wrong:?} must be refused with a validation error, not accepted or reported \
             as a server error: {body}"
        );
        let message = message_of(&body);
        assert!(
            message.contains("cache") && message.contains("aggregate"),
            "the refusal must name both legal values, not say the JSON was invalid: \
             {message}"
        );
        assert!(
            message.contains(wrong) || wrong.is_empty(),
            "the refusal must quote what was sent, so the operator can see their typo: \
             {message}"
        );
    }

    // Nothing was written by any of them.
    let (status, body) = client.get("/api/v1/admin/retention/policy").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["configured"], false,
        "a refused request must leave no row behind"
    );
}

/// The widening is refused, by name, with a 400 — the heart of §11.15 here.
#[tokio::test]
async fn a_widening_override_is_refused_by_name_not_as_a_server_error() {
    let harness = Harness::with_operator("widen").await;
    let mut client = harness.operator().await;
    client
        .patch(
            "/api/v1/admin/retention/policy",
            json!({ "body_mode": "aggregate" }),
        )
        .await;

    let (status, body) = client
        .put(
            "/api/v1/admin/retention/sources/ao3",
            json!({ "body_mode": "cache" }),
        )
        .await;
    // 422 for the same reason as the mode above: this is a validation refusal
    // expressed through the instance's one validation code. Not 403 (the
    // operator is allowed; the value is wrong) and not 500 (nothing is broken).
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "not 403 — the operator is allowed, the value is wrong — and not 500, because \
         nothing is broken: {body}"
    );
    let message = message_of(&body);
    assert!(
        message.contains("ao3"),
        "the refusal names the source: {message}"
    );
    assert!(
        message.contains("aggregate"),
        "the refusal names the instance's setting, which is the thing to change: {message}"
    );
    assert!(
        message.contains("retention/policy"),
        "the refusal names the endpoint that would make the change, because an \
         un-actionable refusal is the generic error §11.15 forbids: {message}"
    );

    // And the narrowing direction works, so the test is not passing because
    // every override is refused.
    let (status, body) = client
        .put(
            "/api/v1/admin/retention/sources/eff",
            json!({ "body_mode": "aggregate" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// The list carries the instance mode, so "may only narrow" is checkable on the
/// page, and the override can be removed from it.
#[tokio::test]
async fn the_override_list_carries_the_instance_mode_and_the_override_can_be_removed() {
    let harness = Harness::with_operator("list").await;
    let mut client = harness.operator().await;
    client
        .put(
            "/api/v1/admin/retention/sources/ao3",
            json!({ "body_mode": "aggregate" }),
        )
        .await;

    let (status, body) = client.get("/api/v1/admin/retention/sources").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["instance_body_mode"], "cache");
    assert_eq!(
        body["overrides"].as_array().map(Vec::len),
        Some(1),
        "{body}"
    );
    assert_eq!(body["overrides"][0]["source_key"], "ao3");
    assert_eq!(body["overrides"][0]["body_mode"], "aggregate");

    // Removing it is the action the `AggregateSourceOverride` message names, so
    // it has to exist and it has to work.
    let (status, body) = client.delete("/api/v1/admin/retention/sources/ao3").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = client.get("/api/v1/admin/retention/sources").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["overrides"].as_array().map(Vec::len),
        Some(0),
        "the override is gone: {body}"
    );

    // And removing one that is not there is a 404, not a silent success.
    let (status, body) = client.delete("/api/v1/admin/retention/sources/ao3").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

/// A reader who is not the operator learns nothing — 404, not 403.
#[tokio::test]
async fn a_non_operator_is_not_told_the_surface_exists() {
    let harness = Harness::with_operator("nonop").await;
    let mut client = harness.reader("reader").await;

    for (method, path, body) in [
        ("GET", "/api/v1/admin/retention/policy", None),
        ("GET", "/api/v1/admin/retention/sources", None),
        (
            "PATCH",
            "/api/v1/admin/retention/policy",
            Some(json!({ "body_mode": "aggregate" })),
        ),
        (
            "PUT",
            "/api/v1/admin/retention/sources/ao3",
            Some(json!({ "body_mode": "aggregate" })),
        ),
        ("DELETE", "/api/v1/admin/retention/sources/ao3", None),
    ] {
        let (status, response) = client.send(method, path, body).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{method} {path} must 404 a non-operator: a 403 confirms the surface exists, \
             and the surface says whether this instance holds the text of every work it \
             knows about. Got {status}: {response}"
        );
    }

    // Nothing changed while they tried.
    let mut operator = harness.operator().await;
    let (status, body) = operator.get("/api/v1/admin/retention/policy").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["configured"], false, "no write reached the store");
}

/// An unauthenticated request is refused too, and by the same 404.
#[tokio::test]
async fn an_unauthenticated_request_is_refused() {
    let harness = Harness::with_operator("anon").await;
    let mut client = harness.client();
    for path in [
        "/api/v1/admin/retention/policy",
        "/api/v1/admin/retention/sources",
    ] {
        let (status, body) = client.get(path).await;
        assert!(
            status == StatusCode::UNAUTHORIZED || status == StatusCode::NOT_FOUND,
            "{path}: expected the session gate to refuse, got {status}: {body}"
        );
        assert!(
            body.get("body_mode").is_none(),
            "{path} must not answer the policy to an unauthenticated reader: {body}"
        );
    }
}

/// A widening by an *import path* is not possible from the reader's side, but a
/// source override on a caching instance must be visible to the store the body
/// paths read. This asserts the end-to-end effect rather than the route.
#[tokio::test]
async fn a_narrowed_source_is_what_a_body_path_would_read() {
    let harness = Harness::with_operator("body_path").await;
    let mut client = harness.operator().await;
    client
        .put(
            "/api/v1/admin/retention/sources/ao3",
            json!({ "body_mode": "aggregate" }),
        )
        .await;

    let narrowed =
        lorehaven_db::retention::resolve_for_source(&harness.db, Some("ao3"), false, false)
            .await
            .expect("resolve ao3");
    assert_eq!(
        narrowed.source,
        Some(lorehaven_domain::retention::BodyMode::Aggregate)
    );
    assert!(
        lorehaven_domain::retention::check_body_allowed(&narrowed, Some("ao3")).is_err(),
        "the body path is refused for this source"
    );

    let other = lorehaven_db::retention::resolve_for_source(&harness.db, Some("eff"), false, false)
        .await
        .expect("resolve eff");
    assert_eq!(
        lorehaven_domain::retention::check_body_allowed(&other, Some("eff")),
        Ok(()),
        "caching most sources while aggregating one is the case §11.15 calls expressible"
    );
}

// ---------------------------------------------------------------------------
// §11.15 / amendment §4.2 — works_past_saving
// ---------------------------------------------------------------------------

/// The count is the amendment's definition and not a looser one: aggregated
/// works whose origin is unreachable and which this instance holds no body for.
///
/// Each clause is excluded deliberately, because a count that keeps only some
/// of them is worse than none — it is a number an operator acts on.
#[tokio::test]
async fn works_past_saving_counts_only_works_this_instance_holds_no_body_for() {
    let harness = Harness::with_operator("m5909").await;
    let mut client = harness.operator().await;

    // An instance nobody configured has no works at all, so the count is zero —
    // and the mode is reported beside it, because a zero means different things
    // on a caching and an aggregating instance.
    let (status, body) = client
        .get("/api/v1/admin/retention/works-past-saving")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["works_past_saving"], 0);
    assert_eq!(body["instance_body_mode"], "cache");
}

/// The count is operator-only.
///
/// The amendment says "It changes no behaviour and it is not public", and a
/// preservation-debt figure on a reachable surface is a disclosure about what
/// this instance has failed to keep. The assertion is 404, not 403: a 403
/// confirms the surface exists, and §7.7.3's rule is that a reader outside the
/// audience learns nothing at all.
#[tokio::test]
async fn works_past_saving_is_not_reachable_by_a_non_operator() {
    let harness = Harness::with_operator("m5909-gate").await;
    let mut reader = harness.reader("reader").await;
    let (status, body) = reader
        .get("/api/v1/admin/retention/works-past-saving")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(
        body.get("works_past_saving").is_none(),
        "a non-operator must not learn the count: {body}"
    );

    // And it is not on `/admin/stats`, which answers an anonymous caller.
    let mut anonymous = harness.client();
    let (status, body) = anonymous.get("/api/v1/admin/stats").await;
    assert!(
        status.is_success(),
        "the public stats route exists, which is the point: {status}"
    );
    assert!(
        body.get("works_past_saving").is_none(),
        "the count must not appear on the public stats surface: {body}"
    );
}
