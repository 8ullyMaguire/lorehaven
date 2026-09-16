//! StoryGraph library export CSV parser.
//!
//! The StoryGraph CSV format is similar to Goodreads but has different column
//! names (e.g., `Title`, `Authors`, `My Rating`, `Date Read`, `Moods`, `Shelves`).
//! We use column-header detection for compatibility.

use crate::csv::{opt_float, opt_int, split_csv_line, split_tags, ImportShelf, ShelfRow};

/// Parse a StoryGraph library export CSV into shelf rows.
pub fn parse(csv: &str) -> Result<ImportShelf, crate::csv::CsvError> {
    let mut lines = csv.lines();
    let header_line = lines
        .next()
        .ok_or(crate::csv::CsvError::Invalid("empty CSV".to_owned()))?;
    let binding = split_csv_line(header_line);
    let header: Vec<&str> = binding.iter().map(|s| s.as_str()).collect();

    let idx = |name: &str| header.iter().position(|h| h.eq_ignore_ascii_case(name));
    let title_idx = idx("Title");
    let author_idx = idx("Authors").or_else(|| idx("Author"));
    let isbn_idx = idx("ISBN").or_else(|| idx("ISBN13"));
    let rating_idx = idx("My Rating");
    let avg_idx = idx("Average Rating");
    let shelves_idx = idx("Shelves");
    let moods_idx = idx("Moods");
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

        // Combine shelves and moods into a single tags list.
        let mut shelves: Vec<String> = Vec::new();
        if let Some(idx) = shelves_idx {
            if let Some(s) = fields.get(idx) {
                shelves.extend(split_tags(s, ','));
            }
        }
        if let Some(idx) = moods_idx {
            if let Some(s) = fields.get(idx) {
                shelves.extend(split_tags(s, ','));
            }
        }

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
    fn parses_basic_storygraph_export() {
        let csv = "Title,Authors,ISBN,My Rating,Average Rating,Shelves,Moods,Date Read,Date Added,Review\nPiranesi,Susanna Clarke,9781526614548,5,4.01,\"fantasy,literary\",\"atmospheric,eerie\",2024-03-10,2024-02-20,hauntingly beautiful";
        let parsed = parse(csv).expect("parse ok");
        assert_eq!(parsed.rows.len(), 1);
        let row = &parsed.rows[0];
        assert_eq!(row.title, "Piranesi");
        assert_eq!(row.author, "Susanna Clarke");
        assert_eq!(row.my_rating, Some(5));
        assert!(row.shelves.contains(&"fantasy".to_string()));
        assert!(row.shelves.contains(&"atmospheric".to_string()));
        assert_eq!(row.date_read.as_deref(), Some("2024-03-10"));
    }

    #[test]
    fn handles_alternative_column_names() {
        let csv = "Title,Author,ISBN13,My Rating\nTest Book,Test Author,1234567890123,4";
        let parsed = parse(csv).expect("parse ok");
        assert_eq!(parsed.rows.len(), 1);
        assert_eq!(parsed.rows[0].isbn.as_deref(), Some("1234567890123"));
    }

    #[test]
    fn skips_rows_missing_required_fields() {
        let csv = "Title,Authors,My Rating\n,Missing Title,4\nReal Book,Real Author,5";
        let parsed = parse(csv).expect("parse ok");
        assert_eq!(parsed.rows.len(), 1);
        assert_eq!(parsed.skipped, 1);
    }
}
