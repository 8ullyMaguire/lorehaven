//! Reading progress, ratings, reviews, history, notes and typography preferences
//! (spec §9).
//!
//! The rule that shapes this module: **private reading data never reaches another
//! account.** Every `reading_progress`, `reading_history_entry`, `reader_note`
//! and `rating` row is scoped to a single account (and, for notes and ratings,
//! to a single *pseud* — the face that wrote them). Nothing here joins across
//! accounts, and the public-rating aggregate is the only function that returns
//! a value derived from another account's rows. It enforces the minimum-count
//! threshold and the `is_public` flag, so a stray private row can never leak.
//!
//! As everywhere else in this crate, every statement is written once per dialect
//! and binds only `String`/`i64`, so rows decode identically on SQLite and
//! PostgreSQL (ADR 0004).

use anyhow::{Context, Result};
use sqlx::FromRow;

use lorehaven_domain::reading::ReadingPosition;
use lorehaven_domain::{AccountId, PseudId, RevisionId, WorkId};

use crate::identity::now_rfc3339;
use crate::{sql_owned, Backend, Database};

// ---------------------------------------------------------------------------
// Reading progress
// ---------------------------------------------------------------------------

/// A stored position row, as decoded.
#[derive(Debug, Clone, FromRow)]
struct ProgressRow {
    content_revision: Option<String>,
    anchor: Option<String>,
    position_permille: i64,
    device_id: Option<String>,
}

impl ProgressRow {
    fn decode(self) -> ReadingPosition {
        ReadingPosition {
            revision: self
                .content_revision
                .as_deref()
                .and_then(|r| r.parse().ok()),
            anchor: self.anchor,
            fraction: u16::try_from(self.position_permille).unwrap_or(0),
            device: self.device_id,
        }
    }
}

/// Everything [`save_progress`] needs, in one value.
///
/// A struct rather than eight positional arguments: five of them are
/// `Option<&str>`-shaped and two are ids, and a call site that passed them in
/// the wrong order would still compile.
pub struct ProgressInput<'a> {
    pub account: AccountId,
    /// The face that read the work, when one was acting.
    pub pseud: Option<PseudId>,
    pub subject_type: &'a str,
    pub subject_id: &'a str,
    pub chapter_id: Option<&'a str>,
    /// The revision the reader was looking at.
    pub revision: Option<RevisionId>,
    pub anchor: Option<&'a str>,
    /// Position in permille (0..=1000).
    pub fraction: u16,
    pub device: Option<&'a str>,
}

/// Upsert this device's position for a subject.
///
/// The upsert is keyed on `(account, pseud, subject, device)`; a NULL device
/// id is treated as its own key, so two sessions with no device id share one
/// row rather than colliding.
pub async fn save_progress(db: &Database, input: ProgressInput<'_>) -> Result<()> {
    let ProgressInput {
        account,
        pseud,
        subject_type,
        subject_id,
        chapter_id,
        revision,
        anchor,
        fraction,
        device,
    } = input;

    let id = uuid::Uuid::new_v4().to_string();
    let now = now_rfc3339();
    let revision_text = revision.as_ref().map(ToString::to_string);
    let fraction_i64 = i64::from(fraction);

    /*
     * Two dialects, two upsert keys.
     *
     * SQLite accepts a conflict target only when it names a unique index
     * exactly, and its two partial indexes are not the ones PostgreSQL's are
     * — the parenthesised key differs between the engines — so the SQLite
     * statement stays bare and lets the engine pick. PostgreSQL infers a
     * *partial* index only when the conflict target carries the index's own
     * predicate, and which of the two indexes applies depends on whether the
     * request has a device id, so the PostgreSQL variant is chosen here rather
     * than by `db.sql`'s single string.
     */
    let postgres_sql = if device.is_some() {
        "INSERT INTO reading_progress
             (id, account_id, pseud_id, subject_type, subject_id, chapter_id,
              content_revision, paragraph_anchor, position_permille, device_id,
              created_at, updated_at, version)
         VALUES (?::uuid, ?::uuid, ?::uuid, ?, ?::uuid, ?::uuid,
                 ?::uuid, ?, ?, ?, ?, ?, 1)
         ON CONFLICT (account_id, pseud_id, subject_type, subject_id, device_id)
             WHERE device_id IS NOT NULL
         DO UPDATE SET
              chapter_id = excluded.chapter_id,
              content_revision = excluded.content_revision,
              paragraph_anchor = excluded.paragraph_anchor,
              position_permille = excluded.position_permille,
              updated_at = excluded.updated_at,
              version = reading_progress.version + 1"
    } else {
        "INSERT INTO reading_progress
             (id, account_id, pseud_id, subject_type, subject_id, chapter_id,
              content_revision, paragraph_anchor, position_permille, device_id,
              created_at, updated_at, version)
         VALUES (?::uuid, ?::uuid, ?::uuid, ?, ?::uuid, ?::uuid,
                 ?::uuid, ?, ?, ?, ?, ?, 1)
         ON CONFLICT (account_id, pseud_id, subject_type, subject_id)
             WHERE device_id IS NULL
         DO UPDATE SET
              chapter_id = excluded.chapter_id,
              content_revision = excluded.content_revision,
              paragraph_anchor = excluded.paragraph_anchor,
              position_permille = excluded.position_permille,
              updated_at = excluded.updated_at,
              version = reading_progress.version + 1"
    };

    let sql = db.sql(
        "INSERT INTO reading_progress
             (id, account_id, pseud_id, subject_type, subject_id, chapter_id,
              content_revision, paragraph_anchor, position_permille, device_id,
              created_at, updated_at, version)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1)
         ON CONFLICT DO UPDATE SET
              chapter_id = excluded.chapter_id,
              content_revision = excluded.content_revision,
              paragraph_anchor = excluded.paragraph_anchor,
              position_permille = excluded.position_permille,
              updated_at = excluded.updated_at,
              version = reading_progress.version + 1",
        postgres_sql,
    );

    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite handle");
            let mut tx = pool.begin().await?;
            // When there is no device id, the partial unique index on
            // (account, pseud, subject) fires only if no such row exists.
            sqlx::query(&sql)
                .bind(&id)
                .bind(account.to_string())
                .bind(pseud.map(|p| p.to_string()))
                .bind(subject_type)
                .bind(subject_id)
                .bind(chapter_id)
                .bind(revision_text.as_deref())
                .bind(anchor)
                .bind(fraction_i64)
                .bind(device)
                .bind(&now)
                .bind(&now)
                .execute(&mut *tx)
                .await
                .context("upserting reading progress")?;
            tx.commit().await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            let mut tx = pool.begin().await?;
            sqlx::query(&sql)
                .bind(&id)
                .bind(account.to_string())
                .bind(pseud.map(|p| p.to_string()))
                .bind(subject_type)
                .bind(subject_id)
                .bind(chapter_id)
                .bind(revision_text.as_deref())
                .bind(anchor)
                .bind(fraction_i64)
                .bind(device)
                .bind(&now)
                .bind(&now)
                .execute(&mut *tx)
                .await
                .context("upserting reading progress")?;
            tx.commit().await?;
        }
    }
    Ok(())
}

