//! Flexible bounties (`crates/db/src/bounties.rs`, spec §20.3.2).
//!
//! Four `pub async fn` with no test touching them, behind the economy page's
//! bounty board: `create_bounty_typed`, `contribute_to_bounty`, `fetch_bounty`
//! and `list_flexible_bounties`.
//!
//! **Unlike `roadmap` (M45), this schema is portable by accident rather than by
//! design.** `bounties` was created in migration 0017 as all-`TEXT` and never
//! converted, so there is no `UUID` and no `TIMESTAMPTZ` anywhere in the module
//! and no `::text` cast. The one hazard is `amount` and `funded_amount`, which
//! migration 0034 added as `INTEGER`: `INT4` on PostgreSQL and a 64-bit integer
//! on SQLite. Both PostgreSQL arms widen them (`amount::bigint` in
//! `fetch_bounty`, `CAST(... AS BIGINT)` in `list_flexible_bounties`) so the
//! shared `BountyRow` tuple can decode as `i64` on both. Those casts are
//! load-bearing, not decoration — `a_single_bounty_decodes_on_both_backends`
//! and `the_list_widens_its_integers` pin that.
//!
//! **Writing the first tests found the contribution ledger broken on
//! PostgreSQL, and quietly wrong on SQLite.** `record_contribution` inserted
//! into `bounty_contributions` without its `id` column, and the two backends
//! disagree about what that means:
//!
//! - PostgreSQL: `id TEXT PRIMARY KEY` is `NOT NULL`, so every insert was
//!   rejected. A crowdfunded bounty accumulated `funded_amount` with no
//!   contribution row at all — the audit trail the table exists to provide was
//!   simply absent on the production backend.
//! - SQLite: a `TEXT PRIMARY KEY` is only `NOT NULL` for the rowid alias, and
//!   this column is a plain non-unique-index primary key, so SQLite **accepted**
//!   the insert and stored `id = NULL`. The ledger had rows, but every primary
//!   key was NULL, so it could not identify a contribution, and a second NULL
//!   is indistinguishable from the first.
//!
//! The caller discarded the error with `let _ =`, so neither backend reported
//! anything. Both arms now supply a minted id, and the swallowed error is
//! logged instead of dropped — it was the silence that hid this.
//!
//! The `create_bounty_typed` INSERT also writes the literal `'pending'` into
//! `escrow_transaction` and `''` into `claimant` on both backends, so
//! `a_standard_bounty_starts_with_a_pending_escrow` pins that rather than
//! letting it be mistaken for a caller-supplied value.

use std::path::PathBuf;

use lorehaven_db::bounties::{
    contribute_to_bounty, create_bounty_typed, fetch_bounty, list_flexible_bounties, Bounty,
    FlexibleBountyError,
};
use test_support::TestDb;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-bounties-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

