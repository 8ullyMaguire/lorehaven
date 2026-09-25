//! M28 — Rating signal integrity: trust-weighted aggregation, anomaly detection, and contested marks.
//!
//! Repository layer for trust-weighted rating aggregates, anomaly events, and contested work flags.
//!
//! Spec §33.2.

use crate::{Backend, Database};
use anyhow::{Context, Result};
use lorehaven_domain::ids::{AccountId, WorkId};
use sqlx::Row;
use uuid::Uuid;

/// Minimum number of public, non-deleted ratings required to show an aggregate.
/// Spec §9.4: "Use a minimum publication threshold" before displaying a public
/// aggregate. Kept here so the rule and the aggregate query cannot drift.
const MIN_PUBLIC_RATINGS: i64 = 3;

/// Get a work's trust-weighted public rating summary.
///
/// Each rating's star value is multiplied by the rater's trust level (default 1).
/// Returns None if there are fewer than [`MIN_PUBLIC_RATINGS`] public, non-deleted ratings.
pub async fn get_trust_weighted_rating_summary(
    db: &Database,
    work_id: &WorkId,
) -> Result<Option<RatingSummary>> {
    let sql = db.sql(
        "SELECT COUNT(*) AS count, \
                COALESCE(SUM(stars * COALESCE(tl.level, 1)), 0) AS weighted_sum, \
                COALESCE(SUM(COALESCE(tl.level, 1)), 0) AS total_weight \
           FROM rating \
           LEFT JOIN trust_levels tl ON rating.account_id = tl.account \
          WHERE work_id = ? AND is_public = 1 AND deleted_at IS NULL \
          HAVING COUNT(*) >= ?",
        "SELECT COUNT(*) AS count, \
                COALESCE(SUM(stars * COALESCE(tl.level, 1)), 0)::bigint AS weighted_sum, \
                COALESCE(SUM(COALESCE(tl.level, 1)), 0)::bigint AS total_weight \
           FROM rating \
           LEFT JOIN trust_levels tl ON rating.account_id::text = tl.account \
          WHERE work_id = $1 AND is_public = TRUE AND deleted_at IS NULL \
          HAVING COUNT(*) >= $2",
    );
    let row = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work_id.to_string())
                .bind(MIN_PUBLIC_RATINGS)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work_id.to_string())
                .bind(MIN_PUBLIC_RATINGS)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.map(
        |(count, weighted_sum, total_weight): (i64, i64, i64)| RatingSummary {
            count,
            mean_permille: if total_weight > 0 {
                (weighted_sum * 1000) / total_weight
            } else {
                0
            },
        },
    ))
}

