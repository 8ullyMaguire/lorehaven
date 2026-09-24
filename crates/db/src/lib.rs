//! Database access for Lorehaven.
//!
//! Spec §2.2 offers SQLite **or** PostgreSQL; §4 requires database-specific
//! migrations and repositories "where SQL differs", and integration tests
//! against both from the beginning. This crate therefore wraps both drivers
//! behind one [`Database`] type rather than pretending they are one database.
//!
//! Two conventions keep the repository code honest:
//!
//! 1. **DDL is per-dialect.** `migrations/sqlite/` and `migrations/postgres/`
//!    are separate files; the build script embeds both.
//! 2. **DML is written once with `?` placeholders** and rewritten to `$1…$n`
//!    for PostgreSQL by [`Database::sql`]. That rewriting is only sound because
//!    we author every statement ourselves: a `?` in the SQL text is *always* a
//!    placeholder, never something quoted inside a string literal. Do not put
//!    a literal `?` inside a DML statement.

pub mod admin;
pub mod analytics;
pub mod bounties;
pub mod browse;
pub mod category_governance;
pub mod collaboration;
pub mod community;
pub mod content;
pub mod derivative;
pub mod directory;
pub mod discovery;
pub mod economy;
pub mod engagement;
pub mod events;
pub mod exports;
pub mod external;
pub mod federation;
pub mod forum_search;
pub mod governance;
pub mod identity;
pub mod imports;
pub mod instance_theme;
pub mod jobs;
pub mod lending;
pub mod library;
pub mod longevity;
pub mod marketplace;
pub mod media;
pub mod media_resilience;
pub mod migrate;
pub mod moderation;
pub mod monetization;
pub mod narration;
pub mod notifications;
pub mod outbox;
pub mod permission;
pub mod positivity;
pub mod rating_integrity;
pub mod reading;
pub mod revisions;
pub mod roadmap;
pub mod roles;
pub mod search;
pub mod secrets;
pub mod sessions;
pub mod settings;
pub mod spoilers;
pub mod storage;
pub mod subscriptions;
pub mod taste_health;
pub mod taste_vectors;
pub mod taxonomy;
pub mod thread_modes;
pub mod translation;
pub mod typed_votes;
pub mod work_backlink;
pub mod work_discussion;
pub mod work_metrics;

use std::borrow::Cow;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{PgPool, SqlitePool};

use migrate::MigrationReport;

/// Which engine we are talking to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Embedded SQLite — the zero-configuration default.
    Sqlite,
    /// PostgreSQL — the concurrent-workload option.
    Postgres,
}

impl Backend {
    /// Stable lowercase name for logs and health output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Postgres => "postgres",
        }
    }

    /// Infer the backend from a connection URL.
    ///
    /// We refuse to guess: an unrecognised scheme is an error rather than a
    /// silent fallback, because silently connecting to the wrong database is
    /// how a development instance ends up serving production traffic.
    pub fn from_url(url: &str) -> Result<Self> {
        let scheme = url
            .split(':')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        match scheme.as_str() {
            "sqlite" => Ok(Self::Sqlite),
            "postgres" | "postgresql" => Ok(Self::Postgres),
            other => anyhow::bail!(
                "unsupported database scheme {other:?}; expected sqlite:, postgres: or postgresql:"
            ),
        }
    }
}

/// Connection settings.
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    /// Connection URL, e.g. `sqlite://./data/lorehaven.sqlite?mode=rwc`.
    pub url: String,
    /// Pool ceiling.
    pub max_connections: u32,
    /// How long to wait for a pooled connection before failing.
    pub acquire_timeout: Duration,
    /// Warn if a statement runs longer than this (0 disables the warning).
    pub slow_query_warn: Duration,
}

impl DatabaseConfig {
    /// Build a configuration from a URL with sane defaults.
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            max_connections: 5,
            acquire_timeout: Duration::from_secs(10),
            slow_query_warn: Duration::from_millis(500),
        }
    }
}

/// A connected database.
///
/// Cloning is cheap and intended: the underlying pool is reference-counted, so
/// a handle can be handed to the HTTP state while the caller keeps one for
/// administrative work (exactly what the tests and the seed command do).
pub struct Database {
    pool: Pool,
    backend: Backend,
    redacted_url: String,
}

impl Clone for Database {
    fn clone(&self) -> Self {
        Self {
            pool: match &self.pool {
                Pool::Sqlite(pool) => Pool::Sqlite(pool.clone()),
                Pool::Postgres(pool) => Pool::Postgres(pool.clone()),
            },
            backend: self.backend,
            redacted_url: self.redacted_url.clone(),
        }
    }
}

enum Pool {
    Sqlite(SqlitePool),
    Postgres(PgPool),
}

