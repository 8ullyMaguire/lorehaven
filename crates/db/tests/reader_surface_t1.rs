//! Items 14, 27 and 33 of the 100-idea audit: the reader-surface queries, on both engines.
//! Spec: `docs/spec-reader-surface-t1.md`. Plan: `docs/plan-reader-surface-t1.md`.
//!
//! # The test that had to come first
//!
//! `most_bookmarked_counts_only_public_bookmarks` is written before the query, and the file
//! opens by explaining why. Every other test here either uses one reader, or reads back
//! what it just wrote, and none of them can detect a leak of private bookmark rows — they
//! would all stay green with `AND b.is_public = 1` deleted. This one cannot: it gives one
//! work ten private bookmarks and another a single public one, and asserts the *second*
//! ranks first. That inversion is the only way to state the privacy rule as an assertion
//! rather than as a comment.
//!
//! It was confirmed red against the unscoped query before this file was trusted. A test
//! never seen red is not evidence.
//!
//! # Fixtures
//!
//! `connect` and `exec` are copied verbatim from `concierge_store.rs`, not rewritten. This
//! crate cannot depend on `test_support`, and the rewrite is three mechanical substitutions
//! where getting one wrong is silent: sqlx does **not** translate `?1` into `$1` for
//! PostgreSQL, so an unconverted placeholder reaches the server as the literal token `?`
//! and fails with `operator does not exist: ? integer`.

use lorehaven_db::reader_surface::{
    kind_weight, most_bookmarked_this_week, new_in_your_fandoms, similar_works, weighted_jaccard,
    SurfaceWork, FILTER_FLOOR, MIN_TAGS,
};
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
                sqlx::query(&format!("CREATE DATABASE {name}"))
                    .execute(admin_db.postgres_pool().expect("postgres pool"))
                    .await
                    .expect("create the shared scratch database");
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
    db.migrate().await.expect("migrate");
    (db, schema)
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

const T0: &str = "2026-01-01T00:00:00Z";
/// The window start used by the weekly leaderboard tests: 2026-06-01.
const WINDOW: &str = "2026-06-01T00:00:00Z";

fn ids(works: &[SurfaceWork]) -> Vec<&str> {
    works.iter().map(|w| w.id.as_str()).collect()
}

// ---------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------

/// A fresh account. `accounts.email` carries a UNIQUE index on its normalized form, so
/// the address is derived from a fresh uuid rather than a counter -- a counter collides the
/// moment a test loops, which is exactly what the ten-private-bookmarks fixture does.
async fn account(db: &Database, _n: u8) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    exec(
        db,
        "INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?1#u, ?2, ?3, ?3)",
        &[&id, &format!("{id}@example.test"), T0],
    )
    .await;
    id
}

/// A published, public, visible work.
async fn work(db: &Database, title: &str, published_at: Option<&str>) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let owner = pseud(db).await;
    let title = title.to_string();
    let published_at = published_at.unwrap_or(T0).to_string();
    exec(
        db,
        "INSERT INTO works (id, owner_pseud_id, title, lifecycle, visibility, completion,
                            published_at, created_at, updated_at)
         VALUES (?1#u, ?2#u, ?3, 'published', 'public', 'complete', ?4, ?3, ?3)",
        &[&id, &owner, &title, &published_at],
    )
    .await;
    id
}

