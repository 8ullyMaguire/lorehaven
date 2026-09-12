//! Blocking and muting domain logic.
//!
//! Spec §7.2.2. Pure functions — no I/O.

use crate::ids::AccountId;

/// The scope of a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockScope {
    /// Block all interactions.
    All,
    /// Block only messages.
    Messages,
    /// Block only comments.
    Comments,
}

impl BlockScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Messages => "messages",
            Self::Comments => "comments",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "all" => Self::All,
            "messages" => Self::Messages,
            "comments" => Self::Comments,
            _ => return None,
        })
    }
}

/// Check if `subject` is blocked by `viewer` for the given scope.
///
/// A block with scope `All` always blocks. A block with scope `Messages`
/// blocks only messages. A block with scope `Comments` blocks only
/// comments.
///
/// This is the single decision function every social path uses.
pub fn blocked_between(
    viewer: &AccountId,
    subject: &AccountId,
    scope: BlockScope,
    viewer_blocks_all: bool,
    viewer_blocks_messages: bool,
    viewer_blocks_comments: bool,
) -> bool {
    if viewer == subject {
        return false;
    }
    match scope {
        BlockScope::All => viewer_blocks_all,
        BlockScope::Messages => viewer_blocks_all || viewer_blocks_messages,
        BlockScope::Comments => viewer_blocks_all || viewer_blocks_comments,
    }
}

/// Check if `subject` is muted by `viewer`.
pub fn muted_between(viewer: &AccountId, subject: &AccountId, viewer_mutes: bool) -> bool {
    if viewer == subject {
        return false;
    }
    viewer_mutes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trips() {
        for scope in [BlockScope::All, BlockScope::Messages, BlockScope::Comments] {
            assert_eq!(BlockScope::parse(scope.as_str()), Some(scope));
        }
        assert_eq!(BlockScope::parse("unknown"), None);
    }

    #[test]
    fn self_not_blocked() {
        let a = AccountId::new();
        assert!(!blocked_between(&a, &a, BlockScope::All, true, true, true));
    }

    #[test]
    fn all_blocks_everything() {
        let a = AccountId::new();
        let b = AccountId::new();
        assert!(blocked_between(&a, &b, BlockScope::All, true, false, false));
        assert!(blocked_between(
            &a,
            &b,
            BlockScope::Messages,
            true,
            false,
            false
        ));
        assert!(blocked_between(
            &a,
            &b,
            BlockScope::Comments,
            true,
            false,
            false
        ));
    }

    #[test]
    fn messages_only_blocks_messages() {
        let a = AccountId::new();
        let b = AccountId::new();
        assert!(!blocked_between(
            &a,
            &b,
            BlockScope::All,
            false,
            true,
            false
        ));
        assert!(blocked_between(
            &a,
            &b,
            BlockScope::Messages,
            false,
            true,
            false
        ));
        assert!(!blocked_between(
            &a,
            &b,
            BlockScope::Comments,
            false,
            true,
            false
        ));
    }

    #[test]
    fn comments_only_blocks_comments() {
        let a = AccountId::new();
        let b = AccountId::new();
        assert!(!blocked_between(
            &a,
            &b,
            BlockScope::All,
            false,
            false,
            true
        ));
        assert!(!blocked_between(
            &a,
            &b,
            BlockScope::Messages,
            false,
            false,
            true
        ));
        assert!(blocked_between(
            &a,
            &b,
            BlockScope::Comments,
            false,
            false,
            true
        ));
    }

    #[test]
    fn not_blocked_without_flag() {
        let a = AccountId::new();
        let b = AccountId::new();
        assert!(!blocked_between(
            &a,
            &b,
            BlockScope::All,
            false,
            false,
            false
        ));
    }

    #[test]
    fn mute_check() {
        let a = AccountId::new();
        let b = AccountId::new();
        assert!(muted_between(&a, &b, true));
        assert!(!muted_between(&a, &b, false));
        assert!(!muted_between(&a, &a, true));
    }
}
