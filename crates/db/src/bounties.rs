//! Flexible bounties (spec §20.3.2, M18 Phase 4.1).
//!
//! Three bounty types share the `bounties` table:
//!
//! * `standard` — the classic escrow bounty. One creator funds it, one
//!   claimant fulfils it.
//! * `crowdfunded` — multiple contributors pool credits. The bounty stays
//!   `funding` until `funded_amount` reaches `amount`; then it activates
//!   (`state = 'open'`, `activated_at` set).
//! * `reverse` — a reader posts what they want to read; authors bid. The
//!   creator prepays, so it is `open` immediately, but the amount is the
//!   creator's maximum, not an escrow.
//!
//! The dual-backend rule applies: every query has a SQLite and a PostgreSQL
//! form, selected by [`Database::backend`].

use crate::Database;
use serde_json::{json, Value};

/// The `bounties` row as selected by the read queries, in column order.
///
/// Named because the row is read in several places and the ten-element tuple is
/// otherwise unreadable. The element order must match the SELECT lists exactly.
/// The two integer columns are `INTEGER` in PostgreSQL and widen to `i64` in
/// the query. They decode as `i64` here only because the PostgreSQL arm casts;
/// SQLite hands back `i64` for the same columns, so one type serves both.
type BountyRow = (
    String,
    String,
    String,
    String,
    i64,
    i64,
    String,
    String,
    String,
    Option<String>,
);

/// A bounty as the API renders it.
#[derive(Debug, Clone)]
pub struct Bounty {
    pub id: String,
    pub bounty_type: String,
    pub job_kind: String,
    pub terms: String,
    pub amount: i64,
    pub funded_amount: i64,
    pub state: String,
    pub created_by: String,
    pub created_at: String,
    pub activated_at: Option<String>,
}

/// Insert a bounty row of any type.
pub async fn create_bounty_typed(db: &Database, bounty: &Bounty) -> Result<(), sqlx::Error> {
    match db.backend() {
        crate::Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite handle");
            sqlx::query(
                "INSERT INTO bounties (id, job_kind, terms, escrow_transaction, state, claimant, created_by, created_at, account, amount, type, funded_amount, activated_at)
                 VALUES (?, ?, ?, 'pending', ?, '', ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&bounty.id)
            .bind(&bounty.job_kind)
            .bind(&bounty.terms)
            .bind(&bounty.state)
            .bind(&bounty.created_by)
            .bind(&bounty.created_at)
            .bind(&bounty.created_by)
            .bind(bounty.amount)
            .bind(&bounty.bounty_type)
            .bind(bounty.funded_amount)
            .bind(&bounty.activated_at)
            .execute(pool)
            .await?;
        }
        crate::Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            sqlx::query(
                "INSERT INTO bounties (id, job_kind, terms, escrow_transaction, state, claimant, created_by, created_at, account, amount, type, funded_amount, activated_at)
                 VALUES ($1, $2, $3, 'pending', $4, '', $5, $6, $7, $8, $9, $10, $11)",
            )
            .bind(&bounty.id)
            .bind(&bounty.job_kind)
            .bind(&bounty.terms)
            .bind(&bounty.state)
            .bind(&bounty.created_by)
            .bind(&bounty.created_at)
            .bind(&bounty.created_by)
            .bind(bounty.amount)
            .bind(&bounty.bounty_type)
            .bind(bounty.funded_amount)
            .bind(&bounty.activated_at)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

/// Contribute credits to a crowdfunded bounty. Activates it when the funding
/// threshold is met. Returns the new `funded_amount` and whether this call
/// activated the bounty.
pub async fn contribute_to_bounty(
    db: &Database,
    bounty_id: &str,
    contributor: &str,
    amount: i64,
    activation_threshold: f64,
) -> Result<(i64, bool), FlexibleBountyError> {
    if amount <= 0 {
        return Err(FlexibleBountyError::InvalidAmount);
    }

    let bounty = fetch_bounty(db, bounty_id)
        .await
        .map_err(FlexibleBountyError::Sql)?
        .ok_or(FlexibleBountyError::NotFound)?;

    if bounty.bounty_type != "crowdfunded" {
        return Err(FlexibleBountyError::NotCrowdfunded);
    }
    if bounty.state != "funding" {
        return Err(FlexibleBountyError::NotFunding);
    }

    let new_funded = bounty.funded_amount + amount;
    let required = (bounty.amount as f64 * activation_threshold).ceil() as i64;
    let activated = required > 0 && new_funded >= required;

    let now = crate::identity::now_rfc3339();
    let new_state = if activated { "open" } else { "funding" };
    let activated_at = if activated {
        Some(now)
    } else {
        bounty.activated_at
    };

    match db.backend() {
        crate::Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite handle");
            sqlx::query(
                "UPDATE bounties SET funded_amount = ?, state = ?, activated_at = ? WHERE id = ?",
            )
            .bind(new_funded)
            .bind(new_state)
            .bind(&activated_at)
            .bind(bounty_id)
            .execute(pool)
            .await
            .map_err(FlexibleBountyError::Sql)?;
        }
        crate::Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            sqlx::query(
                "UPDATE bounties SET funded_amount = $1, state = $2, activated_at = $3 WHERE id = $4",
            )
            .bind(new_funded)
            .bind(new_state)
            .bind(&activated_at)
            .bind(bounty_id)
            .execute(pool)
            .await
            .map_err(FlexibleBountyError::Sql)?;
        }
    }

    // Record the contribution itself for audit (best-effort; the table may
    // not exist on older deployments).
    let _ = record_contribution(db, bounty_id, contributor, amount).await;

    Ok((new_funded, activated))
}

