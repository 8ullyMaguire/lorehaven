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

// ---------------------------------------------------------------------------
// §11.15 — the retention setting itself.
//
// The audience above answers "who may read a body this instance holds". It
// presupposes that it holds one. This half answers the prior question: does it?
// ---------------------------------------------------------------------------

/// Whether this instance stores fetched bodies, or is a catalogue of links
/// (spec §11.15).
///
/// Two values and not three, for the reason §11.15 gives: the media setting
/// (§30.2) needed a third because media has a player shell to omit, and text has
/// no shell. A third value here would be a state with no behaviour behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyMode {
    /// The fetched body is stored here and served from here. A library item is
    /// a durable snapshot referenced by that reader's copy. The default, and the
    /// behaviour of every instance built before this type existed.
    #[default]
    Cache,
    /// Metadata, attribution, provenance and canonical links are stored; the
    /// body is never fetched into this instance's storage. A library item is a
    /// reference to the work at its origin.
    Aggregate,
}

impl BodyMode {
    /// The stored spelling. It is a database value and an API response where
    /// changing it is a migration, so it is one function and not a serde derive
    /// somebody can rename away.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cache => "cache",
            Self::Aggregate => "aggregate",
        }
    }

    /// Parse a stored mode, `None` for absent or unrecognised.
    ///
    /// Absent and unrecognised are kept together for the same reason
    /// `BodyAudience::parse_stored` keeps three inputs together, and with the
    /// same asymmetry: an unrecognised mode fails **closed to `Cache`**, not to
    /// `Aggregate`. That is the non-obvious part. A reader meeting a value from
    /// a newer build would otherwise decide the opposite of what the operator
    /// chose, and the two defaults differ in what they cost — `Aggregate` under
    /// a `Cache` operator silently stops storing every body this instance holds,
    /// which is a policy change made by a downgrade. `Cache` under an
    /// `Aggregate` operator over-retains, which is the pre-existing behaviour of
    /// every instance and is the §10.4 deletion workflow's business, not this
    /// function's.
    #[must_use]
    pub fn parse_stored(value: Option<&str>) -> Option<Self> {
        match value?.trim() {
            "cache" => Some(Self::Cache),
            "aggregate" => Some(Self::Aggregate),
            _ => None,
        }
    }

    /// Whether this mode stores bodies.
    #[must_use]
    pub const fn stores_bodies(self) -> bool {
        matches!(self, Self::Cache)
    }
}

/// Why a body was refused, as a stable code (spec §11.15, amendment §4.1).
///
/// The amendment adds this as an explicit addition to §11.15 precisely so that
/// the six refusal paths are *enumerable*. The alternative — six hand-written
/// strings at six call sites — drifts, and a caller that wants to know whether
/// it is allowed to retry has no way to tell a rate limit from a policy.
///
/// Every variant names a *policy*, never a transient condition. A network error
/// is not a refusal: it is a failure, it retries, and it must not be reported to
/// a reader as the instance declining on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RetentionReason {
    /// The instance is set to `aggregate` and this source has no override.
    Aggregate,
    /// The instance is `cache`, but this source family is overridden to
    /// `aggregate`. Distinct from `Aggregate` because the fix differs: the
    /// operator narrows a source, and the source's own configuration is fine.
    AggregateSourceOverride,
    /// The source is blocked, so nothing may be fetched from it at all
    /// (§11.6). Distinguished from the two above because it is not about
    /// retention: widening the retention mode would not make this work.
    SourceBlocked,
    /// The origin is gone (§11.13) and this instance holds no body, so there is
    /// nothing to serve and nothing to fetch. A dead end that no setting
    /// recovers.
    Vanished,
}

