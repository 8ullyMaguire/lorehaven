//! Unified query language parser for cross-entity search.
//!
//! Spec §11.5.1 — consistent operators across all entity types.
//!
//! Syntax:
//!   tag:"enemies to lovers" fandom:"Good Omens" words:>10000 status:complete
//!   category:"meta" author:nightowl replies:>50 after:2026-01
//!
//! Operators: `:`, `>`, `<`, `>=`, `<=`, `..` (range)
//! Boolean: implicit AND, explicit OR, NOT / `-`
//! Grouping: parentheses

use std::fmt;

/// A parsed search query, ready to be translated to SQL per entity.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchQuery {
    pub terms: Vec<QueryTerm>,
    pub raw: String,
}

/// A single term in a search query.
#[derive(Debug, Clone, PartialEq)]
pub enum QueryTerm {
    /// Free-text term (no field).
    Free(String),
    /// Field:value term.
    Field {
        field: String,
        op: Operator,
        value: QueryValue,
    },
    /// Boolean operator connecting groups.
    Boolean(BooleanOp),
    /// Grouped sub-query (parentheses).
    Group(Box<SearchQuery>),
    /// Negation.
    Negate(Box<QueryTerm>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Operator {
    Equal,
    Greater,
    Less,
    GreaterEq,
    LessEq,
    Range(String, String), // start..end
}

#[derive(Debug, Clone, PartialEq)]
pub enum QueryValue {
    String(String),
    Number(f64),
    Date(String), // RFC 3339 date
}

#[derive(Debug, Clone, PartialEq)]
pub enum BooleanOp {
    And,
    Or,
}

/// Parse a search query string into a structured `SearchQuery`.
pub fn parse_query(input: &str) -> Result<SearchQuery, ParseError> {
    let mut parser = Parser::new(input);
    let terms = parser.parse_terms()?;
    Ok(SearchQuery {
        terms,
        raw: input.to_string(),
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub position: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "parse error at {}: {}", self.position, self.message)
    }
}

impl std::error::Error for ParseError {}

struct Parser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Parser { input, pos: 0 }
    }

    fn parse_terms(&mut self) -> Result<Vec<QueryTerm>, ParseError> {
        let mut terms = Vec::new();
        self.skip_whitespace();

        loop {
            self.skip_whitespace();
            if self.at_eof() {
                break;
            }

            let term = self.parse_single_term()?;
            terms.push(term);

            self.skip_whitespace();

            // Check for explicit boolean operators.
            if self.peek_keyword("OR") {
                terms.push(QueryTerm::Boolean(BooleanOp::Or));
                self.advance_keyword("OR");
            } else if !self.at_eof() && !self.peek_char(')') {
                // Implicit AND between terms (no explicit connector needed).
                // Don't push And — implicit in the Vec ordering.
            }
        }

        Ok(terms)
    }

    fn parse_single_term(&mut self) -> Result<QueryTerm, ParseError> {
        // Handle negation: NOT term or -term
        if self.peek_keyword("NOT") {
            self.advance_keyword("NOT");
            self.skip_whitespace();
            let inner = self.parse_single_term()?;
            return Ok(QueryTerm::Negate(Box::new(inner)));
        }
        if self.peek_char('-') && !self.peek_char2('-') {
            // Check it's not a range like -0.5
            if self.pos + 1 < self.input.len() {
                let next = self.input[self.pos + 1..].chars().next();
                if next
                    .map(|c| c.is_alphanumeric() || c == '"' || c == '\'')
                    .unwrap_or(false)
                {
                    self.advance();
                    let inner = self.parse_single_term()?;
                    return Ok(QueryTerm::Negate(Box::new(inner)));
                }
            }
        }

        // Handle grouped sub-queries: (...)
        if self.peek_char('(') {
            self.advance();
            let inner_terms = self.parse_terms()?;
            if !self.peek_char(')') {
                return Err(ParseError {
                    message: "expected ')'".to_string(),
                    position: self.pos,
                });
            }
            self.advance(); // consume ')'
            return Ok(QueryTerm::Group(Box::new(SearchQuery {
                terms: inner_terms,
                raw: String::new(),
            })));
        }

        // Handle field:value or field>value etc.
        if self.looks_like_field() {
            return self.parse_field_term();
        }

        // Otherwise it's a free-text term.
        let free = self.parse_free_text()?;
        Ok(QueryTerm::Free(free))
    }

