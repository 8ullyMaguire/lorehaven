//! Query language: parser, AST, and SQL rendering.
//!
//! Spec §15.3–15.6. Pure functions — no I/O.

/// A field in a fielded query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QueryField {
    Title,
    Author,
    Fandom,
    Character,
    Relationship,
    Tag,
    Mood,
    Summary,
    Body,
    Language,
    Status,
    Format,
    Edition,
    Rating,
    Completion,
    Published,
    Updated,
    MinQuality,
    Quality,
    // --- Cross-entity fields -------------------------------------------------
    //
    // The operators are shared by every surface; only the field names differ.
    // A reader who learns `words:>10000` for works can immediately write
    // `replies:>50` for forum posts.
    //
    /// Total words, works only.
    Words,
    /// Kudos count, works only.
    Kudos,
    /// Forum reply count, forums only.
    Replies,
    /// Forum category, forums only.
    Category,
    /// Whether a forum entry is a thread or a post, forums only.
    Kind,
    /// Directory ranking score, directories only.
    Rank,
    /// Directory entity type (character, fandom, ship, ...), directories only.
    Type_,
    /// Directory submitter handle, directories only.
    Submitter,
    /// Whether a bookmark is a recommendation, bookmarks only.
    Rec,
    /// Free text of a bookmark note, bookmarks only.
    Note,
    /// A pseud handle, users only.
    User,
    /// Number of works a pseud has published, users only.
    Works,
    /// Fandoms a pseud has written in, users only.
    UserFandom,
    /// Account join date, users only.
    Joined,
    /// Bookmarked date, bookmarks only.
    Bookmarked,
    /// Last activity, forums only.
    Active,
    /// Whether a forum entry is pinned, forums only.
    Pinned,
    /// Whether a forum entry is locked, forums only.
    Locked,
}

impl QueryField {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Author => "author",
            Self::Fandom => "fandom",
            Self::Character => "character",
            Self::Relationship => "relationship",
            Self::Tag => "tag",
            Self::Mood => "mood",
            Self::Summary => "summary",
            Self::Body => "body",
            Self::Language => "language",
            Self::Status => "status",
            Self::Format => "format",
            Self::Edition => "edition",
            Self::Rating => "rating",
            Self::Completion => "completion",
            Self::Published => "published",
            Self::Updated => "updated",
            Self::MinQuality => "min_quality",
            Self::Quality => "quality",
            Self::Words => "words",
            Self::Kudos => "kudos",
            Self::Replies => "replies",
            Self::Category => "category",
            Self::Kind => "kind",
            Self::Rank => "rank",
            Self::Type_ => "type",
            Self::Submitter => "submitter",
            Self::Rec => "rec",
            Self::Note => "note",
            Self::User => "user",
            Self::Works => "works",
            Self::UserFandom => "user_fandom",
            Self::Joined => "joined",
            Self::Bookmarked => "bookmarked",
            Self::Active => "active",
            Self::Pinned => "pinned",
            Self::Locked => "locked",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "title" => Self::Title,
            "author" => Self::Author,
            "fandom" => Self::Fandom,
            "character" => Self::Character,
            "relationship" => Self::Relationship,
            "tag" => Self::Tag,
            "mood" => Self::Mood,
            "summary" => Self::Summary,
            "body" => Self::Body,
            "language" => Self::Language,
            "status" => Self::Status,
            "format" => Self::Format,
            "edition" => Self::Edition,
            "rating" => Self::Rating,
            "completion" => Self::Completion,
            "published" => Self::Published,
            "updated" => Self::Updated,
            "min_quality" => Self::MinQuality,
            "quality" => Self::Quality,
            "words" => Self::Words,
            "kudos" => Self::Kudos,
            "replies" => Self::Replies,
            "category" => Self::Category,
            "kind" => Self::Kind,
            "rank" => Self::Rank,
            "type" => Self::Type_,
            "submitter" => Self::Submitter,
            "rec" => Self::Rec,
            "note" => Self::Note,
            "user" => Self::User,
            "works" => Self::Works,
            "user_fandom" | "fandoms" => Self::UserFandom,
            "joined" => Self::Joined,
            "bookmarked" => Self::Bookmarked,
            "active" => Self::Active,
            "pinned" => Self::Pinned,
            "locked" => Self::Locked,
            _ => return None,
        })
    }

    /// Which entity surface this field belongs to.
    ///
    /// The value is what a surface checks before accepting a query, so a field
    /// that names another entity's data is an error rather than a silent zero.
    pub fn entity(self) -> EntityKind {
        match self {
            // Works-only.
            Self::Title
            | Self::Author
            | Self::Fandom
            | Self::Character
            | Self::Relationship
            | Self::Tag
            | Self::Mood
            | Self::Summary
            | Self::Body
            | Self::Language
            | Self::Status
            | Self::Format
            | Self::Edition
            | Self::Rating
            | Self::Completion
            | Self::Published
            | Self::Updated
            | Self::MinQuality
            | Self::Quality
            | Self::Words
            | Self::Kudos => EntityKind::Works,
            // Forum-only.
            Self::Replies
            | Self::Category
            | Self::Kind
            | Self::Active
            | Self::Pinned
            | Self::Locked => EntityKind::Forum,
            // Directory-only.
            Self::Rank | Self::Type_ | Self::Submitter => EntityKind::Directory,
            // Bookmark-only. `Note` is bookmark-scoped because a bookmark note is
            // personal to the reader; `Bookmarked` is the date it was saved.
            Self::Rec | Self::Note | Self::Bookmarked => EntityKind::Bookmark,
            // User-only.
            Self::User | Self::Works | Self::UserFandom | Self::Joined => EntityKind::User,
        }
    }

    /// Whether this field names a taxonomy term, and so has a closure to expand
    /// through.
    ///
    /// §15.4.1.3. `+children` walks `parent` edges in `taxonomy_closure` (migration
    /// 0104), which only taxonomy nodes have. Allowing it on `words` or `title`
    /// would put the suffix in a value where it matches nothing, so the check is
    /// here instead: an allowlist of the fields that resolve to a node, which makes
    /// adding an expandable field a deliberate act.
    pub fn is_taxonomy(self) -> bool {
        matches!(
            self,
            QueryField::Fandom
                | QueryField::Character
                | QueryField::Relationship
                | QueryField::Tag
                | QueryField::Mood
        )
    }

    /// Whether this field carries an ordering, so `>`, `<` and `..` mean
    /// something on it.
    ///
    /// An allowlist rather than a denylist of the text fields: the failure mode
    /// of a denylist is that a new field is not-orderable by omission, and the
    /// reader gets "no ordering" for a field that has one. The allowlist makes
    /// adding a comparable field a deliberate act.
    ///
    /// It does not call `comparison_value`, which asks this in turn -- a mutual
    /// call recurses until the stack gives out.
    pub fn is_comparable(self) -> bool {
        matches!(
            self,
            QueryField::Words
                | QueryField::Kudos
                | QueryField::Replies
                | QueryField::Rank
                | QueryField::Works
                | QueryField::MinQuality
                | QueryField::Quality
                | QueryField::Active
                | QueryField::Published
                | QueryField::Updated
                | QueryField::Joined
                | QueryField::Bookmarked
        )
    }

    /// What kind of value this field's comparison operators take.
    ///
    /// The parser needs this to reject `words:>"many"` before building a node,
    /// and it is the reason the parser does *not* simply require an integer:
    /// `active:>2026-01-15` is an ordered comparison on a timestamp column, and
    /// a parser that insisted on an integer made it unwritable. The renderer
    /// re-checks against the actual column, because it is the only layer that
    /// knows what is behind the name.
    pub fn comparison_value(self) -> ValueKind {
        match self {
            Self::Active | Self::Published | Self::Updated | Self::Joined | Self::Bookmarked => {
                ValueKind::Date
            }
            _ if self.is_comparable() => ValueKind::Number,
            _ => ValueKind::Text,
        }
    }
}

/// The kind of value a field's comparison operator accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    /// An ordered comparison on a number: `words:>10000`.
    Number,
    /// An ordered comparison on a timestamp: `active:>2026-01-15`.
    Date,
    /// Not orderable. A comparison on one of these is a category error.
    Text,
}

/// The noun a field's value kind takes in an error message.
fn describe_kind(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::Number => "numeric",
        ValueKind::Date => "date",
        ValueKind::Text => "single-value",
    }
}

/// Whether a comparison or range value is usable for this kind of column.
fn valid_bound(kind: ValueKind, value: &str) -> bool {
    match kind {
        ValueKind::Number => value.parse::<i64>().is_ok(),
        ValueKind::Date => is_iso_dateish(value),
        ValueKind::Text => false,
    }
}

