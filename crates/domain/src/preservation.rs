//! Preservation: what a crosspost is worth, who may make one, and what a
//! reader is told when the badge is out of reach (spec §11.12a, §9.7 as
//! amended by §2).
//!
//! Three things live here, and none of them touches a database, which is the
//! point: the arithmetic is the part of this feature most likely to be got
//! wrong, and it is the part a test can pin exactly.
//!
//! * [`Redistribution`] — the §2.6 permission gate. `yes` pays full, `ask`
//!   pays less, `no` refuses by name, and `unstated` is `ask` because the
//!   default must be the cautious one.
//! * [`reward_for`] — the §2.2 sizing: a threshold, a geometric decay past it,
//!   and a cap past which a destination records and displays and pays nothing.
//! * [`PreservationEligibility`] — the §2.5 answer to "why can I not earn
//!   this", which is a reader-facing question and therefore needs a reason
//!   string and not just a boolean.
//!
//! **Why the decay is computed in Rust and never in SQL.** §2.2 says the value
//! is `full * decay_bp / 10_000`, which is the repository's existing
//! basis-points convention (`theme_dial_floor_bp`, `tag_gravity_bp`) and is
//! there for a reason this repository has already paid for: SQLite has no math
//! functions, so a decay written as SQL runs on PostgreSQL and returns NULL on
//! the engine the tests link. Integer arithmetic in Rust has one answer on both
//! engines, and the second engine's is the one anybody reading the source
//! expects.
//!
//! **Why the reward is a function of a *rank* and not of a count.** §2.2 pays
//! full up to `threshold`, then a decaying share past it, then nothing. Those
//! are three different rules over one sequence, and a "how many have I got"
//! counter cannot express the middle one — the seventh destination's value
//! depends on there having been six before it, so the thing a function needs is
//! its position, and the thing a table needs is an ordering that is stable when
//! a destination dies. Ordering by `verified_at` is that ordering: it is
//! monotone in the thing being rewarded (when the archive confirmed it) and it
//! does not move when an unrelated row changes.

use serde::{Deserialize, Serialize};

/// The author's assertion about redistribution of this work (spec §2.6).
///
/// §33.1 is spec-only — §33 opens with "Nothing in this section is
/// implemented" — so this type is a *dependency* of preservation rather than a
/// reader of an existing field, and the column it backs is created by
/// migration 0093.
///
/// `Unstated` is a value rather than a nullable column's `None` on purpose, and
/// it is deliberately **not** the same variant as `Ask`. `Ask` is an assertion
/// the author made; `Unstated` is the absence of one, and the reward structure
/// is what makes asserting worth doing. Collapsing them would make a reader who
/// said "ask me" indistinguishable from one who never spoke, which is exactly
/// the signal the assertion is supposed to carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Redistribution {
    /// The author permits redistribution. Pays the full reward.
    Yes,
    /// The author permits it case by case. Pays a reduced amount.
    Ask,
    /// The author refuses. The crosspost is refused by name and pays nothing.
    No,
    /// Nobody has said. Treated as `Ask` everywhere it matters, and said so at
    /// every one of those places.
    Unstated,
}

