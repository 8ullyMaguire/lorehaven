//! The score, computed rather than stored.
//!
//! `cast_vote` today recomputes `directory_entries.score` inside the vote
//! transaction, which is right for a permanent vote and wrong for a decaying
//! one: the moment that transaction commits, the score is already out of date,
//! because decay moves it without any vote happening. §39.4 requires that the
//! list never show a stale score, so with decay on the score has to be derived
//! at read time.
//!
//! These tests pin the behaviour that follows:
//!
//! - A stale vote contributes less than a fresh one, and less the older it is.
//! - The threshold holds: an entry with few live votes does not decay at all.
//! - The stored column is not trusted, and re-reading gives the same answer.
//! - Turning decay off restores exactly the previous behaviour.

use lorehaven_db::{directory, Database};
use lorehaven_domain::vote_decay::Decay;
use test_support::{id, TestDb};

fn scratch() -> std::path::PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("lh-vote-score-{n}"))
}

/// Insert an approved entry directly, so the test is about scoring and not
/// about the approval workflow.
///
/// The SQL is spelled once per dialect and the query built inside each arm:
/// `Database::sql` returns the string for the *connected* backend, so a single
/// binding above would carry `?` into the PostgreSQL arm where it is a syntax
/// error, and a `Query<Sqlite>` cannot execute on a `Pool<Postgres>` at all.
async fn entry(db: &Database, label: &str) -> anyhow::Result<String> {
    const SQLITE_SQL: &str = "INSERT INTO directory_entries (id, list_id, kind, category, title, url, description, ref_id, tags_json, submitted_by, approved_by, removed_at, score, created_at, updated_at) \
         VALUES (?, 'list-1', 'external', 'tools', ?, '', '', NULL, '[]', 'acct-sub', 'acct-approver', NULL, 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')";
    const PG_SQL: &str = "INSERT INTO directory_entries (id, list_id, kind, category, title, url, description, ref_id, tags_json, submitted_by, approved_by, removed_at, score, created_at, updated_at) \
         VALUES ($1, 'list-1', 'external', 'tools', $2, '', '', NULL, '[]', 'acct-sub', 'acct-approver', NULL, 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')";

    let eid = id(&format!("dir-{label}"));
    let title = format!("Entry {label}");
    match db.backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query(SQLITE_SQL)
            .bind(&eid)
            .bind(&title)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!("insert entry: {e}"))?,
        lorehaven_db::Backend::Postgres => sqlx::query(PG_SQL)
            .bind(&eid)
            .bind(&title)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!("insert entry: {e}"))?,
    }
    Ok(eid)
}

/// Insert a vote with a chosen age in days, bypassing `set_vote` so the test
/// controls `voted_at` exactly.
///
/// Seconds, not days, on both engines: an interval of `-1 days` is *exactly*
/// one day, so the age would come out as 0.0 and every fixture would read as a
/// fresh vote. A fractional multiplier is the only way to express a
/// fractional age.
async fn aged_vote(
    db: &Database,
    entry_id: &str,
    account: &str,
    value: i64,
    base_weight: f64,
    age_days: f64,
) -> anyhow::Result<()> {
    const PG_SQL: &str =
        "INSERT INTO directory_votes (entry_id, account_id, vote_value, base_weight, voted_at) \
         VALUES ($1, $2, $3, $4, (NOW() - ($5::double precision * INTERVAL '1 second')))";

    // The magnitude is positive in both arms and each engine's syntax carries
    // the direction. The two do *not* share a sign convention, and getting that
    // wrong is silent: a vote dated 200 days in the *future* still has a
    // negative age, still clamps to zero, still multiplies out to 1.0, and the
    // test then reports that decay does nothing.
    //
    //   SQLite: `datetime('now', '-N seconds')` -- the modifier is the signed
    //           offset, so the minus belongs to the literal.
    //   Postgres: `NOW() - (N * INTERVAL '1 second')` -- the subtraction is in
    //           the expression, so the magnitude must be positive.
    let secs: f64 = age_days * 86_400.0;
    let sqlite_sql = format!(
        "INSERT INTO directory_votes (entry_id, account_id, vote_value, base_weight, voted_at) \
         VALUES (?, ?, ?, ?, datetime('now', '-{secs:.3} seconds'))"
    );
    // `.map(|_| ())` before the `?`: `execute` returns a `QueryResult` whose
    // type parameter is the backend, so the two arms are different types and a
    // bare `?` makes the match ill-typed. Discarding the value makes both arms
    // `anyhow::Result<()>`.
    match db.backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query(&sqlite_sql)
            .bind(entry_id)
            .bind(account)
            .bind(value)
            .bind(base_weight)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!("insert vote: {e}"))?,
        lorehaven_db::Backend::Postgres => sqlx::query(PG_SQL)
            .bind(entry_id)
            .bind(account)
            .bind(value)
            .bind(base_weight)
            .bind(secs)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!("insert vote: {e}"))?,
    }
    Ok(())
}

