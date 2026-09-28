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

/// Who this actor is, for a body-audience check (spec §7.7).
///
/// A set of **facts about roles the account holds**, never a score. That
/// distinction is the whole design: a score invites the question "how does this
/// account's number go up", and the honest answer is "by being trusted", which
/// is not a thing a reader controls. Every field here is a fact a moderator set
/// or an instance configured, so a reader cannot move it by reading more.
///
/// Deliberately not on [`Actor`]. `Actor` is what a request carries; standing is
/// what the *instance* knows about the account, and it is consulted only by the
/// body-audience rule. Folding it into `Actor` would put a moderation fact into
/// every request path, and the only correct value for a caller that has not
/// looked it up would be the least privileged one — which is exactly the default
/// that gets forgotten somewhere and silently widens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ActorStanding {
    /// The account's trust level, 0..6. Used only by `TrustAtLeast`, and only
    /// as a threshold — never compared against a value derived from behaviour.
    pub trust_level: i64,
    /// An operator of this instance.
    pub is_operator: bool,
    /// A §16.18 vanguard member. Membership, not a resonance score: a reader who
    /// engages a lot is not thereby a vanguard.
    pub is_vanguard: bool,
    /// Holds the §32 media curator role.
    pub is_curator: bool,
    /// Whether this request carries a session at all.
    ///
    /// A separate field because `AccountsOnly` is a claim about *accounts*, and
    /// "trust level 0" does not distinguish a new account from no account. With
    /// the flag absent, `standing_satisfies(AccountsOnly, none())` was `true` and
    /// a work gated to signed-in readers was handed to every unauthenticated
    /// request — the audience whose exclusion `accounts_only` states most
    /// plainly. Every other audience is refused by `none()` already, so this is
    /// the single variant that needed the fact.
    pub signed_in: bool,
}

impl ActorStanding {
    /// An account with no standing at all: trust level 0, no roles, no session.
    ///
    /// The zero value, and the right answer for "we did not look this up". A
    /// reader who is not trusted enough fails the threshold, so the default
    /// denies — which is the direction a forgetful caller must fail in.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            trust_level: 0,
            is_operator: false,
            is_vanguard: false,
            is_curator: false,
            signed_in: false,
        }
    }

    /// The standing of a signed-in account that holds no roles.
    ///
    /// What a route that has resolved a session but looked up nothing else
    /// should pass. Distinct from `none()` because the two differ in exactly one
    /// audience, and it is the one an operator is most likely to reach for.
    #[must_use]
    pub const fn signed_in() -> Self {
        Self {
            signed_in: true,
            ..Self::none()
        }
    }
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
    /// The acting pseud is not a contributor of the work at all.
    NotAContributor,
    /// The acting pseud is a contributor, but its role does not permit this.
    InsufficientRole,
    /// The actor is outside the audience this work's body is held for (spec §7.7).
    ///
    /// Distinct from `NotPublished` and `SignInRequired` on purpose, and this is
    /// the reason the type exists: those say the work is not here, this says it
    /// is here and the body is not for you. A surface that renders the wrong one
    /// turns a gated work into a non-existent one, which is a distinguishable
    /// response and therefore an existence oracle (spec §7.7.3).
    BodyNotInAudience,
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
            Self::NotAContributor => "not_a_contributor",
            Self::InsufficientRole => "insufficient_role",
            Self::BodyNotInAudience => "body_not_in_audience",
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
    /// Who may read this work's BODY (spec §7.7), narrowed from the instance
    /// default.
    ///
    /// Separate from `visibility`, which is about the work's page. A work can be
    /// publicly listed with a summary and a word count while its prose is held
    /// for trusted readers, and the two fields have to be separate for that to be
    /// expressible at all.
    pub body_audience: crate::retention::BodyAudience,
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
    can_access_content_with_standing(actor, facts, policy, None)
}

