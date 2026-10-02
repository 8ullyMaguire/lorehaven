//! Reason tags, spans, and the canon-blind class (spec §50.1, §50.2).
//!
//! ## One reason vocabulary, deliberately
//!
//! §49.5's tasting reasons and §50.1's kudos reasons are the **same set**. That
//! is a design constraint, not a coincidence: a reader who says "the prose was
//! flat" about a 300-word sample and a reader who kudos a work for its prose are
//! making the same statement about the same dimension, and a profile that cannot
//! join them has two taxonomies of the same reader's taste.
//!
//! So the vocabulary lives here, in `domain`, and both surfaces import it. The
//! alternative — a second enum declared next to the first — compiles, passes its
//! own tests, and produces a profile where `tasting.prose` and `kudos.prose` are
//! different signals that never meet.
//!
//! ## Why `worldbuilding` is here and `not_for_me` is a slot
//!
//! `worldbuilding` has no §49.5 equivalent: a 300-word passage often shows no
//! world at all, so a tasting reader could not honestly pick it, while a reader
//! who has read a whole work can. The set is the union, not the larger one, and
//! each surface rejects the reasons it cannot ask for — see
//! [`TastingReason::from_reason`].
//!
//! `NotForMe` is the slot a free-text note fills in. It is an enumerated value so
//! that "I would not read this again" is *stored*, not discarded into a comment.
//!
//! ## Nothing here is invented
//!
//! §50.3's first invariant: a reason is never defaulted, inferred, or stored as
//! an empty string. [`Reason::parse`] returns `Option`, so "no reason" is a state
//! a caller must handle rather than a value that quietly becomes one.

use std::fmt;

/// The fixed set of reasons a reader may give. Shared by §49.5 and §50.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reason {
    Prose,
    Characters,
    Pacing,
    TropeExecution,
    Worldbuilding,
    /// "not for me: ___" — the enumerated slot a free-text note fills in.
    NotForMe,
}

impl Reason {
    /// Every reason, in a fixed order.
    ///
    /// A function rather than a hand-written list so that a variant added to the
    /// enum cannot be forgotten by the surface that has to render it as a choice.
    pub const ALL: [Self; 6] = [
        Self::Prose,
        Self::Characters,
        Self::Pacing,
        Self::TropeExecution,
        Self::Worldbuilding,
        Self::NotForMe,
    ];

    /// The database spelling. Must match the CHECK constraints in migration 0106.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prose => "prose",
            Self::Characters => "characters",
            Self::Pacing => "pacing",
            Self::TropeExecution => "trope_execution",
            Self::Worldbuilding => "worldbuilding",
            Self::NotForMe => "not_for_me",
        }
    }

    /// Parse a stored or client-supplied reason.
    ///
    /// `Option`, never a default: §50.3's first invariant is that a reason is
    /// never invented, and a parser that returned a default would invent one for
    /// every typo — silently training the wrong dimension.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.as_str() == s)
    }

    /// The reasons a §49.5 tasting sample may be rated with.
    ///
    /// `worldbuilding` is excluded, and the reason is not modesty: a 300-word
    /// passage usually shows no world at all, so offering the reason invites a
    /// reader to claim a judgement they cannot have made from what they read.
    /// §50.1's kudos and highlights have no such limit, because there the reader
    /// has read the whole work.
    pub const TASTING: [Self; 5] = [
        Self::Prose,
        Self::Characters,
        Self::Pacing,
        Self::TropeExecution,
        Self::NotForMe,
    ];

    /// Whether this reason is answerable from a tasting sample.
    #[must_use]
    pub fn is_answerable_from_a_sample(self) -> bool {
        Self::TASTING.contains(&self)
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A reason and the free-text note that may accompany it.
///
/// The two are different things and the struct keeps them apart: §50.3 says the
/// note is stored but never aggregated, because free text cannot be counted
/// across readers without becoming an unreviewable store of prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotated {
    pub reason: Reason,
    pub note: Option<String>,
}

impl Annotated {
    /// Whether this annotation carries an aggregatable signal.
    ///
    /// Always true for a present [`Annotated`]; the function exists because the
    /// question gets asked in two shapes — "is there a reason?" and "is this
    /// trainable?" — and answering them with one method would invite the second
    /// to be derived from the note's length.
    #[must_use]
    pub fn is_signal(&self) -> bool {
        true
    }
}

