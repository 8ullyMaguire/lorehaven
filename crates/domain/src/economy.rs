//! M15 — Ledger domain: transaction invariants, bucket precedence.

/// Credit buckets from most- to least-preferred for spending
pub const BUCKET_PRECEDENCE: &[&str] = &["held", "earned", "granted", "purchased"];

/// Credit transaction types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxnType {
    Earn,
    Spend,
    Grant,
    Purchase,
    Hold,
    Release,
    Capture,
    /// A preservation reward, paid when a destination is *verified* carrying a
    /// work (spec §2.1).
    ///
    /// A type of its own rather than `Grant` because the clawback has to find
    /// it: `credit_transactions.type` is free TEXT, so a `preservation` row is
    /// distinguishable from every other award without a schema change, and a
    /// reader auditing their balance can see exactly which credits came from
    /// preservation and therefore which a dead destination takes back.
    ///
    /// `credits` is a single-credit codebase and this is the seventh type, but
    /// the alternative — folding it into `Grant` — makes the reversal
    /// unauditable, which §2.3 explicitly requires: a reader who already spent
    /// the credits has to see the debt, and that needs a row of its own.
    Preservation,
    /// The clawback for a `Preservation` whose destination stopped answering.
    ///
    /// A separate type and not a negative `Preservation`, so that
    /// `WHERE type = 'preservation'` remains a complete statement of what was
    /// paid out and the net is the sum of the two. The pair is
    /// `preservation` + `preservation_reclaim`, and the idempotency keys are
    /// `preservation:grant:<target>` and `preservation:reclaim:<target>`, so
    /// both halves are individually replay-safe.
    PreservationReclaim,
}

impl TxnType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Earn => "earn",
            Self::Spend => "spend",
            Self::Grant => "grant",
            Self::Purchase => "purchase",
            Self::Hold => "hold",
            Self::Release => "release",
            Self::Capture => "capture",
            Self::Preservation => "preservation",
            Self::PreservationReclaim => "preservation_reclaim",
        }
    }
}

impl std::str::FromStr for TxnType {
    type Err = String;
    /// Inverse of [`TxnType::as_str`]; the tests pin the round trip.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "earn" => Ok(Self::Earn),
            "spend" => Ok(Self::Spend),
            "grant" => Ok(Self::Grant),
            "purchase" => Ok(Self::Purchase),
            "hold" => Ok(Self::Hold),
            "release" => Ok(Self::Release),
            "capture" => Ok(Self::Capture),
            "preservation" => Ok(Self::Preservation),
            "preservation_reclaim" => Ok(Self::PreservationReclaim),
            _ => Err(format!("unknown txn type: {s}")),
        }
    }
}

/// Validate that a set of credit entries sums to zero (balanced).
pub fn entries_balanced(entries: &[(String, i64)]) -> bool {
    entries.iter().map(|(_, amt)| amt).sum::<i64>() == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn balanced_entries_sum_to_zero() {
        let entries = vec![("alice".to_string(), 100), ("bob".to_string(), -100)];
        assert!(entries_balanced(&entries));
    }

    #[test]
    fn unbalanced_entries_fail() {
        let entries = vec![("alice".to_string(), 100), ("bob".to_string(), -90)];
        assert!(!entries_balanced(&entries));
    }

    #[test]
    fn txn_type_round_trip() {
        for t in [
            TxnType::Earn,
            TxnType::Spend,
            TxnType::Grant,
            TxnType::Purchase,
            TxnType::Hold,
            TxnType::Release,
            TxnType::Capture,
            TxnType::Preservation,
            TxnType::PreservationReclaim,
        ] {
            assert_eq!(TxnType::from_str(t.as_str()).unwrap(), t);
        }
    }
}
