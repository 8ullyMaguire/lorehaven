//! M2-08: the settings stay **per-domain**.
//!
//! # What this asserts
//!
//! A reader's settings are **six separate domains, each with its own table** —
//! privacy, content, search, filters, notifications — and **never one JSONB
//! blob**.
//!
//! The blob is the tempting shape: one `settings` column, one JSON document,
//! one row per account. It is also the shape that loses every query. A blob
//! cannot be indexed, cannot be filtered in SQL, and cannot be migrated — so
//! "show me every reader who has muted the `underage sex` warning" becomes a
//! scan of every account's JSON in the application, and the answer depends on
//! the application rather than on the database.
//!
//! # What it would have caught
//!
//! A settings table whose column is a JSON/JSONB **bag** — a column named
//! `settings`, `config`, `options`, `preferences` — is the blob coming back, one
//! domain at a time. And a settings table with **no reader or no writer** is a
//! table nothing uses, which the reachability probe (M1-08) would also name, but
//! only by accident of what it happens to scan.
//!
//! **The names are the point.** A JSONB column called `value` in a key/value
//! table is a different thing from a JSONB column called `settings`: the first
//! is one value the reader chose, and the second is a bag of settings that
//! happened to be split out of one bigger bag. The test is about the second.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The settings domains, and the table that holds each.
///
/// **The domain is the unit, not the table.** The notification domain's
/// per-account settings live in `notification_routes` — one row per event type
/// and channel — because "which events do I want, and where" is itself a set of
/// rows and not a document. The card's list named it `notification_preferences`;
/// the *domain* is what matters and the domain's table is `notification_routes`.
const SETTINGS_TABLES: &[&str] = &[
    "privacy_settings",
    "content_settings",
    "search_settings",
    "content_filters",
    "notification_routes",
];

/// The column names that would make a JSON column a **bag** rather than a value.
const BAG_NAMES: &[&str] = &[
    "settings",
    "config",
    "configuration",
    "options",
    "preferences",
    "bag",
    "blob",
    "payload",
    "document",
    "properties",
];

fn migrations_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/db lives two levels under the workspace root")
        .join("migrations")
}

/// Every `CREATE TABLE` block in a dialect, as `(table, [column lines])`.
fn tables(dialect: &str) -> Vec<(String, Vec<String>)> {
    let dir = migrations_root().join(dialect);
    let mut found = Vec::new();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("read the migrations directory")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "sql"))
        .collect();
    entries.sort();

    for path in entries {
        let text = std::fs::read_to_string(&path).expect("read a migration");
        let mut lines = text.lines().peekable();
        while let Some(line) = lines.next() {
            let trimmed = line.trim();
            let Some(rest) = trimmed.strip_prefix("CREATE TABLE") else {
                continue;
            };
            // `IF NOT EXISTS` comes before the name, and the name is what the
            // test is about.
            let rest = rest
                .trim()
                .strip_prefix("IF NOT EXISTS")
                .unwrap_or(rest.trim());
            let name: String = rest
                .trim()
                .split(['(', ' '])
                .next()
                .unwrap_or_default()
                .trim_matches('"')
                .to_owned();
            if name.is_empty() {
                continue;
            }
            let mut columns = Vec::new();
            let mut depth = 1usize;
            for inner in lines.by_ref() {
                let inner_trimmed = inner.trim();
                depth += inner_trimmed.matches('(').count();
                depth = depth.saturating_sub(inner_trimmed.matches(')').count());
                if depth == 0 {
                    break;
                }
                // A column line: a name, then a type.
                let mut parts = inner_trimmed.split_whitespace();
                let Some(column) = parts.next() else { continue };
                let Some(kind) = parts.next() else { continue };
                if column.to_uppercase().starts_with("CHECK")
                    || column.to_uppercase().starts_with("UNIQUE")
                    || column.to_uppercase().starts_with("PRIMARY")
                    || column.to_uppercase().starts_with("FOREIGN")
                    || column.to_uppercase().starts_with("CONSTRAINT")
                {
                    continue;
                }
                columns.push(format!("{column} {kind}").to_lowercase());
            }
            found.push((name.to_lowercase(), columns));
        }
    }
    found
}