/// Every stored position for a subject for this account, one per device.
pub async fn progress_for(
    db: &Database,
    account: AccountId,
    subject_type: &str,
    subject_id: &str,
) -> Result<Vec<ReadingPosition>> {
    let sql = db.sql(
        "SELECT content_revision, paragraph_anchor AS anchor, position_permille, device_id
           FROM reading_progress
          WHERE account_id = ? AND subject_type = ? AND subject_id = ?
          ORDER BY updated_at DESC",
        "SELECT content_revision::text AS content_revision, paragraph_anchor AS anchor, position_permille, device_id
           FROM reading_progress
          WHERE account_id = ?::uuid AND subject_type = ? AND subject_id = ?::uuid
          ORDER BY updated_at DESC",
    );

    let rows: Vec<ProgressRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account.to_string())
                .bind(subject_type)
                .bind(subject_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account.to_string())
                .bind(subject_type)
                .bind(subject_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(rows.into_iter().map(ProgressRow::decode).collect())
}

/// Forget one device's position for a subject.
pub async fn delete_progress(
    db: &Database,
    account: AccountId,
    subject_type: &str,
    subject_id: &str,
    device: Option<&str>,
) -> Result<bool> {
    let sql = db.sql(
        "DELETE FROM reading_progress
          WHERE account_id = ? AND subject_type = ? AND subject_id = ?
            AND device_id IS ?",
        "DELETE FROM reading_progress
          WHERE account_id = ?::uuid AND subject_type = ? AND subject_id = ?::uuid
            AND device_id IS NOT DISTINCT FROM ?",
    );

    let affected = match db.backend() {
        Backend::Sqlite => {
            // SQLite: "device_id IS ?" matches a NULL only when the bound value
            // is NULL, which is what we want for the no-device row.
            sqlx::query(&sql)
                .bind(account.to_string())
                .bind(subject_type)
                .bind(subject_id)
                .bind(device)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?
                .rows_affected()
        }
        Backend::Postgres => sqlx::query(&sql)
            .bind(account.to_string())
            .bind(subject_type)
            .bind(subject_id)
            .bind(device)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };

    Ok(affected > 0)
}

// ---------------------------------------------------------------------------
// Reading history
// ---------------------------------------------------------------------------

/// A history row as decoded, joined with the work's metadata so a list is one
/// query.
#[derive(Debug, Clone, FromRow)]
pub struct HistoryRow {
    pub id: String,
    pub subject_type: String,
    pub subject_id: String,
    pub last_read_at: String,
    pub revision_seen: Option<String>,
    pub title: String,
    pub author_handles: Option<String>,
}

