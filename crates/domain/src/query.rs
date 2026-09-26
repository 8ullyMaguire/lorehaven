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
            "user_fandom" => Self::UserFandom,
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

    /// Whether this field carries an ordering, so `>`, `<` and `..` mean
    /// something on it.
    ///
    /// This lives on the field rather than in a renderer because the parser
    /// needs it to reject `title:a..b` at the point the mistake is made. When
    /// the four remaining renderers land, a field that is comparable here but
    /// unmapped there is a renderer bug, not a query bug -- which is the right
    /// way round for the failure to be.
    pub fn is_comparable(self) -> bool {
        matches!(
            self,
            Self::Words
                | Self::Kudos
                | Self::Replies
                | Self::Rank
                | Self::Works
                | Self::MinQuality
                | Self::Quality
        )
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
/// parentheses, and leading `-` exclusions.
pub fn parse_query(input: &str) -> std::result::Result<QueryAst, QueryError> {
    let mut parser = Parser::new(input);
    parser.parse()
}

struct Parser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
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

        if self.input[self.pos..].starts_with('(') {
            self.pos += 1;
            let inner = self.parse_or()?;
            self.skip_ws();
            if !self.input[self.pos..].starts_with(')') {
                return Err(QueryError::new("expected ')'".to_owned(), self.pos));
            }
            self.pos += 1;
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
                    if value.is_empty() {
                        return Err(QueryError::new(
                            format!("{} {} needs a numeric value", f.as_str(), op.as_str()),
                            self.pos,
                        ));
                    }
                    if value.starts_with('"') {
                        // `words:>"many"` is a category error, not a zero result.
                        return Err(QueryError::new(
                            format!(
                                "{} {} takes a numeric value, not a quoted phrase",
                                f.as_str(),
                                op.as_str()
                            ),
                            self.pos,
                        ));
                    }
                    if value.parse::<i64>().is_err() {
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
                    return Ok(QueryAst::Comparison(f, op, value.to_owned()));
                }

                // Plain equality.
                if value.is_empty() {
                    if self.input[self.pos..].starts_with('"') {
                        let QueryAst::Phrase(value) = self.parse_phrase()? else {
                            unreachable!()
                        };
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
            if bound.parse::<i64>().is_err() {
                return Err(QueryError::new(
                    format!(
                        "{} range bounds must be whole numbers, got {bound:?}",
                        field.as_str()
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
        // they have to reverse-engineer.
        if parts.len() == 2 {
            let low_n: i64 = low.parse().expect("checked above");
            let high_n: i64 = high.parse().expect("checked above");
            if low_n > high_n {
                return Err(QueryError::new(
                    format!(
                        "{} range starts at {low_n} and ends at {high_n}, \
                         so it can never match",
                        field.as_str()
                    ),
                    offset,
                ));
            }
        }

        Ok(Some(if parts.len() == 1 {
            parts.pop().expect("just pushed one")
        } else {
            QueryAst::And(parts)
        }))
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