    fn parse_field_term(&mut self) -> Result<QueryTerm, ParseError> {
        let field = self.parse_identifier()?;
        self.skip_whitespace();

        // Parse operator.
        let op = if self.peek_char(':') {
            self.advance();
            Operator::Equal
        } else if self.peek_chars(">=") {
            self.advance_n(2);
            Operator::GreaterEq
        } else if self.peek_chars("<=") {
            self.advance_n(2);
            Operator::LessEq
        } else if self.peek_char('>') {
            self.advance();
            Operator::Greater
        } else if self.peek_char('<') {
            self.advance();
            Operator::Less
        } else {
            // Bare field name treated as a free term.
            return Ok(QueryTerm::Free(field));
        };

        self.skip_whitespace();

        // Parse value — check for range (value..value).
        let _value_start = self.pos;
        let val1 = self.parse_value_text()?;

        self.skip_whitespace();
        if self.peek_chars("..") {
            self.advance_n(2);
            self.skip_whitespace();
            let val2 = self.parse_value_text()?;
            return Ok(QueryTerm::Field {
                field,
                op: Operator::Range(val1.clone(), val2.clone()),
                value: QueryValue::String(format!("{}..{}", val1, val2)),
            });
        }

        // Try to interpret val1 as a number, date, or keep as string.
        let value = if let Ok(n) = val1.parse::<f64>() {
            QueryValue::Number(n)
        } else if looks_like_date(&val1) {
            QueryValue::Date(val1)
        } else {
            QueryValue::String(val1)
        };

        Ok(QueryTerm::Field { field, op, value })
    }

    fn parse_free_text(&mut self) -> Result<String, ParseError> {
        if self.peek_char('"') || self.peek_char('\'') {
            return self.parse_quoted_string();
        }

        let start = self.pos;
        while !self.at_eof() && !self.is_term_end() {
            self.advance();
        }
        Ok(self.input[start..self.pos].trim().to_string())
    }

    fn parse_quoted_string(&mut self) -> Result<String, ParseError> {
        let quote = self.input[self.pos..].chars().next().unwrap();
        self.advance(); // skip opening quote

        let start = self.pos;
        while !self.at_eof() {
            let c = self.input[self.pos..].chars().next().unwrap();
            if c == quote {
                let result = self.input[start..self.pos].to_string();
                self.advance(); // skip closing quote
                return Ok(result);
            }
            self.advance();
        }
        Err(ParseError {
            message: "unterminated string".to_string(),
            position: start,
        })
    }

    fn parse_value_text(&mut self) -> Result<String, ParseError> {
        if self.peek_char('"') || self.peek_char('\'') {
            return self.parse_quoted_string();
        }
        let start = self.pos;
        while !self.at_eof() && !self.is_value_end() {
            self.advance();
        }
        Ok(self.input[start..self.pos].trim().to_string())
    }

    fn parse_identifier(&mut self) -> Result<String, ParseError> {
        let start = self.pos;
        while !self.at_eof()
            && (self.current_char().is_alphanumeric() || self.current_char() == '_')
        {
            self.advance();
        }
        if self.pos == start {
            return Err(ParseError {
                message: "expected field name".to_string(),
                position: self.pos,
            });
        }
        Ok(self.input[start..self.pos].to_string())
    }

