//! A search query that could not be parsed or rendered.
//!
//! This is a distinct type rather than a bare `anyhow::Error` string because
//! the two failures a reader can cause -- a typo'd operator and a field that
//! belongs to another surface -- are *their* fault, and the response should say
//! so with the reason. Folding them into `anyhow` made every one of them a 500,
//! which tells the reader nothing and tells the operator the server is broken.

use std::fmt;

/// Why a query could not be turned into SQL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryProblem {
    /// The query text did not parse.
    Parse { message: String, offset: usize },
    /// The query parsed but a field or operator cannot be applied here.
    Render { message: String },
}

impl QueryProblem {
    /// The reason, safe to show a reader.
    pub fn message(&self) -> &str {
        match self {
            Self::Parse { message, .. } | Self::Render { message } => message,
        }
    }

    /// The character offset, when the failure is a parse failure.
    pub fn offset(&self) -> Option<usize> {
        match self {
            Self::Parse { offset, .. } => Some(*offset),
            Self::Render { .. } => None,
        }
    }
}

impl fmt::Display for QueryProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse { message, offset } => write!(f, "{message} at offset {offset}"),
            Self::Render { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for QueryProblem {}

/// A search failed because the query was not valid.
#[derive(Debug)]
pub struct SearchError {
    pub problem: QueryProblem,
}

impl SearchError {
    pub fn parse(message: impl Into<String>, offset: usize) -> Self {
        Self {
            problem: QueryProblem::Parse {
                message: message.into(),
                offset,
            },
        }
    }

    pub fn render(message: impl Into<String>) -> Self {
        Self {
            problem: QueryProblem::Render {
                message: message.into(),
            },
        }
    }
}

impl fmt::Display for SearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid search query: {}", self.problem)
    }
}

impl std::error::Error for SearchError {}

// No `From<SearchError> for anyhow::Error` here: `anyhow` has a blanket
// `impl<E: StdError + Send + Sync + 'static> From<E> for anyhow::Error`, so a
// manual one is a conflicting-impl error. The blanket impl is what preserves
// the concrete type for `downcast_ref`, which is the whole point of this type.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parse_failure_reports_its_offset() {
        let error = SearchError::parse("expected a term", 7);
        assert_eq!(error.problem.offset(), Some(7));
        assert!(error.to_string().contains("offset 7"));
    }

    #[test]
    fn a_render_failure_has_no_offset() {
        let error = SearchError::render("replies is not a works field");
        assert_eq!(error.problem.offset(), None);
        assert!(error.to_string().contains("replies is not a works field"));
    }

    #[test]
    fn it_converts_into_anyhow_without_losing_the_message() {
        let error: anyhow::Error = SearchError::render("category is a forum field").into();
        // `Display` of the concrete type, which `anyhow` preserves on the chain.
        assert!(
            error.to_string().contains("category is a forum field"),
            "got: {error}"
        );
    }
}
