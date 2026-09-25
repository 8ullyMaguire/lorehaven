//! M18 — External repository: tokens, bots, feeds, push, federation, AI.

use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::{Backend, Database};
use lorehaven_domain::api_scopes::Scope;

// ---------------------------------------------------------------------------
// Tokens (uses existing api_tokens table from migration 0001)
// ---------------------------------------------------------------------------

pub async fn issue_token(
    db: &Database,
    account: &str,
    _kind: &str,
    name: &str,
    token_hash: &str,
    scopes: &[Scope],
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let scopes_json =
        serde_json::to_string(&scopes.iter().map(|c| c.as_str()).collect::<Vec<_>>()).unwrap();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO api_tokens (id, account_id, name, token_hash, scopes, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(account)
            .bind(name)
            .bind(token_hash)
            .bind(&scopes_json)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO api_tokens (id, account_id, name, token_hash, scopes, created_at)
                 VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6)",
            )
            .bind(&id)
            .bind(account)
            .bind(name)
            .bind(token_hash)
            .bind(&scopes_json)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    };
    Ok(id)
}

async fn resolve_token_sqlite(
    pool: &sqlx::SqlitePool,
    token_hash: &str,
) -> Result<Option<(String, Vec<String>)>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT account_id, scopes FROM api_tokens WHERE token_hash = ? AND revoked_at IS NULL",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;
    match row {
        Some(r) => {
            let scopes_json: String = r.get("scopes");
            let scopes: Vec<String> = serde_json::from_str(&scopes_json).unwrap_or_default();
            Ok(Some((r.get::<String, _>("account_id"), scopes)))
        }
        None => Ok(None),
    }
}

async fn resolve_token_postgres(
    pool: &sqlx::postgres::PgPool,
    token_hash: &str,
) -> Result<Option<(String, Vec<String>)>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT account_id::text, scopes FROM api_tokens WHERE token_hash = $1 AND revoked_at IS NULL",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;
    match row {
        Some(r) => {
            let scopes_json: String = r.get("scopes");
            let scopes: Vec<String> = serde_json::from_str(&scopes_json).unwrap_or_default();
            Ok(Some((r.get::<String, _>("account_id"), scopes)))
        }
        None => Ok(None),
    }
}

pub async fn resolve_token(
    db: &Database,
    token_hash: &str,
) -> Result<Option<(String, Vec<String>)>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            resolve_token_sqlite(db.sqlite_pool().expect("sqlite"), token_hash).await
        }
        Backend::Postgres => {
            resolve_token_postgres(db.postgres_pool().expect("postgres"), token_hash).await
        }
    }
}

pub async fn revoke_token(db: &Database, token_id: &str) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("UPDATE api_tokens SET revoked_at = ? WHERE id = ?")
                .bind(&now)
                .bind(token_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query("UPDATE api_tokens SET revoked_at = $1 WHERE id = $2::uuid")
                .bind(&now)
                .bind(token_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    };
    Ok(())
}

// ---------------------------------------------------------------------------
// Token listing
// ---------------------------------------------------------------------------

pub async fn list_tokens(db: &Database, account: &str) -> Result<Vec<Value>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query("SELECT id, account_id, name, scopes, created_at, last_used_at, expires_at, revoked_at FROM api_tokens WHERE account_id = ? AND revoked_at IS NULL ORDER BY created_at DESC")
                .bind(account)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(rows
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.get::<String, _>("id"),
                        "account_id": r.get::<String, _>("account_id"),
                        "name": r.get::<String, _>("name"),
                        "scopes": r.get::<String, _>("scopes"),
                        "created_at": r.get::<String, _>("created_at"),
                        "last_used_at": r.get::<Option<String>, _>("last_used_at"),
                        "expires_at": r.get::<Option<String>, _>("expires_at"),
                        "revoked_at": r.get::<Option<String>, _>("revoked_at"),
                    })
                })
                .collect())
        }
        Backend::Postgres => {
            let rows = sqlx::query("SELECT id::text, account_id, name, scopes, created_at, last_used_at, expires_at, revoked_at FROM api_tokens WHERE account_id = $1::uuid AND revoked_at IS NULL ORDER BY created_at DESC")
                .bind(account)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(rows
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.get::<String, _>("id"),
                        "account_id": r.get::<String, _>("account_id"),
                        "name": r.get::<String, _>("name"),
                        "scopes": r.get::<String, _>("scopes"),
                        "created_at": r.get::<String, _>("created_at"),
                        "last_used_at": r.get::<Option<String>, _>("last_used_at"),
                        "expires_at": r.get::<Option<String>, _>("expires_at"),
                        "revoked_at": r.get::<Option<String>, _>("revoked_at"),
                    })
                })
                .collect())
        }
    }
}

