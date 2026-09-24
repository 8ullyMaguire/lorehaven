// M47: User Configuration — DB layer for search_settings, content_filters,
// notification_routes (spec §46.3).

use anyhow::Result;
use serde_json::Value;
use uuid::Uuid;

use crate::{Backend, Database};

// ---------------------------------------------------------------------------
// Search settings (per pseud)
// ---------------------------------------------------------------------------

/// Upsert a single search setting for a pseud.
pub async fn upsert_search_setting(
    db: &Database,
    pseud_id: Uuid,
    key: &str,
    value: &Value,
    now: &str,
) -> Result<()> {
    let sql = db.sql(
        "INSERT INTO search_settings (id, pseud_id, key, value, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT (pseud_id, key)
         DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        "INSERT INTO search_settings (id, pseud_id, key, value, created_at, updated_at)
         VALUES ($1::uuid, $2::uuid, $3, $4::jsonb, $5::timestamptz, $6::timestamptz)
         ON CONFLICT (pseud_id, key)
         DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    );
    let id = Uuid::new_v4().to_string();
    let value_json = serde_json::to_string(value).unwrap_or_default();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(pseud_id.to_string())
                .bind(key)
                .bind(&value_json)
                .bind(now)
                .bind(now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(pseud_id.to_string())
                .bind(key)
                .bind(&value_json)
                .bind(now)
                .bind(now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }

    Ok(())
}

/// Read all search settings for a pseud.
pub async fn read_search_settings(db: &Database, pseud_id: Uuid) -> Result<Vec<(String, Value)>> {
    let sql = "SELECT key, value FROM search_settings WHERE pseud_id = ?";
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String, String)>(sql)
                .bind(pseud_id.to_string())
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String, String)>(sql)
                .bind(pseud_id.to_string())
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(rows
        .into_iter()
        .filter_map(|(k, v)| serde_json::from_str::<Value>(&v).ok().map(|val| (k, val)))
        .collect())
}

/// Delete a single search setting (drop override, inherit next level).
pub async fn delete_search_setting(db: &Database, pseud_id: Uuid, key: &str) -> Result<bool> {
    let affected = match db.backend() {
        Backend::Sqlite => {
            sqlx::query("DELETE FROM search_settings WHERE pseud_id = ? AND key = ?")
                .bind(pseud_id.to_string())
                .bind(key)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?
                .rows_affected()
        }
        Backend::Postgres => {
            sqlx::query("DELETE FROM search_settings WHERE pseud_id = ? AND key = ?")
                .bind(pseud_id.to_string())
                .bind(key)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?
                .rows_affected()
        }
    };

    Ok(affected > 0)
}

// ---------------------------------------------------------------------------
// Content filters (per pseud)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ContentFilterRow {
    pub filter_type: String,
    pub value: String,
}

/// Add a content filter for a pseud (idempotent — same type+value is a no-op).
pub async fn add_content_filter(
    db: &Database,
    pseud_id: Uuid,
    filter_type: &str,
    value: &str,
    now: &str,
) -> Result<()> {
    let sql = db.sql(
        "INSERT OR IGNORE INTO content_filters (id, pseud_id, filter_type, value, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)",
        "INSERT INTO content_filters (id, pseud_id, filter_type, value, created_at, updated_at)
         VALUES ($1::uuid, $2::uuid, $3, $4, $5::timestamptz, $6::timestamptz)
         ON CONFLICT DO NOTHING",
    );
    let id = Uuid::new_v4().to_string();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(pseud_id.to_string())
                .bind(filter_type)
                .bind(value)
                .bind(now)
                .bind(now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(pseud_id.to_string())
                .bind(filter_type)
                .bind(value)
                .bind(now)
                .bind(now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }

    Ok(())
}

/// Remove a content filter.
pub async fn remove_content_filter(
    db: &Database,
    pseud_id: Uuid,
    filter_type: &str,
    value: &str,
) -> Result<bool> {
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(
            "DELETE FROM content_filters WHERE pseud_id = ? AND filter_type = ? AND value = ?",
        )
        .bind(pseud_id.to_string())
        .bind(filter_type)
        .bind(value)
        .execute(db.sqlite_pool().expect("sqlite handle"))
        .await?
        .rows_affected(),
        Backend::Postgres => sqlx::query(
            "DELETE FROM content_filters WHERE pseud_id = ? AND filter_type = ? AND value = ?",
        )
        .bind(pseud_id.to_string())
        .bind(filter_type)
        .bind(value)
        .execute(db.postgres_pool().expect("postgres handle"))
        .await?
        .rows_affected(),
    };

    Ok(affected > 0)
}

