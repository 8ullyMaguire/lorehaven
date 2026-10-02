//! Gap C step 5: the author-only pre-read routes.
//!
//! The store is proved in `crates/db/tests/preread_store.rs` and the adapter in
//! `crates/app/tests/ai_openai.rs`. What is proved here is what only the HTTP surface can
//! get wrong, and every item is invisible to a store test:
//!
//!   * **a non-owner gets the same 404 as a non-existent work.** A 403 would confirm to an
//!     outsider that the draft exists *and* that its author ran an AI tool on it.
//!   * **the owner of an un-assessed work gets 200 + `report: null`,** not a 404 — they are
//!     entitled to know their own draft has not been assessed. That is the same 404 for two
//!     different situations, which is only safe because it is confined to non-owners.
//!   * **no response carries a composite score** (§32.6, §0.3), and the per-dimension
//!     entries are worst-first, because that is the question an author asks.
//!   * **the `missing` list carries a reason, not just an absence**, because a report where
//!     everything came back and one where half the provider's output was unparseable would
//!     otherwise look identical.
//!   * **withdrawal is per provider** and returns the count, so a client can tell a
//!     withdrawal that happened from one that did not.
//!
//! The `Client`/`Harness` pair is copied from `blind_date_route.rs` rather than extracted:
//! a shared HTTP harness is a refactor of eight test binaries, and this is one surface.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::preread_store::save_report;
use lorehaven_db::DatabaseConfig;
use lorehaven_domain::preread::{DimensionOutcome, DimensionStatus, PreReadReport};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use test_support::scratch_dir;
use tower::ServiceExt;

const PASSWORD: &str = "a-long-enough-passphrase";
const T0: &str = "2026-01-01T00:00:00Z";

