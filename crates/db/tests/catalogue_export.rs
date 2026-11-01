//! The catalogue download must be queryable locally *and* carry no reader in it.
//!
//! Power users want to run their own SQL over the index instead of paging an API:
//! "Naruto fics updated in the last week, by likes per chapter" is one `SELECT`
//! against a file and forty requests against the route. That is worth shipping.
//!
//! It is worth shipping **only** in the shape asserted here. Two properties carry
//! the whole risk, and neither is visible from a passing route test:
//!
//! 1. **No column from the private side of `library_items`.** The catalogue table
//!    is per-account: `account_id` says which import produced the row, and
//!    `provenance_json` carries the importer's own bookkeeping. A snapshot that
//!    copied the table wholesale would publish, for every work, the account that
//!    catalogued it — a membership list wearing a metadata costume. The export
//!    therefore names its columns rather than selecting `*`, and this test walks
//!    the produced schema looking for the forbidden ones anywhere (column, table,
//!    view or index), because a leaked column does not have to be in `works`.
//!
//! 2. **Exact numbers, not rounded ones.** This is the "de-anonymised" half of
//!    the request, and it is safe for the same reason the k-anonymity floor does
//!    not apply: the floor protects aggregates *about people* — 10 distinct
//!    pseudonyms before a count is shown. Nothing in this file is about a person.
//!    Rounding likes to the nearest ten would only make the download useless. The
//!    test asserts a like count of 3 survives as 3.
//!
//! # What it would have caught
//!
//! A builder that selects `*` (the obvious implementation, and the one that grows
//! a new private column the day somebody adds one), and a builder that quietly
//! applies the analytics suppression rule to a file that has no people in it.

use std::path::PathBuf;
use std::time::Duration;

use darkhold_db::Database;

fn make_config(url: String) -> darkhold_db::DatabaseConfig {
    darkhold_db::DatabaseConfig {
        url,
        max_connections: 5,
        acquire_timeout: Duration::from_secs(5),
        slow_query_warn: Duration::ZERO,
    }
}

