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
//! One caveat the response cannot fix and the UI must not hide: on a XenForo
//! board the crawler sums `data-likes` over every threadmark, so a XenForo number
//! is *total engagement across chapters* while an AO3 number is kudos for the
//! work as a whole. Ordering the catalogue by `likes` therefore rewards long
//! serials, which is also what a board reader means by a popular fic — but it is
//! not the same question as "best per chapter". That is why the row carries
//! `word_count` and `status` too: a caller that wants quality-per-length has the
//! inputs to compute it, and nothing is silently rescaled here.
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
    /// Window on `source_updated_at`: when the author last touched it.
    pub updated: Option<darkhold_domain::window::Bounds>,
    /// Window on `last_synced_at`: when this crawler last rechecked it.
    pub refreshed: Option<darkhold_domain::window::Bounds>,
    /// Window on `created_at`: when it entered this catalogue.
    pub added: Option<darkhold_domain::window::Bounds>,
    /// Restrict to one fandom, as the catalogue spells it.
    ///
    /// Matched against `canonical_entities.norm` (the lowercased canonical name)
    /// for `kind = 'fandom'`; the route resolves the name first so an unknown
    /// fandom is a `422` with a suggestion rather than an empty page. The store
    /// deliberately takes the name rather than the node id: a caller that knows
    /// only "naruto" should not have to run a second query to filter by it.
    pub fandom: Option<String>,
    /// Restrict to one author, matched case-insensitively but exactly (M63-04).
    ///
    /// Exactly, not as a substring: a substring match on `Oshha` also returns
    /// `Oshhalo`, and a reader asking for one author's works is asking for one
    /// author, not for every pseud those letters appear in.
    pub author: Option<String>,
    /// Candidate preselection for a seed search: a row whose title or summary
    /// contains none of these substrings cannot be a neighbour of the seed, so it
    /// never reaches the ranker (M63-04).
    ///
    /// This is a *filter*, not the similarity: it decides which rows are looked
    /// at, and [`crate::metrics::similar`] decides how similar they are. Empty for
    /// every other kind of query, which is why `filters_sql` treats it as an
    /// optional arm rather than a required one.
    pub tokens: Vec<String>,
    /// Exclude what one account is reading (M63-04).
    ///
    /// `--exclude-read` is the reader's own negative set: a recommendation for a
    /// story they are eleven chapters into is not a recommendation. Scoped to the
    /// account that asked, never global.
    pub exclude_account: Option<String>,
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

/// One fandom and how many catalogue rows carry it.
///
/// The count is over distinct library items, not over tags: a work tagged with
/// the same fandom twice (an author's typo, an import of two editions) is one
/// row in the catalogue and must be one here, or the number the CLI prints as
/// "265 fics" disagrees with the page it returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct FandomCount {
    /// The fandom as the catalogue spells it (`Naruto`, `Harry Potter`).
    pub name: String,
    /// How many catalogue rows are tagged with it.
    pub count: i64,
}

/// Every fandom the catalogue knows, largest first.
///
/// Used by `GET /catalogue/fandoms` and, more importantly, by the route's
/// suggestion when someone types `--fandom narut`: the useful answer to an
/// unknown name is the nearest known one, and it has to come from the same
/// query that lists them or the two can disagree.
///
/// # Errors
///
/// Returns the store's error when the query fails.
pub async fn fandoms(db: &Database, limit: i64) -> Result<Vec<FandomCount>> {
    let sql = db.sql(
        "SELECT ce.canonical AS name, COUNT(DISTINCT li.id) AS count \
           FROM canonical_entities ce \
           JOIN work_tags wt ON wt.node_id = ce.id \
           JOIN library_items li ON li.work_id = wt.work_id \
          WHERE ce.kind = 'fandom' \
          GROUP BY ce.canonical \
          ORDER BY count DESC, name ASC \
          LIMIT ?",
        "SELECT ce.canonical AS name, COUNT(DISTINCT li.id)::bigint AS count \
           FROM canonical_entities ce \
           JOIN work_tags wt ON wt.node_id = ce.id \
           JOIN library_items li ON li.work_id = wt.work_id \
          WHERE ce.kind = 'fandom' \
          GROUP BY ce.canonical \
          ORDER BY count DESC, name ASC \
          LIMIT ?",
    );
    let rows: Vec<FandomCount> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows)
}

