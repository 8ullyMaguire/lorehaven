//! Work card aggregate metrics — views, complete reads, reactions, kudos,
//! bookmarks, collection adds, review count (spec §9.5, §9.4, §10).
//!
//! The rule that shapes this module: **aggregates are counts, not averages.**
//! Every number on a work card is "how many readers did X", never a mean.
//! The materialized `work_metric_aggregates` row is updated incrementally by
//! the application layer — never by a trigger — so the dialect-parity test
//! in `migrate.rs` stays green.

use anyhow::Result;
use sqlx::FromRow;

use crate::{Backend, Database};
use lorehaven_domain::WorkId;

/// Public aggregate counts for one work, as shown on the work card.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct WorkMetrics {
    pub work_id: String,
    pub views: i64,
    pub complete_reads: i64,
    pub reactions: i64,
    pub kudos: i64,
    pub bookmarks: i64,
    pub collection_adds: i64,
    pub reviews: i64,
}

impl WorkMetrics {
    /// A work with no activity yet — every counter at zero.
    pub fn empty(work_id: impl Into<String>) -> Self {
        Self {
            work_id: work_id.into(),
            views: 0,
            complete_reads: 0,
            reactions: 0,
            kudos: 0,
            bookmarks: 0,
            collection_adds: 0,
            reviews: 0,
        }
    }
}

/// Read the materialized aggregate for one work, falling back to a live count
/// if the row has not been materialized yet (e.g., before the seed script ran).
pub async fn get_metrics(db: &Database, work_id: &WorkId) -> Result<WorkMetrics> {
    let wid = work_id.to_string();
    let sql = db.sql(
        "SELECT work_id, views, complete_reads, reactions, kudos, bookmarks,
                collection_adds, reviews
           FROM work_metric_aggregates WHERE work_id = ?",
        "SELECT work_id, views, complete_reads, reactions, kudos, bookmarks,
                collection_adds, reviews
           FROM work_metric_aggregates WHERE work_id::text = $1",
    );
    let row: Option<WorkMetrics> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, WorkMetrics>(&sql)
                .bind(&wid)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, WorkMetrics>(&sql)
                .bind(&wid)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    match row {
        Some(m) => Ok(m),
        None => {
            let live = compute_live(db, &wid).await?;
            Ok(WorkMetrics {
                work_id: wid,
                ..live
            })
        }
    }
}

/// Compute every counter from source tables — used by the seed script and as
/// a fallback when the materialized row is missing.
pub async fn compute_live(db: &Database, work_id: &str) -> Result<WorkMetrics> {
    let views = count_views(db, work_id).await?;
    let complete_reads = count_complete_reads(db, work_id).await?;
    let reactions = count_reactions(db, work_id).await?;
    let kudos = count_kudos(db, work_id).await?;
    let bookmarks = count_bookmarks(db, work_id).await?;
    let collection_adds = count_collection_adds(db, work_id).await?;
    let reviews = count_reviews(db, work_id).await?;
    Ok(WorkMetrics {
        work_id: work_id.to_string(),
        views,
        complete_reads,
        reactions,
        kudos,
        bookmarks,
        collection_adds,
        reviews,
    })
}

// ---------------------------------------------------------------------------
// Individual counters
// ---------------------------------------------------------------------------

/// Views: total deduplicated view events from the log, excluding automated
/// traffic. The PK already dedupes (work_id, viewer_hash, viewed_at), so each
/// row is one unique view event.
pub async fn count_views(db: &Database, work_id: &str) -> Result<i64> {
    let sql = db.sql(
        "SELECT COUNT(*) FROM work_view_log
          WHERE work_id = ? AND is_automated = 0",
        "SELECT COUNT(*) FROM work_view_log
          WHERE work_id::text = $1 AND is_automated = 0",
    );
    let count: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(count)
}

/// Complete reads: readers who marked the work finished.
pub async fn count_complete_reads(db: &Database, work_id: &str) -> Result<i64> {
    let sql = db.sql(
        "SELECT COUNT(*) FROM reading_status
          WHERE subject_type = 'work' AND subject_id = ? AND status = 'finished'",
        "SELECT COUNT(*) FROM reading_status
          WHERE subject_type = 'work' AND subject_id::text = $1 AND status = 'finished'",
    );
    let count: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(count)
}

/// Reactions: one per pseud per work (PK enforces it).
pub async fn count_reactions(db: &Database, work_id: &str) -> Result<i64> {
    let sql = db.sql(
        "SELECT COUNT(*) FROM work_reactions WHERE work_id = ?",
        "SELECT COUNT(*) FROM work_reactions WHERE work_id::text = $1",
    );
    let count: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(count)
}

