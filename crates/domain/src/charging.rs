//! M15 — Job charging domain: quote → reserve → submit → complete → capture.

/// Job charging states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChargeState {
    Quoted,
    Reserved,
    Submitted,
    Completed,
    Failed,
}

impl ChargeState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Quoted => "quoted",
            Self::Reserved => "reserved",
            Self::Submitted => "submitted",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

/// Validate a state transition.
pub fn valid_transition(from: ChargeState, to: ChargeState) -> bool {
    matches!(
        (from, to),
        (ChargeState::Quoted, ChargeState::Reserved)
            | (ChargeState::Quoted, ChargeState::Failed)
            | (ChargeState::Reserved, ChargeState::Submitted)
            | (ChargeState::Reserved, ChargeState::Failed)
            | (ChargeState::Submitted, ChargeState::Completed)
            | (ChargeState::Submitted, ChargeState::Failed)
    )
}

/// The capture rule: actual charge may be less than or equal to the quote,
/// but never more without a new quote.
pub fn capture_allowed(reserved: i64, actual: i64) -> bool {
    actual <= reserved && actual >= 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_transitions() {
        assert!(valid_transition(ChargeState::Quoted, ChargeState::Reserved));
        assert!(valid_transition(ChargeState::Reserved, ChargeState::Submitted));
        assert!(valid_transition(ChargeState::Submitted, ChargeState::Completed));
    }

    #[test]
    fn failure_transitions() {
        assert!(valid_transition(ChargeState::Quoted, ChargeState::Failed));
        assert!(valid_transition(ChargeState::Reserved, ChargeState::Failed));
        assert!(valid_transition(ChargeState::Submitted, ChargeState::Failed));
    }

    #[test]
    fn invalid_transitions() {
        assert!(!valid_transition(ChargeState::Completed, ChargeState::Quoted));
        assert!(!valid_transition(ChargeState::Failed, ChargeState::Submitted));
    }

    #[test]
    fn capture_at_or_under_reserve() {
        assert!(capture_allowed(100, 80));
        assert!(capture_allowed(100, 100));
        assert!(!capture_allowed(100, 120));
        assert!(!capture_allowed(100, -1));
    }
}
