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
    /// §15.17: `curated` or `unverified`. A node created by a person through
    /// the taxonomy UI is `curated` — §19.4 quorum is what makes a node
    /// curated. A node auto-created from an exchange signal is `unverified`:
    /// usable immediately, and never presented as curated.
    ///
    /// Read this on every surface that shows a node. §15.17's second acceptance
    /// line is that an unverified entity is never rendered as curated, and the
    /// only way to honour that is for the status to travel with the node rather
    /// than be re-derived by each surface from whatever table it happened to
    /// read.
    pub review_status: String,
    /// §15.17: review priority, nothing else. Never a demand weight
    /// (§16.16.1), never a ranking input, and never a count of readers.
    pub signal_count: i64,
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

/// The normalised identity form for a node name.
///
/// Delegates to the domain's `normalise` rather than trimming and lowercasing
/// here. Two normalisers in one codebase is not a style question: `SLOW   BURN`
/// (doubled spacing) is `slow   burn` under `trim().to_lowercase()` and
/// `slow burn` under the domain rule, so the same submitted name produced two
/// different nodes depending on which function wrote the row — and the unique
/// index on `(kind, norm)` did not catch it, because both rows were internally
/// consistent.
///
/// §15.17's "a signal naming a known tag adds an alias rather than creating a
/// duplicate" is only true if there is one normaliser. The domain rule is also
/// the one the wire contract's tests pin, so it is the one that wins.
fn canonical_form(text: &str) -> String {
    lorehaven_domain::exchange::normalise(text)
}

/// Search nodes by kind and prefix.
pub async fn search_nodes(
    db: &Database,
    kind: Option<&str>,
    prefix: &str,
    limit: i64,
) -> Result<Vec<TaxonomyNode>> {
    let sql = db.sql(
        "SELECT id, kind, canonical, norm, created_at, review_status, CAST(signal_count AS BIGINT) FROM taxonomy_nodes
         WHERE (?1 IS NULL OR kind = ?1) AND norm LIKE ?2 || '%'
         ORDER BY norm ASC LIMIT ?3",
        "SELECT id::text, kind, canonical, norm, created_at, review_status, CAST(signal_count AS BIGINT) FROM taxonomy_nodes
         WHERE ($1 IS NULL OR kind = $1) AND norm LIKE $2 || '%'
         ORDER BY norm ASC LIMIT $3",
    );
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String, String, String, String, String, String, i64)>(&sql)
                .bind(kind)
                .bind(prefix)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String, String, String, String, String, String, i64)>(&sql)
                .bind(kind)
                .bind(prefix)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(
            |(id, kind, canonical, norm, created_at, review_status, signal_count)| TaxonomyNode {
                id,
                kind,
                canonical,
                norm,
                created_at,
                review_status,
                signal_count,
            },
        )
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
        "SELECT id, kind, canonical, norm, created_at, review_status, CAST(signal_count AS BIGINT) FROM taxonomy_nodes
         WHERE (?1 IS NULL OR kind = ?1) AND norm LIKE ?2 || '%'
         ORDER BY norm ASC",
        "SELECT id::text, kind, canonical, norm, created_at, review_status, CAST(signal_count AS BIGINT) FROM taxonomy_nodes
         WHERE ($1 IS NULL OR kind = $1) AND norm LIKE $2 || '%'
         ORDER BY norm ASC",
    );
    let prefix_rows: Vec<(String, String, String, String, String, String, i64)> = match db.backend()
    {
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
        .map(
            |(id, kind, canonical, norm, created_at, review_status, signal_count)| TaxonomyNode {
                id: id.clone(),
                kind: kind.clone(),
                canonical: canonical.clone(),
                norm: norm.clone(),
                created_at: created_at.clone(),
                review_status: review_status.clone(),
                signal_count: *signal_count,
            },
        )
        .collect();

    let exact_count = prefix_nodes.len();
    if exact_count as i64 >= limit {
        return Ok(prefix_nodes);
    }

    // 2. Fuzzy fallback: query non-prefix nodes and rank by similarity.
    let remaining = (limit - exact_count as i64) as usize;
    let sql_rest = db.sql(
        "SELECT id, kind, canonical, norm, created_at, review_status, CAST(signal_count AS BIGINT) FROM taxonomy_nodes
         WHERE (?1 IS NULL OR kind = ?1) AND norm NOT LIKE ?2 || '%'
         ORDER BY norm ASC",
        "SELECT id::text, kind, canonical, norm, created_at, review_status, CAST(signal_count AS BIGINT) FROM taxonomy_nodes
         WHERE ($1 IS NULL OR kind = $1) AND norm NOT LIKE $2 || '%'
         ORDER BY norm ASC",
    );
    let rest_rows: Vec<(String, String, String, String, String, String, i64)> = match db.backend() {
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
        .map(|(id, _k, _c, norm, _cr, _rs, _sc)| (id.clone(), norm.clone()))
        .collect();
    let fuzzy = fuzzy_match(&normalized, candidates, remaining);

    // Map fuzzy match ids back to full TaxonomyNode records.
    let lookup: HashMap<String, TaxonomyNode> = rest_rows
        .into_iter()
        .map(
            |(id, kind, canonical, norm, created_at, review_status, signal_count)| {
                (
                    id.clone(),
                    TaxonomyNode {
                        id,
                        kind,
                        canonical,
                        norm,
                        created_at,
                        review_status,
                        signal_count,
                    },
                )
            },
        )
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
        "SELECT id, kind, canonical, norm, created_at, review_status, CAST(signal_count AS BIGINT) FROM taxonomy_nodes WHERE id = ?",
        "SELECT id, kind, canonical, norm, created_at, review_status, CAST(signal_count AS BIGINT) FROM taxonomy_nodes WHERE id = $1",
    );
    let row = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String, String, String, String, String, String, i64)>(&sql)
                .bind(id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String, String, String, String, String, String, i64)>(&sql)
                .bind(id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.map(
        |(id, kind, canonical, norm, created_at, review_status, signal_count)| TaxonomyNode {
            id,
            kind,
            canonical,
            norm,
            created_at,
            review_status,
            signal_count,
        },
    ))
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
        // A node created through this function was created by a person. Its
        // authority came from §19.4 quorum, not from the exchange, so it starts
        // curated. The signal path uses `ensure_node_from_signal` instead, which
        // is the only way to get an unverified node — so "who made this" is
        // visible in which function was called, not in a boolean argument that
        // could be passed the wrong way round.
        review_status: "curated".to_owned(),
        signal_count: 0,
    })
}

