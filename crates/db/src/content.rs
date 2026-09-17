//! Works, chapters, revisions and publication (spec §4.3, §8).
//!
//! The rules this module exists to hold:
//!
//! * **Every mutating statement carries the caller's expected version** in its
//!   `WHERE` clause. A stale write therefore affects zero rows, and the caller
//!   turns that into `REVISION_CONFLICT` rather than overwriting a change made
//!   in another tab (spec §3.4).
//! * **Revisions are append-only.** [`append_revision`] inserts; it never
//!   updates. [`restore_revision`] copies an old revision into a *new* row, so
//!   a reader's stored position always resolves and nothing a reader has seen
//!   is rewritten (ADR 0002, spec §8 acceptance).
//! * **Publication is one transaction**: the readiness check, the contributor
//!   check, the state change, the publication event and the outbox rows either
//!   all happen or none do. No mail is sent from inside it (spec §8.4).
//! * **The publication service is idempotent.** Replaying an idempotency key
//!   returns [`PublicationOutcome::AlreadyApplied`] and enqueues nothing.
//!
//! As everywhere else in this crate, every statement is written once per
//! dialect and binds only `String`/`i64`, so rows decode identically on SQLite
//! and PostgreSQL (ADR 0004).

use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::FromRow;

use lorehaven_domain::content::{can_publish_work, publication_readiness, PublicationFacts};
use lorehaven_domain::document::Document;
use lorehaven_domain::policy::{Actor, Decision};
use lorehaven_domain::{AppError, ChapterId, PseudId, PublicationEventId, RevisionId, WorkId};

use crate::identity::now_rfc3339;
use crate::{sql_owned, Backend, Database};

/// A content operation's failure: either a refusal that already has a stable
/// public code, or an internal fault.
///
/// Keeping the two apart is what lets a handler say `ACCESS_DENIED` with a 403
/// and still mask a database fault as `INTERNAL`.
#[derive(Debug, thiserror::Error)]
pub enum ContentError {
    /// A policy or validation refusal. Safe to show the caller (spec §3.3).
    #[error(transparent)]
    Refused(#[from] AppError),
    /// Something went wrong internally; the detail is logged, never returned.
    #[error(transparent)]
    Fault(#[from] anyhow::Error),
}

impl From<sqlx::Error> for ContentError {
    fn from(error: sqlx::Error) -> Self {
        Self::Fault(error.into())
    }
}

/// Result alias for content operations.
pub type ContentResult<T> = std::result::Result<T, ContentError>;

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

/// A work row as stored.
#[derive(Debug, Clone, FromRow)]
struct WorkRow {
    id: String,
    owner_pseud_id: String,
    title: String,
    summary: String,
    language: String,
    rating: String,
    visibility: String,
    lifecycle: String,
    completion: String,
    scheduled_for: Option<String>,
    published_at: Option<String>,
    withdrawn_at: Option<String>,
    show_public_ratings: i64,
    created_at: String,
    updated_at: String,
    version: i64,
}

/// A work.
#[derive(Debug, Clone)]
pub struct Work {
    /// Identifier.
    pub id: WorkId,
    /// The pseud that owns it. Never serialised publicly.
    pub owner_pseud_id: PseudId,
    /// Title.
    pub title: String,
    /// Blurb.
    pub summary: String,
    /// BCP 47 language tag.
    pub language: String,
    /// Storage form of the content rating.
    pub rating: String,
    /// Storage form of the visibility.
    pub visibility: String,
    /// Storage form of the lifecycle.
    pub lifecycle: String,
    /// Storage form of the completion state.
    pub completion: String,
    /// When a scheduled publication comes due, RFC 3339.
    pub scheduled_for: Option<String>,
    /// When it was first published, RFC 3339.
    pub published_at: Option<String>,
    /// When it was withdrawn, RFC 3339.
    pub withdrawn_at: Option<String>,
    /// Whether the public rating aggregate is shown on the work page.
    pub show_public_ratings: bool,
    /// Creation time, RFC 3339.
    pub created_at: String,
    /// Last change, RFC 3339.
    pub updated_at: String,
    /// Optimistic-concurrency version.
    pub version: i64,
}

impl Work {
    /// Whether this work is reachable by the public right now.
    #[must_use]
    pub fn is_published(&self) -> bool {
        self.lifecycle == "published"
    }

    /// The lifecycle as a domain value, defaulting to `Draft` for an
    /// unrecognised stored string.
    ///
    /// Draft is the *closed* default: an unknown state must never be treated as
    /// published, or a typo in a row would expose content.
    #[must_use]
    pub fn lifecycle_state(&self) -> lorehaven_domain::policy::Lifecycle {
        use lorehaven_domain::policy::Lifecycle;
        match self.lifecycle.as_str() {
            "scheduled" => Lifecycle::Scheduled,
            "published" => Lifecycle::Published,
            "withdrawn" => Lifecycle::Withdrawn,
            "deleted" => Lifecycle::Deleted,
            _ => Lifecycle::Draft,
        }
    }
}

/// A chapter row as stored, joined with its current revision's word count.
#[derive(Debug, Clone, FromRow)]
struct ChapterRow {
    id: String,
    work_id: String,
    order_key: i64,
    title: String,
    current_revision_id: Option<String>,
    word_count: Option<i64>,
    plain_text: Option<String>,
    revision_count: Option<i64>,
    created_at: String,
    updated_at: String,
    version: i64,
}

/// A chapter.
#[derive(Debug, Clone)]
pub struct Chapter {
    /// Identifier.
    pub id: ChapterId,
    /// The work it belongs to.
    pub work_id: WorkId,
    /// Position within the work, spaced by ten.
    pub order_key: i64,
    /// Chapter title; an empty string means "untitled".
    pub title: String,
    /// The revision a reader is served, while one exists.
    pub current_revision_id: Option<RevisionId>,
    /// Words in the current revision.
    pub word_count: i64,
    /// Plain text of the current revision (for narration).
    pub plain_text: String,
    /// How many revisions exist.
    pub revision_count: i64,
    /// Creation time, RFC 3339.
    pub created_at: String,
    /// Last change, RFC 3339.
    pub updated_at: String,
    /// Optimistic-concurrency version.
    pub version: i64,
}

impl Chapter {
    /// Whether anything has been written in this chapter.
    ///
    /// Word count is the test rather than "a revision exists", because saving
    /// an empty editor creates a revision and that is not a publishable story.
    #[must_use]
    pub fn has_content(&self) -> bool {
        self.word_count > 0
    }
}

/// A revision row as stored.
#[derive(Debug, Clone, FromRow)]
struct RevisionRow {
    id: String,
    chapter_id: String,
    revision_number: i64,
    document_json: String,
    sanitized_html: String,
    plain_text: String,
    word_count: i64,
    note: Option<String>,
    created_by_pseud_id: String,
    restored_from_id: Option<String>,
    created_at: String,
}

/// An immutable revision.
#[derive(Debug, Clone)]
pub struct Revision {
    /// Identifier.
    pub id: RevisionId,
    /// The chapter it belongs to.
    pub chapter_id: ChapterId,
    /// 1 for the first save of a chapter, increasing from there.
    pub revision_number: i64,
    /// The canonical editor document, as JSON text.
    pub document_json: String,
    /// Derived sanitized HTML.
    pub sanitized_html: String,
    /// Derived plain text.
    pub plain_text: String,
    /// Word count of the plain text.
    pub word_count: i64,
    /// The author's note for this revision.
    pub note: Option<String>,
    /// The pseud that made it.
    pub created_by_pseud_id: PseudId,
    /// The revision this one was restored from, when it was.
    pub restored_from_id: Option<RevisionId>,
    /// Creation time, RFC 3339.
    pub created_at: String,
}

/// A revision as the history list shows it: metadata plus the author's handle.
#[derive(Debug, Clone, FromRow)]
pub struct RevisionSummary {
    /// Identifier.
    pub id: String,
    /// 1 for the first save.
    pub revision_number: i64,
    /// Word count.
    pub word_count: i64,
    /// The author's note.
    pub note: Option<String>,
    /// When it was written.
    pub created_at: String,
    /// The revision it was restored from, if any.
    pub restored_from_id: Option<String>,
    /// Handle of the pseud that wrote it.
    pub author_handle: String,
}

/// The stored form of a revision, ready to insert.
#[derive(Debug, Clone)]
pub struct RevisionInput {
    /// Canonical document JSON.
    pub document_json: String,
    /// Sanitized HTML.
    pub sanitized_html: String,
    /// Plain text.
    pub plain_text: String,
    /// Word count.
    pub word_count: i64,
    /// Optional author's note.
    pub note: Option<String>,
}

impl RevisionInput {
    /// Derive every representation from a validated document.
    ///
    /// This is the only way a [`RevisionInput`] is built from author text, so
    /// "the document is the source of truth" is a property of the type rather
    /// than a convention (ADR 0002).
    #[must_use]
    pub fn from_document(document: &Document, note: Option<String>) -> Self {
        Self {
            document_json: document.to_json().to_string(),
            sanitized_html: document.to_sanitized_html(),
            plain_text: document.to_plain_text(),
            word_count: i64::from(document.word_count()),
            note,
        }
    }
}

/// What a work looks like in a list of one's own work.
#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct OwnedWork {
    /// Identifier.
    pub id: String,
    /// Title.
    pub title: String,
    /// Storage form of the lifecycle.
    pub lifecycle: String,
    /// Storage form of the visibility.
    pub visibility: String,
    /// Storage form of the completion.
    pub completion: String,
    /// Storage form of the rating.
    pub rating: String,
    /// When it was last changed.
    pub updated_at: String,
    /// When it was published, if it has been.
    pub published_at: Option<String>,
    /// Optimistic-concurrency version.
    pub version: i64,
    /// How many chapters it has.
    pub chapter_count: i64,
    /// Words across the current revisions.
    pub word_count: i64,
    /// The acting pseud's role on it.
    pub role: String,
}

/// The fields a `PATCH /works/:id` may change.
///
/// Owned strings: the values are already validated copies, and a patch that
/// borrowed from a request struct would tie the database layer to a transport
/// type.
#[derive(Debug, Clone, Default)]
pub struct WorkPatch {
    /// New title.
    pub title: Option<String>,
    /// New blurb.
    pub summary: Option<String>,
    /// New language tag.
    pub language: Option<String>,
    /// New rating.
    pub rating: Option<String>,
    /// New visibility.
    pub visibility: Option<String>,
    /// New completion state.
    pub completion: Option<String>,
    /// Whether the public rating aggregate is displayed.
    pub show_public_ratings: Option<bool>,
}

/// What a publication or withdrawal did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicationOutcome {
    /// Applied; the work's new version.
    Published(i64),
    /// The idempotency key had already been used, so nothing changed and
    /// nothing was enqueued (spec §8 acceptance).
    AlreadyApplied,
}

