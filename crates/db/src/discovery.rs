//! Discovery repository: taste profiles, recipes, dashboards.
//!
//! Spec §16.1–16.8. Both dialects.

use crate::Database;
use anyhow::Result;
use serde::Serialize;

/// A taste profile row.
#[derive(Debug, Clone, Serialize)]
pub struct TasteProfile {
    pub account: String,
    pub signals: serde_json::Value,
    pub computed_at: String,
}

/// A recipe row.
#[derive(Debug, Clone, Serialize)]
pub struct Recipe {
    pub id: String,
    pub owner: String,
    pub name: String,
    pub document: serde_json::Value,
    pub is_public: bool,
    pub created_at: String,
}

/// A dashboard layout.
#[derive(Debug, Clone, Serialize)]
pub struct DashboardLayout {
    pub account: String,
    pub slots: serde_json::Value,
    pub updated_at: String,
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
    let rows_affected = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query(
                "INSERT OR REPLACE INTO taste_profiles (account, signals, computed_at) VALUES (?, ?, ?)",
            )
            .bind(account)
            .bind(&signals_json)
            .bind(computed_at)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected()
        }
        crate::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO taste_profiles (account, signals, computed_at) VALUES ($1, $2, $3) ON CONFLICT (account) DO UPDATE SET signals = EXCLUDED.signals, computed_at = EXCLUDED.computed_at",
            )
            .bind(account)
            .bind(&signals_json)
            .bind(computed_at)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected()
        }
    };
    let _ = rows_affected;
    Ok(())
}

/// Clear a taste profile.
pub async fn clear_taste_profile(db: &Database, account: &str) -> Result<()> {
    let rows_affected = match db.backend() {
        crate::Backend::Sqlite => sqlx::query("DELETE FROM taste_profiles WHERE account = ?")
            .bind(account)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        crate::Backend::Postgres => sqlx::query("DELETE FROM taste_profiles WHERE account = $1")
            .bind(account)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    let _ = rows_affected;
    Ok(())
}

/// Read a recipe by id.
pub async fn recipe_for(db: &Database, id: &str) -> Result<Option<Recipe>> {
    let row: Option<(String, String, String, String, i64, String)> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(
                "SELECT id, owner, name, document, is_public, created_at FROM recipes WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(
                "SELECT id::text, owner, name, document, is_public, created_at FROM recipes WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(db.postgres_pool().expect("postgres"))
            .await?
        }
    };
    Ok(row.map(
        |(id, owner, name, document, is_public, created_at)| Recipe {
            id,
            owner,
            name,
            document: serde_json::from_str(&document).unwrap_or_default(),
            is_public: is_public != 0,
            created_at,
        },
    ))
}

/// Save a recipe.
pub async fn save_recipe(
    db: &Database,
    id: &str,
    owner: &str,
    name: &str,
    document: &serde_json::Value,
    is_public: bool,
    created_at: &str,
) -> Result<()> {
    let doc_json = serde_json::to_string(document)?;
    let is_public_int = if is_public { 1i64 } else { 0i64 };
    let rows_affected = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO recipes (id, owner, name, document, is_public, created_at) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(owner)
            .bind(name)
            .bind(&doc_json)
            .bind(is_public_int)
            .bind(created_at)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected()
        }
        crate::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO recipes (id, owner, name, document, is_public, created_at) VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(id)
            .bind(owner)
            .bind(name)
            .bind(&doc_json)
            .bind(is_public_int)
            .bind(created_at)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected()
        }
    };
    let _ = rows_affected;
    Ok(())
}

/// Read public recipes.
pub async fn public_recipes(
    db: &Database,
    cursor: Option<&str>,
    limit: i64,
) -> Result<Vec<Recipe>> {
    let rows: Vec<(String, String, String, String, i64, String)> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(
                "SELECT id, owner, name, document, is_public, created_at FROM recipes WHERE is_public = 1 AND (?1 IS NULL OR id > ?1) ORDER BY id ASC LIMIT ?2",
            )
            .bind(cursor)
            .bind(limit)
            .fetch_all(db.sqlite_pool().expect("sqlite"))
            .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(
                "SELECT id::text, owner, name, document, is_public, created_at FROM recipes WHERE is_public = 1 AND ($1 IS NULL OR id > $1) ORDER BY id ASC LIMIT $2",
            )
            .bind(cursor)
            .bind(limit)
            .fetch_all(db.postgres_pool().expect("postgres"))
            .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(
            |(id, owner, name, document, is_public, created_at)| Recipe {
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

/// Read a dashboard layout.
pub async fn dashboard_layout_for(db: &Database, account: &str) -> Result<Option<DashboardLayout>> {
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

/// Save a dashboard layout.
pub async fn save_dashboard_layout(
    db: &Database,
    account: &str,
    slots: &serde_json::Value,
    updated_at: &str,
) -> Result<()> {
    let slots_json = serde_json::to_string(slots)?;
    let rows_affected = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query(
                "INSERT OR REPLACE INTO dashboard_layouts (account, slots, updated_at) VALUES (?, ?, ?)",
            )
            .bind(account)
            .bind(&slots_json)
            .bind(updated_at)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected()
        }
        crate::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO dashboard_layouts (account, slots, updated_at) VALUES ($1, $2, $3) ON CONFLICT (account) DO UPDATE SET slots = EXCLUDED.slots, updated_at = EXCLUDED.updated_at",
            )
            .bind(account)
            .bind(&slots_json)
            .bind(updated_at)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected()
        }
    };
    let _ = rows_affected;
    Ok(())
}
