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

use crate::{sql_owned, Backend, Database};

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
    pub source_key: Option<String>,
    pub canonical_url: Option<String>,
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
    pub owning_account_id: Option<String>,
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
    pub rating: String,
    pub visibility: String,
    pub lifecycle: String,
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
    pub edition_kind: String,
    pub label: Option<String>,
    pub parent_edition_id: Option<String>,
    pub published_at: Option<String>,
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
/// The eligibility facet for LIST queries (spec §32.2, ADR 0002's
/// visibility vocabulary, ADR 0003 ownership). Media is published work:
/// drafts never appear in listings. Anonymous callers see public only. A
/// session also sees restricted works (skeleton §7.6: any authenticated
/// session is eligible; the shared service replaces this when wired) and
/// their own published works of any visibility. Unlisted never lists — it
/// is link-reachable through the direct doors, which decide per ADR 0002.
/// Ownership goes through the pseud: works.owner_pseud_id →
/// pseuds.account_id.
fn eligibility_filter(account_id: Option<&str>) -> (String, String, Vec<String>) {
    // The facet is consumed by queries whose FROM aliases works as `w` —
    // once a table is aliased, the bare table name is no longer a valid
    // qualifier in either dialect.
    match account_id {
        Some(account) => {
            let sql = "(w.lifecycle = 'published' AND EXISTS (
                SELECT 1 FROM accounts viewer
                LEFT JOIN content_settings settings ON settings.account_id = viewer.id
                WHERE CAST(viewer.id AS TEXT) = ? AND (
                    w.owner_pseud_id IN (SELECT id FROM pseuds WHERE account_id = viewer.id)
                    OR (w.visibility IN ('public', 'restricted')
                        AND (CASE w.rating WHEN 'general' THEN 0 WHEN 'teen' THEN 1 WHEN 'mature' THEN 2 WHEN 'explicit' THEN 3 ELSE 99 END)
                            <= (CASE viewer.age_state WHEN 'declared_adult' THEN 3 WHEN 'unknown' THEN 1 ELSE 0 END)
                        AND (CASE w.rating WHEN 'general' THEN 0 WHEN 'teen' THEN 1 WHEN 'mature' THEN 2 WHEN 'explicit' THEN 3 ELSE 99 END)
                            <= (CASE COALESCE(settings.max_rating, 'teen') WHEN 'general' THEN 0 WHEN 'teen' THEN 1 WHEN 'mature' THEN 2 WHEN 'explicit' THEN 3 ELSE 0 END)
                    )
                )))".to_owned();
            (sql.clone(), sql, vec![account.to_owned()])
        },
        None => (
            "(w.lifecycle = 'published' AND w.visibility = 'public' AND w.rating IN ('general', 'teen'))".to_string(),
            "(w.lifecycle = 'published' AND w.visibility = 'public' AND w.rating IN ('general', 'teen'))".to_string(),
            Vec::new(),
        ),
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

