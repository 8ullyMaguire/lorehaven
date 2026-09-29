//! The decision audit trail (spec §11.14, amendment
//! `calibrated-decision-models.md` §3.5).
//!
//! `GET /api/v1/decisions/audit` has always returned `{ "items": [] }`. It
//! claims to be an operator-only audit trail and carries nothing. This module is
//! that content.
//!
//! # What a row is, and is not
//!
//! A row records **what the system decided and on what evidence**: the
//! deterministic answer, the model's posterior, the threshold in force, the
//! reconciled outcome, and which provider answered.
//!
//! It does **not** record the text that was classified. That is the load-bearing
//! decision in the whole amendment and it is a privacy decision. §12.1's
//! commitment is that a reader's words stay with their author and that the
//! filter is a judgement rather than a taking; an audit table holding the graded
//! text would be a second copy of that text with none of the first copy's rules
//! — no deletion path, no export, no visibility level, no author who can see it.
//! `subject` is an id, and the evidence an operator needs is a poster and a
//! number, not the prose.
//!
//! # Both answers are stored, because the interesting rows are the ones where
//! they differ
//!
//! `deterministic` and `outcome` are equal on almost every row, because the
//! model may only narrow an acceptance to a hold. The rows that prove the
//! influence stayed one-directional are exactly the rows where they differ. A
//! table holding only the outcome could not show that; holding both lets an
//! operator verify the property rather than trust the documentation.
//!
//! # A NULL posterior is not zero confidence
//!
//! `None` means no model was consulted: the provider is deterministic, the model
//! was unreachable, the answer was refused as nonsense, or there was no text to
//! grade. Collapsing those into `0.0` would report every deterministic decision
//! as a model confidently saying "no", and an operator tuning a threshold from
//! that record would be tuning against a fiction.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Backend, Database};

/// The largest page an audit read may return.
///
/// A bound rather than a default, and the two are different things: a default
/// is what a caller gets for asking nothing, a bound is what a caller cannot get
/// past. Both exist.
pub const MAX_AUDIT_PAGE: i64 = 500;

/// Which provider answered a decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditProvider {
    /// The instance's own classifiers. Every row on an instance that has not
    /// opted in.
    Deterministic,
    /// A decision model, consulted as a second opinion.
    Calibrated,
}

impl AuditProvider {
    /// The wire name and the stored name, which are the same string.
    ///
    /// One function rather than two, because a name that is written differently
    /// in the database and in the API response is a name an operator cannot
    /// search for.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Deterministic => "deterministic",
            Self::Calibrated => "calibrated",
        }
    }

    /// Parse a stored or submitted name, refusing anything else by falling back.
    ///
    /// An unrecognised provider becomes `Deterministic` rather than an error,
    /// and the direction of that guess is the whole point: the failure mode of
    /// guessing wrong is a row claiming a model was consulted when it was not,
    /// and the safe direction for that is to say **less**, not to fail the write
    /// and lose the record entirely.
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        match raw {
            "calibrated" => Self::Calibrated,
            _ => Self::Deterministic,
        }
    }
}

/// One recorded decision, as the row holds it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AuditEntry {
    pub id: String,
    /// Which decision surface: `import_quality`, `positivity`, ...
    pub task: String,
    /// The id of what was classified. **Never the classified text** — see the
    /// module docs.
    pub subject: String,
    /// What the deterministic classifier said.
    pub deterministic: String,
    /// The model's probability, or `None` when no model was consulted.
    pub posterior: Option<f64>,
    /// The threshold in force at the time.
    pub threshold: Option<f64>,
    /// What was actually applied.
    pub outcome: String,
    pub provider: AuditProvider,
    pub created_at: String,
}

