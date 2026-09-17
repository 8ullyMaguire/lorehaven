//! Derivative pipeline: EPUB/PDF/text renditions, OCR for scans, and
//! transcoding for uploaded media (spec §32.4, M25).
//!
//! Every derivative records its parent blob checksum and re-verifies on a
//! schedule — a derivative whose parent has changed is stale and needs
//! rebuilding.

use serde::{Deserialize, Serialize};

/// What kind of derivative this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DerivativeKind {
    /// EPUB rendition of a work.
    Epub,
    /// PDF rendition of a work.
    Pdf,
    /// Plain-text rendition (e.g. from HTML or Markdown).
    Text,
    /// OCR text extracted from a scanned image.
    Ocr,
    /// Transcoded version of uploaded media (e.g. image resize).
    Transcode,
}

impl DerivativeKind {
    /// The wire and column representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Epub => "epub",
            Self::Pdf => "pdf",
            Self::Text => "text",
            Self::Ocr => "ocr",
            Self::Transcode => "transcode",
        }
    }

    /// Parse the stored representation.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "epub" => Self::Epub,
            "pdf" => Self::Pdf,
            "text" => Self::Text,
            "ocr" => Self::Ocr,
            "transcode" => Self::Transcode,
            _ => return None,
        })
    }

    /// Whether this derivative kind is produced by machine processing
    /// (OCR, transcode) rather than author-created.
    #[must_use]
    pub const fn is_machine_produced(self) -> bool {
        matches!(self, Self::Ocr | Self::Transcode)
    }
}

/// Errors from the derivative domain.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DerivativeError {
    #[error("no converter available for this derivative kind")]
    NoConverter,
    #[error("parent blob not found")]
    ParentNotFound,
    #[error("parent blob checksum mismatch — source changed")]
    ParentChanged,
    #[error("derivative build failed: {0}")]
    BuildFailed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derivative_kind_roundtrip() {
        for kind in [
            DerivativeKind::Epub,
            DerivativeKind::Pdf,
            DerivativeKind::Text,
            DerivativeKind::Ocr,
            DerivativeKind::Transcode,
        ] {
            assert_eq!(DerivativeKind::parse(kind.as_str()), Some(kind));
        }
    }

    #[test]
    fn machine_produced_kinds() {
        assert!(DerivativeKind::Ocr.is_machine_produced());
        assert!(DerivativeKind::Transcode.is_machine_produced());
        assert!(!DerivativeKind::Epub.is_machine_produced());
        assert!(!DerivativeKind::Pdf.is_machine_produced());
        assert!(!DerivativeKind::Text.is_machine_produced());
    }

    #[test]
    fn unknown_kind_returns_none() {
        assert_eq!(DerivativeKind::parse("video"), None);
    }
}
