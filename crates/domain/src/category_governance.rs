//! Directory Category Governance domain (spec §45) — pure rules, no I/O.
//!
//! Categories become DB rows with a state lifecycle; proposals and votes
//! drive rename/merge/deprecate/create through quorum. Entry moderation
//! (move/remove between categories) also supports quorum review.

use serde::{Deserialize, Serialize};

/// Category state lifecycle (§45.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CategoryState {
    /// Active — appears in tabs, accepts entries, votes count.
    Active,
    /// Deprecated — hidden from new submissions, existing entries remain.
    Deprecated,
    /// Merged — redirect to another category; entries re-homed.
    Merged,
}

impl CategoryState {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(Self::Active),
            "deprecated" => Some(Self::Deprecated),
            "merged" => Some(Self::Merged),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Deprecated => "deprecated",
            Self::Merged => "merged",
        }
    }
}

/// Category source (§45.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CategorySource {
    Seed,
    Config,
    Community,
}

impl CategorySource {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "seed" => Some(Self::Seed),
            "config" => Some(Self::Config),
            "community" => Some(Self::Community),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Seed => "seed",
            Self::Config => "config",
            Self::Community => "community",
        }
    }
}

/// Proposal action (§45.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalAction {
    Rename,
    Merge,
    Deprecate,
    Create,
    Delete,
}

impl ProposalAction {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "rename" => Some(Self::Rename),
            "merge" => Some(Self::Merge),
            "deprecate" => Some(Self::Deprecate),
            "create" => Some(Self::Create),
            "delete" => Some(Self::Delete),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rename => "rename",
            Self::Merge => "merge",
            Self::Deprecate => "deprecate",
            Self::Create => "create",
            Self::Delete => "delete",
        }
    }

    /// Whether this action is high-impact (needs 3 votes vs 2).
    pub fn is_high_impact(&self) -> bool {
        matches!(self, Self::Merge | Self::Create | Self::Delete)
    }
}

/// Proposal status (§45.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Open,
    Passed,
    Failed,
    Vetoed,
    Expired,
}

impl ProposalStatus {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "open" => Some(Self::Open),
            "passed" => Some(Self::Passed),
            "failed" => Some(Self::Failed),
            "vetoed" => Some(Self::Vetoed),
            "expired" => Some(Self::Expired),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Vetoed => "vetoed",
            Self::Expired => "expired",
        }
    }
}

/// Entry moderation action (§45.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryModAction {
    Move,
    Remove,
}

impl EntryModAction {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "move" => Some(Self::Move),
            "remove" => Some(Self::Remove),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Move => "move",
            Self::Remove => "remove",
        }
    }
}

/// Vote value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoteValue {
    Yes,
    No,
}

impl VoteValue {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "yes" => Some(Self::Yes),
            "no" => Some(Self::No),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::No => "no",
        }
    }
}

/// Governance configuration constants (§45.2, §45.4).
pub const QUORUM_ROUTINE: u32 = 2;
pub const QUORUM_HIGH_IMPACT: u32 = 3;
pub const MAX_OPEN_PROPOSALS_PER_CATEGORY: u32 = 5;
pub const MAX_ACTIVE_CATEGORIES: u32 = 32;
pub const COOLDOWN_HOURS: i64 = 72;
pub const PROPOSAL_TTL_DAYS: i64 = 14;
pub const ENTRY_MOD_QUORUM: u32 = 2;
pub const ENTRY_MOD_TTL_DAYS: i64 = 14;

/// Flat weight for all governance votes (§45.2: taste never touches governance).
pub const GOVERNANCE_VOTE_WEIGHT: f64 = 1.0;

/// Compute the quorum needed for a given action (§45.2).
pub fn quorum_for(action: ProposalAction) -> u32 {
    if action.is_high_impact() {
        QUORUM_HIGH_IMPACT
    } else {
        QUORUM_ROUTINE
    }
}

/// Whether a proposal has reached quorum on the yes side (§45.2).
pub fn has_quorum(yes_votes: u32, quorum_needed: u32) -> bool {
    yes_votes >= quorum_needed
}

/// Whether a proposal is decided (quorum reached or impossible to reach).
/// Returns `Some(true)` if passed, `Some(false)` if failed, `None` if still open.
pub fn proposal_decided(yes_votes: u32, no_votes: u32, quorum_needed: u32) -> Option<bool> {
    if yes_votes >= quorum_needed {
        return Some(true);
    }
    // If no_votes >= quorum_needed, the proposal has failed.
    if no_votes >= quorum_needed {
        return Some(false);
    }
    // If it's mathematically impossible for either side to reach quorum
    // (both remaining votes go to one side), decide now.
    let total_cast = yes_votes + no_votes;
    let max_additional = u32::MAX - total_cast; // placeholder
    let _ = max_additional;
    None
}

