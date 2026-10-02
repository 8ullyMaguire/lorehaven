//! M45-18 — §53: the economy read back as faucets and sinks.
//!
//! The dashboard's promise is that it cannot report a smaller economy than exists, and two
//! decisions in this file carry that.
//!
//! **Classification is derived from `TxnType`, never from the `reference`.** The obvious key
//! is `credit_transactions.reference`, and it is wrong for a reason worth stating: the
//! preservation mechanism posts two transactions carrying the *same* reference — a member id
//! — on opposite sides of the loop. `Preservation` is a sink, `PreservationReclaim` is a
//! faucet. Keying on the reference would either pick one arbitrarily or match nothing, and
//! since member ids differ per member it would match *every* one of them wrongly and report
//! the whole mechanism as undeclared. See migration 0112.
//!
//! **A missing declaration is counted, never dropped.** An unrecognised transaction type
//! still moved real credits. Omitting it would shrink the reported economy, and §0.3's
//! inflation worry is exactly a number smaller than the truth.
//!
//! **`amount_bp` holds whole credits, not basis points.** The column name is a historical
//! artefact — `post_transaction` binds the caller's `i64` straight into it, and
//! `preservation.rs` says so in a comment. Nothing here divides by 100.

use anyhow::Result;
use sqlx::Row;

use crate::{Backend, Database};
use lorehaven_domain::economy::TxnType;
use lorehaven_domain::flows::{Flow, Mechanism, MechanismDeclaration, Purpose};

/// The purpose a mechanism claims, fixed by its identity rather than by operator choice.
///
/// `mechanism_declarations` stores a `flow` but no purpose. The purpose is not a per-instance
/// preference: §0.3 forbids a sink that converts credits into anything but supply, so letting
/// an operator edit it would put the classification's authority behind the exact edit §0.3
/// exists to prevent. Keeping the mapping here means adding a mechanism is a migration row
/// plus one arm, and a declaration nobody can mis-set.
const fn declared_purpose(key: &str) -> Option<Purpose> {
    match key.as_bytes() {
        // §20.3: authors earn for work that already exists. Supply.
        b"author_earnings" => Some(Purpose::Supply),
        // Dues convert credits into supply, and the reclaim is those credits returning.
        b"preservation_dues" | b"preservation_reclaim" => Some(Purpose::Supply),
        // A tip buys an author's work. Supply — which is what makes it legal under
        // `classify_sink`; a sink claiming `Signal` would be buying trust.
        b"tips" => Some(Purpose::Supply),
        _ => None,
    }
}

/// The mechanism a transaction belongs to, or `None` when its type declares none.
///
/// `None` is the undeclared case, not an error. The caller turns it into a mechanism that
/// still contributes to the net but reports `declared: false`.
fn mechanism_for(txn_type: &TxnType, reference: &str) -> Option<&'static str> {
    match txn_type {
        TxnType::Earn => Some("author_earnings"),
        TxnType::Preservation => Some("preservation_dues"),
        TxnType::PreservationReclaim => Some("preservation_reclaim"),
        // Tips are matched by prefix rather than by type alone: a `Spend` is a tip only when
        // the reference says so. If a second sink ever posts a `Spend`, it needs its own arm
        // here and a row in migration 0112 — which is the point of keeping classification in
        // one place instead of spreading it across callers.
        TxnType::Spend if reference.starts_with("tip:") => Some("tips"),
        _ => None,
    }
}

/// The mechanism key for one ledger row, falling back to a stable undeclared key.
///
/// Extracted from [`mechanisms_in_window`] so it can be tested directly: the derivation is
/// where the interesting decisions live, and a helper is the only way a unit test can reach
/// it without a database.
///
/// Both `Err` arms mean the same thing — the transaction is not one any declaration covers —
/// and neither skips the row. A transaction whose type the domain has never heard of still
/// moved real credits, so it keeps its own key and is counted. Skipping it is the single line
/// that would let the dashboard report less than the ledger holds.
fn mechanism_key_for(txn_type: &str, reference: &str) -> String {
    match txn_type.parse::<TxnType>() {
        Ok(parsed) => mechanism_for(&parsed, reference)
            .map_or_else(|| undeclared_key(txn_type, reference), str::to_owned),
        Err(_) => undeclared_key(txn_type, reference),
    }
}

/// Read `mechanism_declarations` into `key -> Flow`.
///
/// This is the only place a mechanism's side comes from, and it is deliberately *not* derived
/// from the sign of the entries: a bug in a faucet produces a negative amount, which is
/// precisely when "negative means sink" is wrong. A mechanism missing from this table stays
/// missing, and the caller counts it as undeclared instead of guessing.
async fn declarations_by_key(db: &Database) -> Result<std::collections::HashMap<String, Flow>> {
    let sql = db.sql(
        "SELECT mechanism_key, flow FROM mechanism_declarations",
        "SELECT mechanism_key, flow FROM mechanism_declarations",
    );
    // The row type differs per engine, so the fetch is split rather than collected into one
    // variable. This is the same shape `economy.rs` uses for `fetch_balances_sqlite` and
    // `fetch_balances_postgres`, and for the same reason: `SqliteRow` and `PgRow` are
    // distinct types and a `Vec` of either cannot be named once.
    let mut out: std::collections::HashMap<String, Flow> = Default::default();
    let mut record = |key: String, flow: String| {
        // A value the CHECK constraint should have stopped is dropped rather than parsed to
        // something plausible. `Flow::parse` returning `None` means the table and the domain
        // enum disagree, and papering over that here would hide the drift.
        match Flow::parse(&flow) {
            Some(flow) => {
                out.insert(key, flow);
            }
            None => {
                tracing::warn!(
                    mechanism_key = %key,
                    flow = %flow,
                    "declaration has an unknown flow and is treated as undeclared"
                );
            }
        }
    };

    match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(sql.as_ref())
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?;
            for row in rows {
                record(row.get("mechanism_key"), row.get("flow"));
            }
        }
        Backend::Postgres => {
            let rows = sqlx::query(sql.as_ref())
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?;
            for row in rows {
                record(row.get("mechanism_key"), row.get("flow"));
            }
        }
    }
    Ok(out)
}