/// Kudos: one per account per work.
pub async fn count_kudos(db: &Database, work_id: &str) -> Result<i64> {
    let sql = db.sql(
        "SELECT COUNT(*) FROM work_kudos WHERE work_id = ?",
        "SELECT COUNT(*) FROM work_kudos WHERE work_id::text = $1",
    );
    let count: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(count)
}

/// Bookmarks: one per account per work (subject_type='work').
pub async fn count_bookmarks(db: &Database, work_id: &str) -> Result<i64> {
    let sql = db.sql(
        "SELECT COUNT(*) FROM bookmarks
          WHERE subject_type = 'work' AND subject_id = ?",
        "SELECT COUNT(*) FROM bookmarks
          WHERE subject_type = 'work' AND subject_id::text = $1",
    );
    let count: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(count)
}

/// Collection adds: how many public collections include this work.
pub async fn count_collection_adds(db: &Database, work_id: &str) -> Result<i64> {
    let sql = db.sql(
        "SELECT COUNT(*) FROM collection_items ci
          JOIN collections c ON c.id = ci.collection_id
          WHERE ci.work_id = ? AND c.is_public = 1",
        "SELECT COUNT(*) FROM collection_items ci
          JOIN collections c ON c.id = ci.collection_id
          WHERE ci.work_id::text = $1 AND c.is_public = true",
    );
    let count: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(count)
}

/// Reviews: public reviews only (spec §9.5).
pub async fn count_reviews(db: &Database, work_id: &str) -> Result<i64> {
    let sql = db.sql(
        "SELECT COUNT(*) FROM review
          WHERE work_id = ? AND is_public = 1 AND deleted_at IS NULL",
        "SELECT COUNT(*) FROM review
          WHERE work_id::text = $1 AND is_public = true AND deleted_at IS NULL",
    );
    let count: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(count)
}

// ---------------------------------------------------------------------------
// Incremental updates
// ---------------------------------------------------------------------------

/// Record a deduplicated view. `viewed_at` is already truncated to the hour
/// by the caller. Returns true if the view was new (and the aggregate should
/// be incremented), false if it was a duplicate within the dedup window.
pub async fn record_view(
    db: &Database,
    work_id: &str,
    viewer_hash: &str,
    viewed_at: &str,
    is_automated: bool,
) -> Result<bool> {
    let sql = db.sql(
        "INSERT INTO work_view_log (work_id, viewer_hash, viewed_at, is_automated)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(work_id, viewer_hash, viewed_at) DO NOTHING",
        "INSERT INTO work_view_log (work_id, viewer_hash, viewed_at, is_automated)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT(work_id, viewer_hash, viewed_at) DO NOTHING",
    );
    let rows = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(work_id)
            .bind(viewer_hash)
            .bind(viewed_at)
            .bind(is_automated)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(work_id)
            .bind(viewer_hash)
            .bind(viewed_at)
            .bind(is_automated)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(rows > 0)
}

/// Increment the materialized hit counter. Creates the aggregate row lazily
/// on first call.
pub async fn increment_views(db: &Database, work_id: &str) -> Result<()> {
    upsert_counter(db, work_id, "views", 1).await
}

/// Toggle kudos for an account on a work. Returns the new state (true = kudoed).
pub async fn toggle_kudos(db: &Database, work_id: &str, account_id: &str) -> Result<bool> {
    let existing = kudo_state(db, work_id, account_id).await?;
    if existing {
        remove_kudos(db, work_id, account_id).await?;
        decrement_counter(db, work_id, "kudos", 1).await?;
        Ok(false)
    } else {
        add_kudos(db, work_id, account_id).await?;
        increment_counter(db, work_id, "kudos", 1).await?;
        Ok(true)
    }
}

