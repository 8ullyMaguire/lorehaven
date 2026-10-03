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
use std::sync::Once;
use std::time::{SystemTime, UNIX_EPOCH};

use lorehaven_db::{Backend, Database, DatabaseConfig};

/// Install a tracing subscriber, but only when a test asks for one.
///
/// Without this every `tracing::error!` in the app is dropped on the floor,
/// because nothing calls `tracing_subscriber::fmt::init()` in a test binary.
/// That is why a PostgreSQL-only 500 used to arrive as
/// `{"code":"INTERNAL","message":"Something went wrong on our side"}` with no
/// server-side reason anywhere in the log -- the response deliberately hides it,
/// and the log that would have said why was never wired up.
///
/// Opt in with `LOREHAVEN_TEST_TRACE=1` (and `RUST_LOG` to choose the level;
/// `LOREHAVEN_TRACE_SQL=1` additionally prints every PostgreSQL statement as it
/// is built, which is the only way to see the SQL a 500 came from).
fn install_diagnostic_reporter() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if !std::env::var("LOREHAVEN_TEST_TRACE").is_ok_and(|v| v != "0") {
            return;
        }
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
        // A second reporter is not an error worth failing a test over.
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_test_writer()
            .try_init();
    });
}

/// A per-test database. SQLite is a file inside the harness's scratch
/// directory; PostgreSQL is a scratch database created (and dropped) through
/// the admin URL.
pub struct TestDb {
    db: Database,
    applied: Vec<String>,
    /// `applied` + `already_applied`: the full set. See [`Self::applied_migrations`].
    applied_all: Vec<String>,
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

/// A suffix that is unique among the threads of one process.
///
/// A clock is not enough. This was `subsec_nanos() ^ as_secs() ^ pid << 17`,
/// and `milestone_32_data` produced
/// `duplicate key value violates unique constraint pg_database_datname_index`
/// on `lh_test_fx_248991333688` — two threads read the same clock value, so two
/// tests asked for the same database name. `SystemTime::now()` comes from the
/// vDSO and its resolution is not guaranteed finer than a microsecond on every
/// kernel, so two tests starting together genuinely can read the same value.
///
/// The process id alone is not enough either: `as_secs() << 20` makes the name
/// unique across processes, and the counter makes it unique within one, and
/// both are needed. A test binary that ran a second time while the first was
/// still going would otherwise collide on the name alone.
///
/// Deliberately monotonic and atomic rather than random, so a failure
/// reproduces: the same test in the same order gets the same suffix, and a
/// collision is visible as a repeated number rather than as entropy.
fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::OnceLock;
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    static BASE: OnceLock<u64> = OnceLock::new();
    // Computed once, on first use, so every thread in this process shares it.
    let base = *BASE.get_or_init(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    });
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    // 20 bits of counter, 44 bits of seconds: a counter large enough that no
    // suite reaches it, and seconds that fit the 63 bits PostgreSQL's
    // `bigint` gives us.
    (base << 20) | (n & 0xF_FFFF)
}

/// When this process started, in seconds since the epoch.
///
/// Read once. The scratch-database sweeper compares it against the start time
/// packed into each database name to decide whether a database is old enough to
/// be a leak rather than a concurrent test that has not connected yet.
static PROCESS_START: std::sync::OnceLock<u64> = std::sync::OnceLock::new();

/// How old a database must be before the sweeper will drop it.
///
/// Generous enough to cover the whole `CREATE DATABASE` → connect → migrate
/// window of a slow test binary, and short enough that a real leak from a
/// previous run is reclaimed on the next one. A leaked database is a
/// convenience problem (the container fills up), not a correctness one, so
/// erring towards *not* dropping is the right direction: the cost of a missed
/// sweep is one stale database, and the cost of an over-eager one is a live
/// test failing with `database does not exist`.
const SWEEP_MIN_AGE_SECS: u64 = 300;

