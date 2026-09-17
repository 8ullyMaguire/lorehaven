//! The error taxonomy.
//!
//! Spec §3.3 requires *stable machine-readable error codes* and a single JSON
//! error envelope:
//!
//! ```json
//! {
//!   "error": {
//!     "code": "REVISION_CONFLICT",
//!     "message": "This chapter changed since you opened it.",
//!     "field_errors": {},
//!     "request_id": "..."
//!   }
//! }
//! ```
//!
//! This module owns the *taxonomy*: which errors exist, their stable codes,
//! and what a client is allowed to be told. Rendering the envelope is an HTTP
//! concern and lives in the server crate, so that the domain stays free of
//! transport and can be reused by the worker, the CLI and (eventually) plugin
//! host without dragging in a web framework.

use std::collections::BTreeMap;

/// Result alias used throughout the workspace.
pub type Result<T, E = AppError> = std::result::Result<T, E>;

/// Machine-readable error codes (spec §3.3). The string form is part of the
/// public API contract: never rename one without a version bump.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorCode {
    /// No session, or the session is expired/revoked.
    AuthRequired,
    /// Authenticated, but the policy decision was Deny.
    AccessDenied,
    /// The resource does not exist, or exists but must not be disclosed.
    NotFound,
    /// The request body failed validation.
    ValidationFailed,
    /// Optimistic concurrency check failed (`expected_version` mismatch).
    RevisionConflict,
    /// Too many requests.
    RateLimited,
    /// A hard quota (storage, rows, jobs) is exhausted.
    QuotaExceeded,
    /// Age policy or rating policy forbids this content for this actor.
    ContentRestricted,
    /// We have no adapter for that source.
    SourceUnsupported,
    /// We have an adapter, but the source cannot be reached or answered badly.
    SourceUnavailable,
    /// A background job failed terminally.
    JobFailed,
    /// The wallet cannot cover the reservation.
    InsufficientCredits,
    /// A plugin asked for a capability it was not granted.
    ExtensionPermissionDenied,
    /// Our fault. Details are logged, never returned.
    Internal,
    /// The instance lacks the program this operation needs.
    ConverterUnavailable,
    /// The operation is recognised but not yet implemented.
    NotImplemented,
}

impl ErrorCode {
    /// The wire representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AuthRequired => "AUTH_REQUIRED",
            Self::AccessDenied => "ACCESS_DENIED",
            Self::NotFound => "NOT_FOUND",
            Self::ValidationFailed => "VALIDATION_FAILED",
            Self::RevisionConflict => "REVISION_CONFLICT",
            Self::RateLimited => "RATE_LIMITED",
            Self::QuotaExceeded => "QUOTA_EXCEEDED",
            Self::ContentRestricted => "CONTENT_RESTRICTED",
            Self::SourceUnsupported => "SOURCE_UNSUPPORTED",
            Self::SourceUnavailable => "SOURCE_UNAVAILABLE",
            Self::JobFailed => "JOB_FAILED",
            Self::InsufficientCredits => "INSUFFICIENT_CREDITS",
            Self::ExtensionPermissionDenied => "EXTENSION_PERMISSION_DENIED",
            Self::ConverterUnavailable => "CONVERTER_UNAVAILABLE",
            Self::Internal => "INTERNAL",
            Self::NotImplemented => "NOT_IMPLEMENTED",
        }
    }
}

