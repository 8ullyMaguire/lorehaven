//! Discovery repository: taste profiles, recommendations, recipes, dashboards.
//!
//! Spec §16.1–16.8. Both dialects.

use crate::Database;
use anyhow::Result;
use lorehaven_domain::ids::WorkId;
use serde::Serialize;

/// A taste profile row.
#[derive(Debug, Clone, Serialize)]
pub struct TasteProfile {
    pub account: String,
    pub signals: serde_json::Value,
    pub computed_at: String,
}

/// A recommendation candidate.
#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub work_id: WorkId,
    pub score: i64,
    pub reason: String,
}

/// Read a taste profile (owner only).
pub async fn taste_profile_for(db: &Database, account: &str) -> Result<Option<TasteProfile>> {
    let row: Option<(String, String, String)> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(
                "SELECT account, signals, computed_at FROM taste_profiles WHERE account = ?",
            )
            .bind(account)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(
                "SELECT account, signals, computed_at FROM taste_profiles WHERE account = $1",
            )
            .bind(account)
            .fetch_optional(db.postgres_pool().expect("postgres"))
            .await?
        }
    };
    Ok(row.map(|(account, signals, computed_at)| TasteProfile {
        account,
        signals: serde_json::from_str(&signals).unwrap_or_default(),
        computed_at,
    }))
}

/// Save a taste profile.
pub async fn save_taste_profile(
    db: &Database,
    account: &str,
    signals: &serde_json::Value,
    computed_at: &str,
) -> Result<()> {
    let signals_json = serde_json::to_string(signals)?;
    match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query(
                "INSERT OR REPLACE INTO taste_profiles (account, signals, computed_at) VALUES (?, ?, ?)",
            )
            .bind(account)
            .bind(&signals_json)
            .bind(computed_at)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        crate::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO taste_profiles (account, signals, computed_at) VALUES ($1, $2, $3) ON CONFLICT (account) DO UPDATE SET signals = EXCLUDED.signals, computed_at = EXCLUDED.computed_at",
            )
            .bind(account)
            .bind(&signals_json)
            .bind(computed_at)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(())
}

/// Clear a taste profile.
pub async fn clear_taste_profile(db: &Database, account: &str) -> Result<()> {
    match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query("DELETE FROM taste_profiles WHERE account = ?")
                .bind(account)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        crate::Backend::Postgres => {
            sqlx::query("DELETE FROM taste_profiles WHERE account = $1")
                .bind(account)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Recompute the taste profile for an account from reading history.
pub async fn recompute_taste_profile(db: &Database, account: &str) -> Result<()> {
    // Aggregate from reading history, ratings, notes, bookmarks
    let signals = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as::<_, (String,)>(
                "SELECT COALESCE(json_group_array(DISTINCT wt.node_id), '[]')
                 FROM reading_history_entry rh
                 JOIN work_tags wt ON wt.work_id = rh.subject_id
                 WHERE rh.account_id = ?
                 AND rh.subject_type = 'work'
                 LIMIT 100",
            )
            .bind(account)
            .fetch_one(db.sqlite_pool().expect("sqlite"))
            .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as::<_, (String,)>(
                "SELECT COALESCE(json_agg(DISTINCT wt.node_id)::text, '[]')
                 FROM reading_history_entry rh
                 JOIN work_tags wt ON wt.work_id = rh.subject_id
                 WHERE rh.account_id = $1
                 AND rh.subject_type = 'work'
                 LIMIT 100",
            )
            .bind(account)
            .fetch_one(db.postgres_pool().expect("postgres"))
            .await?
        }
    };
    let signals_json: serde_json::Value = serde_json::from_str(&signals.0).unwrap_or_default();
    let now = crate::identity::now_rfc3339();
    save_taste_profile(db, account, &signals_json, &now).await
}