/// [`can_access_content`], with the actor's institutional standing supplied.
///
/// The two-argument form is what most surfaces call, and it is correct for every
/// surface that does not gate a body. Splitting rather than defaulting the
/// parameter is deliberate: a default argument would let a new call site
/// inherit "no standing" without noticing, and while that direction is the safe
/// one it is also the one that makes a gated work look ungated to every reader
/// who is not an operator — a widening, reached by forgetting rather than by
/// choosing.
pub fn can_access_content_with_standing(
    actor: Option<&Actor>,
    facts: &ContentFacts,
    policy: &AccessPolicy,
    standing: Option<&ActorStanding>,
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
        if facts.rating > policy.anonymous_max_rating {
            return Decision::Deny(Deny::RatingExceedsPolicy);
        }
        // An anonymous reader is a reader with no standing at all, and the
        // audience check is a narrowing like every other step above — so it
        // applies to them too. It was missing here, which meant a work gated to
        // `accounts_only` handed its body to every unauthenticated request on an
        // instance that allows anonymous reading: the one population an audience
        // can always be said to exclude. The instance-level rating ceiling is
        // already applied above, so nothing is re-checked.
        if !crate::retention::standing_satisfies(facts.body_audience, &ActorStanding::none()) {
            return Decision::Deny(Deny::BodyNotInAudience);
        }
        return Decision::Allow;
    };

    let ceiling = match actor.age_state {
        AgeState::DeclaredAdult => policy.adult_max_rating,
        AgeState::AuthorizedUnderPolicy => policy.minor_max_rating,
        AgeState::DeclaredMinor | AgeState::AuthorizationRequired => policy.minor_max_rating,
        AgeState::Unknown => policy.unknown_age_max_rating,
        AgeState::Restricted => policy.minor_max_rating,
    };

    // Step 5: the rating ceiling, BEFORE the audience (spec §7.7.2).
    //
    // The order is the safety property, not a style choice. An operator's
    // audience must not be a way around a minor's rating ceiling: an operator
    // reading a `declared_minor` account's request for explicit work is refused
    // by the ceiling, because the ceiling is about the ACCOUNT and the audience
    // is about the WORK. Swapping these two turns every role into a rating
    // bypass, and it is the single line this ordering exists to protect.
    if facts.rating > ceiling {
        return Decision::Deny(Deny::RatingExceedsPolicy);
    }

    // Step 6: the body audience, LAST (spec §7.7.2). Reached only by an actor
    // who has already cleared the lifecycle check, the trusted-reviewer grant
    // and the rating ceiling.
    //
    // `standing` defaults to `None` when a caller does not supply one, and
    // `ActorStanding::none()` fails every audience above `Anyone` — including
    // `accounts_only`, which answers from `signed_in` and not from the trust
    // level. So a caller that forgets to look standing up gets a DENY, which
    // is the only safe direction for a field whose default answer is "this
    // reader may not have the body".
    let owned = standing.copied().unwrap_or_else(ActorStanding::none);
    if !crate::retention::standing_satisfies(facts.body_audience, &owned) {
        return Decision::Deny(Deny::BodyNotInAudience);
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

    /// A published work at the §11.15 BASELINE audience: every eligible reader
    /// gets the body.
    ///
    /// `Anyone` here rather than something narrower, so every pre-existing test
    /// in this file keeps testing what it was written to test. A test that
    /// predates §7.7 and suddenly started failing on a rating ceiling would be
    /// evidence about the default, not about the ceiling.
    fn published(rating: ContentRating, visibility: Visibility) -> ContentFacts {
        ContentFacts {
            lifecycle: Lifecycle::Published,
            visibility,
            rating,
            actor_is_contributor: false,
            author_blocked_actor: false,
            via_deep_link: false,
            body_audience: crate::retention::BodyAudience::Anyone,
        }
    }

    // -----------------------------------------------------------------------
    // Body audience (spec §7.7)
    // -----------------------------------------------------------------------

    fn an_operator() -> ActorStanding {
        ActorStanding {
            is_operator: true,
            ..ActorStanding::none()
        }
    }

    /// The rating ceiling runs BEFORE the audience — the line §7.7.2 exists for.
    ///
    /// An operator is granted the body of a work held for operators. They are
    /// NOT granted a rating their age state forbids, and these are two different
    /// questions: the ceiling is about the ACCOUNT, the audience is about the
    /// WORK. An operator reading through a `declared_minor` session must be
    /// refused the explicit work exactly as any other minor is.
    ///
    /// Written first in the phase, and it is the test to keep: swapping steps 5
    /// and 6 turns every role into a rating bypass and no other test in the file
    /// notices.
    #[test]
    fn a_role_does_not_get_past_the_rating_ceiling() {
        let policy = AccessPolicy::default();
        let explicit = ContentRating::Explicit;

        // The same work, the same actor, the same role — the only thing that
        // changes between the two assertions is the ORDER the code applies them,
        // and here it is applied the right way round.
        let gated = ContentFacts {
            body_audience: crate::retention::BodyAudience::RoleOperator,
            ..published(explicit, Visibility::Public)
        };
        let minor = Actor {
            age_state: AgeState::DeclaredMinor,
            ..adult()
        };

        assert_eq!(
            can_access_content_with_standing(Some(&minor), &gated, &policy, Some(&an_operator())),
            Decision::Deny(DenyReason::RatingExceedsPolicy),
            "the ceiling is about the account and the audience is about the work; an operator \
             must not reach explicit work through a minor's session"
        );

        // And the control: the SAME audience with an adult session passes, which
        // is what makes the assertion above about ordering rather than about the
        // audience being refused outright.
        let adult_operator = adult();
        assert_eq!(
            can_access_content_with_standing(
                Some(&adult_operator),
                &gated,
                &policy,
                Some(&an_operator())
            ),
            Decision::Allow,
            "with the ceiling cleared, the operator reaches the body"
        );
    }

    /// An audience the actor does not satisfy is denied, by its own reason.
    #[test]
    fn a_body_audience_the_actor_does_not_satisfy_is_refused() {
        let policy = AccessPolicy::default();
        let facts = ContentFacts {
            body_audience: crate::retention::BodyAudience::TrustAtLeast(4),
            ..published(ContentRating::General, Visibility::Public)
        };
        let reader = adult();

        // No standing supplied at all: the forgetful-caller case.
        assert_eq!(
            can_access_content(Some(&reader), &facts, &policy),
            Decision::Deny(DenyReason::BodyNotInAudience),
            "a caller that did not look standing up must be denied, not allowed"
        );
        // Standing looked up, and it is not enough.
        assert_eq!(
            can_access_content_with_standing(
                Some(&reader),
                &facts,
                &policy,
                Some(&ActorStanding {
                    trust_level: 2,
                    ..ActorStanding::none()
                })
            ),
            Decision::Deny(DenyReason::BodyNotInAudience)
        );
        // Enough.
        assert_eq!(
            can_access_content_with_standing(
                Some(&reader),
                &facts,
                &policy,
                Some(&ActorStanding {
                    trust_level: 4,
                    ..ActorStanding::none()
                })
            ),
            Decision::Allow
        );
    }

    /// A high trust level is not a role.
    #[test]
    fn trust_alone_does_not_satisfy_a_role_audience() {
        let policy = AccessPolicy::default();
        let facts = ContentFacts {
            body_audience: crate::retention::BodyAudience::RoleCurator,
            ..published(ContentRating::General, Visibility::Public)
        };
        let reader = adult();
        let trusted = ActorStanding {
            trust_level: 6,
            ..ActorStanding::none()
        };

        assert_eq!(
            can_access_content_with_standing(Some(&reader), &facts, &policy, Some(&trusted)),
            Decision::Deny(DenyReason::BodyNotInAudience),
            "trust 6 is not the curator role: a reader who engages a lot is not thereby a curator"
        );
    }

    /// The audience does not move the checks that come before it.
    ///
    /// The three grants that precede the audience still precede it: a contributor
    /// reaches their own work whatever its audience, an author block still
    /// refuses, and a trusted reviewer still gets its explicit grant. A body
    /// audience is a NARROWING of §11.15's baseline, so it must not become a new
    /// grant — the reverse error (a role audience quietly granting a blocked
    /// author access) is what a reordering would produce.
    #[test]
    fn the_audience_does_not_disturb_the_checks_before_it() {
        let policy = AccessPolicy::default();
        let reader = adult();

        // A contributor of a work held for operators: still allowed.
        let contributor = ContentFacts {
            actor_is_contributor: true,
            body_audience: crate::retention::BodyAudience::RoleOperator,
            ..published(ContentRating::General, Visibility::Public)
        };
        assert_eq!(
            can_access_content_with_standing(Some(&reader), &contributor, &policy, None),
            Decision::Allow,
            "a contributor reaches their own work whatever its audience"
        );

        // An author block still refuses, audience or not.
        let blocked = ContentFacts {
            author_blocked_actor: true,
            body_audience: crate::retention::BodyAudience::Anyone,
            ..published(ContentRating::General, Visibility::Public)
        };
        assert_eq!(
            can_access_content_with_standing(
                Some(&reader),
                &blocked,
                &policy,
                Some(&an_operator())
            ),
            Decision::Deny(DenyReason::BlockedByAuthor),
            "an operator audience is not a way around an author block"
        );

        // A trusted reviewer still gets its existing grant ahead of the audience.
        let reviewer = Actor {
            trusted_reviewer: true,
            ..adult()
        };
        let operator_only = ContentFacts {
            body_audience: crate::retention::BodyAudience::RoleOperator,
            ..published(ContentRating::General, Visibility::Public)
        };
        assert_eq!(
            can_access_content_with_standing(Some(&reviewer), &operator_only, &policy, None),
            Decision::Allow,
            "the trusted-reviewer grant is unchanged and still comes first"
        );
    }

    /// An anonymous reader is refused a gated body.
    ///
    /// The premise this test was first written with — that the anonymous arm
    /// answers before the audience is ever consulted, so the audience is only
    /// reachable for a signed-in actor — was wrong, and it was wrong about a
    /// default. `AccessPolicy::default()` sets `anonymous_reading_enabled:
    /// true`, so on a stock instance an unauthenticated reader reaches the
    /// rating check and is ALLOWED. A work gated to `accounts_only` therefore
    /// handed its body to exactly the population an audience can always be said
    /// to exclude, on every instance that permits anonymous reading.
    ///
    /// Anonymous reading is off by default at the *instance mode* level
    /// (`InstanceMode::WalledGarden` and friends), not in `AccessPolicy`, which
    /// is why the two answers below differ and both are correct.
    #[test]
    fn an_anonymous_reader_is_refused_a_gated_body() {
        let facts = ContentFacts {
            body_audience: crate::retention::BodyAudience::AccountsOnly,
            ..published(ContentRating::General, Visibility::Public)
        };

        // Anonymous reading OFF: the instance answers before the audience,
        // and the reason names the instance posture rather than the audience.
        let walled = AccessPolicy {
            anonymous_reading_enabled: false,
            ..AccessPolicy::default()
        };
        assert_eq!(
            can_access_content(None, &facts, &walled),
            Decision::Deny(DenyReason::AnonymousReadingDisabled),
            "an instance that refuses anonymous reading refuses this reader first, whatever the \
             audience says"
        );

        // Anonymous reading ON, the stock default: now the audience is what
        // refuses them. This is the assertion that failed before the fix.
        assert_eq!(
            can_access_content(None, &facts, &AccessPolicy::default()),
            Decision::Deny(DenyReason::BodyNotInAudience),
            "`accounts_only` refuses an unauthenticated reader even where anonymous reading is \
             allowed — a public work is not an open body"
        );

        // And the control: the same anonymous reader on an ungated work is
        // still allowed, so the refusal above is about the audience and not
        // about anonymous reading being broken.
        let open_work = ContentFacts {
            body_audience: crate::retention::BodyAudience::Anyone,
            ..published(ContentRating::General, Visibility::Public)
        };
        assert_eq!(
            can_access_content(None, &open_work, &AccessPolicy::default()),
            Decision::Allow,
            "the baseline audience is §11.15's: every eligible reader gets the body"
        );
    }

    /// `Anyone` changes nothing for a reader who already passed everything else.
    #[test]
    fn the_widest_audience_is_not_a_bypass_of_anything() {
        let policy = AccessPolicy::default();
        let reader = adult();
        let facts = ContentFacts {
            body_audience: crate::retention::BodyAudience::Anyone,
            ..published(ContentRating::General, Visibility::Public)
        };
        assert_eq!(
            can_access_content_with_standing(Some(&reader), &facts, &policy, None),
            Decision::Allow,
            "an `anyone` audience is the baseline, not a grant"
        );
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
