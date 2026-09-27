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

/// Every scope, in declaration order.
///
/// For the link flow's confirmation page, which has to show the reader exactly
/// what a bot is asking for. Derived from one list so a scope added to the enum
/// and a scope offered in the UI cannot drift.
pub fn all() -> Vec<Scope> {
    vec![
        Scope::ContentRead,
        Scope::ContentWrite,
        Scope::LibraryRead,
        Scope::CommentsWrite,
        Scope::TranslationRead,
        Scope::TranslationWrite,
        Scope::AdminRead,
        Scope::AdminWrite,
    ]
}

/// Parse a stored scope list, or name what was not understood.
///
/// The single place that decides what a scope list means, so the token resolver
/// and the link flow's grant check cannot disagree about it. Refusing an
/// unrecognised scope is the point: a token row holding `content.read` plus a
/// scope this build does not know is not a token with fewer scopes, it is a
/// token whose authority cannot be determined, and treating it as the former
/// means a scope revoked by renaming it — or written by a newer Lorehaven and
/// read by an older one — silently stops being enforced.
pub fn parse_all(raw: &[String]) -> Result<Vec<Scope>, String> {
    use std::str::FromStr as _;
    raw.iter()
        .map(|s| Scope::from_str(s))
        .collect::<Result<Vec<_>, _>>()
}

/// Whether `granted` is a subset of `offered` — the link flow's grant check.
///
/// A bot asks for scopes; a reader grants some of them; the grant may not exceed
/// the request. This is the same relation as [`scopes_are_subset`] with the
/// argument order made explicit at the call site, because "is what was granted
/// within what was offered" and "is what the token has within what the bot
/// asked for" are the same question asked in two directions and it is worth
/// being unambiguous which is which.
pub fn grant_is_within_offer(granted: &[Scope], offered: &[Scope]) -> bool {
    scopes_are_subset(offered, granted)
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

    // D6: the resolver used to drop an unrecognised scope, so a token whose row
    // carried `content.read` and a scope this build does not know resolved as
    // if it held only the former. The two tests below are the difference between
    // "a token with fewer scopes" and "a token whose authority cannot be
    // determined", which are not the same thing.
    #[test]
    fn parse_all_refuses_an_unrecognised_scope_rather_than_dropping_it() {
        let raw = vec![
            "content.read".to_owned(),
            "scope.from.the.future".to_owned(),
        ];
        let err = parse_all(&raw).expect_err(
            "a scope this build does not know must refuse the whole list, not vanish: \
             a silently narrowed token is a token that is not what it says it is",
        );
        assert!(
            err.contains("scope.from.the.future"),
            "the refusal names the scope, got: {err}"
        );
    }

    #[test]
    fn parse_all_accepts_a_known_list_and_the_empty_one() {
        let raw = vec!["content.read".to_owned(), "library.read".to_owned()];
        assert_eq!(
            parse_all(&raw).unwrap(),
            vec![Scope::ContentRead, Scope::LibraryRead]
        );
        // An empty list is a token with no scopes, which is a real and
        // refusable state — not a malformed one.
        assert_eq!(parse_all(&[]).unwrap(), Vec::<Scope>::new());
    }

    #[test]
    fn every_scope_round_trips_through_all() {
        // `all()` exists so the link confirmation page cannot drift from the
        // enum; this is the test that would notice if it did.
        for s in all() {
            assert_eq!(Scope::from_str(s.as_str()).unwrap(), s);
        }
        assert_eq!(all().len(), 8, "one entry per scope in the enum, no more");
    }

    #[test]
    fn a_grant_cannot_exceed_what_a_bot_offered() {
        let offered = vec![Scope::ContentRead, Scope::LibraryRead];
        assert!(grant_is_within_offer(&[Scope::ContentRead], &offered));
        assert!(
            !grant_is_within_offer(&[Scope::ContentRead, Scope::AdminWrite], &offered),
            "a bot that did not ask for admin.write cannot be granted it by a \
             reader confirming the challenge, however the request was built"
        );
    }
}
