//! Retention proposals: opening one, voting on it, tallying it, and recording
//! what the instance then did (spec §5, §19.15).
//!
//! **The ballots are the privacy surface and this module is where that is
//! enforced.** No function here returns a ballot, and `tally` returns counts
//! rather than rows for exactly that reason: a route that serialised
//! `retention_proposal_votes` would expose who voted which way on a question
//! about how much storage the instance spends, which is a reading of a reader's
//! habits. §45.2's flat-weight rule exists to stop the same thing by another
//! route — a weighted vote lets a habit set policy — so a reader's *choice* is
//! as protected as their *weight* is denied.
//!
//! **A vote cannot be stacked.** `cast_vote` is an upsert keyed on
//! `(proposal_id, account_id)`, which is also the table's primary key, so the
//! second vote from an account replaces the first. The schema enforces it and
//! the statement re-asserts it; a reader changing their mind is a normal
//! event, not an attempt to buy a quorum.

use anyhow::{anyhow, Result};
use lorehaven_domain::retention::BodyMode;
use lorehaven_domain::retention_quorum::{quorum_outcome, QuorumOutcome};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{Backend, Database};

/// The lifecycle of a proposal. Five states, and they are not interchangeable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposalState {
    /// Ballots are being cast.
    Open,
    /// The ballot closed with enough support, and (in binding mode) the
    /// setting was changed.
    Passed,
    /// The ballot closed without it.
    Failed,
    /// An operator answered a `Passed` proposal against its own outcome. A
    /// distinct state from `Passed` because an operator dashboard that cannot
    /// tell them apart is lying about who decided.
    Overridden,
    /// The window closed and it did not reach quorum.
    Expired,
}

impl ProposalState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Overridden => "overridden",
            Self::Expired => "expired",
        }
    }

    /// Parse a stored state, `None` for absent or unrecognised.
    ///
    /// Unrecognised maps to `Open` rather than to a terminal state. A proposal
    /// whose state a newer build introduced is still *open* as far as this
    /// build knows, and the safe error is to keep accepting ballots — a
    /// proposal silently treated as `Failed` would strand its own author.
    #[must_use]
    pub fn parse_stored(value: &str) -> Self {
        match value {
            "passed" => Self::Passed,
            "failed" => Self::Failed,
            "overridden" => Self::Overridden,
            "expired" => Self::Expired,
            _ => Self::Open,
        }
    }

    /// Whether ballots are still accepted.
    #[must_use]
    pub const fn accepts_ballots(self) -> bool {
        matches!(self, Self::Open)
    }
}

/// A proposal, as the store holds it.
#[derive(Debug, Clone)]
pub struct RetentionProposal {
    pub id: String,
    pub proposed_mode: BodyMode,
    /// `None` is the instance-wide setting. A scoped proposal is the common
    /// case, because the decision a reader cares about is usually about one
    /// archive.
    pub source_key: Option<String>,
    pub rationale: String,
    pub opened_by: String,
    pub closes_at: String,
    pub state: ProposalState,
    pub tallied_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
}

/// The counts behind a proposal, and what they mean for the bar.
///
/// **Counts and not rows.** This is the whole privacy boundary of the feature:
/// a reader learns how many people support a change and never who.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tally {
    pub supporters: i64,
    pub opposed: i64,
    /// Whether the bar is met, and if not, by how much.
    pub quorum: QuorumOutcome,
}

/// A recorded change, as the store holds it.
#[derive(Debug, Clone)]
pub struct PolicyChange {
    pub id: String,
    pub from_mode: Option<BodyMode>,
    pub to_mode: BodyMode,
    pub source_key: Option<String>,
    /// Who made the change.
    ///
    /// Always an account: the column is `NOT NULL REFERENCES accounts (id) ON
    /// DELETE RESTRICT`, so an audit row with nobody on it is not
    /// representable and a recorded decision cannot be orphaned by an account
    /// deletion. A binding-mode settlement names
    /// [`lorehaven_db::SYSTEM_ACCOUNT`] — the instance acting on the readers'
    /// recorded decision, which is a fact and not a missing person.
    pub actor: String,
    pub reason: String,
    pub decided_at: String,
}

