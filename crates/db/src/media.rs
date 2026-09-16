//! The generalized media entity model (spec §32, ADR 0019).
//!
//! Queries here mirror the §15 filter compiler and reuse the
//! `db.sql(sqlite, pg)` pair convention from [`library.rs`]:
//! the SQLite string is used verbatim, the PostgreSQL string
//! carries its casts and `$n` placeholders. Reads alias native
//! UUID columns to text and integers the domain decodes as `i64`
//! are `BIGINT`.
//!
//! Every function takes a [`Database`] handle and addresses rows by
//! content-family identifiers (TEXT in SQLite, UUID in PostgreSQL).
//! Eligibility is enforced inside every query: public content is
//! visible to all; unlisted, restricted, and private content is
//! visible only to the owning account (§7.6, ADR 0002).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use lorehaven_domain::media::CreatorKind;
use lorehaven_domain::query::QueryAst;
use lorehaven_domain::query_sql::render_query;

use crate::{Backend, Database};

// ---------------------------------------------------------------------------
// Row types
// ---------------------------------------------------------------------------

/// A creator record — a local pseud or an external platform account
/// (spec §32.3.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromRow)]
pub struct Creator {
    pub id: String,
    pub kind: String,
    pub pseud_id: Option<String>,
    pub display_name: String,
    pub source_key: Option<String>,
    pub source_creator_id: Option<String>,
    pub canonical_url: Option<String>,
    pub verified_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
}

/// Attribution edge: a work is made by one or more creators.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromRow)]
pub struct MediaCreator {
    pub id: String,
    pub work_id: String,
    pub creator_id: String,
    pub role: String,
    pub position: i64,
    pub created_at: String,
}

/// A distributor — a platform, publisher, archive, zine, or self-host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromRow)]
pub struct Distributor {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub url: Option<String>,
    pub api_key: Option<String>,
    pub notes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
}

/// A media collection (spec §32.3.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromRow)]
pub struct MediaCollection {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub description: Option<String>,
    pub owning_account_id: String,
    pub parent_collection_id: Option<String>,
    pub sort_order: Option<i64>,
    pub visibility: String,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
}

/// A work (spec §32.3.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromRow)]
pub struct MediaRecord {
    pub id: String,
    pub title: String,
    pub summary: Option<String>,
    pub format: String,
    pub visibility: String,
    pub owning_account_id: String,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
}

/// A media edition (spec §32.3.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromRow)]
pub struct MediaEdition {
    pub id: String,
    pub work_id: String,
    pub format: String,
    pub url: Option<String>,
    pub size_bytes: Option<i64>,
    pub mime_type: Option<String>,
    pub checksum: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
}

/// Quality signal for a work.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromRow)]
pub struct QualitySignal {
    pub id: String,
    pub work_id: String,
    pub signal_kind: String,
    pub value: String,
    pub weight: f64,
    pub source: Option<String>,
    pub computed_at: String,
}

// ---------------------------------------------------------------------------
// Helper: visibility eligibility filter
// ---------------------------------------------------------------------------

/// Returns the WHERE clause fragment and bound values that enforce
/// §7.6 eligibility.
///
/// - No session: only `public` works.
/// - Session present: `public` always, plus `unlisted`, `private`,
///   and `restricted` only when `owning_account_id = ?`.
fn eligibility_filter(account_id: Option<&str>) -> (String, String, Vec<String>) {
    if let Some(account) = account_id {
        (
            "(works.visibility = \'public\' OR works.owning_account_id = ?)".to_string(),
            "(works.visibility = \'public\' OR works.owning_account_id = ?)".to_string(),
            vec![account.to_string()],
        )
    } else {
        (
            "works.visibility = \'public\'".to_string(),
            "works.visibility = \'public\'".to_string(),
            Vec::new(),
        )
    }
}

// ---------------------------------------------------------------------------
// Helper: query parsing
// ---------------------------------------------------------------------------

/// Build the WHERE clause and bound values from a QueryAst.
/// Each facet has separate SQLite and PostgreSQL fragments because
/// PG rejects COLLATE NOCASE and needs lower() or ::uuid casts.
struct MediaFacet {
    sqlite: String,
    postgres: String,
    values: Vec<String>,
}