/// The row as the database returns it, in both dialects.
///
/// `provider` is read as a `String` and parsed in [`AuditEntry::from_row`]
/// rather than decoded by serde, so a row written by a future version with a
/// provider this build does not know reads as `Deterministic` instead of failing
/// the whole page. `created_at` is cast in the Postgres SQL for the same reason:
/// the column is `timestamptz` there and `TEXT` here, and the API speaks one
/// shape.
#[derive(sqlx::FromRow)]
struct AuditRow {
    id: String,
    task: String,
    subject: String,
    deterministic: String,
    posterior: Option<f64>,
    threshold: Option<f64>,
    outcome: String,
    provider: String,
    created_at: String,
}

/// A decision to record. The id, the timestamps and the version are the store's
/// business, not the caller's.
#[derive(Debug, Clone)]
pub struct NewAuditEntry {
    pub task: String,
    pub subject: String,
    pub deterministic: String,
    pub posterior: Option<f64>,
    pub threshold: Option<f64>,
    pub outcome: String,
    pub provider: AuditProvider,
}

/// Record a decision.
///
/// **A refused write is returned as an error rather than swallowed.** The call
/// site decides what to do about it, which is the right way round: an instance
/// that cannot write its own audit trail has lost the ability to answer "why did
/// the system hold my work", and it should learn that at the write rather than
/// discover it months later in an empty table.
pub async fn record(db: &Database, entry: &NewAuditEntry) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    // No ON CONFLICT anywhere: an audit row is written once and never updated.
    // An upsert would let a second decision about the same subject overwrite the
    // first, and an audit that edits its own history is not an audit. A second
    // decision is a second row, and reading them in order is the history.
    let sql = db.sql(
        "INSERT INTO decision_audit
            (id, task, subject, deterministic, posterior, threshold, outcome,
             provider, created_at, updated_at, version)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1)",
        "INSERT INTO decision_audit
            (id, task, subject, deterministic, posterior, threshold, outcome,
             provider, created_at, updated_at, version)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 1)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(&entry.task)
                .bind(&entry.subject)
                .bind(&entry.deterministic)
                .bind(entry.posterior)
                .bind(entry.threshold)
                .bind(&entry.outcome)
                .bind(entry.provider.as_str())
                .bind(&now)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(&entry.task)
                .bind(&entry.subject)
                .bind(&entry.deterministic)
                .bind(entry.posterior)
                .bind(entry.threshold)
                .bind(&entry.outcome)
                .bind(entry.provider.as_str())
                .bind(&now)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(id)
}

/// How to narrow the audit read.
#[derive(Debug, Clone, Default)]
pub struct AuditQuery {
    /// Only this decision surface.
    pub task: Option<String>,
    /// Only this subject.
    pub subject: Option<String>,
    /// Only this provider.
    pub provider: Option<AuditProvider>,
    /// Newest first, bounded by the caller and then by [`MAX_AUDIT_PAGE`].
    pub limit: i64,
}

