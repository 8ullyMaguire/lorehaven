//! Shared test support for the milestone suites: one harness helper that is
//! backend-aware.
//!
//! By default every test gets a scratch SQLite database exactly as before.
//! Setting `LOREHAVEN_TEST_PG_URL` (an admin PostgreSQL URL, e.g. a scratch
//! container's `postgres` database) makes the same harness create a fresh
//! PostgreSQL database per test instead — so the whole milestone suite runs
//! against both dialects, which is what spec §4 asks for "from the
//! beginning". Suites seed rows with `?` placeholders; on PostgreSQL those
//! are rewritten to `$n` by [`TestDb::sql`].
//!
//! The scratch *directory* stays the harness's business in both modes:
//! export and storage tests write files there regardless of the database
//! backend.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use lorehaven_db::{Backend, Database, DatabaseConfig};

/// A per-test database. SQLite is a file inside the harness's scratch
/// directory; PostgreSQL is a scratch database created (and dropped) through
/// the admin URL.
pub struct TestDb {
    db: Database,
    applied: Vec<String>,
    pg_admin: Option<Database>,
    pg_name: Option<String>,
}

fn sanitize(tag: &str) -> String {
    let cleaned: String = tag
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    cleaned.trim_matches('_').to_string()
}

fn unique_suffix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 ^ d.as_secs())
        .unwrap_or(0)
        ^ (std::process::id() as u64) << 17
}

/// Unique scratch directory per tag (also used for export/storage files).
pub fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lorehaven-test-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

impl TestDb {
    /// Connect (and create) the scratch database for one test. SQLite writes
    /// `lorehaven.sqlite` inside `dir`; PostgreSQL creates and migrates a
    /// uniquely named scratch database.
    pub async fn connect_with_dir(tag: &str, dir: &Path) -> Self {
        match std::env::var("LOREHAVEN_TEST_PG_URL") {
            Ok(admin_url) if !admin_url.is_empty() => {
                let _ = &dir;
                let name = format!("lh_test_{}_{}", sanitize(tag), unique_suffix());
                let admin = Database::connect(&DatabaseConfig::new(admin_url.clone()))
                    .await
                    .expect("connect to the PostgreSQL admin database");
                sqlx::query(&format!("CREATE DATABASE {name}"))
                    .execute(admin.postgres_pool().expect("postgres admin pool"))
                    .await
                    .expect("create the scratch test database");
                let url = admin_url
                    .rsplit_once('/')
                    .map(|(prefix, _)| format!("{prefix}/{name}"))
                    .unwrap_or_else(|| admin_url.clone());
                let db = Database::connect(&DatabaseConfig::new(url))
                    .await
                    .expect("connect to the scratch test database");
                let report = db.migrate().await.expect("migrate");
                Self {
                    applied: report.applied,
                    db,
                    pg_admin: Some(admin),
                    pg_name: Some(name),
                }
            }
            _ => {
                let db = Database::connect(&DatabaseConfig::new(format!(
                    "sqlite://{}/lorehaven.sqlite?mode=rwc",
                    dir.display()
                )))
                .await
                .expect("connect");
                let report = db.migrate().await.expect("migrate");
                Self {
                    applied: report.applied,
                    db,
                    pg_admin: None,
                    pg_name: None,
                }
            }
        }
    }

    /// The migrations this fresh database had applied at connect time —
    /// everything, on both backends, since the database is brand new.
    pub fn applied_migrations(&self) -> &[String] {
        &self.applied
    }

    /// True when the suite is running against PostgreSQL.
    pub fn is_postgres(&self) -> bool {
        self.db.backend() == Backend::Postgres
    }

    /// The database handle for the harness's router and seed helpers.
    pub fn db(&self) -> &Database {
        &self.db
    }

    /// A query string for the active backend: SQLite takes `?` placeholders
    /// as written; PostgreSQL needs `$n`, which this rewrites.
    pub fn sql(&self, query: &str) -> String {
        match self.db.backend() {
            Backend::Postgres => lorehaven_db::rewrite_placeholders(query),
            Backend::Sqlite => query.to_string(),
        }
    }

    /// Close the database and drop the scratch PostgreSQL database if one
    /// was created.
    pub async fn cleanup(self) {
        self.db.close().await;
        if let (Some(admin), Some(name)) = (self.pg_admin, self.pg_name) {
            let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"))
                .execute(admin.postgres_pool().expect("postgres admin pool"))
                .await;
            admin.close().await;
        }
    }
}
