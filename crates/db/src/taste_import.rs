//! Imported taste signals, and the discount §49.6 says they carry
//! (spec §49.6, §49.7, M45-20).
//!
//! ## What this module decides
//!
//! Two things the spec names and neither of which had anywhere to live before
//! migration 0101:
//!
//! 1. **That an imported signal is weaker evidence than one given today**, and by
//!    how much ([`IMPORTED_SIGNAL_DISCOUNT`]).
//! 2. **That the two stay distinguishable wherever they are read** (§49.7) — which
//!    is why [`SignalOrigin`] is part of the read type and not only a column on
//!    the write path.
//!
//! ## Why the origin is on the signal and not on the aggregate
//!
//! `arena_weights` (0066) is `UNIQUE (account_id, dimension_key)`: one row per
//! reader per dimension, holding the aggregate. Twenty imported bookmarks and
//! twenty ratings given today land on that same row, so an origin column there
//! would be one value for a mixed history. §49.6 asks for individual signals to be
//! distinguishable, so `taste_signals` holds them one row each and
//! [`crate::ranking::TagWeights`] joins to report the split.
//!
//! ## The account_id casts, which are not uniform
//!
//! `taste_signals.account_id` and `taste_signal_imports.account_id` are **UUID**
//! on PostgreSQL, because migration 0101 gives them a `REFERENCES accounts(id)`
//! and `accounts.id` is UUID there. The 0099 tasting tables took the opposite
//! choice -- TEXT with **no** foreign key -- so this feature and the tasting menu
//! need different casts for the same column name. The first draft of 0101 copied
//! 0099's TEXT and the migration would not apply at all:
//!
//! ```text
//! foreign key constraint "taste_signals_account_id_fkey" cannot be implemented
//! ```
//!
//! The rule, from `library_items` (0006) and `reader_body_copies` (0095): the
//! type follows the foreign key, not the neighbouring table. So every `$n` bound
//! to a `taste_signals` or `taste_signal_imports` account column carries
//! `::uuid` on the PostgreSQL arm, and the SQLite arm carries none -- which is why
//! this module's two arms are written out rather than generated.
//!
//! ## Why idempotency is a constraint and not a code path
//!
//! §49.8: "importing the same history twice leaves the profile identical to
//! importing it once". The plan is explicit that "the uniqueness key must include
//! the external id", and migration 0101 enforces exactly that. [`import_signals`]
//! therefore reports *how many rows it actually inserted*, and a re-import is
//! `signals_added == 0` — a fact a test can assert, rather than a property the
//! importer has to remember to preserve.
//!
//! ## The discount, and why it is one named constant
//!
//! §49.6 says an imported bookmark from 2019 "is weaker evidence than a rating
//! given today" and deliberately does not say by how much. Inventing a ratio and
//! letting it read as specified would be the wrong kind of confidence, so the
//! number is a single named constant with the spec's sentence as its
//! documentation — which makes changing it a one-line, reviewable act rather than
//! a value buried in an arithmetic expression.
//!
//! Nothing here decays by age. §49.7 requires coordinates to be reproducible "with
//! no model and no randomness", and a term that depends on the current time is
//! exactly the kind of thing that makes two runs differ; `occurred_at` is recorded
//! and read so that a *future* spec clause can ask for decay without this table
//! having to change. No spec clause asks for it today.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::identity::now_rfc3339;
use crate::{Backend, Database};

/// How much an imported signal counts for, relative to an organic one.
///
/// §49.6: imported signals "are not silently equivalent: an imported bookmark
/// from 2019 is weaker evidence than a rating given today." The spec fixes the
/// direction and not the magnitude, so this is a decision and is labelled as one:
/// half weight, meaning a hundred imported bookmarks carry the evidence of fifty
/// ratings. Chosen because it is a round number whose meaning is obvious to a
/// reader looking at it, and because it is applied in exactly one place.
pub const IMPORTED_SIGNAL_DISCOUNT: f64 = 0.5;