async fn kudo_state(db: &Database, work_id: &str, account_id: &str) -> Result<bool> {
    let sql = db.sql(
        "SELECT 1 FROM work_kudos WHERE work_id = ? AND account_id = ?",
        "SELECT 1 FROM work_kudos WHERE work_id::text = $1 AND account_id::text = $2",
    );
    let found: Option<i64> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .bind(account_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .bind(account_id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(found.is_some())
}

async fn add_kudos(db: &Database, work_id: &str, account_id: &str) -> Result<()> {
    let sql = db.sql(
        "INSERT INTO work_kudos (work_id, account_id, created_at) VALUES (?, ?, datetime('now'))
         ON CONFLICT(work_id, account_id) DO NOTHING",
        "INSERT INTO work_kudos (work_id, account_id, created_at) VALUES ($1, $2, now())
         ON CONFLICT(work_id, account_id) DO NOTHING",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(work_id)
                .bind(account_id)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(work_id)
                .bind(account_id)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(())
}

async fn remove_kudos(db: &Database, work_id: &str, account_id: &str) -> Result<()> {
    let sql = db.sql(
        "DELETE FROM work_kudos WHERE work_id = ? AND account_id = ?",
        "DELETE FROM work_kudos WHERE work_id::text = $1 AND account_id::text = $2",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(work_id)
                .bind(account_id)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(work_id)
                .bind(account_id)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(())
}

/// Increment a single counter on the materialized aggregate, creating the row
/// lazily if it does not exist yet.
pub async fn increment_counter(
    db: &Database,
    work_id: &str,
    column: &str,
    delta: i64,
) -> Result<()> {
    upsert_counter(db, work_id, column, delta).await
}

async fn upsert_counter(db: &Database, work_id: &str, column: &str, delta: i64) -> Result<()> {
    // The column name is validated by the caller (always a literal in this
    // module), so interpolating it is safe. Values are bound.
    let now_sql = match db.backend() {
        Backend::Sqlite => "datetime('now')",
        Backend::Postgres => "now()",
    };
    let sql = match db.backend() {
        Backend::Sqlite => format!(
            "INSERT INTO work_metric_aggregates
               (work_id, {column}, updated_at)
             VALUES (?, ?, datetime('now'))
             ON CONFLICT(work_id) DO UPDATE SET
               {column} = {column} + ?,
               updated_at = datetime('now')",
        ),
        Backend::Postgres => format!(
            "INSERT INTO work_metric_aggregates
               (work_id, {column}, updated_at)
             VALUES ($1, $2, now())
             ON CONFLICT(work_id) DO UPDATE SET
               {column} = work_metric_aggregates.{column} + $3,
               updated_at = now()",
        ),
    };
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(work_id)
                .bind(delta)
                .bind(delta)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(work_id)
                .bind(delta)
                .bind(delta)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    let _ = now_sql;
    Ok(())
}

async fn decrement_counter(db: &Database, work_id: &str, column: &str, delta: i64) -> Result<()> {
    upsert_counter(db, work_id, column, -delta).await
}

/// Recompute and overwrite the materialized aggregate for one work. Used by
/// the seed script and as a repair tool.
pub async fn recompute_and_store(db: &Database, work_id: &str) -> Result<WorkMetrics> {
    let live = compute_live(db, work_id).await?;
    let sql = db.sql(
        "INSERT INTO work_metric_aggregates
           (work_id, views, complete_reads, reactions, kudos, bookmarks,
            collection_adds, reviews, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, datetime('now'))
         ON CONFLICT(work_id) DO UPDATE SET
           views = excluded.views,
           complete_reads = excluded.complete_reads,
           reactions = excluded.reactions,
           kudos = excluded.kudos,
           bookmarks = excluded.bookmarks,
           collection_adds = excluded.collection_adds,
           reviews = excluded.reviews,
           updated_at = datetime('now')",
        "INSERT INTO work_metric_aggregates
           (work_id, views, complete_reads, reactions, kudos, bookmarks,
            collection_adds, reviews, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now())
         ON CONFLICT(work_id) DO UPDATE SET
           views = excluded.views,
           complete_reads = excluded.complete_reads,
           reactions = excluded.reactions,
           kudos = excluded.kudos,
           bookmarks = excluded.bookmarks,
           collection_adds = excluded.collection_adds,
           reviews = excluded.reviews,
           updated_at = now()",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&live.work_id)
                .bind(live.views)
                .bind(live.complete_reads)
                .bind(live.reactions)
                .bind(live.kudos)
                .bind(live.bookmarks)
                .bind(live.collection_adds)
                .bind(live.reviews)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&live.work_id)
                .bind(live.views)
                .bind(live.complete_reads)
                .bind(live.reactions)
                .bind(live.kudos)
                .bind(live.bookmarks)
                .bind(live.collection_adds)
                .bind(live.reviews)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(live)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_metrics_are_zero() {
        let m = WorkMetrics::empty("w1");
        assert_eq!(m.views, 0);
        assert_eq!(m.kudos, 0);
        assert_eq!(m.reviews, 0);
    }

    // -----------------------------------------------------------------------
    // Integration tests (SQLite, real migrations)
    // -----------------------------------------------------------------------

    use uuid::Uuid;

    async fn test_db() -> crate::Database {
        let dir = std::env::temp_dir().join(format!(
            "lorehaven-db-metrics-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        let config =
            crate::DatabaseConfig::new(format!("sqlite://{}/test.db?mode=rwc", dir.display()));
        let db = crate::Database::connect(&config).await.expect("connect");
        db.migrate().await.expect("migrate");
        db
    }

    async fn create_test_work(db: &crate::Database) -> String {
        use crate::content::create_work;
        use crate::identity::AccountStatus;
        use crate::identity::{create_account, create_pseud};
        use lorehaven_domain::policy::AgeState;
        let account = create_account(
            db,
            &format!("test-{}@example.test", Uuid::new_v4()),
            AgeState::DeclaredAdult,
            AccountStatus::Active,
        )
        .await
        .expect("create account");
        let handle = format!("user-{}", Uuid::new_v4());
        let pseud = create_pseud(db, account, &handle, "Test User")
            .await
            .expect("create pseud");
        let work = create_work(db, pseud, "Metrics Test Work", None)
            .await
            .expect("create work");
        work.id.to_string()
    }

    async fn create_test_account(db: &crate::Database) -> String {
        use crate::identity::create_account;
        use crate::identity::AccountStatus;
        use lorehaven_domain::policy::AgeState;
        let account = create_account(
            db,
            &format!("test-{}@example.test", Uuid::new_v4()),
            AgeState::DeclaredAdult,
            AccountStatus::Active,
        )
        .await
        .expect("create account");
        account.to_string()
    }

    #[tokio::test]
    async fn views_dedupe_within_the_hour() -> anyhow::Result<()> {
        let db = test_db().await;
        let work_id = create_test_work(&db).await;

        // First view from a reader: new.
        let first = record_view(&db, &work_id, "reader-1", "2026-09-23T10:00:00", false).await?;
        assert!(first, "first view must be new");
        // Same reader, same hour: duplicate.
        let second = record_view(&db, &work_id, "reader-1", "2026-09-23T10:00:00", false).await?;
        assert!(!second, "same reader, same hour must dedupe");
        // Same reader, next hour: new again.
        let third = record_view(&db, &work_id, "reader-1", "2026-09-23T11:00:00", false).await?;
        assert!(third, "same reader, next hour must count");
        // A different reader, same hour: new.
        let fourth = record_view(&db, &work_id, "reader-2", "2026-09-23T10:00:00", false).await?;
        assert!(fourth, "different reader must count");
        // Automated traffic never counts.
        let bot = record_view(&db, &work_id, "bot-1", "2026-09-23T10:00:00", true).await?;
        assert!(bot, "the bot row is recorded (for analytics)");
        assert_eq!(
            count_views(&db, &work_id).await?,
            3,
            "bot excluded from views"
        );
        Ok(())
    }

    #[tokio::test]
    async fn kudos_toggle_roundtrip() -> anyhow::Result<()> {
        let db = test_db().await;
        let work_id = create_test_work(&db).await;
        let account = create_test_account(&db).await;

        assert_eq!(count_kudos(&db, &work_id).await?, 0);
        assert!(
            toggle_kudos(&db, &work_id, &account).await?,
            "first toggle kudoses"
        );
        assert_eq!(count_kudos(&db, &work_id).await?, 1);
        // Idempotent re-add through the raw function.
        assert!(
            !toggle_kudos(&db, &work_id, &account).await?,
            "second toggle removes"
        );
        assert_eq!(count_kudos(&db, &work_id).await?, 0);
        Ok(())
    }

    #[tokio::test]
    async fn counters_increment_and_recompute() -> anyhow::Result<()> {
        let db = test_db().await;
        let work_id = create_test_work(&db).await;

        // Increment views twice through the app path.
        increment_views(&db, &work_id).await?;
        increment_views(&db, &work_id).await?;
        let stored = get_metrics(&db, &work_id.parse().unwrap()).await?;
        assert_eq!(stored.views, 2);

        // Recompute from source: no view log rows, so views drop to live truth.
        let live = recompute_and_store(&db, &work_id).await?;
        assert_eq!(
            live.views, 0,
            "recompute replaces stale counters with live counts"
        );
        let after = get_metrics(&db, &work_id.parse().unwrap()).await?;
        assert_eq!(after.views, 0);
        Ok(())
    }

    #[tokio::test]
    async fn metrics_for_a_fresh_work_are_zero() -> anyhow::Result<()> {
        let db = test_db().await;
        let work_id = create_test_work(&db).await;
        let m = get_metrics(&db, &work_id.parse().unwrap()).await?;
        assert_eq!(m.views, 0);
        assert_eq!(m.complete_reads, 0);
        assert_eq!(m.reactions, 0);
        assert_eq!(m.kudos, 0);
        assert_eq!(m.bookmarks, 0);
        assert_eq!(m.collection_adds, 0);
        assert_eq!(m.reviews, 0);
        Ok(())
    }
}
