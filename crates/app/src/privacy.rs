//! Privacy settings: the canonical key set and its defaults.
//!
//! Spec §7 requires privacy settings to be **stored from onboarding** rather
//! than applied at render time, and requires minor-protective messaging
//! defaults. Both of those only hold if there is exactly one definition of the
//! key set, which is what this module is: registration writes these defaults,
//! the settings endpoint validates against these keys, and nothing else is
//! allowed to invent one.
//!
//! A key that is absent from storage is *not* an error — `default_for` fills it
//! in. That is what lets an account created before a key existed behave the
//! same as one created after, and it is why the defaults are the safe value
//! rather than the permissive one.

use lorehaven_domain::policy::AgeState;

/// Where a privacy setting lives.
///
/// The scope is a property of the key, not of the request. A key that could be
/// set at either scope would be a key whose value depends on which endpoint the
/// client happened to call, which is how "public bookmarks" ends up meaning two
/// different things.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Shared by every pseud of an account: credentials-adjacent policy.
    Account,
    /// Held separately by each pseud, because it describes a public face.
    Pseud,
}

/// A privacy preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrivacyKey {
    /// Storage name.
    pub name: &'static str,
    /// One-line description, shown in the settings interface.
    pub summary: &'static str,
    /// The permitted values.
    pub values: &'static [&'static str],
    /// Whether the key belongs to the account or to a single pseud.
    pub scope: Scope,
}

/// Who may see a pseud's bookmarks.
pub const PUBLIC_BOOKMARKS: PrivacyKey = PrivacyKey {
    name: "public_bookmarks",
    summary: "Whether your saved works are visible on your profile",
    values: &["private", "public"],
    scope: Scope::Pseud,
};

/// Who may start a conversation with you.
pub const MESSAGING_POLICY: PrivacyKey = PrivacyKey {
    name: "messaging_policy",
    summary: "Who is allowed to start a conversation with you",
    values: &["nobody", "contacts_only", "anyone"],
    // Account-level: messaging is a property of the person, not of the face
    // they happen to be wearing. A pseud that could switch to "anyone" would be
    // a way around the protective default.
    scope: Scope::Account,
};

/// Whether this account's pseuds appear in listings and search.
pub const DIRECTORY_LISTING: PrivacyKey = PrivacyKey {
    name: "directory_listing",
    summary: "Whether your pseuds appear in the author directory",
    values: &["listed", "hidden"],
    scope: Scope::Account,
};

/// Whether the instance's discovery preferences may influence this account.
///
/// Spec §15 requires a *meaningful* opt-out, and the default has to be
/// discoverable rather than buried: the setting is opt-out, so the value here
/// is the opt-in one and the interface must say what turning it off does.
pub const INSTANCE_AFFINITY: PrivacyKey = PrivacyKey {
    name: "instance_affinity",
    summary: "Let this instance's discovery preferences shape your recommendations",
    values: &["true", "false"],
    scope: Scope::Account,
};

/// Whether this account's activity may be used to learn those preferences.
///
/// Separate from [`INSTANCE_AFFINITY`] on purpose: a reader might accept being
/// *steered* while refusing to be *learned from*, and conflating the two would
/// take a consent they did not give.
pub const TASTE_LEARNING: PrivacyKey = PrivacyKey {
    name: "taste_learning",
    summary: "Allow your activity to inform this instance's discovery preferences",
    values: &["true", "false"],
    scope: Scope::Account,
};

/// Whether other people can see which works you follow.
pub const PUBLIC_FOLLOWS: PrivacyKey = PrivacyKey {
    name: "public_follows",
    summary: "Whether the authors you follow are visible on your profile",
    values: &["private", "public"],
    scope: Scope::Pseud,
};

/// Every recognised key.
pub const ALL: &[PrivacyKey] = &[
    PUBLIC_BOOKMARKS,
    MESSAGING_POLICY,
    DIRECTORY_LISTING,
    INSTANCE_AFFINITY,
    TASTE_LEARNING,
    PUBLIC_FOLLOWS,
];

/// Look up a key by name.
#[must_use]
pub fn find(name: &str) -> Option<&'static PrivacyKey> {
    ALL.iter().find(|key| key.name == name)
}

/// Every key belonging to a scope.
pub fn keys_in(scope: Scope) -> impl Iterator<Item = &'static PrivacyKey> {
    ALL.iter().filter(move |key| key.scope == scope)
}

/// Whether a value is permitted for a key.
#[must_use]
pub fn is_valid_value(key: &PrivacyKey, value: &str) -> bool {
    key.values.contains(&value)
}

/// The default value for a key, given the account's age state.
///
/// The age state is a parameter rather than something the caller resolves,
/// because the minor-protective rule is a property of the setting itself:
/// leaving it to each call site is how a protective default goes missing.
#[must_use]
pub fn default_for(key: &str, age_state: AgeState) -> &'static str {
    let minor = age_state.is_minor();

    match key {
        // Bookmarks are private unless the reader says otherwise.
        "public_bookmarks" => "private",
        // The important one: an account we know to be a minor may not receive
        // unsolicited messages at all, and that default is stored rather than
        // computed at render time so that it cannot be bypassed by a client
        // that simply does not ask.
        "messaging_policy" => {
            if minor {
                "nobody"
            } else {
                "contacts_only"
            }
        }
        // Minors are not listed in the directory by default.
        "directory_listing" => {
            if minor {
                "hidden"
            } else {
                "listed"
            }
        }
        // Opt-out, but the inverse of "silently on": the interface states what
        // turning it off removes.
        "instance_affinity" => "true",
        // Learning is the more intrusive of the two, so a minor is opted out.
        "taste_learning" => {
            if minor {
                "false"
            } else {
                "true"
            }
        }
        "public_follows" => "private",
        // An unrecognised key has no safe default; "private" is the value that
        // discloses nothing.
        _ => "private",
    }
}

