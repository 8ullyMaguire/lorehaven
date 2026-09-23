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
        "SELECT id, slug, title, description, kind, is_instance_list, position, created_by, created_at \
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
        "SELECT id, slug, title, description, kind, is_instance_list, position, created_by, created_at FROM directory_lists WHERE slug = ?",
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
pub async fn list_entries(
    db: &Database,
    filter: &DirectoryEntryFilter,
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
    let order = match filter.sort {
        DirectorySort::Top => "e.score DESC, e.created_at ASC",
        DirectorySort::New => "e.created_at DESC",
    };
    let sql = format!(
        "SELECT e.id, e.list_id, e.kind, e.category, e.title, e.url, e.description, e.ref_id, e.tags_json, e.submitted_by, e.approved_by, e.score, e.created_at \
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
        "SELECT id, list_id, kind, category, title, url, description, ref_id, tags_json, submitted_by, approved_by, score, created_at \
         FROM directory_entries WHERE id = ? AND removed_at IS NULL",
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
pub async fn my_vote(db: &Database, entry_id: &str, account_id: &str) -> Result<Option<i64>> {
    let sql = db.sql(
        "SELECT vote_value FROM directory_votes WHERE entry_id = ? AND account_id = ?",
        "SELECT vote_value FROM directory_votes WHERE entry_id = ? AND account_id = ?",
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

/// The transactional core of [`set_vote`]. A macro because sqlx queries
/// are typed per-dialect: the same SQL runs against both executors.
macro_rules! vote_tx {
    ($tx:expr, $entry_id:expr, $account_id:expr, $value:expr, $weight:expr, $now:expr) => {{
        let existing: Option<(i64,)> = sqlx::query_as(
            "SELECT vote_value FROM directory_votes WHERE entry_id = ? AND account_id = ?",
        )
        .bind($entry_id)
        .bind($account_id)
        .fetch_optional(&mut *$tx)
        .await?;
        let live: bool = match existing {
            Some((current,)) if current == $value => {
                // Same value again: toggle off.
                sqlx::query("DELETE FROM directory_votes WHERE entry_id = ? AND account_id = ?")
                    .bind($entry_id)
                    .bind($account_id)
                    .execute(&mut *$tx)
                    .await?;
                false
            }
            Some(_) => {
                // Other value: flip in place.
                sqlx::query("UPDATE directory_votes SET vote_value = ?, weight = ?, voted_at = ? WHERE entry_id = ? AND account_id = ?")
                    .bind($value).bind($weight).bind($now).bind($entry_id).bind($account_id)
                    .execute(&mut *$tx)
                    .await?;
                true
            }
            None => {
                sqlx::query("INSERT INTO directory_votes (entry_id, account_id, vote_value, weight, voted_at) VALUES (?, ?, ?, ?, ?)")
                    .bind($entry_id).bind($account_id).bind($value).bind($weight).bind($now)
                    .execute(&mut *$tx)
                    .await?;
                true
            }
        };
        sqlx::query(
            "UPDATE directory_entries SET score = (SELECT COALESCE(SUM(vote_value * weight), 0) FROM directory_votes WHERE entry_id = ?), updated_at = ? WHERE id = ?",
        )
        .bind($entry_id)
        .bind($now)
        .bind($entry_id)
        .execute(&mut *$tx)
        .await?;
        Ok::<bool, anyhow::Error>(live)
    }};
}

pub async fn set_vote(
    db: &Database,
    entry_id: &str,
    account_id: &str,
    value: i64,
    weight: f64,
    now: &str,
) -> Result<(f64, bool)> {
    // The vote change and the score recompute share one transaction
    // (spec §39.4), so the list can never show a stale score.
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let mut tx = pool.begin().await?;
            let live = vote_tx!(tx, entry_id, account_id, value, weight, now)?;
            tx.commit().await?;
            let score = entry_score(db, entry_id).await?;
            Ok((score, live))
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let mut tx = pool.begin().await?;
            let live = vote_tx!(tx, entry_id, account_id, value, weight, now)?;
            tx.commit().await?;
            let score = entry_score(db, entry_id).await?;
            Ok((score, live))
        }
    }
}

/// The denormalised score of an entry.
pub async fn entry_score(db: &Database, entry_id: &str) -> Result<f64> {
    let sql = db.sql(
        "SELECT score FROM directory_entries WHERE id = ?",
        "SELECT score FROM directory_entries WHERE id = ?",
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
        "SELECT id, list_id, kind, category, title, url, description, ref_id, tags_json, submitted_by, approved_by, score, created_at \
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
        "SELECT account_id, vote_value FROM directory_votes WHERE entry_id = $1",
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
