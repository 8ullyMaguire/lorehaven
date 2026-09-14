//! M21 — Monetization repository (spec §20.9).
//!
//! Work pricing, entitlements, author earnings ledger, payouts,
//! monetization assertions, and work gifts.

use sqlx::Row;
use uuid::Uuid;

use crate::{Backend, Database};

// ---------------------------------------------------------------------------
// Helpers: each dialect returns a common type
// ---------------------------------------------------------------------------

async fn fetch_pricing_sqlite(pool: &sqlx::SqlitePool, work_id: &str) -> Result<Vec<PricingRow>, sqlx::Error> {
    let rows = sqlx::query("SELECT id, model, price_minor, currency, public_at_offset, enabled, created_at, updated_at, version FROM work_pricing WHERE work_id = ?")
        .bind(work_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(|r| PricingRow {
        id: r.get::<String, _>("id"),
        model: r.get::<String, _>("model"),
        price_minor: r.get::<i64, _>("price_minor"),
        currency: r.get::<String, _>("currency"),
        public_at_offset: r.get::<Option<i64>, _>("public_at_offset"),
        enabled: r.get::<i64, _>("enabled") != 0,
        created_at: r.get::<String, _>("created_at"),
        updated_at: r.get::<String, _>("updated_at"),
        version: r.get::<i64, _>("version"),
    }).collect())
}

async fn fetch_pricing_postgres(pool: &sqlx::postgres::PgPool, work_id: &str) -> Result<Vec<PricingRow>, sqlx::Error> {
    let rows = sqlx::query("SELECT id, model, price_minor, currency, public_at_offset, enabled, created_at, updated_at, version FROM work_pricing WHERE work_id = $1")
        .bind(work_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(|r| PricingRow {
        id: r.get::<String, _>("id"),
        model: r.get::<String, _>("model"),
        price_minor: r.get::<i64, _>("price_minor"),
        currency: r.get::<String, _>("currency"),
        public_at_offset: r.get::<Option<i64>, _>("public_at_offset"),
        enabled: r.get::<bool, _>("enabled"),
        created_at: r.get::<String, _>("created_at"),
        updated_at: r.get::<String, _>("updated_at"),
        version: r.get::<i64, _>("version"),
    }).collect())
}

async fn fetch_entitlements_sqlite(pool: &sqlx::SqlitePool, account_id: &str) -> Result<Vec<EntitlementRow>, sqlx::Error> {
    let rows = sqlx::query("SELECT id, work_id, kind, source_payment_id, granted_at, expires_at FROM work_entitlements WHERE account_id = ?")
        .bind(account_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(|r| EntitlementRow {
        id: r.get::<String, _>("id"),
        work_id: r.get::<String, _>("work_id"),
        kind: r.get::<String, _>("kind"),
        source_payment_id: r.get::<Option<String>, _>("source_payment_id"),
        granted_at: r.get::<String, _>("granted_at"),
        expires_at: r.get::<Option<String>, _>("expires_at"),
    }).collect())
}

async fn fetch_entitlements_postgres(pool: &sqlx::postgres::PgPool, account_id: &str) -> Result<Vec<EntitlementRow>, sqlx::Error> {
    let rows = sqlx::query("SELECT id, work_id, kind, source_payment_id, granted_at, expires_at FROM work_entitlements WHERE account_id = $1")
        .bind(account_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(|r| EntitlementRow {
        id: r.get::<String, _>("id"),
        work_id: r.get::<String, _>("work_id"),
        kind: r.get::<String, _>("kind"),
        source_payment_id: r.get::<Option<String>, _>("source_payment_id"),
        granted_at: r.get::<String, _>("granted_at"),
        expires_at: r.get::<Option<String>, _>("expires_at"),
    }).collect())
}

async fn fetch_earnings_sqlite(pool: &sqlx::SqlitePool, account_id: &str) -> Result<Vec<EarningsRow>, sqlx::Error> {
    let rows = sqlx::query("SELECT id, amount_minor, currency, kind, payment_id, idempotency_key, created_at FROM author_earnings_ledger WHERE author_account_id = ? ORDER BY created_at DESC")
        .bind(account_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(|r| EarningsRow {
        id: r.get::<String, _>("id"),
        amount_minor: r.get::<i64, _>("amount_minor"),
        currency: r.get::<String, _>("currency"),
        kind: r.get::<String, _>("kind"),
        payment_id: r.get::<Option<String>, _>("payment_id"),
        idempotency_key: r.get::<Option<String>, _>("idempotency_key"),
        created_at: r.get::<String, _>("created_at"),
    }).collect())
}

