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
    // The store's own list, so the assertion and the code that reads the
    // columns cannot drift: a column added to one and not the other is the
    // failure, and this is what makes it a failure rather than a comment.
    use crate::preservation;
    use std::collections::BTreeSet;

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

    // Known foreign-key divergences, all in one place on purpose.
    //
    // PostgreSQL declares these REFERENCES; SQLite does not. They
    // are listed rather than fixed because closing the gap means a
    // 12-step table rebuild per table -- which needs
    // `PRAGMA foreign_keys = OFF`, a no-op inside the transaction
    // migrate() runs each migration in -- so it is a decision
    // about the migration runner, not about this test. A name here
    // means "audited, understood, still divergent"; a new one
    // fails the test. payment_events is how this list was found:
    // milestone_15 invented three account uuids and PostgreSQL
    // answered 23503 where SQLite answered nothing.
    //
    // `works_index`, `works_index_terms` and `topic_work_links` were all here
    // and are not any more. Each is a table the PostgreSQL migrations gave a
    // foreign key and the SQLite twin silently omitted, and each was found by
    // writing the suite for a module that had none:
    //
    //   works_index, works_index_terms -- migration 0011; a deleted work left
    //     its index rows behind on SQLite, and the terms of a removed work
    //     stayed searchable. See `deleting_a_work_cascades_its_index` in
    //     crates/app/tests/milestone_15_search.rs.
    //   topic_work_links -- migration 0038; an orphan link was insertable on
    //     SQLite and impossible on PostgreSQL. See
    //     `a_link_to_a_missing_work_is_refused` in
    //     crates/app/tests/milestone_38_work_backlink.rs.
    //
    // All three are fixed in the original migration rather than a forward one,
    // since each predates every release tag. This list is the check that finds
    // them: an entry naming a divergence that no longer exists fails on purpose.
    //
    // `topic_work_links` in particular: migration 0038 created
    // it on SQLite without the two foreign keys it declares on PostgreSQL, so
    // an orphan link was insertable on one engine and impossible on the other.
    // 0038 is fixed in place -- it predates every release tag, so no deployed
    // instance has a recorded checksum to contradict. Found by the backlink
    // suite, which had no tests until then: see
    // `a_link_to_a_missing_work_is_refused` in
    // crates/app/tests/milestone_38_work_backlink.rs.
    //

    pub(crate) const KNOWN_FK_DIVERGENCES: &[&str] = &[
        "fk:availability_links",
        "fk:curator_rewards",
        "fk:link_health_checks",
        "fk:payment_events",
        "fk:rating_anomaly_events",
        "fk:work_media_references",
        "fk:work_reactions",
        "fk:work_tags",
    ];

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
            let mut foreign_keys: Vec<String> = Vec::new();
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
                    // A column carrying `REFERENCES <table>` is a foreign key.
                    // Recording it is the point: 13 tables used to declare
                    // REFERENCES in one dialect and not the other, and
                    // `payment_events` is how that was found -- a test
                    // inserting invented uuids was accepted on SQLite and
                    // rejected with 23503 on PostgreSQL. The column-name and
                    // index-name checks above cannot see this, because both
                    // dialects declare the same column either way.
                    if let Some(at) = item.to_uppercase().find("REFERENCES") {
                        let target = item[at + "REFERENCES".len()..]
                            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                            .find(|t| !t.is_empty())
                            .unwrap_or("?")
                            .to_string();
                        foreign_keys.push(format!("{col}->{target}"));
                    }
                    columns.push(col.to_string());
                }
            }
            // Sorted so the comparison below is order-independent.
            foreign_keys.sort();
            schema.insert(format!("fk:{name}"), foreign_keys);
            schema.insert(name, columns);
            rest = &after[open..];
        }

        // ALTER TABLE <name> ADD CONSTRAINT ... FOREIGN KEY (cols) REFERENCES t
        //
        // A circular reference cannot be declared inline, because the target
        // does not exist yet. `chapters.current_revision_id` is the one case:
        // `chapters` and `chapter_revisions` point at each other, so the
        // PostgreSQL file creates both and adds the constraint afterwards,
        // while SQLite resolves targets lazily and keeps it inline. Reading
        // only the inline form would report that documented difference as a
        // defect.
        let lines: Vec<&str> = sql.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !line.trim().to_uppercase().starts_with("FOREIGN KEY") {
                continue;
            }
            // The owning table is the nearest preceding ALTER TABLE.
            let owner = lines[..i]
                .iter()
                .rev()
                .map(|l| l.trim())
                .find(|l| l.to_uppercase().starts_with("ALTER TABLE"))
                .map(|l| l["ALTER TABLE".len()..].trim().to_string());
            let Some(owner) = owner else { continue };

            let Some(open) = line.find('(') else {
                continue;
            };
            let Some(close) = line[open..].find(')') else {
                continue;
            };
            let Some(at) = line.to_uppercase().find("REFERENCES") else {
                continue;
            };
            let target = line[at + "REFERENCES".len()..]
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .find(|t| !t.is_empty())
                .unwrap_or("?")
                .to_string();

            let added: Vec<String> = line[open + 1..open + close]
                .split(',')
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .map(|c| format!("{c}->{target}"))
                .collect();
            if let Some(entry) = schema.get_mut(&format!("fk:{owner}")) {
                entry.extend(added);
                entry.sort();
            }
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
                if KNOWN_FK_DIVERGENCES.contains(&name.as_str()) {
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

    /// The gap in the parity test above, and the reason this test has to exist.
    ///
    /// `the_two_dialects_declare_the_same_columns_and_indexes` reads `CREATE
    /// TABLE` and `CREATE INDEX` only, so a column brought in by
    /// `ALTER TABLE ... ADD COLUMN` is **invisible to it** — one migration can
    /// add a column the other does not, every existing test passes, and the two
    /// engines then disagree at runtime. Migration 0093 adds nine columns that
    /// way, so the gap is live rather than hypothetical.
    ///
    /// It was live for exactly one run: the first PostgreSQL run of Phase D
    /// died in `migrate()` with `foreign key constraint
    /// "story_identity_members_created_by_fkey" cannot be implemented`, because
    /// `accounts.id` is UUID on PostgreSQL and the column was declared TEXT. The
    /// SQLite twin accepted it, because SQLite is dynamically typed — which is
    /// why the two files read as identical in review and are not.
    ///
    /// So this asserts the column SET of the added columns against the store's
    /// own list, by reading both migration files. It does **not** assert the
    /// column TYPES, and it cannot: `created_by` is deliberately TEXT in one
    /// dialect and UUID in the other, because it references a column that is
    /// TEXT in one dialect and UUID in the other. Asserting type equality here
    /// would fail on a correct pair of files; the type divergence is carried by
    /// the casts in the store instead, and the engine is what settles it.
    #[test]
    fn a_preservation_member_row_carries_the_columns_both_dialects_declare() {
        for backend in [Backend::Sqlite, Backend::Postgres] {
            let declared = added_columns_for_table(backend, "story_identity_members");
            let expected: BTreeSet<String> = preservation::PRESERVATION_MEMBER_COLUMNS
                .iter()
                .map(|name| (*name).to_owned())
                .collect();
            assert_eq!(
                declared, expected,
                "{backend:?} 0093 adds a different set of preservation columns to \
                 story_identity_members than PRESERVATION_MEMBER_COLUMNS lists; the store \
                 reads all of them and a column missing from one dialect is a statement \
                 that fails only on the engine it is missing from"
            );
        }
    }

    /// The `ADD COLUMN` names in one dialect's 0093 for one table.
    ///
    /// Narrow by table on purpose: the migration also adds a column to `works`,
    /// and a scan that did not bound itself by table would return both sets and
    /// compare them as one.
    fn added_columns_for_table(backend: Backend, table: &str) -> BTreeSet<String> {
        let migration = catalogue(backend)
            .iter()
            .find(|m| m.id().starts_with("0093"))
            .expect("migration 0093 exists in both dialects");
        let mut found = BTreeSet::new();
        for line in strip_comments(migration.sql).lines() {
            let trimmed = line.trim();
            let Some(rest) = trimmed.strip_prefix("ALTER TABLE") else {
                continue;
            };
            let rest = rest.trim();
            let Some(after) = rest.strip_prefix(table) else {
                continue;
            };
            let after = after.trim_start();
            if !after.starts_with("ADD COLUMN") {
                continue;
            }
            let name: String = after["ADD COLUMN".len()..]
                .trim()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                found.insert(name);
            }
        }
        found
    }

    #[test]
    fn the_fk_allowlist_matches_the_divergence_exactly() {
        // The allowlist above is only honest if it is neither empty nor a
        // blanket. Recompute the divergence from the migrations and compare it
        // to the list, so an entry that is later fixed fails (a stale allowance
        // hides a fixed bug) and a new divergence fails (a missing one is the
        // case that bit payment_events).
        let diverge = |backend: Backend| -> BTreeSet<String> {
            let sql: String = catalogue(backend)
                .iter()
                .map(|m| m.sql)
                .collect::<Vec<_>>()
                .join("\n");
            declared_schema(&sql)
                .iter()
                .filter(|(k, v)| k.starts_with("fk:") && !v.is_empty())
                .map(|(k, v)| format!("{k}={}", v.join(",")))
                .collect()
        };
        let mut pg = diverge(Backend::Postgres);
        let lite = diverge(Backend::Sqlite);
        pg.retain(|e| !lite.contains(e));

        let listed: BTreeSet<String> = KNOWN_FK_DIVERGENCES
            .iter()
            .map(|name| {
                let sql: String = catalogue(Backend::Postgres)
                    .iter()
                    .map(|m| m.sql)
                    .collect::<Vec<_>>()
                    .join("\n");
                let entry = declared_schema(&sql)
                    .get(*name)
                    .cloned()
                    .unwrap_or_default();
                format!("{name}={}", entry.join(","))
            })
            .collect();

        assert_eq!(
            pg, listed,
            "KNOWN_FK_DIVERGENCES must equal the actual set of per-dialect \
             foreign-key differences"
        );
    }

    #[test]
    fn the_schema_parser_sees_inline_and_altered_foreign_keys() {
        // Both spellings, because a circular reference can only be expressed
        // with ALTER and the two dialects disagree about which to use. A
        // REFERENCES hiding inside a `-- comment` must not be picked up --
        // the pinned case is a comment that mentions "download".
        let sql = "CREATE TABLE t (\n  a UUID PRIMARY KEY,\n  b TEXT\n);\n\
                   CREATE TABLE u (\n  t_id UUID REFERENCES t (a),\n\
                   -- c UUID REFERENCES t (a)\n  c TEXT\n);\n\
                   ALTER TABLE t\n    ADD CONSTRAINT t_b_fk\n    FOREIGN KEY (b) REFERENCES s (id) ON DELETE SET NULL;";
        let schema = declared_schema(sql);
        assert_eq!(schema["fk:u"], vec!["t_id->t"], "inline REFERENCES");
        assert_eq!(schema["fk:t"], vec!["b->s"], "FOREIGN KEY via ALTER TABLE");
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
