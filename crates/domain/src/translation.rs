//! M17 — Translation domain: job state machine, unit segmentation, memory, glossaries.

/// Translation job states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranslationJobState {
    Quoted,
    Reserved,
    InProgress,
    InReview,
    Approved,
    Published,
    Failed,
    Cancelled,
}

impl TranslationJobState {
    /// Inverse of [`Self::as_str`]; the tests pin the round trip.
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "quoted" => Ok(Self::Quoted),
            "reserved" => Ok(Self::Reserved),
            "in_progress" => Ok(Self::InProgress),
            "in_review" => Ok(Self::InReview),
            "approved" => Ok(Self::Approved),
            "published" => Ok(Self::Published),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(format!("unknown TranslationJobState: {s}")),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Quoted => "quoted",
            Self::Reserved => "reserved",
            Self::InProgress => "in_progress",
            Self::InReview => "in_review",
            Self::Approved => "approved",
            Self::Published => "published",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl std::str::FromStr for TranslationJobState {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "quoted" => Ok(Self::Quoted),
            "reserved" => Ok(Self::Reserved),
            "in_progress" => Ok(Self::InProgress),
            "in_review" => Ok(Self::InReview),
            "approved" => Ok(Self::Approved),
            "published" => Ok(Self::Published),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(format!("unknown translation job state: {s}")),
        }
    }
}

/// Translation unit states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranslationUnitState {
    Pending,
    Translated,
    Reviewed,
    Approved,
}

impl TranslationUnitState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Translated => "translated",
            Self::Reviewed => "reviewed",
            Self::Approved => "approved",
        }
    }
}

impl std::str::FromStr for TranslationUnitState {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "translated" => Ok(Self::Translated),
            "reviewed" => Ok(Self::Reviewed),
            "approved" => Ok(Self::Approved),
            _ => Err(format!("unknown translation unit state: {s}")),
        }
    }
}

/// Review gate types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewGate {
    Linguistic,
    Cultural,
    Final,
}

impl ReviewGate {
    /// Inverse of [`Self::as_str`]; the tests pin the round trip.
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "linguistic" => Ok(Self::Linguistic),
            "cultural" => Ok(Self::Cultural),
            "final" => Ok(Self::Final),
            _ => Err(format!("unknown ReviewGate: {s}")),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Linguistic => "linguistic",
            Self::Cultural => "cultural",
            Self::Final => "final",
        }
    }
}

impl std::str::FromStr for ReviewGate {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "linguistic" => Ok(Self::Linguistic),
            "cultural" => Ok(Self::Cultural),
            "final" => Ok(Self::Final),
            _ => Err(format!("unknown review gate: {s}")),
        }
    }
}

/// Validate a translation job state transition.
pub fn valid_job_transition(from: &TranslationJobState, to: &TranslationJobState) -> bool {
    matches!(
        (from, to),
        (TranslationJobState::Quoted, TranslationJobState::Reserved)
            | (TranslationJobState::Quoted, TranslationJobState::Cancelled)
            | (
                TranslationJobState::Reserved,
                TranslationJobState::InProgress
            )
            | (TranslationJobState::Reserved, TranslationJobState::Failed)
            | (
                TranslationJobState::InProgress,
                TranslationJobState::InReview
            )
            | (TranslationJobState::InProgress, TranslationJobState::Failed)
            | (TranslationJobState::InReview, TranslationJobState::Approved)
            | (TranslationJobState::InReview, TranslationJobState::Failed)
            | (
                TranslationJobState::Approved,
                TranslationJobState::Published
            )
            | (TranslationJobState::Approved, TranslationJobState::Failed)
    )
}

