//! The reader's library: shelves, bookmarks, private tags, reading statuses and
//! saved views (spec §16 as the plan numbers it; §14 in the spec text).
//!
//! This module holds the vocabulary and the rules, and nothing else — no I/O
//! and no database. Three of those rules are the reason it exists rather than
//! being inlined into the repository:
//!
//! 1. **A public view may not carry a private filter** ([`validate_query`]).
//!    Spec §14's acceptance list requires that shared views cannot expose
//!    private filters, and the only place that can be decided is before the
//!    view is stored. A shelf name, a private tag and a reading status are all
//!    facts about one account; a view that filtered on them and was then shared
//!    would publish the filter and, with it, the fact.
//! 2. **A private tag is not a public tag.** [`TagScope`] names the two so the
//!    difference is a type rather than a comment, and the private one can never
//!    be passed to a query that serves another account.
//! 3. **A batch reports per item** ([`BatchOutcome`]). Spec §14.3 gives the
//!    shape; the reason for it is that "3 of 5 removed, 2 were already gone" is
//!    the honest answer and a single boolean is not.

use serde::{Deserialize, Serialize};

/// Where a reader has got to with a work, in their own words.
///
/// These are the five the plan names. They are the *reader's* status, not the
/// work's publication state: a work can be "ongoing" (the author is still
/// posting) while the reader has it "on hold", and the two must not be
/// conflated — see [`crate::content::WorkStatus`] for the other one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReadingStatus {
    /// Wants to read it; has not started.
    WantToRead,
    /// Currently reading.
    Reading,
    /// Paused, with the intention of returning.
    OnHold,
    /// Abandoned.
    Dropped,
    /// Completed.
    Finished,
}

impl ReadingStatus {
    /// The stored form. Matches the `CHECK` constraint on `reading_status`.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WantToRead => "want-to-read",
            Self::Reading => "reading",
            Self::OnHold => "on-hold",
            Self::Dropped => "dropped",
            Self::Finished => "finished",
        }
    }

    /// Parse the stored form.
    ///
    /// Returns `None` rather than a default, because a status this build does
    /// not know about is a row written by a newer build or a bug, and silently
    /// reporting it as "want to read" would be wrong in both cases.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "want-to-read" => Some(Self::WantToRead),
            "reading" => Some(Self::Reading),
            "on-hold" => Some(Self::OnHold),
            "dropped" => Some(Self::Dropped),
            "finished" => Some(Self::Finished),
            _ => None,
        }
    }

    /// Every status, in the order a menu should offer them.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[
            Self::WantToRead,
            Self::Reading,
            Self::OnHold,
            Self::Dropped,
            Self::Finished,
        ]
    }

    /// Whether moving *to* this status means the reader has begun.
    ///
    /// Used to stamp `started_at` once and never overwrite it: a reader who
    /// drops a work and later returns to it started reading the first time.
    #[must_use]
    pub fn is_started(&self) -> bool {
        matches!(
            self,
            Self::Reading | Self::OnHold | Self::Dropped | Self::Finished
        )
    }

    /// Whether moving *to* this status means the reader has finished.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        matches!(self, Self::Finished)
    }
}

impl std::fmt::Display for ReadingStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which of the two tag tables a tag refers to.
///
/// A separate table and a separate type, because the two are not the same fact
/// with a visibility flag: a public tag describes a work and is shared, and a
/// private tag describes one reader's filing and is not. Spec §16's first
/// pitfall is exactly the shape of leak a single table with an `is_private`
/// column produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagScope {
    /// The reader's own tag. Never returned to anyone else.
    Private,
    /// The site's shared taxonomy (M9). Not written by this module.
    Public,
}

/// How a library listing is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LibrarySort {
    /// Most recently added to the library first.
    #[default]
    Recent,
    /// By title.
    Title,
    /// Most recently updated at the source first.
    Updated,
    /// Longest first.
    Words,
    /// The reader's own ordering within a shelf.
    Position,
}

impl LibrarySort {
    /// The stored form.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Recent => "recent",
            Self::Title => "title",
            Self::Updated => "updated",
            Self::Words => "words",
            Self::Position => "position",
        }
    }

    /// Parse the stored form, falling back to the default for an unknown value.
    ///
    /// Unlike [`ReadingStatus::parse`] this one does default: an unrecognised
    /// *sort* changes only the order of a list, so the worst outcome is a list
    /// in the wrong order, where an unrecognised status would be a wrong claim
    /// about the reader's relationship with a work.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "title" => Self::Title,
            "updated" => Self::Updated,
            "words" => Self::Words,
            "position" => Self::Position,
            _ => Self::Recent,
        }
    }
}