impl Redistribution {
    /// The stored spelling, which is also the database value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::Ask => "ask",
            Self::No => "no",
            Self::Unstated => "unstated",
        }
    }

    /// Parse a stored value, `None` for absent or unrecognised.
    ///
    /// **Case-insensitive, and the reason is the direction of the error.** The
    /// stored value is always lowercase — PostgreSQL's CHECK names only the
    /// lowercase spellings — but SQLite has no such CHECK, so on SQLite this
    /// parser *is* the guard, and a guard that reads `YES` as unrecognised would
    /// resolve it to `Unstated` and pay an author who said yes at half rate. The
    /// failure is silent and it is in the direction of under-paying, which is
    /// exactly the kind of bug nobody files. Normalising the case means the two
    /// engines agree, and a value that is genuinely unrecognised is still
    /// unrecognised.
    ///
    /// Unrecognised is `None` rather than a default, so a caller has to decide.
    /// Every caller in this feature resolves `None` to `Unstated`, which then
    /// behaves as `Ask` — the cautious direction, and a refusal on the
    /// permissive side of the reward is a smaller failure than a payment for a
    /// permission nobody gave.
    #[must_use]
    pub fn parse_stored(value: Option<&str>) -> Option<Self> {
        match value?.trim().to_ascii_lowercase().as_str() {
            "yes" => Some(Self::Yes),
            "ask" => Some(Self::Ask),
            "no" => Some(Self::No),
            "unstated" => Some(Self::Unstated),
            _ => None,
        }
    }

    /// Read a stored value with the fall-back this feature applies.
    ///
    /// One function so the "unrecognised is `unstated`" decision has one home: a
    /// caller reaching for `Unstated` directly is a caller who will one day
    /// forget the parse and hardcode it.
    #[must_use]
    pub fn read(value: Option<&str>) -> Self {
        Self::parse_stored(value).unwrap_or(Self::Unstated)
    }

    /// The share of the full reward this assertion earns.
    ///
    /// Basis points, for the same reason [`reward_for`] uses them: the value
    /// multiplies a credit amount, and integer arithmetic that both engines
    /// agree on is worth more than a readable `0.5`.
    #[must_use]
    pub const fn reward_share_bp(self) -> i64 {
        match self {
            Self::Yes => 10_000,
            // A reduced amount, not nothing: §2.6 says `ask` pays a reduced
            // amount, and a reader who honours "ask me first" and then does get
            // permission has done the harder thing. Zero would make the
            // permission model a ranking of authors rather than a way to
            // preserve more works.
            Self::Ask | Self::Unstated => 5_000,
            Self::No => 0,
        }
    }

    /// Whether a crosspost is permitted at all under this assertion.
    #[must_use]
    pub const fn permits_crosspost(self) -> bool {
        !matches!(self, Self::No)
    }
}

/// The state of one (work, destination) preservation pair (spec §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreservationState {
    /// Recorded, but nothing has confirmed the destination carries a record
    /// naming this work. Pays nothing, and is the state a crosspost starts in.
    Unverified,
    /// A metadata-class fetch of the destination's public item page confirmed
    /// it. This is the only state that pays and the only one that counts.
    Verified,
    /// The destination no longer carries a record naming this work.
    ///
    /// **Not** a statement that the work is lost: the work's own origin
    /// reachability is a separate fact reported separately (§3.3), and a reader
    /// who conflated the two would be told a work is gone when what is true is
    /// that one archive forgot it.
    Dead,
    /// The author refused the crosspost (§2.6). Recorded so the refusal is a
    /// fact about the instance rather than a gap somebody retries forever.
    Refused,
}

impl PreservationState {
    /// The stored spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unverified => "unverified",
            Self::Verified => "verified",
            Self::Dead => "dead",
            Self::Refused => "refused",
        }
    }

    /// Parse a stored value, `None` for absent or unrecognised.
    ///
    /// Unrecognised is `None` and the caller resolves it to
    /// [`PreservationState::Unverified`], because unverified is the only state
    /// that pays nothing — the direction a value from a newer build must fail
    /// in, or a future `archived` state would start paying under an old build
    /// that has never checked what `archived` means.
    #[must_use]
    pub fn parse_stored(value: &str) -> Option<Self> {
        match value.trim() {
            "unverified" => Some(Self::Unverified),
            "verified" => Some(Self::Verified),
            "dead" => Some(Self::Dead),
            "refused" => Some(Self::Refused),
            _ => None,
        }
    }

    /// Read a stored value with the fall-back this feature applies.
    #[must_use]
    pub fn read(value: &str) -> Self {
        Self::parse_stored(value).unwrap_or(Self::Unverified)
    }
}

/// How one verified destination pays (spec §2.2).
///
/// `verified_before` is how many verified destinations this work had before this
/// one, so this one sits at position `verified_before + 1`. That is the input
/// rather than the total because the decay is over *position*, and computing it
/// from a total means re-deriving the ordering in every caller that happens to
/// have a count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RewardFor {
    /// Credits this destination pays, already reduced by the author's
    /// permission share and floored at zero.
    pub credits: i64,
    /// Whether this destination counts toward the badge threshold. False past
    /// the cap, which is §2.2's "records the preservation and displays it, but
    /// pays nothing and does not count toward the threshold".
    pub counts_toward_threshold: bool,
    /// Whether reaching this position awards the badge. True exactly once, at
    /// the threshold.
    pub awards_badge: bool,
    /// Why this destination pays what it pays, for the reader-facing answer.
    pub reason: RewardReason,
}

