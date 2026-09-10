//! Resource policies.
//!
//! Spec §3.6: authorization lives in policy functions, never in scattered
//! handlers or frontend checks. Spec §7 additionally requires *one* shared
//! content-eligibility service used by the reader, search, downloads, feeds,
//! notifications, recommendations, APIs and extensions alike — a second,
//! subtly different check is exactly how restricted content leaks.
//!
//! Everything here is pure: no database, no clock, no I/O. Callers load facts
//! and hand them in. That makes every rule unit-testable and keeps the
//! enforcement point identical wherever it is called from.

/// How widely a work is exposed (spec §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    /// Listed and reachable.
    Public,
    /// Reachable by URL, not listed. Explicitly *not* secret.
    Unlisted,
    /// Registered users only.
    Restricted,
}

impl Visibility {
    /// Whether a search index or listing surface may include this work.
    ///
    /// Unlisted works are excluded from listings by definition; that is the
    /// whole point of the state, and it is enforced here rather than by
    /// remembering to filter in each query.
    #[must_use]
    pub const fn is_listable(self) -> bool {
        matches!(self, Self::Public)
    }
}

/// A work's editorial state (spec §8), separate from completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    /// Private to its contributors.
    Draft,
    /// Published, with a future publication time.
    Scheduled,
    /// Live.
    Published,
    /// Previously published, retracted.
    Withdrawn,
    /// Soft-deleted.
    Deleted,
}

impl Lifecycle {
    /// Whether non-contributors may ever read this.
    #[must_use]
    pub const fn is_publicly_readable(self) -> bool {
        matches!(self, Self::Published)
    }
}

/// How finished a work is (spec §8) — orthogonal to lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Completion {
    /// Still being written.
    InProgress,
    /// Finished.
    Complete,
    /// Paused by the author.
    Hiatus,
    /// Abandoned.
    Abandoned,
}

/// Content rating.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ContentRating {
    /// All audiences.
    General,
    /// Teen.
    Teen,
    /// Mature.
    Mature,
    /// Explicit.
    Explicit,
}

/// Age-policy state machine (spec §7).
///
/// The critical property: `DeclaredAdult` is *not* `VerifiedAdult`. A
/// self-declaration is a claim, not an assurance, and the platform must not
/// pretend otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgeState {
    /// We have not been told anything.
    Unknown,
    /// The user states they are below the threshold.
    DeclaredMinor,
    /// The user states they are an adult. Unverified.
    DeclaredAdult,
    /// Below threshold and awaiting guardian authorization.
    AuthorizationRequired,
    /// Below threshold, authorization accepted under the active policy.
    AuthorizedUnderPolicy,
    /// Policy forbids participation; the account is limited to anonymous reads.
    Restricted,
}

impl AgeState {
    /// Whether this state permits registering an account that writes.
    #[must_use]
    pub const fn may_participate(self) -> bool {
        matches!(self, Self::DeclaredAdult | Self::AuthorizedUnderPolicy)
    }

    /// Whether this state is a minor state.
    ///
    /// `Restricted` belongs here. It means "we were told this person is below
    /// the threshold and no authorization workflow exists", which is a minor
    /// state by construction — omitting it would silently give a restricted
    /// account the adult defaults for messaging and taste learning.
    #[must_use]
    pub const fn is_minor(self) -> bool {
        matches!(
            self,
            Self::DeclaredMinor
                | Self::AuthorizationRequired
                | Self::AuthorizedUnderPolicy
                | Self::Restricted
        )
    }
}

/// The effective content policy an instance runs under.
///
/// These are per-instance settings, so the eligibility service takes them as
/// data rather than hard-coding a jurisdiction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccessPolicy {
    /// Highest rating an anonymous visitor may read.
    pub anonymous_max_rating: ContentRating,
    /// Highest rating an unknown-age account may read.
    pub unknown_age_max_rating: ContentRating,
    /// Highest rating a declared minor may read.
    pub minor_max_rating: ContentRating,
    /// Highest rating a declared adult may read.
    pub adult_max_rating: ContentRating,
    /// Whether signed-out visitors may read public works at all.
    ///
    /// Spec §7 requires anonymous reading of suitable public fiction to remain
    /// available, so the default is `true`.
    pub anonymous_reading_enabled: bool,
}

