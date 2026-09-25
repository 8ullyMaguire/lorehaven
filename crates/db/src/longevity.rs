//! Longevity signals repository: half-life scoring and interaction warmth
//! (spec §41). Both dialects.

use anyhow::Result;
use lorehaven_domain::ids::WorkId;

use crate::{sql_owned, Backend, Database};

// ---------------------------------------------------------------------------
// Half-life
// ---------------------------------------------------------------------------

/// Recompute `half_life_bp` for all works published at least `min_age_days` ago.
///
/// For each eligible work:
///   - Count readers who started in the trailing `window_days` ("recent_starts").
///   - Count readers who started in the first `window_days` after publication
///     ("first_window_starts").
///   - Compute basis points via `lorehaven_domain::longevity::half_life_bp`.
///   - Write `half_life_bp` (NULL = not scored).
///
/// Idempotent by construction — pure overwrite.
pub async fn recompute_half_life(
    db: &Database,
    min_age_days: i64,
    window_days: i64,
) -> Result<u64> {
    let mut total_updated: u64 = 0;

    // Eligible works: published at least min_age_days ago, not deleted.
    let eligible: Vec<String> = {
        let sql = sql_owned(
            db,
            format!(
                "SELECT w.id FROM works w
                 WHERE w.deleted_at IS NULL
                   AND w.lifecycle = 'published'
                   AND w.created_at < datetime('now', '-{min_age_days} days')"
            ),
            format!(
                "SELECT w.id::text FROM works w
                 WHERE w.deleted_at IS NULL
                   AND w.lifecycle = 'published'
                   AND w.created_at < NOW() - INTERVAL '{min_age_days} days'"
            ),
        );
        match db.backend() {
            Backend::Sqlite => {
                sqlx::query_scalar(&sql)
                    .fetch_all(db.sqlite_pool().expect("sqlite"))
                    .await?
            }
            Backend::Postgres => {
                sqlx::query_scalar(&sql)
                    .fetch_all(db.postgres_pool().expect("postgres"))
                    .await?
            }
        }
    };

    for work_id in eligible {
        let recent = count_recent_starts(db, &work_id, window_days).await?;
        let first_window = count_first_window_starts(db, &work_id, window_days).await?;
        let bp = lorehaven_domain::longevity::half_life_bp(recent, first_window);

        let sql = db.sql(
            "UPDATE works SET half_life_bp = ? WHERE id = ?",
            "UPDATE works SET half_life_bp = $1 WHERE id::text = $2",
        );
        let rows = match db.backend() {
            Backend::Sqlite => sqlx::query(&sql)
                .bind(bp)
                .bind(&work_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?
                .rows_affected(),
            Backend::Postgres => sqlx::query(&sql)
                .bind(bp)
                .bind(&work_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?
                .rows_affected(),
        };
        total_updated += rows;
    }

    Ok(total_updated)
}

/// Count readers who started the work in the trailing window.
async fn count_recent_starts(db: &Database, work_id: &str, window_days: i64) -> Result<i64> {
    let sql = sql_owned(
        db,
        format!(
            "SELECT COUNT(DISTINCT pseud_id) FROM reading_history_entry
             WHERE subject_type = 'work' AND subject_id = ?
               AND last_read_at >= datetime('now', '-{window_days} days')"
        ),
        format!(
            "SELECT COUNT(DISTINCT pseud_id) FROM reading_history_entry
             WHERE subject_type = 'work' AND subject_id::text = $1
               AND last_read_at >= NOW() - INTERVAL '{window_days} days'"
        ),
    );
    let count: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(count)
}

/// Count readers who started the work in its first window.
async fn count_first_window_starts(db: &Database, work_id: &str, window_days: i64) -> Result<i64> {
    let sql = sql_owned(
        db,
        format!(
            "SELECT COUNT(DISTINCT h.pseud_id) FROM reading_history_entry h
             JOIN works w ON w.id = h.subject_id
             WHERE h.subject_type = 'work' AND h.subject_id = ?
               AND h.last_read_at >= w.created_at
               AND h.last_read_at < datetime(w.created_at, '+{window_days} days')"
        ),
        format!(
            "SELECT COUNT(DISTINCT h.pseud_id) FROM reading_history_entry h
             JOIN works w ON w.id::text = h.subject_id::text
             WHERE h.subject_type = 'work' AND h.subject_id::text = $1
               AND h.last_read_at >= w.created_at
               AND h.last_read_at < w.created_at + INTERVAL '{window_days} days'"
        ),
    );
    let count: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(count)
}

/// Get `half_life_bp` for a work (None if not scored).
pub async fn half_life_of(db: &Database, work_id: &WorkId) -> Result<Option<i64>> {
    let sql = db.sql(
        "SELECT half_life_bp FROM works WHERE id = ?",
        "SELECT half_life_bp FROM works WHERE id::text = $1",
    );
    let bp: Option<i64> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(work_id.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(work_id.to_string())
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(bp)
}

/// Fetch half-life scores for multiple works at once.
pub async fn half_life_map(
    db: &Database,
    work_ids: &[WorkId],
) -> Result<Vec<(String, Option<i64>)>> {
    if work_ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = work_ids.iter().map(|w| w.to_string()).collect();
    // The previous inline builder branched on `i == 0` with an empty string in
    // both arms; `library::placeholders` is the house helper and says what it
    // means. SQLite takes the same "?" for every bind.
    let placeholder_str = crate::library::placeholders(ids.len(), false);

    let sql = format!("SELECT id, half_life_bp FROM works WHERE id IN ({placeholder_str})");
    let rows: Vec<(String, Option<i64>)> = match db.backend() {
        Backend::Sqlite => {
            let mut query = sqlx::query_as(&sql);
            for id in &ids {
                query = query.bind(id);
            }
            query.fetch_all(db.sqlite_pool().expect("sqlite")).await?
        }
        Backend::Postgres => {
            let pg_placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("${i}")).collect();
            let pg_sql = format!(
                "SELECT id::text, half_life_bp FROM works WHERE id::text IN ({})",
                pg_placeholders.join(",")
            );
            let mut query = sqlx::query_as(&pg_sql);
            for id in &ids {
                query = query.bind(id);
            }
            query
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Warmth
// ---------------------------------------------------------------------------

/// Record warmth between a reader and an author, inside an existing transaction.
pub async fn record_warmth(
    db: &Database,
    reader_account: &str,
    author_account: &str,
    delta_bp: i64,
    thresholds: &lorehaven_domain::longevity::WarmthThresholds,
) -> Result<()> {
    let now = crate::identity::now_rfc3339();

    // Upsert warmth, then recompute tier from the accumulated value.
    let upsert_sql = db.sql(
        "INSERT INTO interaction_warmth (account_id, author_account, warmth_bp, tier, updated_at)
         VALUES (?, ?, ?, 'lurk', ?)
         ON CONFLICT (account_id, author_account) DO UPDATE SET
           warmth_bp = warmth_bp + excluded.warmth_bp,
           updated_at = excluded.updated_at",
        "INSERT INTO interaction_warmth (account_id, author_account, warmth_bp, tier, updated_at)
         VALUES (?::uuid, ?::uuid, ?, 'lurk', ?)
         ON CONFLICT (account_id, author_account) DO UPDATE SET
           warmth_bp = interaction_warmth.warmth_bp + EXCLUDED.warmth_bp,
           updated_at = EXCLUDED.updated_at",
    );

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&upsert_sql)
                .bind(reader_account)
                .bind(author_account)
                .bind(delta_bp)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&upsert_sql)
                .bind(reader_account)
                .bind(author_account)
                .bind(delta_bp)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }

    // Now recompute tier based on accumulated warmth.
    let get_bp_sql = db.sql(
        "SELECT warmth_bp FROM interaction_warmth WHERE account_id = ? AND author_account = ?",
        "SELECT CAST(warmth_bp AS BIGINT) FROM interaction_warmth WHERE account_id::text = $1 AND author_account::text = $2",
    );
    let bp: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&get_bp_sql)
                .bind(reader_account)
                .bind(author_account)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&get_bp_sql)
                .bind(reader_account)
                .bind(author_account)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
    };

    let tier = lorehaven_domain::longevity::tier_for(bp, thresholds);

    let update_tier_sql = db.sql(
        "UPDATE interaction_warmth SET tier = ?, updated_at = ? WHERE account_id = ? AND author_account = ?",
        "UPDATE interaction_warmth SET tier = $1, updated_at = $2::timestamptz WHERE account_id::text = $3 AND author_account::text = $4",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&update_tier_sql)
                .bind(tier)
                .bind(&now)
                .bind(reader_account)
                .bind(author_account)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&update_tier_sql)
                .bind(tier)
                .bind(&now)
                .bind(reader_account)
                .bind(author_account)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }

    Ok(())
}

/// Get aggregate tier counts for an author's reader relationships.
/// Returns Vec<(tier, count)>.
pub async fn author_tier_aggregates(
    db: &Database,
    author_account: &str,
) -> Result<Vec<(String, i64)>> {
    let sql = sql_owned(
        db,
        "SELECT tier, COUNT(*) AS count FROM interaction_warmth
         WHERE author_account = ? GROUP BY tier"
            .to_string(),
        "SELECT tier, COUNT(*) AS count FROM interaction_warmth
         WHERE author_account::text = $1 GROUP BY tier"
            .to_string(),
    );
    let rows: Vec<(String, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(author_account)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(author_account)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows)
}
