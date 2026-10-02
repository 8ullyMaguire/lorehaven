//! Generated content, and who gets credit for what (spec §51, M45-12).
//!
//! ## Why this is a domain module and not three schema columns
//!
//! Gaps review A3 said four sentences about generated fic and the CSV carried
//! `forbid | disclose | allow` as the whole of the requirement. The three values
//! are the easy half. The part that decides whether the feature is honest lives in
//! two rules that only code can hold:
//!
//! * **a posture a work can opt out of is not a posture.** §51.1 gives the
//!   operator three values and no per-author override, because `forbid` that an
//!   author can decline is `allow` with extra steps. [`GeneratedContentStore`]'s
//!   write path is where that is enforced — the decision to refuse happens before
//!   the work row exists, which is what makes `forbid` un-evadable.
//! * **the posture is recorded with the work, not looked up at read time.**
//!   [`GeneratedContentDeclaration`] is the value a work *carries*, so
//!   [`declared_under`] is a field rather than a join. An operator who tightens
//!   `allow` → `disclose` is making a change about future writes; a read-time join
//!   would relabel every work already published, changing terms its author agreed
//!   to under different ones.
//!
//! ## What this module deliberately cannot do
//!
//! There is no detector. §51.3 says no sanction derives from detector output, and a
//! type that cannot represent a detection cannot be asked to act on one. There is
//! also no confidence score on [`GeneratedContentDeclaration`], for the same
//! reason: a stored "probably generated" is a sanction with a delay on it, because
//! the next query cannot tell a measurement from an accusation.
//!
//! [`GeneratedContentStore`]: ../../lorehaven_db/generated_content/index.html

use std::fmt;

/// The instance's generated-content posture (§51.1).
///
/// Exactly three values, no `unknown`, and the default is [`Self::Forbid`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GeneratedContentPosture {
    /// Refuse at write time, before the work row exists.
    ///
    /// The default for a new instance. `allow` is the only value that changes what
    /// the corpus *is*, so §51.1 makes an instance choose it deliberately rather
    /// than arriving there by omission — and an operator who wants nothing
    /// generated has to do nothing, which is the safe direction to point.
    #[default]
    Forbid,
    /// Accept, and label the work wherever it is shown.
    Disclose,
    /// Accept, and do not label.
    ///
    /// An instance that wants generated fiction in its corpus does not have to run
    /// a disclosure UI it does not believe in. This is the whole reason the posture
    /// is instance policy rather than a fixed rule: the two other values assume the
    /// operator wants to prevent or to flag generated work, and some do not.
    Allow,
}

impl GeneratedContentPosture {
    /// The stable string the database stores.
    ///
    /// `#[repr]`-style lower-snake, matching the CHECK in 0109. `TryFrom<&str>`
    /// is the only way back, so a typo cannot become a fourth posture at runtime.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Forbid => "forbid",
            Self::Disclose => "disclose",
            Self::Allow => "allow",
        }
    }

    /// Every posture, in the order an operator sees them: safest first.
    ///
    /// Ordered rather than sorted because the order is the choice: the list reads
    /// as "more restrictive, then the labelled middle, then unrestricted".
    #[must_use]
    pub fn all() -> [Self; 3] {
        [Self::Forbid, Self::Disclose, Self::Allow]
    }

    /// Does a work declaring itself generated get accepted under this posture?
    ///
    /// The one decision §51.1 turns on. `Forbid` refuses; the other two accept.
    /// Note what this does *not* ask: nothing here consults the author, because
    /// §51.5 refuses a per-author opt-out under `forbid`.
    #[must_use]
    pub fn accepts_declared_generated(self) -> bool {
        !matches!(self, Self::Forbid)
    }

    /// Must a work under this posture carry a visible marker (§51.1)?
    ///
    /// Only `Disclose`. `Allow` deliberately does not: forcing a label on an
    /// instance that chose not to disclose would make `allow` a worse version of
    /// `disclose` rather than a real third option.
    #[must_use]
    pub fn requires_marker(self) -> bool {
        matches!(self, Self::Disclose)
    }
}

impl fmt::Display for GeneratedContentPosture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for GeneratedContentPosture {
    type Err = UnknownPosture;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Matched exactly. A case-insensitive parse would accept 'DISCLOSE', which
        // the CHECK in 0109 rejects — so a lenient parser would produce a value the
        // database then refuses to store, and the error would name the database
        // rather than the input.
        Self::all()
            .into_iter()
            .find(|p| p.as_str() == s)
            .ok_or_else(|| UnknownPosture(s.to_owned()))
    }
}

/// A posture string outside the fixed set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownPosture(pub String);

impl fmt::Display for UnknownPosture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "generated-content posture {:?} is not one of forbid, disclose, allow",
            self.0
        )
    }
}

impl std::error::Error for UnknownPosture {}

/// What an author declares about their own work (§51.1, §51.3).
///
/// Two values, because "I did not say anything" and "I said it is not generated"
/// are different records — the second is a claim, and this module keeps them apart
/// so a claim cannot be mistaken for an absence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GeneratedContentDeclaration {
    /// The author has not declared anything. The default.
    #[default]
    Undeclared,
    /// The author stated the work contains generated content.
    ///
    /// A *statement*, never a detection: §51.3 forbids persisting detector output
    /// as a fact, so there is no field here that a detector could fill.
    DeclaredGenerated,
}

