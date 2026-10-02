//! Rec blurbs as second summaries (spec §49.4, M45-35).
//!
//! ## The four clauses, and where each is enforced
//!
//! 1. The top-rated rec note is surfaced **beside** the author's summary, never
//!    instead of it, and always attributed and quotable.
//! 2. Only notes the author has **allowed to be quoted** are eligible.
//! 3. The excerpt is **bounded**, so the author's work is not displaced.
//! 4. Rec blurbs are **not** a ranking input.
//!
//! Clause 2's wording says "the author", and that is a slip worth naming: the
//! note is the *reader's* words about the author's work, so the consent is the
//! reader's to give. A work's author cannot consent to a stranger's sentences.
//! `review.allow_quote` is therefore the reader's own setting, and it defaults to
//! false — otherwise this migration would retroactively publish every rec note
//! ever written, which is the exact failure the clause exists to prevent.
//!
//! Clause 4 is enforced *structurally* rather than by a check: nothing in this
//! module writes, reads or returns anything the ranker consumes, and there is no
//! column on any ranking table. There is nothing to turn off later, because
//! nothing was ever connected.

use crate::{Backend, Database};
use anyhow::Result;
use sqlx::Row;

/// How long an excerpt may be, in characters.
///
/// §49.4 asks for "a short pull, not the whole note". This is the number that
/// makes the clause true, and it is a constant rather than a parameter because a
/// caller who can choose the excerpt length can also choose to show the whole
/// note. Two hundred and forty characters is roughly two sentences of a rec note
/// — enough to be a second summary, short enough that the work it describes is
/// still what the reader sees.
///
/// The truncation is on a **character** boundary rather than a word boundary,
/// because §49.4 quotes whatever the reader chose and a mid-word cut in quoted
/// prose reads as a rendering bug. When the note is long the excerpt is
/// `[..n-1]` plus an ellipsis, so the result is never longer than
/// [`EXCERPT_CHARS`].
pub const EXCERPT_CHARS: usize = 240;

/// A rec note surfaced as a second summary.
#[derive(Debug, Clone, PartialEq)]
pub struct RecBlurb {
    pub review_id: String,
    pub work_id: String,
    /// The pseudonym the note is attributed to. Never the account id: §12's
    /// pseudonymity is why rec notes are worth reading.
    pub pseud_handle: String,
    /// The bounded pull-quote. Never the whole note (§49.4, clause 3).
    pub excerpt: String,
    /// True when the note was shortened, so a caller can tell a complete short
    /// note from a truncated long one rather than rendering both identically.
    pub truncated: bool,
    /// The writer's stars for the work, or `None` when they wrote a note without
    /// rating. §49.4 sorts by this, so a missing rating has to sort rather than
    /// be filtered out — a note with no rating is still a second summary.
    pub stars: Option<i64>,
}

/// Cut `body` down to [`EXCERPT_CHARS`], on a character boundary.
///
/// Kept separate from the query because it is pure, so it can be tested without a
/// database, and because it is the one place the bound could be quietly widened.
#[must_use]
pub fn excerpt(body: &str) -> (String, bool) {
    let count = body.chars().count();
    if count <= EXCERPT_CHARS {
        return (body.to_owned(), false);
    }
    let head: String = body.chars().take(EXCERPT_CHARS - 1).collect();
    (format!("{head}…"), true)
}

