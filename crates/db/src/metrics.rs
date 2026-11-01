//! Ranking the catalogue by a quality metric that does not depend on archive
//! size (M63-03).
//!
//! The maths lives in [`darkhold_domain::quality`] and takes no database and no
//! clock. This module's job is only to feed it: the candidate rows, and the
//! freshness in days that the domain layer is not allowed to read itself.
//!
//! # Why the candidate query is ordered by likes
//!
//! A percentile is only meaningful against the whole population it describes, so
//! a page of twenty rows cannot be percentiled. The query therefore pulls up to
//! [`CANDIDATE_CAP`] rows of the filtered set ordered by `likes DESC`, which puts
//! every engaged row of every source inside the cap — measured 2026-11-01, the
//! whole catalogue holds 1,247 of them, and even `sb`, the largest source, has
//! 884. The remainder of the cap is the unmeasured block, which needs 5,000 rows
//! only because triage wants something to choose between.

use darkhold_domain::quality::{rank, Candidate, Metric, Signals};
use darkhold_domain::similar::{self, Idf, Seed};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::catalogue::{
    filters_sql, CatalogueQuery, CatalogueRow, DARKHOLD_KUDOS_PG, DARKHOLD_KUDOS_SQLITE,
    SOURCE_COUNT_PG, SOURCE_COUNT_SQLITE, TOTAL_LIKES_PG, TOTAL_LIKES_SQLITE,
};
use crate::{Backend, Database, Result};

/// How many rows the ranker sees. Must exceed the catalogue's engaged-row count
/// (1,247 on 2026-11-01) or percentiles would be computed against a truncated
/// population.
pub const CANDIDATE_CAP: i64 = 5_000;

/// The light row the domain ranker needs.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CandidateRow {
    /// Library item id.
    pub id: String,
    /// The archive it came from.
    pub source_key: String,
    /// Imported endorsement count.
    pub likes: i64,
    /// The source's word count, when it publishes one.
    pub word_count: Option<i64>,
    /// Likes summed across every copy of the story.
    pub total_likes: i64,
    /// How many archives carry the story.
    pub source_count: i64,
    /// The source's last-update date, in the canonical stored shape.
    pub source_updated_at: Option<String>,
    /// Title, as the source shows it — the heaviest similarity signal (M63-04).
    pub title: String,
    /// Summary, as the source shows it.
    pub summary: String,
    /// The author's pseud, for the same-author bonus.
    pub author_text: String,
}

/// One ranked row: the stored row the caller asked for, plus why it is where it
/// is.
#[derive(Debug, Clone)]
pub struct RankedRow {
    /// The catalogue row itself, unchanged.
    pub row: CatalogueRow,
    /// The source's last-update date, which the catalogue row does not carry.
    pub updated_at: Option<String>,
    /// 0..=1 for every ranked metric.
    pub score: f64,
    /// False when the metric could not measure this row and triage ordered it.
    pub measured: bool,
    /// The components of the score.
    pub signals: Signals,
    /// 1-based position on the page the caller asked for.
    pub rank: usize,
}

/// A page of a metric ranking, with the numbers that stop it reading as a
/// verdict.
#[derive(Debug, Clone)]
pub struct RankedPage {
    /// The metric these scores belong to.
    pub metric: Metric,
    /// Engaged rows the metric could score across the whole filtered set.
    pub ranked: i64,
    /// Rows the filters matched.
    pub total: i64,
    /// The page.
    pub items: Vec<RankedRow>,
}

/// Days between the source's last update and now; `None` when the row has no
/// date, which is 2,009 of `sb`'s rows and every row of the estates import.
fn freshness_days(raw: Option<&str>) -> Option<f64> {
    let parsed = OffsetDateTime::parse(raw?.trim(), &Rfc3339).ok()?;
    let seconds = (OffsetDateTime::now_utc() - parsed).whole_seconds();
    Some(seconds as f64 / 86_400.0)
}

/// The `WHERE` clause shared by the candidate and count queries.
fn predicate_sqlite(query: &CatalogueQuery) -> String {
    format!(
        "WHERE (? IS NULL OR source_key = ?) AND likes >= ? \
         AND (? IS NULL OR lower(title) LIKE '%' || ? || '%' \
                        OR lower(author_text) LIKE '%' || ? || '%'){}",
        filters_sql(query).sqlite
    )
}

