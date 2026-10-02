//! Reason-tagged kudos and line-level highlights (spec §50.1, M45-24).
//!
//! ## Why kudos gain a reason rather than a second table
//!
//! §50.1 says a **bare kudos is still valid**. A kudos with a reason and a kudos
//! without one are the same gesture by the same reader on the same work, so this
//! is a nullable column with a CHECK rather than a NOT NULL column or a second
//! table. Two tables would make "kudos per work" a UNION and make the bare case --
//! which is by far the common one -- slower for no gain.
//!
//! ## Why the reason is optional at the API and required by the profile
//!
//! Making it required would push readers toward a rate-limit rather than toward a
//! reason, which is the opposite of what the column is for. So the write path
//! accepts `None` and stores it. The *training* path is where the reason is
//! required, and that asymmetry is the whole mechanism: §49.5's reason-tagged
//! sampling trains the profile, a bare kudos does not, and both are recorded.
//!
//! ## Why highlights count once
//!
//! §50.1: fifty highlights on one work move its gravity by the weight of one
//! signal. Not implemented here as an integer — [`crate::reasons::HighlightSignal`]
//! owns that arithmetic, because §33.2's influence purchase is a spec clause and
//! the temptation to reintroduce it as a `COUNT(*)` is exactly what this comment
//! exists to prevent.
//!
//! ## Determinism
//!
//! §47.9 transfers: the distinct-reader count is a `COUNT(DISTINCT ...)`, which is
//! a function of the data, so the same rows give the same weight on both engines.

use crate::{Backend, Database};
use anyhow::{bail, Result};
use lorehaven_domain::reasons::{Reason, Span};

/// What a reader wrote when giving kudos or a highlight.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotatedReason {
    /// `None` is a real state: a bare kudos. §50.3's first invariant is that a
    /// reason is never invented, so there is no default and no empty string.
    pub reason: Option<Reason>,
    /// Stored, never aggregated. §50.3: free text cannot be counted across
    /// readers without becoming an unreviewable store of prose.
    pub note: Option<String>,
}

impl AnnotatedReason {
    fn reason_str(&self) -> Option<&'static str> {
        self.reason.map(Reason::as_str)
    }

    fn note_str(&self) -> Option<&str> {
        self.note.as_deref()
    }
}

