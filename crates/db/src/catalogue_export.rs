//! A downloadable SQLite snapshot of the catalogue, for people who would rather
//! run SQL than page a route.
//!
//! `GET /catalogue/export.sqlite` serves the file this module builds. The shape
//! of the file is a privacy decision, not a technical one, so the rule is stated
//! here and enforced by construction: **the export names its columns and never
//! selects `*`.**
//!
//! # Why the snapshot can be exact ("de-anonymised") and still be safe
//!
//! `library_items` is a per-account table. It carries `account_id` — which import
//! put the row there — and `provenance_json`, the importer's own bookkeeping. A
//! snapshot that copied the table would publish, for every story, the reader who
//! catalogued it: a membership list with a metadata costume on.
//!
//! What the export carries instead is the *story* side: title, byline, summary,
//! word count, status, address, and the endorsement counts the source itself
//! publishes. None of that is about a person. That is why the k-anonymity floor
//! (10 distinct pseudonyms before an aggregate about people is shown) does not
//! apply here and the counts are exported exactly: rounding a like count to the
//! nearest ten would make the download useless while protecting nobody, because
//! there is no one in the file to protect. The floor protects people; this file
//! has none.
//!
//! The byline is the source's public byline — the same string the source site
//! shows next to the story — and is not an account on this instance.
//!
//! # One row per (source, source work)
//!
//! Several accounts may catalogue the same story; the export collapses them, so
//! a power user's `COUNT(*)` is a count of works, not of imports. The collapse
//! takes the maximum of each imported field, which is the same value every
//! account's row carries for imported metadata.
//!
//! # The aggregate columns
//!
//! `total_likes` and `source_count` match what `catalogue::search` computes for
//! its `sort=aggregate` ordering: the same story's likes summed across every
//! source it appears on, matched on normalised title + byline. They are
//! *materialised* in the file because the normalisation rule is the expensive
//! part and a client that got it slightly wrong would silently double-count.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::Row;

use crate::{sql_owned, Database, Result};

/// Bumped when the file's schema changes in a way a query would notice.
pub const SCHEMA_VERSION: u32 = 1;

/// What the file is, without opening it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExportSummary {
    /// Schema generation of the file's tables (`SCHEMA_VERSION`).
    pub schema_version: u32,
    /// Catalogue entries written — one per (source, source work).
    pub rows: i64,
    /// Distinct stories, matched on normalised title + byline.
    pub works: i64,
    /// Distinct source keys present.
    pub sources: i64,
    /// Size of the file in bytes.
    pub bytes: u64,
    /// Hex sha256 over the catalogue's `(count, max updated_at, max synced_at)`.
    /// Stable while the catalogue is, so it doubles as the cache key and the ETag.
    pub fingerprint: String,
    /// When the file was written, RFC3339.
    pub built_at: String,
}

/// The privacy statement the file carries in its own `meta` table.
const PRIVACY: &str = "Story metadata only: titles, bylines, summaries, word counts, addresses and \
     the endorsement counts each source publishes. No account, pseudonym, email, import record or \
     reading history is present, and no column on this file identifies a reader of this instance. \
     Exact counts are exported because nothing here describes a person.";

/// One catalogue entry as the export writes it.
#[derive(Debug, Clone, sqlx::FromRow)]
struct EntryRow {
    source_key: String,
    source_work_key: String,
    title: String,
    author_text: String,
    summary: String,
    language: Option<String>,
    word_count: Option<i64>,
    status: String,
    source_url: String,
    source_updated_at: Option<String>,
    likes: i64,
    views: Option<i64>,
    updated_at: Option<String>,
    on_darkhold: i64,
}

/// A group of same-story entries, accumulated for the aggregate columns.
#[derive(Debug, Clone, Copy)]
struct StoryTotals {
    total_likes: i64,
    source_count: i64,
}