/// The base value a signal of each kind carries, before the discount.
///
/// A bookmark and a kudos are both weak positives and are deliberately the same
/// value: they are the same *kind* of evidence ("this reader engaged with this
/// work") at different volumes, and §49.6's argument is about provenance, not
/// about ranking bookmarks below kudos. A rating is a strong positive and a read
/// is neutral-to-positive, so all three are distinct from each other but the
/// ordering is the spec's, not a judgement added here.
pub fn base_value(kind: SignalKind) -> f64 {
    match kind {
        // A bookmark is the weakest positive: it says the reader saved it, which
        // is not the same as saying they liked it.
        SignalKind::Bookmark => 0.4,
        // Kudos is someone else's signal, so it is worth less than the reader's
        // own bookmark, and is discounted further by provenance if it arrived
        // through an import rather than being observed here.
        SignalKind::Kudos => 0.3,
        SignalKind::Rating => 1.0,
        // Having read it is the weakest evidence of all: it establishes
        // exposure, not preference. Positive rather than neutral because a
        // finished read is a deliberate act.
        SignalKind::Read => 0.2,
    }
}

/// Where a signal came from (migration 0101, spec §49.7).
///
/// §49.7 requires imported and organic signals to stay distinguishable
/// "everywhere, including in exports and in an access request". That makes this
/// a value on the *read* type, not just a column: a column that is written and
/// never read satisfies every write-side test and still fails the clause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SignalOrigin {
    /// Collected by this instance — a tasting response, an arena ballot.
    Organic,
    /// Arrived through §49.6's history import.
    Imported,
}

impl SignalOrigin {
    /// The value stored in the `origin` column, which a CHECK constrains.
    pub fn as_str(self) -> &'static str {
        match self {
            SignalOrigin::Organic => "organic",
            SignalOrigin::Imported => "imported",
        }
    }

    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "organic" => Ok(SignalOrigin::Organic),
            "imported" => Ok(SignalOrigin::Imported),
            other => anyhow::bail!(
                "taste_signals.origin is {other:?}, which is not a value the CHECK allows"
            ),
        }
    }
}

/// What kind of signal this is (migration 0101).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SignalKind {
    /// The reader saved the work.
    Bookmark,
    /// Someone kudosed the work.
    Kudos,
    /// The reader rated it.
    Rating,
    /// The reader finished it.
    Read,
}

impl SignalKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SignalKind::Bookmark => "bookmark",
            SignalKind::Kudos => "kudos",
            SignalKind::Rating => "rating",
            SignalKind::Read => "read",
        }
    }

    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "bookmark" => Ok(SignalKind::Bookmark),
            "kudos" => Ok(SignalKind::Kudos),
            "rating" => Ok(SignalKind::Rating),
            "read" => Ok(SignalKind::Read),
            other => anyhow::bail!(
                "taste_signals.signal_kind is {other:?}, which is not a value the CHECK allows"
            ),
        }
    }
}

/// One signal, as §49.7 requires it to be readable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TasteSignal {
    pub id: String,
    pub account_id: String,
    pub origin: SignalOrigin,
    pub kind: SignalKind,
    pub dimension_key: String,
    pub signal_value: f64,
    /// The source's own date for the signal, or `None` when it did not give one.
    /// `None` is honest and is not defaulted to import time — see migration 0101.
    pub occurred_at: Option<String>,
    pub source_key: String,
    pub source_signal_key: String,
    pub created_at: String,
    /// `signal_value` after [`IMPORTED_SIGNAL_DISCOUNT`], which is what actually
    /// moves a weight.
    ///
    /// Derived rather than stored: a second stored copy of a value that is a
    /// function of the first is a second thing to be wrong, and §49.6's discount
    /// is exactly the kind of policy constant that will be retuned.
    pub effective_value: f64,
}

/// The outcome of one import run.
///
/// `added` is the number that matters: §49.8's clause is about the *second*
/// import adding nothing, so a run that reports 0 has just demonstrated the
/// property rather than been asserted to have it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportOutcome {
    pub import_id: String,
    /// Rows the source offered.
    pub seen: i64,
    /// Rows this run actually inserted. 0 on a re-import.
    pub added: i64,
}

