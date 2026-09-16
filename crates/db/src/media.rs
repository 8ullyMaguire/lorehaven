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

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use lorehaven_domain::media::{CollectionKind, CreatorKind, DistributorKind};
use lorehaven_domain::query::QueryAst;
use lorehaven_domain::query_sql::render_query;

use crate::{Backend, Database};

// ---------------------------------------------------------------------------
// Row types
// ---------------------------------------------------------------------------

/// A creator record — a local pseud or an external platform account
/// (spec §32.3.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct MediaCreator {
    pub id: String,
    pub work_id: String,
    pub creator_id: String,
    pub role: String,
    pub position: i64,
    pub created_at: String,
}

/// A distributor — a platform, publisher, archive, zine, or self-host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct Distributor {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub source_key: Option<String>,
    pub canonical_url: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
}

/// Distribution edge: how a work reaches readers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct Distributorship {
    pub id: String,
    pub work_id: String,
    pub distributor_id: String,
    pub role: String,
    pub detail_url: Option<String>,
    pub created_at: String,
}

/// A typed collection: series, anthology, reading list, etc.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct MediaCollection {
    pub id: String,
    pub collection_kind: String,
    pub owning_account_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub visibility: String,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
}

/// Membership of a collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct MediaCollectionItem {
    pub id: String,
    pub collection_id: String,
    pub work_id: String,
    pub position: i64,
    pub note: Option<String>,
    pub added_by_account_id: Option<String>,
    pub added_at: String,
}

/// Publication history for a work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
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

/// Rights and lending statement for a work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct MediaRights {
    pub work_id: String,
    pub license: String,
    pub rights_statement: Option<String>,
    pub lending_class: String,
    pub updated_at: String,
    pub version: i64,
}

/// A quality signal attached to a work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct QualitySignal {
    pub id: String,
    pub work_id: String,
    pub signal_kind: String,
    pub value: i64,
    pub weight: i64,
    pub source: String,
    pub computed_at: String,
}

/// A work with its media metadata — the primary query result type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct MediaRecord {
    pub id: String,
    pub title: String,
    pub summary: Option<String>,
    pub format: String,
    pub visibility: String,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
}

// ---------------------------------------------------------------------------
// Filter compilation
// ---------------------------------------------------------------------------

/// A filter facet as SQL (both dialects) and the values to bind.
struct MediaFacet {
    sqlite: String,
    postgres: String,
    values: Vec<String>,
}

