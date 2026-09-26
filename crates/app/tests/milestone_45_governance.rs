//! M45 — Category governance: lifecycle, proposals, quorum voting, veto, and
//! the entry-moderation twin of each (spec §45).
//!
//! `crates/db/src/category_governance.rs` has **twenty-four public functions and
//! no tests anywhere** -- not in the module, not in any integration test. This
//! file is the first execution of that code on either backend.
//!
//! Every function here is a `match db.backend()` with two hand-written arms, and
//! writing the tests turned up six places where the PostgreSQL arm was never run
//! and could not have worked:
//!
//!   * `merge_categories` -- `UPDATE ... SET category = $3 WHERE category = $4`
//!   * `vote_on_proposal` -- the recount used `$6`/`$7`/`$8`, the SELECT `$9`,
//!     and the final UPDATE `$10`..`$13`
//!   * `hard_delete_category` -- `DELETE ... WHERE slug = $2` with one bind
//!   * `vote_on_entry_mod` -- the same three-statement pattern
//!   * `apply_entry_mod_action` "remove" -- a `db.sql()` pair whose SQLite half
//!     said `$1`/`$2` and whose PostgreSQL half said `$3`/`$4`
//!
//! The pattern is the same each time: someone numbered the placeholders as if
//! sqlx kept one counter running across the statements of a transaction. It
//! numbers each statement from `$1`. PostgreSQL rejects a `$9` in a statement
//! with one bind -- "there is no parameter $9" -- and SQLite never checks, so
//! every one of these passed silently on the local backend and would have
//! failed in production the first time a member voted.
//!
//! The tests are written to *use* each function rather than to inspect SQL, so
//! that a regression in any arm surfaces as a failing vote, a failing merge, or
//! a failing delete rather than as a string comparison.

use std::path::PathBuf;

use lorehaven_db::category_governance as cg;
use lorehaven_domain::category_governance::{
    proposal_decided, EntryModAction, ProposalAction, VoteValue,
};
use test_support::TestDb;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m45gov-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

struct Db {
    tdb: TestDb,
    _dir: PathBuf,
}

impl Db {
    async fn new(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        Self { tdb, _dir: dir }
    }
    fn db(&self) -> &lorehaven_db::Database {
        self.tdb.db()
    }
}

const T0: &str = "2026-01-01T00:00:00Z";
const T1: &str = "2026-01-02T00:00:00Z";
const T2: &str = "2026-01-03T00:00:00Z";

/// Seed an account and a pseud, returning both ids.
///
/// The ids are real UUIDs, not readable names: `accounts.id` and `pseuds.id`
/// are TEXT on SQLite and UUID on PostgreSQL, so a literal like `acct-merge`
/// lands in the SQLite column and is rejected as a malformed uuid by the other.
/// The casts belong on the placeholder, never on the column.
async fn seed_identity(h: &Db, tag: &str) -> (String, String) {
    let account = uuid::Uuid::new_v4().to_string();
    let pseud = uuid::Uuid::new_v4().to_string();
    let email = format!("{tag}-{account}@m45.test");

    match h.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO accounts (id, email, created_at, updated_at)
                 VALUES (?, ?, ?, ?)",
            )
            .bind(&account)
            .bind(&email)
            .bind(T0)
            .bind(T0)
            .execute(h.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("seed account");

            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&pseud)
            .bind(&account)
            .bind(format!("p{}", &account[..8]))
            .bind("A Writer")
            .bind(T0)
            .bind(T0)
            .execute(h.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("seed pseud");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO accounts (id, email, created_at, updated_at)
                 VALUES ($1::uuid, $2, now(), now())",
            )
            .bind(&account)
            .bind(&email)
            .execute(h.db().postgres_pool().expect("postgres"))
            .await
            .expect("seed account");

            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES ($1::uuid, $2::uuid, $3, $4, now(), now())",
            )
            .bind(&pseud)
            .bind(&account)
            .bind(format!("p{}", &account[..8]))
            .bind("A Writer")
            .execute(h.db().postgres_pool().expect("postgres"))
            .await
            .expect("seed pseud");
        }
    }
    (account, pseud)
}