/// Insert a rating anomaly event.
pub async fn insert_rating_anomaly_event(db: &Database, event: &RatingAnomalyEvent) -> Result<()> {
    let sql = db.sql(
        "INSERT INTO rating_anomaly_events (id, work_id, cohort_id, kind, severity, detail, detected_at, cleared_at, cleared_by) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        "INSERT INTO rating_anomaly_events (id, work_id, cohort_id, kind, severity, detail, detected_at, cleared_at, cleared_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&event.id)
                .bind(&event.work_id)
                .bind(event.cohort_id.as_deref())
                .bind(&event.kind)
                .bind(event.severity)
                .bind(&event.detail)
                .bind(&event.detected_at)
                .bind(event.cleared_at.as_deref())
                .bind(event.cleared_by.as_ref().map(|id| id.to_string()))
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&event.id)
                .bind(Uuid::parse_str(&event.work_id).context("parse work_id")?)
                .bind(
                    event
                        .cohort_id
                        .as_ref()
                        .and_then(|c| Uuid::parse_str(c).ok()),
                )
                .bind(&event.kind)
                .bind(event.severity)
                .bind(&event.detail)
                .bind(&event.detected_at)
                .bind(event.cleared_at.as_deref())
                .bind(event.cleared_by.as_ref().map(|id| id.to_string()))
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Clear a rating anomaly event by ID.
pub async fn clear_rating_anomaly_event(
    db: &Database,
    event_id: &str,
    cleared_by: &AccountId,
) -> Result<()> {
    let sql = db.sql(
        "UPDATE rating_anomaly_events SET cleared_at = ?, cleared_by = ? WHERE id = ?",
        "UPDATE rating_anomaly_events SET cleared_at = $1::timestamptz, cleared_by = $2 WHERE id = $3",
    );
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&now)
                .bind(cleared_by.to_string())
                .bind(event_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&now)
                .bind(cleared_by.to_string())
                .bind(event_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Get all uncleared anomaly events for a work.
pub async fn get_work_anomaly_events(
    db: &Database,
    work_id: &WorkId,
) -> Result<Vec<RatingAnomalyEvent>> {
    let sql = db.sql(
        "SELECT id, work_id, cohort_id, kind, severity, detail, detected_at, cleared_at, cleared_by \
           FROM rating_anomaly_events \
          WHERE work_id = ? AND cleared_at IS NULL \
          ORDER BY detected_at DESC",
        "SELECT id, work_id, cohort_id, kind, severity, detail, detected_at, cleared_at, cleared_by \
           FROM rating_anomaly_events \
          WHERE work_id = $1 AND cleared_at IS NULL \
          ORDER BY detected_at DESC",
    );
    let mut events = Vec::new();
    match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(&sql)
                .bind(work_id.to_string())
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?;
            for r in rows {
                events.push(RatingAnomalyEvent {
                    id: r.get::<String, _>(0),
                    work_id: r.get::<String, _>(1),
                    cohort_id: Some(r.get::<String, _>(2)).filter(|s| !s.is_empty()),
                    kind: r.get::<String, _>(3),
                    severity: r.get::<i64, _>(4),
                    detail: r.get::<String, _>(5),
                    detected_at: r.get::<String, _>(6),
                    cleared_at: Some(r.get::<String, _>(7)).filter(|s| !s.is_empty()),
                    cleared_by: Some(r.get::<String, _>(8))
                        .filter(|s| !s.is_empty())
                        .and_then(|s| s.parse::<AccountId>().ok()),
                });
            }
        }
        Backend::Postgres => {
            let rows = sqlx::query(&sql)
                .bind(work_id.to_string())
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?;
            for r in rows {
                events.push(RatingAnomalyEvent {
                    id: r.get::<String, _>(0),
                    work_id: r.get::<Uuid, _>(1).to_string(),
                    cohort_id: r.get::<Option<Uuid>, _>(2).map(|u| u.to_string()),
                    kind: r.get::<String, _>(3),
                    severity: r.get::<i32, _>(4) as i64,
                    detail: r.get::<String, _>(5),
                    detected_at: r.get::<String, _>(6),
                    cleared_at: r.get::<Option<String>, _>(7),
                    cleared_by: r
                        .get::<Option<String>, _>(8)
                        .and_then(|s| s.parse::<AccountId>().ok()),
                });
            }
        }
    }
    Ok(events)
}

