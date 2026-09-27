//! The metadata exchange's persistence (spec §11.17, §15.17, §16.16.1).
//!
//! Dual-dialect throughout, and the shape of each statement is chosen by a rule
//! in the spec rather than by convenience:
//!
//! - Signals are keyed by content hash, so deduplication is the primary key
//!   rather than a check-then-insert. §11.17 promises a re-import "costs a
//!   submitter nothing and creates no second record", and only a key can keep
//!   that promise under two submitters racing.
//! - Entity `signal_count` is incremented with an upsert, not read-modify-write.
//!   A read-then-write loses an increment whenever two signals for the same new
//!   tag arrive together, which is exactly the case on a fresh instance where
//!   everyone is filing the same new name.
//! - Latent demand is keyed `(work_id, source_instance)`, which is what makes
//!   §16.16.1's "once per work per instance" structural rather than a query that
//!   has to be remembered.
//!
//! Nothing in this module decides *policy* — the trust bars, the normalisation
//! and the batch validation live in `lorehaven_domain::exchange`. This module
//! stores what the domain layer decided.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::Backend;
use crate::Database;

/// §11.17: the instance's exchange settings, including the opt-in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExchangeSettings {
    /// Whether the operator enabled the exchange. `false` means the routes
    /// answer 404, not 403: a 403 confirms the door exists.
    pub enabled: bool,
    /// Submissions per account per hour (§11.17 default 1000).
    pub rate_limit_per_hour: i64,
    /// This instance's own identifier, as it appears in a signal it sends.
    pub instance_id: Option<String>,
}

/// A stored signal, as §15.17 requires it to be retained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredSignal {
    pub content_hash: String,
    pub work_id: Option<String>,
    pub account_id: String,
    pub source_instance: Option<String>,
    pub submitted_at: String,
}

/// A canonical entity as this instance knows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalEntity {
    pub kind: String,
    pub norm: String,
    /// The display form — a curator's canonical spelling, or the first spelling
    /// seen while unverified. Always populated.
    pub canonical: String,
    /// `unverified` or `curated` (§15.17). One vocabulary with
    /// `taxonomy_nodes.review_status`; the wire type's `ReviewStatus::Verified`
    /// is the same state under the name the exchange protocol uses.
    pub review_status: String,
    /// Review priority only. Never a demand weight, never a count of readers.
    pub signal_count: i64,
    pub curated_by: Option<String>,
    pub curated_at: Option<String>,
}

/// A latent-demand item (§16.16.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatentDemand {
    pub work_id: String,
    pub source_instance: String,
    /// How many times this instance has signalled. NOT a count of submitters and
    /// not a count of readers: §16.16.1 says re-signalling the same work adds no
    /// *weight*, and this counter exists so an operator can see that a demand
    /// item is still live without treating the number as a headcount.
    pub signal_count: i64,
    pub reinforced_at: String,
}

/// The one row id used for the singleton settings row.
const SETTINGS_ID: &str = "singleton";

fn bool_to_int(v: bool) -> i64 {
    i64::from(v)
}

/// Read the exchange settings, or `None` when the operator has never touched
/// them. `None` and `Some { enabled: false }` mean the same thing to the
/// routes — the exchange is off — but they are kept distinct so "never
/// configured" is distinguishable from "explicitly disabled" in a log.
pub async fn get_settings(db: &Database) -> Result<Option<ExchangeSettings>> {
    #[derive(sqlx::FromRow)]
    struct Row {
        enabled: bool,
        rate_limit_per_hour: i64,
        instance_id: Option<String>,
    }
    let sql = db.sql(
        "SELECT enabled, rate_limit_per_hour, instance_id FROM exchange_settings WHERE id = ?",
        "SELECT enabled, rate_limit_per_hour, instance_id FROM exchange_settings WHERE id = $1",
    );
    let row: Option<Row> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(SETTINGS_ID)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(SETTINGS_ID)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.map(|r| ExchangeSettings {
        enabled: r.enabled,
        rate_limit_per_hour: r.rate_limit_per_hour,
        instance_id: r.instance_id,
    }))
}