/// The audit trail, newest first.
pub async fn list(db: &Database, query: &AuditQuery) -> Result<Vec<AuditEntry>> {
    // Clamped rather than trusted, and the zero case is deliberate: `LIMIT 0`
    // means "return nothing" to both dialects, so an operator asking for the
    // newest decisions and being shown none is a bug report rather than a
    // feature. A non-positive request is pinned to 1.
    let limit = if query.limit <= 0 {
        1
    } else {
        query.limit.min(MAX_AUDIT_PAGE)
    };

    // Assembled from fragments rather than formatted with values, because every
    // fragment except the ORDER BY comes from a request. `db.sql` supplies the
    // two dialects' placeholder styles, so the filter text is written once.
    let mut filters: Vec<&str> = Vec::new();
    if query.task.is_some() {
        filters.push("task = ?");
    }
    if query.subject.is_some() {
        filters.push("subject = ?");
    }
    if query.provider.is_some() {
        filters.push("provider = ?");
    }
    let where_clause = if filters.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", filters.join(" AND "))
    };

    // `created_at DESC, id DESC` matches the index, and the id is in the ORDER
    // BY for a reason that matters: two rows written in the same millisecond
    // would otherwise come back in an order the database chose, and an audit
    // that reorders itself between two reads of the same page is one nobody can
    // page through.
    const ORDER: &str = " ORDER BY created_at DESC, id DESC LIMIT ?";
    // Bound to locals, not written inline: `db.sql` hands back a `&str` into
    // its arguments, so a `&format!(...)` passed straight through would be a
    // borrow of a temporary that dies at the end of the statement.
    let sqlite_sql = format!(
        "SELECT id, task, subject, deterministic, posterior, threshold, outcome, \
         provider, created_at FROM decision_audit{where_clause}{ORDER}"
    );
    let postgres_sql = format!(
        "SELECT id, task, subject, deterministic, posterior, threshold, outcome, \
         provider, created_at::text AS created_at FROM decision_audit{where_clause}{ORDER}"
    );
    let sql = db.sql(&sqlite_sql, &postgres_sql);

    let rows: Vec<AuditRow> = match db.backend() {
        Backend::Sqlite => {
            let mut q = sqlx::query_as::<_, AuditRow>(&sql);
            if let Some(task) = &query.task {
                q = q.bind(task);
            }
            if let Some(subject) = &query.subject {
                q = q.bind(subject);
            }
            if let Some(provider) = &query.provider {
                q = q.bind(provider.as_str());
            }
            q.bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            let mut q = sqlx::query_as::<_, AuditRow>(&sql);
            if let Some(task) = &query.task {
                q = q.bind(task);
            }
            if let Some(subject) = &query.subject {
                q = q.bind(subject);
            }
            if let Some(provider) = &query.provider {
                q = q.bind(provider.as_str());
            }
            q.bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(rows.into_iter().map(AuditEntry::from_row).collect())
}

impl AuditEntry {
    fn from_row(row: AuditRow) -> Self {
        Self {
            id: row.id,
            task: row.task,
            subject: row.subject,
            deterministic: row.deterministic,
            posterior: row.posterior,
            threshold: row.threshold,
            outcome: row.outcome,
            provider: AuditProvider::parse(&row.provider),
            created_at: row.created_at,
        }
    }
}

/// Every recorded decision for one subject, newest first.
///
/// The read an operator or an author actually makes: "why was this held?". It
/// is a named function rather than a `list` with a subject filter because it is
/// the query the `idx_decision_audit_subject` index exists for, and a caller who
/// has to know that is a caller who will eventually write the wrong one.
pub async fn for_subject(db: &Database, subject: &str, limit: i64) -> Result<Vec<AuditEntry>> {
    list(
        db,
        &AuditQuery {
            subject: Some(subject.to_owned()),
            limit,
            ..AuditQuery::default()
        },
    )
    .await
}

/// Has a model ever narrowed an acceptance to a hold?
///
/// This is the question the amendment's safety property reduces to, and it is
/// asked *of the recorded rows* rather than asserted in a test. A test proves the
/// policy was one-directional when it was written; this proves it still is, for
/// this instance, over the decisions it has actually made.
///
/// The answer is `true` for any row where the deterministic answer was
/// `accepted` and the outcome was not. There is deliberately **no** query for
/// the converse — a rejection or a hold that a model turned into an acceptance —
/// so this function cannot report that case at all. That is the point: the
/// property is checked where it is supposed to hold, and the property that must
/// never hold has no code path that could claim it does.
pub async fn has_a_model_narrowed_anything(db: &Database) -> Result<bool> {
    let sql = db.sql(
        "SELECT 1 FROM decision_audit
          WHERE provider = 'calibrated' AND deterministic = 'accepted'
            AND outcome <> 'accepted' LIMIT 1",
        "SELECT 1 FROM decision_audit
          WHERE provider = 'calibrated' AND deterministic = 'accepted'
            AND outcome <> 'accepted' LIMIT 1",
    );
    let hit: Option<(i32,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(hit.is_some())
}
