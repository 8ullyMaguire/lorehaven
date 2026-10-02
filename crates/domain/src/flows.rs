//! §53 — faucets and sinks: the economy as an operator can read it.
//!
//! A11's worry was inflation, and its contribution over §20.3's thirty-odd rows is
//! the *classification*: a faucet pays for signal or supply, a sink converts
//! credits into supply, and neither is undeclared. That last part is what makes
//! the classification worth a type — an undeclared mechanism is a mechanism an
//! operator cannot reason about, and a dashboard that silently omitted it would
//! report a smaller economy than exists.
//!
//! The load-bearing decision is that a mechanism **declares** its side rather than
//! having it inferred from the sign of its ledger entries. Inference would look
//! elegant and would fail exactly when it mattered: a bug in a faucet produces a
//! negative amount, which is precisely the case where "negative means sink" is
//! wrong.

use serde::{Deserialize, Serialize};

/// Which side of the loop a mechanism is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Flow {
    /// Pays out. A11: faucets pay for signal or supply.
    Faucet,
    /// Takes in. A11: sinks convert credits into supply.
    Sink,
    /// Neither — a transfer between buckets, or a mechanism that moves no credits.
    ///
    /// The third value is not a dodge. §20.1's ledger has buckets, and a
    /// hold-then-release moves an entry without changing any balance; classifying
    /// that as a faucet or a sink would put a phantom in the composition.
    Neutral,
    /// Nobody has said which side this mechanism is on.
    ///
    /// Distinct from [`Flow::Neutral`], and the distinction is the whole point.
    /// `Neutral` is a *claim* — somebody decided this mechanism has no side — and
    /// `Undeclared` is the absence of a claim. Rendering an unclassified mechanism
    /// as `Neutral` would put it in the composition looking settled, which is the
    /// failure §53.1 exists to prevent: the dashboard would report a smaller economy
    /// than exists, dressed as a considered answer.
    ///
    /// `sign` is `None` here too, and for a stronger reason than for `Neutral`: a
    /// mechanism whose side is unknown cannot be netted, and asking it to be would
    /// be asking the dashboard to guess.
    Undeclared,
}

impl Flow {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Faucet => "faucet",
            Self::Sink => "sink",
            Self::Neutral => "neutral",
            Self::Undeclared => "undeclared",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "faucet" => Some(Self::Faucet),
            "sink" => Some(Self::Sink),
            "neutral" => Some(Self::Neutral),
            "undeclared" => Some(Self::Undeclared),
            _ => None,
        }
    }

    /// The sign this flow's ledger entries carry, when it has any.
    ///
    /// `None` for [`Flow::Neutral`] and [`Flow::Undeclared`], and the `None` is the
    /// point in both cases: a transfer has no side, and an unclassified mechanism has
    /// no *known* side. Asking either for a sign is asking the dashboard to guess.
    #[must_use]
    pub fn sign(&self) -> Option<i64> {
        match self {
            Self::Faucet => Some(1),
            Self::Sink => Some(-1),
            Self::Neutral | Self::Undeclared => None,
        }
    }

    /// Whether this flow is an actual declaration rather than the absence of one.
    ///
    /// The predicate `FlowSummary::compose` counts on. It is a method on `Flow`
    /// rather than a check on [`MechanismDeclaration::is_valid`] because an
    /// undeclared mechanism is *not* an invalid declaration — it is a mechanism with
    /// no declaration, and conflating the two would mean fixing the wrong thing when
    /// either went wrong.
    #[must_use]
    pub const fn is_declared(&self) -> bool {
        !matches!(self, Self::Undeclared)
    }
}

/// *Why* a mechanism is on its side. A11's rule of thumb, as an enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Purpose {
    /// An action that tells the instance something it did not know: a reader
    /// finishing, a reader saying why, an import bringing a work in.
    Signal,
    /// Work, labour, or attention that makes the corpus better: an author
    /// publishing, a translator producing a chapter, a curator answering a
    /// drought.
    Supply,
    /// Both at once — the common case, and worth distinguishing because it is
    /// where an operator's judgement matters most.
    SignalAndSupply,
}

impl Purpose {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Signal => "signal",
            Self::Supply => "supply",
            Self::SignalAndSupply => "signal+supply",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "signal" => Some(Self::Signal),
            "supply" => Some(Self::Supply),
            "signal+supply" => Some(Self::SignalAndSupply),
            _ => None,
        }
    }
}

