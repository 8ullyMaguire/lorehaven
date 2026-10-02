//! M45-18 — §53: the faucet/sink dashboard, tested at the store and over HTTP.
//!
//! The store's job is to classify every credit movement in a window and to *count* the ones
//! nobody classified. The route's job is to show that without disclosing that it exists.
//!
//! Six cases, in the order they matter. Case 0 is the one that pins the row's design
//! decision, and cases 2, 3 and 6 will not pass by accident — each fails if a specific rule
//! is dropped.
//!
//! Both engines throughout. Every helper takes the SQLite and PostgreSQL statement separately
//! because the two disagree on more than placeholders here: `credit_transactions.id` is TEXT on
//! SQLite and uuid on PostgreSQL, so a literal that works on one is a type error on the other.

use serde_json::{json, Value};

use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_db::economy::post_transaction;
use lorehaven_db::flow_store::mechanisms_in_window;
use lorehaven_db::{Backend, Database};
use lorehaven_domain::economy::TxnType;
use lorehaven_domain::flows::{Flow, FlowSummary, Mechanism};

use test_support::{scratch_dir, sign_in_as, TestClient, TestDb, TEST_PASSWORD};

const SINCE: &str = "2000-01-01T00:00:00Z";
const UNTIL: &str = "2999-12-31T23:59:59Z";

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Harness {
    tdb: TestDb,
    db: Database,
    config: Config,
    _dir: std::path::PathBuf,
}

impl Harness {
    /// A harness whose operator is the account registered under `operator_handle`.
    async fn new(tag: &str, operator_handle: &str) -> Self {
        let dir = scratch_dir(tag);
        let mut config = Config::development_defaults();
        config.storage.root = dir.clone();
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        let db = tdb.db().clone();

        // The account must exist before the config can name it, so it is registered
        // through a throwaway client whose cookies are then discarded. Every test signs in
        // again afterwards.
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

    /// Post one transaction and force its timestamps to `at`.
    ///
    /// `post_transaction` stamps `created_at` with `now()`, so the window cases cannot use it
    /// directly — every row would land inside whatever default window the route picked, and the
    /// boundary test would pass for the wrong reason. Rewriting the timestamps afterwards is
    /// the honest way to place a row in time, and it keeps the production write path (including
    /// its idempotency check) under test.
    async fn post_at(&self, txn_type: TxnType, reference: &str, amount: i64, at: &str) {
        let key = format!("flowtest:{txn_type:?}:{reference}:{amount}:{at}");
        post_transaction(
            &self.db,
            txn_type,
            &key,
            reference,
            &[("acct-flowtest".to_owned(), "earned".to_owned(), amount)],
        )
        .await
        .expect("post_transaction");

        let id_sql = "SELECT id FROM credit_transactions WHERE idempotency_key = ?";
        let id_pg = "SELECT id::text FROM credit_transactions WHERE idempotency_key = $1";
        let txn_id: String = match self.db.backend() {
            Backend::Sqlite => sqlx::query_scalar(id_sql)
                .bind(&key)
                .fetch_one(self.db.sqlite_pool().expect("sqlite"))
                .await
                .expect("transaction row"),
            Backend::Postgres => sqlx::query_scalar(id_pg)
                .bind(&key)
                .fetch_one(self.db.postgres_pool().expect("postgres"))
                .await
                .expect("transaction row"),
        };

        self.exec(
            "UPDATE credit_transactions SET created_at = ? WHERE id = ?",
            "UPDATE credit_transactions SET created_at = $1 WHERE id = $2",
            &[at.to_owned(), txn_id.clone()],
        )
        .await;
        self.exec(
            "UPDATE credit_entries SET created_at = ? WHERE transaction_id = ?",
            "UPDATE credit_entries SET created_at = $1 WHERE transaction_id = $2",
            &[at.to_owned(), txn_id],
        )
        .await;
    }

    /// Run one fixture statement on whichever engine is active.
    ///
    /// Two statements rather than one built from `Database::sql`, because these bind
    /// positional text and the returned `Result` carries a different `QueryResult` type per
    /// engine -- so a single `match` assigning one `result` cannot typecheck. The dialect
    /// split here is the same one `flow_store` makes, for the same reason.
    async fn exec(&self, sqlite: &str, postgres: &str, binds: &[String]) {
        match self.db.backend() {
            Backend::Sqlite => {
                let mut q = sqlx::query(sqlite);
                for b in binds {
                    q = q.bind(b);
                }
                q.execute(self.db.sqlite_pool().expect("sqlite"))
                    .await
                    .expect("fixture statement");
            }
            Backend::Postgres => {
                let mut q = sqlx::query(postgres);
                for b in binds {
                    q = q.bind(b);
                }
                q.execute(self.db.postgres_pool().expect("postgres"))
                    .await
                    .expect("fixture statement");
            }
        }
    }
}

async fn account_id_by_handle(db: &Database, handle: &str) -> lorehaven_domain::ids::AccountId {
    let sql = match db.backend() {
        Backend::Sqlite => "SELECT account_id FROM pseuds WHERE lower(handle) = lower(?)",
        Backend::Postgres => {
            "SELECT account_id::text AS account_id FROM pseuds WHERE lower(handle) = lower($1)"
        }
    };
    // Each arm parses into the same `AccountId` rather than collecting into one variable:
    // `account_id` is TEXT on SQLite and uuid on PostgreSQL, so the two fetch different
    // Rust types. This is the split `m29_transparency::account_of` handles the same way.
    match db.backend() {
        Backend::Sqlite => {
            let value: String = sqlx::query_scalar(sql)
                .bind(handle)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("account id");
            value.parse().expect("account id parses")
        }
        Backend::Postgres => {
            // The type is annotated rather than inferred: without it the compiler takes the
            // arm's type from the SQLite side's `parse` result and then asks whether
            // `AccountId` decodes from `uuid`, which it does not.
            let value: String = sqlx::query_scalar(sql)
                .bind(handle)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await
                .expect("account id");
            value.parse().expect("account id parses")
        }
    }
}

/// The mechanism entry for `key`, failing the test if it is absent.
fn mechanism<'a>(body: &'a Value, key: &str) -> &'a Value {
    body["mechanisms"]
        .as_array()
        .expect("mechanisms is an array")
        .iter()
        .find(|m| m["key"] == key)
        .unwrap_or_else(|| {
            panic!(
                "no mechanism {key:?} in {}",
                serde_json::to_string(&body["mechanisms"]).expect("json")
            )
        })
}

