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
                "INSERT INTO _migrations (version, name, checksum, applied_at) VALUES (?, ?, ?, ?)",
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
                "INSERT INTO _migrations (version, name, checksum, applied_at) VALUES ($1, $2, $3, $4)",
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
