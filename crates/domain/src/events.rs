//! Domain logic for collections, challenges, exchanges, wishlists and events.
//!
//! Spec §18. Pure functions — no I/O — mirroring the discipline of
//! `positivity.rs` and `policy.rs`. The rule layer decides *what* is permitted
//! and *why*; the repository layer (`crates/db/src/events.rs`) persists it.
//!
//! The core idea of this milestone is **composition**: a challenge entry is a
//! work, a wishlist fulfilment is an import or a write, and a collection item
//! is just a work attached to a curator. The rules here reuse the writing,
//! positivity and identity machinery from earlier milestones rather than
//! inventing parallel flows.

use crate::ids::AccountId;

// ---------------------------------------------------------------------------
// Collections
// ---------------------------------------------------------------------------

/// How a collection admits items (spec §18.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemPolicy {
    /// Only the owner adds items — an editorial / reading list.
    OwnerOnly,
    /// Anyone may propose; the owner approves.
    Moderated,
    /// Anyone may add directly; no approval step.
    Open,
}

impl ItemPolicy {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OwnerOnly => "owner_only",
            Self::Moderated => "moderated",
            Self::Open => "open",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "owner_only" => Self::OwnerOnly,
            "moderated" => Self::Moderated,
            "open" => Self::Open,
            _ => return None,
        })
    }
}

/// Whether a caller may add an item to a collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionAction {
    /// May add directly.
    CanAdd,
    /// May propose, pending owner approval.
    CanPropose,
    /// May not add.
    CannotAdd,
}

/// Decide whether `actor` may add an item to a collection (spec §18.1).
///
/// `actor_is_owner` is the only fact that matters for `owner_only`; for
/// `moderated`, any non-owner may propose; for `open`, anyone may add.
#[must_use]
pub fn collection_add_permission(policy: ItemPolicy, actor_is_owner: bool) -> CollectionAction {
    match policy {
        ItemPolicy::OwnerOnly => {
            if actor_is_owner {
                CollectionAction::CanAdd
            } else {
                CollectionAction::CannotAdd
            }
        }
        ItemPolicy::Moderated => {
            if actor_is_owner {
                CollectionAction::CanAdd
            } else {
                CollectionAction::CanPropose
            }
        }
        ItemPolicy::Open => CollectionAction::CanAdd,
    }
}

/// Whether a non-member may confirm a collection's existence (spec §18.1).
///
/// A private collection returns the same 404 as a nonexistent one to anyone
/// who is not a member or the owner.
#[must_use]
pub fn collection_visible(is_public: bool, actor_is_owner: bool, actor_is_member: bool) -> bool {
    is_public || actor_is_owner || actor_is_member
}

// ---------------------------------------------------------------------------
// Challenges
// ---------------------------------------------------------------------------

/// A single constraint on a challenge entry (spec §18.2).
///
/// Constraints are data, never code: evaluation is a pure match over `kind`
/// against facts gathered about the work. No constraint type may execute
/// arbitrary code.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Constraint {
    /// Discriminator: `min_words`, `max_words`, `requires_tag`,
    /// `forbids_tag`, `fandom`, `max_rating`.
    pub kind: String,
    /// Constraint parameters.
    pub params: serde_json::Value,
}

/// The result of evaluating a single constraint.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConstraintResult {
    pub kind: String,
    pub params: serde_json::Value,
    pub passed: bool,
}

/// Facts about a work needed to evaluate constraints (spec §18.2).
#[derive(Debug, Clone, Default)]
pub struct WorkFacts {
    pub word_count: Option<i64>,
    pub tags: Vec<String>,
    pub fandom: Option<String>,
    pub rating: Option<String>,
}