impl Database {
    /// Open a pool. For SQLite the parent directory is created and the file is
    /// created if missing; for PostgreSQL the server must already exist.
    pub async fn connect(config: &DatabaseConfig) -> Result<Self> {
        let backend = Backend::from_url(&config.url)?;

        let pool = match backend {
            Backend::Sqlite => {
                let path = sqlite_path(&config.url)?;
                if let Some(parent) = Path::new(&path).parent() {
                    if !parent.as_os_str().is_empty() {
                        tokio::fs::create_dir_all(parent).await.with_context(|| {
                            format!("creating database directory {}", parent.display())
                        })?;
                    }
                }

                let options: SqliteConnectOptions = config.url.parse().with_context(|| {
                    format!("parsing SQLite URL {url}", url = redact(&config.url))
                })?;

                // WAL keeps readers from blocking the writer, which matters for
                // a single-file deployment serving concurrent reads.
                let options = options
                    .create_if_missing(true)
                    .journal_mode(SqliteJournalMode::Wal)
                    .synchronous(SqliteSynchronous::Normal)
                    .foreign_keys(true)
                    .busy_timeout(Duration::from_secs(5));

                let pool = SqlitePoolOptions::new()
                    .max_connections(config.max_connections)
                    .acquire_timeout(config.acquire_timeout)
                    .connect_with(options)
                    .await
                    .with_context(|| format!("connecting to SQLite at {path}"))?;
                Pool::Sqlite(pool)
            }
            Backend::Postgres => {
                let options: PgConnectOptions = config
                    .url
                    .parse()
                    .with_context(|| format!("parsing PostgreSQL URL {}", redact(&config.url)))?;
                let pool = PgPoolOptions::new()
                    .max_connections(config.max_connections)
                    .acquire_timeout(config.acquire_timeout)
                    .connect_with(options)
                    .await
                    .context("connecting to PostgreSQL")?;
                Pool::Postgres(pool)
            }
        };

        let database = Self {
            pool,
            backend,
            redacted_url: redact(&config.url),
        };

        // Fail fast on a bad credential or an unreachable host, rather than
        // discovering it on the first user request.
        database.ping().await?;
        Ok(database)
    }

    /// Which engine this handle talks to.
    #[must_use]
    pub const fn backend(&self) -> Backend {
        self.backend
    }

    /// The connection URL with any password removed, safe for logs.
    #[must_use]
    pub fn redacted_url(&self) -> &str {
        &self.redacted_url
    }

    /// Select the dialect-appropriate statement and bind it to the driver.
    ///
    /// Spec §4 is explicit that the engines are *not* interchangeable, so the
    /// two statements are written separately rather than pretended into one:
    ///
    /// * the SQLite string is used verbatim;
    /// * the PostgreSQL string is emitted with `?` rewritten to `$1…$n`, which
    ///   lets it carry casts (`?::uuid`) that SQLite does not understand.
    ///
    /// Parameters are therefore bound positionally for both. The one rule:
    /// never put a literal `?` inside a statement's SQL text.
    #[must_use]
    pub fn sql<'a>(&self, sqlite: &'a str, postgres: &'a str) -> Cow<'a, str> {
        match self.backend {
            Backend::Sqlite => Cow::Borrowed(sqlite),
            Backend::Postgres => Cow::Owned(rewrite_placeholders(postgres)),
        }
    }

    /// Verify the connection is usable.
    pub async fn ping(&self) -> Result<()> {
        match &self.pool {
            Pool::Sqlite(pool) => {
                sqlx::query("SELECT 1").execute(pool).await?;
            }
            Pool::Postgres(pool) => {
                sqlx::query("SELECT 1").execute(pool).await?;
            }
        }
        Ok(())
    }

    /// Apply all pending migrations for the active dialect.
    pub async fn migrate(&self) -> Result<MigrationReport> {
        migrate::apply(self).await
    }

    /// Migrations recorded as applied.
    pub async fn applied_migrations(&self) -> Result<Vec<migrate::AppliedMigration>> {
        migrate::applied(self).await
    }

    /// The SQLite pool, when this is a SQLite handle.
    #[must_use]
    pub fn sqlite_pool(&self) -> Option<&SqlitePool> {
        match &self.pool {
            Pool::Sqlite(pool) => Some(pool),
            Pool::Postgres(_) => None,
        }
    }

    /// The PostgreSQL pool, when this is a PostgreSQL handle.
    #[must_use]
    pub fn postgres_pool(&self) -> Option<&PgPool> {
        match &self.pool {
            Pool::Postgres(pool) => Some(pool),
            Pool::Sqlite(_) => None,
        }
    }

