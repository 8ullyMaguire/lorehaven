//! M32-07e (P1): the `device_deliveries` foreign key that migration 0035 broke.
//!
//! The spec's own pitfall, hit for real. 0035 rebuilt `export_jobs`, and SQLite
//! rewrites *referencing* foreign keys when it renames a table — so the
//! `device_deliveries.export_job_id` constraint from 0008 was silently repointed
//! at `export_jobs_old` and then had its target dropped. Every insert failed
//! with `no such table: main.export_jobs_old`.
//!
//! PostgreSQL has the opposite defect from the same migration: it dropped
//! `device_deliveries` outright to invalidate its constraints and never
//! recreated it, so the table has been absent there since 0035.
//!
//! Both shipped green because **nothing in the codebase inserts into this
//! table**. The schema was wrong and unexercised, not working. That is why these
//! tests do the insert themselves instead of only reading the schema: a
//! structural assertion passes on a table whose constraint merely *looks*
//! plausible, and a suite that never touches the table cannot fail.
//!
//! Migration 0075 repairs both dialects. Run on both: the defects differ, so a
//! SQLite-only run leaves the PostgreSQL half unproven.

use lorehaven_db::identity::AccountStatus;
use lorehaven_domain::ids::AccountId;
use lorehaven_domain::policy::AgeState;
use std::path::PathBuf;
use test_support::TestDb;

/// A fresh scratch directory for one test's database.
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-dd-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Open a test database, on whichever backend `LOREHAVEN_TEST_PG_URL` selects.
async fn open(tag: &str) -> TestDb {
    TestDb::connect_with_dir(tag, &scratch_dir(tag)).await
}

/// A query string for the active backend, with the PostgreSQL arm spelling its
/// id placeholders `?::uuid`.
///
/// This is the house pattern from `create_export` in `crates/db/src/exports.rs`,
/// and it is not optional: a `TEXT` bind against a PostgreSQL `UUID` column is
/// rejected with `column "id" is of type uuid but expression is of type text`
/// (42804). `TestDb::sql` only renumbers `?` to `$n`; it does not cast, so a
/// test that uses it directly for an id column passes on SQLite and fails on
/// PostgreSQL.
///
/// Doing it this way rather than with a hand-rolled sqlx `Encode`/`Type` wrapper
/// means the test exercises the same mechanism production relies on.
fn q(tdb: &TestDb, sqlite: &str, postgres: &str) -> String {
    if tdb.is_postgres() {
        tdb.sql(postgres)
    } else {
        tdb.sql(sqlite)
    }
}

/// Run one body against whichever backend the harness opened, so the same
/// assertions execute on SQLite and on PostgreSQL without a duplicated test.
///
/// The workspace has no `sqlx::Any` pool — `Database::sqlite_pool` and
/// `postgres_pool` return concrete, backend-matched pools — so the branch has to
/// be at the executor rather than hidden behind a common type.
macro_rules! on_backend {
    ($tdb:expr, $db:expr, |$pool:ident| $body:block) => {{
        match $db.backend() {
            lorehaven_db::Backend::Sqlite => {
                let $pool = $db.sqlite_pool().expect("sqlite pool");
                $body
            }
            lorehaven_db::Backend::Postgres => {
                let $pool = $db.postgres_pool().expect("postgres pool");
                $body
            }
        }
    }};
}

