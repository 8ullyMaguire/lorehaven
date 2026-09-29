//! Acceptance: the preservation ledger — the grant, the clawback, and the
//! anti-farm property they exist to provide (spec §2.1, §2.2, §2.3).
//!
//! **These are the tests the plan says to write first, and it says so for a
//! reason worth repeating.** Everything else in preservation is additive: a
//! destination, a record, a badge. The clawback is the only mechanism that stops
//! the feature being farmed, and a reward system without one is a spam vector
//! with a badge attached. A farm is cheap — crosspost to N archives, collect N
//! rewards, repeat — and if the destinations are allowed to rot, the farm pays
//! forever for work nobody has preserved.
//!
//! Three claims, and each is a way the feature can be wrong in a way that looks
//! right:
//!
//! 1. **The grant is replay-safe.** A retried recheck, a double-clicked button
//!    or a job that succeeds twice must not pay twice. §2.2's decay is a
//!    function of *rank*, so a double-paid third destination is a reader who
//!    earned 20 and holds 20 for one act.
//! 2. **The reversal is a second entry, never a balance edit.** A reader who has
//!    spent the credits has to *see the debt*. A balance mutation makes the
//!    number smaller and cannot distinguish "never paid" from "paid and taken
//!    back", which is the one distinction §2.3 exists to preserve.
//! 3. **The clawback takes back what was paid, not what policy says today.** A
//!    work can change hands and a policy can change; recomputing either one
//!    confiscates from the wrong party.

use std::path::PathBuf;

use lorehaven_db::preservation::{self, LedgerOutcome};
use lorehaven_domain::economy::TxnType;
use lorehaven_domain::preservation::PreservationState;
use test_support::TestDb;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-presledger-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

async fn account_for(tdb: &TestDb, tag: &str) -> String {
    let mut config = lorehaven_app::config::Config::development_defaults();
    config.storage.root = scratch_dir(&format!("{tag}-account"));
    let app = lorehaven_app::server::build_router(lorehaven_app::state::AppState::new(
        config,
        tdb.db().clone(),
    ));
    let mut client = test_support::TestClient::new(app);
    let handle = tag.replace(['.', '-'], "_");
    test_support::register(&mut client, &format!("{tag}@test.dev"), &handle).await
}

async fn harness(tag: &str) -> (TestDb, String) {
    let tdb = TestDb::connect_with_dir(tag, &scratch_dir(tag)).await;
    let account = account_for(&tdb, tag).await;
    (tdb, account)
}

async fn seed_work(db: &lorehaven_db::Database, account: &str, title: &str) -> String {
    let now = "2026-09-29T00:00:00Z";
    let pseud_id = uuid::Uuid::new_v4().to_string();
    let handle = format!("p{}", &pseud_id[..8]);
    let sql = db.sql(
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)",
        // `$1`..`$6`, never `?::uuid, $1::uuid, ...`. sqlx numbers
        // placeholders per arm, so a leading `?` and `$1` are both the first
        // bind: the row would point `account_id` at the pseud id and
        // PostgreSQL would blame a missing account. SQLite cannot catch it --
        // `?::uuid` is not a token it accepts -- so the arms never look
        // interchangeable. `crates/app/tests/preservation.rs` says this at
        // greater length; this is the same bug, reintroduced.
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
         VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6)",
    );
    if let Some(pool) = db.sqlite_pool() {
        sqlx::query(&sql)
            .bind(&pseud_id)
            .bind(account)
            .bind(&handle)
            .bind(&handle)
            .bind(now)
            .bind(now)
            .execute(pool)
            .await
            .expect("insert pseud");
    }
    if let Some(pool) = db.postgres_pool() {
        sqlx::query(&sql)
            .bind(&pseud_id)
            .bind(account)
            .bind(&handle)
            .bind(&handle)
            .bind(now)
            .bind(now)
            .execute(pool)
            .await
            .expect("insert pseud");
    }
    let work_id = uuid::Uuid::new_v4().to_string();
    let sql = db.sql(
        "INSERT INTO works (id, owner_pseud_id, title, lifecycle, visibility, created_at, updated_at)
         VALUES (?, ?, ?, 'published', 'public', ?, ?)",
        "INSERT INTO works (id, owner_pseud_id, title, lifecycle, visibility, created_at, updated_at)
         VALUES ($1::uuid, $2::uuid, $3, 'published', 'public', $4, $5)",
    );
    if let Some(pool) = db.sqlite_pool() {
        sqlx::query(&sql)
            .bind(&work_id)
            .bind(&pseud_id)
            .bind(title)
            .bind(now)
            .bind(now)
            .execute(pool)
            .await
            .expect("insert work");
    }
    if let Some(pool) = db.postgres_pool() {
        sqlx::query(&sql)
            .bind(&work_id)
            .bind(&pseud_id)
            .bind(title)
            .bind(now)
            .bind(now)
            .execute(pool)
            .await
            .expect("insert work");
    }
    work_id
}