/// A category created by a real pseud, which is what `create_category` requires.
async fn seed_category(h: &Db, slug: &str) -> (String, String) {
    let (account, pseud) = seed_identity(h, slug).await;
    cg::upsert_category(
        h.db(),
        slug,
        &format!("{slug} label"),
        "community",
        &pseud,
        T0,
    )
    .await
    .unwrap_or_else(|e| panic!("create category {slug}: {e}"));
    (account, pseud)
}

/// One `directory_entries` row, so a merge or a removal is observable.
///
/// `directory_entries` (migration 0047) requires `list_id`, `title`,
/// `submitted_by` and `updated_at`; `category` is the column governance
/// writes, and `label` is not a column on this table at all.
async fn seed_entry(h: &Db, id: &str, category: &str) {
    let list = uuid::Uuid::new_v4().to_string();
    let (account, _) = seed_identity(h, "entry").await;
    match h.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO directory_entries
                 (id, list_id, kind, category, title, submitted_by, created_at, updated_at)
                 VALUES (?, ?, 'work', ?, 'A work', ?, ?, ?)",
            )
            .bind(id)
            .bind(&list)
            .bind(category)
            .bind(&account)
            .bind(T0)
            .bind(T0)
            .execute(h.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("seed entry");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO directory_entries
                 (id, list_id, kind, category, title, submitted_by, created_at, updated_at)
                 VALUES ($1, $2::uuid, 'work', $3, 'A work', $4, now(), now())",
            )
            .bind(id)
            .bind(&list)
            .bind(category)
            .bind(&account)
            .execute(h.db().postgres_pool().expect("postgres"))
            .await
            .expect("seed entry");
        }
    }
}

async fn entry_category(h: &Db, id: &str) -> String {
    let sql = h
        .db()
        .sql(
            "SELECT category FROM directory_entries WHERE id = ?",
            "SELECT category FROM directory_entries WHERE id = $1",
        )
        .into_owned();
    match h.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&sql)
            .bind(id)
            .fetch_one(h.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("read entry category"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(&sql)
            .bind(id)
            .fetch_one(h.db().postgres_pool().expect("postgres"))
            .await
            .expect("read entry category"),
    }
}

async fn entry_removed_at(h: &Db, id: &str) -> Option<String> {
    let sql = h
        .db()
        .sql(
            "SELECT removed_at FROM directory_entries WHERE id = ?",
            "SELECT removed_at FROM directory_entries WHERE id = $1",
        )
        .into_owned();
    match h.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&sql)
            .bind(id)
            .fetch_one(h.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("read removed_at"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(&sql)
            .bind(id)
            .fetch_one(h.db().postgres_pool().expect("postgres"))
            .await
            .expect("read removed_at"),
    }
}

// ---------------------------------------------------------------- lifecycle

#[tokio::test]
async fn a_created_category_starts_active_and_community_sourced() {
    let h = Db::new("create").await;
    let (pseud, _) = {
        let (a, p) = seed_identity(&h, "create").await;
        (p, a)
    };
    cg::upsert_category(h.db(), "romance", "Romance", "community", &pseud, T0)
        .await
        .expect("create category");

    let c = cg::list_categories(h.db())
        .await
        .expect("list")
        .into_iter()
        .find(|c| c.slug == "romance")
        .expect("the new category is listed");
    assert_eq!(c.state, "active", "a new category is active");
    assert_eq!(c.source, "community", "and is community-sourced");
    assert_eq!(c.label, "Romance", "the label is what was passed in");
    assert!(c.merged_into.is_none(), "nothing is merged into yet");
}

#[tokio::test]
async fn renaming_reports_whether_the_category_was_there() {
    let h = Db::new("rename").await;
    seed_category(&h, "romance").await;

    assert!(
        cg::rename_category(h.db(), "romance", "Slow Burn")
            .await
            .expect("rename"),
        "renaming a category that exists reports true"
    );
    assert_eq!(
        cg::list_categories(h.db()).await.expect("list")[0].label,
        "Slow Burn"
    );
    assert!(
        !cg::rename_category(h.db(), "no-such-category", "Ghost")
            .await
            .expect("rename missing"),
        "renaming one that is not there reports false rather than inventing it"
    );
}