#[tokio::test]
async fn every_device_deliveries_foreign_key_names_a_table_that_exists() {
    let tdb = open("m32-07e-fk-targets").await;
    let db = tdb.db().clone();

    // The structural invariant, expressed in each dialect's own catalog. The
    // spelling differs; the assertion does not: a constraint may not name a
    // relation that does not exist. On PostgreSQL this additionally fails when
    // the table is missing outright, which is that dialect's actual 0035 bug.
    let dangling: Vec<String> = on_backend!(tdb, &db, |pool| {
        let sql = if tdb.is_postgres() {
            "SELECT a.attname
             FROM pg_constraint c
             JOIN unnest(c.conkey) WITH ORDINALITY AS k(attnum, ord) ON true
             JOIN pg_attribute a
               ON a.attrelid = c.conrelid AND a.attnum = k.attnum
             WHERE c.contype = 'f'
               AND c.conrelid = to_regclass('device_deliveries')
               AND NOT EXISTS (
                   SELECT 1 FROM pg_class r
                   WHERE r.oid = c.confrelid AND r.relkind = 'r')"
        } else {
            "SELECT m.\"from\"
             FROM pragma_foreign_key_list('device_deliveries') m
             WHERE m.\"table\" NOT IN (
                 SELECT name FROM sqlite_master WHERE type = 'table')"
        };
        sqlx::query_scalar(sql)
            .fetch_all(pool)
            .await
            .unwrap_or_default()
    });

    assert!(
        dangling.is_empty(),
        "device_deliveries has foreign keys naming tables that do not exist: \
         {dangling:?} — this is the break migration 0035 caused"
    );

    tdb.cleanup().await;
}

#[tokio::test]
async fn a_device_delivery_can_be_inserted_and_outlives_its_export() {
    let tdb = open("m32-07e-insert").await;
    let db = tdb.db().clone();

    let account = make_account(&db, "device-delivery@example.test").await;

    // A real job row, so the export_jobs FK to jobs(id) is satisfied by a real
    // parent rather than dodged with a fabricated one.
    let job_id = lorehaven_db::jobs::enqueue(
        &db,
        lorehaven_domain::jobs::JobKind::Export,
        "{}",
        None,
        Some(AccountId::from_uuid(account)),
        0,
        &lorehaven_domain::jobs::RetryPolicy::default(),
    )
    .await
    .expect("enqueue job");

    // A real UUID rather than a readable literal: the column is UUID on
    // PostgreSQL, and a hand-written non-UUID string would fail there and pass
    // on SQLite -- the exact shape of bug this milestone is about.
    let export_id = uuid::Uuid::new_v4();
    // `device_deliveries.id` is UUID on PostgreSQL, so the id has to be one
    // there too -- a readable literal like "delivery-m32-07e" is rejected with
    // `invalid input syntax for type uuid` (22P02) while passing on SQLite.
    let delivery_id = uuid::Uuid::new_v4();
    let now = "2026-09-25T00:00:00Z";

    on_backend!(tdb, &db, |pool| {
        sqlx::query(&q(
            &tdb,
            "INSERT INTO export_jobs
                (id, job_id, account_id, subject_type, subject_id, format, created_at, updated_at)
             VALUES (?, ?, ?, 'query', ?, 'epub', ?, ?)",
            "INSERT INTO export_jobs
                (id, job_id, account_id, subject_type, subject_id, format, created_at, updated_at)
             VALUES (?::uuid, ?::uuid, ?::uuid, 'query', ?::uuid, 'epub', \
                      ?::timestamptz, ?::timestamptz)",
        ))
        .bind(export_id.to_string())
        .bind(job_id.to_string())
        .bind(account.to_string())
        .bind(export_id.to_string())
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .expect("export job");
    });

    on_backend!(tdb, &db, |pool| {
        sqlx::query(&q(
            &tdb,
            "INSERT INTO device_deliveries
                (id, export_job_id, account_id, target, address, state, created_at)
             VALUES (?, ?, ?, 'kindle', 'reader@example.test', 'queued', ?)",
            "INSERT INTO device_deliveries
                (id, export_job_id, account_id, target, address, state, created_at)
             VALUES (?::uuid, ?::uuid, ?::uuid, 'kindle', 'reader@example.test', 'queued', ?)",
        ))
        .bind(delivery_id.to_string())
        .bind(export_id.to_string())
        .bind(account.to_string())
        .bind(now)
        .execute(pool)
        .await
        .expect("device delivery insert — this is the statement 0035 broke");
    });

    // The 0008 comment promises the diagnostic outlives the export, via
    // ON DELETE SET NULL. Assert the behaviour, not the constraint's text.
    let (survivors, link): (i64, Option<String>) = on_backend!(tdb, &db, |pool| {
        sqlx::query(&q(
            &tdb,
            "DELETE FROM export_jobs WHERE id = ?",
            "DELETE FROM export_jobs WHERE id = ?::uuid",
        ))
        .bind(export_id.to_string())
        .execute(pool)
        .await
        .expect("delete export");
        sqlx::query_as(&q(
            &tdb,
            "SELECT COUNT(*), MIN(export_job_id) FROM device_deliveries WHERE id = ?",
            "SELECT COUNT(*), MIN(export_job_id::text) FROM device_deliveries WHERE id = ?::uuid",
        ))
        .bind(delivery_id.to_string())
        .fetch_one(pool)
        .await
        .expect("read the delivery back")
    });

    assert_eq!(
        survivors, 1,
        "the delivery row must outlive the export it recorded"
    );
    assert!(
        link.is_none(),
        "export_job_id should be NULL once the export is deleted, got {link:?}"
    );

    tdb.cleanup().await;
}