// ---------------------------------------------------------------------------
// Works
// ---------------------------------------------------------------------------

const WORK_COLUMNS: &str = "id, owner_pseud_id, title, summary, language, rating, visibility, \
    lifecycle, completion, scheduled_for, published_at, withdrawn_at, show_public_ratings, \
    created_at, updated_at, version";

/// The same columns, cast to text for PostgreSQL.
const WORK_COLUMNS_PG: &str = "id::text AS id, owner_pseud_id::text AS owner_pseud_id, title, \
    summary, language, rating, visibility, lifecycle, completion, scheduled_for, published_at, \
    withdrawn_at, show_public_ratings, created_at, updated_at, version";

fn decode_work(row: WorkRow) -> ContentResult<Work> {
    Ok(Work {
        id: parse_id::<WorkId>(&row.id, "work")?,
        owner_pseud_id: parse_id::<PseudId>(&row.owner_pseud_id, "pseud")?,
        title: row.title,
        summary: row.summary,
        language: row.language,
        rating: row.rating,
        visibility: row.visibility,
        lifecycle: row.lifecycle,
        completion: row.completion,
        scheduled_for: row.scheduled_for,
        published_at: row.published_at,
        withdrawn_at: row.withdrawn_at,
        show_public_ratings: row.show_public_ratings != 0,
        created_at: row.created_at,
        updated_at: row.updated_at,
        version: row.version,
    })
}

fn decode_chapter(row: ChapterRow) -> ContentResult<Chapter> {
    Ok(Chapter {
        id: parse_id::<ChapterId>(&row.id, "chapter")?,
        work_id: parse_id::<WorkId>(&row.work_id, "work")?,
        order_key: row.order_key,
        title: row.title,
        current_revision_id: row
            .current_revision_id
            .as_deref()
            .map(|raw| parse_id::<RevisionId>(raw, "revision"))
            .transpose()?,
        word_count: row.word_count.unwrap_or(0),
        plain_text: row.plain_text.unwrap_or_default(),
        revision_count: row.revision_count.unwrap_or(0),
        created_at: row.created_at,
        updated_at: row.updated_at,
        version: row.version,
    })
}

