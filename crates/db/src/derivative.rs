//! Derivative pipeline repository (spec §32.4, M25).

use anyhow::Result;
use lorehaven_domain::derivative::DerivativeKind;
use sqlx::FromRow;

use crate::{sql_owned, Backend, Database};

#[derive(Debug, Clone)]
pub struct Derivative {
    pub id: String,
    pub work_id: String,
    pub edition_kind: String,
    pub derivative_kind: String,
    pub parent_checksum: String,
    pub output_checksum: Option<String>,
    pub output_bytes: Option<i64>,
    pub output_mime_type: Option<String>,
    pub state: String,
    pub job_id: Option<String>,
    pub error_message: Option<String>,
    pub built_at: Option<String>,
    pub verified_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewDerivative<'a> {
    pub work_id: &'a str,
    pub edition_kind: &'a str,
    pub derivative_kind: DerivativeKind,
    pub parent_checksum: &'a str,
}

#[derive(Debug, FromRow)]
struct DerivativeRow {
    id: String,
    work_id: String,
    edition_kind: String,
    derivative_kind: String,
    parent_checksum: String,
    output_checksum: Option<String>,
    output_bytes: Option<i64>,
    output_mime_type: Option<String>,
    state: String,
    job_id: Option<String>,
    error_message: Option<String>,
    built_at: Option<String>,
    verified_at: Option<String>,
}

impl From<DerivativeRow> for Derivative {
    fn from(row: DerivativeRow) -> Self {
        Self {
            id: row.id,
            work_id: row.work_id,
            edition_kind: row.edition_kind,
            derivative_kind: row.derivative_kind,
            parent_checksum: row.parent_checksum,
            output_checksum: row.output_checksum,
            output_bytes: row.output_bytes,
            output_mime_type: row.output_mime_type,
            state: row.state,
            job_id: row.job_id,
            error_message: row.error_message,
            built_at: row.built_at,
            verified_at: row.verified_at,
        }
    }
}

/// Insert a new derivative row.
pub async fn create_derivative(db: &Database, new: NewDerivative<'_>) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO derivatives (id, work_id, edition_kind, derivative_kind, parent_checksum, state, created_at, updated_at, version) VALUES (?, ?, ?, ?, ?, 'queued', ?, ?, 1)",
        "INSERT INTO derivatives (id, work_id, edition_kind, derivative_kind, parent_checksum, state, created_at, updated_at, version) VALUES (?::uuid, ?::uuid, ?, ?, ?, 'queued', ?, ?, 1)",
    );
    match db.backend() {
        Backend::Sqlite => {
            let _ = sqlx::query(&sql)
                .bind(&id)
                .bind(new.work_id)
                .bind(new.edition_kind)
                .bind(new.derivative_kind.as_str())
                .bind(new.parent_checksum)
                .bind(&now)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            let _ = sqlx::query(&sql)
                .bind(&id)
                .bind(new.work_id)
                .bind(new.edition_kind)
                .bind(new.derivative_kind.as_str())
                .bind(new.parent_checksum)
                .bind(&now)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    };
    Ok(id)
}

/// Find derivatives for a work.
pub async fn list_derivatives(db: &Database, work_id: &str) -> Result<Vec<Derivative>> {
    let sql = sql_owned(
        db,
        "SELECT id, work_id, edition_kind, derivative_kind, parent_checksum, output_checksum, output_bytes, output_mime_type, state, job_id, error_message, built_at, verified_at FROM derivatives WHERE work_id = ? ORDER BY created_at".to_string(),
        "SELECT id::text AS id, work_id::text AS work_id, edition_kind, derivative_kind, parent_checksum, output_checksum, output_bytes, output_mime_type, state, job_id::text AS job_id, error_message, built_at, verified_at FROM derivatives WHERE work_id = ?::uuid ORDER BY created_at".to_string(),
    );
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows.into_iter().map(|r: DerivativeRow| r.into()).collect())
}

/// Mark a derivative as built (ready with output).
pub async fn mark_derivative_built(
    db: &Database,
    id: &str,
    output_checksum: &str,
    output_bytes: i64,
    output_mime_type: &str,
) -> Result<bool> {
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "UPDATE derivatives SET output_checksum = ?, output_bytes = ?, output_mime_type = ?, state = 'ready', built_at = ?, updated_at = ?, version = version + 1 WHERE id = ?",
        "UPDATE derivatives SET output_checksum = ?, output_bytes = ?, output_mime_type = ?, state = 'ready', built_at = ?, updated_at = ?, version = version + 1 WHERE id = ?::uuid",
    );
    let rows_affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(output_checksum)
            .bind(output_bytes)
            .bind(output_mime_type)
            .bind(&now)
            .bind(&now)
            .bind(id)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(output_checksum)
            .bind(output_bytes)
            .bind(output_mime_type)
            .bind(&now)
            .bind(&now)
            .bind(id)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(rows_affected > 0)
}