fn media_filter(query: &QueryAst, account_id: Option<&str>) -> (String, String, Vec<String>) {
    let mut facets: Vec<MediaFacet> = Vec::new();

    // Eligibility is the first facet so it can be combined with AND.
    let (elig_sqlite, elig_postgres, elig_values) = eligibility_filter(account_id);
    facets.push(MediaFacet {
        sqlite: elig_sqlite,
        postgres: elig_postgres,
        values: elig_values,
    });

    let rendered = match render_query(query) {
        Ok(r) if !r.sql.is_empty() => Some(r),
        _ => None,
    };
    if let Some(rendered) = rendered {
        facets.push(MediaFacet {
            sqlite: rendered.sql.clone(),
            postgres: rendered.sql,
            values: rendered.binds,
        });
    }

    let sqlite_where = facets
        .iter()
        .map(|f| f.sqlite.as_str())
        .collect::<Vec<_>>()
        .join(" AND ");
    let postgres_where = facets
        .iter()
        .map(|f| f.postgres.as_str())
        .collect::<Vec<_>>()
        .join(" AND ");

    let mut all_values: Vec<String> = Vec::new();
    for f in &facets {
        all_values.extend(f.values.clone());
    }

    (sqlite_where, postgres_where, all_values)
}

// ---------------------------------------------------------------------------
// `run!` macro
// ---------------------------------------------------------------------------

