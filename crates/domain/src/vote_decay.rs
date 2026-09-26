//! Vote decay: a vote is a current statement, not a permanent ballot.
//!
//! # The problem
//!
//! A permanent vote measures *when someone first noticed an entry*, not what
//! anyone believes now. An entry approved in 2019 and never revisited
//! outranks one with a lively, current consensus, and a reader who now knows
//! better has no way to say so.
//!
//! # The rule
//!
//! Weight at read time is `base × decay(age)`. `base` is the trust-and-taste
//! weight from the moment of voting (unchanged, and still never derived from
//! credits, purchases or anything but behaviour — §0.3, §39.4). `decay` is
//! what this module computes.
//!
//! # Why the curve is squared
//!
//! `(1 - age/cutoff) ^ 2` loses weight gently at first and fast at the end.
//! That matches the intent: a day-old vote is barely diminished, a week-old
//! one is meaningfully less, a month-old one is nearly gone, and a
//! two-month-old one is exactly nothing. A linear ramp would make a fresh vote
//! worth 98% — which overstates it relative to the vote cast an hour ago and
//! makes the early differences too small to be worth a reader's attention.
//!
//! # Why the minimum-vote threshold
//!
//! Without it, a new entry with three votes decays toward zero and never
//! ranks — because the ranking signal decay removes is the only signal it
//! had. That is a plausible-looking rule that quietly erases every new
//! submission, so an entry below `min_votes` **live** votes does not decay at
//! all, at any age.
//!
//! The threshold counts vote *rows*, not currently-contributing votes. See
//! [`should_decay`] for why the live-vote version had a cliff in it.
//!
//! # Why a vote at the cutoff is exactly zero
//!
//! Not "approaching zero", and not a floor of 0.01. A vote still nominally
//! counted at four months is a vote the instance is still counting, and the
//! whole claim is that it stopped.

/// How vote decay behaves on this instance.
///
/// Every field is operator-configurable; the defaults are the behaviour the
/// design document asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Decay {
    /// Master switch. `false` restores permanent votes exactly.
    pub enabled: bool,
    /// Age in days at which a vote is worth exactly nothing. Default 60.
    pub cutoff_days: f64,
    /// Entries with fewer *live* votes than this never decay. Default 20.
    pub min_votes: i64,
    /// The curve's shape: 1 is linear, 2 the default gentle-then-sharp.
    ///
    /// An **integer**, and that is not an aesthetic choice. The score query
    /// computes this curve in SQL, and sqlx's bundled SQLite has *no* math
    /// functions at all — `POWER`, `exp`, `ln` and `sqrt` all fail with
    /// "no such function" — so the only curves expressible in portable SQL
    /// are integer powers, which are plain repeated multiplication. A
    /// fractional exponent would be computable in Rust and unreachable in SQL,
    /// and the two would then disagree on the score with nothing to report it.
    pub exponent: u32,
}

impl Default for Decay {
    fn default() -> Self {
        Self {
            enabled: true,
            cutoff_days: 60.0,
            min_votes: 20,
            exponent: 2,
        }
    }
}

impl Decay {
    /// Build from the config file's four values, falling back to the defaults.
    ///
    /// A zero or negative cutoff would make every vote worthless, which is a
    /// configuration mistake an operator would only find by noticing a list
    /// that stopped ranking. It falls back to the default instead.
    #[must_use]
    pub fn from_config(enabled: bool, cutoff_days: f64, min_votes: i64, exponent: u32) -> Self {
        Self {
            enabled,
            cutoff_days: if cutoff_days > 0.0 { cutoff_days } else { 60.0 },
            min_votes: min_votes.max(0),
            // 0 would make every live vote worth `1.0` regardless of age,
            // which is the linear curve inverted into a constant.
            exponent: exponent.max(1),
        }
    }
}

/// The decay multiplier for a vote of `age_days`.
///
/// Returns a value in `0.0..=1.0`. Age is clamped at zero, so a clock skew
/// between hosts — or a `voted_at` written by a machine running fast — cannot
/// produce a weight above 1.
#[must_use]
pub fn decay(age_days: f64, cfg: &Decay) -> f64 {
    if !cfg.enabled {
        return 1.0;
    }
    // A NaN age would slip through both comparisons and return 0.0 via the
    // `t <= 0.0` branch on some platforms and 1.0 on others; treat it as now.
    if age_days.is_nan() {
        return 1.0;
    }
    let age = age_days.max(0.0);
    if age >= cfg.cutoff_days {
        return 0.0;
    }
    let remaining = 1.0 - (age / cfg.cutoff_days);
    // Repeated multiplication, not `powf`, so the Rust curve and the SQL curve
    // are the same operation. `powf(2.0)` and `t * t` agree to the last bit,
    // but `powf` would leave a reader wondering which one the database runs.
    let mut w = 1.0;
    for _ in 0..cfg.exponent {
        w *= remaining;
    }
    w
}

/// Whether an entry with `vote_count` votes decays at all.
///
/// The count is of **vote rows**, not of votes still above zero.
///
/// This was live-vote-counting first, and it was wrong in a way that only
/// showed up once an entry aged past the cutoff. With 20 votes at 59 days the
/// count of live votes is 20, so the entry decays and scores ~0.0006. One day
/// later every vote is dead, the live count is 0, the entry is *under* the
/// threshold — and therefore exempt, and therefore scored at full base weight
/// again. A twenty-point spike caused by a vote ageing past the cutoff, in the
/// wrong direction, on exactly the entries the rule was written for.
///
/// Counting rows makes the threshold a property of the entry rather than of the
/// clock. An entry that has ever attracted `min_votes` opinions keeps decaying;
/// a new one never starts. Both are stable, and the cliff is gone.
#[must_use]
pub fn should_decay(vote_count: i64, cfg: &Decay) -> bool {
    cfg.enabled && vote_count >= cfg.min_votes
}