fn decode_revision(row: RevisionRow) -> ContentResult<Revision> {
    Ok(Revision {
        id: parse_id::<RevisionId>(&row.id, "revision")?,
        chapter_id: parse_id::<ChapterId>(&row.chapter_id, "chapter")?,
        revision_number: row.revision_number,
        document_json: row.document_json,
        sanitized_html: row.sanitized_html,
        plain_text: row.plain_text,
        word_count: row.word_count,
        note: row.note,
        created_by_pseud_id: parse_id::<PseudId>(&row.created_by_pseud_id, "pseud")?,
        restored_from_id: row
            .restored_from_id
            .as_deref()
            .map(|raw| parse_id::<RevisionId>(raw, "revision"))
            .transpose()?,
        created_at: row.created_at,
    })
}

/// Parse a stored identifier, refusing to invent one.
///
/// The rest of the crate defaults a malformed stored id to a fresh UUID; that
/// is survivable for a pseud, and not survivable for a chapter, because the
/// invented id would then be used to address content.
fn parse_id<T: std::str::FromStr>(raw: &str, what: &str) -> ContentResult<T> {
    raw.parse::<T>().map_err(|_| {
        ContentError::Fault(anyhow::anyhow!(
            "stored {what} identifier is not a UUID: {raw:?}"
        ))
    })
}

/// Create a work owned by `owner`, with that pseud as its first contributor.
///
/// The owner row is inserted in the same transaction: a work with no
/// contributors would be a work nobody can edit, including its author.
pub async fn create_work(db: &Database, owner: PseudId, title: &str) -> ContentResult<Work> {
    let id = WorkId::new();
    let now = now_rfc3339();

    let insert_work = db.sql(
        "INSERT INTO works (id, owner_pseud_id, title, summary, language, rating, visibility,
                            lifecycle, completion, show_public_ratings, created_at, updated_at, version)
         VALUES (?, ?, ?, '', 'en', 'general', 'public', 'draft', 'in_progress', 1, ?, ?, 1)",
        "INSERT INTO works (id, owner_pseud_id, title, summary, language, rating, visibility,
                            lifecycle, completion, show_public_ratings, created_at, updated_at, version)
         VALUES (?::uuid, ?::uuid, ?, '', 'en', 'general', 'public', 'draft', 'in_progress', 1, ?, ?, 1)",
    );

    let insert_owner = db.sql(
        "INSERT INTO work_contributors (work_id, pseud_id, role, public_attribution, created_at)
         VALUES (?, ?, 'owner', 1, ?)",
        "INSERT INTO work_contributors (work_id, pseud_id, role, public_attribution, created_at)
         VALUES (?::uuid, ?::uuid, 'owner', 1, ?)",
    );

    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite handle");
            let mut tx = pool.begin().await?;
            sqlx::query(&insert_work)
                .bind(id.to_string())
                .bind(owner.to_string())
                .bind(title)
                .bind(&now)
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            sqlx::query(&insert_owner)
                .bind(id.to_string())
                .bind(owner.to_string())
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            let mut tx = pool.begin().await?;
            sqlx::query(&insert_work)
                .bind(id.to_string())
                .bind(owner.to_string())
                .bind(title)
                .bind(&now)
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            sqlx::query(&insert_owner)
                .bind(id.to_string())
                .bind(owner.to_string())
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
    }

    find_work(db, id)
        .await?
        .ok_or_else(|| ContentError::Fault(anyhow::anyhow!("work vanished after creation")))
}

/// Load a work by identifier. Soft-deleted works are absent.
pub async fn find_work(db: &Database, id: WorkId) -> ContentResult<Option<Work>> {
    let sql = sql_owned(
        db,
        format!("SELECT {WORK_COLUMNS} FROM works WHERE id = ? AND deleted_at IS NULL"),
        format!("SELECT {WORK_COLUMNS_PG} FROM works WHERE id::text = ? AND deleted_at IS NULL"),
    );

    let row: Option<WorkRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(id.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(id.to_string())
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    row.map(decode_work).transpose()
}

/// Every work the pseud contributes to, newest first, with its counts.
pub async fn works_for_pseud(db: &Database, pseud: PseudId) -> Result<Vec<OwnedWork>> {
    let sql = db.sql(
        "SELECT w.id AS id, w.title AS title, w.lifecycle AS lifecycle, w.visibility AS visibility,
                w.completion AS completion, w.rating AS rating, w.updated_at AS updated_at,
                w.published_at AS published_at, w.version AS version,
                (SELECT COUNT(*) FROM chapters c
                  WHERE c.work_id = w.id AND c.deleted_at IS NULL) AS chapter_count,
                (SELECT COALESCE(SUM(r.word_count), 0)
                   FROM chapters c
                   JOIN chapter_revisions r ON r.id = c.current_revision_id
                  WHERE c.work_id = w.id AND c.deleted_at IS NULL) AS word_count,
                wc.role AS role
           FROM works w
           JOIN work_contributors wc ON wc.work_id = w.id AND wc.pseud_id = ?
          WHERE w.deleted_at IS NULL
          ORDER BY w.updated_at DESC",
        "SELECT w.id::text AS id, w.title, w.lifecycle, w.visibility,
                w.completion, w.rating, w.updated_at,
                w.published_at, w.version,
                (SELECT COUNT(*) FROM chapters c
                  WHERE c.work_id = w.id AND c.deleted_at IS NULL) AS chapter_count,
                (SELECT COALESCE(SUM(r.word_count), 0)::bigint
                   FROM chapters c
                   JOIN chapter_revisions r ON r.id = c.current_revision_id
                  WHERE c.work_id = w.id AND c.deleted_at IS NULL) AS word_count,
                wc.role AS role
           FROM works w
           JOIN work_contributors wc ON wc.work_id = w.id AND wc.pseud_id::text = ?
          WHERE w.deleted_at IS NULL
          ORDER BY w.updated_at DESC",
    );

    let rows: Vec<OwnedWork> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(pseud.to_string())
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(pseud.to_string())
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(rows)
}

/// Apply a patch if the caller's version is current.
///
/// Returns `Ok(false)` when the version did not match — the caller turns that
/// into `REVISION_CONFLICT` after re-reading the stored version, so the client
/// learns what it is racing against.
pub async fn update_work(
    db: &Database,
    id: WorkId,
    expected_version: i64,
    patch: &WorkPatch,
) -> Result<bool> {
    let now = now_rfc3339();
    let flags = patch.show_public_ratings.map(i64::from);

    let sql = db.sql(
        "UPDATE works
            SET title = COALESCE(?, title),
                summary = COALESCE(?, summary),
                language = COALESCE(?, language),
                rating = COALESCE(?, rating),
                visibility = COALESCE(?, visibility),
                completion = COALESCE(?, completion),
                show_public_ratings = COALESCE(?, show_public_ratings),
                updated_at = ?, version = version + 1
          WHERE id = ? AND version = ? AND deleted_at IS NULL",
        "UPDATE works
            SET title = COALESCE(?, title),
                summary = COALESCE(?, summary),
                language = COALESCE(?, language),
                rating = COALESCE(?, rating),
                visibility = COALESCE(?, visibility),
                completion = COALESCE(?, completion),
                show_public_ratings = COALESCE(?, show_public_ratings),
                updated_at = ?, version = version + 1
          WHERE id::text = ? AND version = ? AND deleted_at IS NULL",
    );

    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(patch.title.as_deref())
            .bind(patch.summary.as_deref())
            .bind(patch.language.as_deref())
            .bind(patch.rating.as_deref())
            .bind(patch.visibility.as_deref())
            .bind(patch.completion.as_deref())
            .bind(flags)
            .bind(&now)
            .bind(id.to_string())
            .bind(expected_version)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(patch.title.as_deref())
            .bind(patch.summary.as_deref())
            .bind(patch.language.as_deref())
            .bind(patch.rating.as_deref())
            .bind(patch.visibility.as_deref())
            .bind(patch.completion.as_deref())
            .bind(flags)
            .bind(&now)
            .bind(id.to_string())
            .bind(expected_version)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };

    Ok(affected > 0)
}

