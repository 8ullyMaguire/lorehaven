//! Deciding what an import would do, before it does anything (spec §11.13).
//!
//! This module holds the rules and nothing else: no I/O, no adapters, no
//! database. It answers three questions the importer asks in order.
//!
//! 1. **Is this the same work I already have?** — [`looks_like_a_duplicate`].
//! 2. **What would importing it change?** — [`plan_import`].
//! 3. **Is a re-import needed at all?** — [`ImportPlan::is_measured`].
//!
//! The answers matter because spec §11.13 forbids overwriting an imported copy
//! destructively ("Never overwrite imported copies destructively. Create a new
//! snapshot and preserve notes, shelves, bookmarks, ratings, and progress where
//! mappable"), and because the acceptance criterion for Milestone 6 is that
//! importing the same URL twice *updates* rather than *duplicates*.
//!
//! # Two phases, on purpose
//!
//! A preview can only see what the source publishes *before* it reads the
//! chapter bodies: the chapter list, their identifiers, their order, their
//! titles. So [`plan_import`] reports changes in those terms. Whether a
//! chapter's **text** changed is a question that cannot be answered until the
//! chapter has been fetched and hashed, and it is answered then — by comparing
//! the new content checksum against the one the previous import recorded. The
//! two are kept apart rather than blurred, because a plan that claimed to know
//! about text changes before reading any text would be lying.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// One chapter as the planner sees it: where it sits, what identifies it, and
/// what it is called.
///
/// The identifier is the source's own when the source has one, because that is
/// what survives an author inserting a chapter in the middle. A planner that
/// keyed on position would report every later chapter as changed whenever one
/// was added near the front.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChapterIdentity {
    /// The source's stable identifier for this chapter.
    pub source_chapter_key: String,
    /// 1-based position in the work.
    pub ordinal: u32,
    /// The chapter's title as the source shows it.
    pub title: String,
    /// The chapter's word count, if the source reports one.
    pub word_count: Option<u32>,
}

impl ChapterIdentity {
    /// Build an identity.
    #[must_use]
    pub fn new(key: impl Into<String>, ordinal: u32, title: impl Into<String>) -> Self {
        Self {
            source_chapter_key: key.into(),
            ordinal,
            title: title.into(),
            word_count: None,
        }
    }

    /// Set the word count.
    #[must_use]
    pub fn with_word_count(mut self, count: u32) -> Self {
        self.word_count = Some(count);
        self
    }
}

/// The part of a work the planner needs, from either side of the comparison.
///
/// Deliberately small. The domain crate defines the vocabulary and the rules;
/// the adapter crate produces richer types of its own and the application maps
/// one into the other. That keeps the adapter crate free of this workspace's
/// types, which is what makes it testable against a recorded page with no
/// database in sight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedWork {
    /// The work's title as the source shows it.
    pub title: String,
    /// The author as displayed. Free text; most sources have no account to point
    /// at.
    pub author_text: String,
    /// The chapters, in the order the source lists them.
    pub chapters: Vec<ChapterIdentity>,
}

impl ImportedWork {
    /// A work with no chapters yet.
    #[must_use]
    pub fn new(title: impl Into<String>, author_text: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            author_text: author_text.into(),
            chapters: Vec::new(),
        }
    }

    /// Add a chapter.
    #[must_use]
    pub fn with_chapter(mut self, chapter: ChapterIdentity) -> Self {
        self.chapters.push(chapter);
        self
    }

    /// Classify this imported work's quality (spec §11.14).
    ///
    /// - `accepted` — a real title, a real author, a non-zero length.
    /// - `rejected` — certainly not a work: empty or placeholder title/author.
    /// - `held` — no confident call could be made.
    ///
    /// A zero word count is held, not rejected. Classification is a pure
    /// function of the fetched metadata.
    #[must_use]
    pub fn classify_quality(&self) -> ImportQuality {
        let title = self.title.trim();
        if title.is_empty() || is_placeholder_title(title) {
            return ImportQuality::Rejected {
                reason: "empty or placeholder title".into(),
            };
        }

        if self.author_text.trim().is_empty() {
            return ImportQuality::Rejected {
                reason: "empty author".into(),
            };
        }

        let total_words: u32 = self.chapters.iter().filter_map(|c| c.word_count).sum();
        if total_words == 0 {
            return ImportQuality::Held {
                reason: "no word count: cannot determine if content is real".into(),
            };
        }

        ImportQuality::Accepted
    }
}

