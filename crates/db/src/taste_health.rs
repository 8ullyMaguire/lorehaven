//! M17 Phase 2 — Health Layer (spec §0.4.1, §0.4.2, §16.19).
//!
//! Admin taste profile persistence, per-work taste vectors derived from taxonomy
//! tags, onboarding quiz answers, and taste probe engagement.

use sqlx::Row;

use crate::{Backend, Database};

fn pool_err() -> sqlx::Error {
    sqlx::Error::PoolClosed
}

// ---------------------------------------------------------------------------
// Admin taste profile
// ---------------------------------------------------------------------------

/// Replace the stored admin taste profile with `dimensions`.
/// Each entry: (dimension_key, label, admin_target, weight).
pub async fn save_admin_taste_profile(
    db: &Database,
    dimensions: &[(String, String, f64, f64)],
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().ok_or(pool_err())?;
            let mut tx = pool.begin().await?;
            sqlx::query("DELETE FROM admin_taste_profile")
                .execute(&mut *tx)
                .await?;
            for (key, label, target, weight) in dimensions {
                sqlx::query(
                    "INSERT INTO admin_taste_profile (dimension_key, label, admin_target, weight, updated_at)
                     VALUES (?, ?, ?, ?, ?)",
                )
                .bind(key)
                .bind(label)
                .bind(target)
                .bind(weight)
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().ok_or(pool_err())?;
            let mut tx = pool.begin().await?;
            sqlx::query("DELETE FROM admin_taste_profile")
                .execute(&mut *tx)
                .await?;
            for (key, label, target, weight) in dimensions {
                sqlx::query(
                    "INSERT INTO admin_taste_profile (dimension_key, label, admin_target, weight, updated_at)
                     VALUES ($1, $2, $3, $4, $5)",
                )
                .bind(key)
                .bind(label)
                .bind(target)
                .bind(weight)
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await?;
        }
    }
    Ok(())
}

/// Read the stored admin taste profile: Vec of (key, label, target, weight).
pub async fn get_admin_taste_profile(
    db: &Database,
) -> Result<Vec<(String, String, f64, f64)>, sqlx::Error> {
    let sql = "SELECT dimension_key, label, admin_target, weight FROM admin_taste_profile ORDER BY dimension_key ASC";
    let rows: Vec<(String, String, f64, f64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(sql)
                .fetch_all(db.sqlite_pool().ok_or(pool_err())?)
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String, String, f64, f64)>(sql)
                .fetch_all(db.postgres_pool().ok_or(pool_err())?)
                .await?
        }
    };
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Work taste vectors (derived from taxonomy tags)
// ---------------------------------------------------------------------------

/// Compute and cache a work's taste vector from its tags.
///
/// A work's score on dimension `k` is: 0.5 (neutral) + Σ(tag weight / 100)
/// clamped to [0,1] for tags whose canonical name matches `k` (case-insensitive
/// substring match, e.g. tag "angst" matches dimension "angst"). Tags that match
/// no dimension contribute nothing. The vector is ordered by dimension key sort
/// order, matching `admin_taste_profile` ordering.
pub async fn compute_and_store_work_vector(
    db: &Database,
    work_id: &str,
) -> Result<Vec<f64>, sqlx::Error> {
    let dimensions = get_admin_taste_profile(db).await?;
    if dimensions.is_empty() {
        return Ok(vec![]);
    }
    let tag_pairs = crate::taxonomy::tag_weights_for_work(db, work_id).await?;
    let vector = lorehaven_domain::taste_vector::work_vector_from_tags(&dimensions, &tag_pairs);
    let now = crate::identity::now_rfc3339();
    store_work_vector(db, work_id, &vector, &now).await?;
    Ok(vector)
}

