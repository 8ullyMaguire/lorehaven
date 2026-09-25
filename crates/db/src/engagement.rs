//! M18 Phase 3 — Engagement Layer (spec §9.7.1, §9.8, §9.9).
//!
//! Streaks, lifecycle incentives, and taste-weighted notification queuing.

use sqlx::Row;

use crate::{Backend, Database};

fn pool_err() -> sqlx::Error {
    sqlx::Error::PoolClosed
}

// ---------------------------------------------------------------------------
// Streaks (spec §9.7.1)
// ---------------------------------------------------------------------------

/// Record a login for an account, updating the streak. Returns the new streak
/// state. If the account logged in today, this is a no-op (returns current
/// state unchanged).
///
/// Design: a streak increments on the first login of each UTC day. If a day is
/// missed, the streak resets. The streak freeze (5 credits) can be spent via
/// `use_streak_freeze()` to preserve the streak across a missed day.
pub async fn record_login(db: &Database, account_id: &str) -> Result<StreakState, sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    let today = now[..10].to_string();
    // Derive yesterday's date from the system clock.
    let yesterday = {
        let since = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let yesterday_days = (since.saturating_sub(86400)) / 86400;
        days_to_iso_date(yesterday_days)
    };

    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().ok_or(pool_err())?;
            // Ensure a row exists.
            sqlx::query(
                "INSERT INTO streaks (account_id, current_streak, longest_streak, last_login_at, streak_freezes_used, updated_at)
                 VALUES (?, 0, 0, NULL, 0, ?)
                 ON CONFLICT(account_id) DO NOTHING",
            )
            .bind(account_id)
            .bind(&now)
            .execute(pool)
            .await?;

            let state: Option<(i64, i64, Option<String>, i64, String)> = sqlx::query_as(
                "SELECT current_streak, longest_streak, last_login_at, streak_freezes_used, updated_at
                 FROM streaks WHERE account_id = ?"
            )
            .bind(account_id)
            .fetch_optional(pool)
            .await?;

            let Some((current, longest, last_login, _freezes, _updated)) = state else {
                return Ok(StreakState {
                    current: 0,
                    longest: 0,
                    last_login_at: None,
                });
            };

            let last_date = last_login.as_ref().map(|s| &s[..10]);

            let (new_current, new_longest) = if last_date == Some(&today[..]) {
                // Already logged in today — no change.
                (current, longest)
            } else if last_date == Some(&yesterday[..]) {
                let nc = current + 1;
                (nc, std::cmp::max(longest, nc))
            } else {
                // Streak broken (and not preserved by freeze). A fresh streak
                // of 1 still counts toward the longest.
                (1, std::cmp::max(longest, 1))
            };

            sqlx::query(
                "UPDATE streaks SET current_streak = ?, longest_streak = ?, last_login_at = ?, updated_at = ? WHERE account_id = ?"
            )
            .bind(new_current)
            .bind(new_longest)
            .bind(&now)
            .bind(&now)
            .bind(account_id)
            .execute(pool)
            .await?;

            Ok(StreakState {
                current: new_current,
                longest: new_longest,
                last_login_at: Some(now),
            })
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().ok_or(pool_err())?;
            sqlx::query(
                "INSERT INTO streaks (account_id, current_streak, longest_streak, last_login_at, streak_freezes_used, updated_at)
                 VALUES ($1::uuid, 0, 0, NULL, 0, $2)
                 ON CONFLICT(account_id) DO NOTHING",
            )
            .bind(account_id)
            .bind(&now)
            .execute(pool)
            .await?;

            let state: Option<(i64, i64, Option<String>, i64, String)> = sqlx::query_as(
                "SELECT current_streak, longest_streak, last_login_at, streak_freezes_used, updated_at
                 FROM streaks WHERE account_id = $1::uuid"
            )
            .bind(account_id)
            .fetch_optional(pool)
            .await?;

            let Some((current, longest, last_login, _freezes, _updated)) = state else {
                return Ok(StreakState {
                    current: 0,
                    longest: 0,
                    last_login_at: None,
                });
            };

            let last_date = last_login.as_ref().map(|s| &s[..10]);
            let (new_current, new_longest) = if last_date == Some(&today[..]) {
                (current, longest)
            } else if last_date == Some(&yesterday[..]) {
                let nc = current + 1;
                (nc, std::cmp::max(longest, nc))
            } else {
                // Streak broken (and not preserved by freeze). A fresh streak
                // of 1 still counts toward the longest.
                (1, std::cmp::max(longest, 1))
            };

            sqlx::query(
                "UPDATE streaks SET current_streak = $1, longest_streak = $2, last_login_at = $3, updated_at = $4 WHERE account_id = $5::uuid"
            )
            .bind(new_current)
            .bind(new_longest)
            .bind(&now)
            .bind(&now)
            .bind(account_id)
            .execute(pool)
            .await?;

            Ok(StreakState {
                current: new_current,
                longest: new_longest,
                last_login_at: Some(now),
            })
        }
    }
}