/// Touch the last-read timestamp for a subject, inserting a row if needed.
pub async fn touch_history(
    db: &Database,
    account: AccountId,
    pseud: PseudId,
    subject_type: &str,
    subject_id: &str,
    revision: Option<RevisionId>,
) -> Result<()> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = now_rfc3339();
    let revision_text = revision.map(|r| r.to_string());

    let sql = db.sql(
        "INSERT INTO reading_history_entry
             (id, account_id, pseud_id, subject_type, subject_id, last_read_at,
              revision_seen, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT DO UPDATE SET
              last_read_at = excluded.last_read_at,
              revision_seen = excluded.revision_seen",
        "INSERT INTO reading_history_entry
             (id, account_id, pseud_id, subject_type, subject_id, last_read_at,
              revision_seen, created_at)
         VALUES (?::uuid, ?::uuid, ?::uuid, ?, ?::uuid, ?, ?::uuid, ?)
         ON CONFLICT (account_id, pseud_id, subject_type, subject_id) DO UPDATE SET
              last_read_at = excluded.last_read_at,
              revision_seen = excluded.revision_seen",
    );

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(account.to_string())
                .bind(pseud.to_string())
                .bind(subject_type)
                .bind(subject_id)
                .bind(&now)
                .bind(revision_text)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(account.to_string())
                .bind(pseud.to_string())
                .bind(subject_type)
                .bind(subject_id)
                .bind(&now)
                .bind(revision_text)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(())
}

/// The account's reading history, newest first.
///
/// The author handles are returned as a comma-joined string, because neither
/// SQLite nor PostgreSQL returns arrays uniformly and the client wants a
/// single column per row.
pub async fn history_for(
    db: &Database,
    account: AccountId,
    pseud: PseudId,
    limit: i64,
) -> Result<Vec<HistoryRow>> {
    let sql = sql_owned(
        db,
        "SELECT h.id AS id, h.subject_type AS subject_type,
                    h.subject_id AS subject_id, h.last_read_at AS last_read_at,
                    h.revision_seen AS revision_seen,
                    COALESCE(w.title, '') AS title,
                    (
                      SELECT GROUP_CONCAT(p.handle, ', ')
                        FROM work_contributors wc
                        JOIN pseuds p ON p.id = wc.pseud_id
                       WHERE wc.work_id = h.subject_id AND wc.public_attribution = 1
                    ) AS author_handles
               FROM reading_history_entry h
               LEFT JOIN works w ON w.id = h.subject_id AND h.subject_type = 'work'
              WHERE h.account_id = ? AND h.pseud_id = ?
              ORDER BY h.last_read_at DESC
              LIMIT ?"
            .to_string(),
        "SELECT h.id::text AS id, h.subject_type,
                    h.subject_id::text AS subject_id, h.last_read_at,
                    h.revision_seen::text AS revision_seen,
                    COALESCE(w.title, '') AS title,
                    (
                      SELECT STRING_AGG(p.handle, ', ')
                        FROM work_contributors wc
                        JOIN pseuds p ON p.id = wc.pseud_id
                       WHERE wc.work_id = h.subject_id AND wc.public_attribution = 1
                    ) AS author_handles
               FROM reading_history_entry h
               LEFT JOIN works w ON w.id = h.subject_id AND h.subject_type = 'work'
              WHERE h.account_id = ?::uuid AND h.pseud_id = ?::uuid
              ORDER BY h.last_read_at DESC
              LIMIT ?"
            .to_string(),
    );

    let rows: Vec<HistoryRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account.to_string())
                .bind(pseud.to_string())
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account.to_string())
                .bind(pseud.to_string())
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(rows)
}