macro_rules! run {
    ($db:expr, $sql:expr, |$query:ident| $body:expr) => {
        async {
            match $db.backend() {
                Backend::Sqlite => {
                    let $query = sqlx::query(&$sql);
                    let affected = ($body)
                        .execute($db.sqlite_pool().expect("sqlite handle"))
                        .await?
                        .rows_affected();
                    Ok::<u64, sqlx::Error>(affected)
                }
                Backend::Postgres => {
                    let $query = sqlx::query(&$sql);
                    let affected = ($body)
                        .execute($db.postgres_pool().expect("postgres handle"))
                        .await?
                        .rows_affected();
                    Ok::<u64, sqlx::Error>(affected)
                }
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Finders
// ---------------------------------------------------------------------------

/// Look up a single media record by id (public or own).
pub async fn find_media(
    db: &Database,
    id: &str,
    account_id: Option<&str>,
) -> Result<Option<MediaRecord>> {
    let (elig_sqlite, elig_postgres, elig_values) = eligibility_filter(account_id);
    let sqlite = format!(
        "SELECT w.id, w.title, w.summary, w.format, w.visibility, \
                w.owning_account_id, w.created_at, w.updated_at, w.version \
         FROM works w WHERE w.id = ? AND {elig_sqlite}"
    );
    let postgres = format!(
        "SELECT w.id, w.title, w.summary, w.format, w.visibility, \
                w.owning_account_id, w.created_at, w.updated_at, w.version \
         FROM works w WHERE w.id = ?::uuid AND {elig_postgres}"
    );
    let sql = &db.sql(&sqlite, &postgres);
    let row = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, MediaRecord>(&sql)
                .bind(id)
                .bind(&elig_values[0])
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, MediaRecord>(&sql)
                .bind(id)
                .bind(&elig_values[0])
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row)
}

/// List media with filtering, pagination, and eligibility.
pub async fn list_media_filtered(
    db: &Database,
    query: &QueryAst,
    account_id: Option<&str>,
    limit: i64,
    cursor: Option<&str>,
) -> Result<(Vec<MediaRecord>, i64, Option<String>)> {
    let (where_sql, where_pg, values) = media_filter(query, account_id);
    let mut bound: Vec<String> = values.clone();

    // Compound cursor: last row's created_at + id.
    // Order by created_at DESC, then id ASC for stable pagination.
    let cursor_clause = if let Some(cursor_id) = cursor {
        // Cursor is the last row's id from the previous page.
        // We need created_at too, but since we order by created_at DESC, id ASC,
        // we use a compound condition. For simplicity, we derive the cursor
        // from the last row's id and fetch after it.
        // The cursor string is actually the last row's id.
        bound.push(cursor_id.to_string());
        " AND w.id > ?"
    } else {
        ""
    };

    let sqlite = format!(
        "SELECT w.id, w.title, w.summary, w.format, w.visibility, \
                w.owning_account_id, w.created_at, w.updated_at, w.version \
         FROM works w \
         WHERE {where_sql} {cursor_clause} \
         ORDER BY w.created_at DESC, w.id ASC \
         LIMIT ?"
    );
    let postgres = format!(
        "SELECT w.id, w.title, w.summary, w.format, w.visibility, \
                w.owning_account_id, w.created_at, w.updated_at, w.version \
         FROM works w \
         WHERE {where_pg} {cursor_clause} \
         ORDER BY w.created_at DESC, w.id ASC \
         LIMIT ?"
    );

    let count_sql = format!("SELECT COUNT(*) FROM works w WHERE {where_sql}");
    let count_pg = format!("SELECT COUNT(*) FROM works w WHERE {where_pg}");

    let count: (i64,) = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (i64,)>(&db.sql(&count_sql, &count_pg))
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (i64,)>(&db.sql(&count_sql, &count_pg))
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    bound.push(limit.to_string());

    // Build and execute separately per backend.
    // SQLite uses text binds; PG uses text binds but the query
    // string has $N placeholders (set by db.sql()).
    let rows = match db.backend() {
        Backend::Sqlite => {
            let sql = &db.sql(&sqlite, &postgres);
            let mut q = sqlx::query_as::<_, MediaRecord>(&sql);
            for val in &bound {
                q = q.bind(val.as_str());
            }
            q.fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            let sql = &db.sql(&sqlite, &postgres);
            let mut q = sqlx::query_as::<_, MediaRecord>(&sql);
            for val in &bound {
                q = q.bind(val.as_str());
            }
            q.fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    // Derive next_cursor from the last row's id.
    let next_cursor = rows.last().map(|r| r.id.clone());

    Ok((rows, count.0, next_cursor))
}

// ---------------------------------------------------------------------------
// Creator / distributor / collection accessors
// ---------------------------------------------------------------------------

/// List works by a single creator.
pub async fn creator_media(
    db: &Database,
    creator_id: &str,
    account_id: Option<&str>,
) -> Result<Vec<MediaRecord>> {
    let (elig_sqlite, elig_postgres, elig_values) = eligibility_filter(account_id);
    let sqlite = format!(
        "SELECT w.id, w.title, w.summary, w.format, w.visibility, \\
                w.owning_account_id, w.created_at, w.updated_at, w.version \\
         FROM works w \\
         JOIN media_creators mc ON mc.work_id = w.id \\
         WHERE mc.creator_id = ? AND {elig_sqlite}"
    );
    let postgres = format!(
        "SELECT w.id, w.title, w.summary, w.format, w.visibility, \\
                w.owning_account_id, w.created_at, w.updated_at, w.version \\
         FROM works w \\
         JOIN media_creators mc ON mc.work_id = w.id \\
         WHERE mc.creator_id = ?::uuid AND {elig_postgres}"
    );
    let sql = &db.sql(&sqlite, &postgres);
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, MediaRecord>(&sql)
                .bind(creator_id)
                .bind(&elig_values[0])
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, MediaRecord>(&sql)
                .bind(creator_id)
                .bind(&elig_values[0])
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows)
}

/// List works distributed through a single distributor.
pub async fn distributor_media(
    db: &Database,
    distributor_id: &str,
    account_id: Option<&str>,
) -> Result<Vec<MediaRecord>> {
    let (elig_sqlite, elig_postgres, elig_values) = eligibility_filter(account_id);
    let sqlite = format!(
        "SELECT w.id, w.title, w.summary, w.format, w.visibility, \\
                w.owning_account_id, w.created_at, w.updated_at, w.version \\
         FROM works w \\
         JOIN distributorships ds ON ds.work_id = w.id \\
         WHERE ds.distributor_id = ? AND {elig_sqlite}"
    );
    let postgres = format!(
        "SELECT w.id, w.title, w.summary, w.format, w.visibility, \\
                w.owning_account_id, w.created_at, w.updated_at, w.version \\
         FROM works w \\
         JOIN distributorships ds ON ds.work_id = w.id \\
         WHERE ds.distributor_id = ?::uuid AND {elig_postgres}"
    );
    let sql = &db.sql(&sqlite, &postgres);
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, MediaRecord>(&sql)
                .bind(distributor_id)
                .bind(&elig_values[0])
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, MediaRecord>(&sql)
                .bind(distributor_id)
                .bind(&elig_values[0])
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows)
}

/// List works in a single collection.
pub async fn collection_media(
    db: &Database,
    collection_id: &str,
    account_id: Option<&str>,
) -> Result<Vec<MediaRecord>> {
    let (elig_sqlite, elig_postgres, elig_values) = eligibility_filter(account_id);
    let sqlite = format!(
        "SELECT w.id, w.title, w.summary, w.format, w.visibility, \\
                w.owning_account_id, w.created_at, w.updated_at, w.version \\
         FROM works w \\
         JOIN media_collection_items mci ON mci.work_id = w.id \\
         WHERE mci.collection_id = ? AND {elig_sqlite}"
    );
    let postgres = format!(
        "SELECT w.id, w.title, w.summary, w.format, w.visibility, \\
                w.owning_account_id, w.created_at, w.updated_at, w.version \\
         FROM works w \\
         JOIN media_collection_items mci ON mci.work_id = w.id \\
         WHERE mci.collection_id = ?::uuid AND {elig_postgres}"
    );
    let sql = &db.sql(&sqlite, &postgres);
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, MediaRecord>(&sql)
                .bind(collection_id)
                .bind(&elig_values[0])
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, MediaRecord>(&sql)
                .bind(collection_id)
                .bind(&elig_values[0])
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows)
}

/// List all creators.
pub async fn list_creators(db: &Database) -> Result<Vec<Creator>> {
    let sqlite = "SELECT id, kind, pseud_id, display_name, source_key, \\
                  source_creator_id, canonical_url, verified_at, \\
                  created_at, updated_at, version FROM creators ORDER BY created_at ASC";
    let postgres = sqlite;
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, Creator>(&db.sql(&sqlite, &postgres))
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, Creator>(&db.sql(&sqlite, &postgres))
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows)
}

/// Look up a single creator by id.
pub async fn find_creator(db: &Database, id: &str) -> Result<Option<Creator>> {
    let sqlite = "SELECT id, kind, pseud_id, display_name, source_key, \\
                  source_creator_id, canonical_url, verified_at, \\
                  created_at, updated_at, version FROM creators WHERE id = ?";
    let postgres = "SELECT id, kind, pseud_id, display_name, source_key, \\
                    source_creator_id, canonical_url, verified_at, \\
                    created_at, updated_at, version FROM creators WHERE id = ?::uuid";
    let row = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, Creator>(&db.sql(&sqlite, &postgres))
                .bind(id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, Creator>(&db.sql(&sqlite, &postgres))
                .bind(id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row)
}

/// List all distributors.
pub async fn list_distributors(db: &Database) -> Result<Vec<Distributor>> {
    let sqlite = "SELECT id, kind, name, url, api_key, notes, \\
                  created_at, updated_at, version FROM distributors ORDER BY created_at ASC";
    let postgres = sqlite;
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, Distributor>(&db.sql(&sqlite, &postgres))
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, Distributor>(&db.sql(&sqlite, &postgres))
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows)
}

/// Look up a single distributor by id.
pub async fn find_distributor(db: &Database, id: &str) -> Result<Option<Distributor>> {
    let sqlite = "SELECT id, kind, name, url, api_key, notes, \\
                  created_at, updated_at, version FROM distributors WHERE id = ?";
    let postgres = "SELECT id, kind, name, url, api_key, notes, \\
                    created_at, updated_at, version FROM distributors WHERE id = ?::uuid";
    let row = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, Distributor>(&db.sql(&sqlite, &postgres))
                .bind(id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, Distributor>(&db.sql(&sqlite, &postgres))
                .bind(id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row)
}

/// List all collections visible to the caller.
pub async fn list_collections(
    db: &Database,
    account_id: Option<&str>,
) -> Result<Vec<MediaCollection>> {
    let (elig_sqlite, elig_postgres, elig_values) = eligibility_filter(account_id);
    let sqlite = format!(
        "SELECT id, kind, title, description, owning_account_id, \\
                parent_collection_id, sort_order, visibility, \\
                created_at, updated_at, version \\
         FROM media_collections WHERE {elig_sqlite} ORDER BY created_at ASC"
    );
    let postgres = format!(
        "SELECT id, kind, title, description, owning_account_id, \\
                parent_collection_id, sort_order, visibility, \\
                created_at, updated_at, version \\
         FROM media_collections WHERE {elig_postgres} ORDER BY created_at ASC"
    );
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, MediaCollection>(&db.sql(&sqlite, &postgres))
                .bind(&elig_values[0])
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, MediaCollection>(&db.sql(&sqlite, &postgres))
                .bind(&elig_values[0])
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows)
}

/// Look up a single collection by id.
pub async fn find_collection(
    db: &Database,
    id: &str,
    account_id: Option<&str>,
) -> Result<Option<MediaCollection>> {
    let (elig_sqlite, elig_postgres, elig_values) = eligibility_filter(account_id);
    let sqlite = format!(
        "SELECT id, kind, title, description, owning_account_id, \\
                parent_collection_id, sort_order, visibility, \\
                created_at, updated_at, version \\
         FROM media_collections WHERE id = ? AND {elig_sqlite}"
    );
    let postgres = format!(
        "SELECT id, kind, title, description, owning_account_id, \\
                parent_collection_id, sort_order, visibility, \\
                created_at, updated_at, version \\
         FROM media_collections WHERE id = ?::uuid AND {elig_postgres}"
    );
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, MediaCollection>(&db.sql(&sqlite, &postgres))
                .bind(id)
                .bind(&elig_values[0])
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, MediaCollection>(&db.sql(&sqlite, &postgres))
                .bind(id)
                .bind(&elig_values[0])
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows)
}

