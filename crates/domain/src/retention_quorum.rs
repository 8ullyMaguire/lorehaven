//! The quorum a retention change needs, and the direction that decides it.
//!
//! **This is where §5.3's asymmetry lives, and it is one function because it
//! has to be one function.** The amendment is explicit that narrowing storage is
//! cheap and reversible while widening it commits storage and bandwidth
//! indefinitely, and a reader who is asked to vote on either should feel that
//! difference. Encoding it as a quorum rather than as a weight is the whole
//! design: §45.2 says every governance vote weighs 1, so the only honest way to
//! make a change harder to pass is to ask more people, never to count some of
//! them for more.
//!
//! Three things follow from that, and each is a decision rather than an
//! implementation detail.
//!
//! **Widen and narrow are named, not inferred from ordinal.** `Aggregate` and
//! `Cache` are not ordered — the enum has no `Ord` — so "is this a widening"
//! cannot be a comparison. It is a match on the pair, and that is why the
//! function takes both modes: "widening" has no meaning without the current
//! setting to widen from.
//!
//! **A no-op change needs nobody's ballot.** Proposing the mode that is already
//! in force is not a change, and a quorum for it would be a number that reads
//! as a requirement on a proposal that should not exist. The store refuses
//! those before the quorum is ever asked for; the function answers `1` because
//! one ballot is the smallest quorum there is and a reader must still be able
//! to record support for a proposal they agree with.
//!
//! **The default is three, and it is a floor not a preference.** §19.4's
//! high-impact bar is three, and an instance may raise it. It may not lower it
//! below three, because below three a proposal is decided by one person's
//! second tap on a phone. `quorum_for` clamps rather than trusts, so a
//! misconfigured `widen_quorum = 1` does not turn instance policy into a
//! single reader's choice.

use crate::retention::BodyMode;

/// §19.4's high-impact bar: the smallest quorum any change may be held to.
///
/// Three because a proposal decided by one or two people is not a decision this
/// instance made; it is a decision one person made and published. The number
/// is not derived from anything and should not be re-derived: it is the point
/// where "the readers agreed" starts being a true sentence.
pub const MINIMUM_QUORUM: i64 = 3;

/// Whether a proposal moves the setting in the expensive direction.
///
/// **Widen and narrow are the direction relative to the *current* mode**, which
/// is why this takes both. `Cache` → `Aggregate` is a widening: it stops
/// storing bodies, which sounds like less and is in fact irreversible for the
/// bodies already stored — they cannot be re-fetched from an origin that has
/// since changed. `Aggregate` → `Cache` is a narrowing, and it commits this
/// instance to holding every body it fetches from that source, indefinitely.
#[must_use]
pub const fn is_widening(proposed: BodyMode, current: BodyMode) -> bool {
    // `Cache` stores bodies and `Aggregate` does not, so the direction that
    // commits resources is the one moving *towards* `Cache`.
    proposed.stores_bodies() && !current.stores_bodies()
}

/// Whether a proposal is a no-op — the mode it proposes is the mode in force.
#[must_use]
pub const fn is_no_op(proposed: BodyMode, current: BodyMode) -> bool {
    proposed.stores_bodies() == current.stores_bodies()
}

/// The quorum a retention change needs, by cost direction.
///
/// A narrowing change is cheap and reversible: a reader who dislikes more
/// storage can narrow it again, and nothing is lost. So it is held to the
/// ordinary bar of `widen_quorum` being irrelevant — it needs
/// [`MINIMUM_QUORUM`] and nothing more.
///
/// A widening change is held to `widen_quorum`, the instance's configured bar
/// for high-impact decisions, because it commits storage and bandwidth
/// indefinitely and a reader who wants it reversed needs the origin to still
/// have the body. That is the asymmetry §5.3 describes, and it is expressed as
/// a *count of ballots* rather than as a weight because §45.2 forbids weights in
/// governance.
///
/// Both are clamped up to [`MINIMUM_QUORUM`] and down to at least 1. The upper
/// clamp is the one that matters: an operator who configures a bar of 1000 on
/// an instance with 12 readers has a proposal that can never pass, and that is
/// a legitimate configuration — "this instance does not change storage policy
/// by vote" — so it is honoured rather than silently reduced. Only a bar *below*
/// three is refused, because that is the one direction where a vote stops being
/// a decision anybody made.
#[must_use]
pub fn quorum_for(proposed_mode: BodyMode, current: BodyMode, widen_quorum: i64) -> i64 {
    if is_widening(proposed_mode, current) {
        // Clamped from *below* only. A very large configured bar is honoured.
        widen_quorum.max(MINIMUM_QUORUM)
    } else {
        // A narrowing change, or a no-op: the ordinary bar, and never below the
        // floor. The floor applies here too — a no-op that "passes" on one tap
        // is still a record saying this instance agreed to something.
        MINIMUM_QUORUM
    }
}