/// How many rows the catalogue holds.
///
/// The denominator of the coverage line. Read on fandom-scoped requests only,
/// which is why it is its own query rather than a field on every page: a count
/// over 141k rows costs a scan, and a caller who did not ask about fandoms has
/// no use for it.
///
/// # Errors
///
/// Returns the store's error when the query fails.
pub async fn total_rows(db: &Database) -> Result<i64> {
    let sql = db.sql(
        "SELECT COUNT(*) AS count FROM library_items",
        "SELECT COUNT(*)::bigint AS count FROM library_items",
    );
    let count: (i64,) = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(&sql)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(&sql)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(count.0)
}

/// What the fandom tagging actually covers.
///
/// Two numbers, because one of them is the honest one. On the live catalogue all
/// 7,626 tagged rows come from `pawchive` and **none of them publishes a like
/// count**, so `--fandom naruto --sort likes` orders 265 rows that all have zero
/// likes: correct, and meaningless to a reader who is not told. `tagged` says how
/// much of the catalogue has a fandom at all; `with_likes` says how much of that
/// can be ranked by anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FandomCoverage {
    /// Rows carrying at least one fandom tag.
    pub tagged: i64,
    /// Of those, how many have a non-zero imported like count.
    pub with_likes: i64,
}

/// How much of the catalogue the fandom tags cover.
///
/// # Errors
///
/// Returns the store's error when the query fails.
pub async fn fandom_coverage(db: &Database) -> Result<FandomCoverage> {
    let sql = db.sql(
        "SELECT COUNT(DISTINCT li.id) AS tagged, \
                COUNT(DISTINCT CASE WHEN li.likes > 0 THEN li.id END) AS with_likes \
           FROM library_items li \
           JOIN work_tags wt ON wt.work_id = li.work_id \
           JOIN canonical_entities ce ON ce.id = wt.node_id \
          WHERE ce.kind = 'fandom'",
        "SELECT COUNT(DISTINCT li.id)::bigint AS tagged, \
                COUNT(DISTINCT li.id) FILTER (WHERE li.likes > 0)::bigint AS with_likes \
           FROM library_items li \
           JOIN work_tags wt ON wt.work_id = li.work_id \
           JOIN canonical_entities ce ON ce.id = wt.node_id \
          WHERE ce.kind = 'fandom'",
    );
    let row: (i64, i64) = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(&sql)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(&sql)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(FandomCoverage {
        tagged: row.0,
        with_likes: row.1,
    })
}

/// The same story's likes across every source, plus its darkhold kudos.
///
/// Correlated rather than joined because it has to be evaluated once per
/// returned row, not once per candidate row: a search returns twenty rows and
/// this must cost twenty index lookups, not twenty over the whole catalogue.
pub(crate) const TOTAL_LIKES_SQLITE: &str = "COALESCE((SELECT SUM(l2.likes) \
     FROM library_items l2 \
    WHERE lower(trim(l2.title)) = lower(trim(library_items.title)) \
      AND lower(trim(l2.author_text)) = lower(trim(library_items.author_text))), 0)";

pub(crate) const TOTAL_LIKES_PG: &str = "COALESCE((SELECT SUM(l2.likes) \
     FROM library_items l2 \
    WHERE lower(btrim(l2.title)) = lower(btrim(library_items.title)) \
      AND lower(btrim(l2.author_text)) = lower(btrim(library_items.author_text))), 0)";