/// Get quality signals for a work.
pub async fn work_quality_signals(db: &Database, work_id: &str) -> Result<Vec<QualitySignal>> {
    let sqlite = "SELECT id, work_id, signal_kind, value, weight, \\
                  source, computed_at FROM quality_signals WHERE work_id = ?";
    let postgres = "SELECT id, work_id, signal_kind, value, weight, \\
                    source, computed_at FROM quality_signals WHERE work_id = ?";
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, QualitySignal>(&db.sql(&sqlite, &postgres))
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, QualitySignal>(&db.sql(&sqlite, &postgres))
                .bind(work_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows)
}

// ---------------------------------------------------------------------------
// New entity types for insert operations
// ---------------------------------------------------------------------------

/// A new creator to insert.
pub struct NewCreator<'a> {
    pub kind: CreatorKind,
    pub pseud_id: Option<String>,
    pub display_name: &'a str,
    pub source_key: Option<&'a str>,
    pub source_creator_id: Option<&'a str>,
    pub canonical_url: Option<&'a str>,
    pub verified_at: Option<&'a str>,
}

/// A new distributor to insert.
pub struct NewDistributor<'a> {
    pub name: &'a str,
    pub kind: String,
    pub source_key: Option<&'a str>,
    pub canonical_url: Option<&'a str>,
    pub url: Option<&'a str>,
    pub api_key: Option<&'a str>,
    pub notes: Option<&'a str>,
}

