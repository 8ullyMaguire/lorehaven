//! §18.11 — gift-exchange terms and the pairing (M45-27).
//!
//! The rule layer is [`darkhold_domain::exchanges`], which is pure and decides whether a
//! pairing exists; this module is the record: what each participant declared, the pairing
//! that was taken, and when it may be revealed.
//!
//! ## The participant set is `event_participation`, and that is a rule
//!
//! A person joins the event, then declares terms. Terms from somebody who never joined are
//! refused with [`SaveTerms::NotAParticipant`] rather than stored and ignored: a term row
//! that the matcher never reads is a person who believes they are in an exchange they are
//! not in, and the refusal is the only place that can say so.
//!
//! ## Matching runs once, and the rows are the record
//!
//! `run_match` refuses to overwrite an existing pairing ([`MatchOutcome::AlreadyMatched`],
//! naming when it was taken). §58.3's rule applies unchanged: the row is evidence. An
//! exchange that can be re-rolled is one where the first pairing never meant anything.
//!
//! ## A refusal writes nothing
//!
//! When the domain matcher answers `Impossible`, this module inserts no rows — not even a
//! partial pairing — and returns the accounts it could not place. Half an exchange is the
//! one outcome a gift exchange cannot recover from: the people left out were promised
//! nothing and would find out at reveal.

use anyhow::Result;
use sqlx::Row;

use darkhold_domain::exchanges::{match_participants, MatchFailure, Method, Participant, Pairs};

use crate::{identity::now_rfc3339, Backend, Database};

/// One row of the pairing. A macro rather than a generic function: `Row::get` needs
/// `Decode<R::Database>`, which is only satisfied for the concrete driver, so a generic
/// `fn assignment_of<R: Row>` does not compile (the same wall `welcome.rs` hit).
macro_rules! assignment_of {
    ($row:expr) => {
        Assignment {
            giver: $row.get::<String, _>("giver"),
            recipient: $row.get::<String, _>("recipient"),
            assigned_at: $row.get::<String, _>("assigned_at"),
            revealed_at: $row.get::<Option<String>, _>("revealed_at"),
            matched_by: $row.get::<String, _>("matched_by"),
        }
    };
}


/// One participant's declared terms, values already normalised.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Terms {
    pub account: String,
    pub offers: Vec<String>,
    pub wants: Vec<String>,
    pub excludes: Vec<String>,
}

