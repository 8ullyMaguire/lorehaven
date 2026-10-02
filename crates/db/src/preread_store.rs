//! Persisting and reading pre-read reports (gap C step 4).
//!
//! [`PreReadReport`] in `lorehaven-domain` is the value; this is where it lives. The
//! schema is migration 0111 and the reasoning for its shape — one row per
//! (work, provider, task), JSONB dimensions, and **no composite score column** — is in
//! that migration's header. The short version is repeated here because it is the rule a
//! future caller is most likely to break:
//!
//! **Nothing here computes or stores a single number for a work.** §32.6 forbids
//! displaying composite quality scores publicly, and §0.3 forbids credit, payment or
//! trust level moving any ranking signal. A `score` column would be a ranking signal one
//! query away from being wired up. What is stored is per-dimension, and
//! [`save_report`] takes a [`PreReadReport`] — a type that has no way to express a
//! composite — rather than loose scores it could be handed.

use lorehaven_domain::ai::{AiAbstain, AiTask};
use lorehaven_domain::preread::{DimensionOutcome, DimensionStatus, PreReadReport};
use serde_json::Value;

use crate::{Backend, Database, Result};

/// Save a report, replacing any existing one for the same (work, provider, task).
///
/// An UPSERT rather than an append, and that is the whole retention story: a report is the
/// current assessment, not a log. §23.7 gives no retention policy for AI output, so
/// accumulating one row per run would mean a table that grows forever with content the
/// operator cannot delete. Re-scoring overwrites.
///
/// Replacing means `updated_at` and `scored_at` both move, so "when was this assessed"
/// and "when did we last hear about it" cannot drift apart silently.
pub async fn save_report(
    db: &Database,
    report: &PreReadReport,
    provider: &str,
    scored_at: &str,
) -> Result<()> {
    let dimensions = serde_json::to_string(
        &report
            .dimensions
            .values()
            .map(|d| {
                (
                    d.dimension.clone(),
                    serde_json::json!({ "score": d.score, "note": d.note }),
                )
            })
            .collect::<serde_json::Map<_, _>>(),
    )
    .expect("a map of strings is serialisable");
    let missing = serde_json::to_string(&encode_missing(&report.missing))
        .expect("missing statuses are serialisable");
    let task = task_name(AiTask::PreReadScoring);
    let now = scored_at;

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO preread_reports (id, work_id, provider, task, dimensions, missing, scored_at, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?7) ON CONFLICT (work_id, provider, task) DO UPDATE SET dimensions = excluded.dimensions, missing = excluded.missing, scored_at = excluded.scored_at, updated_at = excluded.updated_at",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(&report.work_id)
            .bind(provider)
            .bind(&task)
            .bind(&dimensions)
            .bind(&missing)
            .bind(now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            // `work_id` is UUID here and TEXT on SQLite, so the bind is native rather
            // than the `::uuid` cast dance the older stores use. The JSON columns take
            // the JSON directly rather than a string, because `JSONB` would otherwise
            // store the quotes as part of the value.
            sqlx::query(
                "INSERT INTO preread_reports (id, work_id, provider, task, dimensions, missing, scored_at, created_at, updated_at) VALUES ($1, $2, $3, $4, $5::jsonb, $6::jsonb, $7, $7, $7) ON CONFLICT (work_id, provider, task) DO UPDATE SET dimensions = excluded.dimensions, missing = excluded.missing, scored_at = excluded.scored_at, updated_at = excluded.updated_at",
            )
            .bind(uuid::Uuid::new_v4())
            .bind(uuid::Uuid::parse_str(&report.work_id).map_err(|e| anyhow::anyhow!("work_id is not a uuid: {e}"))?)
            .bind(provider)
            .bind(&task)
            .bind(&dimensions)
            .bind(&missing)
            .bind(now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(())
}

/// The current report for a work, if there is one.
///
/// `None` is the common case and not an error: most works are never pre-scored, because
/// the author has to ask for it (§23.7 — no automatic publication, and a score is
/// something an author opts into).
pub async fn report_for(
    db: &Database,
    work_id: &str,
    provider: &str,
) -> Result<Option<PreReadReport>> {
    // The two arms return different Rust types because SQLite has no JSONB: it stores the
    // JSON as TEXT, so the driver cannot decode it into a `Value`. Both arms hand back the
    // raw JSON text and parse it here, which keeps the decode in one place rather than
    // spread across a per-backend type difference.
    let row: Option<(String, String, String)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(
                "SELECT work_id, dimensions, missing FROM preread_reports \
                     WHERE work_id = ?1 AND provider = ?2 \
                     ORDER BY scored_at DESC LIMIT 1",
            )
            .bind(work_id)
            .bind(provider)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?
        }
        Backend::Postgres => sqlx::query_as::<_, (String, serde_json::Value, serde_json::Value)>(
            "SELECT work_id::text, dimensions, missing \
                     FROM preread_reports \
                     WHERE work_id = $1 AND provider = $2 \
                     ORDER BY scored_at DESC LIMIT 1",
        )
        .bind(work_uuid(work_id)?)
        .bind(provider)
        .fetch_optional(db.postgres_pool().expect("postgres"))
        .await?
        .map(|(w, d, m)| (w, d.to_string(), m.to_string())),
    };

    Ok(row.map(|(work_id, dimensions, missing)| PreReadReport {
        work_id,
        dimensions: decode_dimensions(&parse_json(&dimensions)),
        missing: decode_missing(&parse_json(&missing)),
    }))
}