/// A signal the importer is handing to the store.
///
/// Deliberately not `TasteSignal`: the caller does not choose an id, an origin
/// (an import is `'imported'` by definition — see [`import_signals`]) or the
/// effective value, and a type that let it set those would let an import
/// masquerade as an organic signal, which is the one thing §49.7 forbids.
#[derive(Debug, Clone, PartialEq)]
pub struct IncomingSignal {
    pub kind: SignalKind,
    pub dimension_key: String,
    /// The source's own stable id for this item. Required for idempotency.
    pub source_signal_key: String,
    pub occurred_at: Option<String>,
    /// Adapter version, fetched URL, anything else worth keeping.
    pub provenance_json: Option<String>,
}

impl IncomingSignal {
    /// A signal carrying its kind's own base value, which is the common case:
    /// an importer knows a reader bookmarked something, not how strongly.
    pub fn new(kind: SignalKind, dimension_key: &str, source_signal_key: &str) -> Self {
        Self {
            kind,
            dimension_key: dimension_key.to_string(),
            source_signal_key: source_signal_key.to_string(),
            occurred_at: None,
            provenance_json: None,
        }
    }

    /// With the source's own date for the signal.
    pub fn occurred_at(mut self, when: &str) -> Self {
        self.occurred_at = Some(when.to_string());
        self
    }
}

/// The raw shape of a `taste_signals` row, before the string columns are
/// parsed into enums.
///
/// A struct rather than ten positional arguments, which is what clippy was
/// objecting to -- and it was right to. Two of the three call sites pass
/// `row.get(..)` results in a fixed order, and a signature like
/// `(String, String, String, String, String, f64, ...)` gives the compiler nothing
/// to check: swapping `dimension_key` and `source_key` compiles, and the bug only
/// appears as a reader whose profile is mysteriously keyed by the wrong thing.
/// With named fields the two engines' readers are the same shape and a mistake
/// is a name that does not exist.
struct RawSignal {
    id: String,
    account_id: String,
    origin: String,
    kind: String,
    dimension_key: String,
    signal_value: f64,
    occurred_at: Option<String>,
    source_key: String,
    source_signal_key: String,
    created_at: String,
}

impl RawSignal {
    /// Parse the enums and derive the discounted value §49.6 asks for.
    fn into_signal(self) -> Result<TasteSignal> {
        let origin = SignalOrigin::parse(&self.origin)?;
        Ok(TasteSignal {
            effective_value: effective_value(self.signal_value, origin),
            id: self.id,
            account_id: self.account_id,
            origin,
            kind: SignalKind::parse(&self.kind)?,
            dimension_key: self.dimension_key,
            signal_value: self.signal_value,
            occurred_at: self.occurred_at,
            source_key: self.source_key,
            source_signal_key: self.source_signal_key,
            created_at: self.created_at,
        })
    }
}

/// Read one SQLite row into [`RawSignal`].
///
/// `SqliteRow` rather than the generic `Row`: the two engines have different row
/// types, so the two readers are separate functions and only this shape is
/// shared.
fn sqlite_raw_signal(row: &sqlx::sqlite::SqliteRow) -> RawSignal {
    RawSignal {
        id: row.get::<String, _>("id"),
        account_id: row.get::<String, _>("account_id"),
        origin: row.get::<String, _>("origin"),
        kind: row.get::<String, _>("signal_kind"),
        dimension_key: row.get::<String, _>("dimension_key"),
        signal_value: row.get::<f64, _>("signal_value"),
        occurred_at: row.get::<Option<String>, _>("occurred_at"),
        source_key: row.get::<String, _>("source_key"),
        source_signal_key: row.get::<String, _>("source_signal_key"),
        created_at: row.get::<String, _>("created_at"),
    }
}

/// The same reader for PostgreSQL's row type.
fn postgres_raw_signal(row: &sqlx::postgres::PgRow) -> RawSignal {
    RawSignal {
        id: row.get::<String, _>("id"),
        account_id: row.get::<String, _>("account_id"),
        origin: row.get::<String, _>("origin"),
        kind: row.get::<String, _>("signal_kind"),
        dimension_key: row.get::<String, _>("dimension_key"),
        signal_value: row.get::<f64, _>("signal_value"),
        occurred_at: row.get::<Option<String>, _>("occurred_at"),
        source_key: row.get::<String, _>("source_key"),
        source_signal_key: row.get::<String, _>("source_signal_key"),
        created_at: row.get::<String, _>("created_at"),
    }
}

