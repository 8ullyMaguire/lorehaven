//! Identifier newtypes.
//!
//! Every entity identifier is a UUID. We deliberately use **v4 (random)**
//! rather than v7 (time-ordered): v7 leaks creation time and, worse, leaks
//! *ordering*, which would let an observer estimate how many objects exist and
//! roughly when they appeared. Spec §3.1 forbids exposing sequential account
//! identifiers or account ownership through public URLs; random identifiers
//! make enumeration useless.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! uuid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Mint a new random identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Wrap an existing UUID (used when reading from the database).
            #[must_use]
            pub fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            /// The underlying UUID.
            #[must_use]
            pub fn as_uuid(&self) -> Uuid {
                self.0
            }

            /// Canonical text form, as used in APIs and in SQLite columns.
            #[must_use]
            pub fn to_canonical_string(&self) -> String {
                self.0.to_string()
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::parse_str(s)?))
            }
        }

        impl From<Uuid> for $name {
            fn from(value: Uuid) -> Self {
                Self(value)
            }
        }

        impl From<$name> for Uuid {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

uuid_id! {
    /// An account: the credential-and-security container shared by its pseuds.
    AccountId
}
uuid_id! {
    /// A pseud: the public face under which content is authored.
    PseudId
}
uuid_id! {
    /// An opaque, server-managed session.
    SessionId
}
uuid_id! {
    /// A long-lived API token issued to integrations.
    ApiTokenId
}
uuid_id! {
    /// A work (a story, in the site's vocabulary).
    WorkId
}
uuid_id! {
    /// A single chapter belonging to a work.
    ChapterId
}
uuid_id! {
    /// An immutable chapter revision.
    RevisionId
}
uuid_id! {
    /// A series.
    SeriesId
}
uuid_id! {
    /// A private library item.
    LibraryItemId
}
uuid_id! {
    /// A background job.
    JobId
}
uuid_id! {
    /// A stored file (export, upload, avatar).
    MediaAssetId
}

/// A per-request correlation identifier.
///
/// Distinct from entity IDs: it is opaque, client-supplied or generated, and
/// appears in logs and error envelopes so a user can quote it to support.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(String);

impl RequestId {
    /// Generate a fresh random request id.
    #[must_use]
    pub fn generate() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Wrap a client-supplied id, after validating it is safe to log.
    ///
    /// We accept only characters that are safe in a header and in a log line —
    /// an attacker must not be able to inject newlines into our logs through
    /// the `x-request-id` header.
    #[must_use]
    pub fn sanitize(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.len() > 128 {
            return None;
        }
        let acceptable = trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '+'));
        acceptable.then(|| Self(trimmed.to_owned()))
    }

    /// The raw string form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RequestId({})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_random_and_not_sequential() {
        let a = AccountId::new();
        let b = AccountId::new();
        assert_ne!(a, b);
        // v4 sets the version nibble; v7 would set 7. Assert we never drift.
        assert_eq!(a.as_uuid().get_version_num(), 4);
    }

    #[test]
    fn identifier_round_trips_through_text() {
        let id = WorkId::new();
        let text = id.to_string();
        assert_eq!(text.parse::<WorkId>().expect("parses"), id);
    }

    #[test]
    fn identifier_serializes_as_a_bare_string() {
        let id = PseudId::new();
        let json = serde_json::to_string(&id).expect("serializes");
        assert_eq!(json, format!("\"{id}\""));
    }

    #[test]
    fn request_id_rejects_log_injection() {
        assert!(RequestId::sanitize("abc-123").is_some());
        assert!(RequestId::sanitize("abc\n123").is_none());
        assert!(RequestId::sanitize("").is_none());
        assert!(RequestId::sanitize(&"x".repeat(200)).is_none());
    }
}