struct Harness {
    tdb: TestDb,
    #[allow(dead_code)]
    dir: PathBuf,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        Self { tdb, dir }
    }

    fn db(&self) -> &lorehaven_db::Database {
        self.tdb.db()
    }

    async fn exec(&self, query: &str) {
        let q = self.tdb.sql(query);
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => {
                sqlx::query(&q)
                    .execute(self.db().sqlite_pool().expect("sqlite"))
                    .await
                    .expect("exec");
            }
            lorehaven_db::Backend::Postgres => {
                sqlx::query(&q)
                    .execute(self.db().postgres_pool().expect("pg"))
                    .await
                    .expect("exec");
            }
        }
    }

    /// Read a single text column, unwrapping the outer `Option` the scalar query
    /// returns. `bounties_contributions.amount` is `INTEGER`, which is `INT4` on
    /// PostgreSQL, so integer reads go through [`Self::int`].
    async fn text(&self, query: &str) -> Option<String> {
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query_scalar::<_, String>(&self.tdb.sql(query))
                .fetch_optional(self.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("text"),
            lorehaven_db::Backend::Postgres => {
                sqlx::query_scalar::<_, String>(&self.tdb.sql(query))
                    .fetch_optional(self.db().postgres_pool().expect("pg"))
                    .await
                    .expect("text")
            }
        }
    }

    /// A count of DISTINCT values as `i64`. Used to check that primary keys are
    /// actually distinct, which is the property the ledger fix restored.
    async fn distinct_count(&self, query: &str) -> i64 {
        let q = self.tdb.sql(query);
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query_scalar::<_, i64>(&q)
                .fetch_one(self.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("count"),
            lorehaven_db::Backend::Postgres => {
                // COUNT is BIGINT on both, so it decodes as i64 without a cast.
                sqlx::query_scalar::<_, i64>(&q)
                    .fetch_one(self.db().postgres_pool().expect("pg"))
                    .await
                    .expect("count")
            }
        }
    }

    /// A bounty of any type, inserted through the repository.
    async fn bounty(&self, kind: &str, state: &str, amount: i64, funded: i64) -> Bounty {
        let bounty = Bounty {
            id: format!("bt-{}", uuid::Uuid::new_v4()),
            bounty_type: kind.to_string(),
            job_kind: "translate".to_string(),
            terms: r#"{"lang":"es"}"#.to_string(),
            amount,
            funded_amount: funded,
            state: state.to_string(),
            created_by: "someone".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            activated_at: None,
        };
        create_bounty_typed(self.db(), &bounty)
            .await
            .expect("create_bounty_typed");
        bounty
    }

    /// A crowdfunded bounty in `funding`, which is the only state
    /// `contribute_to_bounty` accepts.
    async fn crowdfunded(&self, amount: i64) -> Bounty {
        self.bounty("crowdfunded", "funding", amount, 0).await
    }

    /// Contribution rows for a bounty, oldest first.
    async fn contributions(&self, bounty_id: &str) -> Vec<(String, i64)> {
        let q = self.tdb.sql(&format!(
            "SELECT contributor, amount FROM bounty_contributions WHERE bounty_id = '{bounty_id}' \
             ORDER BY contributed_at, contributor"
        ));
        let wide = match self.db().backend() {
            lorehaven_db::Backend::Postgres => q.replace(
                "SELECT contributor, amount",
                "SELECT contributor, CAST(amount AS BIGINT)",
            ),
            lorehaven_db::Backend::Sqlite => q,
        };
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query_as::<_, (String, i64)>(&wide)
                .fetch_all(self.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("contributions"),
            lorehaven_db::Backend::Postgres => sqlx::query_as::<_, (String, i64)>(&wide)
                .fetch_all(self.db().postgres_pool().expect("pg"))
                .await
                .expect("contributions"),
        }
    }
}

// ---------------------------------------------------------------------------
// create_bounty_typed
// ---------------------------------------------------------------------------

/// Every field round-trips through a create and a fetch.
#[tokio::test]
async fn a_bounty_round_trips_every_field() {
    let h = Harness::new("bt-roundtrip").await;
    let mut b = h.bounty("crowdfunded", "funding", 5_000, 1_250).await;
    b.activated_at = Some("2026-02-02T00:00:00Z".to_string());
    // Re-create with the activation stamp set, to carry it through the insert.
    h.exec(&format!("DELETE FROM bounties WHERE id = '{}'", b.id))
        .await;
    create_bounty_typed(h.db(), &b)
        .await
        .expect("create_bounty_typed");

    let got = fetch_bounty(h.db(), &b.id)
        .await
        .expect("fetch_bounty")
        .expect("the bounty");
    assert_eq!(got.id, b.id);
    assert_eq!(got.bounty_type, "crowdfunded");
    assert_eq!(got.job_kind, "translate");
    assert_eq!(got.terms, b.terms);
    assert_eq!(got.amount, 5_000);
    assert_eq!(got.funded_amount, 1_250);
    assert_eq!(got.state, "funding");
    assert_eq!(got.created_by, "someone");
    assert_eq!(got.created_at, "2026-01-01T00:00:00Z");
    assert_eq!(got.activated_at.as_deref(), Some("2026-02-02T00:00:00Z"));
}

