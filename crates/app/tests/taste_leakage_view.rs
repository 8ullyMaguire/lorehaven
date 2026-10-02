//! §52.1 — acceptance tests for `GET /admin/discovery/leakage`.
//!
//! The rules under test are about what the response DOES NOT say, so several of
//! these assert on absence: no row count, no completeness claim, no numeric
//! wording, and a 404 rather than a 403 for anyone who is not the operator.
//!
//! Green on SQLite and PostgreSQL.

use std::path::Path;

use axum::http::StatusCode;
use serde_json::json;

use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_db::taste_leakage as tl;
use lorehaven_domain::ids::{PseudId, WorkId};
use lorehaven_domain::leakage::{Disposition, OwnerResonance};
use test_support::{scratch_dir, sign_in_as, TestClient, TestDb};

const T0: i64 = 1_767_225_600; // 2026-01-01

/// A harness whose registered `operator` account holds the operator role.
///
/// The account is registered *before* the config is finalised, because
/// `require_operator` compares the session's account against
/// `config.administration.operator_account_id`.
struct Harness {
    tdb: TestDb,
    config: Config,
}

impl Harness {
    async fn build(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let mut config = config_for(&dir);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        let db = tdb.db().clone();

        let mut bootstrap =
            TestClient::new(server::build_router(AppState::new(config.clone(), db)));
        test_support::register(&mut bootstrap, "operator@example.com", "operator").await;
        let account = account_of(&tdb, "operator@example.com").await;
        config.administration.operator_account_id = Some(account);
        Self { tdb, config }
    }

    fn client(&self) -> TestClient {
        TestClient::new(server::build_router(AppState::new(
            self.config.clone(),
            self.tdb.db().clone(),
        )))
    }

    /// A client already signed in as `handle`.
    async fn signed_in(&self, email: &str, handle: &str) -> TestClient {
        let mut client = self.client();
        sign_in_as(&mut client, &self.tdb, email, handle).await;
        client
    }

    async fn operator(&self) -> TestClient {
        self.signed_in("operator@example.com", "operator").await
    }

    /// The operator's own pseud, which 0110 requires as the reviewer.
    async fn operator_pseud(&self) -> PseudId {
        pseud_of(&self.tdb, "operator").await
    }

    async fn review(&self, artifact: &str, inferable: &str, ease: &str, d: Disposition, at: i64) {
        tl::record_review(
            self.tdb.db(),
            artifact,
            inferable,
            ease,
            d,
            self.operator_pseud().await,
            at,
        )
        .await
        .expect("review recorded");
    }
}

fn config_for(dir: &Path) -> Config {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    // Honoured, so this file runs on whichever backend the selector names.
    config.database = match std::env::var("LOREHAVEN_TEST_PG_URL") {
        Ok(url) => lorehaven_db::DatabaseConfig::new(url),
        Err(_) => lorehaven_db::DatabaseConfig::new(format!(
            "sqlite://{}/lorehaven.sqlite?mode=rwc",
            dir.display()
        )),
    };
    // The rate-limit buckets are process-global at 127.0.0.1, so the development
    // defaults are exhausted by neighbouring suites long before this file's own
    // requests finish.
    config.rate_limits.auth = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config
}

/// Read one text column, per dialect.
///
/// `accounts.id` and `pseuds.id` are TEXT on SQLite and native UUID on
/// PostgreSQL, and `test_support::fetch_text` does not cast -- so reading one
/// through it passes every SQLite test and fails on PostgreSQL with "Rust type
/// Option<String> (as SQL type TEXT) is not compatible with SQL type UUID". The
/// `::text` cast in the PostgreSQL arm is the whole fix, and it is the same shape
/// as `analytics_k_anonymity.rs` and `decision_audit_routes.rs`.
async fn one_text(
    db: &lorehaven_db::Database,
    sqlite_sql: &str,
    postgres_sql: &str,
    arg: &str,
) -> String {
    match db.backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(sqlite_sql)
            .bind(arg)
            .fetch_one(db.sqlite_pool().expect("sqlite pool"))
            .await
            .expect("scalar"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(postgres_sql)
            .bind(arg)
            .fetch_one(db.postgres_pool().expect("postgres pool"))
            .await
            .expect("scalar"),
    }
}

