//! The actor model: **who is asking**, in one type.
//!
//! # Why this exists
//!
//! The estate already had an `Actor` — but it was the *content check's* actor, and it
//! could not say the things the identity model needs to say:
//!
//! * an **anonymous** visitor has no account and no pseud, and the old type's
//!   `account_id` and `pseud_id` were not optional, so "nobody" had to be spelled
//!   `Option<&Actor>` — which is a *second* way to say the same thing, and a
//!   second way is how a distinction nobody chose becomes a distinction somebody
//!   relies on;
//! * a **guest** is a real account with a real trust level and a special status
//!   (spec §14.1: a guest is a row in `accounts` with an empty credential and
//!   `is_guest = true`), and nothing in the old type could say "guest";
//! * the **tier** — T0, T1 or T2 (spec §12.6) — is a fact about the request, and
//!   it was computed nowhere.
//!
//! # The rule this module holds to
//!
//! **The tier and its reason come from ONE evaluation.** `tier_for` and
//! `tier_reason` are two names for the two halves of a single function, so they
//! **cannot** disagree. A tier and an explanation that disagree is a reader being
//! told "you are on the full engine" by one part of the code and "you are on the
//! house theme" by another, and spec §12.6 rule 6 exists because *"three
//! overlapping 'or' conditions produce an ambiguous tier, and an ambiguous tier
//! produces an ambiguous explanation."*

use crate::ids::{AccountId, PseudId};
use crate::policy::AgeState;

/// Who is asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorKind {
    /// Nobody is signed in. There is no account and no pseud.
    Anonymous,
    /// A **guest**: a row in `accounts` with an empty credential and
    /// `is_guest = true` (spec §14.1, D-6). Not a shadow table, not a
    /// cookie-only identity.
    ///
    /// A guest is a real account with a real trust level — level 1, *"new"* —
    /// and can never extend it.
    Guest,
    /// A signed-in reader, acting as one of their pseudonyms.
    Pseud,
}

impl ActorKind {
    /// Whether this kind has an account behind it.
    ///
    /// A guest does. **A guest is an account** — that is the whole of D-6.
    #[must_use]
    pub const fn has_account(self) -> bool {
        !matches!(self, ActorKind::Anonymous)
    }

    /// Whether this kind acts as a named pseudonym.
    ///
    /// A guest does not: a guest is an account with no pseud, and it acts as
    /// itself. The distinction matters because a guest's *content* is the
    /// instance's to attribute and a pseud's is the reader's to choose.
    #[must_use]
    pub const fn has_pseud(self) -> bool {
        matches!(self, ActorKind::Pseud)
    }
}

/// The compute tier a request is served on (spec §12.6).
///
/// **T0 is not "no recommendations".** It is the house theme, which is a real
/// recommendation and is explained. The fallback is never an empty page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// **T0 — house.** The precomputed house-theme ranking, plus a cheap
    /// per-session adjustment. One cached list; negligible cost.
    House,
    /// **T1 — personal (lite).** The reader's own profile blended with the house
    /// floor, in lite mode only: tag overlap and neighbourhood, no embeddings, no
    /// multi-strategy fusion, no co-bookmark walk.
    Personal,
    /// **T2 — full.** The whole engine: the CSR graph neighbourhood, spectral
    /// embeddings, the co-bookmark signal, multi-strategy RRF fusion, the
    /// theme-weighted consensus, the arena, per-fandom diversity budgeting.
    Full,
}

impl Tier {
    /// The tier's name, as the configuration calls it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Tier::House => "house",
            Tier::Personal => "personal",
            Tier::Full => "full",
        }
    }

    /// The tier's short name, as the specification numbers it.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Tier::House => "T0",
            Tier::Personal => "T1",
            Tier::Full => "T2",
        }
    }
}

/// Who is asking, in one type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Actor {
    /// Which kind of actor this is.
    pub kind: ActorKind,
    /// The account behind the request. `None` for an anonymous visitor.
    ///
    /// **A guest has one** — a guest is an account (D-6).
    pub account_id: Option<AccountId>,
    /// The pseudonym the request acts as. `None` for an anonymous visitor and for
    /// a guest, which acts as itself.
    pub pseud_id: Option<PseudId>,
    /// The actor's age-policy state (spec §15.8).
    pub age_state: AgeState,
    /// The actor's trust level, 0..6 (spec §14.4).
    ///
    /// `i64` rather than the `i16` a sketch might use, because
    /// `policy::ActorStanding` already carries a trust level as an `i64` and
    /// **two representations of one number is how they come to disagree.**
    pub trust_level: i64,
    /// The tier this request is served on.
    ///
    /// **This is a cache of [`tier_for`]'s answer, not a second opinion.** The
    /// establishment step computes the tier once and puts it here; the reader of
    /// the field never computes one.
    pub tier: Tier,
}

