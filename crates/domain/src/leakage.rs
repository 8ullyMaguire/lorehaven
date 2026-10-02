//! §52 — taste leakage: what a public artifact gives away.
//!
//! This module exists because A9 found a gap the existing taste types could not
//! cover: `taste_vector::ResonanceLabel` is a READER-facing alignment label
//! ("Aligned: Strong"), which §16.17 permits because it describes the reader's
//! relation to the instance's preferences. §52.3's owner-visible resonance label
//! is a different object with a different audience and a different threat model,
//! and conflating the two would either over-constrain the reader label or
//! under-constrain the owner label.
//!
//! The three types here are the testable parts of §52:
//!
//!   * [`LeakageRow`] / [`Disposition`] — §52.1's rows and what may be done to them.
//!   * [`PayoutAttribution`] — §52.2's instance-attributed, batched payout.
//!   * [`OwnerResonance`] — §52.3's coarse, weekly, owner-only label.

use serde::{Deserialize, Serialize};

use crate::ids::{PseudId, WorkId};

/// Unix seconds, UTC.
///
/// Not `time::OffsetDateTime`, which the rest of this crate uses for *arithmetic*
/// (`jobs.rs` needs to add an interval) but which is not `Serialize`. These types
/// are stored and returned as data, and the migrations store RFC 3339 text, so the
/// domain keeps the primitive and lets the store do the formatting. A helper is
/// provided rather than a newtype because nothing here needs the invariant a
/// newtype would buy.
pub type UnixSeconds = i64;

// ── §52.1 The leakage view ───────────────────────────────────────────────────

/// What may be done to a reviewed artifact.
///
/// The default is [`Disposition::Keep`], and that default is the point: §52.1
/// says a disclosure review is a decision the operator makes with information in
/// front of them, so the review's job is to inform rather than to push. A
/// disposition that defaulted to `Coarsen` would make every row read as a
/// finding, and an operator reading ten rows that all say "remove this" learns
/// nothing about which one matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Disposition {
    /// Stays as is. The artifact was reviewed and judged acceptable.
    Keep,
    /// Stays, with its signal coarsened — bucketed, batched, or aggregated.
    Coarsen,
    /// The artifact is withdrawn. A policy change, handled by §43 proposal
    /// machinery rather than as a side effect of reading a page.
    Remove,
}

impl Default for Disposition {
    /// §52.1's default. See the type's doc comment: a review that defaults to
    /// `Coarsen` makes every row read as a finding.
    fn default() -> Self {
        Self::Keep
    }
}

impl Disposition {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Coarsen => "coarsen",
            Self::Remove => "remove",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "keep" => Some(Self::Keep),
            "coarsen" => Some(Self::Coarsen),
            "remove" => Some(Self::Remove),
            _ => None,
        }
    }
}

/// How confidently an observant reader could draw the stated inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ease {
    /// A curious person could put this together from what they can already see.
    Plain,
    /// It takes a deliberate comparison across several artifacts.
    Derived,
    /// It takes measuring. Coarsening here is the only real remedy, because the
    /// signal IS the measurement.
    Measured,
}

/// One row of the leakage view: an artifact, and the coarse inference a reader
/// could draw from it.
///
/// Deliberately **not** a dimension or a score. §52.1 draws the line at what a
/// person could state out loud, so this type has no field that could hold a taste
/// affinity, a vector component, or a weight. That is enforced by the shape: a
/// future change that wanted to add one would have to add a new type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeakageRow {
    pub artifact: String,
    /// The inference in the words a reader would use, never in the system's.
    pub inferable: String,
    pub ease: Ease,
    pub disposition: Disposition,
    pub reviewed_at: UnixSeconds,
    pub reviewed_by: PseudId,
}

impl LeakageRow {
    /// §52.1's reviewable predicate.
    ///
    /// A row qualifies for review when a person could plausibly state the
    /// inference. This is deliberately a *check on the wording* rather than a
    /// trust in the author: rows are prose, and prose is where a number sneaks
    /// in. A row whose text carries a decimal, or the vocabulary of a score, is
    /// refused here so it cannot be displayed — because §0.3 forbids the lens
    /// being inferable through any user-facing label, and a label reading
    /// "hurt/comfort affinity 0.73" does that even though the mechanism is a
    /// string.
    pub fn qualifies_for_review(&self) -> bool {
        prose_without_precision(&self.inferable)
    }
}