/// The three types are stored in one table and read back distinctly, which is
/// the whole point of the `type` column added in migration 0057.
#[tokio::test]
async fn all_three_bounty_types_are_stored_and_read_back() {
    let h = Harness::new("bt-types").await;
    for kind in ["standard", "crowdfunded", "reverse"] {
        let b = h.bounty(kind, "open", 100, 100).await;
        let got = fetch_bounty(h.db(), &b.id)
            .await
            .expect("fetch")
            .expect("the bounty");
        assert_eq!(got.bounty_type, kind, "{kind} keeps its own type");
    }
}

/// **The defect this suite found.** `record_contribution` omitted the `id`
/// column, so no contribution row had a usable primary key. On PostgreSQL the
/// insert was rejected outright and the ledger was empty; on SQLite it succeeded
/// with `id = NULL`. Either way the caller discarded the error, so nothing
/// reported it.
#[tokio::test]
async fn a_contribution_writes_an_audit_row_with_an_id() {
    let h = Harness::new("bt-ledger").await;
    let b = h.crowdfunded(1_000).await;

    contribute_to_bounty(h.db(), &b.id, "alice", 100, 1.0)
        .await
        .expect("contribute");

    let rows = h.contributions(&b.id).await;
    assert_eq!(
        rows,
        vec![("alice".to_string(), 100)],
        "the ledger has a row"
    );
    assert!(
        h.text(&format!(
            "SELECT id FROM bounty_contributions WHERE bounty_id = '{}'",
            b.id
        ))
        .await
        .is_some(),
        "and that row has a primary key"
    );
}

/// **The test that catches the bug on SQLite as well as PostgreSQL.** The
/// ledger test above only fails on PostgreSQL, because SQLite accepts an insert
/// with a missing primary key and stores NULL. This one asserts the property
/// that is actually broken on SQLite: a non-NULL, distinct primary key per
/// row.
///
/// Without the fix this read `None` on SQLite and failed. With it, it is `2`.
#[tokio::test]
async fn every_contribution_row_has_a_non_null_primary_key() {
    let h = Harness::new("bt-ledger-nonnull").await;
    let b = h.crowdfunded(1_000).await;
    contribute_to_bounty(h.db(), &b.id, "alice", 100, 1.0)
        .await
        .expect("first");
    contribute_to_bounty(h.db(), &b.id, "alice", 150, 1.0)
        .await
        .expect("second");

    // `COUNT(col)` ignores NULLs, so this counts rows that actually have a
    // primary key. No dialect branch needed -- the SQL is the same, and it is
    // precisely the difference in behaviour that made the bug invisible.
    assert_eq!(
        h.distinct_count(&format!(
            "SELECT COUNT(id) FROM bounty_contributions WHERE bounty_id = '{}'",
            b.id
        ))
        .await,
        2,
        "both rows have a real primary key, not NULL"
    );

    assert_eq!(
        h.distinct_count(&format!(
            "SELECT COUNT(DISTINCT id) FROM bounty_contributions WHERE bounty_id = '{}'",
            b.id
        ))
        .await,
        2,
        "and the two keys differ"
    );
}

/// Each contribution is its own row, so the ledger accumulates rather than
/// overwriting.
#[tokio::test]
async fn each_contribution_is_a_separate_row() {
    let h = Harness::new("bt-ledger-many").await;
    let b = h.crowdfunded(1_000).await;

    for who in ["alice", "bob", "carol"] {
        contribute_to_bounty(h.db(), &b.id, who, 100, 1.0)
            .await
            .expect("contribute");
    }

    assert_eq!(
        h.contributions(&b.id).await,
        vec![
            ("alice".to_string(), 100),
            ("bob".to_string(), 100),
            ("carol".to_string(), 100)
        ]
    );
}

/// Two people contributing the same amount produce distinct rows, since `id` is
/// now minted per call rather than derived from the contributor.
#[tokio::test]
async fn two_contributions_by_one_contributor_are_distinct_rows() {
    let h = Harness::new("bt-ledger-same").await;
    let b = h.crowdfunded(1_000).await;

    contribute_to_bounty(h.db(), &b.id, "alice", 100, 1.0)
        .await
        .expect("first");
    contribute_to_bounty(h.db(), &b.id, "alice", 150, 1.0)
        .await
        .expect("second");

    let rows = h.contributions(&b.id).await;
    assert_eq!(
        rows,
        vec![("alice".to_string(), 100), ("alice".to_string(), 150)],
        "the second is a new row, not an update"
    );
    assert_eq!(
        h.distinct_count(&format!(
            "SELECT COUNT(DISTINCT id) FROM bounty_contributions WHERE bounty_id = '{}'",
            b.id
        ))
        .await,
        2,
        "two distinct primary keys"
    );
}

