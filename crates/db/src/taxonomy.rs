//! Taxonomy repository: nodes, aliases, tags.
//!
//! Spec §15.1–15.3. Both dialects.

use crate::{rewrite_placeholders, Backend, Database};
use anyhow::Result;
use serde::Serialize;

/// A taxonomy node.
#[derive(Debug, Clone, Serialize)]
pub struct CanonicalEntity {
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
pub struct EntityAlias {
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
    darkhold_domain::exchange::normalise(text)
}

/// Search nodes by kind and prefix.
pub async fn search_nodes(
    db: &Database,
    kind: Option<&str>,
    prefix: &str,
    limit: i64,
) -> Result<Vec<CanonicalEntity>> {
    let sql = db.sql(
        "SELECT id, kind, canonical, norm, created_at, review_status, CAST(signal_count AS BIGINT) FROM canonical_entities
         WHERE (?1 IS NULL OR kind = ?1) AND norm LIKE ?2 || '%'
         ORDER BY norm ASC LIMIT ?3",
        "SELECT id::text, kind, canonical, norm, created_at, review_status, CAST(signal_count AS BIGINT) FROM canonical_entities
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
            |(id, kind, canonical, norm, created_at, review_status, signal_count)| {
                CanonicalEntity {
                    id,
                    kind,
                    canonical,
                    norm,
                    created_at,
                    review_status,
                    signal_count,
                }
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
) -> Result<Vec<CanonicalEntity>> {
    use darkhold_domain::taxonomy::{canonical_form, fuzzy_match};
    use std::collections::HashMap;

    let prefix = query.trim();
    let normalized = canonical_form(prefix);

    // 1. Exact prefix matches (normalized so LIKE is case-consistent).
    let sql_prefix = db.sql(
        "SELECT id, kind, canonical, norm, created_at, review_status, CAST(signal_count AS BIGINT) FROM canonical_entities
         WHERE (?1 IS NULL OR kind = ?1) AND norm LIKE ?2 || '%'
         ORDER BY norm ASC",
        "SELECT id::text, kind, canonical, norm, created_at, review_status, CAST(signal_count AS BIGINT) FROM canonical_entities
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
    let prefix_nodes: Vec<CanonicalEntity> = prefix_rows
        .iter()
        .map(
            |(id, kind, canonical, norm, created_at, review_status, signal_count)| {
                CanonicalEntity {
                    id: id.clone(),
                    kind: kind.clone(),
                    canonical: canonical.clone(),
                    norm: norm.clone(),
                    created_at: created_at.clone(),
                    review_status: review_status.clone(),
                    signal_count: *signal_count,
                }
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
        "SELECT id, kind, canonical, norm, created_at, review_status, CAST(signal_count AS BIGINT) FROM canonical_entities
         WHERE (?1 IS NULL OR kind = ?1) AND norm NOT LIKE ?2 || '%'
         ORDER BY norm ASC",
        "SELECT id::text, kind, canonical, norm, created_at, review_status, CAST(signal_count AS BIGINT) FROM canonical_entities
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

    // Map fuzzy match ids back to full CanonicalEntity records.
    let lookup: HashMap<String, CanonicalEntity> = rest_rows
        .into_iter()
        .map(
            |(id, kind, canonical, norm, created_at, review_status, signal_count)| {
                (
                    id.clone(),
                    CanonicalEntity {
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
pub async fn node_by_id(db: &Database, id: &str) -> Result<Option<CanonicalEntity>> {
    let sql = db.sql(
        "SELECT id, kind, canonical, norm, created_at, review_status, CAST(signal_count AS BIGINT) FROM canonical_entities WHERE id = ?",
        "SELECT id, kind, canonical, norm, created_at, review_status, CAST(signal_count AS BIGINT) FROM canonical_entities WHERE id = $1",
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
        |(id, kind, canonical, norm, created_at, review_status, signal_count)| CanonicalEntity {
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
pub async fn create_node(db: &Database, kind: &str, canonical: &str) -> Result<CanonicalEntity> {
    let id = uuid::Uuid::new_v4().to_string();
    let norm = canonical_form(canonical);
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO canonical_entities (id, kind, canonical, norm, created_at) VALUES (?, ?, ?, ?, ?)",
        "INSERT INTO canonical_entities (id, kind, canonical, norm, created_at) VALUES ($1::uuid, $2, $3, $4, $5)",
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
    Ok(CanonicalEntity {
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
) -> Result<CanonicalEntity> {
    let norm = canonical_form(canonical);
    let now = crate::identity::now_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();
    // Note what is *not* in the update arm: `canonical` and `review_status`.
    // Reinforcing a node must not rewrite a curator's chosen display form, and
    // agreement is not curation.
    let sql = db.sql(
        "INSERT INTO canonical_entities (id, kind, canonical, norm, created_at, review_status, signal_count)
         VALUES (?, ?, ?, ?, ?, 'unverified', 1)
         ON CONFLICT (kind, norm) DO UPDATE SET signal_count = canonical_entities.signal_count + 1",
        "INSERT INTO canonical_entities (id, kind, canonical, norm, created_at, review_status, signal_count)
         VALUES ($1::uuid, $2, $3, $4, $5, 'unverified', 1)
         ON CONFLICT (kind, norm) DO UPDATE SET signal_count = canonical_entities.signal_count + 1",
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
         FROM canonical_entities WHERE kind = ? AND norm = ?",
        "SELECT id, kind, canonical, norm, created_at, review_status, CAST(signal_count AS BIGINT)
         FROM canonical_entities WHERE kind = $1 AND norm = $2",
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
    Ok(CanonicalEntity {
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
        "UPDATE canonical_entities SET canonical = ?, review_status = 'curated'
         WHERE kind = ? AND norm = ?",
        "UPDATE canonical_entities SET canonical = $1, review_status = 'curated'
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

/// Curate a name in **both** vocabularies, atomically.
///
/// §15.17 keeps two tables describing one state: `canonical_entities` (what a tag
/// browser reads, what `search_nodes` searches, what `work_tags` references) and
/// `canonical_entities` (the exchange's own view, carrying the curator's account and
/// timestamps). Migration 0082 states the rule this function exists to enforce:
///
/// > Two tables describing one state must not spell it two ways, or a query
/// > joining them silently drops every curated row.
///
/// Until now the two writes were two calls in a route handler
/// (`routes/exchange.rs::curate_entity`), which has three consequences:
///
/// 1. **It is a route-only invariant.** Any other caller — a job, an import, the
///    admin path — curates one table and silently misses the other. `curate_node`
///    and `curate_entity` are both `pub`, so that is not a compiler error.
/// 2. **It is not atomic.** The first write commits before the second begins, so a
///    failure between them leaves a name curated in one vocabulary and unverified in
///    the other — the exact state 0082 says a join drops.
/// 3. **The pair is the only place that knows they must be written together.** A
///    fourth writer would have no way to learn that.
///
/// So the pairing moves into the db layer, where it is one call, one transaction,
/// and reachable by anything that can reach the other two.
///
/// `Ok(false)` when neither table had the name: a curate of a name that does not
/// exist is not a silent success, and the route maps that to 404 rather than
/// reporting `curated: true` for work that never happened.
///
/// Neither write may demote: both `UPDATE`s are already narrow (they set
/// `review_status = 'curated'` and `canonical` on a `(kind, norm)` match), and
/// `ensure_node_from_signal`'s update arm deliberately excludes both columns, so a
/// later signal cannot undo this.
pub async fn curate_name(
    db: &Database,
    kind: &str,
    norm: &str,
    canonical_form: &str,
    curator: &str,
) -> Result<bool> {
    let now = crate::identity::now_rfc3339();
    // Two statements rather than one: the tables have no shared key (one is keyed
    // (kind, norm), the other by a uuid `id`), so there is no single statement that
    // writes both. `canonical_entities` also keeps its own `created_at`.
    let node_sql = db.sql(
        "UPDATE canonical_entities SET canonical = ?, review_status = 'curated'
         WHERE kind = ? AND norm = ?",
        "UPDATE canonical_entities SET canonical = $1, review_status = 'curated'
         WHERE kind = $2 AND norm = $3",
    );
    let entity_sql = db.sql(
        "UPDATE canonical_entities
         SET canonical = ?, review_status = 'curated', curated_by = ?, curated_at = ?, updated_at = ?
         WHERE kind = ? AND norm = ?",
        "UPDATE canonical_entities
         SET canonical = $1, review_status = 'curated', curated_by = $2, curated_at = $3, updated_at = $4
         WHERE kind = $5 AND norm = $6",
    );

    match db.backend() {
        Backend::Sqlite => {
            let mut tx = db.sqlite_pool().expect("sqlite").begin().await?;
            let node = sqlx::query(&node_sql)
                .bind(canonical_form)
                .bind(kind)
                .bind(norm)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            let entity = sqlx::query(&entity_sql)
                .bind(canonical_form)
                .bind(curator)
                .bind(&now)
                .bind(&now)
                .bind(kind)
                .bind(norm)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            if node == 0 && entity == 0 {
                tx.rollback().await?;
                return Ok(false);
            }
            tx.commit().await?;
            Ok(true)
        }
        Backend::Postgres => {
            let mut tx = db.postgres_pool().expect("postgres").begin().await?;
            let node = sqlx::query(&node_sql)
                .bind(canonical_form)
                .bind(kind)
                .bind(norm)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            let entity = sqlx::query(&entity_sql)
                .bind(canonical_form)
                .bind(curator)
                .bind(&now)
                .bind(&now)
                .bind(kind)
                .bind(norm)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            if node == 0 && entity == 0 {
                tx.rollback().await?;
                return Ok(false);
            }
            tx.commit().await?;
            Ok(true)
        }
    }
}

/// The nodes awaiting review, most-reinforced first (§15.17's queue).
///
/// `ORDER BY signal_count DESC` is the whole point of the function, so it is in
/// the ORDER BY rather than a Rust sort — the `canonical_entities_review` index is
/// built for it, and sorting a full table read in Rust is both slower and
/// ignores it.
pub async fn list_unverified_nodes(db: &Database, limit: i64) -> Result<Vec<CanonicalEntity>> {
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
         FROM canonical_entities WHERE review_status = 'unverified'
         ORDER BY signal_count DESC, norm ASC LIMIT ?",
        "SELECT id, kind, canonical, norm, created_at, review_status, CAST(signal_count AS BIGINT)
         FROM canonical_entities WHERE review_status = 'unverified'
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
        .map(|r| CanonicalEntity {
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
) -> Result<EntityAlias> {
    let norm = canonical_form(alias);
    let sql = db.sql(
        "INSERT INTO entity_aliases (alias, norm, node_id, source) VALUES (?, ?, ?, ?)",
        "INSERT INTO entity_aliases (alias, norm, node_id, source) VALUES ($1, $2, $3::uuid, $4)",
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
    Ok(EntityAlias {
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
            SELECT COUNT(*) FROM work_tags wt WHERE wt.node_id = canonical_entities.id
        ) AS work_count
         FROM canonical_entities
         WHERE kind = 'tag'
         ORDER BY canonical ASC
         LIMIT ? OFFSET ?",
        "SELECT id::text, canonical, kind, (
            SELECT COUNT(*) FROM work_tags wt WHERE wt.node_id = canonical_entities.id
        ) AS work_count
         FROM canonical_entities
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
             JOIN canonical_entities tn ON tn.id = wt.node_id
             WHERE wt.work_id = ? AND tn.kind = 'tag'",
        )
        .bind(work_id)
        .fetch_all(db.sqlite_pool().expect("sqlite"))
        .await
        .unwrap_or_default(),
        Backend::Postgres => sqlx::query_as(
            "SELECT tn.canonical
             FROM work_tags wt
             JOIN canonical_entities tn ON tn.id = wt.node_id
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
         JOIN canonical_entities tn ON tn.id = wt.node_id
         WHERE wt.work_id = ? AND tn.kind = 'tag'
         ORDER BY tn.canonical ASC",
        "SELECT tn.canonical, wt.weight::bigint
         FROM work_tags wt
         JOIN canonical_entities tn ON tn.id = wt.node_id
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
            SELECT COUNT(*) FROM work_tags wt WHERE wt.node_id = canonical_entities.id
        ) AS work_count
         FROM canonical_entities
         WHERE kind = 'fandom'
         ORDER BY canonical ASC
         LIMIT ? OFFSET ?",
        "SELECT id::text, canonical, (
            SELECT COUNT(*) FROM work_tags wt WHERE wt.node_id = canonical_entities.id
        ) AS work_count
         FROM canonical_entities
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

/// A warning declared on a work (spec §10.2).
///
/// **A declared attribute and never a detection.** The archive does not
/// detect generated content, and it does not detect warnings either: it
/// records what the author said about their own work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkWarning {
    /// The work the warning is on.
    pub work_id: String,
    /// The unit it is scoped to, when it is scoped to one.
    pub unit_id: Option<String>,
    /// The taxonomy node naming the warning.
    pub warning_node_id: String,
    /// The node's canonical name, so a reader sees "graphic violence" and not
    /// an opaque identifier.
    pub canonical: String,
    /// 1 = light, 2 = heavy.
    pub severity: i64,
    /// `on_page`, `referenced` or `implied`.
    pub depiction: String,
    /// Who said so, and in what capacity.
    pub declaration: String,
    /// The author's note for a reader who wants to avoid the detail.
    pub spoiler_safe_note: Option<String>,
}

/// What to declare on a work.
pub struct WorkWarningInput<'a> {
    /// The work.
    pub work_id: &'a str,
    /// The unit it is scoped to, when it is scoped to one.
    pub unit_id: Option<&'a str>,
    /// The taxonomy node naming the warning.
    pub warning_node_id: &'a str,
    /// 1 = light, 2 = heavy.
    pub severity: i64,
    /// `on_page`, `referenced` or `implied`.
    pub depiction: &'a str,
    /// Who said so, and in what capacity.
    pub declaration: &'a str,
    /// The author's note for a reader who wants to avoid the detail.
    pub spoiler_safe_note: Option<&'a str>,
}

/// Declare a warning on a work.
///
/// The warning is **taxonomy-typed**: the node named is an ordinary taxonomy
/// node of kind `warning`, so a warning is searchable with the rest of a
/// work's metadata rather than living in free text that no query can reach.
///
/// `declaration` is kept as the author said it. `none_apply` and
/// `creator_chose_not_to_say` are **different statements** and must not be
/// collapsed: the second is deliberate silence, and collapsing them would let
/// a creator's silence read as a clean bill of health.
pub async fn declare_work_warning(db: &Database, input: WorkWarningInput<'_>) -> Result<()> {
    let WorkWarningInput {
        work_id,
        unit_id,
        warning_node_id,
        severity,
        depiction,
        declaration,
        spoiler_safe_note,
    } = input;
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO work_warnings
             (work_id, unit_id, warning_node_id, severity, depiction, declaration,
              spoiler_safe_note, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        "INSERT INTO work_warnings
             (work_id, unit_id, warning_node_id, severity, depiction, declaration,
              spoiler_safe_note, created_at, updated_at)
         VALUES ($1::uuid, $2, $3, $4, $5, $6, $7, $8, $9)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(work_id)
                .bind(unit_id)
                .bind(warning_node_id)
                .bind(severity)
                .bind(depiction)
                .bind(declaration)
                .bind(spoiler_safe_note)
                .bind(&now)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(work_id)
                .bind(unit_id)
                .bind(warning_node_id)
                .bind(severity)
                .bind(depiction)
                .bind(declaration)
                .bind(spoiler_safe_note)
                .bind(&now)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// The warnings declared on a work, with the node's name.
///
/// **The warnings are attached to the work, not to the filter.** A reader who
/// filters nothing sees everything, *with the warnings attached* (spec §10.2) --
/// so the listing is what every reader sees, and the filter is what some
/// readers choose to do about it.
pub async fn list_work_warnings(db: &Database, work_id: &str) -> Result<Vec<WorkWarning>> {
    let sql = db.sql(
        "SELECT w.work_id, w.unit_id, w.warning_node_id, tn.canonical, w.severity,
                w.depiction, w.declaration, w.spoiler_safe_note
           FROM work_warnings w
           JOIN canonical_entities tn ON tn.id = w.warning_node_id
          WHERE w.work_id = ?
          ORDER BY w.severity DESC, tn.canonical ASC",
        "SELECT w.work_id::text, w.unit_id, w.warning_node_id, tn.canonical,
                CAST(w.severity AS BIGINT) AS severity,
                w.depiction, w.declaration, w.spoiler_safe_note
           FROM work_warnings w
           JOIN canonical_entities tn ON tn.id = w.warning_node_id
          WHERE w.work_id::text = $1
          ORDER BY w.severity DESC, tn.canonical ASC",
    );
    // The row is read through a named struct rather than a tuple of eight:
    // clippy's type-complexity rule is right that a tuple of eight elements
    // is not readable, and the alternative -- eight positional binds at the
    // call site -- is where a swapped pair would hide.
    #[derive(sqlx::FromRow)]
    struct WarningRow {
        work_id: String,
        unit_id: Option<String>,
        warning_node_id: String,
        canonical: String,
        severity: i64,
        depiction: String,
        declaration: String,
        spoiler_safe_note: Option<String>,
    }

    let rows: Vec<WarningRow> = match db.backend() {
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
    Ok(rows
        .into_iter()
        .map(|row| WorkWarning {
            work_id: row.work_id,
            unit_id: row.unit_id,
            warning_node_id: row.warning_node_id,
            canonical: row.canonical,
            severity: row.severity,
            depiction: row.depiction,
            declaration: row.declaration,
            spoiler_safe_note: row.spoiler_safe_note,
        })
        .collect())
}

/// A reader's content filter, as the pool query needs it.
///
/// **`(kind, norm)` and not a bare value.** The taxonomy's unique key is
/// `(kind, norm)` -- the same name is a tag under one kind and a fandom
/// under another -- so a filter that carried only its value could not say
/// which of them it meant, and would exclude the wrong works.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentFilter {
    /// The taxonomy kind: `tag`, `fandom` or `warning`.
    pub kind: String,
    /// The node's normalised name.
    pub norm: String,
}

/// The works a tag listing shows a reader, and how many there are in total.
///
/// The **total** is the count *after* the filter, not before it: a reader
/// who filters "graphic violence" sees a filtered list, and the count has to
/// agree with the list or the reader is told there are forty works and shown
/// twelve with no way to tell which twelve.
pub struct FilteredWorks {
    /// The page of works.
    pub works: Vec<(String, String, String, String)>,
    /// How many works match, before the page is cut.
    pub total: i64,
}

/// List works tagged with a specific tag node, for the /tags/:tag surface (spec §43.1).
///
/// **The reader's filters are applied INSIDE the query** (spec §12.7): *"Filters
/// apply inside the pool query, before scoring. Filtering a finished result
/// list returns a subset of a *ranking*, which is not the same as a ranking
/// over a smaller pool."*
///
/// That is not a performance argument, it is a correctness one. A filter
/// applied after the list is built removes works from a *ranking* -- and the
/// ranking was computed over a pool that included them, so the order the
/// reader sees is the order of a pool they did not ask for. Filtering first
/// changes what is ranked.
///
/// Every filter becomes its own `NOT EXISTS`, so the exclusion is expressed
/// against the work rather than against a list of ids, and the count the
/// query reports is the count of what survived.
pub async fn works_by_tag_filtered(
    db: &Database,
    tag_canonical: &str,
    limit: i64,
    offset: i64,
    filters: &[ContentFilter],
) -> Result<FilteredWorks> {
    // The two kinds of exclusion are different tables: a tag or a fandom is a
    // `work_tags` row, and a warning is a `work_warnings` row.
    //
    // **The clauses and the binds are built together, in one order.** A
    // statement's placeholders are bound by position, so a list of clauses
    // and a list of values that are not in the same order bind each clause to
    // the wrong value -- and the query still runs, and filters the wrong
    // works. Building them together is what makes that impossible.
    let mut sqlite_clauses: Vec<String> = Vec::new();
    let mut postgres_clauses: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();
    // The PostgreSQL numbering starts at 2: `$1` is the tag the listing is
    // for. It is written out here rather than rewritten later, because the
    // two dialects' clauses are different statements -- see the note below.
    let mut pg_index = 2usize;
    for filter in filters {
        match filter.kind.as_str() {
            "tag" | "fandom" => {
                sqlite_clauses.push(
                    "NOT EXISTS (SELECT 1 FROM work_tags fwt \
                       JOIN canonical_entities ftn ON ftn.id = fwt.node_id \
                      WHERE fwt.work_id = w.id AND ftn.kind = ? AND ftn.norm = ?)"
                        .to_owned(),
                );
                postgres_clauses.push(format!(
                    "NOT EXISTS (SELECT 1 FROM work_tags fwt \
                       JOIN canonical_entities ftn ON ftn.id = fwt.node_id \
                      WHERE fwt.work_id = w.id AND ftn.kind = ${pg_index} AND ftn.norm = ${})",
                    pg_index + 1
                ));
                pg_index += 2;
                binds.push(filter.kind.clone());
                binds.push(filter.norm.clone());
            }
            "warning" => {
                sqlite_clauses.push(
                    "NOT EXISTS (SELECT 1 FROM work_warnings fww \
                       JOIN canonical_entities ftn ON ftn.id = fww.warning_node_id \
                      WHERE fww.work_id = w.id AND ftn.kind = 'warning' AND ftn.norm = ?)"
                        .to_owned(),
                );
                postgres_clauses.push(format!(
                    "NOT EXISTS (SELECT 1 FROM work_warnings fww \
                       JOIN canonical_entities ftn ON ftn.id = fww.warning_node_id \
                      WHERE fww.work_id = w.id AND ftn.kind = 'warning' AND ftn.norm = ${pg_index})"
                ));
                pg_index += 1;
                binds.push(filter.norm.clone());
            }
            // An unknown filter type is a **no-op**, not an error and not a
            // blanket exclusion. The table's CHECK keeps the set closed, so
            // this arm is unreachable in practice; and if it were reached, the
            // honest reading of "a filter I do not understand" is "I do not
            // know how to apply it", not "hide everything".
            _ => {}
        }
    }

    let sqlite_clause_text = if sqlite_clauses.is_empty() {
        String::new()
    } else {
        format!(" AND {}", sqlite_clauses.join(" AND "))
    };
    let postgres_clause_text = if postgres_clauses.is_empty() {
        String::new()
    } else {
        format!(" AND {}", postgres_clauses.join(" AND "))
    };

    let sqlite = format!(
        "SELECT w.id, w.title, p.handle, w.updated_at, COUNT(*) OVER () AS total
         FROM works w
         JOIN work_tags wt ON wt.work_id = w.id
         JOIN canonical_entities tn ON tn.id = wt.node_id
         JOIN pseuds p ON p.id = w.owner_pseud_id
         WHERE tn.norm = ? AND w.lifecycle = 'published' AND w.visibility = 'public'{sqlite_clause_text}
         ORDER BY w.updated_at DESC
         LIMIT ? OFFSET ?"
    );
    let postgres = format!(
        "SELECT w.id::text, w.title, p.handle, w.updated_at, COUNT(*) OVER () AS total
         FROM works w
         JOIN work_tags wt ON wt.work_id = w.id
         JOIN canonical_entities tn ON tn.id = wt.node_id
         JOIN pseuds p ON p.id = w.owner_pseud_id
         WHERE tn.norm = $1 AND w.lifecycle = 'published' AND w.visibility = 'public'{postgres_clause_text}
         ORDER BY w.updated_at DESC
         LIMIT ${} OFFSET ${}",
        binds.len() + 2,
        binds.len() + 3
    );

    // The `?` in the SQLite arm and the `$n` in the PostgreSQL arm are
    // **different statements**: the filter clauses are built with `?`, and
    // the PostgreSQL arm's own numbering is written out. So the PostgreSQL
    // arm cannot go through `rewrite_placeholders` -- it would renumber the
    // `?`s and collide with the `$n`s already there, which is the fault
    // `scripts/check-pg-placeholder-mixing.py` exists to catch. The
    // PostgreSQL statement is therefore built with explicit numbers from the
    // start, and the SQLite one is rewritten as usual.
    let sql = match db.backend() {
        Backend::Sqlite => rewrite_placeholders(&sqlite),
        Backend::Postgres => postgres,
    };

    // The query is built **inside** each arm: the two statements differ, and
    // `sqlx`'s query type is tied to the executor it will run on.
    let rows: Vec<(String, String, String, String, i64)> = match db.backend() {
        Backend::Sqlite => {
            let mut query = sqlx::query_as::<_, (String, String, String, String, i64)>(&sql)
                .bind(tag_canonical);
            for bind in &binds {
                query = query.bind(bind);
            }
            query
                .bind(limit)
                .bind(offset)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            let mut query = sqlx::query_as::<_, (String, String, String, String, i64)>(&sql)
                .bind(tag_canonical);
            for bind in &binds {
                query = query.bind(bind);
            }
            query
                .bind(limit)
                .bind(offset)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };

    let total = rows.first().map(|row| row.4).unwrap_or(0);
    Ok(FilteredWorks {
        works: rows
            .into_iter()
            .map(|row| (row.0, row.1, row.2, row.3))
            .collect(),
        total,
    })
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
         JOIN canonical_entities tn ON tn.id = wt.node_id
         JOIN pseuds p ON p.id = w.owner_pseud_id
         WHERE tn.norm = ? AND w.lifecycle = 'published' AND w.visibility = 'public'
         ORDER BY w.updated_at DESC
         LIMIT ? OFFSET ?",
        "SELECT w.id::text, w.title, p.handle, w.updated_at
         FROM works w
         JOIN work_tags wt ON wt.work_id = w.id
         JOIN canonical_entities tn ON tn.id = wt.node_id
         JOIN pseuds p ON p.id = w.owner_pseud_id
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
         JOIN canonical_entities tn ON tn.id = wt.node_id
         JOIN pseuds p ON p.id = w.owner_pseud_id
         WHERE tn.norm = ? AND w.lifecycle = 'published' AND w.visibility = 'public'
         ORDER BY w.updated_at DESC
         LIMIT ? OFFSET ?",
        "SELECT w.id::text, w.title, p.handle, w.updated_at
         FROM works w
         JOIN work_tags wt ON wt.work_id = w.id
         JOIN canonical_entities tn ON tn.id = wt.node_id
         JOIN pseuds p ON p.id = w.owner_pseud_id
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
