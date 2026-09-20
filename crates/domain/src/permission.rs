//! M27 — Permission statements, derivative lineage, and the exclusion registry.
//!
//! Spec §33.1. A **permission statement** on every work and creator covering
//! podfic, translation, remix/fork, continuation, redistribution and AI training,
//! each `yes | ask | no | unstated` and defaulting to `unstated`. A **derivative
//! lineage** edge records when one work derives from another with a relationship
//! kind and provenance. An **exclusion registry** names external creatives and
//! works that must not be imported, narrated, translated, remixed or announced.

use serde::{Deserialize, Serialize};
use std::fmt;

/// The permission answer set for each derivative door.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Permission {
    /// Explicitly permitted.
    Yes,
    /// Asks the author first; never refused in the author's name.
    Ask,
    /// Explicitly refused.
    No,
    /// No statement recorded — defaults to the most restrictive interpretation.
    Unstated,
}

impl Permission {
    /// The wire and column representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::Ask => "ask",
            Self::No => "no",
            Self::Unstated => "unstated",
        }
    }

    /// Parse the stored representation.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "yes" => Self::Yes,
            "ask" => Self::Ask,
            "no" => Self::No,
            "unstated" => Self::Unstated,
            _ => return None,
        })
    }

    /// Whether this permission permits the action outright.
    #[must_use]
    pub fn permits(self) -> bool {
        matches!(self, Self::Yes)
    }

    /// Whether this permission forbids the action outright.
    #[must_use]
    pub fn forbids(self) -> bool {
        matches!(self, Self::No | Self::Unstated)
    }

    /// Whether this permission requires asking the author first.
    #[must_use]
    pub fn asks(self) -> bool {
        matches!(self, Self::Ask)
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The doors a permission statement covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PermissionDoor {
    /// Podfic (fan fiction of another work).
    Podfic,
    /// Translation into another language.
    Translation,
    /// Remix/fork of the work.
    Remix,
    /// Continuation (sequel, side story, etc).
    Continuation,
    /// Redistribution (mirroring, syndication).
    Redistribution,
    /// AI training on the work or creator's corpus.
    AiTraining,
}

impl PermissionDoor {
    /// The wire and column representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Podfic => "podfic",
            Self::Translation => "translation",
            Self::Remix => "remix",
            Self::Continuation => "continuation",
            Self::Redistribution => "redistribution",
            Self::AiTraining => "ai_training",
        }
    }

    /// Parse the stored representation.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "podfic" => Self::Podfic,
            "translation" => Self::Translation,
            "remix" => Self::Remix,
            "continuation" => Self::Continuation,
            "redistribution" => Self::Redistribution,
            "ai_training" => Self::AiTraining,
            _ => return None,
        })
    }

    /// All doors, in canonical order.
    #[must_use]
    pub fn all() -> [Self; 6] {
        [
            Self::Podfic,
            Self::Translation,
            Self::Remix,
            Self::Continuation,
            Self::Redistribution,
            Self::AiTraining,
        ]
    }
}

impl fmt::Display for PermissionDoor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A full permission statement: one answer per door.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionStatement {
    pub podfic: Permission,
    pub translation: Permission,
    pub remix: Permission,
    pub continuation: Permission,
    pub redistribution: Permission,
    pub ai_training: Permission,
}

impl PermissionStatement {
    /// The default statement: all doors `unstated`.
    #[must_use]
    pub fn unstated() -> Self {
        Self {
            podfic: Permission::Unstated,
            translation: Permission::Unstated,
            remix: Permission::Unstated,
            continuation: Permission::Unstated,
            redistribution: Permission::Unstated,
            ai_training: Permission::Unstated,
        }
    }

    /// Look up the answer for a specific door.
    #[must_use]
    pub fn for_door(&self, door: PermissionDoor) -> Permission {
        match door {
            PermissionDoor::Podfic => self.podfic,
            PermissionDoor::Translation => self.translation,
            PermissionDoor::Remix => self.remix,
            PermissionDoor::Continuation => self.continuation,
            PermissionDoor::Redistribution => self.redistribution,
            PermissionDoor::AiTraining => self.ai_training,
        }
    }

    /// Whether every door is `unstated`.
    #[must_use]
    pub fn is_unstated(&self) -> bool {
        self.podfic == Permission::Unstated
            && self.translation == Permission::Unstated
            && self.remix == Permission::Unstated
            && self.continuation == Permission::Unstated
            && self.redistribution == Permission::Unstated
            && self.ai_training == Permission::Unstated
    }
}

impl Default for PermissionStatement {
    fn default() -> Self {
        Self::unstated()
    }
}