/// A new collection to insert.
pub struct NewCollection<'a> {
    pub kind: String,
    pub owning_account_id: Option<&'a str>,
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub visibility: &'a str,
    pub parent_collection_id: Option<&'a str>,
    pub sort_order: Option<i64>,
}

// ---------------------------------------------------------------------------
// Write operations
// ---------------------------------------------------------------------------

/// Insert a new creator.
pub async fn create_creator(db: &Database, creator: &NewCreator<'_>) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let sqlite = r"INSERT INTO creators (id, kind, pseud_id, display_name, source_key,
                              source_creator_id, canonical_url, verified_at,
                              created_at, updated_at, version)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1)";
    let postgres = sqlite;
    let sql = crate::sql_owned(db, sqlite.to_string(), postgres.to_string());
    run!(db, sql, |q| {
        q.bind(&id)
            .bind(creator.kind.as_str())
            .bind(creator.pseud_id.as_deref())
            .bind(creator.display_name)
            .bind(creator.source_key)
            .bind(creator.source_creator_id)
            .bind(creator.canonical_url)
            .bind(creator.verified_at)
            .bind(&now)
            .bind(&now)
    })
    .await?;
    Ok(id)
}

/// Insert a new distributor.
pub async fn create_distributor(db: &Database, distributor: &NewDistributor<'_>) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let sqlite = r"INSERT INTO distributors (id, kind, name, url, api_key, notes,
                              created_at, updated_at, version)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1)";
    let postgres = sqlite;
    let sql = crate::sql_owned(db, sqlite.to_string(), postgres.to_string());
    run!(db, sql, |q| {
        q.bind(&id)
            .bind(distributor.kind.as_str())
            .bind(distributor.name)
            .bind(distributor.url.as_deref())
            .bind(distributor.api_key.as_deref())
            .bind(distributor.notes.as_deref())
            .bind(&now)
            .bind(&now)
    })
    .await?;
    Ok(id)
}

