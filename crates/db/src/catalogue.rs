//! Public catalogue search over `library_items`.
//!
//! `library_items` is a metadata catalogue of the fanfiction web: one row per
//! story per source, with no story text. The reader-facing library routes read
//! it *scoped to an account* (a reader's own imports); this module reads it
//! unscoped, because the catalogue is not anybody's private shelf — it is the
//! index of what exists, and the whole point is that a reader can ask it
//! "show me SpaceBattles, best first".
//!
//! # The quality signal, and what "aggregate" means here
//!
//! Each source reports endorsement in its own unit — threadmark likes on the
//! XenForo boards, kudos on AO3, followers on Royal Road — and the crawler
//! normalises all of them onto `library_items.likes`. That number is *imported*,
//! and the row says so in its provenance.
//!
//! `total_likes` adds two things that `likes` alone cannot answer:
//!
//! 1. the same story's likes from every source it appears on, matched on
//!    normalised title + author (a story cross-posted to SpaceBattles and
//!    Sufficient Velocity is one story with two endorsement pools);
//! 2. the kudos the story earned *here*, on darkhold, when the catalogue row was
//!    materialised into a work (`library_items.work_id` set).
//!
//! The three numbers are kept separate in the response on purpose: summing them
//! silently is how an imported like gets mistaken for an organic kudos.
//!
//! # Ordering
//!
//! `likes` is the default because it is the only signal every source reports and
//! the one the operator asked for ("by quality"). `views` and `words` are there
//! because they answer different questions; `updated` is the catalogue's own
//! freshness, not the source's.

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::{sql_owned, Database, Result};

/// A catalogue row, as the search returns it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct CatalogueRow {
    /// Library item id.
    pub id: String,
    /// Which site the story came from (`sb`, `sv`, `qq`, `ao3`, `royalroad`, …).
    pub source_key: String,
    /// The site's own identifier for the story.
    pub source_work_key: String,
    /// Story title as the source shows it.
    pub title: String,
    /// Author as the source shows it.
    pub author_text: String,
    /// Plain-text summary.
    pub summary: String,
    /// The source's word count, when it publishes one.
    pub word_count: Option<i64>,
    /// The source's lifecycle word: `ongoing`, `complete`, `cancelled`, `unknown`.
    pub status: String,
    /// Imported endorsement count — the normalised likes/kudos/followers.
    pub likes: i64,
    /// The source's view count, when it publishes one.
    pub views: Option<i64>,
    /// The story's canonical address at the source.
    pub source_url: String,
    /// Sum of `likes` across every catalogue row for the same story (all sources).
    pub total_likes: i64,
    /// Kudos the story earned on darkhold itself, when it was published here.
    pub darkhold_kudos: i64,
    /// How many sources carry this story.
    pub source_count: i64,
    /// The crawler's own provenance blob, as stored.
    pub provenance_json: String,
}

/// Which column the catalogue is ordered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogueSort {
    /// Imported likes, descending — the default, and what `quality` aliases.
    Likes,
    /// `total_likes`, descending: every source plus darkhold's own kudos.
    Aggregate,
    /// Views, descending.
    Views,
    /// Word count, descending.
    Words,
    /// Most recently changed catalogue row first.
    Updated,
}

impl CatalogueSort {
    /// Parse the query string's `sort=` parameter.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            // `quality` is the operator's word for it; `likes` is what it is.
            "" | "likes" | "quality" => Some(Self::Likes),
            "aggregate" | "total" => Some(Self::Aggregate),
            "views" => Some(Self::Views),
            "words" | "word_count" => Some(Self::Words),
            "updated" | "recent" => Some(Self::Updated),
            _ => None,
        }
    }

    /// For the in-memory check of the enum's own vocabulary.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Likes => "likes",
            Self::Aggregate => "aggregate",
            Self::Views => "views",
            Self::Words => "words",
            Self::Updated => "updated",
        }
    }
}

/// What the caller asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogueQuery {
    /// Restrict to one source (`sb`, `sv`, …).
    pub source: Option<String>,
    /// Only rows with at least this many imported likes.
    pub min_likes: Option<i64>,
    /// Case-insensitive substring over title and author.
    pub q: Option<String>,
    /// Ordering; `None` means [`CatalogueSort::Likes`].
    pub sort: Option<CatalogueSort>,
    /// Page size, clamped by the store.
    pub limit: Option<i64>,
    /// Page offset.
    pub offset: Option<i64>,
}

/// One source key and how many catalogue rows it contributed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct SourceCount {
    /// The source key (`sb`, `sv`, `ao3`, …).
    pub source_key: String,
    /// How many catalogue rows carry it.
    pub count: i64,
}

/// Every source the catalogue holds, largest first.
///
/// # Errors
///
/// Returns the store's error when the query fails.
pub async fn sources(db: &Database) -> Result<Vec<SourceCount>> {
    let sql = db.sql(
        "SELECT source_key, COUNT(*) AS count FROM library_items GROUP BY source_key ORDER BY count DESC",
        "SELECT source_key, COUNT(*) AS count FROM library_items GROUP BY source_key ORDER BY count DESC",
    );
    let rows: Vec<SourceCount> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(&sql)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(&sql)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows)
}