// ---------------------------------------------------------------------------
// Chapters
// ---------------------------------------------------------------------------

const CHAPTER_COLUMNS: &str = "c.id AS id, c.work_id AS work_id, c.order_key AS order_key, \
    c.title AS title, c.current_revision_id AS current_revision_id, \
    r.word_count AS word_count, \
    r.plain_text AS plain_text, \
    c.created_at AS created_at, c.updated_at AS updated_at, \
    c.version AS version, \
    (SELECT COUNT(*) FROM chapter_revisions cr WHERE cr.chapter_id = c.id) AS revision_count";

const CHAPTER_COLUMNS_PG: &str = "c.id::text AS id, c.work_id::text AS work_id, c.order_key, \
    c.title, c.current_revision_id::text AS current_revision_id, \
    r.word_count, \
    r.plain_text, \
    c.created_at, c.updated_at, c.version, \
    (SELECT COUNT(*) FROM chapter_revisions cr WHERE cr.chapter_id = c.id) AS revision_count";

/// Chapters of a work in reading order.
pub async fn chapters_for_work(db: &Database, work: WorkId) -> ContentResult<Vec<Chapter>> {
    let sql = sql_owned(
        db,
        format!(
            "SELECT {CHAPTER_COLUMNS}
               FROM chapters c
               LEFT JOIN chapter_revisions r ON r.id = c.current_revision_id
              WHERE c.work_id = ? AND c.deleted_at IS NULL
              ORDER BY c.order_key ASC"
        ),
        format!(
            "SELECT {CHAPTER_COLUMNS_PG}
               FROM chapters c
               LEFT JOIN chapter_revisions r ON r.id = c.current_revision_id
              WHERE c.work_id::text = ? AND c.deleted_at IS NULL
              ORDER BY c.order_key ASC"
        ),
    );

    let rows: Vec<ChapterRow> = match db.backend() {
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

    rows.into_iter().map(decode_chapter).collect()
}

/// Load a single chapter.
pub async fn find_chapter(db: &Database, id: ChapterId) -> ContentResult<Option<Chapter>> {
    let sql = sql_owned(
        db,
        format!(
            "SELECT {CHAPTER_COLUMNS}
               FROM chapters c
               LEFT JOIN chapter_revisions r ON r.id = c.current_revision_id
              WHERE c.id = ? AND c.deleted_at IS NULL"
        ),
        format!(
            "SELECT {CHAPTER_COLUMNS_PG}
               FROM chapters c
               LEFT JOIN chapter_revisions r ON r.id = c.current_revision_id
              WHERE c.id::text = ? AND c.deleted_at IS NULL"
        ),
    );

    let row: Option<ChapterRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(id.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(id.to_string())
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    row.map(decode_chapter).transpose()
}

/// Append a chapter to the end of a work.
///
/// Positions are spaced by ten so that inserting between two chapters later is
/// a single write rather than a renumbering of everything after it.
pub async fn create_chapter(db: &Database, work: WorkId, title: &str) -> Result<Chapter> {
    let id = ChapterId::new();
    let now = now_rfc3339();

    let next_sql = db.sql(
        "SELECT COALESCE(MAX(order_key), 0) + 10 FROM chapters WHERE work_id = ?",
        "SELECT COALESCE(MAX(order_key), 0) + 10 FROM chapters WHERE work_id::text = ?",
    );
    let insert_sql = db.sql(
        "INSERT INTO chapters (id, work_id, order_key, title, created_at, updated_at, version)
         VALUES (?, ?, ?, ?, ?, ?, 1)",
        "INSERT INTO chapters (id, work_id, order_key, title, created_at, updated_at, version)
         VALUES (?::uuid, ?::uuid, ?, ?, ?, ?, 1)",
    );

    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite handle");
            let mut tx = pool.begin().await?;
            let order: i64 = sqlx::query_scalar(&next_sql)
                .bind(work.to_string())
                .fetch_one(&mut *tx)
                .await?;
            sqlx::query(&insert_sql)
                .bind(id.to_string())
                .bind(work.to_string())
                .bind(order)
                .bind(title)
                .bind(&now)
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            let mut tx = pool.begin().await?;
            let order: i64 = sqlx::query_scalar(&next_sql)
                .bind(work.to_string())
                .fetch_one(&mut *tx)
                .await?;
            sqlx::query(&insert_sql)
                .bind(id.to_string())
                .bind(work.to_string())
                .bind(order)
                .bind(title)
                .bind(&now)
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
    }

    find_chapter(db, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("chapter vanished after creation"))
}

/// Rename a chapter, if the caller's version is current.
pub async fn update_chapter(
    db: &Database,
    id: ChapterId,
    expected_version: i64,
    title: Option<&str>,
) -> Result<bool> {
    let now = now_rfc3339();
    let sql = db.sql(
        "UPDATE chapters
            SET title = COALESCE(?, title), updated_at = ?, version = version + 1
          WHERE id = ? AND version = ? AND deleted_at IS NULL",
        "UPDATE chapters
            SET title = COALESCE(?, title), updated_at = ?, version = version + 1
          WHERE id::text = ? AND version = ? AND deleted_at IS NULL",
    );

    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(title)
            .bind(&now)
            .bind(id.to_string())
            .bind(expected_version)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(title)
            .bind(&now)
            .bind(id.to_string())
            .bind(expected_version)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };

    Ok(affected > 0)
}