async fn fetch_earnings_postgres(pool: &sqlx::postgres::PgPool, account_id: &str) -> Result<Vec<EarningsRow>, sqlx::Error> {
    let rows = sqlx::query("SELECT id, amount_minor, currency, kind, payment_id, idempotency_key, created_at FROM author_earnings_ledger WHERE author_account_id = $1 ORDER BY created_at DESC")
        .bind(account_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(|r| EarningsRow {
        id: r.get::<String, _>("id"),
        amount_minor: r.get::<i64, _>("amount_minor"),
        currency: r.get::<String, _>("currency"),
        kind: r.get::<String, _>("kind"),
        payment_id: r.get::<Option<String>, _>("payment_id"),
        idempotency_key: r.get::<Option<String>, _>("idempotency_key"),
        created_at: r.get::<String, _>("created_at"),
    }).collect())
}

async fn has_idempotency_sqlite(pool: &sqlx::SqlitePool, key: &str) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM author_earnings_ledger WHERE idempotency_key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
}

async fn has_idempotency_postgres(pool: &sqlx::postgres::PgPool, key: &str) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM author_earnings_ledger WHERE idempotency_key = $1")
        .bind(key)
        .fetch_optional(pool)
        .await
}

async fn fetch_assertion_sqlite(pool: &sqlx::SqlitePool, work_id: &str, kind: &str) -> Result<Option<AssertionRow>, sqlx::Error> {
    sqlx::query("SELECT id, assertion_kind, policy_version, accepted_at, revoked_at FROM monetization_assertions WHERE work_id = ? AND assertion_kind = ? ORDER BY accepted_at DESC LIMIT 1")
        .bind(work_id).bind(kind)
        .fetch_optional(pool)
        .await
        .map(|opt| opt.map(|r| AssertionRow {
            id: r.get::<String, _>("id"),
            assertion_kind: r.get::<String, _>("assertion_kind"),
            policy_version: r.get::<String, _>("policy_version"),
            accepted_at: r.get::<String, _>("accepted_at"),
            revoked_at: r.get::<Option<String>, _>("revoked_at"),
        }))
}

async fn fetch_assertion_postgres(pool: &sqlx::postgres::PgPool, work_id: &str, kind: &str) -> Result<Option<AssertionRow>, sqlx::Error> {
    sqlx::query("SELECT id, assertion_kind, policy_version, accepted_at, revoked_at FROM monetization_assertions WHERE work_id = $1 AND assertion_kind = $2 ORDER BY accepted_at DESC LIMIT 1")
        .bind(work_id).bind(kind)
        .fetch_optional(pool)
        .await
        .map(|opt| opt.map(|r| AssertionRow {
            id: r.get::<String, _>("id"),
            assertion_kind: r.get::<String, _>("assertion_kind"),
            policy_version: r.get::<String, _>("policy_version"),
            accepted_at: r.get::<String, _>("accepted_at"),
            revoked_at: r.get::<Option<String>, _>("revoked_at"),
        }))
}