impl Actor {
    /// An anonymous visitor: no account, no pseud, no trust, the house theme.
    #[must_use]
    pub const fn anonymous(age_state: AgeState) -> Self {
        Self {
            kind: ActorKind::Anonymous,
            account_id: None,
            pseud_id: None,
            age_state,
            trust_level: 0,
            tier: Tier::House,
        }
    }

    /// Whether this actor is signed in at all — as a reader or as a guest.
    #[must_use]
    pub const fn is_signed_in(&self) -> bool {
        self.kind.has_account()
    }
}

/// The tier boundaries (spec §12.6).
///
/// **Every limit is configuration**, and the boundaries are published in
/// `/api/v1/meta` so a client can show the reader where they stand. This is the
/// domain's own copy of that configuration; the application maps its file
/// configuration onto it, and the domain does not read files.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TierConfig {
    /// Whether T0 is always available. The only value is `true`: **T0 is not "no
    /// recommendations"**, it is a real recommendation and it is explained.
    pub house_always: bool,
    /// The trust level T1 (personal, lite) needs.
    pub personal_min_trust: i64,
    /// Whether the Supporter tier reaches T2.
    pub full_supporter: bool,
    /// The trust level that reaches T2.
    pub full_min_trust: i64,
    /// The resonance that reaches T2. A reader with no evidence has resonance 0
    /// regardless of agreement, because agreement without evidence is a
    /// coincidence.
    pub full_min_resonance: f64,
    /// The reader's resonance: how strongly their learned taste agrees with the
    /// instance theme, discounted by the strength of their evidence.
    ///
    /// **It is derived, recomputed, and never trusted from a request** — so it
    /// travels with the actor rather than being read from the body.
    pub resonance: f64,
    /// Whether the actor holds a supporter entitlement, and until when.
    ///
    /// A tier buys **compute**, and never rank (D-10, N-1): this is a
    /// statement about how much work the instance does, not about where the
    /// reader's work appears in a list.
    pub supporter: bool,
}

impl Default for TierConfig {
    fn default() -> Self {
        Self {
            house_always: true,
            personal_min_trust: 1,
            full_supporter: true,
            full_min_trust: 4,
            full_min_resonance: 0.55,
            resonance: 0.0,
            supporter: false,
        }
    }
}

/// The tier a request is served on (spec §12.6).
///
/// The three T2 gates are evaluated in a **fixed order** — supporter, then trust,
/// then resonance — and **exactly one wins**, because three overlapping "or"
/// conditions produce an ambiguous tier and an ambiguous tier produces an
/// ambiguous explanation (spec §12.6 rule 6).
#[must_use]
pub fn tier_for(actor: &Actor, config: &TierConfig) -> Tier {
    evaluate(actor, config).0
}

/// Why a request is served on the tier it is served on (spec §12.6 rule 2).
///
/// **Exactly one reason is named, and it is the one that matched** — the reason
/// that will still be true if the other two change. A reader who qualifies by
/// all three is told they qualified as a supporter, because that is the reason
/// that survives the others changing.
#[must_use]
pub fn tier_reason(actor: &Actor, config: &TierConfig) -> &'static str {
    evaluate(actor, config).1
}

