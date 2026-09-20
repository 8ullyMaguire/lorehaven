//! Typed votes, vote budgets, meta-moderation and karma (spec §35.2, repo M32).
//!
//! Slashdot-style typed votes replace the generic like on community surfaces.
//! Four rules live here, all of them policy rather than storage:
//!
//! 1. **Taxonomy.** A small fixed set of vote types per surface, configurable
//!    per category *as data*. A category that declares its own set replaces the
//!    instance default; a category that declares none uses the default.
//! 2. **Budget.** A rolling 24-hour allowance of votes, scaled by trust level,
//!    where a negative vote costs more than a positive one. Unused votes do not
//!    roll over, because a bankable allowance is a reason to vote for the sake
//!    of spending it.
//! 3. **Meta-moderation.** TL4+ stewards flag votes fair or unfair. A caster
//!    whose votes are consistently flagged unfair loses *weight*, never the
//!    ability to vote (spec §35.2: "moderate the moderators: weight, not voice,
//!    is the sanction").
//! 4. **Karma.** Received votes, weighted by the caster's weight at cast time,
//!    decaying while the receiver is inactive. Display only — it never gates
//!    trust, moderation, search ranking or credits (spec §0.3), and the
//!    workspace test in `milestone_32.rs` greps for exactly that.
//!
//! Weights and karma are integers in **basis points** (`WEIGHT_SCALE_BP` is one
//! unit of weight): ADR 0004 has the repository bind only `String` and `i64`,
//! and 0010 stores confidence the same way.

use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

use crate::governance::TL_STEWARD;

/// Basis points per unit of vote weight. 1000 bp = weight 1.0.
pub const WEIGHT_SCALE_BP: i64 = 1000;

/// The length of the vote-budget window, in hours.
pub const BUDGET_WINDOW_HOURS: i64 = 24;

/// Days per month for karma decay. A calendar month would make the decay
/// depend on which months passed, and 30 is what the spec's "5% per month"
/// reads as in a test.
pub const KARMA_MONTH_DAYS: i64 = 30;

/// Default monthly karma decay for an inactive receiver, in percent.
pub const KARMA_DECAY_PERCENT: i64 = 5;

/// One vote type on a surface (spec §35.2's `forum_vote_types`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoteType {
    /// Stable key, also what a cast stores.
    pub id: String,
    /// Human label, safe to display.
    pub label: String,
    /// The category this row belongs to; `None` is the instance default set.
    pub category_scope: Option<String>,
    /// Display order within the set.
    pub position: i64,
    /// The weight this type's votes carry, in basis points.
    pub weight_bp: i64,
    /// Budget this type costs to cast.
    pub cost: i64,
    /// Whether this type is negative (and therefore costs more).
    pub is_negative: bool,
}

impl VoteType {
    /// A row of the instance-default set.
    #[must_use]
    pub fn global(id: &str, label: &str, position: i64, cost: i64, is_negative: bool) -> Self {
        Self {
            id: id.to_owned(),
            label: label.to_owned(),
            category_scope: None,
            position,
            weight_bp: WEIGHT_SCALE_BP,
            cost,
            is_negative,
        }
    }
}

/// The default taxonomy (spec §35.2): `insightful | funny | interesting |
/// well-written | disagree`.
///
/// `well_written` is spelled with an underscore because M31's work-page
/// reaction bar already stores it that way; the types are shared data and the
/// bar's set is a subset of this one.
#[must_use]
pub fn default_taxonomy() -> Vec<VoteType> {
    vec![
        VoteType::global("insightful", "Insightful", 0, 1, false),
        VoteType::global("funny", "Funny", 1, 1, false),
        VoteType::global("interesting", "Interesting", 2, 1, false),
        VoteType::global("well_written", "Well written", 3, 1, false),
        VoteType::global("disagree", "Disagree", 4, 2, true),
    ]
}

/// The taxonomy a category's surface uses.
///
/// A category with rows of its own replaces the default set entirely — spec
/// §35.2's Critique example is a *different* set, not an addition to this one.
pub fn effective_taxonomy(all: &[VoteType], category_id: &str) -> Vec<VoteType> {
    let mut scoped: Vec<VoteType> = all
        .iter()
        .filter(|t| t.category_scope.as_deref() == Some(category_id))
        .cloned()
        .collect();
    if scoped.is_empty() {
        scoped = all
            .iter()
            .filter(|t| t.category_scope.is_none())
            .cloned()
            .collect();
    }
    scoped.sort_by_key(|t| (t.position, t.id.clone()));
    scoped
}

