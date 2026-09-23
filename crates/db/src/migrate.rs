//! The migration runner.
//!
//! Spec §4: database-specific migrations, applied by an explicit command.
//! Spec §22: upgrades run `migrate` as their own step, and "do not assume
//! replacing the old binary reverses database migrations" — so every applied
//! migration is recorded with its checksum, and a changed checksum is a hard
//! error rather than a silent re-run.
//!
//! Design notes:
//!
//! * Each migration runs in its own transaction. A failure leaves the database
//!   on the last fully applied migration, never half-way through one.
//! * Checksums are verified on every run, so an edited migration that has
//!   already shipped cannot be applied to a second instance unnoticed.
//! * The catalogue is embedded at build time (see `build.rs`), so an operator
//!   upgrades by replacing the executable.

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::{Backend, Database};

include!(concat!(env!("OUT_DIR"), "/migration_catalogue.rs"));

/// One migration as compiled into the binary.
pub struct Migration {
    /// Numeric prefix, e.g. `0001`.
    pub version: &'static str,
    /// Human-readable name, e.g. `identity`.
    pub name: &'static str,
    /// The SQL text.
    pub sql: &'static str,
}

impl Migration {
    /// Stable identifier, `version_name`.
    #[must_use]
    pub fn id(&self) -> String {
        format!("{}_{}", self.version, self.name)
    }

    /// SHA-256 of the SQL text, hex encoded.
    #[must_use]
    pub fn checksum(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.sql.as_bytes());
        hex::encode(hasher.finalize())
    }
}

/// A migration already present in `_migrations`.
#[derive(Debug, Clone)]
pub struct AppliedMigration {
    /// The `version_name` identifier.
    pub id: String,
    /// Recorded checksum.
    pub checksum: String,
    /// When it was applied, RFC 3339.
    pub applied_at: String,
}

/// What a `migrate` run did.
#[derive(Debug, Clone)]
pub struct MigrationReport {
    /// Backend migrated.
    pub backend: Backend,
    /// Identifiers applied by this run, in order.
    pub applied: Vec<String>,
    /// Identifiers already present.
    pub already_applied: Vec<String>,
}

impl MigrationReport {
    /// Whether this run changed the schema.
    #[must_use]
    pub fn changed_schema(&self) -> bool {
        !self.applied.is_empty()
    }
}

/// The migrations compiled in for a dialect.
#[must_use]
pub const fn catalogue(backend: Backend) -> &'static [Migration] {
    match backend {
        Backend::Sqlite => SQLITE_MIGRATIONS,
        Backend::Postgres => POSTGRES_MIGRATIONS,
    }
}

/// Apply every pending migration for the database's dialect.
pub async fn apply(database: &Database) -> Result<MigrationReport> {
    ensure_ledger(database).await?;

    let recorded = applied(database).await?;
    let known = catalogue(database.backend());

    // Verify checksums before touching anything, so a mismatch aborts the run
    // instead of leaving the schema half-migrated.
    for applied_migration in &recorded {
        let Some(compiled) = known.iter().find(|m| m.id() == applied_migration.id) else {
            bail!(
                "database records migration {} which this binary does not contain; \
                 refusing to continue (was the binary downgraded?)",
                applied_migration.id
            );
        };
        if compiled.checksum() != applied_migration.checksum {
            bail!(
                "migration {} has been modified after it was applied (recorded {}, compiled {}); \
                 migrations are append-only",
                applied_migration.id,
                applied_migration.checksum,
                compiled.checksum()
            );
        }
    }

    let mut report = MigrationReport {
        backend: database.backend(),
        applied: Vec::new(),
        already_applied: recorded.iter().map(|m| m.id.clone()).collect(),
    };

    for migration in known {
        let id = migration.id();
        if recorded.iter().any(|m| m.id == id) {
            continue;
        }

        tracing::info!(migration = %id, "applying migration");
        apply_one(database, migration)
            .await
            .with_context(|| format!("applying migration {id}"))?;
        report.applied.push(id);
    }

    Ok(report)
}