/// List all content filters for a pseud.
pub async fn list_content_filters(db: &Database, pseud_id: Uuid) -> Result<Vec<ContentFilterRow>> {
    let sql = "SELECT filter_type, value FROM content_filters WHERE pseud_id = ? ORDER BY filter_type, value";
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String, String)>(sql)
                .bind(pseud_id.to_string())
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String, String)>(sql)
                .bind(pseud_id.to_string())
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(rows
        .into_iter()
        .map(|(filter_type, value)| ContentFilterRow { filter_type, value })
        .collect())
}

// ---------------------------------------------------------------------------
// Notification routes (per account)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct NotificationRouteRow {
    pub event_type: String,
    pub channel: String,
    pub enabled: bool,
}

/// Upsert a notification route for an account.
pub async fn upsert_notification_route(
    db: &Database,
    account_id: Uuid,
    event_type: &str,
    channel: &str,
    enabled: bool,
    now: &str,
) -> Result<()> {
    let sql = db.sql(
        "INSERT INTO notification_routes (id, account_id, event_type, channel, enabled, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (account_id, event_type)
         DO UPDATE SET channel = excluded.channel, enabled = excluded.enabled, updated_at = excluded.updated_at",
        "INSERT INTO notification_routes (id, account_id, event_type, channel, enabled, created_at, updated_at)
         VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6::timestamptz, $7::timestamptz)
         ON CONFLICT (account_id, event_type)
         DO UPDATE SET channel = excluded.channel, enabled = excluded.enabled, updated_at = excluded.updated_at",
    );
    let id = Uuid::new_v4().to_string();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(account_id.to_string())
                .bind(event_type)
                .bind(channel)
                .bind(enabled)
                .bind(now)
                .bind(now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(account_id.to_string())
                .bind(event_type)
                .bind(channel)
                .bind(enabled)
                .bind(now)
                .bind(now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }

    Ok(())
}

/// Resolve the effective channel for an event type, respecting per-event
/// routing (spec §46.4). Returns None if the event is disabled or no route
/// exists (default: in-app).
pub async fn resolve_notification_channel(
    db: &Database,
    account_id: Uuid,
    event_type: &str,
) -> Result<Option<String>> {
    let sql = "SELECT channel, enabled FROM notification_routes WHERE account_id = ? AND event_type = ?";
    let row: Option<(String, bool)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String, bool)>(sql)
                .bind(account_id.to_string())
                .bind(event_type)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String, bool)>(
                "SELECT channel, enabled FROM notification_routes WHERE account_id::text = ? AND event_type = ?",
            )
            .bind(account_id.to_string())
            .bind(event_type)
            .fetch_optional(db.postgres_pool().expect("postgres handle"))
            .await?
        }
    };
    Ok(row.and_then(|(channel, enabled)| if enabled { Some(channel) } else { None }))
}

/// Read all notification routes for an account.
pub async fn read_notification_routes(
    db: &Database,
    account_id: Uuid,
) -> Result<Vec<NotificationRouteRow>> {
    let sql = "SELECT event_type, channel, enabled FROM notification_routes WHERE account_id = ? ORDER BY event_type";
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String, String, bool)>(sql)
                .bind(account_id.to_string())
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String, String, bool)>(sql)
                .bind(account_id.to_string())
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(rows
        .into_iter()
        .map(|(event_type, channel, enabled)| NotificationRouteRow {
            event_type,
            channel,
            enabled,
        })
        .collect())
}

/// Delete a notification route.
pub async fn delete_notification_route(
    db: &Database,
    account_id: Uuid,
    event_type: &str,
) -> Result<bool> {
    let affected = match db.backend() {
        Backend::Sqlite => {
            sqlx::query("DELETE FROM notification_routes WHERE account_id = ? AND event_type = ?")
                .bind(account_id.to_string())
                .bind(event_type)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?
                .rows_affected()
        }
        Backend::Postgres => {
            sqlx::query("DELETE FROM notification_routes WHERE account_id = ? AND event_type = ?")
                .bind(account_id.to_string())
                .bind(event_type)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?
                .rows_affected()
        }
    };

    Ok(affected > 0)
}
