//! The error taxonomy.
//!
//! Spec §3.3 requires *stable machine-readable error codes* and a single JSON
//! error envelope:
//!
//! ```json
//! {
//!   "error": {
//!     "code": "REVISION_CONFLICT",
//!     "message": "This unit changed since you opened it.",
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

/// The optimistic-concurrency detail of a conflict.
///
/// **Named, because a client that cannot retry is a client that clobbers.**
/// A 409 with no version numbers tells the caller that it lost and not what
/// it lost to, so the only way to find out is to re-read the work — and a
/// client that re-reads and retries is a client that has to be right about
/// the second read too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConflictDetail {
    /// The field the conflict is about.
    pub field: &'static str,
    /// The version the client believed it was editing.
    pub expected: i64,
    /// The version the server holds.
    pub actual: i64,
}

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
    /// The account is below the trust level this action requires.
    ///
    /// Distinct from `ExtensionPermissionDenied`: that one is about a *plugin's*
    /// capabilities, this is about a *person's* trust level. Reporting a trust
    /// refusal as a capability refusal tells the reader to go edit a plugin
    /// manifest when the thing they need to change is their own standing.
    TrustLevelInsufficient,
    /// A module of the instance is not available.
    ModuleUnavailable,
    /// The surface exists, but its switch is not the reader's to throw (§59.3).
    ModuleNotUserToggleable,
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
            Self::TrustLevelInsufficient => "TRUST_LEVEL_INSUFFICIENT",
            Self::ModuleUnavailable => "MODULE_UNAVAILABLE",
            Self::ModuleNotUserToggleable => "MODULE_NOT_USER_TOGGLEABLE",
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

    /// The caller is authenticated, and permitted in general, but the token
    /// they presented does not carry the scope this door requires.
    ///
    /// Separate from [`AppError::AccessDenied`] purely so the message can name
    /// the scope. A bot that gets 403 "access denied" cannot tell whether it
    /// asked for the wrong door, was granted too little at link time, or is
    /// being refused for some reason this build does not describe — and the
    /// fix is different in each case. §23.1's contract is that the scope set
    /// is the whole of what the client was granted, so saying which one is
    /// missing discloses nothing the token holder does not already know.
    #[error("this token lacks the {scope} scope")]
    MissingScope {
        /// The scope the door requires, e.g. `content.write`.
        scope: &'static str,
    },

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

    /// The account is below the trust level this action requires.
    ///
    /// Carries both levels rather than only saying no, because §55.2's bar is a
    /// trust threshold a reader can look up and work toward. A refusal that
    /// names neither the level found nor the bar required is not actionable,
    /// and §0.3 makes thresholds visible by design.
    #[error("this action requires trust level {required}; this account is at {level}")]
    TrustLevelInsufficient {
        /// The trust level the account actually holds.
        level: i64,
        /// The trust level the action requires.
        required: i64,
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

    /// A module of the instance is not available.
    ///
    /// **404, not 501.** The thing is not here *yet*, and the client is owed a
    /// "not here" rather than a "never going to happen" — the two are different
    /// facts with different remedies, and `NOT_IMPLEMENTED` says "this instance
    /// does not do that", which is a claim about the instance and not about the
    /// build.
    #[error("the {module} module is not available on this instance")]
    ModuleUnavailable {
        /// The module that is not available.
        module: &'static str,
    },

    /// A reader tried to change a surface whose switch belongs to somebody else.
    ///
    /// **403, not 404.** The surface is here and the reader can use it — what they
    /// cannot do is hold an opinion about it. Answering 404 would say the feature
    /// does not exist, sending them looking for a settings page that will never
    /// appear; `toggle` names who does decide, which is the only useful answer.
    #[error("the switch for `{module}` belongs to the {toggle}, not to a reader")]
    ModuleNotUserToggleable {
        /// The module whose switch was refused.
        module: &'static str,
        /// Who decides it: "operator" or "instance".
        toggle: &'static str,
    },

    /// An unexpected fault. The inner error is logged, never returned.
    #[error("internal error")]
    Internal(#[from] anyhow::Error),
    /// The operation is recognised but not yet implemented.
    #[error("not implemented")]
    NotImplemented,
}

/// The 404-not-403 doctrine, in one place (spec §20.1, N-13).
///
/// **"403 answers 'yes, and you may not'."** So anything the caller may not
/// know about is **404**, and there are three kinds of it:
///
///   * an **inaccessible private object** — a work that is not published, a
///     library item belonging to somebody else, a draft;
///   * a **malformed id** — because a malformed id and a private id must be
///     indistinguishable from the outside, and a caller that can tell them apart
///     is an oracle for what exists;
///   * an **operator surface** — the door exists, but its existence is not
///     disclosed to somebody who may not use it.
///
/// **A capability check is NOT one of them.** "You may not moderate" is a
/// 403: the door is public knowledge, the permission is not, and hiding the door
/// would make the instance's own structure undiscoverable to the people who
/// administer it. The two are different questions — *"may I do this?"* and *"may
/// I know this exists?"* — and only the second one is answered with 404.
///
/// The parameter is the **accessibility**, not the actor, because accessibility
/// is a property of the resource and the actor's question about it is asked
/// elsewhere; what this function answers is only ever "and what do I say when
/// the answer is no".
pub fn not_found_if_inaccessible(
    resource: &'static str,
    accessible: bool,
) -> Result<(), crate::AppError> {
    if accessible {
        Ok(())
    } else {
        Err(crate::AppError::NotFound { resource })
    }
}

impl AppError {
    /// The stable code for this error.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        match self {
            Self::AuthRequired => ErrorCode::AuthRequired,
            Self::InvalidCredentials => ErrorCode::AuthRequired,
            Self::AccessDenied | Self::MissingScope { .. } => ErrorCode::AccessDenied,
            Self::NotFound { .. } => ErrorCode::NotFound,
            Self::ModuleUnavailable { .. } => ErrorCode::ModuleUnavailable,
            Self::ModuleNotUserToggleable { .. } => ErrorCode::ModuleNotUserToggleable,
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
            Self::TrustLevelInsufficient { .. } => ErrorCode::TrustLevelInsufficient,
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
            | Self::MissingScope { .. }
            | Self::ContentRestricted
            | Self::ExtensionPermissionDenied { .. }
            | Self::TrustLevelInsufficient { .. } => 403,
            Self::NotFound { .. } | Self::ModuleUnavailable { .. } => 404,
            Self::ModuleNotUserToggleable { .. } => 403,
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

    /// The optimistic-concurrency detail, when the error is a conflict.
    ///
    /// `None` for every other error, so the response body carries the detail
    /// only where it means something.
    #[must_use]
    pub fn conflict(&self) -> Option<ConflictDetail> {
        match self {
            Self::RevisionConflict { expected, actual } => Some(ConflictDetail {
                field: "version",
                expected: *expected,
                actual: *actual,
            }),
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
            ErrorCode::TrustLevelInsufficient,
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
