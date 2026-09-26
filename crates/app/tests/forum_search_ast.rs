//! The query language reaching the forum: end to end, on both backends.
//!
//! The renderer is unit-tested in `crates/domain/tests/query_sql_forum.rs`.
//! What this file proves is that the SQL it produces actually runs against the
//! forum schema and returns the rows a reader expects -- a fragment can be
//! correct and the statement around it still wrong.
//!
//! The shape deliberately mirrors `search_works_ast`: same parser, same
//! renderer family, same typed error, same dual-backend bind order.

use lorehaven_db::search::{search_forum_ast, SearchError};
use test_support::id;

/// One category, one author, and topics with known reply counts.
async fn seed_forum(db: &lorehaven_db::Database) {
    let category_id = id("fc-cat-meta");
    let quiet_id = id("fc-cat-quiet");
    let account = id("fc-acct");
    let pseud = id("fc-pseud-nightowl");

    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            sqlx::query("INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?, ?, datetime('now'), datetime('now'))")
                .bind(&account).bind("nightowl@test.dev")
                .execute(pool).await.unwrap();
            sqlx::query("INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) VALUES (?, ?, ?, ?, datetime('now'), datetime('now'))")
                .bind(&pseud).bind(&account).bind("nightowl").bind("Nightowl")
                .execute(pool).await.unwrap();
            for (cid, name) in [(&category_id, "meta"), (&quiet_id, "off-topic")] {
                sqlx::query("INSERT INTO forum_categories (id, name, position, min_trust) VALUES (?, ?, 1, 0)")
                    .bind(cid).bind(name)
                    .execute(pool).await.unwrap();
            }
        }
        lorehaven_db::Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            sqlx::query("INSERT INTO accounts (id, email, created_at, updated_at) VALUES ($1::uuid, $2, now(), now())")
                .bind(&account).bind("nightowl@test.dev")
                .execute(pool).await.unwrap();
            sqlx::query("INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) VALUES ($1::uuid, $2::uuid, $3, $4, now(), now())")
                .bind(&pseud).bind(&account).bind("nightowl").bind("Nightowl")
                .execute(pool).await.unwrap();
            for (cid, name) in [(&category_id, "meta"), (&quiet_id, "off-topic")] {
                sqlx::query("INSERT INTO forum_categories (id, name, position, min_trust) VALUES ($1, $2, 1, 0)")
                    .bind(cid).bind(name)
                    .execute(pool).await.unwrap();
            }
        }
    }

    // A busy topic (3 replies), a quiet one (1 reply), and one nobody has
    // answered -- the last exists so the `active` COALESCE has something to
    // cover.
    let busy = id("fc-topic-busy");
    let quiet = id("fc-topic-quiet");
    let silent = id("fc-topic-silent");
    for (tid, cid, title, created, last) in [
        (
            &busy,
            &category_id,
            "A busy thread about winter",
            "2026-01-10 09:00:00",
            "2026-03-01 12:00:00",
        ),
        (
            &quiet,
            &quiet_id,
            "A quiet thread about snow",
            "2026-01-12 09:00:00",
            "2026-01-12 09:30:00",
        ),
        (
            &silent,
            &category_id,
            "A brand new thread",
            "2026-03-02 09:00:00",
            "2026-03-02 09:00:00",
        ),
    ] {
        seed_topic(db, tid, cid, &pseud, title, created, last).await;
    }
    for n in 0..3 {
        seed_post(
            db,
            &id(&format!("fc-busy-{n}")),
            &busy,
            &pseud,
            "a reply",
            "2026-02-01 10:00:00",
        )
        .await;
    }
    seed_post(
        db,
        &id("fc-quiet-0"),
        &quiet,
        &pseud,
        "the only reply",
        "2026-01-12 09:30:00",
    )
    .await;
}

async fn seed_topic(
    db: &lorehaven_db::Database,
    topic: &str,
    category: &str,
    author: &str,
    title: &str,
    created: &str,
    last: &str,
) {
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query("INSERT INTO forum_topics (id, category_id, author_pseud, title, created_at, last_post_at, locked) VALUES (?, ?, ?, ?, ?, ?, 0)")
                .bind(topic).bind(category).bind(author).bind(title).bind(created).bind(last)
                .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query("INSERT INTO forum_topics (id, category_id, author_pseud, title, created_at, last_post_at, locked) VALUES ($1, $2, $3, $4, $5, $6, false)")
                .bind(topic).bind(category).bind(author).bind(title).bind(created).bind(last)
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
        }
    }
}