/// Get public recommendations (popular recent works).
pub async fn public_recommendations(db: &Database, limit: i64) -> Result<Vec<WorkId>> {
    let rows: Vec<(String,)> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(
                "SELECT w.id FROM works w
                 WHERE w.lifecycle = 'published' AND w.visibility = 'public'
                 ORDER BY w.updated_at DESC
                 LIMIT ?",
            )
            .bind(limit)
            .fetch_all(db.sqlite_pool().expect("sqlite"))
            .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(
                "SELECT w.id::text FROM works w
                 WHERE w.lifecycle = 'published' AND w.visibility = 'public'
                 ORDER BY w.updated_at DESC
                 LIMIT $1",
            )
            .bind(limit)
            .fetch_all(db.postgres_pool().expect("postgres"))
            .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(|(id,)| id.parse().unwrap_or_default())
        .collect())
}

/// Get personalized recommendations based on taste profile.
pub async fn personalized_recommendations(
    db: &Database,
    account: &str,
    limit: i64,
) -> Result<Vec<WorkId>> {
    // First try to get the taste profile
    let profile = taste_profile_for(db, account).await?;
    match profile {
        Some(_) => {
            // Use taste profile to find similar works
            let rows: Vec<(String,)> = match db.backend() {
                crate::Backend::Sqlite => {
                    sqlx::query_as(
                        "SELECT DISTINCT w.id FROM works w
                         JOIN work_tags wt ON wt.work_id = w.id
                         WHERE w.lifecycle = 'published' AND w.visibility = 'public'
                         AND wt.node_id IN (
                             SELECT json_each.value FROM taste_profiles tp,
                             json_each(tp.signals)
                             WHERE tp.account = ?
                         )
                         AND w.owner_pseud_id NOT IN (
                             SELECT id FROM pseuds WHERE account_id = ?
                         )
                         ORDER BY w.updated_at DESC
                         LIMIT ?",
                    )
                    .bind(account)
                    .bind(account)
                    .bind(limit)
                    .fetch_all(db.sqlite_pool().expect("sqlite"))
                    .await?
                }
                crate::Backend::Postgres => {
                    sqlx::query_as(
                        "SELECT DISTINCT w.id::text FROM works w
                         JOIN work_tags wt ON wt.work_id = w.id
                         WHERE w.lifecycle = 'published' AND w.visibility = 'public'
                         AND wt.node_id IN (
                             SELECT json_array_elements_text(tp.signals::json)
                             FROM taste_profiles tp
                             WHERE tp.account = $1
                         )
                         AND w.owner_pseud_id NOT IN (
                             SELECT id FROM pseuds WHERE account_id = $2
                         )
                         ORDER BY w.updated_at DESC
                         LIMIT $3",
                    )
                    .bind(account)
                    .bind(account)
                    .bind(limit)
                    .fetch_all(db.postgres_pool().expect("postgres"))
                    .await?
                }
            };
            Ok(rows
                .into_iter()
                .map(|(id,)| id.parse().unwrap_or_default())
                .collect())
        }
        None => public_recommendations(db, limit).await,
    }
}

/// An operator-set work affinity (private, never rendered publicly).
#[derive(Debug, Clone, Serialize)]
pub struct OperatorAffinity {
    pub work_id: String,
    pub affinity_bp: i64,
    pub operator: String,
    pub rationale: String,
    pub set_at: String,
}

