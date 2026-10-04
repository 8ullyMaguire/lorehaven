//! M45-23 — the north-star metric, with per-mechanism attribution (spec §53.5).
//!
//! "North-star" is one word for two different measures, and the tracker note splits
//! them: *"Works rated per month + time-to-find; attribute each loved work to the
//! surfacing mechanism."* So there are two named measures here and **no arithmetic
//! on them** — see `NorthStar` for why a single composite number is refused.
//!
//! Three decisions in this file are load-bearing and each is load-bearing for a
//! reason a reader could otherwise undo by accident:
//!
//! 1. **`median_days_to_find` is `Option<f64>`, never `f64`.** With no completed pair
//!    the value is *undefined*, not zero. §53.5: the rate "reports its own missing
//!    inputs". A zero would read as total failure and invite a change to a recipe that
//!    has simply not been tested yet.
//! 2. **A loved work with no slot row is counted as `unattributed`, not dropped.**
//!    A work can be loved without this instance having served it — it came from an
//!    import, a search, a sister instance, or an author's own shelf. Filtering those
//!    out would silently shrink the denominator and inflate every mechanism's share.
//! 3. **The attribution is the *earliest* slot before the rating, and a slot recorded
//!    *after* the rating claims nothing.** A slot served an hour after a rating cannot
//!    have caused it. This is the one rule that separates "which mechanism surfaced
//!    this" from "which mechanism was nearby", and it is why the store pairs slot and
//!    rating by time rather than by work alone.

use serde::{Deserialize, Serialize};

/// One mechanism's share of the loved works in the window.
///
/// `loved_works + unattributed` is the window total, so the shares across every
/// mechanism *plus* `unattributed` sum to 1.0. That is asserted, not assumed — a
/// share that does not sum to 1.0 means a mechanism name is missing somewhere, which
/// is exactly the kind of quiet accounting error a metric like this is built to
/// survive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MechanismAttribution {
    /// The `SlotMechanism` wire value, or `unattributed`.
    pub key: String,
    /// Loved works attributed to this mechanism.
    pub loved_works: i64,
    /// `loved_works` over the window total. `0.0` when the window has no loved works,
    /// which is honest: it is a share of nothing, not a missing share.
    pub share: f64,
}

/// An input the metric needed and did not have, named rather than defaulted.
///
/// §53.5 requires the response to report its own incompleteness. Naming the input is
/// what lets an operator tell "the recipe is failing" from "the recipe has not been
/// measured yet", which are opposite responses to the same number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingInput {
    /// No rating rows in the window, so the per-month rate has no numerator.
    Ratings,
    /// No `reading_status` completions, so `median_days_to_find` is undefined.
    Completions,
    /// No `recommendation_slots` rows, so nothing can be attributed to a mechanism.
    Slots,
}

impl MissingInput {
    /// The wire form.
    pub fn as_str(self) -> &'static str {
        match self {
            MissingInput::Ratings => "ratings",
            MissingInput::Completions => "completions",
            MissingInput::Slots => "slots",
        }
    }
}

/// The metric. Two measures, plus the attribution and the honesty fields.
///
/// **There is deliberately no `north_star: f64`.** §0.3 and the standing rule against a
/// composite score: one number would be a ranking signal the moment anything sorted on
/// it, and a metric that can be sorted on gets chased rather than read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NorthStar {
    /// §53.5's rate: distinct works rated `>= 4` per month in the window. `0.0` when
    /// the window has no ratings at all, and `missing_inputs` then carries
    /// `MissingInput::Ratings` so the zero is not mistaken for a measurement.
    pub works_rated_per_month: f64,
    /// §53.5's companion: median days from a work being first served to being loved.
    ///
    /// `None` when no completed pair exists. Not `Some(0.0)`.
    pub median_days_to_find: Option<f64>,
    /// Works rated `>= 4` in the window. The numerator's raw count, so the rate can be
    /// checked rather than trusted.
    pub rated_works: i64,
    /// Works rated `>= 4` **or** marked finished. §53.5's definition of "loved".
    pub loved_works: i64,
    /// Loved works with no slot row before the rating. Counted, never dropped — see the
    /// module docs.
    pub unattributed: i64,
    /// Per-mechanism breakdown. Sums with `unattributed` to `loved_works`.
    pub by_mechanism: Vec<MechanismAttribution>,
    /// What the metric could not measure. Empty means it measured everything.
    pub missing_inputs: Vec<MissingInput>,
}

impl NorthStar {
    /// Type-level assertion of §53.2: an economy or metrics view carries no per-account
    /// detail, and this is the existing pattern for saying so at the type boundary
    /// rather than in a review comment. The same shape as
    /// `FlowSummary::carries_account_detail()`.
    pub fn carries_account_detail(&self) -> bool {
        false
    }

    /// The attribution key used for a loved work with no slot row before its rating.
    ///
    /// A named constant rather than a bare `UNATTRIBUTATED`, because the store has to
    /// emit it, the route has to serialise it, and the tests have to assert on it. Three
    /// call sites and one spelling.
    pub const UNATTRIBUTED: &'static str = "unattributed";

    /// Do the shares account for every loved work?
    ///
    /// True when the mechanisms plus `unattributed` equal `loved_works`. Exposed so a
    /// test can assert the invariant rather than infer it from a sum that happens to
    /// come out right for the wrong reason.
    pub fn shares_account_for_everything(&self) -> bool {
        let attributed: i64 = self.by_mechanism.iter().map(|m| m.loved_works).sum();
        attributed + self.unattributed == self.loved_works
    }
}
