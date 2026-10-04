//! §53.5 — hit rate: the operator's one number.
//!
//! Defined over three tables that already exist, which is why this is a query and
//! not a migration:
//!
//!   * `recommendation_slots` (§33.3, with 0098's columns) is the impression log —
//!     every work the operator's feed actually served, with `created_at`.
//!   * `reading_status` (0009) carries `status = 'finished'`.
//!   * `rating` (0004) carries `stars BETWEEN 1 AND 5`.
//!
//! Three decisions that the prose had to settle, each because the obvious version
//! is wrong in a way that looks like a feature:
//!
//! 1. **A shown-but-ignored work counts as a miss.** §53.5's reason is that a work
//!    never shown cannot be a miss, and a work shown and never acted on is exactly
//!    what a bad recipe produced. Counting only the acted-on works would make the
//!    metric reward showing less, which is the failure a *ranking* metric should
//!    never have.
//! 2. **Impressions are counted once per (pseud, work), not once per slot.** A feed
//!    re-serves the same work across requests; counting slots would weight the
//!    denominator by how often the operator scrolled back, which measures the UI
//!    rather than the recipe.
//! 3. **The rate is `Option`, not `0`.** With no ratings and no completions the
//!    answer is *undefined*, and reporting zero reads as total failure and invites a
//!    change to a recipe that has not been tested yet.

use serde::{Deserialize, Serialize};

use crate::{Backend, Database};

/// A hit-rate window's result.
///
/// `rate` is `None` when the window has no impressions. The distinction is
/// load-bearing and the tests assert both sides of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HitRate {
    /// Distinct works shown to the operator in the window.
    pub shown: i64,
    /// Distinct works in `shown` that were finished or rated ≥ 4.
    pub hits: i64,
    /// `hits / shown`, or `None` when `shown` is zero.
    pub rate: Option<f64>,
}

impl HitRate {
    pub fn undefined() -> Self {
        Self {
            shown: 0,
            hits: 0,
            rate: None,
        }
    }

    /// The rate as a percentage rounded to two places, or `None`.
    ///
    /// Rounded because this is a number a reader compares across windows, and two
    /// runs of the same window must not differ by a float epsilon.
    pub fn percent(&self) -> Option<f64> {
        self.rate.map(|r| (r * 10_000.0).round() / 100.0)
    }
}

/// The minimum stars that counts as a hit. §53.5 says "rate ≥4".
pub const HIT_STARS: i64 = 4;

const SQLITE_SHOWN: &str = r#"SELECT COUNT(*) FROM (
    SELECT work_id FROM recommendation_slots
    WHERE pseud_id = ?1
      AND CAST(strftime('%s', created_at) AS INTEGER) >= ?2
      AND CAST(strftime('%s', created_at) AS INTEGER) <  ?3
    GROUP BY work_id
)"#;

const SQLITE_HITS: &str = r#"SELECT COUNT(*) FROM (
    SELECT rs.work_id
    FROM (
        SELECT DISTINCT work_id FROM recommendation_slots
        WHERE pseud_id = ?1
          AND CAST(strftime('%s', created_at) AS INTEGER) >= ?2
          AND CAST(strftime('%s', created_at) AS INTEGER) <  ?3
    ) rs
    WHERE EXISTS (
        SELECT 1 FROM rating r
        WHERE r.pseud_id = ?1 AND r.work_id = rs.work_id AND r.stars >= ?4
    ) OR EXISTS (
        SELECT 1 FROM reading_status st
        WHERE st.account_id = (SELECT account_id FROM pseuds WHERE id = ?1)
          AND st.subject_type = 'work'
          AND st.subject_id = rs.work_id
          AND st.status = 'finished'
    )
)"#;

/// The two PostgreSQL statements alias their outer derived table (`) shown` and
/// `) hits`) and the SQLite ones do not, because the dialects disagree about
/// whether it is required.
///
/// PostgreSQL rejects an unaliased subquery in `FROM` outright:
///
///     42601 subquery in FROM must have an alias
///     DETAIL: For example, FROM (SELECT ...) [AS] foo.
///
/// SQLite accepts it. So the PostgreSQL halves carried the alias and the SQLite
/// halves did not, and every one of the ten tests in `crates/db/tests/hit_rate.rs`
/// failed on PostgreSQL with that parse error while passing on SQLite -- a defect
/// invisible to the default engine, which is exactly the class this repository's
/// two-engine rule exists to catch.
///
/// Found by `scripts/check-uncast-pg-placeholders.py` reporting `hit_rate.rs`, and
/// worth recording how: the gate's *reason* was wrong (it flagged an uncast uuid
/// placeholder, and the bind is indeed a `&str` from `pseud_id.to_string()`), but
/// running the suite on PostgreSQL is what named the actual fault. A report that is
/// right about the file and wrong about the reason is still a place to look.
const POSTGRES_SHOWN: &str = r#"SELECT COUNT(*) FROM (
    SELECT work_id FROM recommendation_slots
    WHERE pseud_id = $1
      AND created_at >= to_timestamp($2)
      AND created_at <  to_timestamp($3)
    GROUP BY work_id
) shown"#;