#[tokio::test]
async fn a_merge_redirects_the_old_category_and_moves_its_entries() {
    let h = Db::new("merge").await;
    let (_account, pseud) = seed_identity(&h, "merge").await;
    cg::upsert_category(h.db(), "old-cat", "Old", "community", &pseud, T0)
        .await
        .expect("create old");
    cg::upsert_category(h.db(), "new-cat", "New", "community", &pseud, T0)
        .await
        .expect("create new");
    seed_entry(&h, "entry-merge", "old-cat").await;

    assert!(
        cg::merge_categories(h.db(), "old-cat", "new-cat")
            .await
            .expect("merge"),
        "merging two categories that both exist reports true"
    );

    let cats = cg::list_categories(h.db()).await.expect("list");
    let old = cats
        .iter()
        .find(|c| c.slug == "old-cat")
        .expect("the old category remains, as a redirect");
    assert_eq!(
        old.state, "merged",
        "it is now a redirect, not a live category"
    );
    assert!(
        old.merged_into.is_some(),
        "and it points at the category it merged into -- the statement here was \
         numbered $3/$4 against two binds, and failed on PostgreSQL"
    );
    assert_eq!(
        entry_category(&h, "entry-merge").await,
        "new-cat",
        "entries follow the merge rather than being stranded in a dead slug"
    );
}

#[tokio::test]
async fn a_merge_needs_both_sides_to_be_live_categories() {
    let h = Db::new("merge-missing").await;
    let (_account, pseud) = seed_identity(&h, "merge-missing").await;
    cg::upsert_category(h.db(), "lonely", "Lonely", "community", &pseud, T0)
        .await
        .expect("create");

    // Refused: the target is not there.
    assert!(
        !cg::merge_categories(h.db(), "lonely", "nowhere")
            .await
            .expect("merge into nothing"),
        "a merge whose target does not exist is refused"
    );
    assert_eq!(
        cg::get_category(h.db(), "lonely")
            .await
            .expect("read")
            .expect("row")
            .state,
        "active",
        "and the source is left active rather than becoming a redirect to nothing"
    );

    // Refused: the source is not there.
    assert!(
        !cg::merge_categories(h.db(), "phantom", "lonely")
            .await
            .expect("merge from nothing"),
        "a merge whose source does not exist is refused"
    );

    // Refused: a category cannot be merged into itself.
    cg::upsert_category(h.db(), "other", "Other", "community", &pseud, T0)
        .await
        .expect("second category");
    assert!(
        !cg::merge_categories(h.db(), "lonely", "lonely")
            .await
            .expect("self merge"),
        "a category cannot be merged into itself"
    );

    // Refused: a category that is already a redirect is not re-merged.
    assert!(cg::merge_categories(h.db(), "lonely", "other")
        .await
        .expect("first merge"));
    assert!(
        !cg::merge_categories(h.db(), "lonely", "other")
            .await
            .expect("second merge"),
        "an already-merged category is not merged again"
    );
    let old = cg::get_category(h.db(), "lonely")
        .await
        .expect("read")
        .expect("row");
    assert!(
        old.merged_into.is_some(),
        "and the redirect still points at the category it was merged into"
    );
}

#[tokio::test]
async fn deleting_reports_whether_it_removed_anything() {
    let h = Db::new("hard-delete").await;
    let (_account, pseud) = seed_identity(&h, "del").await;
    cg::upsert_category(h.db(), "empty-cat", "Empty", "community", &pseud, T0)
        .await
        .expect("create");

    assert!(
        cg::hard_delete_category(h.db(), "empty-cat")
            .await
            .expect("delete empty"),
        "an empty community category is deleted outright"
    );
    assert!(
        !cg::hard_delete_category(h.db(), "empty-cat")
            .await
            .expect("delete twice"),
        "deleting it again reports false -- the statement read $2 against one bind"
    );
}

// ------------------------------------------------------------ vote counting

