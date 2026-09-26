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
/// Ten, not five. Every count this dashboard bands — bookmarks, ratings,
/// reviews, delivered comments — is a count of *other people* acting on
/// someone's work, and §36.12 sets the floor for author-facing analytics at
/// ten. The earlier value of five came from the public rating aggregate's
/// order of magnitude, which is a floor on a number a reader is about
/// themselves, not on a number about a room full of strangers.
///
/// The direction of the change matters: a reader whose own dashboard breaks
/// down their own reading at five must not meet the same five people in an
/// author's breakdown. The threshold has to move stricter as the subject gets
/// less personal, never looser.
pub const CREATOR_DASHBOARD_FLOOR: i64 = lorehaven_domain::analytics::K_OTHERS;

/// A reader count, with the floor applied at the query.
///
/// # Why this type and not an `i64`
///
/// The obvious shape is "run the count, return a number, let the route band
/// it" — which is what [`crate::analytics::creator_totals`] plus the route
/// still does. That has already had the privacy incident before the route
/// runs: the rows were read, the driver had them, and the access log saw the
/// query. The question is never whether the *response* mentions the small
/// number.
///
/// So this count is produced as an aggregate and the floor is applied to the
/// aggregate, and the type makes the suppressed case unrepresentable as a
/// number. There is no `count: 3` a caller could `unwrap_or(0)`, because
/// "too few people to tell you" and "nobody did this" are different claims and
/// only the first is true.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ReaderCount {
    /// The capability's stable name, so a client can key its own copy on it.
    pub scope: &'static str,
    /// "self" or "other". For a work's audience this is always "other" — the
    /// readers are not the author, even when the author is asking.
    pub subject: &'static str,
    /// The true count, or `None` below the floor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<i64>,
    /// The floor, present exactly when `count` is absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fewer_than: Option<i64>,
    /// §24.2's documented computation, carried with the number.
    pub method: MethodDoc,
    /// When this was computed. A number with no freshness looks live forever.
    pub computed_at: String,
}

/// The three parts of §24.2's method requirement, on the wire.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MethodDoc {
    pub definition: &'static str,
    pub freshness: &'static str,
    pub approximation: &'static str,
}

impl MethodDoc {
    fn of(scope: lorehaven_domain::analytics::Scope) -> Self {
        let m = scope.method();
        Self {
            definition: m.definition,
            freshness: m.freshness.as_str(),
            approximation: m.approximation,
        }
    }
}

impl ReaderCount {
    /// Build from a raw count, applying `scope`'s floor.
    pub fn build(count: i64, scope: lorehaven_domain::analytics::Scope) -> Self {
        use lorehaven_domain::analytics::{Reported, Subject};
        let (exact, fewer_than) = match Reported::of(count, scope) {
            Reported::Exact(n) => (Some(n), None),
            Reported::BelowFloor { fewer_than } => (None, Some(fewer_than)),
        };
        Self {
            scope: scope.as_str(),
            subject: match scope.subject() {
                Subject::Self_ => "self",
                Subject::Other => "other",
            },
            count: exact,
            fewer_than,
            method: MethodDoc::of(scope),
            computed_at: crate::sessions::now(),
        }
    }
}

const WORK_READERS_SQLITE: &str = "SELECT COUNT(DISTINCT pseud_id) \
     FROM reading_history_entry \
     WHERE subject_type = 'work' AND subject_id = ?";

// `subject_id` is a UUID column on PostgreSQL, so a text bind compares uuid to
// text and raises rather than matching nothing. Cast at the bind, not the
// column: the column really is a uuid and pretending otherwise would break the
// index.
const WORK_READERS_POSTGRES: &str = "SELECT COUNT(DISTINCT pseud_id) \
     FROM reading_history_entry \
     WHERE subject_type = 'work' AND subject_id = ?::uuid";

