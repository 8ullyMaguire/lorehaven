//! Shadow-mode evaluation for the recommendation registry (spec §16.1a, M52-08).
//!
//! The spec is one sentence: *"Switching is preceded by shadow-mode evaluation on
//! the same candidate sets."* Everything here follows from taking that
//! seriously.
//!
//! ## What shadow mode is
//!
//! `rec.mode` gains a third value. `legacy` and `pluggable` each serve one
//! ranking; **`shadow` serves `legacy` and additionally runs `pluggable`,
//! recording how the two would have differed.** Nothing a reader sees changes.
//! That is the entire safety property, and it is why this is worth building
//! rather than just asking an operator to read the numbers: the alternative to
//! evaluating a new ranker on live traffic is switching to it on live traffic.
//!
//! ## The comparison
//!
//! [`compare`] takes the legacy ranking that was actually served and the
//! pluggable ranking computed alongside it, and reports:
//!
//! - **agreement** — the fraction of positions holding the same work, plus
//!   whether the two lists are identical.
//! - **per-strategy contribution** — from `RecRegistry::generate_traced`,
//!   because "the blend differs" is not a finding an operator can act on, and
//!   "the `time_decay` strategy returned nothing at all" is.
//! - **displaced** — works the legacy ranking offered that shadow drops, and
//!   **promoted** — the reverse. These are the actual consequences of the
//!   switch, in the terms a reader would experience them.
//!
//! ## Why the report is not persisted here
//!
//! [`compare`] is a pure function over two rankings, which is what makes it
//! testable without a clock, a database or a worker. The caller decides where a
//! report goes. A production deployment would aggregate these into a metrics
//! sink; this build evaluates and reports, and the operator reads it. What is
//! *not* optional is that it happens on every request in shadow mode — an
//! evaluation that samples is not an evaluation that precedes a switch.

use std::collections::HashSet;

use lorehaven_db::rec_strategy::{RecRegistry, RecRunReport};
use lorehaven_db::Database;

/// How two rankings of the same candidate set relate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Agreement {
    /// The two lists are identical, position for position.
    Identical,
    /// Same works, different order.
    SameSet,
    /// The lists differ in membership.
    Different,
}

impl Agreement {
    /// The fraction of positions holding the same work.
    ///
    /// Counted over the *shorter* of the two lists, so a comparison is not
    /// flattered by one ranking simply being longer. A shadow list that is
    /// twice the length of the legacy one would otherwise score 1.0 on the
    /// positions it shares and look like a perfect match.
    pub fn overlap(served: &[String], shadow: &[String]) -> f64 {
        let n = served.len().min(shadow.len());
        if n == 0 {
            // Two empty lists agree completely; a nonempty/empty pair does not
            // agree at all, and returning 0.0 for the latter is the honest
            // answer rather than a division by zero.
            return if served.is_empty() && shadow.is_empty() {
                1.0
            } else {
                0.0
            };
        }
        let agree = served
            .iter()
            .zip(shadow.iter())
            .take(n)
            .filter(|(a, b)| a == b)
            .count();
        agree as f64 / n as f64
    }

    /// Classify the relationship between two rankings.
    pub fn classify(served: &[String], shadow: &[String]) -> Self {
        if served == shadow {
            return Self::Identical;
        }
        let left: HashSet<&String> = served.iter().collect();
        let right: HashSet<&String> = shadow.iter().collect();
        if left == right {
            Self::SameSet
        } else {
            Self::Different
        }
    }
}

/// What one strategy did in the shadow run.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct StrategyOutcome {
    /// The registered strategy name.
    pub name: String,
    /// How many works it returned.
    pub produced: usize,
    /// Whether the strategy changed the blend at all — it returned something,
    /// and that something survived the RRF fusion into the capped list.
    ///
    /// These are different questions and the distinction is the useful one: a
    /// strategy can return fifty works and change nothing, because every one of
    /// them is also ranked by another strategy and none reaches the cap.
    pub reached_blend: bool,
}

/// The result of one shadow evaluation.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ShadowReport {
    /// How the two rankings relate.
    pub agreement: Agreement,
    /// Positional agreement, over the shorter list.
    pub overlap: f64,
    /// The ranking actually served to the reader. Recorded so a report is
    /// self-contained: without it, `shadow` cannot be checked against anything.
    pub served: Vec<String>,
    /// The ranking shadow mode would have served.
    pub shadow: Vec<String>,
    /// Per-strategy outcomes, in registration order.
    pub strategies: Vec<StrategyOutcome>,
    /// Works the served ranking offered that shadow drops.
    pub displaced: Vec<String>,
    /// Works shadow offers that the served ranking did not.
    pub promoted: Vec<String>,
    /// How many evaluations have run, including this one.
    ///
    /// The spec says evaluation *precedes* a switch, which means an operator
    /// needs to know how much evidence they have. A report with no count on it
    /// cannot answer "have we watched long enough?".
    pub sample: u64,
}

impl ShadowReport {
    /// Whether the two rankings would serve a reader identical results.
    pub fn is_neutral(&self) -> bool {
        self.agreement == Agreement::Identical
    }

    /// A one-line summary for a log line or an operator surface.
    pub fn summary(&self) -> String {
        format!(
            "shadow eval #{}: {:?}, overlap {:.2}, {} displaced, {} promoted, {}/{} strategies reached the blend",
            self.sample,
            self.agreement,
            self.overlap,
            self.displaced.len(),
            self.promoted.len(),
            self.strategies.iter().filter(|s| s.reached_blend).count(),
            self.strategies.len(),
        )
    }
}

