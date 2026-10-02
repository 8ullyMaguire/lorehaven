//! §53.6 — the earned-bookmark ratio, and the definition it rests on.
//!
//! This is a division over counts §20.3 already gathers, so the risk is not in a query
//! but in the arithmetic's edge cases and, more than that, in whether the *definition*
//! survives contact with the specification.
//!
//! The three that matter:
//!
//!   * a work with no bookmarks is EXCLUDED, not reported as zero — §53.6's explicit
//!     instruction, and the one that keeps §20.3's multipliers off a work that simply
//!     has no readers yet;
//!   * bookmarks with no completions is NOT infinity, because a multiplier that
//!     multiplied by infinity would corrupt a ledger row rather than merely skew it;
//!   * the basis is `Completion`, not §53.5's "finished or rated ≥4", because a
//!     four-star rating costs the one click this ratio exists to be measured against.

use lorehaven_domain::earned_bookmark::{EarnedBookmark, HitBasis};
use lorehaven_domain::payouts::ReaderSignals;

fn signals(bookmarkers: i64, finishers: i64) -> ReaderSignals {
    ReaderSignals {
        starters: finishers.max(1),
        finishers,
        feedback_count: 0,
        positive_feedback: 0,
        rereaders: 0,
        bookmarkers,
    }
}

// ── the ratio itself ────────────────────────────────────────────────────────

#[test]
fn the_ratio_is_bookmarks_per_completion() {
    let got = EarnedBookmark::compute(&signals(30, 10));
    assert_eq!(got.bookmarkers, 30);
    assert_eq!(got.finishers, 10);
    let ratio = got.ratio.expect("a ratio with both terms present");
    assert!(
        (ratio - 3.0).abs() < f64::EPSILON,
        "30 over 10 is 3, got {ratio}"
    );
    assert!(got.usable());
}

#[test]
fn a_ratio_below_one_is_a_real_answer_not_an_error() {
    // Fewer bookmarks than completions is ordinary: many readers finish what they
    // start and bookmark only what they intend to return to. A guard written as
    // `bookmarkers >= finishers` would be a plausible-looking invariant that rejects
    // most good works.
    let got = EarnedBookmark::compute(&signals(4, 40));
    let ratio = got.ratio.expect("a ratio");
    assert!((ratio - 0.1).abs() < 1e-9, "4 over 40 is 0.1, got {ratio}");
    assert!(got.usable(), "a low ratio is still a measurement");
}

#[test]
fn the_terms_are_kept_not_just_the_quotient() {
    // §53.6 requires the denominator to be REPORTED. A struct holding only the
    // quotient cannot satisfy that, and a ratio you cannot reproduce is one you
    // cannot audit.
    let got = EarnedBookmark::compute(&signals(7, 2));
    assert_eq!(got.bookmarkers, 7);
    assert_eq!(got.finishers, 2);
}

// ── the undefined cases, which are most of the interesting ones ─────────────

#[test]
fn a_work_with_no_bookmarks_has_no_ratio_and_is_not_a_zero() {
    // §53.6: "A work with no bookmarks is excluded, not counted as a ratio of zero."
    // Reporting 0.0 would feed §20.3 a multiplier of zero — an author earning nothing
    // for a work nobody has bookmarked, which is a statement about a work nothing
    // happened to.
    let got = EarnedBookmark::compute(&signals(0, 12));
    assert_eq!(got.ratio, None, "undefined, not zero");
    assert!(!got.usable(), "and §20.3 must not act on it");
    assert_eq!(
        got.finishers, 12,
        "the fact that is not a ratio is still there"
    );
}

#[test]
fn bookmarks_with_no_completions_is_not_infinity() {
    // 40 bookmarks and 0 completions is a real observation — and not a *ratio*.
    // `f64::INFINITY` would be arithmetically defensible and operationally poison: a
    // payout multiplier multiplied by infinity is not a wrong number in a ledger, it
    // is a corrupt row.
    let got = EarnedBookmark::compute(&signals(40, 0));
    assert_eq!(got.ratio, None, "not infinity");
    assert!(!got.usable());
    assert_eq!(got.bookmarkers, 40, "but the bookmarks are reported");
    assert_eq!(
        got.bookmarkers as f64 / got.finishers as f64,
        f64::INFINITY,
        "which is exactly the value being refused"
    );
}

#[test]
fn an_empty_window_has_no_ratio() {
    let got = EarnedBookmark::compute(&signals(0, 0));
    assert_eq!(got.ratio, None);
    assert!(!got.usable());
    assert!(got.summary_line().contains("no bookmarks"));
}

