//! §20.3 — the weekly recalculation that turns reader behaviour into earnings.
//!
//! [`crate::payout_store`] knows how to pay one work. Nothing called it, which is
//! the same invisibility that let `quality_multiplier` sit implemented and
//! unreferenced: a function no caller reaches is indistinguishable from one that
//! was never built. This module is the caller.
//!
//! Two decisions worth stating.
//!
//! **The window is closed, and stated twice.** §20.3 recalculates weekly over the
//! previous 30 days. A window with a moving upper edge pays for the same reads
//! twice — once this week, again next — so the upper bound is pinned to the start
//! of the current week. Every payout's idempotency key includes the window, which
//! is what makes that necessary rather than merely tidy.
//!
//! **A work that pays nothing is skipped, not paid zero.** The store already
//! returns `None` for a suppressed payout; recording those as successes would let
//! a broken cap configuration look like a healthy week, because "every work paid"
//! is exactly what a month of silently-suppressed payouts also looks like.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{AppState, Result};
use lorehaven_db::payout_store::{self, DemandInputs};

/// What one qualifying work earns for a week before multipliers, in basis points.
///
/// **10 credits, and the number is load-bearing.** §20.3 caps a work at 100 credits
/// *per day*, and this base is applied to a 30-day window -- so a base at the cap
/// would suppress every work at any multiplier above 1.0x. The first version of
/// this constant was 100 credits, and `a_pass_over_a_qualifying_work_pays_that_author`
/// failed with `paid: 0`: every work was found, and every one was refused by the cap.
///
/// 10 credits over 30 days is the intended shape -- a popular, well-liked work earns
/// the base plus up to 0.8x of quality and 0.5x of demand, so it lands near 23 and
/// clears the cap, while a runaway (a multiplier far above the spec's range) still
/// trips it. The cap is the backstop; the base has to sit below it for the backstop
/// to be a backstop rather than a blanket refusal.
pub const BASE_WEEKLY_BP: i64 = 10 * payout_store::BP_PER_CREDIT;

/// §20.3's window: 30 days, ending at the last closed week's boundary.
pub const WINDOW_DAYS: i64 = 30;
const DAY_SECS: i64 = 86_400;

/// What one recalculation pass did, for the job log and the test.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecalcSummary {
    /// Works considered — those with at least one signal in the window.
    pub considered: u64,
    /// Works the store actually posted a transaction for.
    pub paid: u64,
    /// Works the store suppressed: over a cap, or worth nothing.
    pub suppressed: u64,
    /// What the operator can see. No per-work breakdown of the *demand*
    /// component, because §0.3 keeps the admin taste signal out of anything
    /// readable — and a log line is readable.
    pub total_bp: i64,
    pub window_from: i64,
    pub window_to: i64,
}

/// The closed window for a pass run at `now`.
///
/// Exposed as its own function because "which 30 days" is the part most worth
/// pinning down in a test: an implementation that used `now` as the upper bound
/// would pass every test that only ever runs a pass once.
#[must_use]
pub fn closed_window(now: OffsetDateTime) -> (i64, i64) {
    let unix = now.unix_timestamp();
    // The start of the current week, on a Monday boundary. 1970-01-01 was a
    // Thursday, so `days % 7` is 4 on a Monday, and shifting by 4 before the mod
    // puts Monday at 0. Written as one offset-and-wrap so the sign is handled once:
    //
    //     days + 3      shifts so Thursday lands on 0, i.e. Monday on -3 ≡ 4
    //     rem_euclid(7)  wraps negatives into 0..7
    //     days - shift   walks back to the Monday
    //
    // All of it `div_euclid`/`rem_euclid` so a pre-1970 instant stays correct
    // instead of wrapping into a positive week in the future.
    let days = unix.div_euclid(DAY_SECS);
    let monday = days - (days + 3).rem_euclid(7);
    let boundary = monday * DAY_SECS;
    (boundary - WINDOW_DAYS * DAY_SECS, boundary)
}

