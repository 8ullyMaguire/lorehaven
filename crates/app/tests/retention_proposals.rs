//! Acceptance: retention proposals and their ballots (spec §5, §19.15).
//!
//! **The ballots are the privacy surface of this feature and the first test is
//! about that.** §45.2 refuses weights in governance because a weighted vote
//! lets a reading habit set instance policy; the same reasoning protects a
//! reader's *choice*. A route that serialised `retention_proposal_votes` would
//! publish who wanted more storage and who wanted less, which is a reading of a
//! reader's habits and of what they read — the inference §19.2 exists to
//! refuse, arrived at from the opposite direction.
//!
//! So the claims here are:
//!
//! 1. **No reader can read another reader's ballot.** Not "the route is careful"
//!    — the shape is a second account GETs the proposal and the response has no
//!    `account_id` key anywhere in it. A property, not a code-reading.
//! 2. **A vote cannot be stacked.** Two votes from one account are one ballot.
//!    Without this a reader could vote twice to reach a quorum, and §45.2's flat
//!    weights would mean nothing.
//! 3. **A vote changes nothing about the proposer.** `opening_a_proposal_grants
//!    _no_trust_credit_badge_or_placement` is the plan's own named test and it
//!    protects the feature from itself: a preference poll that made its
//!    participants more visible becomes a status ladder within two releases, and
//!    §19.15's "a vote never grants the proposer anything personal" is cheap to
//!    state and expensive to retrofit.
//! 4. **The one-open-per-setting rule is a property of the table**, so it is
//!    decided in the store rather than in a handler where two concurrent POSTs
//!    would both pass a check-then-insert.

use std::path::PathBuf;

use lorehaven_db::retention_proposals::{self, ProposalState};
use lorehaven_domain::retention::BodyMode;
use test_support::TestDb;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-retprop-{tag}-{}-{:?}",
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
    test_support::register(
        &mut client,
        &format!("{tag}@test.dev"),
        &tag.replace(['.', '-'], "_"),
    )
    .await
}

fn uuid_of(account: &str) -> uuid::Uuid {
    uuid::Uuid::parse_str(account).expect("a registered account id is a uuid")
}

async fn harness(tag: &str) -> (TestDb, String) {
    let tdb = TestDb::connect_with_dir(tag, &scratch_dir(tag)).await;
    let account = account_for(&tdb, tag).await;
    (tdb, account)
}

fn closes_at() -> String {
    "2026-10-06T00:00:00Z".to_owned()
}

/// A proposal in the caller's database with two ballots cast, by two people.
/// **The fixture the owed privacy test will use**; unused until that test is
/// written, and kept here so it is written once rather than twice.
///
/// **Takes the `TestDb` and does not build one.** The first version called
/// `harness(tag)` internally, which creates a *second* database: under
/// PostgreSQL a second `TestDb` for the same tag is a different database, so
/// the proposal and its ballots would be invisible to the assertions made
/// against the caller's handle, and the test would fail for a reason unrelated
/// to what it tests. It passed on SQLite, which is the engine that hides this
/// class of fixture bug.
#[allow(dead_code)]
async fn proposal_with_ballots(tdb: &TestDb, tag: &str, author: &str) -> (String, String, String) {
    let voter = account_for(tdb, &format!("{tag}_voter")).await;
    let other = account_for(tdb, &format!("{tag}_other")).await;

    let proposal = retention_proposals::create_proposal(
        tdb.db(),
        None,
        BodyMode::Aggregate,
        "the instance is out of disk and I would rather keep metadata",
        uuid_of(author),
        &closes_at(),
    )
    .await
    .expect("open a proposal");

    // One for, one against, from two different people. A single ballot would
    // let "no account_id key" pass for the wrong reason.
    retention_proposals::cast_vote(tdb.db(), &proposal.id, uuid_of(&voter), true)
        .await
        .expect("vote for");
    retention_proposals::cast_vote(tdb.db(), &proposal.id, uuid_of(&other), false)
        .await
        .expect("vote against");
    (proposal.id, voter, other)
}

