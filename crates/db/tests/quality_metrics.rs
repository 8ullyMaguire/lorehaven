//! M63-03: source-relative ranking over the catalogue, on both engines.
//!
//! The maths is unit-tested in `darkhold-domain`; this file pins the three
//! properties that only exist once a database is involved:
//!
//! 1. **The population a percentile is computed against is the source's own.**
//!    The same story carrying 33,107 likes on `sb` (in a source with more bigger
//!    rows) and 4,179 on `qq` (where it is the top row) must score close, because
//!    raw likes differ by 8x and archive size is exactly the confound being
//!    removed.
//! 2. **The gate is counted from the filter, not from the page.** `ranked` is how
//!    many matching rows carry an endorsement and `total` is how many matched;
//!    both survive a `limit` of one.
//! 3. **An unmeasured row never sits among measured ones.** It is last on the
//!    page and `measured: false` says why.
//!
//! Run on SQLite by default; set `DARKHOLD_TEST_PG_URL` for the PostgreSQL arm.

use std::time::Duration;

use darkhold_db::catalogue::CatalogueQuery;
use darkhold_db::metrics;
use darkhold_db::Database;
use darkhold_domain::quality::Metric;

const T0: &str = "2026-01-01T00:00:00Z";

fn make_config(url: String) -> darkhold_db::DatabaseConfig {
    darkhold_db::DatabaseConfig {
        url,
        max_connections: 2,
        acquire_timeout: Duration::from_secs(5),
        slow_query_warn: Duration::ZERO,
    }
}