/// The escrow and claimant columns are written as literals by the INSERT, not
/// supplied by the caller — `Bounty` has no such fields.
#[tokio::test]
async fn a_standard_bounty_starts_with_a_pending_escrow() {
    let h = Harness::new("bt-escrow").await;
    let b = h.bounty("standard", "open", 500, 500).await;

    assert_eq!(
        h.text(&format!(
            "SELECT escrow_transaction FROM bounties WHERE id = '{}'",
            b.id
        ))
        .await
        .as_deref(),
        Some("pending"),
        "hard-coded by the INSERT, not caller-supplied"
    );
    assert_eq!(
        h.text(&format!(
            "SELECT claimant FROM bounties WHERE id = '{}'",
            b.id
        ))
        .await
        .as_deref(),
        Some("")
    );
}

/// `account` mirrors `created_by` — the schema's 0017 columns have no account,
/// and 0034 added it. Pinned because the INSERT binds the same value twice.
#[tokio::test]
async fn the_account_column_mirrors_created_by() {
    let h = Harness::new("bt-account").await;
    let b = h.bounty("standard", "open", 500, 500).await;
    let who = h
        .text(&format!(
            "SELECT account FROM bounties WHERE id = '{}'",
            b.id
        ))
        .await
        .expect("account");
    assert_eq!(who, b.created_by);
}

/// Creating a bounty with a duplicate id is refused by the primary key, so a
/// caller cannot silently shadow an existing bounty.
#[tokio::test]
async fn a_duplicate_bounty_id_is_refused() {
    let h = Harness::new("bt-dup").await;
    let b = h.bounty("standard", "open", 500, 500).await;
    let again = create_bounty_typed(h.db(), &b).await;
    assert!(again.is_err(), "the primary key holds");
}

// ---------------------------------------------------------------------------
// fetch_bounty
// ---------------------------------------------------------------------------

/// An unknown id is `None`, not an error.
#[tokio::test]
async fn fetching_an_unknown_bounty_is_none() {
    let h = Harness::new("bt-fetch-unknown").await;
    assert!(fetch_bounty(h.db(), "no-such-bounty")
        .await
        .expect("fetch_bounty")
        .is_none());
}

/// **The `amount::bigint` cast is load-bearing.** `amount` is `INTEGER` and so
/// `INT4` on PostgreSQL; without the cast the shared `BountyRow` tuple could not
/// decode as `i64` and every fetch would fail on the production backend.
#[tokio::test]
async fn a_single_bounty_decodes_on_both_backends() {
    let h = Harness::new("bt-fetch-decode").await;
    let b = h.bounty("crowdfunded", "funding", 1_000_000, 12_345).await;
    let got = fetch_bounty(h.db(), &b.id)
        .await
        .expect("fetch")
        .expect("the bounty");
    assert_eq!(got.amount, 1_000_000, "INT4 widened, not truncated");
    assert_eq!(got.funded_amount, 12_345);
}

/// A bounty that has never been activated has a NULL `activated_at`, which is
/// the one optional field in the row.
#[tokio::test]
async fn an_unactivated_bounty_has_no_activation_stamp() {
    let h = Harness::new("bt-fetch-null").await;
    let b = h.crowdfunded(1_000).await;
    assert_eq!(
        fetch_bounty(h.db(), &b.id)
            .await
            .expect("fetch")
            .expect("bounty")
            .activated_at,
        None
    );
}

// ---------------------------------------------------------------------------
// contribute_to_bounty
// ---------------------------------------------------------------------------

