//! Controlled digital lending (spec §32.4, M25).
//!
//! An instance operator may enable lending. When off, rights metadata is served
//! and loan requests are refused with a policy error. When on, a work with
//! `lending_class = 'lending'` may be loaned to one reader at a time: a bounded
//! window, expiry, revocation, and a configured cap on concurrent copies.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A loan grant: one reader holds one work for a bounded window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loan {
    pub id: String,
    pub work_id: String,
    pub borrower_account_id: String,
    pub granted_at: String,
    pub expires_at: String,
    pub revoked_at: Option<String>,
    pub copy_number: u32,
}

impl Loan {
    pub fn is_active(&self) -> bool {
        self.revoked_at.is_none()
    }
}

/// Errors from the lending domain.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LendingError {
    #[error("lending is disabled on this instance")]
    Disabled,
    #[error("work is not lendable")]
    NotLendable,
    #[error("work is already on loan to another reader")]
    AlreadyOnLoan,
    #[error("lending cap reached: no copies available")]
    CapReached,
    #[error("borrower has reached their loan limit")]
    BorrowerLimitReached,
    #[error("loan has expired")]
    Expired,
    #[error("loan has been revoked")]
    Revoked,
}

/// Validate that a loan request is permissible.
pub fn validate_loan_request(
    lending_enabled: bool,
    work_lending_class: &str,
    work_is_published: bool,
    active_loans: u32,
    copy_cap: u32,
) -> Result<(), LendingError> {
    if !lending_enabled {
        return Err(LendingError::Disabled);
    }
    if work_lending_class != "lending" {
        return Err(LendingError::NotLendable);
    }
    if !work_is_published {
        return Err(LendingError::NotLendable);
    }
    if active_loans >= copy_cap {
        return Err(LendingError::CapReached);
    }
    Ok(())
}

/// Check whether a loan has expired given the current time.
pub fn loan_is_expired(loan: &Loan, now: &str) -> bool {
    loan.expires_at.as_str() < now
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_loan_request_happy_path() {
        assert!(validate_loan_request(true, "lending", true, 0, 3).is_ok());
    }

    #[test]
    fn validate_lending_disabled() {
        assert_eq!(
            validate_loan_request(false, "lending", true, 0, 3),
            Err(LendingError::Disabled)
        );
    }

    #[test]
    fn validate_not_lendable_class() {
        assert_eq!(
            validate_loan_request(true, "none", true, 0, 3),
            Err(LendingError::NotLendable)
        );
    }

    #[test]
    fn validate_not_published() {
        assert_eq!(
            validate_loan_request(true, "lending", false, 0, 3),
            Err(LendingError::NotLendable)
        );
    }

    #[test]
    fn validate_cap_reached() {
        assert_eq!(
            validate_loan_request(true, "lending", true, 3, 3),
            Err(LendingError::CapReached)
        );
    }

    #[test]
    fn validate_under_cap() {
        // Two active loans with cap of three: one more allowed.
        assert!(validate_loan_request(true, "lending", true, 2, 3).is_ok());
    }

    #[test]
    fn loan_is_expired_past() {
        let loan = Loan {
            id: "l1".into(),
            work_id: "w1".into(),
            borrower_account_id: "a1".into(),
            granted_at: "2024-01-01T00:00:00Z".into(),
            expires_at: "2024-01-15T00:00:00Z".into(),
            revoked_at: None,
            copy_number: 1,
        };
        assert!(loan_is_expired(&loan, "2024-01-16T00:00:00Z"));
    }

    #[test]
    fn loan_is_expired_before() {
        let loan = Loan {
            id: "l1".into(),
            work_id: "w1".into(),
            borrower_account_id: "a1".into(),
            granted_at: "2024-01-01T00:00:00Z".into(),
            expires_at: "2024-01-15T00:00:00Z".into(),
            revoked_at: None,
            copy_number: 1,
        };
        assert!(!loan_is_expired(&loan, "2024-01-14T00:00:00Z"));
    }
}
