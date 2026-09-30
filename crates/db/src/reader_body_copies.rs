//! A reader's own copy of an external body (spec §11.15b, amendment §6.2–6.4).
//!
//! **This is the storage half of a request, and the request is a job.** Nothing
//! here fetches. `request_copy` writes a `pending` row; the fetch happens in
//! `lorehaven_app::routes::reader_body_copies`'s worker arm, bounded by the
//! source's robots posture exactly as any import is. A synchronous fetch here
//! would make a reader's request able to hold a worker open for as long as a
//! site takes to answer, which is the thing §6.2 rules out by calling it a job.
//!
//! **A copy is per-reader, and the read path does not consult this table.** A
//! reader who asks for a body does not narrow what any other reader sees —
//! §6.2's last bullet, "does not create a readers'-tier around it". So `copy_for`
//! is the *only* function here that returns a copy, it takes the caller's
//! account, and nothing in the read path joins this table. Joining it "for
//! convenience" would make one reader's standing decide another's access, and
//! `no_read_path_consults_the_request_bar` in
//! `crates/app/tests/config_sections.rs` fails the build if that appears.
//!
//! **A copy's `state` is a job state, not a retention decision.** `Refused`
//! records that *this request* was refused, with the code that refused it. It
//! says nothing about the work's retention mode afterwards: a reader refused
//! once, whose source is later unblocked, gets a fresh decision on a fresh
//! request. `retention_policy_changes` remains the only record of a mode
//! change.
//!
//! **A work has no `source_key` column**, and this module is where the lookup
//! lives. `works` (0003) carries no source at all; the link is `library_items`
//! (0006), where `work_id` sits beside `source_key`. That relationship is
//! ONE-TO-MANY — a re-import from a mirror adds a second row, and
//! `UNIQUE (account_id, source_key, source_work_key)` permits one per account —
//! so `source_for_work` takes the most recent row and says so. A work with no
//! library item yields `None` and the route refuses by name: there is no source,
//! so there is no retention decision to apply, and defaulting to the instance
//! mode would be a guess about where a work came from.

use anyhow::Result;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{Backend, Database};

// Every SELECT in this module needs its UUID and timestamp columns cast to text
// on the PostgreSQL arm — `id::text AS id`, `version::bigint AS version` — because
// 0095's ids are `UUID` there and `TEXT` on SQLite, and a `String` row struct
// cannot decode a `UUID` column. Without the cast a Postgres request fails with
// "error occurred while decoding column \"id\": mismatched types". The idiom is
// `PROPOSAL_COLUMNS_POSTGRES` in `retention_proposals.rs:188`; the INSERTs and
// UPDATEs need none of it, because there PostgreSQL is casting a bound text
// parameter at the comparison.

/// Where a copy is in the fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyState {
    /// Written by the request; the job has not settled it.
    Pending,
    /// The bytes are here and readable by the reader.
    Ready,
    /// The request was refused — the source's posture, or the instance's mode.
    /// A job state: it says nothing about the work's retention mode.
    Refused,
    /// The fetch failed for a reason worth recording, and may be retried by
    /// asking again.
    Failed,
}