fn predicate_postgres(query: &CatalogueQuery) -> String {
    format!(
        "WHERE (?::text IS NULL OR source_key = ?::text) AND likes >= ? \
         AND (?::text IS NULL OR lower(title) LIKE '%' || ?::text || '%' \
                        OR lower(author_text) LIKE '%' || ?::text || '%'){}",
        filters_sql(query).postgres
    )
}

fn needle_of(query: &CatalogueQuery) -> Option<String> {
    query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
}

fn source_of(query: &CatalogueQuery) -> Option<String> {
    query
        .source
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Fetch the candidate population.
///
/// # Errors
///
/// Returns the store's error when the query fails.
pub async fn candidates(db: &Database, query: &CatalogueQuery) -> Result<Vec<CandidateRow>> {
    // Title, summary and author ride along because the similarity ranker needs
    // them; on every other metric they are a few hundred bytes of unused row.
    let columns = match db.backend() {
        Backend::Sqlite => format!(
            "id, source_key, likes, word_count, source_updated_at, title, \
             coalesce(summary, '') AS summary, author_text, \
             {TOTAL_LIKES_SQLITE} AS total_likes, {SOURCE_COUNT_SQLITE} AS source_count"
        ),
        Backend::Postgres => format!(
            "id::text AS id, source_key, likes::bigint AS likes, word_count::bigint AS word_count, \
             source_updated_at, title, coalesce(summary, '')::text AS summary, author_text, \
             {TOTAL_LIKES_PG}::bigint AS total_likes, \
             {SOURCE_COUNT_PG}::bigint AS source_count"
        ),
    };
    let (sqlite, postgres) = (
        format!(
            "SELECT {columns} FROM library_items {} ORDER BY likes DESC, id DESC LIMIT ?",
            predicate_sqlite(query)
        ),
        format!(
            "SELECT {columns} FROM library_items {} ORDER BY likes DESC, id DESC LIMIT ?",
            predicate_postgres(query)
        ),
    );
    let sql = crate::sql_owned(db, sqlite, postgres);

    let source = source_of(query);
    let needle = needle_of(query);
    let min_likes = query.min_likes.unwrap_or(0).max(0);
    let windows = filters_sql(query).binds;

    // Each arm builds its own query: `sqlx::query_as` infers the database from
    // the executor, so a single value cannot be bound to both pools.
    match db.backend() {
        Backend::Sqlite => {
            let mut q = sqlx::query_as::<_, CandidateRow>(&sql)
                .bind(source.clone())
                .bind(source.clone())
                .bind(min_likes)
                .bind(needle.clone())
                .bind(needle.clone())
                .bind(needle.clone());
            for value in &windows {
                q = q.bind(value.clone());
            }
            Ok(q
                .bind(CANDIDATE_CAP)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?)
        }
        Backend::Postgres => {
            let mut q = sqlx::query_as::<_, CandidateRow>(&sql)
                .bind(source.clone())
                .bind(source.clone())
                .bind(min_likes)
                .bind(needle.clone())
                .bind(needle.clone())
                .bind(needle.clone());
            for value in &windows {
                q = q.bind(value.clone());
            }
            Ok(q
                .bind(CANDIDATE_CAP)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?)
        }
    }
}

/// `(total matching, engaged matching)`. Both are needed because the gate reads
/// `ranked N of M`, and `M` must be the filter's own total, not the capped
/// candidate set.
///
/// # Errors
///
/// Returns the store's error when the query fails.
pub async fn counts(db: &Database, query: &CatalogueQuery) -> Result<(i64, i64)> {
    let (sqlite, postgres) = (
        format!(
            "SELECT count(*), count(*) FILTER (WHERE likes > 0) FROM library_items {}",
            predicate_sqlite(query)
        ),
        format!(
            "SELECT count(*), count(*) FILTER (WHERE likes > 0) FROM library_items {}",
            predicate_postgres(query)
        ),
    );
    let sql = crate::sql_owned(db, sqlite, postgres);

    let source = source_of(query);
    let needle = needle_of(query);
    let min_likes = query.min_likes.unwrap_or(0).max(0);
    let windows = filters_sql(query).binds;

    match db.backend() {
        Backend::Sqlite => {
            let mut q = sqlx::query_as::<_, (i64, i64)>(&sql)
                .bind(source.clone())
                .bind(source.clone())
                .bind(min_likes)
                .bind(needle.clone())
                .bind(needle.clone())
                .bind(needle.clone());
            for value in &windows {
                q = q.bind(value.clone());
            }
            Ok(q.fetch_one(db.sqlite_pool().expect("sqlite handle")).await?)
        }
        Backend::Postgres => {
            let mut q = sqlx::query_as::<_, (i64, i64)>(&sql)
                .bind(source.clone())
                .bind(source.clone())
                .bind(min_likes)
                .bind(needle.clone())
                .bind(needle.clone())
                .bind(needle.clone());
            for value in &windows {
                q = q.bind(value.clone());
            }
            Ok(q
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?)
        }
    }
}

/// The full catalogue row plus its source date, for the page's ids.
#[derive(Debug, Clone, sqlx::FromRow)]
struct FullRow {
    id: String,
    source_key: String,
    source_work_key: String,
    title: String,
    author_text: String,
    summary: String,
    word_count: Option<i64>,
    status: String,
    likes: i64,
    views: Option<i64>,
    source_url: String,
    provenance_json: String,
    total_likes: i64,
    darkhold_kudos: i64,
    source_count: i64,
    source_updated_at: Option<String>,
}

impl From<FullRow> for (CatalogueRow, Option<String>) {
    fn from(row: FullRow) -> Self {
        (
            CatalogueRow {
                id: row.id,
                source_key: row.source_key,
                source_work_key: row.source_work_key,
                title: row.title,
                author_text: row.author_text,
                summary: row.summary,
                word_count: row.word_count,
                status: row.status,
                likes: row.likes,
                views: row.views,
                source_url: row.source_url,
                total_likes: row.total_likes,
                darkhold_kudos: row.darkhold_kudos,
                source_count: row.source_count,
                provenance_json: row.provenance_json,
            },
            row.source_updated_at,
        )
    }
}

async fn rows_by_ids(db: &Database, ids: &[String]) -> Result<Vec<(CatalogueRow, Option<String>)>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = vec!["?"; ids.len()].join(",");
    let columns = match db.backend() {
        Backend::Sqlite => format!(
            "id, source_key, source_work_key, title, author_text, summary, word_count, status, \
             likes, views, source_url, provenance_json, source_updated_at, \
             {TOTAL_LIKES_SQLITE} AS total_likes, {DARKHOLD_KUDOS_SQLITE} AS darkhold_kudos, \
             {SOURCE_COUNT_SQLITE} AS source_count"
        ),
        Backend::Postgres => format!(
            "id::text AS id, source_key, source_work_key, title, author_text, summary, \
             word_count::bigint AS word_count, status, likes::bigint AS likes, \
             views::bigint AS views, source_url, provenance_json, source_updated_at, \
             {TOTAL_LIKES_PG}::bigint AS total_likes, \
             {DARKHOLD_KUDOS_PG}::bigint AS darkhold_kudos, \
             {SOURCE_COUNT_PG}::bigint AS source_count"
        ),
    };
    // `id` is `uuid` on PostgreSQL and `TEXT` on SQLite, and the ids arrive as
    // strings: the cast has to happen on the column, per engine, or PostgreSQL
    // compares a uuid to a text bind and answers a type error.
    let id_column = match db.backend() {
        Backend::Sqlite => "id",
        Backend::Postgres => "id::text",
    };
    // Through `sql_owned`, which is what rewrites `?` into `$n` for PostgreSQL.
    // Building the string directly leaves MySQL-style placeholders in a
    // PostgreSQL statement, which the server rejects.
    let statement = format!("SELECT {columns} FROM library_items WHERE {id_column} IN ({placeholders})");
    let sql = crate::sql_owned(db, statement.clone(), statement);
    match db.backend() {
        Backend::Sqlite => {
            let mut q = sqlx::query_as::<_, FullRow>(&sql);
            for id in ids {
                q = q.bind(id.clone());
            }
            let rows: Vec<FullRow> = q.fetch_all(db.sqlite_pool().expect("sqlite handle")).await?;
            Ok(rows.into_iter().map(Into::into).collect())
        }
        Backend::Postgres => {
            let mut q = sqlx::query_as::<_, FullRow>(&sql);
            for id in ids {
                q = q.bind(id.clone());
            }
            let rows: Vec<FullRow> = q
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?;
            Ok(rows.into_iter().map(Into::into).collect())
        }
    }
}