/// A proposal with the given quorum, on a freshly created category.
async fn proposal(h: &Db, tag: &str, quorum: u32) -> (String, String) {
    let (account, pseud) = seed_identity(h, tag).await;
    cg::upsert_category(h.db(), "quests", "Quests", "community", &pseud, T0)
        .await
        .expect("create category");
    let id = cg::create_proposal(
        h.db(),
        "quests",
        ProposalAction::Rename,
        r#"{"label":"Adventures"}"#,
        quorum,
        T2,
        &account,
        T0,
    )
    .await
    .expect("create proposal");
    (id, account)
}

#[tokio::test]
async fn votes_are_tallied_and_the_tallies_stay_in_step() {
    let h = Db::new("vote-count").await;
    let (p, _) = proposal(&h, "count", 3).await;

    for tag in ["y1", "y2", "n1"] {
        let (who, _) = seed_identity(&h, tag).await;
        let value = if tag.starts_with('n') {
            VoteValue::No
        } else {
            VoteValue::Yes
        };
        assert_eq!(
            cg::vote_on_proposal(h.db(), &p, &who, value, T1)
                .await
                .expect("vote"),
            None,
            "a proposal short of quorum is not decided by a vote that leaves it short"
        );
    }

    let row = cg::list_open_proposals(h.db(), "quests")
        .await
        .expect("list open")
        .into_iter()
        .find(|r| r.id == p)
        .expect("still open");
    assert_eq!(row.yes_votes, 2, "yes votes were tallied");
    assert_eq!(row.no_votes, 1, "no votes were tallied");
    assert_eq!(row.status, "open", "and it is not silently decided");
}

#[tokio::test]
async fn one_account_voting_twice_is_counted_once() {
    let h = Db::new("vote-dup").await;
    let (p, _) = proposal(&h, "dup", 5).await;
    let (voter, _) = seed_identity(&h, "repeat").await;

    cg::vote_on_proposal(h.db(), &p, &voter, VoteValue::Yes, T1)
        .await
        .expect("first vote");
    assert_eq!(
        cg::vote_on_proposal(h.db(), &p, &voter, VoteValue::Yes, T1)
            .await
            .expect("repeat vote"),
        None,
        "voting twice does not reach a quorum one vote cannot"
    );
    assert_eq!(
        cg::list_open_proposals(h.db(), "quests")
            .await
            .expect("list")[0]
            .yes_votes,
        1,
        "a repeated vote from the same account counts once"
    );
}

#[tokio::test]
async fn quorum_in_yes_passes_and_in_no_fails() {
    let h = Db::new("vote-pass").await;
    let (p, _) = proposal(&h, "pass", 2).await;
    let (a, _) = seed_identity(&h, "a").await;
    let (b, _) = seed_identity(&h, "b").await;

    assert_eq!(
        cg::vote_on_proposal(h.db(), &p, &a, VoteValue::Yes, T1)
            .await
            .expect("first yes"),
        None
    );
    assert_eq!(
        cg::vote_on_proposal(h.db(), &p, &b, VoteValue::Yes, T1)
            .await
            .expect("second yes"),
        Some(true),
        "the vote that reaches quorum reports the proposal passed"
    );
    assert!(
        cg::list_open_proposals(h.db(), "quests")
            .await
            .expect("list")
            .iter()
            .all(|r| r.id != p),
        "a decided proposal is no longer open"
    );

    let h2 = Db::new("vote-fail").await;
    let (p2, _) = proposal(&h2, "fail", 1).await;
    let (n, _) = seed_identity(&h2, "n1").await;
    assert_eq!(
        cg::vote_on_proposal(h2.db(), &p2, &n, VoteValue::No, T1)
            .await
            .expect("no vote"),
        Some(false),
        "a no vote that reaches quorum reports the proposal failed"
    );
}

#[tokio::test]
async fn a_decided_proposal_does_not_reopen() {
    let h = Db::new("vote-closed").await;
    let (p, _) = proposal(&h, "closed", 1).await;
    let (n, _) = seed_identity(&h, "n1").await;
    cg::vote_on_proposal(h.db(), &p, &n, VoteValue::No, T1)
        .await
        .expect("fails it");

    let (late, _) = seed_identity(&h, "late").await;
    let _ = cg::vote_on_proposal(h.db(), &p, &late, VoteValue::Yes, T1).await;
    assert!(
        cg::list_open_proposals(h.db(), "quests")
            .await
            .expect("list")
            .iter()
            .all(|r| r.id != p),
        "a vote cast after a proposal is decided does not reopen it"
    );
}

