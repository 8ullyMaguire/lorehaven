//! M16 — Marketplace repository: listings, commissions, extensions, webhooks, gallery.

use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::{Backend, Database};
use lorehaven_domain::extension::Capability;
use lorehaven_domain::marketplace::{self, CommissionState, ListingKind};

// ---------------------------------------------------------------------------
// Listings
// ---------------------------------------------------------------------------

pub async fn create_listing(
    db: &Database,
    kind: ListingKind,
    owner: &str,
    work_id: Option<&str>,
    terms: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO listings (id, kind, owner, work_id, terms, state, created_at)
                 VALUES (?, ?, ?, ?, ?, 'draft', ?)",
            )
            .bind(&id)
            .bind(kind.as_str())
            .bind(owner)
            .bind(work_id)
            .bind(terms)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO listings (id, kind, owner, work_id, terms, state, created_at)
                 VALUES ($1, $2, $3, $4, $5, 'draft', $6)",
            )
            .bind(&id)
            .bind(kind.as_str())
            .bind(owner)
            .bind(work_id)
            .bind(terms)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(id)
}

pub async fn list_listings(
    db: &Database,
    kind: Option<&str>,
    state: Option<&str>,
    limit: i64,
) -> Result<Vec<Value>, sqlx::Error> {
    let mut sqlite_sql =
        "SELECT id, kind, owner, work_id, terms, state, created_at FROM listings WHERE 1=1"
            .to_string();
    let mut pg_sql = "SELECT id::text, kind, owner, work_id::text, terms, state, created_at FROM listings WHERE 1=1".to_string();
    let mut param_idx = 1;
    if kind.is_some() {
        sqlite_sql.push_str(" AND kind = ?");
        pg_sql.push_str(&format!(" AND kind = ${}", param_idx));
        param_idx += 1;
    }
    if state.is_some() {
        sqlite_sql.push_str(" AND state = ?");
        pg_sql.push_str(&format!(" AND state = ${}", param_idx));
        param_idx += 1;
    }
    sqlite_sql.push_str(" ORDER BY created_at LIMIT ?");
    pg_sql.push_str(&format!(" ORDER BY created_at LIMIT ${}", param_idx));

    match db.backend() {
        Backend::Sqlite => {
            let mut q = sqlx::query(&sqlite_sql);
            if let Some(k) = kind {
                q = q.bind(k);
            }
            if let Some(s) = state {
                q = q.bind(s);
            }
            let rows = q
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(rows
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.get::<String, _>("id"),
                        "kind": r.get::<String, _>("kind"),
                        "owner": r.get::<String, _>("owner"),
                        "work_id": r.get::<Option<String>, _>("work_id"),
                        "terms": r.get::<String, _>("terms"),
                        "state": r.get::<String, _>("state"),
                        "created_at": r.get::<String, _>("created_at"),
                    })
                })
                .collect())
        }
        Backend::Postgres => {
            let mut q = sqlx::query(&pg_sql);
            if let Some(k) = kind {
                q = q.bind(k);
            }
            if let Some(s) = state {
                q = q.bind(s);
            }
            let rows = q
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(rows
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.get::<String, _>("id"),
                        "kind": r.get::<String, _>("kind"),
                        "owner": r.get::<String, _>("owner"),
                        "work_id": r.get::<Option<String>, _>("work_id"),
                        "terms": r.get::<String, _>("terms"),
                        "state": r.get::<String, _>("state"),
                        "created_at": r.get::<String, _>("created_at"),
                    })
                })
                .collect())
        }
    }
}

// ---------------------------------------------------------------------------
// Commissions
// ---------------------------------------------------------------------------

pub async fn create_commission(
    db: &Database,
    listing_id: &str,
    client: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO commissions (id, listing_id, client, state, created_at, updated_at)
                 VALUES (?, ?, ?, 'quoted', ?, ?)",
            )
            .bind(&id)
            .bind(listing_id)
            .bind(client)
            .bind(&now)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO commissions (id, listing_id, client, state, created_at, updated_at)
                 VALUES ($1, $2, $3, 'quoted', $4, $5)",
            )
            .bind(&id)
            .bind(listing_id)
            .bind(client)
            .bind(&now)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(id)
}

pub async fn transition_commission(
    db: &Database,
    commission_id: &str,
    from: &CommissionState,
    to: &CommissionState,
    ledger_ref: Option<&str>,
) -> Result<(), sqlx::Error> {
    if !marketplace::valid_commission_transition(from, to) {
        return Err(sqlx::Error::Protocol(format!(
            "invalid transition: {:?} -> {:?}",
            from, to
        )));
    }

    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            if let Some(ref ledger) = ledger_ref {
                sqlx::query("UPDATE commissions SET state = ?, updated_at = ?, quote_transaction = ? WHERE id = ? AND state = ?")
                    .bind(to.as_str()).bind(&now).bind(ledger).bind(commission_id).bind(from.as_str())
                    .execute(db.sqlite_pool().expect("sqlite")).await?;
            } else {
                sqlx::query(
                    "UPDATE commissions SET state = ?, updated_at = ? WHERE id = ? AND state = ?",
                )
                .bind(to.as_str())
                .bind(&now)
                .bind(commission_id)
                .bind(from.as_str())
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
            }
        }
        Backend::Postgres => {
            if let Some(ref ledger) = ledger_ref {
                sqlx::query("UPDATE commissions SET state = $1, updated_at = $2, quote_transaction = $3 WHERE id = $4 AND state = $5")
                    .bind(to.as_str()).bind(&now).bind(ledger).bind(commission_id).bind(from.as_str())
                    .execute(db.postgres_pool().expect("postgres")).await?;
            } else {
                sqlx::query("UPDATE commissions SET state = $1, updated_at = $2 WHERE id = $3 AND state = $4")
                    .bind(to.as_str()).bind(&now).bind(commission_id).bind(from.as_str())
                    .execute(db.postgres_pool().expect("postgres")).await?;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Extensions
// ---------------------------------------------------------------------------

pub async fn submit_extension(
    db: &Database,
    slug: &str,
    version: &str,
    document: &str,
    submitted_by: &str,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO extension_manifests (id, version, document, submitted_by, state, created_at)
                 VALUES (?, ?, ?, ?, 'pending', ?)"
            )
            .bind(slug).bind(version).bind(document).bind(submitted_by).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO extension_manifests (id, version, document, submitted_by, state, created_at)
                 VALUES ($1, $2, $3, $4, 'pending', $5)"
            )
            .bind(slug).bind(version).bind(document).bind(submitted_by).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(())
}

