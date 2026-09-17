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

/// Grant a loan: insert a new work_loans row.
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
        "INSERT INTO work_loans (id, work_id, borrower_account_id, granted_at, expires_at, copy_number, created_at, updated_at, version) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1)",
        "INSERT INTO work_loans (id, work_id, borrower_account_id, granted_at, expires_at, copy_number, created_at, updated_at, version) VALUES (?::uuid, ?::uuid, ?::uuid, ?, ?, ?, ?, ?, 1)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(work_id)
                .bind(borrower_account_id)
                .bind(&now)
                .bind(expires_at)
                .bind(copy_number as i64)
                .bind(&now)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(work_id)
                .bind(borrower_account_id)
                .bind(&now)
                .bind(expires_at)
                .bind(copy_number as i64)
                .bind(&now)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }

    Ok(Loan {
        id,
        work_id: work_id.to_string(),
        borrower_account_id: borrower_account_id.to_string(),
        granted_at: now,
        expires_at: expires_at.to_string(),
        revoked_at: None,
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

/// Find a loan by id.
pub async fn find_loan(db: &Database, loan_id: &str) -> Result<Option<Loan>> {
    let sql = sql_owned(
        db,
        "SELECT id, work_id, borrower_account_id, granted_at, expires_at, revoked_at, copy_number FROM work_loans WHERE id = ?".to_string(),
        "SELECT id::text AS id, work_id::text AS work_id, borrower_account_id::text AS borrower_account_id, granted_at, expires_at, revoked_at, copy_number::bigint AS copy_number FROM work_loans WHERE id = ?::uuid".to_string(),
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
