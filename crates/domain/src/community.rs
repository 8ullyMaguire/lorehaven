//! Community domain rules: threads, edit windows, soft-delete semantics,
//! posting rules, and the group visibility matrix.
//!
//! Spec §17.1, §17.4, §7.2.2.

use crate::ids::AccountId;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------

/// Maximum nesting depth before replies are flattened.
pub const MAX_COMMENT_DEPTH: usize = 5;

/// Seconds after creation during which the author may still edit their comment.
pub const COMMENT_EDIT_WINDOW_SECS: u64 = 900; // 15 minutes

/// The life-cycle state of a comment for a given viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CommentState {
    /// Visible in full.
    Visible,
    /// The viewer chose to see their own soft-deleted comment.
    TombstoneForAuthor,
    /// Soft-deleted: body hidden for everyone except the author.
    TombstoneForOthers,
    /// Held by the positivity gate (only the author sees it).
    Held,
}

impl CommentState {
    pub fn is_visible(self) -> bool {
        matches!(self, Self::Visible)
    }
}

/// Decide whether a comment may still be edited by its author.
///
/// The edit window is a fixed duration from creation; once it closes the
/// comment is immutable so the conversation cannot be silently rewritten.
pub fn can_edit_comment(created_at_epoch_secs: i64, now_epoch_secs: i64) -> bool {
    now_epoch_secs.saturating_sub(created_at_epoch_secs) < COMMENT_EDIT_WINDOW_SECS as i64
}

// ---------------------------------------------------------------------------
// Groups
// ---------------------------------------------------------------------------

/// The privacy setting of a group (spec §17.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupPrivacy {
    /// Anyone can see, join, and read.
    Open,
    /// Anyone can see and request; joining needs approval.
    Closed,
    /// Only members can see the group at all.
    Hidden,
}

impl GroupPrivacy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Hidden => "hidden",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "open" => Self::Open,
            "closed" => Self::Closed,
            "hidden" => Self::Hidden,
            _ => return None,
        })
    }
}

/// The role of an account within a group (spec §17.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupRole {
    Owner,
    Moderator,
    Member,
}

impl GroupRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Moderator => "moderator",
            Self::Member => "member",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "owner" => Self::Owner,
            "moderator" => Self::Moderator,
            "member" => Self::Member,
            _ => return None,
        })
    }
}

/// Who may list a group in the directory.
pub fn group_can_list(privacy: GroupPrivacy) -> bool {
    match privacy {
        GroupPrivacy::Open | GroupPrivacy::Closed => true,
        GroupPrivacy::Hidden => false,
    }
}

/// Who may join a group.
pub fn group_can_join(privacy: GroupPrivacy) -> bool {
    match privacy {
        GroupPrivacy::Open | GroupPrivacy::Closed => true,
        GroupPrivacy::Hidden => false, // hidden groups are invite-only
    }
}

/// Who may post to a group's forum.
pub fn group_can_post(_privacy: GroupPrivacy, role: Option<GroupRole>) -> bool {
    role.is_some() // members, moderators, and owners may post
}

/// Who may moderate a group (lock topics, approve members).
pub fn group_can_moderate(role: Option<GroupRole>) -> bool {
    matches!(role, Some(GroupRole::Owner) | Some(GroupRole::Moderator))
}

// ---------------------------------------------------------------------------
// Posting rules
// ---------------------------------------------------------------------------

