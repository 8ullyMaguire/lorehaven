//! M16 — Marketplace domain: listings and commissions state machines.

/// Listing kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListingKind {
    PaidWork,
    Commission,
    Ask,
}

impl ListingKind {
    /// Inverse of [`Self::as_str`]; the tests pin the round trip.
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "paid_work" => Ok(Self::PaidWork),
            "commission" => Ok(Self::Commission),
            "ask" => Ok(Self::Ask),
            _ => Err(format!("unknown ListingKind: {s}")),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PaidWork => "paid_work",
            Self::Commission => "commission",
            Self::Ask => "ask",
        }
    }
}

impl std::str::FromStr for ListingKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "paid_work" => Ok(Self::PaidWork),
            "commission" => Ok(Self::Commission),
            "ask" => Ok(Self::Ask),
            _ => Err(format!("unknown listing kind: {s}")),
        }
    }
}

/// Listing states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListingState {
    Draft,
    Active,
    Paused,
    Closed,
}

impl ListingState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Closed => "closed",
        }
    }
}

/// Commission states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommissionState {
    Quoted,
    Accepted,
    InProgress,
    Delivered,
    AcceptedFinal,
    Refunded,
    Disputed,
}

impl CommissionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Quoted => "quoted",
            Self::Accepted => "accepted",
            Self::InProgress => "in_progress",
            Self::Delivered => "delivered",
            Self::AcceptedFinal => "accepted_final",
            Self::Refunded => "refunded",
            Self::Disputed => "disputed",
        }
    }
}

impl std::str::FromStr for CommissionState {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "quoted" => Ok(Self::Quoted),
            "accepted" => Ok(Self::Accepted),
            "in_progress" => Ok(Self::InProgress),
            "delivered" => Ok(Self::Delivered),
            "accepted_final" => Ok(Self::AcceptedFinal),
            "refunded" => Ok(Self::Refunded),
            "disputed" => Ok(Self::Disputed),
            _ => Err(format!("unknown commission state: {s}")),
        }
    }
}

/// Validate a commission state transition.
pub fn valid_commission_transition(from: &CommissionState, to: &CommissionState) -> bool {
    matches!(
        (from, to),
        (CommissionState::Quoted, CommissionState::Accepted)
            | (CommissionState::Quoted, CommissionState::Disputed)
            | (CommissionState::Accepted, CommissionState::InProgress)
            | (CommissionState::Accepted, CommissionState::Disputed)
            | (CommissionState::InProgress, CommissionState::Delivered)
            | (CommissionState::InProgress, CommissionState::Disputed)
            | (CommissionState::Delivered, CommissionState::AcceptedFinal)
            | (CommissionState::Delivered, CommissionState::Disputed)
            | (CommissionState::Disputed, CommissionState::Refunded)
            | (CommissionState::Disputed, CommissionState::Accepted)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_kind_round_trip() {
        for k in [ListingKind::PaidWork, ListingKind::Commission, ListingKind::Ask] {
            assert_eq!(ListingKind::from_str(k.as_str()).unwrap(), k);
        }
    }

    #[test]
    fn happy_path_commission() {
        use CommissionState::*;
        assert!(valid_commission_transition(&Quoted, &Accepted));
        assert!(valid_commission_transition(&Accepted, &InProgress));
        assert!(valid_commission_transition(&InProgress, &Delivered));
        assert!(valid_commission_transition(&Delivered, &AcceptedFinal));
    }

    #[test]
    fn refund_path() {
        use CommissionState::*;
        assert!(valid_commission_transition(&Delivered, &Disputed));
        assert!(valid_commission_transition(&Disputed, &Refunded));
    }

    #[test]
    fn invalid_transitions() {
        use CommissionState::*;
        assert!(!valid_commission_transition(&AcceptedFinal, &Quoted));
        assert!(!valid_commission_transition(&Refunded, &Accepted));
    }
}
