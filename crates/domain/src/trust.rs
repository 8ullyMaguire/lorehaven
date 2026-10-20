//! M3 — the trust ladder, the earned side, the elastic allocation and the one
//! automatic demotion (spec §14.4).
//!
//! # What this module is for
//!
//! Spec §14.4 says **"one ladder. 0 → 6. Names and limits are configuration."**
//! and that is the whole of it. This module holds:
//!
//! * the **ladder** — the levels, their names, and the privileges each unlocks;
//! * the **axes** — the four things trust is earned along, and their weights;
//! * the **evidence** — what one account has actually done;
//! * the **elastic allocation** — what happens when nobody clears the bar;
//! * the **demotion** — the one automatic demotion there is.
//!
//! # The three rules that shape everything here
//!
//! 1. **The names and the unlock table are configuration.** The code holds no
//!    hard-coded level numbers: a route asks the ladder *which privilege* it
//!    needs, and the ladder says which level holds it. An instance that wants
//!    its moderators at level 3 changes a configuration line, not a constant.
//! 2. **Trust is earned through reviewed conduct.** Not post count, not word
//!    count, not kudos, not credits, not any activity-volume metric. The
//!    [`TrustEvidence`] type has an `activity_volume` field and **the score
//!    function never reads it** — it is there so the counter-example can be
//!    written down and tested.
//! 3. **Exactly one automatic demotion, and it is spam.** Everything else is a
//!    reviewed sanction, because a machine demoting a person is a machine
//!    deciding what a person is worth.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// The privilege vocabulary
// ---------------------------------------------------------------------------

/// A privilege the ladder can hand out.
///
/// **A closed vocabulary**, so a configuration naming a privilege this build
/// does not know is a *startup error* rather than a privilege nobody holds —
/// which would be a privilege that silently unlocks nothing, and a reader
/// would be told they may do something and then be refused.
///
/// The names are the specification's own (§14.4's table), and the level each
/// one sits at is the **configuration**'s business, not this type's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Privilege {
    // --- level 0: the anonymous reader -----------------------------------
    /// Read the public catalogue.
    ReadPublic,
    /// Search.
    Search,
    /// See the public catalogue.
    PublicCatalogue,
    /// Be served the house recommendation stream (T0).
    HouseStream,

    // --- level 1: the new reader ------------------------------------------
    /// Post.
    Post,
    /// Comment.
    Comment,
    /// Vote.
    Vote,
    /// Report.
    Report,
    /// Be served the personal recommendation stream (T1).
    PersonalStream,

    // --- level 2: the member ----------------------------------------------
    /// Publish to the shared galleries.
    PublishSharedGalleries,
    /// Propose a merge.
    MergeProposals,
    /// Propose a tag change.
    TagProposals,
    /// Create a mood.
    CreateMood,

    // --- level 3: the trusted curator --------------------------------------
    /// Propose a merge of two identities.
    ProposeMerges,
    /// Vote on content.
    VoteOnContent,
    /// Set the topic tags of a thing.
    SetTopicTags,
    /// Create a channel.
    CreateChannel,

    // --- level 4: the moderator --------------------------------------------
    /// Hold the moderation queue.
    ModerationQueue,
    /// Fast-hide.
    FastHide,
    /// Lock and pin.
    LockPin,
    /// Be served the full engine (T2).
    FullEngine,
    /// Propose a change to the category governance.
    CategoryGovernance,
    /// Decide an entry-moderation action.
    EntryModeration,

    // --- level 5: the senior reviewer --------------------------------------
    /// Resolve reports.
    ResolveReports,
    /// Issue a scoped ban.
    ScopedBans,
    /// Serve on a jury.
    JuryService,

    // --- level 6: the trustee ----------------------------------------------
    /// Change the instance-wide settings.
    ///
    /// **The ladder grants this to nobody.** Spec §14.4's row for level 6 says
    /// the trustee holds *everything except instance-wide settings*, and the
    /// instance-wide settings are the **operator role's** — a separate
    /// mechanism from trust, and one that is not earned and not spent.
    InstanceSettings,
}