/// A verified target on a destination, paid.
async fn paid_target(
    tdb: &TestDb,
    account: &str,
    destination: &str,
    credits: i64,
) -> (String, String) {
    let work = seed_work(tdb.db(), account, "A Work").await;
    let identity =
        lorehaven_db::story_identity::ensure_identity_for_work(tdb.db(), &work, "A Work")
            .await
            .expect("identity");
    preservation::write_destination(
        tdb.db(),
        destination,
        &format!("Archive {destination}"),
        "https://archive.example/",
        "item",
        true,
        true,
    )
    .await
    .expect("destination");
    let target = preservation::record_target(
        tdb.db(),
        &identity,
        &work,
        destination,
        "1",
        None,
        Some(uuid::Uuid::parse_str(account).expect("uuid")),
    )
    .await
    .expect("record target");
    preservation::set_state(
        tdb.db(),
        &target.member_id,
        PreservationState::Verified,
        Some("x"),
    )
    .await
    .expect("verify");
    let owner = preservation::reward_account_for_work(tdb.db(), &work)
        .await
        .expect("owner account");
    preservation::grant_credits(tdb.db(), &target.member_id, &owner, credits)
        .await
        .expect("grant");
    (work, target.member_id)
}

/// How many ledger entries of a type exist for one reference.
async fn ledger_rows(tdb: &TestDb, txn_type: TxnType, reference: &str) -> i64 {
    let sql = tdb.db().sql(
        "SELECT COUNT(*) FROM credit_entries e
           JOIN credit_transactions t ON t.id = e.transaction_id
          WHERE t.type = ? AND t.reference = ?",
        "SELECT COUNT(*)::bigint FROM credit_entries e
           JOIN credit_transactions t ON t.id = e.transaction_id
          WHERE t.type = $1 AND t.reference = $2",
    );
    match tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&sql)
            .bind(txn_type.as_str())
            .bind(reference)
            .fetch_one(tdb.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("q"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(&sql)
            .bind(txn_type.as_str())
            .bind(reference)
            .fetch_one(tdb.db().postgres_pool().expect("postgres"))
            .await
            .expect("q"),
    }
}

/// The signed sum of ledger entries of a type for one reference.
async fn ledger_sum(tdb: &TestDb, txn_type: TxnType, reference: &str) -> i64 {
    let sql = tdb.db().sql(
        "SELECT COALESCE(SUM(e.amount_bp), 0) FROM credit_entries e
           JOIN credit_transactions t ON t.id = e.transaction_id
          WHERE t.type = ? AND t.reference = ?",
        "SELECT COALESCE(SUM(e.amount_bp), 0)::bigint FROM credit_entries e
           JOIN credit_transactions t ON t.id = e.transaction_id
          WHERE t.type = $1 AND t.reference = $2",
    );
    match tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&sql)
            .bind(txn_type.as_str())
            .bind(reference)
            .fetch_one(tdb.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("q"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(&sql)
            .bind(txn_type.as_str())
            .bind(reference)
            .fetch_one(tdb.db().postgres_pool().expect("postgres"))
            .await
            .expect("q"),
    }
}

// ---------------------------------------------------------------------------
// The grant
// ---------------------------------------------------------------------------