#[test]
fn every_settings_domain_has_a_table_of_its_own() {
    for dialect in ["sqlite", "postgres"] {
        let names: BTreeSet<String> = tables(dialect).into_iter().map(|(n, _)| n).collect();
        for table in SETTINGS_TABLES {
            assert!(
                names.contains(*table),
                "{dialect}: the {table} domain has no table of its own -- the whole \
                 point is that the domains are separate"
            );
        }
    }
}

#[test]
fn no_settings_table_carries_a_json_bag() {
    // The type is read from the **PostgreSQL** migrations, because SQLite has no
    // JSON type at all: every column there is TEXT, so a SQLite scan could
    // never see a bag.
    for (table, columns) in tables("postgres") {
        if !SETTINGS_TABLES.contains(&table.as_str()) {
            continue;
        }
        for column in columns {
            let is_json = column.contains(" json")
                || column.contains(" jsonb")
                || column.ends_with(" json")
                || column.ends_with(" jsonb");
            if !is_json {
                continue;
            }
            let name = column.split_whitespace().next().unwrap_or_default();
            assert!(
                !BAG_NAMES.contains(&name),
                "the {table} table has a JSON column called {name:?} -- a bag of \
                 settings is the shape this rule exists to refuse, and a bag cannot \
                 be indexed, filtered in SQL, or migrated"
            );
        }
    }
}

#[test]
fn the_two_dialects_agree_on_the_settings_columns() {
    // The same rule, the same columns: a settings table that differs between
    // the engines is a settings table that works on one of them.
    let sqlite: BTreeSet<(String, String)> = tables("sqlite")
        .into_iter()
        .filter(|(name, _)| SETTINGS_TABLES.contains(&name.as_str()))
        .flat_map(|(name, columns)| {
            columns.into_iter().map(move |column| {
                (
                    name.clone(),
                    column
                        .split_whitespace()
                        .next()
                        .unwrap_or_default()
                        .to_owned(),
                )
            })
        })
        .collect();
    let postgres: BTreeSet<(String, String)> = tables("postgres")
        .into_iter()
        .filter(|(name, _)| SETTINGS_TABLES.contains(&name.as_str()))
        .flat_map(|(name, columns)| {
            columns.into_iter().map(move |column| {
                (
                    name.clone(),
                    column
                        .split_whitespace()
                        .next()
                        .unwrap_or_default()
                        .to_owned(),
                )
            })
        })
        .collect();
    assert_eq!(
        sqlite, postgres,
        "the settings tables declare different columns per dialect"
    );
    assert!(
        !sqlite.is_empty(),
        "no settings columns were read at all, so the comparison is vacuous"
    );
}

#[test]
fn every_settings_table_has_a_reader_and_a_writer() {
    // **A settings table nothing reads is a table nobody configured.** And one
    // nothing writes is a table with a default and no way to change it.
    //
    // The scan is over the production sources only — a test that writes a row
    // to read it back proves the schema works, not that the instance uses it.
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/db lives two levels under the workspace root")
        .to_path_buf();

    for table in SETTINGS_TABLES {
        let mut read = false;
        let mut written = false;
        for crate_dir in ["app", "db"] {
            let src = repo.join("crates").join(crate_dir).join("src");
            let Ok(entries) = std::fs::read_dir(&src) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() || path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).expect("read a source file");
                // A `#[cfg(test)]` module is a test fixture, not a use.
                let text = match text.find("#[cfg(test)]") {
                    Some(at) => &text[..at],
                    None => text.as_str(),
                };
                // The needles are upper-cased because the line is: a
                // lower-case needle against an upper-cased line never matches,
                // and a check that never matches passes.
                let read_needle = format!("FROM {}", table.to_ascii_uppercase());
                let write_needles = [
                    format!("INTO {}", table.to_ascii_uppercase()),
                    format!("UPDATE {}", table.to_ascii_uppercase()),
                    format!("DELETE FROM {}", table.to_ascii_uppercase()),
                ];
                for line in text.lines() {
                    let upper = line.to_ascii_uppercase();
                    if upper.contains(&read_needle) {
                        read = true;
                    }
                    if write_needles.iter().any(|needle| upper.contains(needle)) {
                        written = true;
                    }
                }
            }
        }
        assert!(
            read,
            "the {table} table has no reader in the production sources"
        );
        assert!(
            written,
            "the {table} table has no writer in the production sources"
        );
    }
}
