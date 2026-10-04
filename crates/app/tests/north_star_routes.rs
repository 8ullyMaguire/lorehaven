//! M45-23 — §53.5's north-star view over HTTP.
//!
//! `crates/app/tests/north_star.rs` drives the store, and this drives the route. The
//! separation is deliberate: the store's job is arithmetic and attribution, the route's job
//! is **not disclosing that the view exists** and **not inventing a target**.
//!
//! Four cases, and each one fails if a specific rule is dropped:
//!
//! | case | rule it pins |
//! |---|---|
//! | 1 | a non-operator gets **404, not 403** — for an operator view the existence is the disclosure |
//! | 2 | the response carries **no per-account detail** (§53.2), at any key depth |
//! | 3 | `missing_inputs` is forwarded verbatim, and an empty list is not hidden |
//! | 4 | there is **no target and no grade** (§53.5: read, not chased) — asserted by scanning the whole body |
//!
//! Case 4 is the one that cannot be written by copying a sibling test: the rule is an
//! *absence*, so it has to be checked by walking every key rather than by asserting a field
//! equals something.
//!
//! Both engines throughout.

use serde_json::{json, Value};

use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_db::Database;

use test_support::{scratch_dir, sign_in_as, TestClient, TestDb, TEST_PASSWORD};

/// A window wide enough that seeded rows are always inside it, so a case about the response
/// shape is never also a case about window arithmetic — that is `north_star.rs`'s job.
const SINCE: &str = "2000-01-01T00:00:00Z";
const UNTIL: &str = "2999-12-31T23:59:59Z";

/// The path with the window attached, built from the two constants so they cannot drift
/// apart — a duplicated literal is how a "boundary" test ends up testing no boundary.
fn path() -> String {
    format!("/api/v1/admin/metrics/north-star?since={SINCE}&until={UNTIL}")
}

struct Harness {
    tdb: TestDb,
    db: Database,
    config: Config,
    _dir: std::path::PathBuf,
}

impl Harness {
    async fn new(tag: &str, operator_handle: &str) -> Self {
        let dir = scratch_dir(tag);
        let mut config = Config::development_defaults();
        config.storage.root = dir.clone();
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        let db = tdb.db().clone();

        // The account must exist before the config can name it, so it registers through a
        // throwaway client whose cookies are discarded. Every test signs in again after.
        let mut bootstrap = TestClient::new(server::build_router(AppState::new(
            config.clone(),
            db.clone(),
        )));
        let (status, body) = bootstrap
            .post(
                "/api/v1/auth/register",
                json!({
                    "email": format!("{operator_handle}@example.com"),
                    "password": TEST_PASSWORD,
                    "handle": operator_handle,
                    "display_name": operator_handle,
                    "age_band": "adult",
                }),
            )
            .await;
        assert!(
            status.is_success(),
            "bootstrap register for {operator_handle}: {status} {body}"
        );
        drop(bootstrap);

        let account = account_id_by_handle(&db, operator_handle).await;
        config.administration.operator_account_id = Some(account);
        // Process-global buckets at 127.0.0.1: neighbouring suites exhaust the development
        // defaults long before this file finishes.
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

        Self {
            tdb,
            db,
            config,
            _dir: dir,
        }
    }

    fn client(&self) -> TestClient {
        TestClient::new(server::build_router(AppState::new(
            self.config.clone(),
            self.db.clone(),
        )))
    }
}

async fn account_id_by_handle(db: &Database, handle: &str) -> lorehaven_domain::ids::AccountId {
    let sql = db.sql(
        "SELECT id FROM accounts WHERE email = ?",
        "SELECT id FROM accounts WHERE email = $1",
    );
    // `accounts.id` is TEXT on SQLite and UUID on PostgreSQL, so the two arms decode
    // through different Rust types. `String` on one side and `uuid::Uuid` on the other --
    // the per-dialect decode this repository keeps rediscovering, and the reason the
    // sibling `flow_dashboard.rs` takes its statements per engine too.
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            let row: (String,) = sqlx::query_as(&sql)
                .bind(format!("{handle}@example.com"))
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("account");
            row.0
                .parse::<lorehaven_domain::ids::AccountId>()
                .expect("an account id is a uuid")
        }
        lorehaven_db::Backend::Postgres => {
            let row: (uuid::Uuid,) = sqlx::query_as(&sql)
                .bind(format!("{handle}@example.com"))
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await
                .expect("account");
            lorehaven_domain::ids::AccountId::from(row.0)
        }
    }
}