async fn connect(tag: &str) -> Database {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "darkhold-quality-{}-{}-{}",
        tag,
        uuid::Uuid::new_v4(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let url = match std::env::var("DARKHOLD_TEST_PG_URL") {
        Ok(admin) => {
            let name = format!(
                "lh_quality_{}_{}",
                std::process::id(),
                uuid::Uuid::new_v4().simple()
            );
            let admin_db = Database::connect(&make_config(admin.clone()))
                .await
                .expect("connect to the admin database");
            sqlx::query(&format!("CREATE DATABASE {name}"))
                .execute(admin_db.postgres_pool().expect("postgres pool"))
                .await
                .expect("create a scratch database");
            admin_db.close().await;
            let (prefix, _) = admin.rsplit_once('/').expect("admin URL ends in a database");
            format!("{prefix}/{name}")
        }
        Err(_) => format!("sqlite://{}/darkhold.sqlite?mode=rwc", dir.display()),
    };
    let db = Database::connect(&make_config(url)).await.expect("connect");
    db.migrate().await.expect("migrate");
    db
}

fn query() -> CatalogueQuery {
    CatalogueQuery {
        source: None,
        min_likes: None,
        q: None,
        sort: None,
        limit: Some(50),
        offset: Some(0),
        updated: None,
        refreshed: None,
        added: None,
        fandom: None,
        ..Default::default()
    }
}

/// One catalogue row: `(source_work_key, source, likes, word_count)`.
type Row = (&'static str, &'static str, i64, i64);

/// Twenty engaged `sb` rows and ten engaged `qq` rows, plus one row with no
/// endorsement at all. The two copies of the same story sit at the same *rank*
/// in their own archive but nowhere near each other in raw likes.
const SEED: &[Row] = &[
    // sb: 20 engaged rows, 1,000 to 20,000 likes, plus the three that lead it.
    // The spread matters: the qq copy below carries 4,179 likes, which is
    // unremarkable on sb and the top of qq — the live situation exactly.
    ("sb-a", "sb", 1_000, 40_000),
    ("sb-b", "sb", 2_000, 40_000),
    ("sb-c", "sb", 3_000, 40_000),
    ("sb-d", "sb", 4_000, 40_000),
    ("sb-e", "sb", 5_000, 40_000),
    ("sb-f", "sb", 6_000, 40_000),
    ("sb-g", "sb", 7_000, 40_000),
    ("sb-h", "sb", 8_000, 40_000),
    ("sb-i", "sb", 9_000, 40_000),
    ("sb-j", "sb", 10_000, 40_000),
    ("sb-k", "sb", 11_000, 40_000),
    ("sb-l", "sb", 12_000, 40_000),
    ("sb-m", "sb", 13_000, 40_000),
    ("sb-n", "sb", 14_000, 40_000),
    ("sb-o", "sb", 15_000, 40_000),
    ("sb-p", "sb", 16_000, 40_000),
    ("sb-q", "sb", 17_000, 40_000),
    ("sb-r", "sb", 18_000, 40_000),
    ("sb-s", "sb", 19_000, 40_000),
    ("sb-t", "sb", 20_000, 40_000),
    ("sb-marvel", "sb", 33_107, 200_000),
    ("sb-second", "sb", 40_000, 200_000),
    ("sb-first", "sb", 45_000, 200_000),
    // qq: 10 engaged rows. `qq-marvel` is the top row, and 4,179 likes is less
    // than a tenth of what the sb copy carries.
    ("qq-a", "qq", 5, 30_000),
    ("qq-b", "qq", 10, 30_000),
    ("qq-c", "qq", 20, 30_000),
    ("qq-d", "qq", 40, 30_000),
    ("qq-e", "qq", 60, 30_000),
    ("qq-f", "qq", 80, 30_000),
    ("qq-g", "qq", 100, 30_000),
    ("qq-h", "qq", 140, 30_000),
    ("qq-i", "qq", 200, 30_000),
    ("qq-marvel", "qq", 4_179, 200_000),
    // No endorsement: the estate import's shape.
    ("sb-silent", "sb", 0, 60_000),
];

async fn insert(db: &Database, account: &str, row: Row) {
    let (key, source, likes, words) = row;
    match db.backend() {
        darkhold_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO library_items
                 (id, account_id, source_key, source_work_key, title, author_text, summary,
                  word_count, status, source_url, source_updated_at, last_synced_at, created_at,
                  updated_at, likes)
                 VALUES ($1::uuid, $2::uuid, $3, $4, $4, 'A', '', $5,
                         'ongoing', 'https://example.invalid/x', $6, $6, $6, $6, $7)",
            )
            .bind(uuid::Uuid::new_v4())
            .bind(uuid::Uuid::parse_str(account).expect("uuid"))
            .bind(source)
            .bind(key)
            .bind(words)
            .bind(T0)
            .bind(likes)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("library item");
        }
        darkhold_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO library_items
                 (id, account_id, source_key, source_work_key, title, author_text, summary,
                  word_count, status, source_url, source_updated_at, last_synced_at, created_at,
                  updated_at, likes)
                 VALUES (?, ?, ?, ?, ?, 'A', '', ?,
                         'ongoing', 'https://example.invalid/x', ?, ?, ?, ?, ?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(account)
            .bind(source)
            .bind(key)
            .bind(key)
            .bind(words)
            .bind(T0)
            .bind(T0)
            .bind(T0)
            .bind(T0)
            .bind(likes)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("library item");
        }
    }
}

async fn seed(db: &Database) -> String {
    let account = uuid::Uuid::new_v4().to_string();
    match db.backend() {
        darkhold_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO accounts (id, email, status, age_state, created_at, updated_at, permission_statement)
                 VALUES ($1::uuid, 'quality@example.invalid', 'active', 'adult', $2, $2, 'minimal')",
            )
            .bind(uuid::Uuid::parse_str(&account).expect("uuid"))
            .bind(T0)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("account");
        }
        darkhold_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO accounts (id, email, status, age_state, created_at, updated_at, permission_statement)
                 VALUES (?, 'quality@example.invalid', 'active', 'adult', ?, ?, 'minimal')",
            )
            .bind(&account)
            .bind(T0)
            .bind(T0)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("account");
        }
    }
    for row in SEED {
        insert(db, &account, *row).await;
    }
    account
}

fn keys(page: &metrics::RankedPage) -> Vec<String> {
    page.items
        .iter()
        .map(|item| item.row.source_work_key.clone())
        .collect()
}

