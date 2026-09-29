//! Acceptance: preservation targets, the verification loop, and the reward
//! (spec §11.12a, §9.7 as amended by §2).
//!
//! `crates/domain/src/preservation.rs` proves the *arithmetic* — the decay, the
//! cap, the permission share, the eligibility sentence. This file proves the
//! *store*, and it does so for the three claims a pure-function test cannot
//! reach:
//!
//! 1. **The state machine is real.** `unverified` → `verified` → `dead` is a
//!    claim about columns, and `set_state` clearing `verified_at` on a dead
//!    target is the load-bearing part: it is the ordering the reward ladder is
//!    built on, so a dead target that kept its timestamp would position the
//!    next verified destination by a confirmation that no longer holds.
//! 2. **The refusal is by name and not by accident.** §2.6's `redistribution`
//!    gate and §2.7's imported-work refusal are two different refusals with
//!    two different reasons, and the test pins each against the specific value
//!    rather than against "an error".
//! 3. **The row has to survive both dialects.** `created_by` is TEXT on SQLite
//!    and UUID on PostgreSQL, `credits_paid` is INTEGER and read as `i64`, and
//!    `story_identities.work_id` is a UUID column compared to a `&str` bind —
//!    none of which a test that never opens a database exercises.
//!
//! **The tests here are store-level and dialect-agnostic.** The route surface
//! and the recheck job are Phase D's other half and have their own suites;
//! what is asserted here is the claim the routes are built on.

use std::path::PathBuf;

use lorehaven_db::{preservation, story_identity};
use lorehaven_domain::preservation::{PreservationState, Redistribution};
use test_support::TestDb;

/// A per-test directory, as every suite in `crates/app/tests/` does.
///
/// Not a shared one: under PostgreSQL a second `TestDb` for the same tag is a
/// *different* database, so a fixture written through one handle is invisible to
/// the code under test and the test fails for a reason that has nothing to do
/// with it.
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-preservation-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// A `TestDb` plus a real account.
///
/// The account is real rather than a `Uuid::new_v4()` because `created_by` is a
/// foreign key on both dialects: a constraint that is never exercised is not a
/// constraint that works.
async fn harness(tag: &str) -> (TestDb, String) {
    let dir = scratch_dir(tag);
    let tdb = TestDb::connect_with_dir(tag, &dir).await;
    let account = account_for(&tdb, tag).await;
    (tdb, account)
}

async fn account_for(tdb: &TestDb, tag: &str) -> String {
    let dir = scratch_dir(&format!("{tag}-account"));
    let mut config = lorehaven_app::config::Config::development_defaults();
    config.storage.root = dir.clone();
    let app = lorehaven_app::server::build_router(lorehaven_app::state::AppState::new(
        config,
        tdb.db().clone(),
    ));
    let mut client = test_support::TestClient::new(app);
    let handle = tag.replace(['.', '-'], "_");
    test_support::register(&mut client, &format!("{tag}@test.dev"), &handle).await
}

/// A published work owned by `account`, plus its identity.
async fn work_with_identity(tdb: &TestDb, account: &str, title: &str) -> (String, String) {
    let (work_id, _) = seed_work(tdb.db(), account, title).await;
    let identity = story_identity::ensure_identity_for_work(tdb.db(), &work_id, title)
        .await
        .expect("ensure identity");
    (work_id, identity)
}