/// Enable or disable the exchange, creating the settings row if it is absent.
///
/// The rate limit defaults to §11.17's 1000/hour on creation, so an operator
/// who flips the switch on does not also have to know and set the second knob.
pub async fn set_enabled(db: &Database, enabled: bool) -> Result<ExchangeSettings> {
    let now = crate::identity::now_rfc3339();
    let enabled_int = bool_to_int(enabled);
    // The insert arm carries the default rate limit; the update arm deliberately
    // does not touch it, so toggling the exchange off and on never silently
    // resets a limit the operator raised.
    let sql = db.sql(
        "INSERT INTO exchange_settings (id, enabled, rate_limit_per_hour, created_at)
         VALUES (?, ?, 1000, ?)
         ON CONFLICT (id) DO UPDATE SET enabled = excluded.enabled",
        "INSERT INTO exchange_settings (id, enabled, rate_limit_per_hour, created_at)
         VALUES ($1, $2, 1000, $3)
         ON CONFLICT (id) DO UPDATE SET enabled = EXCLUDED.enabled",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(SETTINGS_ID)
                .bind(enabled_int)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(SETTINGS_ID)
                .bind(enabled)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    get_settings(db).await?.ok_or_else(|| {
        anyhow::anyhow!("the exchange settings row vanished immediately after being written")
    })
}

/// Set this instance's own identifier, used as `source_instance` on signals it
/// sends and as the dedup key for latent demand.
pub async fn set_instance_id(db: &Database, instance_id: &str) -> Result<()> {
    let sql = db.sql(
        "INSERT INTO exchange_settings (id, enabled, rate_limit_per_hour, instance_id, created_at)
         VALUES (?, 0, 1000, ?, ?)
         ON CONFLICT (id) DO UPDATE SET instance_id = excluded.instance_id",
        "INSERT INTO exchange_settings (id, enabled, rate_limit_per_hour, instance_id, created_at)
         VALUES ($1, false, 1000, $2, $3)
         ON CONFLICT (id) DO UPDATE SET instance_id = EXCLUDED.instance_id",
    );
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(SETTINGS_ID)
                .bind(instance_id)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(SETTINGS_ID)
                .bind(instance_id)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Count an account's submissions in the trailing hour, for §11.17's limit.
///
/// `since` is passed in rather than computed here so the caller decides the
/// clock: the test suite needs a window it can move, and a function that reads
/// `now()` internally cannot be tested for expiry without sleeping.
pub async fn count_recent_submissions(db: &Database, account_id: &str, since: &str) -> Result<i64> {
    // `CAST(COUNT(*) AS BIGINT)` on the PostgreSQL arm and not on the SQLite
    // one: PostgreSQL types COUNT as INT4, and decoding INT4 into an `i64` is a
    // runtime type error on that backend only. Written per-arm rather than as a
    // post-hoc `replace`, because a string substitution on a query is invisible
    // to the reader and to the next dialect check. This is the exact shape of
    // defect the SQLite gate cannot see.
    let sql = match db.backend() {
        Backend::Sqlite => {
            "SELECT COUNT(*) FROM exchange_signals WHERE account_id = ? AND submitted_at >= ?"
        }
        Backend::Postgres => {
            "SELECT CAST(COUNT(*) AS BIGINT) FROM exchange_signals WHERE account_id = $1 AND submitted_at >= $2"
        }
    };
    let n: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(sql)
                .bind(account_id)
                .bind(since)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(sql)
                .bind(account_id)
                .bind(since)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(n)
}

/// Store a signal, or report that it was already stored.
///
/// Returns `true` when the row was newly inserted and `false` when the content
/// hash was already present. §11.17's deduplication, made structural.
pub async fn store_signal(
    db: &Database,
    content_hash: &str,
    work_id: Option<&str>,
    account_id: &str,
    source_instance: Option<&str>,
    payload: &str,
) -> Result<bool> {
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        // `DO NOTHING` rather than `DO UPDATE`: a signal is evidence and §15.17
        // requires the original row to be retained as provenance, never
        // rewritten. An update arm here would silently overwrite a submitter's
        // first statement with their second, which is exactly the loss of
        // provenance §15.17 is written to prevent.
        "INSERT INTO exchange_signals (content_hash, work_id, account_id, source_instance, payload, submitted_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT (content_hash) DO NOTHING",
        "INSERT INTO exchange_signals (content_hash, work_id, account_id, source_instance, payload, submitted_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (content_hash) DO NOTHING",
    );
    let inserted = match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(content_hash)
                .bind(work_id)
                .bind(account_id)
                .bind(source_instance)
                .bind(payload)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?
                .rows_affected()
                > 0
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(content_hash)
                .bind(work_id)
                .bind(account_id)
                .bind(source_instance)
                .bind(payload)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?
                .rows_affected()
                > 0
        }
    };
    Ok(inserted)
}

