//! Regression cover for a PostgreSQL-only 500 in the concierge's read path.
//!
//! The defect: `duration_estimates` and `filter_by_mood` both build
//! `WHERE work_id IN (?, ?, ...)` and both bound each id as a STRING. That is
//! correct on SQLite, where every id column is `TEXT`. On PostgreSQL the columns
//! are `uuid`, so the bind reaches the server as text and PostgreSQL answers
//! `operator does not exist: uuid = text`.
//!
//! Two things made this hide so well, and both are why this file exists:
//!
//! 1. **It only fires on a non-empty list.** An empty `IN ()` compares nothing, so a
//!    queue with no items passed on both engines and a queue with items did not.
//!    Every test that exercised the empty path was green on both.
//! 2. **One fix was no evidence about the other.** The two functions are one
//!    function apart and both bound ids as text; fixing the first left the second
//!    broken. That is why `bind_work_ids!` exists and why both call sites are here.
//!
//! So every case below seeds at least one real work and asks for a NON-empty
//! answer. A case that asserted on an empty result would pass on both engines and
//! prove nothing.
//!
//! This crate cannot depend on `test_support` — the same documented reason as
//! `concierge_store.rs` beside it, whose `connect` this file copies. Reusing the
//! shared `postgres` database instead would trip the append-only migration checksum
//! guard on every later run.

use lorehaven_db::concierge_store::{duration_estimates, filter_by_mood};
use lorehaven_db::{Backend, Database, DatabaseConfig};
use std::time::Duration;

fn make_config(url: String) -> DatabaseConfig {
    DatabaseConfig {
        url,
        max_connections: 5,
        acquire_timeout: Duration::from_secs(5),
        slow_query_warn: Duration::ZERO,
    }
}