/// The story that motivates the module: the same work, in two archives, ranked
/// third-from-top and top respectively. Percentiles land close together even
/// though the raw counts are 8x apart.
#[tokio::test]
async fn the_same_story_scores_close_in_two_archives_of_different_size() {
    let db = connect("bias").await;
    seed(&db).await;

    let page = metrics::ranked(&db, &query(), Metric::SourcePct, 50, 0)
        .await
        .expect("ranked");
    let score = |key: &str| {
        page.items
            .iter()
            .find(|item| item.row.source_work_key == key)
            .unwrap_or_else(|| panic!("{key} is on the page"))
            .score
    };
    let sb = score("sb-marvel");
    let qq = score("qq-marvel");
    assert!(
        (sb - qq).abs() < 0.15,
        "8x raw likes, but the percentile gap should be small: sb {sb}, qq {qq}"
    );
    assert!(sb > 0.6 && qq > 0.6, "both are top-of-archive: {sb} / {qq}");
    // And the raw ranking does the opposite, which is what this metric is for.
    let raw = metrics::ranked(&db, &query(), Metric::Likes, 50, 0)
        .await
        .expect("ranked");
    let order = keys(&raw);
    assert_eq!(order[0], "sb-first", "raw likes lead with the largest archive");
    let sb_at = order.iter().position(|k| k == "sb-marvel").expect("present");
    let qq_at = order.iter().position(|k| k == "qq-marvel").expect("present");
    assert!(
        qq_at > sb_at + 15,
        "raw likes bury the qq copy at {qq_at} against sb's {sb_at}"
    );
}

/// The gate: engaged rows out of everything the filter matched, and it does not
/// move when the page shrinks.
#[tokio::test]
async fn the_gate_counts_the_filter_not_the_page() {
    let db = connect("gate").await;
    seed(&db).await;

    let full = metrics::ranked(&db, &query(), Metric::Quality, 50, 0)
        .await
        .expect("ranked");
    assert_eq!(full.total, SEED.len() as i64, "every seeded row matched");
    assert_eq!(
        full.ranked,
        SEED.iter().filter(|(_, _, likes, _)| *likes > 0).count() as i64,
        "ranked is the engaged rows"
    );

    let one = metrics::ranked(&db, &query(), Metric::Quality, 1, 0)
        .await
        .expect("ranked");
    assert_eq!(one.items.len(), 1);
    assert_eq!((one.ranked, one.total), (full.ranked, full.total), "the gate is page-independent");
}

/// An unmeasured row is last and says so; it is never interleaved with rows the
/// metric could actually score.
#[tokio::test]
async fn an_unmeasured_row_sits_last_and_is_marked() {
    let db = connect("unmeasured").await;
    seed(&db).await;

    let page = metrics::ranked(&db, &query(), Metric::SourcePct, 50, 0)
        .await
        .expect("ranked");
    let last = page.items.last().expect("a page");
    assert_eq!(last.row.source_work_key, "sb-silent");
    assert!(!last.measured, "a zero-like row is unmeasured, not bad");
    assert!(
        page.items.iter().take(page.items.len() - 1).all(|item| item.measured),
        "every measured row precedes the unmeasured block"
    );
}

/// A source filter keeps that source's distribution: the qq page is ranked
/// inside qq, so the filter narrows the population rather than the answer.
#[tokio::test]
async fn the_source_filter_keeps_its_own_distribution() {
    let db = connect("scoped").await;
    seed(&db).await;

    let mut scoped = query();
    scoped.source = Some("qq".to_owned());
    let page = metrics::ranked(&db, &scoped, Metric::SourcePct, 50, 0)
        .await
        .expect("ranked");
    assert!(page.items.iter().all(|item| item.row.source_key == "qq"));
    assert_eq!(page.ranked, 10);
    assert_eq!(page.items[0].row.source_work_key, "qq-marvel");
    assert!(
        page.items[0].score > 0.7,
        "the top row of ten engaged rows is a strong percentile, was {}",
        page.items[0].score
    );
}