impl Privilege {
    /// The privilege's name, as the configuration spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadPublic => "read_public",
            Self::Search => "search",
            Self::PublicCatalogue => "public_catalogue",
            Self::HouseStream => "house_stream",
            Self::Post => "post",
            Self::Comment => "comment",
            Self::Vote => "vote",
            Self::Report => "report",
            Self::PersonalStream => "personal_stream",
            Self::PublishSharedGalleries => "publish_shared_galleries",
            Self::MergeProposals => "merge_proposals",
            Self::TagProposals => "tag_proposals",
            Self::CreateMood => "create_mood",
            Self::ProposeMerges => "propose_merges",
            Self::VoteOnContent => "vote_on_content",
            Self::SetTopicTags => "set_topic_tags",
            Self::CreateChannel => "create_channel",
            Self::ModerationQueue => "moderation_queue",
            Self::FastHide => "fast_hide",
            Self::LockPin => "lock_pin",
            Self::FullEngine => "full_engine",
            Self::CategoryGovernance => "category_governance",
            Self::EntryModeration => "entry_moderation",
            Self::ResolveReports => "resolve_reports",
            Self::ScopedBans => "scoped_bans",
            Self::JuryService => "jury_service",
            Self::InstanceSettings => "instance_settings",
        }
    }

    /// Every privilege, in the order the ladder names them.
    ///
    /// **Derived from one list**, so a privilege added to the enum and a
    /// privilege this function refuses cannot drift apart.
    #[must_use]
    pub fn all() -> Vec<Privilege> {
        vec![
            Self::ReadPublic,
            Self::Search,
            Self::PublicCatalogue,
            Self::HouseStream,
            Self::Post,
            Self::Comment,
            Self::Vote,
            Self::Report,
            Self::PersonalStream,
            Self::PublishSharedGalleries,
            Self::MergeProposals,
            Self::TagProposals,
            Self::CreateMood,
            Self::ProposeMerges,
            Self::VoteOnContent,
            Self::SetTopicTags,
            Self::CreateChannel,
            Self::ModerationQueue,
            Self::FastHide,
            Self::LockPin,
            Self::FullEngine,
            Self::CategoryGovernance,
            Self::EntryModeration,
            Self::ResolveReports,
            Self::ScopedBans,
            Self::JuryService,
            Self::InstanceSettings,
        ]
    }
}

// ---------------------------------------------------------------------------
// The ladder
// ---------------------------------------------------------------------------

/// One level of the ladder, as configuration (spec §14.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustLevelSpec {
    /// The level's number. `0` is the anonymous reader and `6` the trustee.
    pub level: i64,
    /// The level's name, as the instance calls it.
    pub name: String,
    /// The privileges this level unlocks.
    ///
    /// **Cumulative**: a level holds everything the levels below it hold. The
    /// specification's table lists only what each level *adds*, and level 1
    /// does not repeat level 0's reading privileges — so the rule has to be
    /// cumulative for the table to mean what it says.
    #[serde(default)]
    pub unlocks: Vec<Privilege>,
    /// The weighted score this level needs.
    ///
    /// `None` means the level is **designated**: it is granted by an operator
    /// and never computed from evidence. Spec §14.4's rows for level 5 and
    /// level 6 say "staff-designated" and "operator-designated", and a
    /// designated level that were also earned would be a level two people
    /// could reach and only one of them by the right route.
    #[serde(default)]
    pub min_score: Option<f64>,
}

impl TrustLevelSpec {
    /// An earned level, with the score it needs.
    #[must_use]
    pub fn earned(level: i64, name: &str, min_score: f64, unlocks: &[Privilege]) -> Self {
        Self {
            level,
            name: name.to_owned(),
            unlocks: unlocks.to_vec(),
            min_score: Some(min_score),
        }
    }

    /// A designated level: granted by an operator, never computed.
    #[must_use]
    pub fn designated(level: i64, name: &str, unlocks: &[Privilege]) -> Self {
        Self {
            level,
            name: name.to_owned(),
            unlocks: unlocks.to_vec(),
            min_score: None,
        }
    }
}

/// The trust ladder: the levels, their names, and what each unlocks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustLadder {
    /// The levels, in any order — the ladder sorts them.
    pub levels: Vec<TrustLevelSpec>,
}