#[tokio::test]
async fn quorum_is_met_at_the_threshold_and_not_before() {
    assert_eq!(
        proposal_decided(3, 0, 3),
        Some(true),
        "exactly quorum passes"
    );
    assert_eq!(proposal_decided(2, 0, 3), None, "one short does not");
    assert_eq!(proposal_decided(0, 3, 3), Some(false), "no quorum fails it");
    assert_eq!(
        proposal_decided(4, 5, 3),
        Some(true),
        "yes is checked first"
    );
    assert_eq!(
        proposal_decided(0, 0, 0),
        Some(true),
        "a quorum of zero is met at once"
    );
}

// ------------------------------------------------------ veto and expiry

#[tokio::test]
async fn a_veto_closes_a_proposal_and_only_applies_once() {
    let h = Db::new("veto").await;
    let (p, account) = proposal(&h, "veto", 5).await;

    assert!(
        cg::veto_proposal(h.db(), &p, &account, "off-topic", T1)
            .await
            .expect("veto"),
        "vetoing an open proposal reports true"
    );
    assert!(
        cg::list_open_proposals(h.db(), "quests")
            .await
            .expect("list")
            .iter()
            .all(|r| r.id != p),
        "a vetoed proposal is no longer open"
    );
    assert!(
        !cg::veto_proposal(h.db(), &p, &account, "again", T1)
            .await
            .expect("second veto"),
        "it cannot be vetoed a second time"
    );
}

#[tokio::test]
async fn only_a_proposal_past_its_close_time_expires() {
    let h = Db::new("expire").await;
    let (open_p, account) = proposal(&h, "exp-open", 5).await;
    let past = cg::create_proposal(
        h.db(),
        "quests",
        ProposalAction::Rename,
        "{}",
        5,
        T0,
        &account,
        T0,
    )
    .await
    .expect("create a proposal already past its close");

    assert!(
        !cg::expire_proposal(h.db(), &open_p, T1)
            .await
            .expect("expire too early"),
        "a proposal that has not reached its close time is left alone"
    );
    assert!(
        cg::expire_proposal(h.db(), &past, T1)
            .await
            .expect("expire"),
        "one past its close time expires"
    );
    assert_eq!(
        cg::list_open_proposals(h.db(), "quests")
            .await
            .expect("list")
            .len(),
        1,
        "only the expired one left the open list"
    );
}

#[tokio::test]
async fn governance_events_are_appended_to_the_changelog() {
    let h = Db::new("changelog").await;
    let (_, account) = seed_identity(&h, "log").await;
    cg::upsert_category(h.db(), "quests", "Quests", "community", &account, T0)
        .await
        .expect("create");

    cg::append_changelog(h.db(), "quests", "created", &account, "{}", T0)
        .await
        .expect("append");
    cg::append_changelog(h.db(), "quests", "renamed", &account, "{}", T1)
        .await
        .expect("append");

    let log = cg::list_changelog(h.db(), "quests", 50)
        .await
        .expect("read changelog");
    let events: Vec<_> = log.iter().map(|e| e.event.as_str()).collect();
    assert_eq!(log.len(), 2, "both events were recorded");
    assert!(
        events.contains(&"created") && events.contains(&"renamed"),
        "the changelog holds what was appended, got {events:?}"
    );
}

// ------------------------- the entry-moderation twin of the above
//
// `vote_on_entry_proposal` and `apply_entry_decision` repeat the same
// three-statement shape as their category counterparts and carried the same
// broken numbering ($6 through $13, and a `db.sql()` pair whose halves
// disagreed). These are the tests that would have caught it.