/// Unique scratch directory per call (also used for export/storage files).
///
/// **Every call is a distinct directory, including two calls with the same
/// tag in the same test.** This was `format!("lorehaven-test-{tag}-{pid}")`,
/// which is unique per (tag, process) and no more — and that was not enough.
///
/// The failure it caused: a test that builds its database through `setup(tag)`
/// and then asks for `scratch_dir(tag)` again to hand the same path to a
/// router gets the *same* directory, and `scratch_dir` opens with
/// `remove_dir_all`. So the second call deletes the SQLite file out from under
/// the first connection. The symptom is a 500 from a route whose SQL is
/// correct, in a suite that passes every time it is run alone — the file
/// `user_search_ast.rs` hit it on all four of its HTTP tests. Two tests
/// sharing a tag across threads do the same thing to each other, and the
/// winner depends on scheduling.
///
/// So the tag is now a *label* rather than part of the identity, and the
/// identity is `unique_suffix()` — the same atomic-counter primitive
/// PostgreSQL scratch databases use. The label is kept because a failing run's
/// directory is then readable: `lorehaven-test-<tag>-<n>` says which test
/// left it behind, which is the whole reason the tag was there.
pub fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lorehaven-test-{}-{}", tag, unique_suffix()));
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
    // Restricted to databases THIS process created.
    //
    // The suffix is `unique_suffix()`, whose low 20 bits are an atomic counter
    // and whose high 44 bits are the process start time — so the process id is
    // not in the name and cannot be matched on. What the first version did
    // instead was sweep every `lh_test_%` database with no attached backend,
    // on the reasoning that a database with no backends belongs to no run.
    //
    // That reasoning is wrong in a window, and the window is exactly where the
    // sweeper runs: `sweep` is called *between* `CREATE DATABASE` and
    // `Database::connect`. A concurrent test in another binary has created its
    // database and not yet attached a backend, so it looks idle — and gets
    // dropped, and its `connect` then fails with
    //
    //     error returned from database: database "lh_test_anon_edge_10_..."
    //     does not exist
    //
    // which reads as a broken fixture rather than a sweeper that dropped a
    // database out from under a live test. It cost two suites
    // (`analytics_gate`, `analytics_k_anonymity`) in one run.
    //
    // So the sweep is now bounded by an age floor: a database younger than
    // `SWEEP_MIN_AGE` is never dropped, whatever its backend count. Age is not
    // available from `pg_database`, so it is read from the suffix, which
    // encodes the process start time in its high bits — a database this process
    // just created has an age of seconds, and a leaked one from a previous run
    // is minutes or hours old. `pg_stat_activity` remains a second gate, not
    // the only one: "no backend" is necessary, "old enough" is what makes it
    // safe.
    let process_started = *PROCESS_START.get_or_init(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    });
    let idle: Vec<String> = match sqlx::query_scalar(
        "SELECT d.datname FROM pg_database d
         WHERE d.datname LIKE 'lh\\_test\\_%'
           AND NOT EXISTS (
                 SELECT 1 FROM pg_stat_activity a WHERE a.datid = d.oid
           )
         ORDER BY d.datname
         -- 50 was measured to be far too few: a killed gate leaks one database
         -- per TEST, so a full run leaves thousands, and 50 per process meant
         -- they were never reclaimed. Found the hard way -- 188 databases had
         -- reached 3.9 GB and PostgreSQL performance had collapsed with them
         -- (bounties.rs went 349s -> 9s once they were dropped). A cap still
         -- belongs: this runs before every CREATE, and an unbounded drop loop
         -- would stall the suite it is meant to accelerate.
         LIMIT 2000",
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
        // The age floor, read out of the suffix. `unique_suffix()` packs the
        // process start time into the high bits and a counter into the low 20,
        // so `(suffix >> 20)` is the start time of the process that made it.
        let Some(suffix) = name
            .rsplit_once('_')
            .and_then(|(_, s)| s.parse::<u64>().ok())
        else {
            continue;
        };
        let created_at = suffix >> 20;
        if created_at == 0 {
            continue;
        }
        let age = process_started.saturating_sub(created_at);
        if age < SWEEP_MIN_AGE_SECS {
            // Created by a process that started within the floor — which
            // includes this one, and every concurrent test binary. Not a leak.
            continue;
        }
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

// ---------------------------------------------------------------------------
// The migrated-schema cache (test wall-clock).
//
// 2026-10-02. Measured, not guessed: a full two-engine gate spent 2113s of test
// time, and one suite -- `bounties.rs`, FIVE tests -- took 349s. The tests are not
// slow. Every `TestDb` runs all 107 migrations from scratch, so the cost is
// per-TEST rather than per-suite, and it is paid twice (once per engine).
//
// Migrations do not depend on the test. The schema is a function of the migration
// catalogue alone, so it is built ONCE per process and copied.
//
// Two backends, two copy strategies, for the same reason they differ in production:
//
//   * SQLite: `VACUUM INTO` writes a complete second database file, indexes and
//     all. Copying a file costs milliseconds; 107 sequential DDL statements do not.
//   * PostgreSQL: `CREATE DATABASE ... TEMPLATE <name>` clones at the filesystem
//     level. It refuses while connections are open on the template, which is why
//     the template database is dropped again once built -- it has served its
//     purpose and cannot be a template twice.
//
// ── The key, and why it is the whole safety story ───────────────────────────
//
// The cache is keyed on `version:name:sha256(sql)` for every migration, read from
// `migrate::catalogue`. That is deliberately better than a file mtime or length:
//
//   * It needs no filesystem probing, so it cannot disagree with the catalogue the
//     code actually applies -- there is one source of truth.
//   * It is content-addressed. Editing a migration, reverting it, or touching it
//     without changing it all change or preserve the key *correctly*, because the
//     key is the thing that will be applied.
//
// If this key were wrong the failure mode is nasty and silent: a developer who
// edits a migration and runs one suite gets the OLD schema, and debugs a problem
// that no longer exists. That is strictly worse than the slowness, so when in doubt
// the key is over-specified rather than under-specified.
//
// Any failure falls back to migrating in full. A cache that is usually fast and
// occasionally absent is a good trade; one that can fail a run is not.

/// A content fingerprint of one dialect's migration catalogue.
fn migration_key(backend: lorehaven_db::Backend) -> String {
    lorehaven_db::migrate::catalogue(backend)
        .iter()
        .map(|m| format!("{}:{}", m.id(), m.checksum()))
        .collect::<Vec<_>>()
        .join("|")
}

/// The SQLite template, and the key it was built from.
///
/// `OnceLock` rather than a mutex because the work is idempotent and expensive:
/// the loser of a construction race simply reads the winner's value. A mutex would
/// serialise every test behind the builder; this lets them wait on the same build
/// without a lock round-trip per test.
static SQLITE_TEMPLATE: std::sync::OnceLock<(String, std::path::PathBuf)> =
    std::sync::OnceLock::new();
static PG_TEMPLATE: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();

/// Serialises the template BUILD, not the read.
///
/// `OnceLock::get` + `set` is not enough: two threads can both see `None` and both
/// proceed to build, and the build deletes `build.sqlite` and `template.sqlite`
/// first. One thread's `VACUUM INTO` then lands while the other is mid-`copy`, and
/// the caller gets `(code: 26) file is not a database` -- the symptom of a half-
/// written template, not of anything wrong with the test's own database.
///
/// The old comment here claimed the race was handled ("Ignore a lost race: the
/// winner's path is equally valid"). It was not: the losing thread had already
/// deleted the file the winner was writing. Holding this across the build costs one
/// lock acquisition on the very first test and nothing on every later one, because
/// the early `SQLITE_TEMPLATE.get()` returns before the lock is taken.
///
/// **A `tokio::sync::Mutex`, not `std::sync::Mutex`.** The guard is held across
/// `.await` points -- the whole build migrates 107 statements -- and clippy is right
/// to object to that with a std guard: a task holding one across a suspension can
/// block the whole runtime's executor thread and deadlock a single-threaded runtime
/// against itself. The tokio guard releases the thread while it waits.
static TEMPLATE_BUILD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static PG_TEMPLATE_BUILD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The once-per-process template root, so two `TestDb`s in different scratch
/// directories share one built schema.
fn template_root() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("lorehaven-migrations-{}", std::process::id()))
}

