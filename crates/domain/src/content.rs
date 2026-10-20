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

use serde::{Deserialize, Serialize};

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
    /// May edit units; may not publish.
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

// ---------------------------------------------------------------------------
// The work's own vocabulary (spec §6.2)
// ---------------------------------------------------------------------------

/// The form a work takes (spec §6.2).
///
/// **A closed vocabulary of eighteen**, because a work's form is what the
/// reader's expectations are built on: a podfic is listened to and a dataset is
/// read, and an archive that cannot tell them apart cannot serve either well.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkFormat {
    /// Prose fiction.
    Prose,
    /// Poetry.
    Poetry,
    /// An essay.
    Essay,
    /// An article.
    Article,
    /// A book-length work.
    Book,
    /// Fanfiction. **The default**, because this is an archive of fanfiction and
    /// a work that has not said what it is is a work that has not contradicted
    /// being fanfiction.
    #[default]
    Fanwork,
    /// A translation.
    Translation,
    /// A podcast episode.
    Podfic,
    /// An audiobook.
    Audiobook,
    /// A fan film.
    FanFilm,
    /// A video.
    Video,
    /// A fan comic.
    FanComic,
    /// A comic.
    Comic,
    /// A scan of a zine.
    ZineScan,
    /// An image.
    Image,
    /// An interactive work.
    Interactive,
    /// A dataset.
    Dataset,
    /// Something the archive has no better word for.
    Other,
}

impl WorkFormat {
    /// The stable string the database stores.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prose => "prose",
            Self::Poetry => "poetry",
            Self::Essay => "essay",
            Self::Article => "article",
            Self::Book => "book",
            Self::Fanwork => "fanwork",
            Self::Translation => "translation",
            Self::Podfic => "podfic",
            Self::Audiobook => "audiobook",
            Self::FanFilm => "fan_film",
            Self::Video => "video",
            Self::FanComic => "fan_comic",
            Self::Comic => "comic",
            Self::ZineScan => "zine_scan",
            Self::Image => "image",
            Self::Interactive => "interactive",
            Self::Dataset => "dataset",
            Self::Other => "other",
        }
    }

    /// Parse the storage form, refusing an unknown format.
    ///
    /// **Refusing rather than defaulting**, because a default is a claim about
    /// a work nobody made: a work stored as `fan_work` would silently become a
    /// fanwork, and the work's form is what a reader's expectations rest on.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "prose" => Some(Self::Prose),
            "poetry" => Some(Self::Poetry),
            "essay" => Some(Self::Essay),
            "article" => Some(Self::Article),
            "book" => Some(Self::Book),
            "fanwork" => Some(Self::Fanwork),
            "translation" => Some(Self::Translation),
            "podfic" => Some(Self::Podfic),
            "audiobook" => Some(Self::Audiobook),
            "fan_film" => Some(Self::FanFilm),
            "video" => Some(Self::Video),
            "fan_comic" => Some(Self::FanComic),
            "comic" => Some(Self::Comic),
            "zine_scan" => Some(Self::ZineScan),
            "image" => Some(Self::Image),
            "interactive" => Some(Self::Interactive),
            "dataset" => Some(Self::Dataset),
            "other" => Some(Self::Other),
            _ => None,
        }
    }

    /// Every format, in the order the specification names them.
    #[must_use]
    pub fn all() -> Vec<WorkFormat> {
        vec![
            Self::Prose,
            Self::Poetry,
            Self::Essay,
            Self::Article,
            Self::Book,
            Self::Fanwork,
            Self::Translation,
            Self::Podfic,
            Self::Audiobook,
            Self::FanFilm,
            Self::Video,
            Self::FanComic,
            Self::Comic,
            Self::ZineScan,
            Self::Image,
            Self::Interactive,
            Self::Dataset,
            Self::Other,
        ]
    }
}

/// How a work's discussion works (spec §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscussionMode {
    /// **A discussion thread and comments, and the thread is the discussion.**
    ///
    /// The default, and the only mode that is *the archive's own*: the thread
    /// is a first-class surface with its own moderation, its own reading state
    /// and its own export.
    #[default]
    ThreadOnly,
    /// Comments only — the legacy shape.
    ///
    /// **Supported indefinitely**, because a work published before the thread
    /// existed has comments and no thread, and a mode that refused it would be a
    /// mode that deleted the discussion of half the archive.
    CommentsOnly,
    /// Both.
    ///
    /// **Operator-warned**: two surfaces for one discussion means two places a
    /// reader has to look and two places a moderator has to moderate, and the
    /// warning says so out loud.
    Both,
}