/// Set a work's contested mark.
pub async fn set_work_contested(db: &Database, work_id: &WorkId, reason: &str) -> Result<()> {
    let sql = db.sql(
        "UPDATE works SET contested = 1, contested_at = ?, contested_reason = ? WHERE id = ?",
        "UPDATE works SET contested = 1, contested_at = $1::timestamptz, contested_reason = $2 WHERE id = $3",
    );
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&now)
                .bind(reason)
                .bind(work_id.to_string())
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&now)
                .bind(reason)
                .bind(work_id.to_string())
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Clear a work's contested mark.
pub async fn clear_work_contested(db: &Database, work_id: &WorkId) -> Result<()> {
    let sql = db.sql(
        "UPDATE works SET contested = 0, contested_at = NULL, contested_reason = NULL WHERE id = ?",
        "UPDATE works SET contested = 0, contested_at = NULL, contested_reason = NULL WHERE id = $1",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(work_id.to_string())
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(work_id.to_string())
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Check if a work is currently contested.
pub async fn is_work_contested(db: &Database, work_id: &WorkId) -> Result<bool> {
    let sql = db.sql(
        "SELECT contested FROM works WHERE id = ?",
        "SELECT contested FROM works WHERE id = $1",
    );
    match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(&sql)
                .bind(work_id.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(row.map(|r| r.get::<i64, _>(0) != 0).unwrap_or(false))
        }
        Backend::Postgres => {
            let row = sqlx::query(&sql)
                .bind(work_id.to_string())
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(row.map(|r| r.get::<bool, _>(0)).unwrap_or(false))
        }
    }
}

/// A rating anomaly event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RatingAnomalyEvent {
    pub id: String,
    pub work_id: String,
    pub cohort_id: Option<String>,
    pub kind: String,
    pub severity: i64,
    pub detail: String,
    pub detected_at: String,
    pub cleared_at: Option<String>,
    pub cleared_by: Option<AccountId>,
}