pub(crate) const SOURCE_COUNT_SQLITE: &str = "(SELECT COUNT(DISTINCT l3.source_key) \
     FROM library_items l3 \
    WHERE lower(trim(l3.title)) = lower(trim(library_items.title)) \
      AND lower(trim(l3.author_text)) = lower(trim(library_items.author_text)))";

pub(crate) const SOURCE_COUNT_PG: &str = "(SELECT COUNT(DISTINCT l3.source_key) \
     FROM library_items l3 \
    WHERE lower(btrim(l3.title)) = lower(btrim(library_items.title)) \
      AND lower(btrim(l3.author_text)) = lower(btrim(library_items.author_text)))";

/// Kudos earned on darkhold itself, for the work this catalogue row became.
///
/// `COALESCE` wraps the whole subquery, not its inner column: a row with no
/// `work_id`, or a work with no aggregate, returns *no rows* — and an aggregate
/// over no rows is `NULL`, which the decoder refuses for an `i64`.
pub(crate) const DARKHOLD_KUDOS_SQLITE: &str = "COALESCE((SELECT wma.kudos \
     FROM work_metric_aggregates wma \
    WHERE wma.work_id = library_items.work_id), 0)";

pub(crate) const DARKHOLD_KUDOS_PG: &str = "COALESCE((SELECT wma.kudos \
     FROM work_metric_aggregates wma \
    WHERE wma.work_id = library_items.work_id), 0)";

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
        "id::text AS id, source_key, source_work_key, title, author_text, summary, \
         word_count::bigint AS word_count, status, likes::bigint AS likes, \
         views::bigint AS views, source_url, provenance_json, \
         {TOTAL_LIKES_PG}::bigint AS total_likes, \
         {DARKHOLD_KUDOS_PG}::bigint AS darkhold_kudos, \
         {SOURCE_COUNT_PG}::bigint AS source_count"
    );

    // Windows: three clocks, one grammar, one stored shape. M63-02 normalised both
    // sides of every comparison because `library_items` held four timestamp shapes
    // (`2026-10-13` from the out-of-tree XenForo importer, `...T01:04:45+00:00` from
    // this crate's writer, plus `Z` and fractional forms in old fixtures), and a
    // text comparison is wrong at the boundary in every mixed pair. Migration 0132
    // backfilled them to one canonical shape and the database now refuses any other,
    // so the predicate is a plain range -- which is what makes the index usable.
    // docs/plans/m63-31-timestamp-normalisation.md is the record.
    let filters = filters_sql(query);

    let base_sqlite = format!(
        "SELECT {columns_sqlite} FROM library_items \
         WHERE (? IS NULL OR source_key = ?) \
           AND likes >= ? \
           AND (? IS NULL OR lower(title) LIKE '%' || ? || '%' \
                          OR lower(author_text) LIKE '%' || ? || '%'){windows_sqlite} \
         ORDER BY {order} LIMIT ? OFFSET ?",
        windows_sqlite = filters.sqlite
    );
    let base_pg = format!(
        "SELECT {columns_pg} FROM library_items \
         WHERE (?::text IS NULL OR source_key = ?::text) \
           AND likes >= ? \
           AND (?::text IS NULL OR lower(title) LIKE '%' || ?::text || '%' \
                          OR lower(author_text) LIKE '%' || ?::text || '%'){windows_pg} \
         ORDER BY {order} LIMIT ? OFFSET ?",
        windows_pg = filters.postgres
    );
    let sql = sql_owned(db, base_sqlite, base_pg);

    // Bind order is the placeholders' order, repeated where a value is used
    // twice: `?` is positional in both engines and sqlx rewrites it to `$n`.
    let bind_all = |source: Option<&str>, needle: Option<&str>| {
        (
            source.map(str::to_owned),
            source.map(str::to_owned),
            min_likes,
            needle.map(str::to_owned),
            needle.map(str::to_owned),
            needle.map(str::to_owned),
            limit,
            offset,
        )
    };
    let (b1, b2, b3, b4, b5, b6, b7, b8) = bind_all(source, needle.as_deref());
    // Window placeholders sit in the WHERE clause, so their binds come before
    // LIMIT/OFFSET in both engines' positional order.
    let windows_binds = filters.binds.clone();

    match db.backend() {
        crate::Backend::Sqlite => {
            let mut q = sqlx::query_as::<_, CatalogueRow>(&sql)
                .bind(b1)
                .bind(b2)
                .bind(b3)
                .bind(b4)
                .bind(b5)
                .bind(b6);
            for value in &windows_binds {
                q = q.bind(value.clone());
            }
            let rows: Vec<CatalogueRow> = q
                .bind(b7)
                .bind(b8)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            Ok(rows)
        }
        crate::Backend::Postgres => {
            let mut q = sqlx::query_as::<_, CatalogueRow>(&sql)
                .bind(b1)
                .bind(b2)
                .bind(b3)
                .bind(b4)
                .bind(b5)
                .bind(b6);
            for value in &windows_binds {
                q = q.bind(value.clone());
            }
            let rows: Vec<CatalogueRow> = q
                .bind(b7)
                .bind(b8)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?;
            Ok(rows)
        }
    }
}

