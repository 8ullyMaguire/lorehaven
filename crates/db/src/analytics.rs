//! Creator dashboard aggregates (spec §32.3, §24.3).
//!
//! What a creator may see about their own works: counts of the things readers
//! did, framed the way §12.9 frames the author's feedback view (what arrived,
//! not who was held back), and banded so a small number cannot name a reader.
//!
//! # Why every count is banded
//!
//! A creator with three comments can often work out who wrote them from the
//! work's public page; a creator with three *bookmarks* can sometimes work out
//! who saved it from nothing at all, because the number itself is the
//! information. §24.3's rule is that these aggregates are not personal data
//! about the readers in them unless they identify someone, so a count below the
//! floor is reported as a band (`fewer_than_floor`) rather than as a number.
//! The dashboard never returns a reader, a pseud, an account or a per-reader
//! row — only totals over the caller's own works.

use sqlx::Row;

use crate::{sql_owned, Backend, Database};

/// The smallest count this dashboard reports exactly.
///
/// Five is the same order as the public rating aggregate's minimum, so an
/// author does not see a private number their readers' own surfaces hide.
pub const CREATOR_DASHBOARD_FLOOR: i64 = 5;

/// The raw aggregates, before banding.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreatorTotals {
    pub works: i64,
    pub published: i64,
    pub unpublished: i64,
    pub chapters: i64,
    pub words: i64,
    pub ratings: i64,
    pub rating_stars: i64,
    pub reviews: i64,
    pub bookmarks: i64,
    pub comments: i64,
    /// Comments the positivity filter delivered, including comments it never
    /// classified (rules-only mode, where an absent classification is a
    /// delivered one).
    pub comments_delivered: i64,
}

/// Aggregate the caller's own works.
///
/// `owner_pseud` is the acting pseud: a creator's dashboard is about the works
/// that pseud owns, not about every work their account can edit.
pub async fn creator_totals(db: &Database, owner_pseud: &str) -> anyhow::Result<CreatorTotals> {
    let works = work_totals(db, owner_pseud).await?;
    let readers = reader_totals(db, owner_pseud).await?;
    let comments = comment_totals(db, owner_pseud).await?;

    Ok(CreatorTotals {
        works: works.0,
        published: works.1,
        unpublished: works.2,
        chapters: works.3,
        words: works.4,
        ratings: readers.0,
        rating_stars: readers.1,
        reviews: readers.2,
        bookmarks: readers.3,
        comments: comments.0,
        comments_delivered: comments.1,
    })
}

const WORK_TOTALS_SQLITE: &str = "SELECT
     COUNT(*) AS works,
     COALESCE(SUM(CASE WHEN w.lifecycle = 'published' THEN 1 ELSE 0 END), 0) AS published,
     COALESCE(SUM(CASE WHEN w.lifecycle != 'published' THEN 1 ELSE 0 END), 0) AS unpublished,
     (SELECT COUNT(*) FROM chapters c
        JOIN works w2 ON w2.id = c.work_id
       WHERE w2.owner_pseud_id = ?) AS chapters,
     (SELECT COALESCE(CAST(SUM(cr.word_count) AS BIGINT), 0)
        FROM chapters c
        JOIN chapter_revisions cr ON cr.id = c.current_revision_id
        JOIN works w2 ON w2.id = c.work_id
       WHERE w2.owner_pseud_id = ?) AS words
 FROM works w
 WHERE w.owner_pseud_id = ?";

const WORK_TOTALS_POSTGRES: &str = "SELECT
     COUNT(*) AS works,
     COALESCE(SUM(CASE WHEN w.lifecycle = 'published' THEN 1 ELSE 0 END), 0) AS published,
     COALESCE(SUM(CASE WHEN w.lifecycle != 'published' THEN 1 ELSE 0 END), 0) AS unpublished,
     (SELECT COUNT(*) FROM chapters c
        JOIN works w2 ON w2.id = c.work_id
       WHERE w2.owner_pseud_id = ?::uuid) AS chapters,
     (SELECT COALESCE(CAST(SUM(cr.word_count) AS BIGINT), 0)
        FROM chapters c
        JOIN chapter_revisions cr ON cr.id = c.current_revision_id
        JOIN works w2 ON w2.id = c.work_id
       WHERE w2.owner_pseud_id = ?::uuid) AS words
 FROM works w
 WHERE w.owner_pseud_id = ?::uuid";

