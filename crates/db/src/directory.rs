//! Resource Directory repository (spec §39). Both dialects.
//!
//! Removal is a hard DELETE of the entry row: vote rows are left in place
//! for audit, and every read path joins on live entries so orphans are
//! invisible. A resubmission of the same URL starts at zero because the
//! entry row (and its score) is gone.
//!
//! SQL is written SQLite-style (`?`) and rewritten for PostgreSQL by
//! [`Database::sql`], per crate convention.

use crate::{Backend, Database};
use anyhow::Result;
use lorehaven_domain::vote_decay::{self, Decay};
use lorehaven_domain::vote_decay_sql::{self, Dialect};
use serde::Serialize;
use sqlx::FromRow;

/// One directory list (spec §39.1).
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct DirectoryList {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub kind: String,
    pub is_instance_list: bool,
    pub position: i64,
    pub created_by: String,
    pub created_at: String,
}

/// One directory entry with its denormalised score (spec §39.1–39.2).
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct DirectoryEntry {
    pub id: String,
    pub list_id: String,
    pub kind: String,
    pub category: String,
    pub title: String,
    pub url: String,
    pub description: String,
    pub ref_id: Option<String>,
    pub tags_json: String,
    pub submitted_by: String,
    pub approved_by: Option<String>,
    pub score: f64,
    pub created_at: String,
    /// Set for the requesting viewer: 1, -1 or null. Never a weight.
    #[sqlx(default)]
    #[serde(default)]
    pub my_vote: Option<i64>,
}

impl DirectoryEntry {
    /// The entry's tags, parsed from their JSON column.
    pub fn tags(&self) -> Vec<String> {
        serde_json::from_str(&self.tags_json).unwrap_or_default()
    }
}

