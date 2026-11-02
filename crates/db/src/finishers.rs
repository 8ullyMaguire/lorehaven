//! §60 — finisher signals: the two numbers behind the sentence.
//!
//! Nothing here is stored. Completion history is a `GROUP BY` over `works.completion` and cadence is
//! a set of distinct revision days; a stored badge or a cached rate would need an invalidation story
//! for every completion change, and the counts are cheap.
//!
//! ## Days, not timestamps
//!
//! Cadence is measured in **whole days** — `substr(created_at, 1, 10)` — because "updates about every
//! two weeks" is a claim about days and a claim about seconds would be precision nobody asked for.
//! The live instance's revisions carry a six-digit fraction on import
//! (`2026-10-27T15:05:58.248478Z`), which a day-granular measurement is immune to by construction.
//!
//! ## A schema without revisions says so
//!
//! `revisions` is absent from some deployments (M63-33's shape). [`has_revisions`] asks the catalog
//! once, and cadence answers [`Cadence::Unmeasured`] with a reason instead of a `500` — the same
//! answer §58.2 gives for a drought that cannot be measured.

use anyhow::Result;
use sqlx::Row;
use time::Date;

use darkhold_domain::finishers::{cadence, cadence_across, history, Cadence, History};

use crate::{Backend, Database};

/// How many works' worth of revision days an author's cadence is measured over.
///
/// Capped because a prolific author with 700 units would otherwise read every revision row for a
/// median, and the median of the first nineteen multi-revision works is not a different answer from
/// the median of seventy.
pub const CADENCE_SAMPLE: i64 = 20;

/// Whether this database can measure cadence at all.
pub async fn has_revisions(db: &Database) -> Result<bool> {
    crate::wip_adoption::has_table(db, "revisions").await
}

/// Whether this pseud exists. A 404 for an unknown author is a different answer from a zero count.
pub async fn pseud_exists(db: &Database, pseud_id: &str) -> Result<bool> {
    let sql = db.sql(
        "SELECT COUNT(*) AS n FROM pseuds WHERE id = ?",
        "SELECT COUNT(*) AS n FROM pseuds WHERE id::text = $1",
    );
    let n: i64 = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(pseud_id)
            .fetch_one(db.sqlite_pool().expect("sqlite"))
            .await?
            .get::<i64, _>("n"),
        Backend::Postgres => sqlx::query(&sql)
            .bind(pseud_id)
            .fetch_one(db.postgres_pool().expect("postgres"))
            .await?
            .get::<i64, _>("n"),
    };
    Ok(n > 0)
}

/// An author's completion history.
pub async fn author_history(db: &Database, pseud_id: &str) -> Result<History> {
    let sql = db.sql(
        "SELECT
            SUM(CASE WHEN completion = 'complete' THEN 1 ELSE 0 END) AS finished,
            SUM(CASE WHEN completion = 'abandoned' THEN 1 ELSE 0 END) AS abandoned
           FROM works WHERE owner_pseud_id = ?",
        "SELECT
            SUM(CASE WHEN completion = 'complete' THEN 1 ELSE 0 END) AS finished,
            SUM(CASE WHEN completion = 'abandoned' THEN 1 ELSE 0 END) AS abandoned
           FROM works WHERE owner_pseud_id::text = $1",
    );
    let (finished, abandoned): (i64, i64) = match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(&sql)
                .bind(pseud_id)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?;
            // `SUM` over no rows is NULL; an author with no works holds zero, not nothing.
            (
                row.get::<Option<i64>, _>("finished").unwrap_or(0),
                row.get::<Option<i64>, _>("abandoned").unwrap_or(0),
            )
        }
        Backend::Postgres => {
            let row = sqlx::query(&sql)
                .bind(pseud_id)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?;
            (
                row.get::<Option<i64>, _>("finished").unwrap_or(0),
                row.get::<Option<i64>, _>("abandoned").unwrap_or(0),
            )
        }
    };
    Ok(history(finished, abandoned))
}