/// Refuse a reason string the fixed set does not contain.
///
/// §50.4: "a reason outside the fixed set is refused". `Option`, never a default —
/// §50.3's first invariant is that a reason is never invented, and a defaulting
/// parser trains the wrong dimension for every typo.
pub fn parse_reason(raw: Option<&str>) -> Result<Option<Reason>> {
    match raw {
        None => Ok(None),
        Some(s) => match Reason::parse(s) {
            Some(reason) => Ok(Some(reason)),
            None => bail!(
                "unknown reason {s:?}: expected one of {}",
                Reason::ALL
                    .iter()
                    .map(|r| r.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        },
    }
}

/// Give kudos to a work, optionally with a reason.
///
/// Replaces rather than duplicates: `work_kudos` is keyed on
/// (work_id, account_id), so giving kudos twice updates the reason instead of
/// adding a second row. That matters because a reader who kudos with a reason and
/// then re-kudos without one has *withdrawn* the reason, and a second row would
/// leave the trained signal behind with nothing to remove it.
pub async fn give_kudos(
    db: &Database,
    work_id: &str,
    account_id: &str,
    annotated: &AnnotatedReason,
) -> Result<()> {
    let reason = annotated.reason_str();
    let note = annotated.note_str();
    let sql = db.sql(
        "INSERT INTO work_kudos (work_id, account_id, created_at, reason, note)
         VALUES (?, ?, datetime('now'), ?, ?)
         ON CONFLICT(work_id, account_id) DO UPDATE SET reason = excluded.reason, note = excluded.note",
        "INSERT INTO work_kudos (work_id, account_id, created_at, reason, note)
         -- `$3::text` and `$4::text` are not decoration. A bare NULL bind on
         -- PostgreSQL arrives with no type, and sqlx cannot infer one for an
         -- `Option<&str>`, so the INSERT fails with a 500 -- and only when the
         -- reason is ABSENT, because a present value carries its own type. That
         -- makes it invisible on SQLite and on every reasoned kudos, and it is
         -- exactly the case 50.1 requires: a bare kudos must be valid. The cast
         -- names the type for the null case; `retention_proposals.rs` does the
         -- same for the same reason.
         VALUES ($1::uuid, $2::uuid, to_char(now() AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS'), $3::text, $4::text)
         ON CONFLICT(work_id, account_id) DO UPDATE SET reason = EXCLUDED.reason, note = EXCLUDED.note",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(work_id)
                .bind(account_id)
                .bind(reason)
                .bind(note)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(work_id)
                .bind(account_id)
                .bind(reason)
                .bind(note)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(())
}

/// Read back a reader's kudos and its reason.
pub async fn kudos_reason(
    db: &Database,
    work_id: &str,
    account_id: &str,
) -> Result<Option<Reason>> {
    let sql = db.sql(
        "SELECT reason FROM work_kudos WHERE work_id = ? AND account_id = ?",
        "SELECT reason FROM work_kudos WHERE work_id::text = $1 AND account_id::text = $2",
    );
    // The `Option<Option<String>>` is NOT collapsed here, and that is the whole
    // point of this function's shape. `fetch_optional` already reports "no row"
    // (never kudoed) as the outer `None`; the inner one is the column, which is
    // NULL for a bare kudos. Binding the result to a single `Option<String>`
    // annotation instead makes sqlx decode the COLUMN as a non-optional `String`,
    // and the bare kudos -- the case 50.1 requires be valid -- fails with
    // "unexpected null; try decoding as an `Option`". It reproduces on
    // PostgreSQL only: SQLite widens the null to empty text, so the two engines
    // disagreed about a request that is supposed to be backend-independent.
    //
    // So each arm decodes on its own and answers with the inner value, which is
    // what "does this kudos carry a reason" means. A missing row and a bare
    // kudos both answer None -- correctly, since neither trains anything.
    let raw: Option<String> = match db.backend() {
        Backend::Sqlite => sqlx::query_scalar::<_, Option<String>>(&sql)
            .bind(work_id)
            .bind(account_id)
            .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .flatten(),
        Backend::Postgres => sqlx::query_scalar::<_, Option<String>>(&sql)
            .bind(work_id)
            .bind(account_id)
            .fetch_optional(db.postgres_pool().expect("postgres handle"))
            .await?
            .flatten(),
    };
    // A stored reason is always one the set contains -- the CHECK says so -- but
    // `parse` rather than a cast keeps the invariant in one place instead of
    // assuming the database and the enum cannot drift. `None` maps to `None`, so a
    // bare kudos reads back as bare rather than acquiring a reason here.
    Ok(raw.as_deref().and_then(Reason::parse))
}

/// Record a highlight on a span of a work.
pub async fn add_highlight(
    db: &Database,
    work_id: &str,
    account_id: &str,
    span: Span,
    reason: Reason,
    note: Option<&str>,
) -> Result<()> {
    let id = uuid::Uuid::new_v4().to_string();
    let sql = db.sql(
        "INSERT INTO work_highlights
            (id, work_id, account_id, start_offset, end_offset, reason, note, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, datetime('now'))
         ON CONFLICT(work_id, account_id, start_offset, end_offset) DO NOTHING",
        "INSERT INTO work_highlights
            (id, work_id, account_id, start_offset, end_offset, reason, note, created_at)
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4::bigint, $5::bigint, $6::text, $7::text, to_char(now() AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS'))
         ON CONFLICT(work_id, account_id, start_offset, end_offset) DO NOTHING",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(work_id)
                .bind(account_id)
                .bind(span.start)
                .bind(span.end)
                .bind(reason.as_str())
                .bind(note)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(work_id)
                .bind(account_id)
                .bind(span.start)
                .bind(span.end)
                .bind(reason.as_str())
                .bind(note)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(())
}

/// §50.1's distinct-reader count, the number that reaches gravity.
///
/// `COUNT(DISTINCT account_id)` for the readers and a separate count for the
/// reason-bearing ones — and never a single total, because "counts once" is a
/// claim about *readers*. §50.4's "fifty highlights move gravity by one signal" is
/// exactly the difference between `COUNT(*)` and this.
pub async fn highlight_signal(
    db: &Database,
    work_id: &str,
) -> Result<lorehaven_domain::reasons::HighlightSignal> {
    let sql = db.sql(
        "SELECT COUNT(DISTINCT account_id),
                COUNT(*),
                COUNT(DISTINCT CASE WHEN reason IS NOT NULL THEN account_id END)
         FROM work_highlights WHERE work_id = ?",
        "SELECT COUNT(DISTINCT account_id),
                COUNT(*),
                COUNT(DISTINCT CASE WHEN reason IS NOT NULL THEN account_id END)
         FROM work_highlights WHERE work_id::text = $1",
    );
    let row: (i64, i64, i64) = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(lorehaven_domain::reasons::HighlightSignal {
        readers: row.0,
        highlights: row.1,
        reason_bearing_readers: row.2,
    })
}

/// How many kudos on a work carry a trainable reason.
///
/// §50.4: "a kudos with a reason trains the profile on that reason's dimension;
/// the same kudos with the reason omitted trains nothing". This is the query that
/// makes the second half of that clause checkable, and it is *not* the kudos count.
pub async fn reason_bearing_kudos(db: &Database, work_id: &str) -> Result<i64> {
    let sql = db.sql(
        "SELECT COUNT(*) FROM work_kudos WHERE work_id = ? AND reason IS NOT NULL",
        "SELECT COUNT(*)::bigint FROM work_kudos WHERE work_id::text = $1 AND reason IS NOT NULL",
    );
    let n: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §50.4: a reason outside the fixed set is refused.
    #[test]
    fn an_unknown_reason_is_refused() {
        let err = parse_reason(Some("vibes")).unwrap_err();
        assert!(err.to_string().contains("vibes"), "{err}");
        // And the message lists what is allowed, so a client can fix itself.
        assert!(err.to_string().contains("prose"), "{err}");
    }

    /// §50.1: a bare kudos is valid. `None` in, `None` out — not a default.
    #[test]
    fn a_bare_kudos_is_valid_and_carries_no_reason() {
        assert_eq!(parse_reason(None).unwrap(), None);
    }

    /// §50.3: a reason is never invented, so an empty string is not "no reason" —
    /// it is a malformed reason.
    #[test]
    fn an_empty_reason_string_is_refused_rather_than_read_as_absent() {
        assert!(parse_reason(Some("")).is_err());
    }

    /// The whole set round-trips, which is what keeps the CHECK in migration 0106
    /// and the enum from drifting apart.
    #[test]
    fn every_reason_in_the_set_is_accepted() {
        for reason in Reason::ALL {
            assert_eq!(parse_reason(Some(reason.as_str())).unwrap(), Some(reason));
        }
    }

    #[test]
    fn an_annotated_reason_with_only_a_note_has_no_reason() {
        let annotated = AnnotatedReason {
            reason: None,
            note: Some("the second chapter undoes it".to_owned()),
        };
        assert_eq!(annotated.reason_str(), None);
        assert!(annotated.note_str().is_some());
    }
}