    /// Close the pool, waiting for in-flight statements to finish.
    pub async fn close(&self) {
        match &self.pool {
            Pool::Sqlite(pool) => pool.close().await,
            Pool::Postgres(pool) => pool.close().await,
        }
    }
}

/// Select the dialect statement from two owned strings.
///
/// [`Database::sql`] borrows its arguments, so it cannot be handed a temporary
/// `format!` result; statements assembled from shared column lists use this
/// instead. The rewriting rule is the same one documented on
/// [`Database::sql`].
#[must_use]
pub fn sql_owned(db: &Database, sqlite: String, postgres: String) -> String {
    match db.backend() {
        Backend::Sqlite => sqlite,
        Backend::Postgres => rewrite_placeholders(&postgres),
    }
}

/// Rewrite `?` placeholders into PostgreSQL's `$1…$n` form.
#[must_use]
pub fn rewrite_placeholders(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len() + 8);
    let mut index = 1usize;
    for ch in sql.chars() {
        if ch == '?' {
            out.push('$');
            out.push_str(&index.to_string());
            index += 1;
        } else {
            out.push(ch);
        }
    }
    out
}

/// Extract the filesystem path from a SQLite URL.
fn sqlite_path(url: &str) -> Result<String> {
    let rest = url
        .strip_prefix("sqlite://")
        .or_else(|| url.strip_prefix("sqlite:"))
        .ok_or_else(|| anyhow::anyhow!("not a SQLite URL: {}", redact(url)))?;
    // Drop query parameters such as `?mode=rwc`.
    let path = rest.split('?').next().unwrap_or_default();
    if path.is_empty() || path == ":memory:" {
        anyhow::bail!("SQLite URL must name a file path, got {}", redact(url));
    }
    Ok(path.to_owned())
}

/// Strip credentials from a URL so it can be logged.
#[must_use]
pub fn redact(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_owned();
    };
    let (scheme, rest) = url.split_at(scheme_end + 3);
    let Some(at) = rest.find('@') else {
        return url.to_owned();
    };
    let credentials = &rest[..at];
    let user = credentials.split(':').next().unwrap_or(credentials);
    format!("{scheme}{user}:***@{}", &rest[at + 1..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_is_inferred_from_scheme() {
        assert_eq!(Backend::from_url("sqlite://a.db").unwrap(), Backend::Sqlite);
        assert_eq!(Backend::from_url("sqlite:a.db").unwrap(), Backend::Sqlite);
        assert_eq!(
            Backend::from_url("postgres://u@h/db").unwrap(),
            Backend::Postgres
        );
        assert_eq!(
            Backend::from_url("postgresql://u@h/db").unwrap(),
            Backend::Postgres
        );
    }

    #[test]
    fn unknown_scheme_is_refused_rather_than_guessed() {
        assert!(Backend::from_url("mysql://u@h/db").is_err());
        assert!(Backend::from_url("").is_err());
    }

    #[test]
    fn passwords_never_reach_the_logs() {
        assert_eq!(
            redact("postgres://lore:hunter2@db.internal:5432/lorehaven"),
            "postgres://lore:***@db.internal:5432/lorehaven"
        );
        assert_eq!(
            redact("sqlite://./data/lorehaven.sqlite"),
            "sqlite://./data/lorehaven.sqlite"
        );
    }

    #[test]
    fn placeholders_are_rewritten_for_postgres() {
        assert_eq!(
            rewrite_placeholders("INSERT INTO t (a, b) VALUES (?, ?)"),
            "INSERT INTO t (a, b) VALUES ($1, $2)"
        );
        assert_eq!(
            rewrite_placeholders("SELECT 1 FROM t WHERE id::text = ? AND n > ?"),
            "SELECT 1 FROM t WHERE id::text = $1 AND n > $2"
        );
        // The write path still carries explicit casts, which must survive the
        // rewrite attached to the renumbered placeholder.
        assert_eq!(
            rewrite_placeholders("SELECT 1 FROM t WHERE id = ?::uuid AND n > ?"),
            "SELECT 1 FROM t WHERE id = $1::uuid AND n > $2"
        );
        // Numbering restarts per statement, so a caller cannot accidentally
        // reuse a stale index.
        assert_eq!(rewrite_placeholders("SELECT ?"), "SELECT $1");
        assert_eq!(rewrite_placeholders("SELECT 1"), "SELECT 1");
    }

    #[test]
    fn sqlite_path_is_extracted_and_validated() {
        assert_eq!(
            sqlite_path("sqlite://./data/x.db?mode=rwc").unwrap(),
            "./data/x.db"
        );
        assert!(sqlite_path("postgres://h/db").is_err());
        assert!(sqlite_path("sqlite://?mode=memory").is_err());
    }
}
