//! M14 — Governance domain: trust levels, quorum, sanctions policy.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Trust levels (spec §19.1)
// ---------------------------------------------------------------------------

pub const TL_NEW: i64 = 0;
pub const TL_ESTABLISHED: i64 = 1;
pub const TL_REGULAR: i64 = 2;
pub const TL_REVIEWED: i64 = 3;
pub const TL_STEWARD: i64 = 4;
pub const TL_SENIOR: i64 = 5;
pub const TL_TRUSTEE: i64 = 6;

pub const TRUST_MAX: i64 = 6;

/// Trust level progression. Behaviour-record-driven, never purchase-driven.
/// Criteria are documented per level; this function encodes the floor.
pub fn trust_from_behaviour(
    account_age_days: i64,
    good_standing_days: i64,
    prior_sanctions: i64,
    active_sanctions: i64,
    reviewer_nominations: i64,
    quorum_approvals: i64,
) -> i64 {
    if active_sanctions > 0 {
        // Active sanctions cap at TL1 regardless of other signals.
        return TL_ESTABLISHED;
    }
    if account_age_days < 7 {
        return TL_NEW;
    }
    if good_standing_days < 30 || prior_sanctions > 0 {
        return TL_ESTABLISHED;
    }
    if good_standing_days < 90 {
        return TL_REGULAR;
    }
    // TL3 and above require reviewer nomination + quorum approval.
    if reviewer_nominations < 1 || quorum_approvals < 1 {
        return TL_REGULAR;
    }
    if reviewer_nominations >= 1 && quorum_approvals >= 1 && good_standing_days < 365 {
        return TL_REVIEWED;
    }
    if reviewer_nominations >= 2 && quorum_approvals >= 2 {
        return TL_STEWARD;
    }
    TL_REGULAR
}

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
    fn trust_new_account_floor() {
        let level = trust_from_behaviour(1, 0, 0, 0, 0, 0);
        assert_eq!(level, TL_NEW);
    }

    #[test]
    fn trust_active_sanctions_cap() {
        let level = trust_from_behaviour(500, 500, 0, 1, 10, 10);
        assert_eq!(level, TL_ESTABLISHED);
    }

    #[test]
    fn trust_established_standing() {
        let level = trust_from_behaviour(30, 30, 0, 0, 0, 0);
        assert_eq!(level, TL_ESTABLISHED);
    }

    #[test]
    fn trust_regular_without_nominations() {
        let level = trust_from_behaviour(180, 180, 0, 0, 0, 0);
        assert_eq!(level, TL_REGULAR);
    }

    #[test]
    fn trust_reviewed_with_nominations() {
        let level = trust_from_behaviour(200, 200, 0, 0, 1, 1);
        assert_eq!(level, TL_REVIEWED);
    }

    #[test]
    fn trust_steward_with_multiple_nominations() {
        let level = trust_from_behaviour(500, 500, 0, 0, 2, 2);
        assert_eq!(level, TL_STEWARD);
    }

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