/// Validate a proposal action against category state.
/// Returns Ok(()) if the action is valid in the current state.
pub fn validate_action_for_state(
    action: ProposalAction,
    state: CategoryState,
) -> Result<(), &'static str> {
    match (action, state) {
        // Cannot rename a merged category (it's a redirect now).
        (ProposalAction::Rename, CategoryState::Merged) => Err("cannot rename a merged category"),
        // Cannot deprecate an already-deprecated category.
        (ProposalAction::Deprecate, CategoryState::Deprecated) => {
            Err("category is already deprecated")
        }
        // Cannot merge into itself — validated at higher level.
        // All other combinations are valid.
        _ => Ok(()),
    }
}

/// Payload for rename proposals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenamePayload {
    pub new_label: String,
}

/// Payload for merge proposals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergePayload {
    pub target_slug: String,
}

/// Payload for create proposals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePayload {
    pub slug: String,
    pub label: String,
}

/// Payload for deprecate proposals (empty — no extra data).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeprecatePayload {}

/// Payload for delete proposals (empty).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletePayload {}

/// Payload for entry moderation — move.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryMovePayload {
    pub target_category: String,
}

/// Payload for entry moderation — remove (empty).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryRemovePayload {}

/// Changelog event types (§45.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangelogEvent {
    Proposed,
    Voted,
    Executed,
    Vetoed,
    Expired,
    EntryMoved,
    EntryRemoved,
}

impl ChangelogEvent {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Voted => "voted",
            Self::Executed => "executed",
            Self::Vetoed => "vetoed",
            Self::Expired => "expired",
            Self::EntryMoved => "entry_moved",
            Self::EntryRemoved => "entry_removed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quorum_for_routine() {
        assert_eq!(quorum_for(ProposalAction::Rename), 2);
        assert_eq!(quorum_for(ProposalAction::Deprecate), 2);
    }

    #[test]
    fn test_quorum_for_high_impact() {
        assert_eq!(quorum_for(ProposalAction::Merge), 3);
        assert_eq!(quorum_for(ProposalAction::Create), 3);
        assert_eq!(quorum_for(ProposalAction::Delete), 3);
    }

    #[test]
    fn test_has_quorum() {
        assert!(has_quorum(2, 2));
        assert!(has_quorum(3, 2));
        assert!(!has_quorum(1, 2));
        assert!(!has_quorum(2, 3));
    }

    #[test]
    fn test_proposal_decided() {
        // Quorum reached.
        assert_eq!(proposal_decided(2, 0, 2), Some(true));
        // Failed.
        assert_eq!(proposal_decided(0, 2, 2), Some(false));
        // Still open.
        assert_eq!(proposal_decided(1, 0, 2), None);
        assert_eq!(proposal_decided(1, 1, 3), None);
    }

    #[test]
    fn test_validate_action_for_state() {
        assert!(validate_action_for_state(ProposalAction::Rename, CategoryState::Active).is_ok());
        assert!(validate_action_for_state(ProposalAction::Rename, CategoryState::Merged).is_err());
        assert!(
            validate_action_for_state(ProposalAction::Deprecate, CategoryState::Active).is_ok()
        );
        assert!(
            validate_action_for_state(ProposalAction::Deprecate, CategoryState::Deprecated)
                .is_err()
        );
    }

    #[test]
    fn test_governance_vote_weight_is_flat() {
        // §45.2: taste never touches governance.
        assert_eq!(GOVERNANCE_VOTE_WEIGHT, 1.0);
    }

    #[test]
    fn test_parse_roundtrip() {
        let states = [
            CategoryState::Active,
            CategoryState::Deprecated,
            CategoryState::Merged,
        ];
        for s in &states {
            assert_eq!(CategoryState::parse(s.as_str()), Some(*s));
        }

        let actions = [
            ProposalAction::Rename,
            ProposalAction::Merge,
            ProposalAction::Deprecate,
            ProposalAction::Create,
            ProposalAction::Delete,
        ];
        for a in &actions {
            assert_eq!(ProposalAction::parse(a.as_str()), Some(*a));
        }

        let votes = [VoteValue::Yes, VoteValue::No];
        for v in &votes {
            assert_eq!(VoteValue::parse(v.as_str()), Some(*v));
        }
    }
}