/// Delete one history entry, scoped to the caller's account.
pub async fn delete_history_entry(
    db: &Database,
    account: AccountId,
    entry_id: &str,
) -> Result<bool> {
    let sql = db.sql(
        "DELETE FROM reading_history_entry WHERE id = ? AND account_id = ?",
        "DELETE FROM reading_history_entry WHERE id = ?::uuid AND account_id = ?::uuid",
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(entry_id)
            .bind(account.to_string())
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(entry_id)
            .bind(account.to_string())
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// Clear the whole reading history for an account.
pub async fn clear_history(db: &Database, account: AccountId) -> Result<u64> {
    let sql = db.sql(
        "DELETE FROM reading_history_entry WHERE account_id = ?",
        "DELETE FROM reading_history_entry WHERE account_id = ?::uuid",
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(account.to_string())
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(account.to_string())
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected)
}

// ---------------------------------------------------------------------------
// Ratings and reviews
// ---------------------------------------------------------------------------

/// A stored rating row.
#[derive(Debug, Clone, FromRow)]
pub struct Rating {
    pub stars: i64,
    pub is_public: bool,
    pub version: i64,
    pub deleted_at: Option<String>,
}

/// The public aggregate rating for a work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RatingSummary {
    /// Number of public, non-deleted ratings.
    pub count: i64,
    /// Mean rating, in permille (so a mean of 3.5 stars is 3500).
    pub mean_permille: i64,
}

/// The minimum number of public ratings before an aggregate is shown.
///
/// Spec §9.4: "Use a minimum publication threshold" before displaying a public
/// aggregate. Kept here so the rule and the aggregate query cannot drift.
pub const MIN_PUBLIC_RATINGS: i64 = 5;

fn decode_rating(row: (i64, i64, i64, Option<String>)) -> Rating {
    let (stars, is_public_int, version, deleted_at) = row;
    Rating {
        stars,
        is_public: is_public_int != 0,
        version,
        deleted_at,
    }
}

/// Upsert a rating. Returns the new version.
///
/// The rating is keyed on `(pseud, work)` and is private by default: a caller
/// that wants it public sets `is_public = true` explicitly, and the aggregate
/// query enforces it.
pub async fn upsert_rating(
    db: &Database,
    account: AccountId,
    pseud: PseudId,
    work: WorkId,
    stars: i64,
    is_public: bool,
) -> Result<i64> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = now_rfc3339();
    let public_int: i64 = if is_public { 1 } else { 0 };

    let sql = db.sql(
        "INSERT INTO rating
             (id, account_id, pseud_id, work_id, stars, is_public, created_at,
              updated_at, version, deleted_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1, NULL)
         ON CONFLICT (pseud_id, work_id) WHERE deleted_at IS NULL
         DO UPDATE SET stars = excluded.stars,
                       is_public = excluded.is_public,
                       updated_at = excluded.updated_at,
                       deleted_at = NULL,
                       version = rating.version + 1
         RETURNING version",
        "INSERT INTO rating
             (id, account_id, pseud_id, work_id, stars, is_public, created_at,
              updated_at, version, deleted_at)
         VALUES (?::uuid, ?::uuid, ?::uuid, ?::uuid, ?, ?::int::boolean, ?, ?, 1, NULL)
         ON CONFLICT (pseud_id, work_id) WHERE deleted_at IS NULL
         DO UPDATE SET stars = excluded.stars,
                       is_public = excluded.is_public,
                       updated_at = excluded.updated_at,
                       deleted_at = NULL,
                       version = rating.version + 1
         RETURNING version",
    );

    let version: i64 = match db.backend() {
        Backend::Sqlite => {
            // SQLite does not support RETURNING without a newer version; we
            // do the upsert then read the row back. The unique partial index
            // guarantees there is at most one live row.
            let pool = db.sqlite_pool().expect("sqlite handle");
            let mut tx = pool.begin().await?;
            sqlx::query(&sql.replace(" RETURNING version", ""))
                .bind(&id)
                .bind(account.to_string())
                .bind(pseud.to_string())
                .bind(work.to_string())
                .bind(stars)
                .bind(public_int)
                .bind(&now)
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            let version: i64 = sqlx::query_scalar(
                "SELECT version FROM rating WHERE pseud_id = ? AND work_id = ? AND deleted_at IS NULL",
            )
            .bind(pseud.to_string())
            .bind(work.to_string())
            .fetch_one(&mut *tx)
            .await?;
            tx.commit().await?;
            version
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            sqlx::query_scalar(&sql)
                .bind(&id)
                .bind(account.to_string())
                .bind(pseud.to_string())
                .bind(work.to_string())
                .bind(stars)
                .bind(public_int)
                .bind(&now)
                .bind(&now)
                .fetch_one(pool)
                .await?
        }
    };

    Ok(version)
}

/// The caller's rating for a work, if any.
pub async fn rating_for(db: &Database, pseud: PseudId, work: WorkId) -> Result<Option<Rating>> {
    let sql = db.sql(
        "SELECT stars, is_public, version, deleted_at FROM rating
          WHERE pseud_id = ? AND work_id = ? AND deleted_at IS NULL",
        "SELECT stars, is_public::int::bigint, version, deleted_at FROM rating
          WHERE pseud_id = ?::uuid AND work_id = ?::uuid AND deleted_at IS NULL",
    );
    let row: Option<(i64, i64, i64, Option<String>)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(pseud.to_string())
                .bind(work.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(pseud.to_string())
                .bind(work.to_string())
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(decode_rating))
}

/// The public aggregate rating for a work, when the minimum count is met.
///
/// Only rows that are public **and** not deleted contribute, and the result is
/// `None` below [`MIN_PUBLIC_RATINGS`]. A private rating therefore never moves
/// the aggregate, which is the spec §9.4 rule this query enforces.
pub async fn public_rating_summary(db: &Database, work: WorkId) -> Result<Option<RatingSummary>> {
    let sql = db.sql(
        "SELECT COUNT(*) AS count, COALESCE(SUM(stars), 0) AS sum
           FROM rating
          WHERE work_id = ? AND is_public = 1 AND deleted_at IS NULL
          HAVING COUNT(*) >= ?",
        "SELECT COUNT(*) AS count, COALESCE(SUM(stars), 0) AS sum
           FROM rating
          WHERE work_id = ?::uuid AND is_public = TRUE AND deleted_at IS NULL
          HAVING COUNT(*) >= ?",
    );
    let row: Option<(i64, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work.to_string())
                .bind(MIN_PUBLIC_RATINGS)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work.to_string())
                .bind(MIN_PUBLIC_RATINGS)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(|(count, sum)| RatingSummary {
        count,
        mean_permille: (sum * 1000) / count,
    }))
}