/// Store a pre-computed work taste vector.
pub async fn store_work_vector(
    db: &Database,
    work_id: &str,
    vector: &[f64],
    computed_at: &str,
) -> Result<(), sqlx::Error> {
    let vec_str = serde_json::to_string(vector).unwrap_or_else(|_| "[]".to_string());
    let sql = db.sql(
        "INSERT INTO work_taste_vectors (work_id, vector, computed_at) VALUES (?, ?, ?)
         ON CONFLICT(work_id) DO UPDATE SET vector = excluded.vector, computed_at = excluded.computed_at",
        "INSERT INTO work_taste_vectors (work_id, vector, computed_at) VALUES ($1::uuid, $2, $3)
         ON CONFLICT(work_id) DO UPDATE SET vector = excluded.vector, computed_at = excluded.computed_at",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(work_id)
                .bind(&vec_str)
                .bind(computed_at)
                .execute(db.sqlite_pool().ok_or(pool_err())?)
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(work_id)
                .bind(serde_json::to_value(vector).unwrap_or(serde_json::json!([])))
                .bind(computed_at)
                .execute(db.postgres_pool().ok_or(pool_err())?)
                .await?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Work taste vector reads
// ---------------------------------------------------------------------------

/// Read a work's cached taste vector (empty when never computed).
pub async fn get_work_vector(db: &Database, work_id: &str) -> Result<Vec<f64>, sqlx::Error> {
    let sql = db.sql(
        "SELECT vector FROM work_taste_vectors WHERE work_id = ?",
        "SELECT vector FROM work_taste_vectors WHERE work_id = $1::uuid",
    );
    let vector: Option<Vec<f64>> = match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(&sql)
                .bind(work_id)
                .fetch_optional(db.sqlite_pool().ok_or(pool_err())?)
                .await?;
            row.map(|r| {
                let s: String = r.get("vector");
                serde_json::from_str(&s).unwrap_or_default()
            })
        }
        Backend::Postgres => {
            let row = sqlx::query(&sql)
                .bind(work_id)
                .fetch_optional(db.postgres_pool().ok_or(pool_err())?)
                .await?;
            row.map(|r| {
                let v: serde_json::Value = r.get("vector");
                match v {
                    serde_json::Value::Array(arr) => {
                        arr.iter().filter_map(|x| x.as_f64()).collect()
                    }
                    _ => vec![],
                }
            })
        }
    };
    Ok(vector.unwrap_or_default())
}

/// Read cached taste vectors for many works at once. Returns a map keyed by
/// work_id; works without a cached vector are absent.
pub async fn get_work_vectors(
    db: &Database,
    work_ids: &[String],
) -> Result<std::collections::HashMap<String, Vec<f64>>, sqlx::Error> {
    let mut out = std::collections::HashMap::new();
    if work_ids.is_empty() {
        return Ok(out);
    }
    let placeholders = crate::library::placeholders(work_ids.len(), false);
    let sql =
        format!("SELECT work_id, vector FROM work_taste_vectors WHERE work_id IN ({placeholders})");
    let mut rows: Vec<(String, String)> = Vec::new();
    match db.backend() {
        Backend::Sqlite => {
            let mut q = sqlx::query_as::<_, (String, String)>(&sql);
            for id in work_ids {
                q = q.bind(id);
            }
            rows = q.fetch_all(db.sqlite_pool().ok_or(pool_err())?).await?;
        }
        Backend::Postgres => {
            // Rebuild with $n placeholders for PG.
            let pg_placeholders: Vec<String> =
                (1..=work_ids.len()).map(|i| format!("${i}")).collect();
            let sql = format!(
                "SELECT work_id::text AS work_id, vector::text AS vector FROM work_taste_vectors WHERE work_id IN ({})",
                pg_placeholders.join(", ")
            );
            let mut q = sqlx::query_as::<_, (String, String)>(&sql);
            for id in work_ids {
                q = q.bind(id);
            }
            rows = q.fetch_all(db.postgres_pool().ok_or(pool_err())?).await?;
        }
    }
    for (id, vec_str) in rows {
        let vec: Vec<f64> = serde_json::from_str(&vec_str).unwrap_or_default();
        out.insert(id, vec);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Onboarding quiz (spec §0.4.2)
// ---------------------------------------------------------------------------

/// Store quiz answers: one row per (work_id, picked) pair. Replaces any prior
/// answers for the account.
pub async fn save_quiz_answers(
    db: &Database,
    account_id: &str,
    answers: &[(String, bool)],
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    for (work_id, picked) in answers {
        let id = uuid::Uuid::new_v4().to_string();
        let sql = db.sql(
            "INSERT INTO quiz_answers (id, account_id, work_id, picked, answered_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(account_id, work_id) DO UPDATE SET picked = excluded.picked, answered_at = excluded.answered_at",
            "INSERT INTO quiz_answers (id, account_id, work_id, picked, answered_at)
             VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5)
             ON CONFLICT(account_id, work_id) DO UPDATE SET picked = excluded.picked, answered_at = excluded.answered_at",
        );
        match db.backend() {
            Backend::Sqlite => {
                sqlx::query(&sql)
                    .bind(&id)
                    .bind(account_id)
                    .bind(work_id)
                    .bind(if *picked { 1 } else { 0 })
                    .bind(&now)
                    .execute(db.sqlite_pool().ok_or(pool_err())?)
                    .await?;
            }
            Backend::Postgres => {
                sqlx::query(&sql)
                    .bind(&id)
                    .bind(account_id)
                    .bind(work_id)
                    .bind(if *picked { 1 } else { 0 })
                    .bind(&now)
                    .execute(db.postgres_pool().ok_or(pool_err())?)
                    .await?;
            }
        }
    }
    Ok(())
}

/// Read quiz answers for an account: Vec of (work_id, picked).
pub async fn get_quiz_answers(
    db: &Database,
    account_id: &str,
) -> Result<Vec<(String, bool)>, sqlx::Error> {
    let sql = db.sql(
        "SELECT work_id, picked FROM quiz_answers WHERE account_id = ?",
        "SELECT work_id::text, picked FROM quiz_answers WHERE account_id = $1::uuid",
    );
    let rows: Vec<(String, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .fetch_all(db.sqlite_pool().ok_or(pool_err())?)
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .fetch_all(db.postgres_pool().ok_or(pool_err())?)
                .await?
        }
    };
    Ok(rows.into_iter().map(|(w, p)| (w, p != 0)).collect())
}