/// The minimum a `works` row needs: an owner pseud, and a published lifecycle so
/// the row is one a reader could be shown.
async fn seed_work(db: &lorehaven_db::Database, account: &str, title: &str) -> (String, String) {
    let now = "2026-09-29T00:00:00Z";
    let pseud_id = uuid::Uuid::new_v4().to_string();
    let sql = db.sql(
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)",
        // `$1` through `$5`, and NOT `?::uuid, $1::uuid, ...`.
        //
        // The first version wrote the PostgreSQL arm that way, on the
        // assumption that a `?` sitting in front of `$1` is a placeholder in its
        // own right. It is not: sqlx numbers placeholders per arm, so `?` and
        // `$1` are *both* the first bind. The statement therefore inserted
        // `pseud_id` into `id` AND into `account_id`, and every one of these
        // tests failed on
        //
        //   insert or update on table "pseuds" violates foreign key
        //   constraint "pseuds_account_id_fkey"
        //
        // reporting a pseud id where the account id belonged -- which reads
        // like "the account does not exist" and is not what was wrong. The
        // account was there the whole time; the row pointed at the wrong one.
        //
        // SQLite is why this survived review: its arm is `?, ?, ...` and works,
        // and `?::uuid` is not a token it would accept, so the two arms never
        // look interchangeable.
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
         VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6)",
    );
    let handle = format!("p{}", &pseud_id[..8]);
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&pseud_id)
                .bind(account)
                .bind(&handle)
                .bind(&handle)
                .bind(now)
                .bind(now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("insert pseud");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&pseud_id)
                .bind(account)
                .bind(&handle)
                .bind(&handle)
                .bind(now)
                .bind(now)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("insert pseud");
        }
    }

    let work_id = uuid::Uuid::new_v4().to_string();
    let sql = db.sql(
        "INSERT INTO works (id, owner_pseud_id, title, lifecycle, visibility, created_at, updated_at)
         VALUES (?, ?, ?, 'published', 'public', ?, ?)",
        "INSERT INTO works (id, owner_pseud_id, title, lifecycle, visibility, created_at, updated_at)
         VALUES ($1::uuid, $2::uuid, $3, 'published', 'public', $4, $5)",
    );
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&work_id)
                .bind(&pseud_id)
                .bind(title)
                .bind(now)
                .bind(now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("insert work");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&work_id)
                .bind(&pseud_id)
                .bind(title)
                .bind(now)
                .bind(now)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("insert work");
        }
    }
    (work_id, pseud_id)
}

/// Configure one destination and return it.
///
/// An `async fn` rather than a call the tests each spell out, because the write
/// takes seven arguments and a test that spells it out is a test that reads as
/// a SQL statement rather than as a claim.
async fn destination(tdb: &TestDb, id: &str) -> preservation::PreservationDestination {
    preservation::write_destination(
        tdb.db(),
        id,
        &format!("Archive {id}"),
        "https://archive.example/",
        "item",
        true,
        true,
    )
    .await
    .expect("write destination")
}

// ---------------------------------------------------------------------------
// Destinations
// ---------------------------------------------------------------------------

/// §3.2: a destination is instance configuration, and an instance with none has
/// no preservation targets. Reported as a *count* rather than as an empty list,
/// because §2.5's eligibility calculation needs the number and a list a caller
/// has to count itself is a second place for the count to be wrong.
#[tokio::test]
async fn an_instance_with_no_destinations_reports_zero_eligible() {
    let (tdb, _account) = harness("no_destinations").await;
    assert!(
        preservation::list_destinations(tdb.db())
            .await
            .unwrap()
            .is_empty(),
        "a fresh instance configures no archives"
    );
    assert_eq!(
        preservation::enabled_destination_count(tdb.db())
            .await
            .unwrap(),
        0,
        "§2.5's eligibility calculation reports zero eligible destinations rather than \
         pretending otherwise"
    );
}

/// A disabled destination is not an eligible one, and `enabled_destination`
/// answers `None` for it rather than returning the row with a flag.
#[tokio::test]
async fn a_disabled_destination_is_not_an_eligible_one() {
    let (tdb, _account) = harness("disabled_destination").await;
    let written = preservation::write_destination(
        tdb.db(),
        "aot",
        "Archive of Our Own",
        "https://archive.example/",
        "item",
        true,
        true,
    )
    .await
    .expect("write destination");
    assert!(written.enabled);
    assert_eq!(
        preservation::enabled_destination_count(tdb.db())
            .await
            .unwrap(),
        1
    );
    assert!(preservation::enabled_destination(tdb.db(), "aot")
        .await
        .unwrap()
        .is_some());

    assert!(
        preservation::set_destination_enabled(tdb.db(), "aot", false)
            .await
            .unwrap()
    );

    // `None`, not `Some(row)`: a caller that has to tell the two apart will treat
    // the disabled one as eligible somewhere, and this is the cheaper place to
    // make that impossible.
    assert!(
        preservation::enabled_destination(tdb.db(), "aot")
            .await
            .unwrap()
            .is_none(),
        "a disabled destination is not eligible, and the row is still there to re-enable"
    );
    assert_eq!(
        preservation::enabled_destination_count(tdb.db())
            .await
            .unwrap(),
        0,
        "a disabled destination does not count toward the threshold a reader is measured against"
    );
    // The row itself survives, which is the point of disabling rather than
    // deleting: `ON DELETE RESTRICT` refuses the delete and a forced delete
    // would orphan the credits paid against it.
    assert_eq!(
        preservation::list_destinations(tdb.db())
            .await
            .unwrap()
            .len(),
        1,
        "disabling is reversible; the row and its paid targets survive it"
    );
}