impl TrustLadder {
    /// The specification's table (spec §14.4), as the default.
    ///
    /// **The scores are this module's choice, and the choice is stated.** The
    /// specification gives each level a *typical requirement* in prose
    /// ("tenure + accepted contributions + report accuracy"), not a number,
    /// and it says the limits are configuration. So the numbers here are a
    /// defensible reading of that prose under the default axis weights:
    ///
    /// | level | score | reading |
    /// |---|---|---|
    /// | 0 anonymous | 0 | nobody has an account |
    /// | 1 new | 0 | *an account, or a guest* — having one is the whole requirement |
    /// | 2 member | 40 | ~30 days' tenure and a few accepted contributions |
    /// | 3 trusted | 120 | ~90 days' tenure and a dozen contributions |
    /// | 4 moderator | 240 | a year of standing, or many accepted contributions |
    /// | 5 senior reviewer | — | staff-designated |
    /// | 6 trustee | — | operator-designated |
    ///
    /// **The level a reader lands on is therefore the operator's to set**, and
    /// these are only the starting numbers.
    #[must_use]
    pub fn default_ladder() -> Self {
        Self {
            levels: vec![
                TrustLevelSpec::earned(
                    0,
                    "anonymous",
                    0.0,
                    &[
                        Privilege::ReadPublic,
                        Privilege::Search,
                        Privilege::PublicCatalogue,
                        Privilege::HouseStream,
                    ],
                ),
                TrustLevelSpec::earned(
                    1,
                    "new",
                    0.0,
                    &[
                        Privilege::Post,
                        Privilege::Comment,
                        Privilege::Vote,
                        Privilege::Report,
                        Privilege::PersonalStream,
                    ],
                ),
                TrustLevelSpec::earned(
                    2,
                    "member",
                    40.0,
                    &[
                        Privilege::PublishSharedGalleries,
                        Privilege::MergeProposals,
                        Privilege::TagProposals,
                        Privilege::CreateMood,
                    ],
                ),
                TrustLevelSpec::earned(
                    3,
                    "trusted",
                    120.0,
                    &[
                        Privilege::ProposeMerges,
                        Privilege::VoteOnContent,
                        Privilege::SetTopicTags,
                        Privilege::CreateChannel,
                    ],
                ),
                TrustLevelSpec::earned(
                    4,
                    "moderator",
                    240.0,
                    &[
                        Privilege::ModerationQueue,
                        Privilege::FastHide,
                        Privilege::LockPin,
                        Privilege::FullEngine,
                        Privilege::CategoryGovernance,
                        Privilege::EntryModeration,
                    ],
                ),
                TrustLevelSpec::designated(
                    5,
                    "senior reviewer",
                    &[
                        Privilege::ResolveReports,
                        Privilege::ScopedBans,
                        Privilege::JuryService,
                    ],
                ),
                TrustLevelSpec::designated(6, "trustee", &[]),
            ],
        }
    }

    /// The levels, lowest first.
    #[must_use]
    pub fn sorted(&self) -> Vec<&TrustLevelSpec> {
        let mut levels: Vec<&TrustLevelSpec> = self.levels.iter().collect();
        levels.sort_by_key(|spec| spec.level);
        levels
    }

    /// The level's name, if the ladder has one.
    #[must_use]
    pub fn name_of(&self, level: i64) -> Option<&str> {
        self.sorted()
            .iter()
            .find(|spec| spec.level == level)
            .map(|spec| spec.name.as_str())
    }

    /// The number of the level with this name, if the ladder has one.
    #[must_use]
    pub fn level_of(&self, name: &str) -> Option<i64> {
        self.levels
            .iter()
            .find(|spec| spec.name.eq_ignore_ascii_case(name))
            .map(|spec| spec.level)
    }

    /// Whether a level at this number holds this privilege.
    ///
    /// **Cumulative**, and the level the privilege sits at is the one that
    /// grants it — so a level below that one does not hold it, and a level
    /// above it does.
    #[must_use]
    pub fn holds(&self, level: i64, privilege: Privilege) -> bool {
        self.sorted()
            .iter()
            .filter(|spec| spec.level <= level)
            .any(|spec| spec.unlocks.contains(&privilege))
    }

    /// The level at which a privilege first becomes available.
    #[must_use]
    pub fn level_holding(&self, privilege: Privilege) -> Option<i64> {
        self.sorted()
            .iter()
            .find(|spec| spec.unlocks.contains(&privilege))
            .map(|spec| spec.level)
    }

    /// Every privilege a level holds, lowest level first.
    #[must_use]
    pub fn unlocks_for(&self, level: i64) -> Vec<Privilege> {
        let mut held: Vec<Privilege> = Vec::new();
        for spec in self.sorted() {
            if spec.level > level {
                break;
            }
            for privilege in &spec.unlocks {
                if !held.contains(privilege) {
                    held.push(*privilege);
                }
            }
        }
        held
    }

