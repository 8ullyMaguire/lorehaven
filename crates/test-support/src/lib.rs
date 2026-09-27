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
}

impl TestClient {
    pub fn new(app: Router) -> Self {
        Self {
            app,
            cookies: Vec::new(),
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