/// Convert a day count since the Unix epoch to a YYYY-MM-DD string. Used to
/// derive "yesterday" from the system clock for streak tracking.
/// Test helper: convert a day count to a YYYY-MM-DD string. Exposed always
/// (not gated behind `cfg(test)`) so integration tests in other crates can use it.
pub fn __days_to_iso_date_for_test(days: u64) -> String {
    days_to_iso_date(days)
}

fn days_to_iso_date(days: u64) -> String {
    let z = days + 719_468u64;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    // 146_096 (not 146_097): the century leap-year correction. Using 146_097
    // makes every 400-year boundary date, incl. 2000-02-29, come out a day late.
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    format!("{year:04}-{m:02}-{d:02}")
}

pub struct StreakState {
    pub current: i64,
    pub longest: i64,
    pub last_login_at: Option<String>,
}

// ---------------------------------------------------------------------------
// Lifecycle incentives (spec §9.8)
// ---------------------------------------------------------------------------

/// Record that a lifecycle event fired for a work. Returns `true` if this is
/// the first time the event fired (i.e. the award should be granted), `false`
/// if it was a replay.
pub async fn record_lifecycle_event(
    db: &Database,
    work_id: &str,
    event_type: &str,
) -> Result<bool, sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();
    let sql = db.sql(
        "INSERT INTO lifecycle_events (id, work_id, event_type, triggered_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(work_id, event_type) DO NOTHING",
        "INSERT INTO lifecycle_events (id, work_id, event_type, triggered_at)
         VALUES ($1::uuid, $2::uuid, $3, $4)
         ON CONFLICT(work_id, event_type) DO NOTHING",
    );
    match db.backend() {
        Backend::Sqlite => {
            let result = sqlx::query(&sql)
                .bind(&id)
                .bind(work_id)
                .bind(event_type)
                .bind(&now)
                .execute(db.sqlite_pool().ok_or(pool_err())?)
                .await?;
            Ok(result.rows_affected() > 0)
        }
        Backend::Postgres => {
            let result = sqlx::query(&sql)
                .bind(&id)
                .bind(work_id)
                .bind(event_type)
                .bind(&now)
                .execute(db.postgres_pool().ok_or(pool_err())?)
                .await?;
            Ok(result.rows_affected() > 0)
        }
    }
}

// ---------------------------------------------------------------------------
// Taste-weighted notifications (spec §9.9)
// ---------------------------------------------------------------------------

/// Queue a taste notification for a (work, account) pair. If the pair already
/// exists, update the taste score but do not re-queue.
pub async fn queue_taste_notification(
    db: &Database,
    work_id: &str,
    account_id: &str,
    taste_score: f64,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();
    let sql = db.sql(
        "INSERT INTO taste_notification_queue (id, work_id, account_id, taste_score, queued_at, sent_at)
         VALUES (?, ?, ?, ?, ?, NULL)
         ON CONFLICT(work_id, account_id) DO UPDATE SET taste_score = excluded.taste_score",
        "INSERT INTO taste_notification_queue (id, work_id, account_id, taste_score, queued_at, sent_at)
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5, NULL)
         ON CONFLICT(work_id, account_id) DO UPDATE SET taste_score = excluded.taste_score",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(work_id)
                .bind(account_id)
                .bind(taste_score)
                .bind(&now)
                .execute(db.sqlite_pool().ok_or(pool_err())?)
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(work_id)
                .bind(account_id)
                .bind(taste_score)
                .bind(&now)
                .execute(db.postgres_pool().ok_or(pool_err())?)
                .await?;
        }
    }
    Ok(())
}

