// M45-21: Structured DNF (did-not-finish) reasons for abandoned works.
//
// A reader marks a work as DNF with one of six structured reasons.
// DNF records are private by default (is_public = FALSE). Authors can
// view DNF counts/reasons on their works with constructive-feedback opt-in.

use anyhow::Result;
use serde::Serialize;
use uuid::Uuid;

use crate::{Backend, Database};
use lorehaven_domain::ids::{AccountId, PseudId, WorkId};

/// One DNF record as returned from the DB.
#[derive(Debug, Clone, Serialize)]
pub struct DnfRow {
    pub id: String,
    pub account_id: String,
    pub pseud_id: String,
    pub work_id: String,
    pub reason: String,
    pub note: Option<String>,
    pub is_public: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// Upsert a DNF record. A pseud can have only one DNF per work
/// (unique partial index on (pseud_id, work_id) WHERE deleted_at IS NULL).
// DB functions take their parameters explicitly rather than a builder:
// a builder here would only move the same fields one call deeper.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_dnf(
    db: &Database,
    account_id: AccountId,
    pseud_id: PseudId,
    work_id: WorkId,
    reason: &str,
    note: Option<&str>,
    is_public: bool,
    now: &str,
) -> Result<()> {
    let sql = db.sql(
        "INSERT INTO did_not_finish
            (id, account_id, pseud_id, work_id, reason, note, is_public, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (pseud_id, work_id) WHERE deleted_at IS NULL
         DO UPDATE SET reason = excluded.reason,
                       note = excluded.note,
                       is_public = excluded.is_public,
                       updated_at = excluded.updated_at",
        "INSERT INTO did_not_finish
            (id, account_id, pseud_id, work_id, reason, note, is_public, created_at, updated_at)
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid, $5, $6, $7::boolean, $8::timestamptz, $9::timestamptz)
         ON CONFLICT (pseud_id, work_id)
         DO UPDATE SET reason = excluded.reason,
                       note = excluded.note,
                       is_public = excluded.is_public,
                       updated_at = excluded.updated_at",
    );
    let id = Uuid::new_v4();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(id.to_string())
                .bind(account_id.to_string())
                .bind(pseud_id.to_string())
                .bind(work_id.to_string())
                .bind(reason)
                .bind(note)
                .bind(is_public)
                .bind(now)
                .bind(now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(id)
                .bind(account_id.to_string())
                .bind(pseud_id.to_string())
                .bind(work_id.to_string())
                .bind(reason)
                .bind(note)
                .bind(is_public)
                .bind(now)
                .bind(now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }

    Ok(())
}

/// Soft-delete a DNF record (set deleted_at).
pub async fn delete_dnf(
    db: &Database,
    pseud_id: PseudId,
    work_id: WorkId,
    now: &str,
) -> Result<bool> {
    let sql = db.sql(
        "UPDATE did_not_finish SET deleted_at = ?
         WHERE pseud_id = ? AND work_id = ? AND deleted_at IS NULL",
        "UPDATE did_not_finish SET deleted_at = $1
         WHERE pseud_id = $2::uuid AND work_id = $3::uuid AND deleted_at IS NULL",
    );

    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(now)
            .bind(pseud_id.to_string())
            .bind(work_id.to_string())
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(now)
            .bind(pseud_id.to_string())
            .bind(work_id.to_string())
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };

    Ok(affected > 0)
}

/// Read a single DNF record (the pseud's own record for a work).
pub async fn read_dnf(db: &Database, pseud_id: PseudId, work_id: WorkId) -> Result<Option<DnfRow>> {
    let sql = db.sql(
        "SELECT id, account_id, pseud_id, work_id, reason, note, is_public, created_at, updated_at
         FROM did_not_finish
         WHERE pseud_id = ? AND work_id = ? AND deleted_at IS NULL",
        "SELECT id, account_id, pseud_id, work_id, reason, note, is_public, created_at, updated_at
         FROM did_not_finish
         WHERE pseud_id = $1::uuid AND work_id = $2::uuid AND deleted_at IS NULL",
    );

    let row = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<
                _,
                (
                    String,
                    String,
                    String,
                    String,
                    String,
                    Option<String>,
                    bool,
                    String,
                    String,
                ),
            >(&sql)
            .bind(pseud_id.to_string())
            .bind(work_id.to_string())
            .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
            .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<
                _,
                (
                    String,
                    String,
                    String,
                    String,
                    String,
                    Option<String>,
                    bool,
                    String,
                    String,
                ),
            >(&sql)
            .bind(pseud_id.to_string())
            .bind(work_id.to_string())
            .fetch_optional(db.postgres_pool().expect("postgres handle"))
            .await?
        }
    };

    Ok(row.map(
        |(id, account_id, pseud_id, work_id, reason, note, is_public, created_at, updated_at)| {
            DnfRow {
                id,
                account_id,
                pseud_id,
                work_id,
                reason,
                note,
                is_public,
                created_at,
                updated_at,
            }
        },
    ))
}

