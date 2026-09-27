//! Instance theme preferences.
//!
//! Each instance has a theme vector derived from user bookmarks/tags.
//! By default, these are PRIVATE (only used locally for federation matching).
//! Instances may opt-in to publish their theme for public discovery.

use crate::{Backend, Database};
use anyhow::Result;
use serde_json::Value as JsonValue;
use std::collections::HashMap;

/// An instance's theme preferences (private by default).
#[derive(Debug, Clone, serde::Serialize)]
pub struct InstanceTheme {
    pub instance_id: String,
    pub theme_vector: JsonValue, // {tag: weight, ...}
    pub public: bool,
    pub computed_at: String,
    pub updated_at: String,
}

/// Compute theme vector from user bookmarks and tags.
/// Reads works a user has bookmarked/liked and aggregates their tags.
pub async fn compute_theme_from_bookmarks(db: &Database, _instance_id: &str) -> Result<JsonValue> {
    // The tag taxonomy is `taxonomy_nodes` joined through `work_tags.node_id` --
    // there is no `tags` table and no `work_tags.tag_id`, so the previous
    // statement referenced two things that do not exist and failed on both
    // engines. `.unwrap_or_default()` then turned that failure into an empty
    // vector, which is why this function has always returned `{}`.
    //
    // `is_public` is BOOLEAN on PostgreSQL and INTEGER on SQLite, hence one
    // predicate per dialect. Only *private* bookmarks feed the instance's own
    // theme; a public bookmark is a reader's signal, not the instance's.
    let rows: Vec<(String,)> = match db.backend() {
        Backend::Sqlite => sqlx::query_as(
            "SELECT tn.canonical
             FROM bookmarks b
             JOIN work_tags wt ON wt.work_id = b.subject_id
             JOIN taxonomy_nodes tn ON tn.id = wt.node_id
             WHERE b.is_public = FALSE
             GROUP BY tn.canonical
             ORDER BY COUNT(*) DESC
             LIMIT 100",
        )
        .fetch_all(db.sqlite_pool().expect("sqlite"))
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "theme: bookmark tag query failed; theme will be empty");
            Vec::new()
        }),
        Backend::Postgres => sqlx::query_as(
            "SELECT tn.canonical
             FROM bookmarks b
             JOIN work_tags wt ON wt.work_id = b.subject_id
             JOIN taxonomy_nodes tn ON tn.id = wt.node_id
             WHERE b.is_public = FALSE
             GROUP BY tn.canonical
             ORDER BY COUNT(*) DESC
             LIMIT 100",
        )
        .fetch_all(db.postgres_pool().expect("postgres"))
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "theme: bookmark tag query failed; theme will be empty");
            Vec::new()
        }),
    };

    let mut map = serde_json::Map::new();
    let total = rows.len().max(1) as f64;
    for (i, (name,)) in rows.into_iter().enumerate() {
        // Exponential decay weight: higher-ranked tags get more weight
        let weight = (total - i as f64) / total;
        map.insert(name, serde_json::json!(weight));
    }
    Ok(JsonValue::Object(map))
}

/// Upsert the instance theme vector.
pub async fn upsert_theme(
    db: &Database,
    instance_id: &str,
    theme_vector: &JsonValue,
    public: bool,
    now: &str,
) -> Result<()> {
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO instance_themes (instance_id, theme_vector, public, computed_at, updated_at)
                 VALUES (?, ?, ?, ?, ?)
                 ON CONFLICT(instance_id) DO UPDATE SET
                   theme_vector = excluded.theme_vector,
                   public = excluded.public,
                   computed_at = excluded.computed_at,
                   updated_at = excluded.updated_at",
            )
            .bind(instance_id)
            .bind(&serde_json::to_string(theme_vector)?)
            .bind(public)
            .bind(now)
            .bind(now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO instance_themes (instance_id, theme_vector, public, computed_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (instance_id) DO UPDATE SET
                   theme_vector = EXCLUDED.theme_vector,
                   public = EXCLUDED.public,
                   computed_at = EXCLUDED.computed_at,
                   updated_at = EXCLUDED.updated_at",
            )
            .bind(instance_id)
            .bind(&serde_json::to_string(theme_vector)?)
            .bind(public)
            .bind(now)
            .bind(now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(())
}

/// Read the theme vector for a local instance (owner only).
pub async fn get_local_theme(db: &Database, instance_id: &str) -> Result<Option<InstanceTheme>> {
    let row: Option<(String, String, bool, String, String)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as("SELECT instance_id, theme_vector, public, computed_at, updated_at FROM instance_themes WHERE instance_id = ?")
                .bind(instance_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as("SELECT instance_id, theme_vector, public, computed_at, updated_at FROM instance_themes WHERE instance_id = $1")
                .bind(instance_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.map(
        |(instance_id, tv, public, computed_at, updated_at)| InstanceTheme {
            instance_id,
            theme_vector: serde_json::from_str(&tv).unwrap_or_default(),
            public,
            computed_at,
            updated_at,
        },
    ))
}

/// List all public themes (for instance discovery).
pub async fn list_public_themes(db: &Database) -> Result<Vec<InstanceTheme>> {
    let rows: Vec<(String, String, bool, String, String)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as("SELECT instance_id, theme_vector, public, computed_at, updated_at FROM instance_themes WHERE public = TRUE ORDER BY computed_at DESC")
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as("SELECT instance_id, theme_vector, public, computed_at, updated_at FROM instance_themes WHERE public = TRUE ORDER BY computed_at DESC")
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(
            |(instance_id, tv, public, computed_at, updated_at)| InstanceTheme {
                instance_id,
                theme_vector: serde_json::from_str(&tv).unwrap_or_default(),
                public,
                computed_at,
                updated_at,
            },
        )
        .collect())
}

/// Compute tag weights from a theme vector.
pub fn tag_weights(theme: &JsonValue) -> HashMap<String, f64> {
    let mut map = HashMap::new();
    if let Some(obj) = theme.as_object() {
        for (k, v) in obj {
            if let Some(w) = v.as_f64() {
                map.insert(k.clone(), w);
            }
        }
    }
    map
}