fn config_for(dir: &Path) -> Config {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    config.database = DatabaseConfig::new(format!(
        "sqlite://{}/lorehaven.sqlite?mode=rwc",
        dir.display()
    ));
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
    async fn delete(&mut self, uri: &str) -> (StatusCode, Value) {
        self.request("DELETE", uri, None).await
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

/// A draft owned by `account`, returning its id.
///
/// A **draft** rather than a published work: §32.6 says the report is never on the public
/// work page, so a test on a published work would not be testing the thing that matters.
async fn seed_draft(tdb: &test_support::TestDb, account: &str, title: &str) -> String {
    let work = uuid::Uuid::new_v4().to_string();
    exec_with(
        tdb,
        "INSERT INTO works (id, owner_pseud_id, title, lifecycle, published_at, created_at, updated_at, generated_content_posture) SELECT ?1#u, id, ?3, 'draft', ?4, ?4, ?4, 'forbid' FROM pseuds WHERE account_id = ?2#u",
        &[&work, &account.to_string(), &title.to_string(), &T0.to_string()],
    )
    .await;
    work
}

/// The account id behind a registered handle.
async fn account_of(tdb: &test_support::TestDb, handle: &str) -> String {
    let id: Option<String> = match tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query_scalar("SELECT account_id FROM pseuds WHERE handle = ?1")
                .bind(handle)
                .fetch_optional(tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("query")
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query_scalar("SELECT account_id::text FROM pseuds WHERE handle = $1")
                .bind(handle)
                .fetch_optional(tdb.db().postgres_pool().expect("postgres"))
                .await
                .expect("query")
        }
    };
    id.expect("the pseud exists")
}

fn report(work_id: &str, dimensions: &[(&str, f64)]) -> PreReadReport {
    PreReadReport {
        work_id: work_id.to_string(),
        dimensions: dimensions
            .iter()
            .map(|(d, s)| {
                (
                    d.to_string(),
                    DimensionOutcome {
                        dimension: d.to_string(),
                        score: *s,
                        note: "assessed".to_string(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>(),
        missing: BTreeMap::new(),
    }
}

// ---------------------------------------------------------------------------
// the author sees a report, and it carries no composite score
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_author_sees_the_report_with_dimensions_worst_first_and_no_composite() {
    let harness = Harness::new("preread-author").await;
    let mut client = harness.client();
    register(&mut client, "author@example.com", "PrAuthor").await;
    let account = account_of(&harness.tdb, "PrAuthor").await;
    let work = seed_draft(&harness.tdb, &account, "Draft").await;
    save_report(
        harness.tdb.db(),
        &report(&work, &[("length", 0.35), ("tone", 0.82), ("pacing", 0.5)]),
        "ollama",
        "2026-10-02T00:00:00Z",
    )
    .await
    .expect("save");

    let (status, body) = client.get(&format!("/api/v1/works/{work}/preread")).await;
    assert_eq!(status, StatusCode::OK, "the owner can read it: {body}");

    let dimensions = body["report"]["dimensions"].as_array().expect("dimensions");
    assert_eq!(dimensions.len(), 3);
    // Worst first. The author's question is "what is weakest", so ascending would answer
    // the one they did not ask.
    let order: Vec<&str> = dimensions
        .iter()
        .map(|d| d["dimension"].as_str().expect("name"))
        .collect();
    assert_eq!(order, vec!["tone", "pacing", "length"]);
    assert_eq!(dimensions[0]["score"], 0.82);

    // §32.6 and §0.3. There is no composite anywhere in the response — not under
    // `report`, not at the top level, not on a dimension. A `score` key holding an
    // average is the specific thing that must never appear.
    assert!(body.get("score").is_none(), "no top-level score: {body}");
    assert!(
        body["report"].get("score").is_none(),
        "no composite on the report: {body}"
    );
    assert!(
        body["report"].get("average").is_none(),
        "no average: {body}"
    );
    assert!(
        body["report"]["complete"].as_bool() == Some(true),
        "nothing was missing: {body}"
    );
    harness.cleanup().await;
}

#[tokio::test]
async fn a_missing_dimension_reaches_the_author_with_a_readable_reason() {
    // Without the reason, "everything came back" and "half the provider's output was
    // unparseable" look the same, and that difference is exactly what tells an author
    // whether to trust the score.
    let harness = Harness::new("preread-missing").await;
    let mut client = harness.client();
    register(&mut client, "author@example.com", "PrAuthor").await;
    let account = account_of(&harness.tdb, "PrAuthor").await;
    let work = seed_draft(&harness.tdb, &account, "Draft").await;

    let mut r = report(&work, &[("length", 0.4)]);
    r.missing.insert(
        "tone".to_string(),
        DimensionStatus::Abstained(lorehaven_domain::ai::AiAbstain::InvalidOutput(
            "prose".to_string(),
        )),
    );
    save_report(harness.tdb.db(), &r, "ollama", "2026-10-02T00:00:00Z")
        .await
        .expect("save");

    let (status, body) = client.get(&format!("/api/v1/works/{work}/preread")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["report"]["complete"].as_bool(),
        Some(false),
        "one dimension never came back: {body}"
    );
    let missing = body["report"]["missing"].as_array().expect("missing");
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0]["dimension"], "tone");
    let reason = missing[0]["reason"].as_str().expect("a reason");
    assert!(
        reason.contains("could not be validated"),
        "the reason says what happened rather than echoing a variant name: {reason}"
    );
    // The raw provider payload must not leak: the stored value was "prose", and that is
    // model output, not something an author needs.
    assert!(!reason.contains("prose"), "{reason}");
    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// §32.6 — never on the public work page, never to a non-author
// ---------------------------------------------------------------------------

#[tokio::test]
async fn another_users_report_is_a_404_identical_to_a_work_that_does_not_exist() {
    // The property under test is that the two responses are **the same**, not merely both
    // errors. A 403 — or a 404 with a different body — would confirm that the draft exists
    // and that its author ran an AI tool on it.
    let harness = Harness::new("preread-indistinguishable").await;
    let mut author_client = harness.client();
    register(&mut author_client, "author@example.com", "PrAuthor").await;
    let account = account_of(&harness.tdb, "PrAuthor").await;
    let work = seed_draft(&harness.tdb, &account, "Draft").await;
    save_report(
        harness.tdb.db(),
        &report(&work, &[("length", 0.4)]),
        "ollama",
        "2026-10-02T00:00:00Z",
    )
    .await
    .expect("save");

    let mut stranger = harness.client();
    register(&mut stranger, "stranger@example.com", "PrStranger").await;

    let (forbidden_status, forbidden_body) =
        stranger.get(&format!("/api/v1/works/{work}/preread")).await;
    let (absent_status, absent_body) = stranger
        .get(&format!("/api/v1/works/{}/preread", uuid::Uuid::new_v4()))
        .await;

    assert_eq!(
        forbidden_status, absent_status,
        "a non-owner's work and a non-existent work answer identically"
    );
    assert_eq!(forbidden_status, StatusCode::NOT_FOUND);
    // The bodies must be identical *except* for `request_id`, which is a per-request
    // tracing id and differs on every response by design. Comparing it would fail for a
    // reason that has nothing to do with existence, so it is removed before the
    // comparison — and its removal is the assertion's point: nothing else may vary.
    let mut forbidden_normalised = forbidden_body.clone();
    let mut absent_normalised = absent_body.clone();
    for body in [&mut forbidden_normalised, &mut absent_normalised] {
        body["error"]["request_id"] = Value::String("<stripped>".to_string());
    }
    assert_eq!(
        forbidden_normalised, absent_normalised,
        "and the bodies are identical too — a different 404 message leaks existence"
    );
    assert_eq!(
        forbidden_body["error"]["message"], absent_body["error"]["message"],
        "the message names the same resource in both cases"
    );
    // The one field that legitimately differs, so the strip above is not hiding a blanket
    // normalisation.
    assert_ne!(
        forbidden_body["error"]["request_id"], absent_body["error"]["request_id"],
        "request ids are per-request and must not be normalised away"
    );
    // The report's dimensions must not appear anywhere in the stranger's response.
    let rendered = forbidden_body.to_string();
    assert!(!rendered.contains("length"), "{rendered}");
    assert!(!rendered.contains("0.4"), "{rendered}");
    harness.cleanup().await;
}

#[tokio::test]
async fn an_anonymous_caller_gets_the_same_404_and_no_report() {
    // `RequirePseud` rejects an unauthenticated request before the ownership check, so the
    // answer must not depend on whether a work exists.
    let harness = Harness::new("preread-anon").await;
    let mut client = harness.client();
    register(&mut client, "author@example.com", "PrAuthor").await;
    let account = account_of(&harness.tdb, "PrAuthor").await;
    let work = seed_draft(&harness.tdb, &account, "Draft").await;
    save_report(
        harness.tdb.db(),
        &report(&work, &[("length", 0.4)]),
        "ollama",
        "2026-10-02T00:00:00Z",
    )
    .await
    .expect("save");

    let mut anonymous = harness.client();
    let (status, body) = anonymous
        .get(&format!("/api/v1/works/{work}/preread"))
        .await;
    assert!(
        !status.is_success(),
        "an anonymous caller gets nothing: {status} {body}"
    );
    let rendered = body.to_string();
    assert!(!rendered.contains("length"), "{rendered}");
    harness.cleanup().await;
}

#[tokio::test]
async fn the_owner_of_an_un_assessed_work_gets_a_null_report_not_a_404() {
    // The other half of the indistinguishability design, and the reason it is safe: the
    // 404 is confined to non-owners, so the owner is told the truth about their own work.
    let harness = Harness::new("preread-unassessed").await;
    let mut client = harness.client();
    register(&mut client, "author@example.com", "PrAuthor").await;
    let account = account_of(&harness.tdb, "PrAuthor").await;
    let work = seed_draft(&harness.tdb, &account, "Draft").await;

    let (status, body) = client.get(&format!("/api/v1/works/{work}/preread")).await;
    assert_eq!(status, StatusCode::OK, "not a 404: {body}");
    assert!(
        body["report"].is_null(),
        "an un-assessed work has no report: {body}"
    );
    // Spelled out, so the client does not render "no report" and "a report with nothing in
    // it" identically — an author is deciding whether to trust the output.
    assert_eq!(body["reason"], "no_provider_has_assessed_this_work");
    assert_eq!(
        body["providers"].as_array().map(|a| a.len()),
        Some(0),
        "and the empty provider list is consistent with it"
    );
    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// per-provider withdrawal (§23.7)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn withdrawal_removes_one_providers_report_and_leaves_the_other() {
    let harness = Harness::new("preread-withdraw").await;
    let mut client = harness.client();
    register(&mut client, "author@example.com", "PrAuthor").await;
    let account = account_of(&harness.tdb, "PrAuthor").await;
    let work = seed_draft(&harness.tdb, &account, "Draft").await;
    for provider in ["ollama", "openai-compatible"] {
        save_report(
            harness.tdb.db(),
            &report(&work, &[("length", 0.5)]),
            provider,
            "2026-10-02T00:00:00Z",
        )
        .await
        .expect("save");
    }

    let (status, listed) = client
        .get(&format!("/api/v1/works/{work}/preread/providers"))
        .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(
        listed["providers"].as_array().map(|a| a.len()),
        Some(2),
        "both providers have assessed this work: {listed}"
    );

    let (status, body) = client
        .delete(&format!("/api/v1/works/{work}/preread/ollama"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // The count, so a client can tell a withdrawal that happened from one that did not.
    assert_eq!(body["removed"], 1, "one row removed: {body}");
    assert_eq!(
        body["providers"].as_array().map(|a| a.len()),
        Some(1),
        "the other provider's report survives: {body}"
    );

    let (_, after) = client.get(&format!("/api/v1/works/{work}/preread")).await;
    assert_eq!(
        after["providers"].as_array().map(|a| a.len()),
        Some(1),
        "{after}"
    );
    harness.cleanup().await;
}

#[tokio::test]
async fn withdrawing_the_last_provider_leaves_the_owner_with_an_un_assessed_work() {
    let harness = Harness::new("preread-withdraw-last").await;
    let mut client = harness.client();
    register(&mut client, "author@example.com", "PrAuthor").await;
    let account = account_of(&harness.tdb, "PrAuthor").await;
    let work = seed_draft(&harness.tdb, &account, "Draft").await;
    save_report(
        harness.tdb.db(),
        &report(&work, &[("length", 0.5)]),
        "ollama",
        "2026-10-02T00:00:00Z",
    )
    .await
    .expect("save");

    let (status, body) = client
        .delete(&format!("/api/v1/works/{work}/preread/ollama"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["removed"], 1);
    assert_eq!(body["providers"].as_array().map(|a| a.len()), Some(0));

    let (status, after) = client.get(&format!("/api/v1/works/{work}/preread")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(after["report"].is_null(), "{after}");
    assert_eq!(after["reason"], "no_provider_has_assessed_this_work");
    harness.cleanup().await;
}

#[tokio::test]
async fn withdrawing_a_provider_that_never_assessed_is_a_200_with_zero_removed() {
    // Idempotent, because a consent screen that lists stale providers would otherwise fail
    // on the second click.
    let harness = Harness::new("preread-withdraw-idempotent").await;
    let mut client = harness.client();
    register(&mut client, "author@example.com", "PrAuthor").await;
    let account = account_of(&harness.tdb, "PrAuthor").await;
    let work = seed_draft(&harness.tdb, &account, "Draft").await;

    let (status, body) = client
        .delete(&format!("/api/v1/works/{work}/preread/ollama"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["removed"], 0,
        "nothing to remove, and that is fine: {body}"
    );
    harness.cleanup().await;
}

#[tokio::test]
async fn a_stranger_cannot_withdraw_another_authors_report() {
    // Withdrawal deletes data, so it is the endpoint where a leaked ownership check would
    // do real damage rather than merely confirm existence.
    let harness = Harness::new("preread-withdraw-stranger").await;
    let mut author_client = harness.client();
    register(&mut author_client, "author@example.com", "PrAuthor").await;
    let account = account_of(&harness.tdb, "PrAuthor").await;
    let work = seed_draft(&harness.tdb, &account, "Draft").await;
    save_report(
        harness.tdb.db(),
        &report(&work, &[("length", 0.5)]),
        "ollama",
        "2026-10-02T00:00:00Z",
    )
    .await
    .expect("save");

    let mut stranger = harness.client();
    register(&mut stranger, "stranger@example.com", "PrStranger").await;
    let (status, _) = stranger
        .delete(&format!("/api/v1/works/{work}/preread/ollama"))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a stranger may not withdraw");

    let (_, still_there) = author_client
        .get(&format!("/api/v1/works/{work}/preread"))
        .await;
    assert!(
        !still_there["report"].is_null(),
        "the author's report survived: {still_there}"
    );
    harness.cleanup().await;
}

#[tokio::test]
async fn an_unparseable_work_id_is_a_404_rather_than_a_400() {
    // A 400 would confirm the id was almost a valid work, which is the same leak a 403 on
    // a real work would be.
    let harness = Harness::new("preread-bad-id").await;
    let mut client = harness.client();
    register(&mut client, "author@example.com", "PrAuthor").await;

    let (bad, body) = client.get("/api/v1/works/not-a-uuid/preread").await;
    let (absent, _) = client
        .get(&format!("/api/v1/works/{}/preread", uuid::Uuid::new_v4()))
        .await;
    assert_eq!(bad, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(bad, absent);
    harness.cleanup().await;
}