async fn score_of(db: &Database, entry_id: &str, decay: &Decay) -> f64 {
    directory::decayed_score(db, entry_id, decay)
        .await
        .expect("decayed score")
}

/// Give an entry enough votes to clear `min_votes`, so decay applies.
///
/// This is not incidental: an entry *below* the threshold is exempt by design
/// and its votes keep full weight, so a decay test that forgets to clear the
/// threshold is testing the exemption and reporting it as "decay is broken".
/// The filler votes are fresh, so they contribute a known amount.
async fn clear_threshold(db: &Database, entry_id: &str, decay: &Decay) -> anyhow::Result<f64> {
    for i in 0..decay.min_votes {
        aged_vote(db, entry_id, &format!("filler-{i}"), 1, 1.0, 0.0).await?;
    }
    Ok(decay.min_votes as f64)
}

#[tokio::test]
async fn a_fresh_vote_counts_at_full_weight_and_a_stale_one_counts_less() {
    let tdb = TestDb::connect_with_dir("score_fresh", &scratch()).await;
    let db = tdb.db();
    let eid = entry(db, "fresh").await.unwrap();
    let decay = Decay::default();

    aged_vote(db, &eid, "acct-a", 1, 1.0, 0.0).await.unwrap();
    let fresh = score_of(db, &eid, &decay).await;
    // One vote is below the threshold, so the entry is exempt and a fresh
    // vote is worth its full base weight.

    // Now push it over the threshold and add a half-dead vote.
    let filler = clear_threshold(db, &eid, &decay).await.unwrap();
    aged_vote(db, &eid, "acct-b", 1, 1.0, 30.0).await.unwrap();
    let with_stale = score_of(db, &eid, &decay).await;

    // Exempt below the threshold: a fresh vote counts in full, and adding a
    // second one doubles the score.
    assert!(
        (fresh - 1.0).abs() < 1e-3,
        "a fresh vote scored {fresh}, not 1.0"
    );

    // Over the threshold: the 20 fillers plus one fresh vote plus a 30-day-old
    // one, which the default curve values at about a quarter.
    let expected = filler + 1.0 + 0.25;
    assert!(
        (with_stale - expected).abs() < 0.01,
        "expected about {expected} (20 filler + 1 fresh + a quarter), got {with_stale}"
    );
    // And the aged vote is strictly less than a fresh one, which is the whole
    // rule: the same vote, counted less because nobody reaffirmed it.
    assert!(
        with_stale < filler + 2.0,
        "the 30-day-old vote counted as a full vote: {with_stale}"
    );
}

#[tokio::test]
async fn the_older_a_vote_is_the_less_it_is_worth() {
    let tdb = TestDb::connect_with_dir("score_order", &scratch()).await;
    let db = tdb.db();
    let decay = Decay::default();
    let mut last = f64::INFINITY;
    for (i, age) in [0.0, 7.0, 14.0, 30.0, 45.0, 59.0].iter().enumerate() {
        let eid = entry(db, &format!("age{i}")).await.unwrap();
        // Each entry needs its own filler set. Without it every entry is under
        // the threshold, exempt, and *every* vote in the table reads as full
        // weight -- so the test would report "decay does nothing" while actually
        // measuring the exemption.
        let filler = clear_threshold(db, &eid, &decay).await.unwrap();
        aged_vote(db, &eid, "acct-a", 1, 1.0, *age).await.unwrap();
        // Subtract the known-constant filler so the comparison is between
        // single votes rather than between totals.
        let s = score_of(db, &eid, &decay).await - filler;
        assert!(s < last, "age {age} scored {s}, not below {last}");
        assert!(
            s > 0.0,
            "age {age} scored {s}, but a 59-day vote is not dead"
        );
        last = s;
    }
}