/// Record which entities a signal named, and reinforce each one's count.
///
/// The upsert is the load-bearing part. On a fresh instance the common case is
/// two submitters filing the same new tag at the same moment; a
/// read-then-write loses one of the two increments, and the count is the review
/// queue's ordering, so a lost increment means a tag looks less urgent than it
/// is. The `excluded` form is the one that works on both dialects.
///
/// An existing entity is left at its `review_status` — reinforcing an unverified
/// entity does not curate it, and curating does not happen as a side effect of
/// being agreed with.
pub async fn reinforce_entities(
    db: &Database,
    content_hash: &str,
    entities: &[(String, String, String)],
) -> Result<()> {
    let now = crate::identity::now_rfc3339();
    let link_sql = db.sql(
        "INSERT INTO exchange_signal_entities (content_hash, kind, value, norm)
         VALUES (?, ?, ?, ?)
         ON CONFLICT (content_hash, kind, norm) DO NOTHING",
        "INSERT INTO exchange_signal_entities (content_hash, kind, value, norm)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (content_hash, kind, norm) DO NOTHING",
    );
    // The canonical form is `COALESCE(NULLIF(canonical, ''), excluded.canonical)`
    // rather than a bare overwrite: the first spelling seen is the display form
    // for an unverified entity (§15.17), and a second submitter writing
    // `Horror` over `Horror` would otherwise replace a curator's chosen
    // canonical form the moment the same name was signalled again.
    let entity_sql = db.sql(
        "INSERT INTO canonical_entities (kind, norm, canonical, review_status, signal_count, created_at, updated_at)
         VALUES (?, ?, ?, 'unverified', 1, ?, ?)
         ON CONFLICT (kind, norm) DO UPDATE SET
             signal_count = canonical_entities.signal_count + 1,
             updated_at = excluded.updated_at",
        "INSERT INTO canonical_entities (kind, norm, canonical, review_status, signal_count, created_at, updated_at)
         VALUES ($1, $2, $3, 'unverified', 1, $4, $5)
         ON CONFLICT (kind, norm) DO UPDATE SET
             signal_count = canonical_entities.signal_count + 1,
             updated_at = EXCLUDED.updated_at",
    );
    match db.backend() {
        Backend::Sqlite => {
            let mut tx = db.sqlite_pool().expect("sqlite").begin().await?;
            for (kind, value, norm) in entities {
                sqlx::query(&link_sql)
                    .bind(content_hash)
                    .bind(kind)
                    .bind(value)
                    .bind(norm)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query(&entity_sql)
                    .bind(kind)
                    .bind(norm)
                    .bind(value)
                    .bind(&now)
                    .bind(&now)
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
        }
        Backend::Postgres => {
            let mut tx = db.postgres_pool().expect("postgres").begin().await?;
            for (kind, value, norm) in entities {
                sqlx::query(&link_sql)
                    .bind(content_hash)
                    .bind(kind)
                    .bind(value)
                    .bind(norm)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query(&entity_sql)
                    .bind(kind)
                    .bind(norm)
                    .bind(value)
                    .bind(&now)
                    .bind(&now)
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
        }
    }
    Ok(())
}

/// Fetch canonical entities by `(kind, norm)`, in any order.
///
/// Missing keys are simply absent from the result: a client asking for a work
/// whose entities nobody has signalled yet gets the ones that exist, and
/// `GET /canonical` is specified to return canonical metadata for the works it
/// knows about rather than to 404 on the first unknown name.
pub async fn get_entities(
    db: &Database,
    wanted: &[(String, String)],
) -> Result<Vec<CanonicalEntity>> {
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    #[derive(sqlx::FromRow)]
    struct Row {
        kind: String,
        norm: String,
        canonical: String,
        review_status: String,
        signal_count: i64,
        curated_by: Option<String>,
        curated_at: Option<String>,
    }
    let mut out = Vec::new();
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            for (kind, norm) in wanted {
                let sql = "SELECT kind, norm, canonical, review_status, signal_count, curated_by, curated_at FROM canonical_entities WHERE kind = ? AND norm = ?";
                if let Some(r) = sqlx::query_as::<_, Row>(sql)
                    .bind(kind)
                    .bind(norm)
                    .fetch_optional(pool)
                    .await?
                {
                    out.push(CanonicalEntity {
                        kind: r.kind,
                        norm: r.norm,
                        canonical: r.canonical,
                        review_status: r.review_status,
                        signal_count: r.signal_count,
                        curated_by: r.curated_by,
                        curated_at: r.curated_at,
                    });
                }
            }
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            for (kind, norm) in wanted {
                let sql = "SELECT kind, norm, canonical, review_status, signal_count, curated_by, curated_at FROM canonical_entities WHERE kind = $1 AND norm = $2";
                if let Some(r) = sqlx::query_as::<_, Row>(sql)
                    .bind(kind)
                    .bind(norm)
                    .fetch_optional(pool)
                    .await?
                {
                    out.push(CanonicalEntity {
                        kind: r.kind,
                        norm: r.norm,
                        canonical: r.canonical,
                        review_status: r.review_status,
                        signal_count: r.signal_count,
                        curated_by: r.curated_by,
                        curated_at: r.curated_at,
                    });
                }
            }
        }
    }
    Ok(out)
}