/// Whether a row's wording is prose rather than a measurement.
///
/// The check is deliberately blunt and deliberately narrow: a decimal point, a
/// digit run, or a score-shaped word. It is not a natural-language classifier and
/// does not try to be — the guarantee it provides is that no row can carry a
/// number into the view, which is the part §0.3 actually constrains. A row that
/// says "your affinity is high" passes; §52.1 accepts that, because a coarse
/// claim about a dimension is still something a person could say.
pub fn prose_without_precision(s: &str) -> bool {
    const SCORE_WORDS: &[&str] = &[
        "score",
        "affinity:",
        "vector",
        "weight",
        "similarity",
        "coefficient",
        "distance",
    ];
    let lowered = s.to_ascii_lowercase();
    if lowered.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    if lowered.contains('.') {
        return false;
    }
    !SCORE_WORDS.iter().any(|w| lowered.contains(w))
}

/// A reviewed leakage row plus the review's own bookkeeping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeakageReview {
    pub rows: Vec<LeakageRow>,
    /// §52.1: the view never certifies completeness. A review that happened is a
    /// fact; a review that is *exhaustive* is not, and the type has no way to
    /// say it is.
    pub reviewed_at: UnixSeconds,
}

impl LeakageReview {
    pub fn new(rows: Vec<LeakageRow>, reviewed_at: UnixSeconds) -> Self {
        Self { rows, reviewed_at }
    }

    /// Rows whose disposition calls for a change, coarsened ones first.
    ///
    /// §52.1 makes removing an artifact a policy change belonging to §43, so
    /// this ordering puts the cheaper and reversible one first: an operator who
    /// only reads the top of this list will batch rather than delete.
    pub fn needing_action(&self) -> Vec<&LeakageRow> {
        let mut out: Vec<&LeakageRow> = self
            .rows
            .iter()
            .filter(|r| r.disposition != Disposition::Keep)
            .collect();
        out.sort_by_key(|r| match r.disposition {
            Disposition::Coarsen => 0,
            Disposition::Remove => 1,
            Disposition::Keep => 2,
        });
        out
    }

    /// The rows that §52.1 permits to be displayed at all.
    pub fn displayable(&self) -> Vec<&LeakageRow> {
        self.rows
            .iter()
            .filter(|r| r.qualifies_for_review())
            .collect()
    }
}

// ── §52.2 Payout attribution ─────────────────────────────────────────────────

/// What a payout says about who decided it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PayoutAttribution {
    /// "The instance paid for this." The §52.2 form.
    Instance,
    /// "You were rated highly." Forbidden: one rating action at one moment is a
    /// measurement of the lens.
    Rating,
}

impl PayoutAttribution {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Instance => "instance",
            Self::Rating => "rating",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "instance" => Some(Self::Instance),
            "rating" => Some(Self::Rating),
            _ => None,
        }
    }
}

/// The batch window a payout is attributed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchWindow {
    pub opened_at: UnixSeconds,
    /// Exclusive. A window that is open has no `closed_at`, and §52.2's whole
    /// point is that an open window is not yet a payout anyone can correlate.
    pub closed_at: Option<UnixSeconds>,
}

impl BatchWindow {
    pub fn closed(opened_at: UnixSeconds, closed_at: UnixSeconds) -> Self {
        debug_assert!(
            closed_at >= opened_at,
            "a batch window cannot close before it opens"
        );
        Self {
            opened_at,
            closed_at: Some(closed_at),
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed_at.is_some()
    }
}

/// A payout, as §52.2 requires it to be recorded.
///
/// The type makes the forbidden shape unrepresentable rather than merely
/// discouraged: there is no field for a rating event, a rating id, or a rater,
/// because a payout that carried one would hand an observer exactly the
/// correlation §52.2 exists to prevent — and the cheapest place to forbid that is
/// the struct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchedPayout {
    pub recipient: PseudId,
    pub work: WorkId,
    pub credits: i64,
    pub window: BatchWindow,
    pub paid_at: UnixSeconds,
}