/// The same story's likes across every source, plus its darkhold kudos.
///
/// Correlated rather than joined because it has to be evaluated once per
/// returned row, not once per candidate row: a search returns twenty rows and
/// this must cost twenty index lookups, not twenty over the whole catalogue.
const TOTAL_LIKES_SQLITE: &str = "(SELECT COALESCE(SUM(l2.likes), 0) \
     FROM library_items l2 \
    WHERE lower(trim(l2.title)) = lower(trim(library_items.title)) \
      AND lower(trim(l2.author_text)) = lower(trim(library_items.author_text)))";

const TOTAL_LIKES_PG: &str = "(SELECT COALESCE(SUM(l2.likes), 0) \
     FROM library_items l2 \
    WHERE lower(btrim(l2.title)) = lower(btrim(library_items.title)) \
      AND lower(btrim(l2.author_text)) = lower(btrim(library_items.author_text)))";

const SOURCE_COUNT_SQLITE: &str = "(SELECT COUNT(DISTINCT l3.source_key) \
     FROM library_items l3 \
    WHERE lower(trim(l3.title)) = lower(trim(library_items.title)) \
      AND lower(trim(l3.author_text)) = lower(trim(library_items.author_text)))";

const SOURCE_COUNT_PG: &str = "(SELECT COUNT(DISTINCT l3.source_key) \
     FROM library_items l3 \
    WHERE lower(btrim(l3.title)) = lower(btrim(library_items.title)) \
      AND lower(btrim(l3.author_text)) = lower(btrim(library_items.author_text)))";

/// Kudos earned on darkhold itself, for the work this catalogue row became.
const DARKHOLD_KUDOS_SQLITE: &str = "(SELECT COALESCE(wma.kudos, 0) \
     FROM work_metric_aggregates wma \
    WHERE wma.work_id = library_items.work_id)";

const DARKHOLD_KUDOS_PG: &str = "(SELECT COALESCE(wma.kudos, 0) \
     FROM work_metric_aggregates wma \
    WHERE wma.work_id = library_items.work_id)";

fn order_by(sort: CatalogueSort) -> &'static str {
    // The tie-breaker is the id so paging is stable; without it two rows with
    // equal likes can swap places between pages and one gets skipped.
    match sort {
        CatalogueSort::Likes => "likes DESC, id DESC",
        CatalogueSort::Aggregate => "total_likes DESC, likes DESC, id DESC",
        CatalogueSort::Views => "views DESC NULLS LAST, likes DESC, id DESC",
        CatalogueSort::Words => "word_count DESC NULLS LAST, likes DESC, id DESC",
        CatalogueSort::Updated => "updated_at DESC, id DESC",
    }
}

/// Search the catalogue.
///
/// # Errors
///
/// Returns the store's error when the query fails.
pub async fn search(db: &Database, query: &CatalogueQuery) -> Result<Vec<CatalogueRow>> {
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let offset = query.offset.unwrap_or(0).max(0);
    let source = query.source.as_deref().filter(|s| !s.is_empty());
    let min_likes = query.min_likes.unwrap_or(0).max(0);
    let needle = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase);

    let sort = query.sort.unwrap_or(CatalogueSort::Likes);
    let order = order_by(sort);

    // SQLite and PostgreSQL agree on the shape; only the aliasing of the id and
    // the case-insensitive operator differ, which is why both are written out.
    let columns_sqlite = format!(
        "id, source_key, source_work_key, title, author_text, summary, word_count, status, \
         likes, views, source_url, provenance_json, {TOTAL_LIKES_SQLITE} AS total_likes, \
         {DARKHOLD_KUDOS_SQLITE} AS darkhold_kudos, {SOURCE_COUNT_SQLITE} AS source_count"
    );
    let columns_pg = format!(
        "id::text AS id, source_key, source_work_key, title, author_text, summary, word_count, \
         status, likes, views, source_url, provenance_json, {TOTAL_LIKES_PG} AS total_likes, \
         {DARKHOLD_KUDOS_PG} AS darkhold_kudos, {SOURCE_COUNT_PG} AS source_count"
    );

    let base_sqlite = format!(
        "SELECT {columns_sqlite} FROM library_items \
         WHERE (?1 IS NULL OR source_key = ?1) \
           AND likes >= ?2 \
           AND (?3 IS NULL OR lower(title) LIKE '%' || ?3 || '%' \
                          OR lower(author_text) LIKE '%' || ?3 || '%') \
         ORDER BY {order} LIMIT ?4 OFFSET ?5"
    );
    let base_pg = format!(
        "SELECT {columns_pg} FROM library_items \
         WHERE (?1::text IS NULL OR source_key = ?1::text) \
           AND likes >= ?2 \
           AND (?3::text IS NULL OR lower(title) LIKE '%' || ?3::text || '%' \
                          OR lower(author_text) LIKE '%' || ?3::text || '%') \
         ORDER BY {order} LIMIT ?4 OFFSET ?5"
    );
    let sql = sql_owned(db, base_sqlite, base_pg);

    match db.backend() {
        crate::Backend::Sqlite => {
            let rows: Vec<CatalogueRow> = sqlx::query_as(&sql)
                .bind(source)
                .bind(min_likes)
                .bind(needle.as_deref())
                .bind(limit)
                .bind(offset)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            Ok(rows)
        }
        crate::Backend::Postgres => {
            let rows: Vec<CatalogueRow> = sqlx::query_as(&sql)
                .bind(source)
                .bind(min_likes)
                .bind(needle.as_deref())
                .bind(limit)
                .bind(offset)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?;
            Ok(rows)
        }
    }
}