/// The stable stored name of an abstain.
///
/// Only the variants worth distinguishing to a caller are named; anything else reads back
/// as [`AiAbstain::NotConfigured`], which is the honest "some provider declined" rather
/// than a variant invented from a string. The point is that a caller can branch on the
/// *fact* of abstention without this being a decode that can fail.
fn abstain_name(reason: &AiAbstain) -> &'static str {
    match reason {
        AiAbstain::NoConsent => "no_consent",
        AiAbstain::NotConfigured => "not_configured",
        AiAbstain::InvalidOutput(_) => "invalid_output",
        _ => "other",
    }
}

fn abstain_from_name(name: Option<&str>) -> DimensionStatus {
    match name {
        Some("no_consent") => DimensionStatus::Abstained(AiAbstain::NoConsent),
        Some("not_configured") => DimensionStatus::Abstained(AiAbstain::NotConfigured),
        Some("invalid_output") => DimensionStatus::Abstained(AiAbstain::InvalidOutput(
            "the provider returned output that could not be validated".to_string(),
        )),
        _ => DimensionStatus::Abstained(AiAbstain::NotConfigured),
    }
}

/// Parse stored JSON, treating anything unparseable as empty.
///
/// A corrupt row must not make a work's page fail to load. Empty is the honest reading:
/// no dimensions scored, which the report already renders as "nothing came back".
fn parse_json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Object(serde_json::Map::new()))
}

/// Every provider that has a report for this work.
///
/// §23.7 lets a reader opt out of specific providers, so "withdraw consent from this one"
/// has to mean *this one* and not "delete everything this provider said about me". This
/// is what an author-facing consent screen iterates over.
pub async fn providers_for(db: &Database, work_id: &str) -> Result<Vec<String>> {
    let providers: Vec<String> = match db.backend() {
        Backend::Sqlite => sqlx::query_scalar(
            "SELECT DISTINCT provider FROM preread_reports WHERE work_id = ?1 ORDER BY provider",
        )
        .bind(work_id)
        .fetch_all(db.sqlite_pool().expect("sqlite"))
        .await?,
        Backend::Postgres => sqlx::query_scalar(
            "SELECT DISTINCT provider FROM preread_reports WHERE work_id = $1 ORDER BY provider",
        )
        .bind(
            uuid::Uuid::parse_str(work_id)
                .map_err(|e| anyhow::anyhow!("work_id is not a uuid: {e}"))?,
        )
        .fetch_all(db.postgres_pool().expect("postgres"))
        .await?,
    };
    Ok(providers)
}

