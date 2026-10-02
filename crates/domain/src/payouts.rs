//! §20.3 — the author payout multipliers.
//!
//! Specified in full in `docs/spec.md` as two code blocks and implemented nowhere:
//! grep across `crates/**/*.rs` for `quality_multiplier`, `demand_multiplier`,
//! `completion_rate`, `reread_bonus` and `positive_feedback_bonus` returned zero
//! files for all five. The ledger (`credit_entries`, signed `amount_bp`) and the
//! hold machinery exist; the *payout* was the missing half.
//!
//! Two rules from the prose that decide the whole shape, and both are about what
//! the author is allowed to be told:
//!
//!   * §20.3: "The quality multiplier breakdown is shown because it is actionable
//!     and based on public reader behavior. The demand multiplier is not shown
//!     because it contains the private taste signal."
//!
//!   * §20.3: the quality multiplier's four bonuses are each *conditional* — they
//!     apply only above a threshold, so an author below one is not penalised, they
//!     simply receive no bonus. A naive sum would quietly make a bad multiplier
//!     instead of a neutral one.
//!
//! So this module has two types, not one. [`QualityMultiplier`] can be explained to
//! its subject; [`DemandMultiplier`] cannot, and the distinction is enforced by
//! which struct carries which — not by a caller remembering not to print the
//! second one.

use serde::{Deserialize, Serialize};

/// Minimum readers before any quality signal activates. §20.3: "Minimum 10
/// readers before quality signals activate."
pub const MIN_READERS: i64 = 10;

/// The four §20.3 thresholds, as a struct so a threshold cannot be read from the
/// wrong field.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct QualityThresholds {
    pub completion_rate: f64,
    pub positive_feedback: f64,
    pub reread_rate: f64,
    pub bookmark_rate: f64,
}

impl Default for QualityThresholds {
    fn default() -> Self {
        // §20.3 verbatim: >60% finish, >80% positive, >10% return, >15% bookmark.
        Self {
            completion_rate: 0.60,
            positive_feedback: 0.80,
            reread_rate: 0.10,
            bookmark_rate: 0.15,
        }
    }
}

/// The reader-side measurements a multiplier is derived from.
///
/// One window, one work set. §20.3: "Recalculated weekly from the previous 30
/// days", so the caller supplies the window rather than this module inventing a
/// date range — a payout that silently used a different window than the one its
/// author was shown would be the §20.3 disclosure rule broken in the other
/// direction.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ReaderSignals {
    /// Distinct readers who started the work.
    pub starters: i64,
    /// Distinct readers who finished it.
    pub finishers: i64,
    /// Feedback items received.
    pub feedback_count: i64,
    /// Feedback items that were positive.
    pub positive_feedback: i64,
    /// Distinct readers who returned to the work after finishing.
    pub rereaders: i64,
    /// Distinct readers who bookmarked it.
    pub bookmarkers: i64,
}

/// The explainable half: §20.3's `quality_multiplier`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct QualityMultiplier {
    pub value: f64,
    /// Which bonuses fired, for the author-visible explanation.
    pub completion_bonus: bool,
    pub feedback_bonus: bool,
    pub reread_bonus: bool,
    pub bookmark_bonus: bool,
    /// Whether the reader floor was met. When false, every bonus is false and the
    /// value is exactly 1.0 — a new work is not penalised for having no readers.
    pub signals_active: bool,
}

/// §20.3's floor and ceiling. Both are inclusive bounds.
pub const QUALITY_MIN: f64 = 1.0;
pub const QUALITY_MAX: f64 = 1.8;

impl QualityMultiplier {
    /// Compute the quality multiplier for one window.
    ///
    /// Four independent conditional bonuses, each worth a fixed fraction. Below the
    /// reader floor the result is exactly `QUALITY_MIN` with no bonuses — §20.3's
    /// "Minimum 10 readers before quality signals activate", and the reason a new
    /// work sits at 1.0x rather than at a ratio of nothing.
    pub fn compute(signals: &ReaderSignals, t: &QualityThresholds) -> Self {
        if signals.starters < MIN_READERS {
            return Self {
                value: QUALITY_MIN,
                completion_bonus: false,
                feedback_bonus: false,
                reread_bonus: false,
                bookmark_bonus: false,
                signals_active: false,
            };
        }

        let completion = signals.finishers as f64 / signals.starters as f64;
        let feedback = if signals.feedback_count == 0 {
            0.0
        } else {
            signals.positive_feedback as f64 / signals.feedback_count as f64
        };
        let reread = signals.rereaders as f64 / signals.starters as f64;
        let bookmark = signals.bookmarkers as f64 / signals.starters as f64;

        // Strictly greater-than on every threshold, because §20.3 writes ">60%"
        // and a reader sitting exactly on the line has not cleared it.
        let completion_bonus = completion > t.completion_rate;
        let feedback_bonus = feedback > t.positive_feedback;
        let reread_bonus = reread > t.reread_rate;
        let bookmark_bonus = bookmark > t.bookmark_rate;

        let mut value: f64 = 1.0;
        if completion_bonus {
            value += 0.3;
        }
        if feedback_bonus {
            value += 0.2;
        }
        if reread_bonus {
            value += 0.2;
        }
        if bookmark_bonus {
            value += 0.1;
        }

        Self {
            // The clamp is defensive, not decorative: the four maxima sum to exactly
            // QUALITY_MAX, so float addition cannot exceed it in practice — but a
            // future fifth bonus would otherwise breach a range the spec states as
            // a hard 1.0–1.8.
            value: value.clamp(QUALITY_MIN, QUALITY_MAX),
            completion_bonus,
            feedback_bonus,
            reread_bonus,
            bookmark_bonus,
            signals_active: true,
        }
    }