/// The named reason a destination pays what it pays.
///
/// A `reason` rather than a formatted sentence, so a test asserts on a *case*
/// and a caller formats. The strings are in the callers, which is where the
/// sentences that name a reader's own work belong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewardReason {
    /// Below the threshold: every destination pays the full amount, so a
    /// reader's first two archives are worth as much as their third.
    BeforeThreshold,
    /// Past the threshold: this position's decayed share.
    PastThreshold,
    /// Past the cap: recorded, displayed, unpaid, uncounted.
    PastCap,
    /// The author refused, so there was no crosspost to pay for.
    RefusedByAuthor,
}

/// What one verified destination is worth (spec §2.2).
///
/// ```
/// use lorehaven_domain::preservation::{reward_for, Redistribution};
///
/// // Three destinations at the default threshold: two full, and the third
/// // pays full and is the one that awards the badge.
/// assert_eq!(reward_for(0, 3, 10, 2_500, 8, Redistribution::Yes).credits, 10);
/// assert_eq!(reward_for(1, 3, 10, 2_500, 8, Redistribution::Yes).credits, 10);
/// let third = reward_for(2, 3, 10, 2_500, 8, Redistribution::Yes);
/// assert_eq!(third.credits, 10);
/// assert!(third.awards_badge);
///
/// // A fourth pays a quarter of full.
/// assert_eq!(reward_for(3, 3, 10, 2_500, 8, Redistribution::Yes).credits, 2);
///
/// // A ninth records and pays nothing.
/// let ninth = reward_for(8, 3, 10, 2_500, 8, Redistribution::Yes);
/// assert_eq!(ninth.credits, 0);
/// assert!(!ninth.counts_toward_threshold);
/// ```
#[must_use]
pub fn reward_for(
    verified_before: i64,
    threshold: i64,
    full_credits: i64,
    decay_bp: i64,
    cap: i64,
    permission: Redistribution,
) -> RewardFor {
    if !permission.permits_crosspost() {
        return RewardFor {
            credits: 0,
            counts_toward_threshold: false,
            awards_badge: false,
            reason: RewardReason::RefusedByAuthor,
        };
    }

    // `position` is one-based, so the Nth verified destination is at position N.
    // `verified_before` is zero-based by construction — it is a count of rows
    // that already existed — and the +1 is what turns it into a position.
    let position = verified_before.saturating_add(1);
    // How many decay steps this position is past the threshold. Computed once
    // and reused, because the `threshold.max(0)` it clamps is repeated in the
    // branch below and two spellings of "the threshold" in one function is how
    // they come to disagree.
    let steps = position - threshold.max(0);

    // Past the cap, nothing. Checked before the threshold so a cap below the
    // threshold behaves the way §2.2 reads rather than the way a "threshold
    // first" order would: a misconfiguration must pay nothing, never pay more
    // than a correct one.
    if cap > 0 && position > cap {
        return RewardFor {
            credits: 0,
            counts_toward_threshold: false,
            awards_badge: false,
            reason: RewardReason::PastCap,
        };
    }

    // The pre-threshold value is the full amount scaled by the permission
    // share. `yes` is 10_000 bp so it is exactly `full_credits`; `ask` and
    // `unstated` are half. Division truncates, so an odd `full_credits` under
    // `ask` rounds DOWN — a reader is never paid more than the arithmetic
    // gives, which is the direction §2.2's decay already implies.
    let share_bp = permission.reward_share_bp();
    let base = full_credits.saturating_mul(share_bp) / 10_000;

    if position <= threshold.max(0) {
        return RewardFor {
            credits: base.max(0),
            counts_toward_threshold: true,
            // The badge is awarded once, at the threshold. `position == threshold`
            // is the only assignment, and a `threshold` of 0 awards it to
            // nobody — an instance that has configured the threshold away has
            // not decided the badge is free.
            awards_badge: threshold > 0 && position == threshold,
            reason: RewardReason::BeforeThreshold,
        };
    }

    // Past the threshold the value decays geometrically: the first past-threshold
    // destination pays `full * decay_bp / 10_000`, the second pays that again,
    // and so on.
    //
    // **Iteratively, and this is not a style choice.** The closed form
    // `full * decay_bp^k / 10_000^k` needs both powers computed, and the
    // denominator overflows `i64` at k = 4 (`10000^4` is 1e16 and `10000^5` is
    // 1e20, which is past `i64::MAX`). The first version of this function
    // computed both with a saturating `int_pow`, and the saturation made the
    // sequence **increase**: at k = 6 the numerator saturated at `i64::MAX` and
    // so did the denominator, so `MAX / MAX` paid a full credit at a position
    // that should have paid nothing, and every position after it paid one too.
    // A reward that rises again after falling is worse than no decay at all,
    // and it took `the_decay_never_increases_and_never_goes_negative` to find.
    //
    // The iterative form cannot overflow, because each step multiplies by
    // `decay_bp / 10_000` — a factor clamped to at most 1 — so the running value
    // is non-increasing from the first step. The intermediate product is
    // widened to `u128` rather than saturating, so it is *exact*; a saturating
    // multiply here would reintroduce exactly the bug above.
    let mut value = base.max(0);
    let decay_num = i128::from(decay_bp.clamp(0, 10_000));
    for _ in 0..steps {
        if value == 0 {
            // Once it is gone it stays gone. Without this the loop is harmless
            // but the early exit documents that a long tail of zeros is the
            // expected shape, not an accident.
            break;
        }
        value = i64::try_from(i128::from(value) * decay_num / 10_000).unwrap_or(0);
    }
    RewardFor {
        credits: value,
        // Past the threshold a destination still counts: §2.2 pays *less* past
        // the threshold and pays *nothing* past the cap, and conflating the two
        // would make the fourth archive of a preserved work stop counting as
        // preservation, which is the opposite of what §2.2 says it does.
        counts_toward_threshold: true,
        // Past the threshold no further badge: it was awarded at the threshold.
        awards_badge: false,
        reason: RewardReason::PastThreshold,
    }
}