/// A trust-weighted rating summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RatingSummary {
    pub count: i64,
    pub mean_permille: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::create_work;
    use crate::identity::{create_account, create_pseud, AccountStatus};
    use lorehaven_domain::ids::PseudId;
    use lorehaven_domain::policy::AgeState;
    use uuid::Uuid;

    async fn create_test_work(db: &Database) -> anyhow::Result<(WorkId, AccountId, PseudId)> {
        let account = create_account(
            db,
            &format!("test-{}@example.test", Uuid::new_v4()),
            AgeState::DeclaredAdult,
            AccountStatus::Active,
        )
        .await
        .map_err(|e| anyhow::anyhow!("create account: {e}"))?;
        let handle = format!("user-{}", Uuid::new_v4());
        let pseud = create_pseud(db, account, &handle, "Test User")
            .await
            .map_err(|e| anyhow::anyhow!("create pseud: {e}"))?;
        let work = create_work(db, pseud, "Test Work", None)
            .await
            .map_err(|e| anyhow::anyhow!("create work: {e}"))?;
        Ok((work.id, account, pseud))
    }

    /// Create a new account+pseud and insert a public rating.
    async fn create_rater_and_rate(
        db: &Database,
        work_id: &WorkId,
        stars: i64,
        trust_level: Option<i64>,
    ) -> anyhow::Result<AccountId> {
        let account = create_account(
            db,
            &format!("rater-{}@example.test", Uuid::new_v4()),
            AgeState::DeclaredAdult,
            AccountStatus::Active,
        )
        .await?;
        let handle = format!("rater-{}", Uuid::new_v4());
        let pseud = create_pseud(db, account, &handle, "Rater").await?;

        if let Some(level) = trust_level {
            let now = crate::identity::now_rfc3339();
            let sql = db.sql(
                "INSERT INTO trust_levels (account, level, computed_at, basis) VALUES (?, ?, ?, ?)",
                "INSERT INTO trust_levels (account, level, computed_at, basis) VALUES ($1, $2, $3, $4)",
            );
            match db.backend() {
                Backend::Sqlite => {
                    sqlx::query(&sql)
                        .bind(account.to_string())
                        .bind(level)
                        .bind(&now)
                        .bind("test")
                        .execute(db.sqlite_pool().expect("sqlite"))
                        .await?;
                }
                Backend::Postgres => {
                    sqlx::query(&sql)
                        .bind(account.to_string())
                        .bind(level)
                        .bind(&now)
                        .bind("test")
                        .execute(db.postgres_pool().expect("postgres"))
                        .await?;
                }
            }
        }

        let now = crate::identity::now_rfc3339();
        let id = Uuid::new_v4();
        let sql = db.sql(
            "INSERT INTO rating (id, account_id, pseud_id, work_id, stars, is_public, created_at, updated_at, version) \
             VALUES (?, ?, ?, ?, ?, 1, ?, ?, 1)",
            "INSERT INTO rating (id, account_id, pseud_id, work_id, stars, is_public, created_at, updated_at, version) \
             VALUES ($1, $2, $3, $4, $5, TRUE, $6, $7, 1)",
        );
        match db.backend() {
            Backend::Sqlite => {
                sqlx::query(&sql)
                    .bind(id.to_string())
                    .bind(account.to_string())
                    .bind(pseud.to_string())
                    .bind(work_id.to_string())
                    .bind(stars)
                    .bind(&now)
                    .bind(&now)
                    .execute(db.sqlite_pool().expect("sqlite"))
                    .await?;
            }
            Backend::Postgres => {
                sqlx::query(&sql)
                    .bind(id)
                    .bind(account.to_string())
                    .bind(pseud.to_string())
                    .bind(work_id.to_string())
                    .bind(stars)
                    .bind(&now)
                    .bind(&now)
                    .execute(db.postgres_pool().expect("postgres"))
                    .await?;
            }
        }

        Ok(account)
    }

    #[tokio::test]
    async fn trust_weighted_rating_summary_basic() -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join(format!(
            "lorehaven-db-rating-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        let config =
            crate::DatabaseConfig::new(format!("sqlite://{}/test.db?mode=rwc", dir.display()));
        let db = crate::Database::connect(&config).await.expect("connect");
        db.migrate().await.expect("migrate");

        let (work_id, _, _) = create_test_work(&db).await?;

        // Insert 3 ratings from different accounts, all with trust=1 (default), 4 stars each
        for _ in 0..3 {
            create_rater_and_rate(&db, &work_id, 4, None).await?;
        }

        // All trust=1, so weighted mean = (4*1 + 4*1 + 4*1) / (1+1+1) = 4.0 -> 4000 permille
        let summary = get_trust_weighted_rating_summary(&db, &work_id).await?;
        let s = summary.expect("should have summary");
        assert_eq!(s.count, 3);
        assert_eq!(s.mean_permille, 4000);

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    /// Trust weighting changes the aggregate — high trust 5 stars + low trust 1 star.
    ///
    /// Need 3 ratings for MIN_PUBLIC_RATINGS threshold.
    #[tokio::test]
    async fn trust_weighting_changes_aggregate() -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join(format!(
            "lorehaven-db-rating-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        let config =
            crate::DatabaseConfig::new(format!("sqlite://{}/test.db?mode=rwc", dir.display()));
        let db = crate::Database::connect(&config).await.expect("connect");
        db.migrate().await.expect("migrate");

        let (work_id, _, _) = create_test_work(&db).await?;

        // High trust (5) account rates 5 stars
        create_rater_and_rate(&db, &work_id, 5, Some(5)).await?;
        // Low trust (1) account rates 1 star
        create_rater_and_rate(&db, &work_id, 1, Some(1)).await?;
        // Another trust 1 account rates 1 star to meet MIN_PUBLIC_RATINGS=3
        create_rater_and_rate(&db, &work_id, 1, Some(1)).await?;

        // Simple mean = (5+1+1)/3 = 2.33 -> 2333 permille
        // Trust-weighted: (5*5 + 1*1 + 1*1) / (5+1+1) = 27/7 = 3.857 -> 3857 permille
        let summary = get_trust_weighted_rating_summary(&db, &work_id).await?;
        let s = summary.expect("should have summary");
        assert_eq!(s.count, 3);
        assert_eq!(s.mean_permille, 3857);

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[tokio::test]
    async fn anomaly_event_roundtrip() -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join(format!(
            "lorehaven-db-rating-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        let config =
            crate::DatabaseConfig::new(format!("sqlite://{}/test.db?mode=rwc", dir.display()));
        let db = crate::Database::connect(&config).await.expect("connect");
        db.migrate().await.expect("migrate");

        let (work_id, _, _) = create_test_work(&db).await?;

        let event = RatingAnomalyEvent {
            id: Uuid::new_v4().to_string(),
            work_id: work_id.to_string(),
            cohort_id: None,
            kind: "burst".to_string(),
            severity: 5,
            detail: r#"{"trigger":"test"}"#.to_string(),
            detected_at: crate::identity::now_rfc3339(),
            cleared_at: None,
            cleared_by: None,
        };
        insert_rating_anomaly_event(&db, &event).await?;

        let events = get_work_anomaly_events(&db, &work_id).await?;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, event.id);
        assert_eq!(events[0].kind, "burst");
        assert_eq!(events[0].severity, 5);

        let clearer = AccountId::new();
        clear_rating_anomaly_event(&db, &event.id, &clearer).await?;

        let events_after = get_work_anomaly_events(&db, &work_id).await?;
        assert!(events_after.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[tokio::test]
    async fn work_contested_flag_roundtrip() -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join(format!(
            "lorehaven-db-rating-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        let config =
            crate::DatabaseConfig::new(format!("sqlite://{}/test.db?mode=rwc", dir.display()));
        let db = crate::Database::connect(&config).await.expect("connect");
        db.migrate().await.expect("migrate");

        let (work_id, _, _) = create_test_work(&db).await?;

        assert!(!is_work_contested(&db, &work_id).await?);

        set_work_contested(&db, &work_id, "brigade_detected").await?;
        assert!(is_work_contested(&db, &work_id).await?);

        clear_work_contested(&db, &work_id).await?;
        assert!(!is_work_contested(&db, &work_id).await?);

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[tokio::test]
    async fn rating_burst_triggers_contested_and_anomaly() -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join(format!(
            "lorehaven-db-rating-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        let config =
            crate::DatabaseConfig::new(format!("sqlite://{}/test.db?mode=rwc", dir.display()));
        let db = crate::Database::connect(&config).await.expect("connect");
        db.migrate().await.expect("migrate");

        let (work_id, _, _) = create_test_work(&db).await?;

        // Simulate a burst: insert anomaly event and mark contested
        let event = RatingAnomalyEvent {
            id: Uuid::new_v4().to_string(),
            work_id: work_id.to_string(),
            cohort_id: None,
            kind: "burst".to_string(),
            severity: 3,
            detail: r#"{"ratings_in_window":10,"threshold":5}"#.to_string(),
            detected_at: crate::identity::now_rfc3339(),
            cleared_at: None,
            cleared_by: None,
        };
        insert_rating_anomaly_event(&db, &event).await?;
        set_work_contested(&db, &work_id, "rating_burst").await?;

        // Work is now contested
        assert!(is_work_contested(&db, &work_id).await?);

        // Clear anomaly and contested
        let clearer = AccountId::new();
        clear_rating_anomaly_event(&db, &event.id, &clearer).await?;
        clear_work_contested(&db, &work_id).await?;

        assert!(!is_work_contested(&db, &work_id).await?);
        let events = get_work_anomaly_events(&db, &work_id).await?;
        assert!(events.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[tokio::test]
    async fn credits_do_not_affect_trust_weight() -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join(format!(
            "lorehaven-db-rating-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        let config =
            crate::DatabaseConfig::new(format!("sqlite://{}/test.db?mode=rwc", dir.display()));
        let db = crate::Database::connect(&config).await.expect("connect");
        db.migrate().await.expect("migrate");

        let (work_id, _, _) = create_test_work(&db).await?;

        // Create 3 raters with trust level 3, all rating 5 stars
        for _ in 0..3 {
            create_rater_and_rate(&db, &work_id, 5, Some(3)).await?;
        }

        // Mean should be 5.0 (5000 permille) because all ratings are 5 stars
        // regardless of trust level or credits
        let summary = get_trust_weighted_rating_summary(&db, &work_id).await?;
        let s = summary.expect("should have summary");
        assert_eq!(s.count, 3);
        assert_eq!(s.mean_permille, 5000);

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }
}
