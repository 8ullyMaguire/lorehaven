//! Controlled digital lending (M25 / spec §32.4).

use axum::extract::{Path, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

/// POST /api/v1/media/:id/lend — borrow this work.
#[derive(Debug, Deserialize)]
pub struct LendRequest {}

/// PUT /api/v1/media/:id/lend — revoke a loan (borrower only).
#[derive(Debug, Deserialize)]
pub struct RevokeRequest {}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/media/{id}/lending", get(get_lending_status))
        .route("/media/{id}/lend", post(lend_work))
        .route("/media/{id}/lend", put(revoke_loan))
        .route("/me/loans", get(list_my_loans))
}

/// The caller's own loans, newest window first.
///
/// A loan is a bounded window the reader agreed to, so the reader gets to see
/// the window, its end, and how it ended: `active`, `expired` or `revoked`.
/// Only the caller's own rows, and only their own account id keys them.
pub async fn list_my_loans(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
) -> ApiResult<Json<Value>> {
    let db = state.db();
    let now = now_rfc3339();
    let loans =
        lorehaven_db::lending::list_loans_for_borrower(db, &session.account_id.to_string()).await?;
    let items: Vec<Value> = loans
        .iter()
        .map(|loan| {
            json!({
                "id": loan.id,
                "work_id": loan.work_id,
                "state": loan.state_at(&now),
                "granted_at": loan.granted_at,
                "expires_at": loan.expires_at,
                "revoked_at": loan.revoked_at,
                "expired_at": loan.expired_at,
                "copy_number": loan.copy_number,
            })
        })
        .collect();
    Ok(Json(json!({ "items": items, "next_cursor": null })))
}

fn now_rfc3339() -> String {
    use time::format_description::well_known::Rfc3339;
    time::OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .expect("rfc3339 format")
}

fn validation(message: &str) -> ApiError {
    ApiError::from(lorehaven_domain::AppError::Validation {
        message: message.into(),
        field_errors: std::collections::BTreeMap::new(),
    })
}

pub async fn get_lending_status(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let db = state.db();
    let account_id = session.account_id.to_string();

    let config = lorehaven_db::lending::get_lending_config(db).await?;

    if !config.enabled {
        return Ok(Json(json!({
            "enabled": false,
            "lendable": false,
            "current_loan": null,
            "can_borrow": false,
        })));
    }

    let _ = lorehaven_db::media::find_media(db, &id)
        .await?
        .ok_or(lorehaven_domain::AppError::NotFound { resource: "work" })?;

    let (lendable, can_borrow) = lending_capacity(db, &id, &account_id, &config)
        .await
        .ok_or_else(|| {
            ApiError::from(lorehaven_domain::AppError::Internal(anyhow::anyhow!(
                "internal"
            )))
        })?;

    let current_loan = check_active_loan(db, &id, &account_id).await;

    Ok(Json(json!({
        "enabled": true,
        "lendable": lendable,
        "current_loan": current_loan,
        "can_borrow": can_borrow,
    })))
}

pub async fn lend_work(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
    Path(id): Path<String>,
    _body: Json<LendRequest>,
) -> ApiResult<Json<Value>> {
    let db = state.db();
    let account_id = session.account_id.to_string();

    let config = lorehaven_db::lending::get_lending_config(db).await?;

    if !config.enabled {
        return Err(validation("lending is disabled"));
    }

    let media = lorehaven_db::media::find_media(db, &id)
        .await?
        .ok_or(lorehaven_domain::AppError::NotFound { resource: "work" })?;

    if !matches!(media.lifecycle.as_str(), "published" | "active") {
        return Err(validation("only published works may be lent"));
    }

    let (lendable, can_borrow) = lending_capacity(db, &id, &account_id, &config)
        .await
        .ok_or_else(|| {
            ApiError::from(lorehaven_domain::AppError::Internal(anyhow::anyhow!(
                "internal"
            )))
        })?;

    if !lendable {
        return Err(validation("this work is not available for lending"));
    }

    if !can_borrow {
        return Err(validation("all copies of this work are on loan"));
    }

    let borrower_count = lorehaven_db::lending::count_borrower_loans(db, &account_id)
        .await
        .unwrap_or(0);
    if (borrower_count as u32) >= config.borrower_max_loans {
        return Err(validation(&format!(
            "you have reached the maximum of {} active loans",
            config.borrower_max_loans
        )));
    }

    let now = time::OffsetDateTime::now_utc();
    let expires_at = now + time::Duration::days(config.loan_duration_days as i64);
    let expires_at_str = now_rfc3339_from_time(expires_at);
    let active_count = lorehaven_db::lending::count_active_loans(db, &id)
        .await
        .unwrap_or(0);

    let loan = lorehaven_db::lending::grant_loan(
        db,
        &id,
        &account_id,
        (active_count as u32) + 1,
        &expires_at_str,
    )
    .await?;

    Ok(Json(json!({
        "id": loan.id,
        "work_id": loan.work_id,
        "expires_at": loan.expires_at,
    })))
}

