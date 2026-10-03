//! M44 taste vectors — multi-dimensional user taste alignment (spec §16.17).
//! DB functions for computing, storing, and querying taste vectors.

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
         FROM accounts WHERE id = $1::uuid",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await?;
    match row {
        // An account that skipped the quiz has a row but no vector: every one of
        // `taste_vector`, `taste_centroid_distance` and the computed_at that
        // `to_char` derives from it is NULL. SQLite returns an empty string for
        // the stored NULL so it never hits this, but PostgreSQL decodes a NULL
        // into a decode error on a non-optional `String`. "No vector" is the
        // honest answer here, and it is what the SQLite arm already returns.
        Some(r)
            if r.get::<Option<serde_json::Value>, _>("taste_vector")
                .is_none()
                || r.get::<Option<f64>, _>("taste_centroid_distance").is_none() =>
        {
            Ok(None)
        }
        Some(r) => {
            let vec_json: serde_json::Value = r.get("taste_vector");
            let vec: Vec<f64> = match vec_json {
                serde_json::Value::Array(arr) => arr.iter().filter_map(|v| v.as_f64()).collect(),
                _ => vec![],
            };
            let dist: f64 = r.get("taste_centroid_distance");
            // `taste_vector_computed_at` is nullable and nothing backfills it, so
            // a vector written by a path that predates the column reads back with
            // no timestamp. Report the empty string rather than failing the whole
            // read: the vector itself is the useful part, and callers that care
            // about the timestamp already treat "" as unknown.
            let computed_at: String = r
                .try_get::<Option<String>, _>("computed_at")
                .ok()
                .flatten()
                .unwrap_or_default();
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
         taste_vector_computed_at = $3::timestamptz WHERE id = $4::uuid",
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
            store_taste_vector_sqlite(pool, account_id, &user_vector, distance, &now).await?
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().ok_or(pool_err())?;
            store_taste_vector_postgres(pool, account_id, &user_vector, distance, &now).await?
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
    // `Some((vec![], ..))` is the case a `None` default does not cover: the
    // account row exists, so `get_taste_vector` returns `Some`, and its read of a
    // NULL `taste_vector` yields an EMPTY vector rather than `None`. Without this
    // arm the "no vector yet" path produced a zero-width vector, the width check
    // against the centroid failed, and the update silently reported
    // `distance = 1.0` (maximally distant) for a reader who had simply never been
    // scored. An empty vector is therefore treated as absent, which is what it
    // means.
    let (mut current_vec, _, _) = match current {
        Some((vec, dist, at)) if !vec.is_empty() => (vec, dist, at),
        _ => (vec![0.0; admin_centroid.len()], 0.0, String::new()),
    };

    // Compute old weight sum from stored data (simplified: use count of ratings)
    let old_weight_sum = fetch_user_rating_count(db, account_id).await? as f64;
    let _new_sum = taste_vector::update_vector_incremental(
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
                account_id,
                &current_vec,
                distance,
                &now,
            )
            .await?
        }
        Backend::Postgres => {
            store_taste_vector_postgres(
                db.postgres_pool().ok_or(pool_err())?,
                account_id,
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
    _db: &Database,
    work_ids: &[String],
    _mode: &str, // "egalitarian" | "taste_weighted" | "admin_only"
    _admin_weight: f64,
) -> Result<std::collections::HashMap<String, f64>, sqlx::Error> {
    let mut result = std::collections::HashMap::new();
    if work_ids.is_empty() {
        return Ok(result);
    }

    // Build placeholders for IN clause
    let placeholders: Vec<String> = (1..=work_ids.len()).map(|i| format!("?{}", i)).collect();
    let placeholder_str = placeholders.join(",");

    let _query = format!(
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
                // `account_id`, `best_work_id` and `worst_work_id` are UUID on
                // PostgreSQL, so the placeholders need casts. Without them the
                // driver sends them as text and the insert fails 42804.
                "INSERT INTO arena_ballots (account_id, best_work_id, worst_work_id, reason_tags)
                 VALUES ($1::uuid, $2::uuid, $3::uuid, $4::jsonb)",
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
                // `matches_played` is INTEGER on PostgreSQL (INT4) but BIGINT on
                // SQLite, and the row type is i64, so cast it: sqlx refuses to
                // decode INT4 into i64 ("mismatched types ... not compatible").
                "SELECT dimension_key, weight, elo_rating, matches_played::bigint
                 FROM arena_weights WHERE account_id = $1::uuid",
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
                 VALUES ($1::uuid, $2, $3, $4, $5)
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

/// A row in the arena pool: a work's metadata plus the tag weights its taste
/// vector is built from.
#[derive(Debug, Clone)]
pub struct ArenaPoolRow {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub fandom: String,
    pub tags: Vec<String>,
    pub word_count: u32,
    /// The work's tags with their signed weights, ready for
    /// `work_vector_from_tags`.
    pub tag_weights: Vec<(String, i64)>,
}

/// Get works for arena pool (excluding already-voted works).
pub async fn get_arena_pool(
    db: &Database,
    account_id: &str,
    limit: i64,
) -> Result<Vec<ArenaPoolRow>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let _pool = db.sqlite_pool().ok_or(pool_err())?;
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
                     SELECT c.work_id, CAST(SUM(cr.word_count) AS BIGINT) as word_count
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
            let pool = db.sqlite_pool().ok_or(pool_err())?;
            let mut out = Vec::with_capacity(rows.len());
            for (id, title, summary, fandom, tags, wc) in rows {
                let tags: Vec<String> = tags
                    .map(|t| t.split(',').map(|s| s.to_string()).collect())
                    .unwrap_or_default();
                let tag_weights = fetch_work_tag_weights_sqlite(pool, &id).await?;
                out.push(ArenaPoolRow {
                    id,
                    title,
                    summary,
                    fandom,
                    tags,
                    word_count: wc as u32,
                    tag_weights,
                });
            }
            Ok(out)
        }
        Backend::Postgres => {
            let rows = sqlx::query_as::<_, (String, String, String, String, Option<String>, i64)>(
                // `w.id` is UUID on PostgreSQL and the row type is String, so cast
                // it to text: sqlx will not decode UUID into String.
                "SELECT w.id::text, w.title, w.summary,
                        COALESCE((SELECT tn.canonical FROM taxonomy_nodes tn
                                  JOIN work_tags wt ON tn.id = wt.node_id
                                  WHERE wt.work_id = w.id AND tn.kind = 'fandom'
                                  LIMIT 1), ''),
                        COALESCE((SELECT string_agg(tn2.canonical, ',')
                                  FROM taxonomy_nodes tn2
                                  JOIN work_tags wt2 ON tn2.id = wt2.node_id
                                  WHERE wt2.work_id = w.id AND tn2.kind = 'tag'), ''),
                        COALESCE(wc.word_count, 0)::bigint
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
                       SELECT best_work_id FROM arena_ballots WHERE account_id = $1::uuid
                       UNION
                       SELECT worst_work_id FROM arena_ballots WHERE account_id = $1::uuid
                   )
                 LIMIT $2",
            )
            .bind(account_id)
            .bind(limit)
            .fetch_all(db.postgres_pool().ok_or(pool_err())?)
            .await?;
            let pool = db.postgres_pool().ok_or(pool_err())?;
            let mut out = Vec::with_capacity(rows.len());
            for (id, title, summary, fandom, tags, wc) in rows {
                let tags: Vec<String> = tags
                    .map(|t| t.split(',').map(|s| s.to_string()).collect())
                    .unwrap_or_default();
                let tag_weights = fetch_work_tag_weights_postgres(pool, &id).await?;
                out.push(ArenaPoolRow {
                    id,
                    title,
                    summary,
                    fandom,
                    tags,
                    word_count: wc as u32,
                    tag_weights,
                });
            }
            Ok(out)
        }
    }
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
    // The instance taste profile is the admin's own ratings: what they read is
    // what "good" means for this archive. Averaging the admin's ratings across
    // works gives the centroid every vector is measured against.
    //
    // Only accounts holding the admin role are considered, so an instance with
    // no admin ratings falls back to neutral rather than inheriting a random
    // user's taste.
    let dim_count = taste_vector::DEFAULT_DIMENSIONS.len();
    let query = match db.backend() {
        Backend::Sqlite => {
            "SELECT tv.vector FROM taste_vectors tv
             JOIN accounts a ON a.id = tv.account_id
             WHERE a.operator_role = 'admin' AND a.deleted_at IS NULL
             ORDER BY tv.computed_at DESC"
        }
        Backend::Postgres => {
            "SELECT tv.vector FROM taste_vectors tv
             JOIN accounts a ON a.id = tv.account_id
             WHERE a.operator_role = 'admin' AND a.deleted_at IS NULL
             ORDER BY tv.computed_at DESC"
        }
    };

    let rows: Vec<(Option<String>,)> = match db.backend() {
        Backend::Sqlite => sqlx::query_as(query)
            .fetch_all(db.sqlite_pool()?)
            .await
            .ok()?,
        Backend::Postgres => sqlx::query_as(query)
            .fetch_all(db.postgres_pool()?)
            .await
            .ok()?,
    };

    let mut sums = vec![0.0; dim_count];
    let mut counted = 0usize;
    for (raw,) in &rows {
        let Some(vector) = raw.as_deref().and_then(decode_vector) else {
            continue;
        };
        for (slot, value) in sums.iter_mut().zip(vector.iter()) {
            *slot += value;
        }
        counted += 1;
    }

    if counted == 0 {
        return None;
    }
    Some(sums.iter().map(|sum| sum / counted as f64).collect())
}