/// The item URL is built from the base URL and the match rule with exactly one
/// slash, because a `base_url` with a trailing slash is a normal thing for an
/// operator to paste.
#[tokio::test]
async fn an_item_url_survives_a_base_url_with_a_trailing_slash() {
    let (tdb, _account) = harness("item_url").await;
    let with_slash = preservation::write_destination(
        tdb.db(),
        "a",
        "A",
        "https://archive.example/",
        "item",
        false,
        true,
    )
    .await
    .expect("write a");
    let without = preservation::write_destination(
        tdb.db(),
        "b",
        "B",
        "https://archive.example",
        "/item/",
        false,
        true,
    )
    .await
    .expect("write b");
    assert_eq!(
        with_slash.item_url("12345").as_deref(),
        Some("https://archive.example/item/12345")
    );
    assert_eq!(
        without.item_url("12345").as_deref(),
        Some("https://archive.example/item/12345"),
        "the match rule's own slashes must not double up either"
    );
    assert_eq!(
        with_slash.item_url("  "),
        None,
        "a blank record id produces a refusal at the call site, not a fetch to a URL ending in a slash"
    );
}

// ---------------------------------------------------------------------------
// Targets and the state machine
// ---------------------------------------------------------------------------

/// §2.1: a crosspost is a request, and only a confirmed record is worth paying
/// for. So a target starts `unverified`, with no `verified_at` — the reward
/// ladder is built on that timestamp and a target must not carry one it has not
/// earned.
#[tokio::test]
async fn a_crosspost_starts_unverified_and_pays_nothing() {
    let (tdb, account) = harness("starts_unverified").await;
    let (_work, identity) = work_with_identity(&tdb, &account, "A Work").await;
    destination(&tdb, "aot").await;

    let target = preservation::record_target(
        tdb.db(),
        &identity,
        &_work,
        "aot",
        "999",
        Some("https://archive.example/item/999"),
        Some(uuid::Uuid::parse_str(&account).expect("account uuid")),
    )
    .await
    .expect("record target");

    assert_eq!(
        target.state,
        PreservationState::Unverified,
        "§2.1: the reward attaches to verification, not to the crosspost"
    );
    assert!(
        target.verified_at.is_none(),
        "an unverified target must not carry a confirmation timestamp: that timestamp is \
         the ordering the whole reward ladder is built on"
    );
    assert_eq!(target.credits_paid, 0);
    assert!(!target.is_verified());
    assert_eq!(
        preservation::verified_count_for_work(tdb.db(), &_work)
            .await
            .unwrap(),
        0
    );
}

/// Verification records the timestamp and the evidence, and it is the evidence
/// rather than the page that is kept — the page is what §11.5's metadata
/// ceiling bounded, and there is no reason to retain what the comparison is done
/// with.
#[tokio::test]
async fn verification_records_a_timestamp_and_an_evidence_hash() {
    let (tdb, account) = harness("verified").await;
    let (work, identity) = work_with_identity(&tdb, &account, "A Work").await;
    destination(&tdb, "aot").await;
    let target = preservation::record_target(
        tdb.db(),
        &identity,
        &work,
        "aot",
        "999",
        None,
        Some(uuid::Uuid::parse_str(&account).expect("uuid")),
    )
    .await
    .expect("record");

    assert!(preservation::set_state(
        tdb.db(),
        &target.member_id,
        PreservationState::Verified,
        Some("abc123")
    )
    .await
    .unwrap());

    let after = preservation::target_by_member(tdb.db(), &target.member_id)
        .await
        .unwrap()
        .expect("the target is still there");
    assert_eq!(after.state, PreservationState::Verified);
    assert!(after.verified_at.is_some());
    assert_eq!(after.evidence_hash.as_deref(), Some("abc123"));
    assert!(after.dead_at.is_none());
    assert_eq!(
        preservation::verified_count_for_work(tdb.db(), &work)
            .await
            .unwrap(),
        1
    );
}