impl std::fmt::Display for LibrarySort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A library query: what the reader is currently looking at.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LibraryQuery {
    /// Shelf names to include. Empty means every shelf and every unshelved item.
    pub shelves: Vec<String>,
    /// Private tag names to include. Empty means no tag filter.
    pub tags: Vec<String>,
    /// Reading statuses to include. Empty means every status.
    pub statuses: Vec<ReadingStatus>,
    /// Source key (`ao3`, `ffnet`, …), if the reader filtered by source.
    pub source: Option<String>,
    /// Only items the source changed at or after this time, as RFC 3339 text.
    ///
    /// Text rather than `OffsetDateTime` for two reasons: it is what arrives in
    /// a query string, and it is what the columns hold, so the value is never
    /// reformatted on its way to the database. [`validate_query`] is what
    /// insists it is a real instant.
    pub updated_since: Option<String>,
    /// Ordering.
    pub sort: LibrarySort,
}

impl LibraryQuery {
    /// Whether this query names anything that only its owner can see.
    ///
    /// A shelf, a private tag and a reading status are all per-account facts. A
    /// source filter and an `updated_since` bound are not: they describe the
    /// work rather than the reader's relationship with it.
    #[must_use]
    pub fn uses_private_filters(&self) -> bool {
        !self.shelves.is_empty() || !self.tags.is_empty() || !self.statuses.is_empty()
    }

    /// The private filter that would leak, named for the error message.
    ///
    /// Returns the first of them in a stable order so the message does not
    /// depend on which field a caller happened to check first.
    #[must_use]
    pub fn first_private_filter(&self) -> Option<&'static str> {
        if !self.shelves.is_empty() {
            Some("shelves")
        } else if !self.tags.is_empty() {
            Some("tags")
        } else if !self.statuses.is_empty() {
            Some("statuses")
        } else {
            None
        }
    }
}

/// Which audience a saved view is stored for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewScope {
    /// Usable only by its owner, over their own library.
    #[default]
    Library,
    /// Shareable, and therefore restricted to filters that describe works.
    Public,
}

impl ViewScope {
    /// The stored form.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::Public => "public",
        }
    }

    /// Parse the stored form.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "library" => Some(Self::Library),
            "public" => Some(Self::Public),
            _ => None,
        }
    }
}

/// Why a query was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "reason")]
pub enum QueryError {
    /// A public view named a filter that only describes one account's library.
    PrivateFilterInPublicView {
        /// Which filter, for the message.
        filter: String,
    },
    /// The name was empty or only whitespace.
    EmptyName,
    /// A filter value was blank or only whitespace.
    BlankFilter {
        /// Which filter.
        filter: String,
    },
    /// `updated_since` was not an RFC 3339 instant.
    InvalidTimestamp {
        /// What was supplied.
        value: String,
    },
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PrivateFilterInPublicView { filter } => write!(
                f,
                "a public view cannot filter by {filter}: that describes your own library, \
                 and sharing the view would share it"
            ),
            Self::EmptyName => f.write_str("a view needs a name"),
            Self::BlankFilter { filter } => {
                write!(f, "the {filter} filter is blank")
            }
            Self::InvalidTimestamp { value } => write!(
                f,
                "\"{value}\" is not a timestamp; expected an RFC 3339 instant such as \
                 2026-09-11T12:00:00Z"
            ),
        }
    }
}

impl std::error::Error for QueryError {}