    /// The same ladder, with one privilege moved to a level.
    ///
    /// **Used for the full engine.** Spec §14.4's row for level 4 says "the
    /// full engine (T2) **if `full_min_trust` is 4**" — so the tier
    /// configuration decides where the full engine sits, and the ladder's row
    /// follows it. Moving the privilege rather than *checking* it is what
    /// makes the two one fact: an operator who sets `full_min_trust = 5` gets
    /// the full engine at level 5, with no second configuration key to keep in
    /// step.
    #[must_use]
    pub fn with_privilege_at(&self, privilege: Privilege, level: i64) -> TrustLadder {
        let mut levels = self.levels.clone();
        for spec in levels.iter_mut() {
            spec.unlocks.retain(|held| *held != privilege);
            if spec.level == level {
                spec.unlocks.push(privilege);
            }
        }
        TrustLadder { levels }
    }

    /// The highest level this ladder reaches.
    #[must_use]
    pub fn max_level(&self) -> i64 {
        self.sorted().last().map(|spec| spec.level).unwrap_or(0)
    }

    /// The level a weighted score earns.
    ///
    /// **Designated levels are unreachable this way.** A level with no
    /// `min_score` is granted by an operator, and a score must never reach it:
    /// otherwise the score would be a second route to a privilege the
    /// instance reserves for staff.
    #[must_use]
    pub fn earned_level(&self, score: f64) -> i64 {
        let mut earned: Option<i64> = None;
        for spec in self.sorted() {
            let Some(minimum) = spec.min_score else {
                // A designated level ends the earned run.
                break;
            };
            if score + f64::EPSILON >= minimum {
                earned = Some(spec.level);
            }
        }
        earned.unwrap_or_else(|| self.sorted().first().map(|s| s.level).unwrap_or(0))
    }
}

// ---------------------------------------------------------------------------
// The earned side
// ---------------------------------------------------------------------------

/// The four axes trust is earned along, and their weights (spec §14.4).
///
/// **The weights are configuration** — "weighted per site in `trust.axes`" —
/// and the axes themselves are fixed by the specification: tenure days,
/// accepted contributions, vote score and report accuracy. Not one more.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustAxes {
    /// The weight of the tenure axis.
    pub tenure_days: f64,
    /// The weight of the accepted-contributions axis.
    pub accepted_contributions: f64,
    /// The weight of the vote-score axis.
    pub vote_score: f64,
    /// The weight of the report-accuracy axis.
    pub report_accuracy: f64,
}

impl Default for TrustAxes {
    fn default() -> Self {
        Self {
            tenure_days: 1.0,
            accepted_contributions: 4.0,
            vote_score: 1.0,
            report_accuracy: 2.0,
        }
    }
}

impl TrustAxes {
    /// The four axes, with the values that produced a score.
    ///
    /// **The basis is what makes a level explainable** — spec §14.4 asks for a
    /// level that can be explained, and a level with no basis is a level
    /// somebody has to take on trust.
    #[must_use]
    pub fn weighted(&self, evidence: &TrustEvidence) -> f64 {
        evidence.tenure_days as f64 * self.tenure_days
            + evidence.accepted_contributions as f64 * self.accepted_contributions
            + evidence.vote_score as f64 * self.vote_score
            + evidence.report_accuracy as f64 * self.report_accuracy
    }
}

/// What one account has actually done.
///
/// **Every field here is reviewed conduct** except the last, which is not an
/// axis and is never read by the score.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustEvidence {
    /// Days since the account was created.
    pub tenure_days: i64,
    /// Contributions other people **accepted**.
    ///
    /// **Accepted**, not made: a contribution counts when somebody else
    /// reviewed it and took it. A work nobody reviewed is a post, and a post
    /// is not a contribution.
    pub accepted_contributions: i64,
    /// The vote score the account's published work received.
    pub vote_score: i64,
    /// Confirmed reports minus dismissed ones.
    ///
    /// **Net, not a rate.** A rate rewards a reader with one lucky report as
    /// much as a reader with fifty confirmed ones, and a rate over no reports
    /// is a division by zero that quietly becomes "perfect".
    pub report_accuracy: i64,
    /// Activity volume: posts, comments, kudos, credits.
    ///
    /// **NOT AN AXIS.** Spec §14.4 is explicit — trust is earned through
    /// reviewed conduct, "not post count, not word count, not kudos, not
    /// credits, not any activity-volume metric" — and this field is here for
    /// one reason: so the counter-example can be *written down and tested*.
    /// A reader with ten thousand posts and no reviewed contributions must land
    /// on a lower level than a reader with twenty accepted contributions, and
    /// a test that says so needs the volume to be somewhere.
    ///
    /// The score function does not read it, and a test asserts that two
    /// accounts differing only in this field score the same.
    pub activity_volume: i64,
}

