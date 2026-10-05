//! `/admin/media-health` over HTTP: the operator door, and the 404 it owes.
//!
//! `crates/app/tests/media_health.rs` covers the STORE behind this dashboard (counts,
//! link-rot tiers, provider reliability). It also has two route tests — and both of
//! them assert `401 UNAUTHORIZED` with a comment saying a session fixture would be
//! too complex. So the *operator* case, which is the one the dashboard actually
//! distinguishes, had never been exercised: nothing proved a curator gets in, and
//! nothing proved a reader is refused.
//!
//! What it did do was answer **403 `ACCESS_DENIED`** to anyone below trust level 5.
//! That is the wrong code for this view, and the reason is the house rule already
//! followed by `flows::require_operator`, `admin_discovery::require_operator` and
//! `decision_service::require_operator` (and written down in
//! `docs/plans/REMAINING-2026-10-03.md`): **for an operator surface the existence is
//! the disclosure**, so 403 answers "yes, and you may not" to anyone probing
//! `/admin/media-health`. 404 says nothing.
//!
//! The observable defect behind the 403 was six failed calls per page load and a red
//! "That did not work" on a page the reader may not be on. Both halves are fixed
//! here and in `AdminMediaHealth.svelte`; this file is the gate on the first.

use axum::http::StatusCode;
use serde_json::json;

use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;

use test_support::{scratch_dir, sign_in_as, TestClient, TestDb, TEST_PASSWORD};

/// Every door the dashboard opens. One refusal, six endpoints — a check of the
/// first would pass while five of them still answered 403.
const DOORS: [&str; 6] = [
    "/api/v1/admin/media-health/overview",
    "/api/v1/admin/media-health/link-rot",
    "/api/v1/admin/media-health/curator-leaderboard",
    "/api/v1/admin/media-health/bounty-status",
    "/api/v1/admin/media-health/storage",
    "/api/v1/admin/media-health/providers",
];

struct Harness {
    tdb: TestDb,
    config: Config,
    db: lorehaven_db::Database,
    _dir: std::path::PathBuf,
}

impl Harness {
    /// A harness whose operator is the account registered under `operator_handle`.
    ///
    /// Copied from `flow_dashboard::Harness` rather than invented, because the
    /// operator is named by **config** (`administration.operator_account_id`) and
    /// not by a trust tier — the account has to exist before the config can name
    /// it, so it is registered through a throwaway client.
    async fn new(tag: &str, operator_handle: &str) -> Self {
        let dir = scratch_dir(tag);
        let mut config = Config::development_defaults();
        config.storage.root = dir.clone();
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        let db = tdb.db().clone();

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

        config.administration.operator_account_id =
            Some(account_of_handle(&db, operator_handle).await);
        Self {
            tdb,
            config,
            db,
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

async fn account_of_handle(
    db: &lorehaven_db::Database,
    handle: &str,
) -> lorehaven_domain::ids::AccountId {
    // `account_id` is TEXT on SQLite and uuid on PostgreSQL, so each arm fetches
    // its own Rust type — a shared `query_scalar::<_, String>` works on one engine
    // and is a decode error on the other.
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            let value: String =
                sqlx::query_scalar("SELECT account_id FROM pseuds WHERE lower(handle) = lower(?)")
                    .bind(handle)
                    .fetch_one(db.sqlite_pool().expect("sqlite"))
                    .await
                    .expect("account id");
            value.parse().expect("account id parses")
        }
        lorehaven_db::Backend::Postgres => {
            let value: String = sqlx::query_scalar(
                "SELECT account_id::text AS account_id FROM pseuds WHERE lower(handle) = lower($1)",
            )
            .bind(handle)
            .fetch_one(db.postgres_pool().expect("postgres"))
            .await
            .expect("account id");
            value.parse().expect("account id parses")
        }
    }
}

/// A signed-in reader who is not the operator gets **404** on every door.
///
/// Not 403, and the difference is the whole point of this test: 403 confirms the
/// endpoint exists. `assert_eq!(status, NOT_FOUND)` on all six, because a fix that
/// changed `require_operator` and left one handler calling something else would
/// otherwise be invisible.
#[tokio::test]
async fn a_non_operator_gets_404_from_every_media_health_door() {
    let h = Harness::new("mh_404", "mhoperator").await;
    let mut intruder = h.client();
    sign_in_as(
        &mut intruder,
        &h.tdb,
        "not-the-operator@example.com",
        "notmhoperator",
    )
    .await;

    for door in DOORS {
        let (status, body) = intruder.get(door).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{door} must not confirm it exists to a non-operator: {body}"
        );
    }
}

/// Anonymous is still 401, not 404: the session question is a different question
/// from the operator question, and answering 404 to an anonymous caller would tell
/// a stranger that this instance runs a media-health dashboard at all.
#[tokio::test]
async fn an_anonymous_caller_still_gets_401() {
    let h = Harness::new("mh_401", "mhoperator401").await;
    let mut anonymous = h.client();

    let (status, _) = anonymous.get("/api/v1/admin/media-health/overview").await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "the session middleware speaks first, and 401 is the answer to 'who are you'"
    );
}

/// The operator gets in — the case the two pre-existing route tests skipped because
/// they had no session fixture. Without it, "always 404" would satisfy the test above.
#[tokio::test]
async fn the_operator_gets_the_overview() {
    let h = Harness::new("mh_op_ok", "mhoperatorok").await;
    let mut operator = h.client();
    sign_in_as(
        &mut operator,
        &h.tdb,
        "mhoperatorok@example.com",
        "mhoperatorok",
    )
    .await;

    let (status, body) = operator.get("/api/v1/admin/media-health/overview").await;

    assert_eq!(
        status,
        StatusCode::OK,
        "the operator the config names must reach the dashboard: {body}"
    );
    // An empty archive still answers with real numbers rather than an empty body —
    // the refusal and the answer have to be distinguishable by more than the code.
    assert!(
        body["total_references"].is_number(),
        "an empty archive is a zero, not a missing field: {body}"
    );
}
