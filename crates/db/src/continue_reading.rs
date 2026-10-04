//! Item 1 of the 100-idea audit: "Continue Reading".
//!
//! The single highest-impact retention feature on the list, and the cheapest: the data
//! already exists and nothing displays it.
//!
//! ## What this module is NOT
//!
//! It is not a per-work progress query. `routes/reading.rs` already has
//! `resolve_progress`, and `ResumePrompt.svelte` already renders it on a work page — that
//! answers "where am I in THIS work". Item 1 asks a different question: *across* every
//! work the reader has opened, which one did they most recently stop reading, and how far
//! did they get. That cross-work question has no query anywhere in the tree
//! (`grep -rn 'last_read\|recent_work\|continue_reading' crates/` hits only
//! `conversation_participants.last_read_at`, which is the forum).
//!
//! ## The two problems in this query, and why they are here
//!
//! **1. Progress is per-DEVICE, and there are two of this reader's rows.** `0004_reading`
//! declares `reading_progress_unique` on `(account_id, pseud_id, subject_type,
//! subject_id, device_id) WHERE device_id IS NOT NULL` and `reading_progress_no_device` on
//! the same tuple `WHERE device_id IS NULL`. A reader who read on a laptop and a phone has
//! TWO rows per work, and the naive `GROUP BY work_id` picks one arbitrarily — so the
//! banner would jump between devices. This query takes the **MAX `updated_at` per work**
//! first and then picks the most recently touched work, which is the row the reader
//! actually last wrote and is independent of which device won.
//!
//! **2. "Continue" means unfinished, so a 100% row must not be offered.** A work the
//! reader finished has `position_permille = 1000` and belongs on their shelf, not in a
//! "pick up where you left off" banner. `works.completion = 'complete'` is the second
//! guard, because a reader can reach the end of a work the author has not marked complete
//! and vice versa; a reader asking to continue a work they finished is a bug report in
//! costume.
//!
//! ## Both are tested by inversion, not by inspection
//!
//! `continue_reading_picks_the_most_recently_touched_work` inverts the order; the two
//! `*_finished*` tests each delete one guard and watch a specific test go red.

use crate::{Database, Result};

/// One row of the "continue reading" banner.
#[derive(Debug, Clone, PartialEq)]
pub struct ContinueReading {
    pub work_id: String,
    pub title: String,
    /// Where the reader stopped, in per-mille (0..1000). Deliberately the raw column
    /// rather than a percentage: the caller decides how to round, and a store that
    /// returns `83.3%` as a string has taken a presentation decision.
    pub position_permille: i32,
    /// The chapter the reader was on, if the row names one.
    pub chapter_id: Option<String>,
    /// That chapter's title, resolved for display. `None` when the row names no chapter,
    /// or when the chapter was since deleted — which must not fail the whole banner.
    pub chapter_title: Option<String>,
    /// RFC 3339. When this reader last wrote this row.
    pub updated_at: String,
}

impl ContinueReading {
    /// Progress as a whole percentage, for display.
    ///
    /// Clamped to 0..=100: `position_permille` is an INTEGER with no CHECK constraint
    /// (`DEFAULT 0`, and nothing prevents a client sending 1500), so a value outside the
    /// range is a bug elsewhere and must not reach a reader as "150%".
    #[must_use]
    pub fn percent(&self) -> u8 {
        let per_mille = self.position_permille.clamp(0, 1000);
        u8::try_from(per_mille / 10).unwrap_or(100)
    }
}

