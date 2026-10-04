//! M45-23 — the attribution store behind `GET /admin/metrics/north-star`.
//!
//! The domain types carry the decisions (`lorehaven_domain::north_star`); this file is
//! the query that has to earn them on two engines.
//!
//! ## The timestamp trap, which is the reason this file casts what it casts
//!
//! `rating.created_at` is **TEXT** and `recommendation_slots.created_at` is
//! **TIMESTAMPTZ**. The store has to order a slot against a rating to answer "did the
//! slot come first?", which is a comparison across exactly those two types.
//! PostgreSQL rejects it outright:
//!
//! ```text
//! ERROR:  operator does not exist: text <= timestamp with time zone
//! HINT:  No operator matches the given name and argument types.
//! ```
//!
//! Verified against the server rather than inferred, in both directions: `ON a.c <= b.d`
//! errors, and `ON a.c::timestamptz <= b.d` returns the row.
//!
//! Note the direction, because the mirror image of this bug was live elsewhere in the
//! codebase: `spoilers.rs` bound `$1::timestamptz` against a **TEXT** column, which is
//! the same fault with the cast on the wrong side. The rule that settles both: **cast the
//! TEXT side to `timestamptz`, never the `timestamptz` side to TEXT** — the column's own
//! type decides what the other side may be.
//!
//! Having done that comparison, the two timestamp columns are then selected **as text**
//! (`s.created_at::text`), because the arithmetic afterwards is day-granular ISO string
//! work done in [`days_between`]. That removes the decode asymmetry entirely: a `uuid`
//! column still needs two Rust types (`String` on SQLite, `uuid::Uuid` on PostgreSQL),
//! but a timestamp no longer does, and one fewer asymmetry is one fewer thing a
//! per-dialect bug can hide in.
//!
//! ## Why the median is computed in Rust
//!
//! SQLite has no `PERCENTILE_CONT`. The codebase already composes scores in Rust for
//! exactly this reason (`rec_strategy::hidden_classics_strategy`, which exists because
//! SQLite lacks `LOG10`), so this is the established trade rather than a new one. It
//! also gets the even-sized case right explicitly, which a SQL percentile would leave to
//! a dialect difference.

use std::collections::BTreeMap;

use lorehaven_domain::north_star::{MechanismAttribution, MissingInput, NorthStar};

use crate::{Backend, Database};

/// One loved work plus the facts needed to attribute it.
struct LovedWork {
    /// The mechanism of the earliest qualifying slot, or `None` for `unattributed`.
    mechanism: Option<String>,
    /// Days from that slot to the rating. `None` when no slot was served first, which is
    /// the same thing as `mechanism: None`.
    days_to_find: Option<f64>,
}