async fn account_of(tdb: &TestDb, email: &str) -> lorehaven_domain::ids::AccountId {
    let id = one_text(
        tdb.db(),
        "SELECT id FROM accounts WHERE email = ?",
        "SELECT id::text FROM accounts WHERE email = $1",
        email,
    )
    .await;
    lorehaven_domain::ids::AccountId::from_uuid(uuid::Uuid::parse_str(&id).expect("a uuid"))
}

async fn pseud_of(tdb: &TestDb, handle: &str) -> PseudId {
    let id = one_text(
        tdb.db(),
        "SELECT id FROM pseuds WHERE handle = ?",
        "SELECT id::text FROM pseuds WHERE handle = $1",
        handle,
    )
    .await;
    PseudId::from_uuid(uuid::Uuid::parse_str(&id).expect("a uuid"))
}

/// A work owned by the operator, for the §52.3 owner-visibility test.
async fn work_fixture(tdb: &TestDb, owner: PseudId) -> WorkId {
    let w = WorkId::new();
    let now = "2026-01-01T00:00:00Z";
    let db = tdb.db();
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at, \
                 generated_content_posture) VALUES (?1, ?2, 'W', ?3, ?3, 'forbid')",
            )
            .bind(w.to_canonical_string())
            .bind(owner.to_canonical_string())
            .bind(now)
            .execute(db.sqlite_pool().expect("sqlite pool"))
            .await
            .expect("work");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at, \
                 generated_content_posture) VALUES ($1::uuid, $2::uuid, 'W', $3, $3, 'forbid')",
            )
            .bind(w.as_uuid())
            .bind(owner.as_uuid())
            .bind(now)
            .execute(db.postgres_pool().expect("postgres pool"))
            .await
            .expect("work");
        }
    }
    w
}

// ── the tests ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_signed_in_reader_gets_not_found_not_forbidden() {
    // §52.1, and the reason `require_operator` mirrors
    // `decision_service::require_operator`: a 403 confirms the endpoint exists,
    // and for THIS view the existence is a disclosure -- a reader probing
    // /admin/discovery learns the operator runs a leakage review at all, which is
    // one of the things the review is about.
    let h = Harness::build("leak-reader").await;
    let mut reader = h.signed_in("reader@example.com", "reader").await;
    let (status, body) = reader.get("/api/v1/admin/discovery/leakage").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a non-operator gets 404 so the endpoint's existence stays unconfirmed: {body}"
    );
}