    // Helper methods.
    fn at_eof(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn skip_whitespace(&mut self) {
        while !self.at_eof() && self.current_char().is_whitespace() {
            self.advance();
        }
    }

    fn peek_char(&self, c: char) -> bool {
        self.input[self.pos..].starts_with(c)
    }

    fn peek_char2(&self, c: char) -> bool {
        self.input[self.pos..].starts_with(c)
    }

    fn peek_chars(&self, s: &str) -> bool {
        self.input[self.pos..].starts_with(s)
    }

    fn peek_keyword(&self, kw: &str) -> bool {
        let rest = &self.input[self.pos..];
        if !rest.starts_with(kw) {
            return false;
        }
        // Ensure the keyword is followed by whitespace or EOF.
        match rest.get(kw.len()..) {
            None => true,
            Some(rest) => rest.starts_with(|c: char| c.is_whitespace() || c == '(' || c == ')'),
        }
    }

    fn advance(&mut self) {
        if !self.at_eof() {
            self.pos += self.current_char().len_utf8();
        }
    }

    fn advance_n(&mut self, n: usize) {
        for _ in 0..n {
            self.advance();
        }
    }

    fn advance_keyword(&mut self, kw: &str) {
        self.pos += kw.len();
    }

    fn current_char(&self) -> char {
        self.input[self.pos..].chars().next().unwrap_or('\0')
    }

    fn looks_like_field(&self) -> bool {
        if self.at_eof() || !self.current_char().is_alphabetic() {
            return false;
        }
        // Look ahead for operator without mutating self.
        let mut peek_pos = self.pos;
        while peek_pos < self.input.len() {
            let c = self.input[peek_pos..].chars().next().unwrap_or('\0');
            if c.is_alphanumeric() || c == '_' {
                peek_pos += c.len_utf8();
            } else {
                break;
            }
        }
        // Skip whitespace.
        while peek_pos < self.input.len() && self.input[peek_pos..].starts_with(' ') {
            peek_pos += 1;
        }
        let rest = &self.input[peek_pos..];
        rest.starts_with(':')
            || rest.starts_with('>')
            || rest.starts_with('<')
            || rest.starts_with(">=")
            || rest.starts_with("<=")
    }

    fn is_term_end(&self) -> bool {
        if self.at_eof() {
            return true;
        }
        let c = self.current_char();
        c.is_whitespace() || c == ')' || c == '('
    }

    fn is_value_end(&self) -> bool {
        if self.at_eof() {
            return true;
        }
        let c = self.current_char();
        c.is_whitespace() || c == ')'
    }
}

fn looks_like_date(s: &str) -> bool {
    // Simple heuristic: YYYY-MM or YYYY-MM-DD.
    if s.len() < 4 {
        return false;
    }
    s.chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_query() {
        let q = parse_query("").unwrap();
        assert!(q.terms.is_empty());
    }

    #[test]
    fn parse_free_text() {
        let q = parse_query("hello world").unwrap();
        assert_eq!(q.terms.len(), 2);
        assert_eq!(q.terms[0], QueryTerm::Free("hello".to_string()));
        assert_eq!(q.terms[1], QueryTerm::Free("world".to_string()));
    }

    #[test]
    fn parse_field_equal() {
        let q = parse_query("status:complete").unwrap();
        assert_eq!(q.terms.len(), 1);
        match &q.terms[0] {
            QueryTerm::Field { field, op, value } => {
                assert_eq!(field, "status");
                assert_eq!(*op, Operator::Equal);
                assert_eq!(*value, QueryValue::String("complete".to_string()));
            }
            _ => panic!("expected field term"),
        }
    }

    #[test]
    fn parse_quoted_phrase() {
        let q = parse_query("tag:\"enemies to lovers\"").unwrap();
        assert_eq!(q.terms.len(), 1);
        match &q.terms[0] {
            QueryTerm::Field { field, value, .. } => {
                assert_eq!(field, "tag");
                assert_eq!(*value, QueryValue::String("enemies to lovers".to_string()));
            }
            _ => panic!("expected field term"),
        }
    }

    #[test]
    fn parse_numeric_comparison() {
        let q = parse_query("words:>10000").unwrap();
        assert_eq!(q.terms.len(), 1);
        match &q.terms[0] {
            QueryTerm::Field { field, op, value } => {
                assert_eq!(field, "words");
                assert_eq!(*op, Operator::Greater);
                assert_eq!(*value, QueryValue::Number(10000.0));
            }
            _ => panic!("expected field term"),
        }
    }

    #[test]
    fn parse_range() {
        let q = parse_query("date:2026-01..2026-06").unwrap();
        assert_eq!(q.terms.len(), 1);
        match &q.terms[0] {
            QueryTerm::Field { field, op, .. } => {
                assert_eq!(field, "date");
                match op {
                    Operator::Range(start, end) => {
                        assert_eq!(start, "2026-01");
                        assert_eq!(end, "2026-06");
                    }
                    _ => panic!("expected range operator"),
                }
            }
            _ => panic!("expected field term"),
        }
    }

    #[test]
    fn parse_negation() {
        let q = parse_query("NOT tag:spoiler").unwrap();
        assert_eq!(q.terms.len(), 1);
        match &q.terms[0] {
            QueryTerm::Negate(inner) => match inner.as_ref() {
                QueryTerm::Field { field, .. } => assert_eq!(field, "tag"),
                _ => panic!("expected field inside negate"),
            },
            _ => panic!("expected negate term"),
        }
    }

    #[test]
    fn parse_grouping() {
        let q = parse_query("(tag:romance OR tag:angst) words:>5000").unwrap();
        assert!(!q.terms.is_empty());
        // First term should be a group.
        match &q.terms[0] {
            QueryTerm::Group(g) => assert!(!g.terms.is_empty()),
            _ => panic!("expected group term"),
        }
    }

    #[test]
    fn parse_complex_query() {
        let input = "tag:\"enemies to lovers\" fandom:\"Good Omens\" words:>10000 status:complete";
        let q = parse_query(input).unwrap();
        assert!(
            q.terms.len() >= 4,
            "expected at least 4 terms, got {}",
            q.terms.len()
        );
    }

    #[test]
    fn parse_multiple_fields_implicit_and() {
        let q = parse_query("fandom:Naruto tag:romance").unwrap();
        // Two free terms plus no explicit AND pushed.
        assert_eq!(q.terms.len(), 2);
    }

    #[test]
    fn parse_explicit_or() {
        let q = parse_query("tag:romance OR tag:angst").unwrap();
        // [Field("romance"), Boolean(Or), Field("angst")]
        assert_eq!(q.terms.len(), 3);
        match &q.terms[1] {
            QueryTerm::Boolean(BooleanOp::Or) => {}
            _ => panic!("expected OR operator"),
        }
    }

    #[test]
    fn parse_field_operators() {
        // >= and <=.
        let q = parse_query("kudos:>=100 replies:<=50").unwrap();
        match &q.terms[0] {
            QueryTerm::Field {
                op: Operator::GreaterEq,
                ..
            } => {}
            _ => panic!("expected >="),
        }
        match &q.terms[1] {
            QueryTerm::Field {
                op: Operator::LessEq,
                ..
            } => {}
            _ => panic!("expected <="),
        }
    }
}