/// Soft-delete a rating.
pub async fn delete_rating(db: &Database, pseud: PseudId, work: WorkId) -> Result<bool> {
    let sql = db.sql(
        "UPDATE rating SET deleted_at = ?, updated_at = ?
          WHERE pseud_id = ? AND work_id = ? AND deleted_at IS NULL",
        "UPDATE rating SET deleted_at = ?, updated_at = ?
          WHERE pseud_id = ?::uuid AND work_id = ?::uuid AND deleted_at IS NULL",
    );
    let now = now_rfc3339();
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(&now)
            .bind(&now)
            .bind(pseud.to_string())
            .bind(work.to_string())
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(&now)
            .bind(&now)
            .bind(pseud.to_string())
            .bind(work.to_string())
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

// ---------------------------------------------------------------------------
// Reviews
// ---------------------------------------------------------------------------

/// A stored review, with the handle of the pseud that wrote it.
///
/// The two flags are decoded to `bool` here so that no caller has to remember
/// that SQLite stores them as integers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    pub id: String,
    pub author_handle: String,
    pub body: String,
    pub contains_spoilers: bool,
    pub is_public: bool,
    pub published_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
}

/// The row shape both engines return.
///
/// PostgreSQL is asked for `::int::bigint` on the two flags and for `id::text`, so one
/// tuple decodes on both engines (ADR 0004).
#[derive(Debug, Clone, FromRow)]
struct ReviewRow {
    id: String,
    author_handle: String,
    body: String,
    contains_spoilers: i64,
    is_public: i64,
    published_at: Option<String>,
    created_at: String,
    updated_at: String,
    version: i64,
}

impl ReviewRow {
    fn decode(self) -> Review {
        Review {
            id: self.id,
            author_handle: self.author_handle,
            body: self.body,
            contains_spoilers: self.contains_spoilers != 0,
            is_public: self.is_public != 0,
            published_at: self.published_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
            version: self.version,
        }
    }
}

/// Create or update the caller's review of a work, keyed on `(pseud, work)`.
///
/// A review is private until the caller explicitly publishes it: with
/// `is_public = false` the row keeps `published_at` NULL and
/// [`public_reviews`] can never return it. Re-publishing keeps the timestamp
/// of the *first* publication, because that is when readers could first have
/// seen it.
///
/// Returns the new version.
pub async fn upsert_review(
    db: &Database,
    account: AccountId,
    pseud: PseudId,
    work: WorkId,
    body: &str,
    contains_spoilers: bool,
    is_public: bool,
) -> Result<i64> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = now_rfc3339();
    let spoilers_int: i64 = i64::from(contains_spoilers);
    let public_int: i64 = i64::from(is_public);
    let published_at: Option<String> = if is_public { Some(now.clone()) } else { None };

    // The conflict target repeats the partial unique index's predicate
    // (`WHERE deleted_at IS NULL`), because that is the index the upsert has to
    // match; without the predicate SQLite refuses the statement outright.
    let sql = db.sql(
        "INSERT INTO review
             (id, account_id, pseud_id, work_id, body, contains_spoilers, is_public,
              published_at, created_at, updated_at, version, deleted_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, NULL)
         ON CONFLICT (pseud_id, work_id) WHERE deleted_at IS NULL
         DO UPDATE SET body = excluded.body,
                       contains_spoilers = excluded.contains_spoilers,
                       is_public = excluded.is_public,
                       published_at = COALESCE(review.published_at, excluded.published_at),
                       updated_at = excluded.updated_at,
                       deleted_at = NULL,
                       version = review.version + 1
         RETURNING version",
        "INSERT INTO review
             (id, account_id, pseud_id, work_id, body, contains_spoilers, is_public,
              published_at, created_at, updated_at, version, deleted_at)
         VALUES (?::uuid, ?::uuid, ?::uuid, ?::uuid, ?, ?::int::boolean, ?::int::boolean, ?, ?, ?, 1, NULL)
         ON CONFLICT (pseud_id, work_id) WHERE deleted_at IS NULL
         DO UPDATE SET body = excluded.body,
                       contains_spoilers = excluded.contains_spoilers,
                       is_public = excluded.is_public,
                       published_at = COALESCE(review.published_at, excluded.published_at),
                       updated_at = excluded.updated_at,
                       deleted_at = NULL,
                       version = review.version + 1
         RETURNING version",
    );

    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite handle");
            let mut tx = pool.begin().await?;
            sqlx::query(&sql.replace(" RETURNING version", ""))
                .bind(&id)
                .bind(account.to_string())
                .bind(pseud.to_string())
                .bind(work.to_string())
                .bind(body)
                .bind(spoilers_int)
                .bind(public_int)
                .bind(&published_at)
                .bind(&now)
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            let version: i64 = sqlx::query_scalar(
                "SELECT version FROM review
                  WHERE pseud_id = ? AND work_id = ? AND deleted_at IS NULL",
            )
            .bind(pseud.to_string())
            .bind(work.to_string())
            .fetch_one(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok(version)
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            let version: i64 = sqlx::query_scalar(&sql)
                .bind(&id)
                .bind(account.to_string())
                .bind(pseud.to_string())
                .bind(work.to_string())
                .bind(body)
                .bind(spoilers_int)
                .bind(public_int)
                .bind(&published_at)
                .bind(&now)
                .bind(&now)
                .fetch_one(pool)
                .await?;
            Ok(version)
        }
    }
}