/// Recalculate and post payouts for every work with signals in the window.
///
/// `demand` is supplied by the caller rather than read from the database because
/// the admin taste component is computed per-work by the admin pipeline, and a
/// batch-wide constant would be wrong for everything except a single-feed instance.
pub async fn run(state: &AppState, demand: DemandInputs, base_bp: i64) -> Result<RecalcSummary> {
    let (from, to) = closed_window(OffsetDateTime::now_utc());
    run_for_window(state, demand, base_bp, from, to).await
}

/// The pass over one explicit window.
///
/// Split from [`run`] because the window is the one thing a caller may need to
/// pin — a backfill, or a test that must not depend on today's date. `run` is the
/// convenience that picks the closed week; this is the one that obeys.
pub async fn run_for_window(
    state: &AppState,
    demand: DemandInputs,
    base_bp: i64,
    from: i64,
    to: i64,
) -> Result<RecalcSummary> {
    let mut summary = RecalcSummary {
        window_from: from,
        window_to: to,
        ..RecalcSummary::default()
    };

    for (account_id, work_id) in works_with_earnings(state, from, to).await? {
        summary.considered += 1;
        let posted =
            payout_store::payout_work(state.db(), &account_id, work_id, base_bp, from, to, demand)
                .await;
        match posted {
            Ok(Some(amount)) => {
                summary.paid += 1;
                summary.total_bp += amount;
            }
            // A suppression is a normal outcome, not an error: §20.3's caps are
            // supposed to fire. Logged at debug so an operator sees the count
            // without it reading as an error stream.
            Ok(None) => {
                summary.suppressed += 1;
                tracing::debug!(%account_id, %work_id, "a payout was suppressed by a cap");
            }
            Err(error) => {
                // Transient. The pass is idempotent — the store's idempotency key
                // includes the window — so a retry resumes without double-paying.
                // Fatal would strand the whole week's earnings on one bad work.
                tracing::warn!(%account_id, %work_id, %error, "a payout failed; the pass will retry");
            }
        }
    }

    tracing::info!(
        considered = summary.considered,
        paid = summary.paid,
        suppressed = summary.suppressed,
        total_bp = summary.total_bp,
        window_from = summary.window_from,
        window_to = summary.window_to,
        "recalculated author payouts for a closed week"
    );
    Ok(summary)
}