/// §52.2's predicate, on the whole payout path.
///
/// `paid_at` must fall inside its own window, because §52.2's finding was that
/// timing carries the signal even when the payload does not: a payment seconds
/// from a rating is a usable correlation with no detail at all. So the check is
/// not "is the attribution instance-scoped" but "is the attribution instance-scoped
/// AND is the payment not inside the event window".
pub fn payout_is_safe(payout: &BatchedPayout) -> bool {
    let Some(closed) = payout.window.closed_at else {
        // An unclosed window means the payout was made without a batch to
        // attribute it to, which is the correlation this whole section is about.
        return false;
    };
    payout.paid_at >= payout.window.opened_at && payout.paid_at <= closed
}

// ── §52.3 The owner-visible resonance label ──────────────────────────────────

/// The owner-visible resonance label.
///
/// Coarse on purpose. §52.3: "something in the shape of 'quiet,' 'steady,'
/// 'noticed' — enough to tell an author their work is landing, not enough to
/// rank." Four buckets, no ordering visible to a reader, and no number anywhere
/// in the type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OwnerResonance {
    Quiet,
    Steady,
    Noticed,
    Landing,
}

impl OwnerResonance {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Quiet => "quiet",
            Self::Steady => "steady",
            Self::Noticed => "noticed",
            Self::Landing => "landing",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "quiet" => Some(Self::Quiet),
            "steady" => Some(Self::Steady),
            "noticed" => Some(Self::Noticed),
            "landing" => Some(Self::Landing),
            _ => None,
        }
    }

    /// §52.3: bucket a resonance score without exposing its precision.
    ///
    /// The thresholds are deliberately uneven — 0.45 rather than 0.5 — because a
    /// symmetric threshold would put a knife edge exactly where the most works
    /// land, and §52.3's requirement is that a movement be visible to the author
    /// while a *prober* learns nothing. An edge at a round number is an edge
    /// somebody can find by submitting works and watching which side they land
    /// on; an edge at an arbitrary value is only findable by measuring, which is
    /// `Ease::Measured` in §52.1's terms and a far more expensive probe.
    pub fn from_score(score: f64) -> Self {
        match score {
            s if s >= 0.82 => Self::Landing,
            s if s >= 0.58 => Self::Noticed,
            s if s >= 0.31 => Self::Steady,
            _ => Self::Quiet,
        }
    }
}

/// An owner-visible label together with the batch that produced it.
///
/// §52.3's third clause: a batch that has not run reports the previous value and
/// *says it is stale*, because a label that silently updates is worse than one
/// that lags visibly — an author reading it as current will over-read a movement
/// that is three days old.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerResonanceLabel {
    pub owner: PseudId,
    pub work: WorkId,
    pub label: OwnerResonance,
    pub computed_at: UnixSeconds,
}

impl OwnerResonanceLabel {
    /// Whether the label is behind the current batch window.
    pub fn is_stale(&self, window: &BatchWindow) -> bool {
        match window.closed_at {
            None => true,
            Some(closed) => self.computed_at < closed,
        }
    }