/// Whether the value is an ISO-8601 date or timestamp prefix.
///
/// Timestamps are stored as `TEXT` in `YYYY-MM-DD HH:MM:SS` on both backends,
/// so there is no date type to lean on and no reason to add a dependency to
/// this crate. ISO-8601 orders lexicographically -- `2026-01-15` sorts before
/// `2026-01-16` because the character that differs is the one that decides --
/// which is what makes the lexicographic comparison in `bounds_descend`
/// correct. It also means a bare `2026-01` compares correctly against a stored
/// `2026-01-15 ...`: the month prefix sorts before every day in it, so
/// `active:>=2026-01` includes the whole month, which is what a reader means.
///
/// Accepted shapes: `YYYY`, `YYYY-MM`, `YYYY-MM-DD`, and either of those
/// followed by `THH:MM:SS` or ` HH:MM:SS`. A full calendar check follows, so
/// `2026-13` is refused rather than quietly matching nothing.
fn is_iso_dateish(value: &str) -> bool {
    let (date, time) = match value.split_once(['T', ' ']) {
        Some((d, t)) => (d, Some(t)),
        None => (value, None),
    };

    let parts: Vec<&str> = date.split('-').collect();
    let (year, month, day) = match parts.as_slice() {
        [y] => (y, None, None),
        [y, m] => (y, Some(*m), None),
        [y, m, d] => (y, Some(*m), Some(*d)),
        _ => return false,
    };

    let Ok(year) = year.parse::<i32>() else {
        return false;
    };
    if !(1..=9999).contains(&year) {
        return false;
    }

    // A bare `YYYY` is a year, and a bare `YYYY-MM` is a month. Both are
    // useful: a year prefix sorts before every timestamp in it, so
    // `active:>=2026` means all of 2026 and `active:>=2026-01` all of January.
    let month_number = match month {
        Some(text) => {
            let Ok(month_number) = text.parse::<u32>() else {
                return false;
            };
            if !(1..=12).contains(&month_number) {
                return false;
            }
            month_number
        }
        // A year with a day, or a year with a time, is not a shape that exists.
        None => return day.is_none() && time.is_none(),
    };

    if let Some(text) = day {
        let Ok(day) = text.parse::<u32>() else {
            return false;
        };
        // Days-in-month, leap years included. A crate with no date dependency
        // still has to know February is not 31 days, or `active:2026-02-30`
        // would be accepted and then match nothing.
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let lengths = [
            31,
            if leap { 29 } else { 28 },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        if !(1..=lengths[month_number as usize - 1]).contains(&day) {
            return false;
        }
    }

    let Some(t) = time else {
        return true;
    };
    let Some((h, rest)) = t.split_once(':') else {
        return false;
    };
    let Some((m, s)) = rest.split_once(':') else {
        return false;
    };
    h.parse::<u32>().is_ok_and(|h| h < 24)
        && m.parse::<u32>().is_ok_and(|m| m < 60)
        && s.parse::<u32>().is_ok_and(|s| s < 60)
}

/// Whether a range's low bound comes after its high bound.
///
/// Numbers compare numerically and dates lexicographically, and mixing the two
/// is the bug: `"10000" > "5000"` is *false* as a string, because `1` sorts
/// before `5`, so a purely lexicographic check reports `10000..5000` as a
/// valid range and the query then matches nothing. Which comparison applies is
/// the caller's decision because only it knows the column's type.
///
/// Called after both bounds have been validated as the same kind, so there is
/// no mixed-kind case to answer here.
fn bounds_descend(kind: ValueKind, low: &str, high: &str) -> bool {
    match kind {
        ValueKind::Number => low.parse::<i64>().unwrap_or(0) > high.parse::<i64>().unwrap_or(0),
        // ISO-8601 orders lexicographically, and the year-month-day prefixes
        // sort before everything inside them -- which is why a bare `2026-01`
        // behaves as "January 2026" against a stored `2026-01-15 09:00:00`.
        ValueKind::Date | ValueKind::Text => low > high,
    }
}

/// The entity surface a field belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntityKind {
    Works,
    Forum,
    User,
    Bookmark,
    Directory,
}

impl EntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Works => "works",
            Self::Forum => "forum",
            Self::User => "user",
            Self::Bookmark => "bookmark",
            Self::Directory => "directory",
        }
    }
}

/// A comparison operator: the shared numeric/date vocabulary of the language.
///
/// `:` is equality and stays a separate AST node (`Fielded`) precisely so that
/// a surface can render `=` and `<`/`>` without re-inspecting the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompareOp {
    Gt,
    Gte,
    Lt,
    Lte,
}

impl CompareOp {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gt => ">",
            Self::Gte => ">=",
            Self::Lt => "<",
            Self::Lte => "<=",
        }
    }

    /// The SQL operator this renders to.
    pub fn sql(self) -> &'static str {
        match self {
            Self::Gt => ">",
            Self::Gte => ">=",
            Self::Lt => "<",
            Self::Lte => "<=",
        }
    }
}

/// The query AST.
#[derive(Debug, Clone, PartialEq)]
pub enum QueryAst {
    /// Free text search.
    Text(String),
    /// A quoted phrase.
    Phrase(String),
    /// A fielded search: `field:value`.
    Fielded(QueryField, String),
    /// A comparison: `field>value`, `field>=value`, `field<value`, `field<=value`.
    ///
    /// A separate node from `Fielded` so a renderer never has to look at the
    /// value to discover whether the operator was `=`, `>`, `<`, `>=` or `<=`.
    Comparison(QueryField, CompareOp, String),
    /// Logical AND of sub-queries.
    And(Vec<QueryAst>),
    /// Logical OR of sub-queries.
    Or(Vec<QueryAst>),
    /// Logical NOT.
    Not(Box<QueryAst>),
    /// A work has this character, with these properties.
    ///
    /// §15.3. The first of the two nodes that were specified in §15.3 and had no
    /// implementation until migration 0103 gave them a schema.
    ExistsCharacter(CharacterAssertion),
    /// A work has a relationship satisfying these bounds.
    ///
    /// §15.3. `Not(ExistsRelationship { .. }))` is journey 12's second half: "X
    /// present, no relationship involving X".
    ExistsRelationship(RelationshipAssertion),
    /// A fielded sub-predicate whose fields all bind to ONE row of ONE relation.
    ///
    /// §15.4.1.1, new in M46-05. `ship:(with:"A" with:"B" type:romantic)`.
    ///
    /// This node exists because §15.3's rule -- never let one character satisfy
    /// another character's attributes -- is not reachable from the text surface
    /// without it. `ship:"A/B" AND type:romantic` is two independent terms: the
    /// pairing may come from one relationship row and the type from another, so
    /// the pair matches works where A/B exists *somehow* and *something else* is
    /// romantic. That is wrong in a way that looks right, and it was measured
    /// rather than hypothesised: the uncorrelated compilation returns 1 on a
    /// fixture (Alice protagonist, Carol vampire) where the correlated form
    /// returns 0.
    ///
    /// Fields inside bind to the same row. Fields outside are ordinary terms and
    /// are not correlated with anything -- the same rule §15.3 states for the AST.
    Scoped(ScopedPredicate),
    /// At least `n` of the contained terms must hold.
    ///
    /// §15.4.1.2, new in M46-05. `min_match(2, tag:"A", tag:"B", tag:"C")`.
    ///
    /// A quantified conjunction, which §15.7's conjunctive-only filter list could
    /// not express. It counts SATISFIED TERMS -- not a score threshold, and not a
    /// disjunction -- which is why it is a distinct node rather than sugar over
    /// `Or`: `min_match(2, a, b)` and `a OR b` are different questions and must
    /// not compile to the same thing.
    MinMatch {
        /// How many of `terms` must hold. Always at least 1: `min_match(0, …)`
        /// is refused at parse time as meaningless rather than matching everything.
        needed: usize,
        terms: Vec<QueryAst>,
    },
    /// A taxonomy term widened to its descendants.
    ///
    /// §15.4.1.3, new in M46-05. `tag:"Fake Dating"+children` or `+children^2`.
    ///
    /// `depth` is the cap; `None` means the instance default. Expansion follows
    /// `parent` edges only -- `implies` requires the reader to ask for it
    /// explicitly, because a curator's bad edge would otherwise distort every
    /// query touching the affected tags.
    Expand {
        field: QueryField,
        term: String,
        depth: Option<u32>,
    },
}

/// A parenthesised fielded sub-predicate: `ship:(with:"A" with:"B" type:romantic)`.
///
/// §15.4.1.1, new in M46-05. `relation` names WHICH relation the fields bind to —
/// the row a renderer must find — and `fields` are the bounds on that one row.
///
/// `relation` is a `QueryField` rather than a bare string so an unknown relation
/// name is a parse error naming the known ones, which is §15.4.1.6's span-based
/// diagnostic requirement rather than a value that fails later at render time.
///
/// Field order is preserved because it is the reader's order and the pretty-printer
/// must round-trip; a set would print in an arbitrary order and `parse(print(ast))`
/// would not be stable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedPredicate {
    /// The relation whose single row these fields constrain.
    pub relation: QueryField,
    /// Bounds on that one row, in the order written.
    pub fields: Vec<ScopedField>,
}

/// One `key:value` bound inside a [`ScopedPredicate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedField {
    /// The bound's name, e.g. `with` or `type`.
    pub key: String,
    pub value: String,
}