/// Check a query before it is stored as a view.
///
/// The scope decides the rule. For [`ViewScope::Library`] the only check is that
/// the filters are not blank — a private view may filter on anything its owner
/// has. For [`ViewScope::Public`] a filter that describes the reader rather than
/// the work is refused, naming it.
///
/// # Errors
///
/// Returns [`QueryError::PrivateFilterInPublicView`] when a public view carries
/// a shelf, tag or status filter, and [`QueryError::BlankFilter`] when any
/// filter value is empty or whitespace.
pub fn validate_query(query: &LibraryQuery, scope: ViewScope) -> Result<(), QueryError> {
    for (filter, values) in [("shelves", &query.shelves), ("tags", &query.tags)] {
        if values.iter().any(|v| v.trim().is_empty()) {
            return Err(QueryError::BlankFilter {
                filter: filter.to_string(),
            });
        }
    }

    // A freshness bound that will not parse is a mistake in either scope, and
    // checking here means the repository and the SQL never see one.
    if let Some(value) = &query.updated_since {
        let parsed =
            time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339);
        if parsed.is_err() {
            return Err(QueryError::InvalidTimestamp {
                value: value.clone(),
            });
        }
    }

    if scope == ViewScope::Public {
        if let Some(filter) = query.first_private_filter() {
            return Err(QueryError::PrivateFilterInPublicView {
                filter: filter.to_string(),
            });
        }
    }

    Ok(())
}

/// One item a batch operation could not act on.
///
/// The code is a machine-readable reason rather than a sentence, because the
/// client decides how to present it and because the same failures must be
/// countable in aggregate ("3 removed, 2 already gone").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchFailure {
    /// The item the operation could not act on.
    pub id: String,
    /// Why, in a form a program can branch on.
    pub code: String,
    /// A sentence for the reader, when the code needs one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl BatchFailure {
    /// A failure with a code and no message.
    #[must_use]
    pub fn new(id: impl Into<String>, code: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            code: code.into(),
            message: None,
        }
    }

    /// A failure with a message for the reader.
    #[must_use]
    pub fn with_message(
        id: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            code: code.into(),
            message: Some(message.into()),
        }
    }

    /// The item was not there — already deleted, or never the caller's.
    ///
    /// Deliberately one code for both. A batch that distinguished "gone" from
    /// "not yours" would tell a caller which identifiers exist, and library
    /// items belong to one account.
    #[must_use]
    pub fn gone(id: impl Into<String>) -> Self {
        Self::new(id, "NOT_FOUND")
    }
}

/// The outcome of a batch operation: what worked, and what did not.
///
/// Spec §14.3 fixes the serialized shape (`succeeded` as a list of ids, `failed`
/// as a list of `{id, code}`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchOutcome {
    /// Identifiers the operation acted on.
    pub succeeded: Vec<String>,
    /// Identifiers it did not, each with a reason.
    pub failed: Vec<BatchFailure>,
}

impl BatchOutcome {
    /// An outcome decided in advance, which is what a batch over a checked set
    /// produces: the caller has already worked out which ids are reachable, so
    /// the result is assembled rather than discovered.
    #[must_use]
    pub fn from_parts(succeeded: Vec<String>, failed: Vec<BatchFailure>) -> Self {
        Self { succeeded, failed }
    }

    /// How many items were asked about.
    #[must_use]
    pub fn attempted(&self) -> usize {
        self.succeeded.len() + self.failed.len()
    }

    /// Whether anything failed.
    #[must_use]
    pub fn is_complete_success(&self) -> bool {
        self.failed.is_empty()
    }

    /// A sentence naming both counts, because either one alone is misleading.
    ///
    /// "3 removed" over a request of five hides two failures; "2 failed" hides
    /// that the other three worked.
    ///
    /// Takes the verb twice — as the past participle for what happened
    /// (`removed`) and as the infinitive for what did not (`remove`) — because
    /// one form cannot produce a sentence in both positions. The first version
    /// took only the participle and rendered "2 removed, 1 could not be", which
    /// stops mid-clause; the total is named as well, so a reader is not left
    /// counting the list to find out how many were asked for.
    #[must_use]
    pub fn summary(&self, verb: &str, infinitive: &str) -> String {
        let attempted = self.attempted();
        match (self.succeeded.len(), self.failed.len()) {
            (0, 0) => "nothing to do".to_string(),
            (s, 0) => format!("{s} {verb}"),
            (0, f) => format!("none {verb}; {f} could not be {infinitive}"),
            (s, f) => format!("{s} of {attempted} {verb}; {f} could not be {infinitive}"),
        }
    }
}

/// The kind of thing a bookmark, private tag or reading status is attached to.
///
/// Open-ended on purpose: the same tables serve works and, later, other
/// subjects, and a closed enum here would mean a migration to bookmark
/// something new. The value is stored as written.
pub const SUBJECT_WORK: &str = "work";
/// An imported copy in the reader's library.
pub const SUBJECT_LIBRARY_ITEM: &str = "library_item";