/// Rank the filtered catalogue by `metric` and return one page of it.
///
/// The page is two blocks: rows the metric could measure, then rows it could not
/// (ordered by metadata triage). Callers should render the boundary — comparing
/// the two is the error this module exists to prevent.
///
/// # Errors
///
/// Returns the store's error when a query fails.
pub async fn ranked(
    db: &Database,
    query: &CatalogueQuery,
    metric: Metric,
    limit: usize,
    offset: usize,
) -> Result<RankedPage> {
    let rows = candidates(db, query).await?;
    let (total, engaged) = counts(db, query).await?;

    let population: Vec<Candidate> = rows
        .iter()
        .map(|row| Candidate {
            id: row.id.clone(),
            source_key: row.source_key.clone(),
            likes: row.likes,
            word_count: row.word_count,
            source_count: row.source_count,
            total_likes: row.total_likes,
            freshness_days: freshness_days(row.source_updated_at.as_deref()),
        })
        .collect();

    let ranking = rank(metric, &population, limit, offset);
    let ids: Vec<String> = ranking.hits.iter().map(|h| h.id.clone()).collect();
    let stored = rows_by_ids(db, &ids).await?;
    let by_id: std::collections::HashMap<String, (CatalogueRow, Option<String>)> = stored
        .into_iter()
        .map(|(row, updated)| (row.id.clone(), (row, updated)))
        .collect();

    let mut items = Vec::with_capacity(ranking.hits.len());
    for (index, hit) in ranking.hits.iter().enumerate() {
        if let Some((row, updated_at)) = by_id.get(&hit.id) {
            items.push(RankedRow {
                row: row.clone(),
                updated_at: updated_at.clone(),
                score: hit.score,
                measured: hit.measured,
                signals: hit.signals,
                rank: offset + index + 1,
            });
        }
    }

    Ok(RankedPage {
        metric,
        // The filter's own engaged count, not the capped candidate set's: with a
        // 5,000-row cap and 1,247 engaged rows they agree, and if a future import
        // makes them disagree the gate should report the population.
        ranked: engaged,
        total,
        items,
    })
}

