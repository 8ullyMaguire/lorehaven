//! Derivative pipeline: EPUB/PDF/text renditions, OCR for scans, and
//! transcoding for uploaded media (spec §32.4, M25).
//!
//! Every derivative records its parent blob checksum and re-verifies on a
//! schedule — a derivative whose parent has changed is stale and needs
//! rebuilding.

use std::fmt;

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

impl fmt::Display for DerivativeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
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

    /// The programs that can produce this kind, most preferred first.
    ///
    /// Empty for the document renditions, which go through the export
    /// converter layer (`Converters::for_format` resolves Calibre versus
    /// pandoc, including the format quirks only that layer knows). OCR and
    /// transcode have exactly one program each, and naming it here means the
    /// request door, the worker and `lorehaven doctor` cannot disagree about
    /// what has to be installed.
    #[must_use]
    pub const fn required_programs(self) -> &'static [&'static str] {
        match self {
            Self::Epub | Self::Pdf | Self::Text => &[],
            Self::Ocr => &["tesseract"],
            Self::Transcode => &["ffmpeg"],
        }
    }

    /// What to tell an operator who does not have the program.
    #[must_use]
    pub const fn install_hint(self) -> &'static str {
        match self {
            Self::Epub | Self::Pdf | Self::Text => "install Calibre (ebook-convert) or pandoc",
            Self::Ocr => "install Tesseract OCR (tesseract)",
            Self::Transcode => "install ffmpeg",
        }
    }

    /// The media type of the artifact this kind produces.
    ///
    /// It travels with the derivative row so a reader's client is told what it
    /// is about to download, and so a later format change is a visible change
    /// to this function rather than a silent one at a call site.
    #[must_use]
    pub const fn output_media_type(self) -> &'static str {
        match self {
            Self::Epub => "application/epub+zip",
            Self::Pdf => "application/pdf",
            Self::Ocr | Self::Text => "text/plain; charset=utf-8",
            Self::Transcode => "video/mp4",
        }
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

    #[test]
    fn every_kind_names_its_program_and_its_media_type() {
        // The request door refuses with `install_hint` when a required program
        // is missing, so a kind whose program is unnamed would be a kind the
        // door silently accepts and the worker then fails.
        assert!(DerivativeKind::Ocr
            .required_programs()
            .contains(&"tesseract"));
        assert!(DerivativeKind::Transcode
            .required_programs()
            .contains(&"ffmpeg"));
        assert!(DerivativeKind::Epub.required_programs().is_empty());
        assert_eq!(
            DerivativeKind::Ocr.output_media_type(),
            "text/plain; charset=utf-8"
        );
        assert_eq!(
            DerivativeKind::Epub.output_media_type(),
            "application/epub+zip"
        );
        assert_eq!(DerivativeKind::Transcode.output_media_type(), "video/mp4");
        for kind in [
            DerivativeKind::Epub,
            DerivativeKind::Pdf,
            DerivativeKind::Text,
            DerivativeKind::Ocr,
            DerivativeKind::Transcode,
        ] {
            assert!(
                !kind.output_media_type().is_empty(),
                "{kind} has no media type"
            );
            assert!(!kind.install_hint().is_empty(), "{kind} has no remedy");
            // A machine-produced kind is exactly one that needs a program.
            assert_eq!(
                kind.is_machine_produced(),
                !kind.required_programs().is_empty(),
                "{kind}"
            );
        }
    }
}