/// §15.1: a character, bounded.
///
/// `prominence` and `attributes_*` are **bounds on one character**, not on the
/// work. That is the whole point of the type, and it is what §15.3's rule
/// ("never allow one character to satisfy another character's attributes")
/// protects: because the bounds live in one struct next to one `character_id`,
/// the compiler is structurally unable to scatter them across separate `EXISTS`
/// clauses.
///
/// The three `attributes_*` fields are distinct rather than one list with a mode
/// flag because the difference between "all of these" and "any of these" is the
/// difference between a conjunction and a disjunction, and a mode flag on a
/// `Vec` is a mode somebody will eventually forget to check.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CharacterAssertion {
    /// §15.3: `character_id`. The node id, not a display name.
    pub character_id: String,
    /// §15.1: `prominence = protagonist`. Empty means "any prominence".
    pub prominence: Vec<String>,
    /// §15.1: `roles = [mentor]`. Empty means "any role".
    pub roles: Vec<String>,
    /// §15.1: `attributes_all = [BAMF]`. Every listed attribute must be present.
    pub attributes_all: Vec<String>,
    /// Any one of these suffices.
    pub attributes_any: Vec<String>,
    /// Restrict to characters that are a point of view.
    ///
    /// An `Option<bool>` rather than a bool: `None` means "no opinion", which is
    /// different from `Some(false)` ("must NOT be POV") and different again from
    /// `Some(true)`. Collapsing the first two loses a real query.
    pub is_pov: Option<bool>,
}

impl CharacterAssertion {
    /// Whether this assertion bounds anything at all beyond "this character is here".
    ///
    /// A bare `exists_character` with no bounds is a legitimate query — it is
    /// `character:X` — but the compiler still emits it as a correlated `EXISTS`
    /// rather than folding it into `work_tags`, because `work_characters` and
    /// `work_tags` are different facts: a tag can name a character without
    /// recording that character as *present in the work*.
    pub fn is_bare(&self) -> bool {
        self.prominence.is_empty()
            && self.roles.is_empty()
            && self.attributes_all.is_empty()
            && self.attributes_any.is_empty()
            && self.is_pov.is_none()
    }
}

/// §15.2: a relationship, bounded.
///
/// `participant_any` is a list rather than a single id because §15.2 requires
/// supporting more than two participants, and "any of these participants" is the
/// natural query: *any romantic pairing involving X or Y*.
///
/// `excluded_participants` exists for the query §15.3 does not spell out but the
/// use case needs: "X with anyone **except** Y". It compiles to a correlated
/// `NOT EXISTS` **within the same relationship row**, which is the only correct
/// compilation — an `AND NOT EXISTS` at the top level would exclude every work
/// containing a X-with-someone-else pairing, not just X-with-Y.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RelationshipAssertion {
    /// Any of these participants makes the relationship count.
    pub participant_any: Vec<String>,
    /// §15.2: `Kind = romantic`. Empty means "any kind".
    pub kind_any: Vec<String>,
    /// §15.2: `Prominence = central`. Empty means "any prominence".
    pub prominence: Vec<String>,
    /// §15.2: `Dynamics = [enemies_to_lovers]`. Empty means "any".
    pub dynamics: Vec<String>,
    /// A relationship containing any of these does **not** count.
    pub excluded_participants: Vec<String>,
}

/// A parse error with the character offset of the mistake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryError {
    pub message: String,
    pub offset: usize,
}

impl QueryError {
    pub fn new(message: impl Into<String>, offset: usize) -> Self {
        Self {
            message: message.into(),
            offset,
        }
    }
}

/// Parse a query string into an AST.
///
/// Supports: quoted phrases, fielded search (`field:value`), `AND`/`OR`/`NOT`,
/// parentheses, leading `-` exclusions, comparisons, ranges, scoped
/// sub-predicates, `min_match` and expansion.
///
/// Every bound in [`QueryBudget`] is enforced here, and exceeding one is an error
/// naming the bound and the value. Search is already rate-limited
/// (`[rate_limits] search.burst`), which bounds how OFTEN a query arrives; a single
/// query with a deep `OR` tree and a wide `+children` expansion is one request doing
/// unbounded work, so the per-query cost is bounded too. Truncating instead would
/// return a wrong answer that looks like a successful one.
pub fn parse_query(input: &str) -> std::result::Result<QueryAst, QueryError> {
    parse_query_with(input, QueryBudget::default())
}

/// Parse with an explicit budget. Instance config supplies this (§15.4.1.5, §38.6).
pub fn parse_query_with(
    input: &str,
    budget: QueryBudget,
) -> std::result::Result<QueryAst, QueryError> {
    let mut parser = Parser::new(input);
    parser.budget = budget;
    let ast = parser.parse()?;
    budget.check(&ast, 0)?;
    Ok(ast)
}

/// Per-query cost bounds. §15.4.1.5, new in M46-05.
///
/// Defaults are the spec's. Every field is instance config under §38.6, and a
/// bound of 0 means "no limit" rather than "match nothing" — a cap of zero results
/// is never a useful configuration and treating it as one would silently break
/// every query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryBudget {
    /// Maximum AST nesting depth. Depth is what makes a compiled predicate
    /// exponentially wide, which is why it is capped separately from term count.
    pub max_depth: usize,
    /// Maximum terms in the whole query.
    pub max_terms: usize,
    /// Maximum nodes one `+children` expansion may reach.
    pub max_expansion: usize,
    /// Maximum terms inside one `min_match`.
    pub max_min_match_arity: usize,
}

impl Default for QueryBudget {
    fn default() -> Self {
        Self {
            max_depth: 8,
            max_terms: 24,
            max_expansion: 200,
            max_min_match_arity: 8,
        }
    }
}

impl QueryBudget {
    /// Walk the AST and refuse the first bound exceeded.
    ///
    /// Checked after parsing rather than during, so a single traversal decides
    /// every bound and the error names the outermost thing that is too large. A
    /// parse-time check inside the recursive descent would report the innermost
    /// violation, which for a deeply nested query is not what the reader needs to
    /// fix first.
    pub fn check(&self, ast: &QueryAst, depth: usize) -> std::result::Result<(), QueryError> {
        if self.max_depth != 0 && depth > self.max_depth {
            return Err(QueryError::new(
                format!(
                    "this query nests {depth} levels deep, past the limit of {}",
                    self.max_depth
                ),
                0,
            ));
        }
        let mut terms = 0usize;
        self.walk(ast, &mut terms)
    }

    /// Walks the AST enforcing `max_terms`.
    ///
    /// No `depth` parameter: `check` already refuses an over-deep AST at the entry
    /// point, so threading depth through the recursion only ever incremented a
    /// number nothing read. `max_terms` is the limit that actually bites while
    /// walking, because it also bounds the flat case -- a thousand terms joined by
    /// `and` is legal and shallow.
    fn walk(&self, ast: &QueryAst, terms: &mut usize) -> std::result::Result<(), QueryError> {
        *terms += 1;
        if self.max_terms != 0 && *terms > self.max_terms {
            return Err(QueryError::new(
                format!("this query has more than {} terms", self.max_terms),
                0,
            ));
        }
        match ast {
            QueryAst::And(v) | QueryAst::Or(v) => {
                for t in v {
                    self.walk(t, terms)?;
                }
            }
            QueryAst::Not(inner) => self.walk(inner, terms)?,
            // A scoped predicate is ONE term to the budget -- it compiles to a
            // single EXISTS however many bounds it carries. Pricing it by arity
            // would price it against a structure that does not exist at runtime.
            QueryAst::Scoped(_) => {}
            QueryAst::MinMatch {
                needed,
                terms: inner,
            } => {
                if self.max_min_match_arity != 0 && inner.len() > self.max_min_match_arity {
                    return Err(QueryError::new(
                        format!(
                            "min_match has {} terms, past the limit of {}",
                            inner.len(),
                            self.max_min_match_arity
                        ),
                        0,
                    ));
                }
                // `needed` is validated at parse time; re-checked here so a
                // hand-built AST cannot smuggle an unsatisfiable one past the
                // parser. A `MinMatch` that can never hold is the same class of
                // silent-nothing as a backwards range.
                if *needed == 0 || *needed > inner.len() {
                    return Err(QueryError::new(
                        format!(
                            "min_match needs {needed} of {} terms, which can never be satisfied",
                            inner.len()
                        ),
                        0,
                    ));
                }
                for t in inner {
                    self.walk(t, terms)?;
                }
            }
            QueryAst::Expand { depth: cap, .. } => {
                // An explicit `^n` larger than the instance's cap is refused here
                // rather than clamped, so the reader learns their expansion will
                // not run as written.
                if let Some(n) = cap {
                    if self.max_expansion != 0 && *n as usize > self.max_expansion {
                        return Err(QueryError::new(
                            format!(
                                "expansion depth {n} is past the limit of {}",
                                self.max_expansion
                            ),
                            0,
                        ));
                    }
                }
            }
            QueryAst::Text(_)
            | QueryAst::Phrase(_)
            | QueryAst::Fielded(..)
            | QueryAst::Comparison(..)
            | QueryAst::ExistsCharacter(..)
            | QueryAst::ExistsRelationship(..) => {}
        }
        Ok(())
    }
}