/// A contribution below the threshold accumulates without activating.
#[tokio::test]
async fn a_partial_contribution_accumulates_without_activating() {
    let h = Harness::new("bt-partial").await;
    let b = h.crowdfunded(1_000).await;

    let (funded, activated) = contribute_to_bounty(h.db(), &b.id, "alice", 400, 1.0)
        .await
        .expect("contribute");

    assert_eq!(funded, 400);
    assert!(!activated, "400 of 1000 is not enough");
    let got = fetch_bounty(h.db(), &b.id)
        .await
        .expect("fetch")
        .expect("bounty");
    assert_eq!(got.funded_amount, 400);
    assert_eq!(got.state, "funding");
    assert_eq!(got.activated_at, None, "no stamp until it activates");
}

/// Contributions accumulate across several callers.
#[tokio::test]
async fn contributions_accumulate_across_callers() {
    let h = Harness::new("bt-accumulate").await;
    let b = h.crowdfunded(1_000).await;

    let mut expected = 0;
    for who in ["alice", "bob", "carol"] {
        expected += 100;
        let (funded, activated) = contribute_to_bounty(h.db(), &b.id, who, 100, 1.0)
            .await
            .expect("contribute");
        assert!(!activated, "300 of 1000 is still short");
        assert_eq!(funded, expected, "{who} sees the running total");
    }
}

/// Reaching the threshold activates: state becomes `open` and the stamp is set.
#[tokio::test]
async fn reaching_the_threshold_activates_the_bounty() {
    let h = Harness::new("bt-activate").await;
    let b = h.crowdfunded(1_000).await;

    let (funded, activated) = contribute_to_bounty(h.db(), &b.id, "alice", 1_000, 1.0)
        .await
        .expect("contribute");

    assert_eq!(funded, 1_000);
    assert!(activated, "the threshold is met exactly");
    let got = fetch_bounty(h.db(), &b.id)
        .await
        .expect("fetch")
        .expect("bounty");
    assert_eq!(got.state, "open");
    assert!(got.activated_at.is_some(), "and it is stamped");
}

/// A threshold below 1.0 activates early, which is the point of a fractional
/// activation threshold.
#[tokio::test]
async fn a_fractional_threshold_activates_early() {
    let h = Harness::new("bt-fraction").await;
    let b = h.crowdfunded(1_000).await;

    let (funded, activated) = contribute_to_bounty(h.db(), &b.id, "alice", 500, 0.5)
        .await
        .expect("contribute");

    assert_eq!(funded, 500);
    assert!(activated, "500 is half of 1000");
}

/// Crossing the threshold in several steps activates on the step that crosses
/// it, and the earlier steps are not retroactively activated.
#[tokio::test]
async fn activation_happens_on_the_step_that_crosses_the_threshold() {
    let h = Harness::new("bt-cross").await;
    let b = h.crowdfunded(1_000).await;

    assert!(
        !contribute_to_bounty(h.db(), &b.id, "a", 300, 1.0)
            .await
            .expect("first")
            .1
    );
    assert!(
        !contribute_to_bounty(h.db(), &b.id, "b", 300, 1.0)
            .await
            .expect("second")
            .1
    );
    let (funded, activated) = contribute_to_bounty(h.db(), &b.id, "c", 400, 1.0)
        .await
        .expect("third");

    assert_eq!(funded, 1_000);
    assert!(activated);
}

/// Once activated the bounty is no longer `funding`, so a further contribution
/// is refused — the state guard is what stops overfunding.
#[tokio::test]
async fn a_contribution_after_activation_is_refused() {
    let h = Harness::new("bt-after-activate").await;
    let b = h.crowdfunded(1_000).await;
    contribute_to_bounty(h.db(), &b.id, "alice", 1_000, 1.0)
        .await
        .expect("activate");

    let err = contribute_to_bounty(h.db(), &b.id, "bob", 100, 1.0)
        .await
        .expect_err("refused");
    assert!(matches!(err, FlexibleBountyError::NotFunding));
}

/// A standard bounty cannot be crowdfunded.
#[tokio::test]
async fn contributing_to_a_standard_bounty_is_refused() {
    let h = Harness::new("bt-not-crowdfunded").await;
    let b = h.bounty("standard", "open", 1_000, 1_000).await;
    let err = contribute_to_bounty(h.db(), &b.id, "alice", 100, 1.0)
        .await
        .expect_err("refused");
    assert!(matches!(err, FlexibleBountyError::NotCrowdfunded));
}