/// The reader's most recently touched unfinished work, or `None`.
///
/// Returns at most one row. A banner with three entries is a reading history, and the
/// feature is "know instantly where you left off" — which one row answers.
pub async fn continue_reading(db: &Database, account_id: &str) -> Result<Option<ContinueReading>> {
    let sql = db.sql(
        // ## How a reader's DEVICES collapse, and why this is not an aggregate
        //
        // `0004_reading` declares TWO partial unique indexes: one on
        // `(account_id, pseud_id, subject_type, subject_id, device_id) WHERE device_id IS
        // NOT NULL` and one on the same tuple `WHERE device_id IS NULL`. So a reader with
        // a laptop and a phone has TWO rows per work, and any aggregate over them is a
        // choice about which device wins.
        //
        // The first version of this query used `MAX(position_permille)` and
        // `MAX(updated_at)`. That is wrong, and
        // `prefers_the_row_this_reader_wrote_most_recently` caught it: it puts a phone row
        // at 900 per-mille (older) and a laptop row at 150 (newer) on ONE work, and MAX
        // reported 900 -- telling the reader they were 90% through when the last thing
        // they did was put it back to 15%. **MAX is the FURTHEST position, not the LAST
        // one**, and aggregating two columns independently also lets them disagree about
        // which row they came from: `MAX(updated_at)` ordered the group while
        // `MAX(position_permille)` reported a position from a different chapter.
        //
        // So there is no aggregate. Grouping by every selected column and ordering by
        // `updated_at` puts the reader's NEWEST row first, and `LIMIT 1` returns all of
        // its columns from that one row. A reader who has finished the work on one device
        // but is 40% through it on another gets the 40% -- which is right, because that is
        // where they will resume.
        r#"
        SELECT rp.subject_id                       AS work_id,
               w.title                              AS title,
               rp.position_permille                 AS position_permille,
               rp.chapter_id                        AS chapter_id,
               rp.updated_at                        AS updated_at
          FROM reading_progress rp
          JOIN works w ON w.id = rp.subject_id
         WHERE rp.account_id = ?1
           AND rp.subject_type = 'work'
           AND w.completion <> 'complete'
           -- A draft the reader cannot open again must not be offered.
           AND w.published_at IS NOT NULL
           -- Unfinished, and BOTH guards are needed:
           --   position_permille < 1000  the reader has not reached the end
           --   completion <> 'complete'  the AUTHOR has not finished it either (above)
           -- Either alone is insufficient: a reader who reaches the end of an in-progress
           -- serial is not continuing it, and a work the author completed that the reader
           -- abandoned is not either. Both are tested by deleting exactly one at a time.
           AND rp.position_permille < 1000
         GROUP BY rp.subject_id, w.title, rp.position_permille,
                  rp.chapter_id, rp.updated_at
         ORDER BY rp.updated_at DESC, rp.subject_id ASC
         LIMIT 1
        "#,
        // Same query, PostgreSQL dialect. Two casts, not one: `rp.subject_id` is TEXT here
        // and `works.id` is UUID, so the JOIN needs `rp.subject_id::uuid` -- without it
        // this is `operator does not exist: uuid = text` at plan time, which reads as a
        // schema problem rather than a cast problem. The SQLite arm cannot catch it: its
        // `subject_id` really is TEXT and its `works.id` really is TEXT. `chapter_id` is
        // cast in the SELECT because it is read back into a `String`.
        r#"
        SELECT rp.subject_id::text              AS work_id,
               w.title                           AS title,
               rp.position_permille              AS position_permille,
               rp.chapter_id::text               AS chapter_id,
               rp.updated_at                     AS updated_at
          FROM reading_progress rp
          JOIN works w ON w.id = rp.subject_id::uuid
         WHERE rp.account_id = $1::uuid
           AND rp.subject_type = 'work'
           AND w.completion <> 'complete'
           AND w.published_at IS NOT NULL
           -- Unfinished, checked on THIS row rather than on an aggregate over the
           -- reader's devices: the position a reader last saw is the one on their newest
           -- row, so that is the one that decides whether there is anything to continue.
           AND rp.position_permille < 1000
         GROUP BY rp.subject_id, w.title, rp.position_permille,
                  rp.chapter_id, rp.updated_at
         ORDER BY rp.updated_at DESC, rp.subject_id ASC
         LIMIT 1
        "#,
    );

    let row: Option<(String, String, i64, Option<String>, String)> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite pool for a sqlite backend"))
                .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .fetch_optional(
                    db.postgres_pool()
                        .expect("postgres pool for a postgres backend"),
                )
                .await?
        }
    };

    let Some((work_id, title, position_permille, chapter_id, updated_at)) = row else {
        return Ok(None);
    };

    // The chapter title is a SECOND query on purpose. Resolving it in the join above would
    // multiply the `reading_progress` rows by the chapter count and break the
    // one-row-per-work grouping the whole query depends on.
    let chapter_title = match chapter_id.as_deref() {
        None => None,
        Some(id) => chapter_title_for(db, id).await?,
    };

    Ok(Some(ContinueReading {
        work_id,
        title,
        position_permille: i32::try_from(position_permille).unwrap_or(1000),
        chapter_id,
        chapter_title,
        updated_at,
    }))
}

