//! The list must rank by the score it displays.
//!
//! `list_entries` orders by `e.score` and returns `e.score`. Both are the
//! denormalised column, which is the *undecayed* sum written at vote time. So
//! the list ranks on a snapshot and shows a number that stops matching the
//! moment any vote ages — and an entry whose votes all expired keeps its old
//! rank forever, which is the exact opposite of what decay is for.
//!
//! Both the `ORDER BY` and the projected `score` have to be the computed one.

use lorehaven_db::directory::{self, DirectoryEntryFilter, DirectorySort};
use lorehaven_domain::vote_decay::Decay;
use test_support::TestDb;

fn scratch(name: &str) -> std::path::PathBuf {
    std::env::temp_dir()
        .join("lh-votelist")
        .join(format!("{name}-{}", std::process::id()))
}

async fn entry(tdb: &TestDb, label: &str) -> String {
    let db = tdb.db();
    let eid = test_support::id(&format!("votelist-{label}"));
    let title = format!("VoteList {label}");
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

/// Give an entry `n` up-votes, all aged `age_days`.
///
/// Each account is namespaced by entry id, so two entries in the same test
/// can both have twenty-five votes without colliding on the
/// `(entry_id, account_id)` primary key.
async fn votes(tdb: &TestDb, entry_id: &str, n: usize, age_days: f64) {
    for i in 0..n {
        let a = format!("voter-{entry_id}-{i}");
        let secs = age_days * 86_400.0;
        let sqlite_sql = format!(
            "INSERT INTO directory_votes (entry_id, account_id, vote_value, base_weight, voted_at) \
             VALUES (?, ?, 1, 1.0, datetime('now', '-{secs:.3} seconds'))"
        );
        match tdb.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query(&sqlite_sql)
                .bind(entry_id)
                .bind(&a)
                .execute(tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .map(|_| ())
                .expect("vote"),
            lorehaven_db::Backend::Postgres => sqlx::query("INSERT INTO directory_votes (entry_id, account_id, vote_value, base_weight, voted_at) VALUES ($1, $2, 1, 1.0, CAST(NOW() - ($3::double precision * INTERVAL '1 second') AS TEXT))")
                .bind(entry_id)
                .bind(&a)
                .bind(secs)
                .execute(tdb.db().postgres_pool().expect("postgres"))
                .await
                .map(|_| ())
                .expect("vote"),
        }
    }
}

/// The `id` of every entry, in the order the list returned them.
async fn listed_ids(tdb: &TestDb, decay: &Decay) -> Vec<String> {
    let filter = DirectoryEntryFilter {
        sort: DirectorySort::Top,
        limit: 50,
        ..Default::default()
    };
    directory::list_entries_with_decay(tdb.db(), &filter, decay)
        .await
        .expect("list")
        .into_iter()
        .map(|e| e.id)
        .collect()
}

#[tokio::test]
async fn the_list_ranks_a_stale_entry_below_a_fresh_one() {
    let tdb = TestDb::connect_with_dir("votelist_rank", &scratch("rank")).await;
    let decay = Decay::default();

    // Both entries have 25 votes, so both are over `min_votes` and both decay.
    // One's votes are 200 days old and worth zero; the other's are fresh.
    let stale = entry(&tdb, "stale").await;
    let fresh = entry(&tdb, "fresh").await;
    votes(&tdb, &stale, 25, 200.0).await;
    votes(&tdb, &fresh, 25, 0.0).await;

    // Both have the same denormalised score, because the column is written
    // once at vote time and does not age.
    let ids = listed_ids(&tdb, &decay).await;
    let stale_at = ids.iter().position(|i| *i == stale).expect("stale listed");
    let fresh_at = ids.iter().position(|i| *i == fresh).expect("fresh listed");
    assert!(
        fresh_at < stale_at,
        "the entry with live votes ranked below the one with dead votes \
         (fresh at {fresh_at}, stale at {stale_at})"
    );
}

#[tokio::test]
async fn the_score_the_list_returns_is_the_decayed_one() {
    let tdb = TestDb::connect_with_dir("votelist_score", &scratch("score")).await;
    let decay = Decay::default();
    let eid = entry(&tdb, "shown").await;
    // Over the threshold, with every vote long expired: the displayed score
    // must be zero, not the 25 that was stored.
    votes(&tdb, &eid, 25, 200.0).await;

    let filter = DirectoryEntryFilter {
        sort: DirectorySort::Top,
        limit: 50,
        ..Default::default()
    };
    let rows = directory::list_entries_with_decay(tdb.db(), &filter, &decay)
        .await
        .expect("list");
    let row = rows.iter().find(|e| e.id == eid).expect("entry listed");
    assert!(
        row.score.abs() < 1e-6,
        "the list showed score {} for an entry whose 25 votes are all worth zero",
        row.score
    );
}

#[tokio::test]
async fn a_low_count_entry_keeps_its_full_score_in_the_list() {
    let tdb = TestDb::connect_with_dir("votelist_low", &scratch("low")).await;
    let decay = Decay::default();
    let eid = entry(&tdb, "low").await;
    // Under `min_votes`, and ancient. The score must be its full 3.0, because
    // the exemption is the whole reason a new submission can be found at all.
    votes(&tdb, &eid, 3, 200.0).await;

    let filter = DirectoryEntryFilter {
        sort: DirectorySort::Top,
        limit: 50,
        ..Default::default()
    };
    let rows = directory::list_entries_with_decay(tdb.db(), &filter, &decay)
        .await
        .expect("list");
    let row = rows.iter().find(|e| e.id == eid).expect("entry listed");
    assert!(
        (row.score - 3.0).abs() < 1e-5,
        "a three-vote entry scored {} in the list, not its full 3.0",
        row.score
    );
}
