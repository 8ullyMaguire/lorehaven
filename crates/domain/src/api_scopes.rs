//! M18 — API scope vocabulary and token validation.

/// API scope vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    ContentRead,
    ContentWrite,
    LibraryRead,
    CommentsWrite,
    TranslationRead,
    TranslationWrite,
    AdminRead,
    AdminWrite,
}

impl Scope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ContentRead => "content.read",
            Self::ContentWrite => "content.write",
            Self::LibraryRead => "library.read",
            Self::CommentsWrite => "comments.write",
            Self::TranslationRead => "translation.read",
            Self::TranslationWrite => "translation.write",
            Self::AdminRead => "admin.read",
            Self::AdminWrite => "admin.write",
        }
    }
}

impl std::str::FromStr for Scope {
    type Err = String;
    /// Inverse of [`Scope::as_str`]; the tests pin the round trip.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "content.read" => Ok(Self::ContentRead),
            "content.write" => Ok(Self::ContentWrite),
            "library.read" => Ok(Self::LibraryRead),
            "comments.write" => Ok(Self::CommentsWrite),
            "translation.read" => Ok(Self::TranslationRead),
            "translation.write" => Ok(Self::TranslationWrite),
            "admin.read" => Ok(Self::AdminRead),
            "admin.write" => Ok(Self::AdminWrite),
            _ => Err(format!("unknown scope: {s}")),
        }
    }
}

/// Check that a token's scopes are a subset of the registration's requested scopes.
pub fn scopes_are_subset(requested: &[Scope], provided: &[Scope]) -> bool {
    provided.iter().all(|s| requested.contains(s))
}

/// Check if a token has a specific scope.
pub fn has_scope(scopes: &[Scope], required: &Scope) -> bool {
    scopes.contains(required)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn scope_round_trip() {
        for s in [
            Scope::ContentRead,
            Scope::ContentWrite,
            Scope::LibraryRead,
            Scope::CommentsWrite,
            Scope::TranslationRead,
            Scope::TranslationWrite,
            Scope::AdminRead,
            Scope::AdminWrite,
        ] {
            assert_eq!(Scope::from_str(s.as_str()).unwrap(), s);
        }
    }

    #[test]
    fn unknown_scope_rejected() {
        assert!(Scope::from_str("ambient.network").is_err());
        assert!(Scope::from_str("process.control").is_err());
    }

    #[test]
    fn subset_rule() {
        let requested = vec![Scope::ContentRead, Scope::LibraryRead, Scope::CommentsWrite];
        let valid = vec![Scope::ContentRead, Scope::LibraryRead];
        let invalid = vec![Scope::ContentRead, Scope::AdminRead];

        assert!(scopes_are_subset(&requested, &valid));
        assert!(!scopes_are_subset(&requested, &invalid));
    }

    #[test]
    fn has_scope_check() {
        let scopes = vec![Scope::ContentRead, Scope::LibraryRead];
        assert!(has_scope(&scopes, &Scope::ContentRead));
        assert!(has_scope(&scopes, &Scope::LibraryRead));
        assert!(!has_scope(&scopes, &Scope::ContentWrite));
    }
}
