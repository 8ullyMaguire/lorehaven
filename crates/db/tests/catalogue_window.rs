//! M63-02: the window predicates over the catalogue, on both engines.
//!
//! Three properties, and each one is a bug the naive query has:
//!
//! 1. **`7d` and `>7d` partition the table.** The boundary row must be in exactly
//!    one of them, which is what half-open bounds buy. With `<=` on either end a
//!    row at the boundary is in both, and every "how many new vs old" count is
//!    wrong by one while both numbers look plausible.
//!
//! 2. **The three clocks are three columns.** A window on `updated` must constrain
//!    `source_updated_at`, not `last_synced_at`; a crossed pair would answer a
//!    question nobody asked and look correct on a fixture where the two happen to
//!    agree. The fixture below makes them disagree deliberately.
//!
//! 3. **The timestamps are not one format.** Measured on the live catalogue:
//!    `source_updated_at` is a bare date (`2026-10-13`) for the XenForo importer,
//!    while `last_synced_at` and `created_at` carry `+00:00`. A plain text
//!    comparison against an RFC3339 bound is wrong at the boundary in both
//!    shapes, so the predicate normalises both sides — and this file is the only
//!    place that would notice if somebody replaced that with a raw `>=`.
//!
//! Run on SQLite by default; set `DARKHOLD_TEST_PG_URL` to exercise the
//! PostgreSQL arm, which is the one with the `::timestamptz` cast.

use std::time::Duration;

use darkhold_db::catalogue::{self, CatalogueQuery};
use darkhold_db::Database;
use time::macros::datetime;
use time::OffsetDateTime;

fn make_config(url: String) -> darkhold_db::DatabaseConfig {
    darkhold_db::DatabaseConfig {
        url,
        max_connections: 5,
        acquire_timeout: Duration::from_secs(5),
        slow_query_warn: Duration::ZERO,
    }
}