impl RetentionReason {
    /// The stable wire code, which is the thing a caller may branch on.
    #[must_use]
    pub const fn as_code(self) -> &'static str {
        match self {
            Self::Aggregate => "RETENTION_AGGREGATE",
            Self::AggregateSourceOverride => "RETENTION_AGGREGATE_SOURCE_OVERRIDE",
            Self::SourceBlocked => "RETENTION_SOURCE_BLOCKED",
            Self::Vanished => "RETENTION_VANISHED",
        }
    }

    /// The human sentence a refusal carries, naming the policy.
    ///
    /// §11.15 requires that "every refusal names the instance's policy rather
    /// than failing as a generic error", so this returns prose and not just a
    /// code. The sentence is built from the code's own components rather than
    /// stored beside it, so the two cannot drift.
    #[must_use]
    pub fn message(self, source_key: Option<&str>) -> String {
        let source = source_key.unwrap_or("this instance");
        match self {
            Self::Aggregate => format!(
                "{source} is set to aggregate: this instance keeps metadata and links, not \
                 the text. Change the retention policy in admin settings if you want it to hold \
                 bodies."
            ),
            Self::AggregateSourceOverride => format!(
                "{source} is overridden to aggregate on this instance, even though the instance \
                 itself caches bodies. Remove the source override in admin settings to store it."
            ),
            Self::SourceBlocked => format!(
                "{source} is blocked, so nothing is fetched from it. Unblock the source first."
            ),
            Self::Vanished => format!(
                "{source} is no longer reachable and this instance holds no body for this work, \
                 so there is nothing to fetch."
            ),
        }
    }
}

/// A refusal, carrying the code and the sentence together.
///
/// This exists because §11.15 requires that "every refusal names the instance's
/// policy rather than failing as a generic error", and returning a bare
/// `RetentionReason` cannot enforce that. A code is a branch condition; a
/// call site that formats one into its own error has to re-derive the sentence,
/// and the re-derived sentence is where the requirement goes to die — one site
/// writes "not allowed", another writes "forbidden", and §11.15 is satisfied in
/// the type and violated on the wire.
///
/// So the message is a field. The only way to produce the error is to produce
/// the sentence with it, and `message()` is a getter rather than a constructor
/// precisely so that no call site can assemble one by hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionRefusal {
    /// Which of the four paths refused. What a caller branches on.
    pub reason: RetentionReason,
    /// The source the request was for, when the call site named one.
    pub source_key: Option<String>,
}

impl RetentionRefusal {
    /// The sentence a reader or an operator is shown.
    ///
    /// Names the policy and the thing to change. A refusal that only says "no"
    /// costs the reader a support question, and §11.15's whole point is that the
    /// operator's decision is visible to them.
    #[must_use]
    pub fn message(&self) -> String {
        self.reason.message(self.source_key.as_deref())
    }

    /// The stable wire code, forwarded so a serialiser reads one shape.
    #[must_use]
    pub fn code(&self) -> &'static str {
        self.reason.as_code()
    }
}

impl std::fmt::Display for RetentionRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code(), self.message())
    }
}

impl std::error::Error for RetentionRefusal {}

/// The retention policy as it applies to one body request.
///
/// Resolved once, at the door, so that no call site re-derives it. Every
/// field is a fact rather than a question, which is what lets
/// `check_body_allowed` be a total function with no database access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedRetention {
    /// The instance's own setting.
    pub instance: BodyMode,
    /// The per-source override, if this source has one. Narrowing only.
    pub source: Option<BodyMode>,
    /// Whether the source is blocked at all (§11.6).
    pub source_blocked: bool,
    /// Whether the origin is unreachable (§11.13) and no body is held.
    pub vanished: bool,
}

