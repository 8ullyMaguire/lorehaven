//! Monetization domain (spec §20.9) — pure rules, no I/O.
//!
//! The skeleton ships the *rules* the implementation must honour, as typed
//! signatures; bodies are `todo!()`-free stubs returning [`Todo`] so the
//! crate compiles and the contracts are testable. The implementing agent
//! replaces the bodies; the signatures are the contract.

use crate::error::AppError;

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

    /// Imported works are never monetizable in `original` mode (§20.9.1).
    pub fn imported_work_monetizable(eligibility: Eligibility) -> bool {
        !matches!(eligibility, Eligibility::Original)
    }

    /// `any-with-assertion` demands a stored assertion; `original` demands an
    /// `original` assertion recorded at pricing time (§20.9.1).
    pub fn assertion_required(eligibility: Eligibility) -> bool {
        !matches!(eligibility, Eligibility::Disabled)
    }

    /// Early access is a scheduled unlock, not a lock (§20.9.2).
    pub fn early_access_unlocked(public_at_offset_seconds: i64, now_epoch: i64) -> bool {
        now_epoch >= public_at_offset_seconds
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

    /// Skeleton marker: the implementing agent replaces stub bodies.
    pub fn todo() -> AppError {
        AppError::NotImplemented
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
        assert!(Rules::imported_work_monetizable(Eligibility::AnyWithAssertion));
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
}