/// Migrations recorded in `_migrations`, oldest first.
pub async fn applied(database: &Database) -> Result<Vec<AppliedMigration>> {
    let sql = database.sql(
        "SELECT version, name, checksum, applied_at FROM _migrations ORDER BY version ASC",
        "SELECT version, name, checksum, applied_at FROM _migrations ORDER BY version ASC",
    );

    match database.backend() {
        Backend::Sqlite => {
            let pool = database.sqlite_pool().expect("sqlite handle");
            let rows: Vec<(String, String, String, String)> = sqlx::query_as(&sql)
                .fetch_all(pool)
                .await
                .context("reading _migrations")?;
            Ok(rows
                .into_iter()
                .map(|(version, name, checksum, applied_at)| AppliedMigration {
                    id: format!("{version}_{name}"),
                    checksum,
                    applied_at,
                })
                .collect())
        }
        Backend::Postgres => {
            let pool = database.postgres_pool().expect("postgres handle");
            let rows: Vec<(String, String, String, String)> = sqlx::query_as(&sql)
                .fetch_all(pool)
                .await
                .context("reading _migrations")?;
            Ok(rows
                .into_iter()
                .map(|(version, name, checksum, applied_at)| AppliedMigration {
                    id: format!("{version}_{name}"),
                    checksum,
                    applied_at,
                })
                .collect())
        }
    }
}

/// Migrations that have not been applied yet.
///
/// Creates the ledger if it is absent, so a brand-new database reports every
/// migration as pending rather than erroring on a missing table.
pub async fn pending(database: &Database) -> Result<Vec<String>> {
    ensure_ledger(database).await?;
    let recorded = applied(database).await?;
    Ok(catalogue(database.backend())
        .iter()
        .map(Migration::id)
        .filter(|id| !recorded.iter().any(|row| &row.id == id))
        .collect())
}

/// Create the ledger table if it is absent.
async fn ensure_ledger(database: &Database) -> Result<()> {
    let sql = match database.backend() {
        Backend::Sqlite => {
            "CREATE TABLE IF NOT EXISTS _migrations (
                 version    TEXT PRIMARY KEY,
                 name       TEXT NOT NULL,
                 checksum   TEXT NOT NULL,
                 applied_at TEXT NOT NULL
             )"
        }
        Backend::Postgres => {
            "CREATE TABLE IF NOT EXISTS _migrations (
                 version    TEXT PRIMARY KEY,
                 name       TEXT NOT NULL,
                 checksum   TEXT NOT NULL,
                 applied_at TEXT NOT NULL
             )"
        }
    };

    match database.backend() {
        Backend::Sqlite => {
            sqlx::query(sql)
                .execute(database.sqlite_pool().expect("sqlite handle"))
                .await
                .context("creating _migrations")?;
        }
        Backend::Postgres => {
            sqlx::query(sql)
                .execute(database.postgres_pool().expect("postgres handle"))
                .await
                .context("creating _migrations")?;
        }
    }
    Ok(())
}