/// Mark a chapter deleted.
///
/// Chapters are soft-deleted because a reader's stored position may reference
/// one, and because a revision history that outlives the chapter is what makes
/// an accidental deletion recoverable.
pub async fn delete_chapter(db: &Database, id: ChapterId) -> Result<bool> {
    let now = now_rfc3339();
    let sql = db.sql(
        "UPDATE chapters SET deleted_at = ?, updated_at = ?, version = version + 1
          WHERE id = ? AND deleted_at IS NULL",
        "UPDATE chapters SET deleted_at = ?, updated_at = ?, version = version + 1
          WHERE id::text = ? AND deleted_at IS NULL",
    );

    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(&now)
            .bind(&now)
            .bind(id.to_string())
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(&now)
            .bind(&now)
            .bind(id.to_string())
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };

    Ok(affected > 0)
}

/// Reorder the chapters of a work.
///
/// The supplied list must be exactly the work's live chapters; anything else is
/// refused rather than partially applied, so a client with a stale list cannot
/// silently drop a chapter that another tab added.
pub async fn reorder_chapters(
    db: &Database,
    work: WorkId,
    ordered: &[ChapterId],
) -> ContentResult<bool> {
    let existing = chapters_for_work(db, work).await?;
    let existing_ids: Vec<ChapterId> = existing.iter().map(|chapter| chapter.id).collect();

    if existing_ids.len() != ordered.len() || !ordered.iter().all(|id| existing_ids.contains(id)) {
        return Err(ContentError::Refused(AppError::field(
            "chapters",
            "The chapter list changed since this page was loaded. Reload and try again.",
        )));
    }

    let now = now_rfc3339();
    let sql = db.sql(
        "UPDATE chapters SET order_key = ?, updated_at = ? WHERE id = ?",
        "UPDATE chapters SET order_key = ?, updated_at = ? WHERE id::text = ?",
    );

    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite handle");
            let mut tx = pool.begin().await?;
            for (index, id) in ordered.iter().enumerate() {
                let position = i64::try_from(index).unwrap_or(0) * 10 + 10;
                sqlx::query(&sql)
                    .bind(position)
                    .bind(&now)
                    .bind(id.to_string())
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            let mut tx = pool.begin().await?;
            for (index, id) in ordered.iter().enumerate() {
                let position = i64::try_from(index).unwrap_or(0) * 10 + 10;
                sqlx::query(&sql)
                    .bind(position)
                    .bind(&now)
                    .bind(id.to_string())
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
        }
    }

    Ok(true)
}

// ---------------------------------------------------------------------------
// Revisions
// ---------------------------------------------------------------------------

/// Append a revision to a chapter, making it the current one.
///
/// Three writes in one transaction: the immutable revision, the chapter's
/// pointer to it, and an outbox row for the indexer. A chapter whose pointer
/// did not move, or an indexer that never heard about it, would be exactly the
/// kind of half-applied state spec §8.4 forbids.
///
/// `expected_version` carries the caller's optimistic-concurrency expectation
/// (spec §3.4). When it is supplied and does not match, the transaction is
/// abandoned: no revision row is written, and the caller gets a conflict naming
/// the version it was racing.
pub async fn append_revision(
    db: &Database,
    chapter: ChapterId,
    work: WorkId,
    author: PseudId,
    input: &RevisionInput,
    expected_version: Option<i64>,
    restored_from: Option<RevisionId>,
) -> ContentResult<Revision> {
    let id = RevisionId::new();
    let now = now_rfc3339();

    let next_number_sql = db.sql(
        "SELECT COALESCE(MAX(revision_number), 0) + 1 FROM chapter_revisions WHERE chapter_id = ?",
        "SELECT COALESCE(MAX(revision_number), 0) + 1 FROM chapter_revisions WHERE chapter_id::text = ?",
    );
    let insert_sql = db.sql(
        "INSERT INTO chapter_revisions
             (id, chapter_id, revision_number, document_json, sanitized_html, plain_text,
              word_count, note, created_by_pseud_id, restored_from_id, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        "INSERT INTO chapter_revisions
             (id, chapter_id, revision_number, document_json, sanitized_html, plain_text,
              word_count, note, created_by_pseud_id, restored_from_id, created_at)
         VALUES (?::uuid, ?::uuid, ?, ?, ?, ?, ?, ?, ?::uuid, ?::uuid, ?)",
    );
    let move_pointer_sql = db.sql(
        "UPDATE chapters
            SET current_revision_id = ?, updated_at = ?, version = version + 1
          WHERE id = ? AND version = ? AND deleted_at IS NULL",
        "UPDATE chapters
            SET current_revision_id = ?::uuid, updated_at = ?, version = version + 1
          WHERE id::text = ? AND version = ? AND deleted_at IS NULL",
    );
    let move_pointer_unchecked_sql = db.sql(
        "UPDATE chapters
            SET current_revision_id = ?, updated_at = ?, version = version + 1
          WHERE id = ? AND deleted_at IS NULL",
        "UPDATE chapters
            SET current_revision_id = ?::uuid, updated_at = ?, version = version + 1
          WHERE id::text = ? AND deleted_at IS NULL",
    );
    let touch_work_sql = db.sql(
        "UPDATE works SET updated_at = ? WHERE id = ?",
        "UPDATE works SET updated_at = ? WHERE id::text = ?",
    );
    let outbox_sql = db.sql(
        "INSERT INTO outbox_events (id, topic, payload, dedupe_key, created_at, available_at, attempts)
         VALUES (?, 'chapter.revised', ?, NULL, ?, ?, 0)",
        "INSERT INTO outbox_events (id, topic, payload, dedupe_key, created_at, available_at, attempts)
         VALUES (?::uuid, 'chapter.revised', ?, NULL, ?, ?, 0)",
    );
    let version_sql = db.sql(
        "SELECT version FROM chapters WHERE id = ?",
        "SELECT version FROM chapters WHERE id::text = ?",
    );

    let payload = serde_json::json!({ "chapter_id": chapter, "revision_id": id, "work_id": work })
        .to_string();

    macro_rules! write_revision {
        ($tx:expr) => {{
            let mut tx = $tx;
            let number: i64 = sqlx::query_scalar(&next_number_sql)
                .bind(chapter.to_string())
                .fetch_one(&mut *tx)
                .await?;

            // The revision is inserted first: `chapters.current_revision_id`
            // references it, so pointing the chapter at a row that does not
            // exist yet would violate the foreign key.
            sqlx::query(&insert_sql)
                .bind(id.to_string())
                .bind(chapter.to_string())
                .bind(number)
                .bind(&input.document_json)
                .bind(&input.sanitized_html)
                .bind(&input.plain_text)
                .bind(input.word_count)
                .bind(&input.note)
                .bind(author.to_string())
                .bind(restored_from.map(|revision| revision.to_string()))
                .bind(&now)
                .execute(&mut *tx)
                .await?;

            let affected = if let Some(expected) = expected_version {
                sqlx::query(&move_pointer_sql)
                    .bind(id.to_string())
                    .bind(&now)
                    .bind(chapter.to_string())
                    .bind(expected)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected()
            } else {
                sqlx::query(&move_pointer_unchecked_sql)
                    .bind(id.to_string())
                    .bind(&now)
                    .bind(chapter.to_string())
                    .execute(&mut *tx)
                    .await?
                    .rows_affected()
            };

            if affected == 0 {
                // Every statement above is rolled back, because the transaction
                // is dropped rather than committed: a stale save writes nothing
                // at all, not even a revision nobody asked for.
                let actual: Option<i64> = sqlx::query_scalar(&version_sql)
                    .bind(chapter.to_string())
                    .fetch_optional(&mut *tx)
                    .await?;
                return Err(ContentError::Refused(AppError::RevisionConflict {
                    expected: expected_version.unwrap_or(0),
                    actual: actual.unwrap_or(0),
                }));
            }

            sqlx::query(&touch_work_sql)
                .bind(&now)
                .bind(work.to_string())
                .execute(&mut *tx)
                .await?;

            sqlx::query(&outbox_sql)
                .bind(uuid::Uuid::new_v4().to_string())
                .bind(&payload)
                .bind(&now)
                .bind(&now)
                .execute(&mut *tx)
                .await?;

            tx.commit().await
        }};
    }

    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite handle");
            write_revision!(pool.begin().await?)?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            write_revision!(pool.begin().await?)?;
        }
    }

    find_revision(db, id)
        .await?
        .ok_or_else(|| ContentError::Fault(anyhow::anyhow!("revision vanished after creation")))
}