/// A reverse bounty is not crowdfunded either, even though it is `open`.
#[tokio::test]
async fn contributing_to_a_reverse_bounty_is_refused() {
    let h = Harness::new("bt-not-reverse").await;
    let b = h.bounty("reverse", "open", 1_000, 1_000).await;
    let err = contribute_to_bounty(h.db(), &b.id, "alice", 100, 1.0)
        .await
        .expect_err("refused");
    assert!(matches!(err, FlexibleBountyError::NotCrowdfunded));
}

/// An unknown bounty is `NotFound`.
#[tokio::test]
async fn contributing_to_an_unknown_bounty_is_not_found() {
    let h = Harness::new("bt-contrib-unknown").await;
    let err = contribute_to_bounty(h.db(), "no-such-bounty", "alice", 100, 1.0)
        .await
        .expect_err("refused");
    assert!(matches!(err, FlexibleBountyError::NotFound));
}

/// Zero, negative and refused amounts are all rejected before any read.
#[tokio::test]
async fn a_non_positive_amount_is_refused() {
    let h = Harness::new("bt-amount").await;
    let b = h.crowdfunded(1_000).await;
    for bad in [0, -1, i64::MIN] {
        let err = contribute_to_bounty(h.db(), &b.id, "alice", bad, 1.0)
            .await
            .expect_err("refused");
        assert!(
            matches!(err, FlexibleBountyError::InvalidAmount),
            "{bad} is refused"
        );
    }
    assert!(
        h.contributions(&b.id).await.is_empty(),
        "and no ledger row is written"
    );
}

/// A refused contribution leaves the funded amount and the ledger untouched.
#[tokio::test]
async fn a_refused_contribution_changes_nothing() {
    let h = Harness::new("bt-refused-noop").await;
    let b = h.crowdfunded(1_000).await;
    contribute_to_bounty(h.db(), &b.id, "alice", 100, 1.0)
        .await
        .expect("first");

    let _ = contribute_to_bounty(h.db(), &b.id, "bob", 0, 1.0).await;

    let got = fetch_bounty(h.db(), &b.id)
        .await
        .expect("fetch")
        .expect("bounty");
    assert_eq!(got.funded_amount, 100, "unchanged");
    assert_eq!(h.contributions(&b.id).await.len(), 1, "one row, not two");
}

/// A threshold of 0 means the bounty never activates on its own, because the
/// `required > 0` guard is what stops a zero-amount bounty from activating
/// before any credit is committed.
#[tokio::test]
async fn a_zero_threshold_does_not_activate_a_bounty() {
    let h = Harness::new("bt-zero-threshold").await;
    let b = h.crowdfunded(1_000).await;
    let (_, activated) = contribute_to_bounty(h.db(), &b.id, "alice", 100, 0.0)
        .await
        .expect("contribute");
    assert!(!activated, "required > 0 is false, so no activation");
}

// ---------------------------------------------------------------------------
// list_flexible_bounties
// ---------------------------------------------------------------------------

/// The list is JSON, and carries every field the API renders.
#[tokio::test]
async fn the_list_carries_every_field() {
    let h = Harness::new("bt-list").await;
    let b = h.bounty("crowdfunded", "funding", 1_000, 250).await;

    let listed = list_flexible_bounties(h.db()).await.expect("list");
    assert_eq!(listed.len(), 1);
    let row = &listed[0];
    assert_eq!(row["id"], serde_json::json!(b.id));
    assert_eq!(row["type"], serde_json::json!("crowdfunded"));
    assert_eq!(row["job_kind"], serde_json::json!("translate"));
    assert_eq!(row["amount"], serde_json::json!(1_000));
    assert_eq!(row["funded_amount"], serde_json::json!(250));
    assert_eq!(row["state"], serde_json::json!("funding"));
    assert_eq!(row["created_by"], serde_json::json!("someone"));
    assert_eq!(row["created_at"], serde_json::json!("2026-01-01T00:00:00Z"));
    assert_eq!(row["activated_at"], serde_json::Value::Null);
}

