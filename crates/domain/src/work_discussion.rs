//! Work discussion modes and the typed-vote reaction bar (spec §35.0–35.1).
//!
//! Two behaviors live here:
//!
//! 1. **Mode resolution.** Each work carries a discussion mode; the instance
//!    sets a default that applies to *new* works only, and an author may
//!    override it per work. Resolution is: the work's own mode first, then
//!    the instance default.
//! 2. **Reaction rules.** The work-page reaction bar is typed votes — one
//!    per pseud, changeable, retractable — restricted to the positive set
//!    plus `disagree`.

use serde::{Deserialize, Serialize};

/// How discussion happens around a work (spec §35.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkDiscussionMode {
    /// Typed-vote reaction bar on the work page; text discussion lives in
    /// the linked forum thread. The Lorehaven default.
    ThreadOnly,
    /// Inline work/chapter comments, no forum link (legacy surface).
    CommentsOnly,
    /// Both surfaces; splits conversation and is admin-warned.
    Both,
}

impl WorkDiscussionMode {
    /// Parse the storage/API form.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "thread_only" => Some(Self::ThreadOnly),
            "comments_only" => Some(Self::CommentsOnly),
            "both" => Some(Self::Both),
            _ => None,
        }
    }

    /// Storage/API form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ThreadOnly => "thread_only",
            Self::CommentsOnly => "comments_only",
            Self::Both => "both",
        }
    }

    /// Whether the work-page comment form is offered.
    pub fn comments_enabled(self) -> bool {
        matches!(self, Self::CommentsOnly | Self::Both)
    }

    /// Whether the reaction bar and Discuss link are offered.
    pub fn thread_enabled(self) -> bool {
        matches!(self, Self::ThreadOnly | Self::Both)
    }
}

impl Default for WorkDiscussionMode {
    fn default() -> Self {
        // Existing works keep today's behavior: the migration backfills
        // `comments_only`, and flipping the instance default is an
        // operator's deliberate act (spec §35.0 migration path).
        Self::CommentsOnly
    }
}

/// Resolve the effective mode for a work: the work's own mode, or the
/// instance default when the work states none.
///
/// The default is applied at creation time in storage, so `None` here means
/// a legacy row written before the column existed — which resolves to the
/// legacy behavior, never to the new one, retroactively.
pub fn resolve_discussion_mode(
    work_mode: Option<WorkDiscussionMode>,
    instance_default: WorkDiscussionMode,
) -> WorkDiscussionMode {
    work_mode.unwrap_or(WorkDiscussionMode::CommentsOnly)
}

/// The vote types offered on the work-page reaction bar (spec §35.1).
///
/// The positive set mirrors the default forum taxonomy's positive types
/// (§35.2); `disagree` is the one negative type allowed on this surface,
/// costed and meta-moderated like every negative vote.
pub const WORK_REACTION_TYPES: &[&str] = &[
    "well_written",
    "insightful",
    "funny",
    "interesting",
    "disagree",
];

/// Whether a reaction vote type may be cast on the work-page bar.
pub fn is_valid_work_reaction(vote_type: &str) -> bool {
    WORK_REACTION_TYPES.contains(&vote_type)
}

/// The outcome of a reaction cast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReactionOutcome {
    /// A new vote was recorded.
    Cast,
    /// The existing vote was changed to a new type.
    Changed,
    /// The existing vote was retracted (vote_type was `None` in the request,
    /// or matched the stored vote).
    Retracted,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_round_trips() {
        for mode in [
            WorkDiscussionMode::ThreadOnly,
            WorkDiscussionMode::CommentsOnly,
            Self::Both,
        ] {
            assert_eq!(WorkDiscussionMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(WorkDiscussionMode::parse("nonsense"), None);
    }

    #[test]
    fn surface_gating_matches_spec() {
        // ThreadOnly: bar + thread, no comment form.
        let m = WorkDiscussionMode::ThreadOnly;
        assert!(m.thread_enabled());
        assert!(!m.comments_enabled());

        // CommentsOnly: comment form, no thread link.
        let m = WorkDiscussionMode::CommentsOnly;
        assert!(!m.thread_enabled());
        assert!(m.comments_enabled());

        // Both: both surfaces.
        let m = WorkDiscussionMode::Both;
        assert!(m.thread_enabled());
        assert!(m.comments_enabled());
    }

    #[test]
    fn legacy_works_resolve_to_comments_only() {
        // A work with no mode (a legacy row) never picks up the instance
        // default retroactively.
        assert_eq!(
            resolve_discussion_mode(None, WorkDiscussionMode::ThreadOnly),
            WorkDiscussionMode::CommentsOnly
        );
        // A work with its own mode keeps it regardless of the default.
        assert_eq!(
            resolve_discussion_mode(
                Some(WorkDiscussionMode::CommentsOnly),
                WorkDiscussionMode::ThreadOnly
            ),
            WorkDiscussionMode::CommentsOnly
        );
    }

    #[test]
    fn reaction_types_are_the_spec_set() {
        for t in [
            "well_written",
            "insightful",
            "funny",
            "interesting",
            "disagree",
        ] {
            assert!(is_valid_work_reaction(t), "{t} should be valid");
        }
        assert!(!is_valid_work_reaction("like"));
        assert!(!is_valid_work_reaction(""));
    }
}