#[tokio::test]
async fn a_device_delivery_row_may_omit_its_export_entirely() {
    // The column is nullable precisely so a delivery can be recorded before an
    // export job exists. A NOT NULL slipped in here would be a regression, and
    // a test that only ever inserts alongside a job cannot see it.
    let tdb = open("m32-07e-nullable").await;
    let db = tdb.db().clone();
    let account = make_account(&db, "device-null@example.test").await;
    let nullable_delivery_id = uuid::Uuid::new_v4();

    // Diagnostic: prove the account is really in THIS database before blaming
    // the delivery insert. A wrong pool or a rolled-back write looks identical
    // to a broken foreign key from the insert's error alone.
    // No MIN() over the id: PostgreSQL has no `min(uuid)` aggregate, and a
    // diagnostic that only runs on one dialect is not a diagnostic.
    let (total, mine): (i64, i64) = on_backend!(tdb, &db, |pool| {
        sqlx::query_as(&q(
            &tdb,
            "SELECT COUNT(*), COALESCE(SUM(CASE WHEN id = ? THEN 1 ELSE 0 END), 0)
             FROM accounts",
            "SELECT COUNT(*), COALESCE(SUM(CASE WHEN id = ?::uuid THEN 1 ELSE 0 END), 0)
             FROM accounts",
        ))
        .bind(account.to_string())
        .fetch_one(pool)
        .await
        .expect("count accounts")
    });
    assert_eq!(
        (total, mine),
        (1, 1),
        "the account must exist before the delivery FK is tested; \
         found {total} account(s), mine={mine}, looked for {account}"
    );

    on_backend!(tdb, &db, |pool| {
        sqlx::query(&q(
            &tdb,
            "INSERT INTO device_deliveries (id, account_id, target, address, state, created_at)
             VALUES (?, ?, 'email', 'someone@example.test', 'failed', ?)",
            "INSERT INTO device_deliveries (id, account_id, target, address, state, created_at)
             VALUES (?::uuid, ?::uuid, 'email', 'someone@example.test', 'failed', ?)",
        ))
        .bind(nullable_delivery_id.to_string())
        .bind(account.to_string())
        .bind("2026-09-25T00:00:00Z")
        .execute(pool)
        .await
        .expect("a delivery with no export must be recordable");
    });

    tdb.cleanup().await;
}

/// A real account row, since `device_deliveries.account_id` cascades from it.
async fn make_account(db: &lorehaven_db::Database, email: &str) -> uuid::Uuid {
    lorehaven_db::identity::create_account(
        db,
        email,
        AgeState::DeclaredAdult,
        AccountStatus::Active,
    )
    .await
    .expect("create account")
    .as_uuid()
}
