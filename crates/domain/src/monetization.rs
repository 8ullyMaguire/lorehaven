//! Monetization domain (spec §20.9) — pure rules, no I/O.

use serde::{Deserialize, Serialize};

/// Instance-level eligibility setting (§20.9.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eligibility {
    /// Only works the author declares original may carry a price.
    Original,
    /// Fanworks too, but a stored rights assertion is demanded.
    AnyWithAssertion,
    /// No monetization; credit tips still work.
    Disabled,
}

impl Eligibility {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "original" => Some(Self::Original),
            "any-with-assertion" => Some(Self::AnyWithAssertion),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }
}

/// What may be sold (§20.9.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    Tips,
    EarlyAccess,
    Purchase,
    Patronage,
}

impl Model {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "tips" => Some(Self::Tips),
            "early_access" => Some(Self::EarlyAccess),
            "purchase" => Some(Self::Purchase),
            "patronage" => Some(Self::Patronage),
            _ => None,
        }
    }
}

/// One structural rule per §20.9.3 clause, each a pure decision.
pub struct Rules;

impl Rules {
    /// Credits are never convertible to money by the platform (§20.9.2).
    pub fn credits_convert_to_money() -> bool {
        false
    }

    /// Imported works are never monetizable in `original` mode, and never when
    /// monetization is globally disabled (§20.9.1).
    pub fn imported_work_monetizable(eligibility: Eligibility) -> bool {
        !matches!(eligibility, Eligibility::Original | Eligibility::Disabled)
    }

    /// `any-with-assertion` demands a stored assertion; `original` demands an
    /// `original` assertion recorded at pricing time (§20.9.1).
    pub fn assertion_required(eligibility: Eligibility) -> bool {
        !matches!(eligibility, Eligibility::Disabled)
    }

    /// Early access is a scheduled unlock, not a lock (§20.9.2).
    /// `unlock_epoch_seconds` is the absolute Unix timestamp at which the work
    /// becomes public; `now_epoch_seconds` is the current time.
    pub fn early_access_unlocked(unlock_epoch_seconds: i64, now_epoch_seconds: i64) -> bool {
        now_epoch_seconds >= unlock_epoch_seconds
    }

    /// A priced work gains no ranking advantage (§20.9.3). Always false; the
    /// type exists so ranking code has a rule to call rather than a comment
    /// to ignore.
    pub fn ranking_boost_for_paid() -> bool {
        false
    }

    /// Purchases and tips between pseuds of one account are refused (§20.9.3).
    pub fn self_dealing(payer_account: &str, author_account: &str) -> bool {
        payer_account == author_account
    }

    /// Platform fee share: 85/15 in the author's favour by default (§20.9.3).
    /// Returns (author_minor, platform_minor).
    pub fn split(amount_minor: i64, platform_fee_bp: i64) -> (i64, i64) {
        let platform = amount_minor * platform_fee_bp / 10_000;
        (amount_minor - platform, platform)
    }

    /// Processor fee (§20.9.5): net = gross − fee, never negative.
    pub fn net_after_fee(gross_minor: i64, fee_minor: i64) -> i64 {
        (gross_minor - fee_minor).max(0)
    }
}

// ---------------------------------------------------------------------------
// AI content declaration (spec §20.9.4)
// ---------------------------------------------------------------------------

/// AI usage declaration: none | assisted | co-written | generated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AiDeclaration {
    None,
    Assisted,
    CoWritten,
    Generated,
}