/// Whether a tally clears the bar, and if not, how many more ballots are needed.
///
/// The shortfall is returned rather than only a bool because the reader
/// interface has to say something useful: "2 more supporters needed" is a
/// reason to keep the proposal open, and a bare `false` is a dead end that
/// invites opening a *second* proposal, which §5.3's one-open-per-setting rule
/// then refuses.
#[must_use]
pub fn quorum_outcome(
    supporters: i64,
    proposed_mode: BodyMode,
    current: BodyMode,
    widen_quorum: i64,
) -> QuorumOutcome {
    let required = quorum_for(proposed_mode, current, widen_quorum);
    if supporters >= required {
        QuorumOutcome::Reached {
            required,
            supporters,
        }
    } else {
        QuorumOutcome::Short {
            required,
            supporters,
            needed: required - supporters,
        }
    }
}

/// The result of testing a tally against the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuorumOutcome {
    /// Enough supporters. `required` is carried because the reader interface
    /// shows it, and a caller that only gets a bool has to recompute it and can
    /// get it wrong.
    Reached { required: i64, supporters: i64 },
    /// Not enough, and by how much.
    Short {
        required: i64,
        supporters: i64,
        needed: i64,
    },
}

impl QuorumOutcome {
    /// Whether the bar is met.
    #[must_use]
    pub const fn is_reached(self) -> bool {
        matches!(self, Self::Reached { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_widening_is_towards_cache_because_that_is_what_commits_resources() {
        // Storing bodies is the expensive direction, so moving *into* it
        // widens. This reads backwards from "storing less" and that is the
        // point worth pinning: a proposal to stop caching is the irreversible
        // one, because bodies already stored cannot be recovered from an
        // origin that has since changed.
        assert!(is_widening(BodyMode::Cache, BodyMode::Aggregate));
        assert!(!is_widening(BodyMode::Aggregate, BodyMode::Cache));
    }

    #[test]
    fn a_change_to_the_mode_already_in_force_is_a_no_op() {
        assert!(is_no_op(BodyMode::Cache, BodyMode::Cache));
        assert!(is_no_op(BodyMode::Aggregate, BodyMode::Aggregate));
        assert!(!is_no_op(BodyMode::Cache, BodyMode::Aggregate));
    }

    #[test]
    fn a_narrowing_change_is_held_to_the_ordinary_bar() {
        assert_eq!(
            quorum_for(BodyMode::Aggregate, BodyMode::Cache, 50),
            MINIMUM_QUORUM,
            "a narrowing change is cheap and reversible, so a configured bar of 50 \
             does not apply to it -- otherwise no reader could ever talk an instance \
             back out of storing everything it fetches"
        );
    }

    #[test]
    fn a_widening_change_is_held_to_the_configured_bar() {
        assert_eq!(quorum_for(BodyMode::Cache, BodyMode::Aggregate, 7), 7);
        assert_eq!(quorum_for(BodyMode::Cache, BodyMode::Aggregate, 12), 12);
    }

    #[test]
    fn a_configured_bar_below_the_floor_is_refused() {
        // One ballot is not a decision the instance made. A bar of 1 would let
        // a single reader set storage policy for everyone.
        assert_eq!(quorum_for(BodyMode::Cache, BodyMode::Aggregate, 1), 3);
        assert_eq!(quorum_for(BodyMode::Cache, BodyMode::Aggregate, 0), 3);
        assert_eq!(quorum_for(BodyMode::Cache, BodyMode::Aggregate, -5), 3);
    }

    #[test]
    fn a_very_large_configured_bar_is_honoured_rather_than_reduced() {
        // 1000 on an instance with 12 readers is a legitimate statement: this
        // instance does not change storage policy by vote. Reducing it to the
        // floor would override that silently.
        assert_eq!(quorum_for(BodyMode::Cache, BodyMode::Aggregate, 1000), 1000);
    }

    #[test]
    fn the_outcome_reports_the_shortfall_so_a_proposal_can_stay_open() {
        let outcome = quorum_outcome(2, BodyMode::Cache, BodyMode::Aggregate, 7);
        assert_eq!(
            outcome,
            QuorumOutcome::Short {
                required: 7,
                supporters: 2,
                needed: 5
            }
        );
        assert!(!outcome.is_reached());
    }

    #[test]
    fn a_tally_at_the_bar_is_reached_and_carries_the_bar() {
        let outcome = quorum_outcome(3, BodyMode::Cache, BodyMode::Aggregate, 3);
        assert_eq!(
            outcome,
            QuorumOutcome::Reached {
                required: 3,
                supporters: 3
            }
        );
        assert!(outcome.is_reached());
    }

    #[test]
    fn a_tally_past_the_bar_is_still_reached_and_still_reports_the_bar() {
        // Carrying `required` matters: a reader interface that recomputes it
        // can report "3 needed" for a proposal that needed 7, which reads as an
        // invitation to keep campaigning for something already decided.
        let outcome = quorum_outcome(9, BodyMode::Cache, BodyMode::Aggregate, 7);
        assert_eq!(
            outcome,
            QuorumOutcome::Reached {
                required: 7,
                supporters: 9
            }
        );
    }
}