/// Check if a title looks like placeholder text (spec §11.14).
fn is_placeholder_title(title: &str) -> bool {
    let lower = title.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "untitled" | "no title" | "unknown" | "title tbd" | "work" | "story"
    ) || lower.starts_with("chapter ")
        || lower.starts_with("work ")
        || lower.trim().is_empty()
}

/// Quality classification for an imported work (spec §11.14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum ImportQuality {
    /// A real work — real title, real author, non-zero length.
    Accepted,
    /// Certainly not a work; carries the reason.
    Rejected { reason: String },
    /// No confident call could be made; held for a person.
    Held { reason: String },
}

/// One difference between what we hold and what the source now offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum ChapterChange {
    /// A chapter the source has and we do not.
    Added {
        /// The chapter.
        chapter: ChapterIdentity,
    },
    /// A chapter we hold that the source no longer lists.
    ///
    /// Reported rather than acted on: spec §11.13 requires a *notice* for
    /// removed chapters, and deleting a reader's copy of a chapter because an
    /// author unpublished it would destroy notes and progress the reader owns.
    Removed {
        /// The chapter as we hold it.
        chapter: ChapterIdentity,
    },
    /// A chapter that moved. Its text is untouched; only its position changed.
    Reordered {
        /// The source's identifier for the chapter.
        source_chapter_key: String,
        /// Where it was.
        from: u32,
        /// Where it is now.
        to: u32,
    },
    /// A chapter whose title changed. The text may or may not have; that is
    /// answered after the fetch.
    Retitled {
        /// The source's identifier for the chapter.
        source_chapter_key: String,
        /// The title we hold.
        was: String,
        /// The title the source now shows.
        now: String,
    },
}

/// What an import would do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "plan", rename_all = "snake_case")]
pub enum ImportPlan {
    /// Nothing like this is held. The import creates a new copy.
    Create,
    /// A copy is held and the source differs. The import produces a new
    /// snapshot; the reader's notes, progress and ratings are mapped onto it
    /// where the chapter identities match.
    Update {
        /// Every difference found, in a stable order.
        changes: Vec<ChapterChange>,
    },
    /// A copy is held and nothing the preview can see has changed.
    ///
    /// This is not "the text is identical" — that is only knowable after
    /// fetching. It is "re-importing would add, remove, move and rename
    /// nothing", which is the honest answer a preview can give, and it is what
    /// lets the reader decline a pointless fetch of a two-hundred-chapter work.
    NoChange,
}

impl ImportPlan {
    /// Whether the plan would alter anything.
    #[must_use]
    pub const fn is_measured(&self) -> bool {
        matches!(self, Self::NoChange)
    }

    /// Whether the plan creates rather than updates.
    #[must_use]
    pub const fn is_creation(&self) -> bool {
        matches!(self, Self::Create)
    }

    /// The changes, when there are any.
    #[must_use]
    pub fn changes(&self) -> &[ChapterChange] {
        match self {
            Self::Update { changes } => changes,
            Self::Create | Self::NoChange => &[],
        }
    }

    /// How many chapters the plan would add.
    #[must_use]
    pub fn added(&self) -> usize {
        self.changes()
            .iter()
            .filter(|change| matches!(change, ChapterChange::Added { .. }))
            .count()
    }

    /// How many chapters the source no longer lists.
    #[must_use]
    pub fn removed(&self) -> usize {
        self.changes()
            .iter()
            .filter(|change| matches!(change, ChapterChange::Removed { .. }))
            .count()
    }

    /// How many chapters moved.
    #[must_use]
    pub fn reordered(&self) -> usize {
        self.changes()
            .iter()
            .filter(|change| matches!(change, ChapterChange::Reordered { .. }))
            .count()
    }

    /// How many chapters were renamed.
    #[must_use]
    pub fn retitled(&self) -> usize {
        self.changes()
            .iter()
            .filter(|change| matches!(change, ChapterChange::Retitled { .. }))
            .count()
    }
}