/// A vote's contribution to the score: base weight, decayed by its age.
///
/// Decay **multiplies** the base weight rather than replacing it, so the trust
/// ordering survives: a fresh TL4 vote still outweighs a fresh TL0 one, while
/// a month-old TL4 vote falls below a fresh TL0 one — which is the correct
/// answer, because a stale assertion of a high-trust reader is a weaker signal
/// than a current one from anyone.
#[must_use]
pub fn effective_weight(base_weight: f64, age_days: f64, cfg: &Decay) -> f64 {
    // A zero base weight is a policy decision (frozen governance, a zeroed
    // rung) and decay must not resurrect it. Multiplying would handle this
    // anyway; the branch documents that the zero is deliberate.
    if base_weight == 0.0 {
        return 0.0;
    }
    base_weight * decay(age_days, cfg)
}

/// Age in days from a `voted_at` timestamp to `now`, both RFC 3339.
///
/// Kept here rather than in the DB layer so the arithmetic is testable without
/// a database, and so a caller cannot accidentally mix up the two timestamp
/// formats the instance stores (`voted_at` is RFC 3339; `created_at` on some
/// tables is a SQLite-style space separator).
#[must_use]
pub fn age_days(voted_at: &str, now: &str) -> f64 {
    let (Ok(v), Ok(n)) = (
        time::OffsetDateTime::parse(voted_at, &time::format_description::well_known::Rfc3339),
        time::OffsetDateTime::parse(now, &time::format_description::well_known::Rfc3339),
    ) else {
        // An unparseable timestamp must not silently zero a vote. Treating it
        // as "now" keeps the vote at full weight, which is the safe direction:
        // it errs toward the instance over-counting rather than under-counting,
        // and a vote only ever decays when we know how old it is.
        return 0.0;
    };
    let seconds = (n - v).whole_seconds();
    if seconds <= 0 {
        return 0.0;
    }
    seconds as f64 / 86_400.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_malformed_cutoff_falls_back_to_the_default() {
        // A zero cutoff would make every vote worthless, and an operator would
        // only find out by noticing a list that stopped ranking.
        let d = Decay::from_config(true, 0.0, 20, 2);
        assert_eq!(d.cutoff_days, 60.0);
        let d = Decay::from_config(true, -5.0, 20, 2);
        assert_eq!(d.cutoff_days, 60.0);
    }

    #[test]
    fn a_zero_exponent_clamps_to_one() {
        // Exponent 0 would make `w` stay at 1.0 for every age -- the linear
        // curve inverted into a constant, so nothing would ever decay.
        assert_eq!(Decay::from_config(true, 60.0, 20, 0).exponent, 1);
    }

    #[test]
    fn the_curve_matches_an_explicit_power() {
        // The multiplication loop has to agree with the mathematical power it
        // stands for, or "squared" is a name rather than a description.
        let cfg = Decay::default();
        for day in [0.0, 1.0, 17.0, 45.0, 59.0] {
            let t = 1.0 - day / 60.0;
            assert!((decay(day, &cfg) - t * t).abs() < 1e-12, "day {day}");
        }
    }

    #[test]
    fn a_negative_threshold_clamps_to_zero() {
        // Zero means every entry decays, which is a legitimate (if unwise)
        // choice, so it is permitted rather than rejected.
        assert_eq!(Decay::from_config(true, 60.0, -3, 2).min_votes, 0);
    }

    #[test]
    fn a_nan_age_is_treated_as_now() {
        assert_eq!(decay(f64::NAN, &Decay::default()), 1.0);
    }

    #[test]
    fn an_unparseable_timestamp_does_not_zero_a_vote() {
        // Erring toward the instance over-counting, because we only ever decay
        // a vote when we know how old it is.
        assert_eq!(age_days("not a timestamp", "2026-01-01T00:00:00Z"), 0.0);
        assert_eq!(
            age_days("2026-01-01T00:00:00Z", "also not a timestamp"),
            0.0
        );
    }

    #[test]
    fn age_is_measured_in_days() {
        let now = "2026-03-01T00:00:00Z";
        assert!((age_days("2026-03-01T00:00:00Z", now) - 0.0).abs() < 1e-9);
        assert!((age_days("2026-02-22T00:00:00Z", now) - 7.0).abs() < 1e-9);
        assert!((age_days("2026-01-01T00:00:00Z", now) - 59.0).abs() < 1e-9);
    }

    #[test]
    fn a_future_timestamp_is_treated_as_now() {
        // Clock skew, not a vote from the future. It must not exceed full
        // weight and must not go negative.
        let now = "2026-01-01T00:00:00Z";
        assert_eq!(age_days("2026-06-01T00:00:00Z", now), 0.0);
    }

    #[test]
    fn decay_and_should_decay_compose_the_way_the_query_uses_them() {
        // The score query checks the threshold and then multiplies, so the two
        // must agree at the boundary: an entry with exactly `min_votes` live
        // votes decays, and its oldest vote contributes exactly nothing.
        let cfg = Decay::default();
        let live = cfg.min_votes;
        assert!(should_decay(live, &cfg));
        assert_eq!(effective_weight(1.0, cfg.cutoff_days, &cfg), 0.0);
    }
}
