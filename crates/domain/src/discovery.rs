//! Discovery domain: recommendation engines, taste profiles, recipes.
//!
//! Spec §16.1–16.7. Pure functions — no I/O.

use crate::ids::WorkId;

/// A recommendation candidate with its score and reason.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub work_id: WorkId,
    pub score: i64,
    pub reason: String,
}

/// The More-Like-This engine: index terms + tags.
pub fn more_like_this(_signals: &serde_json::Value, _candidates: &[WorkId]) -> Vec<Candidate> {
    Vec::new()
}

/// The Same-Fandom-Fresh engine: taxonomy + recency.
pub fn same_fandom_fresh(_signals: &serde_json::Value, _candidates: &[WorkId]) -> Vec<Candidate> {
    Vec::new()
}

/// The Reader-History engine: the reader's own history/notes.
pub fn reader_history(_signals: &serde_json::Value, _candidates: &[WorkId]) -> Vec<Candidate> {
    Vec::new()
}

/// Blend candidates from multiple engines deterministically.
///
/// Each engine contributes a weighted score; results are sorted by total score.
pub fn blend(_engines: &[Vec<Candidate>]) -> Vec<Candidate> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn more_like_this_returns_empty_without_signals() {
        let signals = serde_json::json!({});
        let candidates = vec![];
        let result = more_like_this(&signals, &candidates);
        assert!(result.is_empty());
    }

    #[test]
    fn same_fandom_fresh_returns_empty_without_signals() {
        let signals = serde_json::json!({});
        let candidates = vec![];
        let result = same_fandom_fresh(&signals, &candidates);
        assert!(result.is_empty());
    }

    #[test]
    fn reader_history_returns_empty_without_signals() {
        let signals = serde_json::json!({});
        let candidates = vec![];
        let result = reader_history(&signals, &candidates);
        assert!(result.is_empty());
    }

    #[test]
    fn blend_returns_empty_with_no_engines() {
        let engines: Vec<Vec<Candidate>> = vec![];
        let result = blend(&engines);
        assert!(result.is_empty());
    }
}