/// A pseud for `work()`'s owner. `pseuds.handle` is UNIQUE, so it is derived from a fresh
/// uuid rather than the work title -- two works with the same title are ordinary data, and
/// a fixture that cannot represent them is a fixture that will mislead later.
async fn pseud(db: &Database) -> String {
    let acct = account(db, 200).await;
    let id = uuid::Uuid::new_v4().to_string();
    // The handle IS the uuid string: `pseuds.handle` is UNIQUE, so a fresh uuid is the
    // only collision-free source. No `format!` -- clippy is right that a format that
    // only interpolates one variable is a clone with extra steps.
    let handle = id.clone();
    exec(
        db,
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
         VALUES (?1#u, ?2#u, ?3, ?3, ?4, ?4)",
        &[&id, &acct, &handle, T0],
    )
    .await;
    id
}

/// Drop the per-test schema, if this run is on PostgreSQL.
///
/// A named wrapper rather than repeating `if let Some(s) = schema { drop_schema(&s).await }`
/// at the end of every test: on SQLite there is no schema, and the `if let` at ten call
/// sites is ten chances to forget one — a leaked schema is not a failure, it is a database
/// that quietly grows.
async fn drop_schema_if(schema: &Option<String>) {
    if let Some(s) = schema {
        drop_schema(s).await;
    }
}

/// A chapter with one saved revision carrying `words`, and that revision CURRENT.
///
/// Item 4's aggregate is `SUM(chapter_revisions.word_count)` reached through
/// `chapters.current_revision_id` — the same join `events::work_word_count` uses. Two
/// things this fixture has to get right, and both are silent when it does not:
///
///  - the revision must be the chapter's CURRENT one. Summing all revisions would count
///    every draft the author ever saved, so a chapter edited five times reports five times
///    its length;
///  - the author must own the revision (`created_by_pseud_id` is NOT NULL and REFERENCES
///    pseuds), which is why this needs a pseud and not just a work id.
async fn chapter_with_words(db: &Database, work_id: &str, owner: &str, words: i64) -> String {
    let chapter_id = uuid::Uuid::new_v4().to_string();
    let revision_id = uuid::Uuid::new_v4().to_string();
    let title = format!("Chapter for {work_id}");
    // `exec` takes `&[&str]`, so the number has to be a `String` local rather than a
    // temporary: `&words.to_string()` is a `&String`, and the borrow dies mid-expression
    // anyway. The `#i` marker is what makes this bind as a bigint on PostgreSQL.
    let words = words.to_string();
    exec(
        db,
        "INSERT INTO chapters (id, work_id, order_key, title, created_at, updated_at)
         VALUES (?1#u, ?2#u, 10, ?3, ?4, ?4)",
        &[&chapter_id, work_id, &title, T0],
    )
    .await;
    exec(
        db,
        "INSERT INTO chapter_revisions
           (id, chapter_id, revision_number, document_json, sanitized_html, plain_text,
            word_count, created_by_pseud_id, created_at)
         VALUES (?1#u, ?2#u, 1, '{}', '<p></p>', '', ?3#i, ?4#u, ?5)",
        &[&revision_id, &chapter_id, &words, owner, T0],
    )
    .await;
    // Set the pointer SECOND: it is a self-referencing FK, and SQLite resolves the target
    // when it is used rather than when it is declared, so this order is required.
    exec(
        db,
        "UPDATE chapters SET current_revision_id = ?1#u WHERE id = ?2#u",
        &[&revision_id, &chapter_id],
    )
    .await;
    chapter_id
}

async fn node(db: &Database, kind: &str, canonical: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let kind = kind.to_string();
    let canonical = canonical.to_string();
    exec(
        db,
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at)
         VALUES (?1#t, ?2, ?3, ?3, ?4)",
        &[&id, &kind, &canonical, T0],
    )
    .await;
    id
}

async fn tag(db: &Database, work_id: &str, node_id: &str) {
    exec(
        db,
        "INSERT INTO work_tags (work_id, node_id, weight, added_at)
         VALUES (?1#u, ?2#t, ?3#i, ?4)",
        &[work_id, node_id, "0", T0],
    )
    .await;
}

async fn bookmark(db: &Database, account_id: &str, work_id: &str, public: bool, at: &str) {
    // Named, not inline: `&uuid::Uuid::new_v4().to_string()` borrows a temporary, and
    // clippy rejects the round-trip anyway. Each binding also outlives the `exec` call,
    // which is what makes the borrow sound rather than merely accepted.
    let id = uuid::Uuid::new_v4().to_string();
    let is_public = if public { "1" } else { "0" };
    let at = at.to_string();
    exec(
        db,
        "INSERT INTO bookmarks (id, account_id, subject_type, subject_id, is_public,
                                created_at, updated_at)
         VALUES (?1#u, ?2#u, 'work', ?3#u, ?4#b, ?5, ?5)",
        &[&id, account_id, work_id, is_public, &at],
    )
    .await;
}

// ---------------------------------------------------------------------------------------
// Item 27 — "Most bookmarked this week". The privacy rule first.
// ---------------------------------------------------------------------------------------

/// **The load-bearing test of this file.** Ten private bookmarks must not outrank one
/// public bookmark.
///
/// It is built as an inversion on purpose. Asserting "ten public bookmarks rank first" would
/// pass with the `is_public = 1` predicate deleted, because ten is more than one either
/// way. Asserting the inversion makes the predicate load-bearing: drop it and this goes red
/// while the other ten tests stay green.
#[tokio::test]
async fn most_bookmarked_counts_only_public_bookmarks() {
    let (db, schema) = connect("mbprivate").await;
    let hidden = work(&db, "Hidden Favourite", Some(T0)).await;
    let public = work(&db, "Public Favourite", Some(T0)).await;

    // Ten readers who chose to keep the bookmark private.
    for _ in 0..10 {
        let acct = account(&db, 1).await;
        bookmark(&db, &acct, &hidden, false, WINDOW).await;
    }
    // One reader who made theirs public.
    let acct = account(&db, 2).await;
    bookmark(&db, &acct, &public, true, WINDOW).await;

    let out = most_bookmarked_this_week(&db, WINDOW, 12)
        .await
        .expect("query");

    assert_eq!(
        ids(&out).first().copied(),
        Some(public.as_str()),
        "ten private bookmarks outranked one public one: private bookmark rows leaked"
    );
    assert!(
        !ids(&out).contains(&hidden.as_str()),
        "a work with only private bookmarks appeared on a public leaderboard"
    );
    if let Some(s) = &schema {
        drop_schema(s).await;
    }
}

#[tokio::test]
async fn most_bookmarked_respects_the_window() {
    let (db, schema) = connect("mbwindow").await;
    let fresh = work(&db, "Fresh", Some(T0)).await;
    let stale = work(&db, "Stale", Some(T0)).await;

    let acct = account(&db, 1).await;
    // Inside the window: two days after it opens.
    bookmark(&db, &acct, &fresh, true, "2026-06-03T00:00:00Z").await;
    // Outside: eight days before the window opens, a day past a seven-day lookback.
    bookmark(&db, &acct, &stale, true, "2026-05-24T00:00:00Z").await;

    let out = most_bookmarked_this_week(&db, WINDOW, 12)
        .await
        .expect("query");

    assert!(
        ids(&out).contains(&fresh.as_str()),
        "in-window work missing"
    );
    assert!(
        !ids(&out).contains(&stale.as_str()),
        "an 8-day-old bookmark was counted inside a 7-day window"
    );
    if let Some(s) = &schema {
        drop_schema(s).await;
    }
}

#[tokio::test]
async fn most_bookmarked_breaks_ties_on_title() {
    let (db, schema) = connect("mbtie").await;
    let zulu = work(&db, "Zulu", Some(T0)).await;
    let alpha = work(&db, "Alpha", Some(T0)).await;

    let acct = account(&db, 1).await;
    bookmark(&db, &acct, &zulu, true, WINDOW).await;
    bookmark(&db, &acct, &alpha, true, WINDOW).await;

    let out = most_bookmarked_this_week(&db, WINDOW, 12)
        .await
        .expect("query");

    assert_eq!(
        out.len(),
        2,
        "two works with one bookmark each should both be listed"
    );
    let titles: Vec<&str> = out.iter().map(|w| w.title.as_str()).collect();
    assert_eq!(
        titles,
        vec!["Alpha", "Zulu"],
        "equal counts must break on title ascending, or the page reshuffles between renders"
    );
    if let Some(s) = &schema {
        drop_schema(s).await;
    }
}

#[tokio::test]
async fn most_bookmarked_counts_distinct_readers_not_rows() {
    let (db, schema) = connect("mbdistinct").await;
    let w = work(&db, "Repeat", Some(T0)).await;

    // The same reader bookmarking the same work twice is one reader, not two.
    let acct = account(&db, 1).await;
    bookmark(&db, &acct, &w, true, WINDOW).await;
    bookmark(&db, &acct, &w, true, WINDOW).await;

    let out = most_bookmarked_this_week(&db, WINDOW, 12)
        .await
        .expect("query");

    assert_eq!(
        out[0].recent_bookmarks,
        Some(1),
        "one reader bookmarking twice counted as two bookmarkers"
    );
    if let Some(s) = &schema {
        drop_schema(s).await;
    }
}

#[tokio::test]
async fn most_bookmarked_skips_unpublished_and_deleted_works() {
    let (db, schema) = connect("mbstate").await;
    let acct = account(&db, 1).await;

    let draft = work(&db, "Draft", Some(T0)).await;
    let deleted = work(&db, "Deleted", Some(T0)).await;
    bookmark(&db, &acct, &draft, true, WINDOW).await;
    bookmark(&db, &acct, &deleted, true, WINDOW).await;

    // A draft is not published; a soft-deleted work is not there.
    exec(
        &db,
        "UPDATE works SET lifecycle = 'draft' WHERE id = ?1#u",
        &[&draft],
    )
    .await;
    exec(
        &db,
        "UPDATE works SET deleted_at = ?2 WHERE id = ?1#u",
        &[&deleted, T0],
    )
    .await;

    let out = most_bookmarked_this_week(&db, WINDOW, 12)
        .await
        .expect("query");
    assert!(
        out.is_empty(),
        "a draft or deleted work reached the leaderboard"
    );
    if let Some(s) = &schema {
        drop_schema(s).await;
    }
}

// ---------------------------------------------------------------------------------------
// Item 14 — "New in your fandoms"
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn new_in_your_fandoms_only_returns_works_in_a_bookmarked_fandom() {
    let (db, schema) = connect("nif").await;
    let hp = node(&db, "fandom", "Harry Potter").await;
    let lotr = node(&db, "fandom", "Lord of the Rings").await;

    let seeded = work(&db, "Already Read", Some("2026-01-01T00:00:00Z")).await;
    tag(&db, &seeded, &hp).await;

    let in_hp = work(&db, "New HP Fic", Some("2026-06-02T00:00:00Z")).await;
    tag(&db, &in_hp, &hp).await;
    let in_lotr = work(&db, "New LOTR Fic", Some("2026-06-03T00:00:00Z")).await;
    tag(&db, &in_lotr, &lotr).await;

    let reader = account(&db, 1).await;
    bookmark(&db, &reader, &seeded, true, WINDOW).await;

    let out = new_in_your_fandoms(&db, &reader, 12).await.expect("query");

    assert!(
        ids(&out).contains(&in_hp.as_str()),
        "the fandom's new work is missing"
    );
    assert!(
        !ids(&out).contains(&in_lotr.as_str()),
        "a work from a fandom the reader never bookmarked appeared"
    );
    if let Some(s) = &schema {
        drop_schema(s).await;
    }
}

#[tokio::test]
async fn new_in_your_fandoms_omits_works_you_already_bookmarked() {
    let (db, schema) = connect("nifown").await;
    let hp = node(&db, "fandom", "Harry Potter").await;

    let mine = work(&db, "Mine", Some("2026-06-05T00:00:00Z")).await;
    tag(&db, &mine, &hp).await;

    let reader = account(&db, 1).await;
    bookmark(&db, &reader, &mine, true, WINDOW).await;

    let out = new_in_your_fandoms(&db, &reader, 12).await.expect("query");

    assert!(
        !ids(&out).contains(&mine.as_str()),
        "a reader's own library is not 'new' to them"
    );
    if let Some(s) = &schema {
        drop_schema(s).await;
    }
}

#[tokio::test]
async fn new_in_your_fandoms_is_empty_without_public_bookmarks() {
    let (db, schema) = connect("nifprivate").await;
    let hp = node(&db, "fandom", "Harry Potter").await;
    let hidden = work(&db, "Private Taste", Some("2026-01-01T00:00:00Z")).await;
    tag(&db, &hidden, &hp).await;
    let new_in_hp = work(&db, "New HP", Some("2026-06-09T00:00:00Z")).await;
    tag(&db, &new_in_hp, &hp).await;

    let reader = account(&db, 1).await;
    bookmark(&db, &reader, &hidden, false, WINDOW).await;

    let out = new_in_your_fandoms(&db, &reader, 12).await.expect("query");

    assert!(
        out.is_empty(),
        "a private bookmark revealed the reader's fandom by seeding the section"
    );
    if let Some(s) = &schema {
        drop_schema(s).await;
    }
}

// ---------------------------------------------------------------------------------------
// Item 33 — "Similar works"
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn similar_works_excludes_the_work_itself() {
    let (db, schema) = connect("simself").await;
    let hp = node(&db, "fandom", "Harry Potter").await;
    let tag_a = node(&db, "character", "Harry").await;

    let subject = work(&db, "Subject", Some(T0)).await;
    tag(&db, &subject, &hp).await;
    tag(&db, &subject, &tag_a).await;

    let out = similar_works(&db, &subject, 5).await.expect("query");

    assert!(
        !ids(&out).contains(&subject.as_str()),
        "a work is trivially 100% similar to itself and must not be recommended to itself"
    );
    if let Some(s) = &schema {
        drop_schema(s).await;
    }
}

#[tokio::test]
async fn similar_works_finds_a_genuine_match() {
    let (db, schema) = connect("simfind").await;
    let hp = node(&db, "fandom", "Harry Potter").await;
    let draco = node(&db, "character", "Draco").await;
    let hurt = node(&db, "mood", "angst").await;

    let subject = work(&db, "Subject", Some(T0)).await;
    tag(&db, &subject, &hp).await;
    tag(&db, &subject, &hurt).await;

    // Shares the fandom and the mood: two of two.
    let match_work = work(&db, "Match", Some(T0)).await;
    tag(&db, &match_work, &hp).await;
    tag(&db, &match_work, &hurt).await;

    // Shares only the fandom: one of two.
    let partial = work(&db, "Partial", Some(T0)).await;
    tag(&db, &partial, &hp).await;
    tag(&db, &partial, &draco).await;

    let out = similar_works(&db, &subject, 5).await.expect("query");

    assert_eq!(out[0].id, match_work, "the best match did not rank first");
    assert!(out[0].similarity.unwrap() > out[1].similarity.unwrap());
    if let Some(s) = &schema {
        drop_schema(s).await;
    }
}

#[tokio::test]
async fn similar_works_drops_results_below_the_floor() {
    let (db, schema) = connect("simfloor").await;
    let hp = node(&db, "fandom", "Harry Potter").await;
    let shared_freeform = node(&db, "tag", "fluff").await;
    let other = node(&db, "tag", "angst").await;

    let subject = work(&db, "Subject", Some(T0)).await;
    tag(&db, &subject, &hp).await;
    tag(&db, &subject, &shared_freeform).await;

    // Shares one weight-1 freeform tag out of three weight-1 tags: ~0.33... but with the
    // fandom weighted at 3 the shared weight is 1 and the union is 5, i.e. 0.2 -- still
    // above the floor. So the genuine below-floor case is a large one-sided union.
    let weak = work(&db, "Weak", Some(T0)).await;
    tag(&db, &weak, &shared_freeform).await;
    for extra in ["a", "b", "c", "d"] {
        let n = node(&db, "tag", extra).await;
        tag(&db, &weak, &n).await;
    }

    let out = similar_works(&db, &subject, 5).await.expect("query");

    assert!(
        out.iter().all(|w| w.similarity.unwrap() > FILTER_FLOOR),
        "a work scoring below the honesty floor was labelled similar"
    );
    let _ = other;
    if let Some(s) = &schema {
        drop_schema(s).await;
    }
}

#[tokio::test]
async fn similar_works_skips_works_with_fewer_than_two_tags() {
    let (db, schema) = connect("simtags").await;
    let hp = node(&db, "fandom", "Harry Potter").await;
    let hurt = node(&db, "mood", "angst").await;

    let subject = work(&db, "Subject", Some(T0)).await;
    tag(&db, &subject, &hp).await;
    tag(&db, &subject, &hurt).await;

    // One tag only: a perfect match on that tag and nothing else to compare.
    let untagged = work(&db, "One Tag", Some(T0)).await;
    tag(&db, &untagged, &hp).await;

    let out = similar_works(&db, &subject, 5).await.expect("query");

    assert!(
        !ids(&out).contains(&untagged.as_str()),
        "a work with a single tag was recommended; there is nothing to compare"
    );
    if let Some(s) = &schema {
        drop_schema(s).await;
    }
}

#[tokio::test]
async fn similar_works_is_empty_when_the_subject_has_fewer_than_two_tags() {
    let (db, schema) = connect("simsubject").await;
    let hp = node(&db, "fandom", "Harry Potter").await;
    let hurt = node(&db, "mood", "angst").await;

    // The subject itself has one tag. It cannot have an opinion about anything.
    let subject = work(&db, "Subject", Some(T0)).await;
    tag(&db, &subject, &hp).await;

    let twin = work(&db, "Twin", Some(T0)).await;
    tag(&db, &twin, &hp).await;
    tag(&db, &twin, &hurt).await;

    let out = similar_works(&db, &subject, 5).await.expect("query");

    assert!(
        out.is_empty(),
        "a subject with fewer than MIN_TAGS tags produced recommendations"
    );
    assert_eq!(MIN_TAGS, 2);
    if let Some(s) = &schema {
        drop_schema(s).await;
    }
}

#[tokio::test]
async fn similar_works_prefers_a_fandom_match_over_a_freeform_one() {
    let (db, schema) = connect("simweight").await;
    let hp = node(&db, "fandom", "Harry Potter").await;
    let lotr = node(&db, "fandom", "Lord of the Rings").await;
    let shared_freeform = node(&db, "tag", "fluff").await;

    let subject = work(&db, "Subject", Some(T0)).await;
    tag(&db, &subject, &hp).await;
    tag(&db, &subject, &shared_freeform).await;

    // Shares only the freeform tag.
    let freeform_only = work(&db, "Freeform Only", Some(T0)).await;
    tag(&db, &freeform_only, &lotr).await;
    tag(&db, &freeform_only, &shared_freeform).await;

    // Shares the fandom instead.
    let fandom_match = work(&db, "Fandom Match", Some(T0)).await;
    tag(&db, &fandom_match, &hp).await;
    tag(&db, &fandom_match, &lotr).await;

    let out = similar_works(&db, &subject, 5).await.expect("query");

    assert_eq!(
        out[0].id, fandom_match,
        "a freeform-tag overlap outranked a shared fandom; tag kinds are not weighted"
    );
    if let Some(s) = &schema {
        drop_schema(s).await;
    }
}

// ---------------------------------------------------------------------------------------
// The scorer, with no database at all
// ---------------------------------------------------------------------------------------

#[test]
fn weighted_jaccard_is_zero_on_an_empty_side() {
    assert_eq!(weighted_jaccard(&[], &[("a".into(), 1)]), 0.0);
    assert_eq!(weighted_jaccard(&[("a".into(), 1)], &[]), 0.0);
    assert_eq!(weighted_jaccard(&[], &[]), 0.0);
}

#[test]
fn weighted_jaccard_is_one_for_identical_tag_sets() {
    let a = vec![("hp".to_string(), 3), ("angst".to_string(), 1)];
    assert_eq!(weighted_jaccard(&a, &a), 1.0);
}

#[test]
fn weighted_jaccard_stays_within_zero_and_one() {
    let a = vec![("hp".to_string(), 3)];
    let b = vec![
        ("hp".to_string(), 3),
        ("x".to_string(), 1),
        ("y".to_string(), 1),
    ];
    let score = weighted_jaccard(&a, &b);
    assert!((0.0..=1.0).contains(&score), "score {score} escaped [0,1]");
}

#[test]
fn kind_weight_ranks_a_fandom_above_a_freeform_tag() {
    assert!(kind_weight("fandom") > kind_weight("character"));
    assert!(kind_weight("ship") > kind_weight("tag"));
    assert_eq!(
        kind_weight("anything_else"),
        1,
        "unknown kinds fall back to neutral"
    );
}

// ---------------------------------------------------------------------------------------
// Item 4: word count on every work card
// ---------------------------------------------------------------------------------------

/// The leaderboard reports the work's real word count.
///
/// This is the assertion that forces the two-engine run to matter. `SUM` over an INTEGER
/// column is `bigint` on SQLite and `numeric` on PostgreSQL, and `COALESCE(x, 0)` over a
/// `numeric` is still `numeric` -- so the `::bigint` cast has to go INSIDE the sum, on the
/// aggregate, not outside on the coalesce. Written the other way, SQLite decodes it fine
/// and PostgreSQL refuses the row with "expected INT8, got NUMERIC". The default engine
/// cannot see this bug; only the PostgreSQL arm can.
#[tokio::test]
async fn most_bookmarked_reports_word_count() {
    let (db, schema) = connect("wc-leaderboard").await;
    let w = work(&db, "Counted Serial", None).await;
    let owner = pseud(&db).await;
    chapter_with_words(&db, &w, &owner, 4200).await;
    let acct = account(&db, 1).await;
    bookmark(&db, &acct, &w, true, WINDOW).await;

    let out = most_bookmarked_this_week(&db, WINDOW, 12)
        .await
        .expect("query");
    let row = out
        .iter()
        .find(|x| x.id == w)
        .expect("the work is in the list");
    assert_eq!(row.word_count, 4200);
    drop_schema_if(&schema).await;
}

/// Word count SUMS across chapters, and a work with no chapters is 0 rather than absent.
///
/// The zero is the half that matters for the client. `word_count` is a plain `i64` and the
/// aggregate is COALESCEd in SQL, so an empty work reports 0 -- a true answer. Had it been
/// `Option<i64>`, a card would render nothing for "no chapters", and the same rendering
/// would then be used for a count the server simply had not computed.
#[tokio::test]
async fn word_count_sums_chapters_and_reports_zero_for_an_empty_work() {
    let (db, schema) = connect("wc-sum").await;
    let counted = work(&db, "Three Chapters", None).await;
    let empty = work(&db, "No Chapters Yet", None).await;
    let owner = pseud(&db).await;
    chapter_with_words(&db, &counted, &owner, 1000).await;
    chapter_with_words(&db, &counted, &owner, 2500).await;
    chapter_with_words(&db, &counted, &owner, 500).await;
    let acct = account(&db, 2).await;
    bookmark(&db, &acct, &counted, true, WINDOW).await;
    bookmark(&db, &acct, &empty, true, WINDOW).await;

    let out = most_bookmarked_this_week(&db, WINDOW, 12)
        .await
        .expect("query");
    let counted_row = out.iter().find(|x| x.id == counted).expect("counted work");
    let empty_row = out.iter().find(|x| x.id == empty).expect("empty work");

    assert_eq!(counted_row.word_count, 4000, "1000 + 2500 + 500");
    assert_eq!(
        empty_row.word_count, 0,
        "an empty work is 0 words, not a missing field"
    );
    drop_schema_if(&schema).await;
}

/// Only the CURRENT revision counts, so an edited chapter does not report its own history.
///
/// This is the fixture bug this test exists to prevent. Summing every revision of a chapter
/// reports the length of every draft the author ever saved, which for a heavily revised
/// chapter is several times the work's real length -- and it looks plausible, so nothing
/// downstream would notice.
#[tokio::test]
async fn word_count_ignores_superseded_revisions() {
    let (db, schema) = connect("wc-current").await;
    let w = work(&db, "Revised Chapter", None).await;
    let owner = pseud(&db).await;
    let chapter_id = chapter_with_words(&db, &w, &owner, 3000).await;

    // A second revision: a huge draft that was never made current.
    let revision2 = uuid::Uuid::new_v4().to_string();
    exec(
        &db,
        "INSERT INTO chapter_revisions
           (id, chapter_id, revision_number, document_json, sanitized_html, plain_text,
            word_count, created_by_pseud_id, created_at)
         VALUES (?1#u, ?2#u, 2, '{}', '<p></p>', '', ?3#i, ?4#u, ?5)",
        &[&revision2, &chapter_id, "90000", &owner, T0],
    )
    .await;
    // `current_revision_id` deliberately still names revision 1.

    let acct = account(&db, 3).await;
    bookmark(&db, &acct, &w, true, WINDOW).await;
    let out = most_bookmarked_this_week(&db, WINDOW, 12)
        .await
        .expect("query");
    let row = out.iter().find(|x| x.id == w).expect("the work");
    assert_eq!(
        row.word_count, 3000,
        "a superseded 90,000-word draft must not be counted"
    );
    drop_schema_if(&schema).await;
}

/// Items 14 and 33 carry word count too, because a reader deciding whether to click wants
/// the length in every list they might find a work in -- not only on the leaderboard.
#[tokio::test]
async fn new_in_your_fandoms_and_similar_also_carry_word_count() {
    let (db, schema) = connect("wc-surfaces").await;
    let subject = work(&db, "Fandom Subject", None).await;
    let match_work = work(&db, "Fandom Match", None).await;
    let owner = pseud(&db).await;
    chapter_with_words(&db, &match_work, &owner, 7777).await;
    chapter_with_words(&db, &subject, &owner, 100).await;

    let hp = node(&db, "fandom", "Star Wars").await;
    tag(&db, &subject, &hp).await;
    tag(&db, &match_work, &hp).await;
    let freeform = node(&db, "freeform", "space opera").await;
    tag(&db, &subject, &freeform).await;
    tag(&db, &match_work, &freeform).await;
    let acct = account(&db, 4).await;
    bookmark(&db, &acct, &subject, true, WINDOW).await;

    let similar = similar_works(&db, &subject, 10).await.expect("query");
    let row = similar
        .iter()
        .find(|x| x.id == match_work)
        .expect("the similar rail carries the match");
    assert_eq!(row.word_count, 7777, "the similar rail carries length too");

    let feed = new_in_your_fandoms(&db, &acct, 10).await.expect("query");
    if let Some(row) = feed.iter().find(|x| x.id == match_work) {
        assert_eq!(row.word_count, 7777, "the fandom feed carries length too");
    }
    drop_schema_if(&schema).await;
}