/// **The `CAST(... AS BIGINT)` in the list arm is load-bearing**, the same reason
/// as in `fetch_bounty`.
#[tokio::test]
async fn the_list_widens_its_integers() {
    let h = Harness::new("bt-list-widen").await;
    h.bounty("standard", "open", 1_000_000, 999_999).await;
    let listed = list_flexible_bounties(h.db()).await.expect("list");
    assert_eq!(listed[0]["amount"], serde_json::json!(1_000_000));
    assert_eq!(listed[0]["funded_amount"], serde_json::json!(999_999));
}

/// Only `open` and `funding` bounties are listed — a fulfilled, cancelled or
/// expired board entry is not shown to readers.
#[tokio::test]
async fn only_open_and_funding_bounties_are_listed() {
    let h = Harness::new("bt-list-filter").await;
    h.bounty("standard", "open", 100, 100).await;
    h.bounty("crowdfunded", "funding", 100, 0).await;
    h.bounty("standard", "fulfilled", 100, 100).await;
    h.bounty("standard", "cancelled", 100, 100).await;
    h.bounty("standard", "expired", 100, 100).await;

    let listed = list_flexible_bounties(h.db()).await.expect("list");
    assert_eq!(listed.len(), 2, "two board entries");
    let states: Vec<&str> = listed
        .iter()
        .map(|r| r["state"].as_str().unwrap())
        .collect();
    assert!(states.contains(&"open"));
    assert!(states.contains(&"funding"));
}

/// Newest first, which is what the board renders.
#[tokio::test]
async fn the_list_is_newest_first() {
    let h = Harness::new("bt-list-order").await;
    for day in ["01", "02", "03"] {
        let mut b = h.bounty("standard", "open", 100, 100).await;
        b.created_at = format!("2026-01-{day}T00:00:00Z");
        h.exec(&format!("DELETE FROM bounties WHERE id = '{}'", b.id))
            .await;
        create_bounty_typed(h.db(), &b)
            .await
            .expect("create_bounty_typed");
    }

    let listed = list_flexible_bounties(h.db()).await.expect("list");
    let days: Vec<String> = listed
        .iter()
        .map(|r| r["created_at"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        days,
        vec![
            "2026-01-03T00:00:00Z",
            "2026-01-02T00:00:00Z",
            "2026-01-01T00:00:00Z"
        ]
    );
}

/// The list is capped at 50, so a busy board does not render unbounded.
#[tokio::test]
async fn the_list_is_capped_at_fifty() {
    let h = Harness::new("bt-list-cap").await;
    for i in 0..55 {
        let mut b = h.bounty("standard", "open", 100, 100).await;
        b.created_at = format!("2026-01-01T00:00:{:02}Z", i);
        h.exec(&format!("DELETE FROM bounties WHERE id = '{}'", b.id))
            .await;
        create_bounty_typed(h.db(), &b)
            .await
            .expect("create_bounty_typed");
    }
    assert_eq!(
        list_flexible_bounties(h.db()).await.expect("list").len(),
        50
    );
}

/// An empty board lists empty rather than erroring.
#[tokio::test]
async fn an_empty_board_lists_nothing() {
    let h = Harness::new("bt-list-empty").await;
    assert!(list_flexible_bounties(h.db())
        .await
        .expect("list")
        .is_empty());
}

/// Deleting a bounty cascades to its contributions, so the ledger cannot outlive
/// the bounty it describes.
#[tokio::test]
async fn deleting_a_bounty_cascades_to_its_contributions() {
    let h = Harness::new("bt-cascade").await;
    let b = h.crowdfunded(1_000).await;
    contribute_to_bounty(h.db(), &b.id, "alice", 100, 1.0)
        .await
        .expect("contribute");
    assert_eq!(h.contributions(&b.id).await.len(), 1);

    h.exec(&format!("DELETE FROM bounties WHERE id = '{}'", b.id))
        .await;

    assert!(
        h.contributions(&b.id).await.is_empty(),
        "ON DELETE CASCADE clears the ledger"
    );
}