/// The top-rated quotable rec note on a work, or `None` when there is none.
///
/// "Top-rated" is the writer's own star rating of the work, descending, with the
/// review id as the tie-break. The tie-break is not cosmetic: §47.9 requires
/// transfers to be deterministic, and two notes with the same rating would
/// otherwise come back in whatever order the planner produced, which differs
/// between engines.
pub async fn top_rec_blurb(db: &Database, work_id: &str) -> Result<Option<RecBlurb>> {
    let sql = db.sql(
        // `rating_pseud_work` is UNIQUE over live rows, so the join is at most
        // one rating per candidate note -- no fan-out, no `GROUP BY` needed.
        //
        // The review must be PUBLIC as well as quotable: a reader who consented
        // to being quoted has not thereby made the note public, and those are two
        // separate decisions they get to make.
        //
        // `LEFT JOIN`, because a rec note with no rating is still a second
        // summary (§49.4 says "the best recommendation note", not "the best
        // rated one"); an inner join would silently drop exactly the unrated
        // notes that most need a second summary.
        r"
        SELECT r.id            AS review_id,
               r.body          AS body,
               p.handle        AS pseud_handle,
               g.stars         AS stars
          FROM review r
          JOIN pseuds p ON p.id = r.pseud_id
          LEFT JOIN rating g
                 ON g.pseud_id = r.pseud_id
                AND g.work_id = r.work_id
                AND g.deleted_at IS NULL
         WHERE r.work_id = ?
           AND r.allow_quote = 1
           AND r.is_public = 1
           AND r.published_at IS NOT NULL
           AND r.deleted_at IS NULL
           AND p.deleted_at IS NULL
         ORDER BY g.stars DESC NULLS LAST, r.id
         LIMIT 1
        ",
        r"
        -- `r.id::text` is not decoration. `review.id` is TEXT on SQLite and UUID
        -- on PostgreSQL, and `RecBlurb.review_id` is a `String`, so without the
        -- cast PostgreSQL fails with `Rust type String (as TEXT) is not
        -- compatible with SQL type UUID` on the very first row. SQLite cannot
        -- catch it: its ids are already TEXT. Which columns are UUID is
        -- per-column and per-dialect, so it is written per arm.
        SELECT r.id::text     AS review_id,
               r.body          AS body,
               p.handle        AS pseud_handle,
               g.stars         AS stars
          FROM review r
          JOIN pseuds p ON p.id = r.pseud_id
          LEFT JOIN rating g
                 ON g.pseud_id = r.pseud_id
                AND g.work_id = r.work_id
                AND g.deleted_at IS NULL
         WHERE r.work_id = $1::uuid
           AND r.allow_quote
           AND r.is_public
           AND r.published_at IS NOT NULL
           AND r.deleted_at IS NULL
           AND p.deleted_at IS NULL
         ORDER BY g.stars DESC NULLS LAST, r.id
         LIMIT 1
        ",
    );

    // Decoded inside each arm rather than after the `match`: `SqliteRow` and
    // `PgRow` are unrelated types, so a single binding cannot hold both. What
    // crosses the arm boundary is plain data, which is also the only way the two
    // engines can be proven to agree.
    let found: Option<(String, String, String, Option<i64>)> = match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(&sql)
                .bind(work_id)
                .fetch_optional(
                    db.sqlite_pool()
                        .ok_or_else(|| anyhow::anyhow!("no sqlite pool"))?,
                )
                .await?;
            // `stars` is NULL when the writer never rated the work, so it is
            // pulled as `Option<i64>`; a non-optional read would be a decode
            // error rather than a NULL, which is the trap
            // `reasons_store::kudos_reason` fell into and had to be fixed for.
            row.map(|r| {
                (
                    r.get::<String, _>("review_id"),
                    r.get::<String, _>("body"),
                    r.get::<String, _>("pseud_handle"),
                    r.get::<Option<i64>, _>("stars"),
                )
            })
        }
        Backend::Postgres => {
            let row = sqlx::query(&sql)
                .bind(work_id)
                .fetch_optional(
                    db.postgres_pool()
                        .ok_or_else(|| anyhow::anyhow!("no postgres pool"))?,
                )
                .await?;
            row.map(|r| {
                (
                    r.get::<String, _>("review_id"),
                    r.get::<String, _>("body"),
                    r.get::<String, _>("pseud_handle"),
                    r.get::<Option<i64>, _>("stars"),
                )
            })
        }
    };

    let Some((review_id, body, pseud_handle, stars)) = found else {
        return Ok(None);
    };
    let (excerpt, truncated) = excerpt(&body);

    Ok(Some(RecBlurb {
        review_id,
        work_id: work_id.to_owned(),
        pseud_handle,
        excerpt,
        truncated,
        stars,
    }))
}