impl CopyState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ready => "ready",
            Self::Refused => "refused",
            Self::Failed => "failed",
        }
    }

    /// Parse a stored state.
    ///
    /// An unrecognised state maps to `Pending`, which is the only non-terminal
    /// one: a row written by a newer build is then re-fetched rather than read
    /// as settled, and re-fetching a body is idempotent. Mapping it to a
    /// terminal state would make an unknown state silently permanent.
    #[must_use]
    pub fn parse_stored(raw: &str) -> Self {
        match raw {
            "ready" => Self::Ready,
            "refused" => Self::Refused,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct ReaderBodyCopy {
    pub id: String,
    pub work_id: String,
    pub account_id: String,
    pub source_key: String,
    pub chapter_key: String,
    pub state: CopyState,
    pub reason_code: Option<String>,
    /// The bytes. `None` until the job settles the copy as `Ready`.
    pub plain_text: Option<String>,
    pub sanitized_html: Option<String>,
    pub requested_at: String,
    pub settled_at: Option<String>,
    pub version: i64,
}

/// The one row of this table a reader may read: their own.
#[derive(Debug, Clone, FromRow)]
pub struct ReaderBodyCopyBody {
    pub work_id: String,
    pub chapter_key: String,
    pub source_key: String,
    pub plain_text: String,
    pub sanitized_html: Option<String>,
    pub settled_at: String,
}

/// A work's source: the `source_key` of its most recent non-deleted library item.
///
/// `None` when the work has no import record. The route refuses by name in that
/// case rather than falling back to the instance's mode — see the module
/// header.
pub async fn source_for_work(db: &Database, work_id: &str) -> Result<Option<String>> {
    // No `deleted_at` filter, because `library_items` has no such column: it
    // carries `created_at`/`updated_at`/`version` and retires a materialised
    // import by `work_id` going NULL (`ON DELETE SET NULL` — the column comment
    // says NULL is a private-library copy). A filter on a column that does not
    // exist fails on the first real request on both engines, which is what it
    // did until this comment was written.
    let sql = db.sql(
        "SELECT source_key FROM library_items \
         WHERE work_id = ?1 ORDER BY created_at DESC LIMIT 1",
        "SELECT source_key FROM library_items \
         WHERE work_id = $1::uuid ORDER BY created_at DESC LIMIT 1",
    );
    let row: Option<(String,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(|(key,)| key))
}

/// A work's chapter list, as the `chapter_key` string the copy records.
///
/// A work's chapters are the `chapter_revisions` rows the import wrote. The copy
/// records *which* list it fetched so a later re-import, which may replace those
/// revisions, does not silently re-point a reader's copy at different bytes.
pub async fn chapter_key_for_work(db: &Database, work_id: &str) -> Result<String> {
    let sql = db.sql(
        "SELECT COUNT(*) FROM chapter_revisions cr \
         JOIN chapters c ON c.id = cr.chapter_id WHERE c.work_id = ?1",
        "SELECT COUNT(*) FROM chapter_revisions cr \
         JOIN chapters c ON c.id = cr.chapter_id WHERE c.work_id = $1::uuid",
    );
    let row: (i64,) = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(format!("chapters:{}", row.0))
}

/// Record a reader's request: a `pending` copy, and the audit row §6.2 requires.
///
/// **An upsert on `(work_id, account_id)`, because a second request is a
/// re-fetch.** The branch is the interesting part:
///
/// * an existing `Ready` row is returned **untouched** — a reader whose body is
///   already there is not made to wait for a refetch of bytes they hold;
/// * a `Refused` or `Failed` row **resets to `Pending`** with its old bytes
///   cleared, because their source may since have been unblocked and asking again
///   is how that gets noticed;
/// * no row becomes a fresh insert.
///
/// Clearing the bytes on the reset is deliberate. A `Refused` row that kept an
/// earlier successful copy's `plain_text` would be a row whose `state` says
/// refused while its body is still readable, and the read path trusts `state`.
pub async fn request_copy(
    db: &Database,
    work_id: &str,
    account_id: &Uuid,
    source_key: &str,
    chapter_key: &str,
    trust_at_request: i64,
) -> Result<ReaderBodyCopy> {
    let existing = copy_id_for(db, work_id, &account_id.to_string()).await?;
    let now = crate::identity::now_rfc3339();
    let copy_id = existing.unwrap_or_else(|| Uuid::new_v4().to_string());

    let sql = db.sql(
        "INSERT INTO reader_body_copies \
         (id, work_id, account_id, source_key, chapter_key, state, plain_text, \
          sanitized_html, requested_at, created_at, updated_at, version) \
         VALUES (?1, ?2, ?3, ?4, ?5, 'pending', NULL, NULL, ?6, ?6, ?6, 1) \
         ON CONFLICT (work_id, account_id) DO UPDATE SET \
           state = CASE WHEN reader_body_copies.state = 'ready' THEN 'ready' ELSE 'pending' END, \
           source_key = ?4, chapter_key = ?5, \
           reason_code = CASE WHEN reader_body_copies.state = 'ready' THEN reader_body_copies.reason_code ELSE NULL END, \
           plain_text = CASE WHEN reader_body_copies.state = 'ready' THEN reader_body_copies.plain_text ELSE NULL END, \
           sanitized_html = CASE WHEN reader_body_copies.state = 'ready' THEN reader_body_copies.sanitized_html ELSE NULL END, \
           requested_at = ?6, \
           updated_at = ?6, \
           version = reader_body_copies.version + 1",
        "INSERT INTO reader_body_copies \
         (id, work_id, account_id, source_key, chapter_key, state, plain_text, \
          sanitized_html, requested_at, created_at, updated_at, version) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5, 'pending', NULL, NULL, $6, $6, $6, 1) \
         ON CONFLICT (work_id, account_id) DO UPDATE SET \
           state = CASE WHEN reader_body_copies.state = 'ready' THEN 'ready' ELSE 'pending' END, \
           source_key = $4, chapter_key = $5, \
           reason_code = CASE WHEN reader_body_copies.state = 'ready' THEN reader_body_copies.reason_code ELSE NULL END, \
           plain_text = CASE WHEN reader_body_copies.state = 'ready' THEN reader_body_copies.plain_text ELSE NULL END, \
           sanitized_html = CASE WHEN reader_body_copies.state = 'ready' THEN reader_body_copies.sanitized_html ELSE NULL END, \
           requested_at = $6, \
           updated_at = $6, \
           version = reader_body_copies.version + 1",
    );
    // `execute`, not `fetch_one`: an INSERT without `RETURNING` produces a row
    // count, not a result set, so asking for a row fails on both engines with
    // "no rows returned by a query that expected to return at least one row" —
    // which is a 500 on the first real request, and what it did until this
    // comment was written.
    //
    // `RETURNING` would read better on PostgreSQL, but `db.sql()` supplies one
    // string per arm and this has to run on SQLite too. The id is already known
    // — `copy_id_for` returned it, or the fresh `Uuid::new_v4()` above — and the
    // read-back below is not wasted: it makes the returned `state` and `version`
    // the database's values rather than the ones this function assumed. On the
    // `ready` path in particular, the `CASE` in the upsert is the database
    // deciding, and only a read-back reports what it decided.
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&copy_id)
                .bind(work_id)
                .bind(account_id.to_string())
                .bind(source_key)
                .bind(chapter_key)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&copy_id)
                .bind(work_id)
                .bind(account_id.to_string())
                .bind(source_key)
                .bind(chapter_key)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    let id = copy_id;

    // The audit row §6.2 asks for, in the same transaction as the copy. Its
    // `trust_at_request` is the reader's level *now*, not a re-derivation later:
    // trust moves, and "who asked, at what standing" is only honest if the
    // standing is kept. A different table from `retention_policy_changes`
    // because that one says an account changed the instance's mode.
    let audit = db.sql(
        "INSERT INTO retention_body_requests \
         (id, work_id, account_id, source_key, trust_at_request, requested_at, outcome) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending')",
        "INSERT INTO retention_body_requests \
         (id, work_id, account_id, source_key, trust_at_request, requested_at, outcome) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5, $6, 'pending')",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&audit)
                .bind(Uuid::new_v4().to_string())
                .bind(work_id)
                .bind(account_id.to_string())
                .bind(source_key)
                .bind(trust_at_request)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&audit)
                .bind(Uuid::new_v4().to_string())
                .bind(work_id)
                .bind(account_id.to_string())
                .bind(source_key)
                .bind(trust_at_request)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }

    copy_by_id(db, &id).await
}