/// The kind of derivative relationship between two works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LineageKind {
    /// Translation of the source work.
    Translation,
    /// Podfic (fan fiction inspired by the source).
    Podfic,
    /// Remix/fork of the source.
    Remix,
    /// Continuation (sequel, side story).
    Continuation,
    /// Inspired by (looser connection).
    InspiredBy,
}

impl LineageKind {
    /// The wire and column representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Translation => "translation",
            Self::Podfic => "podfic",
            Self::Remix => "remix",
            Self::Continuation => "continuation",
            Self::InspiredBy => "inspired_by",
        }
    }

    /// Parse the stored representation.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "translation" => Self::Translation,
            "podfic" => Self::Podfic,
            "remix" => Self::Remix,
            "continuation" => Self::Continuation,
            "inspired_by" => Self::InspiredBy,
            _ => return None,
        })
    }
}

impl fmt::Display for LineageKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A derivative lineage edge: `from_work_id` derives from `to_work_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineageEdge {
    pub id: String,
    pub from_work_id: String,
    pub to_work_id: String,
    pub kind: LineageKind,
    pub provenance: String,
    pub created_at: String,
}

/// An entry in the exclusion registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExclusionEntry {
    pub id: String,
    pub target_type: ExclusionTarget,
    pub target_id: String,
    pub reason: String,
    pub created_by: String,
    pub created_at: String,
}

/// What kind of entity is excluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExclusionTarget {
    /// A specific work.
    Work,
    /// A specific creator (author/pseud).
    Creator,
}

impl ExclusionTarget {
    /// The wire and column representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Work => "work",
            Self::Creator => "creator",
        }
    }

    /// Parse the stored representation.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "work" => Self::Work,
            "creator" => Self::Creator,
            _ => return None,
        })
    }
}

impl fmt::Display for ExclusionTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_as_str_round_trips() {
        for p in [
            Permission::Yes,
            Permission::Ask,
            Permission::No,
            Permission::Unstated,
        ] {
            assert_eq!(Permission::parse(p.as_str()), Some(p));
        }
    }

    #[test]
    fn permission_door_as_str_round_trips() {
        for d in PermissionDoor::all() {
            assert_eq!(PermissionDoor::parse(d.as_str()), Some(d));
        }
    }

    #[test]
    fn lineage_kind_as_str_round_trips() {
        for k in [
            LineageKind::Translation,
            LineageKind::Podfic,
            LineageKind::Remix,
            LineageKind::Continuation,
            LineageKind::InspiredBy,
        ] {
            assert_eq!(LineageKind::parse(k.as_str()), Some(k));
        }
    }

    #[test]
    fn exclusion_target_as_str_round_trips() {
        assert_eq!(ExclusionTarget::parse("work"), Some(ExclusionTarget::Work));
        assert_eq!(
            ExclusionTarget::parse("creator"),
            Some(ExclusionTarget::Creator)
        );
        assert_eq!(ExclusionTarget::parse("bogus"), None);
    }

    #[test]
    fn permission_statement_unstated_by_default() {
        let s = PermissionStatement::unstated();
        assert!(s.is_unstated());
        assert_eq!(s.for_door(PermissionDoor::Podfic), Permission::Unstated);
        assert_eq!(s.for_door(PermissionDoor::AiTraining), Permission::Unstated);
    }

    #[test]
    fn permission_statement_for_door_returns_correct_answer() {
        let mut s = PermissionStatement::unstated();
        s.podfic = Permission::Yes;
        s.translation = Permission::No;
        assert_eq!(s.for_door(PermissionDoor::Podfic), Permission::Yes);
        assert_eq!(s.for_door(PermissionDoor::Translation), Permission::No);
        assert_eq!(s.for_door(PermissionDoor::Remix), Permission::Unstated);
    }

    #[test]
    fn permission_semantics() {
        assert!(Permission::Yes.permits());
        assert!(!Permission::No.permits());
        assert!(!Permission::Unstated.permits());
        assert!(!Permission::Ask.permits());

        assert!(Permission::No.forbids());
        assert!(Permission::Unstated.forbids());
        assert!(!Permission::Yes.forbids());
        assert!(!Permission::Ask.forbids());

        assert!(Permission::Ask.asks());
        assert!(!Permission::Yes.asks());
        assert!(!Permission::No.asks());
        assert!(!Permission::Unstated.asks());
    }

    #[test]
    fn invalid_permission_returns_none() {
        assert_eq!(Permission::parse("bogus"), None);
        assert_eq!(Permission::parse(""), None);
    }
}