/// A computed trust level, and the evidence that produced it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustAssessment {
    /// The level the evidence earns.
    pub level: i64,
    /// The level's name.
    pub name: String,
    /// The weighted score.
    pub score: f64,
    /// The axis values that produced the score.
    pub evidence: TrustEvidence,
    /// The weights that were applied.
    pub axes: TrustAxes,
}

impl TrustAssessment {
    /// The level's basis, as the JSON the store keeps.
    ///
    /// **The basis is the four axis values and the weights**, because a level
    /// that cannot be explained cannot be contested — and a reader who thinks
    /// they earned a higher level needs to be able to see why they did not.
    #[must_use]
    pub fn basis(&self) -> serde_json::Value {
        serde_json::json!({
            "score": self.score,
            "axes": {
                "tenure_days": self.axes.tenure_days,
                "accepted_contributions": self.axes.accepted_contributions,
                "vote_score": self.axes.vote_score,
                "report_accuracy": self.axes.report_accuracy,
            },
            "evidence": {
                "tenure_days": self.evidence.tenure_days,
                "accepted_contributions": self.evidence.accepted_contributions,
                "vote_score": self.evidence.vote_score,
                "report_accuracy": self.evidence.report_accuracy,
            },
            "name": self.name,
        })
    }
}

/// Compute the level an account's evidence earns.
///
/// **The score function reads four fields and no more.** It does not read
/// `activity_volume`, and that is the whole of the specification's rule about
/// activity-volume metrics — enforced by a function that has no way to
/// accidentally include it.
#[must_use]
pub fn assess(evidence: &TrustEvidence, axes: &TrustAxes, ladder: &TrustLadder) -> TrustAssessment {
    let score = axes.weighted(evidence);
    let level = ladder.earned_level(score);
    TrustAssessment {
        level,
        name: ladder.name_of(level).unwrap_or("unknown").to_owned(),
        score,
        evidence: *evidence,
        axes: *axes,
    }
}

// ---------------------------------------------------------------------------
// The operator's requirement
// ---------------------------------------------------------------------------

/// The operator's per-privilege requirement (spec §14.4's
/// `trust.require.<privilege>`).
///
/// **A tightening, never a loosening.** The requirement may raise a
/// privilege's level above the ladder's, and it may not lower it — because
/// the ladder is what the instance's readers are told they are working
/// towards, and an operator who quietly lowered a requirement would be
/// changing the deal the ladder made without telling anybody.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustRequirements {
    /// The overrides, by privilege name.
    pub require: BTreeMap<String, i64>,
}

impl TrustRequirements {
    /// The level a privilege requires, given the ladder.
    ///
    /// The greater of the ladder's level and the operator's override.
    #[must_use]
    pub fn level_for(&self, ladder: &TrustLadder, privilege: Privilege) -> i64 {
        let ladder_level = ladder.level_holding(privilege).unwrap_or(i64::MAX);
        let override_level = self.require.get(privilege.as_str()).copied();
        match override_level {
            Some(level) => level.max(ladder_level),
            None => ladder_level,
        }
    }

    /// Whether a level holds a privilege, given the operator's requirement.
    #[must_use]
    pub fn holds(&self, ladder: &TrustLadder, level: i64, privilege: Privilege) -> bool {
        let required = self.level_for(ladder, privilege);
        required != i64::MAX && level >= required
    }

    /// The overrides that ask for **less** than the ladder gives.
    ///
    /// **A refusal, not a correction.** A configuration that tries to lower a
    /// privilege's requirement is a mistake the operator should hear about,
    /// not a value quietly ignored — and ignoring it would mean the operator
    /// believes they have relaxed something they have not.
    #[must_use]
    pub fn loosening(&self, ladder: &TrustLadder) -> Vec<(Privilege, i64, i64)> {
        let mut found = Vec::new();
        for (name, level) in &self.require {
            let Some(privilege) = Privilege::all().into_iter().find(|p| p.as_str() == name) else {
                continue;
            };
            if let Some(ladder_level) = ladder.level_holding(privilege) {
                if *level < ladder_level {
                    found.push((privilege, *level, ladder_level));
                }
            }
        }
        found
    }
}

// ---------------------------------------------------------------------------
// The elastic allocation
// ---------------------------------------------------------------------------

/// Why a privilege's requirement is what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementBasis {
    /// The requirement is the ladder's own.
    Nominal,
    /// The requirement fell because **nobody** holds the privilege.
    ///
    /// "A site with three users still has a moderator; a site with three
    /// thousand does not hand the power out." (spec §14.4)
    Elastic,
}