/// Whether a subject type is one this module knows how to resolve.
#[must_use]
pub fn is_known_subject(subject_type: &str) -> bool {
    matches!(subject_type, SUBJECT_WORK | SUBJECT_LIBRARY_ITEM)
}

/// The longest a private tag may be, in characters.
///
/// Long enough for a phrase a reader would actually use ("read on the train"),
/// short enough that a tag is a label rather than a note. A note has its own
/// field on a bookmark.
pub const MAX_TAG_CHARS: usize = 64;

/// Clean up a tag, or refuse it.
///
/// Returns `None` when the tag cannot be one, which the caller reports as a
/// validation failure. The rules are deliberately small and total: trim the
/// ends, collapse internal runs of whitespace to one space, refuse anything
/// empty, over [`MAX_TAG_CHARS`], or carrying a control character.
///
/// Collapsing whitespace rather than refusing it means `"  read  later "` and
/// `"read later"` are the same tag, which is what a reader means — and it is
/// what stops the same tag appearing twice in their own tag list. Case is *not*
/// folded: a reader who writes `WIP` and `wip` has two tags, and quietly merging
/// them would be this program deciding what they meant.
#[must_use]
pub fn normalise_tag(raw: &str) -> Option<String> {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() || collapsed.chars().count() > MAX_TAG_CHARS {
        return None;
    }
    if collapsed.chars().any(char::is_control) {
        return None;
    }
    Some(collapsed)
}

/// How long an update-check record is kept, in days (spec §16's retention).
pub const UPDATE_CHECK_RETENTION_DAYS: i64 = 90;