impl DiscussionMode {
    /// The stable string the database stores.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ThreadOnly => "thread_only",
            Self::CommentsOnly => "comments_only",
            Self::Both => "both",
        }
    }

    /// Parse the storage form, refusing an unknown mode.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "thread_only" => Some(Self::ThreadOnly),
            "comments_only" => Some(Self::CommentsOnly),
            "both" => Some(Self::Both),
            _ => None,
        }
    }

    /// Every mode.
    #[must_use]
    pub fn all() -> Vec<DiscussionMode> {
        vec![Self::ThreadOnly, Self::CommentsOnly, Self::Both]
    }
}

/// The author's statement about AI training (spec §15.4).
///
/// **A statement, not a protection.** The archive indexes and recommends
/// AI-generated works (N-4) and this field says nothing about whether a work
/// is indexed; it records what the author said about their work being used to
/// train a model. It is shown as *"this author's stated preference, recorded"*
/// and **never as "protected"** — a word that promises the archive something it
/// does not do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiTraining {
    /// The author allows it.
    Allow,
    /// The author does not allow it.
    Deny,
    /// The author has not said.
    #[default]
    Unset,
}

impl AiTraining {
    /// The stable string the database stores.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
            Self::Unset => "unset",
        }
    }

    /// Parse the storage form, refusing an unknown value.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "allow" => Some(Self::Allow),
            "deny" => Some(Self::Deny),
            "unset" => Some(Self::Unset),
            _ => None,
        }
    }

    /// Every value.
    #[must_use]
    pub fn all() -> Vec<AiTraining> {
        vec![Self::Allow, Self::Deny, Self::Unset]
    }

    /// The wording the work's page shows.
    ///
    /// **"this author's stated preference, recorded"** — the specification's
    /// own words (spec §15.4). And the wording is *never* "protected": a badge
    /// that promises something the archive does not do is a lie with a logo.
    #[must_use]
    pub const fn as_statement(self) -> &'static str {
        match self {
            Self::Allow => "this author's stated preference, recorded: training allowed",
            Self::Deny => "this author's stated preference, recorded: training declined",
            Self::Unset => "no statement recorded",
        }
    }
}

/// The work's **derived** activity (spec §6.2).
///
/// **Never a penalty.** The value says how long ago the work was published and
/// it is *not* a score, a demotion, or a reason to show a work less. A work
/// that is dormant is still in the archive, still reachable, still exportable,
/// and still the author's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DerivedActivity {
    /// Published within three months.
    #[default]
    Active,
    /// Published between three and twelve months ago.
    Slow,
    /// Published more than twelve months ago.
    Dormant,
    /// The author says the work is concluded.
    ///
    /// **The author's state always wins** — the nightly recompute must never
    /// overwrite it, because a derived value that overwrites a stated one is
    /// the archive deciding something the author already decided.
    Concluded,
}

impl DerivedActivity {
    /// The stable string the database stores.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Slow => "slow",
            Self::Dormant => "dormant",
            Self::Concluded => "concluded",
        }
    }

    /// Parse the storage form, refusing an unknown value.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "active" => Some(Self::Active),
            "slow" => Some(Self::Slow),
            "dormant" => Some(Self::Dormant),
            "concluded" => Some(Self::Concluded),
            _ => None,
        }
    }

    /// Every value.
    #[must_use]
    pub fn all() -> Vec<DerivedActivity> {
        vec![Self::Active, Self::Slow, Self::Dormant, Self::Concluded]
    }

    /// Whether this value is the author's own statement.
    ///
    /// **The one the recompute must not overwrite.**
    #[must_use]
    pub const fn is_author_state(self) -> bool {
        matches!(self, Self::Concluded)
    }
}

/// The months a work has to be published within to be *active*.
///
/// Three months, per the specification's table. Named so the rule is one
/// constant rather than a number in a comparison.
pub const ACTIVE_MONTHS: i64 = 3;

/// The months a work has to be published within to be more than *dormant*.
///
/// Twelve months.
pub const DORMANT_MONTHS: i64 = 12;

