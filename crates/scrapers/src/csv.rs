//! CSV import: parse library export files from readers' existing platforms.
//!
//! These are *imports of reading metadata* (ratings, shelves, dates) — not
//! chapter bodies. The reader already has the book; this gives it a new home
//! in their Lorehaven library without ever touching the original source.

pub mod goodreads;
pub mod storygraph;

/// One parsed row from a CSV export.
#[derive(Debug, Clone, PartialEq)]
pub struct ShelfRow {
    pub title: String,
    pub author: String,
    pub isbn: Option<String>,
    pub my_rating: Option<u8>,
    pub average_rating: Option<f64>,
    pub shelves: Vec<String>,
    pub date_read: Option<String>,
    pub date_added: Option<String>,
    pub review: Option<String>,
}

/// A parsed import: a list of rows plus a count of skipped/invalid rows.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportShelf {
    pub rows: Vec<ShelfRow>,
    pub skipped: usize,
}

impl ImportShelf {
    pub fn new(rows: Vec<ShelfRow>, skipped: usize) -> Self {
        Self { rows, skipped }
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// Parse a shelf export in a known format.
///
/// Supported formats:
/// - `"goodreads"` — Goodreads library export CSV
/// - `"storygraph"` — StoryGraph library export CSV
pub fn import_shelf(csv: &str, format: &str) -> Result<ImportShelf, CsvError> {
    match format {
        "goodreads" => goodreads::parse(csv),
        "storygraph" => storygraph::parse(csv),
        _ => Err(CsvError::UnknownFormat(format.to_owned())),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CsvError {
    #[error("unknown CSV format: {0}")]
    UnknownFormat(String),
    #[error("invalid CSV: {0}")]
    Invalid(String),
}

/// Split a CSV line respecting quoted fields (minimal RFC 4180).
pub fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if in_quotes && chars.peek() == Some(&'"') {
                    current.push('"');
                    chars.next();
                } else {
                    in_quotes = !in_quotes;
                }
            }
            ',' if !in_quotes => {
                fields.push(current.trim().to_owned());
                current = String::new();
            }
            _ => current.push(c),
        }
    }
    fields.push(current.trim().to_owned());
    fields
}

/// Parse an optional integer from a CSV field.
pub fn opt_int<T: std::str::FromStr>(s: &str) -> Option<T> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    s.parse().ok()
}

/// Parse an optional float from a CSV field.
pub fn opt_float(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    s.parse().ok()
}

/// Split a field containing a list of tags/shelves separated by `sep`.
pub fn split_tags(s: &str, sep: char) -> Vec<String> {
    s.split(sep)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_csv_line_simple() {
        assert_eq!(split_csv_line("a,b,c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn split_csv_line_quoted() {
        assert_eq!(split_csv_line(r#""a,b","c d""#), vec!["a,b", "c d"]);
    }

    #[test]
    fn split_csv_line_escaped_quotes() {
        // The parser converts "" inside a quoted field to a single ".
        let result = split_csv_line(r#""a""b",c"#);
        assert_eq!(result, vec![String::from("a\"b"), String::from("c")]);
    }

    #[test]
    fn split_csv_line_empty_field() {
        assert_eq!(split_csv_line("a,,c"), vec!["a", "", "c"]);
    }
}
