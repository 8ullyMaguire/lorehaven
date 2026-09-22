//! M21 — Monetization repository (spec §20.9).
//!
//! Work pricing, entitlements, author earnings ledger, payouts,
//! monetization assertions, and work gifts.

use sqlx::Row;
use uuid::Uuid;

use crate::{Backend, Database};

// ---------------------------------------------------------------------------
// Common types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PricingRow {
    pub id: String,
    pub model: String,
    pub price_minor: i64,
    pub currency: String,
    pub public_at_offset: Option<i64>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
}

#[derive(Debug, Clone)]
pub struct EntitlementRow {
    pub id: String,
    pub work_id: String,
    pub kind: String,
    pub source_payment_id: Option<String>,
    pub granted_at: String,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EarningsRow {
    pub id: String,
    pub amount_minor: i64,
    pub currency: String,
    pub kind: String,
    pub payment_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct AssertionRow {
    pub id: String,
    pub assertion_kind: String,
    pub policy_version: String,
    pub accepted_at: String,
    pub revoked_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GiftRow {
    pub id: String,
    pub work_id: String,
    pub gift_note: Option<String>,
    pub challenge_fulfillment_id: Option<String>,
    pub created_at: String,
    pub declined_at: Option<String>,
}

// ---------------------------------------------------------------------------
// Internal helpers: each dialect returns a common type
// ---------------------------------------------------------------------------

async fn fetch_pricing_sqlite(
    pool: &sqlx::SqlitePool,
    work_id: &str,
) -> Result<Vec<PricingRow>, sqlx::Error> {
    let rows = sqlx::query("SELECT id, model, price_minor, currency, public_at_offset, enabled, created_at, updated_at, version FROM work_pricing WHERE work_id = ?")
        .bind(work_id)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| PricingRow {
            id: r.get::<String, _>("id"),
            model: r.get::<String, _>("model"),
            price_minor: r.get::<i64, _>("price_minor"),
            currency: r.get::<String, _>("currency"),
            public_at_offset: r.get::<Option<i64>, _>("public_at_offset"),
            enabled: r.get::<i64, _>("enabled") != 0,
            created_at: r.get::<String, _>("created_at"),
            updated_at: r.get::<String, _>("updated_at"),
            version: r.get::<i64, _>("version"),
        })
        .collect())
}

async fn fetch_pricing_postgres(
    pool: &sqlx::postgres::PgPool,
    work_id: &str,
) -> Result<Vec<PricingRow>, sqlx::Error> {
    let rows = sqlx::query("SELECT id::text, model, price_minor, currency, public_at_offset, enabled, created_at, updated_at, version FROM work_pricing WHERE work_id = $1::uuid")
        .bind(work_id)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| PricingRow {
            id: r.get::<String, _>("id"),
            model: r.get::<String, _>("model"),
            price_minor: r.get::<i64, _>("price_minor"),
            currency: r.get::<String, _>("currency"),
            public_at_offset: r.get::<Option<i64>, _>("public_at_offset"),
            enabled: r.get::<bool, _>("enabled"),
            created_at: r.get::<String, _>("created_at"),
            updated_at: r.get::<String, _>("updated_at"),
            version: r.get::<i64, _>("version"),
        })
        .collect())
}

async fn fetch_entitlements_sqlite(
    pool: &sqlx::SqlitePool,
    account_id: &str,
) -> Result<Vec<EntitlementRow>, sqlx::Error> {
    let rows = sqlx::query("SELECT id, work_id, kind, source_payment_id, granted_at, expires_at FROM work_entitlements WHERE account_id = ?")
        .bind(account_id)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| EntitlementRow {
            id: r.get::<String, _>("id"),
            work_id: r.get::<String, _>("work_id"),
            kind: r.get::<String, _>("kind"),
            source_payment_id: r.get::<Option<String>, _>("source_payment_id"),
            granted_at: r.get::<String, _>("granted_at"),
            expires_at: r.get::<Option<String>, _>("expires_at"),
        })
        .collect())
}

