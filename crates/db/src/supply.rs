//! §58 — supply: keystone authors, droughts, and the routing record (M45-25).
//!
//! ## The problem this module answers
//!
//! The catalogue's supply problem is not "too few works": it holds 141,317 rows.
//! It is that **1,247 of them carry any endorsement at all**, and the authors who
//! wrote those are the ones holding the catalogue up. §58 calls them *keystone
//! authors*, and the three things this module does are the three the section
//! specifies: compute the list, report the conversation those works have (or have
//! not) received, and record a reader's opt-in routing to one of them.
//!
//! ## Supply never touches ranking
//!
//! There is deliberately no function here that takes a keystone score and returns
//! a weight for a recommendation, a sort order or a percentile. `score_bp` orders
//! *this report*: which author a reader is routed to when they ask for one. If a
//! future row wants supply to influence ranking, that is a spec change and a
//! different module, not a call that already exists.
//!
//! ## Percentiles are shrunk, because a mean of raw counts is a size ranking
//!
//! A mean of endorsement counts would make every keystone author a SpaceBattles
//! author — the archive with 884 engaged rows out of 1,247 — which is M63-03's
//! bug in a new place. So a work's score is its percentile **within its own
//! source**, shrunk toward 0.5 by `n / (n + 10)`: a four-row source cannot claim
//! 1.000 on one lucky row, and a 400-row source's top decile still counts.
//!
//! ## Absence is reported, not defaulted
//!
//! Two absences matter and both are visible in the output rather than implied:
//! `admin_taste_profile` empty means a `taste_bonus_bp` of **0**, reported as 0
//! (an operator with no preference and an operator who prefers nothing else are
//! different facts), and a corpus with no comments and no ratings reports
//! `measured: false` instead of listing every engaged work as "in drought".

use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::{Backend, Database};

/// Map a fetched row set to tuples, in whichever driver's row type arrived.
///
/// The two drivers return different row types, so a `match` over the backend
/// cannot hand back `Vec<_>` directly; each arm maps inside itself through this.
/// One routing row, in whichever driver's row type arrived.
macro_rules! route_of {
    ($r:ident) => {
        Route {
            id: $r.get::<String, _>("id"),
            account_id: $r.get::<String, _>("account_id"),
            work_id: $r.get::<String, _>("work_id"),
            title: $r.get::<String, _>("title"),
            source_key: $r.get::<String, _>("source_key"),
            author_key: $r.get::<String, _>("author_key"),
            reason: $r.get::<String, _>("reason"),
            created_at: $r.get::<String, _>("created_at"),
            completed_at: $r.get::<Option<String>, _>("completed_at"),
            credit_bp: $r.get::<Option<i64>, _>("credit_bp"),
        }
    };
}

macro_rules! tuples {
    ($rows:expr, |$r:ident| $body:expr) => {
        $rows.iter().map(|$r| $body).collect::<Vec<_>>()
    };
}

/// An author needs at least this many engaged works to be a keystone (§58.1).
pub const DEFAULT_MIN_WORKS: i64 = 2;

/// …and at least this mean within-source percentile, in basis points.
pub const DEFAULT_MIN_PCT_BP: i64 = 7500;

/// §58.4's patronage: 5.00 credits, expressed in the ledger's basis points.
pub const PATRONAGE_BP: i64 = 500;

/// The shrinkage constant. Ten is not magic: it is the point at which a source
/// needs more than ten engaged rows for its own ranking to move the shrunk value
/// more than its prior, and it is the same ten M63-03 uses.
const SHRINK: f64 = 10.0;

/// Operator taste may raise a keystone by at most this much (§58.1).
const BONUS_CEILING: f64 = 0.25;

