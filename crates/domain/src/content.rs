//! Content policies: who may read, edit, publish and withdraw a work.
//!
//! Spec §3.6 requires authorization to live in policy functions rather than in
//! scattered handlers, and spec §8 gives contributors roles. Both rules are
//! served by one place that answers:
//!
//! * may this actor **read** this work (delegated to the shared eligibility
//!   service in [`crate::policy`], because a second reading rule is how
//!   restricted content leaks);
//! * may this actor **edit** it, which means "is this acting pseud a
//!   contributor with an editing role";
//! * may this actor **publish** or **withdraw** it, which is a narrower set of
//!   roles again.
//!
//! The property worth stating plainly, because it is easy to get wrong:
//! **ownership belongs to a pseud, not to an account.** An account that owns
//! two pseuds and writes a work as one of them does not thereby gain edit
//! rights as the other. That is what spec §8's acceptance criterion "pseud
//! switching does not change ownership" means, and it is why every function
//! here takes an actor (which carries the active pseud) rather than an account.
//!
//! Everything here is pure: no database, no clock, no I/O.

use crate::error::{AppError, Result};
use crate::ids::PseudId;
use crate::policy::{Actor, Decision, DenyReason, Lifecycle};

/// What a pseud is allowed to do on a work (spec §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContributorRole {
    /// Created the work. The only role that may delete it.
    Owner,
    /// May edit, publish and invite; may not delete.
    Coauthor,
    /// May edit chapters; may not publish.
    Editor,
    /// May read a draft and comment; may not change text.
    BetaReader,
}

impl ContributorRole {
    /// Storage and wire form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Coauthor => "coauthor",
            Self::Editor => "editor",
            Self::BetaReader => "beta_reader",
        }
    }

    /// Parse the storage form, refusing anything unrecognised.
    ///
    /// Refusing rather than defaulting matters: an unknown role silently
    /// becoming `Owner` would be a privilege escalation written as a fallback.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "owner" => Some(Self::Owner),
            "coauthor" => Some(Self::Coauthor),
            "editor" => Some(Self::Editor),
            "beta_reader" => Some(Self::BetaReader),
            _ => None,
        }
    }

    /// Whether this role may change a work's text or metadata.
    #[must_use]
    pub const fn may_edit(self) -> bool {
        matches!(self, Self::Owner | Self::Coauthor | Self::Editor)
    }

    /// Whether this role may change publication state.
    #[must_use]
    pub const fn may_publish(self) -> bool {
        matches!(self, Self::Owner | Self::Coauthor)
    }

    /// Whether this role may add or remove contributors.
    #[must_use]
    pub const fn may_manage_contributors(self) -> bool {
        matches!(self, Self::Owner | Self::Coauthor)
    }

    /// Whether this role may delete the work.
    #[must_use]
    pub const fn may_delete(self) -> bool {
        matches!(self, Self::Owner)
    }

    /// Human-readable label for the interface.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Owner => "Owner",
            Self::Coauthor => "Co-author",
            Self::Editor => "Editor",
            Self::BetaReader => "Beta reader",
        }
    }
}

/// A contributor row, as loaded from storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Contributor {
    /// The pseud credited.
    pub pseud_id: PseudId,
    /// What it may do.
    pub role: ContributorRole,
    /// Whether the credit is shown publicly. A private co-author is a real
    /// arrangement — a beta reader or an editor who does not want their name
    /// on the work — so the flag is data, not a presentation detail.
    pub public_attribution: bool,
}

/// The role an acting pseud holds on a work, if any.
#[must_use]
pub fn role_of(pseud_id: PseudId, contributors: &[Contributor]) -> Option<ContributorRole> {
    contributors
        .iter()
        .find(|contributor| contributor.pseud_id == pseud_id)
        .map(|contributor| contributor.role)
}

/// Whether this actor may change a work's text or metadata.
#[must_use]
pub fn can_edit_work(actor: &Actor, contributors: &[Contributor]) -> Decision {
    match role_of(actor.pseud_id, contributors) {
        Some(role) if role.may_edit() => Decision::Allow,
        Some(_) => Decision::Deny(DenyReason::InsufficientRole),
        None => Decision::Deny(DenyReason::NotAContributor),
    }
}

/// Whether this actor may publish, withdraw or schedule a work.
#[must_use]
pub fn can_publish_work(actor: &Actor, contributors: &[Contributor]) -> Decision {
    match role_of(actor.pseud_id, contributors) {
        Some(role) if role.may_publish() => Decision::Allow,
        Some(_) => Decision::Deny(DenyReason::InsufficientRole),
        None => Decision::Deny(DenyReason::NotAContributor),
    }
}

/// Whether this actor may invite, change or remove contributors.
#[must_use]
pub fn can_manage_contributors(actor: &Actor, contributors: &[Contributor]) -> Decision {
    match role_of(actor.pseud_id, contributors) {
        Some(role) if role.may_manage_contributors() => Decision::Allow,
        Some(_) => Decision::Deny(DenyReason::InsufficientRole),
        None => Decision::Deny(DenyReason::NotAContributor),
    }
}

/// Everything a publication decision needs to know about a work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicationFacts<'a> {
    /// The work's title, as the author last set it.
    pub title: &'a str,
    /// How many chapters exist.
    pub chapter_count: usize,
    /// How many of them have a revision with any text.
    pub chapters_with_content: usize,
    /// The current lifecycle state.
    pub lifecycle: Lifecycle,
}