async fn fetch_entitlements_postgres(
    pool: &sqlx::postgres::PgPool,
    account_id: &str,
) -> Result<Vec<EntitlementRow>, sqlx::Error> {
    let rows = sqlx::query("SELECT id::text, work_id::text, kind, source_payment_id, granted_at, expires_at FROM work_entitlements WHERE account_id = $1::uuid")
        .bind(account_id)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| EntitlementRow {
            id: r.get::<String, _>("id"),
            work_id: r.get::<String, _>("work_id"),
            kind: r.get::<String, _>("kind"),
            source_payment_id: r.get::<Option<String>, _>("source_payment_id"),
            granted_at: r.get::<String, _>("granted_at"),
            expires_at: r.get::<Option<String>, _>("expires_at"),
        })
        .collect())
}

async fn fetch_earnings_sqlite(
    pool: &sqlx::SqlitePool,
    account_id: &str,
) -> Result<Vec<EarningsRow>, sqlx::Error> {
    let rows = sqlx::query("SELECT id, amount_minor, currency, kind, payment_id, idempotency_key, created_at FROM author_earnings_ledger WHERE author_account_id = ? ORDER BY created_at DESC")
        .bind(account_id)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| EarningsRow {
            id: r.get::<String, _>("id"),
            amount_minor: r.get::<i64, _>("amount_minor"),
            currency: r.get::<String, _>("currency"),
            kind: r.get::<String, _>("kind"),
            payment_id: r.get::<Option<String>, _>("payment_id"),
            idempotency_key: r.get::<Option<String>, _>("idempotency_key"),
            created_at: r.get::<String, _>("created_at"),
        })
        .collect())
}

async fn fetch_earnings_postgres(
    pool: &sqlx::postgres::PgPool,
    account_id: &str,
) -> Result<Vec<EarningsRow>, sqlx::Error> {
    let rows = sqlx::query("SELECT id::text, amount_minor, currency, kind, payment_id, idempotency_key, created_at FROM author_earnings_ledger WHERE author_account_id = $1::uuid ORDER BY created_at DESC")
        .bind(account_id)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| EarningsRow {
            id: r.get::<String, _>("id"),
            amount_minor: r.get::<i64, _>("amount_minor"),
            currency: r.get::<String, _>("currency"),
            kind: r.get::<String, _>("kind"),
            payment_id: r.get::<Option<String>, _>("payment_id"),
            idempotency_key: r.get::<Option<String>, _>("idempotency_key"),
            created_at: r.get::<String, _>("created_at"),
        })
        .collect())
}

async fn fetch_assertion_sqlite(
    pool: &sqlx::SqlitePool,
    work_id: &str,
    kind: &str,
) -> Result<Option<AssertionRow>, sqlx::Error> {
    let opt = sqlx::query("SELECT id, assertion_kind, policy_version, accepted_at, revoked_at FROM monetization_assertions WHERE work_id = ? AND assertion_kind = ? ORDER BY accepted_at DESC LIMIT 1")
        .bind(work_id).bind(kind)
        .fetch_optional(pool)
        .await?;
    Ok(opt.map(|r| AssertionRow {
        id: r.get::<String, _>("id"),
        assertion_kind: r.get::<String, _>("assertion_kind"),
        policy_version: r.get::<String, _>("policy_version"),
        accepted_at: r.get::<String, _>("accepted_at"),
        revoked_at: r.get::<Option<String>, _>("revoked_at"),
    }))
}

async fn fetch_assertion_postgres(
    pool: &sqlx::postgres::PgPool,
    work_id: &str,
    kind: &str,
) -> Result<Option<AssertionRow>, sqlx::Error> {
    let opt = sqlx::query("SELECT id::text, assertion_kind, policy_version, accepted_at, revoked_at FROM monetization_assertions WHERE work_id = $1::uuid AND assertion_kind = $2 ORDER BY accepted_at DESC LIMIT 1")
        .bind(work_id).bind(kind)
        .fetch_optional(pool)
        .await?;
    Ok(opt.map(|r| AssertionRow {
        id: r.get::<String, _>("id"),
        assertion_kind: r.get::<String, _>("assertion_kind"),
        policy_version: r.get::<String, _>("policy_version"),
        accepted_at: r.get::<String, _>("accepted_at"),
        revoked_at: r.get::<Option<String>, _>("revoked_at"),
    }))
}