/// List all DNF records for a work (public-only by default).
///
/// If `include_private` is true and the requester owns the work, all
/// records are returned (regardless of is_public flag).
pub async fn list_dnf_for_work(
    db: &Database,
    work_id: WorkId,
    include_private: bool,
) -> Result<Vec<DnfRow>> {
    let sql = if include_private {
        db.sql(
            "SELECT id, account_id, pseud_id, work_id, reason, note, is_public, created_at, updated_at
             FROM did_not_finish
             WHERE work_id = ? AND deleted_at IS NULL
             ORDER BY created_at DESC",
            "SELECT id, account_id, pseud_id, work_id, reason, note, is_public, created_at, updated_at
             FROM did_not_finish
             WHERE work_id = $1::uuid AND deleted_at IS NULL
             ORDER BY created_at DESC",
        )
    } else {
        db.sql(
            "SELECT id, account_id, pseud_id, work_id, reason, note, is_public, created_at, updated_at
             FROM did_not_finish
             WHERE work_id = ? AND is_public = TRUE AND deleted_at IS NULL
             ORDER BY created_at DESC",
            "SELECT id, account_id, pseud_id, work_id, reason, note, is_public, created_at, updated_at
             FROM did_not_finish
             WHERE work_id = $1::uuid AND is_public = TRUE AND deleted_at IS NULL
             ORDER BY created_at DESC",
        )
    };

    let rows = match db.backend() {
        Backend::Sqlite => {
            let q = sqlx::query_as::<
                _,
                (
                    String,
                    String,
                    String,
                    String,
                    String,
                    Option<String>,
                    bool,
                    String,
                    String,
                ),
            >(&sql)
            .bind(work_id.to_string());

            q.fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<
                _,
                (
                    String,
                    String,
                    String,
                    String,
                    String,
                    Option<String>,
                    bool,
                    String,
                    String,
                ),
            >(&sql)
            .bind(work_id.to_string())
            .fetch_all(db.postgres_pool().expect("postgres handle"))
            .await?
        }
    };

    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                account_id,
                pseud_id,
                work_id,
                reason,
                note,
                is_public,
                created_at,
                updated_at,
            )| {
                DnfRow {
                    id,
                    account_id,
                    pseud_id,
                    work_id,
                    reason,
                    note,
                    is_public,
                    created_at,
                    updated_at,
                }
            },
        )
        .collect())
}

/// Aggregate DNF reason counts for a work (public-only).
/// Returns a map of reason → count, sorted by count descending.
pub async fn aggregate_dnf_counts(db: &Database, work_id: WorkId) -> Result<Vec<(String, i64)>> {
    let sql = db.sql(
        "SELECT reason, COUNT(*) AS cnt
         FROM did_not_finish
         WHERE work_id = ? AND is_public = TRUE AND deleted_at IS NULL
         GROUP BY reason ORDER BY cnt DESC",
        "SELECT reason, COUNT(*) AS cnt
         FROM did_not_finish
         WHERE work_id = $1::uuid AND is_public = TRUE AND deleted_at IS NULL
         GROUP BY reason ORDER BY cnt DESC",
    );

    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String, i64)>(&sql)
                .bind(work_id.to_string())
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String, i64)>(&sql)
                .bind(work_id.to_string())
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(rows)
}

/// Check if a work allows DNF feedback (author opt-in).
pub async fn work_allows_dnf_feedback(db: &Database, work_id: WorkId) -> Result<bool> {
    // `db.sql`, not one literal for both arms: `?` is SQLite's placeholder and
    // PostgreSQL rejects it, so a shared literal silently disables the read.
    let sql = db.sql(
        "SELECT allow_dnf_feedback FROM works WHERE id = ?",
        "SELECT allow_dnf_feedback FROM works WHERE id = ?::uuid",
    );

    let row: Option<(bool,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work_id.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work_id.to_string())
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(row.map(|(v,)| v).unwrap_or(false))
}

/// List all DNF records by a pseud (for their personal reading history).
pub async fn list_dnf_by_pseud(db: &Database, pseud_id: PseudId) -> Result<Vec<DnfRow>> {
    let sql = db.sql(
        "SELECT id, account_id, pseud_id, work_id, reason, note, is_public, created_at, updated_at
         FROM did_not_finish
         WHERE pseud_id = ? AND deleted_at IS NULL
         ORDER BY updated_at DESC",
        "SELECT id, account_id, pseud_id, work_id, reason, note, is_public, created_at, updated_at
         FROM did_not_finish
         WHERE pseud_id = $1::uuid AND deleted_at IS NULL
         ORDER BY updated_at DESC",
    );

    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<
                _,
                (
                    String,
                    String,
                    String,
                    String,
                    String,
                    Option<String>,
                    bool,
                    String,
                    String,
                ),
            >(&sql)
            .bind(pseud_id.to_string())
            .fetch_all(db.sqlite_pool().expect("sqlite handle"))
            .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<
                _,
                (
                    String,
                    String,
                    String,
                    String,
                    String,
                    Option<String>,
                    bool,
                    String,
                    String,
                ),
            >(&sql)
            .bind(pseud_id.to_string())
            .fetch_all(db.postgres_pool().expect("postgres handle"))
            .await?
        }
    };

    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                account_id,
                pseud_id,
                work_id,
                reason,
                note,
                is_public,
                created_at,
                updated_at,
            )| {
                DnfRow {
                    id,
                    account_id,
                    pseud_id,
                    work_id,
                    reason,
                    note,
                    is_public,
                    created_at,
                    updated_at,
                }
            },
        )
        .collect())
}