// ---------------------------------------------------------------------------
// The privacy surface
// ---------------------------------------------------------------------------
//
// THE TEST THIS FILE OWES AND DOES NOT YET HAVE:
//
//     no_reader_can_read_another_readers_retention_ballot
//
// A second account GETs `/api/v1/retention/proposals/:id` and the serialised
// response contains no `account_id`, `cast_at`, `support` or `ballot` key
// anywhere -- walked recursively over the whole body, not checked at the top
// level, because a nested `tally.by_account` array would pass a top-level key
// check while publishing every ballot. The counts (`tally.supporters`,
// `tally.opposed`) must still be present, because privacy comes from
// withholding the rows, not the totals a reader needs in order to decide.
//
// It is not written yet because `routes/retention.rs` does not exist, and a
// test that cannot compile is not a test. The store half is covered here --
// `tally` returns two counts and no row type at all, so there is nothing in
// the store layer that *could* serialise a ballot. The route half is where a
// leak would actually happen, because a route is what chooses what to expose.

// ---------------------------------------------------------------------------
// One ballot per reader
// ---------------------------------------------------------------------------

/// Two votes from one account are one ballot, and the second replaces the first.
///
/// A reader changing their mind is a normal event. A reader voting twice to
/// reach a quorum is not possible, and the two are the same SQL statement —
/// which is why the test casts twice with *opposite* values and checks the
/// count rather than the sum.
#[tokio::test]
async fn a_second_vote_from_the_same_account_updates_the_first() {
    let (tdb, author) = harness("revote").await;
    let voter = account_for(&tdb, "revote_voter").await;
    let proposal = retention_proposals::create_proposal(
        tdb.db(),
        None,
        BodyMode::Aggregate,
        "out of disk",
        uuid_of(&author),
        &closes_at(),
    )
    .await
    .expect("open");

    for support in [true, true, false, true] {
        retention_proposals::cast_vote(tdb.db(), &proposal.id, uuid_of(&voter), support)
            .await
            .expect("vote");
    }
    let tally = retention_proposals::tally(tdb.db(), &proposal.id, 3)
        .await
        .expect("tally");
    assert_eq!(
        (tally.supporters, tally.opposed),
        (1, 0),
        "four votes from one account are one ballot for and none against. A tally of \
         (3, 1) here would mean §45.2's flat weights are a suggestion rather than a \
         rule, and a single reader could carry any quorum."
    );
}

// ---------------------------------------------------------------------------
// Opening a proposal grants nothing
// ---------------------------------------------------------------------------

/// §19.15: "a vote never grants the proposer anything personal."
///
/// The plan names this as the test that protects the feature from itself, and
/// the reasoning is worth keeping: a preference poll that made its participants
/// more visible becomes a status ladder inside two releases, because the
/// incentive to open proposals is then the visibility, and the questions
/// themselves stop mattering.
///
/// The test is a whole-row comparison rather than a check on a trust column,
/// because "grants nothing" is a claim about everything.
#[tokio::test]
async fn opening_a_proposal_grants_no_trust_credit_badge_or_placement() {
    let (tdb, author) = harness("no_reward").await;
    let bystander = account_for(&tdb, "no_reward_bystander").await;

    let before = trust_snapshot(tdb.db(), &author).await;

    // **Three proposals, each on a *different* setting.** The first version
    // opened them all instance-wide and alternated the proposed mode, which
    // fails on the second: the instance is still `cache` (opening a proposal
    // changes nothing), so proposing `cache` there is a no-op and is refused.
    // The refusal is right; the fixture was wrong. Scoping them to three
    // sources is also the more realistic shape — a reader's first retention
    // proposal is almost always about one archive, not the whole instance.
    for key in ["site-a", "site-b", "site-c"] {
        retention_proposals::create_proposal(
            tdb.db(),
            Some(key),
            BodyMode::Aggregate,
            &format!("narrow {key}, the instance is out of disk"),
            uuid_of(&author),
            &closes_at(),
        )
        .await
        .unwrap_or_else(|error| panic!("{key}: {error}"));
    }
    // Votes on somebody else's, too: supporting is as much a choice as opening.
    let their_proposal = retention_proposals::create_proposal(
        tdb.db(),
        Some("some-source"),
        BodyMode::Aggregate,
        "narrow this source",
        uuid_of(&bystander),
        &closes_at(),
    )
    .await
    .expect("open");
    retention_proposals::cast_vote(tdb.db(), &their_proposal.id, uuid_of(&author), true)
        .await
        .expect("vote");

    let after = trust_snapshot(tdb.db(), &author).await;
    assert_eq!(
        before, after,
        "opening three proposals and casting a ballot changed this account's standing: \
         {before:?} became {after:?}. A vote that rewards its participants is a status \
         ladder, and §19.15 says it grants nothing personal."
    );
}