/// The single application error type.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AppError {
    /// The caller must authenticate.
    #[error("authentication required")]
    AuthRequired,

    /// Credentials were supplied and did not match.
    ///
    /// Deliberately distinct from [`AppError::AuthRequired`] so the message can
    /// be useful without inventing a new public error code: it maps to
    /// `AUTH_REQUIRED` with `401`, and it never says *which* half was wrong.
    #[error("that email address and password do not match an account")]
    InvalidCredentials,

    /// The caller is authenticated but not permitted.
    #[error("access denied")]
    AccessDenied,

    /// The resource is absent, or present but not disclosable.
    ///
    /// Spec §3.3: prefer `404` over revealing that a private object exists.
    /// `resource` is a coarse noun ("work", "library item") and is safe to
    /// disclose because it describes the *kind* asked for, not existence.
    #[error("{resource} not found")]
    NotFound {
        /// Coarse noun for the requested resource.
        resource: &'static str,
    },

    /// Field-level validation failure.
    #[error("{message}")]
    Validation {
        /// Human-readable summary, safe to display.
        message: String,
        /// Per-field messages, safe to display.
        field_errors: BTreeMap<String, String>,
    },

    /// Optimistic concurrency failure (spec §3.4).
    #[error("this resource changed since you opened it")]
    RevisionConflict {
        /// The version the client believed it was editing.
        expected: i64,
        /// The version the server holds.
        actual: i64,
    },

    /// Rate limit exceeded.
    #[error("rate limited")]
    RateLimited {
        /// Seconds the client should wait before retrying.
        retry_after_secs: u64,
    },

    /// Quota exceeded (storage, row counts, job budgets).
    #[error("quota exceeded")]
    QuotaExceeded {
        /// Which quota, in coarse terms.
        quota: &'static str,
    },

    /// Content is not eligible for this actor.
    #[error("this content is not available to you")]
    ContentRestricted,

    /// No adapter handles the supplied URL/file.
    #[error("unsupported source: {domain}")]
    SourceUnsupported {
        /// Domain of the rejected source.
        domain: String,
    },

    /// The source is known but unreachable or misbehaving.
    #[error("source unavailable: {domain}")]
    SourceUnavailable {
        /// Domain of the failing source.
        domain: String,
    },

    /// A job failed terminally.
    #[error("job failed: {code}")]
    JobFailed {
        /// Stable reason code recorded on the job attempt.
        code: String,
    },

    /// Insufficient credits for a reservation.
    #[error("insufficient credits")]
    InsufficientCredits {
        /// Credits required by the quote.
        required: i64,
        /// Credits available.
        available: i64,
    },

    /// A plugin requested a capability it does not hold.
    #[error("extension permission denied: {permission}")]
    ExtensionPermissionDenied {
        /// The capability that was refused.
        permission: String,
    },

    /// The instance cannot produce the requested form: the program that would
    /// is not installed (spec §3.3's `CONVERTER_UNAVAILABLE`).
    ///
    /// Distinct from `Validation` because nothing about the request is wrong
    /// and distinct from `Internal` because it is the operator's gap, not a
    /// fault: the message names what has to be installed.
    #[error("{message}")]
    ConverterUnavailable {
        /// What is missing and what to install, safe to display.
        message: String,
    },

    /// An unexpected fault. The inner error is logged, never returned.
    #[error("internal error")]
    Internal(#[from] anyhow::Error),
    /// The operation is recognised but not yet implemented.
    #[error("not implemented")]
    NotImplemented,
}