/// Set or replace the operator affinity for a work. Audit-logged.
///
/// affinity_bp is clamped to -5000..=10000 (base-point range).
pub async fn set_operator_affinity(
    db: &Database,
    work_id: &str,
    affinity_bp: i64,
    operator: &str,
    rationale: &str,
) -> Result<()> {
    let clamped = affinity_bp.clamp(-5000, 10000);
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query(
                "INSERT OR REPLACE INTO operator_affinities (work_id, affinity_bp, operator, rationale, set_at)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(work_id)
            .bind(clamped)
            .bind(operator)
            .bind(rationale)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        crate::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO operator_affinities (work_id, affinity_bp, operator, rationale, set_at)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (work_id) DO UPDATE SET
                    affinity_bp = EXCLUDED.affinity_bp,
                    operator = EXCLUDED.operator,
                    rationale = EXCLUDED.rationale,
                    set_at = EXCLUDED.set_at",
            )
            .bind(work_id)
            .bind(clamped)
            .bind(operator)
            .bind(rationale)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }

    // Audit trail: operator action recorded server-side.
    let audit_doc = serde_json::json!({
        "work_id": work_id,
        "affinity_bp": clamped,
        "rationale": rationale,
    });
    crate::governance::audit_append(
        db,
        operator,
        "operator.set_affinity",
        "work",
        work_id,
        &audit_doc.to_string(),
    )
    .await?;
    Ok(())
}

/// List all operator affinities. Used internally to apply ranking multipliers.
pub async fn list_operator_affinities(db: &Database) -> Result<Vec<OperatorAffinity>> {
    let rows: Vec<(String, i64, String, String, String)> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(
                "SELECT work_id, affinity_bp, operator, rationale, set_at
                 FROM operator_affinities ORDER BY set_at DESC",
            )
            .fetch_all(db.sqlite_pool().expect("sqlite"))
            .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(
                "SELECT work_id, affinity_bp::bigint, operator, rationale, set_at
                 FROM operator_affinities ORDER BY set_at DESC",
            )
            .fetch_all(db.postgres_pool().expect("postgres"))
            .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(
            |(work_id, affinity_bp, operator, rationale, set_at)| OperatorAffinity {
                work_id,
                affinity_bp,
                operator,
                rationale,
                set_at,
            },
        )
        .collect())
}

/// Save a recipe (create or update).
pub async fn save_recipe(
    db: &Database,
    id: &str,
    owner: &str,
    name: &str,
    document: &serde_json::Value,
    is_public: bool,
    created_at: &str,
) -> Result<()> {
    let now = created_at.to_string();
    match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query(
                "INSERT OR REPLACE INTO recipes (id, owner, name, document, is_public, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(owner)
            .bind(name)
            .bind(document.to_string())
            .bind(is_public as i64)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        crate::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO recipes (id, owner, name, document, is_public, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (id) DO UPDATE SET
                    owner = EXCLUDED.owner,
                    name = EXCLUDED.name,
                    document = EXCLUDED.document,
                    is_public = EXCLUDED.is_public,
                    created_at = EXCLUDED.created_at",
            )
            .bind(id)
            .bind(owner)
            .bind(name)
            .bind(document.to_string())
            .bind(is_public as i64)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(())
}

/// Read a recipe by ID, enforcing visibility: a non-public recipe
/// is only returned to its owner.
pub async fn get_recipe(db: &Database, id: &str, viewer: &str) -> Result<Option<RecipeRow>> {
    let row: Option<(String, String, String, String, i64, String)> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(
                "SELECT id, owner, name, document, is_public, created_at FROM recipes WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?
        }
        crate::Backend::Postgres => sqlx::query_as(
            "SELECT id, owner, name, document, is_public, created_at FROM recipes WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(db.postgres_pool().expect("postgres"))
        .await?,
    };
    match row {
        Some((id, owner, name, document, is_public, created_at)) => {
            if is_public == 0 && owner != viewer {
                return Ok(None);
            }
            Ok(Some(RecipeRow {
                id,
                owner,
                name,
                document: serde_json::from_str(&document).unwrap_or_default(),
                is_public: is_public != 0,
                created_at,
            }))
        }
        None => Ok(None),
    }
}

/// List recipes visible to the viewer (all public ones + the viewer's own private ones).
pub async fn list_recipes(db: &Database, viewer: &str) -> Result<Vec<RecipeRow>> {
    let rows: Vec<(String, String, String, String, i64, String)> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(
                "SELECT id, owner, name, document, is_public, created_at FROM recipes WHERE is_public = 1 OR owner = ? ORDER BY created_at DESC",
            )
            .bind(viewer)
            .fetch_all(db.sqlite_pool().expect("sqlite"))
            .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(
                "SELECT id, owner, name, document, is_public, created_at FROM recipes WHERE is_public = 1 OR owner = $1 ORDER BY created_at DESC",
            )
            .bind(viewer)
            .fetch_all(db.postgres_pool().expect("postgres"))
            .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(
            |(id, owner, name, document, is_public, created_at)| RecipeRow {
                id,
                owner,
                name,
                document: serde_json::from_str(&document).unwrap_or_default(),
                is_public: is_public != 0,
                created_at,
            },
        )
        .collect())
}

