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
    /// The file lines the parser could not read, 1-based, header included.
    ///
    /// A skipped row often has no title to name it by, so the line is its only
    /// identity — and an import that says "3 rows skipped" without them leaves
    /// the reader to search their own export by eye.
    pub skipped_lines: Vec<usize>,
}

impl ImportShelf {
    pub fn new(rows: Vec<ShelfRow>, skipped: usize) -> Self {
        Self {
            rows,
            skipped,
            skipped_lines: Vec::new(),
        }
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

// ---------------------------------------------------------------------------
// Planning: what an import will do, and what it refuses
// ---------------------------------------------------------------------------

/// A row the import can carry into the reader's library.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedItem {
    /// Stable identity for re-imports: the ISBN when the export has one,
    /// otherwise a slug of title and author.
    pub source_work_key: String,
    pub title: String,
    pub author_text: String,
    /// The reader's library state this row implies: `finished` for a row with a
    /// date read, `want-to-read` for one without.
    pub state: &'static str,
    /// The date the reader finished it, when the export says so and it parses.
    pub finished_at: Option<String>,
    /// The row's own metadata — rating, shelves, review, ISBN — as a JSON
    /// document. It is provenance: nothing queries inside it, and it survives
    /// the item being materialised into a work later.
    pub provenance_json: String,
}

/// A row the import refuses, and why.
///
/// The reason is a sentence a reader can act on, and the title names the row so
/// they can find it in the file they exported. "37 rows skipped" without either
/// is an import the reader cannot correct.
#[derive(Debug, Clone, PartialEq)]
pub struct RefusedRow {
    pub title: String,
    pub reason: String,
}

/// A planned import.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportPlan {
    pub items: Vec<PlannedItem>,
    pub refused: Vec<RefusedRow>,
}

impl ImportPlan {
    fn new() -> Self {
        Self {
            items: Vec::new(),
            refused: Vec::new(),
        }
    }
}

/// Plan what a parsed shelf export will do.
///
/// Refusals are the honest half of the acceptance criterion ("refuses rows it
/// cannot map, naming them"): a row with no title cannot be a library item, and
/// a date read that does not parse is not a date this instance will invent.
/// Both are refused by name rather than imported with a guess.
pub fn plan_shelf_import(shelf: &ImportShelf) -> ImportPlan {
    let mut plan = ImportPlan::new();
    let mut seen: Vec<String> = Vec::new();

    // Rows the parser itself could not read: named by line, because a row with
    // no title has no other identity in the file the reader is looking at.
    for line in &shelf.skipped_lines {
        plan.refused.push(RefusedRow {
            title: format!("line {line}"),
            reason: "the export gives this row no title or no author, so there is nothing to \
                     add to the library"
                .to_owned(),
        });
    }

    for row in &shelf.rows {
        let title = row.title.trim();
        if title.is_empty() {
            plan.refused.push(RefusedRow {
                title: "(untitled row)".to_owned(),
                reason: "the row has no title, so there is nothing to add to the library"
                    .to_owned(),
            });
            continue;
        }

        let finished_at = match row.date_read.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(raw) => match normalise_date(raw) {
                Some(date) => Some(date),
                None => {
                    plan.refused.push(RefusedRow {
                        title: title.to_owned(),
                        reason: format!(
                            "the date read \"{raw}\" is not a date this instance can read"
                        ),
                    });
                    continue;
                }
            },
        };

        let source_work_key = match row.isbn.as_deref().map(str::trim) {
            Some(isbn) if !isbn.is_empty() => format!("isbn:{}", isbn.to_ascii_lowercase()),
            _ => format!("title:{}", slug(&format!("{title} {}", row.author))),
        };

        // Two rows can carry the same key (a story in two editions, the same
        // ISBN twice). The last one wins rather than the import failing on a
        // unique constraint the reader cannot see.
        if let Some(index) = seen.iter().position(|key| key == &source_work_key) {
            plan.items.remove(index);
            seen.remove(index);
        }
        seen.push(source_work_key.clone());

        let state = if finished_at.is_some() {
            "finished"
        } else {
            "want-to-read"
        };
        plan.items.push(PlannedItem {
            source_work_key,
            title: title.to_owned(),
            author_text: row.author.trim().to_owned(),
            state,
            finished_at,
            provenance_json: provenance(row),
        });
    }

    plan
}

/// Normalise an exported date to RFC 3339 UTC.
///
/// Exports in the wild use `YYYY/MM/DD` (Goodreads), `YYYY-MM-DD`
/// (StoryGraph) and full timestamps; a value that is none of those is `None`,
/// which the caller turns into a refusal. Inventing a date would put a wrong
/// "finished on" in a reader's own statistics, which is worse than a refused
/// row they can fix in their export.
fn normalise_date(raw: &str) -> Option<String> {
    let raw = raw.trim();
    // A full timestamp first: it also *starts* with a `YYYY-MM-DD` shape, so
    // checking the date-only form first would silently drop the time.
    if raw.len() >= 20 && raw.contains('T') && raw.ends_with('Z') {
        return Some(raw.to_owned());
    }
    if raw.len() >= 10 {
        let (year, rest) = raw.split_at(4);
        let separator = rest.chars().next()?;
        if (separator == '-' || separator == '/') && year.chars().all(|c| c.is_ascii_digit()) {
            let rest = &rest[1..];
            let (month, day_part) = rest.split_at(2);
            let day = day_part.trim_start_matches(['-', '/']);
            let day = day.get(..2)?;
            if month.chars().all(|c| c.is_ascii_digit()) && day.chars().all(|c| c.is_ascii_digit())
            {
                let (month, day) = (month.parse::<u32>().ok()?, day.parse::<u32>().ok()?);
                if (1..=12).contains(&month) && (1..=31).contains(&day) {
                    return Some(format!("{year}-{month:02}-{day:02}T00:00:00Z"));
                }
            }
        }
    }
    None
}

