//! M45-51 — subject access and erasure.
//!
//! Spec: `docs/plans/m45-51-subject-access-and-erasure.md`.
//!
//! ## The cascade is the schema's job, not this module's
//!
//! Parsing every `CREATE TABLE`/`REFERENCES` in `migrations/sqlite/` shows **71 tables** whose
//! foreign-key chain reaches `accounts`, and **exactly one** column into `accounts` that does
//! not cascade: `jobs.requested_by`, `ON DELETE SET NULL`, because a job's audit trail must
//! survive the requester who asked for it.
//!
//! So [`erase_account`] is effectively a single `DELETE FROM accounts`. It is deliberately NOT
//! an application-side list of 71 tables: such a list is 71 entries to keep in sync with the
//! migrations, and it fails *silently* — the day a migration adds a table, the list is stale
//! and the erasure leaks. This project has already shipped `did_not_finish` beside
//! `reading_history_entry` as two tables where one richer one existed; a second parallel
//! erasure path would be that mistake with legal consequences.
//!
//! What this module owns is the part the schema cannot: telling the reader **in advance** what
//! will be destroyed, and assembling what will be disclosed.
//!
//! ## Why `plan_erasure` exists at all
//!
//! `works.owner_pseud_id` is `ON DELETE CASCADE`. Erasing an account therefore **deletes the
//! works that account authored** — for an archive of fanfic, a serious and possibly
//! regrettable consequence. A `DELETE` that silently removes published writing because someone
//! asked to be forgotten is not a small surprise, so the route refuses without `force` and
//! reports the count. See the spec's §2a step 2.

use crate::{sql_owned, Backend, Database, Result};
use serde::Serialize;
use sqlx::FromRow;

/// One reader-private row, as disclosed.
#[derive(Debug, Clone, Serialize, PartialEq, FromRow)]
pub struct BookmarkRow {
    pub id: String,
    pub subject_type: String,
    pub subject_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, FromRow)]
pub struct ReadingProgressRow {
    pub id: String,
    pub subject_type: String,
    pub subject_id: String,
    pub position_permille: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, FromRow)]
pub struct DidNotFinishRow {
    pub id: String,
    pub pseud_id: String,
    pub work_id: String,
    pub reason: String,
}

/// Everything held about one reader, in the shape a subject-access response takes.
///
/// ## Field naming is load-bearing
///
/// This struct serialises to a JSON object, and the test
/// `subject_data_never_carries_a_numeric_resonance_score` walks the serialised keys against
/// [`lorehaven_domain::analytics::FORBIDDEN_SCOPE_NAMES`] — 13 names that must never be
/// readable. Every field here is named from the *reader-facing* vocabulary
/// (`bookmarks`, `reading_progress`, `did_not_finish`) and deliberately avoids the analytics
/// namespace entirely.
///
/// In particular there is **no resonance field**, because `ab.resonance_numeric` is forbidden
/// and `OwnResonanceLabel` exists so that the *label* is the only form ever shown. The
/// temptation to add the number "for debugging" is the obvious future mistake, and a field
/// called `resonance_score` would fail the forbidden-key walk by name.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct SubjectData {
    pub account_id: String,
    pub email: Option<String>,
    pub handles: Vec<String>,
    pub bookmarks: Vec<BookmarkRow>,
    pub reading_progress: Vec<ReadingProgressRow>,
    pub did_not_finish: Vec<DidNotFinishRow>,
}

/// What an erasure would destroy, counted before it happens.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ErasurePlan {
    pub account_id: String,
    pub handle: Option<String>,
    /// Private row counts by table name, so the route can render "12 bookmarks, 3 in progress"
    /// without hardcoding a table list of its own.
    pub private_rows: Vec<(String, i64)>,
    pub published_work_count: i64,
    pub open_export_count: i64,
}

