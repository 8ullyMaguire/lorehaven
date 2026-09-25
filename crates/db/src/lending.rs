//! Controlled digital lending repository (spec §32.4, M25).

use anyhow::Result;
use lorehaven_domain::lending::Loan;
use sqlx::{FromRow, Row};

use crate::{sql_owned, Backend, Database};

#[derive(Debug, Clone)]
pub struct LendingConfig {
    pub enabled: bool,
    pub copies_per_work: u32,
    pub loan_duration_days: u32,
    pub borrower_max_loans: u32,
}

#[derive(Debug, FromRow)]
struct LoanRow {
    id: String,
    work_id: String,
    borrower_account_id: String,
    granted_at: String,
    expires_at: String,
    revoked_at: Option<String>,
    expired_at: Option<String>,
    copy_number: i64,
}

impl From<LoanRow> for Loan {
    fn from(row: LoanRow) -> Self {
        Self {
            id: row.id,
            work_id: row.work_id,
            borrower_account_id: row.borrower_account_id,
            granted_at: row.granted_at,
            expires_at: row.expires_at,
            revoked_at: row.revoked_at,
            expired_at: row.expired_at,
            copy_number: row.copy_number as u32,
        }
    }
}

/// Load the singleton lending configuration. Inserts defaults if absent.
pub async fn get_lending_config(db: &Database) -> Result<LendingConfig> {
    let now = crate::identity::now_rfc3339();
    let init_sql = db.sql(
        "INSERT OR IGNORE INTO lending_config (singleton, enabled, copies_per_work, loan_duration_days, borrower_max_loans, updated_at, version) VALUES (1, 0, 1, 14, 5, ?, 1)",
        "INSERT INTO lending_config (singleton, enabled, copies_per_work, loan_duration_days, borrower_max_loans, updated_at, version) VALUES (1, FALSE, 1, 14, 5, ?, 1) ON CONFLICT DO NOTHING",
    );
    match db.backend() {
        Backend::Sqlite => {
            let _ = sqlx::query(&init_sql)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await;
        }
        Backend::Postgres => {
            let _ = sqlx::query(&init_sql)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await;
        }
    }

    let sql = sql_owned(
        db,
        "SELECT enabled, copies_per_work, loan_duration_days, borrower_max_loans FROM lending_config WHERE singleton = 1".to_string(),
        "SELECT enabled, copies_per_work, loan_duration_days, borrower_max_loans FROM lending_config WHERE singleton = 1".to_string(),
    );
    let (enabled, copies, duration, max_loans) = match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(&sql)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            (
                row.get::<bool, _>("enabled"),
                row.get::<i64, _>("copies_per_work"),
                row.get::<i64, _>("loan_duration_days"),
                row.get::<i64, _>("borrower_max_loans"),
            )
        }
        Backend::Postgres => {
            let row = sqlx::query(&sql)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?;
            (
                row.get::<bool, _>("enabled"),
                row.get::<i64, _>("copies_per_work"),
                row.get::<i64, _>("loan_duration_days"),
                row.get::<i64, _>("borrower_max_loans"),
            )
        }
    };

    Ok(LendingConfig {
        enabled,
        copies_per_work: copies as u32,
        loan_duration_days: duration as u32,
        borrower_max_loans: max_loans as u32,
    })
}

/// Count active (non-expired, non-revoked) loans for a work.
pub async fn count_active_loans(db: &Database, work_id: &str) -> Result<i64> {
    let now = crate::identity::now_rfc3339();
    let sql = sql_owned(
        db,
        "SELECT COUNT(*) FROM work_loans WHERE work_id = ? AND revoked_at IS NULL AND expires_at > ?".to_string(),
        "SELECT COUNT(*) FROM work_loans WHERE work_id = ?::uuid AND revoked_at IS NULL AND expires_at > ?".to_string(),
    );
    let count = match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(&sql)
                .bind(work_id)
                .bind(&now)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            row.get::<i64, _>(0)
        }
        Backend::Postgres => {
            let row = sqlx::query(&sql)
                .bind(work_id)
                .bind(&now)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?;
            row.get::<i64, _>(0)
        }
    };
    Ok(count)
}