/// Filters for [`list_entries`].
#[derive(Debug, Clone, Default)]
pub struct DirectoryEntryFilter {
    pub list_id: Option<String>,
    pub category: Option<String>,
    pub q: Option<String>,
    pub sort: DirectorySort,
    pub limit: i64,
    pub offset: i64,
    /// The requesting account, if signed in.
    pub viewer: Option<String>,
    pub is_operator: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DirectorySort {
    #[default]
    Top,
    New,
}

/// Create a list.
#[allow(clippy::too_many_arguments)]
pub async fn create_list(
    db: &Database,
    id: &str,
    slug: &str,
    title: &str,
    description: &str,
    kind: &str,
    is_instance_list: bool,
    position: i64,
    created_by: &str,
    created_at: &str,
) -> Result<()> {
    let sql = db.sql(
        "INSERT INTO directory_lists (id, slug, title, description, kind, is_instance_list, position, created_by, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        "INSERT INTO directory_lists (id, slug, title, description, kind, is_instance_list, position, created_by, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(id)
                .bind(slug)
                .bind(title)
                .bind(description)
                .bind(kind)
                .bind(is_instance_list)
                .bind(position)
                .bind(created_by)
                .bind(created_at)
                .bind(created_at)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(id)
                .bind(slug)
                .bind(title)
                .bind(description)
                .bind(kind)
                .bind(is_instance_list)
                .bind(position)
                .bind(created_by)
                .bind(created_at)
                .bind(created_at)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// All lists, ordered for display: instance lists first by position, then
/// the rest by creation.
pub async fn list_lists(db: &Database) -> Result<Vec<DirectoryList>> {
    let sql = db.sql(
        "SELECT id, slug, title, description, kind, is_instance_list, position, created_by, created_at \
         FROM directory_lists ORDER BY is_instance_list DESC, position ASC, created_at ASC",
        "SELECT id, slug, title, description, kind, is_instance_list, CAST(position AS BIGINT), created_by, created_at \
         FROM directory_lists ORDER BY is_instance_list DESC, position ASC, created_at ASC",
    );
    let rows: Vec<DirectoryList> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows)
}

/// Look up one list by slug.
pub async fn list_by_slug(db: &Database, slug: &str) -> Result<Option<DirectoryList>> {
    let sql = db.sql(
        "SELECT id, slug, title, description, kind, is_instance_list, position, created_by, created_at FROM directory_lists WHERE slug = ?",
        // `position` is INTEGER; the struct field is i64, so sqlx needs the cast.
        "SELECT id, slug, title, description, kind, is_instance_list, CAST(position AS BIGINT), created_by, created_at \
         FROM directory_lists WHERE slug = $1",
    );
    let row: Option<DirectoryList> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(slug)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(slug)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row)
}

/// Submit an entry (pending approval by default, spec §39.3).
#[allow(clippy::too_many_arguments)]
pub async fn submit_entry(
    db: &Database,
    id: &str,
    list_id: &str,
    kind: &str,
    category: &str,
    title: &str,
    url: &str,
    description: &str,
    ref_id: Option<&str>,
    tags: &[String],
    submitted_by: &str,
    now: &str,
) -> Result<()> {
    let tags_json = serde_json::to_string(tags)?;
    let sql = db.sql(
        "INSERT INTO directory_entries (id, list_id, kind, category, title, url, description, ref_id, tags_json, submitted_by, approved_by, removed_at, score, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, 0, ?, ?)",
        "INSERT INTO directory_entries (id, list_id, kind, category, title, url, description, ref_id, tags_json, submitted_by, approved_by, removed_at, score, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, 0, ?, ?)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(id)
                .bind(list_id)
                .bind(kind)
                .bind(category)
                .bind(title)
                .bind(url)
                .bind(description)
                .bind(ref_id)
                .bind(&tags_json)
                .bind(submitted_by)
                .bind(now)
                .bind(now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(id)
                .bind(list_id)
                .bind(kind)
                .bind(category)
                .bind(title)
                .bind(url)
                .bind(description)
                .bind(ref_id)
                .bind(&tags_json)
                .bind(submitted_by)
                .bind(now)
                .bind(now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Approve a pending entry. Returns false when the entry does not exist or
/// is already approved (idempotent-false, not an error).
pub async fn approve_entry(db: &Database, id: &str, operator_id: &str, now: &str) -> Result<bool> {
    let sql = db.sql(
        "UPDATE directory_entries SET approved_by = ?, updated_at = ? WHERE id = ? AND approved_by IS NULL",
        "UPDATE directory_entries SET approved_by = ?, updated_at = ? WHERE id = ? AND approved_by IS NULL",
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(operator_id)
            .bind(now)
            .bind(id)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(operator_id)
            .bind(now)
            .bind(id)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// Remove an entry: hard DELETE, votes stay for audit (module doc).
pub async fn remove_entry(db: &Database, id: &str) -> Result<bool> {
    let sql = db.sql(
        "DELETE FROM directory_entries WHERE id = ?",
        "DELETE FROM directory_entries WHERE id = ?",
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(id)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(id)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// List entries. The visibility rule (approved, or the viewer's own
/// pending, or everything for the operator — spec §39.3) is expressed in
/// SQL so pagination counts agree with the page.
/// Entries for the list, ranked and scored with vote decay applied.
///
/// This is the list the product should serve. [`list_entries`] remains for
/// callers that genuinely want the stored snapshot — nothing in the request
/// path does any more.
pub async fn list_entries_with_decay(
    db: &Database,
    filter: &DirectoryEntryFilter,
    decay: &Decay,
) -> Result<Vec<DirectoryEntry>> {
    list_entries_inner(db, filter, Some(decay)).await
}

/// Entries ranked by the stored, undecayed `score` column.
///
/// Kept because the denormalised column is still what the vote transaction
/// writes, and the two are asserted equal for a below-threshold entry in
/// `vote_list_ranking`. Prefer [`list_entries_with_decay`].
pub async fn list_entries(
    db: &Database,
    filter: &DirectoryEntryFilter,
) -> Result<Vec<DirectoryEntry>> {
    list_entries_inner(db, filter, None).await
}

async fn list_entries_inner(
    db: &Database,
    filter: &DirectoryEntryFilter,
    decay: Option<&Decay>,
) -> Result<Vec<DirectoryEntry>> {
    let mut where_parts: Vec<String> = vec!["e.removed_at IS NULL".to_string()];
    if let Some(list_id) = &filter.list_id {
        where_parts.push(format!("e.list_id = '{}'", escape(list_id)));
    }
    if let Some(category) = &filter.category {
        if !category.is_empty() {
            where_parts.push(format!(
                "lower(e.category) = '{}'",
                escape(&category.to_lowercase())
            ));
        }
    }
    if let Some(q) = filter.q.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        let needle = format!("%{}%", escape(&q.to_lowercase()));
        where_parts.push(format!(
            "(lower(e.title) LIKE '{needle}' OR lower(e.description) LIKE '{needle}')"
        ));
    }
    match (&filter.viewer, filter.is_operator) {
        (Some(v), false) => where_parts.push(format!(
            "(e.approved_by IS NOT NULL OR e.submitted_by = '{}')",
            escape(v)
        )),
        (None, false) => where_parts.push("e.approved_by IS NOT NULL".to_string()),
        (_, true) => {}
    }
    // The decayed score is a correlated subquery per entry, and it is used in
    // both the projection and the ORDER BY. PostgreSQL will not let a SELECT
    // alias be referenced in ORDER BY when it is an expression over a
    // subquery, so the expression is written out twice rather than aliased --
    // the two must agree, and `vote_list_ranking` fails if they ever do not.
    //
    // The undecayed column is used verbatim when decay is off, so turning it
    // off costs nothing and the statement stays as simple as it was.
    let (score_expr, order_score) = match decay {
        None => (
            "CAST(e.score AS DOUBLE PRECISION)".to_string(),
            "e.score".to_string(),
        ),
        Some(cfg) => {
            let d = match db.backend() {
                Backend::Sqlite => Dialect::Sqlite,
                Backend::Postgres => Dialect::Postgres,
            };
            // A correlated subquery cannot bind `e.id` as a parameter, so the
            // column reference is written into the statement. It is a
            // reference, not a value: quoting it would make the subquery
            // compare every row against the literal string "e.id" and silently
            // return the same count for the whole list.
            let per_entry = |id_expr: &str| -> String {
                // The threshold is per entry, so the count and the sum are one
                // CASE over the entry's own votes rather than two passes.
                let sum = vote_decay_sql::decayed_score_sum_sql(cfg, d, id_expr);
                let count = vote_decay_sql::vote_count_sql(d, id_expr, true);
                format!(
                    "CASE WHEN {count} >= {} THEN {sum} ELSE CAST((SELECT COALESCE(SUM(v.vote_value * v.base_weight), 0) FROM directory_votes{alias} WHERE v.entry_id = {id_expr}) AS DOUBLE PRECISION) END",
                    cfg.min_votes,
                    alias = match d {
                        Dialect::Sqlite => " v",
                        Dialect::Postgres => " AS v",
                    },
                )
            };
            (per_entry("e.id"), per_entry("e.id"))
        }
    };
    let order = match filter.sort {
        DirectorySort::Top => format!("{order_score} DESC, e.created_at ASC"),
        DirectorySort::New => "e.created_at DESC".to_string(),
    };
    let sql = format!(
        "SELECT e.id, e.list_id, e.kind, e.category, e.title, e.url, e.description, e.ref_id, e.tags_json, e.submitted_by, e.approved_by, CAST({score_expr} AS DOUBLE PRECISION) AS score, e.created_at \
         FROM directory_entries e WHERE {} ORDER BY {order} LIMIT {} OFFSET {}",
        where_parts.join(" AND "),
        filter.limit.max(1),
        filter.offset.max(0)
    );
    let rows: Vec<DirectoryEntry> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    // Attach the viewer's own vote state without ever exposing weights.
    let mut entries = rows;
    if let Some(viewer) = &filter.viewer {
        for entry in &mut entries {
            entry.my_vote = my_vote(db, &entry.id, viewer).await?;
        }
    }
    Ok(entries)
}

/// One entry, same visibility rule as the list (spec §39.3).
pub async fn get_entry(
    db: &Database,
    id: &str,
    viewer: Option<&str>,
    is_operator: bool,
) -> Result<Option<DirectoryEntry>> {
    let sql = db.sql(
        "SELECT id, list_id, kind, category, title, url, description, ref_id, tags_json, submitted_by, approved_by, score, created_at \
         FROM directory_entries WHERE id = ? AND removed_at IS NULL",
        // `score` is REAL, i.e. FLOAT4, and the field is f64 (FLOAT8). The PG arm
        // used to be a verbatim copy of the SQLite one, so it also kept the `?`.
        "SELECT id, list_id, kind, category, title, url, description, ref_id, tags_json, submitted_by, approved_by, \
                CAST(score AS DOUBLE PRECISION) AS score, created_at \
         FROM directory_entries WHERE id = $1 AND removed_at IS NULL",
    );
    let row: Option<DirectoryEntry> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    let Some(mut entry) = row else {
        return Ok(None);
    };
    let visible =
        entry.approved_by.is_some() || is_operator || viewer == Some(entry.submitted_by.as_str());
    if !visible {
        return Ok(None);
    }
    if let Some(v) = viewer {
        entry.my_vote = my_vote(db, id, v).await?;
    }
    Ok(Some(entry))
}

/// The viewer's live vote on an entry (1/-1), never the weight.
/// Vote counts for a page of entries, in one query.
///
/// The list needs each entry's count to decide whether that entry is over the
/// `min_votes` threshold. Asking per entry is a query per row, so this groups
/// by entry id and returns the whole page at once.
///
/// Entries with no votes are absent from the map rather than mapped to zero.
/// A caller that treats "absent" as zero gets the right answer -- an entry
/// with no votes is below any threshold -- and the map does not carry a
/// misleading `0` next to a real count.
pub async fn vote_counts(
    db: &Database,
    entry_ids: &[String],
) -> Result<std::collections::HashMap<String, i64>> {
    if entry_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    // `IN (...)` with bound placeholders rather than an interpolated list, so
    // the ids are values the database parses rather than SQL it trusts. The
    // placeholders are generated, not user-supplied, and the number of them
    // is bounded by the page size, which the route already clamps.
    let n = entry_ids.len();
    // A `Query` is generic over its database, so the statement and its binds
    // are built inside each arm rather than shared: one built for the SQLite
    // pool cannot execute on the PostgreSQL one, and the resulting error names
    // the pool rather than the cause.
    let rows: Vec<(String, i64)> = match db.backend() {
        Backend::Sqlite => {
            let marks = vec!["?"; n].join(", ");
            let sql = format!(
                "SELECT CAST(entry_id AS TEXT) AS entry_id, CAST(COUNT(*) AS BIGINT) AS n FROM directory_votes WHERE entry_id IN ({marks}) GROUP BY entry_id"
            );
            let mut query = sqlx::query_as::<_, (String, i64)>(&sql);
            for id in entry_ids {
                query = query.bind(id);
            }
            query.fetch_all(db.sqlite_pool().expect("sqlite")).await?
        }
        Backend::Postgres => {
            let marks = (1..=n)
                .map(|i| format!("${i}"))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT CAST(entry_id AS TEXT) AS entry_id, CAST(COUNT(*) AS BIGINT) AS n FROM directory_votes WHERE entry_id IN ({marks}) GROUP BY entry_id"
            );
            let mut query = sqlx::query_as::<_, (String, i64)>(&sql);
            for id in entry_ids {
                query = query.bind(id);
            }
            query
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows.into_iter().collect())
}

pub async fn my_vote(db: &Database, entry_id: &str, account_id: &str) -> Result<Option<i64>> {
    let sql = db.sql(
        "SELECT vote_value FROM directory_votes WHERE entry_id = ? AND account_id = ?",
        // `vote_value` is INTEGER; the `?` had also never been rewritten.
        "SELECT CAST(vote_value AS BIGINT) FROM directory_votes WHERE entry_id = $1 AND account_id = $2",
    );
    let row: Option<(i64,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(entry_id)
                .bind(account_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(entry_id)
                .bind(account_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.map(|(v,)| v))
}

/// Set, toggle or flip a vote and recompute the score in the SAME
/// transaction (spec §39.4: the list never shows a stale score).
/// Returns the new score and whether a live vote remains.
/// The transactional core of [`set_vote`].
///
/// A macro because the two executors want different placeholder syntax, and a
/// macro lets one body serve both. The original version used `?` throughout on
/// the assumption that sqlx rewrites it per-dialect. It does not: `?` is SQLite
/// syntax, and on PostgreSQL `WHERE entry_id = ? AND account_id = ?` is a syntax
/// error, so every entry vote on a PostgreSQL instance was a 500. The macro now
/// takes both forms, exactly like [`crate::Database::sql`].
macro_rules! vote_tx {
    ($db:expr, $tx:expr, $entry_id:expr, $account_id:expr, $value:expr, $weight:expr, $now:expr) => {{
        // `vote_value` is INTEGER (INT4); reading it as i64 needs the cast on
        // both engines, so both forms carry it.
        let existing: Option<(i64,)> = sqlx::query_as(&$db.sql(
            "SELECT CAST(vote_value AS BIGINT) FROM directory_votes WHERE entry_id = ? AND account_id = ?",
            "SELECT CAST(vote_value AS BIGINT) FROM directory_votes WHERE entry_id = $1 AND account_id = $2",
        ))
        .bind($entry_id)
        .bind($account_id)
        .fetch_optional(&mut *$tx)
        .await?;
        let live: bool = match existing {
            // `value: 0` withdraws. It is a distinct case rather than the
            // `current == value` branch below because the column is CHECK'd to
            // IN (-1, 1): a zero can never be stored, only deleted.
            //
            // Withdrawing a vote that is not there is not an error. A client
            // that renders an un-vote control and double-clicks it would
            // otherwise get a failure from a state that is already correct.
            _ if $value == 0 => {
                sqlx::query(&$db.sql(
                    "DELETE FROM directory_votes WHERE entry_id = ? AND account_id = ?",
                    "DELETE FROM directory_votes WHERE entry_id = $1 AND account_id = $2",
                ))
                .bind($entry_id)
                .bind($account_id)
                .execute(&mut *$tx)
                .await?;
                false
            }
            Some((current,)) if current == $value => {
                // Same value again: refresh, do not toggle off.
                //
                // This was a DELETE, which was right for a permanent vote --
                // clicking up twice should undo the vote -- and it is wrong for
                // a decaying one. "I still think this is good" has to be
                // expressible, and the only way to say it under a toggle was to
                // vote down and back up, which records a down-vote that never
                // happened and, on a list where the direction matters, briefly
                // sinks the entry. The rule being implemented is "voting each
                // day weights slightly more than each week"; a re-vote that
                // deletes the vote makes that sentence unimplementable.
                //
                // `base_weight` is rewritten too, not just the timestamp: it is
                // derived from the voter's trust and taste at the moment they
                // voted, and a re-vote is a fresh act of voting.
                sqlx::query(&$db.sql(
                    "UPDATE directory_votes SET base_weight = ?, voted_at = ? WHERE entry_id = ? AND account_id = ?",
                    "UPDATE directory_votes SET base_weight = $1, voted_at = $2 WHERE entry_id = $3 AND account_id = $4",
                ))
                .bind($weight).bind($now).bind($entry_id).bind($account_id)
                .execute(&mut *$tx)
                .await?;
                true
            }
            Some(_) => {
                // Other value: flip in place.
                sqlx::query(&$db.sql(
                    "UPDATE directory_votes SET vote_value = ?, base_weight = ?, voted_at = ? WHERE entry_id = ? AND account_id = ?",
                    "UPDATE directory_votes SET vote_value = $1, base_weight = $2, voted_at = $3 WHERE entry_id = $4 AND account_id = $5",
                ))
                .bind($value).bind($weight).bind($now).bind($entry_id).bind($account_id)
                .execute(&mut *$tx)
                .await?;
                true
            }
            None => {
                sqlx::query(&$db.sql(
                    "INSERT INTO directory_votes (entry_id, account_id, vote_value, base_weight, voted_at) VALUES (?, ?, ?, ?, ?)",
                    "INSERT INTO directory_votes (entry_id, account_id, vote_value, base_weight, voted_at) VALUES ($1, $2, $3, $4, $5)",
                ))
                .bind($entry_id).bind($account_id).bind($value).bind($weight).bind($now)
                .execute(&mut *$tx)
                .await?;
                true
            }
        };
        // `score` is REAL, and SUM() of a REAL is double precision on PostgreSQL
        // but REAL on SQLite, so the assignment is cast to the column's own type
        // rather than the expression's.
        sqlx::query(&$db.sql(
            "UPDATE directory_entries SET score = CAST((SELECT COALESCE(SUM(vote_value * base_weight), 0) FROM directory_votes WHERE entry_id = ?) AS REAL), updated_at = ? WHERE id = ?",
            "UPDATE directory_entries SET score = CAST((SELECT COALESCE(SUM(vote_value * base_weight), 0) FROM directory_votes WHERE entry_id = $1) AS REAL), updated_at = $2 WHERE id = $3",
        ))
        .bind($entry_id)
        .bind($now)
        .bind($entry_id)
        .execute(&mut *$tx)
        .await?;
        Ok::<bool, anyhow::Error>(live)
    }};
}

/// Record a vote, or refresh the one already there, and return the entry's
/// current decayed score.
///
/// Voting the same value again **refreshes** the vote: it rewrites
/// `base_weight` and `voted_at` on the existing row rather than deleting it.
/// See the `vote_tx!` macro for why that is a change from the old
/// toggle-off behaviour.
///
/// Returns `(score, live)`. `live` is always true — a vote either exists or
/// it does not, and there is no longer an off state reachable by voting the
/// same way twice. It is kept because the route's response type carries it and
/// an unvote path belongs to the product, not to this function's signature.
pub async fn set_vote(
    db: &Database,
    entry_id: &str,
    account_id: &str,
    value: i64,
    weight: f64,
    now: &str,
    decay: &Decay,
) -> Result<(f64, bool)> {
    // The vote change and the score recompute share one transaction
    // (spec §39.4), so the list can never show a stale score.
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let mut tx = pool.begin().await?;
            let live = vote_tx!(db, tx, entry_id, account_id, value, weight, now)?;
            tx.commit().await?;
            // The decayed score, not `entry_score`. The denormalised column is
            // the undecayed sum and is no longer what the list ranks by, so
            // returning it would hand the client a number the directory
            // immediately contradicts.
            let score = decayed_score(db, entry_id, decay).await?;
            Ok((score, live))
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let mut tx = pool.begin().await?;
            let live = vote_tx!(db, tx, entry_id, account_id, value, weight, now)?;
            tx.commit().await?;
            let score = decayed_score(db, entry_id, decay).await?;
            Ok((score, live))
        }
    }
}

/// How many votes an entry has, for the `min_votes` threshold.
///
/// Every row, not the ones still above zero — see
/// [`vote_decay_sql::vote_count_sql`] for why that distinction is the whole
/// difference between a smooth curve and a cliff.
pub async fn vote_count(db: &Database, entry_id: &str) -> Result<i64> {
    let d = match db.backend() {
        Backend::Sqlite => Dialect::Sqlite,
        Backend::Postgres => Dialect::Postgres,
    };
    let sql = match db.backend() {
        Backend::Sqlite => vote_decay_sql::vote_count_sql(d, "?", false),
        Backend::Postgres => vote_decay_sql::vote_count_sql(d, "$1", false),
    };
    let row: (i64,) = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(entry_id)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(entry_id)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.0)
}

/// An entry's score, recomputed from its votes with decay applied.
///
/// The stored `directory_entries.score` column is a denormalised cache that a
/// permanent vote could keep correct inside the vote transaction. A decaying
/// vote cannot: the score moves every second without any vote happening, so
/// the column is wrong the moment the transaction commits. This is the read
/// path §39.4's "the list never shows a stale score" actually requires once
/// decay is on.
///
/// Below `min_votes` votes the entry is exempt and every vote counts at its base
/// weight, which is what keeps a new submission viable — its three votes are
/// the only ranking signal it has, and decaying them would be erasure rather
/// than moderation.
///
/// The threshold is on vote *rows*, not on votes still above zero, so an entry
/// does not fall out of the decaying set as its votes expire. See
/// [`vote_decay::should_decay`] for what the other version did.
pub async fn decayed_score(db: &Database, entry_id: &str, decay: &Decay) -> Result<f64> {
    if !vote_decay::should_decay(vote_count(db, entry_id).await?, decay) {
        return entry_score_sum(db, entry_id, None).await;
    }
    entry_score_sum(db, entry_id, Some(decay)).await
}

/// `SUM(vote_value × base_weight)` for one entry, decayed if `decay` is given.
///
/// `SUM()` over a float column is `double precision` on PostgreSQL and REAL on
/// SQLite, and the cast is what makes the result decodable into an `f64` on
/// both — without it this is a 500 on PostgreSQL and fine on SQLite.
async fn entry_score_sum(db: &Database, entry_id: &str, decay: Option<&Decay>) -> Result<f64> {
    let d = match db.backend() {
        Backend::Sqlite => Dialect::Sqlite,
        Backend::Postgres => Dialect::Postgres,
    };
    let param = if db.backend() == Backend::Postgres {
        "$1"
    } else {
        "?"
    };
    let expr = match decay {
        Some(cfg) => vote_decay_sql::decayed_score_sum_sql(cfg, d, param),
        // Decay off, or an exempt entry: the plain permanent sum, spelled out
        // rather than as a "1.0 multiplier" so the undecayed path stays the
        // query it was before this feature existed.
        None => format!(
            "COALESCE((SELECT SUM(v.vote_value * v.base_weight) FROM directory_votes v WHERE v.entry_id = {param}), 0)"
        ),
    };
    let sql = format!("SELECT CAST({expr} AS DOUBLE PRECISION)");
    let row: (f64,) = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(entry_id)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(entry_id)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.0)
}

/// The denormalised score of an entry.
pub async fn entry_score(db: &Database, entry_id: &str) -> Result<f64> {
    let sql = db.sql(
        "SELECT score FROM directory_entries WHERE id = ?",
        // `score` is REAL (FLOAT4); sqlx will not decode that into the f64 the
        // tuple asks for. The PG arm was a verbatim copy of the SQLite one.
        "SELECT CAST(score AS DOUBLE PRECISION) FROM directory_entries WHERE id = $1",
    );
    let row: Option<(f64,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(entry_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(entry_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.map(|(s,)| s).unwrap_or(0.0))
}

/// Approved-entry counts per category (spec §39.2 tabs).
pub async fn category_counts(db: &Database) -> Result<Vec<(String, i64)>> {
    let sql = db.sql(
        "SELECT category, COUNT(*) FROM directory_entries WHERE approved_by IS NOT NULL AND removed_at IS NULL AND category <> '' GROUP BY category ORDER BY COUNT(*) DESC, category ASC",
        "SELECT category, COUNT(*) FROM directory_entries WHERE approved_by IS NOT NULL AND removed_at IS NULL AND category <> '' GROUP BY category ORDER BY COUNT(*) DESC, category ASC",
    );
    let rows: Vec<(String, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows)
}

/// The operator's pending queue.
pub async fn pending_entries(db: &Database) -> Result<Vec<DirectoryEntry>> {
    let sql = db.sql(
        "SELECT id, list_id, kind, category, title, url, description, ref_id, tags_json, submitted_by, approved_by, score, created_at \
         FROM directory_entries WHERE approved_by IS NULL AND removed_at IS NULL ORDER BY created_at ASC LIMIT 200",
        "SELECT id, list_id, kind, category, title, url, description, ref_id, tags_json, submitted_by, approved_by, \
                CAST(score AS DOUBLE PRECISION) AS score, created_at \
         FROM directory_entries WHERE approved_by IS NULL AND removed_at IS NULL ORDER BY created_at ASC LIMIT 200",
    );
    let rows: Vec<DirectoryEntry> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows)
}

/// Escape a literal for interpolation into a SQL string. Used only for
/// values that cannot be bound parameters (dynamic WHERE fragments);
/// bound parameters remain the default everywhere else.
fn escape(s: &str) -> String {
    s.replace('\'', "''")
}

// --- Test-only helpers -------------------------------------------------
//
// These exist so the app-layer milestone tests can arrange trust levels
// and inspect audit rows without duplicating dialect SQL. They are
// compiled unconditionally (they are tiny and pure), but they are not
// part of the public surface: they are `#[doc(hidden)]`.

/// Set an account's trust level directly (tests only). Writes the
/// `trust_levels` ledger the way `governance::set_trust` does, because the
/// routes read trust through `governance::trust_for`.
#[doc(hidden)]
pub async fn set_account_trust_for_tests(db: &Database, account_id: &str, level: u8) {
    // Bind as i64: PostgreSQL has no unsigned scalar types, so binding a
    // u8 would pin the query to the SQLite dialect.
    let level = i64::from(level);
    let sql = db.sql(
        "INSERT INTO trust_levels (account, level, computed_at, basis) VALUES (?, ?, ?, 'tests') \
         ON CONFLICT(account) DO UPDATE SET level=excluded.level, computed_at=excluded.computed_at, basis=excluded.basis",
        "INSERT INTO trust_levels (account, level, computed_at, basis) VALUES (?, ?, ?, 'tests') \
         ON CONFLICT(account) DO UPDATE SET level=excluded.level, computed_at=excluded.computed_at, basis=excluded.basis",
    );
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(account_id)
                .bind(level)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("set trust level");
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(account_id)
                .bind(level)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("set trust level");
        }
    }
}

/// All votes on an entry (tests only; audit inspection).
#[doc(hidden)]
pub async fn votes_for_tests(db: &Database, entry_id: &str) -> Vec<(String, i64)> {
    let sql = db.sql(
        "SELECT account_id, vote_value FROM directory_votes WHERE entry_id = ?",
        "SELECT account_id, CAST(vote_value AS BIGINT) FROM directory_votes WHERE entry_id = $1",
    );
    match db.backend() {
        Backend::Sqlite => sqlx::query_as(&sql)
            .bind(entry_id)
            .fetch_all(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("votes"),
        Backend::Postgres => sqlx::query_as(&sql)
            .bind(entry_id)
            .fetch_all(db.postgres_pool().expect("postgres"))
            .await
            .expect("votes"),
    }
}
