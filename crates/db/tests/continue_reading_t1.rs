//! Item 1 of the 100-idea audit: "Continue Reading", on both engines.
//!
//! Spec: `docs/plans/100-ideas-remaining.md` §2. Store: `crates/db/src/continue_reading.rs`.
//!
//! ## Why the fixtures are shaped the way they are
//!
//! Two of the guards in this query are things a naive fixture cannot see:
//!
//!  - **Progress is per-device.** `0004_reading` declares two partial unique indexes, one
//!    on `(account_id, pseud_id, subject_type, subject_id, device_id) WHERE device_id IS
//!    NOT NULL` and one on the same tuple `WHERE device_id IS NULL`. So every fixture
//!    writes a `device_id` explicitly, and
//!    `continue_reading_prefers_the_row_this_reader_wrote_most_recently` puts a phone row
//!    and a laptop row on ONE work with different positions and different `updated_at`,
//!    then asserts the banner shows the newer one.
//!  - **The unfinished test is written per-guard.** A fixture that abandons a work by
//!    reaching 1000 AND by the author completing it cannot tell you which guard did the
//!    work. `*_excludes_*` therefore comes in pairs, each deleting exactly one condition.
//!
//! ## The harness
//!
//! `connect` and `exec` are copied from `reader_surface_t1.rs`, which copied them from
//! `concierge_store.rs`. This crate cannot depend on `test_support`, and the copy is not
//! refactorable: sqlx does **not** translate `?1` into `$1` for PostgreSQL, so an
//! unconverted placeholder reaches the server as the literal token `?` and fails with
//! `operator does not exist: ? uuid`. Every fixture is therefore written twice.

use lorehaven_db::continue_reading::{continue_reading, dnf_works_for, unfinished_works};
use lorehaven_db::Database;
use std::time::Duration;

fn make_config(url: String) -> lorehaven_db::DatabaseConfig {
    lorehaven_db::DatabaseConfig {
        url,
        max_connections: 5,
        acquire_timeout: Duration::from_secs(5),
        slow_query_warn: Duration::ZERO,
    }
}