/// Load a revision.
pub async fn find_revision(db: &Database, id: RevisionId) -> ContentResult<Option<Revision>> {
    let sql = db.sql(
        "SELECT id, chapter_id, revision_number, document_json, sanitized_html, plain_text,
                word_count, note, created_by_pseud_id, restored_from_id, created_at
           FROM chapter_revisions WHERE id = ?",
        "SELECT id::text AS id, chapter_id::text AS chapter_id, revision_number, document_json,
                sanitized_html, plain_text, word_count, note,
                created_by_pseud_id::text AS created_by_pseud_id,
                restored_from_id::text AS restored_from_id, created_at
           FROM chapter_revisions WHERE id::text = ?",
    );

    let row: Option<RevisionRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(id.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(id.to_string())
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    row.map(decode_revision).transpose()
}

/// A chapter's revisions, newest first.
pub async fn revisions_for_chapter(
    db: &Database,
    chapter: ChapterId,
) -> Result<Vec<RevisionSummary>> {
    let sql = db.sql(
        "SELECT r.id AS id, r.revision_number AS revision_number, r.word_count AS word_count,
                r.note AS note, r.created_at AS created_at, r.restored_from_id AS restored_from_id,
                p.handle AS author_handle
           FROM chapter_revisions r
           JOIN pseuds p ON p.id = r.created_by_pseud_id
          WHERE r.chapter_id = ?
          ORDER BY r.revision_number DESC",
        "SELECT r.id::text AS id, r.revision_number, r.word_count, r.note, r.created_at,
                r.restored_from_id::text AS restored_from_id, p.handle AS author_handle
           FROM chapter_revisions r
           JOIN pseuds p ON p.id = r.created_by_pseud_id
          WHERE r.chapter_id::text = ?
          ORDER BY r.revision_number DESC",
    );

    let rows: Vec<RevisionSummary> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(chapter.to_string())
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(chapter.to_string())
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(rows)
}

/// Bring an old revision back, as a **new** revision.
///
/// Nothing is rewritten: the restored content is copied into a fresh row that
/// records where it came from (ADR 0002, spec §8 acceptance).
pub async fn restore_revision(
    db: &Database,
    chapter: Chapter,
    revision: RevisionId,
    author: PseudId,
) -> ContentResult<Revision> {
    let source = find_revision(db, revision).await?;

    // A revision belonging to another chapter is not found, rather than found
    // and refused: the caller learns nothing about content it cannot reach.
    let source = source
        .filter(|revision| revision.chapter_id == chapter.id)
        .ok_or(ContentError::Refused(AppError::NotFound {
            resource: "revision",
        }))?;

    let input = RevisionInput {
        document_json: source.document_json.clone(),
        sanitized_html: source.sanitized_html.clone(),
        plain_text: source.plain_text.clone(),
        word_count: source.word_count,
        note: Some(format!("Restored from revision {}", source.revision_number)),
    };

    // Restoring is not an optimistic-concurrency operation: the caller picked a
    // revision from the history, and the new revision lands on top of whatever
    // the chapter holds now.
    append_revision(
        db,
        chapter.id,
        chapter.work_id,
        author,
        &input,
        None,
        Some(source.id),
    )
    .await
}

// ---------------------------------------------------------------------------
// Publication
// ---------------------------------------------------------------------------

/// How many chapters a work has, and how many of them are not empty.
pub async fn chapter_facts(db: &Database, work: WorkId) -> Result<(usize, usize)> {
    let chapters = chapters_for_work(db, work).await?;
    let with_content = chapters
        .iter()
        .filter(|chapter| chapter.has_content())
        .count();
    Ok((chapters.len(), with_content))
}

/// Everything a publication decision needs, loaded from storage.
pub async fn publication_facts<'a>(db: &Database, work: &'a Work) -> Result<PublicationFacts<'a>> {
    let (count, with_content) = chapter_facts(db, work.id).await?;
    Ok(PublicationFacts {
        title: &work.title,
        chapter_count: count,
        chapters_with_content: with_content,
        lifecycle: work.lifecycle_state(),
    })
}

