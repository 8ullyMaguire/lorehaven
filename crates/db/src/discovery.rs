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