/// One computed keystone author.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Keystone {
    /// `lower(trim(author_text))` — the join key the catalogue already uses.
    pub author_key: String,
    /// The author's own spelling, from the first engaged row seen.
    pub display_name: String,
    /// Engaged works (endorsement above zero).
    pub engaged_works: i64,
    /// Mean of the shrunk within-source percentiles, in basis points.
    pub mean_pct_bp: i64,
    /// The operator's taste term, floored at zero and clamped (§58.1).
    pub taste_bonus_bp: i64,
    /// `mean_pct_bp + taste_bonus_bp` — orders this report, nothing else.
    pub score_bp: i64,
    /// When the snapshot row was written.
    pub computed_at: String,
}

/// What one detection run did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeystoneRun {
    /// Engaged rows the run read (endorsement above zero, author present).
    pub engaged_rows: i64,
    /// Distinct normalised authors among them, before the thresholds.
    pub authors_seen: i64,
    /// Authors that qualified.
    pub keystones: i64,
    /// Rows in `admin_taste_profile` the run could read a weight from.
    ///
    /// Zero is the honest answer on an instance nobody has tuned, and it is
    /// reported so a caller can tell "no taste configured" from "taste configured
    /// and irrelevant".
    pub taste_profile_rows: i64,
    /// When the snapshot was written.
    pub computed_at: String,
    /// The thresholds this run used, so a reader of the output can re-derive it.
    pub min_works: i64,
    /// See [`DEFAULT_MIN_PCT_BP`].
    pub min_pct_bp: i64,
}

/// One keystone author's conversation position (§58.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DroughtAuthor {
    /// The normalised author key.
    pub author_key: String,
    /// The author's own spelling.
    pub display_name: String,
    /// Engaged works.
    pub engaged_works: i64,
    /// How many of those carry at least one comment.
    pub works_with_conversation: i64,
    /// Comments on all of them.
    pub comments: i64,
}

/// §58.2's report, refusal included.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DroughtReport {
    /// Whether a drought could be distinguished from the corpus at all.
    pub measured: bool,
    /// Why not, when it could not.
    pub reason: Option<String>,
    /// Comments on works, instance-wide.
    pub comments_total: i64,
    /// Ratings, instance-wide.
    pub ratings_total: i64,
    /// The keystone authors' positions — empty when unmeasured.
    pub authors: Vec<DroughtAuthor>,
    /// When the report ran.
    pub computed_at: String,
}

/// A reader's routing to one work (§58.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Route {
    /// The routing row's id — the idempotency key's subject (§58.4).
    pub id: String,
    /// The reader.
    pub account_id: String,
    /// The catalogue row (`library_items.id`, as text).
    pub work_id: String,
    /// Title, read through a join so a routing can be rendered without one.
    pub title: String,
    /// Which archive it lives on.
    pub source_key: String,
    /// The keystone author it came from.
    pub author_key: String,
    /// Why this work — the same sentence the reader sees.
    pub reason: String,
    /// When the reader was routed.
    pub created_at: String,
    /// When they said they were done, or `None`.
    pub completed_at: Option<String>,
    /// What completion paid, in basis points, or `None`.
    pub credit_bp: Option<i64>,
}

/// The work a routing picked, with the numbers its reason quotes.
struct Picked {
    work_id: String,
    title: String,
    source_key: String,
    author_key: String,
    display_name: String,
    score_bp: i64,
    engaged_works: i64,
    routed: i64,
}

/// One engaged catalogue row, as the detection pass reads it.
struct EngagedRow {
    id: String,
    author_text: String,
    source_key: String,
    likes: i64,
}

/// `lower(trim(x))`, and the empty string for a blank author.
///
/// The catalogue already joins on this key (`?author=`), so an author filter and
/// a keystone key name the same author.
fn normalise_author(raw: &str) -> String {
    raw.trim().to_lowercase()
}

/// Percent, in basis points, rounded — the one place a float becomes storage.
fn to_bp(value: f64) -> i64 {
    (value * 10_000.0).round() as i64
}