fn media_filter(
    query: Option<&QueryAst>,
    account_id: Option<&str>,
) -> Result<(String, String, Vec<String>)> {
    let mut facets: Vec<MediaFacet> = Vec::new();

    // Eligibility is the first facet so it can be combined with AND.
    let (elig_sqlite, elig_postgres, elig_values) = eligibility_filter(account_id);
    facets.push(MediaFacet {
        sqlite: elig_sqlite,
        postgres: elig_postgres,
        values: elig_values,
    });

    // None (or an AST that renders nothing) means match-all: no text
    // facet, so no works_index reference and no LIKE binds.
    if let Some(ast) = query {
        let r = render_query(ast).map_err(|e| anyhow::anyhow!("{} at {}", e.message, e.offset))?;
        if !r.sql.is_empty() {
            let sql = r
                .sql
                .replace("works_index.", "wi.")
                .replace("works.", "w.")
                .replace("pseuds.", "p.");
            facets.push(MediaFacet {
                sqlite: sql.clone(),
                postgres: sql,
                values: r.binds,
            });
        }
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

    Ok((sqlite_where, postgres_where, all_values))
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
/// Fetch one work for the direct doors. Deliberately NO eligibility here:
/// the direct-door rule (ADR 0002 — public and unlisted are link-reachable,
/// restricted needs a session, drafts only for the owner) belongs to the
/// route, which answers 404 to hide existence. The list facet decides what
/// *listings* show; this decides nothing.
pub async fn find_media(db: &Database, id: &str) -> Result<Option<MediaRecord>> {
    let sqlite = "SELECT w.id, w.title, w.summary, w.format, w.rating, w.visibility, w.lifecycle, \
                p.account_id AS owning_account_id, w.created_at, w.updated_at, w.version \
         FROM works w \
         JOIN pseuds p ON p.id = w.owner_pseud_id \
         WHERE w.id = ?"
        .to_string();
    let postgres =
        "SELECT w.id::text AS id, w.title, w.summary, w.format, w.rating, w.visibility, w.lifecycle, \
                p.account_id::text AS owning_account_id, w.created_at, w.updated_at, w.version \
         FROM works w \
         JOIN pseuds p ON p.id = w.owner_pseud_id \
         WHERE w.id = ?::uuid"
            .to_string();
    let sql = &db.sql(&sqlite, &postgres);
    let row = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, MediaRecord>(sql)
                .bind(id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, MediaRecord>(sql)
                .bind(id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row)
}

/// List media with filtering, pagination, and eligibility.
pub async fn list_media_filtered(
    db: &Database,
    query: Option<&QueryAst>,
    account_id: Option<&str>,
    limit: i64,
    cursor: Option<&str>,
) -> Result<(Vec<MediaRecord>, i64, Option<String>)> {
    let (where_sql, where_pg, values) = media_filter(query, account_id)?;
    let mut bound: Vec<String> = values.clone();

    // Compound cursor: "created_at|id" of the previous page's last row,
    // matching the ORDER BY (created_at DESC, id ASC). An id-only cursor
    // cannot paginate this order — it skips and repeats rows.
    let (cursor_sqlite, cursor_pg);
    match cursor {
        Some(cursor) => {
            let Some((created_at, id)) = cursor.rsplit_once('|') else {
                anyhow::bail!("malformed media cursor");
            };
            cursor_sqlite =
                " AND (w.created_at < ? OR (w.created_at = ? AND w.id > ?))".to_string();
            cursor_pg =
                " AND (w.created_at < ?::text OR (w.created_at = ?::text AND w.id > ?::uuid))"
                    .to_string();
            bound.push(created_at.to_string());
            bound.push(created_at.to_string());
            bound.push(id.to_string());
        }
        None => {
            cursor_sqlite = String::new();
            cursor_pg = String::new();
        }
    }

    // works_index backs the text facet; the LEFT JOIN is harmless when the
    // facet is absent. pseuds supplies the owning account (ADR 0003).
    let sqlite = format!(
        "SELECT w.id, w.title, w.summary, w.format, w.rating, w.visibility, w.lifecycle, \
                p.account_id AS owning_account_id, w.created_at, w.updated_at, w.version \
         FROM works w \
         LEFT JOIN works_index wi ON wi.work_id = w.id \
         JOIN pseuds p ON p.id = w.owner_pseud_id \
         WHERE {where_sql}{cursor_sqlite} \
         ORDER BY w.created_at DESC, w.id ASC \
         LIMIT ?"
    );
    let postgres = format!(
        "SELECT w.id::text AS id, w.title, w.summary, w.format, w.rating, w.visibility, w.lifecycle, \
                p.account_id::text AS owning_account_id, w.created_at, w.updated_at, w.version \
         FROM works w \
         LEFT JOIN works_index wi ON wi.work_id = w.id \
         JOIN pseuds p ON p.id = w.owner_pseud_id \
         WHERE {where_pg}{cursor_pg} \
         ORDER BY w.created_at DESC, w.id ASC \
         LIMIT ?"
    );

    let count_sql = format!(
        "SELECT COUNT(*) FROM works w \
         LEFT JOIN works_index wi ON wi.work_id = w.id \
         JOIN pseuds p ON p.id = w.owner_pseud_id \
         WHERE {where_sql}"
    );
    let count_pg = format!(
        "SELECT COUNT(*) FROM works w \
         LEFT JOIN works_index wi ON wi.work_id = w.id \
         JOIN pseuds p ON p.id = w.owner_pseud_id \
         WHERE {where_pg}"
    );

    // The count MUST carry the facet binds too — an unbound parameter
    // silently becomes NULL on SQLite and undercounts.
    let count_sql_final = db.sql(&count_sql, &count_pg);
    let count: (i64,) = match db.backend() {
        Backend::Sqlite => {
            let mut q = sqlx::query_as::<_, (i64,)>(&count_sql_final);
            for val in &values {
                q = q.bind(val.as_str());
            }
            q.fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            let mut q = sqlx::query_as::<_, (i64,)>(&count_sql_final);
            for val in &values {
                q = q.bind(val.as_str());
            }
            q.fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    // LIMIT takes an integer bind, not text: PG rejects a text LIMIT.
    let rows = match db.backend() {
        Backend::Sqlite => {
            let sql = &db.sql(&sqlite, &postgres);
            let mut q = sqlx::query_as::<_, MediaRecord>(sql);
            for val in &bound {
                q = q.bind(val.as_str());
            }
            q.bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            let sql = &db.sql(&sqlite, &postgres);
            let mut q = sqlx::query_as::<_, MediaRecord>(sql);
            for val in &bound {
                q = q.bind(val.as_str());
            }
            q.bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    // A cursor is only emitted when the page was full: the last page of a
    // walk answers without one, so clients stop naturally.
    let next_cursor = if rows.len() as i64 == limit {
        rows.last().map(|r| format!("{}|{}", r.created_at, r.id))
    } else {
        None
    };

    Ok((rows, count.0, next_cursor))
}

// ---------------------------------------------------------------------------
// Creator / distributor / collection accessors
// ---------------------------------------------------------------------------

/// Scoped media listings share one decode and binding path.
async fn attributed_media(
    db: &Database,
    id: &str,
    account_id: Option<&str>,
    table: &str,
    key: &str,
) -> Result<Vec<MediaRecord>> {
    // table/key are private, fixed call-site constants, never request values.
    let (eligible, eligible_pg, values) = eligibility_filter(account_id);
    let sqlite = format!(
        "SELECT w.id, w.title, w.summary, w.format, w.rating, w.visibility, w.lifecycle,
        p.account_id AS owning_account_id, w.created_at, w.updated_at, w.version
        FROM works w JOIN pseuds p ON p.id = w.owner_pseud_id
        WHERE EXISTS (SELECT 1 FROM {table} edge WHERE edge.work_id = w.id AND edge.{key} = ?)
        AND {eligible} ORDER BY w.created_at DESC, w.id ASC LIMIT 200"
    );
    let postgres = format!("SELECT w.id::text AS id, w.title, w.summary, w.format, w.rating, w.visibility, w.lifecycle,
        p.account_id::text AS owning_account_id, w.created_at, w.updated_at, w.version::bigint
        FROM works w JOIN pseuds p ON p.id = w.owner_pseud_id
        WHERE EXISTS (SELECT 1 FROM {table} edge WHERE edge.work_id = w.id AND edge.{key} = ?::uuid)
        AND {eligible_pg} ORDER BY w.created_at DESC, w.id ASC LIMIT 200");
    let sql = db.sql(&sqlite, &postgres);
    match db.backend() {
        Backend::Sqlite => {
            let mut query = sqlx::query_as::<_, MediaRecord>(&sql).bind(id);
            for value in &values {
                query = query.bind(value);
            }
            Ok(query
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?)
        }
        Backend::Postgres => {
            let mut query = sqlx::query_as::<_, MediaRecord>(&sql).bind(id);
            for value in &values {
                query = query.bind(value);
            }
            Ok(query
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?)
        }
    }
}

/// List eligible media attributed to a creator.
pub async fn creator_media(
    db: &Database,
    id: &str,
    account_id: Option<&str>,
) -> Result<Vec<MediaRecord>> {
    attributed_media(db, id, account_id, "media_creators", "creator_id").await
}

/// List eligible media available through a distributor.
pub async fn distributor_media(
    db: &Database,
    id: &str,
    account_id: Option<&str>,
) -> Result<Vec<MediaRecord>> {
    attributed_media(db, id, account_id, "distributorships", "distributor_id").await
}

/// List eligible media in an accessible collection.
pub async fn collection_media(
    db: &Database,
    id: &str,
    account_id: Option<&str>,
) -> Result<Vec<MediaRecord>> {
    if find_collection(db, id, account_id).await?.is_none() {
        return Ok(Vec::new());
    }
    attributed_media(
        db,
        id,
        account_id,
        "media_collection_items",
        "collection_id",
    )
    .await
}

const CREATOR_COLUMNS: &str = "CAST(id AS TEXT) AS id, kind, CAST(pseud_id AS TEXT) AS pseud_id,
    display_name, source_key, source_creator_id, canonical_url, verified_at, created_at, updated_at, version";
const DISTRIBUTOR_COLUMNS: &str = "CAST(id AS TEXT) AS id, kind, name, source_key, canonical_url,
    created_at, updated_at, version";

/// List public attribution records.
pub async fn list_creators(db: &Database) -> Result<Vec<Creator>> {
    let sql = format!("SELECT {CREATOR_COLUMNS} FROM creators ORDER BY created_at, id LIMIT 200");
    match db.backend() {
        Backend::Sqlite => Ok(sqlx::query_as(&sql)
            .fetch_all(db.sqlite_pool().expect("sqlite handle"))
            .await?),
        Backend::Postgres => Ok(sqlx::query_as(&sql)
            .fetch_all(db.postgres_pool().expect("postgres handle"))
            .await?),
    }
}

pub async fn find_creator(db: &Database, id: &str) -> Result<Option<Creator>> {
    let query = format!("SELECT {CREATOR_COLUMNS} FROM creators WHERE CAST(id AS TEXT) = ?");
    let sql = db.sql(&query, &query);
    match db.backend() {
        Backend::Sqlite => Ok(sqlx::query_as(&sql)
            .bind(id)
            .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
            .await?),
        Backend::Postgres => Ok(sqlx::query_as(&sql)
            .bind(id)
            .fetch_optional(db.postgres_pool().expect("postgres handle"))
            .await?),
    }
}

pub async fn list_distributors(db: &Database) -> Result<Vec<Distributor>> {
    let sql =
        format!("SELECT {DISTRIBUTOR_COLUMNS} FROM distributors ORDER BY created_at, id LIMIT 200");
    match db.backend() {
        Backend::Sqlite => Ok(sqlx::query_as(&sql)
            .fetch_all(db.sqlite_pool().expect("sqlite handle"))
            .await?),
        Backend::Postgres => Ok(sqlx::query_as(&sql)
            .fetch_all(db.postgres_pool().expect("postgres handle"))
            .await?),
    }
}

pub async fn find_distributor(db: &Database, id: &str) -> Result<Option<Distributor>> {
    let query =
        format!("SELECT {DISTRIBUTOR_COLUMNS} FROM distributors WHERE CAST(id AS TEXT) = ?");
    let sql = db.sql(&query, &query);
    match db.backend() {
        Backend::Sqlite => Ok(sqlx::query_as(&sql)
            .bind(id)
            .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
            .await?),
        Backend::Postgres => Ok(sqlx::query_as(&sql)
            .bind(id)
            .fetch_optional(db.postgres_pool().expect("postgres handle"))
            .await?),
    }
}

/// List collections visible in discovery for the current viewer.
pub async fn list_collections(
    db: &Database,
    account_id: Option<&str>,
) -> Result<Vec<MediaCollection>> {
    read_collections(db, None, account_id).await
}

/// Look up a collection without disclosing private collections to strangers.
pub async fn find_collection(
    db: &Database,
    id: &str,
    account_id: Option<&str>,
) -> Result<Option<MediaCollection>> {
    Ok(read_collections(db, Some(id), account_id)
        .await?
        .into_iter()
        .next())
}

async fn read_collections(
    db: &Database,
    id: Option<&str>,
    account_id: Option<&str>,
) -> Result<Vec<MediaCollection>> {
    let sqlite = "SELECT id, collection_kind AS kind, title, description, owning_account_id,
        visibility, created_at, updated_at, version FROM media_collections
        WHERE (? IS NULL OR id = ?) AND
        (visibility = 'public' OR owning_account_id = ? OR
         (visibility = 'restricted' AND ? IS NOT NULL) OR
         (visibility = 'unlisted' AND ? IS NOT NULL))
        ORDER BY created_at ASC, id ASC LIMIT 200";
    let postgres =
        "SELECT id::text, collection_kind AS kind, title, description, owning_account_id::text,
        visibility, created_at, updated_at, version::bigint FROM media_collections
        WHERE (?::text IS NULL OR id = ?::uuid) AND
        (visibility = 'public' OR owning_account_id = ?::uuid OR
         (visibility = 'restricted' AND ?::text IS NOT NULL) OR
         (visibility = 'unlisted' AND ?::text IS NOT NULL))
        ORDER BY created_at ASC, id ASC LIMIT 200";
    let sql = db.sql(sqlite, postgres);
    match db.backend() {
        Backend::Sqlite => Ok(sqlx::query_as::<_, MediaCollection>(&sql)
            .bind(id)
            .bind(id)
            .bind(account_id)
            .bind(account_id)
            .bind(id)
            .fetch_all(db.sqlite_pool().expect("sqlite handle"))
            .await?),
        Backend::Postgres => Ok(sqlx::query_as::<_, MediaCollection>(&sql)
            .bind(id)
            .bind(id)
            .bind(account_id)
            .bind(account_id)
            .bind(id)
            .fetch_all(db.postgres_pool().expect("postgres handle"))
            .await?),
    }
}

/// Get quality signals for a work.
pub async fn work_quality_signals(db: &Database, work_id: &str) -> Result<Vec<QualitySignal>> {
    let sqlite = "SELECT id, work_id, signal_kind, value, weight, \\
                  source, computed_at FROM quality_signals WHERE work_id = ?";
    let postgres = "SELECT id, work_id, signal_kind, value, weight, \\
                    source, computed_at FROM quality_signals WHERE work_id = ?";
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, QualitySignal>(&db.sql(sqlite, postgres))
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, QualitySignal>(&db.sql(sqlite, postgres))
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
    let postgres = r"INSERT INTO creators (id, kind, pseud_id, display_name, source_key,
        source_creator_id, canonical_url, verified_at, created_at, updated_at, version)
        VALUES (?::uuid, ?, ?::uuid, ?, ?, ?, ?, ?, ?, ?, 1)";
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
    let sqlite = "INSERT INTO distributors (id, kind, name, source_key, canonical_url,
        created_at, updated_at, version) VALUES (?, ?, ?, ?, ?, ?, ?, 1)";
    let postgres = "INSERT INTO distributors (id, kind, name, source_key, canonical_url,
        created_at, updated_at, version) VALUES (?::uuid, ?, ?, ?, ?, ?, ?, 1)";
    let sql = crate::sql_owned(db, sqlite.to_owned(), postgres.to_owned());
    run!(db, sql, |q| {
        q.bind(&id)
            .bind(&distributor.kind)
            .bind(distributor.name)
            .bind(distributor.source_key)
            .bind(distributor.canonical_url)
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
    let sqlite = "INSERT INTO media_collections (id, collection_kind, title, description,
        owning_account_id, visibility, created_at, updated_at, version)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1)";
    let postgres = "INSERT INTO media_collections (id, collection_kind, title, description,
        owning_account_id, visibility, created_at, updated_at, version)
        VALUES (?::uuid, ?, ?, ?, ?::uuid, ?, ?, ?, 1)";
    let sql = crate::sql_owned(db, sqlite.to_owned(), postgres.to_owned());
    run!(db, sql, |q| {
        q.bind(&id)
            .bind(&collection.kind)
            .bind(collection.title)
            .bind(collection.description)
            .bind(collection.owning_account_id)
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
    let sql = &db.sql(sqlite, postgres);
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
    let sql = &db.sql(sqlite, postgres);
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
    let (rows, total, _) = list_media_filtered(db, Some(query), account_id, limit, None).await?;
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
            rating: "general".to_string(),
            visibility: "public".to_string(),
            lifecycle: "published".to_string(),
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
    fn eligibility_filter_no_session_public_published_only() {
        let (sqlite, postgres, values) = eligibility_filter(None);
        assert_eq!(
            sqlite,
            "(w.lifecycle = 'published' AND w.visibility = 'public' AND w.rating IN ('general', 'teen'))"
        );
        assert_eq!(
            postgres,
            "(w.lifecycle = 'published' AND w.visibility = 'public' AND w.rating IN ('general', 'teen'))"
        );
        assert!(values.is_empty());
    }

    #[test]
    fn eligibility_filter_with_session_adds_restricted_and_own_works() {
        let (sqlite, postgres, values) = eligibility_filter(Some("me"));
        // A session sees public and restricted, but never unlisted — that
        // is link-reachable only. Ownership goes through the pseud, and
        // the PG twin casts the account bind to uuid. Drafts never list.
        // The facet qualifies columns with the `w` alias because the
        // consuming FROM aliases works as `w`.
        assert!(sqlite.contains("w.lifecycle = 'published'"));
        assert!(sqlite.contains("w.visibility IN ('public', 'restricted')"));
        assert!(!sqlite.contains("unlisted"));
        assert!(sqlite.contains("w.owner_pseud_id IN (SELECT id FROM pseuds"));
        assert!(postgres.contains("CAST(viewer.id AS TEXT) = ?"));
        assert!(postgres.contains("viewer.age_state"));
        assert!(postgres.contains("settings.max_rating"));
        assert_eq!(values, vec!["me".to_string()]);
    }
}

/// A file of a work (e.g. a PDF, EPUB, or text file).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromRow)]
pub struct MediaFile {
    pub id: String,
    pub work_id: String,
    pub edition_kind: String,
    pub url: Option<String>,
    pub size_bytes: Option<i64>,
    pub mime_type: Option<String>,
    pub checksum: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
}

/// List editions for a work. The route has already applied the direct-door
/// eligibility rule via `find_media`; this returns every edition row of the
/// work. PG twin: UUID ids decode as text, the bind casts to uuid.
pub async fn list_media_editions(db: &Database, work_id: &str) -> Result<Vec<MediaEdition>> {
    let sqlite = "SELECT id, work_id, edition_kind, label, parent_edition_id, \
                  published_at, created_at, updated_at, version \
                  FROM media_editions WHERE work_id = ? ORDER BY created_at DESC"
        .to_string();
    let postgres = "SELECT id::text AS id, work_id::text AS work_id, edition_kind, label, \
                    parent_edition_id::text AS parent_edition_id, published_at, \
                    created_at, updated_at, version::bigint \
                    FROM media_editions WHERE work_id = ?::uuid ORDER BY created_at DESC"
        .to_string();
    let sql = &db.sql(&sqlite, &postgres);
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, MediaEdition>(sql)
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, MediaEdition>(sql)
                .bind(work_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows)
}

/// Find a single edition by id.
pub async fn find_media_edition(db: &Database, id: &str) -> Result<Option<MediaEdition>> {
    let sql = &db.sql(
        "SELECT id, work_id, edition_kind, label, parent_edition_id, published_at, created_at, updated_at, version
         FROM media_editions WHERE id = ?",
        "SELECT id::text AS id, work_id::text AS work_id, edition_kind, label, parent_edition_id::text AS parent_edition_id, published_at, created_at, updated_at, version::bigint
         FROM media_editions WHERE id = ?::uuid",
    );
    Ok(match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, MediaEdition>(sql)
                .bind(id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, MediaEdition>(sql)
                .bind(id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    })
}

pub async fn create_media_file(
    db: &Database,
    work_id: &str,
    edition_kind: &str,
    checksum: &str,
    size_bytes: i64,
    mime_type: &str,
) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let sql = sql_owned(
        db,
        "INSERT INTO media_files (id, work_id, edition_kind, checksum, size_bytes, mime_type, created_at, updated_at, version)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1)"
            .to_string(),
        "INSERT INTO media_files (id, work_id, edition_kind, checksum, size_bytes, mime_type, created_at, updated_at, version)
         VALUES (?::uuid, ?::uuid, ?, ?, ?, ?, ?, ?, 1)"
            .to_string(),
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(work_id)
                .bind(edition_kind)
                .bind(checksum)
                .bind(size_bytes)
                .bind(mime_type)
                .bind(&now)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(work_id)
                .bind(edition_kind)
                .bind(checksum)
                .bind(size_bytes)
                .bind(mime_type)
                .bind(&now)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(id)
}

/// List files for a work. The route has already applied the direct-door
/// eligibility rule via `find_media`; this returns every file row of the
/// work. PG twin: UUID ids decode as text, the bind casts to uuid.
pub async fn list_media_files(db: &Database, work_id: &str) -> Result<Vec<MediaFile>> {
    let sqlite = "SELECT id, work_id, edition_kind, url, size_bytes, mime_type, checksum, \
                  created_at, updated_at, version \
                  FROM media_files WHERE work_id = ? ORDER BY created_at"
        .to_string();
    let postgres = "SELECT id::text AS id, work_id::text AS work_id, edition_kind, url, \
                    size_bytes, mime_type, checksum, created_at, updated_at, version::bigint \
                    FROM media_files WHERE work_id = ?::uuid ORDER BY created_at"
        .to_string();
    let sql = &db.sql(&sqlite, &postgres);
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, MediaFile>(sql)
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, MediaFile>(sql)
                .bind(work_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows)
}

/// List media in a canon, with the same eligibility filter as list_media_filtered.
/// Returns (items, canon_name) or error. 404 if canon doesn't exist.
pub async fn canon_media(
    db: &Database,
    canon_id: &str,
    account_id: Option<&str>,
) -> Result<(Vec<MediaRecord>, String)> {
    // First: check canon exists and get its name.
    let name = {
        let sqlite = "SELECT name FROM canons WHERE id = ?".to_string();
        let postgres = "SELECT name FROM canons WHERE id = ?::uuid".to_string();
        let sql = &db.sql(&sqlite, &postgres);
        match db.backend() {
            Backend::Sqlite => {
                sqlx::query_scalar::<_, String>(sql)
                    .bind(canon_id)
                    .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                    .await?
            }
            Backend::Postgres => {
                sqlx::query_scalar::<_, String>(sql)
                    .bind(canon_id)
                    .fetch_optional(db.postgres_pool().expect("postgres handle"))
                    .await?
            }
        }
    };
    let name = name.ok_or_else(|| anyhow::anyhow!("canon not found"))?;

    // Second: list works in the canon, with eligibility filter.
    let (eligible_sqlite, eligible_pg, values) = eligibility_filter(account_id);
    let sqlite = format!(
        "SELECT w.id, w.title, w.summary, w.format, w.rating, w.visibility, w.lifecycle,
        p.account_id AS owning_account_id, w.created_at, w.updated_at, w.version
        FROM canon_works cw
        JOIN works w ON w.id = cw.work_id
        JOIN pseuds p ON p.id = w.owner_pseud_id
        WHERE cw.canon_id = ? AND {eligible_sqlite}
        ORDER BY cw.position, w.created_at DESC, w.id ASC LIMIT 50"
    );
    let postgres = format!(
        "SELECT w.id::text AS id, w.title, w.summary, w.format, w.rating, w.visibility, w.lifecycle,
        p.account_id::text AS owning_account_id, w.created_at, w.updated_at, w.version::bigint
        FROM canon_works cw
        JOIN works w ON w.id = cw.work_id
        JOIN pseuds p ON p.id = w.owner_pseud_id
        WHERE cw.canon_id = ?::uuid AND {eligible_pg}
        ORDER BY cw.position, w.created_at DESC, w.id ASC LIMIT 50"
    );
    let sql = db.sql(&sqlite, &postgres);
    let rows = match db.backend() {
        Backend::Sqlite => {
            let mut query = sqlx::query_as::<_, MediaRecord>(&sql).bind(canon_id);
            for value in &values {
                query = query.bind(value);
            }
            query
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            let mut query = sqlx::query_as::<_, MediaRecord>(&sql).bind(canon_id);
            for value in &values {
                query = query.bind(value);
            }
            query
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok((rows, name))
}

/// List media in a space, with the same eligibility filter as list_media_filtered.
/// Returns (items, space_name) or error. 404 if space doesn't exist.
pub async fn space_media(
    db: &Database,
    space_id: &str,
    account_id: Option<&str>,
) -> Result<(Vec<MediaRecord>, String)> {
    let name = {
        let sqlite = "SELECT name FROM spaces WHERE id = ?".to_string();
        let postgres = "SELECT name FROM spaces WHERE id = ?::uuid".to_string();
        let sql = &db.sql(&sqlite, &postgres);
        match db.backend() {
            Backend::Sqlite => {
                sqlx::query_scalar::<_, String>(sql)
                    .bind(space_id)
                    .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                    .await?
            }
            Backend::Postgres => {
                sqlx::query_scalar::<_, String>(sql)
                    .bind(space_id)
                    .fetch_optional(db.postgres_pool().expect("postgres handle"))
                    .await?
            }
        }
    };
    let name = name.ok_or_else(|| anyhow::anyhow!("space not found"))?;

    let (eligible_sqlite, eligible_pg, values) = eligibility_filter(account_id);
    let sqlite = format!(
        "SELECT w.id, w.title, w.summary, w.format, w.rating, w.visibility, w.lifecycle,
        p.account_id AS owning_account_id, w.created_at, w.updated_at, w.version
        FROM space_works sw
        JOIN works w ON w.id = sw.work_id
        JOIN pseuds p ON p.id = w.owner_pseud_id
        WHERE sw.space_id = ? AND {eligible_sqlite}
        ORDER BY sw.position, w.created_at DESC, w.id ASC LIMIT 50"
    );
    let postgres = format!(
        "SELECT w.id::text AS id, w.title, w.summary, w.format, w.rating, w.visibility, w.lifecycle,
        p.account_id::text AS owning_account_id, w.created_at, w.updated_at, w.version::bigint
        FROM space_works sw
        JOIN works w ON w.id = sw.work_id
        JOIN pseuds p ON p.id = w.owner_pseud_id
        WHERE sw.space_id = ?::uuid AND {eligible_pg}
        ORDER BY sw.position, w.created_at DESC, w.id ASC LIMIT 50"
    );
    let sql = db.sql(&sqlite, &postgres);
    let rows = match db.backend() {
        Backend::Sqlite => {
            let mut query = sqlx::query_as::<_, MediaRecord>(&sql).bind(space_id);
            for value in &values {
                query = query.bind(value);
            }
            query
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            let mut query = sqlx::query_as::<_, MediaRecord>(&sql).bind(space_id);
            for value in &values {
                query = query.bind(value);
            }
            query
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok((rows, name))
}
