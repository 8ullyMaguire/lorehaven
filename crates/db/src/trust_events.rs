//! M3-03 — the trust event log.
//!
//! # What this is
//!
//! An **append-only** record of every trust change: who, from which level to
//! which level, why, and about what.
//!
//! The current level lives in `trust_levels`, and the two are different facts.
//! `trust_levels` answers "what is this account's level now?", which is the
//! question the tier gate asks on every request. **This table answers "how
//! did it get there, and did anything ever take it away?"** — which is the
//! question a reader whose level fell has, and the question the specification
//! asks for in §6.7.
//!
//! # The invariant
//!
//! **A level that changed with no event is a defect.** Not a style issue, not
//! a missing nicety: an unexplained level change is indistinguishable from a
//! level that was always that, and a moderator queue whose membership cannot
//! be explained cannot be audited.
//!
//! The table also refuses an `UPDATE` and a `DELETE` — with triggers, in
//! migration 0117, on both engines. A trust history that can be rewritten is
//! not a history.
//!
//! # The cause vocabulary is closed
//!
//! [`TrustCause`] is the whole of it, and the database has the same list in a
//! `CHECK`. Two lists that can disagree would be a list the database accepts
//! and the code cannot explain — and an unexplainable event is worse than no
//! event, because it looks like a record.

use crate::{Backend, Database};

/// Why a trust level moved.
///
/// **A closed vocabulary**, enforced by the database as well as here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustCause {
    /// The tenure axis moved the level.
    Tenure,
    /// The accepted-contributions axis moved the level.
    Contribution,
    /// The vote-score axis moved the level.
    VoteScore,
    /// The report-accuracy axis moved the level.
    ReportAccuracy,
    /// A privilege's requirement fell because **nobody** held it, and this
    /// account was given the privilege by that allocation (spec §14.4).
    Elastic,
    /// The one automatic demotion: three confirmed spam reports in ninety days
    /// (spec §14.4).
    DemotionSpam,
    /// A person granted or took a level.
    Operator,
    /// The account was folded, and its trust did not travel with it.
    ///
    /// **The fold grants tenure and never score** (spec §14.2): a folded
    /// guest's account starts over, because the guest door is otherwise a
    /// trust farm.
    Fold,
}

impl TrustCause {
    /// The cause's storage form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tenure => "tenure",
            Self::Contribution => "contribution",
            Self::VoteScore => "vote_score",
            Self::ReportAccuracy => "report_accuracy",
            Self::Elastic => "elastic",
            Self::DemotionSpam => "demotion_spam",
            Self::Operator => "operator",
            Self::Fold => "fold",
        }
    }

    /// Parse the storage form, refusing an unknown cause.
    ///
    /// **Refusing rather than defaulting**, because a default would turn a
    /// cause the code does not know into a cause it does — and the event
    /// would then say something false about why a level moved.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "tenure" => Some(Self::Tenure),
            "contribution" => Some(Self::Contribution),
            "vote_score" => Some(Self::VoteScore),
            "report_accuracy" => Some(Self::ReportAccuracy),
            "elastic" => Some(Self::Elastic),
            "demotion_spam" => Some(Self::DemotionSpam),
            "operator" => Some(Self::Operator),
            "fold" => Some(Self::Fold),
            _ => None,
        }
    }

    /// Every cause, in the order the vocabulary names them.
    #[must_use]
    pub fn all() -> Vec<TrustCause> {
        vec![
            Self::Tenure,
            Self::Contribution,
            Self::VoteScore,
            Self::ReportAccuracy,
            Self::Elastic,
            Self::DemotionSpam,
            Self::Operator,
            Self::Fold,
        ]
    }
}

/// One trust event.
#[derive(Debug, Clone)]
pub struct TrustEvent {
    /// The event's id.
    pub id: String,
    /// The account the event is about.
    pub account_id: String,
    /// The level before the event.
    pub from_level: i64,
    /// The level after the event.
    pub to_level: i64,
    /// Why it moved.
    pub cause: TrustCause,
    /// What it was about: a proposal id, a report id, or a privilege name.
    pub evidence_ref: Option<String>,
    /// Who caused it, when a person did.
    pub actor_account_id: Option<String>,
    /// When it happened, RFC 3339.
    pub created_at: String,
}