async fn fetch_gifts_sqlite(
    pool: &sqlx::SqlitePool,
    recipient_pseud_id: &str,
) -> Result<Vec<GiftRow>, sqlx::Error> {
    let rows = sqlx::query("SELECT id, work_id, gift_note, challenge_fulfillment_id, created_at, declined_at FROM work_gifts WHERE recipient_pseud_id = ? ORDER BY created_at DESC")
        .bind(recipient_pseud_id)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| GiftRow {
            id: r.get::<String, _>("id"),
            work_id: r.get::<String, _>("work_id"),
            gift_note: r.get::<Option<String>, _>("gift_note"),
            challenge_fulfillment_id: r.get::<Option<String>, _>("challenge_fulfillment_id"),
            created_at: r.get::<String, _>("created_at"),
            declined_at: r.get::<Option<String>, _>("declined_at"),
        })
        .collect())
}

async fn fetch_gifts_postgres(
    pool: &sqlx::postgres::PgPool,
    recipient_pseud_id: &str,
) -> Result<Vec<GiftRow>, sqlx::Error> {
    let rows = sqlx::query("SELECT id::text, work_id::text, gift_note, challenge_fulfillment_id, created_at, declined_at FROM work_gifts WHERE recipient_pseud_id = $1::uuid ORDER BY created_at DESC")
        .bind(recipient_pseud_id)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| GiftRow {
            id: r.get::<String, _>("id"),
            work_id: r.get::<String, _>("work_id"),
            gift_note: r.get::<Option<String>, _>("gift_note"),
            challenge_fulfillment_id: r.get::<Option<String>, _>("challenge_fulfillment_id"),
            created_at: r.get::<String, _>("created_at"),
            declined_at: r.get::<Option<String>, _>("declined_at"),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Set work pricing (upsert). Returns the pricing row id.
pub async fn set_pricing(
    db: &Database,
    work_id: &str,
    model: &str,
    price_minor: i64,
    currency: &str,
    public_at_offset: Option<i64>,
) -> Result<String, sqlx::Error> {
    let now = crate::identity::now_rfc3339();

    // Check if the row already exists so we can return its ID on upsert.
    let existing_id: Option<String> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar("SELECT id FROM work_pricing WHERE work_id = ?")
                .bind(work_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar("SELECT id::text FROM work_pricing WHERE work_id = $1::uuid")
                .bind(work_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    if let Some(id) = existing_id {
        // Row exists — update in place and return the existing ID.
        match db.backend() {
            Backend::Sqlite => {
                sqlx::query(
                    "UPDATE work_pricing SET model = ?, price_minor = ?, currency = ?, public_at_offset = ?, updated_at = ?, version = version + 1 WHERE work_id = ?"
                )
                .bind(model).bind(price_minor).bind(currency)
                .bind(public_at_offset).bind(&now).bind(work_id)
                .execute(db.sqlite_pool().expect("sqlite")).await?;
            }
            Backend::Postgres => {
                sqlx::query(
                    "UPDATE work_pricing SET model = $1, price_minor = $2, currency = $3, public_at_offset = $4, updated_at = $5, version = version + 1 WHERE work_id = $6::uuid"
                )
                .bind(model).bind(price_minor).bind(currency)
                .bind(public_at_offset).bind(&now).bind(work_id)
                .execute(db.postgres_pool().expect("postgres")).await?;
            }
        }
        return Ok(id);
    }

    let id = Uuid::new_v4().to_string();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO work_pricing (id, work_id, model, price_minor, currency, public_at_offset, enabled, created_at, updated_at, version)
                 VALUES (?, ?, ?, ?, ?, ?, 1, ?, ?, 1)"
            )
            .bind(&id).bind(work_id).bind(model).bind(price_minor).bind(currency)
            .bind(public_at_offset).bind(&now).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO work_pricing (id, work_id, model, price_minor, currency, public_at_offset, enabled, created_at, updated_at, version) VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6, TRUE, $7, $8, 1)"
            )
            .bind(&id).bind(work_id).bind(model).bind(price_minor).bind(currency)
            .bind(public_at_offset).bind(&now).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

/// Get active pricing for a work.
pub async fn get_pricing(db: &Database, work_id: &str) -> Result<Option<PricingRow>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => fetch_pricing_sqlite(db.sqlite_pool().expect("sqlite"), work_id).await,
        Backend::Postgres => {
            fetch_pricing_postgres(db.postgres_pool().expect("postgres"), work_id).await
        }
    }
    .map(|r| r.into_iter().next())
}