/// The acting pseud's own review of a work, published or not.
pub async fn review_for(db: &Database, pseud: PseudId, work: WorkId) -> Result<Option<Review>> {
    let sql = db.sql(
        "SELECT r.id, p.handle AS author_handle, r.body, r.contains_spoilers,
                r.is_public, r.published_at, r.created_at, r.updated_at, r.version
           FROM review r
           JOIN pseuds p ON p.id = r.pseud_id
          WHERE r.pseud_id = ? AND r.work_id = ? AND r.deleted_at IS NULL",
        "SELECT r.id::text AS id, p.handle AS author_handle, r.body,
                r.contains_spoilers::int::bigint, r.is_public::int::bigint, r.published_at,
                r.created_at, r.updated_at, r.version
           FROM review r
           JOIN pseuds p ON p.id = r.pseud_id
          WHERE r.pseud_id = ?::uuid AND r.work_id = ?::uuid AND r.deleted_at IS NULL",
    );
    let row: Option<ReviewRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(pseud.to_string())
                .bind(work.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(pseud.to_string())
                .bind(work.to_string())
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(ReviewRow::decode))
}

/// The public reviews of a work, newest publication first.
///
/// Three predicates stand between a private row and a reader — `is_public = 1`,
/// a non-NULL `published_at` and `deleted_at IS NULL` — so a review that was
/// never published, or was withdrawn by its author, cannot leak through this
/// function even if some future caller forgets one of them.
pub async fn public_reviews(db: &Database, work: WorkId) -> Result<Vec<Review>> {
    let sql = db.sql(
        "SELECT r.id, p.handle AS author_handle, r.body, r.contains_spoilers,
                r.is_public, r.published_at, r.created_at, r.updated_at, r.version
           FROM review r
           JOIN pseuds p ON p.id = r.pseud_id
          WHERE r.work_id = ?
            AND r.is_public = 1
            AND r.published_at IS NOT NULL
            AND r.deleted_at IS NULL
          ORDER BY r.published_at DESC, r.id ASC",
        "SELECT r.id::text AS id, p.handle AS author_handle, r.body,
                r.contains_spoilers::int::bigint, r.is_public::int::bigint, r.published_at,
                r.created_at, r.updated_at, r.version
           FROM review r
           JOIN pseuds p ON p.id = r.pseud_id
          WHERE r.work_id = ?::uuid
            AND r.is_public = TRUE
            AND r.published_at IS NOT NULL
            AND r.deleted_at IS NULL
          ORDER BY r.published_at DESC, r.id ASC",
    );
    let rows: Vec<ReviewRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work.to_string())
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work.to_string())
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows.into_iter().map(ReviewRow::decode).collect())
}