/// Derive a work's activity from when it was last published.
///
/// **The author's state wins**, so `Concluded` comes back as `Concluded` no
/// matter what the date says. And the value is *never a penalty* — it says how
/// long ago the work was published and nothing else.
#[must_use]
pub fn derive_activity(
    months_since_publication: Option<i64>,
    author_state: DerivedActivity,
) -> DerivedActivity {
    if author_state.is_author_state() {
        return author_state;
    }
    match months_since_publication {
        None => DerivedActivity::Active,
        Some(months) if months <= ACTIVE_MONTHS => DerivedActivity::Active,
        Some(months) if months <= DORMANT_MONTHS => DerivedActivity::Slow,
        Some(_) => DerivedActivity::Dormant,
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
    /// How many units exist.
    pub unit_count: usize,
    /// How many of them have a revision with any text.
    pub units_with_content: usize,
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
    if facts.unit_count == 0 {
        return Err(AppError::field(
            "units",
            "A work needs at least one unit before it can be published.",
        ));
    }
    if facts.units_with_content == 0 {
        return Err(AppError::field(
            "units",
            "Write something in a unit before publishing: an empty work has nothing to read.",
        ));
    }
    if matches!(facts.lifecycle, Lifecycle::Deleted) {
        return Err(AppError::AccessDenied);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_work_format_round_trips_and_nothing_else_parses() {
        for format in WorkFormat::all() {
            assert_eq!(
                WorkFormat::parse(format.as_str()),
                Some(format),
                "the round trip holds for {format:?}"
            );
        }
        assert_eq!(
            WorkFormat::all().len(),
            18,
            "the specification names eighteen"
        );
        assert_eq!(
            WorkFormat::parse("fan_work"),
            None,
            "a format the specification does not name is refused, not guessed"
        );
    }

    #[test]
    fn the_default_format_is_fanwork_and_the_default_discussion_is_the_thread() {
        assert_eq!(WorkFormat::default(), WorkFormat::Fanwork);
        assert_eq!(DiscussionMode::default(), DiscussionMode::ThreadOnly);
        assert_eq!(AiTraining::default(), AiTraining::Unset);
        assert_eq!(DerivedActivity::default(), DerivedActivity::Active);
    }

    #[test]
    fn the_ai_statement_is_never_called_protected() {
        // **The wording is the specification's** (spec §15.4), and the word
        // "protected" is the one word it must never use -- a badge that promises
        // something the archive does not do is a lie with a logo.
        for value in AiTraining::all() {
            let statement = value.as_statement();
            assert!(
                !statement.to_lowercase().contains("protect"),
                "the statement must never promise protection: {statement}"
            );
        }
        assert!(
            AiTraining::Deny
                .as_statement()
                .contains("stated preference"),
            "and it says what it is: a recorded statement"
        );
    }

    #[test]
    fn the_derived_activity_is_never_a_penalty_and_the_authors_state_wins() {
        // The author's state always wins: a concluded work stays concluded, and
        // the nightly recompute must not overwrite it.
        assert_eq!(
            derive_activity(Some(0), DerivedActivity::Concluded),
            DerivedActivity::Concluded,
            "the author's state wins over the date"
        );
        assert_eq!(
            derive_activity(Some(600), DerivedActivity::Concluded),
            DerivedActivity::Concluded
        );
        // And the derived values are the specification's bands.
        assert_eq!(
            derive_activity(Some(0), DerivedActivity::Active),
            DerivedActivity::Active
        );
        assert_eq!(
            derive_activity(Some(3), DerivedActivity::Active),
            DerivedActivity::Active
        );
        assert_eq!(
            derive_activity(Some(4), DerivedActivity::Active),
            DerivedActivity::Slow
        );
        assert_eq!(
            derive_activity(Some(12), DerivedActivity::Active),
            DerivedActivity::Slow
        );
        assert_eq!(
            derive_activity(Some(13), DerivedActivity::Active),
            DerivedActivity::Dormant
        );
    }

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

    fn facts<'a>(title: &'a str, units: usize, with_content: usize) -> PublicationFacts<'a> {
        PublicationFacts {
            title,
            unit_count: units,
            units_with_content: with_content,
            lifecycle: Lifecycle::Draft,
        }
    }

    #[test]
    fn publication_requires_a_title_a_unit_and_content() {
        let error = publication_readiness(&facts("   ", 1, 1)).expect_err("no title");
        assert!(error.field_errors().contains_key("title"));

        let error = publication_readiness(&facts("A Title", 0, 0)).expect_err("no units");
        assert!(error.field_errors().contains_key("units"));

        let error = publication_readiness(&facts("A Title", 2, 0)).expect_err("nothing written");
        assert!(error.field_errors().contains_key("units"));

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