/// Works, chapters and words owned by the acting pseud.
async fn work_totals(
    db: &Database,
    owner_pseud: &str,
) -> anyhow::Result<(i64, i64, i64, i64, i64)> {
    let sql = sql_owned(
        db,
        WORK_TOTALS_SQLITE.to_string(),
        WORK_TOTALS_POSTGRES.to_string(),
    );
    match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(&sql)
                .bind(owner_pseud)
                .bind(owner_pseud)
                .bind(owner_pseud)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            Ok((
                row.get::<i64, _>("works"),
                row.get::<i64, _>("published"),
                row.get::<i64, _>("unpublished"),
                row.get::<i64, _>("chapters"),
                row.get::<i64, _>("words"),
            ))
        }
        Backend::Postgres => {
            let row = sqlx::query(&sql)
                .bind(owner_pseud)
                .bind(owner_pseud)
                .bind(owner_pseud)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?;
            Ok((
                row.get::<i64, _>("works"),
                row.get::<i64, _>("published"),
                row.get::<i64, _>("unpublished"),
                row.get::<i64, _>("chapters"),
                row.get::<i64, _>("words"),
            ))
        }
    }
}

const READER_TOTALS_SQLITE: &str = "SELECT
     (SELECT COUNT(*) FROM rating r JOIN works w ON w.id = r.work_id
       WHERE w.owner_pseud_id = ? AND r.is_public = 1 AND r.deleted_at IS NULL) AS ratings,
     (SELECT COALESCE(CAST(SUM(r.stars) AS BIGINT), 0) FROM rating r JOIN works w ON w.id = r.work_id
       WHERE w.owner_pseud_id = ? AND r.is_public = 1 AND r.deleted_at IS NULL) AS rating_stars,
     (SELECT COUNT(*) FROM review rv JOIN works w ON w.id = rv.work_id
       WHERE w.owner_pseud_id = ? AND rv.is_public = 1
         AND rv.published_at IS NOT NULL AND rv.deleted_at IS NULL) AS reviews,
     (SELECT COUNT(*) FROM bookmarks b JOIN works w ON w.id = b.subject_id
       WHERE w.owner_pseud_id = ? AND b.subject_type = 'work') AS bookmarks";

// `rating.is_public` and `review.is_public` are BOOLEAN on PostgreSQL and
// INTEGER in the SQLite twin (0004), so the two arms need different
// literals: `= 1` is a type error here, and `= TRUE` is not valid
// SQLite. Same split for `owner_pseud_id`, which is UUID here.
const READER_TOTALS_POSTGRES: &str = "SELECT
     (SELECT COUNT(*) FROM rating r JOIN works w ON w.id = r.work_id
       WHERE w.owner_pseud_id = ?::uuid AND r.is_public = TRUE AND r.deleted_at IS NULL) AS ratings,
     (SELECT COALESCE(CAST(SUM(r.stars) AS BIGINT), 0) FROM rating r JOIN works w ON w.id = r.work_id
       WHERE w.owner_pseud_id = ?::uuid AND r.is_public = TRUE AND r.deleted_at IS NULL) AS rating_stars,
     (SELECT COUNT(*) FROM review rv JOIN works w ON w.id = rv.work_id
       WHERE w.owner_pseud_id = ?::uuid AND rv.is_public = TRUE
         AND rv.published_at IS NOT NULL AND rv.deleted_at IS NULL) AS reviews,
     (SELECT COUNT(*) FROM bookmarks b JOIN works w ON w.id = b.subject_id
       WHERE w.owner_pseud_id = ?::uuid AND b.subject_type = 'work') AS bookmarks";