pub async fn grant_extension(
    db: &Database,
    account: &str,
    manifest_id: &str,
    version: &str,
    capabilities: &[Capability],
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    let caps_json =
        serde_json::to_string(&capabilities.iter().map(|c| c.as_str()).collect::<Vec<_>>())
            .unwrap();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO extension_grants (account, manifest_id, version, capabilities, granted_at)
                 VALUES (?, ?, ?, ?, ?)"
            )
            .bind(account).bind(manifest_id).bind(version).bind(&caps_json).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO extension_grants (account, manifest_id, version, capabilities, granted_at)
                 VALUES ($1, $2, $3, $4, $5)"
            )
            .bind(account).bind(manifest_id).bind(version).bind(&caps_json).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(())
}

pub async fn revoke_extension(
    db: &Database,
    account: &str,
    manifest_id: &str,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "UPDATE extension_grants SET revoked_at = ? WHERE account = ? AND manifest_id = ?",
            )
            .bind(&now)
            .bind(account)
            .bind(manifest_id)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query("UPDATE extension_grants SET revoked_at = $1 WHERE account = $2 AND manifest_id = $3")
                .bind(&now).bind(account).bind(manifest_id)
                .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Webhooks
// ---------------------------------------------------------------------------

pub async fn create_webhook(
    db: &Database,
    owner: &str,
    url: &str,
    secret: &str,
    events: &[String],
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let events_json = serde_json::to_string(events).unwrap();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO webhook_endpoints (id, owner, url, secret, events, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(owner)
            .bind(url)
            .bind(secret)
            .bind(&events_json)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO webhook_endpoints (id, owner, url, secret, events, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(&id)
            .bind(owner)
            .bind(url)
            .bind(secret)
            .bind(&events_json)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(id)
}

pub async fn record_delivery(
    db: &Database,
    endpoint_id: &str,
    event_id: &str,
    payload: &str,
    signature: &str,
    status: &str,
) -> Result<(), sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO webhook_deliveries (id, endpoint_id, event_id, payload, signature, attempted_at, status, attempts)
                 VALUES (?, ?, ?, ?, ?, ?, ?, 1)"
            )
            .bind(&id).bind(endpoint_id).bind(event_id).bind(payload).bind(signature).bind(&now).bind(status)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO webhook_deliveries (id, endpoint_id, event_id, payload, signature, attempted_at, status, attempts)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, 1)"
            )
            .bind(&id).bind(endpoint_id).bind(event_id).bind(payload).bind(signature).bind(&now).bind(status)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Gallery
// ---------------------------------------------------------------------------

pub async fn add_gallery_item(
    db: &Database,
    work_id: &str,
    owner: &str,
    media_type: &str,
    storage_key: &str,
    alt_text: &str,
    sanitized_document: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO gallery_items (id, work_id, owner, media_type, storage_key, alt_text, sanitized_document, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
            )
            .bind(&id).bind(work_id).bind(owner).bind(media_type).bind(storage_key).bind(alt_text).bind(sanitized_document).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO gallery_items (id, work_id, owner, media_type, storage_key, alt_text, sanitized_document, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
            )
            .bind(&id).bind(work_id).bind(owner).bind(media_type).bind(storage_key).bind(alt_text).bind(sanitized_document).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

pub async fn list_gallery_items(db: &Database, work_id: &str) -> Result<Vec<Value>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query("SELECT id, work_id, owner, media_type, storage_key, alt_text, sanitized_document, created_at FROM gallery_items WHERE work_id = ?")
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(rows
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.get::<String, _>("id"),
                        "work_id": r.get::<String, _>("work_id"),
                        "owner": r.get::<String, _>("owner"),
                        "media_type": r.get::<String, _>("media_type"),
                        "storage_key": r.get::<String, _>("storage_key"),
                        "alt_text": r.get::<String, _>("alt_text"),
                        "sanitized_document": r.get::<String, _>("sanitized_document"),
                        "created_at": r.get::<String, _>("created_at"),
                    })
                })
                .collect())
        }
        Backend::Postgres => {
            let rows = sqlx::query("SELECT id, work_id, owner, media_type, storage_key, alt_text, sanitized_document, created_at FROM gallery_items WHERE work_id = $1")
                .bind(work_id)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(rows
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.get::<String, _>("id"),
                        "work_id": r.get::<String, _>("work_id"),
                        "owner": r.get::<String, _>("owner"),
                        "media_type": r.get::<String, _>("media_type"),
                        "storage_key": r.get::<String, _>("storage_key"),
                        "alt_text": r.get::<String, _>("alt_text"),
                        "sanitized_document": r.get::<String, _>("sanitized_document"),
                        "created_at": r.get::<String, _>("created_at"),
                    })
                })
                .collect())
        }
    }
}
