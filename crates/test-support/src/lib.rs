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

/// Sweep scratch databases nobody is connected to. Once per process.
///
/// A fixture drops its own database in [`TestDb::cleanup`], so a *failing* test
/// leaks one and the container accumulates `lh_test_*` databases until
/// `/dev/shm` runs out and every later run reports zero passes — the failure
/// looks like the suite, not like the leftover.
///
/// Liveness, not age, is the criterion. `pg_database` has no creation
/// timestamp, and a name cannot carry one because the suffix is a time/pid
/// hash; what *is* authoritative is whether anyone is attached. A database with
/// no backends belongs to no run: either its test finished and failed before
/// `cleanup`, or its process died, and in both cases nothing will ever drop it.
/// A database another test binary is using has a live backend and is left
/// alone, which is the property that makes this safe to do concurrently.
///
/// Best effort by design: a sweep that fails must not fail the suite it is
/// cleaning up after, so every error here is logged and swallowed.
async fn sweep_idle_scratch_databases(admin: &Database) {
    use std::sync::Once;
    static SWEPT: Once = Once::new();
    let mut sweep = false;
    SWEPT.call_once(|| sweep = true);
    if !sweep {
        return;
    }

    let Some(pool) = admin.postgres_pool() else {
        return;
    };
    let idle: Vec<String> = match sqlx::query_scalar(
        "SELECT d.datname FROM pg_database d
         WHERE d.datname LIKE 'lh\\_test\\_%'
           AND NOT EXISTS (
                 SELECT 1 FROM pg_stat_activity a WHERE a.datid = d.oid
           )
         ORDER BY d.datname
         LIMIT 50",
    )
    .fetch_all(pool)
    .await
    {
        Ok(names) => names,
        Err(error) => {
            eprintln!("test-support: could not list leaked scratch databases: {error}");
            return;
        }
    };

    for name in idle {
        match sqlx::query(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"))
            .execute(pool)
            .await
        {
            Ok(_) => eprintln!("test-support: dropped the leaked scratch database {name}"),
            Err(error) => {
                eprintln!(
                    "test-support: could not drop the leaked scratch database {name}: {error}"
                )
            }
        }
    }
}

impl TestDb {
    /// Connect (and create) the scratch database for one test. SQLite writes
    /// `lorehaven.sqlite` inside `dir`; PostgreSQL creates and migrates a
    /// uniquely named scratch database.
    ///
    /// One test gets exactly one of these. That matters more than it looks:
    /// under SQLite every `TestDb` for a given `dir` resolves to the same
    /// `lorehaven.sqlite` file, so a test that opens a second one and writes a
    /// fixture through it appears to work. Under PostgreSQL the second call
    /// creates a *different* database, so the fixture lands somewhere the code
    /// under test never queries, and the test fails for reasons that have
    /// nothing to do with the code. Worse, it can pass for the wrong reason --
    /// an assertion that should have failed finds an empty table and 404s.
    ///
    /// When a test needs both an `AppState` and a fixture handle, build them
    /// from one `TestDb` and return it alongside the router. `media_resilience::
    /// build_app` is the worked example.
    pub async fn connect_with_dir(tag: &str, dir: &Path) -> Self {
        match std::env::var("LOREHAVEN_TEST_PG_URL") {
            Ok(admin_url) if !admin_url.is_empty() => {
                let _ = &dir;
                let name = format!("lh_test_{}_{}", sanitize(tag), unique_suffix());
                let admin = Database::connect(&DatabaseConfig::new(admin_url.clone()))
                    .await
                    .expect("connect to the PostgreSQL admin database");
                // Before adding to the pile, clear what earlier failed runs left.
                sweep_idle_scratch_databases(&admin).await;
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

/// FNV-1a, 64-bit. Fixed offset basis and prime, so the value depends only on
/// the label -- unlike `DefaultHasher`, which is seeded per process and would
/// hand a different UUID on each test binary run, and would collide between two
/// fixtures running in the same process.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// A stable, valid UUID for a test-scoped name.
///
/// The fixtures were written before the PostgreSQL dialect worked, so they use
/// readable slugs like `"work-a1"` for ids. SQLite stores those in a TEXT column
/// and is perfectly happy; the same columns are `UUID` on PostgreSQL, and a slug
/// fails there with 22P02, or 42804 when the statement casts the placeholder but
/// the value is still not a UUID. Rather than spell out 36 characters at every
/// site, map the slug through a fixed function: the same name always yields the
/// same id, so a fixture binds `id("work-a1")` when it inserts and looks up the
/// same value later.
///
/// Deliberately not `Uuid::new_v5`: the workspace enables only uuid's `v4`
/// feature, and adding `v5` for a test helper would widen the dependency surface
/// of every crate. FNV-1a with a fixed offset basis keeps the result stable across
/// runs; a per-process-seeded hasher would hand out a different UUID on every
/// run, and would collide between two fixtures in the same process.
///
/// `label` only has to be unique within one test's fixture. Two fixtures are
/// separate databases, so they may reuse the same labels freely.
#[must_use]
pub fn id(label: &str) -> String {
    // Two rounds, so near-identical labels ("work-a1" / "work-a2") do not differ
    // in only a single bit of the hashed region.
    let a = fnv1a(label.as_bytes());
    let b = fnv1a(&a.to_be_bytes());

    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&a.to_be_bytes());
    bytes[8..].copy_from_slice(&b.to_be_bytes());
    // Version 4 and the RFC 4122 variant bits, so the value is well-formed by
    // any validator rather than merely parseable.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes).to_string()
}
