//! The reachability pin: every table the known-gaps file claims is one-sided is
//! **still** one-sided, and no table has become one-sided without being claimed.
//!
//! WHY THIS EXISTS
//!
//! A table that is written and never read is a write amplification with no
//! reader: the instance pays for the row on every write and nobody ever looks at
//! it. A table that is read and never written is usually the sign of a feature
//! that was half-built and shipped without its other half. **A table that is
//! neither is a table nothing uses** — a migration with a table.
//!
//! The inherited estate has 75 such tables, and they are listed in
//! `docs/known-gaps.md` under **Unreachable tables**. This test is the pinning
//! half of that entry: it re-derives the one-sided set and asserts it is exactly
//! the listed one.
//!
//! **So the test fails in two directions, and both are the point:**
//!
//!   * a listed table that has since been given a reader or a writer — the gap
//!     is fixed, and the entry has to go, or the file claims a defect that no
//!     longer exists;
//!   * a table that has become one-sided and is not listed — a new gap, and the
//!     file has to claim it.
//!
//! A pin that only ran one way would let the file rot in the direction nobody
//! was looking at.
//!
//! **The scan is written twice — once here, once in `scripts/reachability.py`
//! — and the two are independent.** That is deliberate: two scans that must
//! agree are a cross-check, and a scan that has been written once and never
//! compared with anything is a scan nobody has tested.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The state a table is claimed to be in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    /// Written, never read.
    WrittenOnly,
    /// Read, never written.
    ReadOnly,
    /// Neither read nor written.
    Neither,
}

impl Side {
    fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "written, never read" => Some(Side::WrittenOnly),
            "read, never written" => Some(Side::ReadOnly),
            "neither" => Some(Side::Neither),
            _ => None,
        }
    }
}

/// The tables the known-gaps file claims, and the state it claims for each.
fn known_gap_tables(path: &Path) -> BTreeMap<String, Side> {
    let mut claimed = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return claimed;
    };
    let mut inside = false;
    for line in text.lines() {
        if line.starts_with("## ") {
            inside = line.trim() == "## Unreachable tables";
            continue;
        }
        if !inside {
            continue;
        }
        let stripped = line.trim();
        if !stripped.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = stripped
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        if cells.len() < 2 {
            continue;
        }
        let name = cells[0].trim().trim_matches('`').trim();
        if name.is_empty()
            || name == "table"
            || name.chars().all(|c| c == '-' || c == ':' || c == ' ')
        {
            continue;
        }
        if let Some(side) = Side::parse(cells[1]) {
            claimed.insert(name.to_ascii_lowercase(), side);
        }
    }
    claimed
}

/// Every table the migrations create.
fn schema_tables(migrations: &Path) -> BTreeSet<String> {
    let mut tables = BTreeSet::new();
    for dialect in ["sqlite", "postgres"] {
        let directory = migrations.join(dialect);
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "sql") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            for line in text.lines() {
                let upper = line.trim_start().to_ascii_uppercase();
                let Some(rest) = upper.strip_prefix("CREATE TABLE ") else {
                    continue;
                };
                let rest = rest
                    .strip_prefix("IF NOT EXISTS ")
                    .unwrap_or(rest)
                    .trim_start_matches(['"', '`', '[']);
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    tables.insert(name.to_ascii_lowercase());
                }
            }
        }
    }
    tables
}

/// Every identifier that follows `prefix` in `text`.
///
/// **All** of them, not the first: the first `from ` in a file is often the words
/// "from a sentence", and stopping there would report every table as untouched.
/// The scan therefore collects every occurrence and the caller filters.
fn identifiers_after(text: &str, prefix: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let lowered = text.to_ascii_lowercase();
    let mut from = 0usize;
    while let Some(found) = lowered[from..].find(prefix) {
        let start = from + found + prefix.len();
        let rest = &text[start..];
        let rest = rest.trim_start_matches(['"', '`', '[']);
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            names.insert(name.to_ascii_lowercase());
        }
        from = from + found + prefix.len();
    }
    names
}

