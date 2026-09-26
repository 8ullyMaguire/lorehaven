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