/// Count active loans held by a borrower.
pub async fn count_borrower_loans(db: &Database, account_id: &str) -> Result<i64> {
    let now = crate::identity::now_rfc3339();
    let sql = sql_owned(
        db,
        "SELECT COUNT(*) FROM work_loans WHERE borrower_account_id = ? AND revoked_at IS NULL AND expires_at > ?".to_string(),
        "SELECT COUNT(*) FROM work_loans WHERE borrower_account_id = ?::uuid AND revoked_at IS NULL AND expires_at > ?".to_string(),
    );
    let count = match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(&sql)
                .bind(account_id)
                .bind(&now)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            row.get::<i64, _>(0)
        }
        Backend::Postgres => {
            let row = sqlx::query(&sql)
                .bind(account_id)
                .bind(&now)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?;
            row.get::<i64, _>(0)
        }
    };
    Ok(count)
}

/// Grant a loan: a new row, or the reader's own row for this work re-granted.
///
/// `work_loans` has `UNIQUE (work_id, borrower_account_id)`, so a reader whose
/// loan expired — or who revoked one and changed their mind — still has a row.
/// Inserting a second would be a unique violation on PostgreSQL and a 500 the
/// reader cannot act on, and the visible symptom is the worst kind: the copy is
/// free, the reader is eligible, and borrowing "fails". A re-grant is an UPDATE
/// of that row, which is what the constraint means.
pub async fn grant_loan(
    db: &Database,
    work_id: &str,
    borrower_account_id: &str,
    copy_number: u32,
    expires_at: &str,
) -> Result<Loan> {
    let now = crate::identity::now_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();
    let sql = db.sql(
        "INSERT INTO work_loans
             (id, work_id, borrower_account_id, granted_at, expires_at, copy_number,
              revoked_at, created_at, updated_at, version)
         VALUES (?, ?, ?, ?, ?, ?, NULL, ?, ?, 1)
         ON CONFLICT (work_id, borrower_account_id) DO UPDATE SET
             granted_at = excluded.granted_at,
             expires_at = excluded.expires_at,
             copy_number = excluded.copy_number,
             revoked_at = NULL,
             expired_at = NULL,
             updated_at = excluded.updated_at,
             version = version + 1
         RETURNING id",
        "INSERT INTO work_loans
             (id, work_id, borrower_account_id, granted_at, expires_at, copy_number,
              revoked_at, created_at, updated_at, version)
         VALUES (?::uuid, ?::uuid, ?::uuid, ?, ?, ?, NULL, ?, ?, 1)
         ON CONFLICT (work_id, borrower_account_id) DO UPDATE SET
             granted_at = excluded.granted_at,
             expires_at = excluded.expires_at,
             copy_number = excluded.copy_number,
             revoked_at = NULL,
             expired_at = NULL,
             updated_at = excluded.updated_at,
             version = version + 1
         RETURNING id::text AS id",
    );
    // A re-grant keeps the row's original id, so the id comes back from the
    // statement rather than from the value this function generated.
    let loan_id: String = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(&id)
                .bind(work_id)
                .bind(borrower_account_id)
                .bind(&now)
                .bind(expires_at)
                .bind(copy_number as i64)
                .bind(&now)
                .bind(&now)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(&id)
                .bind(work_id)
                .bind(borrower_account_id)
                .bind(&now)
                .bind(expires_at)
                .bind(copy_number as i64)
                .bind(&now)
                .bind(&now)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(Loan {
        id: loan_id,
        work_id: work_id.to_string(),
        borrower_account_id: borrower_account_id.to_string(),
        granted_at: now,
        expires_at: expires_at.to_string(),
        revoked_at: None,
        expired_at: None,
        copy_number,
    })
}