/// Distinct readers of a work, with the floor already applied.
///
/// A reader is a distinct **pseudonym** with a reading event, not an account
/// and not a session:
///
/// * An account is one person, and counting accounts invites exactly the
///   joins that §7.2 forbids.
/// * A reader with two pseudonyms is two entries in this work's audience. A
///   cosplayer is not half a reader; the number is about the work's reach.
///
/// `COUNT(DISTINCT …)` is what makes a reader who opens a work forty times
/// count once. Counting events is how a small work invents an audience.
///
/// Note the absence of a `min_readers` parameter. Adding one would be a
/// regression: a caller who can pass a floor of zero has the floor switched
/// off, and the signature is the only thing preventing that.
pub async fn work_reader_count(db: &Database, work_id: &str) -> anyhow::Result<ReaderCount> {
    let scope = lorehaven_domain::analytics::Scope::OwnWorkBasic;
    let sql = sql_owned(
        db,
        WORK_READERS_SQLITE.to_string(),
        WORK_READERS_POSTGRES.to_string(),
    );
    let readers: i64 = match db.backend() {
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
    Ok(ReaderCount::build(readers, scope))
}

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

// -- own.reading.basic (spec §9.6) --------------------------------------------

/// The largest gap between two progress updates that counts as reading.
///
/// §9.6 asks for "approximate reading time" and the method for
/// `own.reading.basic` documents the approximation as "wall-clock between two
/// progress updates on the same chapter, capped at 30 minutes per gap". The
/// cap is not a rounding convenience: a reader who left a tab open overnight,
/// or closed a laptop with the reader still open, has contributed zero reading
/// and eight hours of wall-clock, and a dashboard that reports the second is
/// telling a reader something false about themselves.
pub const READING_GAP_CAP_SECONDS: i64 = 30 * 60;

/// A reader's own reading totals, as §9.6 lists them.
///
/// Every field is a fact about the caller. `Subject::Self_` means the floor
/// does not apply, so these are exact numbers and never bands — a reader who
/// knows they finished two works must not be told "fewer than 5".
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct ReadingTotals {
    /// Works the reader marked finished. A decision, not an open (§9.6).
    pub finished_works: i64,
    /// Chapters the reader advanced progress through.
    pub chapters_read: i64,
    /// Estimated words in those chapters. An estimate: it is the word count
    /// of the revision as stored now, not as read then.
    pub words_read: i64,
    /// Capped wall-clock between progress updates. An estimate, per the cap.
    pub reading_seconds: i64,
}

const READING_TOTALS_SQLITE: &str = "SELECT
     (SELECT COUNT(*) FROM reading_status rs
        WHERE rs.account_id = ?
          AND rs.subject_type = 'work'
          AND rs.status = 'finished') AS finished_works,
     (SELECT COUNT(*) FROM reading_progress rp
        WHERE rp.account_id = ?
          AND rp.subject_type = 'work') AS chapters_read,
     (SELECT COALESCE(CAST(SUM(word_total) AS BIGINT), 0) FROM (
        SELECT (SELECT COALESCE(CAST(SUM(cr.word_count) AS BIGINT), 0)
                  FROM chapters c
                  JOIN chapter_revisions cr ON cr.id = c.current_revision_id
                 WHERE c.work_id = rp.subject_id) AS word_total
          FROM reading_progress rp
         WHERE rp.account_id = ?
           AND rp.subject_type = 'work')) AS words_read,
     (SELECT COALESCE(CAST(SUM(MIN(gap, 1800)) AS BIGINT), 0) FROM (
        SELECT (julianday(nxt.created_at) - julianday(prev.created_at)) * 86400.0 AS gap
          FROM reading_progress prev
          JOIN reading_progress nxt
            ON nxt.account_id = prev.account_id
           AND nxt.subject_id = prev.subject_id
           AND nxt.created_at > prev.created_at
         WHERE prev.account_id = ?
           AND prev.subject_type = 'work')) AS reading_seconds";

const READING_TOTALS_POSTGRES: &str = "SELECT
     (SELECT COUNT(*) FROM reading_status rs
        WHERE rs.account_id = $1::uuid
          AND rs.subject_type = 'work'
          AND rs.status = 'finished') AS finished_works,
     (SELECT COUNT(*) FROM reading_progress rp
        WHERE rp.account_id = $2::uuid
          AND rp.subject_type = 'work') AS chapters_read,
     (SELECT COALESCE(CAST(SUM(word_total) AS BIGINT), 0) FROM (
        SELECT (SELECT COALESCE(CAST(SUM(cr.word_count) AS BIGINT), 0)
                  FROM chapters c
                  JOIN chapter_revisions cr ON cr.id = c.current_revision_id
                 WHERE c.work_id = rp.subject_id) AS word_total
          FROM reading_progress rp
         WHERE rp.account_id = $3::uuid
           AND rp.subject_type = 'work')) AS words_read,
     (SELECT COALESCE(CAST(SUM(LEAST(gap, 1800)) AS BIGINT), 0) FROM (
        SELECT EXTRACT(EPOCH FROM (nxt.created_at::timestamptz - prev.created_at::timestamptz)) AS gap
          FROM reading_progress prev
          JOIN reading_progress nxt
            ON nxt.account_id = prev.account_id
           AND nxt.subject_id = prev.subject_id
           AND nxt.created_at > prev.created_at
         WHERE prev.account_id = $4::uuid
           AND prev.subject_type = 'work')) AS reading_seconds";

/// The caller's own reading totals.
///
/// `account_id` is filtered in SQL, never in the route: a post-filter has
/// already read the rows, and this is the shape of leak the whole registry
/// exists to prevent.
///
/// Four subqueries rather than one pass, because each is a different table and
/// a reader who has opened nothing must still get zeros rather than a row
/// that the outer aggregate then has to coalesce. Four binds of the same
/// account id is the price, and the alternative — a `UNION ALL` of four
/// aggregates — is one statement that reads worse than four named counts.
pub async fn reading_totals(db: &Database, account_id: &str) -> anyhow::Result<ReadingTotals> {
    let sql = sql_owned(
        db,
        READING_TOTALS_SQLITE.to_string(),
        READING_TOTALS_POSTGRES.to_string(),
    );
    // Each arm reads its own row and returns the same plain struct. A row
    // cannot cross the `match`: `QueryResult<Sqlite, _>` and
    // `QueryResult<Postgres, _>` are different types, and `AnyRow` needs the
    // `any` feature this workspace does not enable.
    match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(&sql)
                .bind(account_id)
                .bind(account_id)
                .bind(account_id)
                .bind(account_id)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            Ok(totals_from(
                row.get::<i64, _>("finished_works"),
                row.get::<i64, _>("chapters_read"),
                row.get::<i64, _>("words_read"),
                row.get::<i64, _>("reading_seconds"),
            ))
        }
        Backend::Postgres => {
            let row = sqlx::query(&sql)
                .bind(account_id)
                .bind(account_id)
                .bind(account_id)
                .bind(account_id)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?;
            Ok(totals_from(
                row.get::<i64, _>("finished_works"),
                row.get::<i64, _>("chapters_read"),
                row.get::<i64, _>("words_read"),
                row.get::<i64, _>("reading_seconds"),
            ))
        }
    }
}