/// Apply a single migration and record it, atomically.
async fn apply_one(database: &Database, migration: &Migration) -> Result<()> {
    let now = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .context("formatting timestamp")?;
    let checksum = migration.checksum();

    match database.backend() {
        Backend::Sqlite => {
            let pool = database.sqlite_pool().expect("sqlite handle");
            let mut tx = pool.begin().await?;
            sqlx::raw_sql(migration.sql)
                .execute(&mut *tx)
                .await
                .context("executing migration SQL")?;
            sqlx::query(
                "INSERT OR IGNORE INTO _migrations (version, name, checksum, applied_at) VALUES (?, ?, ?, ?)",
            )
            .bind(migration.version)
            .bind(migration.name)
            .bind(&checksum)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .context("recording migration")?;
            tx.commit().await?;
        }
        Backend::Postgres => {
            let pool = database.postgres_pool().expect("postgres handle");
            let mut tx = pool.begin().await?;
            sqlx::raw_sql(migration.sql)
                .execute(&mut *tx)
                .await
                .context("executing migration SQL")?;
            sqlx::query(
                "INSERT INTO _migrations (version, name, checksum, applied_at) VALUES ($1, $2, $3, $4) ON CONFLICT (version) DO NOTHING",
            )
            .bind(migration.version)
            .bind(migration.name)
            .bind(&checksum)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .context("recording migration")?;
            tx.commit().await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_dialects_have_a_catalogue() {
        assert!(
            !catalogue(Backend::Sqlite).is_empty(),
            "sqlite migrations must be embedded"
        );
        assert!(
            !catalogue(Backend::Postgres).is_empty(),
            "postgres migrations must be embedded"
        );
    }

    #[test]
    fn the_two_dialects_define_the_same_migration_ids() {
        let sqlite: Vec<String> = catalogue(Backend::Sqlite)
            .iter()
            .map(Migration::id)
            .collect();
        let postgres: Vec<String> = catalogue(Backend::Postgres)
            .iter()
            .map(Migration::id)
            .collect();
        assert_eq!(
            sqlite, postgres,
            "every migration must exist for both dialects, or a dialect will drift"
        );
    }

    /// The tables a migration declares, with their columns and indexes.
    ///
    /// Compared by *name* only: the two dialects legitimately differ in the
    /// type of a column (`BIGINT` against `INTEGER`, `BOOLEAN` against
    /// `INTEGER`), and asserting the types match would forbid the thing ADR 0004
    /// requires. A column that exists in one dialect and not the other is the
    /// defect this looks for.
    type Schema = std::collections::BTreeMap<String, Vec<String>>;

    fn declared_schema(sql: &str) -> Schema {
        let stripped = strip_comments(sql);
        let sql: &str = &stripped;
        let mut schema = Schema::new();

        // CREATE TABLE <name> ( ... ) — the body may contain parentheses
        // (CHECK, UNIQUE, REFERENCES), so the closing paren is found by depth.
        let mut rest = sql;
        while let Some(at) = rest.find("CREATE TABLE") {
            let after = &rest[at + "CREATE TABLE".len()..];
            let after = after.trim_start();
            let after = after
                .strip_prefix("IF NOT EXISTS")
                .map_or(after, str::trim_start);
            let name: String = after
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            let Some(open) = after.find('(') else { break };
            let body = match balanced(&after[open..]) {
                Some(b) => b,
                None => break,
            };

            let mut columns = Vec::new();
            for item in split_top_level(body) {
                let item = item.trim();
                if item.is_empty() {
                    continue;
                }
                let keyword = item.split_whitespace().next().unwrap_or("").to_uppercase();
                // Table-level constraints are not columns.
                if matches!(
                    keyword.as_str(),
                    "UNIQUE" | "PRIMARY" | "CHECK" | "FOREIGN" | "CONSTRAINT"
                ) {
                    continue;
                }
                if let Some(col) = item.split_whitespace().next() {
                    columns.push(col.to_string());
                }
            }
            schema.insert(name, columns);
            rest = &after[open..];
        }

        // CREATE [UNIQUE] INDEX <name> ON <table> (cols)
        for line in sql.lines() {
            let line = line.trim();
            let Some(at) = line.find("CREATE ") else {
                continue;
            };
            let head = &line[at..];
            if !head.starts_with("CREATE UNIQUE INDEX") && !head.starts_with("CREATE INDEX") {
                continue;
            }
            let Some(on) = head.find(" ON ") else {
                continue;
            };
            let Some(open) = head[on..].find('(') else {
                continue;
            };
            let name = head[..on].split_whitespace().last().unwrap_or_default();
            let Some(cols) = balanced(&head[on + open..]) else {
                continue;
            };
            let cols = split_top_level(cols)
                .into_iter()
                .map(|c| c.trim().to_string())
                .collect::<Vec<_>>()
                .join(",");
            schema.insert(format!("index:{name}"), vec![cols]);
        }

        schema
    }

    /// Remove `--` line comments, leaving anything inside a quoted string.
    ///
    /// Without this the parser reads a comment as a column: the migrations
    /// document their columns inline, and `-- download …` became a column
    /// named `--` and another named `download`.
    fn strip_comments(sql: &str) -> String {
        let mut out = String::with_capacity(sql.len());
        for line in sql.lines() {
            let mut in_quote = false;
            let mut cut = line.len();
            let bytes: Vec<char> = line.chars().collect();
            let mut i = 0;
            while i < bytes.len() {
                match bytes[i] {
                    '\'' => in_quote = !in_quote,
                    '-' if !in_quote && i + 1 < bytes.len() && bytes[i + 1] == '-' => {
                        cut = line.char_indices().nth(i).map_or(line.len(), |(b, _)| b);
                        break;
                    }
                    _ => {}
                }
                i += 1;
            }
            out.push_str(&line[..cut]);
            out.push('\n');
        }
        out
    }

    /// The body inside the first balanced pair of parentheses.
    fn balanced(text: &str) -> Option<&str> {
        let mut depth = 0usize;
        let mut start = None;
        for (i, c) in text.char_indices() {
            match c {
                '(' => {
                    depth += 1;
                    if depth == 1 {
                        start = Some(i + 1);
                    }
                }
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return start.map(|s| &text[s..i]);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Split on commas that are not inside parentheses.
    fn split_top_level(text: &str) -> Vec<&str> {
        let mut parts = Vec::new();
        let mut depth = 0usize;
        let mut start = 0usize;
        for (i, c) in text.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => {
                    parts.push(&text[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
        parts.push(&text[start..]);
        parts
    }

    #[test]
    fn the_two_dialects_declare_the_same_columns_and_indexes() {
        // The id check above compares file names, and the *schema* is what the
        // code depends on. A column present in one dialect and absent in the
        // other passes that check and fails at runtime on the engine no test
        // exercises — which is how `reading_progress` came to be missing a
        // `device_id` in its unique index on PostgreSQL only.
        for (lite, pg) in catalogue(Backend::Sqlite)
            .iter()
            .zip(catalogue(Backend::Postgres))
        {
            let a = declared_schema(lite.sql);
            let b = declared_schema(pg.sql);

            let only_sqlite: Vec<&String> = a.keys().filter(|k| !b.contains_key(*k)).collect();
            let only_postgres: Vec<&String> = b.keys().filter(|k| !a.contains_key(*k)).collect();
            assert!(
                only_sqlite.is_empty() && only_postgres.is_empty(),
                "{}: tables or indexes differ — sqlite only {only_sqlite:?}, \
                 postgres only {only_postgres:?}",
                lite.id()
            );

            for (name, columns) in &a {
                // Full-text search is engine-specific by design: PostgreSQL
                // indexes a generated tsvector (search_vector) with GIN while
                // SQLite falls back to LIKE on the raw column. The index names
                // must match (checked above); their columns legitimately differ.
                const ENGINE_SPECIFIC_SEARCH_INDEXES: &[&str] = &[
                    "index:forum_topics_search_idx",
                    "index:forum_posts_search_idx",
                ];
                if ENGINE_SPECIFIC_SEARCH_INDEXES.contains(&name.as_str()) {
                    continue;
                }
                assert_eq!(
                    columns,
                    &b[name],
                    "{}: {name} declares different columns or index columns \
                     per dialect",
                    lite.id()
                );
            }
        }
    }

    #[test]
    fn the_schema_parser_sees_tables_columns_and_indexes() {
        // The check above is only as good as this parser, so it is pinned.
        let sql = "CREATE TABLE t (\n  -- a comment mentioning download\n  \
                   a BIGINT PRIMARY KEY,\n  b TEXT NOT NULL,\n  UNIQUE (a, b)\n);\n\
                   CREATE UNIQUE INDEX t_b ON t (b);";
        let schema = declared_schema(sql);
        assert_eq!(
            schema["t"],
            vec!["a", "b"],
            "table-level constraints are not columns"
        );
        assert_eq!(schema["index:t_b"], vec!["b"]);
    }

    #[test]
    fn migrations_are_ordered_and_uniquely_versioned() {
        let mut versions: Vec<&str> = catalogue(Backend::Sqlite)
            .iter()
            .map(|m| m.version)
            .collect();
        let original = versions.clone();
        versions.sort_unstable();
        versions.dedup();
        assert_eq!(versions, original, "versions must be sorted and unique");
    }

    #[test]
    fn checksums_differ_when_sql_differs() {
        let a = Migration {
            version: "0001",
            name: "a",
            sql: "SELECT 1",
        };
        let b = Migration {
            version: "0001",
            name: "a",
            sql: "SELECT 2",
        };
        assert_ne!(a.checksum(), b.checksum());
    }
}