impl AiDeclaration {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "none" => Some(Self::None),
            "assisted" => Some(Self::Assisted),
            "co-written" => Some(Self::CoWritten),
            "generated" => Some(Self::Generated),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Assisted => "assisted",
            Self::CoWritten => "co-written",
            Self::Generated => "generated",
        }
    }

    /// Pool B multiplier (§20.9.4): none/assisted 1.0, co-written 0.3,
    /// generated 0 (Pool B-ineligible). In basis points.
    pub fn pool_b_multiplier_bp(self) -> i64 {
        match self {
            Self::None | Self::Assisted => 10_000,
            Self::CoWritten => 3_000,
            Self::Generated => 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Redistribution pools (spec §20.10)
// ---------------------------------------------------------------------------

/// Per-flow pool contribution (§20.10.6): what fraction of a flow's net
/// amount goes to Pool A (the rest to Pool B).
#[derive(Debug, Clone, Copy)]
pub struct PoolSplit {
    /// Fraction of net revenue to Pool A, in basis points (0–10_000).
    pub pool_a_bp: i64,
}

impl Default for PoolSplit {
    fn default() -> Self {
        Self { pool_a_bp: 8_500 }
    }
}

impl PoolSplit {
    /// Split a net amount into (pool_a, pool_b). Never loses a unit.
    pub fn split(&self, net_minor: i64) -> (i64, i64) {
        if net_minor <= 0 {
            return (net_minor, 0);
        }
        let a = net_minor * self.pool_a_bp / 10_000;
        (a, net_minor - a)
    }
}

/// Graduated cap on trailing active-earner median (§20.10.3).
#[derive(Debug, Clone, Copy)]
pub struct GraduatedCap {
    pub median_minor: i64,
    pub band1_multiple: i64,
    pub band2_multiple: i64,
}

impl Default for GraduatedCap {
    fn default() -> Self {
        Self {
            median_minor: 0,
            band1_multiple: 5,
            band2_multiple: 10,
        }
    }
}

impl GraduatedCap {
    /// How much of a Pool A flow is kept in Pool A vs. spilled to Pool B.
    pub fn apply(&self, flow_minor: i64, already_earned_minor: i64) -> (i64, i64) {
        if self.median_minor <= 0 || flow_minor <= 0 {
            return (flow_minor, 0);
        }
        let t1 = self.median_minor * self.band1_multiple;
        let t2 = self.median_minor * self.band2_multiple;
        let before = already_earned_minor;
        let after = before + flow_minor;

        if after <= t1 {
            // Entirely below first band: all Pool A.
            (flow_minor, 0)
        } else if before >= t2 {
            // Already past second band: all Pool B.
            (0, flow_minor)
        } else if before >= t1 && after <= t2 {
            // Entirely within the spill zone: all Pool B.
            (0, flow_minor)
        } else if before < t1 {
            // Crossing the first band: fill up to t1, spill the rest.
            let kept = t1 - before;
            (kept, flow_minor - kept)
        } else {
            // Crossing the second band (before >= t1, after > t2): fill up to t2.
            let kept = t2 - before;
            (kept, flow_minor - kept)
        }
    }
}

/// Quality weights for Pool B distribution (§20.10.4).
#[derive(Debug, Clone, Copy)]
pub struct QualityWeights {
    pub rating_bp: i64,
    pub review_bp: i64,
    pub completion_bp: i64,
    pub retention_bp: i64,
}

impl Default for QualityWeights {
    fn default() -> Self {
        Self {
            rating_bp: 4_000,
            review_bp: 3_000,
            completion_bp: 2_000,
            retention_bp: 1_000,
        }
    }
}

impl QualityWeights {
    /// Composite quality score in basis points, clamped to 10_000.
    pub fn score_bp(&self, rating: i64, review: i64, completion: i64, retention: i64) -> i64 {
        let total = (rating * self.rating_bp
            + review * self.review_bp
            + completion * self.completion_bp
            + retention * self.retention_bp)
            / 10_000;
        total.clamp(0, 10_000)
    }
}

/// Pool B distribution (§20.10.4): quality-weighted, never volume.
pub fn distribute_pool_b(
    pool_b_minor: i64,
    shares: &[(String, i64, i64)], // (author_account, quality_bp, ai_multiplier_bp)
) -> Vec<(String, i64)> {
    if pool_b_minor <= 0 || shares.is_empty() {
        return shares.iter().map(|(a, _, _)| (a.clone(), 0)).collect();
    }
    let weights: Vec<i64> = shares
        .iter()
        .map(|(_, q, ai)| (q * ai).max(0))
        .collect();
    let total_weight: i64 = weights.iter().sum();
    if total_weight == 0 {
        let each = pool_b_minor / shares.len() as i64;
        let mut out: Vec<(String, i64)> = shares
            .iter()
            .map(|(a, _, _)| (a.clone(), each))
            .collect();
        out[0].1 += pool_b_minor - each * shares.len() as i64;
        return out;
    }
    let mut out = Vec::with_capacity(shares.len());
    let mut allocated = 0;
    for (i, (author, _, _)) in shares.iter().enumerate() {
        let amount = if i == shares.len() - 1 {
            pool_b_minor - allocated
        } else {
            let a = pool_b_minor * weights[i] / total_weight;
            allocated += a;
            a
        };
        out.push((author.clone(), amount));
    }
    out
}

/// Pool B eligibility floor (§20.10.5).
#[derive(Debug, Clone, Copy)]
pub struct PoolBFloor {
    pub min_distinct_readers: i64,
    pub min_age_days: i64,
    pub min_trust_level: i64,
}

impl Default for PoolBFloor {
    fn default() -> Self {
        Self {
            min_distinct_readers: 5,
            min_age_days: 30,
            min_trust_level: 1,
        }
    }
}

/// Visible exclusion reason when an author fails the floor (§20.10.5).
pub fn pool_b_exclusion_reason(
    floor: &PoolBFloor,
    distinct_readers: i64,
    age_days: i64,
    trust_level: i64,
    sanctioned: bool,
    unresolved_ai_flag: bool,
) -> Option<&'static str> {
    if sanctioned {
        Some("active sanction")
    } else if unresolved_ai_flag {
        Some("unresolved AI declaration flag")
    } else if distinct_readers < floor.min_distinct_readers {
        Some("too few distinct readers")
    } else if age_days < floor.min_age_days {
        Some("account too young")
    } else if trust_level < floor.min_trust_level {
        Some("trust level below floor")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credits_never_convert_to_money() {
        assert!(!Rules::credits_convert_to_money());
    }

    #[test]
    fn imported_works_never_monetizable_in_original_mode() {
        assert!(!Rules::imported_work_monetizable(Eligibility::Original));
        assert!(!Rules::imported_work_monetizable(Eligibility::Disabled));
        assert!(Rules::imported_work_monetizable(
            Eligibility::AnyWithAssertion
        ));
    }

    #[test]
    fn early_access_is_a_scheduled_unlock() {
        assert!(!Rules::early_access_unlocked(1_000, 999));
        assert!(Rules::early_access_unlocked(1_000, 1_000));
    }

    #[test]
    fn paid_works_gain_no_ranking() {
        assert!(!Rules::ranking_boost_for_paid());
    }

    #[test]
    fn self_dealing_is_between_pseuds_of_one_account() {
        assert!(Rules::self_dealing("a", "a"));
        assert!(!Rules::self_dealing("a", "b"));
    }

    #[test]
    fn split_is_eighty_five_fifteen_by_default() {
        let (author, platform) = Rules::split(10_000, 1_500);
        assert_eq!((author, platform), (8_500, 1_500));
    }

    #[test]
    fn parse_round_trips() {
        assert_eq!(Eligibility::parse("original"), Some(Eligibility::Original));
        assert_eq!(Model::parse("early_access"), Some(Model::EarlyAccess));
        assert_eq!(Eligibility::parse("other"), None);
    }

    #[test]
    fn net_after_fee_is_non_negative() {
        assert_eq!(Rules::net_after_fee(10_000, 1_500), 8_500);
        assert_eq!(Rules::net_after_fee(100, 200), 0);
    }

    #[test]
    fn ai_declaration_multipliers() {
        assert_eq!(AiDeclaration::None.pool_b_multiplier_bp(), 10_000);
        assert_eq!(AiDeclaration::Assisted.pool_b_multiplier_bp(), 10_000);
        assert_eq!(AiDeclaration::CoWritten.pool_b_multiplier_bp(), 3_000);
        assert_eq!(AiDeclaration::Generated.pool_b_multiplier_bp(), 0);
    }

    #[test]
    fn ai_declaration_parses_and_round_trips() {
        for s in ["none", "assisted", "co-written", "generated"] {
            let d = AiDeclaration::parse(s).expect(s);
            assert_eq!(d.as_str(), s);
        }
        assert!(AiDeclaration::parse("maybe").is_none());
    }

    #[test]
    fn pool_split_default_is_85_15() {
        let split = PoolSplit::default();
        assert_eq!(split.split(10_000), (8_500, 1_500));
        assert_eq!(split.split(0), (0, 0));
        let (a, b) = split.split(9_999);
        assert_eq!(a + b, 9_999);
    }

    #[test]
    fn graduated_cap_bands() {
        let cap = GraduatedCap {
            median_minor: 1_000,
            ..Default::default()
        };
        assert_eq!(cap.apply(4_000, 0), (4_000, 0));
        assert_eq!(cap.apply(2_000, 4_000), (1_000, 1_000));
        assert_eq!(cap.apply(500, 6_000), (0, 500));
        assert_eq!(cap.apply(2_000, 9_000), (1_000, 1_000));
        assert_eq!(cap.apply(500, 10_000), (0, 500));
        let off = GraduatedCap::default();
        assert_eq!(off.apply(1_000, 0), (1_000, 0));
    }

    #[test]
    fn quality_weights_composite() {
        let w = QualityWeights::default();
        assert_eq!(w.score_bp(10_000, 10_000, 10_000, 10_000), 10_000);
        assert_eq!(w.score_bp(0, 0, 0, 0), 0);
        assert_eq!(w.score_bp(5_000, 5_000, 5_000, 5_000), 5_000);
    }

    #[test]
    fn pool_b_floor_exclusion_reasons() {
        let floor = PoolBFloor::default();
        assert_eq!(
            pool_b_exclusion_reason(&floor, 10, 100, 1, false, false),
            None
        );
        assert_eq!(
            pool_b_exclusion_reason(&floor, 3, 100, 1, false, false),
            Some("too few distinct readers")
        );
        assert_eq!(
            pool_b_exclusion_reason(&floor, 10, 10, 1, false, false),
            Some("account too young")
        );
        assert_eq!(
            pool_b_exclusion_reason(&floor, 10, 100, 0, false, false),
            Some("trust level below floor")
        );
        assert_eq!(
            pool_b_exclusion_reason(&floor, 10, 100, 1, true, false),
            Some("active sanction")
        );
        assert_eq!(
            pool_b_exclusion_reason(&floor, 10, 100, 1, false, true),
            Some("unresolved AI declaration flag")
        );
    }

    #[test]
    fn pool_b_distribution_is_quality_weighted_and_lossless() {
        let shares = vec![
            ("a".to_string(), 10_000, 10_000),
            ("b".to_string(), 5_000, 10_000),
            ("c".to_string(), 10_000, 3_000),
        ];
        let out = distribute_pool_b(10_000, &shares);
        assert_eq!(out.len(), 3);
        let total: i64 = out.iter().map(|(_, a)| a).sum();
        assert_eq!(total, 10_000, "no unit lost");
        assert!(out[0].1 > out[1].1, "higher quality earns more");
        assert!(out[0].1 > out[2].1, "AI penalty lowers the share");
    }

    #[test]
    fn pool_b_distribution_equal_when_all_zero() {
        let shares = vec![
            ("a".to_string(), 0, 0),
            ("b".to_string(), 0, 0),
        ];
        let out = distribute_pool_b(100, &shares);
        assert_eq!(out[0].1, 50);
        assert_eq!(out[1].1, 50);
    }
}