/// One row of a seed search: the stored row, how similar it is, and why.
#[derive(Debug, Clone)]
pub struct SimilarRow {
    /// The catalogue row itself.
    pub row: CatalogueRow,
    /// The source's last-update date, which the catalogue row does not carry.
    pub updated_at: Option<String>,
    /// 0..=1 similarity to the seed.
    pub score: f64,
    /// The shared tokens that carried the score, most informative first.
    pub shared: Vec<String>,
    /// 1-based position on the page.
    pub rank: usize,
}

/// A page of "similar to this one", with the numbers that stop it reading as a
/// statement about the whole catalogue.
#[derive(Debug, Clone)]
pub struct SimilarPage {
    /// The row the caller seeded with.
    pub seed_id: String,
    /// Its title, so a report can name what was asked for.
    pub seed_title: String,
    /// Rows in the scanned population that share at least one token with the seed.
    pub ranked: i64,
    /// Rows the filters matched, across the whole catalogue.
    pub total: i64,
    /// Rows the ranker actually saw — the candidate cap, not the catalogue.
    pub scanned: i64,
    /// The page.
    pub items: Vec<SimilarRow>,
}

/// A resolved seed row: the fields similarity needs, plus how to name it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedLookup {
    /// Library item id.
    pub id: String,
    /// Title as the source shows it.
    pub title: String,
    /// Author pseud as the source shows it.
    pub author: String,
    /// Summary as the source shows it.
    pub summary: String,
    /// The archive it came from, so a refusal can say which copy.
    pub source_key: String,
    /// Imported endorsement count, which is how "the copy people read" is decided.
    pub likes: i64,
}

/// One row a `--seed` spec could have meant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedCandidate {
    /// Library item id.
    pub id: String,
    /// Title as the source shows it.
    pub title: String,
    /// The archive it came from.
    pub source_key: String,
    /// Imported endorsement count.
    pub likes: i64,
}