impl AppError {
    /// The stable code for this error.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        match self {
            Self::AuthRequired => ErrorCode::AuthRequired,
            Self::InvalidCredentials => ErrorCode::AuthRequired,
            Self::AccessDenied => ErrorCode::AccessDenied,
            Self::NotFound { .. } => ErrorCode::NotFound,
            Self::Validation { .. } => ErrorCode::ValidationFailed,
            Self::RevisionConflict { .. } => ErrorCode::RevisionConflict,
            Self::RateLimited { .. } => ErrorCode::RateLimited,
            Self::QuotaExceeded { .. } => ErrorCode::QuotaExceeded,
            Self::ContentRestricted => ErrorCode::ContentRestricted,
            Self::SourceUnsupported { .. } => ErrorCode::SourceUnsupported,
            Self::SourceUnavailable { .. } => ErrorCode::SourceUnavailable,
            Self::JobFailed { .. } => ErrorCode::JobFailed,
            Self::InsufficientCredits { .. } => ErrorCode::InsufficientCredits,
            Self::ExtensionPermissionDenied { .. } => ErrorCode::ExtensionPermissionDenied,
            Self::ConverterUnavailable { .. } => ErrorCode::ConverterUnavailable,
            Self::Internal(_) => ErrorCode::Internal,
            Self::NotImplemented => ErrorCode::NotImplemented,
        }
    }

    /// The HTTP status this error maps to.
    ///
    /// Returned as a bare `u16` so the domain does not depend on an HTTP crate;
    /// the server crate converts it to its own status type.
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::AuthRequired | Self::InvalidCredentials => 401,
            Self::AccessDenied
            | Self::ContentRestricted
            | Self::ExtensionPermissionDenied { .. } => 403,
            Self::NotFound { .. } => 404,
            Self::Validation { .. } | Self::SourceUnsupported { .. } => 422,
            // The request is well formed and the instance cannot serve it: the
            // program that would is absent. 422 rather than 503 — nothing is
            // temporarily down, and retrying the same request changes nothing
            // until an operator installs something.
            Self::ConverterUnavailable { .. } => 422,
            Self::RevisionConflict { .. } => 409,
            Self::RateLimited { .. } | Self::QuotaExceeded { .. } => 429,
            Self::InsufficientCredits { .. } => 402,
            Self::SourceUnavailable { .. } => 502,
            Self::JobFailed { .. } | Self::Internal(_) => 500,
            Self::NotImplemented => 501,
        }
    }

    /// Seconds the client should wait, when the error carries that advice.
    #[must_use]
    pub const fn retry_after_secs(&self) -> Option<u64> {
        match self {
            Self::RateLimited { retry_after_secs } => Some(*retry_after_secs),
            _ => None,
        }
    }

    /// The per-field messages, when the error has them.
    #[must_use]
    pub fn field_errors(&self) -> BTreeMap<String, String> {
        match self {
            Self::Validation { field_errors, .. } => field_errors.clone(),
            _ => BTreeMap::new(),
        }
    }

    /// The client-facing message. Internal errors are masked.
    #[must_use]
    pub fn public_message(&self) -> String {
        match self {
            Self::Internal(_) => {
                "Something went wrong on our side. The request id below identifies it.".to_owned()
            }
            other => other.to_string(),
        }
    }

    /// Whether the details should be logged as a fault rather than a refusal.
    #[must_use]
    pub const fn is_fault(&self) -> bool {
        matches!(self, Self::Internal(_))
    }

    /// Build a validation error from a single field.
    #[must_use]
    pub fn field(field: &str, message: impl Into<String>) -> Self {
        let message = message.into();
        let mut field_errors = BTreeMap::new();
        field_errors.insert(field.to_owned(), message.clone());
        Self::Validation {
            message,
            field_errors,
        }
    }

    /// Attach context to an internal error, preserving the chain.
    #[must_use]
    pub fn internal(context: &str, source: impl Into<anyhow::Error>) -> Self {
        Self::Internal(source.into().context(context.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable_upper_snake_strings() {
        let all = [
            ErrorCode::AuthRequired,
            ErrorCode::AccessDenied,
            ErrorCode::NotFound,
            ErrorCode::ValidationFailed,
            ErrorCode::RevisionConflict,
            ErrorCode::RateLimited,
            ErrorCode::QuotaExceeded,
            ErrorCode::ContentRestricted,
            ErrorCode::SourceUnsupported,
            ErrorCode::SourceUnavailable,
            ErrorCode::JobFailed,
            ErrorCode::InsufficientCredits,
            ErrorCode::ExtensionPermissionDenied,
            ErrorCode::Internal,
        ];
        for code in all {
            let text = code.as_str();
            assert!(!text.is_empty());
            assert!(
                text.chars().all(|c| c.is_ascii_uppercase() || c == '_'),
                "code {text} is not upper snake case"
            );
        }
    }

    #[test]
    fn statuses_follow_the_documented_contract() {
        assert_eq!(AppError::AuthRequired.status_code(), 401);
        assert_eq!(AppError::AccessDenied.status_code(), 403);
        assert_eq!(AppError::NotFound { resource: "work" }.status_code(), 404);
        assert_eq!(
            AppError::RevisionConflict {
                expected: 1,
                actual: 2
            }
            .status_code(),
            409
        );
        assert_eq!(AppError::field("title", "required").status_code(), 422);
        assert_eq!(
            AppError::RateLimited {
                retry_after_secs: 30
            }
            .status_code(),
            429
        );
        assert_eq!(
            AppError::InsufficientCredits {
                required: 10,
                available: 2
            }
            .status_code(),
            402
        );
        assert_eq!(
            AppError::SourceUnavailable {
                domain: "example.invalid".to_owned()
            }
            .status_code(),
            502
        );
    }

    #[test]
    fn private_absence_is_reported_as_not_found() {
        let error = AppError::NotFound { resource: "work" };
        assert_eq!(error.code(), ErrorCode::NotFound);
        assert!(!error.is_fault());
    }

    #[test]
    fn internal_errors_do_not_leak_details() {
        let error = AppError::internal("db blew up", anyhow::anyhow!("secret dsn here"));
        let message = error.public_message();
        assert!(!message.contains("secret dsn here"));
        assert!(!message.contains("db blew up"));
        assert!(error.is_fault());
        assert_eq!(error.status_code(), 500);
        assert_eq!(error.code(), ErrorCode::Internal);
    }

    #[test]
    fn validation_errors_carry_their_fields() {
        let error = AppError::field("handle", "is already taken");
        let fields = error.field_errors();
        assert_eq!(
            fields.get("handle").map(String::as_str),
            Some("is already taken")
        );
    }

    #[test]
    fn only_rate_limits_advise_a_retry_delay() {
        assert_eq!(
            AppError::RateLimited {
                retry_after_secs: 12
            }
            .retry_after_secs(),
            Some(12)
        );
        assert_eq!(AppError::AuthRequired.retry_after_secs(), None);
    }
}