/// Compute and store the initial taste vector from quiz selections (spec
/// §0.4.2): the weighted centroid of the picked works' vectors, blended 50/50
/// with neutral so quiz-only data never produces extreme vectors.
pub async fn compute_and_store_quiz_vector(
    db: &Database,
    account_id: &str,
) -> Result<Vec<f64>, sqlx::Error> {
    let answers = get_quiz_answers(db, account_id).await?;
    let picked_ids: Vec<String> = answers
        .iter()
        .filter(|(_, picked)| *picked)
        .map(|(work_id, _)| work_id.clone())
        .collect();
    let vectors = get_work_vectors(db, &picked_ids).await?;
    let mut work_vectors: Vec<Vec<f64>> = Vec::new();
    for id in &picked_ids {
        if let Some(v) = vectors.get(id) {
            if !v.is_empty() {
                work_vectors.push(v.clone());
            }
        }
    }
    let quiz_vector = if work_vectors.is_empty() {
        vec![]
    } else {
        let n = work_vectors[0].len();
        let weights = vec![1.0; work_vectors.len()];
        let centroid = lorehaven_domain::taste_vector::compute_user_vector(&work_vectors, &weights);
        // Blend toward neutral 0.5 by (n+2)/(n+4) so quiz-only data stays moderate.
        let t = 2.0 / (work_vectors.len() as f64 + 4.0);
        lorehaven_domain::taste_vector::blend_vectors(&centroid, &vec![0.5; n], t)
    };
    if quiz_vector.is_empty() {
        return Ok(vec![]);
    }
    // Distance against the admin centroid.
    let dimensions = get_admin_taste_profile(db).await?;
    let admin_centroid: Vec<f64> = {
        let mut sorted = dimensions.clone();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        sorted.iter().map(|(_, _, target, _)| *target).collect()
    };
    let distance = if quiz_vector.len() == admin_centroid.len() {
        lorehaven_domain::taste_vector::normalized_distance(&quiz_vector, &admin_centroid, 2.0)
    } else {
        1.0
    };
    let now = crate::identity::now_rfc3339();
    crate::taste_vectors::store_taste_vector_public(db, account_id, &quiz_vector, distance, &now)
        .await?;
    Ok(quiz_vector)
}

// ---------------------------------------------------------------------------
// Taste probes (spec §16.19)
// ---------------------------------------------------------------------------