/// Revoke a loan by setting revoked_at. Returns true if the loan was active.
pub async fn revoke_loan(db: &Database, loan_id: &str) -> Result<bool> {
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "UPDATE work_loans SET revoked_at = ?, updated_at = ?, version = version + 1 WHERE id = ? AND revoked_at IS NULL",
        "UPDATE work_loans SET revoked_at = ?, updated_at = ?, version = version + 1 WHERE id = ?::uuid AND revoked_at IS NULL",
    );
    let rows_affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(&now)
            .bind(&now)
            .bind(loan_id)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(&now)
            .bind(&now)
            .bind(loan_id)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(rows_affected > 0)
}

/// The column list every `LoanRow` read uses.
///
/// A `FromRow` struct fails at runtime when a column it names is missing from
/// the result set, so the list lives next to the struct rather than being
/// retyped per query — the two cannot drift apart this way.
const LOAN_COLUMNS_SQLITE: &str = "id, work_id, borrower_account_id, granted_at, expires_at, \
                                   revoked_at, expired_at, copy_number";

const LOAN_COLUMNS_POSTGRES: &str = "id::text AS id, work_id::text AS work_id, \
                                     borrower_account_id::text AS borrower_account_id, granted_at, \
                                     expires_at, revoked_at, expired_at, copy_number::bigint AS copy_number";

/// Find a loan by id.
pub async fn find_loan(db: &Database, loan_id: &str) -> Result<Option<Loan>> {
    let sql = sql_owned(
        db,
        format!("SELECT {LOAN_COLUMNS_SQLITE} FROM work_loans WHERE id = ?"),
        format!("SELECT {LOAN_COLUMNS_POSTGRES} FROM work_loans WHERE id = ?::uuid"),
    );
    let row: Option<LoanRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(loan_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(loan_id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(Loan::from))
}

/// Every loan a borrower holds, newest window first.
///
/// Expired and revoked rows are included: a reader's loan list is where "the
/// window closed" and "you revoked it" are told apart, and hiding those rows
/// would leave a reader with no record that they ever had the item.
pub async fn list_loans_for_borrower(
    db: &Database,
    borrower_account_id: &str,
) -> Result<Vec<Loan>> {
    let sql = sql_owned(
        db,
        format!(
            "SELECT {LOAN_COLUMNS_SQLITE} FROM work_loans WHERE borrower_account_id = ? \
             ORDER BY granted_at DESC, id ASC"
        ),
        format!(
            "SELECT {LOAN_COLUMNS_POSTGRES} FROM work_loans \
             WHERE borrower_account_id = ?::uuid ORDER BY granted_at DESC, id ASC"
        ),
    );
    let rows: Vec<LoanRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(borrower_account_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(borrower_account_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows.into_iter().map(Loan::from).collect())
}

/// Stamp every loan whose window has closed, and return what was stamped.
///
/// The returned rows are what the caller reports: the count an operator reads
/// and the borrowers a notification would reach. The `expired_at IS NULL`
/// guard is what makes this a transition rather than a re-stamp — a sweep that
/// rewrote the timestamp every pass would turn "when did this end" into "when
/// did the worker last run".
pub async fn expire_due_loans(db: &Database, now: &str) -> Result<Vec<Loan>> {
    let sql = sql_owned(
        db,
        format!(
            "UPDATE work_loans SET expired_at = ?, updated_at = ?, version = version + 1 \
             WHERE revoked_at IS NULL AND expired_at IS NULL AND expires_at <= ? \
             RETURNING {LOAN_COLUMNS_SQLITE}"
        ),
        format!(
            "UPDATE work_loans SET expired_at = ?, updated_at = ?, version = version + 1 \
             WHERE revoked_at IS NULL AND expired_at IS NULL AND expires_at <= ? \
             RETURNING {LOAN_COLUMNS_POSTGRES}"
        ),
    );
    let rows: Vec<LoanRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(now)
                .bind(now)
                .bind(now)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(now)
                .bind(now)
                .bind(now)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows.into_iter().map(Loan::from).collect())
}