/// Fingerprint the catalogue so a caller can tell whether a rebuild is needed.
///
/// Two cheap aggregates, hashed: the row count and the newest change on either
/// clock (our `updated_at`, the source's `last_synced_at`). A delete and an
/// insert that cancel out would keep the count but move neither maximum, which
/// is the one collision worth accepting — the alternative is hashing the table.
pub async fn fingerprint(db: &Database) -> Result<String> {
    let sql = "SELECT COUNT(*) AS n, COALESCE(MAX(updated_at), '') AS u, \
                      COALESCE(MAX(last_synced_at), '') AS s \
               FROM library_items";
    let (n, u, s): (i64, String, String) = match db.backend() {
        crate::Backend::Sqlite => {
            let row = sqlx::query(sql)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            (row.get("n"), row.get("u"), row.get("s"))
        }
        crate::Backend::Postgres => {
            let row = sqlx::query(sql)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?;
            (row.get("n"), row.get("u"), row.get("s"))
        }
    };
    let mut hasher = Sha256::new();
    hasher.update(format!("{n}\u{1}{u}\u{1}{s}").as_bytes());
    Ok(hex::encode(hasher.finalize()))
}

/// Build the snapshot at `path`, atomically.
///
/// The file is written beside its destination under a `.tmp` name and renamed
/// into place, so a reader never sees a half-written database and a crash leaves
/// the previous snapshot intact.
pub async fn build_sqlite(db: &Database, path: &Path) -> Result<ExportSummary> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp: PathBuf = {
        let mut name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "catalogue.sqlite".to_owned());
        name.push_str(&format!(".{}.tmp", uuid::Uuid::new_v4().simple()));
        path.with_file_name(name)
    };

    let fingerprint = fingerprint(db).await?;
    let built_at = now_rfc3339();
    let totals = story_totals(db).await?;

    let out = SqlitePoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(30))
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&tmp)
                .create_if_missing(true)
                .journal_mode(SqliteJournalMode::Off)
                .synchronous(sqlx::sqlite::SqliteSynchronous::Off),
        )
        .await?;

    let mut tx = out.begin().await?;
    for statement in SCHEMA {
        sqlx::query(statement).execute(&mut *tx).await?;
    }

    // Page through the collapsed catalogue on (source_key, source_work_key) so
    // the whole table never sits in memory beside the map of story totals.
    let mut written = 0_i64;
    let mut cursor: (String, String) = (String::new(), String::new());
    loop {
        let page = fetch_page(db, &cursor, PAGE).await?;
        if page.is_empty() {
            break;
        }
        for row in &page {
            let key = format!("{}:{}", row.source_key, row.source_work_key);
            let totals_for = totals
                .get(&(normalise(&row.title), normalise(&row.author_text)))
                .copied()
                .unwrap_or(StoryTotals {
                    total_likes: row.likes,
                    source_count: 1,
                });
            sqlx::query(
                "INSERT INTO works (work_key, source_key, source_work_key, title, author_text, \
                 summary, language, word_count, status, source_url, source_updated_at, likes, \
                 views, on_darkhold, total_likes, source_count, catalogue_updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&key)
            .bind(&row.source_key)
            .bind(&row.source_work_key)
            .bind(&row.title)
            .bind(&row.author_text)
            .bind(&row.summary)
            .bind(&row.language)
            .bind(row.word_count)
            .bind(&row.status)
            .bind(&row.source_url)
            .bind(&row.source_updated_at)
            .bind(row.likes)
            .bind(row.views)
            .bind(row.on_darkhold)
            .bind(totals_for.total_likes)
            .bind(totals_for.source_count)
            .bind(&row.updated_at)
            .execute(&mut *tx)
            .await?;
            written += 1;
        }
        let last = page.last().expect("non-empty page");
        cursor = (last.source_key.clone(), last.source_work_key.clone());
    }

    for statement in INDEXES {
        sqlx::query(statement).execute(&mut *tx).await?;
    }

    sqlx::query(
        "INSERT INTO sources (source_key, works, likes, newest_update) \
         SELECT source_key, COUNT(*), SUM(likes), MAX(catalogue_updated_at) \
           FROM works GROUP BY source_key ORDER BY source_key",
    )
    .execute(&mut *tx)
    .await?;

    let distinct: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM (SELECT DISTINCT lower(trim(title)) AS t, \
                                        lower(trim(author_text)) AS a FROM works)",
    )
    .fetch_one(&mut *tx)
    .await?;
    let sources: (i64,) = sqlx::query_as("SELECT COUNT(DISTINCT source_key) FROM works")
        .fetch_one(&mut *tx)
        .await?;

    let meta: [(&str, String); 9] = [
        ("schema_version", SCHEMA_VERSION.to_string()),
        ("generator", format!("darkhold catalogue export v{SCHEMA_VERSION}")),
        ("built_at", built_at.clone()),
        ("fingerprint", fingerprint.clone()),
        ("rows", written.to_string()),
        ("works", distinct.0.to_string()),
        ("sources", sources.0.to_string()),
        ("backend", format!("{:?}", db.backend()).to_ascii_lowercase()),
        ("privacy", PRIVACY.to_owned()),
    ];
    for (k, v) in meta {
        sqlx::query("INSERT INTO meta (key, value) VALUES (?, ?)")
            .bind(k)
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    out.close().await;

    tokio::fs::rename(&tmp, path).await?;
    let bytes = tokio::fs::metadata(path).await?.len();

    Ok(ExportSummary {
        schema_version: SCHEMA_VERSION,
        rows: written,
        works: distinct.0,
        sources: sources.0,
        bytes,
        fingerprint,
        built_at,
    })
}