#[derive(FromRow)]
struct ProposalRow {
    id: String,
    proposed_mode: String,
    source_key: Option<String>,
    rationale: String,
    opened_by: String,
    closes_at: String,
    state: String,
    tallied_at: Option<String>,
    created_at: String,
    updated_at: String,
    version: i64,
}

#[derive(FromRow)]
struct ChangeRow {
    id: String,
    from_mode: Option<String>,
    to_mode: String,
    source_key: Option<String>,
    actor: String,
    reason: String,
    decided_at: String,
}

fn into_proposal(row: ProposalRow) -> RetentionProposal {
    RetentionProposal {
        // An unrecognised `proposed_mode` fails closed to the mode in force,
        // matching `BodyMode::parse_stored`'s own reasoning: a reader meeting a
        // value from a newer build should not be shown a proposal that appears
        // to stop storing what this instance stores.
        proposed_mode: BodyMode::parse_stored(Some(&row.proposed_mode)).unwrap_or_default(),
        id: row.id,
        source_key: row.source_key,
        rationale: row.rationale,
        opened_by: row.opened_by,
        closes_at: row.closes_at,
        state: ProposalState::parse_stored(&row.state),
        tallied_at: row.tallied_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
        version: row.version,
    }
}

/// The columns every proposal read projects.
///
/// Per-dialect because `id` is UUID on PostgreSQL and TEXT on SQLite, and
/// `version` is INTEGER there (INT4) against an `i64` decode. Sharing one list
/// and casting per arm is the shape that produced the five faults Phase D
/// already paid for once.
const PROPOSAL_COLUMNS: &str = "id, proposed_mode, source_key, rationale, opened_by, \
     closes_at, state, tallied_at, created_at, updated_at, version";
const PROPOSAL_COLUMNS_SQLITE: &str = "id, proposed_mode, source_key, rationale, opened_by, \
     closes_at, state, tallied_at, created_at, updated_at, version";
const PROPOSAL_COLUMNS_POSTGRES: &str = "id::text AS id, proposed_mode, source_key, rationale, \
     opened_by::text AS opened_by, closes_at::text AS closes_at, state, \
     tallied_at::text AS tallied_at, created_at::text AS created_at, \
     updated_at::text AS updated_at, version::bigint AS version";

fn proposal_select(db: &Database) -> String {
    let columns = match db.backend() {
        Backend::Sqlite => PROPOSAL_COLUMNS_SQLITE,
        Backend::Postgres => PROPOSAL_COLUMNS_POSTGRES,
    };
    debug_assert!(!columns.is_empty());
    let _ = PROPOSAL_COLUMNS;
    format!("SELECT {columns} FROM retention_proposals")
}