/// Disable pricing for a work.
pub async fn disable_pricing(db: &Database, work_id: &str) -> Result<u64, sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let r = sqlx::query(
                "UPDATE work_pricing SET enabled = 0, updated_at = ? WHERE work_id = ?",
            )
            .bind(&now)
            .bind(work_id)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
            Ok(r.rows_affected())
        }
        Backend::Postgres => {
            let r = sqlx::query(
                "UPDATE work_pricing SET enabled = FALSE, updated_at = $1 WHERE work_id = $2::uuid",
            )
            .bind(&now)
            .bind(work_id)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
            Ok(r.rows_affected())
        }
    }
}

/// Grant an entitlement to an account.
pub async fn grant_entitlement(
    db: &Database,
    account_id: &str,
    work_id: &str,
    kind: &str,
    source_payment_id: Option<&str>,
    expires_at: Option<&str>,
) -> Result<String, sqlx::Error> {
    let now = crate::identity::now_rfc3339();

    // Check for existing row so we return the real ID on upsert.
    let existing_id: Option<String> = match db.backend() {
        Backend::Sqlite => sqlx::query_scalar(
            "SELECT id FROM work_entitlements WHERE account_id = ? AND work_id = ? AND kind = ?",
        )
        .bind(account_id)
        .bind(work_id)
        .bind(kind)
        .fetch_optional(db.sqlite_pool().expect("sqlite"))
        .await?,
        Backend::Postgres => sqlx::query_scalar(
            "SELECT id::text FROM work_entitlements WHERE account_id = $1::uuid AND work_id = $2::uuid AND kind = $3",
        )
        .bind(account_id)
        .bind(work_id)
        .bind(kind)
        .fetch_optional(db.postgres_pool().expect("postgres"))
        .await?,
    };

    if let Some(id) = existing_id {
        match db.backend() {
            Backend::Sqlite => {
                sqlx::query("UPDATE work_entitlements SET source_payment_id = ?, expires_at = ?, granted_at = ? WHERE id = ?")
                    .bind(source_payment_id).bind(expires_at).bind(&now).bind(&id)
                    .execute(db.sqlite_pool().expect("sqlite")).await?;
            }
            Backend::Postgres => {
                sqlx::query("UPDATE work_entitlements SET source_payment_id = $1, expires_at = $2, granted_at = $3 WHERE id = $4")
                    .bind(source_payment_id).bind(expires_at).bind(&now).bind(&id)
                    .execute(db.postgres_pool().expect("postgres")).await?;
            }
        }
        return Ok(id);
    }

    let id = Uuid::new_v4().to_string();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("INSERT INTO work_entitlements (id, account_id, work_id, kind, source_payment_id, granted_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
            .bind(&id).bind(account_id).bind(work_id).bind(kind).bind(source_payment_id).bind(&now).bind(expires_at)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query("INSERT INTO work_entitlements (id, account_id, work_id, kind, source_payment_id, granted_at, expires_at) VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5, $6, $7)")
            .bind(&id).bind(account_id).bind(work_id).bind(kind).bind(source_payment_id).bind(&now).bind(expires_at)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

/// Check whether an account has a valid entitlement for a work.
/// Uses lexicographic RFC3339 timestamp comparison — safe because
/// both `expires_at` and the `now` bound come from `now_rfc3339`
/// (same formatter, same clock). Do not mix with system-time
/// timestamps elsewhere without reformatting.
pub async fn has_entitlement(
    db: &Database,
    account_id: &str,
    work_id: &str,
) -> Result<bool, sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    let count: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(
                "SELECT COUNT(*) FROM work_entitlements WHERE account_id = ? AND work_id = ? AND (expires_at IS NULL OR expires_at > ?)"
            )
            .bind(account_id).bind(work_id).bind(&now)
            .fetch_one(db.sqlite_pool().expect("sqlite")).await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(
                "SELECT COUNT(*) FROM work_entitlements WHERE account_id = $1::uuid AND work_id = $2::uuid AND (expires_at IS NULL OR expires_at > $3)"
            )
            .bind(account_id).bind(work_id).bind(&now)
            .fetch_one(db.postgres_pool().expect("postgres")).await?
        }
    };
    if count > 0 {
        return Ok(true);
    }
    // Gifts also grant entitlement (spec §16.8).
    let gift_count: i64 =
        match db.backend() {
            Backend::Sqlite => {
                sqlx::query_scalar(
                    "SELECT COUNT(*) FROM work_gifts WHERE recipient_pseud_id = ? AND work_id = ?",
                )
                .bind(account_id)
                .bind(work_id)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
            }
            Backend::Postgres => sqlx::query_scalar(
                "SELECT COUNT(*) FROM work_gifts WHERE recipient_pseud_id = $1::uuid AND work_id = $2::uuid",
            )
            .bind(account_id)
            .bind(work_id)
            .fetch_one(db.postgres_pool().expect("postgres"))
            .await?,
        };
    Ok(gift_count > 0)
}