struct Parser<'a> {
    input: &'a str,
    pos: usize,
    /// Carried so `parse_query_with` can install an instance budget. The bounds are
    /// enforced by [`QueryBudget::check`] after parsing; the parser only needs the
    /// value for the two checks that must happen mid-parse to produce a good
    /// message (`min_match` arity, expansion depth).
    budget: QueryBudget,
    /// Nesting depth reached so far, counted during the descent.
    ///
    /// The AST alone cannot measure this: `parse_primary` returns the inner node for
    /// a parenthesised group, so `((((a))))` and `a` produce the SAME tree and a
    /// post-parse walk sees depth 1 either way. A reader can type arbitrarily deep
    /// nesting, and it is exactly the nesting that makes the compiled predicate
    /// exponentially wide -- so the depth is counted as it is entered.
    depth: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            pos: 0,
            budget: QueryBudget::default(),
            depth: 0,
        }
    }

    fn parse(&mut self) -> std::result::Result<QueryAst, QueryError> {
        let ast = self.parse_or()?;
        self.skip_ws();
        if self.pos < self.input.len() {
            return Err(QueryError::new(
                format!("unexpected character at offset {}", self.pos),
                self.pos,
            ));
        }
        Ok(ast)
    }

    fn parse_or(&mut self) -> std::result::Result<QueryAst, QueryError> {
        let mut left = self.parse_and()?;
        self.skip_ws();
        while self.peek_keyword("OR") {
            self.advance_keyword("OR");
            self.skip_ws();
            let right = self.parse_and()?;
            left = QueryAst::Or(vec![left, right]);
            self.skip_ws();
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> std::result::Result<QueryAst, QueryError> {
        let mut terms = vec![self.parse_not()?];
        self.skip_ws();
        while self.peek_keyword("AND") {
            self.advance_keyword("AND");
            self.skip_ws();
            terms.push(self.parse_not()?);
            self.skip_ws();
        }
        // Implicit conjunction: two terms with no operator.
        //
        // A `NOT` between two terms is a *trailing* negation -- `a NOT b` is
        // `a AND NOT b`, the form every search box advertises. This loop used
        // to break on `NOT` and let `parse()` demand EOF, so the query was
        // rejected outright. Consume the keyword and let `parse_not` build the
        // negation, which is the same path a leading `NOT` takes.
        loop {
            self.skip_ws();
            if self.pos >= self.input.len() {
                break;
            }
            if self.peek_keyword("OR") || self.peek_keyword("AND") {
                break;
            }
            if self.input[self.pos..].starts_with(')') {
                break;
            }
            // A failure here is the reader's mistake, not a reason to stop
            // quietly. `winter words:a lot..5000` used to break out of the loop
            // and return `Text("winter")`, so the broken term vanished and the
            // search answered with something the reader never asked for --
            // the same failure as ignoring a field from another surface, and
            // worse, because the query looked like it parsed.
            let term = self.parse_not()?;
            if term == QueryAst::Text(String::new()) {
                break;
            }
            // A term that expands to an `And` -- which is what a `..` range
            // does -- is spliced into the enclosing conjunction rather than
            // nested. Both render identically, but a nested `And` would leave
            // every consumer of this list needing to know both shapes exist,
            // and `NOT` over a spliced range needs re-grouping to stay correct.
            if let QueryAst::And(range_parts) = term {
                terms.extend(range_parts);
            } else {
                terms.push(term);
            }
            self.skip_ws();
        }
        if terms.len() == 1 {
            Ok(terms.into_iter().next().unwrap())
        } else {
            Ok(QueryAst::And(terms))
        }
    }

    fn parse_not(&mut self) -> std::result::Result<QueryAst, QueryError> {
        self.skip_ws();
        if self.peek_keyword("NOT") {
            self.advance_keyword("NOT");
            self.skip_ws();
            let inner = self.parse_not()?;
            return Ok(QueryAst::Not(Box::new(inner)));
        }
        if self.pos < self.input.len() && self.input[self.pos..].starts_with('-') {
            self.pos += 1;
            self.skip_ws();
            let inner = self.parse_primary()?;
            return Ok(QueryAst::Not(Box::new(inner)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> std::result::Result<QueryAst, QueryError> {
        self.skip_ws();
        if self.pos >= self.input.len() {
            return Ok(QueryAst::Text(String::new()));
        }

        // `min_match(n, term, term, ...)` is checked BEFORE the bare-group case,
        // because both start with a word and `min_match` is a function-shaped term,
        // not a group. A bare `(` after the identifier check below would otherwise
        // never be reached for this input.
        if self.peek_word("min_match") {
            return self.parse_min_match();
        }

        if self.input[self.pos..].starts_with('(') {
            self.depth += 1;
            if self.budget.max_depth != 0 && self.depth > self.budget.max_depth {
                return Err(QueryError::new(
                    format!(
                        "this query nests {} levels deep, past the limit of {}",
                        self.depth, self.budget.max_depth
                    ),
                    self.pos,
                ));
            }
            self.pos += 1;
            let inner = self.parse_or()?;
            self.skip_ws();
            if !self.input[self.pos..].starts_with(')') {
                return Err(QueryError::new("expected ')'".to_owned(), self.pos));
            }
            self.pos += 1;
            self.depth -= 1;
            return Ok(inner);
        }

        if self.input[self.pos..].starts_with('"') {
            return self.parse_phrase();
        }

        self.parse_term()
    }

    fn parse_phrase(&mut self) -> std::result::Result<QueryAst, QueryError> {
        let start = self.pos;
        self.pos += 1; // skip opening quote
        while self.pos < self.input.len() && !self.input[self.pos..].starts_with('"') {
            self.pos += self.input[self.pos..].chars().next().unwrap().len_utf8();
        }
        if self.pos >= self.input.len() {
            return Err(QueryError::new("unterminated phrase".to_owned(), start));
        }
        let phrase = self.input[start + 1..self.pos].to_owned();
        self.pos += 1; // skip closing quote
        Ok(QueryAst::Phrase(phrase))
    }

    fn parse_term(&mut self) -> std::result::Result<QueryAst, QueryError> {
        let start = self.pos;
        while self.pos < self.input.len() {
            let c = self.input[self.pos..].chars().next().unwrap();
            if c.is_whitespace() || c == '(' || c == ')' || c == '"' {
                break;
            }
            self.pos += c.len_utf8();
        }
        let term = &self.input[start..self.pos];
        if term.is_empty() {
            return Err(QueryError::new("expected a term".to_owned(), start));
        }

        // Check for fielded search: field:value, or a comparison field<op>value.
        //
        // The operator is read from *after* the colon, and the two-character
        // forms are tried first: `kudos:>=100` must not parse as `>` with an
        // `=` glued to the value. Only `:`-introduced values may compare, so a
        // bare `>100` stays free text.
        if let Some(colon) = term.find(':') {
            let field = &term[..colon];
            if let Some(f) = QueryField::parse(field) {
                let after_colon = &term[colon + 1..];

                // A scoped sub-predicate: `ship:(with:"A" type:romantic)`.
                //
                // Checked before the comparison operators because `after_colon` is
                // empty here — the `(` is the next character in the INPUT, since the
                // term scan stops at `(`. Reading it as an operator would produce
                // "expected a field value" for a perfectly well-formed correlated
                // predicate, which is the category error §15.4.1.6's diagnostics
                // exist to avoid.
                if after_colon.is_empty() && self.input[self.pos..].starts_with('(') {
                    return self.parse_scoped(f);
                }

                // A comparison operator, longest match first.
                let (op, value) = if let Some(v) = after_colon.strip_prefix(">=") {
                    (Some(CompareOp::Gte), v)
                } else if let Some(v) = after_colon.strip_prefix("<=") {
                    (Some(CompareOp::Lte), v)
                } else if let Some(v) = after_colon.strip_prefix('>') {
                    (Some(CompareOp::Gt), v)
                } else if let Some(v) = after_colon.strip_prefix('<') {
                    (Some(CompareOp::Lt), v)
                } else {
                    (None, after_colon)
                };

                if let Some(op) = op {
                    // The quoted check looks at the *input*, not at `value`.
                    // The term scan stopped at the opening quote, so `value` is
                    // already empty here and the `"` is the next character in
                    // the input -- testing `value` for emptiness first would
                    // report "needs a value" for what is really a category
                    // error, and send the reader looking for a missing number
                    // they did type.
                    let quoted_value = value.is_empty() && self.input[self.pos..].starts_with('"');
                    if quoted_value || value.starts_with('"') {
                        // `words:>"many"` is a category error, not a zero result.
                        return Err(QueryError::new(
                            format!(
                                "{} {} takes a {} value, not a quoted phrase",
                                f.as_str(),
                                op.as_str(),
                                describe_kind(f.comparison_value())
                            ),
                            self.pos,
                        ));
                    }
                    if value.is_empty() {
                        return Err(QueryError::new(
                            format!("{} {} needs a value", f.as_str(), op.as_str()),
                            self.pos,
                        ));
                    }
                    // The value is checked against the *field's* kind, not
                    // against "is it an integer". `active:>2026-01-15` is an
                    // ordered comparison on a timestamp column, and a parser
                    // that demanded an integer made it unwritable. The renderer
                    // re-checks, because it is the only layer that knows what
                    // is behind the field name.
                    match f.comparison_value() {
                        ValueKind::Number if value.parse::<i64>().is_err() => {
                            return Err(QueryError::new(
                                format!(
                                    "{} {} takes an integer, got {:?}",
                                    f.as_str(),
                                    op.as_str(),
                                    value
                                ),
                                self.pos,
                            ));
                        }
                        ValueKind::Text => {
                            return Err(QueryError::new(
                                format!(
                                    "{} has no ordering, so {} cannot be compared -- \
                                     it is a {} field",
                                    f.as_str(),
                                    op.as_str(),
                                    f.entity().as_str()
                                ),
                                self.pos,
                            ));
                        }
                        ValueKind::Number | ValueKind::Date => {}
                    }
                    return Ok(QueryAst::Comparison(f, op, value.to_owned()));
                }

                // Plain equality.
                if value.is_empty() {
                    if self.input[self.pos..].starts_with('"') {
                        let QueryAst::Phrase(value) = self.parse_phrase()? else {
                            unreachable!()
                        };
                        // NOT an early return: a quoted value can carry a range or an
                        // expansion suffix, and returning here made `tag:"X"+children`
                        // parse as `tag:"X"` followed by the free text `+children`.
                        // The reader would get a silently narrower query that looks
                        // like it worked. Both suffixes are read from the input, so
                        // they work identically for quoted and unquoted values.
                        if let Some(node) = self.try_range(f, &value, start)? {
                            return Ok(node);
                        }
                        if let Some(node) = self.take_expansion_suffix(f)? {
                            return Ok(node);
                        }
                        return Ok(QueryAst::Fielded(f, value));
                    }
                    return Err(QueryError::new("expected a field value", self.pos));
                }

                // A range: `field:low..high`, either bound optional.
                //
                // Checked before the equality return so a range never becomes a
                // `Fielded` carrying the literal text "10000..50000" -- that
                // would render as `words = '10000..50000'` and match nothing,
                // which reads to the reader as "no such work" rather than
                // "you wrote a range where I expected a value".
                if let Some(node) = self.try_range(f, value, start)? {
                    return Ok(node);
                }

                // `tag:"Fake Dating"+children` or `+children^2`.
                //
                // Two cases, and both are needed:
                //
                // * QUOTED (`tag:"Fake Dating"+children`) -- the term scan stopped at
                //   the closing quote, so the suffix is the next characters in the
                //   INPUT and `current_value` recovers the term from the text.
                // * UNQUOTED (`tag:Fake+children`) -- the term scan does NOT stop at
                //   `+`, so the suffix arrived inside `value`. Handling only the
                //   quoted form made `tag:Fake+children` a `Fielded` whose value is
                //   literally "Fake+children", which matches nothing and looks like
                //   a correct search. The `+` is split off here.
                if let Some(node) = self.take_expansion_suffix(f)? {
                    return Ok(node);
                }
                if let Some((head, _tail)) = value.split_once('+') {
                    if head.is_empty() {
                        return Err(QueryError::new(
                            format!("{}: needs a term before +children", f.as_str()),
                            start,
                        ));
                    }
                    // Put the cursor on the `+` so the suffix reader sees the marker
                    // in the input exactly as it does for a quoted value. The value
                    // began at `start + colon + 1` (the colon is one byte), and
                    // `head` is its prefix, so the `+` is that far along.
                    self.pos = start + colon + 1 + head.len();
                    if let Some(node) = self.take_expansion_suffix(f)? {
                        return Ok(node);
                    }
                }

                return Ok(QueryAst::Fielded(f, value.to_owned()));
            }
        }

        Ok(QueryAst::Text(term.to_owned()))
    }

    /// Expands `low..high` into the comparison(s) it stands for.
    ///
    /// Returns `Ok(None)` when the value contains no `..` and is therefore a
    /// plain equality. Returns `Err` for a range that cannot be honoured --
    /// nothing at either end, a non-numeric bound, a backwards range, or a
    /// field with no ordering. In every one of those cases an error is
    /// returned rather than a silently looser query, because all four would
    /// otherwise widen the result set while looking like a filter.
    fn try_range(
        &self,
        field: QueryField,
        value: &str,
        offset: usize,
    ) -> std::result::Result<Option<QueryAst>, QueryError> {
        let Some(dot) = value.find("..") else {
            return Ok(None);
        };

        // A range is only meaningful where the values order, and only on a
        // column of one type -- so a range on a text field, or one mixing a
        // number and a date, is refused here rather than half-rendered.
        let kind = field.comparison_value();
        if !field.is_comparable() {
            return Err(QueryError::new(
                format!(
                    "{} takes a single value, not a range -- it has no ordering",
                    field.as_str()
                ),
                offset,
            ));
        }

        let (low, high) = (&value[..dot], &value[dot + 2..]);
        if low.is_empty() && high.is_empty() {
            return Err(QueryError::new(
                format!("{}:.. has no bounds to compare against", field.as_str()),
                offset,
            ));
        }

        for bound in [low, high].into_iter().filter(|b| !b.is_empty()) {
            if !valid_bound(kind, bound) {
                return Err(QueryError::new(
                    format!(
                        "{} range bounds must be {}, got {bound:?}",
                        field.as_str(),
                        describe_kind(kind)
                    ),
                    offset,
                ));
            }
        }

        let mut parts = Vec::with_capacity(2);
        if !low.is_empty() {
            parts.push(QueryAst::Comparison(field, CompareOp::Gte, low.to_owned()));
        }
        if !high.is_empty() {
            parts.push(QueryAst::Comparison(field, CompareOp::Lte, high.to_owned()));
        }

        // A backwards range can never match. Saying so is the difference
        // between a sentence the reader can act on and an empty result page
        // they have to reverse-engineer. Only meaningful when both bounds are
        // the same kind of thing, which `valid_bound` has already ensured.
        if parts.len() == 2 && bounds_descend(kind, low, high) {
            return Err(QueryError::new(
                format!(
                    "{} range starts at {low} and ends at {high}, \
                     so it can never match",
                    field.as_str()
                ),
                offset,
            ));
        }

        Ok(Some(if parts.len() == 1 {
            parts.pop().expect("just pushed one")
        } else {
            QueryAst::And(parts)
        }))
    }

    /// True when `word` is here and is not merely the prefix of a longer
    /// identifier.
    ///
    /// A plain `starts_with` would make `min_matches:2` parse as `min_match` with
    /// the reader's field silently consumed. The next character must not be a word
    /// character, mirroring what `peek_keyword` already does for `AND`/`OR`/`NOT`.
    fn peek_word(&self, word: &str) -> bool {
        if !self.input[self.pos..].starts_with(word) {
            return false;
        }
        match self.input[self.pos + word.len()..].chars().next() {
            None => true,
            Some(c) => !(c.is_alphanumeric() || c == '_' || c == ':'),
        }
    }

    /// `min_match(n, term, term, ...)`.
    ///
    /// §15.4.1.2. The arity and the `n <= arity` check happen HERE rather than in the
    /// budget walk, because only here can the error point at the reader's own `n`:
    /// the budget check is a backstop for hand-built ASTs, and a message about
    /// "terms" when the reader wrote a number reads as though their number was
    /// ignored.
    fn parse_min_match(&mut self) -> std::result::Result<QueryAst, QueryError> {
        let start = self.pos;
        self.pos += "min_match".len();
        self.skip_ws();
        if !self.input[self.pos..].starts_with('(') {
            return Err(QueryError::new(
                "min_match needs an argument list: min_match(2, tag:\"A\", tag:\"B\")",
                self.pos,
            ));
        }
        self.pos += 1;
        self.skip_ws();

        // The count.
        let count_start = self.pos;
        while self.pos < self.input.len() {
            let c = self.input[self.pos..].chars().next().unwrap();
            if c.is_ascii_digit() {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == count_start {
            return Err(QueryError::new(
                "min_match's first argument is how many terms must match, e.g. \
                 min_match(2, tag:\"A\", tag:\"B\")",
                count_start,
            ));
        }
        let needed: usize = self.input[count_start..self.pos]
            .parse()
            .map_err(|_| QueryError::new("min_match's count is too large", count_start))?;
        if needed == 0 {
            return Err(QueryError::new(
                "min_match(0, ...) would match everything, so it is not a filter",
                count_start,
            ));
        }

        // The terms, comma separated to the closing paren.
        let mut terms = Vec::new();
        loop {
            self.skip_ws();
            if self.pos >= self.input.len() {
                return Err(QueryError::new(
                    "min_match is missing its closing ')'",
                    self.pos,
                ));
            }
            if self.input[self.pos..].starts_with(')') {
                self.pos += 1;
                break;
            }
            if self.input[self.pos..].starts_with(',') {
                self.pos += 1;
                continue;
            }
            let term = self.parse_not()?;
            if term == QueryAst::Text(String::new()) {
                return Err(QueryError::new("min_match has an empty term", self.pos));
            }
            terms.push(term);
            if self.budget.max_min_match_arity != 0 && terms.len() > self.budget.max_min_match_arity
            {
                return Err(QueryError::new(
                    format!(
                        "min_match has more than {} terms",
                        self.budget.max_min_match_arity
                    ),
                    start,
                ));
            }
        }

        if terms.is_empty() {
            return Err(QueryError::new(
                "min_match needs at least one term to count",
                start,
            ));
        }
        if needed > terms.len() {
            // An unsatisfiable count is refused rather than returning nothing, for
            // the same reason a backwards range is: an empty result page the reader
            // has to reverse-engineer is the worse outcome.
            return Err(QueryError::new(
                format!(
                    "min_match needs {needed} of {} terms, so it can never match",
                    terms.len()
                ),
                start,
            ));
        }
        Ok(QueryAst::MinMatch { needed, terms })
    }

    /// `relation:(key:value key:value)` — a correlated sub-predicate.
    ///
    /// §15.4.1.1. Called from `parse_term` once a `field:` is recognised AND the
    /// next character is `(`, which is what distinguishes `ship:(...)` from a plain
    /// `ship:value`.
    fn parse_scoped(&mut self, relation: QueryField) -> std::result::Result<QueryAst, QueryError> {
        let start = self.pos;
        // Refused here rather than at render time so the error carries the reader's
        // offset and can be underlined (§15.4.1.6). A renderer can only report
        // offset 0, which points at the start of the whole query.
        if relation != QueryField::Relationship {
            return Err(QueryError::new(
                format!(
                    "{} has no single row to scope a sub-predicate to; only \
                     `relationship:` does",
                    relation.as_str()
                ),
                start,
            ));
        }
        self.pos += 1; // the '('
        let mut fields = Vec::new();
        loop {
            self.skip_ws();
            if self.pos >= self.input.len() {
                return Err(QueryError::new(
                    format!("{}:( is missing its closing ')'", relation.as_str()),
                    self.pos,
                ));
            }
            if self.input[self.pos..].starts_with(')') {
                self.pos += 1;
                break;
            }

            // One `key:value`, where the value may be quoted.
            let key_start = self.pos;
            while self.pos < self.input.len() {
                let c = self.input[self.pos..].chars().next().unwrap();
                if c.is_whitespace() || c == ':' || c == ')' {
                    break;
                }
                self.pos += c.len_utf8();
            }
            let key = self.input[key_start..self.pos].to_owned();
            if key.is_empty() {
                return Err(QueryError::new(
                    format!("{}:( expects `key:value` pairs", relation.as_str()),
                    key_start,
                ));
            }
            if !self.input[self.pos..].starts_with(':') {
                return Err(QueryError::new(
                    format!(
                        "{key:?} in {}(:) needs a value, as {key}:\"...\"",
                        relation.as_str()
                    ),
                    self.pos,
                ));
            }
            self.pos += 1; // the ':'

            let value = if self.input[self.pos..].starts_with('"') {
                let QueryAst::Phrase(v) = self.parse_phrase()? else {
                    unreachable!("parse_phrase returns a Phrase")
                };
                v
            } else {
                let v_start = self.pos;
                while self.pos < self.input.len() {
                    let c = self.input[self.pos..].chars().next().unwrap();
                    if c.is_whitespace() || c == ')' {
                        break;
                    }
                    self.pos += c.len_utf8();
                }
                self.input[v_start..self.pos].to_owned()
            };
            if value.is_empty() {
                return Err(QueryError::new(
                    format!("{key}: in {}(:) needs a value", relation.as_str()),
                    self.pos,
                ));
            }
            // The accepted key set is fixed and small, so an unknown key is caught
            // here where the error can name it and underline it. Letting it through
            // to the renderer would defer the complaint to compile time, with no
            // span.
            if !matches!(
                key.as_str(),
                "with" | "participant" | "type" | "rel_type" | "prominence" | "label"
            ) {
                return Err(QueryError::new(
                    format!(
                        "{key:?} is not a relationship bound; expected one of with, \
                         type, prominence, label"
                    ),
                    key_start,
                ));
            }
            fields.push(ScopedField { key, value });
        }

        if fields.is_empty() {
            return Err(QueryError::new(
                format!("{}(:) needs at least one key:value pair", relation.as_str()),
                start,
            ));
        }
        if !fields
            .iter()
            .any(|f| matches!(f.key.as_str(), "with" | "participant"))
        {
            return Err(QueryError::new(
                format!(
                    "a scoped {} needs at least one `with:` naming a participant; \
                     without one it would match every relationship in the instance",
                    relation.as_str()
                ),
                start,
            ));
        }
        Ok(QueryAst::Scoped(ScopedPredicate { relation, fields }))
    }

    /// Read a `+children` / `+children^N` suffix if one follows the current value.
    ///
    /// §15.4.1.3. Returns `Ok(Some(node))` when an expansion was consumed, and
    /// `Ok(None)` when there is no suffix — in which case the value stands alone and
    /// the caller emits a plain `Fielded`.
    ///
    /// Only taxonomy fields expand. `words:1000+children` is a category error, and
    /// saying so beats storing the `+children` in the value where it matches nothing.
    fn take_expansion_suffix(
        &mut self,
        field: QueryField,
    ) -> std::result::Result<Option<QueryAst>, QueryError> {
        if !self.input[self.pos..].starts_with('+') {
            return Ok(None);
        }
        let marker_start = self.pos;
        if !field.is_taxonomy() {
            return Err(QueryError::new(
                format!(
                    "{} is not a taxonomy term, so it has no children to expand to",
                    field.as_str()
                ),
                marker_start,
            ));
        }
        // The value has already been read into the caller's `value`; recover it from
        // the input so the node carries the term the reader typed.
        let term = self.current_value(marker_start);
        self.pos += 1; // '+'

        const MARKER: &str = "children";
        if !self.input[self.pos..].starts_with(MARKER) {
            return Err(QueryError::new(
                "the only expansion is +children (optionally +children^2)",
                marker_start,
            ));
        }
        self.pos += MARKER.len();

        // An optional `^N` depth cap.
        let mut depth = None;
        if self.input[self.pos..].starts_with('^') {
            let caret = self.pos;
            self.pos += 1;
            let num_start = self.pos;
            while self.pos < self.input.len() {
                let c = self.input[self.pos..].chars().next().unwrap();
                if c.is_ascii_digit() {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if self.pos == num_start {
                return Err(QueryError::new(
                    "expansion depth needs a number, as +children^2",
                    caret,
                ));
            }
            let n: u32 = self.input[num_start..self.pos]
                .parse()
                .map_err(|_| QueryError::new("expansion depth is too large", num_start))?;
            if self.budget.max_expansion != 0 && n as usize > self.budget.max_expansion {
                return Err(QueryError::new(
                    format!(
                        "expansion depth {n} is past the limit of {}",
                        self.budget.max_expansion
                    ),
                    caret,
                ));
            }
            depth = Some(n);
        }
        Ok(Some(QueryAst::Expand { field, term, depth }))
    }

    /// The literal value immediately before `end`, stripped of surrounding quotes.
    ///
    /// A recovery helper rather than a parameter: the value was consumed by
    /// `parse_term` before this runs, and threading it through would mean changing
    /// the signature of the equality path for one caller. It walks backwards over
    /// the already-parsed text, so it cannot see past the value.
    fn current_value(&self, end: usize) -> String {
        let before = &self.input[..end];
        let trimmed = before.trim_end();
        if let Some(stripped) = trimmed.strip_suffix('"') {
            // Quoted: the opening quote is the last `"` before the closing one.
            if let Some(open) = stripped.rfind('"') {
                return stripped[open + 1..].to_owned();
            }
        }
        // Unquoted: back to the `:` that introduced it.
        match trimmed.rfind(':') {
            Some(c) => trimmed[c + 1..].to_owned(),
            None => trimmed.to_owned(),
        }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.input.len() {
            let c = self.input[self.pos..].chars().next().unwrap();
            if c.is_whitespace() {
                self.pos += c.len_utf8();
            } else {
                break;
            }
        }
    }

    fn peek_keyword(&self, kw: &str) -> bool {
        if self.input[self.pos..].starts_with(kw) {
            let after = self.pos + kw.len();
            if after >= self.input.len()
                || self.input[after..]
                    .starts_with(|c: char| c.is_whitespace() || c == '(' || c == ')')
            {
                return true;
            }
        }
        false
    }

    fn advance_keyword(&mut self, kw: &str) {
        self.pos += kw.len();
    }
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at offset {}", self.message, self.offset)
    }
}

impl std::error::Error for QueryError {}

#[cfg(test)]
mod tests {
    #[test]
    fn quoted_unicode_and_fielded_phrases_parse() {
        let fielded = super::parse_query(concat!("title:", '"', "été bleu", '"')).unwrap();
        assert_eq!(
            fielded,
            super::QueryAst::Fielded(super::QueryField::Title, "été bleu".into())
        );
        let phrase = super::parse_query(concat!('"', "été bleu", '"')).unwrap();
        assert_eq!(phrase, super::QueryAst::Phrase("été bleu".into()));
        assert!(super::parse_query("title:").is_err());
    }

    use super::*;

    #[test]
    fn parse_free_text() {
        let ast = parse_query("winter").unwrap();
        assert_eq!(ast, QueryAst::Text("winter".to_owned()));
    }

    #[test]
    fn parse_phrase() {
        let ast = parse_query("\"we were never alone\"").unwrap();
        assert_eq!(ast, QueryAst::Phrase("we were never alone".to_owned()));
    }

    #[test]
    fn parse_fielded() {
        let ast = parse_query("fandom:Harry").unwrap();
        assert_eq!(
            ast,
            QueryAst::Fielded(QueryField::Fandom, "Harry".to_owned())
        );
    }

    #[test]
    fn parse_and() {
        let ast = parse_query("fandom:x AND tag:y").unwrap();
        match ast {
            QueryAst::And(parts) => assert_eq!(parts.len(), 2),
            _ => panic!("expected AND, got {:?}", ast),
        }
    }

    #[test]
    fn parse_or() {
        let ast = parse_query("a OR b").unwrap();
        match ast {
            QueryAst::Or(parts) => assert_eq!(parts.len(), 2),
            _ => panic!("expected OR, got {:?}", ast),
        }
    }

    #[test]
    fn parse_not() {
        let ast = parse_query("NOT a").unwrap();
        match ast {
            QueryAst::Not(_) => {}
            _ => panic!("expected NOT, got {:?}", ast),
        }
    }

    #[test]
    fn parse_leading_minus() {
        let ast = parse_query("-tag:death").unwrap();
        match ast {
            QueryAst::Not(inner) => match inner.as_ref() {
                QueryAst::Fielded(QueryField::Tag, val) => assert_eq!(val, "death"),
                other => panic!("expected fielded tag, got {:?}", other),
            },
            _ => panic!("expected NOT, got {:?}", ast),
        }
    }

    #[test]
    fn parse_parentheses() {
        let ast = parse_query("(a OR b) AND c").unwrap();
        match ast {
            QueryAst::And(parts) => {
                assert_eq!(parts.len(), 2);
                match &parts[0] {
                    QueryAst::Or(sub) => assert_eq!(sub.len(), 2),
                    other => panic!("expected OR, got {:?}", other),
                }
            }
            _ => panic!("expected AND, got {:?}", ast),
        }
    }

    #[test]
    fn parse_error_includes_offset() {
        let err = parse_query("\"unterminated").unwrap_err();
        assert_eq!(err.offset, 0);
        assert!(err.message.contains("unterminated"));
    }

    #[test]
    fn implicit_conjunction() {
        let ast = parse_query("a b c").unwrap();
        match ast {
            QueryAst::And(parts) => assert_eq!(parts.len(), 3),
            _ => panic!("expected AND, got {:?}", ast),
        }
    }

    #[test]
    fn fielded_unknown_falls_back_to_text() {
        let ast = parse_query("unknown:value").unwrap();
        assert_eq!(ast, QueryAst::Text("unknown:value".to_owned()));
    }

    // --- Comparison operators -------------------------------------------------
    //
    // The operators are `:`, `>`, `<`, `>=`, `<=` and `..`. They are shared by
    // every entity type, so they are parsed in the language, not per surface.

    #[test]
    fn fielded_greater_than() {
        let ast = parse_query("words:>10000").unwrap();
        assert_eq!(
            ast,
            QueryAst::Comparison(QueryField::Words, CompareOp::Gt, "10000".to_owned())
        );
    }

    #[test]
    fn fielded_greater_or_equal() {
        let ast = parse_query("kudos:>=100").unwrap();
        assert_eq!(
            ast,
            QueryAst::Comparison(QueryField::Kudos, CompareOp::Gte, "100".to_owned())
        );
    }

    #[test]
    fn fielded_less_than() {
        let ast = parse_query("replies:<50").unwrap();
        assert_eq!(
            ast,
            QueryAst::Comparison(QueryField::Replies, CompareOp::Lt, "50".to_owned())
        );
    }

    #[test]
    fn fielded_less_or_equal() {
        let ast = parse_query("words:<=5000").unwrap();
        assert_eq!(
            ast,
            QueryAst::Comparison(QueryField::Words, CompareOp::Lte, "5000".to_owned())
        );
    }

    #[test]
    fn fielded_equality_is_not_a_comparison() {
        // `:` is equality, and must stay distinguishable from `>=`.
        let ast = parse_query("words:10000").unwrap();
        assert_eq!(
            ast,
            QueryAst::Fielded(QueryField::Words, "10000".to_owned())
        );
    }

    #[test]
    fn the_longest_operator_wins() {
        // `>=` must not parse as `>` with the `=` left in the value, which is
        // the exact bug the orphan parser had.
        let ast = parse_query("kudos:>=100").unwrap();
        match ast {
            QueryAst::Comparison(_, CompareOp::Gte, v) => assert_eq!(v, "100"),
            other => panic!("expected Gte, got {other:?}"),
        }
    }

    #[test]
    fn a_bare_greater_than_is_not_a_field() {
        // No field name, so this is free text -- not a comparison.
        let ast = parse_query(">100").unwrap();
        assert_eq!(ast, QueryAst::Text(">100".to_owned()));
    }

    #[test]
    fn comparison_inside_boolean_expression() {
        let ast = parse_query("words:>10000 AND tag:romance").unwrap();
        match ast {
            QueryAst::And(parts) => {
                assert_eq!(parts.len(), 2);
                assert!(matches!(
                    parts[0],
                    QueryAst::Comparison(QueryField::Words, CompareOp::Gt, _)
                ));
                assert!(matches!(parts[1], QueryAst::Fielded(QueryField::Tag, _)));
            }
            other => panic!("expected AND, got {other:?}"),
        }
    }

    #[test]
    fn comparison_inside_a_group() {
        let ast = parse_query("(words:>1000 OR kudos:>500) AND tag:romance").unwrap();
        match &ast {
            QueryAst::And(parts) => match &parts[0] {
                QueryAst::Or(inner) => {
                    assert_eq!(inner.len(), 2);
                    assert!(matches!(
                        inner[0],
                        QueryAst::Comparison(QueryField::Words, CompareOp::Gt, _)
                    ));
                }
                other => panic!("expected OR, got {other:?}"),
            },
            other => panic!("expected AND, got {other:?}"),
        }
    }

    #[test]
    fn comparison_equality_and_comparison_mix() {
        let ast = parse_query("status:complete words:>500").unwrap();
        match ast {
            QueryAst::And(parts) => {
                assert!(matches!(parts[0], QueryAst::Fielded(QueryField::Status, _)));
                assert!(matches!(
                    parts[1],
                    QueryAst::Comparison(QueryField::Words, CompareOp::Gt, _)
                ));
            }
            other => panic!("expected AND, got {other:?}"),
        }
    }

    #[test]
    fn negated_comparison() {
        let ast = parse_query("NOT words:>10000").unwrap();
        match ast {
            QueryAst::Not(inner) => {
                assert!(matches!(
                    *inner,
                    QueryAst::Comparison(QueryField::Words, CompareOp::Gt, _)
                ));
            }
            other => panic!("expected NOT, got {other:?}"),
        }
    }

    #[test]
    fn a_trailing_not_negates_the_next_term() {
        // `a NOT b` reads as `a AND NOT b`. The parser's implicit-conjunction
        // loop used to break on a `NOT` keyword and then `parse()` demanded
        // EOF, so the whole query was rejected -- a pre-existing fault, and the
        // grammar every search box advertises.
        let ast = parse_query("a NOT b").unwrap();
        match ast {
            QueryAst::And(parts) => {
                assert_eq!(parts.len(), 2);
                assert_eq!(parts[0], QueryAst::Text("a".to_owned()));
                match &parts[1] {
                    QueryAst::Not(inner) => {
                        assert_eq!(**inner, QueryAst::Text("b".to_owned()))
                    }
                    other => panic!("expected a negated second term, got {other:?}"),
                }
            }
            other => panic!("expected AND, got {other:?}"),
        }
    }

    #[test]
    fn a_trailing_not_works_after_a_group() {
        // The same break in the same loop, reached through a parenthesised
        // sub-query: `(a OR b) NOT c`.
        let ast = parse_query("(a OR b) NOT c").unwrap();
        match ast {
            QueryAst::And(parts) => {
                assert_eq!(parts.len(), 2);
                assert!(matches!(&parts[0], QueryAst::Or(inner) if inner.len() == 2));
                assert!(matches!(&parts[1], QueryAst::Not(_)));
            }
            other => panic!("expected AND, got {other:?}"),
        }
    }

    #[test]
    fn a_trailing_not_negates_a_fielded_term() {
        let ast = parse_query("a NOT tag:spoiler").unwrap();
        match ast {
            QueryAst::And(parts) => match &parts[1] {
                QueryAst::Not(inner) => assert_eq!(
                    **inner,
                    QueryAst::Fielded(QueryField::Tag, "spoiler".to_owned())
                ),
                other => panic!("expected a negated field, got {other:?}"),
            },
            other => panic!("expected AND, got {other:?}"),
        }
    }

    #[test]
    fn a_leading_not_still_binds_its_own_term() {
        // `NOT a b` is `NOT a AND b`, which is what it has always meant. The fix
        // for the trailing form must not change it.
        let ast = parse_query("NOT a b").unwrap();
        match ast {
            QueryAst::And(parts) => {
                assert_eq!(parts.len(), 2);
                match &parts[0] {
                    QueryAst::Not(inner) => assert_eq!(**inner, QueryAst::Text("a".to_owned())),
                    other => panic!("expected a leading NOT, got {other:?}"),
                }
                assert_eq!(parts[1], QueryAst::Text("b".to_owned()));
            }
            other => panic!("expected AND, got {other:?}"),
        }
    }

    #[test]
    fn a_double_not_is_allowed() {
        let ast = parse_query("a NOT NOT b").unwrap();
        match ast {
            QueryAst::And(parts) => match &parts[1] {
                QueryAst::Not(outer) => {
                    assert!(matches!(outer.as_ref(), QueryAst::Not(_)));
                }
                other => panic!("expected a double negation, got {other:?}"),
            },
            other => panic!("expected AND, got {other:?}"),
        }
    }

    // --- Cross-entity fields -------------------------------------------------
    //
    // The field names are entity-specific; the operators are not. A user who
    // learns `words:>10000` for works can immediately use `replies:>50` for
    // forum posts because the grammar is the same.

    #[test]
    fn cross_entity_fields_parse() {
        // Forum.
        assert!(matches!(
            parse_query("category:meta").unwrap(),
            QueryAst::Fielded(QueryField::Category, _)
        ));
        // Directory.
        assert!(matches!(
            parse_query("rank:>100").unwrap(),
            QueryAst::Comparison(QueryField::Rank, CompareOp::Gt, _)
        ));
        // Bookmark.
        assert!(matches!(
            parse_query("rec:true").unwrap(),
            QueryAst::Fielded(QueryField::Rec, _)
        ));
        // User.
        assert!(matches!(
            parse_query("works:>10").unwrap(),
            QueryAst::Comparison(QueryField::Works, CompareOp::Gt, _)
        ));
    }

    #[test]
    fn a_quoted_comparison_value_is_a_phrase() {
        // `tag:"slow burn"` keeps working; a quoted value is never a number.
        let ast = parse_query(concat!("tag:", '"', "slow burn", '"')).unwrap();
        assert_eq!(
            ast,
            QueryAst::Fielded(QueryField::Tag, "slow burn".to_owned())
        );
    }

    #[test]
    fn comparison_on_a_quoted_phrase_is_rejected() {
        // `words:>"many"` is a category error, not a silently-zero result.
        let err = parse_query(concat!("words:>", '"', "many", '"')).unwrap_err();
        assert!(
            err.message.contains("numeric") || err.message.contains("integer"),
            "expected a numeric-value error, got: {}",
            err.message
        );
    }

    // --- `..` ranges --------------------------------------------------------
    //
    // A range is sugar for two comparisons, never a new AST node: a renderer
    // that understands `>=` and `<=` already understands `10000..50000`, and a
    // distinct `Range` variant would need its own arm in every renderer --
    // four of them, once the other surfaces land -- for no expressive gain.

    /// The comparisons a range expands to, as a slice. A one-bound range is a
    /// single node rather than an `And` of one, so this normalises both shapes
    /// and the test states the comparison count directly.
    fn range_parts(q: &str) -> Vec<QueryAst> {
        match parse_query(q).unwrap() {
            QueryAst::And(parts) => parts,
            other => vec![other],
        }
    }

    #[test]
    fn a_closed_range_is_two_inclusive_bounds() {
        let parts = range_parts("words:10000..50000");
        assert_eq!(parts.len(), 2, "a range is exactly two bounds");
        assert_eq!(
            parts[0],
            QueryAst::Comparison(QueryField::Words, CompareOp::Gte, "10000".into())
        );
        assert_eq!(
            parts[1],
            QueryAst::Comparison(QueryField::Words, CompareOp::Lte, "50000".into())
        );
    }

    #[test]
    fn an_open_lower_bound_keeps_only_the_upper() {
        // `..50000` means "up to 50k". Inclusive, like every other bound here:
        // Rust's own `..` is exclusive at the top, but a reader typing a word
        // count almost never means "not including exactly 50,000", and a
        // language where `..` and `<=` disagree at the boundary is a language
        // nobody can predict. Exclusive stays reachable as `words:<50000`.
        let parts = range_parts("words:..50000");
        assert_eq!(
            parts.len(),
            1,
            "an absent bound is not a wildcard comparison"
        );
        assert_eq!(
            parts[0],
            QueryAst::Comparison(QueryField::Words, CompareOp::Lte, "50000".into())
        );
    }

    #[test]
    fn an_open_upper_bound_keeps_only_the_lower() {
        let parts = range_parts("words:10000..");
        assert_eq!(parts.len(), 1);
        assert_eq!(
            parts[0],
            QueryAst::Comparison(QueryField::Words, CompareOp::Gte, "10000".into())
        );
    }

    #[test]
    fn a_range_with_no_bounds_at_all_is_rejected() {
        // `words:..` bounds nothing. Accepting it would silently drop the term
        // and widen the result set, which is the same class of failure as
        // ignoring a field that belongs to another surface.
        let err = parse_query("words:..").unwrap_err();
        assert!(
            err.message.contains("range") || err.message.contains("bound"),
            "expected a range/bound error, got: {}",
            err.message
        );
    }

    #[test]
    fn a_backwards_range_is_rejected_rather_than_returning_nothing() {
        // `10000..5000` can match no row at all. Saying so at parse time turns
        // a baffling empty result into a sentence the reader can act on.
        let err = parse_query("words:10000..5000").unwrap_err();
        assert!(
            err.message.contains("range") || err.message.contains("bound"),
            "expected a range/bound error, got: {}",
            err.message
        );
    }

    #[test]
    fn a_non_numeric_range_bound_is_rejected() {
        // One token, so the range is not split by a space: `a lot` would be two
        // terms and the parser would be right to treat them as such.
        let err = parse_query("words:many..5000").unwrap_err();
        assert!(
            err.message.contains("whole numbers") || err.message.contains("numeric"),
            "expected a numeric-value error, got: {}",
            err.message
        );
    }

    #[test]
    fn a_range_with_a_space_in_it_is_two_terms_not_a_bad_range() {
        // `words:a lot..5000` has a space, so it is `words:a` AND `lot..5000`
        // -- the free-text `lot..5000`, not a range. Erroring here would
        // reject a query whose every token is individually valid.
        let ast = parse_query("words:a lot..5000").unwrap();
        assert!(
            matches!(ast, QueryAst::And(ref parts) if parts.len() == 2),
            "expected two terms, got {ast:?}"
        );
    }

    #[test]
    fn a_range_on_a_text_field_is_rejected() {
        // `title:abc..def` is a category error: there is no ordering on a
        // string, so the two bounds have nothing to mean.
        let err = parse_query("title:abc..def").unwrap_err();
        assert!(
            err.message.contains("range") || err.message.contains("comparable"),
            "expected a range/comparable error, got: {}",
            err.message
        );
    }

    #[test]
    fn a_range_composes_with_other_terms() {
        // The range expands to an `And` node, so it has to compose with the
        // terms around it -- a range that silently dropped out of a longer
        // query would widen the results without saying so.
        let ast = parse_query("winter words:1000..5000").unwrap();
        match ast {
            QueryAst::And(parts) => {
                assert_eq!(parts.len(), 3, "winter, plus two bounds: {parts:?}");
                assert_eq!(parts[0], QueryAst::Text("winter".into()));
                assert_eq!(
                    parts[1],
                    QueryAst::Comparison(QueryField::Words, CompareOp::Gte, "1000".into())
                );
                assert_eq!(
                    parts[2],
                    QueryAst::Comparison(QueryField::Words, CompareOp::Lte, "5000".into())
                );
            }
            other => panic!("expected a three-part And, got {other:?}"),
        }
    }

    #[test]
    fn a_spliced_range_still_negates_as_a_pair() {
        // Splicing flattens the range into two sibling terms, so `NOT` has to
        // be applied by the reader grouping them: `NOT (a AND b)`, not
        // `NOT a` and `NOT b` separately. The parser cannot express that from
        // `NOT words:1000..5000` alone -- the `NOT` is seen before the range
        // expands -- so what it must guarantee is that the expansion lands
        // *inside* the negation rather than beside it.
        let ast = parse_query("NOT words:1000..5000").unwrap();
        assert!(
            matches!(ast, QueryAst::Not(_)),
            "the range must stay inside the NOT, got {ast:?}"
        );
    }

    #[test]
    fn a_negated_spliced_range_is_a_double_negation_of_each_bound() {
        // `NOT a AND NOT b` and `NOT (a AND b)` agree on the empty and
        // full-result cases and disagree on "both bounds hold", so pin the
        // shape the reader has to write to get what they mean, and document
        // that the bare form is the looser one.
        let grouped = parse_query("NOT (words:1000..5000)").unwrap();
        assert!(
            matches!(grouped, QueryAst::Not(ref inner) if matches!(**inner, QueryAst::And(_))),
            "parenthesised NOT negates the pair, got {grouped:?}"
        );
    }
}