/// The shrunk within-source percentile of `n` rows, index by index.
///
/// `raw = (rank + 0.5) / n` for a row's 0-based rank in ascending order, then
/// `0.5 + (raw - 0.5) * n / (n + 10)`. The second term is the whole point: a
/// one-row source lands at 0.5 + 0.5 * (1/11) = 0.545, not 0.75.
fn shrunk_pct(rank: usize, n: usize) -> f64 {
    let raw = (rank as f64 + 0.5) / n as f64;
    let shrink = n as f64 / (n as f64 + SHRINK);
    0.5 + (raw - 0.5) * shrink
}

// ---------------------------------------------------------------------------
// Reading the inputs
// ---------------------------------------------------------------------------

async fn engaged_rows(db: &Database) -> Result<Vec<EngagedRow>> {
    let sql = db.sql(
        "SELECT id, author_text, source_key, CAST(likes AS BIGINT) AS likes FROM library_items
          WHERE likes > 0 AND trim(author_text) <> ''",
        "SELECT id::text AS id, author_text, source_key, CAST(likes AS BIGINT) AS likes FROM library_items
          WHERE likes > 0 AND trim(author_text) <> ''",
    );
    let rows = match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(&sql)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            tuples!(rows, |r| EngagedRow {
                id: r.get::<String, _>("id"),
                author_text: r.get::<String, _>("author_text"),
                source_key: r.get::<String, _>("source_key"),
                likes: r.get::<i64, _>("likes"),
            })
        }
        Backend::Postgres => {
            let rows = sqlx::query(&sql)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?;
            tuples!(rows, |r| EngagedRow {
                id: r.get::<String, _>("id"),
                author_text: r.get::<String, _>("author_text"),
                source_key: r.get::<String, _>("source_key"),
                likes: r.get::<i64, _>("likes"),
            })
        }
    };
    Ok(rows)
}

/// Every operator weight that names an author, keyed by the author key.
///
/// §58.1 stores the operator's targets as `dimension_key = 'author:<key>'`; a row
/// naming a fandom or a tag is not an author weight and is not read here.
async fn taste_weights(db: &Database) -> Result<HashMap<String, f64>> {
    let sql = "SELECT lower(dimension_key) AS k, weight FROM admin_taste_profile";
    let pairs: Vec<(String, f64)> = match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(sql)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            tuples!(rows, |r| (
                r.get::<String, _>("k"),
                r.get::<f64, _>("weight")
            ))
        }
        Backend::Postgres => {
            let rows = sqlx::query(sql)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?;
            tuples!(rows, |r| (
                r.get::<String, _>("k"),
                r.get::<f64, _>("weight")
            ))
        }
    };
    let mut out: HashMap<String, f64> = HashMap::new();
    for (key, weight) in pairs {
        let Some(author) = key.strip_prefix("author:") else {
            continue;
        };
        *out.entry(author.trim().to_string()).or_insert(0.0) += weight;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// §58.1 — the keystone list
// ---------------------------------------------------------------------------

/// Recompute the keystone snapshot, replacing whatever was there.
///
/// Replacing rather than appending is the difference between a snapshot and a
/// history: a keystone list that keeps yesterday's authors would route readers to
/// an author who no longer qualifies, and nothing in the output would say which
/// run a row came from.
pub async fn recompute(db: &Database, min_works: i64, min_pct_bp: i64) -> Result<KeystoneRun> {
    let rows = engaged_rows(db).await?;
    let weights = taste_weights(db).await?;

    // Percentile within each source, on that source's own engaged rows.
    let mut by_source: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, r) in rows.iter().enumerate() {
        by_source.entry(r.source_key.as_str()).or_default().push(i);
    }
    let mut pct = vec![0.0_f64; rows.len()];
    for (_source, mut idx) in by_source {
        // Ascending likes, ties broken by id so two runs agree on a tie.
        idx.sort_by(|&a, &b| {
            rows[a]
                .likes
                .cmp(&rows[b].likes)
                .then_with(|| rows[a].id.cmp(&rows[b].id))
        });
        let n = idx.len();
        for (rank, &i) in idx.iter().enumerate() {
            pct[i] = shrunk_pct(rank, n);
        }
    }

    struct Acc {
        display: String,
        works: i64,
        sum: f64,
    }
    let mut acc: HashMap<String, Acc> = HashMap::new();
    for (i, r) in rows.iter().enumerate() {
        let key = normalise_author(&r.author_text);
        if key.is_empty() {
            continue;
        }
        let entry = acc.entry(key).or_insert_with(|| Acc {
            display: r.author_text.trim().to_string(),
            works: 0,
            sum: 0.0,
        });
        entry.works += 1;
        entry.sum += pct[i];
    }

    let authors_seen = acc.len() as i64;
    let mut kept: Vec<(String, String, i64, i64, i64, i64)> = Vec::new();
    for (key, a) in acc {
        if a.works < min_works {
            continue;
        }
        let mean_bp = to_bp(a.sum / a.works as f64);
        if mean_bp < min_pct_bp {
            continue;
        }
        // Floored at zero and clamped: the operator may raise a keystone, never
        // lower one, and never by more than a quarter of the scale.
        let bonus = weights
            .get(&key)
            .copied()
            .unwrap_or(0.0)
            .clamp(0.0, BONUS_CEILING);
        let bonus_bp = to_bp(bonus);
        kept.push((
            key,
            a.display,
            a.works,
            mean_bp,
            bonus_bp,
            mean_bp + bonus_bp,
        ));
    }
    // Descending score, then key, so the table and the API agree on ties.
    kept.sort_by(|a, b| b.5.cmp(&a.5).then_with(|| a.0.cmp(&b.0)));

    let computed_at = crate::identity::now_rfc3339();
    replace_snapshot(db, &kept, &computed_at).await?;

    Ok(KeystoneRun {
        engaged_rows: rows.len() as i64,
        authors_seen,
        keystones: kept.len() as i64,
        taste_profile_rows: weights.len() as i64,
        computed_at,
        min_works,
        min_pct_bp,
    })
}