#[tokio::test]
async fn an_anonymous_caller_is_refused_before_the_operator_check() {
    let h = Harness::build("leak-anon").await;
    let mut anon = h.client();
    let (status, _body) = anon.get("/api/v1/admin/discovery/leakage").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_operator_sees_the_reviewed_rows() {
    let h = Harness::build("leak-rows").await;
    h.review(
        "standing_bounty_payout",
        "standing bounties in this fandom pay promptly",
        "plain",
        Disposition::Coarsen,
        T0,
    )
    .await;
    h.review(
        "vanguard_selection",
        "the operator's picks lean toward one kind of story",
        "derived",
        Disposition::Keep,
        T0 + 60,
    )
    .await;

    let mut client = h.operator().await;
    let (status, body) = client.get("/api/v1/admin/discovery/leakage").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows = body["rows"].as_array().expect("rows is an array");
    assert_eq!(rows.len(), 2, "both reviewed rows are listed: {body}");
    assert_eq!(rows[0]["artifact"], "vanguard_selection", "newest first");
    assert_eq!(rows[0]["disposition"], "keep");
    assert_eq!(rows[0]["ease"], "derived");
    assert_eq!(rows[1]["disposition"], "coarsen");
}

#[tokio::test]
async fn the_response_carries_no_count_and_no_completeness_claim() {
    // §52.1: a public "we hide N things" count would itself become a probe, since
    // the difference between the count now and after a configuration change is a
    // measurement of the operator's taste.
    let h = Harness::build("leak-nocount").await;
    for i in 0..3 {
        h.review(
            &format!("artifact_{i}"),
            "a reader could put this together",
            "plain",
            Disposition::Keep,
            T0 + i,
        )
        .await;
    }

    let mut client = h.operator().await;
    let (status, body) = client.get("/api/v1/admin/discovery/leakage").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let keys: Vec<String> = body
        .as_object()
        .expect("an object")
        .keys()
        .cloned()
        .collect();
    for forbidden in ["total", "count", "reviewed_count", "exhaustive_count"] {
        assert!(
            !keys.contains(&forbidden.to_string()),
            "the response must not report a count; found {forbidden:?} in {keys:?}"
        );
    }
    assert_eq!(
        body["exhaustive"],
        json!(false),
        "§52.1: the view never certifies completeness, and says so rather than \
         leaving it to be assumed either way"
    );
}

#[tokio::test]
async fn a_row_whose_wording_carries_a_number_is_withheld_entirely() {
    // §0.3 forbids the lens being inferable through any user-facing label, and a
    // string can carry a measurement. The row is neither shown NOR named in a
    // withheld list -- naming it would publish its contents, which is the
    // disclosure the wording rule was preventing.
    let h = Harness::build("leak-precision").await;
    h.review(
        "taste_vector",
        "your affinity for this trope is 0.73",
        "measured",
        Disposition::Remove,
        T0,
    )
    .await;
    h.review(
        "topics",
        "you favour this fandom's tropes",
        "plain",
        Disposition::Keep,
        T0 + 1,
    )
    .await;
    // A digit with NO decimal point. The `.` branch cannot catch this, so it
    // exercises the digit check on its own -- and without it, deleting the digit
    // check leaves this whole file green, because every other case carries a
    // decimal too.
    h.review(
        "reading_speed",
        "your sessions tend to finish in under 7 minutes",
        "derived",
        Disposition::Coarsen,
        T0 + 2,
    )
    .await;

    let mut client = h.operator().await;
    let (status, body) = client.get("/api/v1/admin/discovery/leakage").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows = body["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 1, "only the prose row is displayable: {body}");
    assert_eq!(rows[0]["artifact"], "topics");
    let serialized = body.to_string();
    assert!(
        !serialized.contains("taste_vector"),
        "a withheld row must not be named anywhere in the response: {serialized}"
    );
    assert!(
        !serialized.contains("0.73"),
        "the withheld row's wording must not appear: {serialized}"
    );
}

#[tokio::test]
async fn the_disposition_filter_narrows_and_validates() {
    let h = Harness::build("leak-filter").await;
    h.review(
        "kept_artifact",
        "a reader could put this together",
        "plain",
        Disposition::Keep,
        T0,
    )
    .await;
    h.review(
        "removed_artifact",
        "and this one leaks by timing",
        "measured",
        Disposition::Remove,
        T0 + 1,
    )
    .await;

    let mut client = h.operator().await;
    let (status, body) = client
        .get("/api/v1/admin/discovery/leakage?disposition=remove")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows = body["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 1, "{body}");
    assert_eq!(rows[0]["artifact"], "removed_artifact");

    let (status, body) = client
        .get("/api/v1/admin/discovery/leakage?disposition=nonsense")
        .await;
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
        "an unknown disposition is a client error, not a silent empty list: {status} {body}"
    );
}

#[tokio::test]
async fn an_instance_with_no_reviews_returns_an_empty_list_and_no_date() {
    // A review that has not happened has no date. Inventing one would let an
    // operator's own view imply a review ran.
    let h = Harness::build("leak-empty").await;
    let mut client = h.operator().await;
    let (status, body) = client.get("/api/v1/admin/discovery/leakage").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["rows"], json!([]), "{body}");
    assert_eq!(body["reviewed_at"], json!(0), "{body}");
}

#[tokio::test]
async fn the_owner_visible_label_never_reaches_a_reader() {
    // §52.3: the label is owner-visible only -- never on a public work row, never
    // in an API response a reader can see. Asserted against the public work view,
    // because that is the surface a reader actually holds.
    let h = Harness::build("leak-owneronly").await;
    let owner = h.operator_pseud().await;
    let w = work_fixture(&h.tdb, owner).await;
    let window = tl::open_window(h.tdb.db(), T0).await.expect("window opens");
    tl::close_window(h.tdb.db(), window, T0 + 7 * 86_400)
        .await
        .expect("window closes");
    tl::set_resonance_label(
        h.tdb.db(),
        owner,
        w,
        OwnerResonance::Noticed,
        window,
        T0 + 7 * 86_400,
    )
    .await
    .expect("label written");

    let mut reader = h.signed_in("reader@example.com", "reader").await;
    let (status, body) = reader
        .get(&format!("/api/v1/works/{}", w.to_canonical_string()))
        .await;
    // The work is a draft, so a reader may legitimately be refused. Either way the
    // label must not be in the response.
    if status == StatusCode::OK {
        let serialized = body.to_string();
        assert!(
            !serialized.contains("noticed") && !serialized.contains("resonance"),
            "§52.3: the owner-visible label must not reach a reader: {serialized}"
        );
    }
}