/// Record how a user engaged with a probe work.
pub async fn record_probe_engagement(
    db: &Database,
    account_id: &str,
    work_id: &str,
    engagement: &str,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();
    let sql = db.sql(
        "INSERT INTO taste_probes (id, account_id, work_id, engagement, engaged_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(account_id, work_id) DO UPDATE SET engagement = excluded.engagement, engaged_at = excluded.engaged_at",
        "INSERT INTO taste_probes (id, account_id, work_id, engagement, engaged_at)
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5)
         ON CONFLICT(account_id, work_id) DO UPDATE SET engagement = excluded.engagement, engaged_at = excluded.engaged_at",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(account_id)
                .bind(work_id)
                .bind(engagement)
                .bind(&now)
                .execute(db.sqlite_pool().ok_or(pool_err())?)
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(account_id)
                .bind(work_id)
                .bind(engagement)
                .bind(&now)
                .execute(db.postgres_pool().ok_or(pool_err())?)
                .await?;
        }
    }
    Ok(())
}

/// Read probe engagement for a user: Vec of (work_id, engagement kind).
pub async fn get_probe_engagements(
    db: &Database,
    account_id: &str,
) -> Result<Vec<(String, String)>, sqlx::Error> {
    let sql = db.sql(
        "SELECT work_id, engagement FROM taste_probes WHERE account_id = ?",
        "SELECT work_id::text, engagement FROM taste_probes WHERE account_id = $1::uuid",
    );
    let rows: Vec<(String, String)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .fetch_all(db.sqlite_pool().ok_or(pool_err())?)
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .fetch_all(db.postgres_pool().ok_or(pool_err())?)
                .await?
        }
    };
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Admin-curated quiz work set
// ---------------------------------------------------------------------------

/// The admin-curated quiz work pool (spec §0.4.2). When empty, quiz works fall
/// back to recently published works.
pub async fn list_quiz_works(
    db: &Database,
    limit: i64,
) -> Result<Vec<serde_json::Value>, sqlx::Error> {
    let sql = db.sql(
        "SELECT w.id, w.title, w.summary, w.rating, w.language
         FROM quiz_works qw
         JOIN works w ON w.id = qw.work_id
         WHERE w.deleted_at IS NULL
         ORDER BY qw.position ASC
         LIMIT ?",
        "SELECT w.id::text, w.title, w.summary, w.rating, w.language
         FROM quiz_works qw
         JOIN works w ON w.id = qw.work_id
         WHERE w.deleted_at IS NULL
         ORDER BY qw.position ASC
         LIMIT $1",
    );
    let out: Vec<serde_json::Value> = match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(&sql)
                .bind(limit)
                .fetch_all(db.sqlite_pool().ok_or(pool_err())?)
                .await?;
            rows.iter()
                .map(|row| {
                    serde_json::json!({
                        "work_id": row.get::<String, _>("id"),
                        "title": row.get::<String, _>("title"),
                        "summary": row.get::<String, _>("summary"),
                        "rating": row.get::<String, _>("rating"),
                        "language": row.get::<String, _>("language"),
                    })
                })
                .collect()
        }
        Backend::Postgres => {
            let rows = sqlx::query(&sql)
                .bind(limit)
                .fetch_all(db.postgres_pool().ok_or(pool_err())?)
                .await?;
            rows.iter()
                .map(|row| {
                    serde_json::json!({
                        "work_id": row.get::<String, _>("id"),
                        "title": row.get::<String, _>("title"),
                        "summary": row.get::<String, _>("summary"),
                        "rating": row.get::<String, _>("rating"),
                        "language": row.get::<String, _>("language"),
                    })
                })
                .collect()
        }
    };
    Ok(out)
}

/// Replace the admin-curated quiz work pool.
pub async fn set_admin_quiz_works(db: &Database, work_ids: &[String]) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().ok_or(pool_err())?;
            let mut tx = pool.begin().await?;
            sqlx::query("DELETE FROM quiz_works")
                .execute(&mut *tx)
                .await?;
            for (i, id) in work_ids.iter().enumerate() {
                sqlx::query("INSERT INTO quiz_works (work_id, position, set_at) VALUES (?, ?, ?)")
                    .bind(id)
                    .bind(i as i64)
                    .bind(&now)
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().ok_or(pool_err())?;
            let mut tx = pool.begin().await?;
            sqlx::query("DELETE FROM quiz_works")
                .execute(&mut *tx)
                .await?;
            for (i, id) in work_ids.iter().enumerate() {
                sqlx::query(
                    "INSERT INTO quiz_works (work_id, position, set_at) VALUES ($1::uuid, $2, $3)",
                )
                .bind(id)
                .bind(i as i64)
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await?;
        }
    }
    Ok(())
}