/// The review queue (§15.17): unverified entities, most-signal-counted first.
///
/// The ordering is the whole point of the function, so it lives in the ORDER BY
/// rather than being sorted in Rust — the `canonical_entities_queue` index is
/// defined in that order, and a Rust sort over a full table read would be both
/// slower and would not use it.
pub async fn list_review_queue(db: &Database, limit: i64) -> Result<Vec<CanonicalEntity>> {
    #[derive(sqlx::FromRow)]
    struct Row {
        kind: String,
        norm: String,
        canonical: String,
        review_status: String,
        signal_count: i64,
        curated_by: Option<String>,
        curated_at: Option<String>,
    }
    let sql = db.sql(
        "SELECT kind, norm, canonical, review_status, signal_count, curated_by, curated_at
         FROM canonical_entities WHERE review_status = 'unverified'
         ORDER BY signal_count DESC, norm ASC
         LIMIT ?",
        "SELECT kind, norm, canonical, review_status, signal_count, curated_by, curated_at
         FROM canonical_entities WHERE review_status = 'unverified'
         ORDER BY signal_count DESC, norm ASC
         LIMIT $1",
    );
    let rows: Vec<Row> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(|r| CanonicalEntity {
            kind: r.kind,
            norm: r.norm,
            canonical: r.canonical,
            review_status: r.review_status,
            signal_count: r.signal_count,
            curated_by: r.curated_by,
            curated_at: r.curated_at,
        })
        .collect())
}