/// Soft-delete the acting pseud's review of a work.
///
/// Soft rather than hard, because a moderation review may still need to see
/// what was written (migration 0004's retention comment).
pub async fn delete_review(db: &Database, pseud: PseudId, work: WorkId) -> Result<bool> {
    let sql = db.sql(
        "UPDATE review SET deleted_at = ?, updated_at = ?
          WHERE pseud_id = ? AND work_id = ? AND deleted_at IS NULL",
        "UPDATE review SET deleted_at = ?, updated_at = ?
          WHERE pseud_id = ?::uuid AND work_id = ?::uuid AND deleted_at IS NULL",
    );
    let now = now_rfc3339();
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(&now)
            .bind(&now)
            .bind(pseud.to_string())
            .bind(work.to_string())
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(&now)
            .bind(&now)
            .bind(pseud.to_string())
            .bind(work.to_string())
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

// ---------------------------------------------------------------------------
// Reader notes
// ---------------------------------------------------------------------------

/// A stored note row.
#[derive(Debug, Clone, FromRow)]
pub struct Note {
    pub id: String,
    pub subject_type: String,
    pub subject_id: String,
    pub anchor: Option<String>,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
}

/// All of a pseud's notes for a subject.
pub async fn notes_for(
    db: &Database,
    pseud: PseudId,
    subject_type: &str,
    subject_id: &str,
) -> Result<Vec<Note>> {
    let sql = db.sql(
        "SELECT id, subject_type, subject_id, anchor, body, created_at, updated_at, version
           FROM reader_note
          WHERE pseud_id = ? AND subject_type = ? AND subject_id = ? AND deleted_at IS NULL
          ORDER BY created_at ASC",
        "SELECT id::text AS id, subject_type, subject_id::text AS subject_id, anchor,
                body, created_at, updated_at, version
           FROM reader_note
          WHERE pseud_id = ?::uuid AND subject_type = ? AND subject_id = ?::uuid AND deleted_at IS NULL
          ORDER BY created_at ASC",
    );
    let rows: Vec<Note> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(pseud.to_string())
                .bind(subject_type)
                .bind(subject_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(pseud.to_string())
                .bind(subject_type)
                .bind(subject_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows)
}

/// Upsert a note keyed on `(pseud, subject, anchor)`; a NULL anchor is one note.
///
/// The schema deliberately carries no unique index for that key: an anchor may
/// be NULL and both engines treat NULLs as distinct, so a constraint would not
/// express it. That makes `ON CONFLICT DO UPDATE` useless here — with nothing to
/// conflict on, it degrades to a plain insert, and every edit appended a second
/// note (see `saving_the_same_note_twice_updates_it_in_place`). The row is
/// therefore looked up and updated, or inserted when it is not there, and both
/// statements run in one transaction so two writers cannot both insert.
pub async fn save_note(
    db: &Database,
    account: AccountId,
    pseud: PseudId,
    subject_type: &str,
    subject_id: &str,
    anchor: Option<&str>,
    body: &str,
) -> Result<()> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = now_rfc3339();

    let lookup = db.sql(
        "SELECT id FROM reader_note
          WHERE pseud_id = ? AND subject_type = ? AND subject_id = ?
            AND COALESCE(anchor, '') = COALESCE(?, '') AND deleted_at IS NULL",
        "SELECT id::text AS id FROM reader_note
          WHERE pseud_id = ?::uuid AND subject_type = ? AND subject_id = ?::uuid
            AND COALESCE(anchor, '') = COALESCE(?, '') AND deleted_at IS NULL",
    );
    let update = db.sql(
        "UPDATE reader_note
            SET body = ?, updated_at = ?, version = version + 1
          WHERE id = ? AND deleted_at IS NULL",
        "UPDATE reader_note
            SET body = ?, updated_at = ?, version = version + 1
          WHERE id = ?::uuid AND deleted_at IS NULL",
    );
    let insert = db.sql(
        "INSERT INTO reader_note
             (id, account_id, pseud_id, subject_type, subject_id, anchor, body,
              created_at, updated_at, version)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1)",
        "INSERT INTO reader_note
             (id, account_id, pseud_id, subject_type, subject_id, anchor, body,
              created_at, updated_at, version)
         VALUES (?::uuid, ?::uuid, ?::uuid, ?, ?::uuid, ?, ?, ?, ?, 1)",
    );

    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite handle");
            let mut tx = pool.begin().await?;
            let existing: Option<(String,)> = sqlx::query_as(&lookup)
                .bind(pseud.to_string())
                .bind(subject_type)
                .bind(subject_id)
                .bind(anchor)
                .fetch_optional(&mut *tx)
                .await?;
            match existing {
                Some((existing_id,)) => {
                    sqlx::query(&update)
                        .bind(body)
                        .bind(&now)
                        .bind(&existing_id)
                        .execute(&mut *tx)
                        .await?;
                }
                None => {
                    sqlx::query(&insert)
                        .bind(&id)
                        .bind(account.to_string())
                        .bind(pseud.to_string())
                        .bind(subject_type)
                        .bind(subject_id)
                        .bind(anchor)
                        .bind(body)
                        .bind(&now)
                        .bind(&now)
                        .execute(&mut *tx)
                        .await?;
                }
            }
            tx.commit().await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            let mut tx = pool.begin().await?;
            let existing: Option<(String,)> = sqlx::query_as(&lookup)
                .bind(pseud.to_string())
                .bind(subject_type)
                .bind(subject_id)
                .bind(anchor)
                .fetch_optional(&mut *tx)
                .await?;
            match existing {
                Some((existing_id,)) => {
                    sqlx::query(&update)
                        .bind(body)
                        .bind(&now)
                        .bind(&existing_id)
                        .execute(&mut *tx)
                        .await?;
                }
                None => {
                    sqlx::query(&insert)
                        .bind(&id)
                        .bind(account.to_string())
                        .bind(pseud.to_string())
                        .bind(subject_type)
                        .bind(subject_id)
                        .bind(anchor)
                        .bind(body)
                        .bind(&now)
                        .bind(&now)
                        .execute(&mut *tx)
                        .await?;
                }
            }
            tx.commit().await?;
        }
    }
    Ok(())
}

/// Soft-delete a note.
pub async fn delete_note(db: &Database, pseud: PseudId, note_id: &str) -> Result<bool> {
    let sql = db.sql(
        "UPDATE reader_note SET deleted_at = ?, updated_at = ?
          WHERE id = ? AND pseud_id = ? AND deleted_at IS NULL",
        "UPDATE reader_note SET deleted_at = ?, updated_at = ?
          WHERE id = ?::uuid AND pseud_id = ?::uuid AND deleted_at IS NULL",
    );
    let now = now_rfc3339();
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(&now)
            .bind(&now)
            .bind(note_id)
            .bind(pseud.to_string())
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(&now)
            .bind(&now)
            .bind(note_id)
            .bind(pseud.to_string())
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

// ---------------------------------------------------------------------------
// Typography preferences
// ---------------------------------------------------------------------------

/// A typography preference, or a sentinel for "use the defaults".
#[derive(Debug, Clone, PartialEq)]
pub struct Typography {
    pub font_scale: f64,
    pub line_height: f64,
    pub measure: i64,
    pub reader_theme: String,
    pub distraction_free: bool,
    pub version: i64,
}

impl Default for Typography {
    fn default() -> Self {
        Self {
            font_scale: 1.0,
            line_height: 1.6,
            measure: 66,
            reader_theme: "sepia".to_owned(),
            distraction_free: false,
            version: 0,
        }
    }
}

/// The caller's typography preferences, or the defaults.
pub async fn typography_for(db: &Database, account: AccountId) -> Result<Typography> {
    let sql = db.sql(
        "SELECT font_scale, line_height, measure, reader_theme, distraction_free, version
           FROM typography_preference WHERE account_id = ?",
        "SELECT font_scale::double precision, line_height::double precision, measure,
                reader_theme, distraction_free::int::bigint, version
           FROM typography_preference WHERE account_id = ?::uuid",
    );
    let row: Option<(f64, f64, i64, String, i64, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account.to_string())
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map_or(
        Typography::default(),
        |(font, height, measure, theme, free, version)| Typography {
            font_scale: font,
            line_height: height,
            measure,
            reader_theme: theme,
            distraction_free: free != 0,
            version,
        },
    ))
}