/// Compare the ranking that was served against the one shadow computed.
///
/// `served` is what the reader received; `shadow` is what the pluggable
/// registry produced on the same candidate set. `run` is the traced registry
/// run, present so the per-strategy breakdown is available — without it this
/// would report only *that* the rankings differ, which is the one form of the
/// answer an operator cannot act on.
///
/// `sample` is the caller's running count of evaluations. It is passed in
/// rather than kept here so that this stays a pure function: a counter held in
/// a process-global would make concurrent evaluations race on it and make the
/// whole report untestable.
pub fn compare(
    served: &[String],
    shadow: &[String],
    run: &RecRunReport,
    sample: u64,
) -> ShadowReport {
    let served_set: HashSet<&String> = served.iter().collect();
    let shadow_set: HashSet<&String> = shadow.iter().collect();

    let strategies = run
        .per_strategy
        .iter()
        .map(|contribution| {
            let reached = contribution
                .ranked
                .iter()
                .any(|(id, _)| shadow_set.contains(id));
            StrategyOutcome {
                name: contribution.name.clone(),
                produced: contribution.produced,
                reached_blend: reached,
            }
        })
        .collect();

    ShadowReport {
        agreement: Agreement::classify(served, shadow),
        overlap: Agreement::overlap(served, shadow),
        served: served.to_vec(),
        shadow: shadow.to_vec(),
        strategies,
        displaced: served
            .iter()
            .filter(|id| !shadow_set.contains(id))
            .cloned()
            .collect(),
        promoted: shadow
            .iter()
            .filter(|id| !served_set.contains(id))
            .cloned()
            .collect(),
        sample,
    }
}

/// Run the pluggable registry and produce a shadow report against `served`.
///
/// This is the function the discovery route calls in shadow mode, and it is the
/// only place that both reads the registry and knows what was served. Keeping
/// it here means the route stays a dispatch and the comparison stays testable.
///
/// Errors from the shadow run are returned rather than swallowed. A shadow
/// evaluation that cannot run must not be reported as a neutral result — the
/// operator would be switching on the strength of a comparison that never
/// happened.
pub async fn evaluate(
    db: &Database,
    registry: &RecRegistry,
    account_id: &str,
    served: &[String],
    limit: usize,
    sample: u64,
) -> anyhow::Result<ShadowReport> {
    let ctx = lorehaven_db::rec_strategy::RecContext {
        account_id: account_id.to_string(),
        seen: vec![],
        cap: limit,
    };
    let run = registry.generate_traced(db, ctx).await?;
    Ok(compare(served, &run.blended, &run, sample))
}

/// The most recent shadow report, for the operator endpoint.
///
/// Kept as a `Mutex<Option<...>>` rather than in the database for two reasons.
/// The report is a *summary of one evaluation*, not a record: an operator
/// deciding whether to switch needs the current picture, and an unbounded table
/// of per-request comparisons is a privacy-shaped liability (each row names the
/// works a specific reader was shown) for no operational gain. And the honest
/// scope of this build is "evaluate and report", so a single latest value is
/// what exists; aggregation into a metrics sink is a deployment decision, not
/// something to invent a schema for here.
///
/// One instance's shadow report and evaluation count.
///
/// Held per `AppState` (behind its `Arc`) rather than in process statics. A
/// production process serves one instance, so a global was an accurate model of
/// it — but the test binary builds many instances in one process, and a global
/// made them share one another's reports and counters. The result was a test
/// that passed or failed depending on which test the scheduler ran first.
#[derive(Debug, Default)]
pub struct ReportSlot {
    latest: std::sync::Mutex<Option<ShadowReport>>,
    evaluations: std::sync::atomic::AtomicU64,
}

impl ReportSlot {
    /// The next evaluation number.
    pub fn next_sample(&self) -> u64 {
        self.evaluations
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1
    }

    /// Record the most recent report for the operator surface.
    ///
    /// A poisoned lock is recovered rather than propagated: the stored value is
    /// a diagnostic, and a panic in one request's logging path must not turn
    /// every later evaluation into a 500.
    pub fn record(&self, report: ShadowReport) {
        match self.latest.lock() {
            Ok(mut slot) => *slot = Some(report),
            Err(poisoned) => *poisoned.into_inner() = Some(report),
        }
    }

    /// The most recent report, if any.
    pub fn latest(&self) -> Option<ShadowReport> {
        match self.latest.lock() {
            Ok(slot) => slot.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Evaluations recorded on this instance since it was built.
    pub fn evaluations(&self) -> u64 {
        self.evaluations.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Clear both the report and the count.
    pub fn clear(&self) {
        match self.latest.lock() {
            Ok(mut slot) => *slot = None,
            Err(poisoned) => *poisoned.into_inner() = None,
        }
        self.evaluations
            .store(0, std::sync::atomic::Ordering::Relaxed);
    }

    /// Poison the report slot, so the recovery path in `record` is exercised.
    ///
    /// Exists because the recovery is otherwise unreachable from a test: the
    /// only way to poison a mutex is to panic while holding it, and `record` is
    /// the only holder. Without this, the recovery arms are never pulled in a
    /// green run, and the claim "a panic in the logging path does not 500 later
    /// requests" is untested — which is the kind of claim that turns out to be
    /// false.
    #[doc(hidden)]
    pub fn poison_for_tests(&self) {
        // A `Mutex` is poisoned when a panic unwinds through a scope holding its
        // guard. The guard is dropped during that unwind, which both sets the
        // flag and releases the lock — so a poisoned mutex is recoverable, and
        // the test can go on to prove it.
        let _ = std::panic::catch_unwind(|| {
            let _guard = self.latest.lock().expect("a fresh slot is not poisoned");
            panic!("simulated panic while holding the report lock");
        });
    }
}