/// Get entitlements for an account.
pub async fn get_entitlements(
    db: &Database,
    account_id: &str,
) -> Result<Vec<EntitlementRow>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            fetch_entitlements_sqlite(db.sqlite_pool().expect("sqlite"), account_id).await
        }
        Backend::Postgres => {
            fetch_entitlements_postgres(db.postgres_pool().expect("postgres"), account_id).await
        }
    }
}

/// Post an earnings entry. Replay-safe via idempotency key. Returns the row id.
pub async fn post_earnings(
    db: &Database,
    author_account_id: Option<&str>,
    amount_minor: i64,
    currency: &str,
    kind: &str,
    payment_id: Option<&str>,
    idempotency_key: Option<&str>,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            let mut tx = db.sqlite_pool().expect("sqlite").begin().await?;
            if let Some(key) = idempotency_key {
                let existing: Option<String> = sqlx::query_scalar(
                    "SELECT id FROM author_earnings_ledger WHERE idempotency_key = ?",
                )
                .bind(key)
                .fetch_optional(&mut *tx)
                .await?;
                if let Some(id) = existing {
                    tx.commit().await?;
                    return Ok(id);
                }
            }
            sqlx::query(
                "INSERT INTO author_earnings_ledger (id, author_account_id, amount_minor, currency, kind, payment_id, idempotency_key, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
            )
            .bind(&id).bind(author_account_id).bind(amount_minor).bind(currency).bind(kind)
            .bind(payment_id).bind(idempotency_key).bind(&now)
            .execute(&mut *tx).await?;
            tx.commit().await?;
        }
        Backend::Postgres => {
            let mut tx = db.postgres_pool().expect("postgres").begin().await?;
            if let Some(key) = idempotency_key {
                let existing: Option<String> = sqlx::query_scalar(
                    "SELECT id::text FROM author_earnings_ledger WHERE idempotency_key = $1",
                )
                .bind(key)
                .fetch_optional(&mut *tx)
                .await?;
                if let Some(id) = existing {
                    tx.commit().await?;
                    return Ok(id);
                }
            }
            sqlx::query(
                "INSERT INTO author_earnings_ledger (id, author_account_id, amount_minor, currency, kind, payment_id, idempotency_key, created_at) VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6, $7, $8)"
            )
            .bind(&id).bind(author_account_id).bind(amount_minor).bind(currency).bind(kind)
            .bind(payment_id).bind(idempotency_key).bind(&now)
            .execute(&mut *tx).await?;
            tx.commit().await?;
        }
    }
    Ok(id)
}

/// Get earnings for an account.
pub async fn get_earnings(
    db: &Database,
    account_id: &str,
) -> Result<Vec<EarningsRow>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            fetch_earnings_sqlite(db.sqlite_pool().expect("sqlite"), account_id).await
        }
        Backend::Postgres => {
            fetch_earnings_postgres(db.postgres_pool().expect("postgres"), account_id).await
        }
    }
}