/// Evaluate one constraint against one work's facts.
///
/// Returns `Ok(true)` / `Ok(false)` for pass/fail; `Err` only for a malformed
/// constraint (which should never happen if the domain constructed it).
#[must_use]
pub fn evaluate_constraint(constraint: &Constraint, facts: &WorkFacts) -> bool {
    match constraint.kind.as_str() {
        "min_words" => {
            let target = constraint.params["value"].as_i64().unwrap_or(0);
            facts.word_count.unwrap_or(0) >= target
        }
        "max_words" => {
            let target = constraint.params["value"].as_i64().unwrap_or(i64::MAX);
            facts.word_count.unwrap_or(0) <= target
        }
        "requires_tag" => {
            let tag = constraint.params["value"].as_str().unwrap_or("");
            facts.tags.iter().any(|t| t == tag)
        }
        "forbids_tag" => {
            let tag = constraint.params["value"].as_str().unwrap_or("");
            !facts.tags.iter().any(|t| t == tag)
        }
        "fandom" => {
            let target = constraint.params["value"].as_str().unwrap_or("");
            facts.fandom.as_deref() == Some(target)
        }
        "max_rating" => {
            let target = constraint.params["value"].as_str().unwrap_or("general");
            // Rating order: general < teen < mature < explicit
            let order = |r: &str| match r {
                "general" => 0,
                "teen" => 1,
                "mature" => 2,
                "explicit" => 3,
                _ => 0,
            };
            order(facts.rating.as_deref().unwrap_or("general")) <= order(target)
        }
        _ => false,
    }
}

/// Evaluate all constraints at once, producing a result list (spec §18.2).
#[must_use]
pub fn evaluate_constraints(
    constraints: &[Constraint],
    facts: &WorkFacts,
) -> Vec<ConstraintResult> {
    constraints
        .iter()
        .map(|c| {
            let passed = evaluate_constraint(c, facts);
            ConstraintResult {
                kind: c.kind.clone(),
                params: c.params.clone(),
                passed,
            }
        })
        .collect()
}

/// Whether an entry window allows new entries (spec §18.2).
///
/// `now` is a comparable RFC 3339 timestamp string (lexicographic comparison
/// works for same-offset RFC 3339).
#[must_use]
pub fn entry_window_open(now: &str, opens: &str, closes: &str) -> bool {
    now >= opens && now <= closes
}

// ---------------------------------------------------------------------------
// Requests / Exchanges (claims, anonymity, fulfilment)
// ---------------------------------------------------------------------------

/// The state of a request/claim (spec §18.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestState {
    /// No claim yet.
    Open,
    /// Claimed but not yet fulfilled.
    Claimed,
    /// Claimed and fulfilled with a work.
    Fulfilled,
    /// Claim lapsed without fulfilment.
    Expired,
}

impl RequestState {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Claimed => "claimed",
            Self::Fulfilled => "fulfilled",
            Self::Expired => "expired",
        }
    }
}

/// Whether a claimant's reveal is needed before the requester sees them
/// (spec §18.3).
#[must_use]
pub fn request_is_anonymous(now: &str, anonym_until: Option<&str>) -> bool {
    match anonym_until {
        None => false,
        Some(until) => now < until,
    }
}

// ---------------------------------------------------------------------------
// Wishlist
// ---------------------------------------------------------------------------