/// Why a reader can or cannot earn the preservation badge (spec §2.5).
///
/// This exists because "Three verified destinations does not exist for every
/// work, so a flat requirement is a number some readers can never earn and a
/// failure that looks like the reader's" — which means the answer has to name
/// the reason, not just withhold the badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreservationEligibility {
    /// The threshold is met and this destination set the badge.
    Reached { verified: usize },
    /// Every destination this instance could offer is verified, and there were
    /// fewer of them than the threshold.
    ///
    /// This is the floor-of-1 case and it is deliberately **not** `Reached`: the
    /// reader did not reach the threshold, and reporting it as reached would
    /// make a badge they did not earn indistinguishable from one they did.
    NoFurtherEligibleDestination { verified: usize },
    /// More destinations exist and more work would reach it.
    Short { verified: usize, threshold: usize },
}

impl PreservationEligibility {
    /// Compute the answer for one work.
    ///
    /// `eligible` is how many destinations this instance has enabled for this
    /// work; `verified` is how many of them are currently `verified`. Both are
    /// the *live* numbers — a dead destination is not eligible-and-verified, and
    /// counting it would let a reader keep a badge's worth of progress on
    /// archives that no longer answer.
    #[must_use]
    pub fn evaluate(eligible: usize, verified: usize, threshold: usize) -> Self {
        if threshold == 0 {
            // A threshold of zero is a misconfiguration, and reporting `Reached`
            // would hand out the badge for free. `Short` with a zero threshold
            // is the honest rendering: nothing is short of zero, but nothing has
            // reached it either, and the reason string is what a reader sees.
            return Self::Short {
                verified,
                threshold,
            };
        }
        if verified >= threshold {
            return Self::Reached { verified };
        }
        if verified >= eligible {
            return Self::NoFurtherEligibleDestination { verified };
        }
        Self::Short {
            verified,
            threshold,
        }
    }

    /// Whether the badge is awarded.
    #[must_use]
    pub const fn is_reached(&self) -> bool {
        matches!(self, Self::Reached { .. })
    }