/// §15.17: get or create a node from a name a signal carried.
///
/// The one function that can produce an **unverified** node, and that is the
/// point. §15.17's load-bearing claim is that a new name from a signal is
/// usable immediately — attachable to a work, searchable, visible in the tag
/// browser — while carrying `unverified`.
///
/// The upsert is not an optimisation. A check-then-insert races: two signals
/// naming the same brand-new tag on a young instance, which is the *common*
/// case there, and the loser gets a unique-constraint error instead of a node.
/// The `signal_count` increment is in the same statement for the same reason —
/// the count orders the review queue, and a lost increment makes a genuinely
/// urgent tag look idle.
///
/// A node that already exists is returned **unchanged**, including its
/// `review_status`. An existing curated node must not be demoted because someone
/// signalled the same name, and an existing unverified node must not be promoted
/// because enough people agreed — promotion is §19.4 quorum work, done by a
/// curator, never a side effect of counting.
pub async fn ensure_node_from_signal(
    db: &Database,
    kind: &str,
    canonical: &str,
) -> Result<TaxonomyNode> {
    let norm = canonical_form(canonical);
    let now = crate::identity::now_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();
    // Note what is *not* in the update arm: `canonical` and `review_status`.
    // Reinforcing a node must not rewrite a curator's chosen display form, and
    // agreement is not curation.
    let sql = db.sql(
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at, review_status, signal_count)
         VALUES (?, ?, ?, ?, ?, 'unverified', 1)
         ON CONFLICT (kind, norm) DO UPDATE SET signal_count = taxonomy_nodes.signal_count + 1",
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at, review_status, signal_count)
         VALUES ($1::uuid, $2, $3, $4, $5, 'unverified', 1)
         ON CONFLICT (kind, norm) DO UPDATE SET signal_count = taxonomy_nodes.signal_count + 1",
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
    let sql = db.sql(
        "SELECT id, kind, canonical, norm, created_at, review_status, signal_count
         FROM taxonomy_nodes WHERE kind = ? AND norm = ?",
        "SELECT id, kind, canonical, norm, created_at, review_status, CAST(signal_count AS BIGINT)
         FROM taxonomy_nodes WHERE kind = $1 AND norm = $2",
    );
    #[derive(sqlx::FromRow)]
    struct Row {
        id: String,
        kind: String,
        canonical: String,
        norm: String,
        created_at: String,
        review_status: String,
        signal_count: i64,
    }
    let row: Row = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(kind)
                .bind(&norm)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(kind)
                .bind(&norm)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(TaxonomyNode {
        id: row.id,
        kind: row.kind,
        canonical: row.canonical,
        norm: row.norm,
        created_at: row.created_at,
        review_status: row.review_status,
        signal_count: row.signal_count,
    })
}

