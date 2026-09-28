//! Body audience: who may read a body this instance holds (spec §7.7).
//!
//! A *narrowing* of §11.15's baseline, which gives every eligible reader the
//! body. The audience is set at up to three levels — instance, source, work —
//! and all three may only narrow. A wider setting is refused by name at the
//! setter rather than silently ignored, because a silently-ignored widening is
//! indistinguishable from compliance: the file says one thing, the instance
//! behaves as though it had been left alone, and nothing reports a difference.

use serde::{Deserialize, Serialize};

/// Who may read a body, narrowing the instance baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyAudience {
    /// Everyone, including an unauthenticated reader. The widest setting, and
    /// the default a work inherits when it names nothing.
    Anyone,
    /// Any signed-in account, at trust level 0.
    AccountsOnly,
    /// An account at or above this trust level.
    ///
    /// Parameterised rather than one variant per level, so `TrustAtLeast(4)` is
    /// not accidentally the same value as `TrustAtLeast(2)` — a per-level enum
    /// is 6 variants and 6 places to forget one, and the two that matter most
    /// are the ones a reader is most likely to be moved into.
    TrustAtLeast(i64),
    /// An operator of this instance.
    RoleOperator,
    /// A §16.18 vanguard member.
    RoleVanguard,
    /// A holder of the §32 media curator role.
    RoleCurator,
}

impl BodyAudience {
    /// The stored spelling. `snake_case` in serde, so this is documentation
    /// rather than a parser.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Anyone => "anyone",
            Self::AccountsOnly => "accounts_only",
            Self::TrustAtLeast(_) => "trust_at_least",
            Self::RoleOperator => "role_operator",
            Self::RoleVanguard => "role_vanguard",
            Self::RoleCurator => "role_curator",
        }
    }

    /// How much of the body this audience removes — **lower removes more**.
    ///
    /// A total order, so "narrowest wins" is a `min` and not a hand-written
    /// comparison with a case per pair. Written as one `match` with every
    /// variant listed: a new variant added later is a compile error here, which
    /// is the point. An unranked variant would be ranked by whatever the
    /// compiler's fallback happened to be.
    #[must_use]
    pub const fn strictness(self) -> u8 {
        match self {
            Self::RoleVanguard => 0,
            Self::RoleCurator => 1,
            Self::RoleOperator => 2,
            // Threshold arm: a HIGHER level is a SMALLER audience, so the rank
            // counts down. Level 6 -> 3, 5 -> 4, ... 0 -> 9.
            //
            // The span is 3..=9 inclusive — seven ranks for seven levels — and
            // the two ends are chosen so the arm sits exactly between the roles
            // (2) and `AccountsOnly` (10). The first version used `9 - clamped`
            // and ran into the one collision this table is written to prevent:
            // `TrustAtLeast(0)` landed on the same rank as `TrustAtLeast(1)`
            // shifted, which is `AccountsOnly`, so three distinct audiences
            // shared a rank and `narrowest` picked between them by argument
            // order. `every_audience_has_its_own_rank` caught it, which is what
            // that test is for.
            Self::TrustAtLeast(level) => {
                let clamped: u8 = if level < 0 {
                    0
                } else if level > 6 {
                    6
                } else {
                    level as u8
                };
                9 - clamped
            }
            Self::AccountsOnly => 10,
            Self::Anyone => 11,
        }
    }
}

/// The narrower of two audiences.
///
/// The narrower is the one that removes more, which is `strictness() ==
/// min`. Every level may only narrow, so this is the whole of the three-level
/// resolution: an instance default, narrowed by a source, narrowed by a work.
#[must_use]
pub fn narrowest(a: BodyAudience, b: BodyAudience) -> BodyAudience {
    if a.strictness() <= b.strictness() {
        a
    } else {
        b
    }
}

/// The narrowest of up to three settings, treating `None` as "inherit".
///
/// `None` is NOT `Anyone`. A work with no audience set inherits whatever the
/// source and instance say, and an explicit `anyone` on a work would override a
/// narrower instance default the moment anyone edited that work — which is a
/// widening path, and the reason the column is NULL when absent rather than the
/// string `'anyone'`.
#[must_use]
pub fn resolve_audience(
    instance: BodyAudience,
    source: Option<BodyAudience>,
    work: Option<BodyAudience>,
) -> BodyAudience {
    let mut resolved = instance;
    if let Some(source) = source {
        resolved = narrowest(resolved, source);
    }
    if let Some(work) = work {
        resolved = narrowest(resolved, work);
    }
    resolved
}