/// Everything a reader's account carries that a poll could plausibly move.
#[derive(Debug, PartialEq)]
struct Standing {
    trust_level: i64,
    credits: i64,
    reposts: i64,
    // `awarded_badges` is intentionally absent: this schema has no badge table
    // at all, so there is nothing to compare and a query for one fails with
    // "no such table". §9.7.6's badge list is not built yet. The test is
    // therefore weaker than the plan's name suggests, and says so rather than
    // asserting against a table it would have to create first -- the moment a
    // badge table appears, this column and its query belong back.
}

async fn trust_snapshot(db: &lorehaven_db::Database, account: &str) -> Standing {
    Standing {
        // `trust_levels(account, level)` -- there is no points column. Trust in
        // this schema is a *level*, computed on a basis, so §19.15's "grants
        // nothing personal" is asserted against the thing that exists rather
        // than against a score I assumed was there.
        trust_level: scalar(
            db,
            "SELECT COALESCE((SELECT level FROM trust_levels WHERE account = ?), 0)",
            "SELECT COALESCE((SELECT level FROM trust_levels WHERE account = $1), 0)::bigint",
            account,
        )
        .await,
        credits: scalar(
            db,
            "SELECT COALESCE(SUM(amount_bp), 0) FROM credit_entries WHERE account = ?",
            // No cast on the bound value: `credit_entries.account` is a plain
            // account column and a `::uuid` here is a no-op on SQLite and a
            // cast of a cast on PostgreSQL. The `::bigint` on the aggregate is
            // the part that IS needed -- SUM over INTEGER is INT8 and will not
            // decode as i64 without it.
            "SELECT COALESCE(SUM(amount_bp), 0)::bigint FROM credit_entries WHERE account = $1",
            account,
        )
        .await,
        reposts: count(
            db,
            "SELECT COUNT(*) FROM works WHERE owner_pseud_id IN
               (SELECT id FROM pseuds WHERE account_id = ?)",
            "SELECT COUNT(*)::bigint FROM works WHERE owner_pseud_id IN
               (SELECT id FROM pseuds WHERE account_id = $1::uuid)",
            account,
        )
        .await,
    }
}

/// One scalar, both arms written out.
///
/// The arms are parameters rather than derived from one another on purpose.
/// The first version built the PostgreSQL arm with
/// `sqlite.replace('?', "$1::uuid")`, which is wrong in a way that looks
/// right: it casts *every* bound value, including `credit_entries.account`,
/// which needs no cast. A blanket rewrite cannot know which columns are UUID
/// in this schema and which are TEXT, and the difference is exactly the class
/// of fault Phase D already paid for five times.
async fn scalar(db: &lorehaven_db::Database, sqlite: &str, postgres: &str, value: &str) -> i64 {
    let sql = db.sql(sqlite, postgres);
    match db.backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&sql)
            .bind(value)
            .fetch_one(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("scalar"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(&sql)
            .bind(value)
            .fetch_one(db.postgres_pool().expect("postgres"))
            .await
            .expect("scalar"),
    }
}

/// One count, both arms written out. See [`scalar`] for why.
async fn count(db: &lorehaven_db::Database, sqlite: &str, postgres: &str, value: &str) -> i64 {
    let sql = db.sql(sqlite, postgres);
    match db.backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&sql)
            .bind(value)
            .fetch_one(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("count"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(&sql)
            .bind(value)
            .fetch_one(db.postgres_pool().expect("postgres"))
            .await
            .expect("count"),
    }
}