/// The cutoff before which an update check is expired.
#[must_use]
pub fn update_check_retention_cutoff(now: time::OffsetDateTime) -> time::OffsetDateTime {
    now - time::Duration::days(UPDATE_CHECK_RETENTION_DAYS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn a_reading_status_round_trips_through_its_stored_form() {
        for status in ReadingStatus::all() {
            assert_eq!(
                ReadingStatus::parse(status.as_str()),
                Some(*status),
                "{status} did not survive a round trip"
            );
        }
    }

    #[test]
    fn an_unknown_reading_status_is_not_guessed_at() {
        // The row may have been written by a newer build. Reporting it as
        // "want to read" would be a claim about the reader that nobody made.
        assert_eq!(ReadingStatus::parse("re-reading"), None);
        assert_eq!(ReadingStatus::parse(""), None);
    }

    #[test]
    fn an_unknown_sort_falls_back_rather_than_failing() {
        // Ordering a list wrongly is a smaller wrong than refusing to render it.
        assert_eq!(LibrarySort::parse("popularity"), LibrarySort::Recent);
    }

    #[test]
    fn a_status_that_means_started_does_not_include_wanting_to_read() {
        assert!(!ReadingStatus::WantToRead.is_started());
        for status in [
            ReadingStatus::Reading,
            ReadingStatus::OnHold,
            ReadingStatus::Dropped,
            ReadingStatus::Finished,
        ] {
            assert!(status.is_started(), "{status} should count as started");
        }
        assert!(ReadingStatus::Finished.is_finished());
        assert!(!ReadingStatus::Dropped.is_finished());
    }

    #[test]
    fn a_private_view_may_filter_on_anything_its_owner_has() {
        let query = LibraryQuery {
            shelves: vec!["Comfort reads".into()],
            tags: vec!["quiet".into()],
            statuses: vec![ReadingStatus::Reading],
            ..LibraryQuery::default()
        };
        assert!(validate_query(&query, ViewScope::Library).is_ok());
        assert!(query.uses_private_filters());
    }

    #[test]
    fn a_public_view_cannot_filter_by_a_shelf_a_tag_or_a_status() {
        // The acceptance criterion: "Shared views cannot expose private filters".
        for (query, expected) in [
            (
                LibraryQuery {
                    shelves: vec!["Comfort reads".into()],
                    ..LibraryQuery::default()
                },
                "shelves",
            ),
            (
                LibraryQuery {
                    tags: vec!["quiet".into()],
                    ..LibraryQuery::default()
                },
                "tags",
            ),
            (
                LibraryQuery {
                    statuses: vec![ReadingStatus::Reading],
                    ..LibraryQuery::default()
                },
                "statuses",
            ),
        ] {
            let refused = validate_query(&query, ViewScope::Public);
            match refused {
                Err(QueryError::PrivateFilterInPublicView { filter }) => {
                    assert_eq!(filter, expected);
                }
                other => panic!("expected the {expected} filter to be refused, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_public_view_may_filter_by_the_work_itself() {
        // A source and a freshness bound describe the work, not the reader.
        let query = LibraryQuery {
            source: Some("ao3".into()),
            updated_since: Some("2026-09-01T00:00:00Z".to_string()),
            sort: LibrarySort::Updated,
            ..LibraryQuery::default()
        };
        assert!(!query.uses_private_filters());
        assert!(validate_query(&query, ViewScope::Public).is_ok());
    }

    #[test]
    fn a_freshness_bound_that_is_not_a_timestamp_is_refused() {
        let query = LibraryQuery {
            updated_since: Some("last tuesday".to_string()),
            ..LibraryQuery::default()
        };
        assert_eq!(
            validate_query(&query, ViewScope::Library),
            Err(QueryError::InvalidTimestamp {
                value: "last tuesday".to_string()
            })
        );
    }

    #[test]
    fn a_blank_filter_is_refused_even_in_a_private_view() {
        // A filter that names nothing is a mistake wherever it is stored.
        let query = LibraryQuery {
            tags: vec!["  ".into()],
            ..LibraryQuery::default()
        };
        assert_eq!(
            validate_query(&query, ViewScope::Library),
            Err(QueryError::BlankFilter {
                filter: "tags".into()
            })
        );
    }

    #[test]
    fn the_private_filter_is_named_in_a_stable_order() {
        // Whichever field a caller checks first, the message says the same.
        let query = LibraryQuery {
            shelves: vec!["a".into()],
            tags: vec!["b".into()],
            statuses: vec![ReadingStatus::Reading],
            ..LibraryQuery::default()
        };
        assert_eq!(query.first_private_filter(), Some("shelves"));
    }

    #[test]
    fn a_batch_reports_both_counts_rather_than_one_boolean() {
        let outcome = BatchOutcome::from_parts(
            vec!["a".into(), "b".into(), "c".into()],
            vec![BatchFailure::gone("d"), BatchFailure::gone("e")],
        );
        assert_eq!(outcome.attempted(), 5);
        assert!(!outcome.is_complete_success());
        assert_eq!(
            outcome.summary("removed", "removed"),
            "3 of 5 removed; 2 could not be removed"
        );
    }

    #[test]
    fn a_batch_that_wholly_succeeded_says_so_without_a_failure_clause() {
        let outcome = BatchOutcome::from_parts(vec!["a".into(), "b".into()], vec![]);
        assert!(outcome.is_complete_success());
        assert_eq!(outcome.summary("removed", "removed"), "2 removed");
    }

    #[test]
    fn a_batch_that_wholly_failed_does_not_claim_a_success() {
        let outcome = BatchOutcome::from_parts(vec![], vec![BatchFailure::gone("a")]);
        assert_eq!(
            outcome.summary("removed", "removed"),
            "none removed; 1 could not be removed"
        );
    }

    #[test]
    fn an_empty_batch_is_not_a_success_or_a_failure() {
        let outcome = BatchOutcome::default();
        assert_eq!(outcome.summary("removed", "removed"), "nothing to do");
    }

    #[test]
    fn a_gone_item_is_reported_the_same_whether_it_is_missing_or_not_yours() {
        // One code for both: distinguishing them would confirm which ids exist.
        let failure = BatchFailure::gone("abc");
        assert_eq!(failure.code, "NOT_FOUND");
        assert!(failure.message.is_none());
    }

    #[test]
    fn the_update_check_cutoff_is_ninety_days_back() {
        let now = datetime!(2026-09-11 12:00 UTC);
        let cutoff = update_check_retention_cutoff(now);
        assert_eq!(cutoff, datetime!(2026-06-13 12:00 UTC));
    }

    #[test]
    fn only_the_two_known_subject_types_are_accepted() {
        assert!(is_known_subject(SUBJECT_WORK));
        assert!(is_known_subject(SUBJECT_LIBRARY_ITEM));
        assert!(!is_known_subject("series"));
        assert!(!is_known_subject(""));
    }

    #[test]
    fn a_view_scope_round_trips() {
        for scope in [ViewScope::Library, ViewScope::Public] {
            assert_eq!(ViewScope::parse(scope.as_str()), Some(scope));
        }
        assert_eq!(ViewScope::parse("shared"), None);
    }
}