    /// The label as the owner should read it.
    ///
    /// §52.3 requires the staleness to be visible, so this returns prose that
    /// names it rather than a separate boolean the caller might drop.
    pub fn display(&self, window: &BatchWindow) -> String {
        if self.is_stale(window) {
            format!("{} (last week's reading)", self.label.as_str())
        } else {
            self.label.as_str().to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: UnixSeconds = 1_700_000_000;
    const T1: UnixSeconds = 1_700_000_600;
    const T2: UnixSeconds = 1_700_001_200;
    const T3: UnixSeconds = 1_700_001_800;

    fn pseud(n: u8) -> PseudId {
        PseudId::from_uuid(uuid::Uuid::from_bytes([n; 16]))
    }

    fn work_id(n: u8) -> WorkId {
        WorkId::from_uuid(uuid::Uuid::from_bytes([n; 16]))
    }

    fn row(inferable: &str, disposition: Disposition) -> LeakageRow {
        LeakageRow {
            artifact: "standing_bounty_payout".into(),
            inferable: inferable.into(),
            ease: Ease::Plain,
            disposition,
            reviewed_at: T0,
            reviewed_by: pseud(1),
        }
    }

    #[test]
    fn prose_wording_is_reviewable_and_a_number_is_not() {
        assert!(prose_without_precision(
            "standing bounties in this fandom pay promptly"
        ));
        assert!(prose_without_precision(
            "your work matched a standing bounty after curation"
        ));
        // The §52.1 refusal: a label can carry a measurement in plain text.
        assert!(!prose_without_precision("hurt/comfort affinity is 0.73"));
        assert!(!prose_without_precision("resonance 0.8"));
        assert!(!prose_without_precision("top 10% by score"));
        // A digit with no decimal point: the `.` branch cannot catch this, so it
        // exercises the digit check on its own.
        assert!(!prose_without_precision("ranked 7th this week"));
        assert!(!prose_without_precision(
            "your weight on this trope is high"
        ));
        assert!(!prose_without_precision("weight on the operator's lens"));
    }

    #[test]
    fn a_row_carrying_precision_cannot_be_displayed() {
        let leaky = LeakageReview::new(
            vec![row(
                "your affinity for this topic is 0.73",
                Disposition::Keep,
            )],
            T0,
        );
        assert!(leaky.displayable().is_empty());
    }

    #[test]
    fn display_keeps_coarse_claims_about_a_dimension() {
        // §52.1 accepts a coarse dimension claim; only precision is refused.
        let coarse = LeakageReview::new(
            vec![row("you favour this fandom's tropes", Disposition::Keep)],
            T0,
        );
        assert_eq!(coarse.displayable().len(), 1);
    }

    #[test]
    fn action_is_ordered_coarsen_before_remove() {
        // §52.1: removing is a §43 policy change, so the reversible action sorts first.
        let review = LeakageReview::new(
            vec![
                row("remove me", Disposition::Remove),
                row("batch me", Disposition::Coarsen),
                row("fine", Disposition::Keep),
            ],
            T0,
        );
        let actions = review.needing_action();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].disposition, Disposition::Coarsen);
        assert_eq!(actions[1].disposition, Disposition::Remove);
    }

    #[test]
    fn keep_is_the_default_disposition() {
        // A review that defaults to `Coarsen` makes every row read as a finding.
        assert_eq!(Disposition::default(), Disposition::Keep);
    }

    fn payout(window: BatchWindow, paid_at: UnixSeconds) -> BatchedPayout {
        BatchedPayout {
            recipient: pseud(2),
            work: work_id(3),
            credits: 50,
            window,
            paid_at,
        }
    }

    #[test]
    fn a_payout_inside_its_window_is_safe() {
        assert!(payout_is_safe(&payout(BatchWindow::closed(T0, T2), T1)));
    }

    #[test]
    fn a_payout_in_an_open_window_is_unsafe() {
        // §52.2's actual finding: timing carries the signal even with no payload
        // detail, so an unclosed window means no batch to attribute it to.
        let open = BatchWindow {
            opened_at: T0,
            closed_at: None,
        };
        assert!(!payout_is_safe(&payout(open, T1)));
    }

    #[test]
    fn a_payout_outside_its_window_is_unsafe() {
        // Paid before its window opened, and after it closed.
        assert!(!payout_is_safe(&payout(BatchWindow::closed(T1, T2), T0)));
        assert!(!payout_is_safe(&payout(BatchWindow::closed(T0, T1), T3)));
    }

    #[test]
    fn the_resonance_label_is_four_buckets_with_no_number() {
        assert_eq!(OwnerResonance::from_score(0.0), OwnerResonance::Quiet);
        assert_eq!(OwnerResonance::from_score(0.95), OwnerResonance::Landing);
        // Every score maps somewhere and nothing falls outside the enum.
        for step in 0..=100 {
            let s = f64::from(step) / 100.0;
            let _ = OwnerResonance::from_score(s);
        }
    }

