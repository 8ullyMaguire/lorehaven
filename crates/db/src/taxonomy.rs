//! Taxonomy repository: nodes, aliases, tags.
//!
//! Spec §15.1–15.3. Both dialects.

use crate::{Backend, Database};
use anyhow::Result;
use serde::Serialize;

/// A taxonomy node.
#[derive(Debug, Clone, Serialize)]
pub struct TaxonomyNode {
    pub id: String,
    pub kind: String,
    pub canonical: String,
    pub norm: String,
    pub created_at: String,
}

/// A taxonomy alias.
#[derive(Debug, Clone, Serialize)]
pub struct TaxonomyAlias {
    pub alias: String,
    pub norm: String,
    pub node_id: String,
    pub source: String,
}

/// A work tag.
#[derive(Debug, Clone, Serialize)]
pub struct WorkTag {
    pub work_id: String,
    pub node_id: String,
    pub weight: i64,
    pub added_at: String,
}

/// Canonical form helper.
fn canonical_form(text: &str) -> String {
    text.trim().to_lowercase()
}

/// Search nodes by kind and prefix.
pub async fn search_nodes(
    db: &Database,
    kind: Option<&str>,
    prefix: &str,
    limit: i64,
) -> Result<Vec<TaxonomyNode>> {
    let sql = db.sql(
        "SELECT id, kind, canonical, norm, created_at FROM taxonomy_nodes
         WHERE (?1 IS NULL OR kind = ?1) AND norm LIKE ?2 || '%'
         ORDER BY norm ASC LIMIT ?3",
        "SELECT id::text, kind, canonical, norm, created_at FROM taxonomy_nodes
         WHERE ($1 IS NULL OR kind = $1) AND norm LIKE $2 || '%'
         ORDER BY norm ASC LIMIT $3",
    );
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String, String, String, String, String)>(&sql)
                .bind(kind)
                .bind(prefix)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String, String, String, String, String)>(&sql)
                .bind(kind)
                .bind(prefix)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(|(id, kind, canonical, norm, created_at)| TaxonomyNode {
            id,
            kind,
            canonical,
            norm,
            created_at,
        })
        .collect())
}

/// Get a node by id.
pub async fn node_by_id(db: &Database, id: &str) -> Result<Option<TaxonomyNode>> {
    let sql = db.sql(
        "SELECT id, kind, canonical, norm, created_at FROM taxonomy_nodes WHERE id = ?",
        "SELECT id::text, kind, canonical, norm, created_at FROM taxonomy_nodes WHERE id = $1::uuid",
    );
    let row = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String, String, String, String, String)>(&sql)
                .bind(id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String, String, String, String, String)>(&sql)
                .bind(id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(
        row.map(|(id, kind, canonical, norm, created_at)| TaxonomyNode {
            id,
            kind,
            canonical,
            norm,
            created_at,
        }),
    )
}

/// Create a taxonomy node.
pub async fn create_node(db: &Database, kind: &str, canonical: &str) -> Result<TaxonomyNode> {
    let id = uuid::Uuid::new_v4().to_string();
    let norm = canonical_form(canonical);
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at) VALUES (?, ?, ?, ?, ?)",
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at) VALUES ($1::uuid, $2, $3, $4, $5)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(kind)
                .bind(canonical)
                .bind(&norm)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(kind)
                .bind(canonical)
                .bind(&norm)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(TaxonomyNode {
        id,
        kind: kind.to_owned(),
        canonical: canonical.to_owned(),
        norm,
        created_at: now,
    })
}

/// Create an alias.
pub async fn create_alias(
    db: &Database,
    alias: &str,
    node_id: &str,
    source: &str,
) -> Result<TaxonomyAlias> {
    let norm = canonical_form(alias);
    let sql = db.sql(
        "INSERT INTO taxonomy_aliases (alias, norm, node_id, source) VALUES (?, ?, ?, ?)",
        "INSERT INTO taxonomy_aliases (alias, norm, node_id, source) VALUES ($1, $2, $3::uuid, $4)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(alias)
                .bind(&norm)
                .bind(node_id)
                .bind(source)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(alias)
                .bind(&norm)
                .bind(node_id)
                .bind(source)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(TaxonomyAlias {
        alias: alias.to_owned(),
        norm,
        node_id: node_id.to_owned(),
        source: source.to_owned(),
    })
}

/// Tag a work.
pub async fn tag_work(db: &Database, work_id: &str, node_id: &str, weight: i64) -> Result<WorkTag> {
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO work_tags (work_id, node_id, weight, added_at) VALUES (?, ?, ?, ?)",
        "INSERT INTO work_tags (work_id, node_id, weight, added_at) VALUES ($1::uuid, $2::uuid, $3, $4)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(work_id)
                .bind(node_id)
                .bind(weight)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(work_id)
                .bind(node_id)
                .bind(weight)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(WorkTag {
        work_id: work_id.to_owned(),
        node_id: node_id.to_owned(),
        weight,
        added_at: now,
    })
}