/// The filter predicates, in one place, with the binds in the order the
/// placeholders appear.
pub(crate) struct FilterSql {
    pub(crate) sqlite: String,
    pub(crate) postgres: String,
    pub(crate) binds: Vec<String>,
}

pub(crate) fn filters_sql(query: &CatalogueQuery) -> FilterSql {
    let mut sqlite = String::new();
    let mut postgres = String::new();
    let mut binds = Vec::new();
    for (column, bounds) in [
        ("source_updated_at", query.updated),
        ("last_synced_at", query.refreshed),
        ("created_at", query.added),
    ] {
        let Some(bounds) = bounds else { continue };
        // One comparison, no function on the column. That is only correct because the stored
        // shape is now canonical and enforced (migration 0132): every value is
        // `YYYY-MM-DDTHH:MM:SS+00:00`, so text order is chronological order — measured on the
        // live database before the index was added: 0 disagreements between text order and
        // instant order across 91,471 dated rows under the database's en_US.UTF-8 collation.
        if let Some(from) = bounds.from_sql() {
            sqlite.push_str(&format!(" AND {column} >= ?"));
            postgres.push_str(&format!(" AND {column} >= ?"));
            binds.push(from);
        }
        if let Some(to) = bounds.to_sql() {
            sqlite.push_str(&format!(" AND {column} < ?"));
            postgres.push_str(&format!(" AND {column} < ?"));
            binds.push(to);
        }
    }
    // The fandom arm is an `EXISTS`, not a join, and that is load-bearing: a work
    // tagged with two fandoms that share a name would otherwise appear twice in a
    // page whose `limit` the caller asked for once. `work_id` is nullable on rows
    // the estate import could not link to a work, and `EXISTS` drops those for
    // free instead of needing a second predicate.
    if let Some(fandom) = query
        .fandom
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let predicate = " AND EXISTS (\
             SELECT 1 FROM work_tags wt \
               JOIN canonical_entities ce ON ce.id = wt.node_id \
              WHERE wt.work_id = library_items.work_id \
                AND ce.kind = 'fandom' \
                AND ce.norm = ?)";
        sqlite.push_str(predicate);
        postgres.push_str(predicate);
        binds.push(fandom.to_lowercase());
    }
    // The author arm. Both sides are `lower(trim(...))` so a pseud typed with a
    // trailing space or in another case still matches; the column stays bare on
    // its left side so an index on `author_text` is still a candidate (it is not
    // used today, because the filter is never the only predicate).
    if let Some(author) = query
        .author
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let predicate = " AND lower(trim(author_text)) = lower(trim(?))";
        sqlite.push_str(predicate);
        postgres.push_str(predicate);
        binds.push(author.to_owned());
    }
    // The seed preselection. An OR over `LIKE` is a table scan on both engines,
    // which is acceptable here and only here: the alternative is a trigram index
    // (PostgreSQL-only) or an FTS table (a migration, and a second engine-specific
    // schema). The cost is measured on the live catalogue in the M63-04 plan's
    // live-proof step rather than guessed at here.
    //
    // `coalesce(summary, '')` because the column is nullable in the SQLite schema;
    // without it a single null summary makes the whole concatenation null and the
    // row silently un-matchable.
    if !query.tokens.is_empty() {
        let arms: Vec<&str> = query
            .tokens
            .iter()
            .map(|_| "lower(title || ' ' || coalesce(summary, '')) LIKE '%' || ? || '%'")
            .collect();
        let predicate = format!(" AND ({})", arms.join(" OR "));
        sqlite.push_str(&predicate);
        postgres.push_str(&predicate);
        for token in &query.tokens {
            binds.push(token.to_lowercase());
        }
    }
    // The read-history arm. `subject_type = 'work'` because `reading_progress`
    // holds progress against a work, a unit or a revision, and only the work-level
    // rows describe "this account is reading this story"; `library_items.work_id`
    // is null for the 83,283 rows the estate import never linked, and `NOT EXISTS`
    // keeps those rows rather than dropping them.
    if let Some(account) = query.exclude_account.as_deref().filter(|s| !s.is_empty()) {
        // `account_id` is `uuid` on PostgreSQL and `TEXT` on SQLite, so the bind has
        // to be cast on one engine and left alone on the other; `work_id` follows
        // the same rule.
        let sqlite_predicate = " AND NOT EXISTS (\
             SELECT 1 FROM reading_progress rp \
              WHERE rp.account_id = ? \
                AND rp.subject_type = 'work' \
                AND rp.subject_id = library_items.work_id)";
        let postgres_predicate = " AND NOT EXISTS (\
             SELECT 1 FROM reading_progress rp \
              WHERE rp.account_id = ?::uuid \
                AND rp.subject_type = 'work' \
                AND rp.subject_id = library_items.work_id)";
        sqlite.push_str(sqlite_predicate);
        postgres.push_str(postgres_predicate);
        binds.push(account.to_owned());
    }
    FilterSql {
        sqlite,
        postgres,
        binds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_is_likes_because_that_is_what_the_operator_called_it() {
        assert_eq!(CatalogueSort::parse("quality"), Some(CatalogueSort::Likes));
        assert_eq!(CatalogueSort::parse("LIKES"), Some(CatalogueSort::Likes));
        assert_eq!(CatalogueSort::parse("  "), Some(CatalogueSort::Likes));
    }

    #[test]
    fn every_documented_sort_parses_and_round_trips() {
        for sort in [
            CatalogueSort::Likes,
            CatalogueSort::Aggregate,
            CatalogueSort::Views,
            CatalogueSort::Words,
            CatalogueSort::Updated,
        ] {
            assert_eq!(CatalogueSort::parse(sort.as_str()), Some(sort));
        }
    }

    #[test]
    fn an_unknown_sort_is_refused_rather_than_silently_reordered() {
        assert_eq!(CatalogueSort::parse("kudos"), None);
        assert_eq!(CatalogueSort::parse("likes desc; drop table"), None);
    }

    #[test]
    fn aggregate_ordering_breaks_ties_on_the_raw_count_then_the_id() {
        // A tie on total_likes must still page stably, so the ordering has to be
        // total: description order alone would let rows swap between pages.
        assert_eq!(
            order_by(CatalogueSort::Aggregate),
            "total_likes DESC, likes DESC, id DESC"
        );
        assert_eq!(order_by(CatalogueSort::Likes), "likes DESC, id DESC");
    }

    #[test]
    fn a_nullable_sort_column_sinks_without_hiding_rows() {
        // `views` is null for every board row; NULLS LAST keeps them on the page
        // instead of dropping them, which a bare DESC on a null column would do.
        assert!(order_by(CatalogueSort::Views).starts_with("views DESC NULLS LAST"));
    }
}