/// Whether a reader has consented to their note being quoted.
///
/// Separate from [`top_rec_blurb`] because the writer needs to read and change
/// their own setting, and a read-only helper gives them no way to set it.
pub async fn quote_consent(db: &Database, review_id: &str) -> Result<Option<bool>> {
    let sql = db.sql(
        "SELECT allow_quote FROM review WHERE id = ?",
        "SELECT allow_quote FROM review WHERE id::text = $1",
    );
    // `allow_quote` is NOT NULL, so it decodes as a plain `bool` on both engines
    // -- SQLite widens 0/1 to false/true. The outer `Option` is only the
    // row-missing case, and the two must not be collapsed into one type: a
    // missing review is not a refusal to quote.
    let found: Option<bool> = match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(&sql)
                .bind(review_id)
                .fetch_optional(
                    db.sqlite_pool()
                        .ok_or_else(|| anyhow::anyhow!("no sqlite pool"))?,
                )
                .await?;
            row.map(|r| r.get::<bool, _>("allow_quote"))
        }
        Backend::Postgres => {
            let row = sqlx::query(&sql)
                .bind(review_id)
                .fetch_optional(
                    db.postgres_pool()
                        .ok_or_else(|| anyhow::anyhow!("no postgres pool"))?,
                )
                .await?;
            row.map(|r| r.get::<bool, _>("allow_quote"))
        }
    };
    Ok(found)
}

/// Set whether a rec note may be quoted.
///
/// Returns false when no such review exists, so a caller cannot create consent
/// for a note that is not there. Consent can only ever be withdrawn or given for
/// an existing note; it is never granted by this call on a row that does not
/// exist.
pub async fn set_quote_consent(db: &Database, review_id: &str, allow: bool) -> Result<bool> {
    // The boolean is bound differently per engine: SQLite has no boolean type,
    // so it takes 1/0, while PostgreSQL takes a real `BOOLEAN`. Binding a Rust
    // `bool` to SQLite stores 1/0 anyway, but spelling it out keeps the two
    // arms honest about which one is doing the converting.
    let updated = match db.backend() {
        Backend::Sqlite => {
            let value = if allow { 1i64 } else { 0i64 };
            sqlx::query("UPDATE review SET allow_quote = ? WHERE id = ?")
                .bind(value)
                .bind(review_id)
                .execute(
                    db.sqlite_pool()
                        .ok_or_else(|| anyhow::anyhow!("no sqlite pool"))?,
                )
                .await?
                .rows_affected()
        }
        Backend::Postgres => sqlx::query("UPDATE review SET allow_quote = $1 WHERE id::text = $2")
            .bind(allow)
            .bind(review_id)
            .execute(
                db.postgres_pool()
                    .ok_or_else(|| anyhow::anyhow!("no postgres pool"))?,
            )
            .await?
            .rows_affected(),
    };
    Ok(updated > 0)
}

#[cfg(test)]
mod tests {
    use super::{excerpt, EXCERPT_CHARS};

    #[test]
    fn a_short_note_is_shown_whole() {
        let (text, truncated) = excerpt("Two of the best things about it.");
        assert_eq!(text, "Two of the best things about it.");
        assert!(
            !truncated,
            "a short note is not truncated, and must not be marked as one"
        );
    }

    #[test]
    fn a_long_note_is_bounded_and_marked() {
        let body = "x".repeat(EXCERPT_CHARS * 3);
        let (text, truncated) = excerpt(&body);
        assert_eq!(
            text.chars().count(),
            EXCERPT_CHARS,
            "the bound must hold exactly"
        );
        assert!(truncated);
        assert!(
            text.ends_with('…'),
            "a truncated pull says so, rather than ending mid-word"
        );
    }

    #[test]
    fn the_bound_is_measured_in_characters_not_bytes() {
        // Every one of these is 3 bytes. A byte-counting implementation would cut
        // this to 240 bytes = 80 characters, which is a third of the note and
        // invisible to an ASCII-only test.
        let body = "あ".repeat(EXCERPT_CHARS + 40);
        let (text, truncated) = excerpt(&body);
        assert_eq!(text.chars().count(), EXCERPT_CHARS);
        assert!(truncated);
    }

    #[test]
    fn a_note_exactly_at_the_bound_is_not_truncated() {
        let body = "y".repeat(EXCERPT_CHARS);
        let (text, truncated) = excerpt(&body);
        assert_eq!(text.chars().count(), EXCERPT_CHARS);
        assert!(!truncated, "at the bound is not over it");
    }
}