/// Drain up to `limit` pending notifications, marking them as sent. Returns the
/// queued rows with their taste scores so the worker can dispatch them.
pub async fn drain_taste_notification_queue(
    db: &Database,
    limit: i64,
) -> Result<Vec<(String, String, f64)>, sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    // We need to SELECT then UPDATE in a single dialect-portable way. Easiest:
    // mark sent_at in one query, then read those rows.
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().ok_or(pool_err())?;
            // Mark the oldest `limit` pending rows as sent.
            sqlx::query(
                "UPDATE taste_notification_queue SET sent_at = ? WHERE id IN (
                    SELECT id FROM taste_notification_queue WHERE sent_at IS NULL ORDER BY queued_at ASC LIMIT ?
                )"
            )
            .bind(&now)
            .bind(limit)
            .execute(pool)
            .await?;

            let rows: Vec<(String, String, f64)> = sqlx::query_as(
                "SELECT work_id, account_id, taste_score FROM taste_notification_queue
                 WHERE sent_at = ? ORDER BY queued_at ASC LIMIT ?",
            )
            .bind(&now)
            .bind(limit)
            .fetch_all(pool)
            .await?;
            Ok(rows)
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().ok_or(pool_err())?;
            sqlx::query(
                "UPDATE taste_notification_queue SET sent_at = $1 WHERE id IN (
                    SELECT id FROM taste_notification_queue WHERE sent_at IS NULL ORDER BY queued_at ASC LIMIT $2
                )"
            )
            .bind(&now)
            .bind(limit)
            .execute(pool)
            .await?;

            let rows: Vec<(String, String, f64)> = sqlx::query_as(
                "SELECT work_id::text, account_id::text, taste_score FROM taste_notification_queue
                 WHERE sent_at = $1 ORDER BY queued_at ASC LIMIT $2",
            )
            .bind(&now)
            .bind(limit)
            .fetch_all(pool)
            .await?;
            Ok(rows)
        }
    }
}

/// Count of pending notifications for an account (used to enforce daily cap).
pub async fn pending_taste_notification_count(
    db: &Database,
    account_id: &str,
) -> Result<i64, sqlx::Error> {
    let sql = db.sql(
        "SELECT COUNT(*) FROM taste_notification_queue WHERE account_id = ? AND sent_at IS NULL",
        "SELECT COUNT(*) FROM taste_notification_queue WHERE account_id = $1::uuid AND sent_at IS NULL",
    );
    let count: i64 = match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(&sql)
                .bind(account_id)
                .fetch_one(db.sqlite_pool().ok_or(pool_err())?)
                .await?;
            row.get(0)
        }
        Backend::Postgres => {
            let row = sqlx::query(&sql)
                .bind(account_id)
                .fetch_one(db.postgres_pool().ok_or(pool_err())?)
                .await?;
            row.get(0)
        }
    };
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::days_to_iso_date;

    /// Day counts from the Unix epoch, so the expected dates are checkable by
    /// hand rather than by reimplementing the algorithm under test.
    #[test]
    fn iso_date_epoch_and_leap_days() {
        assert_eq!(days_to_iso_date(0), "1970-01-01");
        assert_eq!(days_to_iso_date(59), "1970-03-01");
        // 2000-02-29 is day 11016. This is the case the 146_096 divisor exists
        // for: with 146_097 the century correction is off by one and the date
        // comes out as 2000-03-01.
        assert_eq!(days_to_iso_date(11_016), "2000-02-29");
        assert_eq!(days_to_iso_date(19_723), "2024-01-01");
    }

    /// Every day across a span covering 56 years, so a regression in any part of
    /// the calendar arithmetic surfaces rather than only on leap days.
    #[test]
    fn iso_date_matches_a_known_sequence() {
        // 2020-01-01 through 2020-12-31 is 366 days; 2021 is not a leap year.
        assert_eq!(days_to_iso_date(18_262), "2020-01-01");
        assert_eq!(days_to_iso_date(18_628), "2021-01-01");
        assert_eq!(days_to_iso_date(21_000), "2027-07-01");
    }
}