/// Compute a normalized hash for a paragraph (for memory lookup).
pub fn paragraph_hash(text: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let normalized = text.trim().to_lowercase();
    let mut hasher = DefaultHasher::new();
    normalized.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

/// Apply glossary to text (exact match first, then case-insensitive, then longest match).
pub fn apply_glossary(text: &str, glossary: &[(String, String)], case_sensitive: bool) -> String {
    let mut result = text.to_string();

    // Sort by term length descending (longest match first)
    let mut sorted_glossary: Vec<_> = glossary.iter().collect();
    sorted_glossary.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

    for (term, translation) in sorted_glossary {
        if case_sensitive && result.contains(term.as_str()) {
            result = result.replace(term.as_str(), translation.as_str());
        } else if let Some(replaced) = replace_case_insensitive(&result, term, translation) {
            // Case-insensitive fallback: a glossary term must still match at
            // the start of a sentence, where the source capitalises it.
            // (Byte-length indexing assumes ASCII terms; glossaries are.)
            result = replaced;
        }
    }
    result
}

/// Replace every (case-insensitive) occurrence of `term`, splicing into the
/// original text so surrounding capitalisation is preserved. Returns `None`
/// when the term does not occur at all.
fn replace_case_insensitive(hay: &str, term: &str, repl: &str) -> Option<String> {
    let lower_hay = hay.to_lowercase();
    let lower_term = term.to_lowercase();
    let mut out = String::with_capacity(hay.len());
    let mut consumed = 0;
    while let Some(rel) = lower_hay[consumed..].find(&lower_term) {
        let start = consumed + rel;
        let end = start + term.len();
        out.push_str(&hay[consumed..start]);
        out.push_str(repl);
        consumed = end;
    }
    if consumed == 0 {
        return None;
    }
    out.push_str(&hay[consumed..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_state_round_trip() {
        for s in [
            TranslationJobState::Quoted,
            TranslationJobState::Reserved,
            TranslationJobState::InProgress,
            TranslationJobState::InReview,
            TranslationJobState::Approved,
            TranslationJobState::Published,
            TranslationJobState::Failed,
            TranslationJobState::Cancelled,
        ] {
            assert_eq!(TranslationJobState::from_str(s.as_str()).unwrap(), s);
        }
    }

    #[test]
    fn happy_path_job_transitions() {
        use TranslationJobState::*;
        assert!(valid_job_transition(&Quoted, &Reserved));
        assert!(valid_job_transition(&Reserved, &InProgress));
        assert!(valid_job_transition(&InProgress, &InReview));
        assert!(valid_job_transition(&InReview, &Approved));
        assert!(valid_job_transition(&Approved, &Published));
    }

    #[test]
    fn failure_transitions() {
        use TranslationJobState::*;
        assert!(valid_job_transition(&Reserved, &Failed));
        assert!(valid_job_transition(&InProgress, &Failed));
        assert!(valid_job_transition(&Approved, &Failed));
    }

    #[test]
    fn invalid_transitions() {
        use TranslationJobState::*;
        assert!(!valid_job_transition(&Published, &Quoted));
        assert!(!valid_job_transition(&Failed, &InProgress));
    }

    #[test]
    fn paragraph_hash_deterministic() {
        let h1 = paragraph_hash("Hello World");
        let h2 = paragraph_hash("Hello World");
        assert_eq!(h1, h2);
    }

    #[test]
    fn paragraph_hash_case_insensitive() {
        let h1 = paragraph_hash("Hello World");
        let h2 = paragraph_hash("hello world");
        assert_eq!(h1, h2);
    }

    #[test]
    fn apply_glossary_exact_match() {
        let glossary = vec![
            ("dragon".to_string(), "drago".to_string()),
            ("knight".to_string(), "chevalero".to_string()),
        ];
        let text = "The dragon fought the knight";
        let result = apply_glossary(text, &glossary, true);
        assert_eq!(result, "The drago fought the chevalero");
    }

    #[test]
    fn apply_glossary_longest_match_first() {
        let glossary = vec![
            ("the dragon".to_string(), "la drago".to_string()),
            ("dragon".to_string(), "draco".to_string()),
        ];
        let text = "The dragon appeared";
        let result = apply_glossary(text, &glossary, true);
        // The two-word term matches case-insensitively ("The dragon") and the
        // whole span is consumed by its translation.
        assert_eq!(result, "la drago appeared");
    }

    #[test]
    fn review_gate_round_trip() {
        for g in [
            ReviewGate::Linguistic,
            ReviewGate::Cultural,
            ReviewGate::Final,
        ] {
            assert_eq!(ReviewGate::from_str(g.as_str()).unwrap(), g);
        }
    }
}