impl Default for AccessPolicy {
    fn default() -> Self {
        Self {
            anonymous_max_rating: ContentRating::Teen,
            unknown_age_max_rating: ContentRating::Teen,
            minor_max_rating: ContentRating::General,
            adult_max_rating: ContentRating::Explicit,
            anonymous_reading_enabled: true,
        }
    }
}

/// Who is asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Actor {
    /// The account behind the request.
    pub account_id: crate::ids::AccountId,
    /// The pseud currently active for this request.
    pub pseud_id: crate::ids::PseudId,
    /// The actor's age-policy state.
    pub age_state: AgeState,
    /// Whether the actor is explicitly authorized to view restricted content
    /// by an instance-level grant (administrator, moderator review context).
    pub trusted_reviewer: bool,
}

/// The outcome of a policy evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Permitted.
    Allow,
    /// Refused, with a coarse reason safe to log.
    Deny(DenyReason),
}

impl Decision {
    /// Whether this decision permits the operation.
    #[must_use]
    pub const fn is_allowed(self) -> bool {
        matches!(self, Self::Allow)
    }
}

/// Why access was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DenyReason {
    /// No session.
    NotAuthenticated,
    /// Anonymous reading is disabled instance-wide.
    AnonymousReadingDisabled,
    /// The work is not published.
    NotPublished,
    /// The work is restricted to signed-in readers.
    SignInRequired,
    /// The rating exceeds what this actor may see.
    RatingExceedsPolicy,
    /// The author blocked or muted the actor.
    BlockedByAuthor,
}

impl DenyReason {
    /// A coarse, loggable description.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotAuthenticated => "not_authenticated",
            Self::AnonymousReadingDisabled => "anonymous_reading_disabled",
            Self::NotPublished => "not_published",
            Self::SignInRequired => "sign_in_required",
            Self::RatingExceedsPolicy => "rating_exceeds_policy",
            Self::BlockedByAuthor => "blocked_by_author",
        }
    }
}

/// The facts needed to decide whether an actor may read a work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentFacts {
    /// The work's lifecycle state.
    pub lifecycle: Lifecycle,
    /// The work's visibility.
    pub visibility: Visibility,
    /// The work's rating.
    pub rating: ContentRating,
    /// Whether the actor is a contributor (author/coauthor) of the work.
    pub actor_is_contributor: bool,
    /// Whether the author has blocked the actor.
    pub author_blocked_actor: bool,
    /// Whether the actor asked for a deep link to an unlisted work.
    pub via_deep_link: bool,
}