/// Whether `standing` satisfies `audience`.
///
/// `trust_level` alone answers the threshold arm and the accounts-only arm; the
/// three roles are exact facts about the account, not a level. An audience that
/// names a role is NOT satisfied by a higher trust level, because a reader who
/// engages a lot is not thereby an operator — the plan's own note, and the
/// reason `is_vanguard` is membership rather than a resonance score.
///
/// `AccountsOnly` answers from `signed_in` and not from the trust level, so an
/// unauthenticated request is refused it. That is the whole point of naming the
/// variant: "any account" excludes the reader who has none.
#[must_use]
pub fn standing_satisfies(audience: BodyAudience, standing: &super::policy::ActorStanding) -> bool {
    match audience {
        BodyAudience::Anyone => true,
        BodyAudience::AccountsOnly => standing.signed_in,
        BodyAudience::TrustAtLeast(level) => standing.trust_level >= level,
        BodyAudience::RoleOperator => standing.is_operator,
        BodyAudience::RoleVanguard => standing.is_vanguard,
        BodyAudience::RoleCurator => standing.is_curator,
    }
}

impl BodyAudience {
    /// Parse a stored audience, `None` for absent — and absent means INHERIT.
    ///
    /// `None` is the honest answer for three different inputs, and they are kept
    /// together on purpose: the column is NULL, the string is empty, or the
    /// string is something this build has never heard of. A reader that
    /// defaulted an unrecognised value to `Anyone` would be a widening caused by
    /// a typo or by a downgrade — a database written by a newer build, read by an
    /// older one, would hand every gated work to every reader. Failing closed is
    /// the only safe direction for a field whose whole purpose is to withhold.
    ///
    /// `trust_at_least:4` carries its level; a bare `trust_at_least` with no
    /// level parses as level 0, which is the least privileged reading of an
    /// incomplete instruction rather than the most convenient one.
    #[must_use]
    pub fn parse_stored(value: Option<&str>) -> Option<Self> {
        let raw = value?.trim();
        if raw.is_empty() {
            return None;
        }
        match raw {
            "anyone" => Some(Self::Anyone),
            "accounts_only" => Some(Self::AccountsOnly),
            "role_operator" => Some(Self::RoleOperator),
            "role_vanguard" => Some(Self::RoleVanguard),
            "role_curator" => Some(Self::RoleCurator),
            // Only the exact prefix, and only when what follows is a colon or
            // nothing at all. `strip_prefix("trust_at_least")` alone also
            // accepts `trust_at_least_x` and `trust_at_leastely`, which then
            // parse as a level of 0 and silently become a real audience. The
            // distinction matters because an unknown value must INHERIT: a
            // misspelling that parses is a restriction nobody asked for.
            _ => raw
                .strip_prefix("trust_at_least")
                .filter(|rest| rest.is_empty() || rest.starts_with(':'))
                .map(|rest| {
                    let level = rest
                        .trim_start_matches(':')
                        .trim()
                        .parse::<i64>()
                        // An unparseable level is 0, so `trust_at_least:oops` is the
                        // narrowest threshold rather than the widest.
                        .unwrap_or(0);
                    Self::TrustAtLeast(level)
                }),
        }
    }