/// Decode a stored taste vector.
///
/// The vector is persisted as a JSON array. Anything unparseable is treated as
/// absent rather than fatal: one corrupt row must not take the recommender down.
fn decode_vector(raw: &str) -> Option<Vec<f64>> {
    serde_json::from_str::<Vec<f64>>(raw).ok().filter(|v| !v.is_empty())
}

/// The weight a star rating carries in a taste vector.
///
/// A 5-star rating is a strong positive signal, 1-star a strong negative one,
/// and 3 stars — the midpoint — is deliberately near-neutral so that "I read
/// it, it was fine" neither pulls a profile nor pushes it.
fn star_weight(stars: i64) -> f64 {
    match stars {
        1 => -1.0,
        2 => -0.5,
        3 => 0.1,
        4 => 0.5,
        _ => 1.0,
    }
}

async fn fetch_user_rated_work_vectors(
    db: &Database,
    account_id: &str,
) -> Result<(Vec<Vec<f64>>, Vec<f64>), sqlx::Error> {
    // Each rated work contributes its own taste vector, weighted by how much
    // the reader liked it. The work's vector comes from its tags via
    // `work_vector_from_tags`, so the profile is built from the same tag
    // semantics the arena and recommendations use.
    let dim_count = taste_vector::DEFAULT_DIMENSIONS.len();
    let dimensions: Vec<(String, String, f64, f64)> = taste_vector::DEFAULT_DIMENSIONS
        .iter()
        .map(|key| ((*key).to_string(), (*key).to_string(), 0.5, 1.0))
        .collect();

    let rows: Vec<(i64, Vec<(String, i64)>)> = match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().ok_or(pool_err())?;
            let rated = sqlx::query_as::<_, (String, i64)>(
                "SELECT r.work_id, r.stars FROM rating r
                 WHERE r.account_id = ? AND r.deleted_at IS NULL",
            )
            .bind(account_id)
            .fetch_all(pool)
            .await?;

            let mut out = Vec::with_capacity(rated.len());
            for (work_id, stars) in rated {
                let tags = fetch_work_tag_weights_sqlite(pool, &work_id).await?;
                out.push((stars, tags));
            }
            out
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().ok_or(pool_err())?;
            let rated = sqlx::query_as::<_, (String, i64)>(
                "SELECT r.work_id, r.stars FROM rating r
                 WHERE r.account_id = $1 AND r.deleted_at IS NULL",
            )
            .bind(account_id)
            .fetch_all(pool)
            .await?;

            let mut out = Vec::with_capacity(rated.len());
            for (work_id, stars) in rated {
                let tags = fetch_work_tag_weights_postgres(pool, &work_id).await?;
                out.push((stars, tags));
            }
            out
        }
    };

    let vectors: Vec<Vec<f64>> = rows
        .iter()
        .map(|(_, tags)| taste_vector::work_vector_from_tags(&dimensions, tags))
        .filter(|v| v.len() == dim_count)
        .collect();
    let weights: Vec<f64> = rows.iter().map(|(stars, _)| star_weight(*stars)).collect();

    Ok((vectors, weights))
}

