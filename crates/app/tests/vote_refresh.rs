//! Voting again on the same day must refresh the vote, not remove it.
//!
//! `vote_tx!` treats a repeat of the same value as a toggle-off and DELETEs
//! the row. That was correct for permanent votes — clicking up twice should
//! undo the vote — and it is wrong now that a vote decays: "I still think
//! this is good" has to be expressible, and under the old behaviour the only
//! way to say it was to vote down and back up, which writes a down-vote into
//! the history of a vote that never went that way.
//!
//! The decay rule is "voting each day weights slightly more than each week",
//! and that sentence is unimplementable if a re-vote is a delete.

use lorehaven_db::directory;
use lorehaven_domain::vote_decay::Decay;
use test_support::TestDb;

/// A per-test, per-run scratch directory.
///
/// Both halves matter. `connect_with_dir` names the *PostgreSQL* database
/// from the tag and ignores the directory, but the SQLite branch uses a fixed
/// `lorehaven.sqlite` inside the directory with `mode=rwc` — which reuses
/// whatever file is already there. So a fixed directory means a failed run's
/// rows survive into the next one, and the next run fails on a UNIQUE
/// constraint against data it thinks it created. The process id keeps each run
/// isolated from the last.
fn scratch(name: &str) -> std::path::PathBuf {
    std::env::temp_dir()
        .join("lh-revote")
        .join(format!("{name}-{}", std::process::id()))
}

/// An approved entry, straight into the table so the test does not depend on
/// the submission workflow.
async fn entry(tdb: &TestDb, label: &str) -> String {
    let db = tdb.db();
    let eid = test_support::id(&format!("revote-{label}"));
    let title = format!("Revote {label}");
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

/// The number of vote rows on an entry.
async fn rows(tdb: &TestDb, entry_id: &str) -> i64 {
    let db = tdb.db();
    let sql = match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            "SELECT CAST(COUNT(*) AS BIGINT) FROM directory_votes WHERE entry_id = ?"
        }
        lorehaven_db::Backend::Postgres => {
            "SELECT CAST(COUNT(*) AS BIGINT) FROM directory_votes WHERE entry_id = $1"
        }
    };
    match db.backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(sql)
            .bind(entry_id)
            .fetch_one(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("count"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(sql)
            .bind(entry_id)
            .fetch_one(db.postgres_pool().expect("postgres"))
            .await
            .expect("count"),
    }
}

#[tokio::test]
async fn voting_the_same_value_again_refreshes_the_vote_instead_of_removing_it() {
    let tdb = TestDb::connect_with_dir("revote_same", &scratch("same")).await;
    let eid = entry(&tdb, "same").await;
    let decay = Decay::default();
    let now = "2026-09-26T12:00:00Z";

    let (_score, live) = directory::set_vote(tdb.db(), &eid, "acct-a", 1, 1.0, now, &decay)
        .await
        .expect("first vote");
    assert!(live, "the first vote should be live");
    assert_eq!(rows(&tdb, &eid).await, 1, "one row after one vote");

    // The same value again. This is the whole point of the test: it must
    // refresh, not toggle off.
    let (score, live) = directory::set_vote(tdb.db(), &eid, "acct-a", 1, 1.0, now, &decay)
        .await
        .expect("second vote");
    assert!(live, "a repeat of the same vote is still a live vote");
    assert_eq!(
        rows(&tdb, &eid).await,
        1,
        "a repeat vote must not add a second row, and must not delete the first"
    );
    assert!(
        (score - 1.0).abs() < 1e-6,
        "the refreshed entry still scores its one vote: {score}"
    );
}

#[tokio::test]
async fn a_revote_rewrites_the_timestamp_so_the_vote_starts_again_from_full_weight() {
    let tdb = TestDb::connect_with_dir("revote_stamp", &scratch("stamp")).await;
    let eid = entry(&tdb, "stamp").await;
    let decay = Decay::default();

    // A vote from long ago, written directly so the test controls the age.
    let aged_rows = match tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query("INSERT INTO directory_votes (entry_id, account_id, vote_value, base_weight, voted_at) VALUES (?, 'acct-a', 1, 1.0, datetime('now', '-5184000 seconds'))")
                .bind(&eid)
                .execute(tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("aged vote")
                .rows_affected()
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query("INSERT INTO directory_votes (entry_id, account_id, vote_value, base_weight, voted_at) VALUES ($1, 'acct-a', 1, 1.0, CAST(NOW() - (5184000.0::double precision * INTERVAL '1 second') AS TEXT))")
                .bind(&eid)
                .execute(tdb.db().postgres_pool().expect("postgres"))
                .await
                .expect("aged vote")
                .rows_affected()
        }
    };
    assert_eq!(aged_rows, 1);

    // Below the threshold, so the entry is exempt and the score is the base
    // weight either way. What matters is the timestamp, not the score.
    let before = voted_at(&tdb, &eid).await;

    directory::set_vote(
        tdb.db(),
        &eid,
        "acct-a",
        1,
        1.0,
        "2026-09-26T12:00:00Z",
        &decay,
    )
    .await
    .expect("re-vote");

    let after = voted_at(&tdb, &eid).await;
    assert_ne!(
        before, after,
        "the re-vote did not touch voted_at, so the vote never came back to life"
    );
    assert_eq!(
        after, "2026-09-26T12:00:00Z",
        "voted_at was not set to the re-vote's time"
    );
    assert_eq!(rows(&tdb, &eid).await, 1, "the re-vote replaced the row");
}