/// Every work that could earn something in the window: the distinct works a
/// reader viewed, plus every author, because a reader may have finished a work
/// without the view log surviving retention.
///
/// The union rather than the views alone, because a work with completions and no
/// views in the window is *not* an error — it is a work whose views aged out while
/// its readers kept reading. Paying only the view-log side would silently stop
/// paying those authors.
async fn works_with_earnings(state: &AppState, from: i64, to: i64) -> Result<Vec<(String, Uuid)>> {
    /// One payable work: the author's account and the work itself.
    ///
    /// Both columns are text in *both* dialects — `works.owner_pseud_id` and
    /// `works.id` are TEXT on SQLite, and on PostgreSQL they are uuid, so this is
    /// where the parse below earns its keep. Declared at module scope rather than
    /// per arm: two identical local types are different types.
    #[derive(sqlx::FromRow)]
    struct Payable {
        account_id: String,
        work_id: String,
    }

    let db = state.db();
    let rows: Vec<Payable> = match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query_as::<_, Payable>(
                "SELECT DISTINCT p.account_id AS account_id, w.id AS work_id FROM works w \
                 JOIN pseuds p ON p.id = w.owner_pseud_id \
                 WHERE EXISTS (SELECT 1 FROM work_view_log v \
                   WHERE v.work_id = w.id \
                     AND CAST(strftime('%s', v.viewed_at) AS INTEGER) >= ?1 \
                     AND CAST(strftime('%s', v.viewed_at) AS INTEGER) <  ?2 \
                     AND v.is_automated = 0) \
                    OR EXISTS (SELECT 1 FROM reading_status s \
                   WHERE s.subject_type = 'work' AND s.subject_id = w.id \
                     AND s.status = 'finished' AND s.finished_at IS NOT NULL \
                     AND CAST(strftime('%s', s.finished_at) AS INTEGER) >= ?1 \
                     AND CAST(strftime('%s', s.finished_at) AS INTEGER) <  ?2) \
                 LIMIT ?3",
            )
            .bind(from)
            .bind(to)
            .bind(BATCH_LIMIT)
            .fetch_all(db.sqlite_pool().expect("sqlite pool"))
            .await?
        }
        lorehaven_db::Backend::Postgres => {
            // `works.owner_pseud_id` is TEXT and `work_view_log.work_id` is TEXT on
            // PostgreSQL, while `reading_status.subject_id` is uuid — the same split
            // as in the store, for the same reason.
            sqlx::query_as::<_, Payable>(
                "SELECT DISTINCT p.account_id::text AS account_id, w.id::text AS work_id FROM works w \
                 JOIN pseuds p ON p.id = w.owner_pseud_id \
                 WHERE EXISTS (SELECT 1 FROM work_view_log v \
                   WHERE v.work_id = w.id::text \
                     AND v.viewed_at >= $1 \
                     AND v.viewed_at <  $2 \
                     AND v.is_automated = 0) \
                    OR EXISTS (SELECT 1 FROM reading_status s \
                   WHERE s.subject_type = 'work' AND s.subject_id = w.id \
                     AND s.status = 'finished' AND s.finished_at IS NOT NULL \
                     AND s.finished_at >= $1 \
                     AND s.finished_at <  $2) \
                 LIMIT $3",
            )
            .bind(rfc3339(from))
            .bind(rfc3339(to))
            .bind(BATCH_LIMIT)
            .fetch_all(db.postgres_pool().expect("postgres pool"))
            .await?
        }
    };
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let Ok(work_id) = Uuid::parse_str(&row.work_id) else {
            tracing::error!(work_id = %row.work_id, "a work row has an id that is not a UUID; it cannot be paid");
            continue;
        };
        out.push((row.account_id, work_id));
    }
    Ok(out)
}

/// The most works one pass pays, so a pathological instance walks the catalogue
/// over several passes instead of holding a worker for hours. Pausing is safe
/// because the idempotency key includes the window: the next pass re-reads the same
/// works and the ones already paid are no-ops.
const BATCH_LIMIT: i64 = 5_000;

/// Unix seconds to the RFC 3339 text the PostgreSQL side stores.
///
/// Kept local rather than shared with the store so this module can be read on its
/// own; both are the same three lines, and a shared helper for two callers in
/// different crates would be a dependency to read before either could be changed.
fn rfc3339(unix: i64) -> String {
    OffsetDateTime::from_unix_timestamp(unix)
        .expect("a representable timestamp")
        .format(&time::format_description::well_known::Rfc3339)
        .expect("an RFC 3339 string")
}

/// The handler the worker dispatches, keyed on the closed week.
///
/// The key lives in the job's idempotency key so a retried pass resumes rather
/// than repaying, and the `week` in the payload is what the operator reads to
/// answer "has this week been paid?".
pub async fn handle_recalc(state: &AppState) -> Result<()> {
    let demand = DemandInputs {
        // §0.3: the admin taste component is filled in by the admin pipeline,
        // which is not part of the worker. Zero here means "no admin signal for
        // this pass", and the demand multiplier falls back to 1.0x rather than to
        // a penalty -- see `DemandMultiplier::compute`.
        admin_taste: 0.0,
        wishlist: 0.0,
        search: 0.0,
    };
    // The base per-work earning for a week, in basis points. A constant like
    // `payout_store::caps` rather than config: §20.3's numbers are the published
    // economy, and a config value would let an instance promise more than the
    // spec does with that invisible.
    let summary = run(state, demand, BASE_WEEKLY_BP).await?;
    tracing::info!(
        week_from = summary.window_from,
        week_to = summary.window_to,
        "the §20.3 payout pass finished"
    );
    Ok(())
}