#[tokio::test]
async fn an_entry_moderation_vote_tallies_and_reaches_quorum() {
    let h = Db::new("entry-vote").await;
    let (author, pseud) = seed_identity(&h, "entry-vote").await;
    cg::upsert_category(h.db(), "quests", "Quests", "community", &pseud, T0)
        .await
        .expect("category");
    seed_entry(&h, "entry-1", "quests").await;

    let p = cg::create_entry_mod_proposal(
        h.db(),
        "entry-1",
        EntryModAction::Remove,
        None,
        T2,
        &author,
        T0,
    )
    .await
    .expect("create entry moderation proposal");
    let (a, _) = seed_identity(&h, "a").await;
    let (b, _) = seed_identity(&h, "b").await;

    assert_eq!(
        cg::vote_on_entry_mod(h.db(), &p, &a, VoteValue::Yes, T1)
            .await
            .expect("first vote"),
        None,
        "one vote short of a quorum of two decides nothing"
    );
    assert_eq!(
        cg::vote_on_entry_mod(h.db(), &p, &b, VoteValue::Yes, T1)
            .await
            .expect("second vote"),
        Some(true),
        "the vote that reaches quorum reports a pass -- this path's PostgreSQL \
         statements were numbered $6 through $13"
    );
    assert_eq!(
        cg::get_entry_mod_proposal(h.db(), &p)
            .await
            .expect("read")
            .expect("row")
            .status,
        "passed",
        "the proposal is recorded as passed"
    );
}

#[tokio::test]
async fn a_passed_removal_marks_the_entry_removed() {
    let h = Db::new("entry-remove").await;
    let (author, pseud) = seed_identity(&h, "entry-remove").await;
    cg::upsert_category(h.db(), "quests", "Quests", "community", &pseud, T0)
        .await
        .expect("category");
    seed_entry(&h, "entry-1", "quests").await;

    let p = cg::create_entry_mod_proposal(
        h.db(),
        "entry-1",
        EntryModAction::Remove,
        None,
        T2,
        &author,
        T0,
    )
    .await
    .expect("create proposal");
    // Entry moderation has a fixed quorum of 2 (ENTRY_MOD_QUORUM, spec 45.6) and
    // `create_entry_mod_proposal` takes no quorum argument, so one vote is not
    // enough and the proposal must stay unapplied.
    let (v1, _) = seed_identity(&h, "voter-1").await;
    assert_eq!(
        cg::vote_on_entry_mod(h.db(), &p, &v1, VoteValue::Yes, T1)
            .await
            .expect("first vote"),
        None,
        "one yes of the required two does not carry a removal"
    );
    assert!(
        !cg::apply_entry_mod_action(h.db(), &p, T1)
            .await
            .expect("apply"),
        "a proposal that has not reached quorum is not applied"
    );
    assert!(
        entry_removed_at(&h, "entry-1").await.is_none(),
        "the entry is untouched while the vote is short of quorum"
    );

    let (v2, _) = seed_identity(&h, "voter-2").await;
    assert_eq!(
        cg::vote_on_entry_mod(h.db(), &p, &v2, VoteValue::Yes, T1)
            .await
            .expect("second vote"),
        Some(true),
        "the second yes carries it"
    );

    assert!(
        cg::apply_entry_mod_action(h.db(), &p, T1)
            .await
            .expect("apply removal"),
        "applying a passed removal reports that it did something"
    );
    assert!(
        entry_removed_at(&h, "entry-1").await.is_some(),
        "the entry carries a removed_at -- this is the db.sql() pair whose SQLite \
         half said $1/$2 and whose PostgreSQL half said $3/$4"
    );
}