/// Curate an entity (§19.4 quorum work, gated to TL3 by the caller).
///
/// Sets the canonical form and marks it curated. The originating signals are
/// untouched: §15.17 requires them retained as provenance, never rewritten, and
/// this statement deliberately does not touch `exchange_signals` or
/// `exchange_signal_entities`.
pub async fn curate_entity(
    db: &Database,
    kind: &str,
    norm: &str,
    canonical_form: &str,
    curator: &str,
) -> Result<bool> {
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "UPDATE canonical_entities
         SET canonical = ?, review_status = 'curated', curated_by = ?, curated_at = ?, updated_at = ?
         WHERE kind = ? AND norm = ?",
        "UPDATE canonical_entities
         SET canonical = $1, review_status = 'curated', curated_by = $2, curated_at = $3, updated_at = $4
         WHERE kind = $5 AND norm = $6",
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(canonical_form)
            .bind(curator)
            .bind(&now)
            .bind(&now)
            .bind(kind)
            .bind(norm)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(canonical_form)
            .bind(curator)
            .bind(&now)
            .bind(&now)
            .bind(kind)
            .bind(norm)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// Create or reinforce a latent-demand item (§16.16.1).
///
/// Once per work per submitting instance: the primary key is
/// `(work_id, source_instance)`, so a second signal from the same instance for
/// the same work updates the existing row rather than adding a vote. That is
/// the difference between "a reader on a sister instance cannot find this" —
/// genuine, additive demand — and "forty submitters", which §16.16.1 forbids
/// reading as a headcount.
pub async fn reinforce_latent_demand(
    db: &Database,
    work_id: &str,
    source_instance: &str,
) -> Result<()> {
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO exchange_latent_demand (work_id, source_instance, reinforced_at, signal_count, created_at)
         VALUES (?, ?, ?, 1, ?)
         ON CONFLICT (work_id, source_instance) DO UPDATE SET
             reinforced_at = excluded.reinforced_at,
             signal_count = exchange_latent_demand.signal_count + 1",
        "INSERT INTO exchange_latent_demand (work_id, source_instance, reinforced_at, signal_count, created_at)
         VALUES ($1, $2, $3, 1, $4)
         ON CONFLICT (work_id, source_instance) DO UPDATE SET
             reinforced_at = EXCLUDED.reinforced_at,
             signal_count = exchange_latent_demand.signal_count + 1",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(work_id)
                .bind(source_instance)
                .bind(&now)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(work_id)
                .bind(source_instance)
                .bind(&now)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Fetch latent-demand items, most-recently-reinforced first.
pub async fn list_latent_demand(db: &Database, limit: i64) -> Result<Vec<LatentDemand>> {
    #[derive(sqlx::FromRow)]
    struct Row {
        work_id: String,
        source_instance: String,
        signal_count: i64,
        reinforced_at: String,
    }
    let sql = db.sql(
        "SELECT work_id, source_instance, signal_count, reinforced_at
         FROM exchange_latent_demand ORDER BY reinforced_at DESC LIMIT ?",
        "SELECT work_id, source_instance, signal_count, reinforced_at
         FROM exchange_latent_demand ORDER BY reinforced_at DESC LIMIT $1",
    );
    let rows: Vec<Row> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(|r| LatentDemand {
            work_id: r.work_id,
            source_instance: r.source_instance,
            signal_count: r.signal_count,
            reinforced_at: r.reinforced_at,
        })
        .collect())
}

/// Count the distinct signals retained for a work — provenance, not a headcount.
///
/// Named to make the distinction unmissable at the call site. There is
/// deliberately no `count_holders` anywhere in this module: §11.17 says a count
/// of holders is not computable from the data the instance holds, and a function
/// that returned something plausible under that name would be the exact defect
/// the spec warns about.
pub async fn count_provenance_signals(db: &Database, work_id: &str) -> Result<i64> {
    let sql = db.sql(
        "SELECT COUNT(*) FROM exchange_signals WHERE work_id = ?",
        "SELECT CAST(COUNT(*) AS BIGINT) FROM exchange_signals WHERE work_id = $1",
    );
    let n: i64 = match db.backend() {
        Backend::Sqlite => {
            // `&sql` rather than `sql`: `db.sql` returns a `Cow`, and a bare
            // `sql` would move it in the first arm and leave the second arm with
            // nothing. Clippy's `needless_borrow` fires on the arms where the
            // value is a plain `String`, which is why this is written out
            // per-arm rather than hoisted.
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(n)
}
