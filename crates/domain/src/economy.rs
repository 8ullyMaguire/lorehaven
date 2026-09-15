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
        ] {
            assert_eq!(TxnType::from_str(t.as_str()).unwrap(), t);
        }
    }
}
