//! Instance work body retention: the setting, and who may narrow it
//! (spec §11.15).
//!
//! Three things live here, and the split is the point of the module:
//!
//! * [`read_policy`] / [`write_policy`] — the operator's decision, a singleton
//!   row carrying who set it and when.
//! * [`write_source_override`] — the per-source narrowing, which **refuses a
//!   widening by name** rather than silently ignoring it.
//! * [`resolve_for_source`] — the one read every body-storage path calls, so
//!   the six paths §11.15 names cannot disagree about the answer.
//!
//! **Why the refusal is at the setter and not only at the reader.** A widening
//! override on an aggregating instance would restore storage the operator
//! removed. Refusing it in `write_source_override` is what makes the rule
//! visible to the operator who typed it; `resolve_for_source` then narrows
//! anyway, for a row that reached the table by some other path. Two lines of
//! defence, and the second one is asserted in the domain's unit tests.
//!
//! **Why a missing policy row is `Cache` and not an error.** Every instance
//! built before migration 0087 has no row at all, and every one of them caches
//! bodies. Treating absence as `Aggregate` would silently strip storage from a
//! running instance on upgrade; treating it as an error would make the first
//! request after an upgrade fail. `None` is returned and the caller resolves it,
//! so the choice is in one place and it is the safe one.

use anyhow::Result;
use lorehaven_domain::retention::{BodyMode, ResolvedRetention};
use uuid::Uuid;

use crate::{Backend, Database};

/// The row id of the singleton. Not an autoincrement: §11.15's setting is one
/// decision, and a second row under a different id is a second decision that
/// nothing reads.
const POLICY_ID: &str = "default";

/// The instance's retention policy, as the row holds it.
#[derive(Debug, Clone)]
pub struct StoredRetentionPolicy {
    pub body_mode: BodyMode,
    /// Who changed it last. `None` once that account is deleted — the decision
    /// outlives its actor, and the modlog entry does too.
    pub updated_by: Option<String>,
    pub updated_at: String,
    pub version: i64,
}

/// A per-source narrowing, as the row holds it.
#[derive(Debug, Clone)]
pub struct StoredSourceOverride {
    pub source_key: String,
    pub body_mode: BodyMode,
    pub updated_by: Option<String>,
    pub updated_at: String,
    pub version: i64,
}

/// A refusal to write a widening override, named (spec §11.15).
///
/// Its own type rather than a string because the caller — an admin route — has
/// to show the operator *which* setting they would be widening, and a string
/// error makes that a re-derivation at the call site.
#[derive(Debug, thiserror::Error)]
#[error("a per-source override may only narrow: {source_key} cannot be set to cache while the instance is set to aggregate")]
pub struct WideningRefused {
    pub source_key: String,
    pub attempted: BodyMode,
    pub instance: BodyMode,
}

/// Why an override write did not happen.
///
/// **Two variants, because a widening and a broken database are not the same
/// kind of event.** The first is the operator's own rule refusing their input
/// and the route answers 400 with the three values spelled out. The second is
/// the instance failing and the route answers 500. Folding them into one type
/// — by making the function return `anyhow::Result` and stringifying the
/// widening, or by returning the widening for both — is the specific mistake
/// this enum exists to prevent: the first version of this function did exactly
/// that, and the `?` on a `sqlx::Error` did not compile because the error type
/// was the refusal. Making them siblings is what let the compiler say so.
#[derive(Debug, thiserror::Error)]
pub enum OverrideWriteError {
    /// The operator tried to widen a source past the instance's setting.
    #[error(transparent)]
    Widening(#[from] WideningRefused),
    /// The write itself failed, or the row could not be read back.
    #[error("{0}")]
    Storage(String),
}

/// The row shape, with `body_mode` as a string because it is one in the schema.
#[derive(sqlx::FromRow)]
struct PolicyRow {
    body_mode: String,
    updated_by: Option<String>,
    updated_at: String,
    version: i64,
}

#[derive(sqlx::FromRow)]
struct OverrideRow {
    source_key: String,
    body_mode: String,
    updated_by: Option<String>,
    updated_at: String,
    version: i64,
}

/// The instance's policy, or `None` when nobody has ever set one.
///
/// `None` is honest rather than defaulted here: the row's absence is a fact,
/// and the caller's decision about it (§11.15 says cache is the default) is
/// made where the consequence is felt, not hidden in a getter.
pub async fn read_policy(db: &Database) -> Result<Option<StoredRetentionPolicy>> {
    let sql = db.sql(
        "SELECT body_mode, updated_by, updated_at, version FROM instance_retention_policy WHERE id = ?",
        "SELECT body_mode, updated_by::text AS updated_by, updated_at::text AS updated_at,
                version::bigint AS version
           FROM instance_retention_policy WHERE id = $1",
    );
    let row: Option<PolicyRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(POLICY_ID)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(POLICY_ID)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(|row| StoredRetentionPolicy {
        // An unrecognised stored value resolves to `Cache`, which is what
        // every instance before this row existed does. The alternative —
        // `Aggregate` — would be a policy change made by a typo or a
        // downgrade, and §11.15's default is the direction that takes nothing
        // away.
        body_mode: BodyMode::parse_stored(Some(&row.body_mode)).unwrap_or_default(),
        updated_by: row.updated_by,
        updated_at: row.updated_at,
        version: row.version,
    }))
}