/// Page size for the collapsed catalogue read.
const PAGE: i64 = 5_000;

/// Tables first, then indexes: an index on a table that does not exist yet is a
/// migration-order bug in a different costume.
const SCHEMA: &[&str] = &[
    "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
    "CREATE TABLE works (
        work_key              TEXT PRIMARY KEY,
        source_key            TEXT NOT NULL,
        source_work_key       TEXT NOT NULL,
        title                 TEXT NOT NULL,
        author_text           TEXT NOT NULL,
        summary               TEXT NOT NULL,
        language              TEXT,
        word_count            INTEGER,
        status                TEXT NOT NULL,
        source_url            TEXT NOT NULL,
        source_updated_at     TEXT,
        likes                 INTEGER NOT NULL,
        views                 INTEGER,
        on_darkhold           INTEGER NOT NULL,
        total_likes           INTEGER NOT NULL,
        source_count          INTEGER NOT NULL,
        catalogue_updated_at  TEXT
    )",
    "CREATE TABLE sources (
        source_key    TEXT PRIMARY KEY,
        works         INTEGER NOT NULL,
        likes         INTEGER NOT NULL,
        newest_update TEXT
    )",
];

/// The indexes a power user's queries actually use: filter by source, sort by
/// either like count, look up a title, or ask what changed recently.
const INDEXES: &[&str] = &[
    "CREATE INDEX idx_works_source ON works (source_key, likes DESC)",
    "CREATE INDEX idx_works_likes ON works (likes DESC)",
    "CREATE INDEX idx_works_total_likes ON works (total_likes DESC)",
    "CREATE INDEX idx_works_updated ON works (catalogue_updated_at DESC)",
    "CREATE INDEX idx_works_title ON works (lower(title))",
    "CREATE INDEX idx_works_words ON works (word_count DESC)",
];

/// Normalised story key, matching the rule `catalogue::search` sums with.
fn normalise(value: &str) -> String {
    value.trim().to_lowercase()
}