// ---------------------------------------------------------------------------
// Case 0 — the same key prefix on opposite sides of the loop
// ---------------------------------------------------------------------------

/// The row's central claim: `preservation_dues` and `preservation_reclaim` post the **same**
/// `reference` (a member id) on **opposite** sides of the closed loop.
///
/// An implementation that keyed the registry on `credit_transactions.reference` cannot pass
/// this: it would find one key claiming two sides, and would either drop both or pick one.
/// Since a member id differs per member, it would also match nothing at all in production and
/// report the whole preservation mechanism as undeclared.
#[tokio::test]
async fn preservation_dues_and_its_reclaim_classify_oppositely_from_one_reference() {
    let f = Harness::new("flow_same_ref", "flowop0").await;
    let member = "member-shared-reference";

    f.post_at(TxnType::Preservation, member, -400, SINCE).await;
    f.post_at(TxnType::PreservationReclaim, member, 250, SINCE)
        .await;

    let mechanisms = mechanisms_in_window(&f.db, SINCE, UNTIL)
        .await
        .expect("window");
    let by_key = |k: &str| {
        mechanisms
            .iter()
            .find(|m| m.key == k)
            .unwrap_or_else(|| panic!("no {k:?} among {:?}", keys(&mechanisms)))
    };

    assert_eq!(
        by_key("preservation_dues").declaration.flow,
        Flow::Sink,
        "dues are a sink"
    );
    assert_eq!(
        by_key("preservation_reclaim").declaration.flow,
        Flow::Faucet,
        "the reclaim returns the credits, so it is a faucet"
    );
    assert_eq!(by_key("preservation_dues").net_credits, -400);
    assert_eq!(by_key("preservation_reclaim").net_credits, 250);

    // Two distinct mechanisms despite one shared reference. An implementation that grouped on
    // the reference would report a single row here.
    assert_ne!(
        by_key("preservation_dues").key,
        by_key("preservation_reclaim").key
    );
}