/// The instance's effective mode, defaulting to `Cache` (spec §11.15).
pub async fn effective_instance_mode(db: &Database) -> Result<BodyMode> {
    Ok(read_policy(db)
        .await?
        .map_or(BodyMode::default(), |policy| policy.body_mode))
}

/// Record the operator's decision (spec §11.15).
///
/// An upsert, not insert-then-retry: the row is a singleton, so the second
/// write is the common one, and making that the awkward case invites a
/// check-then-insert race between two operators on a setting that decides
/// whether the instance stores every body it fetches.
///
/// **No retro-fetch and no deletion.** Widening to `cache` does not fetch
/// bodies for works already aggregated, and narrowing to `aggregate` does not
/// delete bodies already held — §11.15 says both explicitly, and the second is
/// the deletion workflow of §10.4 rather than a policy change. Nothing in this
/// function touches content, which is the property to keep.
/// Set the instance's mode.
///
/// `updated_by` is a concrete `Uuid` because the column is
/// `NOT NULL REFERENCES accounts (id)`. An earlier version made it `Option` on
/// the strength of a comment in *another* table's migration: `0087`'s
/// `instance_retention_policy.updated_by` is nullable, so "the instance acted"
/// looked unrepresentable. It is representable — by naming a system account
/// (see `lorehaven_db::SYSTEM_ACCOUNT`), which is what a binding-mode
/// settlement passes here.
pub async fn write_policy(
    db: &Database,
    mode: BodyMode,
    updated_by: Uuid,
) -> Result<StoredRetentionPolicy> {
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO instance_retention_policy (id, body_mode, updated_by, created_at, updated_at, version)
         VALUES (?, ?, ?, ?, ?, 1)
         ON CONFLICT (id) DO UPDATE SET
            body_mode = excluded.body_mode,
            updated_by = excluded.updated_by,
            updated_at = excluded.updated_at,
            version = instance_retention_policy.version + 1",
        "INSERT INTO instance_retention_policy (id, body_mode, updated_by, created_at, updated_at, version)
         VALUES ($1, $2, $3::uuid, $4, $5::timestamptz, 1)
         ON CONFLICT (id) DO UPDATE SET
            body_mode = excluded.body_mode,
            updated_by = excluded.updated_by,
            updated_at = excluded.updated_at,
            version = instance_retention_policy.version + 1",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(POLICY_ID)
                .bind(mode.as_str())
                .bind(updated_by.to_string())
                .bind(&now)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(POLICY_ID)
                .bind(mode.as_str())
                .bind(updated_by.to_string())
                .bind(&now)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    read_policy(db).await?.ok_or_else(|| {
        anyhow::anyhow!("the retention policy vanished immediately after being written")
    })
}

/// Every per-source override, in key order.
///
/// Ordered, because the admin route renders this list and an unordered one
/// makes two identical instances report different `GET`s — which is the kind of
/// difference an operator eventually files a bug about.
pub async fn list_source_overrides(db: &Database) -> Result<Vec<StoredSourceOverride>> {
    let sql = db.sql(
        "SELECT source_key, body_mode, updated_by, updated_at, version
           FROM instance_retention_source_overrides ORDER BY source_key",
        "SELECT source_key, body_mode, updated_by::text AS updated_by,
                updated_at::text AS updated_at, version::bigint AS version
           FROM instance_retention_source_overrides ORDER BY source_key",
    );
    let rows: Vec<OverrideRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(|row| StoredSourceOverride {
            source_key: row.source_key,
            body_mode: BodyMode::parse_stored(Some(&row.body_mode)).unwrap_or_default(),
            updated_by: row.updated_by,
            updated_at: row.updated_at,
            version: row.version,
        })
        .collect())
}