/// One mechanism's declared side plus the purpose it claims.
///
/// The side comes from the registry and the purpose from [`declared_purpose`], and the two
/// are joined here so no caller can mix them up. A declared side with no known purpose falls
/// back to `Supply` rather than becoming undeclared: §0.3 only ever forbids a *sink* claiming
/// signal, so supply is the one purpose that cannot violate it, and a mechanism that is
/// genuinely a faucet or sink is still honestly described by the side it declared.
fn build_declaration(flow: Flow, key: &str) -> MechanismDeclaration {
    MechanismDeclaration {
        flow,
        purpose: match (flow, declared_purpose(key)) {
            // A neutral or undeclared mechanism has no side, and §53.1 says a neutral one
            // may not claim a purpose at all.
            (Flow::Neutral | Flow::Undeclared, _) => None,
            (_, Some(purpose)) => Some(purpose),
            (_, None) => Some(Purpose::Supply),
        },
    }
}

/// Every mechanism that moved credits in `[since, until]`, one row per mechanism.
///
/// Ordered by key so two windows with the same data render identically.
pub async fn mechanisms_in_window(
    db: &Database,
    since: &str,
    until: &str,
) -> Result<Vec<Mechanism>> {
    // Grouping happens in Rust rather than in SQL for a reason that is easy to miss: the
    // mechanism is a *derived* value (see `mechanism_for`), and SQLite has no `CASE` that
    // both engines spell the same way here. The row count is small — one per transaction —
    // and the grouping is a few lines, so the portable statement is the better trade.
    //
    // Both endpoints inclusive, and deliberately so: an operator reading a window of
    // "2026-10-01 through 2026-10-02" expects both of those days in it.
    let sql = db.sql(
        "SELECT t.type, t.reference, SUM(e.amount_bp) AS total
           FROM credit_entries e
           JOIN credit_transactions t ON t.id = e.transaction_id
          WHERE e.created_at >= ? AND e.created_at <= ?
          GROUP BY t.type, t.reference",
        "SELECT t.type, t.reference, CAST(SUM(e.amount_bp) AS BIGINT) AS total
           FROM credit_entries e
           JOIN credit_transactions t ON t.id = e.transaction_id
          WHERE e.created_at >= $1 AND e.created_at <= $2
          GROUP BY t.type, t.reference",
    );
    // `BTreeMap` so the output order is deterministic and does not depend on hash order.
    let mut totals: std::collections::BTreeMap<String, i64> = Default::default();
    // One accumulator for both engines: `SqliteRow` and `PgRow` are distinct types, so the
    // rows are drained in each arm and folded through a closure rather than collected into
    // one `Vec`. `economy.rs` splits into two whole functions instead, which is also fine;
    // the closure is shorter here because the folding is three lines.
    let mut accumulate = |txn_type: String, reference: String, total: i64| {
        let key = mechanism_key_for(&txn_type, &reference);
        *totals.entry(key).or_default() += total;
    };

    match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(sql.as_ref())
                .bind(since)
                .bind(until)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?;
            for row in rows {
                accumulate(row.get("type"), row.get("reference"), row.get("total"));
            }
        }
        Backend::Postgres => {
            let rows = sqlx::query(sql.as_ref())
                .bind(since)
                .bind(until)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?;
            for row in rows {
                accumulate(row.get("type"), row.get("reference"), row.get("total"));
            }
        }
    }

    let declarations = declarations_by_key(db).await?;
    let mut out: Vec<Mechanism> = totals
        .into_iter()
        .map(|(key, net_credits)| {
            let declaration = match declarations.get(&key) {
                Some(flow) => build_declaration(*flow, &key),
                // `Undeclared`, not `neutral`: a neutral mechanism is one somebody decided
                // has no side. Rendering a missing declaration as neutral would show it in
                // the composition looking settled, and `undeclared` would stay at zero —
                // the dashboard reporting less than the ledger holds, which is the one
                // thing §53.1 forbids.
                None => MechanismDeclaration {
                    flow: Flow::Undeclared,
                    purpose: None,
                },
            };
            Mechanism {
                key,
                declaration,
                net_credits,
            }
        })
        .collect();
    out.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(out)
}

/// A stable key for a transaction no declaration covers.
///
/// Prefixed so it cannot collide with a declared mechanism name, and includes the reference
/// so two different undeclared transactions stay distinguishable rather than collapsing into
/// one another — collapsing would hide the very thing the dashboard is meant to show.
fn undeclared_key(txn_type: &str, reference: &str) -> String {
    format!("undeclared:{txn_type}:{reference}")
}