    /// The author-visible explanation.
    ///
    /// §20.3: names the bonuses that fired and *nothing else*. In particular it
    /// never mentions a threshold the author failed to clear — an author told
    /// "your completion rate is 55%, below the 60% needed" is being told something
    /// actionable, which is the rule; but the phrasing below only ever reports
    /// credit given, so it cannot become a ranking of authors by shortfall.
    pub fn author_explanation(&self) -> String {
        if !self.signals_active {
            return "Quality bonus: none yet (building reader history).".to_owned();
        }
        let mut parts: Vec<&str> = Vec::new();
        if self.completion_bonus {
            parts.push("high completion rate");
        }
        if self.feedback_bonus {
            parts.push("positive reader feedback");
        }
        if self.reread_bonus {
            parts.push("readers returning to read again");
        }
        if self.bookmark_bonus {
            parts.push("readers bookmarking");
        }
        if parts.is_empty() {
            return "Quality bonus: none yet (building reader history).".to_owned();
        }
        let pct = ((self.value - 1.0) * 100.0).round() as i64;
        format!("Quality bonus: +{pct}% ({}).", parts.join(", "))
    }
}

/// The silent half: §20.3's `demand_multiplier`.
///
/// Deliberately carries **no** method that produces a string, and no breakdown
/// field. §20.3 requires the author to see only their total credits and the
/// quality breakdown, because the demand multiplier "contains the private taste
/// signal". Making it un-printable is stronger than documenting that callers must
/// not print it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DemandMultiplier {
    pub value: f64,
}

impl DemandMultiplier {
    pub const MIN: f64 = 1.0;
    pub const MAX: f64 = 1.5;

    /// §20.3: `1.0 + 0.25×admin_taste_affinity + 0.15×wishlist + 0.10×search`.
    ///
    /// The three components arrive as already-normalised 0..1 values. Clamping each
    /// input matters more than clamping the output: `admin_taste_affinity` is the
    /// §0.3 secret signal, and a component above 1.0 would be a way for it to
    /// inflate a payout beyond the spec's stated ceiling.
    pub fn compute(admin_taste: f64, wishlist: f64, search: f64) -> Self {
        let clamp01 = |v: f64| v.clamp(0.0, 1.0);
        let value =
            1.0 + 0.25 * clamp01(admin_taste) + 0.15 * clamp01(wishlist) + 0.10 * clamp01(search);
        Self {
            value: value.clamp(Self::MIN, Self::MAX),
        }
    }
}

/// What an author is shown about their earnings.
///
/// §20.3's disclosure boundary as a type: the quality breakdown travels, the
/// demand multiplier does not. A response struct that could not carry the secret is
/// the enforcement; a doc comment saying "do not include demand" is not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthorEarningsView {
    /// Credits earned in the window.
    pub credits: i64,
    /// The explainable multiplier.
    pub quality: QualityMultiplier,
}

impl AuthorEarningsView {
    pub const fn new(credits: i64, quality: QualityMultiplier) -> Self {
        Self { credits, quality }
    }