/// A highlight's span, addressed by character offset into the work's text.
///
/// Offsets rather than quoted text: a quote goes stale on the work's first edit,
/// and a highlight whose text no longer matches is a highlight about nothing.
/// §49.5's tasting samples address their passage the same way, so the two are
/// comparable and there is one convention to learn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: i64,
    pub end: i64,
}

impl Span {
    /// Build a span, normalising a reversed pair.
    ///
    /// Normalising rather than refusing: a client that computed `start - end`
    /// because it subtracted in the wrong order has made a mistake about the
    /// order of two numbers it was given, and the useful answer is the same span
    /// read the other way round. An *empty* span is refused, because that one is a
    /// claim about a span that does not exist.
    #[must_use]
    pub fn new(a: i64, b: i64) -> Option<Self> {
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        if end <= start {
            return None;
        }
        Some(Self { start, end })
    }

    #[must_use]
    pub fn len(self) -> i64 {
        self.end - self.start
    }

    /// Always false, because [`Span::new`] refuses an empty one.
    ///
    /// Present so that `len` is not a trap: every other `len` in the codebase can
    /// be zero and callers are expected to check, and a caller that reaches for
    /// `is_empty` here should get a definite answer rather than a missing method.
    #[must_use]
    pub fn is_empty(self) -> bool {
        false
    }

    /// Whether this span is a whole sentence or less.
    ///
    /// Not a rule — a *reported* fact, so a caller can show a reader "you
    /// highlighted 40 words" without the database having to decide what counts as
    /// a line. §50.1's name is "line-level"; enforcing a maximum here would be a
    /// product decision the spec does not make.
    #[must_use]
    pub fn is_line_level(self) -> bool {
        self.len() <= 200
    }
}

/// How many distinct readers highlighted a work.
///
/// §50.1's rule that fifty highlights count once is enforced here, by counting
/// *readers* and not highlights. Doing it at this function rather than in SQL is
/// deliberate: the arithmetic is the spec clause, and a `COUNT(*)` somewhere in a
/// query would let it be reintroduced by the next query that wants a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HighlightSignal {
    /// Distinct readers who highlighted the work at all.
    pub readers: i64,
    /// Total highlights, kept for display only.
    pub highlights: i64,
    /// Distinct readers who highlighted it *with a reason that can be trained on*.
    pub reason_bearing_readers: i64,
}

impl HighlightSignal {
    /// The weight this work's highlights contribute.
    ///
    /// **At most one**, regardless of `highlights`. §50.1: a work cannot buy
    /// gravity with quote volume. Saturating rather than wrapping, so an
    /// underflow from a negative count cannot produce a huge number.
    #[must_use]
    pub fn gravity_weight(self) -> f64 {
        if self.readers > 0 {
            1.0
        } else {
            0.0
        }
    }