/// The account-level defaults to write at registration.
///
/// Returns `(key, value)` pairs so that onboarding and the settings endpoint
/// cannot disagree about what a new account looks like.
#[must_use]
pub fn onboarding_defaults(age_state: AgeState) -> Vec<(&'static str, &'static str)> {
    keys_in(Scope::Account)
        .map(|key| (key.name, default_for(key.name, age_state)))
        .collect()
}

/// The pseud-level defaults to write for each pseud.
#[must_use]
pub fn onboarding_pseud_defaults(age_state: AgeState) -> Vec<(&'static str, &'static str)> {
    keys_in(Scope::Pseud)
        .map(|key| (key.name, default_for(key.name, age_state)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_has_a_default_that_is_one_of_its_values() {
        for age_state in [
            AgeState::DeclaredAdult,
            AgeState::DeclaredMinor,
            AgeState::Restricted,
            AgeState::Unknown,
        ] {
            for key in ALL {
                let value = default_for(key.name, age_state);
                assert!(
                    is_valid_value(key, value),
                    "default {value:?} for {} is not one of {:?}",
                    key.name,
                    key.values
                );
            }
        }
    }

    #[test]
    fn account_and_pseud_keys_do_not_overlap() {
        let account: Vec<&str> = keys_in(Scope::Account).map(|key| key.name).collect();
        let pseud: Vec<&str> = keys_in(Scope::Pseud).map(|key| key.name).collect();

        assert!(!account.is_empty());
        assert!(!pseud.is_empty());
        for name in &account {
            assert!(
                !pseud.contains(name),
                "{name} is declared at both scopes, so its meaning depends on which \
                 endpoint wrote it"
            );
        }
    }

    #[test]
    fn messaging_and_learning_are_account_scoped() {
        // These two carry the protective defaults, so a pseud that could change
        // them would be a route around those defaults.
        assert_eq!(MESSAGING_POLICY.scope, Scope::Account);
        assert_eq!(TASTE_LEARNING.scope, Scope::Account);
        assert_eq!(PUBLIC_BOOKMARKS.scope, Scope::Pseud);
    }

    #[test]
    fn an_unknown_key_has_a_non_disclosing_default() {
        // The fallback must fail towards privacy, never towards exposure.
        assert_eq!(
            default_for("something_new", AgeState::DeclaredAdult),
            "private"
        );
    }

    #[test]
    fn minors_may_not_receive_unsolicited_messages_by_default() {
        assert_eq!(
            default_for("messaging_policy", AgeState::DeclaredMinor),
            "nobody"
        );
        assert_eq!(
            default_for("messaging_policy", AgeState::DeclaredAdult),
            "contacts_only"
        );
    }

    #[test]
    fn minors_are_not_listed_and_are_opted_out_of_taste_learning() {
        assert_eq!(
            default_for("directory_listing", AgeState::DeclaredMinor),
            "hidden"
        );
        assert_eq!(
            default_for("taste_learning", AgeState::DeclaredMinor),
            "false"
        );
        assert_eq!(
            default_for("taste_learning", AgeState::DeclaredAdult),
            "true"
        );
    }

    #[test]
    fn an_authorised_minor_keeps_the_protective_defaults() {
        // Authorization changes what the account may *do*; it does not reverse
        // the protective defaults it was created with.
        assert_eq!(
            default_for("messaging_policy", AgeState::AuthorizedUnderPolicy),
            "nobody"
        );
    }

    #[test]
    fn onboarding_writes_every_key_at_its_own_scope() {
        let age_state = AgeState::DeclaredAdult;
        let account = onboarding_defaults(age_state);
        let pseud = onboarding_pseud_defaults(age_state);

        assert_eq!(account.len() + pseud.len(), ALL.len());
        for key in ALL {
            let written = match key.scope {
                Scope::Account => account.iter().any(|(name, _)| *name == key.name),
                Scope::Pseud => pseud.iter().any(|(name, _)| *name == key.name),
            };
            assert!(written, "onboarding omitted {} at its own scope", key.name);
        }
    }

    #[test]
    fn keys_are_looked_up_by_name() {
        assert!(find("messaging_policy").is_some());
        assert!(find("not_a_key").is_none());
    }

    #[test]
    fn key_names_are_unique() {
        let mut names: Vec<&str> = ALL.iter().map(|key| key.name).collect();
        let original = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), original, "privacy key names must be unique");
    }

    #[test]
    fn values_are_validated_against_the_key() {
        assert!(is_valid_value(&MESSAGING_POLICY, "contacts_only"));
        assert!(!is_valid_value(&MESSAGING_POLICY, "sometimes"));
        assert!(!is_valid_value(&PUBLIC_BOOKMARKS, "friends"));
    }
}