impl ErasurePlan {
    /// The count for one table. `0` for a table with no rows, so a caller never has to
    /// distinguish "absent" from "zero" — a distinction with no meaning here, and exactly the
    /// sort of thing that renders as `undefined` in a client.
    pub fn private_row_count(&self, table: &str) -> i64 {
        self.private_rows
            .iter()
            .find(|(t, _)| t == table)
            .map(|(_, n)| *n)
            .unwrap_or(0)
    }

    /// Whether the plan would destroy anything at all. A reader with nothing stored should be
    /// told the account itself is all that would go, not handed a list of zeroes.
    pub fn has_private_rows(&self) -> bool {
        self.private_rows.iter().any(|(_, n)| *n > 0)
    }

    pub fn total_private_rows(&self) -> i64 {
        self.private_rows.iter().map(|(_, n)| n).sum()
    }
}

/// The tables an erasure reports on, in the order a reader would hear them.
///
/// "your bookmarks, what you were reading, the things you marked as not for you" is a
/// sentence; alphabetical is a dump. These are the *reported* tables, not the *deleted* ones —
/// deletion is the cascade's job (see the module docs).
const PRIVATE_TABLES: &[&str] = &["bookmarks", "reading_progress", "did_not_finish"];

// ─────────────────────────────────────────────────────────────────────────────
// Subject access
// ─────────────────────────────────────────────────────────────────────────────