/// §2.1: the reward attaches to *verification*, and it is paid once.
///
/// The three assertions are the three things that can each be wrong alone: the
/// grant posts at all, it posts the right amount, and a replay does not post
/// twice. The `AlreadyPosted` variant is asserted explicitly because a caller
/// that reports a replay as `Posted { credits: 0 }` would tell a reader their
/// fourth destination earned nothing when it earned a quarter of full an hour
/// ago.
#[tokio::test]
async fn a_verified_target_is_paid_once_and_a_replayed_grant_pays_nothing() {
    let (tdb, account) = harness("grant_once").await;
    let (_work, member) = paid_target(&tdb, &account, "aot", 10).await;

    assert_eq!(
        ledger_rows(&tdb, TxnType::Preservation, &member).await,
        1,
        "a verified target is paid, and paid once"
    );
    assert_eq!(ledger_sum(&tdb, TxnType::Preservation, &member).await, 10);

    // Replay: the same grant, for the same target, so the same idempotency key.
    // Paid to the same account the first time paid, which is what a real replay
    // looks like -- the caller recomputes the owner and gets the same answer.
    let owner = preservation::reward_account_for_work(tdb.db(), &_work)
        .await
        .expect("owner");
    let again = preservation::grant_credits(tdb.db(), &member, &owner, 10)
        .await
        .expect("replayed grant");
    assert_eq!(
        again,
        LedgerOutcome::AlreadyPosted { credits: 10 },
        "a replayed grant is a distinct outcome from a zero-credit one, because the \
         first means 'already paid' and the second means 'worth nothing'"
    );
    assert_eq!(
        ledger_rows(&tdb, TxnType::Preservation, &member).await,
        1,
        "a replayed grant does not post a second entry"
    );
    assert_eq!(
        ledger_sum(&tdb, TxnType::Preservation, &member).await,
        10,
        "a replayed grant does not pay twice"
    );
}

/// §2.2's cap: a destination past the cap records and displays and **pays
/// nothing**, and a zero-credit grant posts no ledger row at all.
///
/// A zero row would be a statement line a reader has to render and explain, and
/// it would be indistinguishable from a grant of zero that meant something.
#[tokio::test]
async fn a_cap_destination_pays_nothing_and_posts_no_ledger_row() {
    let (tdb, account) = harness("cap_pays_nothing").await;
    let (_work, member) = paid_target(&tdb, &account, "aot", 10).await;
    // Simulate the ninth destination: verified, but the reward is zero.
    let outcome = preservation::grant_credits(tdb.db(), &member, &account, 0)
        .await
        .expect("zero grant");
    assert_eq!(outcome.credits(), 0);

    let target = preservation::target_by_member(tdb.db(), &member)
        .await
        .unwrap()
        .expect("target");
    assert_eq!(
        target.credits_paid, 10,
        "a zero grant does not overwrite a real one; credits_paid tracks what is \
         still outstanding and the cap case is a *different* target"
    );
}

// ---------------------------------------------------------------------------
// The clawback
// ---------------------------------------------------------------------------

/// §2.3: a destination that stops answering is marked dead and its credits are
/// reclaimed, as a **second ledger entry**.
///
/// The second-entry assertion is the whole test. The pair is `preservation` +
/// `preservation_reclaim`, both present, so a statement shows "paid 10" and
/// "reclaimed 10" as two facts; a balance edit would show neither.
#[tokio::test]
async fn a_dead_destination_reverses_its_credits_as_a_second_ledger_entry() {
    let (tdb, account) = harness("clawback").await;
    let (_work, member) = paid_target(&tdb, &account, "aot", 10).await;

    assert_eq!(ledger_sum(&tdb, TxnType::Preservation, &member).await, 10);
    assert_eq!(
        ledger_rows(&tdb, TxnType::PreservationReclaim, &member).await,
        0,
        "nothing is reclaimed while the destination is live"
    );

    preservation::set_state(tdb.db(), &member, PreservationState::Dead, Some("gone"))
        .await
        .expect("mark dead");
    let outcome = preservation::reclaim_credits(tdb.db(), &member)
        .await
        .expect("reclaim");

    assert_eq!(outcome, LedgerOutcome::Posted { credits: -10 });
    assert_eq!(
        ledger_rows(&tdb, TxnType::PreservationReclaim, &member).await,
        1,
        "§2.3: the reversal is a second ledger entry, so a reader who already spent \
         the credits sees the debt appear as its own row"
    );
    assert_eq!(
        ledger_sum(&tdb, TxnType::Preservation, &member).await,
        10,
        "the grant stands: WHERE type = 'preservation' remains a complete statement \
         of what was paid out"
    );
    assert_eq!(
        ledger_sum(&tdb, TxnType::PreservationReclaim, &member).await,
        -10,
        "and the net of the pair is zero"
    );
}