/// Check the preconditions for publishing (spec §8.4, "validate required
/// metadata"), returning a field-level error the interface can place.
///
/// Deliberately not a boolean: the caller must be told *which* requirement is
/// unmet, or the interface can only say "something is wrong".
pub fn publication_readiness(facts: &PublicationFacts<'_>) -> Result<()> {
    if facts.title.trim().is_empty() {
        return Err(AppError::field(
            "title",
            "A work needs a title before it can be published.",
        ));
    }
    if facts.chapter_count == 0 {
        return Err(AppError::field(
            "chapters",
            "A work needs at least one chapter before it can be published.",
        ));
    }
    if facts.chapters_with_content == 0 {
        return Err(AppError::field(
            "chapters",
            "Write something in a chapter before publishing: an empty work has nothing to read.",
        ));
    }
    if matches!(facts.lifecycle, Lifecycle::Deleted) {
        return Err(AppError::AccessDenied);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::AccountId;
    use crate::policy::AgeState;

    fn actor(pseud: PseudId) -> Actor {
        Actor {
            account_id: AccountId::new(),
            pseud_id: pseud,
            age_state: AgeState::DeclaredAdult,
            trusted_reviewer: false,
        }
    }

    fn contributor(pseud: PseudId, role: ContributorRole) -> Contributor {
        Contributor {
            pseud_id: pseud,
            role,
            public_attribution: true,
        }
    }

    #[test]
    fn an_unknown_role_is_refused_rather_than_defaulted() {
        assert_eq!(
            ContributorRole::parse("owner"),
            Some(ContributorRole::Owner)
        );
        assert_eq!(
            ContributorRole::parse("coauthor"),
            Some(ContributorRole::Coauthor)
        );
        assert_eq!(
            ContributorRole::parse("editor"),
            Some(ContributorRole::Editor)
        );
        assert_eq!(
            ContributorRole::parse("beta_reader"),
            Some(ContributorRole::BetaReader)
        );
        assert_eq!(ContributorRole::parse("administrator"), None);
        assert_eq!(ContributorRole::parse(""), None);
    }

    #[test]
    fn only_the_owner_may_delete() {
        assert!(ContributorRole::Owner.may_delete());
        assert!(!ContributorRole::Coauthor.may_delete());
        assert!(!ContributorRole::Editor.may_delete());
        assert!(!ContributorRole::BetaReader.may_delete());
    }

    #[test]
    fn editors_may_edit_but_not_publish() {
        assert!(ContributorRole::Editor.may_edit());
        assert!(!ContributorRole::Editor.may_publish());
        assert!(!ContributorRole::Editor.may_manage_contributors());
    }

    #[test]
    fn beta_readers_may_change_nothing() {
        assert!(!ContributorRole::BetaReader.may_edit());
        assert!(!ContributorRole::BetaReader.may_publish());
        assert!(!ContributorRole::BetaReader.may_manage_contributors());
    }

    #[test]
    fn a_second_pseud_of_the_same_account_is_not_a_contributor() {
        // The whole point of pseud-level ownership (spec §8 acceptance:
        // "pseud switching does not change ownership").
        let owner = PseudId::new();
        let other = PseudId::new();
        let contributors = [contributor(owner, ContributorRole::Owner)];

        let switched = actor(other);
        // Same account, different face:
        let mut switched = switched;
        switched.account_id = actor(owner).account_id;

        assert_eq!(
            can_edit_work(&switched, &contributors),
            Decision::Deny(DenyReason::NotAContributor)
        );
        assert_eq!(
            can_publish_work(&switched, &contributors),
            Decision::Deny(DenyReason::NotAContributor)
        );
    }

    #[test]
    fn a_coauthor_may_edit_and_publish_but_a_beta_reader_may_not_edit() {
        let owner = PseudId::new();
        let coauthor = PseudId::new();
        let beta = PseudId::new();
        let contributors = [
            contributor(owner, ContributorRole::Owner),
            contributor(coauthor, ContributorRole::Coauthor),
            contributor(beta, ContributorRole::BetaReader),
        ];

        assert!(can_edit_work(&actor(coauthor), &contributors).is_allowed());
        assert!(can_publish_work(&actor(coauthor), &contributors).is_allowed());
        assert_eq!(
            can_edit_work(&actor(beta), &contributors),
            Decision::Deny(DenyReason::InsufficientRole)
        );
    }

    fn facts<'a>(title: &'a str, chapters: usize, with_content: usize) -> PublicationFacts<'a> {
        PublicationFacts {
            title,
            chapter_count: chapters,
            chapters_with_content: with_content,
            lifecycle: Lifecycle::Draft,
        }
    }

    #[test]
    fn publication_requires_a_title_a_chapter_and_content() {
        let error = publication_readiness(&facts("   ", 1, 1)).expect_err("no title");
        assert!(error.field_errors().contains_key("title"));

        let error = publication_readiness(&facts("A Title", 0, 0)).expect_err("no chapters");
        assert!(error.field_errors().contains_key("chapters"));

        let error = publication_readiness(&facts("A Title", 2, 0)).expect_err("nothing written");
        assert!(error.field_errors().contains_key("chapters"));

        publication_readiness(&facts("A Title", 2, 1)).expect("ready");
    }

    #[test]
    fn a_deleted_work_is_never_ready() {
        let mut facts = facts("A Title", 1, 1);
        facts.lifecycle = Lifecycle::Deleted;
        assert_eq!(
            publication_readiness(&facts)
                .expect_err("deleted")
                .status_code(),
            403
        );
    }
}