/// The stored `voted_at` of an entry's single vote.
async fn voted_at(tdb: &TestDb, entry_id: &str) -> String {
    let db = tdb.db();
    let sql = match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            "SELECT CAST(voted_at AS TEXT) FROM directory_votes WHERE entry_id = ?"
        }
        lorehaven_db::Backend::Postgres => {
            "SELECT CAST(voted_at AS TEXT) FROM directory_votes WHERE entry_id = $1"
        }
    };
    match db.backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(sql)
            .bind(entry_id)
            .fetch_one(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("voted_at"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(sql)
            .bind(entry_id)
            .fetch_one(db.postgres_pool().expect("postgres"))
            .await
            .expect("voted_at"),
    }
}

#[tokio::test]
async fn switching_direction_still_flips_in_place() {
    let tdb = TestDb::connect_with_dir("revote_flip", &scratch("flip")).await;
    let eid = entry(&tdb, "flip").await;
    let decay = Decay::default();
    let now = "2026-09-26T12:00:00Z";

    directory::set_vote(tdb.db(), &eid, "acct-a", 1, 1.0, now, &decay)
        .await
        .expect("up");
    let (score, live) = directory::set_vote(tdb.db(), &eid, "acct-a", -1, 1.0, now, &decay)
        .await
        .expect("down");

    assert!(live, "changing direction leaves a live vote");
    assert_eq!(
        rows(&tdb, &eid).await,
        1,
        "a direction change is still one row"
    );
    assert!(
        (score + 1.0).abs() < 1e-6,
        "the vote flipped to down, so the entry scores -1: {score}"
    );
    let v: i64 = match tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(
            "SELECT CAST(vote_value AS BIGINT) FROM directory_votes WHERE entry_id = ?",
        )
        .bind(&eid)
        .fetch_one(tdb.db().sqlite_pool().expect("sqlite"))
        .await
        .expect("vote_value"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(
            "SELECT CAST(vote_value AS BIGINT) FROM directory_votes WHERE entry_id = $1",
        )
        .bind(&eid)
        .fetch_one(tdb.db().postgres_pool().expect("postgres"))
        .await
        .expect("vote_value"),
    };
    assert_eq!(v, -1, "the stored value is the down-vote, not the up-vote");
}

#[tokio::test]
async fn the_score_a_vote_returns_is_the_decayed_one_not_the_stored_column() {
    let tdb = TestDb::connect_with_dir("revote_score", &scratch("score")).await;
    let eid = entry(&tdb, "score").await;
    let decay = Decay::default();

    // Twenty accounts, all long expired, so the entry is over the threshold and
    // every vote is worth zero. Then one fresh vote, which must be the entire
    // score.
    for i in 0..20 {
        let a = format!("stale-{i}");
        match tdb.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query("INSERT INTO directory_votes (entry_id, account_id, vote_value, base_weight, voted_at) VALUES (?, ?, 1, 1.0, datetime('now', '-200 days'))")
                .bind(&eid).bind(&a)
                .execute(tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .map(|_| ())
                .expect("stale"),
            lorehaven_db::Backend::Postgres => sqlx::query("INSERT INTO directory_votes (entry_id, account_id, vote_value, base_weight, voted_at) VALUES ($1, $2, 1, 1.0, CAST(NOW() - (200.0::double precision * INTERVAL '1 day') AS TEXT))")
                .bind(&eid).bind(&a)
                .execute(tdb.db().postgres_pool().expect("postgres"))
                .await
                .map(|_| ())
                .expect("stale"),
        };
    }

    let (score, live) = directory::set_vote(
        tdb.db(),
        &eid,
        "acct-fresh",
        1,
        1.0,
        "2026-09-26T12:00:00Z",
        &decay,
    )
    .await
    .expect("fresh vote");

    assert!(live);
    let expected = directory::decayed_score(tdb.db(), &eid, &decay)
        .await
        .expect("decayed score");
    assert!(
        (score - expected).abs() < 1e-6,
        "set_vote returned {score} but the decayed score is {expected}"
    );
}