/// Build the shared `WHERE` body for a media query.
///
/// Visibility is part of every clause: an aggregation must never leak
/// a restricted work (spec §7.6). The account predicate is bound once
/// and each facet compares against the account so the bind order stays
/// identical in both dialects.
fn media_filter(query: &QueryAst, account_id: Option<&str>) -> (String, String, Vec<String>) {
    let mut facets: Vec<MediaFacet> = Vec::new();

    if let Some(account) = account_id {
        facets.push(MediaFacet {
            sqlite: "(works.visibility = 'public' OR works.visibility = 'restricted' OR works.account_id = ?)".to_string(),
            postgres: "(works.visibility = 'public' OR works.visibility = 'restricted' OR works.account_id = ?)".to_string(),
            values: vec![account.to_string()],
        });
    } else {
        facets.push(MediaFacet {
            sqlite: "works.visibility = 'public'".to_string(),
            postgres: "works.visibility = 'public'".to_string(),
            values: Vec::new(),
        });
    }

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

/// Look up a single media record by id.
pub async fn find_media(db: &Database, id: &str) -> Result<Option<MediaRecord>> {
    let sqlite = "SELECT w.id, w.title, w.summary, w.format, w.visibility, \
                  w.created_at, w.updated_at, w.version \
                  FROM works w WHERE w.id = ?";
    let postgres = "SELECT w.id, w.title, w.summary, w.format, w.visibility, \
                     w.created_at, w.updated_at, w.version \
                     FROM works w WHERE w.id = ?";
    let row: Option<MediaRecord> = sqlx::query_as::<_, MediaRecord>(&crate::sql_owned(
        db,
        sqlite.to_string(),
        postgres.to_string(),
    ))
    .bind(id)
    .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
    .await?;
    Ok(row)
}

/// List media with filtering, pagination, and eligibility.
pub async fn list_media_filtered(
    db: &Database,
    query: &QueryAst,
    _account_id: Option<&str>,
    limit: i64,
) -> Result<(Vec<MediaRecord>, i64)> {
    let (where_sql, where_pg, values) = media_filter(query, _account_id);

    let sqlite = format!(
        "SELECT w.id, w.title, w.summary, w.format, w.visibility, \
                w.created_at, w.updated_at, w.version \
         FROM works w \
         WHERE {where_sql} \
         ORDER BY w.created_at DESC \
         LIMIT ?"
    );
    let postgres = format!(
        "SELECT w.id, w.title, w.summary, w.format, w.visibility, \
                w.created_at, w.updated_at, w.version \
         FROM works w \
         WHERE {where_pg} \
         ORDER BY w.created_at DESC \
         LIMIT ?"
    );

    let count_sql = format!("SELECT COUNT(*) FROM works w WHERE {where_sql}");
    let count_pg = format!("SELECT COUNT(*) FROM works w WHERE {where_pg}");

    let count: (i64,) = sqlx::query_as::<_, (i64,)>(&db.sql(&count_sql, &count_pg))
        .fetch_one(db.sqlite_pool().expect("sqlite handle"))
        .await?;

    let mut bound: Vec<String> = vec![limit.to_string()];
    bound.extend(values);

    let rows: Vec<MediaRecord> = sqlx::query_as::<_, MediaRecord>(&db.sql(&sqlite, &postgres))
        .bind(limit)
        .fetch_all(db.sqlite_pool().expect("sqlite handle"))
        .await?;

    Ok((rows, count.0))
}

/// List works by a single creator (spec §32.3.2).
pub async fn creator_media(
    db: &Database,
    creator_id: &str,
    _account_id: Option<&str>,
) -> Result<Vec<MediaRecord>> {
    let sql = "SELECT w.id, w.title, w.summary, w.format, w.visibility, \
               w.created_at, w.updated_at, w.version \
               FROM works w \
               JOIN media_creators mc ON mc.work_id = w.id \
               WHERE mc.creator_id = ?";
    let rows: Vec<MediaRecord> =
        sqlx::query_as::<_, MediaRecord>(&crate::sql_owned(db, sql.to_string(), sql.to_string()))
            .bind(creator_id)
            .fetch_all(db.sqlite_pool().expect("sqlite handle"))
            .await?;
    Ok(rows)
}

/// List works distributed through a single distributor.
pub async fn distributor_media(
    db: &Database,
    distributor_id: &str,
    _account_id: Option<&str>,
) -> Result<Vec<MediaRecord>> {
    let sql = "SELECT w.id, w.title, w.summary, w.format, w.visibility, \
               w.created_at, w.updated_at, w.version \
               FROM works w \
               JOIN distributorships ds ON ds.work_id = w.id \
               WHERE ds.distributor_id = ?";
    let rows: Vec<MediaRecord> =
        sqlx::query_as::<_, MediaRecord>(&crate::sql_owned(db, sql.to_string(), sql.to_string()))
            .bind(distributor_id)
            .fetch_all(db.sqlite_pool().expect("sqlite handle"))
            .await?;
    Ok(rows)
}

/// List works in a single collection.
pub async fn collection_media(
    db: &Database,
    collection_id: &str,
    _account_id: Option<&str>,
) -> Result<Vec<MediaRecord>> {
    let sql = "SELECT w.id, w.title, w.summary, w.format, w.visibility, \
               w.created_at, w.updated_at, w.version \
               FROM works w \
               JOIN media_collection_items mci ON mci.work_id = w.id \
               WHERE mci.collection_id = ?";
    let rows: Vec<MediaRecord> =
        sqlx::query_as::<_, MediaRecord>(&crate::sql_owned(db, sql.to_string(), sql.to_string()))
            .bind(collection_id)
            .fetch_all(db.sqlite_pool().expect("sqlite handle"))
            .await?;
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Creator operations
// ---------------------------------------------------------------------------

/// List all creators.
pub async fn list_creators(db: &Database) -> Result<Vec<Creator>> {
    let sql = "SELECT id, kind, pseud_id, display_name, source_key, \
               source_creator_id, canonical_url, verified_at, \
               created_at, updated_at, version FROM creators ORDER BY display_name COLLATE NOCASE";
    let rows: Vec<Creator> =
        sqlx::query_as::<_, Creator>(&crate::sql_owned(db, sql.to_string(), sql.to_string()))
            .fetch_all(db.sqlite_pool().expect("sqlite handle"))
            .await?;
    Ok(rows)
}

/// Look up a single creator.
pub async fn find_creator(db: &Database, id: &str) -> Result<Option<Creator>> {
    let sql = "SELECT id, kind, pseud_id, display_name, source_key, \
               source_creator_id, canonical_url, verified_at, \
               created_at, updated_at, version FROM creators WHERE id = ?";
    let row: Option<Creator> =
        sqlx::query_as::<_, Creator>(&crate::sql_owned(db, sql.to_string(), sql.to_string()))
            .bind(id)
            .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
            .await?;
    Ok(row)
}

// ---------------------------------------------------------------------------
// Distributor operations
// ---------------------------------------------------------------------------

/// List all distributors.
pub async fn list_distributors(db: &Database) -> Result<Vec<Distributor>> {
    let sql = "SELECT id, name, kind, source_key, canonical_url, \
               created_at, updated_at, version FROM distributors ORDER BY name COLLATE NOCASE";
    let rows: Vec<Distributor> =
        sqlx::query_as::<_, Distributor>(&crate::sql_owned(db, sql.to_string(), sql.to_string()))
            .fetch_all(db.sqlite_pool().expect("sqlite handle"))
            .await?;
    Ok(rows)
}

/// Look up a single distributor.
pub async fn find_distributor(db: &Database, id: &str) -> Result<Option<Distributor>> {
    let sql = "SELECT id, name, kind, source_key, canonical_url, \
               created_at, updated_at, version FROM distributors WHERE id = ?";
    let row: Option<Distributor> =
        sqlx::query_as::<_, Distributor>(&crate::sql_owned(db, sql.to_string(), sql.to_string()))
            .bind(id)
            .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
            .await?;
    Ok(row)
}

// ---------------------------------------------------------------------------
// Collection operations
// ---------------------------------------------------------------------------

/// List all collections.
pub async fn list_collections(
    db: &Database,
    _account_id: Option<&str>,
) -> Result<Vec<MediaCollection>> {
    let sql = "SELECT id, collection_kind, owning_account_id, title, \
               description, visibility, created_at, updated_at, version \
               FROM media_collections ORDER BY title COLLATE NOCASE";
    let rows: Vec<MediaCollection> = sqlx::query_as::<_, MediaCollection>(&crate::sql_owned(
        db,
        sql.to_string(),
        sql.to_string(),
    ))
    .fetch_all(db.sqlite_pool().expect("sqlite handle"))
    .await?;
    Ok(rows)
}

/// Look up a single collection.
pub async fn find_collection(db: &Database, id: &str) -> Result<Option<MediaCollection>> {
    let sql = "SELECT id, collection_kind, owning_account_id, title, \
               description, visibility, created_at, updated_at, version \
               FROM media_collections WHERE id = ?";
    let row: Option<MediaCollection> = sqlx::query_as::<_, MediaCollection>(&crate::sql_owned(
        db,
        sql.to_string(),
        sql.to_string(),
    ))
    .bind(id)
    .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
    .await?;
    Ok(row)
}

// ---------------------------------------------------------------------------
// Rights operations
// ---------------------------------------------------------------------------

/// Look up rights for a work.
pub async fn find_rights(db: &Database, work_id: &str) -> Result<Option<MediaRights>> {
    let sql = "SELECT work_id, license, rights_statement, lending_class, \
               updated_at, version FROM media_rights WHERE work_id = ?";
    let row: Option<MediaRights> =
        sqlx::query_as::<_, MediaRights>(&crate::sql_owned(db, sql.to_string(), sql.to_string()))
            .bind(work_id)
            .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
            .await?;
    Ok(row)
}

// ---------------------------------------------------------------------------
// Quality signal operations
// ---------------------------------------------------------------------------

/// Get quality signals for a work.
pub async fn work_quality_signals(db: &Database, work_id: &str) -> Result<Vec<QualitySignal>> {
    let sql = "SELECT id, work_id, signal_kind, value, weight, \
               source, computed_at FROM quality_signals WHERE work_id = ?";
    let rows: Vec<QualitySignal> =
        sqlx::query_as::<_, QualitySignal>(&crate::sql_owned(db, sql.to_string(), sql.to_string()))
            .bind(work_id)
            .fetch_all(db.sqlite_pool().expect("sqlite handle"))
            .await?;
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Write operations
// ---------------------------------------------------------------------------

/// Insert a new creator.
pub async fn create_creator(db: &Database, creator: &NewCreator<'_>) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let sqlite = r#"INSERT INTO creators (id, kind, pseud_id, display_name, source_key,
                              source_creator_id, canonical_url, verified_at,
                              created_at, updated_at, version)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1)"#;
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
    let sqlite = r#"INSERT INTO distributors (id, name, kind, source_key, canonical_url,
                                  created_at, updated_at, version)
        VALUES (?, ?, ?, ?, ?, ?, ?, 1)"#;
    let postgres = sqlite;
    let sql = crate::sql_owned(db, sqlite.to_string(), postgres.to_string());
    run!(db, sql, |q| {
        q.bind(&id)
            .bind(distributor.name)
            .bind(distributor.kind.as_str())
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
    let sqlite = r#"INSERT INTO media_collections (id, collection_kind, owning_account_id,
                                       title, description, visibility,
                                       created_at, updated_at, version)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1)"#;
    let postgres = sqlite;
    let sql = crate::sql_owned(db, sqlite.to_string(), postgres.to_string());
    run!(db, sql, |q| {
        q.bind(&id)
            .bind(collection.kind.as_str())
            .bind(collection.owning_account_id)
            .bind(collection.title)
            .bind(collection.description)
            .bind(collection.visibility)
            .bind(&now)
            .bind(&now)
    })
    .await?;
    Ok(id)
}

// ---------------------------------------------------------------------------
// Row constructors
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
    pub kind: DistributorKind,
    pub source_key: Option<&'a str>,
    pub canonical_url: Option<&'a str>,
}

/// A new collection to insert.
pub struct NewCollection<'a> {
    pub kind: CollectionKind,
    pub owning_account_id: Option<&'a str>,
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub visibility: &'a str,
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
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            version: 1,
        };
        assert_eq!(rec.format, "prose");
        assert_eq!(rec.visibility, "public");
    }

    #[test]
    fn creator_kind_round_trip() {
        use std::str::FromStr;
        for kind in CreatorKind::ALL {
            assert_eq!(CreatorKind::from_str(kind.as_str()).ok(), Some(*kind));
        }
    }

    #[test]
    fn media_format_round_trip() {
        use lorehaven_domain::media::MediaFormat;
        use std::str::FromStr;
        for fmt in MediaFormat::ALL {
            assert_eq!(MediaFormat::from_str(fmt.as_str()).ok(), Some(*fmt));
        }
    }
}