/// The one place the four counts become a struct.
///
/// `COUNT` is BIGINT on PostgreSQL and the `MIN`/`LEAST` sum is NUMERIC, so
/// every field is cast in the statement rather than decoded and converted here:
/// a decode error names the column, and `i64` in both arms is what lets the
/// two dialects agree.
fn totals_from(
    finished_works: i64,
    chapters_read: i64,
    words_read: i64,
    reading_seconds: i64,
) -> ReadingTotals {
    ReadingTotals {
        finished_works,
        chapters_read,
        words_read,
        reading_seconds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_floor_is_the_stricter_others_value() {
        // Not a magic number, and not five. Every count this module bands is a
        // count of other people, and §36.12 puts author-facing analytics at
        // ten. The earlier value of five matched the public rating aggregate's
        // order of magnitude, which is a floor on a number a reader is about
        // themselves -- not on a count of a room full of strangers.
        assert_eq!(CREATOR_DASHBOARD_FLOOR, 10);
        assert_eq!(
            CREATOR_DASHBOARD_FLOOR,
            lorehaven_domain::analytics::K_OTHERS
        );
    }

    #[test]
    fn a_suppressed_reader_count_is_not_a_number_on_the_wire() {
        let v = ReaderCount::build(7, lorehaven_domain::analytics::Scope::OwnWorkBasic);
        let json = serde_json::to_string(&v).unwrap();
        assert!(!json.contains("\"count\""), "a count leaked: {json}");
        assert!(json.contains("\"fewer_than\":10"), "{json}");
    }

    #[test]
    fn an_exact_reader_count_carries_no_floor() {
        let json = serde_json::to_string(&ReaderCount::build(
            40,
            lorehaven_domain::analytics::Scope::OwnWorkBasic,
        ))
        .unwrap();
        assert!(json.contains("\"count\":40"), "{json}");
        assert!(!json.contains("fewer_than"), "{json}");
    }

    #[test]
    fn the_method_and_freshness_travel_with_the_number() {
        // §24.2. A count with no definition and no timestamp is not
        // reviewable, and a reader cannot tell a stale number from a wrong one.
        let v = ReaderCount::build(7, lorehaven_domain::analytics::Scope::OwnWorkBasic);
        assert!(v.method.definition.contains("pseudonym"));
        assert!(!v.method.freshness.is_empty());
        assert!(!v.method.approximation.is_empty());
        assert!(!v.computed_at.is_empty());
    }

    #[test]
    fn a_single_entity_fact_is_not_floored() {
        // A work's own word count is not a k-anonymity problem, and
        // "fewer than 10 words" would be absurd.
        let v = ReaderCount::build(3, lorehaven_domain::analytics::Scope::PublicWorkBasic);
        assert_eq!(v.count, Some(3));
        assert_eq!(v.fewer_than, None);
    }
}

// -- own.reading.trend (spec §9.6) -------------------------------------------

/// How many weeks `own.reading.trend` reports.
///
/// Four, matching the `Method` text's "trailing window". A reader wants to see
/// "this week so far" beside a comparable recent run, and a longer series
/// mostly shows a reader their own absence.
pub const READING_TREND_WEEKS: i64 = 4;

/// One week of a reader's own reading.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TrendWeek {
    /// The Monday of the week, as `YYYY-MM-DD`. A date and not an instant
    /// because a week is a calendar thing, and a reader in any timezone reads
    /// the same Monday.
    pub week_start: String,
    /// Progress updates in the week. §9.6's "reads": advances, not opens.
    pub reads: i64,
    /// Finished works in the week, from `reading_status`. A decision, so it
    /// moves at most once per work per week.
    pub finished_works: i64,
}