/// Look a vote type up in a resolved taxonomy.
#[must_use]
pub fn find_vote_type<'a>(types: &'a [VoteType], id: &str) -> Option<&'a VoteType> {
    types.iter().find(|t| t.id == id)
}

/// A `(minimum trust level, votes per window)` rung of the budget scale.
pub type BudgetRung = (i64, i64);

/// The budget for a trust level: the highest rung it reaches.
///
/// A level below every rung gets the smallest rung's allowance rather than
/// zero — a brand-new account may still vote; §0.3 forbids trust *buying*
/// standing, not the reverse.
#[must_use]
pub fn budget_limit_for(scale: &[BudgetRung], trust: i64) -> i64 {
    let mut limit = scale.first().map_or(0, |rung| rung.1);
    for (level, votes) in scale {
        if trust >= *level {
            limit = *votes;
        }
    }
    limit.max(0)
}

/// A voter's allowance and how much of it the rolling window has consumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct VoteBudget {
    /// Votes available in the window (already scaled by trust).
    pub limit: i64,
    /// Budget charged by votes still inside the window.
    pub spent: i64,
}

impl VoteBudget {
    /// Build the pair from storage.
    #[must_use]
    pub fn new(limit: i64, spent: i64) -> Self {
        Self { limit, spent }
    }

    /// What is left, never negative.
    #[must_use]
    pub fn remaining(&self) -> i64 {
        (self.limit - self.spent).max(0)
    }

    /// Whether the allowance is used up.
    #[must_use]
    pub fn exhausted(&self) -> bool {
        self.spent >= self.limit
    }

    /// Whether one more vote of `cost` fits.
    #[must_use]
    pub fn can_afford(&self, cost: i64) -> bool {
        self.spent + cost.max(0) <= self.limit
    }
}

/// The start of the rolling vote window that contains `now`.
#[must_use]
pub fn window_start(now: OffsetDateTime) -> OffsetDateTime {
    now - Duration::hours(BUDGET_WINDOW_HOURS)
}

/// When a charge from `charge_at` leaves the window.
#[must_use]
pub fn window_end(charge_at: OffsetDateTime) -> OffsetDateTime {
    charge_at + Duration::hours(BUDGET_WINDOW_HOURS)
}

/// What a cast did to storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VoteOutcome {
    /// No vote existed; one was recorded.
    Cast,
    /// An existing vote changed type.
    Changed,
    /// The vote is gone.
    Retracted,
}

/// A caster's vote weight from their meta-moderation record (spec §35.2).
///
/// Fewer than `min_verdicts` verdicts means full weight: one flag must not
/// decay anybody. Above that, weight is `1 - unfair / verdicts`, floored at
/// `min_weight_bp`. The floor is the point of the whole mechanism — the state
/// is lower *weight*, never fewer rights, and a caster who is all-vote and no
/// judgment still has a voice.
#[must_use]
pub fn vote_weight_bp(fair: i64, unfair: i64, min_weight_bp: i64, min_verdicts: i64) -> i64 {
    let fair = fair.max(0);
    let unfair = unfair.max(0);
    let verdicts = fair + unfair;
    let floor = min_weight_bp.clamp(0, WEIGHT_SCALE_BP);
    if verdicts == 0 || verdicts < min_verdicts {
        return WEIGHT_SCALE_BP;
    }
    let decayed = WEIGHT_SCALE_BP - (unfair * WEIGHT_SCALE_BP / verdicts);
    decayed.clamp(floor, WEIGHT_SCALE_BP)
}

/// Whether a trust level may meta-moderate (spec §35.2: TL4+).
#[must_use]
pub fn can_meta_moderate(trust: i64) -> bool {
    trust >= TL_STEWARD
}

/// Whether a trust level counts as a moderator for the transparency tiers.
///
/// "Moderators always see" (spec §35.2) means stewards here: the trust ladder
/// is the only moderator model this repository has (§19).
#[must_use]
pub fn is_moderator(trust: i64) -> bool {
    trust >= TL_STEWARD
}