/// Curate a node (§19.4 quorum work, gated to TL3 by the caller).
///
/// Sets the canonical form and the curated status. §15.17 requires the
/// originating signals to be retained as provenance, so nothing here touches
/// `exchange_signals` or `exchange_signal_entities`.
pub async fn curate_node(
    db: &Database,
    kind: &str,
    norm: &str,
    canonical_form: &str,
) -> Result<bool> {
    let sql = db.sql(
        "UPDATE taxonomy_nodes SET canonical = ?, review_status = 'curated'
         WHERE kind = ? AND norm = ?",
        "UPDATE taxonomy_nodes SET canonical = $1, review_status = 'curated'
         WHERE kind = $2 AND norm = $3",
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(canonical_form)
            .bind(kind)
            .bind(norm)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(canonical_form)
            .bind(kind)
            .bind(norm)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// The nodes awaiting review, most-reinforced first (§15.17's queue).
///
/// `ORDER BY signal_count DESC` is the whole point of the function, so it is in
/// the ORDER BY rather than a Rust sort — the `taxonomy_nodes_review` index is
/// built for it, and sorting a full table read in Rust is both slower and
/// ignores it.
pub async fn list_unverified_nodes(db: &Database, limit: i64) -> Result<Vec<TaxonomyNode>> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: String,
        kind: String,
        canonical: String,
        norm: String,
        created_at: String,
        review_status: String,
        signal_count: i64,
    }
    let sql = db.sql(
        // No CAST on the SQLite arm: `signal_count` is INTEGER there, and the
        // cast exists only because PostgreSQL types a bare COUNT-style integer
        // column INT4. Carrying it on both arms is harmless SQL that hides which
        // one actually needs it.
        "SELECT id, kind, canonical, norm, created_at, review_status, signal_count
         FROM taxonomy_nodes WHERE review_status = 'unverified'
         ORDER BY signal_count DESC, norm ASC LIMIT ?",
        "SELECT id, kind, canonical, norm, created_at, review_status, CAST(signal_count AS BIGINT)
         FROM taxonomy_nodes WHERE review_status = 'unverified'
         ORDER BY signal_count DESC, norm ASC LIMIT $1",
    );
    let rows: Vec<Row> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(|r| TaxonomyNode {
            id: r.id,
            kind: r.kind,
            canonical: r.canonical,
            norm: r.norm,
            created_at: r.created_at,
            review_status: r.review_status,
            signal_count: r.signal_count,
        })
        .collect())
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
        "INSERT INTO work_tags (work_id, node_id, weight, added_at) VALUES ($1::uuid, $2, $3, $4)",
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

/// List all tags (taxonomy nodes of kind 'tag') with work counts, for the Tags surface (spec §43.1).
pub async fn list_tags(
    db: &Database,
    limit: i64,
    offset: i64,
) -> Result<Vec<(String, String, String, i64)>> {
    let sql = db.sql(
        "SELECT id, canonical, kind, (
            SELECT COUNT(*) FROM work_tags wt WHERE wt.node_id = taxonomy_nodes.id
        ) AS work_count
         FROM taxonomy_nodes
         WHERE kind = 'tag'
         ORDER BY canonical ASC
         LIMIT ? OFFSET ?",
        "SELECT id::text, canonical, kind, (
            SELECT COUNT(*) FROM work_tags wt WHERE wt.node_id = taxonomy_nodes.id
        ) AS work_count
         FROM taxonomy_nodes
         WHERE kind = 'tag'
         ORDER BY canonical ASC
         LIMIT $1 OFFSET $2",
    );
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String, String, String, i64)>(&sql)
                .bind(limit)
                .bind(offset)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String, String, String, i64)>(&sql)
                .bind(limit)
                .bind(offset)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows)
}

/// Look up all tag names for a work (canonical forms).
pub async fn tag_names_for_work(db: &Database, work_id: &str) -> Result<Vec<String>> {
    let rows: Vec<(String,)> = match db.backend() {
        Backend::Sqlite => sqlx::query_as(
            "SELECT tn.canonical
             FROM work_tags wt
             JOIN taxonomy_nodes tn ON tn.id = wt.node_id
             WHERE wt.work_id = ? AND tn.kind = 'tag'",
        )
        .bind(work_id)
        .fetch_all(db.sqlite_pool().expect("sqlite"))
        .await
        .unwrap_or_default(),
        Backend::Postgres => sqlx::query_as(
            "SELECT tn.canonical
             FROM work_tags wt
             JOIN taxonomy_nodes tn ON tn.id = wt.node_id
             WHERE wt.work_id = $1::uuid AND tn.kind = 'tag'",
        )
        .bind(work_id)
        .fetch_all(db.postgres_pool().expect("postgres"))
        .await
        .unwrap_or_default(),
    };
    Ok(rows.into_iter().map(|(n,)| n).collect())
}