/// The title of a chapter, or `None` if it no longer exists.
///
/// A missing chapter is not a failure: the reader's progress row outlives the chapter it
/// points at, and a deleted chapter must cost the banner its chapter NAME, not the whole
/// banner. A 500 here would blank the retention feature because an author deleted a
/// chapter.
async fn chapter_title_for(db: &Database, chapter_id: &str) -> Result<Option<String>> {
    let sql = db.sql(
        "SELECT title FROM chapters WHERE id = ?1",
        "SELECT title FROM chapters WHERE id = $1::uuid",
    );

    let found: Option<(String,)> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(chapter_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite pool for a sqlite backend"))
                .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(chapter_id)
                .fetch_optional(
                    db.postgres_pool()
                        .expect("postgres pool for a postgres backend"),
                )
                .await?
        }
    };

    Ok(found.map(|(title,)| title))
}

/// The reader's unfinished works, most recent first, up to `limit`.
///
/// Not used by the banner, which is deliberately one row. This is for the library and
/// for tests, and the bound is required rather than trusted: `LIMIT $1` with an
/// unbounded value from a query string is an unbounded result set.
pub async fn unfinished_works(
    db: &Database,
    account_id: &str,
    limit: i64,
) -> Result<Vec<ContinueReading>> {
    let capped = limit.clamp(1, 50);
    let sql = db.sql(
        // No aggregate here either, and for the same reason as the single-row query: the
        // reader's newest row is the position they will resume from, and MAX would report
        // the furthest one instead.
        r#"
        SELECT rp.subject_id, w.title,
               rp.position_permille,
               rp.chapter_id,
               rp.updated_at
          FROM reading_progress rp
          JOIN works w ON w.id = rp.subject_id
         WHERE rp.account_id = ?1
           AND rp.subject_type = 'work'
           AND w.completion <> 'complete'
           AND w.published_at IS NOT NULL
           AND rp.position_permille < 1000
         GROUP BY rp.subject_id, w.title, rp.position_permille,
                  rp.chapter_id, rp.updated_at
         ORDER BY rp.updated_at DESC, rp.subject_id ASC
         LIMIT ?2
        "#,
        r#"
        SELECT rp.subject_id::text, w.title, rp.position_permille,
               rp.chapter_id::text, rp.updated_at
          FROM reading_progress rp
          JOIN works w ON w.id = rp.subject_id::uuid
         WHERE rp.account_id = $1::uuid
           AND rp.subject_type = 'work'
           AND w.completion <> 'complete'
           AND w.published_at IS NOT NULL
           AND rp.position_permille < 1000
         -- Grouping by every selected column, then ordering by `updated_at`, collapses
         -- this reader's devices to the row they wrote LAST rather than to an aggregate
         -- over all of them.
         GROUP BY rp.subject_id, w.title, rp.position_permille,
                  rp.chapter_id, rp.updated_at
         ORDER BY rp.updated_at DESC, rp.subject_id ASC
         LIMIT $2
        "#,
    );

    let rows: Vec<(String, String, i64, Option<String>, String)> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .bind(capped)
                .fetch_all(db.sqlite_pool().expect("sqlite pool for a sqlite backend"))
                .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .bind(capped)
                .fetch_all(
                    db.postgres_pool()
                        .expect("postgres pool for a postgres backend"),
                )
                .await?
        }
    };

    let mut out = Vec::with_capacity(rows.len());
    for (work_id, title, position_permille, chapter_id, updated_at) in rows {
        let chapter_title = match chapter_id.as_deref() {
            None => None,
            Some(id) => chapter_title_for(db, id).await?,
        };
        out.push(ContinueReading {
            work_id,
            title,
            position_permille: i32::try_from(position_permille).unwrap_or(1000),
            chapter_id,
            chapter_title,
            updated_at,
        });
    }
    Ok(out)
}