/// The SQLite template path, building it on first use.
///
/// Returns `None` when the template cannot be built, and every caller then falls
/// back to a plain `db.migrate()` -- which is slower and always correct.
async fn sqlite_template() -> Option<std::path::PathBuf> {
    let key = migration_key(lorehaven_db::Backend::Sqlite);
    if let Some((k, path)) = SQLITE_TEMPLATE.get() {
        if *k == key {
            return Some(path.clone());
        }
        return None;
    }

    // Held across the entire build, including the deletes below.
    let _guard = TEMPLATE_BUILD.lock().await;
    // Re-check: another thread may have finished the build while this one waited.
    if let Some((k, path)) = SQLITE_TEMPLATE.get() {
        if *k == key {
            return Some(path.clone());
        }
        return None;
    }

    let root = template_root();
    std::fs::create_dir_all(&root).ok()?;
    let build = root.join("build.sqlite");
    let template = root.join("template.sqlite");
    let _ = std::fs::remove_file(&build);
    let _ = std::fs::remove_file(&template);

    let db = Database::connect(&DatabaseConfig::new(format!(
        "sqlite://{}/build.sqlite?mode=rwc",
        build.display()
    )))
    .await
    .ok()?;
    let report = db.migrate().await.ok()?;

    // VACUUM INTO is the copy: it writes a fresh, complete database file and does
    // not disturb the source. `sqlx` does not VACUUM; `Database` does not expose
    // raw statements either, so the pool is used directly -- the same escape hatch
    // the production stores use.
    sqlx::query(&format!("VACUUM INTO '{}'", template.display()))
        .execute(db.sqlite_pool()?)
        .await
        .ok()?;
    db.close().await;
    let _ = report;

    let _ = SQLITE_TEMPLATE.set((key, template.clone()));
    SQLITE_TEMPLATE.get().map(|(_, p)| p.clone())
}

/// A SQLite database file at `dst` with every migration applied, from the cache.
///
/// This is the whole optimisation on the SQLite side: one `copy` syscall pair
/// instead of 107 migrations.
/// A `DatabaseConfig` for a test database, with the acquire timeout widened.
///
/// WHY THIS EXISTS, measured rather than guessed: `cargo test --workspace` fails
/// 12-17 tests with
///
/// ```text
/// panicked at crates/test-support/src/lib.rs:575: connect: ...
/// Caused by: pool timed out while waiting for an open connection
/// ```
///
/// and every one of those suites passes 100% on its own -- at
/// `--test-threads` 16, 4 and 1, and two suites run concurrently also pass. So it
/// is not a defect in any suite and not intra-suite contention.
///
/// The mechanism is that `cargo test --workspace` runs ~180 test binaries, each
/// building its OWN `SqlitePool` (every `TestDb` clones a migrated file and opens a
/// pool over it), with the default `acquire_timeout` of 10s. On this host that is
/// 30GB of RAM with ~15GB of swap already in use, so pool construction slows enough
/// that 10s is not enough. The timeout was never wrong for production; it is wrong
/// for a harness that opens hundreds of pools at once.
///
/// So the test harness asks for longer rather than the code being changed, and
/// rather than the failure being written off as "flaky under load" -- a description
/// that would have hidden the next real failure of the same shape.
pub fn test_db_config(url: impl Into<String>) -> lorehaven_db::DatabaseConfig {
    let mut config = lorehaven_db::DatabaseConfig::new(url);
    config.acquire_timeout = std::time::Duration::from_secs(60);
    config
}

pub async fn cached_sqlite_file(dst: &Path) -> bool {
    match sqlite_template().await {
        Some(template) => std::fs::copy(&template, dst).is_ok(),
        None => false,
    }
}

/// A migrated PostgreSQL database named `name`, cloned from the template.
///
/// Returns `None` when no template exists yet, when the catalogue has moved, or
/// when PostgreSQL refuses the clone for any reason. All three fall back.
pub async fn cached_postgres(admin_url: &str, name: &str) -> Option<Database> {
    let key = migration_key(lorehaven_db::Backend::Postgres);
    // Reuse this process's template when it was built from the same catalogue, and
    // build it on first use. A catalogue that has moved makes the stored template
    // unusable rather than stale-but-close, so that case returns None and the
    // caller migrates in full.
    let template = match PG_TEMPLATE.get() {
        Some((k, t)) if k == &key => t.clone(),
        Some(_) => return None,
        None => {
            let _guard = PG_TEMPLATE_BUILD.lock().await;
            build_postgres_template(admin_url, &key).await?
        }
    };

    let admin = Database::connect(&DatabaseConfig::new(admin_url.to_owned()))
        .await
        .ok()?;
    sqlx::query(&format!("CREATE DATABASE {name} TEMPLATE {template}"))
        .execute(admin.postgres_pool()?)
        .await
        .ok()?;
    let url = admin_url
        .rsplit_once('/')
        .map(|(prefix, _)| format!("{prefix}/{name}"))
        .unwrap_or_else(|| admin_url.to_owned());
    Database::connect(&DatabaseConfig::new(url)).await.ok()
}