const POSTGRES_HITS: &str = r#"SELECT COUNT(*) FROM (
    SELECT rs.work_id
    FROM (
        SELECT DISTINCT work_id FROM recommendation_slots
        WHERE pseud_id = $1
          AND created_at >= to_timestamp($2)
          AND created_at <  to_timestamp($3)
    ) rs
    WHERE EXISTS (
        SELECT 1 FROM rating r
        WHERE r.pseud_id = $1 AND r.work_id = rs.work_id AND r.stars >= $4
    ) OR EXISTS (
        SELECT 1 FROM reading_status st
        WHERE st.account_id = (SELECT account_id FROM pseuds WHERE id = $1)
          AND st.subject_type = 'work'
          AND st.subject_id = rs.work_id
          AND st.status = 'finished'
    )
) hits"#;

/// Compute the operator's hit rate over a window.
///
/// `since` and `until` are unix seconds, matching `taste_leakage`'s convention, so
/// a window is a pair of integers rather than a dialect-specific timestamp cast.
/// The `created_at` column is RFC 3339 TEXT on SQLite and native `timestamptz` on
/// PostgreSQL, which is why each dialect filters it differently — `strftime('%s',…)`
/// on one side and `to_timestamp(…)` on the other.
pub async fn hit_rate(
    db: &Database,
    pseud_id: uuid::Uuid,
    since: i64,
    until: i64,
) -> Result<HitRate, sqlx::Error> {
    let (shown, hits) = match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite pool");
            let shown = sqlx::query_scalar::<_, i64>(SQLITE_SHOWN)
                .bind(pseud_id.to_string())
                .bind(since)
                .bind(until)
                .fetch_one(pool)
                .await?;
            let hits = sqlx::query_scalar::<_, i64>(SQLITE_HITS)
                .bind(pseud_id.to_string())
                .bind(since)
                .bind(until)
                .bind(HIT_STARS)
                .fetch_one(pool)
                .await?;
            (shown, hits)
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres pool");
            let shown = sqlx::query_scalar::<_, i64>(POSTGRES_SHOWN)
                .bind(pseud_id)
                .bind(since)
                .bind(until)
                .fetch_one(pool)
                .await?;
            let hits = sqlx::query_scalar::<_, i64>(POSTGRES_HITS)
                .bind(pseud_id)
                .bind(since)
                .bind(until)
                .bind(HIT_STARS)
                .fetch_one(pool)
                .await?;
            (shown, hits)
        }
    };

    // Undefined, not zero. §53.5: a zero would read as total failure and invite a
    // change to a recipe that has simply not been tested yet.
    let rate = if shown == 0 {
        None
    } else {
        Some(hits as f64 / shown as f64)
    };

    Ok(HitRate { shown, hits, rate })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_window_is_undefined_rather_than_zero() {
        // §53.5 insists on this distinction, and it is the one a dashboard is most
        // likely to lose: a zero here is indistinguishable from a window where
        // nothing the recipe served was any good.
        let h = HitRate::undefined();
        assert_eq!(h.shown, 0);
        assert_eq!(h.hits, 0);
        assert_eq!(h.rate, None, "no impressions is not total failure");
        assert_eq!(h.percent(), None);
    }

    #[test]
    fn the_percent_rounds_to_two_places() {
        let h = HitRate {
            shown: 3,
            hits: 1,
            rate: Some(1.0 / 3.0),
        };
        assert_eq!(h.percent(), Some(33.33));
        assert_eq!(
            HitRate {
                shown: 2,
                hits: 1,
                rate: Some(0.5)
            }
            .percent(),
            Some(50.0)
        );
    }

    #[test]
    fn four_stars_is_the_threshold() {
        // A policy constant rather than a literal buried in the query, so that
        // changing one without the other is visible as a diff.
        //
        // Note what this does NOT assert: that `HIT_STARS > 3` is true, which is
        // a tautology about a constant and clippy is right to flag it. The
        // behaviour is checked where it lives, in
        // `crates/db/tests/hit_rate.rs::three_stars_is_not_a_hit` and
        // `::a_work_shown_and_rated_four_is_a_hit` -- a constant's value and its
        // effect are different questions, and only the second is worth a test.
        assert_eq!(HIT_STARS, 4);
    }

    #[test]
    fn the_two_dialects_filter_the_same_window() {
        // The window is the same question in both, and a store test that only runs
        // one engine would not notice if one side silently lost its upper bound.
        // Asserted as text so the shape is pinned even without a database.
        for sql in [SQLITE_SHOWN, SQLITE_HITS] {
            assert!(sql.contains(">= ?2"), "the lower bound is inclusive: {sql}");
            assert!(sql.contains("<  ?3"), "the upper bound is exclusive: {sql}");
        }
        for sql in [POSTGRES_SHOWN, POSTGRES_HITS] {
            assert!(sql.contains(">= to_timestamp($2)"));
            assert!(sql.contains("<  to_timestamp($3)"));
        }
    }
}