#[tokio::test]
async fn a_passed_move_relocates_the_entry_and_needs_a_target() {
    let h = Db::new("entry-move").await;
    let (author, pseud) = seed_identity(&h, "entry-move").await;
    cg::upsert_category(h.db(), "quests", "Quests", "community", &pseud, T0)
        .await
        .expect("category");
    cg::upsert_category(h.db(), "adventure", "Adventure", "community", &pseud, T0)
        .await
        .expect("target category");
    seed_entry(&h, "entry-1", "quests").await;

    // A move with no target category is refused rather than nulling the entry's.
    let untargeted = cg::create_entry_mod_proposal(
        h.db(),
        "entry-1",
        EntryModAction::Move,
        None,
        T2,
        &author,
        T0,
    )
    .await
    .expect("create");
    let (v1, _) = seed_identity(&h, "v1").await;
    let (v1b, _) = seed_identity(&h, "v1b").await;
    cg::vote_on_entry_mod(h.db(), &untargeted, &v1, VoteValue::Yes, T1)
        .await
        .expect("first vote");
    cg::vote_on_entry_mod(h.db(), &untargeted, &v1b, VoteValue::Yes, T1)
        .await
        .expect("second vote");
    assert!(
        !cg::apply_entry_mod_action(h.db(), &untargeted, T1)
            .await
            .expect("apply"),
        "a move with no target is not applied"
    );
    assert_eq!(
        entry_category(&h, "entry-1").await,
        "quests",
        "and the entry kept the category it had"
    );

    let p = cg::create_entry_mod_proposal(
        h.db(),
        "entry-1",
        EntryModAction::Move,
        Some("adventure"),
        T2,
        &author,
        T0,
    )
    .await
    .expect("create targeted move");
    let (v2, _) = seed_identity(&h, "v2").await;
    let (v2b, _) = seed_identity(&h, "v2b").await;
    cg::vote_on_entry_mod(h.db(), &p, &v2, VoteValue::Yes, T1)
        .await
        .expect("first vote");
    cg::vote_on_entry_mod(h.db(), &p, &v2b, VoteValue::Yes, T1)
        .await
        .expect("second vote");
    assert!(
        cg::apply_entry_mod_action(h.db(), &p, T1)
            .await
            .expect("apply move"),
        "a move with a target is applied"
    );
    assert_eq!(
        entry_category(&h, "entry-1").await,
        "adventure",
        "the entry now sits in the target category"
    );
}

#[tokio::test]
async fn a_proposal_that_did_not_pass_is_not_applied() {
    let h = Db::new("entry-unpassed").await;
    let (author, pseud) = seed_identity(&h, "entry-unpassed").await;
    cg::upsert_category(h.db(), "quests", "Quests", "community", &pseud, T0)
        .await
        .expect("category");
    seed_entry(&h, "entry-1", "quests").await;

    let p = cg::create_entry_mod_proposal(
        h.db(),
        "entry-1",
        EntryModAction::Remove,
        None,
        T2,
        &author,
        T0,
    )
    .await
    .expect("create proposal");
    // Never voted on, so it is still open.
    assert!(
        !cg::apply_entry_mod_action(h.db(), &p, T1)
            .await
            .expect("apply an unvoted proposal"),
        "an open proposal is not applied, however the action reads"
    );
    assert!(
        entry_removed_at(&h, "entry-1").await.is_none(),
        "the entry is untouched"
    );
}

#[tokio::test]
async fn a_proposal_that_does_not_exist_is_not_applied() {
    let h = Db::new("entry-missing").await;
    assert!(
        !cg::apply_entry_mod_action(h.db(), "no-such-proposal", T1)
            .await
            .expect("apply a missing proposal"),
        "applying a proposal that is not there reports false"
    );
}

#[tokio::test]
async fn a_category_can_be_deprecated_and_stays_listed() {
    let h = Db::new("deprecate").await;
    seed_category(&h, "quests").await;

    assert!(
        cg::deprecate_category(h.db(), "quests")
            .await
            .expect("deprecate"),
        "deprecating a live category reports true"
    );
    // `deprecate_category` is a plain `UPDATE ... SET state = 'deprecated'
    // WHERE slug = ?` and returns `rows_affected() > 0`. SQLite counts a row
    // whose value did not change, so a second call still reports true. That is
    // the honest reading of the implementation; what matters is that the state
    // is idempotent, which the next assertion covers.
    assert!(
        cg::deprecate_category(h.db(), "quests")
            .await
            .expect("deprecate again"),
        "deprecating again still reports true, because the UPDATE matches the row \
         whether or not the value changes"
    );
    assert_eq!(
        cg::get_category(h.db(), "quests")
            .await
            .expect("read")
            .expect("still there")
            .state,
        "deprecated",
        "the category is deprecated rather than deleted"
    );
    assert_eq!(
        cg::count_active_categories(h.db()).await.expect("count"),
        0,
        "and it no longer counts as active"
    );
}