// ISO weeks start on Monday. On both backends the floor is computed as
// "the most recent Monday at or before the row's date", so the two agree on
// which week a row belongs to rather than merely producing a similar
// distribution.
//
// SQLite: `strftime('%w')` is 0 for Sunday and 6 for Saturday, so the offset
// to the preceding Monday is `(%w + 6) % 7`. Postgres: `date_trunc('week', …)`
// is already Monday-based, and `EXTRACT(ISODOW …)` agrees with it.
//
// The weeks themselves come from a generated series rather than from the rows,
// so a week with no reads is still reported. That is the whole point of the
// capability: a reader who has read nothing this week must see a zero for it,
// not a gap that reads as missing data.
// Two SQLite details, both found the hard way.
//
// **No `weeks` modifier.** SQLite's date modifiers are `NNN days`, `NNN hours`,
// `NNN minutes`, `NNN seconds`, `NNN months`, `NNN years` — there is no
// `weeks`, in any version. `date('2026-09-21', '-1 weeks')` returns NULL,
// silently: no error, no warning, just a series of empty week labels and a
// trend of four zeroes, which looks exactly like a working query against an
// empty database.
//
// **All placeholders anonymous, never `?1` mixed with `?`.** libsqlite3 numbers
// anonymous placeholders from the highest explicit index it has already seen,
// so a statement mixing `?1` with bare `?` does not number the bare ones the
// way a reader expects, and the binds land in the wrong slots. With the week
// anchor bound in first and the account twice after, the account id was
// compared against the *date*, every week matched nothing, and the capability
// reported four zeroes over a table that demonstrably held the reader's rows.
// Every placeholder here is a bare `?`, bound positionally, which is the only
// form that is unambiguous.
const READING_TREND_SQLITE: &str = "WITH weeks(week_start) AS (
     SELECT date(?, '-' || (value * 7) || ' days')
       FROM (SELECT 0 AS value UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3)
   )
   SELECT w.week_start AS week_start,
          (SELECT COUNT(*) FROM reading_progress rp
             WHERE rp.account_id = ?
               AND rp.subject_type = 'work'
               AND date(rp.created_at) >= w.week_start
               AND date(rp.created_at) < date(w.week_start, '+7 days')) AS reads,
          (SELECT COUNT(*) FROM reading_status rs
             WHERE rs.account_id = ?
               AND rs.subject_type = 'work'
               AND rs.status = 'finished'
               AND date(rs.updated_at) >= w.week_start
               AND date(rs.updated_at) < date(w.week_start, '+7 days')) AS finished_works
     FROM weeks w
    ORDER BY w.week_start";