/// A mechanism's declaration.
///
/// `Flow::Neutral` may not carry a `Purpose`. A11's rule has nothing to say about
/// a transfer, and letting it claim "signal" would put the classification's
/// authority behind a declaration that means nothing — which is the failure mode
/// §53.1 exists to prevent, in the one case where the check is cheap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MechanismDeclaration {
    pub flow: Flow,
    pub purpose: Option<Purpose>,
}

impl MechanismDeclaration {
    pub const fn faucet(purpose: Purpose) -> Self {
        Self {
            flow: Flow::Faucet,
            purpose: Some(purpose),
        }
    }

    pub const fn sink() -> Self {
        Self {
            flow: Flow::Sink,
            purpose: Some(Purpose::Supply),
        }
    }

    pub const fn neutral() -> Self {
        Self {
            flow: Flow::Neutral,
            purpose: None,
        }
    }

    /// Whether the declaration is internally consistent.
    ///
    /// One rule, and it is a type-level rule expressed as a predicate so a
    /// migration CHECK and a unit test can both state it.
    pub fn is_valid(&self) -> bool {
        match self.flow {
            Flow::Neutral => self.purpose.is_none(),
            Flow::Faucet | Flow::Sink => self.purpose.is_some(),
            // An undeclared mechanism carries no purpose because nobody has said what it
            // is for, and it is *correct* for it to carry none. This is not the invalid
            // case `is_valid` exists to reject; it is the missing one. Counting it as
            // invalid would make `is_valid` lie about a declaration nobody made.
            Flow::Undeclared => self.purpose.is_none(),
        }
    }
}

/// Why a sink cannot declare `Signal`.
///
/// §0.3: "Credits and gamification rewards must never purchase trust, moderation
/// authority, or search ranking." A sink that spent credits to obtain *signal* —
/// to buy a rating, a review, or a vote — would be purchasing exactly that. A11's
/// rule says sinks convert credits into supply, and this is the reading that makes
/// the rule enforceable rather than aspirational.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RankPurchase;

/// A sink declaration that would buy ranking, which no declaration may be.
///
/// A dedicated uninhabited type rather than a returned error: the only way to
/// produce one is to name it, so the attempt is visible in a diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkRefusal {
    /// The sink declared a purpose other than supply.
    PurposeNotSupply(Purpose),
}

/// Classify a sink's declared purpose.
///
/// `Err` names the offending purpose, because "no" is not a useful review comment
/// and a11's rule needs a reason attached to it.
pub const fn classify_sink(purpose: Purpose) -> Result<(), SinkRefusal> {
    match purpose {
        Purpose::Supply => Ok(()),
        other => Err(SinkRefusal::PurposeNotSupply(other)),
    }
}

/// One mechanism as the dashboard reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mechanism {
    /// Stable identifier, matching the ledger's mechanism key.
    pub key: String,
    pub declaration: MechanismDeclaration,
    /// Net credits in the window, signed. Sinks are negative here regardless of
    /// which side the *declaration* says, so a misdeclared faucet shows up as a
    /// negative faucet rather than being reclassified.
    pub net_credits: i64,
}

/// What §53.2 says the dashboard shows: a balance and a composition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowSummary {
    pub faucet_credits: i64,
    pub sink_credits: i64,
    pub net_credits: i64,
    /// Undeclared mechanisms in the window. §53.1: a mechanism with no
    /// declaration is a mechanism the operator cannot reason about.
    pub undeclared: usize,
}

impl FlowSummary {
    /// Compose a window's mechanisms.
    ///
    /// `undeclared` is counted here rather than being a lookup, because the
    /// dashboard's whole value is that it cannot report a smaller economy than
    /// exists: a number that silently omits the mechanisms nobody classified is
    /// the inflation problem restated.
    pub fn compose(mechanisms: &[Mechanism]) -> Self {
        let mut faucet_credits = 0_i64;
        let mut sink_credits = 0_i64;
        let mut net_credits = 0_i64;
        let mut undeclared = 0_usize;
        for m in mechanisms {
            net_credits += m.net_credits;
            match m.declaration.flow {
                Flow::Faucet => faucet_credits += m.net_credits,
                Flow::Sink => sink_credits += m.net_credits,
                // An undeclared mechanism still lands in `net_credits` above, and must
                // land in neither side: putting it on a side would be the guess this
                // whole type exists to refuse.
                Flow::Neutral | Flow::Undeclared => {}
            }
            if !m.declaration.flow.is_declared() || !m.declaration.is_valid() {
                undeclared += 1;
            }
        }
        Self {
            faucet_credits,
            sink_credits,
            net_credits,
            undeclared,
        }
    }