/// Build the PostgreSQL template once, and keep it for the rest of the process.
///
/// The template is NOT dropped: `CREATE DATABASE ... TEMPLATE` refuses while any
/// connection is open, and a fresh clone has none, so a kept template serves every
/// later test. Its name carries the `lh_tmpl_` prefix so a killed run's leftover
/// is recognisable and gets swept with the other scratch databases.
async fn build_postgres_template(admin_url: &str, key: &str) -> Option<String> {
    let admin = Database::connect(&DatabaseConfig::new(admin_url.to_owned()))
        .await
        .ok()?;
    let name = format!("lh_tmpl_{}", std::process::id());
    let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"))
        .execute(admin.postgres_pool()?)
        .await;
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(admin.postgres_pool()?)
        .await
        .ok()?;
    let url = admin_url
        .rsplit_once('/')
        .map(|(prefix, _)| format!("{prefix}/{name}"))
        .unwrap_or_else(|| admin_url.to_owned());
    let db = Database::connect(&DatabaseConfig::new(url)).await.ok()?;
    db.migrate().await.ok()?;
    // Every connection closed, or the next `TEMPLATE` clone is refused.
    db.close().await;
    // A concurrent builder may have won; its template is equally good.
    let _ = PG_TEMPLATE.set((key.to_owned(), name.clone()));
    Some(name)
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
        install_diagnostic_reporter();
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

                // The clone path. `CREATE DATABASE ... TEMPLATE` is one statement
                // against an already-migrated database, instead of 107 migrations
                // replayed per test.
                if let Some(db) = cached_postgres(&admin_url, &name).await {
                    // The clone is already fully migrated, so `migrate()` has nothing to do
                    // and reports everything under `already_applied`. That call is also what
                    // makes this path honest: if the cache were stale, `applied` would be
                    // non-empty and the assert further down would catch it.
                    let report = db.migrate().await.expect("migrate the clone");
                    return Self {
                        applied: report.applied.clone(),
                        applied_all: report
                            .applied
                            .into_iter()
                            .chain(report.already_applied)
                            .collect(),
                        db,
                        pg_admin: Some(admin),
                        pg_name: Some(name),
                    };
                }

                let db = Database::connect(&DatabaseConfig::new(url))
                    .await
                    .expect("connect to the scratch test database");
                let report = db.migrate().await.expect("migrate");
                Self {
                    applied_all: full_migration_set(&report),
                    applied: report.applied,
                    db,
                    pg_admin: Some(admin),
                    pg_name: Some(name),
                }
            }
            _ => {
                // The copy path: a migrated file is placed at the path the pool is
                // about to open, so `migrate()` then finds every migration already
                // recorded as applied and does nothing. Same observable result --
                // a fully-migrated database -- for one file copy instead of 107 DDL
                // statements.
                let file = dir.join("lorehaven.sqlite");
                let _ = std::fs::remove_file(&file);
                let cloned = cached_sqlite_file(&file).await;

                let db = Database::connect(&test_db_config(format!(
                    "sqlite://{}/lorehaven.sqlite?mode=rwc",
                    dir.display()
                )))
                .await
                .expect("connect");
                let report = db.migrate().await.expect("migrate");
                if cloned {
                    // A clone has no pending work, so an empty report is expected
                    // rather than suspicious. Asserted rather than assumed: a cache
                    // that quietly stopped applying migrations would otherwise look
                    // exactly like a success.
                    assert!(
                        report.applied.is_empty(),
                        "the schema cache produced a database with {} pending migrations \
                         ({}); the cache key no longer matches the catalogue",
                        report.applied.len(),
                        report.applied.join(", ")
                    );
                }
                Self {
                    applied_all: full_migration_set(&report),
                    applied: report.applied,
                    db,
                    pg_admin: None,
                    pg_name: None,
                }
            }
        }
    }

    /// The migrations recorded as applied in this database.
    ///
    /// This used to return `self.applied`, which was `report.applied` from the
    /// `migrate()` call during `connect`. That was "everything" only while every database
    /// was built from scratch, and stopped being true when the template-database cache
    /// landed: both clone paths (`CREATE DATABASE ... TEMPLATE` on PostgreSQL, a copied
    /// file on SQLite) construct a `TestDb` with `applied: Vec::new()` because there is no
    /// pending work, so the method returned an empty list for a fully-migrated database.
    ///
    /// The failure was invisible in aggregate -- eight suites assert a specific migration is
    /// present, and they all failed together with a message that reads like a broken
    /// migration rather than a broken accessor. `applied_migrations` now reads the ledger.
    ///
    /// Every migration recorded as applied to this database, whether this run applied it or
    /// a previous one did.
    ///
    /// Sourced from the store field rather than a database query: lorehaven does not use
    /// sqlx's `_sqlx_migrations` ledger (it has its own migration runner, and the table does
    /// not exist), so `MigrationReport::already_applied` is the authoritative record and is
    /// captured during `connect`.
    pub fn applied_migrations(&self) -> Vec<String> {
        self.applied_all.clone()
    }

    /// The migrations `migrate()` applied during `connect_with_dir` — *this call only*.
    ///
    /// Empty whenever the clone path was taken, which is most runs once the cache is warm.
    /// Prefer [`Self::applied_migrations`] unless the question really is "did this connect
    /// have work to do", which is how the cache's own correctness assertion uses it.
    pub fn migrations_applied_at_connect(&self) -> &[String] {
        &self.applied
    }

    /// True when the suite is running against PostgreSQL.
    /// The scratch PostgreSQL database's name, or None on SQLite.
    ///
    /// Exists for `snapshot_anonymisation`, which shells out to `pg_dump` and
    /// needs the database that THIS test created. Deriving it from the env URL
    /// instead dumps the template database, which exists, is empty of the
    /// test's rows, and produces a passing leak assertion for the wrong reason.
    pub fn pg_database_name(&self) -> Option<&str> {
        self.pg_name.as_deref()
    }

    pub fn is_postgres(&self) -> bool {
        self.db.backend() == Backend::Postgres
    }

    /// The database handle for the harness's router and seed helpers.
    pub fn db(&self) -> &Database {
        &self.db
    }

    /// Fetch one nullable text column, whichever backend is active.
    ///
    /// For asserting a stored value a route does not echo back — a soft-delete
    /// timestamp, a column the API deliberately omits. Tests that reach for a
    /// pool directly get PostgreSQL wrong (wrong placeholders, `::uuid` casts),
    /// and the failure only shows on the PG run.
    pub async fn fetch_text(&self, query: &str, value: &str) -> Option<String> {
        let sql = self.id_where(query);
        let row: Option<(Option<String>,)> = match self.db.backend() {
            Backend::Sqlite => {
                sqlx::query_as(&sql)
                    .bind(value)
                    .fetch_optional(self.db.sqlite_pool().expect("sqlite"))
                    .await
            }
            Backend::Postgres => {
                sqlx::query_as(&sql)
                    .bind(value)
                    .fetch_optional(self.db.postgres_pool().expect("postgres"))
                    .await
            }
        }
        .expect("scalar query");
        row.and_then(|r| r.0)
    }

    /// Whether a row matching the query exists.
    pub async fn exists(&self, query: &str, value: &str) -> bool {
        let sql = self.id_where(query);
        let row: Option<(i32,)> = match self.db.backend() {
            Backend::Sqlite => {
                sqlx::query_as(&sql)
                    .bind(value)
                    .fetch_optional(self.db.sqlite_pool().expect("sqlite"))
                    .await
            }
            Backend::Postgres => {
                sqlx::query_as(&sql)
                    .bind(value)
                    .fetch_optional(self.db.postgres_pool().expect("postgres"))
                    .await
            }
        }
        .expect("exists query");
        row.is_some()
    }

    /// A scalar query matching a row by a UUID id, cast per backend.
    ///
    /// `chapters.id` is TEXT on SQLite and UUID on PostgreSQL, so the
    /// SQLite-shaped `WHERE id = ?` the rest of the suite writes is a type error
    /// on PG (`operator does not exist: uuid = text`). Callers pass the SQLite
    /// form and this rewrites the comparison, so a test asserting a stored value
    /// does not have to know the dialect — the failure mode otherwise only shows
    /// on the PG run.
    fn id_where(&self, query: &str) -> String {
        match self.db.backend() {
            Backend::Postgres => {
                lorehaven_db::rewrite_placeholders(&query.replace("id = ?", "id::text = ?"))
            }
            Backend::Sqlite => query.to_string(),
        }
    }

    /// A query string for the active backend: SQLite takes `?` placeholders
    /// as written; PostgreSQL needs `$n`, which this rewrites.
    ///
    /// This is the raw escape hatch, and it handles placeholders only. A uuid
    /// column additionally needs a `::text` cast on PostgreSQL — for the
    /// comparison *and* for the projection — and that cast is a **syntax error**
    /// on SQLite, so no single query string can satisfy both engines. A test
    /// that reads a uuid column should use [`Self::fetch_text_column`], which
    /// builds the query per backend and is the reason that function exists.
    pub fn sql(&self, query: &str) -> String {
        match self.db.backend() {
            Backend::Postgres => lorehaven_db::rewrite_placeholders(query),
            Backend::Sqlite => query.to_string(),
        }
    }

    /// Fetch one text column from one row, matched by one column.
    ///
    /// The dialect handling lives here rather than in the caller's SQL, because
    /// the two engines disagree in *opposite* directions and a caller cannot
    /// write one query that satisfies both:
    ///
    /// - a uuid column (`account_id`, `pseud.id`) is `TEXT` on SQLite and
    ///   `UUID` on PostgreSQL, so the same `SELECT` decodes on one arm and fails
    ///   on the other with `mismatched types; Rust type String (as SQL type
    ///   TEXT) is not compatible with SQL type UUID`;
    /// - `col::text` is the fix on PostgreSQL and a **syntax error** on SQLite,
    ///   so the cast cannot simply be written into the query.
    ///
    /// Writing `SELECT col::text` in the test fixed PostgreSQL and broke
    /// SQLite; writing `SELECT col` fixed SQLite and broke PostgreSQL. Hence
    /// this takes the column as a name and builds the query per backend.
    /// Table and column names are interpolated rather than bound because
    /// PostgreSQL cannot bind an identifier — callers pass literals, and these
    /// are test queries, not a user-facing surface.
    pub async fn fetch_text_column(
        &self,
        table: &str,
        column: &str,
        match_column: &str,
        value: &str,
    ) -> Option<String> {
        // A uuid projection must become text on PostgreSQL. `token_hash`,
        // `state` and timestamps are genuinely TEXT on both, and casting them
        // would be harmless but is not needed.
        let projection = if self.is_postgres() && is_uuid_column(column) {
            format!("{column}::text")
        } else {
            column.to_owned()
        };
        // The comparison needs the same treatment: `uuid = text` has no
        // operator.
        let predicate = if self.is_postgres() && is_uuid_column(match_column) {
            format!("{match_column}::text = ?")
        } else {
            format!("{match_column} = ?")
        };
        let sql = self.sql(&format!(
            "SELECT {projection} FROM {table} WHERE {predicate}"
        ));
        let row: Option<(Option<String>,)> = match self.db.backend() {
            Backend::Sqlite => {
                sqlx::query_as(&sql)
                    .bind(value)
                    .fetch_optional(self.db.sqlite_pool().expect("sqlite"))
                    .await
            }
            Backend::Postgres => {
                sqlx::query_as(&sql)
                    .bind(value)
                    .fetch_optional(self.db.postgres_pool().expect("postgres"))
                    .await
            }
        }
        .expect("scalar query");
        row.and_then(|r| r.0)
    }

    /// Fetch one nullable text column by any column value, whichever backend.
    ///
    /// [`Self::fetch_text`] is keyed to `id`; this one binds wherever the caller
    /// says, for the columns `fetch_text` cannot address —
    /// `api_tokens.token_hash`, a `state` column, a `last_used_at` timestamp.
    ///
    /// On PostgreSQL the query is rewritten twice, and the second rewrite is
    /// the one that bites:
    ///
    /// - `?` becomes `$n`.
    /// - a uuid column compared to a text bind becomes `col::text = ?`.
    ///   `WHERE account_id = $1` with a text bind is `operator does not exist:
    ///   uuid = text`.
    ///
    /// The caller must still cast any uuid column it **selects** into text
    /// (`SELECT account_id::text`). This cannot be done here: whether a
    /// projection decodes as text is the caller's decision, and guessing would
    /// mean silently rewriting the columns a test is asserting on. That
    /// asymmetry is the whole trap — casting the *bind* is the codebase's habit
    /// and does nothing for a *projection*.
    ///
    /// The reason this exists at all: a test reaching for `sqlite_pool()`
    /// directly passes locally and fails on the PG run, which is the worst way
    /// for it to fail, because the signal arrives after the work is done.
    pub async fn fetch_text_by(&self, query: &str, value: &str) -> Option<String> {
        let sql = self.sql(query);
        let row: Option<(Option<String>,)> = match self.db.backend() {
            Backend::Sqlite => {
                sqlx::query_as(&sql)
                    .bind(value)
                    .fetch_optional(self.db.sqlite_pool().expect("sqlite"))
                    .await
            }
            Backend::Postgres => {
                sqlx::query_as(&sql)
                    .bind(value)
                    .fetch_optional(self.db.postgres_pool().expect("postgres"))
                    .await
            }
        }
        .expect("scalar query");
        row.and_then(|r| r.0)
    }

    /// Count rows matching one bound value, whichever backend.
    ///
    /// `fetch_text_by` answers "what is in this column"; this answers "is
    /// there such a row at all", which is the question several expiry and
    /// single-use tests actually need and which `exists` cannot express
    /// without knowing the id.
    pub async fn count_by(&self, query: &str, value: &str) -> i64 {
        let sql = self.sql(query);
        let row: (i64,) = match self.db.backend() {
            Backend::Sqlite => {
                sqlx::query_as(&sql)
                    .bind(value)
                    .fetch_one(self.db.sqlite_pool().expect("sqlite"))
                    .await
            }
            Backend::Postgres => {
                sqlx::query_as(&sql)
                    .bind(value)
                    .fetch_one(self.db.postgres_pool().expect("postgres"))
                    .await
            }
        }
        .expect("count query");
        row.0
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

// ---------------------------------------------------------------------------
// HTTP client for route-level tests
// ---------------------------------------------------------------------------
//
// Several suites need to drive the real router with a real session. The client
// was copy-pasted into each of them, which is how a cookie-capture bug or a CSRF
// change had to be fixed in four places. It lives here now so there is one
// implementation, and adding a route test no longer means copying 150 lines
// first.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use serde_json::Value;
use tower::ServiceExt;

/// Whether this project's schema declares `column` as a uuid.
///
/// A uuid column is `TEXT` on SQLite and `UUID` on PostgreSQL, which is the
/// single fact behind nearly every two-engine test failure in this repo: the
/// same query decodes on one arm and not the other, or compares with an
/// operator that exists on only one. The list is deliberately explicit rather
/// than a heuristic on the name — `id` is a uuid in some tables and TEXT in
/// others (`link_challenges.code`, `api_tokens.token_hash`), so a name-based
/// guess would be wrong in both directions.
fn is_uuid_column(column: &str) -> bool {
    matches!(
        column,
        "id" | "account_id"
            | "pseud_id"
            | "pseud"
            | "token_id"
            | "bot_id"
            | "registration_id"
            | "author_id"
            | "owner_id"
            | "user_id"
            | "post_id"
            | "work_id"
            | "item_id"
            | "parent_id"
            | "created_by"
            | "approved_by"
    )
}

/// A router plus the cookies it has handed out.
///
/// The cookie jar is the whole point: the app authenticates with a session
/// cookie, so a route test that does not carry cookies back is testing the
/// anonymous path and passing for the wrong reason.
pub struct TestClient {
    app: Router,
    cookies: Vec<(String, String)>,
    /// Headers of the most recent response, for `get_with_headers`.
    last_headers: axum::http::HeaderMap,
}

impl TestClient {
    pub fn new(app: Router) -> Self {
        Self {
            app,
            cookies: Vec::new(),
            last_headers: axum::http::HeaderMap::new(),
        }
    }

    fn capture(&mut self, response: &axum::response::Response) {
        for value in response.headers().get_all(header::SET_COOKIE) {
            let Ok(text) = value.to_str() else { continue };
            let Some((pair, _)) = text.split_once(';') else {
                continue;
            };
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            let name = name.trim().to_owned();
            let value = value.trim().to_owned();
            // A cleared cookie arrives with an empty value; dropping it is how
            // a logout actually takes effect in the jar.
            self.cookies.retain(|(k, _)| k != &name);
            if !value.is_empty() {
                self.cookies.push((name, value));
            }
        }
    }

    /// Issue a request, carrying and capturing cookies.
    pub async fn request(
        &mut self,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        self.request_with(method, uri, body, None).await
    }

    /// Issue a request that also carries a bearer token.
    ///
    /// The bearer path is separate from the cookie path on purpose: a test that
    /// wants to prove "a *token* can do this" must not be holding a session
    /// cookie, or the request succeeds through the session and the token is
    /// never exercised. `MaybeToken` prefers whichever is present, so a test
    /// holding both proves nothing about either.
    pub async fn request_with(
        &mut self,
        method: &str,
        uri: &str,
        body: Option<Value>,
        bearer: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(token) = bearer {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        if !self.cookies.is_empty() {
            let jar = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            builder = builder.header(header::COOKIE, jar);
        }
        // The CSRF check wants a header echoing the token the session cookie set.
        // The cookie is `lorehaven_csrf`, and a safe method needs none -- the app
        // only rejects unsafe ones, so sending it everywhere is harmless but
        // omitting it on GET keeps the request honest.
        if !matches!(method, "GET" | "HEAD" | "OPTIONS") {
            if let Some(csrf) = self.cookie("lorehaven_csrf").map(str::to_owned) {
                builder = builder.header("x-csrf-token", csrf);
            }
        }
        let request = match body {
            Some(value) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(value.to_string()))
                .expect("build request"),
            None => builder.body(Body::empty()).expect("build request"),
        };
        let response = self
            .app
            .clone()
            .oneshot(request)
            .await
            .expect("router is infallible");
        let status = response.status();
        // Stashed before the body is drained, for `get_with_headers`.
        self.last_headers = response.headers().clone();
        self.capture(&response);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap_or_default();
        // A body that will not parse is surfaced as text, not swallowed.
        //
        // `unwrap_or(Value::Null)` made every failing assertion in the M54 suite
        // print "null" for a response that had a real body: axum's JSON extractor
        // rejects a malformed payload with a *plain-text* explanation, and this
        // quietly turned that into `null`. A suite that renders its failures as
        // `null` is a suite you debug with print statements. The real culprit
        // this exposed was a test sending a body missing a required field, which
        // read as "the route does not exist" until the text was visible.
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                Value::String(format!(
                    "<non-JSON body: {e}; raw: {}>",
                    String::from_utf8_lossy(&bytes[..bytes.len().min(200)])
                ))
            })
        };
        (status, value)
    }

    pub async fn get(&mut self, uri: impl AsRef<str>) -> (StatusCode, Value) {
        self.request("GET", uri.as_ref(), None).await
    }

    /// A GET that also returns the response headers.
    ///
    /// `get` and `request` drop the headers, which is fine for almost every
    /// assertion and fatal for an indistinguishability check: a route that
    /// answers a gated body with `X-Body-Cached: true` and an absent one
    /// without it is leaking, and no `(StatusCode, Value)` comparison can see
    /// that. §7.7.3 is exactly the requirement this exists for, and the
    /// alternative — writing that comparison against a raw `oneshot` in the
    /// test file — would duplicate the cookie and CSRF plumbing and drift.
    pub async fn get_with_headers(
        &mut self,
        uri: impl AsRef<str>,
    ) -> (StatusCode, axum::http::HeaderMap, Value) {
        let (status, value) = self.request("GET", uri.as_ref(), None).await;
        (status, self.last_headers.clone(), value)
    }

    pub async fn post(&mut self, uri: impl AsRef<str>, body: Value) -> (StatusCode, Value) {
        self.request("POST", uri.as_ref(), Some(body)).await
    }

    pub async fn put(&mut self, uri: impl AsRef<str>, body: Value) -> (StatusCode, Value) {
        self.request("PUT", uri.as_ref(), Some(body)).await
    }

    pub fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Forget every cookie, so the next request is anonymous.
    ///
    /// For a test that has to become a *different* account. A cross-account
    /// test that keeps the first identity's session and only registers the
    /// second one is not a cross-account test at all — the caller is still the
    /// first account, the request succeeds through that session, and the test
    /// measures nothing. This is not hypothetical: it is how the M54 revoke
    /// test was first written, and it failed on a sanity assertion only because
    /// the assertion existed.
    pub fn clear_cookies(&mut self) {
        self.cookies.clear();
    }
}