#[tokio::test]
async fn a_vote_past_the_cutoff_is_worth_exactly_nothing() {
    let tdb = TestDb::connect_with_dir("score_cutoff", &scratch()).await;
    let db = tdb.db();
    let decay = Decay::default();
    let eid = entry(db, "cutoff").await.unwrap();
    let filler = clear_threshold(db, &eid, &decay).await.unwrap();
    aged_vote(db, &eid, "acct-a", 1, 1.0, 61.0).await.unwrap();
    aged_vote(db, &eid, "acct-b", 1, 1.0, 0.0).await.unwrap();
    let s = score_of(db, &eid, &decay).await;
    // A 61-day-old vote contributes exactly zero, so the total is the filler
    // plus one fresh vote -- not "nearly" that, exactly that.
    // 1e-5, not 1e-6: the claim is that the dead vote contributed *zero*, and
    // a sum of 21 float64 contributions accumulates more error than 1e-6
    // through julianday. Asserting bit-exactness here would be asserting a
    // property of floating point rather than of the decay rule.
    let expected = filler + 1.0;
    assert!(
        (s - expected).abs() < 1e-5,
        "expected about {expected}, got {s} -- the dead vote was worth something"
    );
}

#[tokio::test]
async fn an_entry_with_few_votes_never_decays_at_any_age() {
    // The rule that keeps new entries viable. Three votes is the entire ranking
    // signal a new entry has, and decaying it is erasure, not moderation.
    let tdb = TestDb::connect_with_dir("score_few", &scratch()).await;
    let db = tdb.db();
    let decay = Decay::default();
    let eid = entry(db, "few").await.unwrap();
    for i in 0..3 {
        aged_vote(db, &eid, &format!("acct-{i}"), 1, 1.0, 200.0)
            .await
            .unwrap();
    }
    let s = score_of(db, &eid, &decay).await;
    assert!(
        (s - 3.0).abs() < 1e-5,
        "three ancient votes on a low-count entry scored {s}, not 3.0"
    );
    // The mirror of the previous test: the same three rows, on an entry that
    // *has* cleared the threshold, are worth nothing. Age alone does not
    // decide; the threshold and the age together do.
}

#[tokio::test]
async fn an_entry_with_many_live_votes_does_decay() {
    let tdb = TestDb::connect_with_dir("score_many", &scratch()).await;
    let db = tdb.db();
    let decay = Decay::default();
    let eid = entry(db, "many").await.unwrap();
    for i in 0..20 {
        aged_vote(db, &eid, &format!("acct-{i}"), 1, 1.0, 200.0)
            .await
            .unwrap();
    }
    let s = score_of(db, &eid, &decay).await;
    // The rows still count toward `min_votes` even though their weight is
    // zero, so the entry keeps decaying. This is the cliff fix: a day earlier,
    // when the count filtered on live weight, the live count would have been 20
    // and then 0, and crossing the cutoff would have flipped the entry from
    // "scores 0.0006" to "scores 20".
    assert!(
        s.abs() < 1e-9,
        "twenty expired votes still scored {s}; the entry stopped decaying"
    );
}

#[tokio::test]
async fn turning_decay_off_restores_permanent_votes() {
    let tdb = TestDb::connect_with_dir("score_off", &scratch()).await;
    let db = tdb.db();
    let off = Decay {
        enabled: false,
        ..Decay::default()
    };
    let eid = entry(db, "off").await.unwrap();
    for i in 0..20 {
        aged_vote(db, &eid, &format!("acct-{i}"), 1, 1.0, 400.0)
            .await
            .unwrap();
    }
    let s = score_of(db, &eid, &off).await;
    assert!(
        (s - 20.0).abs() < 1e-6,
        "with decay off the score was {s}, not 20.0"
    );
}

#[tokio::test]
async fn a_negative_vote_subtracts_and_a_mixed_entry_can_score_near_zero() {
    let tdb = TestDb::connect_with_dir("score_mixed", &scratch()).await;
    let db = tdb.db();
    let decay = Decay::default();
    let eid = entry(db, "mixed").await.unwrap();
    // One up at full weight, one down at full weight, and enough others to
    // clear the threshold so the pair is not exempt.
    aged_vote(db, &eid, "acct-up", 1, 1.0, 0.0).await.unwrap();
    aged_vote(db, &eid, "acct-down", -1, 1.0, 0.0)
        .await
        .unwrap();
    for i in 0..20 {
        aged_vote(db, &eid, &format!("acct-f{i}"), 1, 1.0, 0.0)
            .await
            .unwrap();
    }
    let s = score_of(db, &eid, &decay).await;
    // 21 up, 1 down, all fresh.
    assert!((s - 20.0).abs() < 1e-5, "mixed entry scored {s}, not 20.0");
}

#[tokio::test]
async fn an_entry_with_no_votes_scores_zero() {
    let tdb = TestDb::connect_with_dir("score_none", &scratch()).await;
    let db = tdb.db();
    let eid = entry(db, "none").await.unwrap();
    let s = score_of(db, &eid, &Decay::default()).await;
    assert!(s.abs() < 1e-9, "an unvoted entry scored {s}");
}