/// Decide what importing `fetched` would do to `existing`.
///
/// # The rules
///
/// * Identity is the source's chapter key, never the position. A chapter that
///   keeps its key and moves is [`ChapterChange::Reordered`] — the reader's
///   notes and progress stay attached to it, which is the whole point of having
///   a key.
/// * A chapter we hold and the source no longer lists is [`ChapterChange::Removed`].
///   It is reported, never deleted: spec §11.13 requires a notice, and the
///   reader's copy is the reader's.
/// * A chapter with the same key and a different title is
///   [`ChapterChange::Retitled`]. Whether its text changed too is decided after
///   the fetch.
/// * Two chapters with the same key in the source's own list is a source fault,
///   and it is reported as [`ChapterChange::Removed`] for the duplicate rather
///   than silently collapsing the two — an import that quietly drops a chapter
///   the source lists is worse than one that says so.
///
/// # Ordering
///
/// The changes come back in a stable order — additions by position, then
/// removals, then moves, then renames — so that a plan is comparable and a test
/// can assert on it without sorting.
#[must_use]
pub fn plan_import(existing: Option<&ImportedWork>, fetched: &ImportedWork) -> ImportPlan {
    let Some(existing) = existing else {
        return ImportPlan::Create;
    };

    let mut changes = Vec::new();

    // Index the two sides by the source's own key.
    let mut held: BTreeMap<&str, &ChapterIdentity> = BTreeMap::new();
    let mut duplicates: BTreeSet<String> = BTreeSet::new();
    for chapter in &existing.chapters {
        if held
            .insert(chapter.source_chapter_key.as_str(), chapter)
            .is_some()
        {
            duplicates.insert(chapter.source_chapter_key.clone());
        }
    }
    let fetched_by_key: BTreeMap<&str, &ChapterIdentity> = fetched
        .chapters
        .iter()
        .map(|chapter| (chapter.source_chapter_key.as_str(), chapter))
        .collect();

    // Additions, in the source's order.
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for chapter in &fetched.chapters {
        let key = chapter.source_chapter_key.as_str();
        if !held.contains_key(key) {
            changes.push(ChapterChange::Added {
                chapter: chapter.clone(),
            });
        } else if seen.contains(key) {
            // A key the source lists twice. Recorded as a duplicate rather than
            // imported twice; the unique constraint on the chapter table would
            // refuse the second write anyway, and failing the whole import over
            // it would help nobody.
            changes.push(ChapterChange::Removed {
                chapter: chapter.clone(),
            });
        } else {
            seen.insert(key);
        }
    }

    // Removals, in the order we held them.
    for chapter in &existing.chapters {
        if !fetched_by_key.contains_key(chapter.source_chapter_key.as_str()) {
            changes.push(ChapterChange::Removed {
                chapter: chapter.clone(),
            });
        }
    }

    // Moves and renames, over the keys both sides know.
    for chapter in &fetched.chapters {
        let Some(previous) = held.get(chapter.source_chapter_key.as_str()) else {
            continue;
        };
        if previous.ordinal != chapter.ordinal {
            changes.push(ChapterChange::Reordered {
                source_chapter_key: chapter.source_chapter_key.clone(),
                from: previous.ordinal,
                to: chapter.ordinal,
            });
        }
        if previous.title != chapter.title {
            changes.push(ChapterChange::Retitled {
                source_chapter_key: chapter.source_chapter_key.clone(),
                was: previous.title.clone(),
                now: chapter.title.clone(),
            });
        }
    }

    // A duplicate key in what we hold is not a change to report — it is data we
    // should never have written — but surfacing it means the import does not
    // quietly look "unchanged" while holding two rows for one chapter. This runs
    // *before* the no-change decision, because a work with a doubled row is not
    // unchanged: it is wrong, and saying "nothing to do" would leave it wrong.
    for key in &duplicates {
        changes.push(ChapterChange::Retitled {
            source_chapter_key: key.clone(),
            was: format!("(held twice: {key})"),
            now: String::new(),
        });
    }

    // A work whose *metadata* changed with no chapter changing is still a
    // change: a renamed work or a corrected author is what an update is for.
    let metadata_changed =
        existing.title != fetched.title || existing.author_text != fetched.author_text;

    if changes.is_empty() && !metadata_changed {
        return ImportPlan::NoChange;
    }

    ImportPlan::Update { changes }
}

