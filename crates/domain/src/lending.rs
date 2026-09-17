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
    /// When the periodic sweep recorded that the window had closed.
    ///
    /// Distinct from `expires_at` being in the past: the timestamp is when the
    /// instance *noticed*, which is what a reader's history and an operator's
    /// sweep report read. A loan whose window has passed but which no sweep has
    /// reached yet has `expired_at: None` and is already inactive.
    pub expired_at: Option<String>,
    pub copy_number: u32,
}

impl Loan {
    /// Whether the loan is live at `now`.
    ///
    /// The time is a parameter because it is a fact about the clock, not about
    /// the row: a loan does not become expired by being read, and a function
    /// that consulted the clock itself could not be asked about a past moment.
    #[must_use]
    pub fn is_active_at(&self, now: &str) -> bool {
        self.revoked_at.is_none() && !loan_is_expired(self, now)
    }

    /// How this loan reads at `now`: `active`, `expired` or `revoked`.
    ///
    /// Revocation wins over expiry: a revoked loan that would also have expired
    /// is reported as revoked, because that is the thing a reader did.
    #[must_use]
    pub fn state_at(&self, now: &str) -> &'static str {
        if self.revoked_at.is_some() {
            "revoked"
        } else if loan_is_expired(self, now) {
            "expired"
        } else {
            "active"
        }
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
            expired_at: None,
            copy_number: 1,
        };
        assert!(loan_is_expired(&loan, "2024-01-16T00:00:00Z"));
    }

    #[test]
    fn loan_is_expired_before() {
        let loan = loan(None);
        assert!(!loan_is_expired(&loan, "2024-01-14T00:00:00Z"));
    }

    fn loan(revoked_at: Option<&str>) -> Loan {
        Loan {
            id: "l1".into(),
            work_id: "w1".into(),
            borrower_account_id: "a1".into(),
            granted_at: "2024-01-01T00:00:00Z".into(),
            expires_at: "2024-01-15T00:00:00Z".into(),
            revoked_at: revoked_at.map(str::to_owned),
            expired_at: None,
            copy_number: 1,
        }
    }

    #[test]
    fn a_loan_past_its_window_is_not_active() {
        // The bug this pins: an `is_active` that only asked about revocation
        // called an expired loan live, which is the opposite of what the door
        // and the cap need from it.
        let loan = loan(None);
        assert!(loan.is_active_at("2024-01-14T00:00:00Z"));
        assert!(!loan.is_active_at("2024-01-16T00:00:00Z"));
        assert_eq!(loan.state_at("2024-01-14T00:00:00Z"), "active");
        assert_eq!(loan.state_at("2024-01-16T00:00:00Z"), "expired");
    }

    #[test]
    fn revocation_wins_over_expiry_in_the_reported_state() {
        let loan = loan(Some("2024-01-10T00:00:00Z"));
        assert_eq!(loan.state_at("2024-01-16T00:00:00Z"), "revoked");
        assert!(!loan.is_active_at("2024-01-11T00:00:00Z"));
    }
}