// ---------------------------------------------------------------------------
// Bots
// ---------------------------------------------------------------------------

pub async fn register_bot(
    db: &Database,
    token_id: &str,
    owner: &str,
    contact: &str,
    user_agent: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO bot_registrations (id, token_id, owner, contact, user_agent, state, registered_at)
                 VALUES (?, ?, ?, ?, ?, 'active', ?)"
            )
            .bind(&id).bind(token_id).bind(owner).bind(contact).bind(user_agent).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO bot_registrations (id, token_id, owner, contact, user_agent, state, registered_at)
                 VALUES ($1, $2, $3, $4, $5, 'active', $6)"
            )
            .bind(&id).bind(token_id).bind(owner).bind(contact).bind(user_agent).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

// ---------------------------------------------------------------------------
// Feeds
// ---------------------------------------------------------------------------

pub async fn upsert_feed_handle(
    db: &Database,
    kind: &str,
    subject: &str,
    handle: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO feed_handles (id, kind, subject, handle, created_at)
                 VALUES (?, ?, ?, ?, ?)
                 ON CONFLICT(handle) DO UPDATE SET subject = excluded.subject",
            )
            .bind(&id)
            .bind(kind)
            .bind(subject)
            .bind(handle)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO feed_handles (id, kind, subject, handle, created_at)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT(handle) DO UPDATE SET subject = EXCLUDED.subject",
            )
            .bind(&id)
            .bind(kind)
            .bind(subject)
            .bind(handle)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(id)
}

// ---------------------------------------------------------------------------
// Push
// ---------------------------------------------------------------------------

pub async fn register_push_subscription(
    db: &Database,
    account: &str,
    endpoint: &str,
    keys: &str,
    device_name: Option<&str>,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO push_subscriptions (id, account, endpoint, keys, device_name, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)"
            )
            .bind(&id).bind(account).bind(endpoint).bind(keys).bind(device_name).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO push_subscriptions (id, account, endpoint, keys, device_name, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6)"
            )
            .bind(&id).bind(account).bind(endpoint).bind(keys).bind(device_name).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

// ---------------------------------------------------------------------------
// Federation
// ---------------------------------------------------------------------------

pub async fn record_inbound(
    db: &Database,
    peer_host: &str,
    object_type: &str,
    object_id: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO federation_inbound (id, peer_host, object_type, object_id, received_at, state)
                 VALUES (?, ?, ?, ?, ?, 'quarantined')"
            )
            .bind(&id).bind(peer_host).bind(object_type).bind(object_id).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO federation_inbound (id, peer_host, object_type, object_id, received_at, state)
                 VALUES ($1, $2, $3, $4, $5, 'quarantined')"
            )
            .bind(&id).bind(peer_host).bind(object_type).bind(object_id).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

// ---------------------------------------------------------------------------
// AI
// ---------------------------------------------------------------------------

pub async fn record_ai_request(
    db: &Database,
    work_id: &str,
    provider: &str,
    purpose: &str,
    charged_transaction: Option<&str>,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO ai_requests (id, work_id, provider, account, purpose, charged_transaction, requested_at)
                 VALUES (?, ?, ?, NULL, ?, ?, ?)"
            )
            .bind(&id).bind(work_id).bind(provider).bind(purpose).bind(charged_transaction).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO ai_requests (id, work_id, provider, account, purpose, charged_transaction, requested_at)
                 VALUES ($1, $2, $3, NULL, $4, $5, $6)"
            )
            .bind(&id).bind(work_id).bind(provider).bind(purpose).bind(charged_transaction).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}