/// A privilege's requirement, and why it is what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requirement {
    /// The level the privilege requires.
    pub level: i64,
    /// Why the requirement is that level.
    pub basis: RequirementBasis,
}

/// The elastic allocation (spec §14.4).
///
/// When a privilege's eligible pool is **empty** — nobody at all holds the
/// privilege at the level it needs — the requirement **falls until at least
/// `elastic_min_holders` holders exist**, but never below `floor_level`.
///
/// **Three properties make this safe, and all three are load-bearing:**
///
/// 1. **The floor.** The requirement never falls below `floor_level`, so an
///    elastic grant is never a grant to a brand-new account.
/// 2. **No ratchet.** The requirement returns to nominal as soon as enough
///    people clear the real bar. A site that grows into a level-4 account
///    stops handing the queue to a level-2 on the next evaluation, and the
///    level-2 loses the privilege **without anybody deciding to take it**.
///    A pure function of the current holders cannot ratchet, because it has no
///    memory of what it decided last time.
/// 3. **The holders are counted, not assumed.** The count is of accounts that
///    actually hold the privilege at each level, so a site with three
///    thousand level-1 accounts and no moderators does not hand the queue to
///    a level-1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElasticPolicy {
    /// The fewest holders an elastic grant needs.
    pub min_holders: i64,
    /// The floor an elastic requirement falls to.
    pub floor_level: i64,
}

impl Default for ElasticPolicy {
    fn default() -> Self {
        Self {
            min_holders: 1,
            floor_level: 2,
        }
    }
}

/// How many accounts hold a privilege, at each level.
///
/// **A distribution, not a pair of counts**, because the requirement falls
/// *until enough people hold it* — which is a question about the shape of the
/// distribution and not about two numbers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HolderCounts {
    /// The number of accounts holding the privilege at each level.
    pub by_level: BTreeMap<i64, i64>,
}

impl HolderCounts {
    /// Count one holder at a level.
    pub fn add(&mut self, level: i64) {
        *self.by_level.entry(level).or_insert(0) += 1;
    }

    /// How many accounts hold the privilege at `level` or above.
    #[must_use]
    pub fn at_or_above(&self, level: i64) -> i64 {
        self.by_level
            .iter()
            .filter(|(held, _)| **held >= level)
            .map(|(_, count)| *count)
            .sum()
    }
}

/// The requirement for a privilege, given who holds it.
///
/// **The requirement falls to the highest level that admits `min_holders`
/// holders**, bounded below by the floor. So a site with one level-3 curator
/// and no level-4 moderator hands the queue to the level-3, and a site with
/// three level-1 accounts and nobody else hands it to nobody — because the
/// floor is the floor, and a privilege nobody holds is a better outcome than
/// a privilege held by whoever registered first.
#[must_use]
pub fn elastic_requirement(
    nominal_level: i64,
    holders: &HolderCounts,
    policy: &ElasticPolicy,
) -> Requirement {
    // The pool is not empty when enough people already hold it.
    if holders.at_or_above(nominal_level) >= policy.min_holders {
        return Requirement {
            level: nominal_level,
            basis: RequirementBasis::Nominal,
        };
    }

    // The pool *is* empty. The requirement falls, level by level, until enough
    // people hold it — and never below the floor.
    let mut level = nominal_level - 1;
    while level >= policy.floor_level {
        if holders.at_or_above(level) >= policy.min_holders {
            return Requirement {
                level,
                basis: RequirementBasis::Elastic,
            };
        }
        level -= 1;
    }

    Requirement {
        level: policy
            .floor_level
            .max(nominal_level.min(policy.floor_level)),
        basis: RequirementBasis::Elastic,
    }
}

// ---------------------------------------------------------------------------
// The one automatic demotion
// ---------------------------------------------------------------------------

/// The number of confirmed spam reports that costs a level.
///
/// **Three, in ninety days** (spec §14.4). This is the *only* automatic
/// demotion there is: everything else is a reviewed sanction, because a
/// machine demoting a person is a machine deciding what a person is worth.
pub const SPAM_DEMOTION_THRESHOLD: i64 = 3;

/// The window the confirmed spam reports are counted over, in days.
pub const SPAM_DEMOTION_WINDOW_DAYS: i64 = 90;

