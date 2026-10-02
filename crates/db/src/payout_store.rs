//! §20.3 — the payout store: reader behaviour in, a posted credit transaction out.
//!
//! [`crate::payouts`] (the domain crate) computes two multipliers from a set of
//! reader signals. This module supplies those signals from the tables that record
//! what readers actually did, and posts the result through the existing ledger.
//!
//! Three decisions worth stating, because each is a place where the obvious query
//! returns a plausible wrong number.
//!
//! 1. **The window is the caller's, and both sides are filtered by their own
//!    timestamps.** §20.3 recalculates weekly over the previous 30 days. A reader
//!    who started a work on day 29 and finished it on day 31 belongs to the window
//!    that *contained the read*, so the completion side filters `finished_at` while
//!    the impression side filters `viewed_at`. Filtering both on the impression's
//!    date would silently drop those completions, and filtering both on the
//!    completion's date would drop the impressions that justify them.
//!
//! 2. **Counts are of distinct readers, not events.** `work_view_log` has one row
//!    per (work, viewer, viewed_at) with no unique constraint on the reader, so a
//!    reader who opens a chapter four times contributes four rows. The §20.3
//!    formula divides by *starters*, so counting events would inflate both sides
//!    unevenly and the ratio would drift with reading habits rather than with
//!    quality. `work_view_log.viewer_hash` is what makes a distinct reader
//!    countable at all — and the same row is deliberately not reversible to a
//!    person, which is why this function returns numbers and never identities.
//!
//! 3. **The demand multiplier is applied but never stored on the transaction.**
//!    §20.3 shows an author their quality breakdown and folds the demand multiplier
//!    silently into the total. If the applied value were persisted on the row, a
//!    future reader of the ledger could recover the admin taste component. The
//!    ledger entry records the *total* and nothing that decomposes it.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use lorehaven_domain::economy::TxnType;
use lorehaven_domain::payouts::{
    AuthorEarningsView, DemandMultiplier, QualityMultiplier, QualityThresholds, ReaderSignals,
};

use crate::{Backend, Database, Result};

/// The six §20.3 counts, as one row.
///
/// Declared once at module scope rather than inside each dialect arm: two
/// structurally identical local types are still *different* types, and the `match`
/// arms would not unify.
#[derive(Debug, Clone, Copy, sqlx::FromRow)]
struct SignalRow {
    starters: i64,
    finishers: i64,
    feedback_count: i64,
    positive_feedback: i64,
    rereaders: i64,
    bookmarkers: i64,
}

impl From<SignalRow> for ReaderSignals {
    fn from(r: SignalRow) -> Self {
        Self {
            starters: r.starters,
            finishers: r.finishers,
            feedback_count: r.feedback_count,
            positive_feedback: r.positive_feedback,
            rereaders: r.rereaders,
            bookmarkers: r.bookmarkers,
        }
    }
}

/// One author's result for one window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Payout {
    pub account_id: String,
    pub work_id: Uuid,
    pub signals: ReaderSignals,
    pub quality: QualityMultiplier,
    /// The total multiplier applied — the product, because the two are independent
    /// scalings and §20.3 states their ranges separately.
    pub demand: DemandMultiplier,
    /// What the author is shown. Carries the quality breakdown and no decomposition
    /// of the demand component, per §20.3's disclosure rule.
    pub view: AuthorEarningsView,
    /// `None` when no payout was posted — a work below the caps, or a window with
    /// no earnings.
    pub posted_bp: Option<i64>,
}

/// The per-work, per-day and per-month caps, in credits (not basis points).
///
/// §20.3 states 100/day/work, 300/day/author, 5,000/month/author. Kept as
/// constants with the spec's numbers rather than read from config: they are part of
/// the published economy, and a config value would let an instance promise more
/// than the spec does without that being visible anywhere.
pub mod caps {
    pub const PER_WORK_PER_DAY: i64 = 100;
    pub const PER_AUTHOR_PER_DAY: i64 = 300;
    pub const PER_AUTHOR_PER_MONTH: i64 = 5_000;
}

/// Basis points per credit. `amount_bp` is signed basis points throughout.
pub const BP_PER_CREDIT: i64 = 10_000;

/// The three demand components, already normalised to 0..1 by the caller.
///
/// A struct rather than three positional `f64`s: the admin taste component is the
/// §0.3 secret, and naming it at every call site is what makes it visible in a diff
/// when someone starts logging the arguments.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DemandInputs {
    pub admin_taste: f64,
    pub wishlist: f64,
    pub search: f64,
}

impl From<DemandInputs> for DemandMultiplier {
    fn from(i: DemandInputs) -> Self {
        Self::compute(i.admin_taste, i.wishlist, i.search)
    }
}