/// The single content-eligibility service (spec §7).
///
/// Every surface — reader, search, downloads, feeds, notifications,
/// recommendations, APIs, extensions — must call this and honor the answer.
#[must_use]
pub fn can_access_content(
    actor: Option<&Actor>,
    facts: &ContentFacts,
    policy: &AccessPolicy,
) -> Decision {
    use DenyReason as Deny;

    // Contributors always reach their own work, including drafts.
    if facts.actor_is_contributor {
        return Decision::Allow;
    }

    if facts.author_blocked_actor {
        return Decision::Deny(Deny::BlockedByAuthor);
    }

    // Trusted reviewers may inspect content in a review context regardless of
    // lifecycle, but this is an explicit grant — not an inference from a role.
    if actor.is_some_and(|a| a.trusted_reviewer) {
        return Decision::Allow;
    }

    if !facts.lifecycle.is_publicly_readable() {
        return Decision::Deny(Deny::NotPublished);
    }

    // Unlisted is readable by URL; the deep-link flag documents that the caller
    // knows it resolved a direct link rather than a listing.
    let _ = facts.via_deep_link;

    let Some(actor) = actor else {
        if !policy.anonymous_reading_enabled {
            return Decision::Deny(Deny::AnonymousReadingDisabled);
        }
        if facts.visibility == Visibility::Restricted {
            return Decision::Deny(Deny::SignInRequired);
        }
        return if facts.rating <= policy.anonymous_max_rating {
            Decision::Allow
        } else {
            Decision::Deny(Deny::RatingExceedsPolicy)
        };
    };

    let ceiling = match actor.age_state {
        AgeState::DeclaredAdult => policy.adult_max_rating,
        AgeState::AuthorizedUnderPolicy => policy.minor_max_rating,
        AgeState::DeclaredMinor | AgeState::AuthorizationRequired => policy.minor_max_rating,
        AgeState::Unknown => policy.unknown_age_max_rating,
        AgeState::Restricted => policy.minor_max_rating,
    };

    if facts.rating > ceiling {
        return Decision::Deny(Deny::RatingExceedsPolicy);
    }

    Decision::Allow
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{AccountId, PseudId};

    fn adult() -> Actor {
        Actor {
            account_id: AccountId::new(),
            pseud_id: PseudId::new(),
            age_state: AgeState::DeclaredAdult,
            trusted_reviewer: false,
        }
    }

    fn published(rating: ContentRating, visibility: Visibility) -> ContentFacts {
        ContentFacts {
            lifecycle: Lifecycle::Published,
            visibility,
            rating,
            actor_is_contributor: false,
            author_blocked_actor: false,
            via_deep_link: false,
        }
    }

    #[test]
    fn drafts_are_invisible_to_everyone_but_contributors() {
        let facts = ContentFacts {
            lifecycle: Lifecycle::Draft,
            ..published(ContentRating::General, Visibility::Public)
        };
        assert_eq!(
            can_access_content(Some(&adult()), &facts, &AccessPolicy::default()),
            Decision::Deny(DenyReason::NotPublished)
        );
        let own = ContentFacts {
            actor_is_contributor: true,
            ..facts
        };
        assert!(can_access_content(Some(&adult()), &own, &AccessPolicy::default()).is_allowed());
    }

    #[test]
    fn explicit_content_is_refused_to_an_unknown_age_actor() {
        let facts = published(ContentRating::Explicit, Visibility::Public);
        let mut actor = adult();
        actor.age_state = AgeState::Unknown;
        assert_eq!(
            can_access_content(Some(&actor), &facts, &AccessPolicy::default()),
            Decision::Deny(DenyReason::RatingExceedsPolicy)
        );
    }

    #[test]
    fn restricted_works_require_a_session() {
        let facts = published(ContentRating::General, Visibility::Restricted);
        assert_eq!(
            can_access_content(None, &facts, &AccessPolicy::default()),
            Decision::Deny(DenyReason::SignInRequired)
        );
        assert!(can_access_content(Some(&adult()), &facts, &AccessPolicy::default()).is_allowed());
    }

    #[test]
    fn anonymous_reading_can_be_disabled_instance_wide() {
        let facts = published(ContentRating::General, Visibility::Public);
        let policy = AccessPolicy {
            anonymous_reading_enabled: false,
            ..AccessPolicy::default()
        };
        assert_eq!(
            can_access_content(None, &facts, &policy),
            Decision::Deny(DenyReason::AnonymousReadingDisabled)
        );
    }

    #[test]
    fn a_blocked_actor_is_refused_even_for_public_content() {
        let facts = ContentFacts {
            author_blocked_actor: true,
            ..published(ContentRating::General, Visibility::Public)
        };
        assert_eq!(
            can_access_content(Some(&adult()), &facts, &AccessPolicy::default()),
            Decision::Deny(DenyReason::BlockedByAuthor)
        );
    }

    #[test]
    fn declared_minors_see_less_than_declared_adults() {
        let explicit = published(ContentRating::Explicit, Visibility::Public);
        let mut minor = adult();
        minor.age_state = AgeState::DeclaredMinor;
        assert_eq!(
            can_access_content(Some(&minor), &explicit, &AccessPolicy::default()),
            Decision::Deny(DenyReason::RatingExceedsPolicy)
        );
        assert!(
            can_access_content(Some(&adult()), &explicit, &AccessPolicy::default()).is_allowed()
        );
    }

    #[test]
    fn a_declared_adult_is_not_treated_as_verified() {
        // The state machine keeps the distinction; this test pins it so that a
        // future refactor cannot quietly merge the two states.
        assert_ne!(AgeState::DeclaredAdult, AgeState::AuthorizedUnderPolicy);
        assert!(AgeState::AuthorizationRequired.is_minor());
        assert!(!AgeState::DeclaredAdult.is_minor());
    }

    #[test]
    fn a_restricted_account_is_a_minor_state() {
        // Restricted means "below the threshold, with no workflow to authorize
        // it". Treating it as anything other than a minor state would hand a
        // child the adult defaults for messaging and discovery.
        assert!(AgeState::Restricted.is_minor());
        assert!(!AgeState::Restricted.may_participate());
    }
}