    #[test]
    fn bucket_edges_are_not_at_guessable_round_numbers() {
        // §52.3: a threshold at 0.5 or 0.75 is findable by submitting works and
        // watching which side they land on.
        //
        // Asserted on the BEHAVIOUR, not on a copy of the constants -- an earlier
        // version of this test read the three edges out of the source and checked
        // them against 0.5, and mutating the mapping to start at 0.5 kept it green
        // because the constants it inspected were untouched. A test that restates
        // the implementation cannot detect a change to it.
        // Interior thresholds only: 1.0 is the end of the range, and nothing
        // sits above it, so a bucket boundary there would not be findable by
        // probing -- which is exactly why the highest bucket is capped at 0.82
        // rather than being open-ended at the top.
        const GUESSABLE: [f64; 3] = [0.25, 0.5, 0.75];
        // Two scores straddling a guessable threshold must land in the SAME bucket.
        //
        // This is the direction that matters and I had it backwards first time:
        // an edge sitting exactly at 0.5 makes 0.49 and 0.51 differ, and that
        // difference is what a prober submits works to find. Equal on both sides
        // means no boundary there, and no boundary at a round number means the
        // buckets cannot be located by guessing.
        for g in GUESSABLE {
            let just_below = OwnerResonance::from_score(g - 0.01);
            let just_above = OwnerResonance::from_score(g + 0.01);
            assert_eq!(
                just_below, just_above,
                "an edge sits at {g}: {g} and either neighbour map differently, so a \
                 prober finds the boundary by submitting works near {g}"
            );
        }
    }

    #[test]
    fn the_bucket_mapping_is_monotone_and_covers_the_range() {
        // Non-decreasing across the whole range, and never out of order: a label
        // that went down as resonance rose would tell an author their work was
        // landing when the system says the opposite.
        let mut prev = OwnerResonance::from_score(0.0);
        let rank = |l: OwnerResonance| match l {
            OwnerResonance::Quiet => 0,
            OwnerResonance::Steady => 1,
            OwnerResonance::Noticed => 2,
            OwnerResonance::Landing => 3,
        };
        for step in 0..=1000 {
            let s = f64::from(step) / 1000.0;
            let cur = OwnerResonance::from_score(s);
            assert!(
                rank(cur) >= rank(prev),
                "score {s} maps to {} but a lower score mapped to {}",
                cur.as_str(),
                prev.as_str()
            );
            prev = cur;
        }
    }

    #[test]
    fn a_label_behind_the_window_reads_as_stale() {
        // §52.3's third clause, and the reason it exists: an author reading a
        // three-day-old movement as current over-reads it.
        let label = OwnerResonanceLabel {
            owner: pseud(4),
            work: work_id(5),
            label: OwnerResonance::Noticed,
            computed_at: T0,
        };
        let window = BatchWindow::closed(T1, T2);
        assert!(label.is_stale(&window));
        assert!(label.display(&window).contains("last week"));
    }

    #[test]
    fn a_label_current_with_its_window_reads_plain() {
        let label = OwnerResonanceLabel {
            owner: pseud(4),
            work: work_id(5),
            label: OwnerResonance::Steady,
            computed_at: T2,
        };
        let window = BatchWindow::closed(T1, T2);
        assert!(!label.is_stale(&window));
        assert_eq!(label.display(&window), "steady");
    }

    #[test]
    fn an_open_window_makes_even_a_fresh_label_stale() {
        let label = OwnerResonanceLabel {
            owner: pseud(4),
            work: work_id(5),
            label: OwnerResonance::Quiet,
            computed_at: T3,
        };
        let open = BatchWindow {
            opened_at: T0,
            closed_at: None,
        };
        assert!(label.is_stale(&open));
    }

    #[test]
    fn the_reader_facing_alignment_label_is_a_different_type() {
        // The reason this module exists at all: `taste_vector::ResonanceLabel` is
        // reader-facing and §16.17 permits it. Making them one type would either
        // over-constrain the reader label or under-constrain the owner label.
        let owner = OwnerResonance::Noticed;
        assert_eq!(owner.as_str(), "noticed");
        // No shared variant set: the reader label has Strong/Moderate/Weak/
        // Neutral/Dissonant and the owner label has none of them.
        for reader_only in ["strong", "moderate", "weak", "dissonant"] {
            assert!(OwnerResonance::parse(reader_only).is_none());
        }
    }
}