/// Whether this instance may store a body for this source (spec §11.15).
///
/// The one function every storage path calls, so the six paths cannot disagree
/// about the answer. Order matters and is the order the reasons are ordered in:
/// a blocked source and a vanished origin are both answered before the mode,
/// because neither is fixed by changing the mode, and reporting
/// `RETENTION_AGGREGATE` to somebody whose source is blocked would send them to
/// change a setting that cannot help them.
///
/// A per-source override may only narrow. `narrowest_mode` decides, so an
/// override of `Cache` on an `Aggregate` instance resolves to `Aggregate`
/// rather than being an error here — the *setter* refuses a widening by name
/// (§11.15: "the reverse on an aggregate instance is not"), because refusing at
/// the write is what makes the rule visible to the operator who typed it. A
/// reader that widened through some other path still gets the safe answer here.
pub fn check_body_allowed(
    policy: &ResolvedRetention,
    source_key: Option<&str>,
) -> Result<(), RetentionRefusal> {
    let refuse = |reason| RetentionRefusal {
        reason,
        source_key: source_key.map(str::to_owned),
    };
    if policy.source_blocked {
        return Err(refuse(RetentionReason::SourceBlocked));
    }
    if policy.vanished {
        return Err(refuse(RetentionReason::Vanished));
    }
    let effective = narrowest_mode(policy.instance, policy.source);
    if effective == BodyMode::Aggregate {
        return Err(refuse(if policy.source == Some(BodyMode::Aggregate) {
            RetentionReason::AggregateSourceOverride
        } else {
            RetentionReason::Aggregate
        }));
    }
    Ok(())
}

