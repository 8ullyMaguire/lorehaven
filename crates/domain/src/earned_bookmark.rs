//! §53.6 — the earned-bookmark ratio: bookmarks per completion.
//!
//! The cheapest of the six ranked gaps, and the one the ideas list argued hardest for:
//! it is cheap to compute and hard to fake, because faking a bookmark costs one click
//! and faking a completion costs a reader's time. A ratio of bookmarks to *views*
//! would measure nothing — a refresh is free, so any such ratio is a statement about
//! how a reader browses.
//!
//! **It is pure arithmetic over counts §20.3 already gathers.** [`ReaderSignals`]
//! carries both `bookmarkers` and `finishers` from the same query that feeds the
//! author multipliers, so there is no new SQL here. That is worth stating plainly,
//! because the audit called this gap "not a missing query over existing data" and it
//! was right about *why* while being wrong about the consequence: the query already
//! existed, which makes this a definition and a division rather than a build.
//!
//! What the definition cost is the part that mattered. §53.5 already defines "hit" as
//! *finished or rated ≥4*. Borrowing that here would have put the cheap signal inside
//! the expensive one — a four-star rating on chapter one costs exactly the one click
//! this ratio is measured against — so `payouts.rs`'s broader definition and this one
//! are deliberately different, and [`HitBasis`] names which is in force.

use serde::{Deserialize, Serialize};

use crate::payouts::ReaderSignals;

/// Which sense of "hit" a ratio was computed against.
///
/// Not a parameter to every function — [`EarnedBookmark`] is always `Completion`, and
/// this enum exists so the *choice* is nameable at a call site rather than implied.
/// The variants that are not implemented are the ones a reader of this file will ask
/// about, and each says why it is not the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HitBasis {
    /// `reading_status.status = 'finished'` with a `finished_at`. §53.6's definition,
    /// and the only one computed.
    ///
    /// §53.5's `FinishedOrFourStars` is deliberately *not* this, and the reason is
    /// worth keeping next to the variant rather than in a changelog: §53.5 asks
    /// whether a feed is working, where a four-star rating is real evidence a reader
    /// engaged. §53.6 asks whether a work earns a reader's time, and a rating costs
    /// one click. Reusing the broad sense would let the cheap signal into the
    /// expensive one.
    Completion,
    /// §53.5's sense: finished, or rated four stars or better.
    ///
    /// Not used here, and not a default. It exists because two engines in this
    /// codebase use the word "hit" for two different things, and a future reader who
    /// finds one of them must be able to see the collision rather than guess.
    FinishedOrFourStars,
    /// A row in `work_view_log`.
    ///
    /// Never. Listed so the option is visibly closed: a refresh is free, so a ratio
    /// built on views cannot distinguish a reader who cared from one who reloaded.
    View,
}

/// One work's earned-bookmark ratio over a window.
///
/// `bookmarkers` and `finishers` are both kept, not just the quotient, because a
/// ratio whose terms are not visible cannot be trusted or reproduced — and because
/// §53.6 requires the denominator to be *reported*, not merely divided.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EarnedBookmark {
    /// Distinct readers who bookmarked the work in the window.
    pub bookmarkers: i64,
    /// Distinct readers who finished it in the window. §53.6's "hit".
    pub finishers: i64,
    /// Bookmarks per completion.
    ///
    /// `None` when there is no denominator — no bookmarks, or no completions. §53.6:
    /// "A work with no bookmarks is excluded, not counted as a ratio of zero." A zero
    /// here would read as "nobody bookmarked and nobody finished", which is a claim
    /// about a work nothing happened to, and would pull §20.3's multipliers toward
    /// zero for a work that simply has no readers yet.
    pub ratio: Option<f64>,
    pub basis: HitBasis,
}

impl EarnedBookmark {
    /// Compute §53.6's ratio from signals already gathered for §20.3.
    ///
    /// Both counts come from the same [`ReaderSignals`], so this cannot disagree with
    /// the payout about who bookmarked and who finished — which is the property that
    /// makes it safe to feed the one to the other.
    #[must_use]
    pub fn compute(signals: &ReaderSignals) -> Self {
        // The undefined cases, stated rather than collapsed into zero:
        //   * no bookmarks  -- the spec excludes the work rather than reporting 0.
        //     A bookmark count of zero over a completion count of zero is the
        //     "undefined, not zero" rule of §53.5 arriving by a different route.
        //   * no completions with bookmarks present -- a real observation, but not a
        //     *ratio*: 40 bookmarks and 0 completions is an infinite ratio, and
        //     `f64::INFINITY` would propagate into a multiplier as a poison value
        //     rather than as the fact it is. `None` is the honest answer, and the
        //     bookmark count is right there for a caller that wants the story.
        let ratio = match (signals.bookmarkers, signals.finishers) {
            (bookmarks, completions) if bookmarks > 0 && completions > 0 => {
                Some(bookmarks as f64 / completions as f64)
            }
            _ => None,
        };
        Self {
            bookmarkers: signals.bookmarkers,
            finishers: signals.finishers,
            ratio,
            basis: HitBasis::Completion,
        }
    }

    /// Whether this ratio is one §20.3 may act on.
    ///
    /// **A thin alias for `ratio.is_some()`, and that is on purpose.** I first wrote
    /// it as `is_some_and(f64::is_finite)` to guard against a caller constructing a
    /// ratio by dividing raw counts and reaching infinity by the `0 completions` path.
    /// Mutating the finiteness check to a plain `is_some()` left all 13 tests green.
    ///
    /// So the extra test could not fail, because `compute` never produces a non-finite
    /// ratio — the `match` in `compute` already refuses a zero denominator, and
    /// `ratio` is a private field, so nothing outside this module can build one. The
    /// finiteness check was defensive code defending against an unreachable state,
    /// which is the kind that rots silently and then misleads the next reader into
    /// thinking the type is looser than it is.
    ///
    /// The defence that actually matters is upstream: `compute` refuses the zero
    /// denominator, and the zero-denominator test proves it does. `usable` exists
    /// because callers should not have to remember that `None` is the only unusable
    /// state.
    #[must_use]
    pub fn usable(&self) -> bool {
        self.ratio.is_some()
    }

    /// The line an operator or an author may be shown.
    ///
    /// Two lines' worth of decision in one: the ratio is a quality signal, so it is
    /// shown with its terms and its window, and never as a bare number.
    #[must_use]
    pub fn summary_line(&self) -> String {
        match self.ratio {
            // Rounded to two places for display only. The stored value is untouched —
            // a rounded 2.667 would misreport a real ratio as 2.67, which is a
            // different claim about the work.
            Some(ratio) => format!(
                "{:.2} bookmarks per completion ({} bookmarked, {} finished)",
                ratio, self.bookmarkers, self.finishers
            ),
            None if self.bookmarkers > 0 => {
                format!("{} bookmarks and no completions yet", self.bookmarkers)
            }
            None => "no bookmarks yet".to_owned(),
        }
    }
}

impl ReaderSignals {
    /// §53.6's ratio for this set of signals.
    #[must_use]
    pub fn earned_bookmark(&self) -> EarnedBookmark {
        EarnedBookmark::compute(self)
    }
}