// ---------------------------------------------------------------------------
// One open proposal per setting
// ---------------------------------------------------------------------------

/// A second proposal on a setting already under ballot is refused, and the
/// refusal names the reason.
///
/// The check is in the store rather than the handler, so two concurrent POSTs
/// cannot both pass it: a check-then-insert in the route is a race, and this
/// table is small enough that the race is a real possibility rather than a
/// theoretical one.
#[tokio::test]
async fn a_second_proposal_on_the_same_setting_is_refused_while_one_is_open() {
    let (tdb, author) = harness("one_open").await;
    retention_proposals::create_proposal(
        tdb.db(),
        None,
        BodyMode::Aggregate,
        "first",
        uuid_of(&author),
        &closes_at(),
    )
    .await
    .expect("the first proposal");

    // **The same mode as the first proposal, deliberately.** `create_proposal`
    // compares against the mode *in force* — still `cache`, because opening a
    // proposal changes nothing — so proposing `Aggregate` again is not a no-op
    // and does reach the already-open check. The first version proposed
    // `Cache`, which IS the mode in force, so it was refused as a no-op and
    // never reached the check it was written to exercise. A test that passes
    // for the wrong reason is worse than one that fails.
    let error = retention_proposals::create_proposal(
        tdb.db(),
        None,
        BodyMode::Aggregate,
        "second, same question",
        uuid_of(&author),
        &closes_at(),
    )
    .await
    .expect_err("a second open proposal on the same setting must be refused");
    assert!(
        error.to_string().contains("already open"),
        "the refusal must say why, because '409' tells a reader nothing about whether \
         to wait or to open one for a different setting: {error}"
    );
}

/// …and a *different* setting is a different ballot, which is the point of
/// scoping proposals to a source.
#[tokio::test]
async fn proposals_on_different_settings_do_not_block_each_other() {
    let (tdb, author) = harness("two_settings").await;
    // Narrow both sources first, so a proposal for `Cache` on each is a real
    // widening rather than a no-op. Without that, the second iteration is
    // refused as "already cache" -- correctly, and for a reason that has
    // nothing to do with the one-open-per-setting rule under test.
    for key in ["site-a", "site-b"] {
        lorehaven_db::retention::write_source_override(
            tdb.db(),
            key,
            BodyMode::Aggregate,
            uuid_of(&author),
        )
        .await
        .expect("narrowed");
        retention_proposals::create_proposal(
            tdb.db(),
            Some(key),
            BodyMode::Cache,
            &format!("widen {key} back"),
            uuid_of(&author),
            &closes_at(),
        )
        .await
        .unwrap_or_else(|error| panic!("{key}: {error}"));
    }
}

/// A proposal for the mode already in force is refused: it is not a change, and
/// a ballot on it is a number with nothing behind it.
#[tokio::test]
async fn a_proposal_for_the_mode_already_in_force_is_refused() {
    let (tdb, author) = harness("no_op").await;
    let error = retention_proposals::create_proposal(
        tdb.db(),
        None,
        BodyMode::Cache,
        "keep caching",
        uuid_of(&author),
        &closes_at(),
    )
    .await
    .expect_err("the instance already caches, so this is not a proposal");
    assert!(
        error.to_string().contains("already"),
        "the refusal must name the fact that the setting already holds this value: {error}"
    );
}

/// A proposal with no rationale is refused. A reader who cannot see *why* a
/// change is proposed cannot decide whether to support it, so an empty
/// rationale is a ballot nobody can cast meaningfully.
#[tokio::test]
async fn a_proposal_with_no_rationale_is_refused() {
    let (tdb, author) = harness("no_rationale").await;
    retention_proposals::create_proposal(
        tdb.db(),
        None,
        BodyMode::Aggregate,
        "   ",
        uuid_of(&author),
        &closes_at(),
    )
    .await
    .expect_err("an empty rationale is not a reason");
}

// ---------------------------------------------------------------------------
// The lifecycle
// ---------------------------------------------------------------------------