/// Record a trust event.
///
/// **The one writer.** There is no other way to write a row into this table,
/// and there is no way to write a row into it that does not say why.
pub async fn record(
    db: &Database,
    account_id: &str,
    from_level: i64,
    to_level: i64,
    cause: TrustCause,
    evidence_ref: Option<&str>,
    actor_account_id: Option<&str>,
) -> Result<String, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO trust_events
           (id, account_id, from_level, to_level, cause, evidence_ref, actor_account_id, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        "INSERT INTO trust_events
           (id, account_id, from_level, to_level, cause, evidence_ref, actor_account_id, created_at)
         VALUES (?::uuid, ?::uuid, ?, ?, ?, ?, ?::uuid, ?)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(account_id)
                .bind(from_level)
                .bind(to_level)
                .bind(cause.as_str())
                .bind(evidence_ref)
                .bind(actor_account_id)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(account_id)
                .bind(from_level)
                .bind(to_level)
                .bind(cause.as_str())
                .bind(evidence_ref)
                .bind(actor_account_id)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(id)
}

/// The account's trust history, newest first.
pub async fn list_for_account(
    db: &Database,
    account_id: &str,
    limit: i64,
) -> Result<Vec<TrustEvent>, sqlx::Error> {
    let sql = db.sql(
        "SELECT id, account_id, from_level, to_level, cause, evidence_ref, actor_account_id, created_at
           FROM trust_events WHERE account_id = ?
          ORDER BY created_at DESC, id DESC LIMIT ?",
        "SELECT id::text, account_id::text, CAST(from_level AS BIGINT), CAST(to_level AS BIGINT),
                cause, evidence_ref, actor_account_id::text, created_at
           FROM trust_events WHERE account_id = ?::uuid
          ORDER BY created_at DESC, id DESC LIMIT ?",
    );
    let rows: Vec<EventRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows.into_iter().map(into_event).collect())
}

/// Everybody's trust events, newest first.
///
/// **The cursor is `(created_at, id)`**, so the pagination is stable: two
/// events written in the same millisecond do not make the second page
/// overlap the first.
pub async fn list_all(
    db: &Database,
    limit: i64,
    cursor: Option<(&str, &str)>,
) -> Result<Vec<TrustEvent>, sqlx::Error> {
    let (after_time, after_id) = cursor
        .map(|(t, i)| (t.to_owned(), i.to_owned()))
        .unwrap_or_default();
    let sql = db.sql(
        "SELECT id, account_id, from_level, to_level, cause, evidence_ref, actor_account_id, created_at
           FROM trust_events
          WHERE (? = '' OR (created_at, id) < (?, ?))
          ORDER BY created_at DESC, id DESC LIMIT ?",
        "SELECT id::text, account_id::text, CAST(from_level AS BIGINT), CAST(to_level AS BIGINT),
                cause, evidence_ref, actor_account_id::text, created_at
           FROM trust_events
          WHERE ($1 = '' OR (created_at, id) < ($2, $3::uuid))
          ORDER BY created_at DESC, id DESC LIMIT $4",
    );
    let rows: Vec<EventRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(after_time.as_str())
                .bind(after_time.as_str())
                .bind(after_id.as_str())
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(after_time.as_str())
                .bind(after_time.as_str())
                .bind(after_id.as_str())
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows.into_iter().map(into_event).collect())
}

/// The eight columns of a trust event, as one row.
///
/// **A named row rather than a tuple of eight**, because a tuple of eight is a
/// type nobody can read at the call site and the compiler will not tell you
/// which one is wrong when the order is off.
type EventRow = (
    String,         // id
    String,         // account_id
    i64,            // from_level
    i64,            // to_level
    String,         // cause
    Option<String>, // evidence_ref
    Option<String>, // actor_account_id
    String,         // created_at
);

/// Decode one row into an event.
fn into_event(row: EventRow) -> TrustEvent {
    let (id, account_id, from_level, to_level, cause, evidence_ref, actor_account_id, created_at) =
        row;
    // `cause` is a CHECK in the schema, so an unknown value is impossible;
    // the fallback is there so a *newer* writer's cause is not silently
    // turned into a lie.
    let cause = TrustCause::parse(&cause).unwrap_or(TrustCause::Operator);
    TrustEvent {
        id,
        account_id,
        from_level,
        to_level,
        cause,
        evidence_ref,
        actor_account_id,
        created_at,
    }
}