/// A work's tags and their signed weights, for building its taste vector.
async fn fetch_work_tag_weights_sqlite(
    pool: &sqlx::SqlitePool,
    work_id: &str,
) -> Result<Vec<(String, i64)>, sqlx::Error> {
    sqlx::query_as::<_, (String, i64)>(
        "SELECT tn.canonical, COALESCE(wt.weight, 0)
         FROM work_tags wt
         JOIN taxonomy_nodes tn ON tn.id = wt.node_id
         WHERE wt.work_id = ?",
    )
    .bind(work_id)
    .fetch_all(pool)
    .await
}

/// A work's tags and their signed weights, for building its taste vector.
async fn fetch_work_tag_weights_postgres(
    pool: &sqlx::PgPool,
    work_id: &str,
) -> Result<Vec<(String, i64)>, sqlx::Error> {
    sqlx::query_as::<_, (String, i64)>(
        "SELECT tn.canonical, COALESCE(wt.weight, 0)
         FROM work_tags wt
         JOIN taxonomy_nodes tn ON tn.id = wt.node_id
         WHERE wt.work_id = $1",
    )
    .bind(work_id)
    .fetch_all(pool)
    .await
}

/// How many live ratings this account has, which is the `old_weight_sum` the
/// incremental update needs.
///
/// The table is `rating`, not `work_ratings`. Nothing named `work_ratings` has
/// ever existed in any migration on either dialect, so this query could only ever
/// have returned `relation "work_ratings" does not exist` -- every call site of
/// this function raised that, and the `?` at the call site propagated it into
/// whatever asked for a taste update. Counting `rating` is the same question
/// asked of the table that holds the answers.
///
/// Only rows with `deleted_at IS NULL` are counted, because a soft-deleted
/// rating no longer contributes weight: the incremental update divides by
/// `old_weight_sum + work_weight`, so counting a withdrawn rating would
/// silently shrink every subsequent step rather than fail.
async fn fetch_user_rating_count(db: &Database, account_id: &str) -> Result<i64, sqlx::Error> {
    let count: i64 = match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(
                "SELECT COUNT(*) as cnt FROM rating
                 WHERE account_id = ? AND deleted_at IS NULL",
            )
            .bind(account_id)
            .fetch_one(db.sqlite_pool().ok_or(pool_err())?)
            .await?;
            row.get("cnt")
        }
        Backend::Postgres => {
            // `COUNT(*)` is INT8 on both engines, so `i64` decodes without a cast.
            let row = sqlx::query(
                "SELECT COUNT(*) as cnt FROM rating
                 WHERE account_id = $1::uuid AND deleted_at IS NULL",
            )
            .bind(account_id)
            .fetch_one(db.postgres_pool().ok_or(pool_err())?)
            .await?;
            row.get("cnt")
        }
    };
    Ok(count)
}