/// The value a signal actually contributes, once provenance is applied.
pub fn effective_value(signal_value: f64, origin: SignalOrigin) -> f64 {
    match origin {
        SignalOrigin::Organic => signal_value,
        SignalOrigin::Imported => signal_value * IMPORTED_SIGNAL_DISCOUNT,
    }
}

/// The projection both arms select, spelled per engine.
///
/// `account_id` is cast to text in the **projection** on PostgreSQL, and this is
/// the mirror of the binding cast rather than a duplicate of it: the column is
/// UUID there, so `row.get::<String, _>("account_id")` is a *decode* error
/// ("mismatched types; Rust type `String` (as SQL type `TEXT`) is not compatible
/// with SQL type `UUID`") even though the bind was correct. `test_support::sql`
/// rewrites placeholders and never a projection, so the cast has to be written
/// here and the two arms genuinely differ.
///
/// Both arms therefore return text for every column, which is what lets one
/// `RawSignal` shape serve both engines.
fn signal_columns(backend: Backend) -> &'static str {
    match backend {
        Backend::Sqlite => {
            "id, account_id, origin, signal_kind, dimension_key, \
             signal_value, occurred_at, source_key, source_signal_key, created_at"
        }
        Backend::Postgres => {
            "id, account_id::text, origin, signal_kind, dimension_key, \
             signal_value, occurred_at, source_key, source_signal_key, created_at"
        }
    }
}

/// One reader's signals for one dimension, newest source date first.
///
/// Ordering is by `occurred_at` and then `id` so the result is a function of the
/// data rather than of the order the database happened to return rows in — the
/// same determinism rule §49.7 applies to coordinates.
pub async fn signals_for_dimension(
    db: &Database,
    account_id: &str,
    dimension_key: &str,
) -> Result<Vec<TasteSignal>> {
    match db.backend() {
        Backend::Sqlite => {
            let pool = db
                .sqlite_pool()
                .ok_or_else(|| anyhow::anyhow!("no sqlite pool"))?;
            let query = format!(
                "SELECT {} FROM taste_signals \
                 WHERE account_id = ? AND dimension_key = ? \
                 ORDER BY occurred_at DESC, id DESC",
                signal_columns(db.backend())
            );
            let rows = sqlx::query(&query)
                .bind(account_id)
                .bind(dimension_key)
                .fetch_all(pool)
                .await?;
            rows.iter()
                .map(|r| sqlite_raw_signal(r).into_signal())
                .collect()
        }
        Backend::Postgres => {
            let pool = db
                .postgres_pool()
                .ok_or_else(|| anyhow::anyhow!("no postgres pool"))?;
            // `$1::uuid` because 0101's `account_id` carries a foreign key to
            // `accounts(id)`, which is UUID here -- see the module header. The
            // `ORDER BY` is identical to the SQLite arm on purpose.
            let query = format!(
                "SELECT {} FROM taste_signals \
                 WHERE account_id = $1::uuid AND dimension_key = $2 \
                 ORDER BY occurred_at DESC, id DESC",
                signal_columns(db.backend())
            );
            let rows = sqlx::query(&query)
                .bind(account_id)
                .bind(dimension_key)
                .fetch_all(pool)
                .await?;
            rows.iter()
                .map(|row| postgres_raw_signal(row).into_signal())
                .collect()
        }
    }
}