/// Sum each story's likes across its sources, over the *collapsed* catalogue so
/// two accounts holding the same entry do not double-count it.
async fn story_totals(db: &Database) -> Result<HashMap<(String, String), StoryTotals>> {
    let sql = "SELECT lower(trim(title)) AS t, lower(trim(author_text)) AS a, \
                      COALESCE(SUM(likes), 0) AS total, COUNT(DISTINCT source_key) AS n \
                 FROM (SELECT source_key, source_work_key, title, author_text, \
                              MAX(likes) AS likes \
                         FROM library_items \
                        GROUP BY source_key, source_work_key) AS w \
                GROUP BY 1, 2";
    let rows: Vec<(String, String, i64, i64)> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(sql)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(sql)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(|(t, a, total, n)| {
            (
                (t, a),
                StoryTotals {
                    total_likes: total,
                    source_count: n,
                },
            )
        })
        .collect())
}

/// One page of the collapsed catalogue, keyset-paginated.
///
/// `MAX(...)` per column is the collapse rule: imported metadata is the same on
/// every account's row, and taking the maximum makes the result independent of
/// which import happened to run last.
async fn fetch_page(db: &Database, cursor: &(String, String), limit: i64) -> Result<Vec<EntryRow>> {
    let sqlite = "SELECT source_key, source_work_key, MAX(title) AS title, \
                         MAX(author_text) AS author_text, MAX(summary) AS summary, \
                         MAX(language) AS language, MAX(word_count) AS word_count, \
                         MAX(status) AS status, MAX(source_url) AS source_url, \
                         MAX(source_updated_at) AS source_updated_at, MAX(likes) AS likes, \
                         MAX(views) AS views, MAX(updated_at) AS updated_at, \
                         MAX(CASE WHEN work_id IS NULL THEN 0 ELSE 1 END) AS on_darkhold \
                    FROM library_items \
                   WHERE (source_key, source_work_key) > (?, ?) \
                   GROUP BY source_key, source_work_key \
                   ORDER BY source_key, source_work_key \
                   LIMIT ?";
    let postgres = sqlite.replace("(?, ?)", "(?::text, ?::text)").replace(
        "MAX(CASE WHEN work_id IS NULL THEN 0 ELSE 1 END)",
        "MAX(CASE WHEN work_id IS NULL THEN 0 ELSE 1 END)::bigint",
    );
    let sql = sql_owned(db, sqlite.to_owned(), postgres);
    match db.backend() {
        crate::Backend::Sqlite => Ok(sqlx::query_as(&sql)
            .bind(&cursor.0)
            .bind(&cursor.1)
            .bind(limit)
            .fetch_all(db.sqlite_pool().expect("sqlite handle"))
            .await?),
        crate::Backend::Postgres => Ok(sqlx::query_as(&sql)
            .bind(&cursor.0)
            .bind(&cursor.1)
            .bind(limit)
            .fetch_all(db.postgres_pool().expect("postgres handle"))
            .await?),
    }
}

/// RFC3339 UTC, without pulling a date library into this crate.
fn now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Days-to-civil algorithm (Howard Hinnant), so the value is a real instant
    // rather than an epoch integer a reader cannot read.
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Convert days since 1970-01-01 to (year, month, day).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schema_names_no_private_concept() {
        // The same assertion the integration test makes against a built file, run
        // against the declarations: a forbidden column fails here, at `cargo test`,
        // before anyone builds a snapshot to discover it.
        let forbidden = ["account", "email", "pseud", "user", "reader", "provenance"];
        for statement in SCHEMA.iter().chain(INDEXES.iter()) {
            let lower = statement.to_ascii_lowercase();
            for bad in forbidden {
                assert!(
                    !lower.contains(bad),
                    "schema statement names a private concept ({bad}): {statement}"
                );
            }
        }
    }

    #[test]
    fn the_privacy_statement_says_what_is_not_in_the_file() {
        assert!(PRIVACY.contains("No account"));
        assert!(PRIVACY.contains("Exact counts"));
    }

    #[test]
    fn dates_round_trip_through_the_civil_conversion() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_000), (2024, 10, 4));
        assert!(now_rfc3339().ends_with('Z'));
        assert_eq!(now_rfc3339().len(), 20);
    }
}