/// Whether a wishlist is publicly readable (spec §18.4).
#[must_use]
pub fn wishlist_visible(is_public: bool, viewer: Option<&AccountId>, owner: &AccountId) -> bool {
    is_public || viewer.map(|v| v == owner).unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Whether an account may join an event (spec §18.5).
///
/// Currently this is always yes for signed-in accounts; the restriction point
/// exists so future per-event gating (e.g. invite-only) lands here.
#[must_use]
pub fn may_join_event(account: Option<&AccountId>, _document: &str) -> bool {
    account.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::AccountId;

    // --- collections ---

    #[test]
    fn owner_can_add_to_owner_only_collection() {
        assert_eq!(
            collection_add_permission(ItemPolicy::OwnerOnly, true),
            CollectionAction::CanAdd
        );
    }

    #[test]
    fn non_owner_cannot_add_to_owner_only_collection() {
        assert_eq!(
            collection_add_permission(ItemPolicy::OwnerOnly, false),
            CollectionAction::CannotAdd
        );
    }

    #[test]
    fn moderated_lets_non_owner_propose() {
        assert_eq!(
            collection_add_permission(ItemPolicy::Moderated, false),
            CollectionAction::CanPropose
        );
    }

    #[test]
    fn open_lets_anyone_add() {
        assert_eq!(
            collection_add_permission(ItemPolicy::Open, false),
            CollectionAction::CanAdd
        );
        assert_eq!(
            collection_add_permission(ItemPolicy::Open, true),
            CollectionAction::CanAdd
        );
    }

    #[test]
    fn private_collection_invisible_to_non_member() {
        let owner = AccountId::new();
        let stranger = AccountId::new();
        assert!(!collection_visible(false, false, false));
        // Owner always sees it.
        assert!(collection_visible(false, true, false));
        // Member sees it.
        assert!(collection_visible(false, false, true));
        let _ = (owner, stranger);
    }

    // --- challenges ---

    #[test]
    fn min_words_passes() {
        let c = Constraint {
            kind: "min_words".into(),
            params: serde_json::json!({ "value": 100 }),
        };
        let facts = WorkFacts {
            word_count: Some(150),
            ..Default::default()
        };
        assert!(evaluate_constraint(&c, &facts));
    }

    #[test]
    fn min_words_fails() {
        let c = Constraint {
            kind: "min_words".into(),
            params: serde_json::json!({ "value": 100 }),
        };
        let facts = WorkFacts {
            word_count: Some(50),
            ..Default::default()
        };
        assert!(!evaluate_constraint(&c, &facts));
    }

    #[test]
    fn requires_tag_passes_when_present() {
        let c = Constraint {
            kind: "requires_tag".into(),
            params: serde_json::json!({ "value": "fluff" }),
        };
        let facts = WorkFacts {
            tags: vec!["fluff".into(), "angst".into()],
            ..Default::default()
        };
        assert!(evaluate_constraint(&c, &facts));
    }

    #[test]
    fn requires_tag_fails_when_absent() {
        let c = Constraint {
            kind: "requires_tag".into(),
            params: serde_json::json!({ "value": "fluff" }),
        };
        let facts = WorkFacts {
            tags: vec!["angst".into()],
            ..Default::default()
        };
        assert!(!evaluate_constraint(&c, &facts));
    }

    #[test]
    fn entry_window_rejects_before_open() {
        assert!(!entry_window_open(
            "2024-01-01T00:00:00Z",
            "2024-01-02T00:00:00Z",
            "2024-01-03T00:00:00Z"
        ));
    }

    #[test]
    fn entry_window_allows_inside() {
        assert!(entry_window_open(
            "2024-01-02T12:00:00Z",
            "2024-01-02T00:00:00Z",
            "2024-01-03T00:00:00Z"
        ));
    }

    #[test]
    fn entry_window_rejects_after_close() {
        assert!(!entry_window_open(
            "2024-01-04T00:00:00Z",
            "2024-01-02T00:00:00Z",
            "2024-01-03T00:00:00Z"
        ));
    }

    #[test]
    fn unknown_constraint_kind_fails_safely() {
        let c = Constraint {
            kind: "unknown".into(),
            params: serde_json::json!({}),
        };
        let facts = WorkFacts::default();
        assert!(!evaluate_constraint(&c, &facts));
    }

    // --- requests / exchanges ---

    #[test]
    fn request_anonymous_inside_window() {
        assert!(request_is_anonymous(
            "2024-01-01T00:00:00Z",
            Some("2024-01-02T00:00:00Z")
        ));
    }

    #[test]
    fn request_revealed_after_window() {
        assert!(!request_is_anonymous(
            "2024-01-03T00:00:00Z",
            Some("2024-01-02T00:00:00Z")
        ));
    }

    #[test]
    fn no_anon_window_means_always_revealed() {
        assert!(!request_is_anonymous("2024-01-01T00:00:00Z", None));
    }

    // --- wishlist ---

    #[test]
    fn private_wishlist_visible_only_to_owner() {
        let owner = AccountId::new();
        let other = AccountId::new();
        assert!(!wishlist_visible(false, Some(&other), &owner));
        assert!(wishlist_visible(false, Some(&owner), &owner));
        assert!(wishlist_visible(true, Some(&other), &owner));
        // Public means public: a stranger who is not signed in can read it.
        assert!(wishlist_visible(true, None, &owner));
    }
}