/// A scratch database on whichever backend `LOREHAVEN_TEST_PG_URL` names.
async fn connect(tag: &str) -> Database {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "lorehaven-concuuid-{}-{}-{}",
        tag,
        uuid::Uuid::new_v4(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let url = match std::env::var("LOREHAVEN_TEST_PG_URL") {
        Ok(admin) => {
            let name = format!(
                "lh_concuuid_{}_{}",
                std::process::id(),
                uuid::Uuid::new_v4().simple()
            );
            let admin_db = Database::connect(&make_config(admin.clone()))
                .await
                .expect("admin connect");
            sqlx::query(&format!("CREATE DATABASE {name}"))
                .execute(admin_db.postgres_pool().expect("pg pool"))
                .await
                .expect("create a scratch database");
            // Replace the LAST path segment, not append to the whole URL. The admin
            // URL already names a database — `.../postgres` — so appending gives
            // `.../postgres/lh_...`, which sqlx parses as a unix-socket host and
            // fails with `failed to lookup address information: Name or service not
            // known`. Copied from `concierge_store.rs`, which hit exactly that.
            let (prefix, _) = admin
                .rsplit_once('/')
                .expect("the admin URL ends in a database");
            format!("{prefix}/{name}")
        }
        Err(_) => format!("sqlite://{}/lorehaven.sqlite?mode=rwc", dir.display()),
    };
    let db = Database::connect(&make_config(url)).await.expect("connect");
    db.migrate().await.expect("migrate");
    db
}

const T0: &str = "2026-01-01T00:00:00Z";

/// Run one fixture statement, `?N#u` for a native uuid, `?N#i` for a bigint and
/// `?N#t` for a timestamp (nonexistent on SQLite).
///
/// Copied from `concierge_store.rs`, and the reason for copying rather than
/// reaching for `rewrite_placeholders` is the bug this file's first version had:
///
/// `rewrite_placeholders` (db/src/lib.rs:462) walks the string and renumbers every
/// `?` it meets in order, IGNORING the digits. So `VALUES (?3#t, ?3#t)` — one slot
/// used twice — comes out as `$3, $4`, and the statement asks for a fourth bind it
/// was never given. PostgreSQL reports that as 42P18 "could not determine data type
/// of parameter $4".
///
/// The fold below replaces `?N` at fixed positions instead, so a repeated slot
/// stays repeated. `#u` and `#i` are folded BEFORE the bare `?N`, or the marker
/// survives as a column named `u`.
async fn exec(db: &Database, tmpl: &str, args: &[&str]) {
    match db.backend() {
        Backend::Sqlite => {
            let sql = tmpl.replace("#u", "").replace("#i", "").replace("#t", "");
            let mut q = sqlx::query(&sql);
            for a in args {
                q = q.bind(*a);
            }
            q.execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("fixture insert");
        }
        Backend::Postgres => {
            let pg = (1..=12).fold(tmpl.to_string(), |acc, n| {
                acc.replace(&format!("?{n}#u"), &format!("${n}::uuid"))
                    .replace(&format!("?{n}#i"), &format!("${n}::bigint"))
                    .replace(&format!("?{n}#t"), &format!("${n}::timestamptz"))
                    .replace(&format!("?{n}"), &format!("${n}"))
            });
            let mut q = sqlx::query(&pg);
            for a in args {
                match uuid::Uuid::parse_str(a) {
                    Ok(u) => q = q.bind(u),
                    Err(_) => q = q.bind(*a),
                }
            }
            q.execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("fixture insert");
        }
    }
}

/// One published, complete, public work with a chapter of 4000 words, so it is
/// eligible for every engine AND has a duration estimate. Returns the work id.
async fn seed_one(db: &Database) -> String {
    let work = uuid::Uuid::new_v4().to_string();
    let account = uuid::Uuid::new_v4().to_string();
    let pseud = uuid::Uuid::new_v4().to_string();
    let chapter = uuid::Uuid::new_v4().to_string();
    let revision = uuid::Uuid::new_v4().to_string();
    let node = uuid::Uuid::new_v4().to_string();

    exec(
        db,
        "INSERT INTO accounts (id, status, email, age_state, created_at, updated_at, permission_statement)
         VALUES (?1#u, 'active', ?2, 'adult', ?3#t, ?3#t, 'minimal')",
        &[&account, &format!("seed-{account}@example.com"), T0],
    )
    .await;
    exec(
        db,
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
         VALUES (?1#u, ?2#u, ?3, ?3, ?4#t, ?4#t)",
        &[&pseud, &account, &format!("seed{}", &work[..8]), T0],
    )
    .await;
    exec(
        db,
        "INSERT INTO works (id, owner_pseud_id, title, lifecycle, visibility, completion, created_at, updated_at, generated_content_posture)
         VALUES (?1#u, ?2#u, 'Seeded', 'published', 'public', 'complete', ?3#t, ?3#t, 'forbid')",
        &[&work, &pseud, T0],
    )
    .await;
    // Chapter first with a NULL current_revision_id, then the revision, then the
    // link — `chapters.current_revision_id` REFERENCES `chapter_revisions(id)`, and
    // only PostgreSQL enforces that on insert.
    exec(
        db,
        "INSERT INTO chapters (id, work_id, order_key, title, current_revision_id, created_at, updated_at)
         VALUES (?1#u, ?2#u, ?3#i, 'One', NULL, ?4#t, ?4#t)",
        &[&chapter, &work, "10", T0],
    )
    .await;
    exec(
        db,
        "INSERT INTO chapter_revisions (id, chapter_id, revision_number, document_json, sanitized_html, plain_text, word_count, created_by_pseud_id, created_at)
         VALUES (?1#u, ?2#u, 1, '{}', '<p>x</p>', 'x', ?3#i, ?4#u, ?5#t)",
        &[&revision, &chapter, "4000", &pseud, T0],
    )
    .await;
    exec(
        db,
        "UPDATE chapters SET current_revision_id = ?2#u WHERE id = ?1#u",
        &[&chapter, &revision],
    )
    .await;
    exec(
        db,
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at, review_status, signal_count)
         VALUES (?1#u, 'mood', 'Comfort', 'comfort', ?2#t, 'pending', 0)",
        &[&node, T0],
    )
    .await;
    exec(
        db,
        "INSERT INTO work_tags (work_id, node_id, weight, added_at) VALUES (?1#u, ?2#u, 1, ?3#t)",
        &[&work, &node, T0],
    )
    .await;
    work
}

#[tokio::test]
async fn duration_estimates_binds_ids_as_uuids_not_text() {
    let db = connect("est").await;
    let work = seed_one(&db).await;

    // The non-empty list is the whole point. On PostgreSQL this raised
    // `operator does not exist: uuid = text`, which the route turned into a 500.
    let estimates = duration_estimates(&db, std::slice::from_ref(&work))
        .await
        .expect("duration_estimates must not fail on a real work id");

    assert_eq!(
        estimates.len(),
        1,
        "the seeded work has a chapter, so it has an estimate: {estimates:?}"
    );
    let minutes = estimates.get(&work).copied().expect("an estimate");
    assert!(
        minutes > 0.0,
        "4000 words at §11's rate is a real number of minutes, not zero: {minutes}"
    );
}

#[tokio::test]
async fn filter_by_mood_binds_ids_as_uuids_not_text() {
    let db = connect("mood").await;
    let work = seed_one(&db).await;

    // Same defect, one function over. It was written twice, so fixing the first was
    // no evidence about the second.
    let matched = filter_by_mood(&db, std::slice::from_ref(&work), "comfort")
        .await
        .expect("filter_by_mood must not fail on a real work id");

    assert_eq!(
        matched,
        vec![work.clone()],
        "the seeded work carries the mood, so it is a match — an empty result here \
         would be the uuid/text mismatch returning nothing rather than erroring"
    );

    // And the negative case, which is the one that hid it: an empty result is a
    // legitimate answer for a mood nothing carries, and must not be an error.
    let none = filter_by_mood(&db, std::slice::from_ref(&work), "grief")
        .await
        .expect("a mood nothing carries is an empty answer, not a failure");
    assert!(none.is_empty(), "nothing carries grief: {none:?}");
}

/// An empty list must also work, and must return empty rather than a
/// malformed-`IN ()` error.
///
/// Asserted because it is the SHAPE that hid the defect: `IN ()` is a syntax error
/// in standard SQL and an empty result in SQLite, so an implementation that
/// short-circuited on `is_empty()` would pass here while still failing for the
/// non-empty case above. Both are needed; neither alone is enough.
#[tokio::test]
async fn an_empty_id_list_is_an_empty_answer_on_both_engines() {
    let db = connect("empty").await;

    let estimates = duration_estimates(&db, &[])
        .await
        .expect("an empty list is a valid question");
    assert!(estimates.is_empty(), "{estimates:?}");

    let matched = filter_by_mood(&db, &[], "comfort")
        .await
        .expect("an empty list is a valid question");
    assert!(matched.is_empty(), "{matched:?}");
}

/// A malformed id must not take the whole answer down.
///
/// The behaviour being pinned is the NULL BIND, and both halves of it matter:
///
/// - The bad id must not fail the query. It is bound as NULL, which never matches.
/// - It must not SHIFT the others either. PostgreSQL binds positionally, so a
///   "just skip this one" implementation raises `bind message supplies 1 parameters,
///   but prepared statement requires 2` (08P01) — an error rather than a filter, and
///   a worse outcome than the one this test exists to prevent.
///
/// The valid id beside it must still come back, which is the assertion that tells
/// the two apart.
#[tokio::test]
async fn a_malformed_id_is_skipped_and_the_valid_one_still_answers() {
    let db = connect("bad").await;
    let work = seed_one(&db).await;

    let ids = vec!["not-a-uuid".to_string(), work.clone()];
    let estimates = duration_estimates(&db, &ids)
        .await
        .expect("one bad id must not fail the call");
    assert_eq!(
        estimates.len(),
        1,
        "the valid id still answered: {estimates:?}"
    );
    assert!(estimates.contains_key(&work), "{estimates:?}");
}