impl GeneratedContentDeclaration {
    /// The stable string the database stores in `generated_declared_at`.
    ///
    /// Not the other way round: `None` means undeclared, and the *presence* of a
    /// declaration is what 0109's pair-check tests, so there is no string to parse
    /// and no fourth state to guard against.
    #[must_use]
    pub fn as_declared_at(self) -> Option<&'static str> {
        match self {
            Self::Undeclared => None,
            Self::DeclaredGenerated => Some("declared"),
        }
    }
}

/// What a work carries: its declaration and the posture it was published under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeneratedContentPostureOnWork {
    /// What the author declared, if anything.
    pub declaration: GeneratedContentDeclaration,
    /// The instance posture in force when the work was written.
    ///
    /// Recorded rather than looked up, and `Copy` because it is two small enums:
    /// there is no reason for a caller to hold this in an `Arc`.
    pub posture: GeneratedContentPosture,
}

impl GeneratedContentPostureOnWork {
    /// The combination a given declaration and instance posture produce.
    ///
    /// The one place `forbid` becomes a refusal. It is a pure function so the rule
    /// can be unit-tested without a database, and so the write path has exactly one
    /// place where it can be wrong.
    ///
    /// Not `const`: the refusal calls [`GeneratedContentPosture::accepts_declared_generated`],
    /// and making that `const` too would spread `const fn` across three types to
    /// avoid a runtime call on a two-variant match. The value is not used in any
    /// constant context.
    ///
    /// # Errors
    ///
    /// Returns [`PostureRefusal`] when the work declares itself generated and the
    /// posture is [`GeneratedContentPosture::Forbid`].
    pub fn resolve(
        declaration: GeneratedContentDeclaration,
        posture: GeneratedContentPosture,
    ) -> Result<Self, PostureRefusal> {
        if matches!(declaration, GeneratedContentDeclaration::DeclaredGenerated)
            && !posture.accepts_declared_generated()
        {
            return Err(PostureRefusal { posture });
        }
        Ok(Self {
            declaration,
            posture,
        })
    }

    /// Does this work need a visible generated-content marker?
    ///
    /// `true` only under `disclose`, per §51.1. A work that declared nothing under
    /// `disclose` has nothing to disclose, so the answer is `false` — the clause is
    /// about *labeling*, not about the instance's policy.
    #[must_use]
    pub fn needs_marker(&self) -> bool {
        self.posture.requires_marker()
            && matches!(
                self.declaration,
                GeneratedContentDeclaration::DeclaredGenerated
            )
    }
}

/// `forbid` refuses a work that declares itself generated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostureRefusal {
    /// The posture that refused it.
    pub posture: GeneratedContentPosture,
}

impl fmt::Display for PostureRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "this instance forbids generated content (posture `{}`), so the work \
             was not created",
            self.posture
        )
    }
}

impl std::error::Error for PostureRefusal {}

/// §51.2: author credit vests on reader completion, never on posting.
///
/// The rule that answers A3's farming problem. The paid-for action becomes "was
/// this finished and read", which a generator cannot farm — generating the last
/// chapter is not the expensive part, getting readers to finish it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreditVesting {
    /// A reader completed the work. §33's trust tiers decide whether they count.
    ReaderCompleted {
        /// Whether this reader is in the trusted set.
        trusted: bool,
    },
    /// Words were posted. Never vests anything, by itself.
    WordsPosted,
}

/// How many distinct trusted readers §51.2 requires before credit vests.
///
/// Named and documented rather than inlined so retuning it is one act, not a
/// number found in three places. The reasoning: one reader is an accident, and two
/// readers who happen to share a taste profile is not much better. Three is the
/// smallest count at which "people finished this" is not one person's habit — and
/// it is low enough that a genuine niche author is not left unconfirmed forever,
/// which matters because §51.2 keeps un-vested credit *visible*.
pub const TRUSTED_READERS_TO_VEST: usize = 3;

/// Whether a work's author credit has vested, given its completion events.
///
/// A pure count over the events, so §51.2's rule is testable without a database and
/// so the same rule runs on both engines — §49.3's reproducibility requirement,
/// restated for §51.2's arithmetic.
#[must_use]
pub fn credit_vested(events: &[CreditVesting]) -> bool {
    trusted_reader_completions(events) >= TRUSTED_READERS_TO_VEST
}

/// How many *distinct trusted readers* completed the work.
///
/// Untrusted completions are ignored entirely rather than counted and weighted —
/// §51.2 names trusted readers as the set, and a partial count would be a weight
/// §33's tiers do not describe.
///
/// ## The distinctness has to happen before this function
///
/// "Distinct readers" cannot be computed from [`CreditVesting`] alone, because the
/// variant carries *whether* a completion was trusted and not *who* it was. So this
/// counts trusted completion EVENTS, and it is the caller's job to pass one event
/// per reader.
///
/// That is a real constraint on callers rather than a convenient shortcut, so it is
/// stated where a caller reads it: the store reads `completion_events` grouped by
/// reader, and `deduplicated_by_reader` is what produces that grouping. Deduplicating
/// here instead would need a reader id on the variant, and putting the id on the
/// event rather than on the query would let a caller pass the same reader twice and
/// believe they were counting readers.
#[must_use]
pub fn trusted_reader_completions(events: &[CreditVesting]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, CreditVesting::ReaderCompleted { trusted: true }))
        .count()
}