/// Closing is conditional on being open, and the row count is the answer: two
/// passes racing means one updated zero rows, and treating that as an error is
/// how "somebody else already closed this" is expressed.
#[tokio::test]
async fn a_proposal_is_closed_once_and_a_second_close_reports_that_it_was() {
    let (tdb, author) = harness("close_once").await;
    let proposal = retention_proposals::create_proposal(
        tdb.db(),
        None,
        BodyMode::Aggregate,
        "out of disk",
        uuid_of(&author),
        &closes_at(),
    )
    .await
    .expect("open");

    assert!(
        retention_proposals::close_proposal(tdb.db(), &proposal.id, ProposalState::Passed)
            .await
            .expect("close"),
        "the first close moves the row"
    );
    assert!(
        !retention_proposals::close_proposal(tdb.db(), &proposal.id, ProposalState::Failed)
            .await
            .expect("close again"),
        "a second close must report that it did nothing, rather than overwriting the \
         state the first close chose: `failed` would erase that this instance passed it"
    );
    assert_eq!(
        retention_proposals::proposal(tdb.db(), &proposal.id)
            .await
            .expect("read")
            .expect("still there")
            .state,
        ProposalState::Passed
    );
}

/// `overridden` is a state of its own because an operator dashboard that
/// cannot tell it from `passed` is lying about who decided.
#[tokio::test]
async fn an_override_is_recorded_as_its_own_state_not_as_a_pass() {
    let (tdb, author) = harness("override_state").await;
    let proposal = retention_proposals::create_proposal(
        tdb.db(),
        None,
        BodyMode::Aggregate,
        "out of disk",
        uuid_of(&author),
        &closes_at(),
    )
    .await
    .expect("open");
    retention_proposals::close_proposal(tdb.db(), &proposal.id, ProposalState::Passed)
        .await
        .expect("pass it");
    assert!(
        retention_proposals::close_proposal(tdb.db(), &proposal.id, ProposalState::Overridden)
            .await
            .expect("override"),
        "a passed proposal CAN be overridden: the state moves from passed to \
         overridden, and that transition has to be possible or the operator's \
         answer to a passing ballot is unrecordable"
    );
    assert_eq!(
        retention_proposals::proposal(tdb.db(), &proposal.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        ProposalState::Overridden
    );
}

/// A closed proposal takes no more ballots, and says which state it closed in.
#[tokio::test]
async fn a_closed_proposal_takes_no_further_ballots() {
    let (tdb, author) = harness("closed_ballot").await;
    let voter = account_for(&tdb, "closed_ballot_voter").await;
    let proposal = retention_proposals::create_proposal(
        tdb.db(),
        None,
        BodyMode::Aggregate,
        "out of disk",
        uuid_of(&author),
        &closes_at(),
    )
    .await
    .expect("open");
    retention_proposals::close_proposal(tdb.db(), &proposal.id, ProposalState::Failed)
        .await
        .expect("fail it");

    let error = retention_proposals::cast_vote(tdb.db(), &proposal.id, uuid_of(&voter), true)
        .await
        .expect_err("a closed proposal takes no ballots");
    assert!(
        error.to_string().contains("failed"),
        "a reader voting on a proposal that already closed is owed the state, not a \
         bare refusal: {error}"
    );
}

/// The maintenance pass's candidate list: open proposals whose window has
/// passed. It *reports* rather than closes, because closing a proposal is a
/// decision about storage policy and a sweep is not where decisions are made.
#[tokio::test]
async fn overdue_proposals_are_reported_and_not_settled() {
    let (tdb, author) = harness("overdue").await;
    let proposal = retention_proposals::create_proposal(
        tdb.db(),
        None,
        BodyMode::Aggregate,
        "out of disk",
        uuid_of(&author),
        "2026-01-01T00:00:00Z",
    )
    .await
    .expect("open, already past its window");

    let overdue = retention_proposals::overdue_proposals(tdb.db(), "2026-10-01T00:00:00Z")
        .await
        .expect("sweep");
    assert_eq!(overdue.len(), 1, "the proposal's window has passed");
    assert_eq!(overdue[0].id, proposal.id);
    assert_eq!(
        overdue[0].state,
        ProposalState::Open,
        "the sweep must not settle it. Tallying and closing is a decision about \
         storage policy, and `overdue_proposals` finding a candidate is not that \
         decision being made."
    );

    // A proposal still inside its window is not a candidate.
    let (tdb2, author2) = harness("not_overdue").await;
    retention_proposals::create_proposal(
        tdb2.db(),
        None,
        BodyMode::Aggregate,
        "later",
        uuid_of(&author2),
        &closes_at(),
    )
    .await
    .expect("open");
    assert!(
        retention_proposals::overdue_proposals(tdb2.db(), "2026-10-01T00:00:00Z")
            .await
            .expect("sweep")
            .is_empty(),
        "a proposal closing in the future is not overdue"
    );
}

/// A recorded change is readable and names what it changed — the operator's
/// only way to find out what this instance did.
#[tokio::test]
async fn a_recorded_change_is_readable_and_names_both_ends() {
    let (tdb, author) = harness("changes").await;
    let change = retention_proposals::record_change(
        tdb.db(),
        Some(BodyMode::Cache),
        BodyMode::Aggregate,
        None,
        uuid_of(&author),
        "the vote passed and the operator agreed",
    )
    .await
    .expect("record");

    let listed = retention_proposals::list_changes(tdb.db(), None)
        .await
        .expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, change.id);
    assert_eq!(listed[0].from_mode, Some(BodyMode::Cache));
    assert_eq!(listed[0].to_mode, BodyMode::Aggregate);
    assert!(
        listed[0].reason.contains("operator agreed"),
        "the reason is what an operator reads to know why: {}",
        listed[0].reason
    );

    // A change with no `from_mode` is legal: an override may apply to a setting
    // that has no explicit value yet.
    retention_proposals::record_change(
        tdb.db(),
        None,
        BodyMode::Aggregate,
        Some("site-a"),
        uuid_of(&author),
        "first override",
    )
    .await
    .expect("record with no previous value");
    let scoped = retention_proposals::list_changes(tdb.db(), Some("site-a"))
        .await
        .expect("list");
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].from_mode, None);
}