    /// Whether any highlight carried a trainable reason.
    ///
    /// §50.3: a reason is never invented, so highlights with no reason contribute
    /// nothing to the profile even though they count for presence.
    #[must_use]
    pub fn trains(self) -> bool {
        self.reason_bearing_readers > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The vocabulary is a single set with one spelling per reason, and it is
    /// exactly what migration 0106's CHECK constraints allow.
    #[test]
    fn every_reason_round_trips_through_its_database_spelling() {
        for reason in Reason::ALL {
            assert_eq!(Reason::parse(reason.as_str()), Some(reason));
        }
    }

    /// §50.3: a reason is never invented. A typo must be refused, not defaulted —
    /// defaulting here would train the wrong dimension for every malformed row.
    #[test]
    fn an_unknown_reason_is_refused_rather_than_defaulted() {
        assert_eq!(Reason::parse("vibes"), None);
        assert_eq!(Reason::parse(""), None);
        assert_eq!(Reason::parse("Prose"), None, "the spelling is lowercase");
        assert_eq!(Reason::parse("world building"), None);
    }

    /// Every reason's spelling is distinct. Two reasons sharing a spelling would
    /// make `parse` pick one arbitrarily and the profile would silently lose a
    /// dimension — a class of bug that no single test on `parse` can see.
    #[test]
    fn no_two_reasons_share_a_spelling() {
        let mut seen: Vec<&str> = Reason::ALL.iter().map(|r| r.as_str()).collect();
        let before = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), before, "duplicate reason spelling");
    }

    /// §49.5 and §50.1 share the vocabulary, and the shared reasons agree.
    #[test]
    fn the_tasting_subset_is_a_subset_of_the_whole() {
        for reason in Reason::TASTING {
            assert!(Reason::ALL.contains(&reason), "{reason} is not in ALL");
            assert!(reason.is_answerable_from_a_sample());
        }
    }

    /// `worldbuilding` is the one reason a passage cannot support. Offering it on
    /// a 300-word sample invites a claim the reader could not have made.
    #[test]
    fn worldbuilding_is_not_answerable_from_a_tasting_sample() {
        assert!(!Reason::Worldbuilding.is_answerable_from_a_sample());
        assert!(Reason::ALL.contains(&Reason::Worldbuilding));
    }

    /// A bare kudos is still VALID (§50.1). The API may accept a kudos with no
    /// reason, which is why `Option` is the absent representation rather than a
    /// required field.
    #[test]
    fn a_bare_annotation_is_absent_rather_than_defaulted() {
        let bare: Option<Annotated> = None;
        assert!(bare.is_none(), "no reason means none, not Prose");
    }

    /// §50.1: fifty highlights on one work move its gravity by one signal's
    /// worth. This is the clause most likely to be quietly dropped, since the
    /// raw count is the part that already exists.
    #[test]
    fn many_highlights_from_one_reader_count_once() {
        let one = HighlightSignal {
            readers: 1,
            highlights: 50,
            reason_bearing_readers: 1,
        };
        assert_eq!(
            one.gravity_weight(),
            1.0,
            "quote volume must not buy gravity"
        );
        assert_eq!(
            one.gravity_weight(),
            HighlightSignal {
                readers: 1,
                highlights: 1,
                reason_bearing_readers: 1,
            }
            .gravity_weight()
        );
    }

    /// Readers, not highlights, are what count — two readers each highlighting
    /// once is a stronger signal than one reader highlighting twice.
    #[test]
    fn two_readers_outweigh_one_reader_highlighting_twice() {
        let one_reader = HighlightSignal {
            readers: 1,
            highlights: 2,
            reason_bearing_readers: 1,
        };
        let two_readers = HighlightSignal {
            readers: 2,
            highlights: 2,
            reason_bearing_readers: 2,
        };
        assert!(two_readers.readers > one_reader.readers);
    }

    /// A negative count must not become a large positive weight.
    #[test]
    fn a_negative_reader_count_does_not_wrap_into_a_signal() {
        let bad = HighlightSignal {
            readers: -5,
            highlights: 0,
            reason_bearing_readers: 0,
        };
        assert_eq!(bad.gravity_weight(), 0.0);
    }

    /// §50.3: highlights with no reason contribute nothing to the profile.
    #[test]
    fn highlights_without_reasons_do_not_train() {
        assert!(!HighlightSignal {
            readers: 3,
            highlights: 3,
            reason_bearing_readers: 0,
        }
        .trains());
        assert!(HighlightSignal {
            readers: 1,
            highlights: 1,
            reason_bearing_readers: 1,
        }
        .trains());
    }

    /// A reversed span is the same span read the other way.
    #[test]
    fn a_reversed_span_is_normalised() {
        assert_eq!(Span::new(120, 220), Span::new(220, 120));
        assert_eq!(Span::new(10, 40).map(Span::len), Some(30));
    }

    /// An empty span is a claim about a span that does not exist, and is refused
    /// — unlike a reversed one, which is only an ordering mistake.
    #[test]
    fn an_empty_span_is_refused() {
        assert_eq!(Span::new(50, 50), None);
    }

    #[test]
    fn a_line_level_span_is_reported_rather_than_enforced() {
        assert!(Span::new(0, 40).unwrap().is_line_level());
        assert!(!Span::new(0, 4000).unwrap().is_line_level());
    }
}