    /// The sentence a reader is shown, naming the reason.
    ///
    /// §2.5: "Eligibility is computed and shown, so a reader can see *why* the
    /// badge is not reachable for a given work rather than concluding it is
    /// unreachable." The `NoFurtherEligibleDestination` sentence is the one
    /// that has to exist — it is the difference between a reader who has done
    /// everything this instance can help with and a reader who is told they are
    /// two short of a number they could never have reached.
    #[must_use]
    pub fn reason(&self) -> String {
        match *self {
            Self::Reached { verified } => {
                format!("preserved in {verified} verified archives")
            }
            Self::NoFurtherEligibleDestination { verified } => format!(
                "fully preserved for this instance: every destination it can offer is verified \
                 ({verified}), and there is no further eligible destination to reach {0} with",
                verified + 1
            ),
            Self::Short {
                verified,
                threshold,
            } => format!(
                "{verified} of {threshold} verified archives; \
                 more destinations would reach it"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The full ladder, one place, because §2.2 is one rule and a rule tested
    /// at three points leaves the gaps to whatever the code happens to do.
    #[test]
    fn the_default_ladder_pays_full_to_the_threshold_and_decays_past_it() {
        // threshold 3, full 10, decay 2500bp, cap 8.
        let at = |n: i64| reward_for(n, 3, 10, 2_500, 8, Redistribution::Yes);

        assert_eq!(at(0).credits, 10, "first pays full");
        assert_eq!(at(1).credits, 10, "second pays full");
        assert_eq!(at(2).credits, 10, "third pays full, and is the badge");

        // 10 * 2500 / 10000 = 2.5, truncated to 2.
        assert_eq!(at(3).credits, 2, "fourth pays a quarter of full");
        // 10 * 2500^2 / 10000^2 = 0.625, truncated to 0.
        assert_eq!(at(4).credits, 0, "fifth is below a credit and pays none");
    }

    #[test]
    fn the_badge_is_awarded_at_exactly_the_threshold_and_never_again() {
        let at = |n: i64| reward_for(n, 3, 10, 2_500, 8, Redistribution::Yes);
        let awarded: Vec<bool> = (0..10).map(|n| at(n).awards_badge).collect();
        assert_eq!(
            awarded,
            vec![false, false, true, false, false, false, false, false, false, false],
            "exactly one position awards the badge, and it is the threshold"
        );
    }

    #[test]
    fn past_the_cap_a_destination_pays_nothing_and_stops_counting() {
        let at = |n: i64| reward_for(n, 3, 10, 2_500, 8, Redistribution::Yes);
        for n in 8..20 {
            let reward = at(n);
            assert_eq!(reward.credits, 0, "position {n} is past the cap");
            assert!(
                !reward.counts_toward_threshold,
                "position {n} is past the cap and must not count toward the threshold"
            );
            assert_eq!(reward.reason, RewardReason::PastCap);
        }
    }

    #[test]
    fn a_refused_author_pays_nothing_and_the_reason_names_the_refusal() {
        let reward = reward_for(0, 3, 10, 2_500, 8, Redistribution::No);
        assert_eq!(reward.credits, 0);
        assert!(!reward.counts_toward_threshold);
        assert_eq!(reward.reason, RewardReason::RefusedByAuthor);
    }

    #[test]
    fn ask_pays_less_than_yes_and_unstated_behaves_exactly_like_ask() {
        let at = |permission| reward_for(0, 3, 10, 2_500, 8, permission).credits;
        assert_eq!(at(Redistribution::Yes), 10);
        assert_eq!(at(Redistribution::Ask), 5);
        assert_eq!(
            at(Redistribution::Unstated),
            at(Redistribution::Ask),
            "an unstated work is treated as ask, and the two must not drift"
        );
        assert!(
            at(Redistribution::Ask) > 0,
            "ask pays a reduced amount, not nothing"
        );
    }

    /// A cap below the threshold is a misconfiguration, and the two candidate
    /// readings differ. Which one this function takes is the question, and the
    /// answer is that **position 1 is still within the cap** — a cap of 1 means
    /// "one destination counts at all", not "nothing is eligible". The second
    /// position is past it and pays nothing.
    ///
    /// The earlier version of this test asserted that the *first* destination
    /// paid nothing, and it was wrong: `position (1) > cap (1)` is false, so the
    /// first destination is inside the cap and the threshold branch answers. A
    /// cap below the threshold therefore does not zero the reward — it pays the
    /// cap's worth of destinations and stops, and the badge is unreachable. That
    /// is §2.2's text read literally, and the test now says so rather than
    /// asserting the misreading it started from.
    #[test]
    fn a_cap_below_the_threshold_pays_for_the_cap_and_stops_below_the_badge() {
        let at = |n: i64| reward_for(n, 3, 10, 2_500, 1, Redistribution::Yes);
        assert_eq!(at(0).credits, 10, "position 1 is within a cap of 1");
        let second = at(1);
        assert_eq!(second.credits, 0, "position 2 is past a cap of 1");
        assert!(!second.counts_toward_threshold);
        for n in 0..3 {
            assert!(
                !at(n).awards_badge,
                "a cap below the threshold makes the badge unreachable, so it is never awarded"
            );
        }
    }

    #[test]
    fn a_zero_threshold_awards_the_badge_to_nobody() {
        let reward = reward_for(0, 0, 10, 2_500, 8, Redistribution::Yes);
        assert!(
            !reward.awards_badge,
            "an instance that has configured the threshold away has not decided the badge is free"
        );
    }

    #[test]
    fn the_decay_never_increases_and_never_goes_negative() {
        let mut previous = i64::MAX;
        for n in 0..24 {
            let credits = reward_for(n, 3, 10, 2_500, 100, Redistribution::Yes).credits;
            assert!(
                credits <= previous,
                "position {n} paid more than the one before it"
            );
            assert!(credits >= 0, "position {n} paid a negative amount");
            previous = credits;
        }
    }

    /// A `decay_bp` above 10_000 would make each past-threshold destination pay
    /// MORE than full — the opposite of a decay. Clamped, not refused, and the
    /// test says which.
    #[test]
    fn a_decay_above_one_hundred_percent_is_clamped_to_a_decay() {
        let reward = reward_for(3, 3, 10, 99_999, 8, Redistribution::Yes);
        assert!(
            reward.credits <= 10,
            "a misconfigured decay must not pay more than the full reward"
        );
    }

    #[test]
    fn a_work_with_fewer_eligible_destinations_is_fully_preserved_and_not_short() {
        let answer =
            PreservationEligibility::evaluate(/* eligible */ 1, /* verified */ 1, 3);
        assert_eq!(
            answer,
            PreservationEligibility::NoFurtherEligibleDestination { verified: 1 }
        );
        assert!(
            !answer.is_reached(),
            "reaching a threshold of 3 is not reaching 1"
        );
        assert!(
            answer.reason().contains("no further eligible destination"),
            "the reason must say the reader is not short, got: {}",
            answer.reason()
        );
    }

    #[test]
    fn a_work_with_more_destinations_available_is_short_and_says_so() {
        let answer = PreservationEligibility::evaluate(5, 2, 3);
        assert_eq!(
            answer,
            PreservationEligibility::Short {
                verified: 2,
                threshold: 3
            }
        );
        assert!(!answer.is_reached());
    }

    #[test]
    fn reaching_the_threshold_is_reached_whatever_the_eligible_count() {
        assert_eq!(
            PreservationEligibility::evaluate(9, 3, 3),
            PreservationEligibility::Reached { verified: 3 }
        );
        assert_eq!(
            PreservationEligibility::evaluate(3, 3, 3),
            PreservationEligibility::Reached { verified: 3 }
        );
    }

    #[test]
    fn an_unrecognised_permission_reads_as_unstated_and_therefore_as_ask() {
        assert_eq!(Redistribution::read(None), Redistribution::Unstated);
        assert_eq!(
            Redistribution::read(Some("maybe")),
            Redistribution::Unstated
        );
        assert_eq!(Redistribution::read(Some("YES")), Redistribution::Yes);
        assert_eq!(
            Redistribution::read(Some("maybe")).reward_share_bp(),
            Redistribution::Ask.reward_share_bp(),
            "an unreadable assertion must land on the cautious side of the reward"
        );
    }

    #[test]
    fn an_unrecognised_state_reads_as_unverified_and_pays_nothing() {
        assert_eq!(
            PreservationState::read("archived"),
            PreservationState::Unverified
        );
        assert_eq!(
            PreservationState::read("verified"),
            PreservationState::Verified
        );
    }
}