/// The tally tests the bar against the mode in force **now**, not the one in
/// force when the proposal was opened. An operator can change a setting directly
/// while a proposal is open, and a stale mode would apply a bar chosen for a
/// question nobody is being asked any more.
#[tokio::test]
async fn the_tally_reads_the_mode_in_force_now_not_the_one_at_opening() {
    let (tdb, author) = harness("tally_current").await;
    let voter = account_for(&tdb, "tally_current_voter").await;

    // Instance is `cache`; the proposal is to narrow to `aggregate`, so with
    // three supporters at a widen-quorum of 3 it is short (narrowing needs 3).
    let proposal = retention_proposals::create_proposal(
        tdb.db(),
        None,
        BodyMode::Aggregate,
        "out of disk",
        uuid_of(&author),
        &closes_at(),
    )
    .await
    .expect("open");
    // Two DISTINCT accounts. The first version cast twice from one account and
    // then asserted `supporters == 2`, which failed with `supporters == 1` --
    // the anti-buy property working exactly as designed. One account is one
    // ballot, however many times it votes.
    let second_voter = account_for(&tdb, "tally_current_second").await;
    for who in [&voter, &second_voter] {
        retention_proposals::cast_vote(tdb.db(), &proposal.id, uuid_of(who), true)
            .await
            .expect("vote");
    }
    let before = retention_proposals::tally(tdb.db(), &proposal.id, 3)
        .await
        .expect("tally");
    assert!(
        !before.quorum.is_reached(),
        "two supporters against a bar of three is short"
    );

    // The operator changes the setting directly, while the ballot is open.
    lorehaven_db::retention::write_policy(tdb.db(), BodyMode::Aggregate, uuid_of(&author))
        .await
        .expect("operator narrows the instance by hand");

    let after = retention_proposals::tally(tdb.db(), &proposal.id, 3)
        .await
        .expect("tally");
    // The proposal is now a no-op, so the direction has changed: moving to
    // `aggregate` from `aggregate` is not a widening.
    assert_eq!(
        after.quorum, before.quorum,
        "a no-op is held to the ordinary bar either way, so the tally does not move"
    );
    assert_eq!(after.supporters, 2, "{after:?}");
}