/// Everything held about `account_id`.
///
/// Every section is a `Vec`, never `Option<Vec<_>>`: a client that has to branch on null-ness
/// is a client with a bug, and an empty reader must still receive `[]`.
pub async fn subject_data(db: &Database, account_id: &str) -> Result<SubjectData> {
    let sql = db.sql(
        "SELECT email FROM accounts WHERE id = ?",
        "SELECT email FROM accounts WHERE id::text = $1",
    );
    let email: Option<String> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar::<_, String>(&sql)
                .bind(account_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar::<_, String>(&sql)
                .bind(account_id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    let sql = db.sql(
        "SELECT handle FROM pseuds WHERE account_id = ? ORDER BY created_at",
        "SELECT handle FROM pseuds WHERE account_id::text = $1 ORDER BY created_at",
    );
    let handles: Vec<String> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar::<_, String>(&sql)
                .bind(account_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar::<_, String>(&sql)
                .bind(account_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    let sql = db.sql(
        "SELECT id, subject_type, subject_id, created_at FROM bookmarks \
         WHERE account_id = ? ORDER BY created_at, id",
        "SELECT id::text AS id, subject_type, subject_id::text AS subject_id, created_at \
         FROM bookmarks WHERE account_id::text = $1 ORDER BY created_at, id",
    );
    let bookmarks: Vec<BookmarkRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, BookmarkRow>(&sql)
                .bind(account_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, BookmarkRow>(&sql)
                .bind(account_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    let sql = db.sql(
        "SELECT id, subject_type, subject_id, position_permille, updated_at \
         FROM reading_progress WHERE account_id = ? ORDER BY updated_at, id",
        "SELECT id::text AS id, subject_type, subject_id::text AS subject_id, \
         position_permille, updated_at FROM reading_progress \
         WHERE account_id::text = $1 ORDER BY updated_at, id",
    );
    let reading_progress: Vec<ReadingProgressRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, ReadingProgressRow>(&sql)
                .bind(account_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, ReadingProgressRow>(&sql)
                .bind(account_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    let sql = db.sql(
        "SELECT id, pseud_id, work_id, reason FROM did_not_finish \
         WHERE account_id = ? ORDER BY id",
        "SELECT id::text AS id, pseud_id::text AS pseud_id, work_id::text AS work_id, reason \
         FROM did_not_finish WHERE account_id::text = $1 ORDER BY id",
    );
    let did_not_finish: Vec<DidNotFinishRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, DidNotFinishRow>(&sql)
                .bind(account_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, DidNotFinishRow>(&sql)
                .bind(account_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(SubjectData {
        account_id: account_id.to_string(),
        email,
        handles,
        bookmarks,
        reading_progress,
        did_not_finish,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Erasure planning
// ─────────────────────────────────────────────────────────────────────────────

/// Count what an erasure would destroy, without destroying anything.
pub async fn plan_erasure(db: &Database, account_id: &str) -> Result<ErasurePlan> {
    let sql = db.sql(
        "SELECT handle FROM pseuds WHERE account_id = ? ORDER BY created_at LIMIT 1",
        "SELECT handle FROM pseuds WHERE account_id::text = $1 ORDER BY created_at LIMIT 1",
    );
    let handle: Option<String> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar::<_, String>(&sql)
                .bind(account_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar::<_, String>(&sql)
                .bind(account_id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    let mut private_rows = Vec::new();
    for table in PRIVATE_TABLES {
        let sql = sql_owned(
            db,
            format!("SELECT COUNT(*) FROM {table} WHERE account_id = ?"),
            format!("SELECT COUNT(*) FROM {table} WHERE account_id::text = $1"),
        );
        let n: i64 = match db.backend() {
            Backend::Sqlite => {
                sqlx::query_scalar::<_, i64>(&sql)
                    .bind(account_id)
                    .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                    .await?
            }
            Backend::Postgres => {
                sqlx::query_scalar::<_, i64>(&sql)
                    .bind(account_id)
                    .fetch_one(db.postgres_pool().expect("postgres handle"))
                    .await?
            }
        };
        private_rows.push((table.to_string(), n));
    }

    // Published works are counted across ALL of the account's handles, not just the first. The
    // plan reports "the works you would lose", and a reader with three handles has three bodies
    // of work — counting only the first under-reports, and errs in the direction that destroys
    // someone's writing without warning.
    let sql = db.sql(
        "SELECT COUNT(*) FROM works WHERE owner_pseud_id IN \
             (SELECT id FROM pseuds WHERE account_id = ?) AND lifecycle = 'published'",
        "SELECT COUNT(*) FROM works WHERE owner_pseud_id IN \
             (SELECT id FROM pseuds WHERE account_id::text = $1) AND lifecycle = 'published'",
    );
    let published_work_count: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar::<_, i64>(&sql)
                .bind(account_id)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar::<_, i64>(&sql)
                .bind(account_id)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    // An export in flight would re-materialise the data moments after erasure.
    let sql = db.sql(
        "SELECT COUNT(*) FROM export_jobs WHERE account_id = ? \
           AND state IN ('queued', 'running')",
        "SELECT COUNT(*) FROM export_jobs WHERE account_id::text = $1 \
           AND state IN ('queued', 'running')",
    );
    let open_export_count: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar::<_, i64>(&sql)
                .bind(account_id)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar::<_, i64>(&sql)
                .bind(account_id)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(ErasurePlan {
        account_id: account_id.to_string(),
        handle,
        private_rows,
        published_work_count,
        open_export_count,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Erasure
// ─────────────────────────────────────────────────────────────────────────────

/// Erase `account_id` and everything hanging off it.
///
/// The deletion is one statement: the 71-table cascade, `api_tokens`' own `ON DELETE CASCADE`,
/// and `jobs.requested_by` going NULL are all the database's doing.
///
/// Open export jobs are cancelled first, deliberately. A job already running would assemble a
/// copy of the reader's data and hand it back through a download grant moments after the
/// account ceased to exist — an erasure that leaks via the export queue is not an erasure.
/// This is the one ordering in the module that is not obvious, so it is the one the spec's
/// mutation step reverts.
pub async fn erase_account(db: &Database, account_id: &str) -> Result<()> {
    let cancel = db.sql(
        "UPDATE export_jobs SET state = 'cancelled' \
         WHERE account_id = ? AND state IN ('queued', 'running')",
        "UPDATE export_jobs SET state = 'cancelled' \
         WHERE account_id::text = $1 AND state IN ('queued', 'running')",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&cancel)
                .bind(account_id)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&cancel)
                .bind(account_id)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }

    let del = db.sql(
        "DELETE FROM accounts WHERE id = ?",
        "DELETE FROM accounts WHERE id::text = $1",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&del)
                .bind(account_id)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&del)
                .bind(account_id)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }

    Ok(())
}