    /// The two lines §20.3 specifies an author sees.
    pub fn author_lines(&self) -> Vec<String> {
        vec![
            format!("Your work earned {} credits this week.", self.credits),
            self.quality.author_explanation(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A work clearing every §20.3 threshold.
    fn strong() -> ReaderSignals {
        ReaderSignals {
            starters: 100,
            finishers: 80,
            feedback_count: 50,
            positive_feedback: 45,
            rereaders: 20,
            bookmarkers: 25,
        }
    }

    #[test]
    fn a_new_work_is_exactly_one_and_is_not_penalised() {
        // §20.3: "New works start at 1.0x". Below the reader floor there are no
        // ratios to speak of, and a multiplier derived from them would be a
        // punishment for having no readers.
        let m =
            QualityMultiplier::compute(&ReaderSignals::default(), &QualityThresholds::default());
        assert_eq!(m.value, QUALITY_MIN);
        assert!(!m.signals_active);
        assert!(!m.completion_bonus);
    }

    #[test]
    fn all_four_bonuses_reach_the_stated_ceiling() {
        // 1.0 + 0.3 + 0.2 + 0.2 + 0.1 = 1.8, which is exactly §20.3's maximum.
        let m = QualityMultiplier::compute(&strong(), &QualityThresholds::default());
        assert_eq!(m.value, 1.8);
        assert!(m.signals_active);
        assert!(m.completion_bonus && m.feedback_bonus && m.reread_bonus && m.bookmark_bonus);
    }

    #[test]
    fn each_bonus_is_independently_conditional() {
        // §20.3 writes four *conditional* bonuses. A plain sum would make a weak
        // work pay less than a neutral one; instead it pays exactly the neutral
        // 1.0 plus whatever it earned.
        let t = QualityThresholds::default();

        let completion_only = QualityMultiplier::compute(
            &ReaderSignals {
                starters: 100,
                finishers: 90,
                feedback_count: 10,
                positive_feedback: 0,
                rereaders: 0,
                bookmarkers: 0,
            },
            &t,
        );
        assert!(completion_only.completion_bonus);
        assert_eq!(
            completion_only.value, 1.3,
            "only the completion bonus fires"
        );

        let none_fire = QualityMultiplier::compute(
            &ReaderSignals {
                starters: 100,
                finishers: 10,
                feedback_count: 10,
                positive_feedback: 2,
                rereaders: 0,
                bookmarkers: 1,
            },
            &t,
        );
        assert_eq!(
            none_fire.value, 1.0,
            "below every threshold is neutral, not below"
        );
    }

    #[test]
    fn sitting_exactly_on_a_threshold_does_not_clear_it() {
        // §20.3 writes ">60%" etc. A reader at exactly 60% of 100 has not cleared
        // "more than 60%", and using >= would quietly pay out at the line.
        let m = QualityMultiplier::compute(
            &ReaderSignals {
                starters: 100,
                finishers: 60,
                feedback_count: 0,
                positive_feedback: 0,
                rereaders: 0,
                bookmarkers: 0,
            },
            &QualityThresholds::default(),
        );
        assert!(!m.completion_bonus, "60% is not >60%");
        assert_eq!(m.value, 1.0);
    }

    #[test]
    fn the_reader_floor_is_ten_and_is_inclusive() {
        let t = QualityThresholds::default();
        let just_under = QualityMultiplier::compute(
            &ReaderSignals {
                starters: MIN_READERS - 1,
                finishers: MIN_READERS - 1,
                feedback_count: 0,
                positive_feedback: 0,
                rereaders: 0,
                bookmarkers: 0,
            },
            &t,
        );
        assert!(!just_under.signals_active, "9 readers is below the floor");

        let at_floor = QualityMultiplier::compute(
            &ReaderSignals {
                starters: MIN_READERS,
                finishers: MIN_READERS,
                feedback_count: 0,
                positive_feedback: 0,
                rereaders: 0,
                bookmarkers: 0,
            },
            &t,
        );
        assert!(at_floor.signals_active, "10 readers meets it");
    }

    #[test]
    fn the_author_explanation_names_only_credit_given() {
        // §20.3 shows the breakdown "because it is actionable". The wording must
        // never reveal a shortfall, or the line becomes a league table of who is
        // closest to missing a bonus.
        let m = QualityMultiplier::compute(&strong(), &QualityThresholds::default());
        let text = m.author_explanation();
        assert!(text.contains("+80%"), "{text}");
        assert!(text.contains("high completion rate"), "{text}");

        // Assertion by ABSENCE, with no escape hatch. The previous version of this
        // test looped over forbidden tokens but guarded each with
        // `|| text.starts_with("Quality bonus: +")` -- a clause that is true for any
        // passing explanation, so the loop could never fail and the mutation
        // survived. A guard that cannot fail is not a guard.
        for forbidden in [
            "threshold",
            "needed",
            "below",
            "short of",
            "only ",
            "of 60",
            "of 80",
        ] {
            assert!(
                !text.contains(forbidden),
                "the explanation must not report a shortfall, found {forbidden:?} in {text:?}"
            );
        }
    }

    #[test]
    fn a_work_with_no_bonuses_is_told_it_is_building_history() {
        let m = QualityMultiplier::compute(
            &ReaderSignals {
                starters: 100,
                finishers: 5,
                feedback_count: 5,
                positive_feedback: 1,
                rereaders: 0,
                bookmarkers: 0,
            },
            &QualityThresholds::default(),
        );
        assert_eq!(m.value, 1.0);
        assert_eq!(
            m.author_explanation(),
            "Quality bonus: none yet (building reader history)."
        );
    }

    #[test]
    fn the_below_the_reader_floor_branch_also_names_no_shortfall() {
        // The `!signals_active` early return is a SECOND place wording is chosen,
        // and the first version of this suite only checked the fired-bonuses path.
        // So a mutation reworded this branch's string and stayed green. One test
        // per place the wording is decided -- the same rule that keeps biting.
        let m =
            QualityMultiplier::compute(&ReaderSignals::default(), &QualityThresholds::default());
        assert_eq!(
            m.author_explanation(),
            "Quality bonus: none yet (building reader history).",
            "a work under the reader floor says the same thing, with no shortfall named"
        );
        for forbidden in ["threshold", "needed", "below", "insufficient"] {
            assert!(
                !m.author_explanation().contains(forbidden),
                "found {forbidden:?} in {:?}",
                m.author_explanation()
            );
        }
    }

    #[test]
    fn a_partial_work_names_the_bonuses_it_earned_and_not_the_rest() {
        // The path where SOME bonuses fire: the wording must list only those, so a
        // reader cannot infer the others were evaluated and failed.
        let m = QualityMultiplier::compute(
            &ReaderSignals {
                starters: 100,
                finishers: 90,
                feedback_count: 4,
                positive_feedback: 1,
                rereaders: 30,
                bookmarkers: 1,
            },
            &QualityThresholds::default(),
        );
        let text = m.author_explanation();
        assert!(
            text.contains("completion") && text.contains("readers returning"),
            "{text}"
        );
        assert!(
            !text.contains("feedback"),
            "a bonus that did not fire is not named: {text}"
        );
        assert!(!text.contains("bookmarking"), "{text}");
        // 0.3 completion + 0.2 reread. Feedback is 1 of 4 = 25%, which does not
        // clear the 80% threshold, and bookmarks 1 of 100 does not clear 15% --
        // so neither fires. (I first wrote 1.7 here by adding a bonus that the
        // thresholds do not grant; the assertion caught my arithmetic, not a bug.)
        assert_eq!(m.value, 1.5, "0.3 completion + 0.2 reread, nothing else");
    }

    #[test]
    fn the_demand_multiplier_reaches_its_ceiling_and_floor() {
        assert_eq!(DemandMultiplier::compute(1.0, 1.0, 1.0).value, 1.5);
        assert_eq!(DemandMultiplier::compute(0.0, 0.0, 0.0).value, 1.0);
    }

    #[test]
    fn each_demand_component_is_clamped_independently() {
        // ONE case per component, deliberately. An earlier version clamped all
        // three at once (compute(9.0, 9.0, 9.0)), which left the suite green when
        // any single component's clamp was deleted -- the other two still pinned
        // the total to 1.5. Same shape as the digit/decimal redundancy: a shared
        // assertion covers neighbours, so a removed condition hides behind it.
        //
        // This matters more than tidiness: admin_taste_affinity is the §0.3 secret
        // signal, and an unbounded component is a way to inflate a payout past the
        // ceiling the spec states as hard.
        assert_eq!(
            DemandMultiplier::compute(9.0, 0.0, 0.0).value,
            1.25,
            "an out-of-range admin taste must not add past its 0.25"
        );
        assert_eq!(
            DemandMultiplier::compute(0.0, 9.0, 0.0).value,
            1.15,
            "an out-of-range wishlist must not add past its 0.15"
        );
        assert_eq!(
            DemandMultiplier::compute(0.0, 0.0, 9.0).value,
            1.10,
            "an out-of-range search demand must not add past its 0.10"
        );
        // And the negative side, per component, for the same reason.
        assert_eq!(DemandMultiplier::compute(-3.0, 0.0, 0.0).value, 1.0);
        assert_eq!(DemandMultiplier::compute(0.0, -3.0, 0.0).value, 1.0);
        assert_eq!(DemandMultiplier::compute(0.0, 0.0, -3.0).value, 1.0);
    }

    #[test]
    fn the_earnings_view_carries_the_two_lines_and_no_demand() {
        let m = QualityMultiplier::compute(&strong(), &QualityThresholds::default());
        let view = AuthorEarningsView::new(180, m);
        let lines = view.author_lines();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "Your work earned 180 credits this week.");
        assert!(lines[1].starts_with("Quality bonus: +80%"));

        // And the serialized view cannot leak the secret: the struct has no field
        // for it.
        let json = serde_json::to_string(&view).expect("serializes");
        assert!(!json.contains("demand"), "{json}");
        assert!(!json.contains("taste"), "{json}");
    }
}
