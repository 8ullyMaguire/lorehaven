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
    let admin_centroid = get_admin_centroid(db)
        .await
        .unwrap_or_else(|| vec![0.5, 0.5, 0.5, 0.5, 0.5]);

    let current = get_taste_vector(db, account_id).await?;
    let (mut current_vec, _, _) =
        current.unwrap_or_else(|| (vec![0.0; admin_centroid.len()], 0.0, String::new()));

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
            store_taste_vector_sqlite(
                db.sqlite_pool().ok_or(pool_err())?,
                &account_id,
                &current_vec,
                distance,
                &now,
            )
            .await?
        }
        Backend::Postgres => {
            store_taste_vector_postgres(
                db.postgres_pool().ok_or(pool_err())?,
                &account_id,
                &current_vec,
                distance,
                &now,
            )
            .await?
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
        Backend::Sqlite => {
            fetch_taste_vector_sqlite(db.sqlite_pool().ok_or(pool_err())?, account_id).await
        }
        Backend::Postgres => {
            fetch_taste_vector_postgres(db.postgres_pool().ok_or(pool_err())?, account_id).await
        }
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

// ---------------------------------------------------------------------------
// Taste Calibration Arena (spec §0.4.2a)
// ---------------------------------------------------------------------------

/// Record an arena ballot and update per-dimension Elo ratings.
pub async fn record_arena_ballot(
    db: &Database,
    account_id: &str,
    best_work_id: &str,
    worst_work_id: &str,
    reason_tags: &[String],
) -> Result<(), sqlx::Error> {
    let tags_json = serde_json::to_string(reason_tags).unwrap_or_else(|_| "[]".to_string());
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().ok_or(pool_err())?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let now_str = format!("{}", now);
            sqlx::query(
                "INSERT INTO arena_ballots (id, account_id, best_work_id, worst_work_id, reason_tags, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(account_id)
            .bind(best_work_id)
            .bind(worst_work_id)
            .bind(tags_json)
            .bind(now_str)
            .execute(pool)
            .await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().ok_or(pool_err())?;
            let tags_json = serde_json::to_value(reason_tags).unwrap_or(serde_json::json!([]));
            sqlx::query(
                "INSERT INTO arena_ballots (account_id, best_work_id, worst_work_id, reason_tags)
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(account_id)
            .bind(best_work_id)
            .bind(worst_work_id)
            .bind(tags_json)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

/// Get or create arena weights for an account.
pub async fn get_arena_weights(
    db: &Database,
    account_id: &str,
) -> Result<Vec<(String, f64, f64, i64)>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query_as::<_, (String, f64, f64, i64)>(
                "SELECT dimension_key, weight, elo_rating, matches_played
                 FROM arena_weights WHERE account_id = ?",
            )
            .bind(account_id)
            .fetch_all(db.sqlite_pool().ok_or(pool_err())?)
            .await?;
            Ok(rows)
        }
        Backend::Postgres => {
            let rows = sqlx::query_as::<_, (String, f64, f64, i64)>(
                "SELECT dimension_key, weight, elo_rating, matches_played
                 FROM arena_weights WHERE account_id = $1",
            )
            .bind(account_id)
            .fetch_all(db.postgres_pool().ok_or(pool_err())?)
            .await?;
            Ok(rows)
        }
    }
}

/// Update arena weights after a ballot.
pub async fn update_arena_weights(
    db: &Database,
    account_id: &str,
    dimension_key: &str,
    weight: f64,
    elo_rating: f64,
    matches_played: i64,
) -> Result<(), sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().ok_or(pool_err())?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let now_str = format!("{}", now);
            sqlx::query(
                "INSERT INTO arena_weights (id, account_id, dimension_key, weight, elo_rating, matches_played, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT (account_id, dimension_key) DO UPDATE SET
                   weight = excluded.weight,
                   elo_rating = excluded.elo_rating,
                   matches_played = excluded.matches_played,
                   updated_at = excluded.updated_at",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(account_id)
            .bind(dimension_key)
            .bind(weight)
            .bind(elo_rating)
            .bind(matches_played)
            .bind(now_str.clone())
            .bind(now_str)
            .execute(pool)
            .await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().ok_or(pool_err())?;
            sqlx::query(
                "INSERT INTO arena_weights (account_id, dimension_key, weight, elo_rating, matches_played)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (account_id, dimension_key) DO UPDATE SET
                   weight = EXCLUDED.weight,
                   elo_rating = EXCLUDED.elo_rating,
                   matches_played = EXCLUDED.matches_played,
                   updated_at = now()",
            )
            .bind(account_id)
            .bind(dimension_key)
            .bind(weight)
            .bind(elo_rating)
            .bind(matches_played)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

/// Get works for arena pool (excluding already-voted works).
/// Returns (work_id, title, summary, fandom, tags, word_count).
pub async fn get_arena_pool(
    db: &Database,
    account_id: &str,
    limit: i64,
) -> Result<Vec<(String, String, String, String, Vec<String>, u32)>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().ok_or(pool_err())?;
            let rows = sqlx::query_as::<_, (String, String, String, String, Option<String>, i64)>(
                "SELECT w.id, w.title, w.summary,
                        COALESCE((SELECT tn.canonical FROM taxonomy_nodes tn
                                  JOIN work_tags wt ON tn.id = wt.node_id
                                  WHERE wt.work_id = w.id AND tn.kind = 'fandom'
                                  LIMIT 1), ''),
                        COALESCE((SELECT GROUP_CONCAT(tn2.canonical, ',')
                                  FROM taxonomy_nodes tn2
                                  JOIN work_tags wt2 ON tn2.id = wt2.node_id
                                  WHERE wt2.work_id = w.id AND tn2.kind = 'tag'), ''),
                        COALESCE(wc.word_count, 0)
                 FROM works w
                 LEFT JOIN (
                     SELECT c.work_id, SUM(cr.word_count) as word_count
                     FROM chapters c
                     JOIN chapter_revisions cr ON cr.id = c.current_revision_id
                     GROUP BY c.work_id
                 ) wc ON w.id = wc.work_id
                 WHERE w.lifecycle = 'published'
                   AND w.visibility = 'public'
                   AND w.deleted_at IS NULL
                   AND w.id NOT IN (
                       SELECT best_work_id FROM arena_ballots WHERE account_id = ?
                       UNION
                       SELECT worst_work_id FROM arena_ballots WHERE account_id = ?
                   )
                 LIMIT ?",
            )
            .bind(account_id)
            .bind(account_id)
            .bind(limit)
            .fetch_all(db.sqlite_pool().ok_or(pool_err())?)
            .await?;
            Ok(rows
                .into_iter()
                .map(|(id, title, summary, fandom, tags, wc)| {
                    let tags: Vec<String> = tags
                        .map(|t| t.split(',').map(|s| s.to_string()).collect())
                        .unwrap_or_default();
                    (id, title, summary, fandom, tags, wc as u32)
                })
                .collect())
        }
        Backend::Postgres => {
            let rows = sqlx::query_as::<_, (String, String, String, String, Option<String>, i64)>(
                "SELECT w.id, w.title, w.summary,
                        COALESCE((SELECT tn.canonical FROM taxonomy_nodes tn
                                  JOIN work_tags wt ON tn.id = wt.node_id
                                  WHERE wt.work_id = w.id AND tn.kind = 'fandom'
                                  LIMIT 1), ''),
                        COALESCE((SELECT string_agg(tn2.canonical, ',')
                                  FROM taxonomy_nodes tn2
                                  JOIN work_tags wt2 ON tn2.id = wt2.node_id
                                  WHERE wt2.work_id = w.id AND tn2.kind = 'tag'), ''),
                        COALESCE(wc.word_count, 0)
                 FROM works w
                 LEFT JOIN (
                     SELECT c.work_id, SUM(cr.word_count) as word_count
                     FROM chapters c
                     JOIN chapter_revisions cr ON cr.id = c.current_revision_id
                     GROUP BY c.work_id
                 ) wc ON w.id = wc.work_id
                 WHERE w.lifecycle = 'published'
                   AND w.visibility = 'public'
                   AND w.deleted_at IS NULL
                   AND w.id NOT IN (
                       SELECT best_work_id FROM arena_ballots WHERE account_id = $1
                       UNION
                       SELECT worst_work_id FROM arena_ballots WHERE account_id = $1
                   )
                 LIMIT $2",
            )
            .bind(account_id)
            .bind(limit)
            .fetch_all(db.postgres_pool().ok_or(pool_err())?)
            .await?;
            Ok(rows
                .into_iter()
                .map(|(id, title, summary, fandom, tags, wc)| {
                    let tags: Vec<String> = tags
                        .map(|t| t.split(',').map(|s| s.to_string()).collect())
                        .unwrap_or_default();
                    (id, title, summary, fandom, tags, wc as u32)
                })
                .collect())
        }
    }
}

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