/// The tier and its reason, from **one** evaluation.
///
/// **This is the whole point of the module.** `tier_for` and `tier_reason` are
/// two names for the two halves of this one function, so a tier and an
/// explanation cannot disagree. The alternative — two functions that each
/// re-evaluate the gates — is how a reader ends up being told *"you are on the
/// full engine because you are a supporter"* by one part of the code and
/// *"you are on the house theme"* by another.
fn evaluate(actor: &Actor, config: &TierConfig) -> (Tier, &'static str) {
    // --- T2, in the fixed order: supporter, then trust, then resonance --------
    if config.full_supporter && config.supporter {
        return (Tier::Full, "you are a supporter");
    }
    if actor.trust_level >= config.full_min_trust {
        return (Tier::Full, "your trust level is high enough");
    }
    if config.resonance >= config.full_min_resonance {
        return (Tier::Full, "your taste agrees with this instance's theme");
    }

    // --- T1: a registered reader --------------------------------------------
    if actor.is_signed_in() && actor.trust_level >= config.personal_min_trust {
        return (Tier::Personal, "you are signed in");
    }

    // --- T0: the house theme -------------------------------------------------
    // T0 is NOT "no recommendations". It is the house ranking, and it is
    // explained like every other tier.
    if !actor.is_signed_in() {
        (Tier::House, "you have not signed in")
    } else {
        (
            Tier::House,
            "your trust level is below the threshold for a personal stream",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reader(trust_level: i64) -> Actor {
        Actor {
            kind: ActorKind::Pseud,
            account_id: Some(AccountId::new()),
            pseud_id: Some(PseudId::new()),
            age_state: AgeState::DeclaredAdult,
            trust_level,
            tier: Tier::House,
        }
    }

    #[test]
    fn an_anonymous_visitor_gets_the_house_theme() {
        let actor = Actor::anonymous(AgeState::Unknown);
        let config = TierConfig::default();
        assert_eq!(tier_for(&actor, &config), Tier::House);
        assert_eq!(tier_reason(&actor, &config), "you have not signed in");
    }

    #[test]
    fn a_registered_reader_gets_the_personal_stream() {
        let actor = reader(1);
        let config = TierConfig::default();
        assert_eq!(tier_for(&actor, &config), Tier::Personal);
    }

    #[test]
    fn the_three_full_gates_are_evaluated_in_a_fixed_order() {
        // A reader who qualifies by all three is told they qualified as a
        // supporter, because that is the reason that survives the other two
        // changing.
        let actor = reader(6);
        let config = TierConfig {
            resonance: 0.9,
            supporter: true,
            ..TierConfig::default()
        };
        assert_eq!(tier_for(&actor, &config), Tier::Full);
        assert_eq!(
            tier_reason(&actor, &config),
            "you are a supporter",
            "the supporter gate is first, and it is the reason that will still be true"
        );
    }

    #[test]
    fn trust_is_the_reason_when_the_reader_is_not_a_supporter() {
        let actor = reader(6);
        let config = TierConfig {
            resonance: 0.9,
            supporter: false,
            ..TierConfig::default()
        };
        assert_eq!(tier_for(&actor, &config), Tier::Full);
        assert_eq!(
            tier_reason(&actor, &config),
            "your trust level is high enough"
        );
    }

    #[test]
    fn resonance_is_the_reason_when_neither_of_the_others_matched() {
        let actor = reader(1);
        let config = TierConfig {
            resonance: 0.9,
            supporter: false,
            ..TierConfig::default()
        };
        assert_eq!(tier_for(&actor, &config), Tier::Full);
        assert_eq!(
            tier_reason(&actor, &config),
            "your taste agrees with this instance's theme"
        );
    }

    #[test]
    fn the_tier_and_its_reason_cannot_disagree() {
        // The module's own contract, asserted over the whole grid: for every
        // combination of the three gates, the reason names the gate that
        // matched, and the tier is the one that gate implies.
        for trust in 0..=6 {
            for resonance in [0.0_f64, 0.6, 0.9] {
                for supporter in [false, true] {
                    let actor = reader(trust);
                    let config = TierConfig {
                        resonance,
                        supporter,
                        ..TierConfig::default()
                    };
                    let tier = tier_for(&actor, &config);
                    let reason = tier_reason(&actor, &config);
                    assert_eq!(
                        tier,
                        evaluate(&actor, &config).0,
                        "the two public functions disagree"
                    );
                    assert_eq!(
                        reason,
                        evaluate(&actor, &config).1,
                        "the two public functions disagree"
                    );
                    if tier == Tier::Full {
                        assert!(
                            reason.contains("supporter")
                                || reason.contains("trust level")
                                || reason.contains("taste agrees"),
                            "a full tier must name the gate that matched: {reason}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_guest_is_an_account() {
        // D-6: a guest is a row in `accounts` with an empty credential and
        // `is_guest = true`. Not a shadow table, not a cookie-only identity.
        let guest = Actor {
            kind: ActorKind::Guest,
            account_id: Some(AccountId::new()),
            pseud_id: None,
            age_state: AgeState::Unknown,
            trust_level: 1,
            tier: Tier::Personal,
        };
        assert!(guest.is_signed_in());
        assert!(guest.kind.has_account());
        assert!(
            !guest.kind.has_pseud(),
            "a guest acts as itself; it has no pseudonym to choose"
        );
        assert!(!ActorKind::Anonymous.has_account());
    }
}
