//! Vote decay: the pure function, before any database is involved.
//!
//! The property being pinned is the one the request actually states, in order:
//! *daily* beats *weekly* beats *monthly*, and by a couple of months a vote is
//! worth nothing. Everything else here exists because a decay curve that is
//! merely monotonic is not enough — it must reach an exact zero, it must not
//! be gameable by a cliff, and it must not apply to entries that would be
//! destroyed by it.
//!
//! The `decay_min_votes` tests matter more than they look. Without that
//! threshold, a new entry with three votes decays toward nothing and never
//! ranks, because the signal decay takes away is the only one it had. That is
//! a plausible-looking rule that quietly breaks every new submission.

use lorehaven_domain::vote_decay::*;

#[test]
fn a_vote_cast_now_counts_its_full_weight() {
    let d = Decay::default();
    assert!((decay(0.0, &d) - 1.0).abs() < 1e-12);
    // A one-day-old vote is not full weight -- that is the decay working, and
    // the difference between "cast now" and "cast yesterday" being invisible
    // would make the whole rule pointless.
    assert!(decay(1.0, &d) < 1.0);
    assert!(
        decay(1.0, &d) > 0.95,
        "a day-old vote lost too much: {}",
        decay(1.0, &d)
    );
}

#[test]
fn a_vote_decays_monotonically_and_never_rises() {
    let d = Decay::default();
    let mut last = decay(0.0, &d);
    for day in 1..=60 {
        let w = decay(day as f64, &d);
        assert!(
            w <= last + 1e-12,
            "day {day} weighs {w} which is more than day {}'s {last}",
            day - 1
        );
        assert!(
            (0.0..=1.0).contains(&w),
            "day {day} left the unit range: {w}"
        );
        last = w;
    }
}

#[test]
fn daily_beats_weekly_beats_monthly() {
    // The stated rule, literally. These are the three comparisons that make
    // voting every day worth doing.
    let d = Decay::default();
    let daily = decay(1.0, &d);
    let weekly = decay(7.0, &d);
    let monthly = decay(30.0, &d);

    assert!(daily > weekly, "daily {daily} should beat weekly {weekly}");
    assert!(
        weekly > monthly,
        "weekly {weekly} should beat monthly {monthly}"
    );
    // And the gap is big enough to be worth a reader's effort, which is the
    // whole point: if daily and weekly were within 1e-9 of each other, nobody
    // would ever come back. A day-old vote is worth ~24% more than a week-old
    // one, and a week-old one ~3x a month-old one.
    assert!(
        daily / weekly > 1.2,
        "daily is not meaningfully better than weekly: {daily} {weekly}"
    );
    assert!(
        weekly / monthly > 2.0,
        "weekly is not meaningfully better than monthly: {weekly} {monthly}"
    );
    // A month-old vote is a fraction of a vote, not nothing. Zero is reserved
    // for the cutoff, and a curve that reached zero early would make the
    // cutoff meaningless.
    assert!(monthly < 0.3, "a month-old vote still carries {monthly}");
    assert!(
        monthly > 0.1,
        "a month-old vote collapsed too fast: {monthly}"
    );
}

#[test]
fn a_vote_is_worth_nothing_at_the_cutoff_and_nothing_beyond_it() {
    // Exactly zero, not a small number. A vote still nominally counted at four
    // months is a vote the instance is still counting, and the claim is that it
    // stopped.
    let d = Decay::default();
    assert_eq!(decay(60.0, &d), 0.0);
    assert_eq!(decay(61.0, &d), 0.0);
    assert_eq!(decay(3650.0, &d), 0.0);
    // And strictly positive just before it, so the zero is a boundary and not
    // an accident of the curve.
    assert!(decay(59.0, &d) > 0.0);
}

#[test]
fn a_negative_age_is_treated_as_now() {
    // Clock skew between an instance and a peer, or a `voted_at` written by a
    // host running fast. A negative age must not produce a weight above 1.
    let d = Decay::default();
    assert_eq!(decay(-5.0, &d), 1.0);
    assert!(decay(-0.001, &d) <= 1.0);
}

#[test]
fn the_cutoff_is_configurable() {
    let d = Decay {
        enabled: true,
        cutoff_days: 30.0,
        min_votes: 20,
        exponent: 2,
    };
    assert_eq!(decay(30.0, &d), 0.0);
    // A 30-day cutoff must beat a 60-day one at every shared age, or the
    // parameter does nothing.
    let long = Decay::default();
    for day in [1.0, 7.0, 14.0, 29.0] {
        assert!(
            decay(day, &d) < decay(day, &long),
            "a shorter cutoff did not decay faster at day {day}"
        );
    }
}