/// Publish a work: the contributor check, the readiness check, the state
/// change, the publication event and the outbox rows, in one transaction.
pub async fn publish_work(
    db: &Database,
    work: &Work,
    actor: &Actor,
    expected_version: i64,
    idempotency_key: Option<&str>,
) -> ContentResult<PublicationOutcome> {
    // A replay must do nothing at all — not merely skip the notification — so
    // this is checked before the transaction opens.
    if let Some(key) = idempotency_key {
        if publication_event_exists(db, key).await? {
            return Ok(PublicationOutcome::AlreadyApplied);
        }
    }

    let contributors = crate::collaboration::contributors_for_work(db, work.id).await?;
    if let Decision::Deny(reason) = can_publish_work(actor, &contributors) {
        tracing::debug!(work = %work.id, reason = reason.as_str(), "publication refused");
        return Err(ContentError::Refused(AppError::AccessDenied));
    }

    let facts = publication_facts(db, work).await?;
    publication_readiness(&facts)?;

    let now = now_rfc3339();
    let event_id = PublicationEventId::new();

    let update_sql = db.sql(
        "UPDATE works
            SET lifecycle = 'published', published_at = COALESCE(published_at, ?),
                withdrawn_at = NULL, updated_at = ?, version = version + 1
          WHERE id = ? AND version = ? AND deleted_at IS NULL",
        "UPDATE works
            SET lifecycle = 'published', published_at = COALESCE(published_at, ?),
                withdrawn_at = NULL, updated_at = ?, version = version + 1
          WHERE id::text = ? AND version = ? AND deleted_at IS NULL",
    );
    let event_sql = db.sql(
        "INSERT INTO publication_events (id, work_id, chapter_id, action, actor_pseud_id,
                                         idempotency_key, note, occurred_at)
         VALUES (?, ?, NULL, 'publish', ?, ?, NULL, ?)",
        "INSERT INTO publication_events (id, work_id, chapter_id, action, actor_pseud_id,
                                         idempotency_key, note, occurred_at)
         VALUES (?::uuid, ?::uuid, NULL, 'publish', ?::uuid, ?, NULL, ?)",
    );
    let outbox_sql = db.sql(
        "INSERT INTO outbox_events (id, topic, payload, dedupe_key, created_at, available_at, attempts)
         VALUES (?, ?, ?, ?, ?, ?, 0)",
        "INSERT INTO outbox_events (id, topic, payload, dedupe_key, created_at, available_at, attempts)
         VALUES (?::uuid, ?, ?, ?, ?, ?, 0)",
    );

    let payload =
        serde_json::json!({ "work_id": work.id, "actor_pseud_id": actor.pseud_id }).to_string();

    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite handle");
            let mut tx = pool.begin().await?;
            let affected = sqlx::query(&update_sql)
                .bind(&now)
                .bind(&now)
                .bind(work.id.to_string())
                .bind(expected_version)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            if affected == 0 {
                let actual = current_work_version(db, work.id).await?;
                return Err(ContentError::Refused(AppError::RevisionConflict {
                    expected: expected_version,
                    actual,
                }));
            }
            sqlx::query(&event_sql)
                .bind(event_id.to_string())
                .bind(work.id.to_string())
                .bind(actor.pseud_id.to_string())
                .bind(idempotency_key)
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            for topic in ["publish.notify", "publish.index"] {
                sqlx::query(&outbox_sql)
                    .bind(uuid::Uuid::new_v4().to_string())
                    .bind(topic)
                    .bind(&payload)
                    .bind(format!("{}:{topic}", event_id))
                    .bind(&now)
                    .bind(&now)
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            let mut tx = pool.begin().await?;
            let affected = sqlx::query(&update_sql)
                .bind(&now)
                .bind(&now)
                .bind(work.id.to_string())
                .bind(expected_version)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            if affected == 0 {
                let actual = current_work_version(db, work.id).await?;
                return Err(ContentError::Refused(AppError::RevisionConflict {
                    expected: expected_version,
                    actual,
                }));
            }
            sqlx::query(&event_sql)
                .bind(event_id.to_string())
                .bind(work.id.to_string())
                .bind(actor.pseud_id.to_string())
                .bind(idempotency_key)
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            for topic in ["publish.notify", "publish.index"] {
                sqlx::query(&outbox_sql)
                    .bind(uuid::Uuid::new_v4().to_string())
                    .bind(topic)
                    .bind(&payload)
                    .bind(format!("{}:{topic}", event_id))
                    .bind(&now)
                    .bind(&now)
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
        }
    }

    Ok(PublicationOutcome::Published(expected_version + 1))
}