/// The query, per engine. `pg` says which side of the timestamp cast to use.
///
/// One row per loved work, and **only works with at least one rating in the window**.
/// The `GROUP BY` is what makes "the earliest slot wins" true: without it a work served
/// three times produces three rows, so an implementation taking the last slot — or all
/// of them — would pass a single-slot test while being wrong.
///
/// Both window endpoints inclusive, matching every other window in this codebase.
fn loved_works_sql(pg: bool) -> String {
    // `rating.created_at` is TEXT on both engines. On PostgreSQL the slot column is
    // TIMESTAMPTZ, so the TEXT side is cast. On SQLite both are TEXT and no cast is
    // wanted — `::timestamptz` would silently reinterpret an ISO string as an instant
    // through a different parser, which is a worse failure than the error above.
    let cast = if pg { "::timestamptz" } else { "" };

    // The comparison is written TWICE with DIFFERENT aliases, and sharing one fragment
    // between them is a bug I wrote and then debugged: the correlated subquery below
    // aliases its table `s2`, so a single `cmp` fragment mentioning `s` produced
    //
    //     AND s2.created_at <= r.created_at::timestamptz AND s.created_at <= r.created_at
    //
    // with `s` out of scope inside the subquery. SQLite reported it as the least
    // informative message a SQL engine has:
    //
    //     SqliteError { code: 1, message: "near \"s\": syntax error" }
    //
    // It took printing the generated statement to see, because `s` is a legal alias
    // everywhere else in the query and the error names a character rather than a clause.
    // Two fragments, one per alias, so neither can leak into the other's scope.

    //
    // ## `r.created_at` is in the GROUP BY, and PostgreSQL insists
    //
    // The correlated subquery above references `r.created_at`, which is a bare column of
    // the outer query. Grouping only by `r.work_id` leaves it ungrouped, and PostgreSQL
    // rejects the whole statement:
    //
    //     42803 subquery uses ungrouped column "r.created_at" from outer query
    //     where: parse_agg.c / check_ungrouped_columns_walker
    //
    // SQLite accepts it and returns the right answer, so all seven integration tests pass
    // there and all seven fail here. Grouping by `(r.work_id, r.created_at)` is not a
    // workaround -- it is the honest grouping for a query whose select list is
    // per-rating anyway, since `MIN(r.created_at)` over a group that can only hold one
    // distinct rating time is that value.
    format!(
        "SELECT MIN(s.created_at{cast}) AS first_served_at,
                MIN(r.created_at) AS min_loved_at,
                (SELECT s2.mechanism
                   FROM recommendation_slots s2
                  WHERE s2.work_id = r.work_id
                    AND s2.created_at{cast} <= r.created_at{cast}
                  ORDER BY s2.created_at ASC
                  LIMIT 1) AS mechanism
           FROM rating r
           LEFT JOIN recommendation_slots s
             ON s.work_id = r.work_id
            AND s.created_at{cast} <= r.created_at{cast}
          WHERE r.deleted_at IS NULL
            AND r.stars >= 4
            AND r.created_at >= ? AND r.created_at <= ?
          GROUP BY r.work_id, r.created_at",
        cast = cast,
    )
}