pub async fn revoke_loan(
    State(state): State<AppState>,
    RequireSession(session): RequireSession,
    Path(id): Path<String>,
    _body: Json<RevokeRequest>,
) -> ApiResult<Json<Value>> {
    let db = state.db();
    let account_id = session.account_id.to_string();

    if let Some(loan_id) = check_active_loan_id(db, &id, &account_id).await {
        let result = lorehaven_db::lending::revoke_loan(db, &loan_id).await?;
        if result {
            Ok(Json(json!({"ok": true})))
        } else {
            Err(validation("already revoked"))
        }
    } else {
        Err(validation("no active loan"))
    }
}

async fn lending_capacity(
    db: &lorehaven_db::Database,
    work_id: &str,
    _account_id: &str,
    config: &lorehaven_db::lending::LendingConfig,
) -> Option<(bool, bool)> {
    let sql = db.sql(
        "SELECT lending_class FROM media_rights WHERE work_id = ?",
        "SELECT lending_class FROM media_rights WHERE work_id = ?::uuid",
    );
    let lending_class: Option<String> = match db.backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query(&sql)
            .bind(work_id)
            .fetch_optional(db.sqlite_pool()?)
            .await
            .ok()?
            .map(|r: sqlx::sqlite::SqliteRow| r.get("lending_class")),
        lorehaven_db::Backend::Postgres => sqlx::query(&sql)
            .bind(work_id)
            .fetch_optional(db.postgres_pool()?)
            .await
            .ok()?
            .map(|r: sqlx::postgres::PgRow| r.get("lending_class")),
    };
    let lendable = lending_class.as_deref() == Some("lending");
    let active_count = lorehaven_db::lending::count_active_loans(db, work_id)
        .await
        .unwrap_or(0);
    let can_borrow = lendable && (active_count as u32) < config.copies_per_work;
    Some((lendable, can_borrow))
}

async fn check_active_loan_id(
    db: &lorehaven_db::Database,
    work_id: &str,
    account_id: &str,
) -> Option<String> {
    let now = now_rfc3339();
    let sql = db.sql(
        "SELECT id FROM work_loans WHERE work_id = ? AND borrower_account_id = ? AND revoked_at IS NULL AND expires_at > ? LIMIT 1",
        // `work_loans.id` is UUID and this reads it as String, so the SELECT list
        // needs the cast. The two WHERE casts are the opposite direction.
        "SELECT id::text AS id FROM work_loans WHERE work_id = ?::uuid AND borrower_account_id = ?::uuid AND revoked_at IS NULL AND expires_at > ? LIMIT 1",
    );
    let row = match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            let r = sqlx::query(&sql)
                .bind(work_id)
                .bind(account_id)
                .bind(&now)
                .fetch_optional(db.sqlite_pool()?)
                .await
                .ok()?;
            // `try_get`, not `get().ok()`. Both arms return `Option<String>`, but
            // `get` panics the request task on a decode failure -- which is what a
            // UUID column read as String does -- so a wrong cast on the lending
            // path surfaced as a panicked handler rather than a reported error.
            r.and_then(|row| row.try_get::<String, _>("id").ok())
        }
        lorehaven_db::Backend::Postgres => {
            let r = sqlx::query(&sql)
                .bind(work_id)
                .bind(account_id)
                .bind(&now)
                .fetch_optional(db.postgres_pool()?)
                .await
                .ok()?;
            r.and_then(|row| row.try_get::<String, _>("id").ok())
        }
    };
    row
}

async fn check_active_loan(
    db: &lorehaven_db::Database,
    work_id: &str,
    account_id: &str,
) -> Option<Value> {
    check_active_loan_id(db, work_id, account_id)
        .await
        .map(|id| json!({ "id": id, "work_id": work_id }))
}

fn now_rfc3339_from_time(t: time::OffsetDateTime) -> String {
    use time::format_description::well_known::Rfc3339;
    t.format(&Rfc3339).expect("rfc3339 format")
}