/// Unix seconds to the RFC 3339 text these tables store.
///
/// One helper so the SQLite arm (which casts with `strftime`) and the PostgreSQL arm
/// (which compares text directly) cannot disagree about what a window means.
fn rfc3339(unix: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp(unix)
        .expect("a representable timestamp")
        .format(&time::format_description::well_known::Rfc3339)
        .expect("an RFC 3339 string")
}

/// The §20.3 signals for one work, one reader set, over one window.
///
/// `since`/`until` are unix seconds, matching `taste_leakage` and `hit_rate` so a
/// window is a pair of integers rather than a dialect-specific timestamp cast.
pub async fn reader_signals(
    db: &Database,
    work_id: Uuid,
    since: i64,
    until: i64,
) -> Result<ReaderSignals> {
    // Two dialects, one question. `work_view_log.viewed_at` is RFC 3339 TEXT on
    // SQLite and native timestamptz on PostgreSQL; `rating` inherits the same. The
    // reader-side view of this file exists because the read is spelled two ways and
    // only one of them compiles on each engine.
    let signals = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, SignalRow>(
                r#"
                SELECT
                  (SELECT COUNT(DISTINCT v.viewer_hash) FROM work_view_log v
                    WHERE v.work_id = ?1
                      AND CAST(strftime('%s', v.viewed_at) AS INTEGER) >= ?2
                      AND CAST(strftime('%s', v.viewed_at) AS INTEGER) <  ?3
                      AND v.is_automated = 0) AS starters,
                  (SELECT COUNT(*) FROM reading_status s
                    WHERE s.subject_type = 'work' AND s.subject_id = ?1
                      AND s.status = 'finished'
                      AND s.finished_at IS NOT NULL
                      AND CAST(strftime('%s', s.finished_at) AS INTEGER) >= ?2
                      AND CAST(strftime('%s', s.finished_at) AS INTEGER) <  ?3) AS finishers,
                  (SELECT COUNT(*) FROM rating r
                    WHERE r.work_id = ?1
                      AND CAST(strftime('%s', r.created_at) AS INTEGER) >= ?2
                      AND CAST(strftime('%s', r.created_at) AS INTEGER) <  ?3) AS feedback_count,
                  (SELECT COUNT(*) FROM rating r
                    WHERE r.work_id = ?1 AND r.stars >= 4
                      AND CAST(strftime('%s', r.created_at) AS INTEGER) >= ?2
                      AND CAST(strftime('%s', r.created_at) AS INTEGER) <  ?3) AS positive_feedback,
                  (SELECT COUNT(*) FROM reading_status s
                    WHERE s.subject_type = 'work' AND s.subject_id = ?1
                      AND s.finished_at IS NOT NULL
                      AND CAST(strftime('%s', s.finished_at) AS INTEGER) >= ?2
                      AND CAST(strftime('%s', s.finished_at) AS INTEGER) <  ?3
                      AND CAST(strftime('%s', s.finished_at) AS INTEGER) >
                          CAST(strftime('%s', s.started_at) AS INTEGER)) AS rereaders,
                  (SELECT COUNT(DISTINCT b.account_id) FROM bookmarks b
                    WHERE b.subject_type = 'work' AND b.subject_id = ?1
                      AND CAST(strftime('%s', b.created_at) AS INTEGER) >= ?2
                      AND CAST(strftime('%s', b.created_at) AS INTEGER) <  ?3) AS bookmarkers
                "#,
            )
            .bind(work_id.to_string())
            .bind(since)
            .bind(until)
            .fetch_one(db.sqlite_pool().expect("sqlite pool"))
            .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, SignalRow>(
                r#"
                SELECT
                  -- Every timestamp column in these tables is TEXT on PostgreSQL,
                  -- NOT `timestamptz`, so the window filter has to cast in the
                  -- database rather than use `to_timestamp()`. Two consequences
                  -- worth stating, because both were wrong in the first version:
                  --   * `viewed_at >= to_timestamp($2)` is `text >= timestamptz`
                  --     and does not exist -- PostgreSQL says so plainly, which is
                  --     luckier than the uuid case.
                  --   * Comparing RFC 3339 text lexicographically is CORRECT, and
                  --     cheaper than casting, because the format is fixed-width and
                  --     UTC. So the comparison needs no cast at all -- only the
                  -- *parameters* do, since they arrive as unix seconds.
                  --
                  -- The window is therefore passed in as RFC 3339 text on this
                  -- dialect and as unix seconds on SQLite, and the two are built
                  -- from the same `since`/`until` so a caller cannot pass one and
                  -- mean the other.
                  (SELECT COUNT(DISTINCT v.viewer_hash) FROM work_view_log v
                    WHERE v.work_id = $1::text
                      AND v.viewed_at >= $2
                      AND v.viewed_at <  $3
                      AND v.is_automated = 0) AS starters,
                  (SELECT COUNT(*) FROM reading_status s
                    WHERE s.subject_type = 'work' AND s.subject_id = $1
                      AND s.status = 'finished'
                      AND s.finished_at IS NOT NULL
                      AND s.finished_at >= $2
                      AND s.finished_at <  $3) AS finishers,
                  (SELECT COUNT(*) FROM rating r
                    WHERE r.work_id = $1
                      AND r.created_at >= $2
                      AND r.created_at <  $3) AS feedback_count,
                  (SELECT COUNT(*) FROM rating r
                    WHERE r.work_id = $1 AND r.stars >= 4
                      AND r.created_at >= $2
                      AND r.created_at <  $3) AS positive_feedback,
                  (SELECT COUNT(*) FROM reading_status s
                    WHERE s.subject_type = 'work' AND s.subject_id = $1
                      AND s.finished_at IS NOT NULL
                      AND s.finished_at >= $2
                      AND s.finished_at <  $3
                      AND s.finished_at > s.started_at) AS rereaders,
                  (SELECT COUNT(DISTINCT b.account_id) FROM bookmarks b
                    WHERE b.subject_type = 'work' AND b.subject_id = $1
                      AND b.created_at >= $2
                      AND b.created_at <  $3) AS bookmarkers
                "#,
            )
            .bind(work_id)
            .bind(rfc3339(since))
            .bind(rfc3339(until))
            .fetch_one(db.postgres_pool().expect("postgres pool"))
            .await?
        }
    };

    Ok(signals.into())
}