/// Withdraw a published work: it stops being readable and stops being indexed.
pub async fn withdraw_work(
    db: &Database,
    work: &Work,
    actor: &Actor,
    expected_version: i64,
    idempotency_key: Option<&str>,
) -> ContentResult<PublicationOutcome> {
    if let Some(key) = idempotency_key {
        if publication_event_exists(db, key).await? {
            return Ok(PublicationOutcome::AlreadyApplied);
        }
    }

    let contributors = crate::collaboration::contributors_for_work(db, work.id).await?;
    if let Decision::Deny(reason) = can_publish_work(actor, &contributors) {
        tracing::debug!(work = %work.id, reason = reason.as_str(), "withdrawal refused");
        return Err(ContentError::Refused(AppError::AccessDenied));
    }

    let now = now_rfc3339();
    let event_id = PublicationEventId::new();

    let update_sql = db.sql(
        "UPDATE works
            SET lifecycle = 'withdrawn', withdrawn_at = ?, updated_at = ?, version = version + 1
          WHERE id = ? AND version = ? AND deleted_at IS NULL",
        "UPDATE works
            SET lifecycle = 'withdrawn', withdrawn_at = ?, updated_at = ?, version = version + 1
          WHERE id::text = ? AND version = ? AND deleted_at IS NULL",
    );
    let event_sql = db.sql(
        "INSERT INTO publication_events (id, work_id, chapter_id, action, actor_pseud_id,
                                         idempotency_key, note, occurred_at)
         VALUES (?, ?, NULL, 'withdraw', ?, ?, NULL, ?)",
        "INSERT INTO publication_events (id, work_id, chapter_id, action, actor_pseud_id,
                                         idempotency_key, note, occurred_at)
         VALUES (?::uuid, ?::uuid, NULL, 'withdraw', ?::uuid, ?, NULL, ?)",
    );
    let outbox_sql = db.sql(
        "INSERT INTO outbox_events (id, topic, payload, dedupe_key, created_at, available_at, attempts)
         VALUES (?, 'withdraw.deindex', ?, ?, ?, ?, 0)",
        "INSERT INTO outbox_events (id, topic, payload, dedupe_key, created_at, available_at, attempts)
         VALUES (?::uuid, 'withdraw.deindex', ?, ?, ?, ?, 0)",
    );

    let payload = serde_json::json!({ "work_id": work.id }).to_string();

    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite handle");
            let mut tx = pool.begin().await?;
            let affected = sqlx::query(&update_sql)
                .bind(&now)
                .bind(&now)
                .bind(work.id.to_string())
                .bind(expected_version)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            if affected == 0 {
                let actual = current_work_version(db, work.id).await?;
                return Err(ContentError::Refused(AppError::RevisionConflict {
                    expected: expected_version,
                    actual,
                }));
            }
            sqlx::query(&event_sql)
                .bind(event_id.to_string())
                .bind(work.id.to_string())
                .bind(actor.pseud_id.to_string())
                .bind(idempotency_key)
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            sqlx::query(&outbox_sql)
                .bind(uuid::Uuid::new_v4().to_string())
                .bind(&payload)
                .bind(format!("{}:withdraw.deindex", event_id))
                .bind(&now)
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            let mut tx = pool.begin().await?;
            let affected = sqlx::query(&update_sql)
                .bind(&now)
                .bind(&now)
                .bind(work.id.to_string())
                .bind(expected_version)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            if affected == 0 {
                let actual = current_work_version(db, work.id).await?;
                return Err(ContentError::Refused(AppError::RevisionConflict {
                    expected: expected_version,
                    actual,
                }));
            }
            sqlx::query(&event_sql)
                .bind(event_id.to_string())
                .bind(work.id.to_string())
                .bind(actor.pseud_id.to_string())
                .bind(idempotency_key)
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            sqlx::query(&outbox_sql)
                .bind(uuid::Uuid::new_v4().to_string())
                .bind(&payload)
                .bind(format!("{}:withdraw.deindex", event_id))
                .bind(&now)
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
    }

    Ok(PublicationOutcome::Published(expected_version + 1))
}

/// The stored version of a work, for reporting a conflict honestly.
pub async fn current_work_version(db: &Database, work: WorkId) -> Result<i64> {
    let sql = db.sql(
        "SELECT version FROM works WHERE id = ?",
        "SELECT version FROM works WHERE id::text = ?",
    );
    let version: Option<i64> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(work.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(work.to_string())
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(version.unwrap_or(0))
}

/// Whether an idempotency key has already been used.
pub async fn publication_event_exists(db: &Database, key: &str) -> Result<bool> {
    let sql = db.sql(
        "SELECT COUNT(*) FROM publication_events WHERE idempotency_key = ?",
        "SELECT COUNT(*) FROM publication_events WHERE idempotency_key = ?",
    );
    let count: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(key)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(key)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(count > 0)
}

/// The publication history of a work, newest first.
pub async fn publication_events(
    db: &Database,
    work: WorkId,
    limit: i64,
) -> Result<Vec<(String, String, String)>> {
    let sql = db.sql(
        "SELECT id, action, occurred_at FROM publication_events
          WHERE work_id = ? ORDER BY occurred_at DESC LIMIT ?",
        "SELECT id::text AS id, action, occurred_at FROM publication_events
          WHERE work_id::text = ? ORDER BY occurred_at DESC LIMIT ?",
    );
    let rows: Vec<(String, String, String)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work.to_string())
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work.to_string())
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows)
}

/// Load the document of a revision, for the preview and the reader.
pub async fn revision_document(db: &Database, revision: RevisionId) -> Result<Option<Value>> {
    let revision = find_revision(db, revision).await?;
    Ok(revision.and_then(|revision| serde_json::from_str::<Value>(&revision.document_json).ok()))
}

/// Delete every row this module owns. Development seed reset only.
pub async fn wipe_content(db: &Database) -> Result<()> {
    let statements = [
        "DELETE FROM outbox_events",
        "DELETE FROM publication_events",
        "DELETE FROM collaboration_invites",
        "UPDATE chapters SET current_revision_id = NULL",
        "DELETE FROM chapter_revisions",
        "DELETE FROM chapters",
        "DELETE FROM work_contributors",
        "DELETE FROM works",
    ];

    for statement in statements {
        match db.backend() {
            Backend::Sqlite => {
                sqlx::query(statement)
                    .execute(db.sqlite_pool().expect("sqlite handle"))
                    .await
                    .with_context(|| format!("wiping with `{statement}`"))?;
            }
            Backend::Postgres => {
                sqlx::query(statement)
                    .execute(db.postgres_pool().expect("postgres handle"))
                    .await
                    .with_context(|| format!("wiping with `{statement}`"))?;
            }
        }
    }
    Ok(())
}

/// Contributors of a work, as the policy layer wants them.
///
/// Delegates to [`crate::collaboration`] so the loading rule lives in one
/// place; re-exported here because the publication path needs it.
pub use crate::collaboration::contributors_for_work;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unrecognised_lifecycle_is_treated_as_a_draft() {
        // The closed default: an unknown stored state must never be published.
        let work = Work {
            id: WorkId::new(),
            owner_pseud_id: PseudId::new(),
            title: "t".to_owned(),
            summary: String::new(),
            language: "en".to_owned(),
            rating: "general".to_owned(),
            visibility: "public".to_owned(),
            lifecycle: "publshed".to_owned(),
            completion: "in_progress".to_owned(),
            scheduled_for: None,
            published_at: None,
            withdrawn_at: None,
            show_public_ratings: true,
            created_at: String::new(),
            updated_at: String::new(),
            version: 1,
        };
        assert!(!work.is_published());
        assert_eq!(
            work.lifecycle_state(),
            lorehaven_domain::policy::Lifecycle::Draft
        );
    }

    #[test]
    fn revisions_derive_every_representation_from_the_document() {
        let document = Document::from_plain("One two three.");
        let input = RevisionInput::from_document(&document, Some("first".to_owned()));
        assert_eq!(input.word_count, 3);
        assert_eq!(input.plain_text, "One two three.");
        assert_eq!(input.sanitized_html, "<p>One two three.</p>");
        assert!(input.document_json.contains("\"type\":\"doc\""));
    }

    #[test]
    fn a_malformed_stored_identifier_is_a_fault_rather_than_an_invention() {
        let error = parse_id::<ChapterId>("not-a-uuid", "chapter").expect_err("must refuse");
        assert!(matches!(error, ContentError::Fault(_)));
    }
}