async fn replace_snapshot(
    db: &Database,
    rows: &[(String, String, i64, i64, i64, i64)],
    computed_at: &str,
) -> Result<()> {
    let insert = db.sql(
        "INSERT INTO supply_keystones
            (author_key, display_name, engaged_works, mean_pct_bp, taste_bonus_bp, score_bp, computed_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        "INSERT INTO supply_keystones
            (author_key, display_name, engaged_works, mean_pct_bp, taste_bonus_bp, score_bp, computed_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    );
    match db.backend() {
        Backend::Sqlite => {
            let mut tx = db.sqlite_pool().expect("sqlite handle").begin().await?;
            sqlx::query("DELETE FROM supply_keystones")
                .execute(&mut *tx)
                .await?;
            for (key, display, works, mean_bp, bonus_bp, score_bp) in rows {
                sqlx::query(&insert)
                    .bind(key)
                    .bind(display)
                    .bind(works)
                    .bind(mean_bp)
                    .bind(bonus_bp)
                    .bind(score_bp)
                    .bind(computed_at)
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
        }
        Backend::Postgres => {
            let mut tx = db.postgres_pool().expect("postgres handle").begin().await?;
            sqlx::query("DELETE FROM supply_keystones")
                .execute(&mut *tx)
                .await?;
            for (key, display, works, mean_bp, bonus_bp, score_bp) in rows {
                sqlx::query(&insert)
                    .bind(key)
                    .bind(display)
                    .bind(works)
                    .bind(mean_bp)
                    .bind(bonus_bp)
                    .bind(score_bp)
                    .bind(computed_at)
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
        }
    }
    Ok(())
}

/// The stored keystone list, highest score first.
pub async fn keystones(db: &Database, limit: i64) -> Result<Vec<Keystone>> {
    let sql = db.sql(
        "SELECT author_key, display_name, CAST(engaged_works AS BIGINT) AS engaged_works,
                CAST(mean_pct_bp AS BIGINT) AS mean_pct_bp,
                CAST(taste_bonus_bp AS BIGINT) AS taste_bonus_bp,
                CAST(score_bp AS BIGINT) AS score_bp, computed_at
           FROM supply_keystones
          ORDER BY score_bp DESC, author_key ASC
          LIMIT ?",
        "SELECT author_key, display_name, CAST(engaged_works AS BIGINT) AS engaged_works,
                CAST(mean_pct_bp AS BIGINT) AS mean_pct_bp,
                CAST(taste_bonus_bp AS BIGINT) AS taste_bonus_bp,
                CAST(score_bp AS BIGINT) AS score_bp, computed_at
           FROM supply_keystones
          ORDER BY score_bp DESC, author_key ASC
          LIMIT $1",
    );
    let rows = match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(&sql)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            tuples!(rows, |r| Keystone {
                author_key: r.get::<String, _>("author_key"),
                display_name: r.get::<String, _>("display_name"),
                engaged_works: r.get::<i64, _>("engaged_works"),
                mean_pct_bp: r.get::<i64, _>("mean_pct_bp"),
                taste_bonus_bp: r.get::<i64, _>("taste_bonus_bp"),
                score_bp: r.get::<i64, _>("score_bp"),
                computed_at: r.get::<String, _>("computed_at"),
            })
        }
        Backend::Postgres => {
            let rows = sqlx::query(&sql)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?;
            tuples!(rows, |r| Keystone {
                author_key: r.get::<String, _>("author_key"),
                display_name: r.get::<String, _>("display_name"),
                engaged_works: r.get::<i64, _>("engaged_works"),
                mean_pct_bp: r.get::<i64, _>("mean_pct_bp"),
                taste_bonus_bp: r.get::<i64, _>("taste_bonus_bp"),
                score_bp: r.get::<i64, _>("score_bp"),
                computed_at: r.get::<String, _>("computed_at"),
            })
        }
    };
    Ok(rows)
}

// ---------------------------------------------------------------------------
// §58.2 — droughts, and the refusal when there is nothing to measure
// ---------------------------------------------------------------------------

async fn scalar_count(db: &Database, sql: &str) -> Result<i64> {
    Ok(match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar::<_, i64>(sql)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar::<_, i64>(sql)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    })
}

/// §58.2's report.
///
/// `measured: false` with a reason and no authors is the answer on a catalogue
/// that has never received a comment — see the module docs. It is deliberately
/// not "every engaged work is in drought", which is true and useless.
pub async fn drought(db: &Database, limit: i64) -> Result<DroughtReport> {
    let comments_total = scalar_count(
        db,
        "SELECT CAST(COUNT(*) AS BIGINT) FROM comments WHERE subject_type = 'work'",
    )
    .await?;
    let ratings_total = scalar_count(db, "SELECT CAST(COUNT(*) AS BIGINT) FROM work_ratings").await?;
    let computed_at = crate::identity::now_rfc3339();

    if comments_total == 0 && ratings_total == 0 {
        return Ok(DroughtReport {
            measured: false,
            reason: Some(
                "no comments and no ratings in this catalogue yet: a drought cannot be told \
                 apart from the whole corpus, so nothing is reported as one"
                    .to_string(),
            ),
            comments_total,
            ratings_total,
            authors: Vec::new(),
            computed_at,
        });
    }

    let list = keystones(db, limit).await?;
    if list.is_empty() {
        return Ok(DroughtReport {
            measured: true,
            reason: None,
            comments_total,
            ratings_total,
            authors: Vec::new(),
            computed_at,
        });
    }

    // One grouped query for every keystone author on the page, rather than one
    // query per author: the IN list is built from the page, which is bounded.
    let placeholders = vec!["?"; list.len()].join(", ");
    let sqlite_sql = format!(
            "SELECT lower(trim(li.author_text)) AS author_key,
                    CAST(COUNT(DISTINCT CASE WHEN c.id IS NOT NULL THEN li.id END) AS BIGINT) AS works_with_conversation,
                    CAST(COUNT(c.id) AS BIGINT) AS comments
               FROM library_items li
               LEFT JOIN comments c
                      ON c.subject_type = 'work' AND c.subject_id = li.id
              WHERE li.likes > 0 AND lower(trim(li.author_text)) IN ({placeholders})
              GROUP BY 1");
    let pg_sql = format!(
        "SELECT lower(trim(li.author_text)) AS author_key,
                CAST(COUNT(DISTINCT CASE WHEN c.id IS NOT NULL THEN li.id END) AS BIGINT) AS works_with_conversation,
                CAST(COUNT(c.id) AS BIGINT) AS comments
           FROM library_items li
           LEFT JOIN comments c
                  ON c.subject_type = 'work' AND c.subject_id = li.id::text
          WHERE li.likes > 0 AND lower(trim(li.author_text)) IN ({placeholders})
          GROUP BY 1"
    );
    let sql = db.sql(&sqlite_sql, &pg_sql);
    let found: Vec<(String, i64, i64)> = match db.backend() {
        Backend::Sqlite => {
            let mut q = sqlx::query(&sql);
            for k in &list {
                q = q.bind(&k.author_key);
            }
            let rows = q
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            tuples!(rows, |r| (
                r.get::<String, _>("author_key"),
                r.get::<i64, _>("works_with_conversation"),
                r.get::<i64, _>("comments")
            ))
        }
        Backend::Postgres => {
            let mut q = sqlx::query(&sql);
            for k in &list {
                q = q.bind(&k.author_key);
            }
            let rows = q
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?;
            tuples!(rows, |r| (
                r.get::<String, _>("author_key"),
                r.get::<i64, _>("works_with_conversation"),
                r.get::<i64, _>("comments")
            ))
        }
    };
    let found: HashMap<String, (i64, i64)> = found
        .into_iter()
        .map(|(k, works, comments)| (k, (works, comments)))
        .collect();

    Ok(DroughtReport {
        measured: true,
        reason: None,
        comments_total,
        ratings_total,
        authors: list
            .into_iter()
            .map(|k| {
                let (works_with_conversation, comments) =
                    found.get(&k.author_key).copied().unwrap_or((0, 0));
                DroughtAuthor {
                    author_key: k.author_key,
                    display_name: k.display_name,
                    engaged_works: k.engaged_works,
                    works_with_conversation,
                    comments,
                }
            })
            .collect(),
        computed_at,
    })
}

// ---------------------------------------------------------------------------
// §58.3 — routing
// ---------------------------------------------------------------------------

/// Route this reader to one work, or `None` when every keystone work has already
/// been offered to them.
///
/// `author_key` pins the author; `None` takes the highest-scoring keystone with a
/// work this reader has not seen. Either way the work with the fewest routings
/// comes first, then the most endorsed — so the supply is spread rather than
/// concentrated on whichever row ranks highest.
pub async fn route(db: &Database, account: &str, author_key: Option<&str>) -> Result<Option<Route>> {
    let (filter_sqlite, filter_pg) = if author_key.is_some() {
        ("AND k.author_key = ?", "AND k.author_key = $2")
    } else {
        ("", "")
    };
    let sqlite_sql = format!(
            "SELECT li.id AS work_id, li.title, li.source_key, k.author_key, k.display_name,
                    CAST(k.score_bp AS BIGINT) AS score_bp,
                    CAST(k.engaged_works AS BIGINT) AS engaged_works,
                    CAST((SELECT count(*) FROM supply_routing r WHERE r.work_id = li.id) AS BIGINT) AS routed
               FROM library_items li
               JOIN supply_keystones k ON k.author_key = lower(trim(li.author_text))
              WHERE li.likes > 0
                AND NOT EXISTS (SELECT 1 FROM supply_routing x
                                 WHERE x.work_id = li.id AND x.account_id = ?)
                {filter_sqlite}
              ORDER BY k.score_bp DESC, routed ASC, li.likes DESC, li.id ASC
              LIMIT 1");
    let pg_sql = format!(
            "SELECT li.id::text AS work_id, li.title, li.source_key, k.author_key, k.display_name,
                    CAST(k.score_bp AS BIGINT) AS score_bp,
                    CAST(k.engaged_works AS BIGINT) AS engaged_works,
                    CAST((SELECT count(*) FROM supply_routing r WHERE r.work_id = li.id::text) AS BIGINT) AS routed
               FROM library_items li
               JOIN supply_keystones k ON k.author_key = lower(trim(li.author_text))
              WHERE li.likes > 0
                AND NOT EXISTS (SELECT 1 FROM supply_routing x
                                 WHERE x.work_id = li.id::text AND x.account_id = $1)
                {filter_pg}
              ORDER BY k.score_bp DESC, routed ASC, li.likes DESC, li.id::text ASC
              LIMIT 1"
    );
    let sql = db.sql(&sqlite_sql, &pg_sql);
    let picked: Vec<Picked> = match db.backend() {
        Backend::Sqlite => {
            let mut q = sqlx::query(&sql).bind(account);
            if let Some(k) = author_key {
                q = q.bind(k);
            }
            let rows = q
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            tuples!(rows, |r| Picked {
                work_id: r.get::<String, _>("work_id"),
                title: r.get::<String, _>("title"),
                source_key: r.get::<String, _>("source_key"),
                author_key: r.get::<String, _>("author_key"),
                display_name: r.get::<String, _>("display_name"),
                score_bp: r.get::<i64, _>("score_bp"),
                engaged_works: r.get::<i64, _>("engaged_works"),
                routed: r.get::<i64, _>("routed"),
            })
        }
        Backend::Postgres => {
            let mut q = sqlx::query(&sql).bind(account);
            if let Some(k) = author_key {
                q = q.bind(k);
            }
            let rows = q
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?;
            tuples!(rows, |r| Picked {
                work_id: r.get::<String, _>("work_id"),
                title: r.get::<String, _>("title"),
                source_key: r.get::<String, _>("source_key"),
                author_key: r.get::<String, _>("author_key"),
                display_name: r.get::<String, _>("display_name"),
                score_bp: r.get::<i64, _>("score_bp"),
                engaged_works: r.get::<i64, _>("engaged_works"),
                routed: r.get::<i64, _>("routed"),
            })
        }
    };
    let Some(Picked {
        work_id,
        title,
        source_key,
        author_key: key,
        display_name: display,
        score_bp,
        engaged_works: works,
        routed,
    }) = picked.into_iter().next()
    else {
        return Ok(None);
    };

    let id = Uuid::new_v4().to_string();
    let created_at = crate::identity::now_rfc3339();
    let reason = format!(
        "{} is a keystone author here ({works} endorsed works, score {:.2}); this one has been \
         offered {routed} time(s)",
        display,
        score_bp as f64 / 10_000.0
    );

    let insert = db.sql(
        "INSERT INTO supply_routing
            (id, account_id, work_id, author_key, reason, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
        "INSERT INTO supply_routing
            (id, account_id, work_id, author_key, reason, created_at)
         VALUES ($1, $2, $3, $4, $5, $6)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&insert)
                .bind(&id)
                .bind(account)
                .bind(&work_id)
                .bind(&key)
                .bind(&reason)
                .bind(&created_at)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&insert)
                .bind(&id)
                .bind(account)
                .bind(&work_id)
                .bind(&key)
                .bind(&reason)
                .bind(&created_at)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }

    Ok(Some(Route {
        id,
        account_id: account.to_string(),
        work_id,
        title,
        source_key,
        author_key: key,
        reason,
        created_at,
        completed_at: None,
        credit_bp: None,
    }))
}

/// One routing row, read through its work so it renders without a second call.
async fn find_route(db: &Database, id: &str) -> Result<Option<Route>> {
    let sql = db.sql(
        "SELECT r.id, r.account_id, r.work_id, r.author_key, r.reason, r.created_at,
                r.completed_at, CAST(r.credit_bp AS BIGINT) AS credit_bp,
                COALESCE(li.title, '') AS title, COALESCE(li.source_key, '') AS source_key
           FROM supply_routing r
           LEFT JOIN library_items li ON li.id = r.work_id
          WHERE r.id = ?",
        "SELECT r.id, r.account_id, r.work_id, r.author_key, r.reason, r.created_at,
                r.completed_at, CAST(r.credit_bp AS BIGINT) AS credit_bp,
                COALESCE(li.title, '') AS title, COALESCE(li.source_key, '') AS source_key
           FROM supply_routing r
           LEFT JOIN library_items li ON li.id::text = r.work_id
          WHERE r.id = $1",
    );
    let rows: Vec<Route> = match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(&sql)
                .bind(id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            tuples!(rows, |r| route_of!(r))
        }
        Backend::Postgres => {
            let rows = sqlx::query(&sql)
                .bind(id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?;
            tuples!(rows, |r| route_of!(r))
        }
    };
    Ok(rows.into_iter().next())
}

/// This reader's routings, newest first.
pub async fn routes_for(db: &Database, account: &str, limit: i64) -> Result<Vec<Route>> {
    let sql = db.sql(
        "SELECT r.id, r.account_id, r.work_id, r.author_key, r.reason, r.created_at,
                r.completed_at, CAST(r.credit_bp AS BIGINT) AS credit_bp,
                COALESCE(li.title, '') AS title, COALESCE(li.source_key, '') AS source_key
           FROM supply_routing r
           LEFT JOIN library_items li ON li.id = r.work_id
          WHERE r.account_id = ?
          ORDER BY r.created_at DESC, r.id ASC
          LIMIT ?",
        "SELECT r.id, r.account_id, r.work_id, r.author_key, r.reason, r.created_at,
                r.completed_at, CAST(r.credit_bp AS BIGINT) AS credit_bp,
                COALESCE(li.title, '') AS title, COALESCE(li.source_key, '') AS source_key
           FROM supply_routing r
           LEFT JOIN library_items li ON li.id::text = r.work_id
          WHERE r.account_id = $1
          ORDER BY r.created_at DESC, r.id ASC
          LIMIT $2",
    );
    let rows: Vec<Route> = match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(&sql)
                .bind(account)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            tuples!(rows, |r| route_of!(r))
        }
        Backend::Postgres => {
            let rows = sqlx::query(&sql)
                .bind(account)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?;
            tuples!(rows, |r| route_of!(r))
        }
    };
    Ok(rows)
}