/// The typography values to store, and the version they were read at.
pub struct TypographyInput<'a> {
    pub account: AccountId,
    /// The version the caller read. `0` means "no row yet".
    pub expected_version: i64,
    pub font_scale: f64,
    pub line_height: f64,
    pub measure: i64,
    pub reader_theme: &'a str,
    pub distraction_free: bool,
}

/// Upsert the typography preferences if the caller's version is current.
///
/// When no row exists yet, `expected_version` must be 0 (the "no row"
/// sentinel). Any other value means the client believes a row exists that
/// does not — a conflict.
pub async fn save_typography(db: &Database, input: TypographyInput<'_>) -> Result<bool> {
    let TypographyInput {
        account,
        expected_version,
        font_scale,
        line_height,
        measure,
        reader_theme,
        distraction_free,
    } = input;

    let now = now_rfc3339();
    let distraction_int: i64 = if distraction_free { 1 } else { 0 };

    // Guard: when no row exists, only expected_version == 0 is valid.
    let existing_version: Option<i64> = {
        let sql_check = db.sql(
            "SELECT version FROM typography_preference WHERE account_id = ?",
            "SELECT version FROM typography_preference WHERE account_id = ?::uuid",
        );
        match db.backend() {
            Backend::Sqlite => {
                sqlx::query_scalar(&sql_check)
                    .bind(account.to_string())
                    .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                    .await?
            }
            Backend::Postgres => {
                sqlx::query_scalar(&sql_check)
                    .bind(account.to_string())
                    .fetch_optional(db.postgres_pool().expect("postgres handle"))
                    .await?
            }
        }
    };
    match existing_version {
        None if expected_version != 0 => return Ok(false),
        _ => {}
    }

    let sql = db.sql(
        "INSERT INTO typography_preference
             (account_id, font_scale, line_height, measure, reader_theme,
              distraction_free, created_at, updated_at, version)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1)
         ON CONFLICT (account_id) DO UPDATE SET
              font_scale = excluded.font_scale,
              line_height = excluded.line_height,
              measure = excluded.measure,
              reader_theme = excluded.reader_theme,
              distraction_free = excluded.distraction_free,
              updated_at = excluded.updated_at,
              version = typography_preference.version + 1
         WHERE typography_preference.version = ?",
        "INSERT INTO typography_preference
             (account_id, font_scale, line_height, measure, reader_theme,
              distraction_free, created_at, updated_at, version)
         VALUES (?::uuid, ?, ?, ?, ?, ?::int::boolean, ?, ?, 1)
         ON CONFLICT (account_id) DO UPDATE SET
              font_scale = excluded.font_scale,
              line_height = excluded.line_height,
              measure = excluded.measure,
              reader_theme = excluded.reader_theme,
              distraction_free = excluded.distraction_free,
              updated_at = excluded.updated_at,
              version = typography_preference.version + 1
         WHERE typography_preference.version = ?",
    );

    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(account.to_string())
            .bind(font_scale)
            .bind(line_height)
            .bind(measure)
            .bind(reader_theme)
            .bind(distraction_int)
            .bind(&now)
            .bind(&now)
            .bind(expected_version)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(account.to_string())
            .bind(font_scale)
            .bind(line_height)
            .bind(measure)
            .bind(reader_theme)
            .bind(distraction_int)
            .bind(&now)
            .bind(&now)
            .bind(expected_version)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };

    Ok(affected > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rating_row_decodes_its_public_flag() {
        let public = decode_rating((4, 1, 1, None));
        assert!(public.is_public);
        let private = decode_rating((4, 0, 1, None));
        assert!(!private.is_public);
    }

    #[test]
    fn a_deleted_rating_keeps_its_deleted_at() {
        let deleted = decode_rating((4, 1, 1, Some("2026-01-01T00:00:00Z".to_owned())));
        assert_eq!(deleted.deleted_at.as_deref(), Some("2026-01-01T00:00:00Z"));
    }

    #[test]
    fn the_minimum_public_rating_count_is_five() {
        assert_eq!(MIN_PUBLIC_RATINGS, 5);
    }

    #[test]
    fn typography_defaults_are_sensible() {
        let defaults = Typography::default();
        assert_eq!(defaults.font_scale, 1.0);
        assert_eq!(defaults.line_height, 1.6);
        assert_eq!(defaults.measure, 66);
        assert_eq!(defaults.reader_theme, "sepia");
        assert!(!defaults.distraction_free);
    }
}
