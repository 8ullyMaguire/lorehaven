//! M6-04: the work-tag assertions carry their provenance (spec §6.4).
//!
//! # What this asserts
//!
//! * **A tag assertion says who made it.** `source` is `author` · `curator` ·
//!   `imported` · `exchanged` · `inferred`, and the distinction is the one
//!   that matters to a reader: a tag the *author* put on their own work is a
//!   statement of fact, and a tag the importer guessed is a guess.
//! * **A tag assertion says what role it plays.** `role` is `primary` ·
//!   `secondary` · `background`, and the primary/secondary split is the
//!   **theme overlap rule** (spec §2.2).
//! * **The confidence is fed by the votes and by nothing else** — never by
//!   gravity, never by a rank, never by a reader count. And **a vote is not
//!   a reader**: the votes are counted, not the voters.
//! * **The old `weight` column is the tag's `prominence`**, and the rename
//!   is a change of vocabulary and not of content: every row keeps its
//!   number.

use darkhold_db::taxonomy::{self, TagAssertion, TagRole, TagSource};
use darkhold_db::Database;

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "darkhold-provenance-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

async fn database(tag: &str) -> (Database, std::path::PathBuf) {
    let dir = scratch_dir(tag);
    let db = match std::env::var("DARKHOLD_TEST_PG_URL") {
        Ok(admin_url) if !admin_url.is_empty() => {
            static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let name = format!(
                "lh_test_prov_{}_{}",
                std::process::id(),
                COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            );
            test_support::empty_postgres(&admin_url, &name)
                .await
                .expect("an empty PostgreSQL database")
        }
        _ => {
            let config = darkhold_db::DatabaseConfig::new(format!(
                "sqlite://{}/darkhold.sqlite?mode=rwc",
                dir.display()
            ));
            Database::connect(&config).await.expect("connect")
        }
    };
    db.migrate().await.expect("migrate");
    (db, dir)
}