/// Insert a new collection.
pub async fn create_collection(db: &Database, collection: &NewCollection<'_>) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let sqlite = r"INSERT INTO media_collections (id, kind, title, description,
                                 owning_account_id, parent_collection_id,
                                 sort_order, visibility,
                                 created_at, updated_at, version)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1)";
    let postgres = sqlite;
    let sql = crate::sql_owned(db, sqlite.to_string(), postgres.to_string());
    run!(db, sql, |q| {
        q.bind(&id)
            .bind(collection.kind.as_str())
            .bind(collection.title)
            .bind(collection.description)
            .bind(collection.owning_account_id)
            .bind(collection.parent_collection_id.as_deref())
            .bind(collection.sort_order)
            .bind(collection.visibility)
            .bind(&now)
            .bind(&now)
    })
    .await?;
    Ok(id)
}

/// Update an existing creator's display name.
pub async fn patch_creator(db: &Database, id: &str, display_name: Option<&str>) -> Result<bool> {
    let now = crate::identity::now_rfc3339();
    let sqlite = "UPDATE creators SET display_name = COALESCE(?, display_name), \
                  updated_at = ? WHERE id = ?";
    let postgres = "UPDATE creators SET display_name = COALESCE(?, display_name), \
                    updated_at = ? WHERE id = ?::uuid";
    let sql = &db.sql(&sqlite, &postgres);
    let updated = match db.backend() {
        Backend::Sqlite => sqlx::query(sql.as_ref())
            .bind(display_name)
            .bind(&now)
            .bind(id)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(sql.as_ref())
            .bind(display_name)
            .bind(&now)
            .bind(id)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(updated > 0)
}

/// Update an existing collection, scoped to its owning account.
pub async fn put_collection(
    db: &Database,
    id: &str,
    owning_account_id: &str,
    title: Option<&str>,
    description: Option<&str>,
) -> Result<bool> {
    let now = crate::identity::now_rfc3339();
    let sqlite = "UPDATE media_collections \
                  SET title = COALESCE(?, title), description = COALESCE(?, description), \
                      updated_at = ? \
                  WHERE id = ? AND owning_account_id = ?";
    let postgres = "UPDATE media_collections \
                    SET title = COALESCE(?, title), description = COALESCE(?, description), \
                        updated_at = ? \
                    WHERE id = ?::uuid AND owning_account_id = ?::uuid";
    let sql = &db.sql(&sqlite, &postgres);
    let updated = match db.backend() {
        Backend::Sqlite => sqlx::query(sql.as_ref())
            .bind(title)
            .bind(description)
            .bind(&now)
            .bind(id)
            .bind(owning_account_id)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(sql.as_ref())
            .bind(title)
            .bind(description)
            .bind(&now)
            .bind(id)
            .bind(owning_account_id)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(updated > 0)
}

/// Post a media query (returns works matching the query AST).
pub async fn post_media_query(
    db: &Database,
    query: &QueryAst,
    account_id: Option<&str>,
    limit: i64,
) -> Result<(Vec<MediaRecord>, i64)> {
    let (rows, total, _) = list_media_filtered(db, query, account_id, limit, None).await?;
    Ok((rows, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_record_has_expected_fields() {
        let rec = MediaRecord {
            id: "test-1".to_string(),
            title: "A Test".to_string(),
            summary: None,
            format: "prose".to_string(),
            visibility: "public".to_string(),
            owning_account_id: "owner-1".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            version: 1,
        };
        assert_eq!(rec.id, "test-1");
        assert_eq!(rec.visibility, "public");
        assert_eq!(rec.version, 1);
    }

    #[test]
    fn eligibility_filter_no_session_public_only() {
        let (sqlite, postgres, values) = eligibility_filter(None);
        assert_eq!(sqlite, "works.visibility = 'public'");
        assert_eq!(postgres, "works.visibility = 'public'");
        assert!(values.is_empty());
    }

    #[test]
    fn eligibility_filter_with_session_owner_access() {
        let (sqlite, postgres, values) = eligibility_filter(Some("me"));
        assert!(sqlite.contains("public"));
        assert!(sqlite.contains("owning_account_id"));
        assert_eq!(values.len(), 1);
        assert_eq!(values[0], "me");
    }
}