/// §58.4 — mark a routing complete and pay the patronage, once.
///
/// Returns `None` when the routing does not exist **or belongs to another
/// account**, which is one answer on purpose: a caller probing for someone else's
/// id learns that it does not exist for them, and nothing else.
pub async fn complete(db: &Database, account: &str, route_id: &str) -> Result<Option<Route>> {
    let Some(existing) = find_route(db, route_id).await? else {
        return Ok(None);
    };
    if existing.account_id != account {
        return Ok(None);
    }
    if existing.completed_at.is_some() {
        // Paid already: the row carries the amount and the second call is a read.
        return Ok(Some(existing));
    }

    // Pay first with an idempotency key derived from the routing id, so a retry
    // after a crash is the same transaction rather than a second grant.
    crate::economy::post_transaction(
        db,
        darkhold_domain::economy::TxnType::Grant,
        &format!("supply-route:{route_id}"),
        &format!("supply:{}", existing.work_id),
        &[(
            account.to_string(),
            "credit".to_string(),
            PATRONAGE_BP,
        )],
    )
    .await?;

    let completed_at = crate::identity::now_rfc3339();
    let sql = db.sql(
        "UPDATE supply_routing SET completed_at = ?, credit_bp = ? WHERE id = ?",
        "UPDATE supply_routing SET completed_at = $1, credit_bp = $2 WHERE id = $3",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&completed_at)
                .bind(PATRONAGE_BP)
                .bind(route_id)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&completed_at)
                .bind(PATRONAGE_BP)
                .bind(route_id)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }

    find_route(db, route_id).await
}