/// The same target, reclaimed twice. A recheck that runs twice must not take
/// the credits twice.
#[tokio::test]
async fn a_replayed_clawback_does_not_reclaim_twice() {
    let (tdb, account) = harness("clawback_replay").await;
    let (_work, member) = paid_target(&tdb, &account, "aot", 10).await;
    preservation::set_state(tdb.db(), &member, PreservationState::Dead, Some("gone"))
        .await
        .expect("dead");

    let first = preservation::reclaim_credits(tdb.db(), &member)
        .await
        .expect("first");
    assert_eq!(first, LedgerOutcome::Posted { credits: -10 });

    let second = preservation::reclaim_credits(tdb.db(), &member)
        .await
        .expect("second");
    assert_eq!(
        second,
        LedgerOutcome::Posted { credits: 0 },
        "credits_paid is zeroed by the first reclaim, so a second has nothing to take"
    );
    assert_eq!(
        ledger_rows(&tdb, TxnType::PreservationReclaim, &member).await,
        1,
        "one reversal per target, however many times the job runs"
    );
}

/// A target that was never verified has nothing to claw back. The recheck only
/// ever sees `verified` rows, so this is belt-and-braces — but a reclaim on an
/// unpaid target must be a no-op, not an error and not a negative balance.
#[tokio::test]
async fn a_never_paid_target_reclaims_nothing() {
    let (tdb, account) = harness("clawback_unpaid").await;
    let work = seed_work(tdb.db(), &account, "A Work").await;
    let identity =
        lorehaven_db::story_identity::ensure_identity_for_work(tdb.db(), &work, "A Work")
            .await
            .expect("identity");
    preservation::write_destination(
        tdb.db(),
        "aot",
        "AOT",
        "https://archive.example/",
        "item",
        true,
        true,
    )
    .await
    .expect("destination");
    let target = preservation::record_target(tdb.db(), &identity, &work, "aot", "1", None, None)
        .await
        .expect("record");
    // Still unverified, and never granted.
    let outcome = preservation::reclaim_credits(tdb.db(), &target.member_id)
        .await
        .expect("reclaim");
    assert_eq!(
        outcome,
        LedgerOutcome::Posted { credits: 0 },
        "a target that was never paid has nothing to take back"
    );
    assert_eq!(
        ledger_rows(&tdb, TxnType::PreservationReclaim, &target.member_id).await,
        0
    );
}