async fn connect(tag: &str) -> Database {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "darkhold-window-{}-{}-{}",
        tag,
        uuid::Uuid::new_v4(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let url = match std::env::var("DARKHOLD_TEST_PG_URL") {
        Ok(admin) => {
            let name = format!(
                "lh_window_{}_{}",
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

async fn exec_with(db: &Database, sqlite: &str, args: &[&str]) {
    match db.backend() {
        darkhold_db::Backend::Sqlite => {
            let sql = sqlite.replace("#u", "").replace("#i", "");
            let mut q = sqlx::query(&sql);
            for a in args {
                q = q.bind(*a);
            }
            q.execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("fixture insert");
        }
        darkhold_db::Backend::Postgres => {
            let pg = (1..=9).fold(sqlite.to_string(), |acc, n| {
                acc.replace(&format!("?{n}#u"), &format!("${n}::uuid"))
                    .replace(&format!("?{n}#i"), &format!("${n}::bigint"))
                    .replace(&format!("?{n}"), &format!("${n}"))
            });
            let mut q = sqlx::query(&pg);
            for a in args {
                match uuid::Uuid::parse_str(a) {
                    Ok(u) => q = q.bind(u),
                    Err(_) => q = q.bind(*a),
                }
            }
            q.execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("fixture insert");
        }
    }
}

/// Four catalogue rows, chosen so each property above has a witness:
///
/// | key | source_updated_at | why |
/// |---|---|---|
/// | `bare-date` | `2026-10-13` | older than 7d **and** stored without a time |
/// | `offset` | `2026-10-30T10:00:00+00:00` | within 7d, `+00:00` shape |
/// | `boundary` | `2026-10-25T12:00:00Z` | exactly 7 days before `NOW` |
/// | `unknown` | NULL | neither window may claim it |
///
/// `now` is 2026-11-01T12:00:00Z, so 7d starts at 2026-10-25T12:00:00Z exactly.
async fn seed(db: &Database) {
    // A concrete id, not a subselect: SQLite permits NULL in a TEXT primary key,
    // so an unbound `?1` inserts an account whose id is NULL and every later
    // reference fails with a NOT NULL on `account_id` instead of on the cause.
    let account = uuid::Uuid::new_v4().to_string();
    exec_with(
        db,
        "INSERT INTO accounts (id, email, created_at, updated_at) \
         VALUES (?1#u, 'window@example.invalid', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        &[account.as_str()],
    )
    .await;

    // `library_items.id` is a UUID on PostgreSQL, so the fixture's own labels go
    // in `source_work_key` and the id is generated. The assertions read the key.
    for (key, source_updated, synced) in [
        ("bare-date", Some("2026-10-13"), "2026-03-01T00:00:00+00:00"),
        // Updated three days ago, last rechecked in September: the pair that
        // makes a crossed column visible.
        ("offset", Some("2026-10-30T10:00:00+00:00"), "2026-09-01T00:00:00+00:00"),
        ("boundary", Some("2026-10-25T12:00:00Z"), "2026-10-25T12:00:00+00:00"),
        ("unknown", None, "2026-10-31T00:00:00+00:00"),
    ] {
        let id = uuid::Uuid::new_v4().to_string();
        match source_updated {
            Some(source) => exec_with(
                db,
                "INSERT INTO library_items \
                 (id, account_id, source_key, source_work_key, title, author_text, summary, \
                  word_count, status, source_url, source_updated_at, last_synced_at, created_at, \
                  updated_at, likes) \
                 VALUES (?1#u, ?5#u, 'sb', ?2, ?2, 'A', '', 1000, \
                         'ongoing', 'https://example.invalid/x', ?3, ?4, ?4, ?4, 5)",
                &[id.as_str(), key, source, synced, account.as_str()],
            )
            .await,
            None => exec_with(
                db,
                "INSERT INTO library_items \
                 (id, account_id, source_key, source_work_key, title, author_text, summary, \
                  word_count, status, source_url, source_updated_at, last_synced_at, created_at, \
                  updated_at, likes) \
                 VALUES (?1#u, ?4#u, 'sb', ?2, ?2, 'A', '', 1000, \
                         'ongoing', 'https://example.invalid/x', NULL, ?3, ?3, ?3, 5)",
                &[id.as_str(), key, synced, account.as_str()],
            )
            .await,
        }
    }
}

fn now() -> OffsetDateTime {
    datetime!(2026-11-01 12:00:00 UTC)
}

fn window(raw: &str) -> darkhold_domain::window::Bounds {
    darkhold_domain::window::parse(raw, now()).expect("window")
}

async fn keys(db: &Database, updated: Option<&str>, refreshed: Option<&str>) -> Vec<String> {
    let query = CatalogueQuery {
        source: None,
        min_likes: None,
        q: None,
        sort: None,
        limit: Some(50),
        offset: Some(0),
        updated: updated.map(window),
        refreshed: refreshed.map(window),
        added: None,
    };
    let mut out: Vec<String> = catalogue::search(db, &query)
        .await
        .expect("search")
        .into_iter()
        .map(|row| row.source_work_key)
        .collect();
    out.sort();
    out
}

#[tokio::test]
async fn a_window_partitions_the_catalogue_exactly() {
    let db = connect("partition").await;
    seed(&db).await;

    let within = keys(&db, Some("7d"), None).await;
    let older = keys(&db, Some(">7d"), None).await;

    assert_eq!(within, vec!["boundary", "offset"], "7d includes its boundary");
    assert_eq!(older, vec!["bare-date"], ">7d excludes it");
    assert!(
        within.iter().all(|k| !older.contains(k)),
        "the two windows must not overlap: {within:?} and {older:?}"
    );
    // The row with no source clock is in neither, which is the honest answer:
    // "we do not know when the author touched it" is not "recently".
    let all: Vec<String> = within.iter().chain(older.iter()).cloned().collect();
    assert!(!all.contains(&"unknown".to_string()));
    assert_eq!(all.len(), 3, "every dated row is in exactly one window");

    db.close().await;
}

#[tokio::test]
async fn the_three_clocks_are_three_columns() {
    let db = connect("clocks").await;
    seed(&db).await;

    // `offset` was updated three days ago but last rechecked in September, so the
    // two windows disagree on it. Swapping the columns in the SQL would put it in
    // the refreshed set and out of the updated one, and nothing else would notice.
    let by_source = keys(&db, Some("7d"), None).await;
    let by_sync = keys(&db, None, Some("7d")).await;
    assert_eq!(by_source, vec!["boundary", "offset"], "the source clock is October");
    // `unknown` appears here and in no source-clock window: it has no source
    // clock at all, but we did recheck it yesterday. That asymmetry is the point
    // of having two flags -- "the author touched it" and "we looked" are
    // different facts, and a row can have one without the other.
    assert_eq!(
        by_sync,
        vec!["boundary", "unknown"],
        "our clock is September for `offset`"
    );
    assert!(
        by_source.contains(&"offset".to_string()) && !by_sync.contains(&"offset".to_string()),
        "the two clocks must be read from different columns"
    );

    db.close().await;
}

#[tokio::test]
async fn mixed_timestamp_shapes_compare_as_instants() {
    let db = connect("shapes").await;
    seed(&db).await;

    // A bare date is midnight that day. `@2026-10-13` covers exactly that day, so
    // the row is inside it -- a raw text comparison against `2026-10-13T00:00:00Z`
    // would say the value is smaller than its own day start and return nothing.
    let day = keys(&db, Some("@2026-10-13"), None).await;
    assert_eq!(day, vec!["bare-date"], "a date-only value is that whole day");

    // The boundary row is stored with `Z` and the window bound is formatted with a
    // space; both normalise to the same instant. Compared as raw text, `Z` sorts
    // after a space and the row would fall out of its own window.
    let boundary_day = keys(&db, Some("@2026-10-25"), None).await;
    assert_eq!(boundary_day, vec!["boundary"]);

    // And an `+00:00` value near the boundary is inside the rolling week.
    let week = keys(&db, Some("7d"), None).await;
    assert!(week.contains(&"offset".to_string()), "{week:?}");

    db.close().await;
}