/// Whether two imports of *different* sources look like the same work.
///
/// Spec §11.10 requires distinguishing "duplicate imports from the same source"
/// from "confirmed cross-posting" and "similar but unrelated works". This is the
/// weak signal only — the same title and the same author under two source keys —
/// and it is deliberately conservative.
///
/// It is a **suggestion**, never an action. Nothing merges, hides or links two
/// works on this basis: a title is not provenance, fanfiction titles repeat
/// constantly, and an automatic merge on a title match would silently attach one
/// author's work to another's. The reader is told; the reader decides. Merging
/// with evidence and review is spec §11.10's later work.
#[must_use]
pub fn looks_like_a_duplicate(a: &ImportedWork, b: &ImportedWork) -> bool {
    normalize_title(&a.title) == normalize_title(&b.title)
        && normalize_author(&a.author_text) == normalize_author(&b.author_text)
        && !normalize_title(&a.title).is_empty()
}

/// A title reduced to what makes two titles "the same" for a suggestion: case,
/// surrounding whitespace, and the punctuation that varies between sites.
#[must_use]
pub fn normalize_title(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut pending_space = false;
    for ch in raw.chars() {
        if ch.is_alphanumeric() {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
        } else {
            pending_space = true;
        }
    }
    out
}

/// An author reduced the same way, with the separators sources use between
/// multiple authors collapsed.
#[must_use]
pub fn normalize_author(raw: &str) -> String {
    let mut names: Vec<String> = raw
        .split(['&', ',', ';'])
        .map(normalize_title)
        .filter(|part| !part.is_empty())
        .collect();
    names.sort();
    names.join(" & ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work(title: &str, chapters: &[(&str, u32, &str)]) -> ImportedWork {
        let mut work = ImportedWork::new(title, "an author");
        for (key, ordinal, chapter_title) in chapters {
            work = work.with_chapter(ChapterIdentity::new(*key, *ordinal, *chapter_title));
        }
        work
    }

    #[test]
    fn a_work_we_do_not_hold_is_created() {
        let fetched = work("New", &[("1", 1, "One")]);
        assert_eq!(plan_import(None, &fetched), ImportPlan::Create);
        assert!(plan_import(None, &fetched).is_creation());
    }

    #[test]
    fn an_identical_work_is_unchanged() {
        let held = work("Same", &[("1", 1, "One"), ("2", 2, "Two")]);
        let fetched = work("Same", &[("1", 1, "One"), ("2", 2, "Two")]);
        let plan = plan_import(Some(&held), &fetched);
        assert_eq!(plan, ImportPlan::NoChange);
        assert!(plan.is_measured());
        assert_eq!(plan.added(), 0);
    }

    #[test]
    fn a_new_chapter_is_an_addition_not_everything_after_it_changing() {
        // The point of keying on the source's identifier: chapter 3 arriving at
        // the end must not look like chapters 1 and 2 having moved.
        let held = work("W", &[("1", 1, "One"), ("2", 2, "Two")]);
        let fetched = work("W", &[("1", 1, "One"), ("2", 2, "Two"), ("3", 3, "Three")]);
        let plan = plan_import(Some(&held), &fetched);
        assert_eq!(plan.added(), 1);
        assert_eq!(plan.reordered(), 0);
        assert_eq!(plan.removed(), 0);
        assert_eq!(plan.retitled(), 0);
        assert_eq!(
            plan.changes(),
            &[ChapterChange::Added {
                chapter: ChapterIdentity::new("3", 3, "Three")
            }]
        );
    }

    #[test]
    fn a_chapter_inserted_at_the_front_moves_the_others_without_renaming_them() {
        let held = work("W", &[("10", 1, "First"), ("20", 2, "Second")]);
        let fetched = work(
            "W",
            &[
                ("5", 1, "Prologue"),
                ("10", 2, "First"),
                ("20", 3, "Second"),
            ],
        );
        let plan = plan_import(Some(&held), &fetched);
        assert_eq!(plan.added(), 1);
        assert_eq!(plan.reordered(), 2);
        assert_eq!(plan.removed(), 0);
        // The chapters that moved keep their keys, so a reader's notes on
        // "First" stay on "First".
        assert!(plan.changes().contains(&ChapterChange::Reordered {
            source_chapter_key: "10".to_owned(),
            from: 1,
            to: 2,
        }));
    }

    #[test]
    fn a_chapter_the_source_dropped_is_reported_and_not_forgotten() {
        let held = work("W", &[("1", 1, "One"), ("2", 2, "Two")]);
        let fetched = work("W", &[("1", 1, "One")]);
        let plan = plan_import(Some(&held), &fetched);
        assert_eq!(plan.removed(), 1);
        // The change says what it was, so the notice can name it.
        let removed = plan
            .changes()
            .iter()
            .find_map(|change| match change {
                ChapterChange::Removed { chapter } => Some(chapter.clone()),
                _ => None,
            })
            .expect("a removal");
        assert_eq!(removed.title, "Two");
    }

    #[test]
    fn a_renamed_chapter_is_reported_as_a_rename() {
        let held = work("W", &[("1", 1, "Chapter One")]);
        let fetched = work("W", &[("1", 1, "The Beginning")]);
        let plan = plan_import(Some(&held), &fetched);
        assert_eq!(plan.retitled(), 1);
        assert_eq!(plan.added(), 0);
        assert_eq!(
            plan.changes(),
            &[ChapterChange::Retitled {
                source_chapter_key: "1".to_owned(),
                was: "Chapter One".to_owned(),
                now: "The Beginning".to_owned(),
            }]
        );
    }

    #[test]
    fn a_retitled_work_with_no_chapter_change_is_still_a_change() {
        let held = work("Old Name", &[("1", 1, "One")]);
        let fetched = work("New Name", &[("1", 1, "One")]);
        let plan = plan_import(Some(&held), &fetched);
        assert!(
            matches!(plan, ImportPlan::Update { .. }),
            "a renamed work is an update, got {plan:?}"
        );
    }

    #[test]
    fn a_key_the_source_lists_twice_does_not_silently_drop_a_chapter() {
        let held = work("W", &[("1", 1, "One")]);
        let fetched = work("W", &[("1", 1, "One"), ("1", 2, "One again")]);
        let plan = plan_import(Some(&held), &fetched);
        // The duplicate is surfaced as a removal — the second entry cannot be
        // stored, and an import that said "no change" while dropping it would be
        // lying about the source's own list.
        assert_eq!(plan.removed(), 1);
    }

    #[test]
    fn a_row_held_twice_is_surfaced_rather_than_looked_unchanged() {
        let held = work("W", &[("1", 1, "One"), ("1", 1, "One")]);
        let fetched = work("W", &[("1", 1, "One")]);
        let plan = plan_import(Some(&held), &fetched);
        assert!(
            matches!(plan, ImportPlan::Update { .. }),
            "a doubled row must not read as unchanged, got {plan:?}"
        );
    }

    #[test]
    fn changes_come_back_in_a_stable_order() {
        // A fixture in which all four kinds of change occur, so the ordering
        // rule is actually pinned rather than asserted over an empty tail.
        let held = work("W", &[("1", 1, "One"), ("2", 2, "Two"), ("3", 3, "Three")]);
        let fetched = work(
            "W",
            &[("1", 1, "One"), ("3", 2, "Three renamed"), ("4", 3, "Four")],
        );
        let first = plan_import(Some(&held), &fetched);
        let second = plan_import(Some(&held), &fetched);
        assert_eq!(first, second);
        let kinds: Vec<&str> = first
            .changes()
            .iter()
            .map(|change| match change {
                ChapterChange::Added { .. } => "added",
                ChapterChange::Removed { .. } => "removed",
                ChapterChange::Reordered { .. } => "reordered",
                ChapterChange::Retitled { .. } => "retitled",
            })
            .collect();
        assert_eq!(kinds, vec!["added", "removed", "reordered", "retitled"]);
        assert_eq!(first.added(), 1);
        assert_eq!(first.removed(), 1);
        assert_eq!(first.reordered(), 1);
        assert_eq!(first.retitled(), 1);
    }

    #[test]
    fn duplicate_detection_needs_both_the_title_and_the_author() {
        let a = ImportedWork::new("The Long Road", "Alice");
        let mut b = ImportedWork::new("The Long Road", "Bob");
        assert!(!looks_like_a_duplicate(&a, &b));
        b.author_text = "Alice".to_owned();
        assert!(looks_like_a_duplicate(&a, &b));
    }

    #[test]
    fn duplicate_detection_ignores_case_and_punctuation_but_not_words() {
        let a = ImportedWork::new("The Long Road!", "Alice");
        let b = ImportedWork::new("  the   long road ", "alice");
        assert!(looks_like_a_duplicate(&a, &b));
        // A different word is a different work, unlike a different dash.
        let c = ImportedWork::new("The Short Road", "Alice");
        assert!(!looks_like_a_duplicate(&a, &c));
    }

    #[test]
    fn an_empty_title_is_never_a_duplicate() {
        let a = ImportedWork::new("", "Alice");
        let b = ImportedWork::new("!!!", "Alice");
        assert!(!looks_like_a_duplicate(&a, &b));
    }

    #[test]
    fn author_normalisation_is_order_and_separator_insensitive() {
        assert_eq!(
            normalize_author("Alice & Bob"),
            normalize_author("Bob, Alice")
        );
        assert_eq!(
            normalize_author("Alice;Bob"),
            normalize_author("bob, alice")
        );
        assert_eq!(normalize_author("Alice & Bob"), "alice & bob");
        assert_eq!(normalize_author(""), "");
        assert_eq!(normalize_author("   "), "");
    }

    #[test]
    fn title_normalisation_keeps_non_latin_text() {
        assert_eq!(normalize_title("Bokura no"), "bokura no");
        assert_eq!(normalize_title("悪魔城ドラキュラ"), "悪魔城ドラキュラ");
        assert_eq!(normalize_title("Æon — Flux"), "æon flux");
    }

    // --- ImportQuality tests (spec §11.14) ---

    #[test]
    fn classify_accepted_real_work() {
        let work = ImportedWork::new("A Real Title", "Some Author")
            .with_chapter(ChapterIdentity::new("1", 1, "Chapter One").with_word_count(500));
        assert_eq!(work.classify_quality(), ImportQuality::Accepted);
    }

    #[test]
    fn classify_rejected_empty_title() {
        let work = ImportedWork::new("", "Author")
            .with_chapter(ChapterIdentity::new("1", 1, "Ch").with_word_count(100));
        assert_eq!(
            work.classify_quality(),
            ImportQuality::Rejected {
                reason: "empty or placeholder title".into()
            }
        );
    }

    #[test]
    fn classify_rejected_placeholder_title() {
        for title in &["untitled", "no title", "unknown", "title tbd", "chapter 1", "work"] {
            let work = ImportedWork::new(*title, "Author")
                .with_chapter(ChapterIdentity::new("1", 1, "Ch").with_word_count(100));
            assert_eq!(
                work.classify_quality(),
                ImportQuality::Rejected {
                    reason: "empty or placeholder title".into()
                },
                "title: {}",
                title
            );
        }
    }

    #[test]
    fn classify_rejected_empty_author() {
        let work = ImportedWork::new("Title", "")
            .with_chapter(ChapterIdentity::new("1", 1, "Ch").with_word_count(100));
        assert_eq!(
            work.classify_quality(),
            ImportQuality::Rejected {
                reason: "empty author".into()
            }
        );
    }

    #[test]
    fn classify_held_zero_word_count() {
        let work = ImportedWork::new("Title", "Author")
            .with_chapter(ChapterIdentity::new("1", 1, "Ch"));
        assert_eq!(
            work.classify_quality(),
            ImportQuality::Held {
                reason: "no word count: cannot determine if content is real".into()
            }
        );
    }

    #[test]
    fn classify_held_no_chapters() {
        let work = ImportedWork::new("Title", "Author");
        assert_eq!(
            work.classify_quality(),
            ImportQuality::Held {
                reason: "no word count: cannot determine if content is real".into()
            }
        );
    }

    #[test]
    fn classify_quality_is_pure() {
        // Same input always produces same output.
        let work = ImportedWork::new("Story", "Writer")
            .with_chapter(ChapterIdentity::new("1", 1, "One").with_word_count(100));
        let first = work.classify_quality();
        let second = work.classify_quality();
        assert_eq!(first, second);
    }
}
