//! M14 — Governance domain: trust levels, quorum, sanctions policy.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Trust levels (spec §19.1)
// ---------------------------------------------------------------------------
//
// **The ladder itself lives in [`crate::trust`]**, where the names and the
// unlock table are configuration (spec §14.4). What is left here is the
// *vocabulary of the numbers* — the seven levels, named — because a number
// with no name is a magic constant and a magic constant is how a ladder stops
// being a ladder.
//
// The names are **the default ladder's** names. An instance that renames a
// level has not renumbered it, and the code below still refers to the level
// *number* while the route asks the ladder which *privilege* it needs.

/// The lowest level: a brand-new account.
pub const TL_NEW: i64 = 0;
/// The level of a reader with some standing.
pub const TL_ESTABLISHED: i64 = 1;
/// The level of an established member.
pub const TL_REGULAR: i64 = 2;
/// The level of a reviewed contributor.
pub const TL_REVIEWED: i64 = 3;
/// The level of a moderator.
pub const TL_STEWARD: i64 = 4;
/// The level of a senior reviewer.
pub const TL_SENIOR: i64 = 5;
/// The level of a trustee.
pub const TL_TRUSTEE: i64 = 6;

/// The highest level on the ladder.
pub const TRUST_MAX: i64 = 6;

/// Whether a trust level is permitted to perform an action that requires a
/// minimum level. Trust raises ceilings; it does not grant permissions that
/// are gated elsewhere (spec §19.2).
pub fn meets_trust_floor(level: i64, required: i64) -> bool {
    level >= required
}

// ---------------------------------------------------------------------------
// Sanctions (spec §19.5–19.7)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SanctionKind {
    RateLimit,
    Shadow,
    Suspend,
}

impl SanctionKind {
    /// Default duration in days. Shadowbans are 30 days (spec §19.7);
    /// rate-limits 7 days; suspensions 14 days. Operator-held suspensions
    /// have no default (must be explicitly ended).
    pub fn default_duration_days(&self) -> Option<i64> {
        match self {
            Self::RateLimit => Some(7),
            Self::Shadow => Some(30),
            Self::Suspend => Some(14),
        }
    }

    /// Whether this kind of sanction requires a quorum (shadow + suspend do;
    /// rate-limit does not for single-issuer).
    pub fn requires_quorum(&self) -> bool {
        matches!(self, Self::Shadow | Self::Suspend)
    }
}

// ---------------------------------------------------------------------------
// Quorum (spec §19.4)
// ---------------------------------------------------------------------------

/// Minimum independent reviewers for a given action type.
pub fn quorum_size(action: QuorumAction) -> u32 {
    match action {
        QuorumAction::RoutineTagChange => 2,
        QuorumAction::HighImpactTagOrIdentityMerge => 3,
        QuorumAction::RoutineSanction => 2, // proposer + independent
        QuorumAction::PermanentBan => 3,
        QuorumAction::Appeal => 2,
        QuorumAction::EmergencyContainment => 1,
        QuorumAction::Shadowban => 2,
        QuorumAction::DmcaTakedown => 1,
        QuorumAction::MetadataCorrection => 2,
    }
}

#[derive(Debug, Clone, Copy)]
pub enum QuorumAction {
    RoutineTagChange,
    HighImpactTagOrIdentityMerge,
    RoutineSanction,
    PermanentBan,
    Appeal,
    EmergencyContainment,
    Shadowban,
    DmcaTakedown,
    MetadataCorrection,
}

/// Validate that a reviewer is independent of the subject: not the actor
/// themselves, not the same account behind two pseuds, not the issuer of a
/// sanction being appealed.
pub fn is_independent_reviewer(
    reviewer_account: &str,
    subject_account: &str,
    sanction_issuer: Option<&str>,
) -> bool {
    if reviewer_account == subject_account {
        return false;
    }
    if let Some(issuer) = sanction_issuer {
        if reviewer_account == issuer {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Reports (spec §19.3)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReportState {
    Open,
    InReview,
    Resolved,
    Dismissed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReportReason {
    Spam,
    Harassment,
    Copyright,
    InappropriateContent,
    PositivityViolation,
    SafetyConcern,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutcome {
    Uphold,
    Dismiss,
    Escalate,
    Recuse,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meets_trust_floor_works() {
        assert!(meets_trust_floor(TL_REGULAR, TL_ESTABLISHED));
        assert!(!meets_trust_floor(TL_NEW, TL_ESTABLISHED));
    }

    #[test]
    fn sanction_kind_defaults() {
        assert_eq!(SanctionKind::RateLimit.default_duration_days(), Some(7));
        assert_eq!(SanctionKind::Shadow.default_duration_days(), Some(30));
        assert_eq!(SanctionKind::Suspend.default_duration_days(), Some(14));
        assert!(SanctionKind::Shadow.requires_quorum());
        assert!(!SanctionKind::RateLimit.requires_quorum());
    }

    #[test]
    fn quorum_sizes() {
        assert_eq!(quorum_size(QuorumAction::RoutineTagChange), 2);
        assert_eq!(quorum_size(QuorumAction::HighImpactTagOrIdentityMerge), 3);
        assert_eq!(quorum_size(QuorumAction::PermanentBan), 3);
        assert_eq!(quorum_size(QuorumAction::Shadowban), 2);
        assert_eq!(quorum_size(QuorumAction::EmergencyContainment), 1);
    }

    #[test]
    fn independence_check() {
        assert!(!is_independent_reviewer("acc_a", "acc_a", None));
        assert!(!is_independent_reviewer("acc_a", "acc_b", Some("acc_a")));
        assert!(is_independent_reviewer("acc_a", "acc_b", Some("acc_c")));
        assert!(is_independent_reviewer("acc_a", "acc_b", None));
    }
}