async fn copy_id_for(db: &Database, work_id: &str, account_id: &str) -> Result<Option<String>> {
    let sql = db.sql(
        "SELECT id FROM reader_body_copies WHERE work_id = ?1 AND account_id = ?2",
        "SELECT id::text AS id FROM reader_body_copies \
         WHERE work_id = $1::uuid AND account_id = $2::uuid",
    );
    let row: Option<(String,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .bind(account_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .bind(account_id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(|(id,)| id))
}

async fn copy_by_id(db: &Database, id: &str) -> Result<ReaderBodyCopy> {
    let sql = db.sql(
        "SELECT id, work_id, account_id, source_key, chapter_key, state, reason_code, \
                plain_text, sanitized_html, requested_at, settled_at, version \
         FROM reader_body_copies WHERE id = ?1",
        "SELECT id::text AS id, work_id::text AS work_id, account_id::text AS account_id, \
                source_key, chapter_key, state, reason_code, plain_text, sanitized_html, \
                requested_at::text AS requested_at, settled_at::text AS settled_at, \
                version::bigint AS version \
         FROM reader_body_copies WHERE id = $1::uuid",
    );
    let raw: Option<ReaderBodyCopyRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    let row = raw.ok_or_else(|| anyhow::anyhow!("reader body copy {id} vanished mid-request"))?;
    Ok(row.into_copy())
}

#[derive(sqlx::FromRow)]
struct ReaderBodyCopyRow {
    id: String,
    work_id: String,
    account_id: String,
    source_key: String,
    chapter_key: String,
    state: String,
    reason_code: Option<String>,
    plain_text: Option<String>,
    sanitized_html: Option<String>,
    requested_at: String,
    settled_at: Option<String>,
    version: i64,
}

impl ReaderBodyCopyRow {
    fn into_copy(self) -> ReaderBodyCopy {
        ReaderBodyCopy {
            id: self.id,
            work_id: self.work_id,
            account_id: self.account_id,
            source_key: self.source_key,
            chapter_key: self.chapter_key,
            state: CopyState::parse_stored(&self.state),
            reason_code: self.reason_code,
            plain_text: self.plain_text,
            sanitized_html: self.sanitized_html,
            requested_at: self.requested_at,
            settled_at: self.settled_at,
            version: self.version,
        }
    }
}

/// The reader's own copy of a body's bytes, or `None`.
///
/// **The account comes from the caller's session, never from a parameter the
/// route chose.** A signature with a `work_id` and nothing else means there is
/// no argument for a route to get wrong, and no path by which one reader's id
/// reaches another's row.
pub async fn body_for(
    db: &Database,
    work_id: &str,
    account_id: &Uuid,
) -> Result<Option<ReaderBodyCopyBody>> {
    let sql = db.sql(
        "SELECT work_id, chapter_key, source_key, plain_text, sanitized_html, settled_at \
         FROM reader_body_copies \
         WHERE work_id = ?1 AND account_id = ?2 AND state = 'ready' AND plain_text IS NOT NULL",
        "SELECT work_id::text AS work_id, chapter_key, source_key, plain_text, \
                sanitized_html, settled_at::text AS settled_at \
         FROM reader_body_copies \
         WHERE work_id = $1::uuid AND account_id = $2::uuid \
           AND state = 'ready' AND plain_text IS NOT NULL",
    );
    let row: Option<ReaderBodyCopyBody> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .bind(account_id.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .bind(account_id.to_string())
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row)
}

/// The reader's own copy's *state*, for the `GET` that answers "did my request
/// go through". `None` when they have never asked.
pub async fn status_for(
    db: &Database,
    work_id: &str,
    account_id: &Uuid,
) -> Result<Option<(CopyState, Option<String>, String)>> {
    let sql = db.sql(
        "SELECT state, reason_code, requested_at FROM reader_body_copies \
         WHERE work_id = ?1 AND account_id = ?2",
        "SELECT state, reason_code, requested_at::text AS requested_at \
         FROM reader_body_copies \
         WHERE work_id = $1::uuid AND account_id = $2::uuid",
    );
    let row: Option<(String, Option<String>, String)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .bind(account_id.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .bind(account_id.to_string())
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(|(state, reason, at)| (CopyState::parse_stored(&state), reason, at)))
}

/// Settle a copy as `Ready` with its bytes. `false` when the row is gone.
///
/// The `state = 'pending'` guard is what makes a job idempotent: a retried
/// settlement of a copy that already settled affects nothing, rather than
/// overwriting bytes a later import replaced.
pub async fn settle_ready(
    db: &Database,
    id: &str,
    plain_text: &str,
    sanitized_html: Option<&str>,
) -> Result<bool> {
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "UPDATE reader_body_copies \
         SET state = 'ready', reason_code = NULL, plain_text = ?2, sanitized_html = ?3, \
             settled_at = ?4, updated_at = ?4, version = version + 1 \
         WHERE id = ?1 AND state = 'pending'",
        "UPDATE reader_body_copies \
         SET state = 'ready', reason_code = NULL, plain_text = $2, sanitized_html = $3, \
             settled_at = $4, updated_at = $4, version = version + 1 \
         WHERE id = $1::uuid AND state = 'pending'",
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(id)
            .bind(plain_text)
            .bind(sanitized_html)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(id)
            .bind(plain_text)
            .bind(sanitized_html)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// Settle a copy as `Refused` with the code that refused it. `false` when gone.
///
/// §6.4.5: a refusal to read is not retried, so the worker calls this and moves
/// on. The `state = 'pending'` guard means a late success cannot overwrite a
/// refusal with bytes from a request the instance had already declined to make.
pub async fn settle_refused(db: &Database, id: &str, reason_code: &str) -> Result<bool> {
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "UPDATE reader_body_copies \
         SET state = 'refused', reason_code = ?2, plain_text = NULL, sanitized_html = NULL, \
             settled_at = ?3, updated_at = ?3, version = version + 1 \
         WHERE id = ?1 AND state = 'pending'",
        "UPDATE reader_body_copies \
         SET state = 'refused', reason_code = $2, plain_text = NULL, sanitized_html = NULL, \
             settled_at = $3, updated_at = $3, version = version + 1 \
         WHERE id = $1::uuid AND state = 'pending'",
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(id)
            .bind(reason_code)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(id)
            .bind(reason_code)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    if affected > 0 {
        record_outcome(db, id, "refused", Some(reason_code)).await?;
    }
    Ok(affected > 0)
}

/// Record the outcome on the audit row, keyed by the copy's work/account pair.
///
/// A copy may have more than one request behind it — a reader can ask twice —
/// so this updates the *most recent* request for that pair rather than assuming
/// a one-to-one relationship the schema does not promise.
async fn record_outcome(
    db: &Database,
    copy_id: &str,
    outcome: &str,
    reason_code: Option<&str>,
) -> Result<()> {
    let sql = db.sql(
        "UPDATE retention_body_requests SET outcome = ?2, reason_code = ?3 \
         WHERE id = (SELECT r.id FROM retention_body_requests r \
                     JOIN reader_body_copies c \
                       ON c.work_id = r.work_id AND c.account_id = r.account_id \
                     WHERE c.id = ?1 ORDER BY r.requested_at DESC LIMIT 1)",
        "UPDATE retention_body_requests SET outcome = $2, reason_code = $3 \
         WHERE id = (SELECT r.id FROM retention_body_requests r \
                     JOIN reader_body_copies c \
                       ON c.work_id = r.work_id AND c.account_id = r.account_id \
                     WHERE c.id = $1::uuid ORDER BY r.requested_at DESC LIMIT 1)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(copy_id)
                .bind(outcome)
                .bind(reason_code)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(copy_id)
                .bind(outcome)
                .bind(reason_code)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(())
}

/// The number of copies a work has, for tests and the settlement job.
///
/// Named `count_copies` and not `count` because migration 0094's system account
/// is the standing reminder that a bare count of an accounts-adjacent table is
/// rarely the number a test means.
pub async fn count_copies(db: &Database, work_id: &str) -> Result<i64> {
    let sql = db.sql(
        "SELECT COUNT(*) FROM reader_body_copies WHERE work_id = ?1",
        "SELECT COUNT(*) FROM reader_body_copies WHERE work_id = $1::uuid",
    );
    let row: (i64,) = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.0)
}