/// Turn `YYYY-MM-DD` into a day ordinal.
///
/// A timestamp whose date part does not parse is skipped rather than failing the read: one malformed
/// import row must not make an author's cadence a `500`.
fn day_ordinal(text: &str) -> Option<i64> {
    let date = text.get(..10)?;
    let parsed = Date::parse(date, &time::format_description::well_known::Iso8601::DATE).ok()?;
    Some(parsed.to_julian_day() as i64)
}

/// The distinct days on which this work gained a revision, as day ordinals, ascending.
pub async fn work_days(db: &Database, work_id: &str) -> Result<Vec<i64>> {
    if !has_revisions(db).await? {
        return Ok(Vec::new());
    }
    let sql = db.sql(
        "SELECT DISTINCT substr(r.created_at, 1, 10) AS day
           FROM revisions r
           JOIN units u ON u.id = r.unit_id
          WHERE u.work_id = ?
          ORDER BY 1",
        "SELECT DISTINCT substr(r.created_at, 1, 10) AS day
           FROM revisions r
           JOIN units u ON u.id = r.unit_id
          WHERE u.work_id::text = $1
          ORDER BY 1",
    );
    let rows: Vec<String> = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(work_id)
            .fetch_all(db.sqlite_pool().expect("sqlite"))
            .await?
            .iter()
            .map(|r| r.get::<String, _>("day"))
            .collect(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(work_id)
            .fetch_all(db.postgres_pool().expect("postgres"))
            .await?
            .iter()
            .map(|r| r.get::<String, _>("day"))
            .collect(),
    };
    Ok(rows.iter().filter_map(|d| day_ordinal(d)).collect())
}

/// One work's cadence.
pub async fn work_cadence(db: &Database, work_id: &str) -> Result<Cadence> {
    if !has_revisions(db).await? {
        return Ok(Cadence::Unmeasured {
            reason: "revisions is absent from this instance's schema, so a writing history cannot \
                     be measured here (M63-33)"
                .to_string(),
        });
    }
    let days = work_days(db, work_id).await?;
    Ok(cadence(&days))
}

/// An author's cadence: the day ordinals of their works that have something to measure, up to
/// [`CADENCE_SAMPLE`] works, pooled into one list of days.
pub async fn author_cadence(db: &Database, pseud_id: &str) -> Result<Cadence> {
    if !has_revisions(db).await? {
        return Ok(Cadence::Unmeasured {
            reason: "revisions is absent from this instance's schema, so a writing history cannot \
                     be measured here (M63-33)"
                .to_string(),
        });
    }
    let sql = db.sql(
        "SELECT w.id AS work_id
           FROM works w
          WHERE w.owner_pseud_id = ?
            AND EXISTS (SELECT 1 FROM units u WHERE u.work_id = w.id)
          ORDER BY w.id
          LIMIT ?",
        "SELECT w.id::text AS work_id
           FROM works w
          WHERE w.owner_pseud_id::text = $1
            AND EXISTS (SELECT 1 FROM units u WHERE u.work_id = w.id)
          ORDER BY 1
          LIMIT $2",
    );
    let works: Vec<String> = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(pseud_id)
            .bind(CADENCE_SAMPLE)
            .fetch_all(db.sqlite_pool().expect("sqlite"))
            .await?
            .iter()
            .map(|r| r.get::<String, _>("work_id"))
            .collect(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(pseud_id)
            .bind(CADENCE_SAMPLE)
            .fetch_all(db.postgres_pool().expect("postgres"))
            .await?
            .iter()
            .map(|r| r.get::<String, _>("work_id"))
            .collect(),
    };

    if works.is_empty() {
        return Ok(Cadence::Unmeasured {
            reason: "this author has no works with revisions yet, so there is no cadence to \
                     measure"
                .to_string(),
        });
    }

    // Each work's own median gap, then the median of those — not the pooled days, which would
    // measure the gap BETWEEN two works as if it were an update interval.
    let mut per_work: Vec<i64> = Vec::new();
    for work in &works {
        if let Cadence::Measurable { median_days, .. } = cadence(&work_days(db, work).await?) {
            per_work.push(median_days);
        }
    }
    Ok(cadence_across(&per_work))
}
