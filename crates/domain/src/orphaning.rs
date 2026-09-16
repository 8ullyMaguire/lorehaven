//! Work orphaning and succession (spec §24.8, §32.3).
//!
//! Orphaning marks a work as ownerless — its creator has relinquished it.
//! A successor may later claim it. Orphaning is not deletion: the work,
//! its chapters, its revisions and its publication history remain. Only
//! the ownership link is severed.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Why a work was orphaned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrphanReason {
    /// The creator chose to relinquish the work.
    Relinquished,
    /// The creator's pseud was deleted.
    PseudDeleted,
    /// The creator's account was deleted.
    AccountDeleted,
}

impl OrphanReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Relinquished => "relinquished",
            Self::PseudDeleted => "pseud_deleted",
            Self::AccountDeleted => "account_deleted",
        }
    }
}

impl std::fmt::Display for OrphanReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for OrphanReason {
    type Err = OrphanError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "relinquished" => Ok(Self::Relinquished),
            "pseud_deleted" => Ok(Self::PseudDeleted),
            "account_deleted" => Ok(Self::AccountDeleted),
            _ => Err(OrphanError::UnknownReason(s.to_owned())),
        }
    }
}

/// The kind of succession that replaced an orphaned work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuccessionKind {
    /// A new work was created as a successor.
    NewWork,
    /// An existing work was linked as a successor.
    ExistingWork,
}

impl SuccessionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NewWork => "new_work",
            Self::ExistingWork => "existing_work",
        }
    }
}

impl std::fmt::Display for SuccessionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Errors that can occur during orphaning or succession.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum OrphanError {
    #[error("unknown orphan reason: {0}")]
    UnknownReason(String),
    #[error("work is not published")]
    NotPublished,
    #[error("work is already orphaned")]
    AlreadyOrphaned,
    #[error("work is orphaned; only the owner can orphan it")]
    NotOwner,
    #[error("succession requires a successor work id")]
    MissingSuccessor,
    #[error("successor work must be published")]
    SuccessorNotPublished,
    #[error("work is orphaned and cannot be edited")]
    IsOrphaned,
}

/// The result of orphaning a work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrphanRecord {
    pub work_id: String,
    pub orphaned_at: String,
    pub reason: OrphanReason,
    pub former_owner_pseud_id: String,
    pub successor_work_id: Option<String>,
    pub succession_kind: Option<SuccessionKind>,
}

impl OrphanRecord {
    pub fn is_orphaned(&self) -> bool {
        true
    }

    pub fn successor(&self) -> Option<&str> {
        self.successor_work_id.as_deref()
    }
}

/// Validate that an orphan request is well-formed.
///
/// - The work must be published (a draft is deleted, not orphaned).
/// - The caller must be the owner (unless the pseud/account is being deleted).
/// - The work must not already be orphaned.
pub fn validate_orphan(
    work_owner_pseud_id: &str,
    caller_pseud_id: &str,
    work_lifecycle: &str,
    already_orphaned: bool,
    reason: OrphanReason,
) -> Result<(), OrphanError> {
    if already_orphaned {
        return Err(OrphanError::AlreadyOrphaned);
    }
    if work_lifecycle != "published" {
        return Err(OrphanError::NotPublished);
    }
    // Pseud/account deletion orphans works regardless of caller.
    if reason == OrphanReason::Relinquished && work_owner_pseud_id != caller_pseud_id {
        return Err(OrphanError::NotOwner);
    }
    Ok(())
}

/// Validate a succession request.
pub fn validate_succession(
    successor_lifecycle: &str,
    has_successor_id: bool,
) -> Result<(), OrphanError> {
    if !has_successor_id {
        return Err(OrphanError::MissingSuccessor);
    }
    if successor_lifecycle != "published" {
        return Err(OrphanError::SuccessorNotPublished);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn reason_round_trips() {
        for r in [
            OrphanReason::Relinquished,
            OrphanReason::PseudDeleted,
            OrphanReason::AccountDeleted,
        ] {
            assert_eq!(OrphanReason::from_str(r.as_str()).unwrap(), r);
        }
        assert!(OrphanReason::from_str("unknown").is_err());
    }

    #[test]
    fn validate_orphan_happy_path() {
        assert!(
            validate_orphan("p1", "p1", "published", false, OrphanReason::Relinquished).is_ok()
        );
    }

    #[test]
    fn validate_orphan_not_published() {
        assert_eq!(
            validate_orphan("p1", "p1", "draft", false, OrphanReason::Relinquished),
            Err(OrphanError::NotPublished)
        );
    }

    #[test]
    fn validate_orphan_not_owner() {
        assert_eq!(
            validate_orphan("p1", "p2", "published", false, OrphanReason::Relinquished),
            Err(OrphanError::NotOwner)
        );
    }

    #[test]
    fn validate_orphan_already_orphaned() {
        assert_eq!(
            validate_orphan("p1", "p1", "published", true, OrphanReason::Relinquished),
            Err(OrphanError::AlreadyOrphaned),
        );
    }

    #[test]
    fn validate_orphan_pseud_deleted_bypasses_owner() {
        // Pseud deletion orphans works regardless of who calls.
        assert!(validate_orphan(
            "p1",
            "system",
            "published",
            false,
            OrphanReason::PseudDeleted
        )
        .is_ok());
    }

    #[test]
    fn validate_succession_happy_path() {
        assert!(validate_succession("published", true).is_ok());
    }

    #[test]
    fn validate_succession_missing_id() {
        assert_eq!(
            validate_succession("published", false),
            Err(OrphanError::MissingSuccessor)
        );
    }

    #[test]
    fn validate_succession_not_published() {
        assert_eq!(
            validate_succession("draft", true),
            Err(OrphanError::SuccessorNotPublished)
        );
    }
}