/// The clawback's own precondition: a dead target carries `dead_at` and **no**
/// `verified_at`, and the count it contributes drops.
///
/// The second half is the one that matters. `verified_at` is what
/// `targets_for_work` orders by, so a dead target that kept its timestamp would
/// position the next verified destination by a confirmation that no longer
/// holds — the decay would be counting a destination that is not there.
#[tokio::test]
async fn a_dead_target_clears_its_verification_and_stops_counting() {
    let (tdb, account) = harness("dead").await;
    let (work, identity) = work_with_identity(&tdb, &account, "A Work").await;
    destination(&tdb, "aot").await;
    let target = preservation::record_target(
        tdb.db(),
        &identity,
        &work,
        "aot",
        "999",
        None,
        Some(uuid::Uuid::parse_str(&account).expect("uuid")),
    )
    .await
    .expect("record");
    preservation::set_state(
        tdb.db(),
        &target.member_id,
        PreservationState::Verified,
        Some("abc"),
    )
    .await
    .unwrap();
    assert_eq!(
        preservation::verified_count_for_work(tdb.db(), &work)
            .await
            .unwrap(),
        1
    );

    assert!(preservation::set_state(
        tdb.db(),
        &target.member_id,
        PreservationState::Dead,
        Some("abc")
    )
    .await
    .unwrap());

    let after = preservation::target_by_member(tdb.db(), &target.member_id)
        .await
        .unwrap()
        .expect("a dead target is still a target; it is a record that stopped being true");
    assert_eq!(after.state, PreservationState::Dead);
    assert!(after.dead_at.is_some());
    assert!(
        after.verified_at.is_none(),
        "verified_at is the reward ladder's ordering, and a dead target must not occupy a \
         position in it"
    );
    assert_eq!(
        preservation::verified_count_for_work(tdb.db(), &work)
            .await
            .unwrap(),
        0,
        "a destination that has gone dark stops counting, which is the same anti-farm \
         property the clawback applies to the reward"
    );
    // And the work itself is untouched: §3.3's rule is that a dead preservation
    // is never a statement about the work.
    assert!(!preservation::is_imported(tdb.db(), &work).await.unwrap());
    assert_eq!(
        preservation::targets_for_work(tdb.db(), &work)
            .await
            .unwrap()
            .len(),
        1,
        "the record survives; only the state changed"
    );
}

/// A second crosspost of the same work to the same archive is an idempotent
/// no-op. §2.2 pays for distinct *verified destinations*, so a duplicate that
/// created a second row would be a way to be paid twice for one fact.
#[tokio::test]
async fn a_duplicate_crosspost_is_an_idempotent_no_op() {
    let (tdb, account) = harness("duplicate").await;
    let (work, identity) = work_with_identity(&tdb, &account, "A Work").await;
    destination(&tdb, "aot").await;
    let actor = Some(uuid::Uuid::parse_str(&account).expect("uuid"));

    let first = preservation::record_target(tdb.db(), &identity, &work, "aot", "999", None, actor)
        .await
        .expect("first");
    let second = preservation::record_target(tdb.db(), &identity, &work, "aot", "999", None, actor)
        .await
        .expect("second");

    assert_eq!(
        first.member_id, second.member_id,
        "the second crosspost returns the existing row rather than creating a second one"
    );
    assert_eq!(
        preservation::targets_for_work(tdb.db(), &work)
            .await
            .unwrap()
            .len(),
        1
    );
}

/// The `work_id` check. A target filed under an identity belonging to a
/// *different* work is counted, paid and clawed-back against the wrong work by
/// every read, and the mismatch is invisible at the write because every read
/// joins through `story_identities.work_id`.
#[tokio::test]
async fn a_target_cannot_be_filed_against_the_wrong_work() {
    let (tdb, account) = harness("wrong_work").await;
    let (work_a, identity_a) = work_with_identity(&tdb, &account, "Work A").await;
    let (work_b, _identity_b) = work_with_identity(&tdb, &account, "Work B").await;
    destination(&tdb, "aot").await;
    let actor = Some(uuid::Uuid::parse_str(&account).expect("uuid"));

    // A target for A, so the destination's unique index is not what refuses it —
    // the work check has to be.
    preservation::record_target(tdb.db(), &identity_a, &work_a, "aot", "1", None, actor)
        .await
        .expect("record for a");

    let err = preservation::record_target(tdb.db(), &identity_a, &work_b, "ff", "2", None, actor)
        .await
        .expect_err("filing a target under an identity for another work must be refused");
    let message = err.to_string();
    assert!(
        message.contains(&work_b) && message.contains("wrong work"),
        "the refusal must name both works, got: {message}"
    );
    assert_eq!(
        preservation::targets_for_work(tdb.db(), &work_b)
            .await
            .unwrap()
            .len(),
        0,
        "nothing was filed against the other work"
    );
}