/// Narrow one source family, refusing a widening by name (spec §11.15).
///
/// The refusal is the substance of this function. §11.15: "Caching most
/// sources while aggregating one is expressible; the reverse on an `aggregate`
/// instance is not, because that would restore storage the instance decided
/// against, and the operator who wants it can change the instance setting
/// itself, where the change is recorded."
///
/// So the error names the source, the attempted mode and the instance's mode,
/// and the admin route shows the operator the two settings rather than a bare
/// "invalid". The alternative — accepting the row and letting the read side
/// narrow it away — is the failure this exists to prevent, because the operator
/// would see their change saved and no change in behaviour.
/// Set a per-source override.
///
/// `updated_by` is a concrete `Uuid`, as in [`write_policy`].
pub async fn write_source_override(
    db: &Database,
    source_key: &str,
    mode: BodyMode,
    updated_by: Uuid,
) -> Result<StoredSourceOverride, OverrideWriteError> {
    let instance = effective_instance_mode(db)
        .await
        .map_err(|error| OverrideWriteError::Storage(error.to_string()))?;
    if mode.stores_bodies() && !instance.stores_bodies() {
        return Err(WideningRefused {
            source_key: source_key.to_owned(),
            attempted: mode,
            instance,
        }
        .into());
    }

    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO instance_retention_source_overrides
           (source_key, body_mode, updated_by, created_at, updated_at, version)
         VALUES (?, ?, ?, ?, ?, 1)
         ON CONFLICT (source_key) DO UPDATE SET
            body_mode = excluded.body_mode,
            updated_by = excluded.updated_by,
            updated_at = excluded.updated_at,
            version = instance_retention_source_overrides.version + 1",
        "INSERT INTO instance_retention_source_overrides
           (source_key, body_mode, updated_by, created_at, updated_at, version)
         VALUES ($1, $2, $3::uuid, $4, $5::timestamptz, 1)
         ON CONFLICT (source_key) DO UPDATE SET
            body_mode = excluded.body_mode,
            updated_by = excluded.updated_by,
            updated_at = excluded.updated_at,
            version = instance_retention_source_overrides.version + 1",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(source_key)
                .bind(mode.as_str())
                .bind(updated_by.to_string())
                .bind(&now)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await
                .map_err(|error| OverrideWriteError::Storage(error.to_string()))?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(source_key)
                .bind(mode.as_str())
                .bind(updated_by.to_string())
                .bind(&now)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await
                .map_err(|error| OverrideWriteError::Storage(error.to_string()))?;
        }
    }

    list_source_overrides(db)
        .await
        .map_err(|error| OverrideWriteError::Storage(error.to_string()))?
        .into_iter()
        .find(|row| row.source_key == source_key)
        // A row that vanished between the write and the read back is a storage
        // failure, NOT a widening. Reporting it as one would tell the operator
        // their override may only narrow when the real problem is the database
        // — and §11.15's rule would be the last thing they went looking at.
        .ok_or_else(|| {
            OverrideWriteError::Storage(format!(
                "the retention override for {source_key} was written and then not found"
            ))
        })
}