/// Tag names plus their work-tag weights for a work, ordered by canonical name.
pub async fn tag_weights_for_work(
    db: &Database,
    work_id: &str,
) -> Result<Vec<(String, i64)>, sqlx::Error> {
    let sql = db.sql(
        "SELECT tn.canonical, wt.weight
         FROM work_tags wt
         JOIN taxonomy_nodes tn ON tn.id = wt.node_id
         WHERE wt.work_id = ? AND tn.kind = 'tag'
         ORDER BY tn.canonical ASC",
        "SELECT tn.canonical, wt.weight::bigint
         FROM work_tags wt
         JOIN taxonomy_nodes tn ON tn.id = wt.node_id
         WHERE wt.work_id = $1::uuid AND tn.kind = 'tag'
         ORDER BY tn.canonical ASC",
    );
    let rows: Vec<(String, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows)
}

/// List all fandoms (taxonomy nodes of kind 'fandom') with work counts, for the Fandoms surface (spec §43.1).
pub async fn list_fandoms(
    db: &Database,
    limit: i64,
    offset: i64,
) -> Result<Vec<(String, String, i64)>> {
    let sql = db.sql(
        "SELECT id, canonical, (
            SELECT COUNT(*) FROM work_tags wt WHERE wt.node_id = taxonomy_nodes.id
        ) AS work_count
         FROM taxonomy_nodes
         WHERE kind = 'fandom'
         ORDER BY canonical ASC
         LIMIT ? OFFSET ?",
        "SELECT id::text, canonical, (
            SELECT COUNT(*) FROM work_tags wt WHERE wt.node_id = taxonomy_nodes.id
        ) AS work_count
         FROM taxonomy_nodes
         WHERE kind = 'fandom'
         ORDER BY canonical ASC
         LIMIT $1 OFFSET $2",
    );
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String, String, i64)>(&sql)
                .bind(limit)
                .bind(offset)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String, String, i64)>(&sql)
                .bind(limit)
                .bind(offset)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows)
}

/// List works tagged with a specific tag node, for the /tags/:tag surface (spec §43.1).
pub async fn works_by_tag(
    db: &Database,
    tag_canonical: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<(String, String, String, String)>> {
    let sql = db.sql(
        "SELECT w.id, w.title, p.handle, w.updated_at
         FROM works w
         JOIN work_tags wt ON wt.work_id = w.id
         JOIN taxonomy_nodes tn ON tn.id = wt.node_id
         JOIN pseuds p ON p.id = w.owner_pseud_id
         WHERE tn.norm = ? AND w.lifecycle = 'published' AND w.visibility = 'public'
         ORDER BY w.updated_at DESC
         LIMIT ? OFFSET ?",
        "SELECT w.id::text, w.title, p.handle, w.updated_at
         FROM works w
         JOIN work_tags wt ON wt.work_id = w.id
         JOIN taxonomy_nodes tn ON tn.id = wt.node_id
         JOIN pseuds p ON p.id = w.owner_pseud_id::text
         WHERE tn.norm = $1 AND w.lifecycle = 'published' AND w.visibility = 'public'
         ORDER BY w.updated_at DESC
         LIMIT $2 OFFSET $3",
    );
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String, String, String, String)>(&sql)
                .bind(tag_canonical)
                .bind(limit)
                .bind(offset)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String, String, String, String)>(&sql)
                .bind(tag_canonical)
                .bind(limit)
                .bind(offset)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows)
}

/// List works tagged with a specific fandom node, for the /fandoms/:fandom surface (spec §43.1).
pub async fn works_by_fandom(
    db: &Database,
    fandom_canonical: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<(String, String, String, String)>> {
    let sql = db.sql(
        "SELECT w.id, w.title, p.handle, w.updated_at
         FROM works w
         JOIN work_tags wt ON wt.work_id = w.id
         JOIN taxonomy_nodes tn ON tn.id = wt.node_id
         JOIN pseuds p ON p.id = w.owner_pseud_id
         WHERE tn.norm = ? AND w.lifecycle = 'published' AND w.visibility = 'public'
         ORDER BY w.updated_at DESC
         LIMIT ? OFFSET ?",
        "SELECT w.id::text, w.title, p.handle, w.updated_at
         FROM works w
         JOIN work_tags wt ON wt.work_id = w.id
         JOIN taxonomy_nodes tn ON tn.id = wt.node_id
         JOIN pseuds p ON p.id = w.owner_pseud_id::text
         WHERE tn.norm = $1 AND w.lifecycle = 'published' AND w.visibility = 'public'
         ORDER BY w.updated_at DESC
         LIMIT $2 OFFSET $3",
    );
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String, String, String, String)>(&sql)
                .bind(fandom_canonical)
                .bind(limit)
                .bind(offset)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String, String, String, String)>(&sql)
                .bind(fandom_canonical)
                .bind(limit)
                .bind(offset)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows)
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