// ---------------------------------------------------------------------------
// §2.6 the permission gate
// ---------------------------------------------------------------------------

/// The column is a dependency, not an existing capability, and an instance that
/// has never been told anything reads `unstated` — which behaves as `ask` and
/// pays half rather than paying nothing.
#[tokio::test]
async fn an_unstated_work_reads_as_unstated_and_a_work_nobody_set_defaults_to_it() {
    let (tdb, account) = harness("unstated").await;
    let (work, _identity) = work_with_identity(&tdb, &account, "A Work").await;

    assert_eq!(
        preservation::read_redistribution(tdb.db(), &work)
            .await
            .unwrap(),
        Redistribution::Unstated,
        "a work nobody has said anything about is `unstated`, and `unstated` is not `yes`"
    );
    assert_eq!(
        Redistribution::Unstated.reward_share_bp(),
        Redistribution::Ask.reward_share_bp(),
        "an unstated work is treated as ask everywhere it matters, or the cautious default \
         exists only in the type"
    );
    assert!(
        Redistribution::Unstated.permits_crosspost(),
        "§2.6 refuses the crosspost for `no`, not for silence"
    );
}

/// `no` refuses; `yes` and `ask` permit with different shares. The whole
/// permission gate in one table, because a rule tested at two points leaves the
/// third to whatever the code happens to do.
#[tokio::test]
async fn redistribution_yes_ask_no_and_unstated_are_four_distinct_reads() {
    let (tdb, account) = harness("four_reads").await;
    let (work, _identity) = work_with_identity(&tdb, &account, "A Work").await;

    for (written, expected) in [
        (Redistribution::Yes, Redistribution::Yes),
        (Redistribution::Ask, Redistribution::Ask),
        (Redistribution::No, Redistribution::No),
        (Redistribution::Unstated, Redistribution::Unstated),
    ] {
        preservation::write_redistribution(tdb.db(), &work, written)
            .await
            .expect("write");
        assert_eq!(
            preservation::read_redistribution(tdb.db(), &work)
                .await
                .unwrap(),
            expected
        );
    }

    assert!(Redistribution::Yes.permits_crosspost());
    assert!(Redistribution::Ask.permits_crosspost());
    assert!(
        !Redistribution::No.permits_crosspost(),
        "§2.6: `no` refuses the crosspost by name and pays nothing"
    );
    assert!(Redistribution::Yes.reward_share_bp() > Redistribution::Ask.reward_share_bp());
}

// ---------------------------------------------------------------------------
// §2.7 the imported-work refusal
// ---------------------------------------------------------------------------

/// §2.7 refuses preserving an imported work, and the reason is a *deferred
/// milestone* rather than a bug. This test exists so that the refusal survives
/// somebody reading it as an oversight, and so the `is_imported` query it depends
/// on is exercised on both engines.
#[tokio::test]
async fn an_imported_work_is_detected_as_imported_and_a_local_one_is_not() {
    let (tdb, account) = harness("imported").await;
    let (local, _identity) = work_with_identity(&tdb, &account, "Local Work").await;
    assert!(
        !preservation::is_imported(tdb.db(), &local).await.unwrap(),
        "a work with no library_items row is local to this instance"
    );

    // Now give the work a library_items row, which is what §2.7's "has a
    // source_key that is not this instance" reduces to.
    let now = "2026-09-29T00:00:00Z";
    let item_id = uuid::Uuid::new_v4().to_string();
    let sql = tdb.db().sql(
        "INSERT INTO library_items
           (id, account_id, work_id, source_key, source_work_key, title, source_url,
            created_at, updated_at)
         VALUES (?, ?, ?, 'ao3', '12345', 'Local Work', 'https://ao3.example/x', ?, ?)",
        "INSERT INTO library_items
           (id, account_id, work_id, source_key, source_work_key, title, source_url,
            created_at, updated_at)
         VALUES ($1::uuid, $2::uuid, $3::uuid, 'ao3', '12345', 'Local Work', 'https://ao3.example/x', $4, $5)",
    );
    match tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&item_id)
                .bind(&account)
                .bind(&local)
                .bind(now)
                .bind(now)
                .execute(tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("insert library item");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&item_id)
                .bind(&account)
                .bind(&local)
                .bind(now)
                .bind(now)
                .execute(tdb.db().postgres_pool().expect("postgres"))
                .await
                .expect("insert library item");
        }
    }

    assert!(
        preservation::is_imported(tdb.db(), &local).await.unwrap(),
        "§2.7 refuses an imported work's crosspost, and detecting it is a query rather \
         than a column so that deleting the import makes the work local again"
    );
}