/// Every key at every depth, flattened. Used by the cases whose rule is an *absence*, which
/// is the only way to test one: asserting a field equals something proves it is right, not
/// that nothing else was added.
fn all_keys(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                out.push(k.clone());
                all_keys(v, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                all_keys(item, out);
            }
        }
        _ => {}
    }
}

#[tokio::test]
async fn a_non_operator_gets_404_rather_than_403() {
    // Case 1. A 403 answers "yes, and you may not", which tells a reader probing URLs that
    // this instance tracks its own discovery quality. 404 says nothing.
    let h = Harness::new("ns_http_404", "nsop1").await;

    let mut intruder = h.client();
    sign_in_as(
        &mut intruder,
        &h.tdb,
        "not-the-operator@example.com",
        "notnsop",
    )
    .await;
    let (status, body) = intruder.get(&path()).await;
    assert_eq!(
        status,
        axum::http::StatusCode::NOT_FOUND,
        "a 403 would confirm the view exists: {body}"
    );

    // And the operator does see it — otherwise 404-for-everyone would pass this test while
    // the route was simply unmounted.
    let mut op = h.client();
    sign_in_as(&mut op, &h.tdb, "nsop1@example.com", "nsop1").await;
    let (status, body) = op.get(&path()).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
}

#[tokio::test]
async fn the_response_carries_no_per_account_detail() {
    // Case 2. §53.2 forbids per-account detail in a metrics view outright. The type-level
    // assertion exists too (`NorthStar::carries_account_detail`), but a type cannot stop a
    // route from adding a field, so the body is walked.
    let h = Harness::new("ns_http_detail", "nsop2").await;
    let mut op = h.client();
    sign_in_as(&mut op, &h.tdb, "nsop2@example.com", "nsop2").await;
    let (status, body) = op.get(&path()).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");

    let mut keys = Vec::new();
    all_keys(&body, &mut keys);
    for key in &keys {
        assert!(
            !key.contains("account")
                && !key.contains("pseud")
                && !key.contains("reader")
                && !key.contains("user"),
            "§53.2 forbids per-account detail, and the response carries `{key}`: {body}"
        );
    }
}

#[tokio::test]
async fn missing_inputs_are_forwarded_verbatim() {
    // Case 3. §53.5: the metric reports its own incompleteness. An empty database has no
    // ratings, no completions and no slots, so all three must be named — and the list must
    // not be hidden behind a default.
    let h = Harness::new("ns_http_missing", "nsop3").await;
    let mut op = h.client();
    sign_in_as(&mut op, &h.tdb, "nsop3@example.com", "nsop3").await;
    let (status, body) = op.get(&path()).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");

    let missing: Vec<String> = body["missing_inputs"]
        .as_array()
        .expect("missing_inputs is an array")
        .iter()
        .map(|v| v.as_str().expect("a string").to_string())
        .collect();
    for expected in ["ratings", "completions", "slots"] {
        assert!(
            missing.iter().any(|m| m == expected),
            "an empty window must report `{expected}` as missing: {body}"
        );
    }
    // §53.5 in the same breath: undefined is not zero. With nothing measured the median is
    // null, not 0 — a zero would read as instant discovery.
    assert_eq!(
        body["median_days_to_find"],
        Value::Null,
        "no completed pair is null, not 0: {body}"
    );
}

#[tokio::test]
async fn the_response_carries_no_target_and_no_grade() {
    // Case 4. §53.5: "The number is read, not chased." So there is no "you should be at 40%"
    // field, and §0.3 plus the standing rule against a composite score mean no single
    // north-star number either.
    //
    // This is an absence, so it can only be tested by walking every key. A test that
    // asserted `body["target"]` is absent would pass just as well with a field named
    // `goal_rate` or `expected_percentile`.
    let h = Harness::new("ns_http_no_target", "nsop4").await;
    let mut op = h.client();
    sign_in_as(&mut op, &h.tdb, "nsop4@example.com", "nsop4").await;
    let (status, body) = op.get(&path()).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");

    let mut keys = Vec::new();
    all_keys(&body, &mut keys);
    for key in &keys {
        for banned in [
            "target",
            "goal",
            "grade",
            "expected",
            "threshold",
            "benchmark",
        ] {
            assert!(
                !key.contains(banned),
                "§53.5 says the number is read, not chased, and the response carries \
                 `{key}`: {body}"
            );
        }
    }
    // And the two measures stay separate: no arithmetic on them.
    assert!(
        !keys.contains(&"north_star_score".to_string()),
        "a composite score is a ranking signal the moment anything sorts on it: {body}"
    );
}