/// A scratch SQLite database. The export writes a *portable* file, so the test
/// needs the source backend to be real; PostgreSQL is exercised by the same code
/// path on the deployed instance and is not gated here.
async fn connect(tag: &str) -> Database {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "darkhold-cexport-{}-{}-{}",
        tag,
        uuid::Uuid::new_v4(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let db = Database::connect(&make_config(format!(
        "sqlite://{}/darkhold.sqlite?mode=rwc",
        dir.display()
    )))
    .await
    .expect("connect");
    db.migrate().await.expect("migrate");
    db
}

fn out_path(tag: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "darkhold-cexport-out-{}-{}-{}",
        tag,
        uuid::Uuid::new_v4(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir.join("catalogue.sqlite")
}

/// One account, three catalogue rows: two sources for the same story (so the
/// cross-source aggregate has something to add up) and one story with 3 likes
/// (so exactness is observable).
async fn seed(db: &Database) {
    let pool = db.sqlite_pool().expect("sqlite pool");
    sqlx::query(
        "INSERT INTO accounts (id, email, created_at, updated_at) \
         VALUES ('acc-1', 'reader@example.invalid', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(pool)
    .await
    .expect("account");

    for (id, source, key, title, author, likes, views, updated) in [
        ("li-1", "sb", "t-1", "Winter's Wake", "Vesna", 3_i64, Some(900_i64), "2026-10-30T10:00:00Z"),
        ("li-2", "sv", "t-2", "Winter's Wake", "Vesna", 40_i64, None, "2026-10-29T10:00:00Z"),
        ("li-3", "sb", "t-3", "The Long Road", "Orrin", 120_i64, Some(5000_i64), "2026-10-01T10:00:00Z"),
    ] {
        sqlx::query(
            "INSERT INTO library_items \
             (id, account_id, source_key, source_work_key, title, author_text, summary, \
              word_count, status, source_url, source_updated_at, provenance_json, \
              created_at, updated_at, likes, views) \
             VALUES (?, 'acc-1', ?, ?, ?, ?, '', 90000, 'ongoing', 'https://example.invalid/x', ?, \
                     '{\"adapter\":\"xenforo\",\"account\":\"reader@example.invalid\"}', \
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', ?, ?)",
        )
        .bind(id)
        .bind(source)
        .bind(key)
        .bind(title)
        .bind(author)
        .bind(updated)
        .bind(likes)
        .bind(views)
        .execute(pool)
        .await
        .expect("library item");
    }
}

#[tokio::test]
async fn the_snapshot_is_a_sqlite_file_with_exact_counts_and_no_readers_in_it() {
    let db = connect("shape").await;
    seed(&db).await;

    let path = out_path("shape");
    let summary = darkhold_db::catalogue_export::build_sqlite(&db, &path)
        .await
        .expect("build the snapshot");

    // A real SQLite file, not a renamed CSV.
    let magic = std::fs::read(&path).expect("read");
    assert_eq!(&magic[..16], b"SQLite format 3\0", "not a sqlite file");

    assert_eq!(summary.schema_version, darkhold_db::catalogue_export::SCHEMA_VERSION);
    assert_eq!(summary.rows, 3, "one row per catalogue entry");
    assert_eq!(summary.works, 2, "two distinct stories");
    assert_eq!(summary.sources, 2, "sb and sv");

    // Read it back the way a power user would: a second connection, plain SQL.
    let reader = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!("sqlite://{}?mode=ro", path.display()))
        .await
        .expect("open the snapshot read-only");

    let (rows,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM works")
        .fetch_one(&reader)
        .await
        .expect("count works");
    assert_eq!(rows, 3);

    // Exact, unsuppressed. 3 likes stays 3: nothing here is about a person.
    let (likes,): (i64,) =
        sqlx::query_as("SELECT likes FROM works WHERE source_key = 'sb' AND source_work_key = 't-1'")
            .fetch_one(&reader)
            .await
            .expect("likes");
    assert_eq!(likes, 3, "a like count must not be rounded or floor-suppressed");

    // The cross-source aggregate the API computes is materialised in the file,
    // because doing it client-side needs the normalisation rule.
    let (total, count): (i64, i64) = sqlx::query_as(
        "SELECT total_likes, source_count FROM works WHERE source_work_key = 't-1'",
    )
    .fetch_one(&reader)
    .await
    .expect("aggregate");
    assert_eq!((total, count), (43, 2), "3 + 40 across two sources");

    // The meta table says what the file is, so a downloaded file explains itself.
    let meta: Vec<(String, String)> = sqlx::query_as("SELECT key, value FROM meta ORDER BY key")
        .fetch_all(&reader)
        .await
        .expect("meta");
    let get = |k: &str| {
        meta.iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    assert_eq!(get("schema_version"), "1");
    assert_eq!(get("rows"), "3");
    assert!(get("built_at").ends_with('Z'), "built_at is an RFC3339 instant");
    assert!(
        get("privacy").to_lowercase().contains("no account"),
        "the file must state its own privacy boundary: {}",
        get("privacy")
    );

    // ===== the assertion this test exists for =====
    // Walk every object in the file and refuse any name from the private side.
    let objects: Vec<(String, String)> =
        sqlx::query_as("SELECT type, name FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'")
            .fetch_all(&reader)
            .await
            .expect("sqlite_master");
    let forbidden = [
        "account_id",
        "account",
        "email",
        "provenance_json",
        "pseud",
        "user",
        "reader",
    ];
    for (kind, name) in &objects {
        let lower = name.to_ascii_lowercase();
        for bad in forbidden {
            assert!(
                !lower.contains(bad),
                "export object {kind} {name:?} names a private concept ({bad})"
            );
        }
    }
    // A view or table cannot smuggle a column past the object check above, so
    // check columns too, across every table.
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )
    .fetch_all(&reader)
    .await
    .expect("tables");
    for table in tables {
        let cols: Vec<String> = sqlx::query_scalar(&format!("SELECT name FROM pragma_table_info('{table}')"))
            .fetch_all(&reader)
            .await
            .expect("columns");
        for col in cols {
            let lower = col.to_ascii_lowercase();
            for bad in forbidden {
                assert!(
                    !lower.contains(bad),
                    "column {table}.{col} names a private concept ({bad})"
                );
            }
        }
    }
    // And the provenance blob's *contents* must not have travelled either: it
    // carried an email in the fixture.
    let hits: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM works WHERE summary LIKE '%example.invalid%' \
            OR author_text LIKE '%example.invalid%'",
    )
    .fetch_one(&reader)
    .await
    .expect("scan");
    assert_eq!(hits, 0, "no personal data from provenance_json may reach the file");

    reader.close().await;
}

#[tokio::test]
async fn rebuilding_is_deterministic_and_atomic() {
    let db = connect("rebuild").await;
    seed(&db).await;
    let path = out_path("rebuild");

    let first = darkhold_db::catalogue_export::build_sqlite(&db, &path)
        .await
        .expect("first build");
    let bytes_first = std::fs::read(&path).expect("read first");

    // A fingerprint that changes only when the catalogue does: the cache key the
    // route uses to decide whether to rebuild at all.
    let again = darkhold_db::catalogue_export::fingerprint(&db)
        .await
        .expect("fingerprint");
    assert_eq!(again, first.fingerprint, "an unchanged catalogue has a stable fingerprint");
    assert_eq!(again.len(), 64, "hex sha256");

    // No stray temp file: the write is a rename, so a half-written snapshot is
    // never served.
    let dir = path.parent().expect("dir");
    let leftovers: Vec<_> = std::fs::read_dir(dir)
        .expect("read dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.contains(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "temp files left behind: {leftovers:?}");
    assert!(bytes_first.len() > 1024, "the file has content");

    let second = darkhold_db::catalogue_export::build_sqlite(&db, &path)
        .await
        .expect("second build");
    assert_eq!(second.fingerprint, first.fingerprint);
}
