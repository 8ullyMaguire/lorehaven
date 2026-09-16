//! Goodreads library export CSV parser.
//!
//! Reads the CSV file Goodreads lets you download from your library. The
//! parser is tolerant of minor format changes: column order is detected from
//! the header row, so older exports still work as long as the key columns
//! (`Title`, `Author`, `My Rating`) are present.

use crate::csv::{opt_float, opt_int, split_csv_line, split_tags, ImportShelf, ShelfRow};

/// Parse a Goodreads library export CSV into shelf rows.
pub fn parse(csv: &str) -> Result<ImportShelf, crate::csv::CsvError> {
    let mut lines = csv.lines();
    let header_line = lines
        .next()
        .ok_or(crate::csv::CsvError::Invalid("empty CSV".to_owned()))?;
    let binding = split_csv_line(header_line);
    let header: Vec<&str> = binding.iter().map(|s| s.as_str()).collect();

    // Detect columns we care about by index.
    let idx = |name: &str| header.iter().position(|h| h.eq_ignore_ascii_case(name));
    let title_idx = idx("Title");
    let author_idx = idx("Author");
    let isbn_idx = idx("ISBN");
    let rating_idx = idx("My Rating");
    let avg_idx = idx("Average Rating");
    let shelves_idx = idx("Shelves");
    let date_read_idx = idx("Date Read");
    let date_added_idx = idx("Date Added");
    let review_idx = idx("Review");

    let mut rows = Vec::new();
    let mut skipped = 0;
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let fields = split_csv_line(line);

        let title = title_idx
            .and_then(|i| fields.get(i))
            .filter(|s| !s.is_empty())
            .cloned();
        let author = author_idx
            .and_then(|i| fields.get(i))
            .filter(|s| !s.is_empty())
            .cloned();

        let (title, author) = match (title, author) {
            (Some(t), Some(a)) => (t, a),
            _ => {
                skipped += 1;
                continue;
            }
        };

        let isbn = isbn_idx.and_then(|i| fields.get(i)).cloned();
        let my_rating = rating_idx
            .and_then(|i| fields.get(i))
            .and_then(|s| opt_int::<u8>(s));
        let average_rating = avg_idx
            .and_then(|i| fields.get(i))
            .and_then(|s| opt_float(s));
        let shelves = shelves_idx.map_or(Vec::new(), |i| {
            fields
                .get(i)
                .map(|s| split_tags(s, ';'))
                .unwrap_or_default()
        });
        let date_read = date_read_idx.and_then(|i| fields.get(i)).cloned();
        let date_added = date_added_idx.and_then(|i| fields.get(i)).cloned();
        let review = review_idx.and_then(|i| fields.get(i)).cloned();

        rows.push(ShelfRow {
            title,
            author,
            isbn,
            my_rating,
            average_rating,
            shelves,
            date_read,
            date_added,
            review,
        });
    }

    Ok(ImportShelf::new(rows, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_goodreads_export() {
        let csv = "Title,Author,ISBN,My Rating,Average Rating,Shelves,Date Read,Date Added,Review\nThe Left Hand of Darkness,Ursula K. Le Guin,9780060500249,5,3.94,sci-fi;classic,2024-01-15,2024-01-01,loved it";
        let parsed = parse(csv).expect("parse ok");
        assert_eq!(parsed.rows.len(), 1);
        let row = &parsed.rows[0];
        assert_eq!(row.title, "The Left Hand of Darkness");
        assert_eq!(row.author, "Ursula K. Le Guin");
        assert_eq!(row.my_rating, Some(5));
        assert_eq!(row.shelves, vec!["sci-fi", "classic"]);
        assert_eq!(row.date_read.as_deref(), Some("2024-01-15"));
    }

    #[test]
    fn handles_missing_optional_fields() {
        let csv = "Title,Author,My Rating\nTest Book,Test Author,";
        let parsed = parse(csv).expect("parse ok");
        assert_eq!(parsed.rows.len(), 1);
        assert_eq!(parsed.rows[0].my_rating, None);
        assert!(parsed.rows[0].shelves.is_empty());
    }

    #[test]
    fn skips_rows_missing_required_fields() {
        let csv = "Title,Author,My Rating\n,Missing Title,4\nReal Book,Real Author,5";
        let parsed = parse(csv).expect("parse ok");
        assert_eq!(parsed.rows.len(), 1);
        assert_eq!(parsed.skipped, 1);
    }

    #[test]
    fn handles_empty_csv() {
        let result = parse("");
        assert!(result.is_err());
    }

    #[test]
    fn handles_only_header() {
        let csv = "Title,Author,My Rating\n";
        let parsed = parse(csv).expect("parse ok");
        assert!(parsed.is_empty());
    }
}