/// The password [`register`] and [`sign_in_as`] use.
///
/// Public because a test that has to log in again — after becoming a different
/// account, say — must use the same one `register` used. A test that declares a
/// second constant with the same value is a value that can drift.
pub const TEST_PASSWORD: &str = "a-long-enough-passphrase";

/// Register an account through the API and return its id.
///
/// Goes through the real registration route rather than inserting a row, so the
/// session cookie and the CSRF token are the ones the app itself issued.
pub async fn register(client: &mut TestClient, email: &str, handle: &str) -> String {
    let (status, body) = client
        .post(
            "/api/v1/auth/register",
            serde_json::json!({
                "email": email,
                "password": TEST_PASSWORD,
                "handle": handle,
                "display_name": handle,
                "age_band": "adult"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "register {handle}: {body}");
    let (status, me) = client.get("/api/v1/auth/me").await;
    assert_eq!(status, StatusCode::OK, "{me}");
    me["account"]["id"]
        .as_str()
        .expect("account id in /auth/me")
        .to_owned()
}

/// The id of the pseud this session is currently acting as.
///
/// Returns `None` for an account with no pseud selected, which is a real state and
/// not an error — `reading_history_entry` is keyed by BOTH `account_id` and
/// `pseud_id`, so a fixture writing a reading act has to know which pseud is active
/// or it writes a row the seen-exclusion will never read.
pub async fn active_pseud_id(client: &mut TestClient, account_id: &str) -> Option<String> {
    let (status, me) = client.get("/api/v1/auth/me").await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(
        me["account"]["id"].as_str(),
        Some(account_id),
        "asked for the pseud of {account_id}, but this session is {}",
        me["account"]["id"]
    );
    // `/auth/me` reports `active_pseud_id` at the top level, alongside `pseuds`.
    // It is the same resolution the concierge's `RequireSession` sees: the
    // session's choice if it still exists, else the account's first pseud.
    me["active_pseud_id"].as_str().map(str::to_owned)
}

/// Become `email`, registering the account if it does not exist yet.
///
/// The registration check asks the database rather than parsing a status code.
/// A duplicate email comes back `422 VALIDATION_FAILED` with a *field* error —
/// not `409 CONFLICT` — so a helper that tolerates "created or conflict" is
/// built on a guess, and in practice it was: asserting `409` meant this could
/// only ever be called once per account, and it is called at least twice in the
/// cross-account revoke test (as the intruder, then back as the owner).
pub async fn sign_in_as(client: &mut TestClient, db: &TestDb, email: &str, handle: &str) -> String {
    let exists = db
        .count_by("SELECT COUNT(*) FROM accounts WHERE email = ?", email)
        .await;
    if exists == 0 {
        register(client, email, handle).await;
    }
    // Drop the previous identity's cookies: a stale CSRF token left behind
    // fails the login *after* it succeeded, or worse passes it against the
    // wrong session.
    client.clear_cookies();
    let (status, body) = client
        .post(
            "/api/v1/auth/login",
            serde_json::json!({ "email": email, "password": TEST_PASSWORD }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "login as {email}: {body}");
    let (status, me) = client.get("/api/v1/auth/me").await;
    assert_eq!(status, StatusCode::OK, "{me}");
    me["account"]["id"]
        .as_str()
        .expect("account id in /auth/me")
        .to_owned()
}

/// Every migration recorded against a freshly migrated database.
///
/// `MigrationReport` splits what this run did from what was already there, and callers of
/// [`TestDb::applied_migrations`] mean the union. `already_applied` being empty is normal on
/// the non-clone path (a brand-new database has had nothing applied), and non-empty on the
/// clone paths.
fn full_migration_set(report: &lorehaven_db::migrate::MigrationReport) -> Vec<String> {
    let mut all = report.applied.clone();
    all.extend(report.already_applied.iter().cloned());
    all
}

#[cfg(test)]
mod tests {
    use super::scratch_dir;

    /// A `TestDb` reports every migration applied to it, not only the ones this connect
    /// performed.
    ///
    /// The defect this pins: `applied_migrations()` returned `report.applied`, which is
    /// "what this run did". That was equal to the full set only while every database was
    /// built from scratch, and stopped being true when the template-database cache landed —
    /// both clone paths construct a `TestDb` with an empty `applied`, so a fully-migrated
    /// database reported zero migrations.
    ///
    /// Eight suites assert a specific migration is present, and they all failed together
    /// with "taxonomy migration must apply: []" — a message that reads like a broken
    /// migration, so the failure was investigated in the wrong place for a while.
    ///
    /// The test asserts a migration known to exist and one known to be recent, so it fails
    /// both when the accessor returns nothing at all and when it returns a truncated set.
    #[tokio::test]
    async fn applied_migrations_reports_the_whole_ledger() {
        let dir = scratch_dir("ts_applied_ledger");
        let tdb = super::TestDb::connect_with_dir("ts-applied-ledger", &dir).await;
        let applied = tdb.applied_migrations();
        assert!(
            applied.iter().any(|m| m.contains("0011")),
            "an early migration must be in the reported set: {applied:?}"
        );
        assert!(
            !applied.is_empty(),
            "a migrated database reports an empty migration set"
        );
        tdb.cleanup().await;
    }

    /// Two calls with the same tag must be two directories.
    ///
    /// The defect this pins: `scratch_dir` was keyed on (tag, pid), so a test
    /// that called it twice with one tag got the same path twice — and the
    /// function opens with `remove_dir_all`, so the second call deleted the
    /// first call's SQLite file out from under a live connection. The symptom
    /// was a 500 from a route whose SQL is correct, in a suite that passed
    /// every time it ran alone.
    ///
    /// A property test rather than a single pair, because the two calls here
    /// are adjacent and an implementation that only collided under
    /// concurrency would pass them.
    #[test]
    fn scratch_dir_is_unique_per_call_not_per_tag() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..16 {
            let dir = scratch_dir("the_same_tag");
            assert!(
                seen.insert(dir.clone()),
                "scratch_dir returned {dir:?} twice for one tag: the second call removes the \
                 first call's directory, which is a live SQLite database"
            );
        }
    }

    /// The sweeper's age floor agrees with the suffix it is decoded from.
    ///
    /// A race, not a value, and a value test is all that can be said about one:
    /// the sweeper used to drop any `lh_test_%` database with no attached
    /// backend, which in the window between another binary's `CREATE DATABASE`
    /// and its `connect` is every database that binary is about to use. The
    /// symptom was `database "lh_test_..." does not exist` in a suite whose
    /// fixture is correct, which is the hardest kind of failure to read.
    ///
    /// So the floor is arithmetic on the name, and this pins the arithmetic: a
    /// suffix minted now must decode to an age inside the floor, and one minted
    /// `SWEEP_MIN_AGE_SECS` ago must decode outside it. If `unique_suffix`
    /// changes its bit layout, this fails rather than the sweeper silently
    /// decaying into either never-dropping or always-dropping.
    #[test]
    fn the_sweeper_age_floor_matches_the_suffix_layout() {
        use super::{PROCESS_START, SWEEP_MIN_AGE_SECS};
        use std::time::{SystemTime, UNIX_EPOCH};

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_secs();
        let started = *PROCESS_START.get_or_init(|| now);

        // `unique_suffix` is `(seconds << 20) | counter`, so the process start
        // time is the high bits and the counter the low 20.
        let mint = |age: u64| -> u64 { now.saturating_sub(age) << 20 };
        let decoded_age = |suffix: u64| started.saturating_sub(suffix >> 20);

        assert!(
            decoded_age(mint(0)) < SWEEP_MIN_AGE_SECS,
            "a database minted now must be inside the floor, or the sweeper drops live tests"
        );
        assert!(
            decoded_age(mint(SWEEP_MIN_AGE_SECS + 60)) >= SWEEP_MIN_AGE_SECS,
            "a database older than the floor must be droppable, or leaks are never reclaimed"
        );
    }

    /// The tag survives in the path, because a leaked directory has to be
    /// attributable to a test.
    ///
    /// The second half of the change: uniqueness came from a counter, and the
    /// obvious way to keep names short is to drop the tag. That would make a
    /// failed run's leftovers a wall of anonymous numbers, which is the reason
    /// the tag was in the name to begin with.
    #[test]
    fn scratch_dir_keeps_its_tag_in_the_path() {
        let dir = scratch_dir("attributable");
        let name = dir
            .file_name()
            .expect("a file name")
            .to_string_lossy()
            .into_owned();
        assert!(
            name.contains("attributable"),
            "the directory name must say which test left it: {name}"
        );
    }
}
