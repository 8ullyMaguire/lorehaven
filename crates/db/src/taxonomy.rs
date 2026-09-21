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

/// Search taxonomy nodes by prefix, then fall back to fuzzy matching.
///
/// Exact prefix matches are returned first (up to `limit`), then fuzzy
/// matches fill any remaining slots. Fuzzy matching uses Levenshtein
/// similarity with a floor of `lexicon::taxonomy::FUZZY_SIMILARITY_FLOOR`.
pub async fn search_nodes_fuzzy(
    db: &Database,
    kind: Option<&str>,
    query: &str,
    limit: i64,
) -> Result<Vec<TaxonomyNode>> {
    use lorehaven_domain::taxonomy::{canonical_form, fuzzy_match};
    use std::collections::HashMap;

    let prefix = query.trim();
    let normalized = canonical_form(prefix);

    // 1. Exact prefix matches (normalized so LIKE is case-consistent).
    let sql_prefix = db.sql(
        "SELECT id, kind, canonical, norm, created_at FROM taxonomy_nodes
         WHERE (?1 IS NULL OR kind = ?1) AND norm LIKE ?2 || '%'
         ORDER BY norm ASC",
        "SELECT id::text, kind, canonical, norm, created_at FROM taxonomy_nodes
         WHERE ($1 IS NULL OR kind = $1) AND norm LIKE $2 || '%'
         ORDER BY norm ASC",
    );
    let prefix_rows: Vec<(String, String, String, String, String)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql_prefix)
                .bind(kind)
                .bind(&normalized)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql_prefix)
                .bind(kind)
                .bind(&normalized)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    let prefix_nodes: Vec<TaxonomyNode> = prefix_rows
        .iter()
        .map(|(id, kind, canonical, norm, created_at)| TaxonomyNode {
            id: id.clone(),
            kind: kind.clone(),
            canonical: canonical.clone(),
            norm: norm.clone(),
            created_at: created_at.clone(),
        })
        .collect();

    let exact_count = prefix_nodes.len();
    if exact_count as i64 >= limit {
        return Ok(prefix_nodes);
    }

    // 2. Fuzzy fallback: query non-prefix nodes and rank by similarity.
    let remaining = (limit - exact_count as i64) as usize;
    let sql_rest = db.sql(
        "SELECT id, kind, canonical, norm, created_at FROM taxonomy_nodes
         WHERE (?1 IS NULL OR kind = ?1) AND norm NOT LIKE ?2 || '%'
         ORDER BY norm ASC",
        "SELECT id::text, kind, canonical, norm, created_at FROM taxonomy_nodes
         WHERE ($1 IS NULL OR kind = $1) AND norm NOT LIKE $2 || '%'
         ORDER BY norm ASC",
    );
    let rest_rows: Vec<(String, String, String, String, String)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql_rest)
                .bind(kind)
                .bind(&normalized)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql_rest)
                .bind(kind)
                .bind(&normalized)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };

    let candidates: Vec<(String, String)> = rest_rows
        .iter()
        .map(|(id, _, _, norm, _)| (id.clone(), norm.clone()))
        .collect();
    let fuzzy = fuzzy_match(&normalized, candidates, remaining);

    // Map fuzzy match ids back to full TaxonomyNode records.
    let lookup: HashMap<String, TaxonomyNode> = rest_rows
        .into_iter()
        .map(|(id, kind, canonical, norm, created_at)| {
            (
                id.clone(),
                TaxonomyNode {
                    id,
                    kind,
                    canonical,
                    norm,
                    created_at,
                },
            )
        })
        .collect();

    let mut result = prefix_nodes;
    for m in fuzzy {
        if let Some(node) = lookup.get(&m.id) {
            result.push(node.clone());
        }
    }
    Ok(result)
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

/// Get all node_ids tagged on a work, in insertion order.
pub async fn tags_for_work(db: &Database, work_id: &str) -> Result<Vec<String>> {
    let sql = db.sql(
        "SELECT node_id FROM work_tags WHERE work_id = ? ORDER BY added_at",
        "SELECT node_id FROM work_tags WHERE work_id = $1::uuid ORDER BY added_at",
    );
    let rows: Vec<String> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows)
}