/// The proposal already open on a setting, if any.
///
/// **One open proposal per setting, and the check is here rather than in the
/// route** because it is a property of the table, not of the API: two
/// concurrent `POST`s from two readers would both pass a check-then-insert
/// done in the handler. The store's `open_proposal` does the check and the
/// insert, so the race is decided by the database.
///
/// `source_key` is compared with `IS`, not `=`, because `NULL = NULL` is
/// unknown in SQL and an instance-wide proposal would never be found by its own
/// key. That is the bug this predicate exists to prevent, and it is invisible
/// on a test that only ever opens scoped proposals.
pub async fn open_proposal(
    db: &Database,
    source_key: Option<&str>,
) -> Result<Option<RetentionProposal>> {
    // `IS NOT DISTINCT FROM`, and the reason is not a style preference.
    //
    // `source_key IS ?` is the null-safe equality SQLite accepts and
    // PostgreSQL rejects at *parse* time: `IS` is the three-valued-logic
    // operator and requires `NULL`/`TRUE`/`FALSE` on its right, not an
    // expression. The PostgreSQL arm failed with `syntax error at or near "?"`
    // and a position naming the whole statement.
    //
    // `IS NOT DISTINCT FROM` *is* null-safe equality and does take a
    // parameter, so one predicate covers both the scoped case and the
    // instance-wide one -- and the instance-wide case is why it cannot be `=`.
    // A `source_key = 'x'` predicate can never match the row whose
    // `source_key` is NULL, so an instance-wide proposal would be invisible to
    // its own one-open-per-setting check and a second one would be accepted.
    let sqlite_sql = format!(
        "{} WHERE state = 'open' AND source_key IS ? ORDER BY created_at LIMIT 1",
        proposal_select(db)
    );
    let postgres_sql = format!(
        "{} WHERE state = 'open' AND source_key IS NOT DISTINCT FROM $1
          ORDER BY created_at LIMIT 1",
        proposal_select(db)
    );
    let sql = db.sql(&sqlite_sql, &postgres_sql);
    let row: Option<ProposalRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(source_key)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(source_key)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(into_proposal))
}

/// The mode currently in force for a setting.
///
/// Instance-wide is the instance policy; a scoped setting is the instance
/// policy narrowed by an override, if one exists. §11.15 allows a source to
/// be narrowed *only* -- never widened past the instance setting -- so the
/// override is the answer when there is one and the instance mode otherwise.
///
/// This is deliberately *not* `resolve_for_source`, which is a refusal-
/// resolution function for the import path: it takes `source_blocked` and
/// `vanished` and returns a `ResolvedRetention` carrying the whole decision.
/// A proposal needs one of the two fields from it, and calling it with two
/// invented booleans would be asking a question about a fetch in order to
/// learn a setting.
///
/// The override table is scanned rather than keyed. An instance has one row
/// per source an operator has ever narrowed -- tens, not thousands -- and a
/// proposal is opened a handful of times a day, so the scan costs nothing and
/// saves a second query function whose only difference is a WHERE clause.
async fn mode_in_force(db: &Database, source_key: Option<&str>) -> Result<BodyMode> {
    let Some(key) = source_key else {
        return crate::retention::effective_instance_mode(db).await;
    };
    for override_row in crate::retention::list_source_overrides(db).await? {
        if override_row.source_key == key {
            return Ok(override_row.body_mode);
        }
    }
    crate::retention::effective_instance_mode(db).await
}

/// Open a proposal on a setting.
///
/// Refuses while one is already open on the same setting, in the same call that
/// would otherwise insert — see [`open_proposal`] for why the check lives here.
///
/// **Refuses a proposal for the mode already in force.** A no-op proposal is not
/// a harmless row: it is something a reader can see, vote on, and believe
/// changed something. `quorum_for` would answer `3` for it, which is a number
/// with no meaning behind it.
pub async fn create_proposal(
    db: &Database,
    source_key: Option<&str>,
    proposed_mode: BodyMode,
    rationale: &str,
    opened_by: Uuid,
    closes_at: &str,
) -> Result<RetentionProposal> {
    if rationale.trim().is_empty() {
        return Err(anyhow!(
            "a retention proposal needs a rationale: a reader who cannot see why a \
             change is proposed cannot decide whether to support it"
        ));
    }
    let current = mode_in_force(db, source_key).await?;
    if proposed_mode.stores_bodies() == current.stores_bodies() {
        return Err(anyhow!(
            "this setting is already {}; a proposal for the mode in force is not a change",
            current.as_str()
        ));
    }
    if open_proposal(db, source_key).await?.is_some() {
        return Err(anyhow!(
            "a proposal for this setting is already open; wait for it to close rather \
             than running two ballots at once"
        ));
    }

    let id = Uuid::new_v4();
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO retention_proposals
             (id, proposed_mode, source_key, rationale, opened_by, closes_at, state,
              tallied_at, created_at, updated_at, version)
         VALUES (?, ?, ?, ?, ?, ?, 'open', NULL, ?, ?, 1)",
        // `$1`..`$6` and never `?::uuid, $1::uuid, ...`: sqlx numbers
        // placeholders per arm, so a leading `?` would be the first bind too and
        // the row would point `opened_by` at the wrong id.
        "INSERT INTO retention_proposals
             (id, proposed_mode, source_key, rationale, opened_by, closes_at, state,
              tallied_at, created_at, updated_at, version)
         VALUES ($1::uuid, $2, $3, $4, $5::uuid, $6, 'open', NULL, $7, $8, 1)",
    );
    if let Some(pool) = db.sqlite_pool() {
        sqlx::query(&sql)
            .bind(id.to_string())
            .bind(proposed_mode.as_str())
            .bind(source_key)
            .bind(rationale)
            .bind(opened_by.to_string())
            .bind(closes_at)
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await?;
    }
    if let Some(pool) = db.postgres_pool() {
        sqlx::query(&sql)
            .bind(id.to_string())
            .bind(proposed_mode.as_str())
            .bind(source_key)
            .bind(rationale)
            .bind(opened_by.to_string())
            .bind(closes_at)
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await?;
    }
    proposal(db, &id.to_string())
        .await?
        .ok_or_else(|| anyhow!("the proposal {id} was written and then could not be read back"))
}