#[test]
fn usable_is_false_for_every_undefined_ratio() {
    // `usable` is the gate §20.3 will use, so it must not be a second opinion that
    // disagrees with `ratio` being `None`. One signal, checked once, here.
    for (bookmarks, finishers) in [(0, 0), (0, 5), (5, 0), (1, 1), (100, 1)] {
        let got = EarnedBookmark::compute(&signals(bookmarks, finishers));
        assert_eq!(
            got.usable(),
            got.ratio.is_some(),
            "{bookmarks} bookmarks, {finishers} completions: `usable` and `ratio` \
             must not disagree"
        );
        if let Some(ratio) = got.ratio {
            assert!(ratio.is_finite(), "{ratio} should never reach a multiplier");
        }
    }
}

// ── the definition, which is the actual content of the feature ──────────────

#[test]
fn the_basis_is_a_completion_and_not_four_stars() {
    // The decision, stated as an assertion so it cannot drift silently. §53.5 defines
    // "hit" as finished OR rated >=4; §53.6 deliberately does not reuse it, because a
    // four-star rating on chapter one costs exactly the one click this ratio is
    // measured against.
    let got = EarnedBookmark::compute(&signals(10, 5));
    assert_eq!(
        got.basis,
        HitBasis::Completion,
        "§53.6's hit is a completion alone"
    );
    assert_ne!(
        got.basis,
        HitBasis::FinishedOrFourStars,
        "and specifically NOT §53.5's broader sense"
    );
    assert_ne!(
        got.basis,
        HitBasis::View,
        "and never a view: a refresh is free"
    );
}

#[test]
fn ratings_do_not_change_the_ratio_at_all() {
    // The strongest form of the rule above. `ReaderSignals` carries `feedback_count`
    // and `positive_feedback`; §53.6's ratio must be blind to both. If a future change
    // wires ratings into the numerator, this fails with a number rather than a
    // discussion.
    let plain = signals(10, 5);
    let mut rated = plain;
    rated.feedback_count = 500;
    rated.positive_feedback = 500;

    let a = EarnedBookmark::compute(&plain);
    let b = EarnedBookmark::compute(&rated);
    assert_eq!(a.ratio, b.ratio, "ratings are not a hit in §53.6");
    assert_eq!(a, b, "and nothing else about the ratio moves either");
}

#[test]
fn a_view_never_counts_as_a_completion() {
    // `starters` is the closest thing to views in the signals. It must not be the
    // denominator — that is the "easy to inflate" option §53.6 refuses.
    let mut viewed = signals(10, 0);
    viewed.starters = 999;
    let got = EarnedBookmark::compute(&viewed);
    assert_eq!(got.ratio, None, "views do not make a denominator");
    assert_eq!(got.finishers, 0);
}

// ── what may be shown ───────────────────────────────────────────────────────

#[test]
fn the_summary_reports_its_terms_not_a_bare_number() {
    // §53.6 requires the denominator reported, and the reason a bare ratio is refused
    // is that 3.00 means something completely different over 3 completions than over
    // 300.
    let line = EarnedBookmark::compute(&signals(30, 10)).summary_line();
    assert!(line.contains("30"), "the numerator is visible: {line}");
    assert!(line.contains("10"), "and so is the denominator: {line}");
    assert!(line.contains("3.00"), "the ratio itself: {line}");
}

#[test]
fn the_summary_distinguishes_the_three_undefined_shapes() {
    // "no ratio" is one state; it has three causes, and an operator reading this line
    // is trying to tell them apart. Collapsing them into "n/a" loses the distinction
    // between "nobody bookmarked this" and "nobody finished it", which are opposite
    // problems.
    let none_at_all = EarnedBookmark::compute(&signals(0, 0)).summary_line();
    let no_bookmarks = EarnedBookmark::compute(&signals(0, 8)).summary_line();
    let no_completions = EarnedBookmark::compute(&signals(12, 0)).summary_line();

    assert!(none_at_all.contains("no bookmarks"), "{none_at_all}");
    assert!(no_bookmarks.contains("no bookmarks"), "{no_bookmarks}");
    assert!(
        no_completions.contains("no completions"),
        "a work people bookmark but nobody finishes is a different problem from one \
         nobody bookmarks: {no_completions}"
    );
    assert_ne!(no_bookmarks, no_completions);
}

// ── the seam with §20.3 ─────────────────────────────────────────────────────

#[test]
fn the_ratio_agrees_with_the_signals_it_was_built_from() {
    // The property that makes it safe to feed §20.3: both counts come from the same
    // query, so the ratio cannot claim a different set of readers than the payout did.
    let raw = ReaderSignals {
        starters: 40,
        finishers: 25,
        feedback_count: 12,
        positive_feedback: 9,
        rereaders: 3,
        bookmarkers: 60,
    };
    let via_method = raw.earned_bookmark();
    let via_function = EarnedBookmark::compute(&raw);
    assert_eq!(
        via_method, via_function,
        "the convenience method and the constructor must not be two implementations"
    );
    let ratio = via_method.ratio.expect("a ratio");
    assert!((ratio - 2.4).abs() < 1e-9, "60 over 25 is 2.4, got {ratio}");
}