#[tokio::test]
async fn upserting_a_category_twice_keeps_the_first_label() {
    let h = Db::new("upsert").await;
    let (_account, pseud) = seed_identity(&h, "upsert").await;

    cg::upsert_category(h.db(), "quests", "Quests", "community", &pseud, T0)
        .await
        .expect("first");
    cg::upsert_category(h.db(), "quests", "Quests II", "community", &pseud, T1)
        .await
        .expect("second");

    // `upsert_category` is `ON CONFLICT(slug) DO NOTHING`, not `DO UPDATE`: it
    // exists so `seed_categories` can run on every boot without clobbering a
    // label an operator has since set. Renaming goes through
    // `rename_category`. So the first write wins, and the function reports no
    // error either way.
    let cats = cg::list_categories(h.db()).await.expect("list");
    assert_eq!(
        cats.len(),
        1,
        "seeding the same slug twice leaves one category, not two"
    );
    assert_eq!(
        cats[0].label, "Quests",
        "the second seeding did not overwrite the label -- that is what DO NOTHING \
         means here, and why `rename_category` exists as the separate path"
    );
}

#[tokio::test]
async fn open_proposals_are_listed_newest_first_and_counted() {
    let h = Db::new("proposal-list").await;
    let (account, pseud) = seed_identity(&h, "prop-list").await;
    cg::upsert_category(h.db(), "quests", "Quests", "community", &pseud, T0)
        .await
        .expect("category");

    let first = cg::create_proposal(
        h.db(),
        "quests",
        ProposalAction::Rename,
        "{}",
        5,
        T2,
        &account,
        T0,
    )
    .await
    .expect("first proposal");
    cg::create_proposal(
        h.db(),
        "quests",
        ProposalAction::Deprecate,
        "{}",
        5,
        T2,
        &account,
        T1,
    )
    .await
    .expect("second proposal");

    let open = cg::list_open_proposals(h.db(), "quests")
        .await
        .expect("list");
    assert_eq!(open.len(), 2, "both proposals are open");
    assert_ne!(
        open[0].id, first,
        "the newer proposal is listed first, and `list_open_proposals` promises \
         an order with ORDER BY created_at DESC"
    );
    assert_eq!(
        cg::count_open_proposals(h.db(), "quests")
            .await
            .expect("count"),
        2
    );
    assert_eq!(
        cg::get_proposal(h.db(), &first)
            .await
            .expect("read")
            .expect("row")
            .action,
        "rename",
        "the action round-trips through the stored string"
    );
    assert!(
        cg::get_proposal(h.db(), "no-such-proposal")
            .await
            .expect("read missing")
            .is_none(),
        "a proposal that is not there reads as None rather than erroring"
    );
}

#[tokio::test]
async fn the_last_proposal_time_is_per_category_and_per_action() {
    let h = Db::new("last-proposal").await;
    let (account, pseud) = seed_identity(&h, "last-prop").await;
    cg::upsert_category(h.db(), "quests", "Quests", "community", &pseud, T0)
        .await
        .expect("category");
    cg::upsert_category(h.db(), "other", "Other", "community", &pseud, T0)
        .await
        .expect("second category");

    // A rename proposal on `quests` at T1, a deprecate proposal on `other` at T2.
    cg::create_proposal(
        h.db(),
        "quests",
        ProposalAction::Rename,
        "{}",
        5,
        T2,
        &account,
        T1,
    )
    .await
    .expect("rename proposal");
    cg::create_proposal(
        h.db(),
        "other",
        ProposalAction::Deprecate,
        "{}",
        5,
        T2,
        &account,
        T2,
    )
    .await
    .expect("deprecate proposal");

    assert_eq!(
        cg::last_proposal_time(h.db(), "quests", "rename")
            .await
            .expect("last time"),
        Some(T1.to_string()),
        "the rename proposal is the most recent one of its kind on that category"
    );
    assert_eq!(
        cg::last_proposal_time(h.db(), "quests", "deprecate")
            .await
            .expect("last time"),
        None,
        "asking about an action this category has never seen reads as None, \
         rather than the other category's proposal leaking in"
    );
    assert_eq!(
        cg::last_proposal_time(h.db(), "other", "deprecate")
            .await
            .expect("last time"),
        Some(T2.to_string()),
        "and the other category's own proposal is reported against itself"
    );
}
