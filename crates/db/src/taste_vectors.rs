//! M44 taste vectors — multi-dimensional user taste alignment (spec §16.17).
//! DB functions for computing, storing, and querying taste vectors.

use serde_json::{json, Value};
use sqlx::Row;

use crate::{Backend, Database};
use lorehaven_domain::taste_vector;

fn pool_err() -> sqlx::Error {
    sqlx::Error::PoolClosed
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

async fn fetch_taste_vector_sqlite(
    pool: &sqlx::SqlitePool,
    account_id: &str,
) -> Result<Option<(Vec<f64>, f64, String)>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT taste_vector, taste_centroid_distance, taste_vector_computed_at
         FROM accounts WHERE id = ?",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await?;
    match row {
        Some(r) => {
            let vec_str: String = r.get("taste_vector");
            let vec: Vec<f64> = serde_json::from_str(&vec_str).unwrap_or_default();
            let dist: f64 = r.get("taste_centroid_distance");
            let computed_at: String = r.get("taste_vector_computed_at");
            Ok(Some((vec, dist, computed_at)))
        }
        None => Ok(None),
    }
}

async fn fetch_taste_vector_postgres(
    pool: &sqlx::postgres::PgPool,
    account_id: &str,
) -> Result<Option<(Vec<f64>, f64, String)>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT taste_vector, taste_centroid_distance::float8,
                to_char(taste_vector_computed_at, 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') as computed_at
         FROM accounts WHERE id = $1",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await?;
    match row {
        Some(r) => {
            let vec_json: serde_json::Value = r.get("taste_vector");
            let vec: Vec<f64> = match vec_json {
                serde_json::Value::Array(arr) => arr.iter().filter_map(|v| v.as_f64()).collect(),
                _ => vec![],
            };
            let dist: f64 = r.get("taste_centroid_distance");
            let computed_at: String = r.get("computed_at");
            Ok(Some((vec, dist, computed_at)))
        }
        None => Ok(None),
    }
}

async fn store_taste_vector_sqlite(
    pool: &sqlx::SqlitePool,
    account_id: &str,
    vector: &[f64],
    distance: f64,
    computed_at: &str,
) -> Result<(), sqlx::Error> {
    let vec_str = serde_json::to_string(vector).unwrap_or_else(|_| "[]".to_string());
    sqlx::query(
        "UPDATE accounts SET taste_vector = ?, taste_centroid_distance = ?,
         taste_vector_computed_at = ? WHERE id = ?",
    )
    .bind(&vec_str)
    .bind(distance)
    .bind(computed_at)
    .bind(account_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn store_taste_vector_postgres(
    pool: &sqlx::postgres::PgPool,
    account_id: &str,
    vector: &[f64],
    distance: f64,
    computed_at: &str,
) -> Result<(), sqlx::Error> {
    let vec_json = serde_json::to_value(vector).unwrap_or(serde_json::json!([]));
    sqlx::query(
        "UPDATE accounts SET taste_vector = $1, taste_centroid_distance = $2,
         taste_vector_computed_at = $3::timestamptz WHERE id = $4",
    )
    .bind(&vec_json)
    .bind(distance)
    .bind(computed_at)
    .bind(account_id)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Store a pre-computed taste vector and centroid distance for an account
/// (used by the onboarding quiz path, spec §0.4.2).
pub async fn store_taste_vector_public(
    db: &Database,
    account_id: &str,
    vector: &[f64],
    distance: f64,
    computed_at: &str,
) -> Result<(), sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().ok_or(pool_err())?;
            store_taste_vector_sqlite(pool, account_id, vector, distance, computed_at).await?
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().ok_or(pool_err())?;
            store_taste_vector_postgres(pool, account_id, vector, distance, computed_at).await?
        }
    }
    Ok(())
}

/// Compute and store a user's taste vector from their rated works.
/// Returns (vector, centroid_distance).
pub async fn compute_and_store_taste_vector(
    db: &Database,
    account_id: &str,
) -> Result<(Vec<f64>, f64), sqlx::Error> {
    // Fetch admin centroid (from config or computed from admin ratings)
    let admin_centroid = get_admin_centroid(db).await.unwrap_or_else(|| {
        vec![0.5, 0.5, 0.5, 0.5, 0.5] // default neutral centroid
    });

    // Fetch user's rated works with their vectors and weights
    let (work_vectors, work_weights) = fetch_user_rated_work_vectors(db, account_id).await?;

    let user_vector = if work_vectors.is_empty() {
        vec![0.0; admin_centroid.len()]
    } else {
        taste_vector::compute_user_vector(&work_vectors, &work_weights)
    };

    let distance = if user_vector.len() == admin_centroid.len() {
        taste_vector::normalized_distance(&user_vector, &admin_centroid, 2.0)
    } else {
        1.0
    };

    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().ok_or(pool_err())?;
            store_taste_vector_sqlite(pool, &account_id, &user_vector, distance, &now).await?
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().ok_or(pool_err())?;
            store_taste_vector_postgres(pool, &account_id, &user_vector, distance, &now).await?
        }
    }

    Ok((user_vector, distance))
}