/// A work, a tag and three voters, so the votes have something to vote on.
///
/// **Three voters and not three votes from one.** The votes' primary key is
/// `(work, tag, voter)`, so a voter votes once and a second vote from the
/// same voter would be a *change* of vote -- which is what the key says, and
/// what a reader who votes twice is not: two opinions from one reader.
async fn fixture(db: &Database) -> (String, String, Vec<String>) {
    let owner = uuid::Uuid::new_v4().to_string();
    let work = uuid::Uuid::new_v4().to_string();
    let node = uuid::Uuid::new_v4().to_string();
    let mut voters = Vec::new();

    for statement in [
        format!(
            "INSERT INTO accounts (id, email, created_at, updated_at) VALUES ('{owner}', 'prov-{owner}@x.test', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        ),
        format!(
            "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) VALUES ('{owner}', '{owner}', 'prov{owner}', 'Prov', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        ),
        format!(
            "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at) VALUES ('{work}', '{owner}', 'Provenanced', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        ),
        format!(
            "INSERT INTO canonical_entities (id, kind, canonical, norm, created_at, review_status, signal_count, node_state, node_count) VALUES ('{node}', 'tag', 'a-tag', 'a-tag', '2026-01-01T00:00:00Z', 'curated', 0, 'browsable', 10)"
        ),
    ] {
        let result = match db.backend() {
            darkhold_db::Backend::Sqlite => sqlx::query(&statement)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .map(|_| ()),
            darkhold_db::Backend::Postgres => sqlx::query(&statement)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .map(|_| ()),
        };
        result.expect("the statement");
    }

    // **The voters come after their account**, because a pseud's foreign key
    // says so.
    for index in 0..3 {
        let voter = uuid::Uuid::new_v4().to_string();
        let statement = format!(
            "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) VALUES ('{voter}', '{owner}', 'vot{index}{owner}', 'Voter', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        );
        let result = match db.backend() {
            darkhold_db::Backend::Sqlite => sqlx::query(&statement)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .map(|_| ()),
            darkhold_db::Backend::Postgres => sqlx::query(&statement)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .map(|_| ()),
        };
        result.expect("the voter");
        voters.push(voter);
    }

    (work, node, voters)
}

/// The assertion's row.
async fn assertion_row(
    db: &Database,
    work_id: &str,
    node_id: &str,
) -> Option<(i64, String, String, f64)> {
    let sql = match db.backend() {
        darkhold_db::Backend::Sqlite => {
            "SELECT prominence, source, role, confidence FROM work_tags WHERE work_id = ? AND node_id = ?"
        }
        darkhold_db::Backend::Postgres => {
            "SELECT prominence::bigint, source, role, confidence::float8 FROM work_tags \
             WHERE work_id::text = $1 AND node_id = $2"
        }
    };
    let row = match db.backend() {
        darkhold_db::Backend::Sqlite => sqlx::query_as::<_, (i64, String, String, f64)>(sql)
            .bind(work_id)
            .bind(node_id)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("the row"),
        darkhold_db::Backend::Postgres => sqlx::query_as::<_, (i64, String, String, f64)>(sql)
            .bind(work_id)
            .bind(node_id)
            .fetch_optional(db.postgres_pool().expect("postgres"))
            .await
            .expect("the row"),
    };
    row
}

#[tokio::test]
async fn an_assertion_carries_its_provenance_and_its_role() {
    let (db, _dir) = database("provenance").await;
    let (work, node, _voters) = fixture(&db).await;

    taxonomy::tag_work(
        &db,
        &work,
        &node,
        TagAssertion {
            prominence: 7,
            source: TagSource::Author,
            role: TagRole::Secondary,
        },
    )
    .await
    .expect("the assertion");

    let (prominence, source, role, _confidence) =
        assertion_row(&db, &work, &node).await.expect("the row");
    assert_eq!(
        prominence, 7,
        "the prominence is the old weight: {prominence}"
    );
    assert_eq!(source, "author", "**who said it** is recorded: {source}");
    assert_eq!(role, "secondary", "and what role it plays: {role}");
}

#[tokio::test]
async fn the_confidence_is_fed_by_the_votes_and_by_nothing_else() {
    let (db, _dir) = database("confidence").await;
    let (work, node, voters) = fixture(&db).await;

    taxonomy::tag_work(
        &db,
        &work,
        &node,
        TagAssertion {
            prominence: 1,
            source: TagSource::Imported,
            role: TagRole::Primary,
        },
    )
    .await
    .expect("the assertion");

    // **A vote, and a vote against.** The share of the votes that agree is the
    // confidence, and the +1/+1 is the smoothing that keeps one vote from
    // being a certainty.
    for (voter, vote) in voters.iter().zip([1, 1, -1]) {
        let statement = format!(
            "INSERT INTO work_tag_votes (work_id, node_id, voter_pseud_id, vote, created_at, updated_at) VALUES ('{work}', '{node}', '{voter}', {vote}, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        );
        let result = match db.backend() {
            darkhold_db::Backend::Sqlite => sqlx::query(&statement)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .map(|_| ()),
            darkhold_db::Backend::Postgres => sqlx::query(&statement)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .map(|_| ()),
        };
        result.expect("the vote");
    }

    taxonomy::recompute_confidence(&db)
        .await
        .expect("recompute");

    let (_prominence, _source, _role, confidence) =
        assertion_row(&db, &work, &node).await.expect("the row");
    // Two of the three votes agree: (2 + 1) / (3 + 1) = 0.75.
    assert!(
        (confidence - 0.75).abs() < 0.001,
        "**the confidence is the votes' share, smoothed**: {confidence}"
    );
}

#[tokio::test]
async fn an_unvoted_assertion_is_fully_confident() {
    let (db, _dir) = database("unvoted").await;
    let (work, node, _voters) = fixture(&db).await;

    taxonomy::tag_work(&db, &work, &node, TagAssertion::default())
        .await
        .expect("the assertion");
    taxonomy::recompute_confidence(&db)
        .await
        .expect("recompute");

    let (_prominence, _source, _role, confidence) =
        assertion_row(&db, &work, &node).await.expect("the row");
    assert_eq!(
        confidence, 1.0,
        "**no votes is not low confidence**: the absence of a claim is not a \\
         claim against the tag"
    );
}