/// The works a reader should not be offered again, as `(work_id, reason)`.
///
/// This is what item 11's DNF feeds. It is a query rather than a filter inside
/// `continue_reading` for one reason: a DNF mark has SIX reasons and the caller may want
/// different treatment for each — `abandoned_by_author` should probably be hidden while
/// `triggering` MUST be, and `not_my_taste` should be shown again. Returning the reason
/// alongside keeps that judgement in the caller where the policy lives.
///
/// Private DNF is still excluded. Not because the aggregate would leak it — this is the
/// reader's OWN list — but because a reader who marked a work private has expressed a
/// preference about it that is not "stop recommending it".
pub async fn dnf_works_for(db: &Database, account_id: &str) -> Result<Vec<(String, String)>> {
    let sql = db.sql(
        r#"
        SELECT work_id, reason
          FROM did_not_finish
         WHERE account_id = ?1
           AND deleted_at IS NULL
           -- `= 1` is correct HERE and must not be "harmonised" with the PostgreSQL arm's
           -- `= TRUE`: this column is INTEGER on SQLite and BOOLEAN on PostgreSQL.
           AND is_public = 1
         ORDER BY updated_at DESC
        "#,
        r#"
        SELECT work_id::text, reason
          FROM did_not_finish
         WHERE account_id = $1::uuid
           AND deleted_at IS NULL
           -- TRUE, not 1. `did_not_finish.is_public` is BOOLEAN on PostgreSQL and INTEGER
           -- on SQLite, and `boolean = integer` is not a comparison PostgreSQL has:
           -- `operator does not exist: boolean = integer`. The SQLite arm's `= 1` is
           -- CORRECT there, so this defect is invisible on the default engine and the
           -- two-engine run is the only thing that catches it. Same class as the
           -- `::uuid` cast this query's other arm needs.
           AND is_public = TRUE
         ORDER BY updated_at DESC
        "#,
    );

    let rows: Vec<(String, String)> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .fetch_all(db.sqlite_pool().expect("sqlite pool for a sqlite backend"))
                .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .fetch_all(
                    db.postgres_pool()
                        .expect("postgres pool for a postgres backend"),
                )
                .await?
        }
    };

    // The id stays a `String`. Wrapping it in a `WorkId` here would be a convenience the
    // caller pays for in a `match` on the parse, and `did_not_finish.work_id` is TEXT on
    // SQLite and UUID on PostgreSQL — so a value that parsed on one engine could fail to
    // parse on the other for a row this query just returned successfully.
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_is_whole_and_clamped() {
        let make = |p| ContinueReading {
            work_id: "w".into(),
            title: "t".into(),
            position_permille: p,
            chapter_id: None,
            chapter_title: None,
            updated_at: "t".into(),
        };
        assert_eq!(make(0).percent(), 0);
        assert_eq!(make(5).percent(), 0);
        assert_eq!(make(10).percent(), 1);
        assert_eq!(make(835).percent(), 83);
        assert_eq!(make(1000).percent(), 100);
        // `position_permille` has no CHECK constraint, so an out-of-range value is a bug
        // elsewhere and must not reach a reader as "150%".
        assert_eq!(make(1500).percent(), 100);
        assert_eq!(make(-5).percent(), 0);
    }
}
