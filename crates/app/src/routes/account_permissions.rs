//! M27 — Account permission statement endpoints.
//!
//! ```text
//! GET  /me/permissions    current account's permission statement
//! PUT  /me/permissions    upsert the account's statement
//! ```

use axum::extract::State;
use axum::routing::{get, put};
use axum::{Json, Router};
use lorehaven_db::permission;
use lorehaven_domain::permission::PermissionStatement;
use serde::Deserialize;

use crate::auth::RequireSession;
use crate::http::ApiResult;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me/permissions", get(get_account_permissions))
        .route("/me/permissions", put(put_account_permissions))
}

#[derive(Debug, Deserialize)]
pub struct UpdateAccountPermissions {
    pub podfic: Option<String>,
    pub translation: Option<String>,
    pub remix: Option<String>,
    pub continuation: Option<String>,
    pub redistribution: Option<String>,
    pub ai_training: Option<String>,
}

async fn get_account_permissions(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<PermissionStatement>> {
    let db = state.db();
    let stmt = permission::get_account_permission_statement(db, &user.account_id.to_string())
        .await
        .map_err(|e| crate::http::ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(stmt))
}

async fn put_account_permissions(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<UpdateAccountPermissions>,
) -> ApiResult<Json<PermissionStatement>> {
    let db = state.db();
    let current = permission::get_account_permission_statement(db, &user.account_id.to_string())
        .await
        .map_err(|e| crate::http::ApiError(lorehaven_domain::AppError::Internal(e)))?;
    let mut next = current;
    if let Some(v) = body.podfic {
        next.podfic = parse_perm(&v)?;
    }
    if let Some(v) = body.translation {
        next.translation = parse_perm(&v)?;
    }
    if let Some(v) = body.remix {
        next.remix = parse_perm(&v)?;
    }
    if let Some(v) = body.continuation {
        next.continuation = parse_perm(&v)?;
    }
    if let Some(v) = body.redistribution {
        next.redistribution = parse_perm(&v)?;
    }
    if let Some(v) = body.ai_training {
        next.ai_training = parse_perm(&v)?;
    }
    permission::set_account_permission_statement(db, &user.account_id.to_string(), &next)
        .await
        .map_err(|e| crate::http::ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(next))
}

fn parse_perm(raw: &str) -> Result<lorehaven_domain::permission::Permission, crate::http::ApiError> {
    lorehaven_domain::permission::Permission::parse(raw).ok_or_else(|| {
        crate::http::ApiError(lorehaven_domain::AppError::Validation {
            message: format!("unknown permission value: {raw}"),
            field_errors: std::collections::BTreeMap::new(),
        })
    })
}