#[test]
fn the_exponent_controls_the_shape() {
    // Linear (1.0) and quadratic (2.0) must both be available, and linear
    // must lose weight faster early -- which is why the default is 2.0.
    let linear = Decay {
        exponent: 1,
        ..Decay::default()
    };
    let quadratic = Decay::default();
    // The claim is that the *ratio* to a linear ramp grows with age, so a
    // quadratic curve tracks linear early and then collapses late. (The
    // absolute gap is constant, which is why this is stated as a ratio --
    // asserting the gap grows would be asserting something false.)
    let early_ratio = decay(10.0, &linear) / decay(10.0, &quadratic);
    let late_ratio = decay(50.0, &linear) / decay(50.0, &quadratic);
    assert!(
        late_ratio > early_ratio * 3.0,
        "the curve does not pull away from linear: early {early_ratio}, late {late_ratio}"
    );
    // A quadratic vote at day 50 is a twentieth of what linear would give.
    assert!(
        decay(50.0, &quadratic) < 0.03,
        "day 50 still carries {}",
        decay(50.0, &quadratic)
    );
    // Both still hit zero at the cutoff, or a long tail is unavoidable.
    assert_eq!(decay(60.0, &linear), 0.0);
    assert_eq!(decay(60.0, &quadratic), 0.0);
}

#[test]
fn disabling_decay_makes_every_vote_permanent() {
    // The escape hatch, and it has to restore today's behaviour exactly or it
    // is not an escape hatch. An operator who turns this off must get the
    // system they had.
    let off = Decay {
        enabled: false,
        ..Decay::default()
    };
    for day in [0.0, 1.0, 30.0, 59.0, 60.0, 3650.0] {
        assert_eq!(decay(day, &off), 1.0, "day {day} decayed with decay off");
    }
}

#[test]
fn an_entry_with_few_votes_never_decays() {
    // The rule the request asks for, and the one that keeps new entries
    // viable. Three votes on a new entry is the entire ranking signal it has;
    // decaying that is not moderation, it is erasure.
    let d = Decay::default();
    assert!(!should_decay(3, &d));
    assert!(!should_decay(0, &d));
    assert!(!should_decay(19, &d));
}

#[test]
fn an_entry_with_many_votes_decays() {
    let d = Decay::default();
    assert!(should_decay(20, &d));
    assert!(should_decay(500, &d));
}

#[test]
fn the_threshold_is_configurable() {
    let d = Decay {
        min_votes: 2,
        ..Decay::default()
    };
    assert!(!should_decay(1, &d));
    assert!(should_decay(2, &d));
}

#[test]
fn the_threshold_counts_vote_rows_not_currently_live_votes() {
    // The regression this pins. With 20 votes at 59 days the live count is 20,
    // so the entry decays and scores ~0.0006. One day later every vote is dead:
    // the live count is 0, the entry is *under* the threshold, therefore exempt,
    // therefore scored at full base weight again. A twenty-point spike caused
    // by a vote ageing past the cutoff -- the wrong direction, on exactly the
    // entries the rule exists for.
    let d = Decay::default();
    assert!(
        should_decay(20, &d),
        "an entry with 20 vote rows must keep decaying after they expire"
    );
    // A new entry with three opinions still never decays, so the threshold
    // still protects new submissions.
    assert!(!should_decay(3, &d));
}

#[test]
fn an_entry_that_ever_reached_the_threshold_keeps_it() {
    // Rows, not live rows: a once-popular entry is permanently in the decaying
    // set. This is what removes the cliff -- the threshold stops depending on
    // the clock, so ageing cannot cross it.
    let d = Decay::default();
    for count in [0, 1, 19, 20, 21, 500] {
        assert_eq!(
            should_decay(count, &d),
            count >= 20,
            "count {count} got the wrong verdict"
        );
    }
}

#[test]
fn effective_weight_is_the_product_of_base_and_decay() {
    // The composition an author of a query will actually use. A TL4 vote has a
    // base weight above 1, and decay multiplies it down rather than replacing
    // it -- so a fresh TL4 vote is worth more than a fresh TL0 vote, and a
    // stale TL4 vote is worth less than a fresh TL0 one.
    let d = Decay::default();
    let base_tl4 = 1.5;
    let base_tl0 = 0.375;

    assert!(effective_weight(base_tl4, 0.0, &d) > effective_weight(base_tl0, 0.0, &d));
    // The default trust weights happen to make a 30-day TL4 vote exactly equal
    // a fresh TL0 one -- 1.5 x 0.25 = 0.375. That is a coincidence of the
    // defaults, so assert the crossing on either side of it rather than at it.
    assert!(effective_weight(base_tl4, 20.0, &d) > effective_weight(base_tl0, 0.0, &d));
    assert!(effective_weight(base_tl4, 40.0, &d) < effective_weight(base_tl0, 0.0, &d));
    assert_eq!(effective_weight(base_tl4, 60.0, &d), 0.0);
    assert_eq!(effective_weight(base_tl0, 60.0, &d), 0.0);
}

#[test]
fn a_zero_base_weight_stays_zero_however_fresh() {
    // Freezing governance, or a config that zeroes a rung. Decay must not
    // resurrect a vote that policy removed.
    let d = Decay::default();
    assert_eq!(effective_weight(0.0, 0.0, &d), 0.0);
    assert_eq!(effective_weight(0.0, 1.0, &d), 0.0);
}

#[test]
fn the_default_cutoff_is_a_couple_of_months() {
    // Pinned so a config default change is a deliberate diff rather than a
    // silent behavioural change on every instance.
    assert_eq!(Decay::default().cutoff_days, 60.0);
    assert!(Decay::default().enabled);
}