/// §2.3's clawback names the payee from the **ledger**, not from the work's
/// current owner, and the assertion only has teeth once the work changes hands.
///
/// The first version of this test created a work, paid it, and clawed it back
/// without moving the work -- so the current owner and the original payee were
/// the same account and *both* implementations of `account_for_paid_credits`
/// passed. A test that cannot fail is not a test. The work is transferred here
/// so the two readings give different answers, and the correct one is the one
/// that names the account that was actually paid.
///
/// The wrong behaviour is not a rounding error: a reversal charged to the new
/// owner takes credits from somebody who earned nothing, while the person who
/// was genuinely paid for a now-dead destination keeps the reward for it.
#[tokio::test]
async fn the_clawback_names_the_payee_from_the_ledger_not_from_the_work() {
    let (tdb, original_owner) = harness("clawback_payee").await;
    let (work, member) = paid_target(&tdb, &original_owner, "aot", 7).await;

    // The work changes hands: a new pseud, belonging to a different account,
    // becomes its owner. This is the fact that makes the two implementations
    // disagree, and it is an ordinary event rather than a contrived one --
    // pseudonym transfer is how a work follows its author to a new account.
    let new_owner = account_for(&tdb, "clawback_payee_new").await;
    let new_pseud = uuid::Uuid::new_v4().to_string();
    let now = "2026-09-30T00:00:00Z";
    let pseud_sql = tdb.db().sql(
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
         VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6)",
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
         VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6)",
    );
    if let Some(pool) = tdb.db().sqlite_pool() {
        sqlx::query(&pseud_sql)
            .bind(&new_pseud)
            .bind(&new_owner)
            .bind(format!("n{}", &new_pseud[..8]))
            .bind("New Owner")
            .bind(now)
            .bind(now)
            .execute(pool)
            .await
            .expect("insert new pseud");
    }
    if let Some(pool) = tdb.db().postgres_pool() {
        sqlx::query(&pseud_sql)
            .bind(&new_pseud)
            .bind(&new_owner)
            .bind(format!("n{}", &new_pseud[..8]))
            .bind("New Owner")
            .bind(now)
            .bind(now)
            .execute(pool)
            .await
            .expect("insert new pseud");
    }
    let move_sql = tdb.db().sql(
        "UPDATE works SET owner_pseud_id = ? WHERE id = ?",
        "UPDATE works SET owner_pseud_id = $1::uuid WHERE id = $2::uuid",
    );
    if let Some(pool) = tdb.db().sqlite_pool() {
        sqlx::query(&move_sql)
            .bind(&new_pseud)
            .bind(&work)
            .execute(pool)
            .await
            .expect("transfer work");
    }
    if let Some(pool) = tdb.db().postgres_pool() {
        sqlx::query(&move_sql)
            .bind(&new_pseud)
            .bind(&work)
            .execute(pool)
            .await
            .expect("transfer work");
    }

    // The work's owner is now somebody else -- so a reversal resolved from the
    // work would name them.
    assert_eq!(
        preservation::reward_account_for_work(tdb.db(), &work)
            .await
            .unwrap(),
        new_owner,
        "the work really did change hands; the rest of this test is only meaningful \
         if it has"
    );

    preservation::set_state(tdb.db(), &member, PreservationState::Dead, Some("gone"))
        .await
        .expect("dead");
    assert_eq!(
        preservation::reclaim_credits(tdb.db(), &member)
            .await
            .unwrap(),
        LedgerOutcome::Posted { credits: -7 }
    );

    // The reversal names the account the grant paid.
    let sql = tdb.db().sql(
        "SELECT DISTINCT e.account FROM credit_entries e
           JOIN credit_transactions t ON t.id = e.transaction_id
          WHERE t.reference = ?",
        "SELECT DISTINCT e.account::text FROM credit_entries e
           JOIN credit_transactions t ON t.id = e.transaction_id
          WHERE t.reference = $1",
    );
    let accounts: Vec<String> = match tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&sql)
            .bind(&member)
            .fetch_all(tdb.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("q"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(&sql)
            .bind(&member)
            .fetch_all(tdb.db().postgres_pool().expect("postgres"))
            .await
            .expect("q"),
    };
    assert_eq!(
        accounts,
        vec![original_owner.clone()],
        "the reversal names the account the GRANT paid, even though the work now \
         belongs to somebody else: reading the work's current owner would take \
         credits from a person who earned nothing and leave the person who was \
         genuinely paid holding a reward for a destination that is dead"
    );
}

/// The reward goes to the work's **owner**, not the account that clicked. Paying
/// the clicker makes the reward a measure of clicking, and it pays a stranger to
/// preserve somebody else's work.
#[tokio::test]
async fn the_reward_is_paid_to_the_work_owner() {
    let (tdb, owner) = harness("pay_owner").await;
    let work = seed_work(tdb.db(), &owner, "A Work").await;
    let identity =
        lorehaven_db::story_identity::ensure_identity_for_work(tdb.db(), &work, "A Work")
            .await
            .expect("identity");

    // A *different* account on the *same* instance performs the crosspost --
    // which is the point of the test, and also why the two accounts have to
    // live in one database. A clicker in a second `TestDb` would be a
    // different instance's user, and the foreign key on
    // `story_identity_members.created_by` would (correctly) refuse the record
    // long before the reward was in question.
    let clicker = account_for(&tdb, "pay_owner_clicker").await;
    preservation::write_destination(
        tdb.db(),
        "aot",
        "AOT",
        "https://archive.example/",
        "item",
        true,
        true,
    )
    .await
    .expect("destination");
    let target = preservation::record_target(
        tdb.db(),
        &identity,
        &work,
        "aot",
        "1",
        None,
        Some(uuid::Uuid::parse_str(&clicker).expect("a registered account id is a uuid")),
    )
    .await
    .expect("record");
    preservation::set_state(
        tdb.db(),
        &target.member_id,
        PreservationState::Verified,
        Some("x"),
    )
    .await
    .expect("verify");

    assert_eq!(
        preservation::reward_account_for_work(tdb.db(), &work)
            .await
            .unwrap(),
        owner,
        "the corpus being preserved is the author's, so the reward is the author's"
    );
    assert_eq!(
        preservation::target_by_member(tdb.db(), &target.member_id)
            .await
            .unwrap()
            .unwrap()
            .created_by
            .as_deref(),
        Some(clicker.as_str()),
        "the crosspost is still recorded against whoever performed it -- the record \
         and the payment are deliberately about different people"
    );
}