/// Create a payout request.
pub async fn create_payout(
    db: &Database,
    author_account_id: &str,
    amount_minor: i64,
    currency: &str,
    processor_reference: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO payouts (id, author_account_id, amount_minor, currency, processor_reference, status, initiated_at)
                 VALUES (?, ?, ?, ?, ?, 'initiated', ?)"
            )
            .bind(&id).bind(author_account_id).bind(amount_minor).bind(currency).bind(processor_reference).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO payouts (id, author_account_id, amount_minor, currency, processor_reference, status, initiated_at) VALUES ($1::uuid, $2::uuid, $3, $4, $5, 'initiated', $6)"
            )
            .bind(&id).bind(author_account_id).bind(amount_minor).bind(currency).bind(processor_reference).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

/// Record a monetization assertion (original | rights-held).
pub async fn record_assertion(
    db: &Database,
    work_id: &str,
    assertion_kind: &str,
    policy_version: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO monetization_assertions (id, work_id, assertion_kind, policy_version, accepted_at)
                 VALUES (?, ?, ?, ?, ?)"
            )
            .bind(&id).bind(work_id).bind(assertion_kind).bind(policy_version).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO monetization_assertions (id, work_id, assertion_kind, policy_version, accepted_at) VALUES ($1::uuid, $2::uuid, $3, $4, $5)"
            )
            .bind(&id).bind(work_id).bind(assertion_kind).bind(policy_version).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

/// Get the latest assertion for a work.
pub async fn get_assertion(
    db: &Database,
    work_id: &str,
    kind: &str,
) -> Result<Option<AssertionRow>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            fetch_assertion_sqlite(db.sqlite_pool().expect("sqlite"), work_id, kind).await
        }
        Backend::Postgres => {
            fetch_assertion_postgres(db.postgres_pool().expect("postgres"), work_id, kind).await
        }
    }
}

/// Create a gift. Note: block-neutrality (spec §18.10) is not yet enforced
/// in the repository layer — TODO: add block check before insert so the
/// response does not reveal whether the recipient blocked the giver.
pub async fn create_gift(
    db: &Database,
    work_id: &str,
    recipient_pseud_id: &str,
    gift_note: Option<&str>,
    challenge_fulfillment_id: Option<&str>,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO work_gifts (id, work_id, recipient_pseud_id, gift_note, challenge_fulfillment_id, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)"
            )
            .bind(&id).bind(work_id).bind(recipient_pseud_id).bind(gift_note).bind(challenge_fulfillment_id).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO work_gifts (id, work_id, recipient_pseud_id, gift_note, challenge_fulfillment_id, created_at) VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5, $6)"
            )
            .bind(&id).bind(work_id).bind(recipient_pseud_id).bind(gift_note).bind(challenge_fulfillment_id).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

/// Get gifts for a recipient pseud.
pub async fn get_gifts_for_recipient(
    db: &Database,
    recipient_pseud_id: &str,
) -> Result<Vec<GiftRow>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            fetch_gifts_sqlite(db.sqlite_pool().expect("sqlite"), recipient_pseud_id).await
        }
        Backend::Postgres => {
            fetch_gifts_postgres(db.postgres_pool().expect("postgres"), recipient_pseud_id).await
        }
    }
}

/// Total platform revenue (sum of `amount_minor` across all earnings rows),
/// or 0 when the ledger is empty. ADR 0004: balanced ledger, idempotency keys.
pub async fn total_platform_revenue(db: &Database) -> Result<i64, sqlx::Error> {
    let sql = "SELECT COALESCE(SUM(amount_minor), 0) FROM author_earnings_ledger WHERE kind = 'platform_fee'";
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(sql)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await
        }
        Backend::Postgres => {
            sqlx::query_scalar(sql)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await
        }
    }
}

/// Total pending payouts (sum of amount_minor across payouts not yet processed).
pub async fn pending_payout_total(db: &Database) -> Result<i64, sqlx::Error> {
    let sql =
        "SELECT COALESCE(SUM(amount_minor), 0) FROM payouts WHERE status = 'initiated'";
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(sql)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await
        }
        Backend::Postgres => {
            sqlx::query_scalar(sql)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await
        }
    }
}

/// Count of distinct author accounts with earnings in the ledger.
pub async fn active_earning_authors(db: &Database) -> Result<i64, sqlx::Error> {
    let sql = "SELECT COUNT(DISTINCT author_account_id) FROM author_earnings_ledger WHERE kind = 'platform_fee'";
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(sql)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await
        }
        Backend::Postgres => {
            sqlx::query_scalar(sql)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await
        }
    }
}