/// One proposal by id.
pub async fn proposal(db: &Database, id: &str) -> Result<Option<RetentionProposal>> {
    let sql = format!(
        "{} WHERE id = {}",
        proposal_select(db),
        match db.backend() {
            Backend::Sqlite => "?",
            Backend::Postgres => "$1::uuid",
        }
    );
    let row: Option<ProposalRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(into_proposal))
}

/// Every proposal, newest first. **No ballot data — see the module header.**
pub async fn list_proposals(db: &Database) -> Result<Vec<RetentionProposal>> {
    let sql = format!("{} ORDER BY created_at DESC", proposal_select(db));
    let rows: Vec<ProposalRow> = match db.backend() {
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
    Ok(rows.into_iter().map(into_proposal).collect())
}

/// Cast or change a ballot.
///
/// An upsert on `(proposal_id, account_id)`, which is the table's primary key.
/// **A second vote replaces the first rather than adding to it**, and that is
/// the anti-buy mechanism: a reader who changes their mind is normal, and a
/// reader who votes twice to reach a quorum is not possible.
///
/// Refuses a ballot on a closed proposal, and says which state it closed in —
/// a reader voting on a proposal that already passed needs to know that, not to
/// watch a 409 with no explanation.
pub async fn cast_vote(
    db: &Database,
    proposal_id: &str,
    account: Uuid,
    support: bool,
) -> Result<()> {
    let Some(open) = proposal(db, proposal_id).await? else {
        return Err(anyhow!("no proposal {proposal_id}"));
    };
    if !open.state.accepts_ballots() {
        return Err(anyhow!(
            "this proposal is {} and no longer takes ballots",
            open.state.as_str()
        ));
    }
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO retention_proposal_votes (proposal_id, account_id, support, cast_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT (proposal_id, account_id) DO UPDATE SET support = excluded.support,
                                                          cast_at   = excluded.cast_at",
        "INSERT INTO retention_proposal_votes (proposal_id, account_id, support, cast_at)
         VALUES ($1::uuid, $2::uuid, $3, $4)
         ON CONFLICT (proposal_id, account_id) DO UPDATE SET support = excluded.support,
                                                          cast_at   = excluded.cast_at",
    );
    if let Some(pool) = db.sqlite_pool() {
        sqlx::query(&sql)
            .bind(proposal_id)
            .bind(account.to_string())
            .bind(i64::from(support))
            .bind(&now)
            .execute(pool)
            .await?;
    }
    if let Some(pool) = db.postgres_pool() {
        sqlx::query(&sql)
            .bind(proposal_id)
            .bind(account.to_string())
            .bind(i64::from(support))
            .bind(&now)
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// Count a proposal's ballots and test them against the bar.
///
/// **Two aggregate queries and no `GROUP BY` over accounts.** The result is two
/// numbers; a query that returned per-account rows would make it possible for a
/// future caller to leak a ballot by accident, and the privacy property should
/// not depend on every future caller remembering.
///
/// `current` is the mode in force *now*, not the one in force when the proposal
/// was opened. An operator can change a setting directly while a proposal is
/// open, and a tally that used the stale mode would apply a bar chosen for a
/// question nobody is being asked any more.
pub async fn tally(db: &Database, proposal_id: &str, widen_quorum: i64) -> Result<Tally> {
    let Some(proposal) = proposal(db, proposal_id).await? else {
        return Err(anyhow!("no proposal {proposal_id}"));
    };
    let current = mode_in_force(db, proposal.source_key.as_deref()).await?;

    let sql = db.sql(
        "SELECT support, COUNT(*) FROM retention_proposal_votes
          WHERE proposal_id = ? GROUP BY support",
        // `::bigint` on BOTH arms' worth of columns, and column 0 is the one
        // that needed it. The decode failure named `support`, not the count:
        // `INTEGER` is INT4 on PostgreSQL and INT8 on SQLite, so an `i64`
        // decode of either column fails there and works on SQLite. Casting
        // only the count would have left column 0 short.
        "SELECT support::bigint, COUNT(*)::bigint FROM retention_proposal_votes
          WHERE proposal_id = $1::uuid GROUP BY support",
    );
    let counts: Vec<(i64, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(proposal_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(proposal_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    let supporters = counts.iter().find(|(s, _)| *s == 1).map_or(0, |(_, n)| *n);
    let opposed = counts.iter().find(|(s, _)| *s == 0).map_or(0, |(_, n)| *n);
    Ok(Tally {
        supporters,
        opposed,
        quorum: quorum_outcome(supporters, proposal.proposed_mode, current, widen_quorum),
    })
}

/// Move a proposal to a terminal state, stamping the tally.
///
/// A conditional UPDATE on `state = 'open'`, and **the row count is the
/// answer**: two closing passes racing on the same proposal means one of them
/// updated zero rows, and the caller that treats that as an error is
/// expressing "somebody else already closed this" rather than silently
/// overwriting a state another process chose.
pub async fn close_proposal(
    db: &Database,
    proposal_id: &str,
    state: ProposalState,
) -> Result<bool> {
    if state == ProposalState::Open {
        return Err(anyhow!("`open` is not a state to close a proposal into"));
    }
    // Which source states a given target may be reached from.
    //
    // **A single `state = 'open'` guard was a real defect.** It made
    // `overridden` unreachable: the only way into a terminal state was from
    // `open`, so a proposal that `passed` could never become `overridden` --
    // and the override IS the operator's answer to a *passing* ballot, which
    // is the case `overridden` exists to record. The state was in the schema
    // and unreachable from the store.
    //
    // The permitted transitions, and why each is here:
    //
    // * `open -> passed | failed | expired` -- the ballot closed.
    // * `open -> overridden` -- the operator answered a ballot that is **still
    //   running**. The ordinary case: a reader opened a proposal, two of five
    //   have voted, and the operator disagrees now rather than after a week of
    //   waiting for a vote that is not coming. This was refused, and
    //   `override_setting` already handled it -- the close silently affected
    //   zero rows, so the route returned 200 with the setting changed and the
    //   proposal still reading `open`, which is a governance record claiming
    //   nothing was recorded.
    // * `passed -> overridden` -- the operator answered a ballot that *had*
    //   finished. The setting was already changed by the commit, so reversing
    //   it is a second fact about the same proposal, not a re-tally.
    //
    // Both `overridden` rows are the same act — the operator answering a
    // ballot — and they differ only in whether the ballot finished first. There
    // is no reading under which "the ballot must be decided before it can be
    // overridden" makes sense: the override *is* the decision being recorded,
    // not a precondition for it.
    //
    // Everything else is refused, so a second close of a terminal proposal
    // still reports "somebody else already closed this" rather than
    // overwriting a state another process chose. `passed -> failed` is
    // included in that refusal deliberately: a proposal that passed does not
    // stop having passed because an operator later disagreed. `failed ->
    // overridden` is refused for a different reason: there is nothing left to
    // disagree with, the ballot already failed and the setting never moved.
    let permitted_from: &[&str] = match state {
        ProposalState::Overridden => &["open", "passed"],
        _ => &["open"],
    };
    let clause = permitted_from
        .iter()
        .map(|from| format!("state = '{from}'"))
        .collect::<Vec<_>>()
        .join(" OR ");

    let now = crate::identity::now_rfc3339();
    // Bound to locals first: `db.sql` takes `&str`, and a `&format!(...)`
    // temporary is dropped at the end of the enclosing statement -- which is
    // the `sql` binding, not the end of the function.
    let sqlite_sql = format!(
        "UPDATE retention_proposals
            SET state = ?, tallied_at = ?, updated_at = ?, version = version + 1
          WHERE id = ? AND ({clause})"
    );
    let postgres_sql = format!(
        "UPDATE retention_proposals
            SET state = $1, tallied_at = $2::text, updated_at = $3::text,
                version = version + 1
          WHERE id = $4::uuid AND ({clause})"
    );
    let sql = db.sql(&sqlite_sql, &postgres_sql);
    let affected = if let Some(pool) = db.sqlite_pool() {
        sqlx::query(&sql)
            .bind(state.as_str())
            .bind(&now)
            .bind(&now)
            .bind(proposal_id)
            .execute(pool)
            .await?
            .rows_affected()
    } else if let Some(pool) = db.postgres_pool() {
        sqlx::query(&sql)
            .bind(state.as_str())
            .bind(&now)
            .bind(&now)
            .bind(proposal_id)
            .execute(pool)
            .await?
            .rows_affected()
    } else {
        0
    };
    Ok(affected > 0)
}

/// Record a change, and write the setting it changed.
///
/// **Both, in one call, and the order is fixed: setting first, record second.**
/// The reverse order would leave a record of a change that did not happen, and
/// `retention_policy_changes` is the thing an operator reads to find out what
/// this instance did — a record that can be ahead of reality is worse than no
/// record. If the second write fails, the setting is changed and unrecorded,
/// which an operator can still see by reading the setting.
///
/// `from_mode` is what was in force *before* the write, passed in rather than
/// re-read, because the caller read it to decide the bar and re-reading is a
/// second answer to a question that was already answered.
/// Record that a retention setting moved, and why.
///
/// `actor` is always an account. A binding-mode settlement passes
/// [`crate::SYSTEM_ACCOUNT`], which is the point: the readers' decision moved
/// the setting, the instance is what acted, and no reader's name is on the row
/// — the ballot leak stays out of the audit trail because the actor is a system
/// account rather than one of the three people who voted.
pub async fn record_change(
    db: &Database,
    from_mode: Option<BodyMode>,
    to_mode: BodyMode,
    source_key: Option<&str>,
    actor: Uuid,
    reason: &str,
) -> Result<PolicyChange> {
    let id = Uuid::new_v4();
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO retention_policy_changes
             (id, from_mode, to_mode, source_key, actor, reason, decided_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        "INSERT INTO retention_policy_changes
             (id, from_mode, to_mode, source_key, actor, reason, decided_at)
         VALUES ($1::uuid, $2, $3, $4, $5::uuid, $6, $7::text)",
    );
    if let Some(pool) = db.sqlite_pool() {
        sqlx::query(&sql)
            .bind(id.to_string())
            .bind(from_mode.map(BodyMode::as_str))
            .bind(to_mode.as_str())
            .bind(source_key)
            .bind(actor.to_string())
            .bind(reason)
            .bind(&now)
            .execute(pool)
            .await?;
    }
    if let Some(pool) = db.postgres_pool() {
        sqlx::query(&sql)
            .bind(id.to_string())
            .bind(from_mode.map(BodyMode::as_str))
            .bind(to_mode.as_str())
            .bind(source_key)
            .bind(actor.to_string())
            .bind(reason)
            .bind(&now)
            .execute(pool)
            .await?;
    }
    Ok(PolicyChange {
        id: id.to_string(),
        from_mode,
        to_mode,
        source_key: source_key.map(str::to_owned),
        actor: actor.to_string(),
        reason: reason.to_owned(),
        decided_at: now,
    })
}

/// The changes on a setting, most recent first. Instance-wide when `None`.
pub async fn list_changes(db: &Database, source_key: Option<&str>) -> Result<Vec<PolicyChange>> {
    let sql = db.sql(
        "SELECT id, from_mode, to_mode, source_key, actor, reason, decided_at
           FROM retention_policy_changes
          WHERE source_key IS ? ORDER BY decided_at DESC",
        // `IS NOT DISTINCT FROM` for the same reason as `open_proposal`: `IS $1`
        // is a parse error on PostgreSQL, and `= 'x'` cannot match the NULL row an
        // instance-wide change leaves behind.
        "SELECT id::text AS id, from_mode, to_mode, source_key, actor::text AS actor,
                reason::text AS reason, decided_at::text AS decided_at
           FROM retention_policy_changes
          WHERE source_key IS NOT DISTINCT FROM $1 ORDER BY decided_at DESC",
    );
    let rows: Vec<ChangeRow> = if let Some(pool) = db.sqlite_pool() {
        sqlx::query_as(&sql)
            .bind(source_key)
            .fetch_all(pool)
            .await?
    } else if let Some(pool) = db.postgres_pool() {
        sqlx::query_as(&sql)
            .bind(source_key)
            .fetch_all(pool)
            .await?
    } else {
        Vec::new()
    };
    Ok(rows
        .into_iter()
        .map(|row| PolicyChange {
            id: row.id,
            from_mode: BodyMode::parse_stored(row.from_mode.as_deref()),
            to_mode: BodyMode::parse_stored(Some(&row.to_mode)).unwrap_or_default(),
            source_key: row.source_key,
            actor: row.actor,
            reason: row.reason,
            decided_at: row.decided_at,
        })
        .collect())
}

/// Open proposals whose window has closed, for the maintenance pass to settle.
///
/// Returned rather than acted on: **closing a proposal is a decision about
/// storage policy, and the maintenance pass is not where a decision is made.**
/// It finds the candidates and the caller tallies and closes them.
pub async fn overdue_proposals(db: &Database, now: &str) -> Result<Vec<RetentionProposal>> {
    // `closes_at` is TEXT in both dialects -- an RFC3339 string, not a
    // timestamp column -- so the comparison is lexicographic and needs no
    // cast. That is deliberate: a TIMESTAMP column would make it depend on
    // both engines agreeing on a timezone, and it is sound only because every
    // writer uses `now_rfc3339()`, which emits a fixed-width UTC form.
    //
    // Both placeholders written out. A `format!` that interpolates a single
    // `match` on the backend is how the literal `?` reached the PostgreSQL
    // statement the first time; two arms is the only shape that cannot.
    let sqlite_sql = format!(
        "{} WHERE state = 'open' AND closes_at <= ? ORDER BY closes_at",
        proposal_select(db)
    );
    let postgres_sql = format!(
        "{} WHERE state = 'open' AND closes_at <= $1 ORDER BY closes_at",
        proposal_select(db)
    );
    let sql = db.sql(&sqlite_sql, &postgres_sql);
    let rows: Vec<ProposalRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(now)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(now)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows.into_iter().map(into_proposal).collect())
}