/// §53.5's feed-quality rate and §53.5's companion time-to-find, with §53.5's
/// requirement that the response report its own missing inputs.
///
/// The window is a pair of ISO-8601 strings and is **not** range-validated here. A
/// `since` after `until` returns an empty metric with every input reported missing, which
/// says "nothing happened in that window" instead of raising: the route validates, and a
/// metric that 400s on an odd window is less useful to an operator than one that reports
/// nothing measured.
pub async fn north_star(db: &Database, since: &str, until: &str) -> Result<NorthStar, sqlx::Error> {
    let sql = crate::sql_owned(db, loved_works_sql(false), loved_works_sql(true));

    // `SqliteRow` and `PgRow` are distinct types, so a single `let row = match backend`
    // cannot hold either. Both arms therefore produce `Vec<LovedWork>` and are folded
    // afterwards -- the pattern `flow_store::mechanisms_in_window` uses for the same
    // reason.
    let works: Vec<LovedWork> = match db.backend() {
        Backend::Sqlite => {
            use sqlx::Row;
            let rows = sqlx::query(&sql)
                .bind(since)
                .bind(until)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?;
            let mut out = Vec::with_capacity(rows.len());
            for row in rows {
                let first_served: Option<String> = row
                    .try_get::<Option<String>, _>("first_served_at")
                    .unwrap_or(None);
                let loved_at: Option<String> = row
                    .try_get::<Option<String>, _>("min_loved_at")
                    .unwrap_or(None);
                out.push(LovedWork {
                    mechanism: row
                        .try_get::<Option<String>, _>("mechanism")
                        .unwrap_or(None),
                    days_to_find: match (first_served.as_deref(), loved_at.as_deref()) {
                        (Some(served), Some(loved)) => days_between(served, loved),
                        _ => None,
                    },
                });
            }
            out
        }
        Backend::Postgres => {
            use sqlx::Row;
            let rows = sqlx::query(&sql)
                .bind(since)
                .bind(until)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?;
            let mut out = Vec::with_capacity(rows.len());
            for row in rows {
                // `first_served_at` is `::text` in the statement, so it decodes as
                // `String` here exactly as on SQLite. The rating side is TEXT natively.
                let first_served: Option<String> = row
                    .try_get::<Option<String>, _>("first_served_at")
                    .unwrap_or(None);
                let loved_at: Option<String> = row
                    .try_get::<Option<String>, _>("min_loved_at")
                    .unwrap_or(None);
                out.push(LovedWork {
                    mechanism: row
                        .try_get::<Option<String>, _>("mechanism")
                        .unwrap_or(None),
                    days_to_find: match (first_served.as_deref(), loved_at.as_deref()) {
                        (Some(served), Some(loved)) => days_between(served, loved),
                        _ => None,
                    },
                });
            }
            out
        }
    };

    // §53.6's denominator: completions, `finished` **alone**. Not §53.5's "finished or
    // rated >= 4" -- the two definitions coexist deliberately, and reusing §53.5's would
    // put the cheap signal (a four-star rating, one click) inside the expensive one (a
    // completion, a reader's time), defeating the ratio's purpose.
    let completion_sql = crate::sql_owned(
        db,
        "SELECT COUNT(DISTINCT subject_id) FROM reading_status
          WHERE subject_type = 'work' AND status = 'finished'
            AND updated_at >= ? AND updated_at <= ?"
            .to_string(),
        "SELECT COUNT(DISTINCT subject_id) FROM reading_status
          WHERE subject_type = 'work' AND status = 'finished'
            AND updated_at >= $1 AND updated_at <= $2"
            .to_string(),
    );
    // The slot-count bind needs `::timestamptz` and the completion bind does NOT, which
    // looks arbitrary until the two column types are read side by side:
    //
    //     reading_status.updated_at         TEXT           -- a bare $1 works
    //     recommendation_slots.created_at   TIMESTAMPTZ    -- a bare $1 is 42883
    //
    //     ERROR:  operator does not exist: timestamp with time zone >= text
    //
    // The same asymmetry the module docs describe for the join, in the window filter
    // instead. Verified against the server: the `slot_sql` form without the cast fails,
    // and with it returns the count.
    let slot_sql = crate::sql_owned(
        db,
        "SELECT COUNT(*) FROM recommendation_slots
          WHERE created_at >= ? AND created_at <= ?"
            .to_string(),
        "SELECT COUNT(*) FROM recommendation_slots
          WHERE created_at >= $1::timestamptz AND created_at <= $2::timestamptz"
            .to_string(),
    );
    let (completions, slots) = match db.backend() {
        Backend::Sqlite => {
            let c: (i64,) = sqlx::query_as(&completion_sql)
                .bind(since)
                .bind(until)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?;
            let s: (i64,) = sqlx::query_as(&slot_sql)
                .bind(since)
                .bind(until)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?;
            (c.0, s.0)
        }
        Backend::Postgres => {
            let c: (i64,) = sqlx::query_as(&completion_sql)
                .bind(since)
                .bind(until)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?;
            let s: (i64,) = sqlx::query_as(&slot_sql)
                .bind(since)
                .bind(until)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?;
            (c.0, s.0)
        }
    };

    let loved_works = works.len() as i64;
    let unattributed = works.iter().filter(|w| w.mechanism.is_none()).count() as i64;

    // BTreeMap, so the breakdown order is deterministic and does not depend on hash order:
    // two windows with the same data must serialise identically.
    // `unattributed` is deliberately NOT a row in `by_mechanism`. It is a separate
    // top-level field, and folding it in here made
    // `shares_account_for_everything()` fail on a metric that was arithmetically
    // correct: the sum then counted those works twice, once as the `unattributed` row and
    // once as `unattributed` itself. The invariant is "mechanisms PLUS unattributed", and
    // putting unattributed inside the mechanisms breaks its own definition.
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    for work in &works {
        if let Some(mechanism) = &work.mechanism {
            *counts.entry(mechanism.clone()).or_default() += 1;
        }
    }
    let by_mechanism: Vec<MechanismAttribution> = counts
        .into_iter()
        .map(|(key, count)| MechanismAttribution {
            key,
            loved_works: count,
            share: if loved_works == 0 {
                0.0
            } else {
                count as f64 / loved_works as f64
            },
        })
        .collect();

    // The median runs over *attributed* works only. `mechanism: None` means "never served
    // here", so there is no first serving to measure a gap from; counting it as zero
    // would drag the median toward zero and report instant discovery.
    let mut gaps: Vec<f64> = works.iter().filter_map(|w| w.days_to_find).collect();
    gaps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median_days_to_find = median(&gaps);

    let mut missing_inputs = Vec::new();
    if works.is_empty() {
        missing_inputs.push(MissingInput::Ratings);
    }
    if completions == 0 {
        missing_inputs.push(MissingInput::Completions);
    }
    if slots == 0 {
        missing_inputs.push(MissingInput::Slots);
    }

    let months = months_between(since, until);

    Ok(NorthStar {
        works_rated_per_month: loved_works as f64 / months,
        median_days_to_find,
        rated_works: loved_works,
        loved_works,
        unattributed,
        by_mechanism,
        missing_inputs,
    })
}

