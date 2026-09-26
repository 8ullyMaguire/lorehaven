//! Un-voting must still be possible after the same-value re-vote became a
//! refresh.
//!
//! `vote_tx!` used to DELETE the row when a vote repeated the same value, so
//! "click up twice to undo" worked. It is now a refresh, which is what the
//! decay rule needs, and it took the un-vote with it. This pins the way back:
//! an explicit `value: 0` withdraws the vote, and nothing else does.
//!
//! The distinction matters because of how a voter retracts a mistake. A
//! double-click now means "I still mean this", not "oops", so a mistaken vote
//! has to be correctable in one deliberate step rather than by clicking again.

use lorehaven_db::directory;
use lorehaven_domain::vote_decay::Decay;
use test_support::TestDb;

fn scratch(name: &str) -> std::path::PathBuf {
    std::env::temp_dir()
        .join("lh-unvote")
        .join(format!("{name}-{}", std::process::id()))
}

async fn entry(tdb: &TestDb, label: &str) -> String {
    let db = tdb.db();
    let eid = test_support::id(&format!("unvote-{label}"));
    let title = format!("Unvote {label}");
    const SQLITE_SQL: &str = "INSERT INTO directory_entries (id, list_id, kind, category, title, url, description, ref_id, tags_json, submitted_by, approved_by, removed_at, score, created_at, updated_at) \
         VALUES (?, 'list-1', 'external', 'tools', ?, '', '', NULL, '[]', 'acct-sub', 'acct-approver', NULL, 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')";
    const PG_SQL: &str = "INSERT INTO directory_entries (id, list_id, kind, category, title, url, description, ref_id, tags_json, submitted_by, approved_by, removed_at, score, created_at, updated_at) \
         VALUES ($1, 'list-1', 'external', 'tools', $2, '', '', NULL, '[]', 'acct-sub', 'acct-approver', NULL, 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')";
    match db.backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query(SQLITE_SQL)
            .bind(&eid)
            .bind(&title)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .map(|_| ())
            .expect("insert entry"),
        lorehaven_db::Backend::Postgres => sqlx::query(PG_SQL)
            .bind(&eid)
            .bind(&title)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .map(|_| ())
            .expect("insert entry"),
    };
    eid
}

async fn rows(tdb: &TestDb, entry_id: &str) -> i64 {
    let sql = match tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            "SELECT CAST(COUNT(*) AS BIGINT) FROM directory_votes WHERE entry_id = ?"
        }
        lorehaven_db::Backend::Postgres => {
            "SELECT CAST(COUNT(*) AS BIGINT) FROM directory_votes WHERE entry_id = $1"
        }
    };
    match tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(sql)
            .bind(entry_id)
            .fetch_one(tdb.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("count"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(sql)
            .bind(entry_id)
            .fetch_one(tdb.db().postgres_pool().expect("postgres"))
            .await
            .expect("count"),
    }
}

#[tokio::test]
async fn value_zero_withdraws_the_vote() {
    let tdb = TestDb::connect_with_dir("unvote_zero", &scratch("zero")).await;
    let eid = entry(&tdb, "zero").await;
    let decay = Decay::default();
    let now = "2026-09-26T12:00:00Z";

    directory::set_vote(tdb.db(), &eid, "acct-a", 1, 1.0, now, &decay)
        .await
        .expect("up");
    assert_eq!(rows(&tdb, &eid).await, 1);

    let (score, live) = directory::set_vote(tdb.db(), &eid, "acct-a", 0, 1.0, now, &decay)
        .await
        .expect("withdraw");

    assert!(!live, "a withdrawn vote is not live");
    assert_eq!(rows(&tdb, &eid).await, 0, "the row is gone");
    assert!(
        score.abs() < 1e-9,
        "an entry with no votes scores zero: {score}"
    );
}

#[tokio::test]
async fn withdrawing_a_vote_that_does_not_exist_is_harmless() {
    let tdb = TestDb::connect_with_dir("unvote_absent", &scratch("absent")).await;
    let eid = entry(&tdb, "absent").await;
    let decay = Decay::default();

    // No vote to withdraw. This must not be an error: a client that renders an
    // un-vote button and double-clicks it would otherwise see a 500.
    let (score, live) = directory::set_vote(
        tdb.db(),
        &eid,
        "acct-a",
        0,
        1.0,
        "2026-09-26T12:00:00Z",
        &decay,
    )
    .await
    .expect("withdrawing nothing is not an error");

    assert!(!live, "there is no vote, so none is live");
    assert_eq!(rows(&tdb, &eid).await, 0);
    assert!(score.abs() < 1e-9, "score {score}");
}

#[tokio::test]
async fn withdrawing_leaves_another_accounts_vote_alone() {
    let tdb = TestDb::connect_with_dir("unvote_others", &scratch("others")).await;
    let eid = entry(&tdb, "others").await;
    let decay = Decay::default();
    let now = "2026-09-26T12:00:00Z";

    directory::set_vote(tdb.db(), &eid, "acct-a", 1, 1.0, now, &decay)
        .await
        .expect("a up");
    directory::set_vote(tdb.db(), &eid, "acct-b", 1, 1.0, now, &decay)
        .await
        .expect("b up");

    let (score, _) = directory::set_vote(tdb.db(), &eid, "acct-a", 0, 1.0, now, &decay)
        .await
        .expect("a withdraws");

    assert_eq!(
        rows(&tdb, &eid).await,
        1,
        "one account withdrawing must not remove the other's vote"
    );
    assert!(
        (score - 1.0).abs() < 1e-6,
        "the remaining vote is still worth its full weight: {score}"
    );
}

#[tokio::test]
async fn a_withdrawn_vote_can_be_cast_again() {
    let tdb = TestDb::connect_with_dir("unvote_recast", &scratch("recast")).await;
    let eid = entry(&tdb, "recast").await;
    let decay = Decay::default();
    let now = "2026-09-26T12:00:00Z";

    directory::set_vote(tdb.db(), &eid, "acct-a", 1, 1.0, now, &decay)
        .await
        .expect("up");
    directory::set_vote(tdb.db(), &eid, "acct-a", 0, 1.0, now, &decay)
        .await
        .expect("withdraw");
    let (score, live) = directory::set_vote(tdb.db(), &eid, "acct-a", -1, 1.0, now, &decay)
        .await
        .expect("down");

    assert!(live);
    assert_eq!(rows(&tdb, &eid).await, 1, "one row, the new direction");
    assert!(
        (score + 1.0).abs() < 1e-6,
        "the new vote is a down-vote: {score}"
    );
}