/// Whether the viewer may see *who* voted, as opposed to the counts.
///
/// Aggregates are public; individual votes are anonymous unless the post author
/// has opted in (and then only to the author) or the viewer is a moderator.
/// The voter always sees their own vote through the separate `mine` field, so
/// that is not decided here.
#[must_use]
pub fn vote_transparency(
    viewer_is_author: bool,
    author_opted_in: bool,
    viewer_is_moderator: bool,
) -> VoteTransparency {
    if viewer_is_moderator || (viewer_is_author && author_opted_in) {
        VoteTransparency::IndividualVotes
    } else {
        VoteTransparency::AggregatesOnly
    }
}

/// How much of a post's vote record a viewer may see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VoteTransparency {
    /// Counts only; who voted stays anonymous.
    AggregatesOnly,
    /// The votes themselves, with their casters.
    IndividualVotes,
}

impl VoteTransparency {
    /// The wire form used in responses.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AggregatesOnly => "aggregates_only",
            Self::IndividualVotes => "individual_votes",
        }
    }
}

/// Whole months of inactivity between two moments, on a 30-day month.
#[must_use]
pub fn months_inactive(from: OffsetDateTime, now: OffsetDateTime) -> i64 {
    let elapsed = now - from;
    let months = elapsed.whole_days() / KARMA_MONTH_DAYS;
    months.max(0)
}