/// A scratch database on whichever backend `LOREHAVEN_TEST_PG_URL` names.
///
/// **One PostgreSQL SCHEMA per test, not one DATABASE.** The earlier version created a
/// database per test and never dropped it, so the container accumulated scratch databases
/// until it refused connections -- 52 of them after a few runs, after which every test in
/// this file failed at `connect` with an error that said nothing about the real cause.
///
/// A schema is the same isolation for a fraction of the cost, and `search_path` scopes
/// every statement to it. `CREATE DATABASE` also cannot run inside a transaction, which
/// makes cleanup in a test harness awkward; `DROP SCHEMA` can.
///
/// Returns the schema name so the caller can drop it, or `None` on SQLite where the
/// scratch file is already unique per test.
#[must_use]
async fn connect(tag: &str) -> (Database, Option<String>) {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "lorehaven-rs1-{}-{}-{}",
        tag,
        uuid::Uuid::new_v4(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let mut schema = None;
    let url = match std::env::var("LOREHAVEN_TEST_PG_URL") {
        Ok(admin) => {
            // The scratch database is created ONCE and reused. Every test then gets its own
            // schema inside it. Creating a database needs no other session connected to
            // the template, but it cannot be done concurrently by many threads, which is
            // what made a per-test database slow and eventually fatal.
            let name = SCHEMA_DB.to_string();
            let admin_db = Database::connect(&make_config(admin.clone()))
                .await
                .expect("connect to the admin database");
            let exists: Option<i32> =
                sqlx::query_scalar("SELECT 1 FROM pg_database WHERE datname = $1")
                    .bind(&name)
                    .fetch_one(admin_db.postgres_pool().expect("postgres pool"))
                    .await
                    .unwrap_or(None);
            if exists.is_none() {
                // PostgreSQL has no `CREATE DATABASE IF NOT EXISTS`, so this is
                // check-then-create and two threads can both see "absent" and both
                // issue the statement. The loser gets
                //
                //   duplicate key value violates unique constraint "pg_database_datname_index"
                //
                // which is the OUTCOME, not a fault: the database it wanted now exists.
                // Failing here reported a working harness as broken, and it only showed
                // up when two suites happened to run concurrently -- which is why it
                // read as a flake rather than as the race it is.
                //
                // `--test-threads=1` hides it. It does not fix it.
                let created = sqlx::query(&format!("CREATE DATABASE {name}"))
                    .execute(admin_db.postgres_pool().expect("postgres pool"))
                    .await;
                if let Err(e) = created {
                    let still_absent: Option<i32> =
                        sqlx::query_scalar("SELECT 1 FROM pg_database WHERE datname = $1")
                            .bind(&name)
                            .fetch_one(admin_db.postgres_pool().expect("postgres pool"))
                            .await
                            .unwrap_or(None);
                    assert!(
                        still_absent.is_some(),
                        "CREATE DATABASE {name} failed for a reason other than a race: {e}"
                    );
                }
            }
            admin_db.close().await;

            let s = format!(
                "rs1_{}_{}",
                std::process::id(),
                uuid::Uuid::new_v4().simple()
            );
            let (prefix, _) = admin
                .rsplit_once('/')
                .expect("the admin URL ends in a database");
            let scratch = format!("{prefix}/{name}");
            let admin_db = Database::connect(&make_config(scratch.clone()))
                .await
                .expect("connect to the scratch database");
            sqlx::query(&format!("CREATE SCHEMA {s}"))
                .execute(admin_db.postgres_pool().expect("postgres pool"))
                .await
                .expect("create a scratch schema");
            admin_db.close().await;
            schema = Some(s.clone());
            // `options=-csearch_path%3D<schema>` is what scopes the pool: PostgreSQL has no
            // other way to set search_path per connection, and setting it in the SQL would
            // mean qualifying every statement in every query.
            format!("{scratch}?options=-csearch_path%3D{s}")
        }
        Err(_) => format!("sqlite://{}/lorehaven.sqlite?mode=rwc", dir.display()),
    };
    let db = Database::connect(&make_config(url)).await.expect("connect");
    // Migration 0092 runs `CREATE EXTENSION IF NOT EXISTS pgcrypto`, and extensions are
    // DATABASE-scoped while these schemas are not: eight parallel test threads share
    // one `lorehaven_rs1`, so eight migrators reach 0092 together. `IF NOT EXISTS` is
    // NOT race-safe here -- it takes a lock that does not serialise the existence check
    // -- and the losers die with
    //
    //   duplicate key value violates unique constraint "pg_extension_name_index"
    //
    // which names the extension, not the collision. Serialising the migrators is the
    // fix, because the alternative (retrying the migration) re-runs every DDL statement
    // in the file for a failure that has already left the schema correct.
    migrate_serialised(&db).await;
    (db, schema)
}

/// One migrator at a time per process.
///
/// The lock is process-local on purpose: these suites share a scratch DATABASE between
/// threads, so an in-process mutex is exactly the scope of the collision. Two processes
/// against one database would still race, which is a different problem with a different
/// owner.
async fn migrate_serialised(db: &lorehaven_db::Database) {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = LOCK.lock().await;
    db.migrate().await.expect("migrate a scratch schema");
}

/// The shared PostgreSQL database every scratch schema lives in. One per container, not one
/// per test and not one per run.
const SCHEMA_DB: &str = "lorehaven_rs1";

/// Drop a scratch schema. Errors are ignored on purpose: a leftover schema is untidy, and
/// failing a test because cleanup raced something would be worse than the untidiness.
async fn drop_schema(schema: &str) {
    let Ok(admin) = std::env::var("LOREHAVEN_TEST_PG_URL") else {
        return;
    };
    let (prefix, _) = admin
        .rsplit_once('/')
        .expect("the admin URL ends in a database");
    let Ok(db) = Database::connect(&make_config(format!("{prefix}/{SCHEMA_DB}"))).await else {
        return;
    };
    let _ = sqlx::query(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .execute(db.postgres_pool().expect("postgres pool"))
        .await;
    db.close().await;
}

/// One statement, with `?N#u` for a native uuid, `?N#i` for an integer, `?N#b` for a
/// boolean and `?N#t` for text that must NOT be cast.
///
/// `#t` exists because the two engines disagree on which columns are native types.
/// `work_tags.work_id` is uuid on PostgreSQL and TEXT on SQLite, while
/// `taxonomy_nodes.id` and `work_tags.node_id` are text on BOTH. Casting the latter to
/// uuid fails with "cannot cast type text to uuid" -- and it passes on SQLite, so this is
/// invisible until the PostgreSQL run.
async fn exec(db: &Database, tmpl: &str, args: &[&str]) {
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            let sql = tmpl
                .replace("#u", "")
                .replace("#i", "")
                .replace("#b", "")
                .replace("#t", "");
            let mut q = sqlx::query(&sql);
            for a in args {
                q = q.bind(*a);
            }
            q.execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("fixture insert");
        }
        lorehaven_db::Backend::Postgres => {
            // THREE markers, not two. `#u` is a uuid and `#i` a bigint, and until this
            // was third there was no way to write a boolean -- so `bookmarks.is_public`,
            // which is INTEGER on SQLite and BOOLEAN on PostgreSQL, was bound through
            // `#i` and PostgreSQL answered 42804 "column is of type boolean but
            // expression is of type bigint". SQLite took it silently, so the fixture
            // worked on one engine and failed on the other, which is the shape of bug
            // that only shows up in the two-engine run.
            //
            // The bind follows the ARGUMENT, not the column: `0`/`1` become `false`/
            // `true` on PostgreSQL, and a real uuid string parses as a uuid.
            let pg = (1..=8).fold(tmpl.to_string(), |acc, n| {
                acc.replace(&format!("?{n}#b"), &format!("${n}::boolean"))
                    .replace(&format!("?{n}#u"), &format!("${n}::uuid"))
                    .replace(&format!("?{n}#i"), &format!("${n}::bigint"))
                    .replace(&format!("?{n}#t"), &format!("${n}::text"))
                    .replace(&format!("?{n}"), &format!("${n}"))
            });
            // The bind follows the MARKER in the template, not the value. Binding by
            // value was the previous version and it is wrong: `work_tags.weight` is bigint
            // and its fixture argument is the string "0", so a value-keyed rule bound a
            // boolean to a bigint column and PostgreSQL answered 42846 "cannot cast type
            // boolean to bigint". Every test using `tag()` failed, and SQLite accepted all
            // of them, so the defect was invisible until the PostgreSQL run.
            let mut q = sqlx::query(&pg);
            let mut n = 0usize;
            for a in args {
                n += 1;
                let marker_is_bool = tmpl.contains(&format!("?{n}#b"));
                if marker_is_bool {
                    q = q.bind(*a == "1");
                } else if uuid::Uuid::parse_str(a).is_ok() {
                    q = q.bind(uuid::Uuid::parse_str(a).expect("re-parsed above"));
                } else {
                    q = q.bind(*a);
                }
            }
            q.execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("fixture insert");
        }
    }
}