/// Lightweight incremental update: add a single work's contribution.
pub async fn update_taste_vector_incremental(
    db: &Database,
    account_id: &str,
    work_vector: &[f64],
    work_weight: f64,
) -> Result<(Vec<f64>, f64), sqlx::Error> {
    let admin_centroid = get_admin_centroid(db).await.unwrap_or_else(|| {
        vec![0.5, 0.5, 0.5, 0.5, 0.5]
    });

    let current = get_taste_vector(db, account_id).await?;
    let (mut current_vec, _, _) = current.unwrap_or_else(|| {
        (vec![0.0; admin_centroid.len()], 0.0, String::new())
    });

    // Compute old weight sum from stored data (simplified: use count of ratings)
    let old_weight_sum = fetch_user_rating_count(db, account_id).await? as f64;
    let new_sum = taste_vector::update_vector_incremental(
        &mut current_vec,
        work_vector,
        work_weight,
        old_weight_sum,
    );

    let distance = if current_vec.len() == admin_centroid.len() {
        taste_vector::normalized_distance(&current_vec, &admin_centroid, 2.0)
    } else {
        1.0
    };

    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            store_taste_vector_sqlite(db.sqlite_pool().ok_or(pool_err())?, &account_id, &current_vec, distance, &now).await?
        }
        Backend::Postgres => {
            store_taste_vector_postgres(db.postgres_pool().ok_or(pool_err())?, &account_id, &current_vec, distance, &now).await?
        }
    }

    Ok((current_vec, distance))
}

/// Read cached taste vector for a user.
pub async fn get_taste_vector(
    db: &Database,
    account_id: &str,
) -> Result<Option<(Vec<f64>, f64, String)>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => fetch_taste_vector_sqlite(db.sqlite_pool().ok_or(pool_err())?, account_id).await,
        Backend::Postgres => fetch_taste_vector_postgres(db.postgres_pool().ok_or(pool_err())?, account_id).await,
    }
}

/// List users ordered by centroid distance (closest to admin first).
pub async fn list_users_by_centroid_distance(
    db: &Database,
    limit: i64,
) -> Result<Vec<(String, f64)>, sqlx::Error> {
    let rows: Vec<(String, f64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String, f64)>("SELECT id, taste_centroid_distance FROM accounts WHERE taste_vector != '[]' ORDER BY taste_centroid_distance ASC LIMIT ?")
                .bind(limit)
                .fetch_all(db.sqlite_pool().ok_or(pool_err())?)
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String, f64)>("SELECT id, taste_centroid_distance::float8 FROM accounts WHERE taste_vector != '[]'::jsonb ORDER BY taste_centroid_distance ASC LIMIT $1")
                .bind(limit)
                .fetch_all(db.postgres_pool().ok_or(pool_err())?)
                .await?
        }
    };
    Ok(rows)
}

/// Compute taste-weighted engagement signal for a set of works.
/// Returns a map of work_id → taste_signal.
pub async fn taste_signal_for_works(
    db: &Database,
    work_ids: &[String],
    mode: &str, // "egalitarian" | "taste_weighted" | "admin_only"
    admin_weight: f64,
) -> Result<std::collections::HashMap<String, f64>, sqlx::Error> {
    let mut result = std::collections::HashMap::new();
    if work_ids.is_empty() {
        return Ok(result);
    }

    // Build placeholders for IN clause
    let placeholders: Vec<String> = (1..=work_ids.len()).map(|i| format!("?{}", i)).collect();
    let placeholder_str = placeholders.join(",");

    let query = format!(
        "SELECT w.id,
                COALESCE(AVG(CASE WHEN a.taste_vector = '[]' THEN 0.0 ELSE a.taste_centroid_distance END), 0.0) as avg_distance,
                COUNT(DISTINCT a.id) as engager_count
         FROM works w
         LEFT JOIN work_engagements we ON we.work_id = w.id
         LEFT JOIN accounts a ON a.id = we.account_id
         WHERE w.id IN ({})
         GROUP BY w.id",
        placeholder_str
    );

    // This is a simplified version. In production, we'd need proper parameter binding.
    // For now, return empty signals (the full implementation would join engagement data).
    for work_id in work_ids {
        result.insert(work_id.clone(), 0.0);
    }

    Ok(result)
}

/// Weekly batch: recompute all taste vectors.
pub async fn recompute_all_taste_vectors(db: &Database) -> Result<(), sqlx::Error> {
    let account_ids: Vec<(String,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as("SELECT id FROM accounts")
                .fetch_all(db.sqlite_pool().ok_or(pool_err())?)
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as("SELECT id FROM accounts")
                .fetch_all(db.postgres_pool().ok_or(pool_err())?)
                .await?
        }
    };

    for (id,) in account_ids {
        let _ = compute_and_store_taste_vector(db, &id).await;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

async fn get_admin_centroid(db: &Database) -> Option<Vec<f64>> {
    // In a real implementation, this would fetch from config or compute from admin ratings.
    // For now, return a default neutral centroid.
    Some(vec![0.5, 0.5, 0.5, 0.5, 0.5])
}

async fn fetch_user_rated_work_vectors(
    db: &Database,
    account_id: &str,
) -> Result<(Vec<Vec<f64>>, Vec<f64>), sqlx::Error> {
    // Fetch the user's ratings with associated work vectors.
    // Simplified: return empty for now (full implementation would join ratings with work metadata).
    Ok((vec![], vec![]))
}

async fn fetch_user_rating_count(db: &Database, account_id: &str) -> Result<i64, sqlx::Error> {
    let count: i64 = match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query("SELECT COUNT(*) as cnt FROM work_ratings WHERE account_id = ?")
                .bind(account_id)
                .fetch_one(db.sqlite_pool().ok_or(pool_err())?)
                .await?;
            row.get("cnt")
        }
        Backend::Postgres => {
            let row = sqlx::query("SELECT COUNT(*) as cnt FROM work_ratings WHERE account_id = $1")
                .bind(account_id)
                .fetch_one(db.postgres_pool().ok_or(pool_err())?)
                .await?;
            row.get("cnt")
        }
    };
    Ok(count)
}
