//! Query language: parser, AST, and SQL rendering.
//!
//! Spec §15.3–15.6. Pure functions — no I/O.

/// A field in a fielded query.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
            _ => return None,
        })
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
        loop {
            self.skip_ws();
            if self.pos >= self.input.len() {
                break;
            }
            if self.peek_keyword("OR") || self.peek_keyword("AND") || self.peek_keyword("NOT") {
                break;
            }
            if self.input[self.pos..].starts_with(')') {
                break;
            }
            match self.parse_not() {
                Ok(term) if term != QueryAst::Text(String::new()) => terms.push(term),
                _ => break,
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
            self.pos += 1;
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
            if c.is_whitespace() || c == '(' || c == ')' || c == '"' || c == '-' {
                break;
            }
            self.pos += c.len_utf8();
        }
        let term = &self.input[start..self.pos];
        if term.is_empty() {
            return Err(QueryError::new("expected a term".to_owned(), start));
        }

        // Check for fielded search: field:value
        if let Some((field, value)) = term.split_once(':') {
            if let Some(f) = QueryField::parse(field) {
                return Ok(QueryAst::Fielded(f, value.to_owned()));
            }
        }

        Ok(QueryAst::Text(term.to_owned()))
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
}