/// One row of the pairing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub giver: String,
    pub recipient: String,
    pub assigned_at: String,
    pub revealed_at: Option<String>,
    pub matched_by: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub participants: i64,
    pub matched: i64,
    pub revealed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveTerms {
    Saved(Terms),
    NotAParticipant,
    /// The account named itself in `excludes`. Refused rather than dropped: a self-exclusion
    /// is a misunderstanding of the field, and silently discarding it hides the mistake.
    SelfExclusion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchOutcome {
    Matched {
        assignments: Vec<Assignment>,
        method: Method,
    },
    AlreadyMatched {
        assigned_at: String,
    },
    Impossible {
        unmatched: Vec<String>,
    },
    TooFew {
        participants: usize,
    },
    /// The event does not exist, so there is nothing to pair.
    NoEvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reveal {
    Revealed { revealed_at: String },
    NotMatched,
    NoEvent,
}

/// Normalise a term value: trimmed, lowercased, inner whitespace collapsed.
///
/// Fandom names arrive from a text input, where `"Dragon  Age"` and `"dragon age"` are the
/// same fandom. The matcher compares strings exactly, so the normalisation has to happen
/// once, on the way in — the alternative is a matcher that re-normalises on every comparison
/// and a record that says what the person did not type.
pub fn normalise(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn normalise_all(values: &[String]) -> Vec<String> {
    let mut out: Vec<String> = values
        .iter()
        .map(|v| normalise(v))
        .filter(|v| !v.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Whether `account` has joined `event_id`.
pub async fn is_participant(db: &Database, event_id: &str, account: &str) -> Result<bool> {
    let sql = db.sql(
        "SELECT COUNT(*) AS n FROM event_participation WHERE event_id = ? AND account = ?",
        "SELECT COUNT(*) AS n FROM event_participation WHERE event_id = $1 AND account = $2",
    );
    let count: i64 = match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(&sql)
                .bind(event_id)
                .bind(account)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?;
            row.get::<i64, _>("n")
        }
        Backend::Postgres => {
            let row = sqlx::query(&sql)
                .bind(event_id)
                .bind(account)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?;
            row.get::<i64, _>("n")
        }
    };
    Ok(count > 0)
}

/// Replace one participant's terms wholesale.
///
/// Wholesale because a preference is not a log: `PUT` with the whole set is the honest
/// shape, and "remove one fandom" is expressed by sending the set without it.
pub async fn save_terms(
    db: &Database,
    event_id: &str,
    account: &str,
    offers: &[String],
    wants: &[String],
    excludes: &[String],
) -> Result<SaveTerms> {
    if !is_participant(db, event_id, account).await? {
        return Ok(SaveTerms::NotAParticipant);
    }
    let offers = normalise_all(offers);
    let wants = normalise_all(wants);
    let excludes = normalise_all(excludes);
    if excludes.iter().any(|e| e == account) {
        return Ok(SaveTerms::SelfExclusion);
    }

    let now = now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let mut tx = pool.begin().await?;
            sqlx::query("DELETE FROM exchange_terms WHERE event_id = ? AND account = ?")
                .bind(event_id)
                .bind(account)
                .execute(&mut *tx)
                .await?;
            for (kind, values) in [
                ("offers_fandom", &offers),
                ("wants_fandom", &wants),
                ("excludes_pseud", &excludes),
            ] {
                for value in values {
                    sqlx::query(
                        "INSERT INTO exchange_terms (event_id, account, kind, value, created_at)
                         VALUES (?, ?, ?, ?, ?)",
                    )
                    .bind(event_id)
                    .bind(account)
                    .bind(kind)
                    .bind(value)
                    .bind(&now)
                    .execute(&mut *tx)
                    .await?;
                }
            }
            tx.commit().await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let mut tx = pool.begin().await?;
            sqlx::query("DELETE FROM exchange_terms WHERE event_id = $1 AND account = $2")
                .bind(event_id)
                .bind(account)
                .execute(&mut *tx)
                .await?;
            for (kind, values) in [
                ("offers_fandom", &offers),
                ("wants_fandom", &wants),
                ("excludes_pseud", &excludes),
            ] {
                for value in values {
                    sqlx::query(
                        "INSERT INTO exchange_terms (event_id, account, kind, value, created_at)
                         VALUES ($1, $2, $3, $4, $5)",
                    )
                    .bind(event_id)
                    .bind(account)
                    .bind(kind)
                    .bind(value)
                    .bind(&now)
                    .execute(&mut *tx)
                    .await?;
                }
            }
            tx.commit().await?;
        }
    }

    Ok(SaveTerms::Saved(Terms {
        account: account.to_string(),
        offers,
        wants,
        excludes,
    }))
}

/// Every participant's terms for one event, in account order.
pub async fn terms(db: &Database, event_id: &str) -> Result<Vec<Terms>> {
    let sql = db.sql(
        "SELECT account, kind, value FROM exchange_terms WHERE event_id = ?
         ORDER BY account, kind, value",
        "SELECT account, kind, value FROM exchange_terms WHERE event_id = $1
         ORDER BY account, kind, value",
    );
    let rows: Vec<(String, String, String)> = match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(&sql)
                .bind(event_id)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?;
            rows.iter()
                .map(|r| {
                    (
                        r.get::<String, _>("account"),
                        r.get::<String, _>("kind"),
                        r.get::<String, _>("value"),
                    )
                })
                .collect()
        }
        Backend::Postgres => {
            let rows = sqlx::query(&sql)
                .bind(event_id)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?;
            rows.iter()
                .map(|r| {
                    (
                        r.get::<String, _>("account"),
                        r.get::<String, _>("kind"),
                        r.get::<String, _>("value"),
                    )
                })
                .collect()
        }
    };

    let mut out: Vec<Terms> = Vec::new();
    for (account, kind, value) in rows {
        if out.last().map(|t| t.account.as_str()) != Some(account.as_str()) {
            out.push(Terms {
                account: account.clone(),
                ..Terms::default()
            });
        }
        let entry = out.last_mut().expect("just pushed");
        match kind.as_str() {
            "offers_fandom" => entry.offers.push(value),
            "wants_fandom" => entry.wants.push(value),
            "excludes_pseud" => entry.excludes.push(value),
            // The CHECK constraint makes this unreachable; answering instead of panicking
            // keeps a schema drift from taking the process down.
            _ => {}
        }
    }
    Ok(out)
}

/// The participants of an event, in account order, with their terms folded in.
pub async fn participants(db: &Database, event_id: &str) -> Result<Vec<Participant>> {
    let sql = db.sql(
        "SELECT account FROM event_participation WHERE event_id = ? ORDER BY account",
        "SELECT account FROM event_participation WHERE event_id = $1 ORDER BY account",
    );
    let accounts: Vec<String> = match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(&sql)
                .bind(event_id)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?;
            rows.iter().map(|r| r.get::<String, _>("account")).collect()
        }
        Backend::Postgres => {
            let rows = sqlx::query(&sql)
                .bind(event_id)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?;
            rows.iter().map(|r| r.get::<String, _>("account")).collect()
        }
    };

    let declared = terms(db, event_id).await?;
    Ok(accounts
        .into_iter()
        .map(|account| {
            let t = declared.iter().find(|t| t.account == account);
            Participant {
                account: account.clone(),
                offers: t
                    .map(|t| t.offers.iter().cloned().collect())
                    .unwrap_or_default(),
                wants: t.map(|t| t.wants.iter().cloned().collect()).unwrap_or_default(),
                excludes: t
                    .map(|t| t.excludes.iter().cloned().collect())
                    .unwrap_or_default(),
            }
        })
        .collect())
}