impl From<&SeedLookup> for SeedCandidate {
    fn from(row: &SeedLookup) -> Self {
        Self {
            id: row.id.clone(),
            title: row.title.clone(),
            source_key: row.source_key.clone(),
            likes: row.likes,
        }
    }
}

/// What a `--seed` spec resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeedMatch {
    /// Exactly one row.
    One(SeedLookup),
    /// Several rows could be meant and none is dominant: the caller must choose.
    Many(Vec<SeedCandidate>),
    /// Nothing matches.
    None,
}

/// How many candidates a `--seed` search looks at.
const AMBIGUOUS_LIMIT: i64 = 20;

/// One lookup, on either engine, with the id cast each engine needs.
///
/// `where_*` may use `?` more than once; every bind is passed in order.
async fn seed_rows(
    db: &Database,
    where_sqlite: &str,
    where_postgres: &str,
    binds: &[String],
    order: &str,
    limit: i64,
) -> Result<Vec<SeedLookup>> {
    let (columns_sqlite, columns_pg) = (
        "id, title, author_text, coalesce(summary, ''), source_key, likes",
        "id::text AS id, title, author_text, coalesce(summary, '')::text AS summary, \
         source_key, likes::bigint AS likes",
    );
    let sqlite = format!(
        "SELECT {columns_sqlite} FROM library_items WHERE {where_sqlite} ORDER BY {order} LIMIT {limit}"
    );
    let postgres = format!(
        "SELECT {columns_pg} FROM library_items WHERE {where_postgres} ORDER BY {order} LIMIT {limit}"
    );
    let sql = crate::sql_owned(db, sqlite, postgres);
    match db.backend() {
        Backend::Sqlite => {
            let mut q = sqlx::query_as::<_, (String, String, String, String, String, i64)>(&sql);
            for bind in binds {
                q = q.bind(bind.clone());
            }
            Ok(q.fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
                .into_iter()
                .map(|(id, title, author, summary, source_key, likes)| SeedLookup {
                    id,
                    title,
                    author,
                    summary,
                    source_key,
                    likes,
                })
                .collect())
        }
        Backend::Postgres => {
            let mut q = sqlx::query_as::<_, (String, String, String, String, String, i64)>(&sql);
            for bind in binds {
                q = q.bind(bind.clone());
            }
            Ok(q.fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
                .into_iter()
                .map(|(id, title, author, summary, source_key, likes)| SeedLookup {
                    id,
                    title,
                    author,
                    summary,
                    source_key,
                    likes,
                })
                .collect())
        }
    }
}

/// Resolve a `--seed` argument to the row it names.
///
/// # The order, and why it is not "exact title first"
///
/// An id or a source URL is an identity and wins outright. A **title** is not: on
/// this catalogue the exact title is usually the *dead* copy. Measured on the live
/// instance 2026-11-01, `--seed "Ghost in the City"` matches two `archive` rows
/// with **0 likes** by the same author, while the story a reader means is
/// `Ghost in the City. Cyberpunk Gamer SI` on `sb` with 602,176. An exact-title
/// rule sends every reader to the stub, and an "ambiguous" refusal that lists two
/// identical titles tells them nothing.
///
/// So the title pass ranks by likes and accepts a row when either
///
/// * every candidate is the same story — same title, same author, several archives
///   (the cross-post case, which `total_likes` already treats as one story), or
/// * the most-read candidate is well ahead of the next one: strictly more than
///   zero likes and at least ten times the runner-up's.
///
/// Otherwise the caller is given the candidates with their archive, likes and id,
/// which is a refusal they can act on.
///
/// # Errors
///
/// Returns the store's error when a query fails.
pub async fn resolve_seed(db: &Database, spec: &str) -> Result<SeedMatch> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Ok(SeedMatch::None);
    }
    // `id` is `uuid` on PostgreSQL and `TEXT` on SQLite: the cast has to be on the
    // column, or PostgreSQL compares a uuid to a text bind and answers a type
    // error that surfaces as a 500 with no SQL in it.
    let by_id = seed_rows(db, "id = ?", "id::text = ?", &[spec.to_owned()], "id", 1).await?;
    if let Some(found) = by_id.into_iter().next() {
        return Ok(SeedMatch::One(found));
    }
    if spec.starts_with("https://") || spec.starts_with("http://") {
        let by_url = seed_rows(
            db,
            "source_url = ?",
            "source_url = ?",
            &[spec.to_owned()],
            "id",
            1,
        )
        .await?;
        if let Some(found) = by_url.into_iter().next() {
            return Ok(SeedMatch::One(found));
        }
    }
    let rows = seed_rows(
        db,
        "lower(title) = lower(trim(?)) OR lower(title) LIKE '%' || lower(?) || '%'",
        "lower(btrim(title)) = lower(btrim(?)) OR lower(title) LIKE '%' || lower(?::text) || '%'",
        &[spec.to_owned(), spec.to_owned()],
        "likes DESC, id DESC",
        AMBIGUOUS_LIMIT,
    )
    .await?;

    let Some(top) = rows.first() else {
        return Ok(SeedMatch::None);
    };
    if rows.len() == 1 {
        return Ok(SeedMatch::One(top.clone()));
    }
    let second = &rows[1];
    let same_story = rows.iter().all(|row| {
        row.title.trim().eq_ignore_ascii_case(top.title.trim())
            && row.author.trim().eq_ignore_ascii_case(top.author.trim())
    });
    let dominant = top.likes > 0 && (second.likes == 0 || top.likes >= second.likes.saturating_mul(10));
    if same_story || dominant {
        return Ok(SeedMatch::One(top.clone()));
    }
    Ok(SeedMatch::Many(rows.iter().map(SeedCandidate::from).collect()))
}