/// Public ratings, published reviews and bookmarks on the caller's works.
async fn reader_totals(db: &Database, owner_pseud: &str) -> anyhow::Result<(i64, i64, i64, i64)> {
    let sql = sql_owned(
        db,
        READER_TOTALS_SQLITE.to_string(),
        READER_TOTALS_POSTGRES.to_string(),
    );
    match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(&sql)
                .bind(owner_pseud)
                .bind(owner_pseud)
                .bind(owner_pseud)
                .bind(owner_pseud)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            Ok((
                row.get::<i64, _>("ratings"),
                row.get::<i64, _>("rating_stars"),
                row.get::<i64, _>("reviews"),
                row.get::<i64, _>("bookmarks"),
            ))
        }
        Backend::Postgres => {
            let row = sqlx::query(&sql)
                .bind(owner_pseud)
                .bind(owner_pseud)
                .bind(owner_pseud)
                .bind(owner_pseud)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?;
            Ok((
                row.get::<i64, _>("ratings"),
                row.get::<i64, _>("rating_stars"),
                row.get::<i64, _>("reviews"),
                row.get::<i64, _>("bookmarks"),
            ))
        }
    }
}

/// Comments on the caller's works, and how many the positivity filter delivered.
///
/// A comment with no classification row was never classified (rules-only mode
/// or a reaction-class surface), and an unclassified comment is a delivered
/// one: the filter is not a queue that holds everything it has not judged.
const COMMENT_TOTALS_SQLITE: &str = "SELECT
     COUNT(*) AS comments,
     COALESCE(SUM(CASE WHEN cc.outcome IS NULL OR cc.outcome = 'delivered' THEN 1 ELSE 0 END), 0)
         AS comments_delivered
 FROM comments c
 JOIN works w ON w.id = c.subject_id
 LEFT JOIN comment_classifications cc ON cc.comment_id = c.id
 WHERE w.owner_pseud_id = ? AND c.subject_type = 'work' AND c.deleted_at IS NULL";

// `comments.subject_id` is TEXT on both engines (0013) and `works.id` is
// UUID on PostgreSQL, so the join needs the *text* side cast --
// "operator does not exist: uuid = text". `cc.comment_id` and
// `comments.id` are both TEXT, so that join is left alone.
const COMMENT_TOTALS_POSTGRES: &str = "SELECT
     COUNT(*) AS comments,
     COALESCE(SUM(CASE WHEN cc.outcome IS NULL OR cc.outcome = 'delivered' THEN 1 ELSE 0 END), 0)
         AS comments_delivered
 FROM comments c
 JOIN works w ON w.id = c.subject_id::uuid
 LEFT JOIN comment_classifications cc ON cc.comment_id = c.id
 WHERE w.owner_pseud_id = ?::uuid AND c.subject_type = 'work' AND c.deleted_at IS NULL";

async fn comment_totals(db: &Database, owner_pseud: &str) -> anyhow::Result<(i64, i64)> {
    let sql = sql_owned(
        db,
        COMMENT_TOTALS_SQLITE.to_string(),
        COMMENT_TOTALS_POSTGRES.to_string(),
    );
    match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(&sql)
                .bind(owner_pseud)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            Ok((
                row.get::<i64, _>("comments"),
                row.get::<i64, _>("comments_delivered"),
            ))
        }
        Backend::Postgres => {
            let row = sqlx::query(&sql)
                .bind(owner_pseud)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?;
            Ok((
                row.get::<i64, _>("comments"),
                row.get::<i64, _>("comments_delivered"),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_floor_is_the_public_aggregate_order() {
        // Not a magic number: it is the same order as the public rating
        // aggregate's minimum, so a creator cannot see through this door a
        // number their readers' own surfaces hide.
        assert_eq!(CREATOR_DASHBOARD_FLOOR, 5);
    }
}