/// Every Rust file under a crate's `src/`, and nothing else.
///
/// `src/` only: the integration tests live in `crates/*/tests/`, and a test
/// fixture is not a use of a table.
fn source_files(crates: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(crates) else {
        return found;
    };
    for entry in entries.flatten() {
        let crate_dir = entry.path();
        if !crate_dir.is_dir() {
            continue;
        }
        let src = crate_dir.join("src");
        if !src.is_dir() {
            continue;
        }
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            let Ok(children) = std::fs::read_dir(&dir) else {
                continue;
            };
            for child in children.flatten() {
                let path = child.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    found.push(path);
                }
            }
        }
    }
    found.sort();
    found
}

/// The tables read and the tables written, by the production sources.
fn scan(sources: &[PathBuf], tables: &BTreeSet<String>) -> (BTreeSet<String>, BTreeSet<String>) {
    const WRITES: [&str; 4] = ["insert into ", "update ", "delete from ", "replace into "];
    const READS: [&str; 2] = ["from ", "join "];

    let mut readers: BTreeSet<String> = BTreeSet::new();
    let mut writers: BTreeSet<String> = BTreeSet::new();

    for path in sources {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        // A `#[cfg(test)]` module inside a `src/` file is a test fixture: a row
        // it inserts to read back proves the schema works, not that the
        // instance uses the table.
        let text = match text.find("#[cfg(test)]") {
            Some(at) => &text[..at],
            None => text.as_str(),
        };
        for prefix in WRITES {
            for name in identifiers_after(text, prefix) {
                if tables.contains(&name) {
                    writers.insert(name);
                }
            }
        }
        for prefix in READS {
            for name in identifiers_after(text, prefix) {
                if tables.contains(&name) {
                    readers.insert(name);
                }
            }
        }
    }
    (readers, writers)
}

#[test]
fn the_unreachable_tables_are_exactly_the_ones_the_known_gaps_file_claims() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let gaps = repo.join("docs/known-gaps.md");
    let claimed = known_gap_tables(&gaps);
    assert!(
        !claimed.is_empty(),
        "the known-gaps file has no Unreachable tables section, so this test is \
         measuring nothing: {}",
        gaps.display()
    );

    let tables = schema_tables(&repo.join("migrations"));
    assert!(
        tables.len() > 50,
        "only {} table(s) were read from the migrations, so this test is measuring nothing",
        tables.len()
    );

    let (readers, writers) = scan(&source_files(&repo.join("crates")), &tables);

    // --- a claimed table that has since been given the other side -----------
    let mut fixed: Vec<String> = Vec::new();
    for (name, side) in &claimed {
        let has_reader = readers.contains(name);
        let has_writer = writers.contains(name);
        let still_one_sided = match side {
            Side::WrittenOnly => has_writer && !has_reader,
            Side::ReadOnly => has_reader && !has_writer,
            Side::Neither => !has_reader && !has_writer,
        };
        if !still_one_sided {
            fixed.push(format!(
                "{name} is claimed as {side:?} but it now has a reader={has_reader}, a writer={has_writer}"
            ));
        }
    }
    assert!(
        fixed.is_empty(),
        "the known-gaps file claims a defect that no longer exists:\n  {}\n\
         Remove the row, or fix the claim.",
        fixed.join("\n  ")
    );

    // --- a table that has become one-sided without being claimed ----------
    let mut unclaimed: Vec<String> = Vec::new();
    for table in &tables {
        if claimed.contains_key(table) {
            continue;
        }
        let has_reader = readers.contains(table);
        let has_writer = writers.contains(table);
        if has_writer != has_reader {
            unclaimed.push(table.clone());
        }
    }
    assert!(
        unclaimed.is_empty(),
        "{} table(s) have one side and not the other and are NOT listed under \
         \"Unreachable tables\" in {}:\n  {}\n\
         Either give the table a reader and a writer, or list it with the reason.",
        unclaimed.len(),
        gaps.display(),
        unclaimed.join("\n  ")
    );
}