/// Mark a derivative as failed.
pub async fn mark_derivative_failed(db: &Database, id: &str, message: &str) -> Result<bool> {
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "UPDATE derivatives SET state = 'failed', error_message = ?, updated_at = ?, version = version + 1 WHERE id = ?",
        "UPDATE derivatives SET state = 'failed', error_message = ?, updated_at = ?, version = version + 1 WHERE id = ?::uuid",
    );
    let rows_affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(message)
            .bind(&now)
            .bind(id)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(message)
            .bind(&now)
            .bind(id)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(rows_affected > 0)
}

/// Mark a derivative as stale (parent changed).
pub async fn mark_derivative_stale(db: &Database, id: &str) -> Result<bool> {
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "UPDATE derivatives SET state = 'stale', updated_at = ?, version = version + 1 WHERE id = ? AND state != 'stale'",
        "UPDATE derivatives SET state = 'stale', updated_at = ?, version = version + 1 WHERE id = ?::uuid AND state != 'stale'",
    );
    let rows_affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(&now)
            .bind(id)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(&now)
            .bind(id)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(rows_affected > 0)
}

/// Find derivatives that need re-verification (verified_at older than threshold).
pub async fn find_stale_for_verification(
    db: &Database,
    older_than_rfc3339: &str,
    limit: i64,
) -> Result<Vec<Derivative>> {
    let sqlite_sql = "SELECT id, work_id, edition_kind, derivative_kind, parent_checksum, output_checksum, output_bytes, output_mime_type, state, job_id, error_message, built_at, verified_at FROM derivatives WHERE state = 'ready' AND (verified_at IS NULL OR verified_at < ?) ORDER BY verified_at IS NULL DESC, verified_at LIMIT ?".to_string();
    let postgres_sql = "SELECT id::text AS id, work_id::text AS work_id, edition_kind, derivative_kind, parent_checksum, output_checksum, output_bytes, output_mime_type, state, job_id::text AS job_id, error_message, built_at, verified_at FROM derivatives WHERE state = 'ready' AND (verified_at IS NULL OR verified_at < ?) ORDER BY verified_at NULLS LAST LIMIT ?".to_string();
    let rows = match db.backend() {
        Backend::Sqlite => {
            let rows: Vec<DerivativeRow> = sqlx::query_as(&sqlite_sql)
                .bind(older_than_rfc3339)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            rows.into_iter().map(|r| r.into()).collect()
        }
        Backend::Postgres => {
            let rows: Vec<DerivativeRow> = sqlx::query_as(&postgres_sql)
                .bind(older_than_rfc3339)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?;
            rows.into_iter().map(|r| r.into()).collect()
        }
    };
    Ok(rows)
}

/// Update verified_at timestamp after successful re-verification.
pub async fn touch_derivative_verified(db: &Database, id: &str) -> Result<bool> {
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "UPDATE derivatives SET verified_at = ?, updated_at = ?, version = version + 1 WHERE id = ?",
        "UPDATE derivatives SET verified_at = ?, updated_at = ?, version = version + 1 WHERE id = ?::uuid",
    );
    let rows_affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(&now)
            .bind(&now)
            .bind(id)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(&now)
            .bind(&now)
            .bind(id)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(rows_affected > 0)
}

/// Find one derivative by id.
pub async fn find_derivative(db: &Database, id: &str) -> Result<Option<Derivative>> {
    let sql = sql_owned(
        db,
        "SELECT id, work_id, edition_kind, derivative_kind, parent_checksum, output_checksum, output_bytes, output_mime_type, state, job_id, error_message, built_at, verified_at FROM derivatives WHERE id = ?".to_string(),
        "SELECT id::text AS id, work_id::text AS work_id, edition_kind, derivative_kind, parent_checksum, output_checksum, output_bytes, output_mime_type, state, job_id::text AS job_id, error_message, built_at, verified_at FROM derivatives WHERE id = ?::uuid".to_string(),
    );
    let row: Option<DerivativeRow> = match db.backend() {
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
    Ok(row.map(|r| r.into()))
}