/// Decide whether a sender may post a comment on a subject.
///
/// The rule is: the sender must be a pseud (not an anonymous visitor) and
/// must not be blocked by the subject's owner in the comments scope.
pub fn can_post_comment(
    sender_pseud: &str,
    subject_owner: &AccountId,
    sender_blocks_owner_comments: bool,
    owner_blocks_sender_comments: bool,
) -> Result<(), PostBlock> {
    if sender_pseud.is_empty() {
        return Err(PostBlock::NoPseud);
    }
    if sender_blocks_owner_comments {
        return Err(PostBlock::YouHaveBlocked);
    }
    if owner_blocks_sender_comments {
        return Err(PostBlock::YouAreBlocked);
    }
    let _ = subject_owner;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostBlock {
    NoPseud,
    YouHaveBlocked,
    YouAreBlocked,
}

// ---------------------------------------------------------------------------
// Presence
// ---------------------------------------------------------------------------

/// Seconds before a typing indicator expires if not refreshed.
pub const TYPING_TIMEOUT_SECS: u64 = 10;

/// Presence granularity: we expose "active now" only.
#[derive(Debug, Clone, Serialize)]
pub struct PresenceView {
    pub active_now: bool,
    pub typing: bool,
}

/// Decide whether a viewer may see a subject's presence.
///
/// Presence is opt-in per-pseud, never leaks across pseuds, and never
/// reaches a blocked or muted user.
pub fn presence_visible_to(
    subject_presence_enabled: bool,
    viewer_is_blocked: bool,
    viewer_is_muted: bool,
) -> bool {
    if !subject_presence_enabled {
        return false;
    }
    if viewer_is_blocked {
        return false;
    }
    if viewer_is_muted {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_privacy_round_trips() {
        for p in [
            GroupPrivacy::Open,
            GroupPrivacy::Closed,
            GroupPrivacy::Hidden,
        ] {
            assert_eq!(GroupPrivacy::parse(p.as_str()), Some(p));
        }
        assert_eq!(GroupPrivacy::parse("secret"), None);
    }

    #[test]
    fn group_role_round_trips() {
        for r in [GroupRole::Owner, GroupRole::Moderator, GroupRole::Member] {
            assert_eq!(GroupRole::parse(r.as_str()), Some(r));
        }
        assert_eq!(GroupRole::parse("admin"), None);
    }

    #[test]
    fn group_visibility_matrix() {
        // open: listable, joinable, postable for members, not moderatable for members
        assert!(group_can_list(GroupPrivacy::Open));
        assert!(group_can_join(GroupPrivacy::Open));
        assert!(group_can_post(GroupPrivacy::Open, Some(GroupRole::Member)));
        assert!(!group_can_moderate(Some(GroupRole::Member)));
        // closed: listable, joinable (request), postable for members
        assert!(group_can_list(GroupPrivacy::Closed));
        assert!(group_can_join(GroupPrivacy::Closed));
        assert!(group_can_post(
            GroupPrivacy::Closed,
            Some(GroupRole::Member)
        ));
        // hidden: not listable, not joinable, non-members cannot post
        assert!(!group_can_list(GroupPrivacy::Hidden));
        assert!(!group_can_join(GroupPrivacy::Hidden));
        assert!(!group_can_post(GroupPrivacy::Hidden, None));
        assert!(group_can_post(
            GroupPrivacy::Hidden,
            Some(GroupRole::Member)
        ));
        // moderator can moderate
        assert!(group_can_moderate(Some(GroupRole::Moderator)));
        assert!(group_can_moderate(Some(GroupRole::Owner)));
    }

    #[test]
    fn comment_edit_window() {
        assert!(can_edit_comment(1000, 1000)); // just created
        assert!(can_edit_comment(1000, 1899)); // 14m59s
        assert!(!can_edit_comment(1000, 1900)); // 15m00s — closed
        assert!(!can_edit_comment(1000, 2000)); // way past
    }

    #[test]
    fn comment_posting_rules() {
        let subject_owner = AccountId::from_uuid(uuid::Uuid::new_v4());
        // Empty pseud blocked
        assert_eq!(
            can_post_comment("", &subject_owner, false, false),
            Err(PostBlock::NoPseud)
        );
        // Blocked by owner → sender sees "you are blocked"
        assert_eq!(
            can_post_comment("alice", &subject_owner, false, true),
            Err(PostBlock::YouAreBlocked)
        );
        // Sender blocked owner → "you have blocked"
        assert_eq!(
            can_post_comment("alice", &subject_owner, true, false),
            Err(PostBlock::YouHaveBlocked)
        );
        // Clean
        assert!(can_post_comment("alice", &subject_owner, false, false).is_ok());
    }

    #[test]
    fn presence_rules() {
        // Opt-in: off → never visible
        assert!(!presence_visible_to(false, false, false));
        // Blocked → never visible
        assert!(!presence_visible_to(true, true, false));
        // Muted → never visible
        assert!(!presence_visible_to(true, false, true));
        // All clear → visible
        assert!(presence_visible_to(true, false, false));
    }
}