/// A stable, comparison-friendly slug.
fn slug(raw: &str) -> String {
    let mut out = String::new();
    let mut last_dash = true;
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// The row's own metadata as a JSON document.
fn provenance(row: &ShelfRow) -> String {
    let shelves: Vec<serde_json::Value> = row
        .shelves
        .iter()
        .map(|shelf| serde_json::Value::String(shelf.clone()))
        .collect();
    let document = serde_json::json!({
        "imported_from_shelf_export": true,
        "isbn": row.isbn,
        "my_rating": row.my_rating,
        "average_rating": row.average_rating,
        "shelves": shelves,
        "date_added": row.date_added,
        "review": row.review,
    });
    document.to_string()
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

    fn row(title: &str, author: &str, date_read: Option<&str>, isbn: Option<&str>) -> ShelfRow {
        ShelfRow {
            title: title.to_owned(),
            author: author.to_owned(),
            isbn: isbn.map(str::to_owned),
            my_rating: Some(4),
            average_rating: Some(4.1),
            shelves: vec!["read".to_owned()],
            date_read: date_read.map(str::to_owned),
            date_added: Some("2024/01/01".to_owned()),
            review: Some("Loved it.".to_owned()),
        }
    }

    #[test]
    fn a_row_with_a_date_read_plans_a_finished_state() {
        let plan = plan_shelf_import(&ImportShelf::new(
            vec![row("A Book", "An Author", Some("2024/03/09"), None)],
            0,
        ));
        assert!(plan.refused.is_empty());
        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].state, "finished");
        assert_eq!(
            plan.items[0].finished_at.as_deref(),
            Some("2024-03-09T00:00:00Z")
        );
        // The row's own metadata survives as provenance.
        assert!(plan.items[0].provenance_json.contains("Loved it."));
        assert!(plan.items[0].provenance_json.contains("4"));
    }

    #[test]
    fn a_row_without_a_date_read_plans_want_to_read() {
        let plan = plan_shelf_import(&ImportShelf::new(
            vec![row("A Book", "An Author", Some(""), None)],
            0,
        ));
        assert_eq!(plan.items[0].state, "want-to-read");
        assert!(plan.items[0].finished_at.is_none());
    }

    #[test]
    fn a_row_with_no_title_is_refused_by_name() {
        let plan = plan_shelf_import(&ImportShelf::new(
            vec![row("  ", "An Author", None, None)],
            0,
        ));
        assert!(plan.items.is_empty());
        assert_eq!(plan.refused.len(), 1);
        assert_eq!(plan.refused[0].title, "(untitled row)");
        assert!(plan.refused[0].reason.contains("no title"));
    }

    #[test]
    fn a_row_with_an_unreadable_date_is_refused_rather_than_guessed() {
        let plan = plan_shelf_import(&ImportShelf::new(
            vec![row(
                "A Book",
                "An Author",
                Some("sometime last spring"),
                None,
            )],
            0,
        ));
        assert!(plan.items.is_empty());
        assert_eq!(plan.refused[0].title, "A Book");
        assert!(
            plan.refused[0].reason.contains("sometime last spring"),
            "{}",
            plan.refused[0].reason
        );
    }

    #[test]
    fn an_isbn_keys_the_item_and_a_title_author_slug_otherwise() {
        let with_isbn = plan_shelf_import(&ImportShelf::new(
            vec![row("A Book", "An Author", None, Some("978-0-00-000000-0"))],
            0,
        ));
        assert_eq!(with_isbn.items[0].source_work_key, "isbn:978-0-00-000000-0");
        let without = plan_shelf_import(&ImportShelf::new(
            vec![row("A Book", "An Author", None, None)],
            0,
        ));
        assert_eq!(without.items[0].source_work_key, "title:a-book-an-author");
        // The same row twice in one file plans one item, not two.
        let duplicated = plan_shelf_import(&ImportShelf::new(
            vec![
                row("A Book", "An Author", None, None),
                row("A Book", "An Author", None, None),
            ],
            0,
        ));
        assert_eq!(duplicated.items.len(), 1);
    }

    #[test]
    fn a_row_the_parser_could_not_read_is_reported_by_line() {
        let mut shelf = ImportShelf::new(vec![row("A Book", "An Author", None, None)], 1);
        shelf.skipped_lines = vec![3];
        let plan = plan_shelf_import(&shelf);
        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.refused.len(), 1);
        assert_eq!(plan.refused[0].title, "line 3");
        assert!(
            plan.refused[0].reason.contains("no title"),
            "{:?}",
            plan.refused[0]
        );
    }

    #[test]
    fn dates_in_the_shapes_exports_actually_use() {
        assert_eq!(
            normalise_date("2019/12/31").as_deref(),
            Some("2019-12-31T00:00:00Z")
        );
        assert_eq!(
            normalise_date("2019-12-31").as_deref(),
            Some("2019-12-31T00:00:00Z")
        );
        assert_eq!(
            normalise_date("2019-12-31T10:11:12Z").as_deref(),
            Some("2019-12-31T10:11:12Z")
        );
        assert_eq!(normalise_date(""), None);
        assert_eq!(normalise_date("13/45/2019"), None);
        assert_eq!(normalise_date("31 December 2019"), None);
    }
}