/// Delete every report from one provider for one work.
///
/// The withdrawal action. `providers_for` is what an author sees before calling this, and
/// this is the only way a report leaves the database besides the work itself — which is
/// why there is no DELETE-by-work route and why the ON DELETE CASCADE in migration 0111
/// is the work's business, not this provider's.
pub async fn forget_provider(db: &Database, work_id: &str, provider: &str) -> Result<u64> {
    let affected = match db.backend() {
        Backend::Sqlite => {
            sqlx::query("DELETE FROM preread_reports WHERE work_id = ?1 AND provider = ?2")
                .bind(work_id)
                .bind(provider)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?
                .rows_affected()
        }
        Backend::Postgres => {
            sqlx::query("DELETE FROM preread_reports WHERE work_id = $1 AND provider = $2")
                .bind(
                    uuid::Uuid::parse_str(work_id)
                        .map_err(|e| anyhow::anyhow!("work_id is not a uuid: {e}"))?,
                )
                .bind(provider)
                .execute(db.postgres_pool().expect("postgres"))
                .await?
                .rows_affected()
        }
    };
    Ok(affected)
}

/// A work id as a native uuid, for the PostgreSQL arm only.
///
/// `work_id` is TEXT on SQLite and uuid on PostgreSQL — the sixth site in this codebase
/// with that split. A malformed id is a caller error rather than an operational failure,
/// so it is an `anyhow` error rather than something a UI could render.
fn work_uuid(work_id: &str) -> Result<uuid::Uuid> {
    uuid::Uuid::parse_str(work_id).map_err(|e| anyhow::anyhow!("work_id is not a uuid: {e}"))
}

/// `missing` as JSON.
///
/// `Abstained` carries the provider's reason, which is the difference between "not
/// configured" and "the provider returned something unparseable" — two different facts to
/// debug a week later, so both are persisted.
fn encode_missing(missing: &std::collections::BTreeMap<String, DimensionStatus>) -> Value {
    let mut out = serde_json::Map::new();
    for (dimension, status) in missing {
        out.insert(
            dimension.clone(),
            match status {
                DimensionStatus::Scored => serde_json::json!("scored"),
                DimensionStatus::NotConfigured => serde_json::json!("not_configured"),
                // The variant *name*, not the variant. An `AiAbstain` carries data — a
                // provider's error text, a timeout message — and persisting that as a type
                // tag would both leak provider detail into the report and require
                // re-parsing a Debug dump to read back. The name is the whole fact a caller
                // branches on.
                DimensionStatus::Abstained(reason) => serde_json::json!({
                    "abstained": abstain_name(reason),
                }),
            },
        );
    }
    Value::Object(out)
}

fn decode_missing(value: &Value) -> std::collections::BTreeMap<String, DimensionStatus> {
    let mut out = std::collections::BTreeMap::new();
    if let Some(map) = value.as_object() {
        for (dimension, status) in map {
            // Decoding is deliberately lenient about the *reason* and strict about the
            // fact of absence. A stored reason this build does not recognise still means
            // "not answered", which is the only thing a caller branches on; inventing a
            // specific reason would be worse than saying "some provider declined".
            let parsed = match status.as_str() {
                Some("scored") => Some(DimensionStatus::Scored),
                Some("not_configured") => Some(DimensionStatus::NotConfigured),
                _ => Some(abstain_from_name(
                    status.get("abstained").and_then(Value::as_str),
                )),
            };
            if let Some(parsed) = parsed {
                out.insert(dimension.clone(), parsed);
            }
        }
    }
    out
}

fn decode_dimensions(value: &Value) -> std::collections::BTreeMap<String, DimensionOutcome> {
    let mut out = std::collections::BTreeMap::new();
    if let Some(map) = value.as_object() {
        for (dimension, entry) in map {
            // A missing or non-numeric score makes the whole dimension absent rather than
            // a default of 0.0. A stored zero is a claim ("this scored zero"); a stored
            // zero because the JSON was malformed is a lie, and the difference matters
            // because the value is shown to an author making a decision about their work.
            let Some(score) = entry.get("score").and_then(Value::as_f64) else {
                continue;
            };
            if !(0.0..=1.0).contains(&score) {
                continue;
            }
            out.insert(
                dimension.clone(),
                DimensionOutcome {
                    dimension: dimension.clone(),
                    score,
                    note: entry
                        .get("note")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                },
            );
        }
    }
    out
}

/// The stored spelling of a task.
///
/// `serde_json::to_string` of a unit-variant enum variant produces `"PreReadScoring"`, and
/// the column is `TEXT`; `snake_case` is what the rest of the schema uses, so the
/// conversion lives in one function rather than being re-derived per call site.
fn task_name(task: AiTask) -> String {
    serde_json::to_value(task)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "unknown".to_string())
        .replace("PreReadScoring", "pre_read_scoring")
}