/// Rank the filtered catalogue by similarity to `seed_row` (M63-04).
///
/// `ranked` counts the scanned population that shares at least one token — those
/// are the rows that are actually neighbours. `total` is what the filters matched
/// and `scanned` is the candidate cap, so a page can never read as a statement
/// about the whole catalogue when only 5,000 rows were looked at.
///
/// # Errors
///
/// Returns the store's error when a query fails.
pub async fn similar(
    db: &Database,
    query: &CatalogueQuery,
    seed_row: &SeedLookup,
    limit: usize,
    offset: usize,
) -> Result<SimilarPage> {
    let rows = candidates(db, query).await?;
    let (total, _engaged) = counts(db, query).await?;
    let scanned = rows.len() as i64;

    let seed = Seed::of(
        seed_row.id.clone(),
        &seed_row.title,
        &seed_row.summary,
        &seed_row.author,
    );
    let population: Vec<Seed> = rows
        .iter()
        .map(|row| Seed::of(row.id.clone(), &row.title, &row.summary, &row.author_text))
        .collect();
    let idf = Idf::build(&population);
    let hits = similar::rank(&seed, &population, &idf);
    let ranked = hits.len() as i64;

    let page: Vec<&similar::SimHit> = hits.iter().skip(offset).take(limit).collect();
    let ids: Vec<String> = page.iter().map(|hit| hit.id.clone()).collect();
    let stored = rows_by_ids(db, &ids).await?;
    let by_id: std::collections::HashMap<String, (CatalogueRow, Option<String>)> = stored
        .into_iter()
        .map(|(row, updated)| (row.id.clone(), (row, updated)))
        .collect();

    let mut items = Vec::with_capacity(page.len());
    for (index, hit) in page.iter().enumerate() {
        if let Some((row, updated_at)) = by_id.get(&hit.id) {
            items.push(SimilarRow {
                row: row.clone(),
                updated_at: updated_at.clone(),
                score: hit.score,
                shared: hit.shared.clone(),
                rank: offset + index + 1,
            });
        }
    }

    Ok(SimilarPage {
        seed_id: seed_row.id.clone(),
        seed_title: seed_row.title.clone(),
        ranked,
        total,
        scanned,
        items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_or_unparseable_date_is_absent_rather_than_ancient() {
        assert_eq!(freshness_days(None), None);
        assert_eq!(freshness_days(Some("")), None);
        assert_eq!(freshness_days(Some("not a date")), None);
    }

    #[test]
    fn the_stored_canonical_shape_parses() {
        // Migration 0132 makes this the only shape the database accepts.
        let days = freshness_days(Some("2026-10-27T17:30:00+00:00")).expect("parses");
        assert!(days > 0.0, "a past date is a positive age, was {days}");
    }
}