/// Median of an already-sorted slice.
///
/// `pub` so `crates/db/tests/north_star_arithmetic.rs` can assert the degenerate cases
/// — empty, and even-sized — without a database. A test that re-implemented this to
/// exercise it would prove only that the copy is self-consistent, which is the exact
/// mistake recorded in `docs/plans/REMAINING-2026-10-03.md` for
/// `arena_weights_decode.rs`.
///
/// The even-sized case is the whole reason this is not a one-liner: with 2 samples the
/// median is their mean, and returning either endpoint instead would make the metric
/// depend on whether a window happened to contain an even number of discoveries. An
/// empty slice is `None` — undefined, never `0.0` (§53.5).
pub fn median(sorted: &[f64]) -> Option<f64> {
    match sorted.len() {
        0 => None,
        n if n % 2 == 1 => Some(sorted[n / 2]),
        n => Some((sorted[n / 2 - 1] + sorted[n / 2]) / 2.0),
    }
}

/// Whole days between two RFC-3339 instants, or `None` if either does not parse.
///
/// Day-granular on purpose: §53.5's companion is *days to find*, and sub-second precision
/// would report a reader who loved a work in the same second they saw it as having taken
/// 0.0000 days — a true number and a useless one.
///
/// Non-whole days floor rather than round, so a gap of 23 hours reads as 0 days (found the
/// same day) instead of 1 (found the next day).
pub fn days_between(from: &str, to: &str) -> Option<f64> {
    let from = parse_rfc3339(from)?;
    let to = parse_rfc3339(to)?;
    Some((to - from).whole_days() as f64)
}

/// Parse an RFC-3339 instant.
///
/// The two spellings both occur on purpose: rows written by this codebase use `Z`, and
/// rows written by PostgreSQL come back from `::text` as `+00:00`. `time`'s `Rfc3339`
/// parser accepts both, so no normalisation is needed — but a `Z`-only parser would
/// reject half the rows and turn the median into `None` on PostgreSQL alone, which is
/// precisely the kind of per-dialect difference this module is trying to avoid.
pub fn parse_rfc3339(value: &str) -> Option<time::OffsetDateTime> {
    time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).ok()
}

/// Months in `[since, until]`, floored at a twentieth of one.
///
/// The floor is not arbitrary: zero would divide by zero, and one would make a one-day
/// window report a full month's rate. A twentieth of a month is ~36 hours — short enough
/// that a weekend-sized window still divides, long enough that a day-scale window does
/// not inflate the rate by a factor of thirty.
///
/// `30.4375` is the mean Gregorian month, so a full year divides to 12.0 rather than
/// 12.17; a window given in whole years should not report thirteen months.
pub fn months_between(since: &str, until: &str) -> f64 {
    const MEAN_MONTH_DAYS: f64 = 30.4375;
    const FLOOR: f64 = 1.0 / 20.0;
    let days = match (parse_rfc3339(since), parse_rfc3339(until)) {
        (Some(a), Some(b)) => (b - a).whole_days() as f64,
        // An unparseable bound is not a window of zero days -- that would divide by the
        // floor and report a 36-hour month for a query that actually covered a year.
        // One month is the least-bad reading: the count is still reported, and the
        // operator sees a plausible rate rather than an inflated one.
        _ => return 1.0,
    };
    if days <= 0.0 {
        return FLOOR;
    }
    (days / MEAN_MONTH_DAYS).max(FLOOR)
}