// ---------------------------------------------------------------------------
// Common types (returned regardless of dialect)
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
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO work_pricing (id, work_id, model, price_minor, currency, public_at_offset, enabled, created_at, updated_at, version)
                 VALUES (?, ?, ?, ?, ?, ?, 1, ?, ?, 1)
                 ON CONFLICT(work_id) DO UPDATE SET
                   model = excluded.model,
                   price_minor = excluded.price_minor,
                   currency = excluded.currency,
                   public_at_offset = excluded.public_at_offset,
                   updated_at = excluded.updated_at,
                   version = version + 1"
            )
            .bind(&id).bind(work_id).bind(model).bind(price_minor).bind(currency)
            .bind(public_at_offset).bind(&now).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO work_pricing (id, work_id, model, price_minor, currency, public_at_offset, enabled, created_at, updated_at, version)
                 VALUES ($1, $2, $3, $4, $5, $6, true, $7, $8, 1)
                 ON CONFLICT(work_id) DO UPDATE SET
                   model = excluded.model,
                   price_minor = excluded.price_minor,
                   currency = excluded.currency,
                   public_at_offset = excluded.public_at_offset,
                   updated_at = excluded.updated_at,
                   version = work_pricing.version + 1"
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
    let rows = match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            sqlx::query("SELECT id, model, price_minor, currency, public_at_offset, enabled, created_at, updated_at, version FROM work_pricing WHERE work_id = ? AND enabled = 1")
                .bind(work_id).fetch_all(pool).await?
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            sqlx::query("SELECT id, model, price_minor, currency, public_at_offset, enabled, created_at, updated_at, version FROM work_pricing WHERE work_id = $1 AND enabled = true")
                .bind(work_id).fetch_all(pool).await?
        }
    };
    Ok(rows.first().map(|r| PricingRow {
        id: r.get::<String, _>("id"),
        model: r.get::<String, _>("model"),
        price_minor: r.get::<i64, _>("price_minor"),
        currency: r.get::<String, _>("currency"),
        public_at_offset: if db.backend() == Backend::Sqlite {
            r.get::<Option<i64>, _>("public_at_offset")
        } else {
            r.get::<Option<i64>, _>("public_at_offset")
        },
        enabled: if db.backend() == Backend::Sqlite {
            r.get::<i64, _>("enabled") != 0
        } else {
            r.get::<bool, _>("enabled")
        },
        created_at: r.get::<String, _>("created_at"),
        updated_at: r.get::<String, _>("updated_at"),
        version: r.get::<i64, _>("version"),
    }))
}

/// Disable pricing for a work.
pub async fn disable_pricing(db: &Database, work_id: &str) -> Result<u64, sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    let res = match db.backend() {
        Backend::Sqlite => {
            sqlx::query("UPDATE work_pricing SET enabled = 0, updated_at = ? WHERE work_id = ?")
                .bind(&now).bind(work_id)
                .execute(db.sqlite_pool().expect("sqlite")).await?
        }
        Backend::Postgres => {
            sqlx::query("UPDATE work_pricing SET enabled = false, updated_at = $1 WHERE work_id = $2")
                .bind(&now).bind(work_id)
                .execute(db.postgres_pool().expect("postgres")).await?
        }
    };
    Ok(res.rows_affected())
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
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO work_entitlements (id, account_id, work_id, kind, source_payment_id, granted_at, expires_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(account_id, work_id, kind) DO UPDATE SET
                   granted_at = excluded.granted_at"
            )
            .bind(&id).bind(account_id).bind(work_id).bind(kind).bind(source_payment_id).bind(&now).bind(expires_at)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO work_entitlements (id, account_id, work_id, kind, source_payment_id, granted_at, expires_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT(account_id, work_id, kind) DO UPDATE SET
                   granted_at = excluded.granted_at"
            )
            .bind(&id).bind(account_id).bind(work_id).bind(kind).bind(source_payment_id).bind(&now).bind(expires_at)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

/// Check whether an account has a valid entitlement for a work.
pub async fn has_entitlement(db: &Database, account_id: &str, work_id: &str) -> Result<bool, sqlx::Error> {
    let count: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(
                "SELECT COUNT(*) FROM work_entitlements
                 WHERE account_id = ? AND work_id = ? AND (expires_at IS NULL OR expires_at > ?)"
            )
            .bind(account_id).bind(work_id).bind(&crate::identity::now_rfc3339())
            .fetch_one(db.sqlite_pool().expect("sqlite")).await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(
                "SELECT COUNT(*) FROM work_entitlements
                 WHERE account_id = $1 AND work_id = $2 AND (expires_at IS NULL OR expires_at > $3)"
            )
            .bind(account_id).bind(work_id).bind(&crate::identity::now_rfc3339())
            .fetch_one(db.postgres_pool().expect("postgres")).await?
        }
    };
    Ok(count > 0)
}

/// Get entitlements for an account.
pub async fn get_entitlements(db: &Database, account_id: &str) -> Result<Vec<EntitlementRow>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => fetch_entitlements_sqlite(db.sqlite_pool().expect("sqlite"), account_id).await,
        Backend::Postgres => fetch_entitlements_postgres(db.postgres_pool().expect("postgres"), account_id).await,
    }
}