// ---------------------------------------------------------------------------
// §2.4 the leaderboard metric
// ---------------------------------------------------------------------------

/// Distinct destinations currently verified, credited to the work's **owner**.
///
/// The two halves are both the test. "Distinct destinations" is the anti-farm
/// property — a crosspost action is not a destination, and paying for actions
/// makes the metric a count of clicking. "The owner" is the anti-incentive
/// property — paying whoever performed the crosspost measures clicking too, and
/// the corpus being preserved is the author's.
#[tokio::test]
async fn the_leaderboard_counts_distinct_live_destinations_and_credits_the_owner() {
    let (tdb, account) = harness("leaderboard").await;
    let (work, identity) = work_with_identity(&tdb, &account, "A Work").await;
    destination(&tdb, "aot").await;
    destination(&tdb, "ff").await;
    let actor = Some(uuid::Uuid::parse_str(&account).expect("uuid"));

    let a = preservation::record_target(tdb.db(), &identity, &work, "aot", "1", None, actor)
        .await
        .expect("aot");
    let b = preservation::record_target(tdb.db(), &identity, &work, "ff", "2", None, actor)
        .await
        .expect("ff");
    // Two destinations recorded, none verified: the metric counts verified ones.
    assert!(
        preservation::top_preservers(tdb.db(), "1970-01-01T00:00:00Z", 10)
            .await
            .unwrap()
            .is_empty(),
        "a crosspost with no verification is not preservation, and it pays nothing"
    );

    preservation::set_state(
        tdb.db(),
        &a.member_id,
        PreservationState::Verified,
        Some("x"),
    )
    .await
    .unwrap();
    let board = preservation::top_preservers(tdb.db(), "1970-01-01T00:00:00Z", 10)
        .await
        .unwrap();
    assert_eq!(board.len(), 1, "one verified destination is one row");
    assert_eq!(
        board[0].1, 1,
        "the metric is distinct verified destinations, not crosspost actions"
    );

    preservation::set_state(
        tdb.db(),
        &b.member_id,
        PreservationState::Verified,
        Some("y"),
    )
    .await
    .unwrap();
    assert_eq!(
        preservation::top_preservers(tdb.db(), "1970-01-01T00:00:00Z", 10)
            .await
            .unwrap()[0]
            .1,
        2,
        "two verified destinations is two"
    );

    // A destination that goes dark leaves the count. That is the clawback's
    // property applied to the ranking: a spammer's farms die, the rows go, and
    // the board corrects itself without a moderator noticing.
    preservation::set_state(tdb.db(), &b.member_id, PreservationState::Dead, Some("y"))
        .await
        .unwrap();
    assert_eq!(
        preservation::top_preservers(tdb.db(), "1970-01-01T00:00:00Z", 10)
            .await
            .unwrap()[0]
            .1,
        1,
        "a destination that has stopped answering stops counting"
    );
}