// ---------------------------------------------------------------------------
// Case 1 — declared sides land in the right totals
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_declared_faucet_and_sink_land_on_their_own_sides() {
    let f = Harness::new("flow_sides", "flowop1").await;
    f.post_at(TxnType::Earn, "work-a", 900, SINCE).await;
    f.post_at(TxnType::Spend, "tip:work-b", -300, SINCE).await;

    let summary = FlowSummary::compose(
        &mechanisms_in_window(&f.db, SINCE, UNTIL)
            .await
            .expect("window"),
    );
    assert_eq!(summary.faucet_credits, 900, "earnings are a faucet");
    assert_eq!(summary.sink_credits, -300, "a tip is a sink");
    assert_eq!(summary.net_credits, 600);
    assert_eq!(summary.undeclared, 0, "both are declared");
}

fn keys(mechanisms: &[Mechanism]) -> Vec<&str> {
    mechanisms.iter().map(|m| m.key.as_str()).collect()
}

// ---------------------------------------------------------------------------
// Case 2 — a negative faucet stays a faucet
// ---------------------------------------------------------------------------

/// The reason the side is declared rather than inferred from the sign.
///
/// A bug in a faucet produces a *negative* amount, which is precisely the case where
/// "negative means sink" is wrong. Inferring would move it to the sink side and the
/// composition would look correct — the failure that would be invisible on a dashboard.
#[tokio::test]
async fn a_negative_faucet_still_counts_as_a_faucet() {
    let f = Harness::new("flow_neg_faucet", "flowop2").await;
    f.post_at(TxnType::Earn, "work-buggy", -777, SINCE).await;

    let mechanisms = mechanisms_in_window(&f.db, SINCE, UNTIL)
        .await
        .expect("window");
    let earnings = mechanisms
        .iter()
        .find(|m| m.key == "author_earnings")
        .expect("author_earnings");

    assert_eq!(
        earnings.declaration.flow,
        Flow::Faucet,
        "the declaration is not re-read from the sign"
    );
    let summary = FlowSummary::compose(&mechanisms);
    assert_eq!(
        summary.faucet_credits, -777,
        "it stays on its declared side"
    );
    assert_eq!(
        summary.sink_credits, 0,
        "and does not migrate to the sink side"
    );
    assert_eq!(summary.undeclared, 0);
}

// ---------------------------------------------------------------------------
// Case 3 — an undeclared mechanism is counted, never dropped
// ---------------------------------------------------------------------------

/// §53.1: a mechanism with no declaration is a mechanism the operator cannot reason about,
/// and a dashboard that silently omitted it would report a smaller economy than exists.
///
/// `TxnType::Grant` has no declaration: nothing in the economy posts it today. If a future
/// mechanism does, this row must show up rather than disappear.
#[tokio::test]
async fn an_undeclared_mechanism_is_counted_and_its_credits_still_count() {
    let f = Harness::new("flow_undeclared", "flowop3").await;
    f.post_at(TxnType::Earn, "work-real", 500, SINCE).await;
    f.post_at(TxnType::Grant, "mystery-thing", 4_200, SINCE)
        .await;

    let mechanisms = mechanisms_in_window(&f.db, SINCE, UNTIL)
        .await
        .expect("window");
    let summary = FlowSummary::compose(&mechanisms);

    assert_eq!(
        summary.undeclared,
        1,
        "the undeclared mechanism is counted, not dropped: {:?}",
        keys(&mechanisms)
    );
    assert_eq!(
        summary.net_credits, 4_700,
        "its credits are still real and must be in the total"
    );
    assert_eq!(summary.faucet_credits, 500, "but it is on neither side");
    assert_eq!(summary.sink_credits, 0);

    let phantom = mechanisms
        .iter()
        .find(|m| !m.declaration.flow.is_declared())
        .expect("the undeclared mechanism is present as its own row");
    assert_eq!(phantom.net_credits, 4_200, "with its amount intact");
}