/// Post an earnings entry. Replay-safe via idempotency key. Returns the row id.
pub async fn post_earnings(
    db: &Database,
    author_account_id: &str,
    amount_minor: i64,
    currency: &str,
    kind: &str,
    payment_id: Option<&str>,
    idempotency_key: Option<&str>,
) -> Result<String, sqlx::Error> {
    // Idempotency check
    if let Some(key) = idempotency_key {
        let existing = match db.backend() {
            Backend::Sqlite => has_idempotency_sqlite(db.sqlite_pool().expect("sqlite"), key).await?,
            Backend::Postgres => has_idempotency_postgres(db.postgres_pool().expect("postgres"), key).await?,
        };
        if let Some(id) = existing {
            return Ok(id);
        }
    }

    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO author_earnings_ledger (id, author_account_id, amount_minor, currency, kind, payment_id, idempotency_key, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
            )
            .bind(&id).bind(author_account_id).bind(amount_minor).bind(currency).bind(kind)
            .bind(payment_id).bind(idempotency_key).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO author_earnings_ledger (id, author_account_id, amount_minor, currency, kind, payment_id, idempotency_key, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
            )
            .bind(&id).bind(author_account_id).bind(amount_minor).bind(currency).bind(kind)
            .bind(payment_id).bind(idempotency_key).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

/// Get earnings for an account.
pub async fn get_earnings(db: &Database, account_id: &str) -> Result<Vec<EarningsRow>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => fetch_earnings_sqlite(db.sqlite_pool().expect("sqlite"), account_id).await,
        Backend::Postgres => fetch_earnings_postgres(db.postgres_pool().expect("postgres"), account_id).await,
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
                "INSERT INTO payouts (id, author_account_id, amount_minor, currency, processor_reference, status, initiated_at)
                 VALUES ($1, $2, $3, $4, $5, 'initiated', $6)"
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
                "INSERT INTO monetization_assertions (id, work_id, assertion_kind, policy_version, accepted_at)
                 VALUES ($1, $2, $3, $4, $5)"
            )
            .bind(&id).bind(work_id).bind(assertion_kind).bind(policy_version).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

/// Get the latest assertion for a work.
pub async fn get_assertion(db: &Database, work_id: &str, kind: &str) -> Result<Option<AssertionRow>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => fetch_assertion_sqlite(db.sqlite_pool().expect("sqlite"), work_id, kind).await,
        Backend::Postgres => fetch_assertion_postgres(db.postgres_pool().expect("postgres"), work_id, kind).await,
    }
}

/// Create a gift. Returns error if the recipient has blocked the giver (reveals nothing).
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
                "INSERT INTO work_gifts (id, work_id, recipient_pseud_id, gift_note, challenge_fulfillment_id, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6)"
            )
            .bind(&id).bind(work_id).bind(recipient_pseud_id).bind(gift_note).bind(challenge_fulfillment_id).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

/// Get gifts for a recipient pseud.
pub async fn get_gifts_for_recipient(db: &Database, recipient_pseud_id: &str) -> Result<Vec<GiftRow>, sqlx::Error> {
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query("SELECT id, work_id, gift_note, challenge_fulfillment_id, created_at, declined_at FROM work_gifts WHERE recipient_pseud_id = ? ORDER BY created_at DESC")
                .bind(recipient_pseud_id)
                .fetch_all(db.sqlite_pool().expect("sqlite")).await?
        }
        Backend::Postgres => {
            sqlx::query("SELECT id, work_id, gift_note, challenge_fulfillment_id, created_at, declined_at FROM work_gifts WHERE recipient_pseud_id = $1 ORDER BY created_at DESC")
                .bind(recipient_pseud_id)
                .fetch_all(db.postgres_pool().expect("postgres")).await?
        }
    };
    Ok(rows.iter().map(|r| GiftRow {
        id: r.get::<String, _>("id"),
        work_id: r.get::<String, _>("work_id"),
        gift_note: r.get::<Option<String>, _>("gift_note"),
        challenge_fulfillment_id: r.get::<Option<String>, _>("challenge_fulfillment_id"),
        created_at: r.get::<String, _>("created_at"),
        declined_at: r.get::<Option<String>, _>("declined_at"),
    }).collect())
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