    /// The stored spelling, parameterised so a threshold keeps its level.
    ///
    /// `as_str` returns `"trust_at_least"` for every level because that is the
    /// variant's NAME; this is what a writer needs, and
    /// `parse_stored(as_stored(x)) == x` is asserted over the whole table.
    #[must_use]
    pub fn as_stored(self) -> String {
        match self {
            Self::TrustAtLeast(level) => format!("trust_at_least:{level}"),
            other => other.as_str().to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::ActorStanding;

    /// Every audience against every other, one table.
    ///
    /// The whole table, because "narrowest" is a relation over seven values and
    /// a relation tested a few pairs at a time leaves the rest to whatever the
    /// code happens to do. This is the same trap as the robots posture rank, and
    /// the same answer.
    fn all() -> Vec<BodyAudience> {
        vec![
            BodyAudience::RoleVanguard,
            BodyAudience::RoleCurator,
            BodyAudience::RoleOperator,
            BodyAudience::TrustAtLeast(6),
            BodyAudience::TrustAtLeast(5),
            BodyAudience::TrustAtLeast(4),
            BodyAudience::TrustAtLeast(2),
            BodyAudience::TrustAtLeast(1),
            BodyAudience::TrustAtLeast(0),
            BodyAudience::AccountsOnly,
            BodyAudience::Anyone,
        ]
    }

    /// No two distinct audiences may share a rank.
    ///
    /// A tie would make `narrowest` pick by argument order rather than by which
    /// audience is smaller — so `TrustAtLeast(4)` and `TrustAtLeast(2)` could be
    /// the same value, and the whole "may only narrow" rule becomes a coin flip
    /// on which of two differently-narrow settings appears first.
    #[test]
    fn every_audience_has_its_own_rank() {
        let mut seen: Vec<(u8, BodyAudience)> = Vec::new();
        for audience in all() {
            let rank = audience.strictness();
            if let Some((other, _)) = seen.iter().find(|(r, _)| *r == rank) {
                panic!(
                    "{audience:?} and {other:?} share rank {rank}; narrowest would pick by \
                     argument order rather than by which audience is smaller"
                );
            }
            seen.push((rank, audience));
        }
    }

    /// The order is by how much each removes, in the order §7.7.2 states.
    #[test]
    fn the_strictness_order_is_the_one_the_spec_states() {
        let ordered = [
            BodyAudience::RoleVanguard,
            BodyAudience::RoleCurator,
            BodyAudience::RoleOperator,
            BodyAudience::TrustAtLeast(6),
            BodyAudience::TrustAtLeast(5),
            BodyAudience::TrustAtLeast(4),
            BodyAudience::TrustAtLeast(2),
            BodyAudience::TrustAtLeast(1),
            BodyAudience::AccountsOnly,
            BodyAudience::Anyone,
        ];
        for window in ordered.windows(2) {
            let (narrower, wider) = (window[0], window[1]);
            assert!(
                narrower.strictness() < wider.strictness(),
                "{narrower:?} must remove more than {wider:?}: {} vs {}",
                narrower.strictness(),
                wider.strictness()
            );
            assert_eq!(
                narrowest(narrower, wider),
                narrower,
                "narrowest({narrower:?}, {wider:?}) is the narrower one, whichever way round it is called"
            );
            assert_eq!(
                narrowest(wider, narrower),
                narrower,
                "and the answer does not depend on argument order"
            );
        }
    }

    /// `AccountsOnly` sits between the trust thresholds and `Anyone`.
    ///
    /// Asserted separately because it is the one variant whose position is
    /// arguable — a signed-in reader at trust 0 is exactly what
    /// `TrustAtLeast(0)` describes, and the two must not be the same value or
    /// an instance that names one has silently named the other.
    #[test]
    fn accounts_only_is_not_the_same_as_a_zero_threshold() {
        assert_ne!(
            BodyAudience::AccountsOnly.strictness(),
            BodyAudience::TrustAtLeast(0).strictness(),
            "the two describe the same population and must still be distinguishable, or an \
             instance that names one has named the other"
        );
        assert!(
            BodyAudience::TrustAtLeast(0).strictness() < BodyAudience::AccountsOnly.strictness(),
            "a stated threshold is the narrower claim: it says which accounts, not merely that \
             there is one"
        );
    }

    /// A threshold outside 0..6 clamps rather than becoming a strange rank.
    ///
    /// The first version of `strictness` treated an unmeetable threshold as the
    /// narrowest audience there is, which reads well and is wrong twice: it
    /// underflowed on a negative level, and it made a typo in a config file
    /// (`trust_at_least = 6o` aside, `= 99`) silently lock a work to nobody
    /// rather than behaving like the top of the range. Clamping to 6 says what
    /// the operator most likely meant — "the top" — and is the reading that
    /// fails toward the *account* rather than away from it.
    #[test]
    fn a_threshold_outside_the_range_clamps_to_the_range() {
        for (written, clamps_to) in [(99_i64, 6_i64), (6, 6), (0, 0), (-1, 0), (-99, 0)] {
            assert_eq!(
                BodyAudience::TrustAtLeast(written).strictness(),
                BodyAudience::TrustAtLeast(clamps_to).strictness(),
                "a threshold of {written} is the same audience as {clamps_to}, not a new one"
            );
        }
        // And the clamped value still is a real threshold: level 6 does not pass
        // it by accident.
        assert!(!standing_satisfies(
            BodyAudience::TrustAtLeast(99),
            &ActorStanding {
                trust_level: 5,
                ..ActorStanding::none()
            }
        ));
    }

    /// Three levels may only narrow; `None` inherits rather than widening.
    #[test]
    fn three_levels_narrow_and_absence_inherits() {
        assert_eq!(
            resolve_audience(BodyAudience::Anyone, None, None),
            BodyAudience::Anyone,
            "nothing set means the instance's own setting, which is the widest here"
        );
        // `RoleOperator` is the narrowest of these three — one role removes more
        // than "trust 4 or above", because it names a smaller population. The
        // first version of this test expected `TrustAtLeast(4)` and failed,
        // which is the test being wrong rather than the ranking: the spec's own
        // order puts the roles ahead of the thresholds for exactly this reason.
        assert_eq!(
            resolve_audience(
                BodyAudience::RoleOperator,
                Some(BodyAudience::AccountsOnly),
                Some(BodyAudience::TrustAtLeast(4))
            ),
            BodyAudience::RoleOperator,
            "narrowest of the three wins, and a role is narrower than any threshold"
        );
        assert_eq!(
            resolve_audience(
                BodyAudience::RoleOperator,
                Some(BodyAudience::TrustAtLeast(4)),
                Some(BodyAudience::AccountsOnly)
            ),
            BodyAudience::RoleOperator,
            "and swapping the last two does not change the answer"
        );
        // And a threshold does beat the two open settings, which is the case a
        // reader is most likely to be moved into.
        assert_eq!(
            resolve_audience(
                BodyAudience::Anyone,
                Some(BodyAudience::AccountsOnly),
                Some(BodyAudience::TrustAtLeast(2))
            ),
            BodyAudience::TrustAtLeast(2),
            "the narrowest of the three open settings is a threshold, not a role"
        );
        // A work cannot widen what the source narrowed.
        assert_eq!(
            resolve_audience(
                BodyAudience::Anyone,
                Some(BodyAudience::RoleCurator),
                Some(BodyAudience::Anyone)
            ),
            BodyAudience::RoleCurator,
            "a work set to `anyone` does not override a narrower source — which is why the column \
             is NULL for absent and never the string 'anyone'"
        );
    }

    /// A role is a fact about the account, not a level a reader can climb.
    #[test]
    fn a_role_audience_is_not_satisfied_by_a_high_trust_level() {
        let trusted = ActorStanding {
            trust_level: 6,
            ..ActorStanding::none()
        };
        for role in [
            BodyAudience::RoleOperator,
            BodyAudience::RoleVanguard,
            BodyAudience::RoleCurator,
        ] {
            assert!(
                !standing_satisfies(role, &trusted),
                "{role:?} must not be satisfied by trust level alone: a reader who engages a lot \
                 is not thereby an operator"
            );
        }
        // And each role is satisfied only by itself.
        for (role, flag) in [
            (BodyAudience::RoleOperator, "is_operator"),
            (BodyAudience::RoleVanguard, "is_vanguard"),
            (BodyAudience::RoleCurator, "is_curator"),
        ] {
            let mut with_other = ActorStanding::none();
            // Set the two roles this is NOT.
            for other in ["is_operator", "is_vanguard", "is_curator"] {
                if other != flag {
                    match other {
                        "is_operator" => with_other.is_operator = true,
                        "is_vanguard" => with_other.is_vanguard = true,
                        _ => with_other.is_curator = true,
                    }
                }
            }
            assert!(
                !standing_satisfies(role, &with_other),
                "{role:?} is not satisfied by holding {flag}'s neighbours"
            );
        }
    }

    /// A threshold is a threshold: at the level passes, below it does not.
    #[test]
    fn a_threshold_is_satisfied_at_the_level_and_not_below() {
        for level in 0..=6 {
            let at = ActorStanding {
                trust_level: level,
                ..ActorStanding::none()
            };
            assert!(standing_satisfies(BodyAudience::TrustAtLeast(level), &at));
            if level > 0 {
                let below = ActorStanding {
                    trust_level: level - 1,
                    ..ActorStanding::none()
                };
                assert!(
                    !standing_satisfies(BodyAudience::TrustAtLeast(level), &below),
                    "trust {level} must not pass a threshold of {level}"
                );
            }
        }
    }

    /// A stored value round-trips, and an absent one stays absent.
    ///
    /// Round-tripping is what makes the column trustworthy: a writer that
    /// spelled `TrustAtLeast(4)` as `trust_at_least` and a reader that parsed it
    /// back as level 0 would narrow a work to every account without any visible
    /// change. The `None` half is the other direction — absent must stay absent,
    /// because absent is "inherit" and inventing a value here is a widening.
    #[test]
    fn a_stored_audience_round_trips_and_an_absent_one_stays_absent() {
        for audience in all() {
            let stored = audience.as_stored();
            assert_eq!(
                BodyAudience::parse_stored(Some(&stored)),
                Some(audience),
                "{audience:?} stored as {stored:?} must read back as itself"
            );
        }
        assert_eq!(BodyAudience::parse_stored(None), None, "NULL inherits");
        assert_eq!(
            BodyAudience::parse_stored(Some("")),
            None,
            "and so does an empty string, which is what a NOT NULL default of '' would give"
        );
        assert_eq!(
            BodyAudience::parse_stored(Some("   ")),
            None,
            "whitespace is not a value"
        );
    }

    /// A value this build does not understand is treated as absent, not as open.
    ///
    /// The downgrade case: a database written by a newer build, read by an older
    /// one. Defaulting the unknown to `Anyone` would hand every gated work to
    /// every reader, and the only symptom would be that a restriction quietly
    /// stopped applying. So the unknown parses to "inherit" — which resolves
    /// through the instance and source defaults, the same as a NULL, and is the
    /// narrowest honest reading of an instruction this build cannot follow.
    #[test]
    fn an_unrecognised_stored_value_is_inherited_not_opened() {
        for unknown in [
            "everyone",
            "public",
            "trust_at_least_x",
            "ROLE_OPERATOR",
            "*",
        ] {
            assert_eq!(
                BodyAudience::parse_stored(Some(unknown)),
                None,
                "{unknown:?} is not an audience; inheriting is the safe reading and `Anyone` is \
                 the one that widens"
            );
        }
        // A threshold with an unparseable level is the NARROWEST threshold, not
        // the widest.
        assert_eq!(
            BodyAudience::parse_stored(Some("trust_at_least:oops")),
            Some(BodyAudience::TrustAtLeast(0)),
            "a level that cannot be read is 0, the least privileged reading"
        );
        assert_eq!(
            BodyAudience::parse_stored(Some("trust_at_least")),
            Some(BodyAudience::TrustAtLeast(0)),
            "and a bare threshold with no level is the same"
        );
    }

    /// The default standing satisfies nothing but `Anyone`.
    ///
    /// `AccountsOnly` was in this list until the `signed_in` field existed: it
    /// was returning `true` for a standing that has no account at all, which
    /// meant a work gated to signed-in readers served its body to anonymous
    /// requests. The test passed while asserting that. It now names the one
    /// audience the default *does* satisfy, and `accounts_only` is asserted
    /// separately below with the fact that distinguishes the two cases.
    ///
    /// `TrustAtLeast(0)` is deliberately absent from the list, and the comment
    /// above it is load-bearing rather than an oversight: a threshold of 0 *is*
    /// satisfied by a standing at level 0, which is what
    /// `a_threshold_is_satisfied_at_the_level_and_not_below` pins. It differs
    /// from `AccountsOnly` in exactly that it does not ask whether there is an
    /// account — which is why the two must not be collapsed into one variant.
    #[test]
    fn the_default_standing_is_the_least_privileged() {
        let none = ActorStanding::none();
        assert!(standing_satisfies(BodyAudience::Anyone, &none));
        for audience in [
            BodyAudience::AccountsOnly,
            BodyAudience::TrustAtLeast(1),
            BodyAudience::RoleOperator,
            BodyAudience::RoleVanguard,
            BodyAudience::RoleCurator,
        ] {
            assert!(
                !standing_satisfies(audience, &none),
                "a caller that forgot to look standing up must fail {audience:?}, not pass it"
            );
        }
    }

    /// `accounts_only` is about having an account, not about trust level.
    ///
    /// Both halves, because the two differ in exactly one field and a test that
    /// asserted only one of them would pass against either implementation: trust
    /// 0 with no session is refused, and trust 0 with a session is allowed.
    #[test]
    fn accounts_only_asks_for_an_account_not_for_trust() {
        let anonymous = ActorStanding {
            trust_level: 0,
            ..ActorStanding::none()
        };
        assert!(
            !standing_satisfies(BodyAudience::AccountsOnly, &anonymous),
            "no session is not an account, whatever the trust level says"
        );

        let fresh = ActorStanding::signed_in();
        assert_eq!(fresh.trust_level, 0, "a new account is at trust 0");
        assert!(
            standing_satisfies(BodyAudience::AccountsOnly, &fresh),
            "a signed-in account at trust 0 is exactly what `accounts_only` describes"
        );
    }
}