/// Count of distinct purchasing account IDs that hold at least one entitlement.
pub async fn active_purchaser_count(db: &Database) -> Result<i64, sqlx::Error> {
    let sql = "SELECT COUNT(DISTINCT account_id) FROM work_entitlements";
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(sql)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await
        }
        Backend::Postgres => {
            sqlx::query_scalar(sql)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await
        }
    }
}

/// Check whether a work has an enabled "purchase" pricing model (i.e., is monetized).
pub async fn is_work_priced(db: &Database, work_id: &str) -> Result<bool, sqlx::Error> {
    let sql = "SELECT EXISTS(SELECT 1 FROM work_pricing WHERE work_id = ? AND model = 'purchase' AND enabled)";
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(sql).bind(work_id).fetch_one(db.sqlite_pool().expect("sqlite")).await
        }
        Backend::Postgres => {
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM work_pricing WHERE work_id = $1::uuid AND model = 'purchase' AND enabled)")
                .bind(work_id).fetch_one(db.postgres_pool().expect("postgres")).await
        }
    }
}

/// Set or update the AI declaration for a work.
pub async fn set_ai_declaration(
    db: &Database,
    work_id: &str,
    declaration: &str,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO work_ai_declarations (work_id, declaration, declared_at)
                 VALUES (?, ?, ?)
                 ON CONFLICT(work_id) DO UPDATE SET declaration = ?, revised_at = ?"
            )
            .bind(work_id).bind(declaration).bind(&now)
            .bind(declaration).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO work_ai_declarations (work_id, declaration, declared_at)
                 VALUES ($1::uuid, $2, $3)
                 ON CONFLICT(work_id) DO UPDATE SET declaration = $2, revised_at = $3"
            )
            .bind(work_id).bind(declaration).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(())
}

/// Get the AI declaration for a work (if any).
pub async fn get_ai_declaration(db: &Database, work_id: &str) -> Result<Option<String>, sqlx::Error> {
    let sql = "SELECT declaration FROM work_ai_declarations WHERE work_id = ?";
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(sql)
                .bind(work_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite")).await
        }
        Backend::Postgres => {
            sqlx::query_scalar("SELECT declaration FROM work_ai_declarations WHERE work_id = $1::uuid")
                .bind(work_id)
                .fetch_optional(db.postgres_pool().expect("postgres")).await
        }
    }
}

/// Record a payment event with processor fee.
pub async fn record_payment_event(
    db: &Database,
    kind: &str,
    account_id: Option<&str>,
    work_id: Option<&str>,
    author_account: Option<&str>,
    amount_minor: i64,
    processor_fee_minor: i64,
    currency: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let net = amount_minor - processor_fee_minor;
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO payment_events (id, kind, account_id, work_id, author_account, amount_minor, processor_fee_minor, currency, net_minor, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
            )
            .bind(&id).bind(kind).bind(account_id).bind(work_id).bind(author_account)
            .bind(amount_minor).bind(processor_fee_minor).bind(currency).bind(net).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO payment_events (id, kind, account_id, work_id, author_account, amount_minor, processor_fee_minor, currency, net_minor, created_at)
                 VALUES ($1::uuid, $2, $3::uuid, $4::uuid, $5::uuid, $6, $7, $8, $9, $10)"
            )
            .bind(&id).bind(kind).bind(account_id).bind(work_id).bind(author_account)
            .bind(amount_minor).bind(processor_fee_minor).bind(currency).bind(net).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

// ---------------------------------------------------------------------------
// Pool settlement (spec §20.10)
// ---------------------------------------------------------------------------