/// The narrower of two modes, where `aggregate` is the narrower.
///
/// `None` is "inherit", so the instance's own mode is returned unchanged. Two
/// values make this a comparison rather than a rank table, but it is still one
/// function because the direction is the thing that must not be guessed: a
/// reader that returned the *wider* of two settings here would let a per-source
/// `Cache` override restore storage on an aggregating instance, which is the
/// one widening §11.15 refuses.
#[must_use]
pub fn narrowest_mode(instance: BodyMode, source: Option<BodyMode>) -> BodyMode {
    match source {
        None => instance,
        Some(BodyMode::Aggregate) => BodyMode::Aggregate,
        // `Cache` is wider, so it never wins over the instance — including when
        // the instance is `Cache` too, where the two are the same answer.
        Some(BodyMode::Cache) => instance,
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

    // -- §11.15: the mode, the reasons, and the one function that reads them --

    /// The source every test below asks about, so the `source_key` field of a
    /// refusal is the same value in the expectation as in the result.
    ///
    /// Without this the equality assertions would compare a refusal built from
    /// `Some("ao3")` against a stub with `None`, and every one of them would
    /// fail for a reason that has nothing to do with the code under test. A
    /// shared fixture is the difference between the test failing because the
    /// policy is wrong and failing because the harness is.
    fn refusal() -> RetentionRefusal {
        RetentionRefusal {
            reason: RetentionReason::Aggregate,
            source_key: Some("ao3".to_owned()),
        }
    }

    /// The stored spelling round-trips for both values.
    ///
    /// A writer and a reader of the same column disagreeing is a setting that
    /// flips, so the pair is asserted rather than assumed. `parse_stored` takes
    /// `Option` and `as_stored` returns `String` for the audience types, and the
    /// same shape is used here so a caller cannot accidentally use one for the
    /// other.
    #[test]
    fn a_body_mode_round_trips_through_its_stored_spelling() {
        for mode in [BodyMode::Cache, BodyMode::Aggregate] {
            assert_eq!(
                BodyMode::parse_stored(Some(mode.as_str())),
                Some(mode),
                "{mode:?} does not survive its own stored spelling"
            );
        }
        assert_eq!(BodyMode::default(), BodyMode::Cache);
    }

    /// An unrecognised mode is refused, and the caller's default decides.
    ///
    /// Both halves, because the dangerous failure is the *pair* agreeing: a
    /// parser that returned `Some(Aggregate)` for garbage would make a
    /// downgrade stop storing every body on the instance, and a test that only
    /// asserted "not Cache" would pass against that. `None` is the only honest
    /// answer, and the caller then uses `BodyMode::default()`.
    #[test]
    fn an_unrecognised_body_mode_is_refused_rather_than_guessed() {
        for value in [
            "",
            "  ",
            "CACHE",
            "Aggregate",
            "cached",
            "agggregate",
            "metadata",
        ] {
            assert_eq!(
                BodyMode::parse_stored(Some(value)),
                None,
                "{value:?} must not parse as a mode: it is a value this build has never heard of"
            );
        }
        assert_eq!(BodyMode::parse_stored(None), None);
    }

    /// The four codes are exactly the four the amendment names, and distinct.
    ///
    /// Enumerable on purpose (amendment §4.1): a test that lists them is what
    /// makes a fifth reason a deliberate addition rather than a quiet one. Two
    /// reasons sharing a code would let a caller branch on a code that does not
    /// identify the case it was written for.
    #[test]
    fn the_refusal_codes_are_the_four_the_amendment_names() {
        let reasons = [
            RetentionReason::Aggregate,
            RetentionReason::AggregateSourceOverride,
            RetentionReason::SourceBlocked,
            RetentionReason::Vanished,
        ];
        let expected = [
            "RETENTION_AGGREGATE",
            "RETENTION_AGGREGATE_SOURCE_OVERRIDE",
            "RETENTION_SOURCE_BLOCKED",
            "RETENTION_VANISHED",
        ];
        for (reason, code) in reasons.iter().zip(expected.iter()) {
            assert_eq!(reason.as_code(), *code);
            assert!(
                !reason.message(Some("ao3")).is_empty(),
                "{code} must carry a sentence naming the policy, not fail generically"
            );
        }
        let mut seen: Vec<&str> = reasons.iter().map(|r| r.as_code()).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), reasons.len(), "two reasons share a code");
    }

    /// A caching instance stores; an aggregating one refuses, and the reason
    /// distinguishes the instance from a per-source override.
    ///
    /// The reason distinction is the substantive half. Both refusals say
    /// "aggregate", but they are fixed by different actions — one by the
    /// instance setting, one by removing a source override — and a reader sent
    /// to the wrong one of those two settings concludes the operator is wrong
    /// about something they are not.
    #[test]
    fn an_aggregate_instance_refuses_and_a_caching_one_stores() {
        let caching = ResolvedRetention {
            instance: BodyMode::Cache,
            source: None,
            source_blocked: false,
            vanished: false,
        };
        assert_eq!(check_body_allowed(&caching, Some("ao3")), Ok(()));

        let aggregating = ResolvedRetention {
            instance: BodyMode::Aggregate,
            ..caching
        };
        assert_eq!(
            check_body_allowed(&aggregating, Some("ao3")),
            Err(RetentionRefusal {
                reason: RetentionReason::Aggregate,
                ..refusal()
            }),
            "an aggregating instance with no override is the plain case"
        );

        let overridden = ResolvedRetention {
            instance: BodyMode::Cache,
            source: Some(BodyMode::Aggregate),
            ..caching
        };
        assert_eq!(
            check_body_allowed(&overridden, Some("ao3")),
            Err(RetentionRefusal {
                reason: RetentionReason::AggregateSourceOverride,
                ..refusal()
            }),
            "the instance caches, so this refusal is the override's doing and says so"
        );
    }

    /// A per-source override may narrow and never widen.
    ///
    /// The widening half is the one with teeth: a per-source `Cache` on an
    /// `Aggregate` instance would restore storage the operator removed, and
    /// §11.15 refuses that at the setter. This is the second line of defence,
    /// for a row that reached the table by some other path — and a test that
    /// only checked the narrowing would pass against a `narrowest_mode` that
    /// returned the override unconditionally.
    #[test]
    fn a_per_source_override_may_only_narrow() {
        assert_eq!(
            narrowest_mode(BodyMode::Cache, Some(BodyMode::Aggregate)),
            BodyMode::Aggregate,
            "narrowing is expressible"
        );
        assert_eq!(
            narrowest_mode(BodyMode::Aggregate, Some(BodyMode::Cache)),
            BodyMode::Aggregate,
            "a widening override must not restore storage on an aggregating instance"
        );
        assert_eq!(
            narrowest_mode(BodyMode::Cache, Some(BodyMode::Cache)),
            BodyMode::Cache,
            "an override equal to the instance is the same answer"
        );
        assert_eq!(
            narrowest_mode(BodyMode::Aggregate, None),
            BodyMode::Aggregate,
            "no override means the instance decides"
        );

        let widened = ResolvedRetention {
            instance: BodyMode::Aggregate,
            source: Some(BodyMode::Cache),
            source_blocked: false,
            vanished: false,
        };
        assert_eq!(
            check_body_allowed(&widened, Some("ao3")),
            Err(RetentionRefusal {
                reason: RetentionReason::Aggregate,
                ..refusal()
            }),
            "the read side refuses a widening row even though the setter would have"
        );
    }

    /// A blocked source and a vanished origin outrank the mode.
    ///
    /// Both orders, because the ordering is the thing being asserted: a refused
    /// call that names `RETENTION_AGGREGATE` when the source is blocked sends
    /// the operator to change a setting that cannot unblock a source. The
    /// blocked arm is the *first* check, so it wins even when the instance is
    /// `Aggregate` too — which is the ambiguous case, and the reason it is in
    /// the table twice rather than once.
    #[test]
    fn a_blocked_source_and_a_vanished_origin_outrank_the_mode() {
        let blocked_while_caching = ResolvedRetention {
            instance: BodyMode::Cache,
            source: None,
            source_blocked: true,
            vanished: false,
        };
        assert_eq!(
            check_body_allowed(&blocked_while_caching, Some("ao3")),
            Err(RetentionRefusal {
                reason: RetentionReason::SourceBlocked,
                ..refusal()
            })
        );

        let blocked_while_aggregating = ResolvedRetention {
            instance: BodyMode::Aggregate,
            source: None,
            source_blocked: true,
            vanished: false,
        };
        assert_eq!(
            check_body_allowed(&blocked_while_aggregating, Some("ao3")),
            Err(RetentionRefusal {
                reason: RetentionReason::SourceBlocked,
                ..refusal()
            }),
            "an aggregating instance also has a blocked source; the reason is still the block"
        );

        let vanished = ResolvedRetention {
            instance: BodyMode::Cache,
            source: None,
            source_blocked: false,
            vanished: true,
        };
        assert_eq!(
            check_body_allowed(&vanished, Some("ao3")),
            Err(RetentionRefusal {
                reason: RetentionReason::Vanished,
                ..refusal()
            }),
            "a vanished origin is a dead end no setting recovers"
        );

        let both = ResolvedRetention {
            instance: BodyMode::Cache,
            source: None,
            source_blocked: true,
            vanished: true,
        };
        assert_eq!(
            check_body_allowed(&both, Some("ao3")),
            Err(RetentionRefusal {
                reason: RetentionReason::SourceBlocked,
                ..refusal()
            }),
            "the earlier check wins when both hold; the order is the decision"
        );
    }

    /// Every refusal names the source it was asked about.
    ///
    /// §11.15: "every refusal names the instance's policy rather than failing as
    /// a generic error". A message that says "aggregate" without saying *which*
    /// source is the generic error the sentence forbids, and on an instance with
    /// per-source overrides it is genuinely ambiguous.
    #[test]
    fn every_refusal_names_the_source_it_was_asked_about() {
        for reason in [
            RetentionReason::Aggregate,
            RetentionReason::AggregateSourceOverride,
            RetentionReason::SourceBlocked,
            RetentionReason::Vanished,
        ] {
            let named = reason.message(Some("archiveofourown"));
            assert!(
                named.contains("archiveofourown"),
                "{reason:?} did not name the source: {named}"
            );
            // And with no source named, it still says something rather than
            // rendering an empty sentence.
            assert!(!reason.message(None).is_empty());
        }
    }
}