/// The pairing for one event, or empty when it has not been run.
pub async fn assignments(db: &Database, event_id: &str) -> Result<Vec<Assignment>> {
    let sql = db.sql(
        "SELECT giver, recipient, assigned_at, revealed_at, matched_by
           FROM exchange_assignments WHERE event_id = ? ORDER BY giver",
        "SELECT giver, recipient, assigned_at, revealed_at, matched_by
           FROM exchange_assignments WHERE event_id = $1 ORDER BY giver",
    );
    // The two drivers have different row types, so the mapping happens inside each arm.
    let rows: Vec<Assignment> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(event_id)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
                .iter()
                .map(|r| assignment_of!(r))
                .collect()
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(event_id)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
                .iter()
                .map(|r| assignment_of!(r))
                .collect()
        }
    };
    Ok(rows)
}

/// One giver's own row.
pub async fn mine(db: &Database, event_id: &str, account: &str) -> Result<Option<Assignment>> {
    Ok(assignments(db, event_id)
        .await?
        .into_iter()
        .find(|a| a.giver == account))
}

/// Run the matcher for an event and store the pairing.
pub async fn run_match(db: &Database, event_id: &str) -> Result<MatchOutcome> {
    let existing = assignments(db, event_id).await?;
    if let Some(first) = existing.first() {
        return Ok(MatchOutcome::AlreadyMatched {
            assigned_at: first.assigned_at.clone(),
        });
    }

    let people = participants(db, event_id).await?;
    if people.len() < 2 {
        return Ok(MatchOutcome::TooFew {
            participants: people.len(),
        });
    }

    let Pairs { pairs, method } = match match_participants(&people) {
        Ok(pairs) => pairs,
        Err(MatchFailure::TooFew { participants }) => {
            return Ok(MatchOutcome::TooFew { participants })
        }
        Err(MatchFailure::Impossible { unmatched }) => {
            return Ok(MatchOutcome::Impossible { unmatched })
        }
    };

    let now = now_rfc3339();
    let matched_by = method.as_str();
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let mut tx = pool.begin().await?;
            for pair in &pairs {
                sqlx::query(
                    "INSERT INTO exchange_assignments
                     (event_id, giver, recipient, assigned_at, revealed_at, matched_by)
                     VALUES (?, ?, ?, ?, NULL, ?)",
                )
                .bind(event_id)
                .bind(&pair.giver)
                .bind(&pair.recipient)
                .bind(&now)
                .bind(matched_by)
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let mut tx = pool.begin().await?;
            for pair in &pairs {
                sqlx::query(
                    "INSERT INTO exchange_assignments
                     (event_id, giver, recipient, assigned_at, revealed_at, matched_by)
                     VALUES ($1, $2, $3, $4, NULL, $5)",
                )
                .bind(event_id)
                .bind(&pair.giver)
                .bind(&pair.recipient)
                .bind(&now)
                .bind(matched_by)
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await?;
        }
    }

    Ok(MatchOutcome::Matched {
        assignments: assignments(db, event_id).await?,
        method,
    })
}

/// Make the pairing visible to the givers. Idempotent: the second call reports the first
/// timestamp rather than writing a new one.
pub async fn reveal(db: &Database, event_id: &str) -> Result<Reveal> {
    let existing = assignments(db, event_id).await?;
    if existing.is_empty() {
        return Ok(Reveal::NotMatched);
    }
    if let Some(revealed_at) = existing[0].revealed_at.clone() {
        return Ok(Reveal::Revealed { revealed_at });
    }
    let now = now_rfc3339();
    let sql = db.sql(
        "UPDATE exchange_assignments SET revealed_at = ? WHERE event_id = ? AND revealed_at IS NULL",
        "UPDATE exchange_assignments SET revealed_at = $1 WHERE event_id = $2 AND revealed_at IS NULL",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&now)
                .bind(event_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&now)
                .bind(event_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(Reveal::Revealed { revealed_at: now })
}

/// Counts only: how many joined, how many are paired, whether it is revealed.
pub async fn summary(db: &Database, event_id: &str) -> Result<Summary> {
    let participants = {
        let sql = db.sql(
            "SELECT COUNT(*) AS n FROM event_participation WHERE event_id = ?",
            "SELECT COUNT(*) AS n FROM event_participation WHERE event_id = $1",
        );
        match db.backend() {
            Backend::Sqlite => {
                let row = sqlx::query(&sql)
                    .bind(event_id)
                    .fetch_one(db.sqlite_pool().expect("sqlite"))
                    .await?;
                row.get::<i64, _>("n")
            }
            Backend::Postgres => {
                let row = sqlx::query(&sql)
                    .bind(event_id)
                    .fetch_one(db.postgres_pool().expect("postgres"))
                    .await?;
                row.get::<i64, _>("n")
            }
        }
    };
    let assigned = assignments(db, event_id).await?;
    Ok(Summary {
        participants,
        matched: assigned.len() as i64,
        revealed: assigned.iter().any(|a| a.revealed_at.is_some()),
    })
}