/// Record a Pool B distribution (idempotent via idempotency_key).
pub async fn record_pool_b_distribution(
    db: &Database,
    period_start: &str,
    period_end: &str,
    author_account_id: &str,
    amount_minor: i64,
    currency: &str,
    quality_score_bp: i64,
    attributed_reading_time_seconds: i64,
    ai_multiplier_bp: i64,
    idempotency_key: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO pool_b_distributions
                 (id, period_start, period_end, author_account_id, amount_minor,
                  currency, quality_score_bp, attributed_reading_time_seconds,
                  ai_multiplier_bp, idempotency_key, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(idempotency_key) DO NOTHING"
            )
            .bind(&id).bind(period_start).bind(period_end)
            .bind(author_account_id).bind(amount_minor).bind(currency)
            .bind(quality_score_bp).bind(attributed_reading_time_seconds)
            .bind(ai_multiplier_bp).bind(idempotency_key).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO pool_b_distributions
                 (id, period_start, period_end, author_account_id, amount_minor,
                  currency, quality_score_bp, attributed_reading_time_seconds,
                  ai_multiplier_bp, idempotency_key, created_at)
                 VALUES ($1::uuid, $2, $3, $4::uuid, $5, $6, $7, $8, $9, $10, $11)
                 ON CONFLICT (idempotency_key) DO NOTHING"
            )
            .bind(&id).bind(period_start).bind(period_end)
            .bind(author_account_id).bind(amount_minor).bind(currency)
            .bind(quality_score_bp).bind(attributed_reading_time_seconds)
            .bind(ai_multiplier_bp).bind(idempotency_key).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

/// Upsert a monetization period summary.
pub async fn upsert_period_summary(
    db: &Database,
    period_start: &str,
    period_end: &str,
    pool_a_total_minor: i64,
    pool_b_total_minor: i64,
    active_earner_median_minor: i64,
    cap_value_minor: i64,
    authors_in_pool_a: i64,
    authors_in_pool_b: i64,
    authors_capped: i64,
    processor_fee_min_minor: i64,
    processor_fee_max_minor: i64,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO monetization_period_summaries
                 (id, period_start, period_end, pool_a_total_minor, pool_b_total_minor,
                  active_earner_median_minor, cap_value_minor, authors_in_pool_a,
                  authors_in_pool_b, authors_capped, processor_fee_min_minor,
                  processor_fee_max_minor, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(period_start, period_end) DO UPDATE SET
                   pool_a_total_minor = excluded.pool_a_total_minor,
                   pool_b_total_minor = excluded.pool_b_total_minor,
                   active_earner_median_minor = excluded.active_earner_median_minor,
                   cap_value_minor = excluded.cap_value_minor,
                   authors_in_pool_a = excluded.authors_in_pool_a,
                   authors_in_pool_b = excluded.authors_in_pool_b,
                   authors_capped = excluded.authors_capped,
                   processor_fee_min_minor = excluded.processor_fee_min_minor,
                   processor_fee_max_minor = excluded.processor_fee_max_minor"
            )
            .bind(&id).bind(period_start).bind(period_end)
            .bind(pool_a_total_minor).bind(pool_b_total_minor)
            .bind(active_earner_median_minor).bind(cap_value_minor)
            .bind(authors_in_pool_a).bind(authors_in_pool_b)
            .bind(authors_capped).bind(processor_fee_min_minor)
            .bind(processor_fee_max_minor).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO monetization_period_summaries
                 (id, period_start, period_end, pool_a_total_minor, pool_b_total_minor,
                  active_earner_median_minor, cap_value_minor, authors_in_pool_a,
                  authors_in_pool_b, authors_capped, processor_fee_min_minor,
                  processor_fee_max_minor, created_at)
                 VALUES ($1::uuid, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                 ON CONFLICT (period_start, period_end) DO UPDATE SET
                   pool_a_total_minor = excluded.pool_a_total_minor,
                   pool_b_total_minor = excluded.pool_b_total_minor,
                   active_earner_median_minor = excluded.active_earner_median_minor,
                   cap_value_minor = excluded.cap_value_minor,
                   authors_in_pool_a = excluded.authors_in_pool_a,
                   authors_in_pool_b = excluded.authors_in_pool_b,
                   authors_capped = excluded.authors_capped,
                   processor_fee_min_minor = excluded.processor_fee_min_minor,
                   processor_fee_max_minor = excluded.processor_fee_max_minor"
            )
            .bind(&id).bind(period_start).bind(period_end)
            .bind(pool_a_total_minor).bind(pool_b_total_minor)
            .bind(active_earner_median_minor).bind(cap_value_minor)
            .bind(authors_in_pool_a).bind(authors_in_pool_b)
            .bind(authors_capped).bind(processor_fee_min_minor)
            .bind(processor_fee_max_minor).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}