/// Update a recipe's name or document (owner-only).
pub async fn update_recipe(
    db: &Database,
    id: &str,
    owner: &str,
    name: &str,
    document: &serde_json::Value,
) -> Result<bool> {
    let updated = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query("UPDATE recipes SET name = ?, document = ? WHERE id = ? AND owner = ?")
                .bind(name)
                .bind(document.to_string())
                .bind(id)
                .bind(owner)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?
                .rows_affected()
        }
        crate::Backend::Postgres => {
            sqlx::query("UPDATE recipes SET name = $1, document = $2 WHERE id = $3 AND owner = $4")
                .bind(name)
                .bind(document.to_string())
                .bind(id)
                .bind(owner)
                .execute(db.postgres_pool().expect("postgres"))
                .await?
                .rows_affected()
        }
    };
    Ok(updated > 0)
}

/// Delete a recipe (owner-only).
pub async fn delete_recipe(db: &Database, id: &str, owner: &str) -> Result<bool> {
    let deleted = match db.backend() {
        crate::Backend::Sqlite => sqlx::query("DELETE FROM recipes WHERE id = ? AND owner = ?")
            .bind(id)
            .bind(owner)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        crate::Backend::Postgres => sqlx::query("DELETE FROM recipes WHERE id = $1 AND owner = $2")
            .bind(id)
            .bind(owner)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    Ok(deleted > 0)
}

/// A recipe row read from the database.
#[derive(Debug, Clone, Serialize)]
pub struct RecipeRow {
    pub id: String,
    pub owner: String,
    pub name: String,
    pub document: serde_json::Value,
    pub is_public: bool,
    pub created_at: String,
}

/// A dashboard layout row read from the database.
#[derive(Debug, Clone, Serialize)]
pub struct DashboardLayout {
    pub account: String,
    pub slots: serde_json::Value,
    pub updated_at: String,
}

/// Read a dashboard layout for an account.
pub async fn get_dashboard_layout(db: &Database, account: &str) -> Result<Option<DashboardLayout>> {
    let row: Option<(String, String, String)> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(
                "SELECT account, slots, updated_at FROM dashboard_layouts WHERE account = ?",
            )
            .bind(account)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(
                "SELECT account, slots, updated_at FROM dashboard_layouts WHERE account = $1",
            )
            .bind(account)
            .fetch_optional(db.postgres_pool().expect("postgres"))
            .await?
        }
    };
    Ok(row.map(|(account, slots, updated_at)| DashboardLayout {
        account,
        slots: serde_json::from_str(&slots).unwrap_or_default(),
        updated_at,
    }))
}

/// Save a dashboard layout for an account.
pub async fn save_dashboard_layout(
    db: &Database,
    account: &str,
    slots: &serde_json::Value,
) -> Result<()> {
    let now = crate::identity::now_rfc3339();
    let slots_str = slots.to_string();
    match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query(
                "INSERT OR REPLACE INTO dashboard_layouts (account, slots, updated_at)
                 VALUES (?, ?, ?)",
            )
            .bind(account)
            .bind(&slots_str)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        crate::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO dashboard_layouts (account, slots, updated_at)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (account) DO UPDATE SET
                    slots = EXCLUDED.slots,
                    updated_at = EXCLUDED.updated_at",
            )
            .bind(account)
            .bind(&slots_str)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(())
}