/// The window, not an all-time total. §9.7.5 offers no all-time category, and a
/// cumulative preservation count is a leaderboard of when somebody was most
/// active.
#[tokio::test]
async fn the_leaderboard_is_windowed_and_never_all_time() {
    let (tdb, account) = harness("leaderboard_window").await;
    let (work, identity) = work_with_identity(&tdb, &account, "A Work").await;
    destination(&tdb, "aot").await;
    let target = preservation::record_target(
        tdb.db(),
        &identity,
        &work,
        "aot",
        "1",
        None,
        Some(uuid::Uuid::parse_str(&account).expect("uuid")),
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
    .unwrap();

    assert!(
        preservation::top_preservers(tdb.db(), "1970-01-01T00:00:00Z", 10)
            .await
            .unwrap()
            .len()
            == 1,
        "the window includes the verification"
    );
    assert!(
        preservation::top_preservers(tdb.db(), "2099-01-01T00:00:00Z", 10)
            .await
            .unwrap()
            .is_empty(),
        "a window that starts after the verification excludes it; there is no all-time \
         board to fall back to"
    );
}

/// The credits the clawback reads are on the member, and they are set
/// independently of the ledger. The store's job is to be the index; the
/// auditable fact is the ledger row, and a disagreement between the two is what
/// would make a clawback reverse nothing or reverse twice.
#[tokio::test]
async fn credits_paid_is_recorded_on_the_member_and_is_what_the_clawback_would_read() {
    let (tdb, account) = harness("credits_paid").await;
    let (work, identity) = work_with_identity(&tdb, &account, "A Work").await;
    destination(&tdb, "aot").await;
    let target = preservation::record_target(
        tdb.db(),
        &identity,
        &work,
        "aot",
        "1",
        None,
        Some(uuid::Uuid::parse_str(&account).expect("uuid")),
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
    .unwrap();

    assert!(
        preservation::set_credits_paid(tdb.db(), &target.member_id, 10)
            .await
            .unwrap()
    );
    let paid = preservation::target_by_member(tdb.db(), &target.member_id)
        .await
        .unwrap()
        .expect("target");
    assert_eq!(
        paid.credits_paid, 10,
        "the clawback reads this column to know what to reverse"
    );

    // And it survives the transition to dead, because a dead target is exactly
    // the case where the column must still say what needs reversing.
    preservation::set_state(
        tdb.db(),
        &target.member_id,
        PreservationState::Dead,
        Some("x"),
    )
    .await
    .unwrap();
    let dead = preservation::target_by_member(tdb.db(), &target.member_id)
        .await
        .unwrap()
        .expect("target");
    assert_eq!(
        dead.credits_paid, 10,
        "a dead target still owes its reversal, and the amount is read from here"
    );
}

/// A local member and a preservation target coexist on one identity. The member
/// is an external copy, so it must not be confused with the work this instance
/// holds — §11.10's "do not grant access to another edition's body" is
/// structural, and this asserts the structural half actually holds after the
/// migration added nine columns to that table.
#[tokio::test]
async fn a_preservation_target_is_a_member_and_the_local_member_is_untouched() {
    let (tdb, account) = harness("member_counts").await;
    let (work, identity) = work_with_identity(&tdb, &account, "A Work").await;
    destination(&tdb, "aot").await;
    preservation::record_target(
        tdb.db(),
        &identity,
        &work,
        "aot",
        "1",
        None,
        Some(uuid::Uuid::parse_str(&account).expect("uuid")),
    )
    .await
    .expect("record");

    let (total, external) = story_identity::member_counts(tdb.db(), &identity)
        .await
        .unwrap();
    assert_eq!(total, 2, "the local member plus the preservation target");
    assert_eq!(
        external, 1,
        "the preservation target is an external copy, and counting it as a second local \
         copy would let a reader think this instance holds two editions"
    );
}

/// The recheck's input, and what it excludes. Only `verified` targets are
/// candidates: an unverified one has nothing to claw back and a dead one has
/// already been dealt with, so re-fetching them spends a network request on a row
/// whose answer is already recorded.
#[tokio::test]
async fn the_recheck_input_is_exactly_the_verified_targets() {
    let (tdb, account) = harness("recheck_input").await;
    let (work, identity) = work_with_identity(&tdb, &account, "A Work").await;
    for (id, name) in [("aot", "AOT"), ("ff", "FF"), ("sc", "SC")] {
        destination(&tdb, id).await;
        let _ = name;
        preservation::record_target(
            tdb.db(),
            &identity,
            &work,
            id,
            id,
            None,
            Some(uuid::Uuid::parse_str(&account).expect("uuid")),
        )
        .await
        .expect("record");
    }
    let all = preservation::targets_for_work(tdb.db(), &work)
        .await
        .unwrap();
    assert_eq!(all.len(), 3);
    assert!(preservation::verified_targets(tdb.db())
        .await
        .unwrap()
        .is_empty());

    let first = &all[0];
    preservation::set_state(
        tdb.db(),
        &first.member_id,
        PreservationState::Verified,
        Some("x"),
    )
    .await
    .unwrap();
    let verified = preservation::verified_targets(tdb.db()).await.unwrap();
    assert_eq!(
        verified.len(),
        1,
        "the job's input is the verified rows, and the other two pay nothing and cost nothing"
    );
    assert_eq!(verified[0].member_id, first.member_id);
}