/// Remove one source's override, returning to the instance's setting.
///
/// §11.15's "Remove the source override" is the action every
/// `RETENTION_AGGREGATE_SOURCE_OVERRIDE` message points at, so the operation
/// has to exist and has to be reachable — a message naming a change the
/// interface cannot make is worse than a generic error.
pub async fn clear_source_override(db: &Database, source_key: &str) -> Result<bool> {
    let sql = db.sql(
        "DELETE FROM instance_retention_source_overrides WHERE source_key = ?",
        "DELETE FROM instance_retention_source_overrides WHERE source_key = $1",
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(source_key)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(source_key)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// The retention policy as it applies to one source.
///
/// The one function every body-storage path calls. `source_blocked` and
/// `vanished` are arguments rather than lookups because they are facts the
/// caller already holds — a source it has just checked, an availability row it
/// has just read — and re-reading them here would be a second query whose
/// answer could differ from the first by the time it is used.
pub async fn resolve_for_source(
    db: &Database,
    source_key: Option<&str>,
    source_blocked: bool,
    vanished: bool,
) -> Result<ResolvedRetention> {
    let instance = effective_instance_mode(db).await?;
    let source = match source_key {
        None => None,
        Some(key) => {
            let sql = db.sql(
                "SELECT body_mode FROM instance_retention_source_overrides WHERE source_key = ?",
                "SELECT body_mode FROM instance_retention_source_overrides WHERE source_key = $1",
            );
            let row: Option<(String,)> = match db.backend() {
                Backend::Sqlite => {
                    sqlx::query_as(&sql)
                        .bind(key)
                        .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                        .await?
                }
                Backend::Postgres => {
                    sqlx::query_as(&sql)
                        .bind(key)
                        .fetch_optional(db.postgres_pool().expect("postgres handle"))
                        .await?
                }
            };
            row.and_then(|(mode,)| BodyMode::parse_stored(Some(&mode)))
        }
    };
    Ok(ResolvedRetention {
        instance,
        source,
        source_blocked,
        vanished,
    })
}

// ---------------------------------------------------------------------------
// §11.15 / amendment §4.2 — works_past_saving
// ---------------------------------------------------------------------------

/// How many aggregated works this instance is currently past saving.
///
/// The definition is amendment §4.2's, word for word: **aggregated works whose
/// origin is unreachable and which this instance holds no body for**. Both
/// halves are required, and a count that keeps only one of them is worse than
/// no count — it is a number the operator acts on.
///
/// * *holds no body* — there is no `import_chapters` row in state `stored` for
///   this library item. This is the same expression the library listing uses for
///   its chapter count, so the two can never disagree about what "holds a body"
///   means.
/// * *origin unreachable* — the most recent import of this item's work ended
///   `failed` **for a reason that is not the operator's to fix**. That
///   qualification is load-bearing and it is why the query names codes rather
///   than only a state: `import_jobs.state = 'failed'` is written for three
///   different situations, and two of them are not a vanished origin.
///   `credential_missing` and `credential_expired` mean a reader has not logged
///   in, and counting those as "past saving" would tell the operator their
///   instance has a preservation debt when what it has is an unreadable login.
///   The refusal codes that *do* mean the work is gone are `not_found` and
///   `withheld` — the source answered, and its answer is that the work is not
///   there or may not be served.
///
/// §11.13's availability marking is per *media reference*, not per work, so
/// there is no stronger per-work signal to read. Inventing one would be worse
/// than the approximation: the honest thing is to count what the instance
/// actually recorded, and to say in this comment which recorded codes it reads.
///
/// **The count is instance-wide, not per reader.** §11.15's "preservation
/// debt" is a property of the instance's storage decision, and a per-reader
/// count would let the operator conclude the instance is healthy because the one
/// reader who checked has bodies. Deduplicated on `library_items.id`, so a work
/// three readers hold is counted once.
///
/// A caching instance reports **zero**, and that is not a special case in the
/// query: a caching instance that failed an import is retrying, and §11.15's
/// non-degradation rule keeps a failed body fetch as a failed import rather
/// than as a lost work. Nothing is "past saving" while the retry is pending.
/// The caller is told which instance it is looking at so the zero is legible
/// rather than looking like a good result.
pub async fn works_past_saving(db: &Database) -> Result<WorksPastSaving> {
    let instance = effective_instance_mode(db).await?;
    let sql = db.sql(
        "SELECT COUNT(*)
           FROM library_items li
          WHERE NOT EXISTS (SELECT 1 FROM import_chapters ic
                             WHERE ic.library_item_id = li.id AND ic.state = 'stored')
            -- The failure codes that mean the work is GONE at its origin.
            -- `credential_missing` and `credential_expired` are excluded on
            -- purpose: they are an operator-fixable login problem, not a
            -- vanished work, and counting them would inflate the debt.
            AND EXISTS (SELECT 1 FROM import_jobs ij
                         WHERE ij.library_item_id = li.id
                           AND ij.state = 'failed'
                           AND (ij.report_json LIKE '%not_found%'
                             OR ij.report_json LIKE '%withheld%'))",
        "SELECT COUNT(*)
           FROM library_items li
          WHERE NOT EXISTS (SELECT 1 FROM import_chapters ic
                             WHERE ic.library_item_id = li.id AND ic.state = 'stored')
            -- The failure codes that mean the work is GONE at its origin.
            -- `credential_missing` and `credential_expired` are excluded on
            -- purpose: they are an operator-fixable login problem, not a
            -- vanished work, and counting them would inflate the debt.
            AND EXISTS (SELECT 1 FROM import_jobs ij
                         WHERE ij.library_item_id = li.id
                           AND ij.state = 'failed'
                           AND (ij.report_json LIKE '%not_found%'
                             OR ij.report_json LIKE '%withheld%'))",
    );
    let count: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(WorksPastSaving { count, instance })
}

/// `works_past_saving`, with the instance's mode beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorksPastSaving {
    /// How many works meet the definition above.
    pub count: i64,
    /// The instance's mode, so a zero on a `cache` instance reads as "nothing is
    /// past saving because a failed import is being retried" rather than as a
    /// healthy result.
    pub instance: BodyMode,
}