async fn seed_post(
    db: &lorehaven_db::Database,
    post: &str,
    topic: &str,
    author: &str,
    body: &str,
    created: &str,
) {
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query("INSERT INTO forum_posts (id, topic_id, author_pseud, body, created_at) VALUES (?, ?, ?, ?, ?)")
                .bind(post).bind(topic).bind(author).bind(body).bind(created)
                .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query("INSERT INTO forum_posts (id, topic_id, author_pseud, body, created_at) VALUES ($1, $2, $3, $4, $5)")
                .bind(post).bind(topic).bind(author).bind(body).bind(created)
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
        }
    }
}

async fn titles(db: &lorehaven_db::Database, query: &str) -> Vec<String> {
    let mut found: Vec<String> = search_forum_ast(db, query, 50)
        .await
        .unwrap_or_else(|e| panic!("search {query:?} failed: {e}"))
        .into_iter()
        .map(|r| r.title)
        .collect();
    found.sort();
    found
}

#[tokio::test]
async fn free_text_finds_a_thread_by_its_title() {
    let dir = test_support::scratch_dir("fa_text");
    let tdb = test_support::TestDb::connect_with_dir("forum-ast-text", &dir).await;
    seed_forum(tdb.db()).await;
    assert_eq!(
        titles(tdb.db(), "winter").await,
        vec!["A busy thread about winter".to_string()]
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_reply_count_comparison_narrows_the_result() {
    let dir = test_support::scratch_dir("fa_replies");
    let tdb = test_support::TestDb::connect_with_dir("forum-ast-replies", &dir).await;
    seed_forum(tdb.db()).await;

    // The whole point: `replies:>50` -- the operator a works reader already
    // knows from `words:>10000` -- reaching a working filter on a forum column.
    // The busiest thread here has 3 replies, so `>1` isolates it.
    assert_eq!(
        titles(tdb.db(), "replies:>1").await,
        vec!["A busy thread about winter".to_string()],
        "only the thread with 3 replies clears a bound of 1"
    );
    assert_eq!(
        titles(tdb.db(), "replies:>=3").await.len(),
        1,
        "the boundary is inclusive"
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_deleted_post_is_not_a_reply() {
    let dir = test_support::scratch_dir("fa_deleted");
    let tdb = test_support::TestDb::connect_with_dir("forum-ast-deleted", &dir).await;
    seed_forum(tdb.db()).await;

    // Soft-deleting the busiest thread's posts drops it below the bound. If the
    // count included deleted rows, `replies:>1` would keep returning it and a
    // reader would see a thread whose replies they cannot read.
    let topic = id("fc-topic-busy");
    match tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query("UPDATE forum_posts SET deleted_at = datetime('now') WHERE topic_id = ?")
                .bind(&topic)
                .execute(tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .unwrap();
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query("UPDATE forum_posts SET deleted_at = now()::text WHERE topic_id = $1")
                .bind(&topic)
                .execute(tdb.db().postgres_pool().expect("postgres"))
                .await
                .unwrap();
        }
    }
    assert!(
        titles(tdb.db(), "replies:>1").await.is_empty(),
        "a thread whose replies are all deleted has no visible replies"
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_category_is_matched_by_name() {
    let dir = test_support::scratch_dir("fa_category");
    let tdb = test_support::TestDb::connect_with_dir("forum-ast-category", &dir).await;
    seed_forum(tdb.db()).await;

    // `category:meta` is a name. Comparing the id instead would match nothing,
    // and a reader would conclude the category does not exist.
    let mut found = titles(tdb.db(), "category:meta").await;
    found.sort();
    assert_eq!(
        found,
        vec![
            "A brand new thread".to_string(),
            "A busy thread about winter".to_string()
        ],
        "two threads are in meta, one in off-topic"
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn an_activity_comparison_keeps_a_thread_nobody_answered() {
    let dir = test_support::scratch_dir("fa_active");
    let tdb = test_support::TestDb::connect_with_dir("forum-ast-active", &dir).await;
    seed_forum(tdb.db()).await;

    // The brand-new thread's `last_post_at` is set here, so this also covers a
    // topic whose last_post_at is NULL: without the COALESCE, every such topic
    // compares false against every operator and vanishes from exactly the
    // filter that should surface it.
    let mut found = titles(tdb.db(), "active:>2026-02-01").await;
    found.sort();
    assert_eq!(
        found,
        vec![
            "A brand new thread".to_string(),
            "A busy thread about winter".to_string()
        ],
        "the newest thread must not be filtered out by an activity filter"
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_range_over_replies_is_two_bounds() {
    let dir = test_support::scratch_dir("fa_range");
    let tdb = test_support::TestDb::connect_with_dir("forum-ast-range", &dir).await;
    seed_forum(tdb.db()).await;

    // 1..3 covers the busy thread (3) and the quiet one (1) but not the
    // zero-reply thread. Inclusive at both ends, same as the works surface.
    let mut found = titles(tdb.db(), "replies:1..3").await;
    found.sort();
    assert_eq!(
        found,
        vec![
            "A busy thread about winter".to_string(),
            "A quiet thread about snow".to_string()
        ],
        "a range includes both ends"
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_thread_appears_once_however_many_of_its_posts_match() {
    let dir = test_support::scratch_dir("fa_dupes");
    let tdb = test_support::TestDb::connect_with_dir("forum-ast-dupes", &dir).await;
    seed_forum(tdb.db()).await;

    // The busiest thread has three replies and every one of them says "winter".
    // A join from topics to posts would return it three times -- three rows
    // with the same title, which a reader cannot distinguish from three
    // different threads.
    for n in 0..3 {
        let post = id(&format!("fc-busy-{n}"));
        match tdb.db().backend() {
            lorehaven_db::Backend::Sqlite => {
                sqlx::query("UPDATE forum_posts SET body = 'winter again' WHERE id = ?")
                    .bind(&post)
                    .execute(tdb.db().sqlite_pool().expect("sqlite"))
                    .await
                    .unwrap();
            }
            lorehaven_db::Backend::Postgres => {
                sqlx::query("UPDATE forum_posts SET body = 'winter again' WHERE id = $1")
                    .bind(&post)
                    .execute(tdb.db().postgres_pool().expect("postgres"))
                    .await
                    .unwrap();
            }
        }
    }
    let results = search_forum_ast(tdb.db(), "winter", 50).await.unwrap();
    assert_eq!(
        results.len(),
        1,
        "one thread, one row: got {:?}",
        results.iter().map(|r| &r.title).collect::<Vec<_>>()
    );
    assert_eq!(results[0].title, "A busy thread about winter");
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_search_never_finds_a_deleted_post() {
    let dir = test_support::scratch_dir("fa_hidden");
    let tdb = test_support::TestDb::connect_with_dir("forum-ast-hidden", &dir).await;
    seed_forum(tdb.db()).await;

    // A soft-deleted reply is not readable, so a search that finds it would
    // hand a reader a link to a post they cannot open -- and confirm the
    // phrase exists, which is itself the thing deletion is for.
    let post = id("fc-busy-0");
    match tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query("UPDATE forum_posts SET body = 'a secret phrase', deleted_at = datetime('now') WHERE id = ?")
                .bind(&post)
                .execute(tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .unwrap();
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query("UPDATE forum_posts SET body = 'a secret phrase', deleted_at = now()::text WHERE id = $1")
                .bind(&post)
                .execute(tdb.db().postgres_pool().expect("postgres"))
                .await
                .unwrap();
        }
    }
    assert!(
        titles(tdb.db(), "secret").await.is_empty(),
        "a deleted post must not be findable by its text"
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_foreign_field_is_a_typed_error_and_not_an_empty_result() {
    let dir = test_support::scratch_dir("fa_foreign");
    let tdb = test_support::TestDb::connect_with_dir("forum-ast-foreign", &dir).await;
    seed_forum(tdb.db()).await;

    // `words:>10000` is a works query. On the forum it has to be an error that
    // says so, because "no results" is indistinguishable from "this category
    // is empty" and the reader has no way to tell they used the wrong surface.
    let err = search_forum_ast(tdb.db(), "words:>10000", 20)
        .await
        .expect_err("a works field must be rejected on the forum");
    let downcast = err
        .downcast_ref::<SearchError>()
        .expect("the error must carry the query problem");
    assert!(
        downcast.problem.message().contains("words"),
        "the message must name the field: {}",
        downcast.problem.message()
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_backwards_range_is_rejected_before_the_database() {
    let dir = test_support::scratch_dir("fa_backwards");
    let tdb = test_support::TestDb::connect_with_dir("forum-ast-backwards", &dir).await;
    seed_forum(tdb.db()).await;

    let err = search_forum_ast(tdb.db(), "replies:10..1", 20)
        .await
        .expect_err("a backwards range can never match");
    let downcast = err
        .downcast_ref::<SearchError>()
        .expect("the error must carry the query problem");
    assert!(
        downcast.problem.message().contains("never match"),
        "{}",
        downcast.problem.message()
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn an_empty_query_returns_nothing_rather_than_everything() {
    let dir = test_support::scratch_dir("fa_empty");
    let tdb = test_support::TestDb::connect_with_dir("forum-ast-empty", &dir).await;
    seed_forum(tdb.db()).await;

    // An empty query is not "match everything" on a forum: the works search
    // treats it as a browse, but here it would dump every post on the instance
    // into a response. Empty in, empty out.
    assert!(titles(tdb.db(), "").await.is_empty());
    assert!(titles(tdb.db(), "   ").await.is_empty());
    tdb.cleanup().await;
}