    /// Whether the window crossed the operator's configured threshold.
    ///
    /// A warning and nothing more. §53.2: §0.3 makes bought ranking and bought
    /// trust non-negotiable, and a threshold that silently clamped would be the
    /// economy deciding what a reader may earn.
    pub fn exceeds(&self, threshold: i64) -> bool {
        self.net_credits > threshold
    }

    /// Whether this view carries any per-account information.
    ///
    /// Always false, and asserted as a method rather than left implicit: §53.2
    /// forbids per-account balances in an economy view, and §0.3 says readers
    /// control their data. A type that could not represent one would be better,
    /// but a *test* that fails if someone adds one is worth having today.
    pub const fn carries_account_detail(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(key: &str, flow: Flow, purpose: Option<Purpose>, net: i64) -> Mechanism {
        Mechanism {
            key: key.to_owned(),
            declaration: MechanismDeclaration { flow, purpose },
            net_credits: net,
        }
    }

    #[test]
    fn a_faucet_is_positive_a_sink_is_negative_and_neither_has_a_sign() {
        assert_eq!(Flow::Faucet.sign(), Some(1));
        assert_eq!(Flow::Sink.sign(), Some(-1));
        // The third value is load-bearing: a transfer has no side, and asking it
        // for one is a type error rather than a wrong answer.
        assert_eq!(Flow::Neutral.sign(), None);
    }

    #[test]
    fn a_neutral_declaration_may_not_claim_a_purpose() {
        assert!(MechanismDeclaration::neutral().is_valid());
        assert!(MechanismDeclaration::faucet(Purpose::Signal).is_valid());
        assert!(MechanismDeclaration::sink().is_valid());
        // A11's rule has nothing to say about a transfer, so letting one claim
        // "signal" would put the classification's authority behind nothing.
        let lying = MechanismDeclaration {
            flow: Flow::Neutral,
            purpose: Some(Purpose::Signal),
        };
        assert!(!lying.is_valid(), "a transfer cannot claim a purpose");
    }

    #[test]
    fn a_sink_may_only_convert_credits_into_supply() {
        assert!(classify_sink(Purpose::Supply).is_ok());
        // §0.3: credits must never purchase trust or ranking. A sink buying
        // *signal* would be purchasing exactly that, so it is refused by name.
        assert_eq!(
            classify_sink(Purpose::Signal),
            Err(SinkRefusal::PurposeNotSupply(Purpose::Signal))
        );
        assert_eq!(
            classify_sink(Purpose::SignalAndSupply),
            Err(SinkRefusal::PurposeNotSupply(Purpose::SignalAndSupply))
        );
    }

    #[test]
    fn the_composition_sums_faucets_and_sinks_separately() {
        let summary = FlowSummary::compose(&[
            m(
                "reader_completion",
                Flow::Faucet,
                Some(Purpose::Signal),
                5_000,
            ),
            m(
                "translation_bounty",
                Flow::Sink,
                Some(Purpose::Supply),
                -2_000,
            ),
            m("hold_release", Flow::Neutral, None, 0),
        ]);
        assert_eq!(summary.faucet_credits, 5_000);
        assert_eq!(summary.sink_credits, -2_000);
        assert_eq!(summary.net_credits, 3_000);
        assert_eq!(summary.undeclared, 0);
    }

    #[test]
    fn a_misdeclared_faucet_shows_up_negative_rather_than_reclassified() {
        // The reason the declaration is stored rather than inferred from the sign.
        // A bug in a faucet produces a negative amount; inferring would move it to
        // the sink side and the composition would look *correct*.
        let summary =
            FlowSummary::compose(&[m("buggy_faucet", Flow::Faucet, Some(Purpose::Signal), -100)]);
        assert_eq!(
            summary.faucet_credits, -100,
            "it stays on its declared side"
        );
        assert_eq!(summary.sink_credits, 0, "and does not migrate");
        assert_eq!(summary.net_credits, -100);
    }

    #[test]
    fn an_undeclared_mechanism_is_counted_rather_than_omitted() {
        // §53.1: a dashboard that silently omits unclassified mechanisms reports a
        // smaller economy than exists, which is the inflation problem restated.
        let summary = FlowSummary::compose(&[
            m("declared", Flow::Faucet, Some(Purpose::Signal), 100),
            m("forgot", Flow::Neutral, Some(Purpose::Supply), 999),
        ]);
        assert_eq!(summary.undeclared, 1);
        // And the phantom amount is still in the net, so it cannot be missed.
        assert_eq!(summary.net_credits, 1_099);
    }

    #[test]
    fn exceeding_a_threshold_is_a_warning_and_nothing_else() {
        // §53.2: no automatic throttle. §0.3 makes bought ranking and bought trust
        // non-negotiable, and a silent clamp would be the economy deciding what a
        // reader may earn.
        let minting = FlowSummary::compose(&[m(
            "reader_completion",
            Flow::Faucet,
            Some(Purpose::Signal),
            1_000_000,
        )]);
        assert!(minting.exceeds(50_000));
        // The method reports; nothing about it mutates the summary.
        assert_eq!(minting.net_credits, 1_000_000);
    }

    #[test]
    fn the_economy_view_carries_no_account_detail() {
        assert!(!FlowSummary::compose(&[]).carries_account_detail());
    }

    #[test]
    fn an_undeclared_mechanism_is_distinct_from_a_neutral_one() {
        // §53.1's distinction, and the reason `Flow` has four variants rather than three.
        // A neutral mechanism is one somebody *decided* has no side; an undeclared one is
        // a mechanism nobody has classified. Collapsing them would render a missing
        // declaration as a considered answer, and the dashboard would report a smaller
        // economy than exists while looking settled.
        assert!(Flow::Neutral.is_declared(), "neutral is a claim");
        assert!(
            !Flow::Undeclared.is_declared(),
            "undeclared is the absence of one"
        );
        assert!(Flow::Faucet.is_declared());
        assert!(Flow::Sink.is_declared());

        // And neither may carry a purpose, which is what makes `is_valid` agree with
        // `is_declared` for them: an undeclared mechanism has no purpose because nobody
        // has said what it is *for*.
        assert!(MechanismDeclaration::neutral().is_valid());
        assert!(MechanismDeclaration {
            flow: Flow::Undeclared,
            purpose: None,
        }
        .is_valid());
        assert!(
            !MechanismDeclaration {
                flow: Flow::Undeclared,
                purpose: Some(Purpose::Supply),
            }
            .is_valid(),
            "an undeclared mechanism cannot claim a purpose"
        );

        // Round trip, because the registry stores the string and the store reads it back.
        assert_eq!(Flow::parse("undeclared"), Some(Flow::Undeclared));
        assert_eq!(Flow::Undeclared.as_str(), "undeclared");
    }

    #[test]
    fn an_undeclared_mechanism_counts_without_being_netted_to_a_side() {
        // The load-bearing case for the new variant. A mechanism with no declaration still
        // moved credits, so its net is in the total — but it lands on neither the faucet nor
        // the sink side, because putting it on one is the guess this whole type refuses to
        // make. The old behaviour (rendering it `Neutral`) produced *identical* numbers here,
        // which is exactly why this test exists: `undeclared` was the only thing that
        // distinguished them, and it was silently zero.
        let summary = FlowSummary::compose(&[
            m(
                "author_earnings",
                Flow::Faucet,
                Some(Purpose::Supply),
                1_000,
            ),
            m("undeclared:grant:someone", Flow::Undeclared, None, 7_500),
        ]);
        assert_eq!(summary.undeclared, 1, "it is counted, not dropped");
        assert_eq!(summary.net_credits, 8_500, "and its credits are still real");
        assert_eq!(summary.faucet_credits, 1_000, "but it is on neither side");
        assert_eq!(summary.sink_credits, 0);
    }

    #[test]
    fn a_mechanism_with_an_invalid_declaration_is_also_counted_as_undeclared() {
        // The other half of the compose predicate: `!is_declared() || !is_valid()`. A
        // declaration that *was* made and is internally inconsistent (a faucet claiming no
        // purpose) is as unreportable as one that was never made, so both increment.
        let summary = FlowSummary::compose(&[
            m("forgotten_faucet", Flow::Faucet, None, 100),
            m("declared", Flow::Sink, Some(Purpose::Supply), -50),
        ]);
        assert_eq!(summary.undeclared, 1);
        assert_eq!(summary.net_credits, 50);
    }
}