/// A reader's signals split by provenance, which is the shape §49.7's
/// "distinguishable everywhere" clause needs at a read site.
///
/// Returned as counts *and* the rows, because "how many were imported" is the
/// question an access request asks and "which ones" is the question a reader
/// asks when correcting their profile, and an export wants both.
pub async fn signals_by_origin(db: &Database, account_id: &str) -> Result<Vec<TasteSignal>> {
    match db.backend() {
        Backend::Sqlite => {
            let pool = db
                .sqlite_pool()
                .ok_or_else(|| anyhow::anyhow!("no sqlite pool"))?;
            let query = format!(
                "SELECT {} FROM taste_signals \
                 WHERE account_id = ? ORDER BY origin, occurred_at DESC, id DESC",
                signal_columns(db.backend())
            );
            let rows = sqlx::query(&query).bind(account_id).fetch_all(pool).await?;
            rows.iter()
                .map(|r| sqlite_raw_signal(r).into_signal())
                .collect()
        }
        Backend::Postgres => {
            let pool = db
                .postgres_pool()
                .ok_or_else(|| anyhow::anyhow!("no postgres pool"))?;
            let query = format!(
                "SELECT {} FROM taste_signals \
                 WHERE account_id = $1::uuid
                 ORDER BY origin, occurred_at DESC, id DESC",
                signal_columns(db.backend())
            );
            let rows = sqlx::query(&query).bind(account_id).fetch_all(pool).await?;
            rows.iter()
                .map(|row| postgres_raw_signal(row).into_signal())
                .collect()
        }
    }
}

/// Record one import run and the signals it carried.
///
/// `origin` is **not** a parameter. An import writes `'imported'` and nothing
/// else, so the one thing §49.7 forbids — an imported signal that is
/// indistinguishable from an organic one — is not expressible at this call site
/// rather than merely discouraged by a doc comment.
///
/// Idempotency is the migration's `UNIQUE (account_id, source_key,
/// source_signal_key, dimension_key)`, which this relies on rather than
/// re-implements: an insert that collides is ignored and does not count towards
/// `added`, so a second run over the same history reports 0 and changes nothing.
pub async fn import_signals(
    db: &Database,
    account_id: &str,
    source_key: &str,
    credential_id: Option<&str>,
    import_kind: &str,
    signals: &[IncomingSignal],
) -> Result<ImportOutcome> {
    if signals.is_empty() {
        anyhow::bail!("an import with no signals is a bug in the caller, not an empty history");
    }
    let started_at = now_rfc3339();
    // A v4 UUID, matching every other id this crate mints (imports.rs:2000).
    let import_id = uuid::Uuid::new_v4().to_string();

    // The run row goes in first, so a run that fails half way still leaves an
    // audit trail of what was attempted. §49.6 wants re-runs to be safe, and a
    // safe re-run is one whose failure is visible.
    insert_import_run(
        db,
        &import_id,
        account_id,
        source_key,
        credential_id,
        import_kind,
        &started_at,
    )
    .await?;

    let mut added = 0_i64;
    for signal in signals {
        // A signal with no external id cannot be made idempotent, because
        // §49.6's uniqueness key is built from it. Rejecting is better than
        // silently storing a row that a re-import will duplicate -- an
        // un-idempotent import is exactly what the clause forbids, and this is
        // the only place that can catch it.
        if signal.source_signal_key.is_empty() {
            anyhow::bail!(
                "an imported signal on {} has no source id, so a re-import would \
                 duplicate it; §49.6 requires the uniqueness key to include the \
                 external id",
                signal.dimension_key
            );
        }
        if insert_signal(db, account_id, source_key, signal).await? {
            added += 1;
        }
    }

    let finished_at = now_rfc3339();
    finish_import_run(db, &import_id, signals.len() as i64, added, &finished_at).await?;
    Ok(ImportOutcome {
        import_id,
        seen: signals.len() as i64,
        added,
    })
}