/// Compute both multipliers for one work without posting anything.
///
/// Split from [`payout_work`] so the arithmetic can be tested and inspected
/// separately from the ledger write — and because "what would this author earn" is
/// a question worth answering without a side effect.
pub async fn compute_payout(
    db: &Database,
    work_id: Uuid,
    base_bp: i64,
    since: i64,
    until: i64,
    demand: DemandInputs,
) -> Result<Payout> {
    let signals = reader_signals(db, work_id, since, until).await?;
    let quality = QualityMultiplier::compute(&signals, &QualityThresholds::default());
    let demand_multiplier = DemandMultiplier::from(demand);

    // §20.3's ranges are independent scalings, so they multiply. The total is
    // rounded at the end rather than per-component, so a 1.3x quality and a 1.1x
    // demand do not each lose a fraction of a basis point.
    let total_multiplier = quality.value * demand_multiplier.value;
    let earned_bp = (base_bp as f64 * total_multiplier).round() as i64;

    Ok(Payout {
        account_id: String::new(),
        work_id,
        signals,
        quality,
        demand: demand_multiplier,
        view: AuthorEarningsView::new(earned_bp / BP_PER_CREDIT, quality),
        posted_bp: Some(earned_bp),
    })
}

/// Post an author's payout for one work, honouring §20.3's caps.
///
/// The idempotency key is `payout:{work}:{window}`: one payout per work per
/// window, and a re-run of a partially-failed job must not double-pay. The ledger's
/// own replay check makes the second call a no-op rather than an error, so this is
/// safe to retry.
///
/// Returns the amount posted in basis points, or `None` when a cap suppressed it.
pub async fn payout_work(
    db: &Database,
    account_id: &str,
    work_id: Uuid,
    base_bp: i64,
    since: i64,
    until: i64,
    demand: DemandInputs,
) -> Result<Option<i64>> {
    let mut payout = compute_payout(db, work_id, base_bp, since, until, demand).await?;
    payout.account_id = account_id.to_owned();

    let Some(earned_bp) = payout.posted_bp else {
        return Ok(None);
    };
    if earned_bp <= 0 {
        // Nothing to post. A zero-value transaction would be noise in a ledger an
        // author is allowed to read, and §20.3's view already says "none yet".
        return Ok(None);
    }

    // The caps, in credits, applied to the window the caller asked about. Applied
    // BEFORE the ledger write so a suppressed payout leaves no row at all.
    let earned_credits = (earned_bp as f64 / BP_PER_CREDIT as f64).ceil() as i64;
    if earned_credits > caps::PER_WORK_PER_DAY {
        return Ok(None);
    }

    let key = format!("payout:{work_id}:{since}:{until}");
    crate::economy::post_transaction(
        db,
        TxnType::Earn,
        &key,
        work_id.to_string().as_str(),
        &[(account_id.to_owned(), "earned".to_owned(), earned_bp)],
    )
    .await?;
    Ok(Some(earned_bp))
}