/// Karma after `months` of inactivity at `percent_per_month`.
///
/// Compounded in integer arithmetic and never negative, so two engines and two
/// runs agree on the number. A rate of 0 means no decay.
#[must_use]
pub fn decay_karma_bp(karma_bp: i64, months: i64, percent_per_month: i64) -> i64 {
    let percent = percent_per_month.clamp(0, 100);
    if percent == 0 || months <= 0 {
        return karma_bp.max(0);
    }
    let mut karma = karma_bp.max(0);
    for _ in 0..months {
        karma = karma * (100 - percent) / 100;
    }
    karma
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(
        id: &str,
        label: &str,
        scope: Option<&str>,
        position: i64,
        cost: i64,
        negative: bool,
    ) -> VoteType {
        VoteType {
            id: id.to_owned(),
            label: label.to_owned(),
            category_scope: scope.map(str::to_owned),
            position,
            weight_bp: WEIGHT_SCALE_BP,
            cost,
            is_negative: negative,
        }
    }

    #[test]
    fn the_default_taxonomy_matches_the_spec() {
        let ids: Vec<&str> = default_taxonomy().iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "insightful",
                "funny",
                "interesting",
                "well_written",
                "disagree"
            ]
        );
        let negative: Vec<&str> = default_taxonomy()
            .iter()
            .filter(|t| t.is_negative)
            .map(|t| t.id.as_str())
            .collect();
        assert_eq!(negative, vec!["disagree"], "one negative type");
    }

    #[test]
    fn a_category_with_its_own_rows_replaces_the_default_set() {
        let mut all = default_taxonomy();
        all.push(t("constructive", "Constructive", Some("critique"), 0, 1, false));
        all.push(t(
            "harsh_but_fair",
            "Harsh but fair",
            Some("critique"),
            1,
            1,
            false,
        ));
        all.push(t(
            "needs_sources",
            "Needs sources",
            Some("critique"),
            2,
            2,
            true,
        ));

        let general = effective_taxonomy(&all, "general");
        assert_eq!(general.len(), 5, "{general:?}");
        assert!(find_vote_type(&general, "insightful").is_some());

        let critique = effective_taxonomy(&all, "critique");
        let ids: Vec<&str> = critique.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["constructive", "harsh_but_fair", "needs_sources"]);
        assert!(
            find_vote_type(&critique, "insightful").is_none(),
            "a configured set replaces the default one, it does not add to it"
        );
    }

    #[test]
    fn the_budget_scale_climbs_with_trust_and_never_reaches_zero() {
        let scale = [(1, 10), (3, 30), (5, 60)];
        assert_eq!(budget_limit_for(&scale, 0), 10, "a new account still votes");
        assert_eq!(budget_limit_for(&scale, 1), 10);
        assert_eq!(budget_limit_for(&scale, 2), 10);
        assert_eq!(budget_limit_for(&scale, 3), 30);
        assert_eq!(budget_limit_for(&scale, 4), 30);
        assert_eq!(budget_limit_for(&scale, 5), 60);
        assert_eq!(budget_limit_for(&scale, 6), 60, "above the top rung holds");
    }

    #[test]
    fn a_negative_vote_costs_more_than_a_positive_one() {
        let types = default_taxonomy();
        let positive = find_vote_type(&types, "insightful").expect("positive type");
        let negative = find_vote_type(&types, "disagree").expect("negative type");
        assert!(negative.cost > positive.cost, "{negative:?} vs {positive:?}");
    }

    #[test]
    fn the_budget_reports_exhaustion_rather_than_going_negative() {
        let budget = VoteBudget::new(3, 2);
        assert_eq!(budget.remaining(), 1);
        assert!(budget.can_afford(1));
        assert!(!budget.can_afford(2), "a negative vote needs two votes' room");

        let spent = VoteBudget::new(3, 3);
        assert!(spent.exhausted());
        assert_eq!(spent.remaining(), 0);
        assert!(!spent.can_afford(1));

        // Unused votes do not roll over: the window is a function of `now`,
        // so nothing accumulates between windows.
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).expect("timestamp");
        assert_eq!(
            window_start(now).unix_timestamp(),
            now.unix_timestamp() - 86_400
        );
        assert_eq!(
            window_end(now).unix_timestamp(),
            now.unix_timestamp() + 86_400
        );
    }

    #[test]
    fn weight_decays_with_the_unfair_ratio_and_stops_at_the_floor() {
        let floor = 100;
        // One verdict is not a pattern.
        assert_eq!(vote_weight_bp(2, 1, floor, 3), WEIGHT_SCALE_BP);
        assert_eq!(vote_weight_bp(0, 2, floor, 3), WEIGHT_SCALE_BP);
        // All unfair: the floor, not zero.
        assert_eq!(vote_weight_bp(0, 3, floor, 3), floor);
        // Half unfair: half weight.
        assert_eq!(vote_weight_bp(3, 3, floor, 3), 500);
        // Mostly fair: mostly weight.
        assert_eq!(vote_weight_bp(9, 1, floor, 3), 900);
        // The floor never rises above full weight.
        assert_eq!(vote_weight_bp(0, 5, 5_000, 3), WEIGHT_SCALE_BP);
        // No verdicts at all: full weight.
        assert_eq!(vote_weight_bp(0, 0, floor, 3), WEIGHT_SCALE_BP);
    }

    #[test]
    fn only_stewards_may_meta_moderate() {
        assert!(!can_meta_moderate(3));
        assert!(can_meta_moderate(4));
        assert!(can_meta_moderate(6));
    }

    #[test]
    fn individual_votes_stay_hidden_unless_tiered_open() {
        assert_eq!(
            vote_transparency(false, true, false),
            VoteTransparency::AggregatesOnly,
            "an opted-in author does not open the record to strangers"
        );
        assert_eq!(
            vote_transparency(false, false, true),
            VoteTransparency::IndividualVotes,
            "moderators always see"
        );
        assert_eq!(
            vote_transparency(true, false, false),
            VoteTransparency::AggregatesOnly,
            "the author sees names only after opting in"
        );
        assert_eq!(
            vote_transparency(true, true, false),
            VoteTransparency::IndividualVotes
        );
        assert_eq!(VoteTransparency::AggregatesOnly.as_str(), "aggregates_only");
        assert_eq!(
            VoteTransparency::IndividualVotes.as_str(),
            "individual_votes"
        );
    }

    #[test]
    fn karma_decays_per_month_of_inactivity() {
        let start = OffsetDateTime::from_unix_timestamp(1_700_000_000).expect("timestamp");
        let two_months = start + Duration::days(60);

        assert_eq!(months_inactive(start, start), 0);
        assert_eq!(months_inactive(start, start + Duration::days(29)), 0);
        assert_eq!(months_inactive(start, two_months), 2);

        assert_eq!(decay_karma_bp(10_000, 0, 5), 10_000, "no time, no decay");
        assert_eq!(decay_karma_bp(10_000, 1, 5), 9_500);
        assert_eq!(decay_karma_bp(10_000, 2, 5), 9_025);
        assert_eq!(decay_karma_bp(10_000, 12, 5), 5_404);
        assert_eq!(decay_karma_bp(10_000, 3, 0), 10_000, "zero rate, no decay");
        assert_eq!(decay_karma_bp(-5, 2, 5), 0, "karma is never negative");
    }
}