const NOW: &str = "2026-10-04 12:00:00";

/// `accounts` has no `handle` — the handle lives on `pseuds`, and ownership belongs to
/// the PSEUD (ADR 0003). Every fixture here was written against the wrong shape first:
/// `accounts.handle`, `works.account_id` and `works.slug` do not exist, and the error
/// names a column the reader assumed was there.
async fn account(db: &Database, tag: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    exec(
        db,
        "INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?1#u, ?2, ?3, ?3)",
        &[
            &id,
            &format!("{tag}-{}@example.test", uuid::Uuid::new_v4().simple()),
            NOW,
        ],
    )
    .await;
    id
}

async fn pseud(db: &Database, account_id: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    exec(
        db,
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
         VALUES (?1#u, ?2#u, ?3, ?3, ?4, ?4)",
        &[&id, account_id, &format!("p{}", &id[..8]), NOW],
    )
    .await;
    id
}

async fn work(
    db: &Database,
    account_id: &str,
    pseud_id: &str,
    title: &str,
    completion: &str,
) -> String {
    let _ = account_id;
    let id = uuid::Uuid::new_v4().to_string();
    exec(db,
        "INSERT INTO works (id, owner_pseud_id, title, lifecycle, completion, published_at, created_at, updated_at)
         VALUES (?1#u, ?2#u, ?3, 'published', ?4, ?5, ?5, ?5)",
        &[&id, pseud_id, title, completion, NOW]).await;
    id
}

/// A work whose id is FIXED, so tests that depend on ordering are deterministic.
///
/// Mutation testing found this: `returns_the_work_the_reader_last_touched` asserts which
/// of two works the banner shows, and deleting `ORDER BY rp.updated_at DESC` left the
/// order to be the random uuid order -- which passed about half the time. The mutation
/// was reported as a survivor when it was really a test that could not fail. Ordering
/// assertions need ids whose sort order is known, so these are sequential and fixed.
async fn fixed_work(
    db: &Database,
    account_id: &str,
    pseud_id: &str,
    seq: u32,
    title: &str,
    completion: &str,
) -> String {
    let _ = account_id;
    let id = format!("00000000-0000-4000-8000-{seq:012}");
    exec(db,
        "INSERT INTO works (id, owner_pseud_id, title, lifecycle, completion, published_at, created_at, updated_at)
         VALUES (?1#u, ?2#u, ?3, 'published', ?4, ?5, ?5, ?5)",
        &[&id, pseud_id, title, completion, NOW]).await;
    id
}

/// A progress row. `at` is the timestamp of the write, and it is what the banner orders by.
async fn progress(
    db: &Database,
    account_id: &str,
    pseud_id: &str,
    work_id: &str,
    device: &str,
    per_mille: i32,
    at: &str,
) {
    progress_at(
        db,
        ProgressRow {
            account_id,
            pseud_id,
            work_id,
            device,
            per_mille,
            at,
            chapter_id: None,
        },
    )
    .await;
}

/// The same row, naming the chapter the reader was on.
///
/// TWO templates rather than one with a nullable bind. `exec` takes `&[&str]`, so it
/// cannot express SQL NULL, and passing an empty string for "no chapter" would insert
/// `''` into a column that is UUID on PostgreSQL -- `22P02 invalid input syntax for type
/// uuid` there, while SQLite stores the empty string happily. NULL therefore has to be
/// literal SQL text, which means the branch is in the template rather than the argument.
struct ProgressRow<'a> {
    account_id: &'a str,
    pseud_id: &'a str,
    work_id: &'a str,
    device: &'a str,
    per_mille: i32,
    at: &'a str,
    chapter_id: Option<&'a str>,
}

/// A struct, not eight parameters.
///
/// Clippy refuses more than seven, and it is right to: `device` and `at` are adjacent
/// `&str`s that mean different things, and every caller of the positional version had to
/// read the signature to get them the right way round. Named fields cannot be swapped.
async fn progress_at(db: &Database, row: ProgressRow<'_>) {
    let ProgressRow {
        account_id,
        pseud_id,
        work_id,
        device,
        per_mille,
        at,
        chapter_id,
    } = row;
    let row_id = uuid::Uuid::new_v4().to_string();
    let per_mille = per_mille.to_string();
    let (tmpl, args): (&str, Vec<&str>) = match chapter_id {
        None => (
            "INSERT INTO reading_progress
               (id, account_id, pseud_id, subject_type, subject_id,
                position_permille, device_id, created_at, updated_at)
             VALUES (?1#u, ?2#u, ?3#u, 'work', ?4#u, ?5#i, ?6, ?7, ?7)",
            vec![
                &row_id, account_id, pseud_id, work_id, &per_mille, device, at,
            ],
        ),
        Some(chapter) => (
            "INSERT INTO reading_progress
               (id, account_id, pseud_id, subject_type, subject_id, chapter_id,
                position_permille, device_id, created_at, updated_at)
             VALUES (?1#u, ?2#u, ?3#u, 'work', ?4#u, ?5#u, ?6#i, ?7, ?8, ?8)",
            vec![
                &row_id, account_id, pseud_id, work_id, chapter, &per_mille, device, at,
            ],
        ),
    };
    exec(db, tmpl, &args).await;
}

async fn chapter(db: &Database, work_id: &str, title: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    exec(
        db,
        "INSERT INTO chapters (id, work_id, order_key, title, created_at, updated_at)
         VALUES (?1#u, ?2#u, 10, ?3, ?4, ?4)",
        &[&id, work_id, title, NOW],
    )
    .await;
    id
}

// ---------------------------------------------------------------------------

#[tokio::test]
async fn returns_none_when_the_reader_has_read_nothing() {
    let (db, schema) = connect("empty").await;
    let a = account(&db, "empty").await;
    assert!(continue_reading(&db, &a).await.expect("query").is_none());
    if let Some(s) = schema {
        drop_schema(&s).await;
    }
}

#[tokio::test]
async fn returns_the_work_the_reader_last_touched() {
    let (db, schema) = connect("recent").await;
    let a = account(&db, "recent").await;
    let p = pseud(&db, &a).await;
    // FIXED ids, and the older work deliberately gets the LOWER id. Dropping
    // `ORDER BY rp.updated_at DESC` then makes the older work win, so the assertion below
    // is actually reachable. With random uuids it passed about half the time, which is a
    // test that cannot fail -- see `fixed_work`.
    let older = fixed_work(&db, &a, &p, 1, "Older Unfinished", "in_progress").await;
    let newer = fixed_work(&db, &a, &p, 2, "Newer Unfinished", "in_progress").await;
    progress(&db, &a, &p, &older, "laptop", 400, "2026-10-01T10:00:00Z").await;
    progress(&db, &a, &p, &newer, "laptop", 200, "2026-10-03T10:00:00Z").await;

    let got = continue_reading(&db, &a)
        .await
        .expect("query")
        .expect("a row");
    assert_eq!(got.work_id, newer, "must be the most recently updated work");
    assert_eq!(got.title, "Newer Unfinished");
    assert_eq!(got.position_permille, 200);
    assert_eq!(got.percent(), 20);
    if let Some(s) = schema {
        drop_schema(&s).await;
    }
}

/// The per-device collapse, which is the whole reason this query groups before ordering.
#[tokio::test]
async fn prefers_the_row_this_reader_wrote_most_recently() {
    let (db, schema) = connect("device").await;
    let a = account(&db, "device").await;
    let p = pseud(&db, &a).await;
    let w = fixed_work(&db, &a, &p, 3, "Read On Two Devices", "in_progress").await;
    let phone_chapter = chapter(&db, &w, "Chapter 1: The Phone Stop").await;
    let laptop_chapter = chapter(&db, &w, "Chapter 9: The Laptop Stop").await;
    // The phone row is OLDER but FURTHER ALONG; the laptop row is newer and behind.
    // Ordering by `updated_at` must win over ordering by position, or the banner jumps
    // between devices every time a sync lands.
    progress_at(
        &db,
        ProgressRow {
            account_id: &a,
            pseud_id: &p,
            work_id: &w,
            device: "phone",
            per_mille: 900,
            at: "2026-10-01T10:00:00Z",
            chapter_id: Some(&phone_chapter),
        },
    )
    .await;
    progress_at(
        &db,
        ProgressRow {
            account_id: &a,
            pseud_id: &p,
            work_id: &w,
            device: "laptop",
            per_mille: 150,
            at: "2026-10-03T10:00:00Z",
            chapter_id: Some(&laptop_chapter),
        },
    )
    .await;

    let got = continue_reading(&db, &a)
        .await
        .expect("query")
        .expect("a row");
    assert_eq!(got.position_permille, 150, "the newer device's position");
    assert_eq!(got.updated_at, "2026-10-03T10:00:00Z");
    assert_eq!(
        got.chapter_title.as_deref(),
        Some("Chapter 9: The Laptop Stop"),
        "the chapter must belong to the row that supplied the position"
    );
    // The chapter travels with the POSITION, not with whichever row happened to carry
    // one. Aggregating the columns independently let the position come from one device
    // row and the chapter from another, which is how the banner ends up claiming the
    // reader is 90% through while quoting chapter 1.
    if let Some(s) = schema {
        drop_schema(&s).await;
    }
}

#[tokio::test]
async fn resolves_the_chapter_title_the_reader_stopped_at() {
    let (db, schema) = connect("chapter").await;
    let a = account(&db, "chapter").await;
    let p = pseud(&db, &a).await;
    let w = work(&db, &a, &p, "Has Chapters", "in_progress").await;
    let c = chapter(&db, &w, "Chapter 7: The Turn").await;
    exec(&db,
        "INSERT INTO reading_progress
           (id, account_id, pseud_id, subject_type, subject_id, chapter_id, position_permille, device_id, created_at, updated_at)
         VALUES (?1#u, ?2#u, ?3#u, 'work', ?4#u, ?5#u, 333, 'laptop', ?6, ?6)",
        &[&uuid::Uuid::new_v4().to_string(), &a, &p, &w, &c, NOW]).await;

    let got = continue_reading(&db, &a)
        .await
        .expect("query")
        .expect("a row");
    assert_eq!(got.chapter_title.as_deref(), Some("Chapter 7: The Turn"));
    assert_eq!(got.position_permille, 333);
    if let Some(s) = schema {
        drop_schema(&s).await;
    }
}

/// A deleted chapter must cost the banner its chapter NAME, not the banner.
#[tokio::test]
async fn a_missing_chapter_does_not_fail_the_banner() {
    let (db, schema) = connect("nochapt").await;
    let a = account(&db, "nochapt").await;
    let p = pseud(&db, &a).await;
    let w = work(&db, &a, &p, "Chapter Was Deleted", "in_progress").await;
    // No `chapters` row exists for this id at all.
    exec(&db,
        "INSERT INTO reading_progress
           (id, account_id, pseud_id, subject_type, subject_id, chapter_id, position_permille, device_id, created_at, updated_at)
         VALUES (?1#u, ?2#u, ?3#u, 'work', ?4#u, ?5#u, 500, 'laptop', ?6, ?6)",
        &[&uuid::Uuid::new_v4().to_string(), &a, &p, &w, &uuid::Uuid::new_v4().to_string(), NOW]).await;

    let got = continue_reading(&db, &a)
        .await
        .expect("query")
        .expect("a row");
    assert_eq!(
        got.chapter_title, None,
        "a deleted chapter costs the name, not the banner"
    );
    assert!(got.chapter_id.is_some(), "the chapter id is still reported");
    if let Some(s) = schema {
        drop_schema(&s).await;
    }
}

/// Guard one of two: the reader reached the end.
#[tokio::test]
async fn excludes_a_work_the_reader_finished() {
    let (db, schema) = connect("done").await;
    let a = account(&db, "done").await;
    let p = pseud(&db, &a).await;
    let finished = work(&db, &a, &p, "Reader Finished This", "in_progress").await;
    let open = work(&db, &a, &p, "Still Going", "in_progress").await;
    // The author has NOT marked `finished` complete, so only `position_permille < 1000`
    // can exclude it. That is what makes this the single-guard test.
    progress(
        &db,
        &a,
        &p,
        &finished,
        "laptop",
        1000,
        "2026-10-03T10:00:00Z",
    )
    .await;
    progress(&db, &a, &p, &open, "laptop", 100, "2026-10-01T10:00:00Z").await;

    let got = continue_reading(&db, &a)
        .await
        .expect("query")
        .expect("a row");
    assert_eq!(
        got.work_id, open,
        "a finished work must not be offered to continue"
    );
    if let Some(s) = schema {
        drop_schema(&s).await;
    }
}

/// Guard two of two: the author completed it.
#[tokio::test]
async fn excludes_a_work_the_author_completed() {
    let (db, schema) = connect("cdone").await;
    let a = account(&db, "cdone").await;
    let p = pseud(&db, &a).await;
    // The reader stopped at 200 per-mille, so only `completion <> 'complete'` excludes it.
    let authored_done = work(&db, &a, &p, "Author Finished It", "complete").await;
    let open = work(&db, &a, &p, "Author Still Writing", "in_progress").await;
    progress(
        &db,
        &a,
        &p,
        &authored_done,
        "laptop",
        200,
        "2026-10-03T10:00:00Z",
    )
    .await;
    progress(&db, &a, &p, &open, "laptop", 100, "2026-10-01T10:00:00Z").await;

    let got = continue_reading(&db, &a)
        .await
        .expect("query")
        .expect("a row");
    assert_eq!(got.work_id, open);
    if let Some(s) = schema {
        drop_schema(&s).await;
    }
}

#[tokio::test]
async fn excludes_an_unpublished_draft() {
    let (db, schema) = connect("draft").await;
    let a = account(&db, "draft").await;
    let p = pseud(&db, &a).await;
    let draft = uuid::Uuid::new_v4().to_string();
    exec(&db,
        "INSERT INTO works (id, owner_pseud_id, title, lifecycle, completion, created_at, updated_at)
         VALUES (?1#u, ?2#u, 'An Unpublished Draft', 'draft', 'in_progress', ?3, ?3)",
        &[&draft, &p, NOW]).await;
    let published = work(&db, &a, &p, "Published", "in_progress").await;
    progress(&db, &a, &p, &draft, "laptop", 300, "2026-10-03T10:00:00Z").await;
    progress(
        &db,
        &a,
        &p,
        &published,
        "laptop",
        100,
        "2026-10-01T10:00:00Z",
    )
    .await;

    let got = continue_reading(&db, &a)
        .await
        .expect("query")
        .expect("a row");
    assert_eq!(
        got.work_id, published,
        "a draft the reader cannot open is not continuable"
    );
    if let Some(s) = schema {
        drop_schema(&s).await;
    }
}

#[tokio::test]
async fn never_shows_another_readers_work() {
    let (db, schema) = connect("scope").await;
    let mine = account(&db, "mine").await;
    let theirs = account(&db, "theirs").await;
    let p = pseud(&db, &theirs).await;
    let w = work(&db, &theirs, &p, "Someone Elses Book", "in_progress").await;
    progress(&db, &theirs, &p, &w, "laptop", 500, "2026-10-03T10:00:00Z").await;

    assert!(
        continue_reading(&db, &mine).await.expect("query").is_none(),
        "a banner showing another reader's library is a privacy leak"
    );
    if let Some(s) = schema {
        drop_schema(&s).await;
    }
}

#[tokio::test]
async fn unfinished_works_ranks_by_recency_and_is_bounded() {
    let (db, schema) = connect("many").await;
    let a = account(&db, "many").await;
    let p = pseud(&db, &a).await;
    for (i, day) in ["2026-10-01", "2026-10-03", "2026-10-02"]
        .iter()
        .enumerate()
    {
        let w = work(&db, &a, &p, &format!("Serial {i}"), "in_progress").await;
        progress(
            &db,
            &a,
            &p,
            &w,
            "laptop",
            100 * (i as i32 + 1),
            &format!("{day}T10:00:00Z"),
        )
        .await;
    }

    let all = unfinished_works(&db, &a, 10).await.expect("query");
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].title, "Serial 1", "the 10-03 work is most recent");
    assert_eq!(all[1].title, "Serial 2");
    assert_eq!(all[2].title, "Serial 0");

    // A bound from a query string must be enforced here, not trusted from the caller.
    assert_eq!(unfinished_works(&db, &a, 2).await.expect("query").len(), 2);
    assert_eq!(
        unfinished_works(&db, &a, 0).await.expect("query").len(),
        1,
        "0 clamps to 1"
    );
    assert_eq!(
        unfinished_works(&db, &a, 9999).await.expect("query").len(),
        3
    );
    if let Some(s) = schema {
        drop_schema(&s).await;
    }
}

/// The DNF list item 11's mark feeds, with the reason kept so the caller can decide.
#[tokio::test]
async fn dnf_works_for_returns_only_shared_marks_with_their_reason() {
    let (db, schema) = connect("dnf").await;
    let a = account(&db, "dnf").await;
    let p = pseud(&db, &a).await;
    let shared = work(&db, &a, &p, "Shared Reason", "in_progress").await;
    let priv_row = work(&db, &a, &p, "Private Reason", "in_progress").await;
    let gone = work(&db, &a, &p, "Soft Deleted", "in_progress").await;

    // Every column is bound, and only `reason` and `deleted_at` are literals. `is_public`
    // binds through `#b` because it is BOOLEAN on PostgreSQL and INTEGER on SQLite: a
    // literal `'1'` in the SQL text is an integer expression there and the insert is
    // refused with 42804, while SQLite takes it silently -- so the fixture would pass on
    // the engine nobody deploys.
    // `work_id` and the public flag are NOT parameters here: the work id is bound as `?4`
    // by the caller, and `is_public` is the `?5#b` marker -- so a closure parameter for
    // either would be a second, unbindable source of the same value.
    let insert_dnf = |reason: &str, deleted_at: Option<&str>| {
        format!(
            "INSERT INTO did_not_finish
               (id, account_id, pseud_id, work_id, reason, is_public, created_at, updated_at, deleted_at)
             VALUES (?1#u, ?2#u, ?3#u, ?4#u, '{reason}', ?5#b, '{NOW}', '{NOW}', {ts})",
            ts = deleted_at.map_or_else(|| "NULL".to_string(), |d| format!("'{d}'")),
        )
    };
    let ids = || uuid::Uuid::new_v4().to_string();
    // The fifth argument is the `is_public` flag, as "1"/"0": the harness binds a `#b`
    // marker to a boolean, and it keys on the MARKER rather than on the value. A marker
    // with no fifth argument binds nothing at all.
    exec(
        &db,
        &insert_dnf("slow_pacing", None),
        &[&ids(), &a, &p, &shared, "1"],
    )
    .await;
    exec(
        &db,
        &insert_dnf("triggering", None),
        &[&ids(), &a, &p, &priv_row, "0"],
    )
    .await;
    exec(
        &db,
        &insert_dnf("not_my_taste", Some("2026-10-04 00:00:00")),
        &[&ids(), &a, &p, &gone, "1"],
    )
    .await;

    let rows = dnf_works_for(&db, &a).await.expect("query");
    assert_eq!(
        rows.len(),
        1,
        "private and soft-deleted marks must not appear"
    );
    assert_eq!(rows[0].0, shared);
    assert_eq!(rows[0].1, "slow_pacing", "the reason travels with the id");
    if let Some(s) = schema {
        drop_schema(&s).await;
    }
}

/// `subject_type` really is load-bearing, proven by a row that is NOT a work.
///
/// Without this, deleting `AND rp.subject_type = 'work'` changed no result: every fixture
/// row was already `'work'`, so the guard was vacuous and the mutation survived. This
/// row points at the same work id under `subject_type = 'author'` and carries the newest
/// timestamp of all, so a query without the guard selects it and reports a position the
/// reader never set.
#[tokio::test]
async fn a_progress_row_for_another_subject_type_is_not_a_work() {
    let (db, schema) = connect("subjtype").await;
    let a = account(&db, "subjtype").await;
    let p = pseud(&db, &a).await;
    let w = fixed_work(&db, &a, &p, 1, "A Real Work", "in_progress").await;
    progress(&db, &a, &p, &w, "laptop", 250, "2026-10-02T10:00:00Z").await;

    exec(&db,
        "INSERT INTO reading_progress
           (id, account_id, pseud_id, subject_type, subject_id, position_permille, device_id, created_at, updated_at)
         VALUES (?1#u, ?2#u, ?3#u, 'author', ?4#u, 999, 'laptop', '2026-10-03T10:00:00Z', '2026-10-03T10:00:00Z')",
        &[&uuid::Uuid::new_v4().to_string(), &a, &p, &w]).await;

    let got = continue_reading(&db, &a)
        .await
        .expect("query")
        .expect("a row");
    assert_eq!(
        got.position_permille, 250,
        "a progress row for a non-work subject must not be served as reading progress"
    );
    if let Some(s) = schema {
        drop_schema(&s).await;
    }
}