/// Record a contribution row (SQLite and PostgreSQL).
async fn record_contribution(
    db: &Database,
    bounty_id: &str,
    contributor: &str,
    amount: i64,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        crate::Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite handle");
            sqlx::query(
                "INSERT INTO bounty_contributions (bounty_id, contributor, amount, contributed_at) VALUES (?, ?, ?, ?)",
            )
            .bind(bounty_id)
            .bind(contributor)
            .bind(amount)
            .bind(&now)
            .execute(pool)
            .await?;
        }
        crate::Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            sqlx::query(
                "INSERT INTO bounty_contributions (bounty_id, contributor, amount, contributed_at) VALUES ($1, $2, $3, $4)",
            )
            .bind(bounty_id)
            .bind(contributor)
            .bind(amount)
            .bind(&now)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

/// Fetch a single bounty by id.
pub async fn fetch_bounty(db: &Database, id: &str) -> Result<Option<Bounty>, sqlx::Error> {
    let row: Option<BountyRow> = match db.backend() {
        crate::Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite handle");
            sqlx::query_as(
                "SELECT id, type, job_kind, terms, amount, funded_amount, state, created_by, created_at, activated_at
                 FROM bounties WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(pool)
            .await?
        }
        crate::Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            sqlx::query_as(
                "SELECT id, type, job_kind, terms, amount::bigint, funded_amount::bigint,
                        state, created_by, created_at, activated_at
                 FROM bounties WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(pool)
            .await?
        }
    };

    Ok(row.map(
        |(
            id,
            bounty_type,
            job_kind,
            terms,
            amount,
            funded_amount,
            state,
            created_by,
            created_at,
            activated_at,
        )| {
            Bounty {
                id,
                bounty_type,
                job_kind,
                terms,
                amount,
                funded_amount,
                state,
                created_by,
                created_at,
                activated_at,
            }
        },
    ))
}

/// List open (or funding) bounties, newest first.
pub async fn list_flexible_bounties(db: &Database) -> Result<Vec<Value>, sqlx::Error> {
    let rows: Vec<BountyRow> = match db.backend() {
        crate::Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite handle");
            sqlx::query_as(
                "SELECT id, type, job_kind, terms, amount, funded_amount, state, created_by, created_at, activated_at
                 FROM bounties WHERE state IN ('open', 'funding') ORDER BY created_at DESC LIMIT 50",
            )
            .fetch_all(pool)
            .await?
        }
        crate::Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            // `amount` and `funded_amount` are INTEGER; widen for the i64 row type.
            // `fetch_bounty` does the same, for the same reason.
            sqlx::query_as(
                "SELECT id, type, job_kind, terms, CAST(amount AS BIGINT), CAST(funded_amount AS BIGINT),
                        state, created_by, created_at, activated_at",
            )
            .fetch_all(pool)
            .await?
        }
    };

    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                bounty_type,
                job_kind,
                terms,
                amount,
                funded_amount,
                state,
                created_by,
                created_at,
                activated_at,
            )| {
                json!({
                    "id": id,
                    "type": bounty_type,
                    "job_kind": job_kind,
                    "terms": terms,
                    "amount": amount,
                    "funded_amount": funded_amount,
                    "state": state,
                    "created_by": created_by,
                    "created_at": created_at,
                    "activated_at": activated_at,
                })
            },
        )
        .collect())
}

/// Errors surfaced by the flexible bounties layer.
#[derive(Debug, thiserror::Error)]
pub enum FlexibleBountyError {
    #[error("bounty not found")]
    NotFound,
    #[error("not a crowdfunded bounty")]
    NotCrowdfunded,
    #[error("bounty is not accepting contributions")]
    NotFunding,
    #[error("contribution amount must be positive")]
    InvalidAmount,
    #[error("database error: {0}")]
    Sql(#[source] sqlx::Error),
}
