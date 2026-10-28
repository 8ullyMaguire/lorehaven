//! The vocabulary as data, for another site to import (T2.1).
//!
//! Ascend needs one taxonomy and Darkhold is its naming authority: a card tagged `regression` and a work
//! tagged `Regression System` must be one id on both sites, or the two halves of the network describe
//! different worlds and every count across them is wrong.
//!
//! The export is deliberately a *document*, not an API to call at runtime: a file that can be diffed,
//! versioned and imported on a schedule. That is what makes it possible before shared identity exists — no
//! cross-service call, no shared session, just rows and a version.
//!
//! Pagination is by offset because the ordering is stable (`canonical`, then `id`); 4,760 entities is two
//! pages at the default limit, and a cursor would be machinery for a problem nobody has.

use crate::Database;

/// One entity as another site needs it: the identity, both spellings, and where it sits.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EntityRow {
    pub id: String,
    pub kind: String,
    pub canonical: String,
    pub canonical_name: Option<String>,
    pub norm: Option<String>,
    pub slug: Option<String>,
    pub parent_id: Option<String>,
    pub status: Option<String>,
    pub node_state: Option<String>,
    pub work_count: i64,
}

type RawRow = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    i64,
);

fn row_to_entity(row: RawRow) -> EntityRow {
    EntityRow {
        id: row.0,
        kind: row.1,
        canonical: row.2,
        canonical_name: row.3,
        norm: row.4,
        slug: row.5,
        parent_id: row.6,
        status: row.7,
        node_state: row.8,
        work_count: row.9,
    }
}

const COLUMNS: &str = "id, kind, canonical, canonical_name, norm, slug, parent_id, status, node_state, work_count";

/// One page of the vocabulary, plus the total so a caller knows when to stop.
pub async fn page(
    db: &Database,
    kind: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<EntityRow>, i64), sqlx::Error> {
    let limit = limit.clamp(1, 500);
    let offset = offset.max(0);

    // The formatted statements are owned here rather than inline: `db.sql` borrows them, and a
    // `&format!(…)` temporary would be dropped at the end of the statement.
    let page_sqlite = match kind {
        Some(_) => format!(
            "SELECT {COLUMNS} FROM canonical_entities WHERE kind = ? ORDER BY canonical, id LIMIT ? OFFSET ?"
        ),
        None => format!("SELECT {COLUMNS} FROM canonical_entities ORDER BY canonical, id LIMIT ? OFFSET ?"),
    };
    let page_postgres = match kind {
        Some(_) => format!(
            "SELECT {COLUMNS} FROM canonical_entities WHERE kind = $1 ORDER BY canonical, id LIMIT $2 OFFSET $3"
        ),
        None => format!("SELECT {COLUMNS} FROM canonical_entities ORDER BY canonical, id LIMIT $1 OFFSET $2"),
    };
    let (count_sql, page_sql) = match kind {
        Some(_) => (
            db.sql(
                "SELECT COUNT(*) FROM canonical_entities WHERE kind = ?",
                "SELECT COUNT(*)::bigint FROM canonical_entities WHERE kind = $1",
            ),
            db.sql(&page_sqlite, &page_postgres),
        ),
        None => (
            db.sql(
                "SELECT COUNT(*) FROM canonical_entities",
                "SELECT COUNT(*)::bigint FROM canonical_entities",
            ),
            db.sql(&page_sqlite, &page_postgres),
        ),
    };

    let (total, rows): (i64, Vec<RawRow>) = match db.backend() {
        crate::Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite handle");
            let total = match kind {
                Some(kind) => sqlx::query_scalar::<_, i64>(&count_sql).bind(kind).fetch_one(pool).await?,
                None => sqlx::query_scalar::<_, i64>(&count_sql).fetch_one(pool).await?,
            };
            let rows = match kind {
                Some(kind) => {
                    sqlx::query_as::<_, RawRow>(&page_sql)
                        .bind(kind)
                        .bind(limit)
                        .bind(offset)
                        .fetch_all(pool)
                        .await?
                }
                None => {
                    sqlx::query_as::<_, RawRow>(&page_sql)
                        .bind(limit)
                        .bind(offset)
                        .fetch_all(pool)
                        .await?
                }
            };
            (total, rows)
        }
        crate::Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            let total = match kind {
                Some(kind) => sqlx::query_scalar::<_, i64>(&count_sql).bind(kind).fetch_one(pool).await?,
                None => sqlx::query_scalar::<_, i64>(&count_sql).fetch_one(pool).await?,
            };
            let rows = match kind {
                Some(kind) => {
                    sqlx::query_as::<_, RawRow>(&page_sql)
                        .bind(kind)
                        .bind(limit)
                        .bind(offset)
                        .fetch_all(pool)
                        .await?
                }
                None => {
                    sqlx::query_as::<_, RawRow>(&page_sql)
                        .bind(limit)
                        .bind(offset)
                        .fetch_all(pool)
                        .await?
                }
            };
            (total, rows)
        }
    };

    Ok((rows.into_iter().map(row_to_entity).collect(), total))
}

/// What this document describes, as a string two sites can compare: the entity count and the newest change.
///
/// Not a hash of the rows — that would need reading every row on every request. `count + max(updated_at)`
/// changes whenever an entity is added, removed or renamed, which is exactly when an importer must re-read.
pub async fn version(db: &Database) -> Result<String, sqlx::Error> {
    let sql = db.sql(
        "SELECT COUNT(*), MAX(updated_at) FROM canonical_entities",
        "SELECT COUNT(*)::bigint, MAX(updated_at) FROM canonical_entities",
    );
    let (count, newest): (i64, Option<String>) = match db.backend() {
        crate::Backend::Sqlite => sqlx::query_as(&sql).fetch_one(db.sqlite_pool().expect("sqlite handle")).await?,
        crate::Backend::Postgres => {
            sqlx::query_as(&sql)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(format!(
        "{count}@{}",
        newest.unwrap_or_else(|| "never".to_string())
    ))
}

/// Aliases, when there are any. Empty on this instance today, and shipped anyway: an importer that assumes
/// the array exists is correct on the day aliases appear.
pub async fn aliases(db: &Database) -> Result<Vec<(String, String, Option<String>)>, sqlx::Error> {
    let sql = db.sql(
        "SELECT node_id, alias, norm FROM entity_aliases ORDER BY alias",
        "SELECT node_id, alias, norm FROM entity_aliases ORDER BY alias",
    );
    match db.backend() {
        crate::Backend::Sqlite => sqlx::query_as(&sql).fetch_all(db.sqlite_pool().expect("sqlite handle")).await,
        crate::Backend::Postgres => {
            sqlx::query_as(&sql)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await
        }
    }
}