// `rp.created_at::timestamptz`, not `rp.created_at`.
//
// Every timestamp column in this schema is TEXT on both backends -- RFC 3339 in
// a TEXT column, which is what lets one seed string be stored either way. So a
// Postgres comparison against a `date` needs the cast spelled out, and omitting
// it is a hard error rather than a wrong answer:
//
//     ERROR: operator does not exist: text >= date
//
// which surfaces to the reader as a 500, not as an empty trend. `::timestamptz`
// is the same cast `reading_totals` already uses on this file for the same
// column, so the two capabilities cannot drift apart.
const READING_TREND_POSTGRES: &str = "WITH weeks(week_start) AS (
     SELECT $2::date - (value * 7)::int AS week_start
       FROM (VALUES (0), (1), (2), (3)) AS seq(value)
   )
   SELECT to_char(w.week_start, 'YYYY-MM-DD') AS week_start,
          (SELECT COUNT(*) FROM reading_progress rp
             WHERE rp.account_id = $1::uuid
               AND rp.subject_type = 'work'
               AND rp.created_at::timestamptz >= w.week_start
               AND rp.created_at::timestamptz < w.week_start + 7) AS reads,
          (SELECT COUNT(*) FROM reading_status rs
             WHERE rs.account_id = $1::uuid
               AND rs.subject_type = 'work'
               AND rs.status = 'finished'
               AND rs.updated_at::timestamptz >= w.week_start
               AND rs.updated_at::timestamptz < w.week_start + 7) AS finished_works
     FROM weeks w
    ORDER BY w.week_start";

/// The caller's own reading trend, one entry per week, oldest first.
///
/// `account_id` is filtered in SQL for the same reason as `reading_totals`: a
/// post-filter has already read the rows, and this is the leak the whole
/// registry exists to prevent.
/// `week_start` is the most recent Monday, as `YYYY-MM-DD`, and the caller
/// supplies it.
///
/// Passed in rather than read from a clock here: `lorehaven-db` has no `chrono`
/// dependency, and the alternative is adding one to the crate for a single date
/// arithmetic that every caller already needs to do. It also makes the query
/// testable — a test can ask for the weeks around a known Monday.
pub async fn reading_trend(
    db: &Database,
    account_id: &str,
    week_start: &str,
) -> anyhow::Result<Vec<TrendWeek>> {
    let sql = sql_owned(
        db,
        READING_TREND_SQLITE.to_string(),
        READING_TREND_POSTGRES.to_string(),
    );
    // The week series is generated by the query, so the result is never empty
    // and a reader who has read nothing still gets their zeros. That is a
    // property of the SQL, and this assert states it rather than trusting it.
    macro_rules! collect {
        ($rows:expr) => {{
            let rows = $rows;
            let weeks: Vec<TrendWeek> = rows
                .iter()
                .map(|row| TrendWeek {
                    week_start: row.get::<String, _>("week_start"),
                    reads: row.get::<i64, _>("reads"),
                    finished_works: row.get::<i64, _>("finished_works"),
                })
                .collect();
            anyhow::ensure!(
                weeks.len() as i64 == READING_TREND_WEEKS,
                "the trend must report {READING_TREND_WEEKS} weeks, got {}",
                weeks.len()
            );
            weeks
        }};
    }
    match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(&sql)
                .bind(week_start)
                .bind(account_id)
                .bind(account_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            Ok(collect!(rows))
        }
        Backend::Postgres => {
            let rows = sqlx::query(&sql)
                .bind(account_id)
                .bind(week_start)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?;
            Ok(collect!(rows))
        }
    }
}