// ---------------------------------------------------------------------------
// Case 4 — window boundaries are inclusive at both ends
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_window_includes_both_endpoints_and_excludes_either_side() {
    let f = Harness::new("flow_window", "flowop4").await;
    let early = "2026-01-01T00:00:00Z";
    let on_since = "2026-03-01T00:00:00Z";
    let on_until = "2026-03-31T23:59:59Z";
    let late = "2026-06-01T00:00:00Z";

    f.post_at(TxnType::Earn, "work-early", 1, early).await;
    f.post_at(TxnType::Earn, "work-since", 10, on_since).await;
    f.post_at(TxnType::Earn, "work-until", 100, on_until).await;
    f.post_at(TxnType::Earn, "work-late", 1_000, late).await;

    let summary = FlowSummary::compose(
        &mechanisms_in_window(&f.db, on_since, on_until)
            .await
            .expect("window"),
    );

    assert_eq!(
        summary.faucet_credits, 110,
        "the row on `since` and the row on `until` are both inside, \
         and the rows outside contribute nothing"
    );
}

// ---------------------------------------------------------------------------
// Case 5 — the route does not confirm it exists
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_non_operator_gets_404_rather_than_403_and_the_totals_never_clamp() {
    let f = Harness::new("flow_http", "flowop5").await;
    // Dated *now*, not at `SINCE`: this test asks the route with no `?since=`, so the row
    // has to fall inside the route's default 30-day window or it is correctly excluded and
    // the net is 0. The window cases above are the ones that use `SINCE`.
    f.post_at(
        TxnType::Earn,
        "work-a",
        1_000_000,
        &lorehaven_db::identity::now_rfc3339(),
    )
    .await;

    // A non-operator must get the same 404 the rest of the admin surface uses: a 403
    // confirms the endpoint exists, and for this view the existence IS the disclosure.
    let mut intruder = f.client();
    sign_in_as(
        &mut intruder,
        &f.tdb,
        "not-the-operator@example.com",
        "nottheop",
    )
    .await;
    let (status, _) = intruder.get("/api/v1/admin/economy/flows").await;
    assert_eq!(
        status,
        axum::http::StatusCode::NOT_FOUND,
        "a 403 would confirm the dashboard exists"
    );

    // The operator sees the real numbers, and crossing the threshold changes nothing about
    // them. §53.2: no automatic throttle — §0.3 makes bought ranking and bought trust
    // non-negotiable, and a silent clamp would be the economy deciding what a reader may earn.
    let mut op = f.client();
    sign_in_as(&mut op, &f.tdb, "flowop5@example.com", "flowop5").await;
    let (status, body) = op.get("/api/v1/admin/economy/flows").await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");

    assert_eq!(body["net_credits"], json!(1_000_000), "the real net");
    assert_eq!(body["faucet_credits"], json!(1_000_000));
    assert_eq!(body["mechanisms"].as_array().expect("array").len(), 1);

    // The breakdown is present, not just the totals: §53.2's "balance and a composition"
    // is useless without the parts.
    assert_eq!(mechanism(&body, "author_earnings")["declared"], json!(true));
    assert_eq!(mechanism(&body, "author_earnings")["flow"], json!("faucet"));

    // No per-account detail anywhere in the response. §53.2 forbids it, and
    // FlowSummary::carries_account_detail() is the existing assertion of that.
    let rendered = body.to_string();
    assert!(
        !rendered.contains("acct-flowtest"),
        "the response leaked an account id: {rendered}"
    );
    assert!(
        !rendered.contains("per_account") && !rendered.contains("perAccount"),
        "the response carries a per-account field: {rendered}"
    );
}