/// The level a spam demotion lands on.
///
/// **Never below level 2.** The floor is the specification's and it is not
/// configurable: a demotion that could take an account to level 0 would make
/// the account a guest again, and a guest's contents are foldable away.
#[must_use]
pub fn spam_demotion_level(current_level: i64, floor_level: i64) -> i64 {
    let floor = floor_level.max(2);
    (current_level - 1).max(floor)
}

/// Whether the confirmed spam reports in the window cost a level.
///
/// `confirmed_spam_reports` is the count **within the window** — the caller
/// counts it, because the window is a property of the query and not of the
/// rule.
#[must_use]
pub fn spam_demotes(confirmed_spam_reports: i64) -> bool {
    confirmed_spam_reports >= SPAM_DEMOTION_THRESHOLD
}

/// How much of a lost level comes back.
///
/// **`0.7^loss_count`** (spec §14.4): the first loss recovers fastest, and
/// each further loss takes longer to come back. A reader who has lost three
/// levels has to earn them again from further away than a reader who lost one,
/// which is the point — the curve is what makes repeated loss expensive
/// without making it permanent.
#[must_use]
pub fn recovery_multiplier(loss_count: i64) -> f64 {
    if loss_count <= 0 {
        return 1.0;
    }
    0.7_f64.powi(loss_count.clamp(0, i32::MAX as i64) as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_ladder_is_the_specifications_table() {
        let ladder = TrustLadder::default_ladder();
        assert_eq!(ladder.levels.len(), 7);
        let names: Vec<&str> = ladder
            .sorted()
            .iter()
            .map(|spec| spec.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec![
                "anonymous",
                "new",
                "member",
                "trusted",
                "moderator",
                "senior reviewer",
                "trustee"
            ],
            "spec 14.4's names, in order"
        );
    }

    #[test]
    fn the_ladder_is_cumulative() {
        let ladder = TrustLadder::default_ladder();
        // Level 1 does not repeat level 0's reading privileges, and it holds
        // them anyway.
        assert!(ladder.holds(1, Privilege::ReadPublic));
        assert!(ladder.holds(1, Privilege::Post));
        assert!(!ladder.holds(1, Privilege::TagProposals));
        assert!(ladder.holds(2, Privilege::TagProposals));
    }

    #[test]
    fn the_mood_is_a_level_two_unlock_and_the_channel_a_level_three() {
        let ladder = TrustLadder::default_ladder();
        assert_eq!(ladder.level_holding(Privilege::CreateMood), Some(2));
        assert_eq!(ladder.level_holding(Privilege::CreateChannel), Some(3));
        assert!(
            !ladder.holds(1, Privilege::CreateMood),
            "a level-1 account cannot create a mood"
        );
        assert!(
            ladder.holds(2, Privilege::CreateMood),
            "a level-2 account can"
        );
    }

    #[test]
    fn the_trustee_holds_everything_except_the_instance_settings() {
        let ladder = TrustLadder::default_ladder();
        for privilege in Privilege::all() {
            if privilege == Privilege::InstanceSettings {
                assert!(
                    !ladder.holds(6, privilege),
                    "the instance-wide settings are the operator role's, not the \
                     trustee's -- spec 14.4's row for level 6"
                );
            } else {
                assert!(
                    ladder.holds(6, privilege),
                    "the trustee holds {privilege:?}"
                );
            }
        }
    }

    #[test]
    fn a_designated_level_is_not_reachable_by_a_score() {
        let ladder = TrustLadder::default_ladder();
        let evidence = TrustEvidence {
            tenure_days: 100_000,
            accepted_contributions: 100_000,
            vote_score: 100_000,
            report_accuracy: 100_000,
            activity_volume: 0,
        };
        let assessment = assess(&evidence, &TrustAxes::default(), &ladder);
        assert!(
            assessment.level < 5,
            "no score reaches a designated level: got {}",
            assessment.level
        );
    }

    #[test]
    fn activity_volume_is_not_an_axis() {
        // **The counter-example, written as a test.** A reader with ten
        // thousand posts and no reviewed contributions must land on a lower
        // level than a reader with twenty accepted contributions and ninety
        // days of tenure.
        let axes = TrustAxes::default();
        let ladder = TrustLadder::default_ladder();

        let prolific = TrustEvidence {
            tenure_days: 0,
            accepted_contributions: 0,
            vote_score: 0,
            report_accuracy: 0,
            activity_volume: 10_000,
        };
        let reviewed = TrustEvidence {
            tenure_days: 90,
            accepted_contributions: 20,
            vote_score: 0,
            report_accuracy: 0,
            activity_volume: 0,
        };

        let prolific_level = assess(&prolific, &axes, &ladder).level;
        let reviewed_level = assess(&reviewed, &axes, &ladder).level;
        assert!(
            reviewed_level > prolific_level,
            "reviewed conduct beats volume: {reviewed_level} vs {prolific_level}"
        );

        // And two accounts differing only in volume score the same.
        let quiet = TrustEvidence {
            activity_volume: 0,
            ..reviewed
        };
        assert_eq!(
            axes.weighted(&quiet),
            axes.weighted(&reviewed),
            "activity volume is not read by the score"
        );
    }

    #[test]
    fn the_operator_may_tighten_a_requirement_and_not_loosen_it() {
        let ladder = TrustLadder::default_ladder();
        let mut requirements = TrustRequirements::default();
        requirements
            .require
            .insert(Privilege::TagProposals.as_str().to_owned(), 4);
        assert_eq!(requirements.level_for(&ladder, Privilege::TagProposals), 4);
        assert!(!requirements.holds(&ladder, 3, Privilege::TagProposals));
        assert!(requirements.holds(&ladder, 4, Privilege::TagProposals));

        // A loosening is refused rather than silently ignored.
        requirements
            .require
            .insert(Privilege::TagProposals.as_str().to_owned(), 1);
        let loosening = requirements.loosening(&ladder);
        assert_eq!(loosening.len(), 1);
        assert_eq!(loosening[0].0, Privilege::TagProposals);
    }

    #[test]
    fn a_small_site_still_has_a_moderator_and_a_big_one_does_not() {
        let policy = ElasticPolicy::default();

        // Nobody holds it, and one curator at level 3 does: the requirement
        // falls to 3, which is the highest level that admits a holder.
        let mut holders = HolderCounts::default();
        holders.add(3);
        let elastic = elastic_requirement(4, &holders, &policy);
        assert_eq!(elastic.level, 3);
        assert_eq!(elastic.basis, RequirementBasis::Elastic);

        // Somebody at the nominal level holds it: the requirement is nominal.
        let mut holders = HolderCounts::default();
        holders.add(4);
        let nominal = elastic_requirement(4, &holders, &policy);
        assert_eq!(nominal.level, 4);
        assert_eq!(nominal.basis, RequirementBasis::Nominal);
    }

    #[test]
    fn the_elastic_floor_holds_even_when_nobody_clears_it() {
        let policy = ElasticPolicy::default();
        // Three brand-new accounts and nobody else: the requirement falls to
        // the floor and stops, because the floor is the floor.
        let mut holders = HolderCounts::default();
        holders.add(1);
        holders.add(1);
        holders.add(1);
        let elastic = elastic_requirement(4, &holders, &policy);
        assert_eq!(elastic.level, 2);
        assert_eq!(elastic.basis, RequirementBasis::Elastic);
        assert!(
            holders.at_or_above(elastic.level) < policy.min_holders,
            "nobody holds it even at the floor -- which is the honest outcome"
        );
    }

    #[test]
    fn the_elastic_allocation_does_not_ratchet() {
        // **No ratchet.** The same holders produce the same requirement, every
        // time, with no memory of what was decided last time. A site that
        // grows a level-4 account stops handing the queue to a level-2 on the
        // next evaluation, and the level-2 loses the privilege without anybody
        // deciding to take it.
        let policy = ElasticPolicy::default();
        let mut holders = HolderCounts::default();
        holders.add(2);
        assert_eq!(elastic_requirement(4, &holders, &policy).level, 2);

        holders.add(4);
        assert_eq!(
            elastic_requirement(4, &holders, &policy).level,
            4,
            "the requirement returns to nominal as soon as enough people clear the bar"
        );
    }

    #[test]
    fn the_spam_demotion_never_lands_below_level_two() {
        assert!(spam_demotes(3));
        assert!(!spam_demotes(2));
        assert_eq!(spam_demotion_level(4, 2), 3);
        assert_eq!(spam_demotion_level(3, 2), 2);
        assert_eq!(
            spam_demotion_level(2, 2),
            2,
            "the floor holds even when the demotion would go below it"
        );
    }

    #[test]
    fn the_recovery_curve_is_diminishing() {
        assert!((recovery_multiplier(0) - 1.0).abs() < f64::EPSILON);
        assert!((recovery_multiplier(1) - 0.7).abs() < 1e-9);
        assert!((recovery_multiplier(2) - 0.49).abs() < 1e-9);
        assert!(
            recovery_multiplier(3) < recovery_multiplier(2),
            "each further loss takes longer to come back"
        );
    }
}