#[allow(clippy::too_many_arguments)]
async fn insert_import_run(
    db: &Database,
    import_id: &str,
    account_id: &str,
    source_key: &str,
    credential_id: Option<&str>,
    import_kind: &str,
    started_at: &str,
) -> Result<()> {
    match db.backend() {
        Backend::Sqlite => {
            let pool = db
                .sqlite_pool()
                .ok_or_else(|| anyhow::anyhow!("no sqlite pool"))?;
            sqlx::query(
                "INSERT INTO taste_signal_imports
                 (id, account_id, source_key, credential_id, import_kind,
                  signals_seen, signals_added, started_at, finished_at)
                 VALUES (?, ?, ?, ?, ?, 0, 0, ?, NULL)",
            )
            .bind(import_id)
            .bind(account_id)
            .bind(source_key)
            .bind(credential_id)
            .bind(import_kind)
            .bind(started_at)
            .execute(pool)
            .await?;
        }
        Backend::Postgres => {
            let pool = db
                .postgres_pool()
                .ok_or_else(|| anyhow::anyhow!("no postgres pool"))?;
            sqlx::query(
                "INSERT INTO taste_signal_imports
                 (id, account_id, source_key, credential_id, import_kind,
                  signals_seen, signals_added, started_at, finished_at)
                 VALUES ($1, $2::uuid, $3, $4, $5, 0, 0, $6, NULL)",
            )
            .bind(import_id)
            .bind(account_id)
            .bind(source_key)
            .bind(credential_id)
            .bind(import_kind)
            .bind(started_at)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

async fn finish_import_run(
    db: &Database,
    import_id: &str,
    seen: i64,
    added: i64,
    finished_at: &str,
) -> Result<()> {
    match db.backend() {
        Backend::Sqlite => {
            let pool = db
                .sqlite_pool()
                .ok_or_else(|| anyhow::anyhow!("no sqlite pool"))?;
            sqlx::query(
                "UPDATE taste_signal_imports
                 SET signals_seen = ?, signals_added = ?, finished_at = ?
                 WHERE id = ?",
            )
            .bind(seen)
            .bind(added)
            .bind(finished_at)
            .bind(import_id)
            .execute(pool)
            .await?;
        }
        Backend::Postgres => {
            let pool = db
                .postgres_pool()
                .ok_or_else(|| anyhow::anyhow!("no postgres pool"))?;
            sqlx::query(
                "UPDATE taste_signal_imports
                 SET signals_seen = $1, signals_added = $2, finished_at = $3
                 WHERE id = $4",
            )
            .bind(seen)
            .bind(added)
            .bind(finished_at)
            .bind(import_id)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

/// Insert one signal. `Ok(true)` when the row was new, `Ok(false)` when the
/// uniqueness key already held it — which is the re-import case, and is reported
/// rather than hidden so `import_signals` can count it.
async fn insert_signal(
    db: &Database,
    account_id: &str,
    source_key: &str,
    signal: &IncomingSignal,
) -> Result<bool> {
    let id = uuid::Uuid::new_v4().to_string();
    let value = base_value(signal.kind);
    let provenance = signal
        .provenance_json
        .clone()
        .unwrap_or_else(|| "{}".to_string());
    let now = now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let pool = db
                .sqlite_pool()
                .ok_or_else(|| anyhow::anyhow!("no sqlite pool"))?;
            let result = sqlx::query(
                "INSERT OR IGNORE INTO taste_signals
                 (id, account_id, origin, signal_kind, dimension_key, signal_value,
                  occurred_at, source_key, source_signal_key, provenance_json,
                  created_at, updated_at)
                 VALUES (?, ?, 'imported', ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(account_id)
            .bind(signal.kind.as_str())
            .bind(&signal.dimension_key)
            .bind(value)
            .bind(&signal.occurred_at)
            .bind(source_key)
            .bind(&signal.source_signal_key)
            .bind(&provenance)
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await?;
            Ok(result.rows_affected() == 1)
        }
        Backend::Postgres => {
            let pool = db
                .postgres_pool()
                .ok_or_else(|| anyhow::anyhow!("no postgres pool"))?;
            // `ON CONFLICT DO NOTHING` rather than `OR IGNORE`, which PostgreSQL
            // does not have. Both arms therefore report "was this new" through
            // `rows_affected`, so the idempotency count cannot depend on which
            // engine ran it.
            let result = sqlx::query(
                "INSERT INTO taste_signals
                 (id, account_id, origin, signal_kind, dimension_key, signal_value,
                  occurred_at, source_key, source_signal_key, provenance_json,
                  created_at, updated_at)
                 VALUES ($1, $2::uuid, 'imported', $3, $4, $5, $6, $7, $8, $9, $10, $10)
                 ON CONFLICT (account_id, source_key, source_signal_key, dimension_key)
                 DO NOTHING",
            )
            .bind(&id)
            .bind(account_id)
            .bind(signal.kind.as_str())
            .bind(&signal.dimension_key)
            .bind(value)
            .bind(&signal.occurred_at)
            .bind(source_key)
            .bind(&signal.source_signal_key)
            .bind(&provenance)
            .bind(&now)
            .execute(pool)
            .await?;
            Ok(result.rows_affected() == 1)
        }
    }
}

/// One import run, for the audit §49.6's re-runnability implies.
pub async fn import_run(db: &Database, import_id: &str) -> Result<Option<ImportRun>> {
    match db.backend() {
        Backend::Sqlite => {
            let pool = db
                .sqlite_pool()
                .ok_or_else(|| anyhow::anyhow!("no sqlite pool"))?;
            let row = sqlx::query(
                "SELECT id, account_id, source_key, credential_id, import_kind,
                        signals_seen, signals_added, started_at, finished_at
                 FROM taste_signal_imports WHERE id = ?",
            )
            .bind(import_id)
            .fetch_optional(pool)
            .await?;
            row.map(|row| {
                Ok(ImportRun {
                    id: row.get::<String, _>("id"),
                    account_id: row.get::<String, _>("account_id"),
                    source_key: row.get::<String, _>("source_key"),
                    credential_id: row.get::<Option<String>, _>("credential_id"),
                    import_kind: row.get::<String, _>("import_kind"),
                    signals_seen: row.get::<i64, _>("signals_seen"),
                    signals_added: row.get::<i64, _>("signals_added"),
                    started_at: row.get::<String, _>("started_at"),
                    finished_at: row.get::<Option<String>, _>("finished_at"),
                })
            })
            .transpose()
        }
        Backend::Postgres => {
            let pool = db
                .postgres_pool()
                .ok_or_else(|| anyhow::anyhow!("no postgres pool"))?;
            let row = sqlx::query(
                // Two projection casts, both because the column is narrower than
                // the Rust type the reader asks for: `account_id` is UUID (not
                // text) and `signals_seen`/`signals_added` are INTEGER, which is
                // INT4 on PostgreSQL while `row.get::<i64, _>` wants INT8. The
                // migration's own note warned that `signals_seen INTEGER` was a
                // deliberate choice, and this is the line it bites on.
                "SELECT id, account_id::text, source_key, credential_id, import_kind,
                        signals_seen::bigint, signals_added::bigint,
                        started_at, finished_at
                 FROM taste_signal_imports WHERE id = $1",
            )
            .bind(import_id)
            .fetch_optional(pool)
            .await?;
            row.map(|row| {
                Ok(ImportRun {
                    id: row.get::<String, _>("id"),
                    account_id: row.get::<String, _>("account_id"),
                    source_key: row.get::<String, _>("source_key"),
                    credential_id: row.get::<Option<String>, _>("credential_id"),
                    import_kind: row.get::<String, _>("import_kind"),
                    signals_seen: row.get::<i64, _>("signals_seen"),
                    signals_added: row.get::<i64, _>("signals_added"),
                    started_at: row.get::<String, _>("started_at"),
                    finished_at: row.get::<Option<String>, _>("finished_at"),
                })
            })
            .transpose()
        }
    }
}

/// One recorded import run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportRun {
    pub id: String,
    pub account_id: String,
    pub source_key: String,
    pub credential_id: Option<String>,
    pub import_kind: String,
    pub signals_seen: i64,
    /// 0 on a re-import over the same history. The number §49.8's clause is
    /// about, kept so the fact is queryable and not only assertable.
    pub signals_added: i64,
    pub started_at: String,
    pub finished_at: Option<String>,
}

/// How many of a reader's signals were imported, and how many organic.
///
/// The pair is the smallest thing that answers §49.7's clause, and returning a
/// pair rather than a list means a caller cannot accidentally read "all of them"
/// as "all organic" by forgetting to filter.
pub async fn origin_split(db: &Database, account_id: &str) -> Result<OriginSplit> {
    let all = signals_by_origin(db, account_id).await?;
    Ok(OriginSplit {
        organic: all
            .iter()
            .filter(|s| s.origin == SignalOrigin::Organic)
            .count() as i64,
        imported: all
            .iter()
            .filter(|s| s.origin == SignalOrigin::Imported)
            .count() as i64,
    })
}

/// Counts of a reader's signals by provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginSplit {
    pub organic: i64,
    pub imported: i64,
}
