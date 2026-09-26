//! The forum search endpoint, speaking the shared query language.
//!
//! The DB layer is proved in `forum_search_ast.rs`. What this file proves is
//! the HTTP surface: that a `replies:>50` typed into the forum search box
//! actually reaches the database, that a bad query comes back as `422` with a
//! reason a reader can act on, and that the old `from`/`to` parameters still
//! work so no existing link breaks.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use serde_json::Value;
use test_support::id;
use tower::ServiceExt;

async fn seed(db: &lorehaven_db::Database) {
    let account = id("rt-acct");
    let pseud = id("rt-pseud");
    let category = id("rt-cat-meta");
    let topic = id("rt-topic-busy");

    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            sqlx::query("INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?, ?, datetime('now'), datetime('now'))")
                .bind(&account).bind("rt@test.dev").execute(pool).await.unwrap();
            sqlx::query("INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) VALUES (?, ?, ?, ?, datetime('now'), datetime('now'))")
                .bind(&pseud).bind(&account).bind("rt-author").bind("RT")
                .execute(pool).await.unwrap();
            sqlx::query(
                "INSERT INTO forum_categories (id, name, position, min_trust) VALUES (?, ?, 1, 0)",
            )
            .bind(&category)
            .bind("meta")
            .execute(pool)
            .await
            .unwrap();
            sqlx::query("INSERT INTO forum_topics (id, category_id, author_pseud, title, created_at, last_post_at, locked) VALUES (?, ?, ?, ?, ?, ?, 0)")
                .bind(&topic).bind(&category).bind(&pseud).bind("A thread about winter")
                .bind("2026-01-10 09:00:00").bind("2026-03-01 12:00:00")
                .execute(pool).await.unwrap();
            for n in 0..3 {
                sqlx::query("INSERT INTO forum_posts (id, topic_id, author_pseud, body, created_at) VALUES (?, ?, ?, ?, ?)")
                    .bind(id(&format!("rt-post-{n}"))).bind(&topic).bind(&pseud)
                    .bind("a reply").bind("2026-02-01 10:00:00")
                    .execute(pool).await.unwrap();
            }
        }
        lorehaven_db::Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            sqlx::query("INSERT INTO accounts (id, email, created_at, updated_at) VALUES ($1::uuid, $2, now(), now())")
                .bind(&account).bind("rt@test.dev").execute(pool).await.unwrap();
            sqlx::query("INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) VALUES ($1::uuid, $2::uuid, $3, $4, now(), now())")
                .bind(&pseud).bind(&account).bind("rt-author").bind("RT")
                .execute(pool).await.unwrap();
            sqlx::query("INSERT INTO forum_categories (id, name, position, min_trust) VALUES ($1, $2, 1, 0)")
                .bind(&category).bind("meta").execute(pool).await.unwrap();
            sqlx::query("INSERT INTO forum_topics (id, category_id, author_pseud, title, created_at, last_post_at, locked) VALUES ($1, $2, $3, $4, $5, $6, false)")
                .bind(&topic).bind(&category).bind(&pseud).bind("A thread about winter")
                .bind("2026-01-10 09:00:00").bind("2026-03-01 12:00:00")
                .execute(pool).await.unwrap();
            for n in 0..3 {
                sqlx::query("INSERT INTO forum_posts (id, topic_id, author_pseud, body, created_at) VALUES ($1, $2, $3, $4, $5)")
                    .bind(id(&format!("rt-post-{n}"))).bind(&topic).bind(&pseud)
                    .bind("a reply").bind("2026-02-01 10:00:00")
                    .execute(pool).await.unwrap();
            }
        }
    }
}

/// The router under test, on the fixture's own handle.
///
/// One `TestDb` for the app and the fixture: under PostgreSQL each
/// `connect_with_dir` builds a *separate* scratch database, so a second handle
/// would write the fixture somewhere the app cannot see it -- and the test would
/// pass on SQLite (one file, both handles) and fail on PostgreSQL.
fn router_for(tdb: &test_support::TestDb, dir: &std::path::Path) -> axum::Router {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    server::build_router(AppState::new(config, tdb.db().clone()))
}

/// GET a URI and return the status with the decoded body.
async fn get_status(router: axum::Router, uri: &str) -> (StatusCode, Value) {
    let response = router
        .oneshot(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 10_000_000)
        .await
        .expect("body");
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, json)
}

/// Percent-encode a query string value, so a test reads as the query a reader
/// would type rather than as a wall of `%3A`.
fn q(query: &str) -> String {
    query
        .replace(':', "%3A")
        .replace('>', "%3E")
        .replace('<', "%3C")
        .replace(' ', "%20")
        .replace('"', "%22")
}

/// The titles in `items`, sorted. A search has no natural order, so asserting
/// one would be asserting the tie-break rather than the behaviour.
async fn titles(tdb: &test_support::TestDb, dir: &std::path::Path, query: &str) -> Vec<String> {
    let (status, body) = get_status(router_for(tdb, dir), query).await;
    assert_eq!(status, StatusCode::OK, "query {query:?} -> {body}");
    let mut found: Vec<String> = body["items"]
        .as_array()
        .unwrap_or_else(|| panic!("items must be an array: {body}"))
        .iter()
        .map(|i| i["title"].as_str().expect("a title").to_string())
        .collect();
    found.sort();
    found
}

#[tokio::test]
async fn a_query_language_filter_reaches_the_database() {
    let dir = test_support::scratch_dir("rt_filter");
    let tdb = test_support::TestDb::connect_with_dir("forum-route-filter", &dir).await;
    seed(tdb.db()).await;

    // The endpoint behaves as though the query box were the only input. A reader
    // who has learned `words:>10000` on the works search should not have to
    // learn a second language for the forum.
    assert_eq!(
        titles(
            &tdb,
            &dir,
            &format!("/api/v1/forum-search?q={}", q("replies:>1"))
        )
        .await,
        vec!["A thread about winter".to_string()],
        "replies:>1 must reach the database as a filter"
    );
    assert!(
        titles(
            &tdb,
            &dir,
            &format!("/api/v1/forum-search?q={}", q("replies:>99"))
        )
        .await
        .is_empty(),
        "a bound no thread clears must return nothing, not everything"
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_field_from_another_surface_is_422_with_a_reason() {
    let dir = test_support::scratch_dir("rt_wrongfield");
    let tdb = test_support::TestDb::connect_with_dir("forum-route-wrongfield", &dir).await;
    seed(tdb.db()).await;

    // `words` is a works field. Answering 200 with an empty list would be
    // indistinguishable from "this category is empty", and the reader would have
    // no way to learn they used the wrong surface.
    let (status, body) = get_status(
        router_for(&tdb, &dir),
        &format!("/api/v1/forum-search?q={}", q("words:>10000")),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let message = body["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("the error must carry a reason: {body}"));
    assert!(
        message.contains("words") && message.contains("forum"),
        "the reason must name the field and the surface: {message}"
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_malformed_query_is_422_and_not_500() {
    let dir = test_support::scratch_dir("rt_malformed");
    let tdb = test_support::TestDb::connect_with_dir("forum-route-malformed", &dir).await;
    seed(tdb.db()).await;

    for query in ["replies:>lots", "replies:10..1", "title:>abc"] {
        let (status, body) = get_status(
            router_for(&tdb, &dir),
            &format!("/api/v1/forum-search?q={}", q(query)),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{query:?} is the reader's mistake, not a server fault: {body}"
        );
    }
    tdb.cleanup().await;
}

#[tokio::test]
async fn the_old_from_and_to_parameters_still_work() {
    let dir = test_support::scratch_dir("rt_legacy");
    let tdb = test_support::TestDb::connect_with_dir("forum-route-legacy", &dir).await;
    seed(tdb.db()).await;

    // The old endpoint took `from`/`to` as separate parameters. The query
    // language now covers them with `active:>...`, but a link someone bookmarked
    // or a form that still posts them must keep working -- dropping them would
    // break every existing URL to make the new one tidier.
    assert_eq!(
        titles(&tdb, &dir, "/api/v1/forum-search?q=&from=2026-02-01").await,
        vec!["A thread about winter".to_string()],
        "from= is still a date floor"
    );
    assert!(
        titles(&tdb, &dir, "/api/v1/forum-search?q=&from=2027-01-01")
            .await
            .is_empty(),
        "a future floor matches nothing"
    );
    assert!(
        titles(&tdb, &dir, "/api/v1/forum-search?q=&to=2026-01-01")
            .await
            .is_empty(),
        "to= is still a date ceiling"
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn the_old_category_and_author_parameters_still_work() {
    let dir = test_support::scratch_dir("rt_legacy2");
    let tdb = test_support::TestDb::connect_with_dir("forum-route-legacy2", &dir).await;
    seed(tdb.db()).await;

    assert_eq!(
        titles(&tdb, &dir, "/api/v1/forum-search?q=&category=meta").await,
        vec!["A thread about winter".to_string()],
        "category= is still a name filter"
    );
    assert!(
        titles(&tdb, &dir, "/api/v1/forum-search?q=&category=nope")
            .await
            .is_empty(),
        "an unknown category matches nothing"
    );
    assert_eq!(
        titles(&tdb, &dir, "/api/v1/forum-search?q=&author=rt-author").await,
        vec!["A thread about winter".to_string()],
        "author= is still a handle filter"
    );
    assert!(
        titles(&tdb, &dir, "/api/v1/forum-search?q=&author=nobody")
            .await
            .is_empty(),
        "an unknown author matches nothing"
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_minimum_reply_count_is_honoured_and_zero_is_a_real_bound() {
    let dir = test_support::scratch_dir("rt_minreplies");
    let tdb = test_support::TestDb::connect_with_dir("forum-route-minreplies", &dir).await;
    seed(tdb.db()).await;

    assert_eq!(
        titles(&tdb, &dir, "/api/v1/forum-search?q=&min_replies=2").await,
        vec!["A thread about winter".to_string()],
        "the thread has three replies"
    );
    // `0` is a question people actually ask -- "show me everything" -- and it
    // is inclusive like every other bound, so it returns the thread rather than
    // being treated as "unset". Treating it as unset would be the subtle bug:
    // the parameter would look like it works and would quietly do nothing.
    assert_eq!(
        titles(&tdb, &dir, "/api/v1/forum-search?q=&min_replies=0").await,
        vec!["A thread about winter".to_string()],
        "a zero bound is a real bound, not a stand-in for unset"
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_negative_reply_bound_is_422_and_not_an_empty_page() {
    let dir = test_support::scratch_dir("rt_negative");
    let tdb = test_support::TestDb::connect_with_dir("forum-route-negative", &dir).await;
    seed(tdb.db()).await;

    // A negative count can never match. Answering 200 with an empty list is the
    // same answer as "no thread has fewer than minus one replies", which is
    // indistinguishable from "this instance has no threads" -- and the reader
    // has no way to learn their own input was the problem.
    let (status, body) = get_status(
        router_for(&tdb, &dir),
        "/api/v1/forum-search?q=&min_replies=-5",
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let message = body["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("the error must carry a reason: {body}"));
    assert!(
        message.contains("min_replies") && message.contains("-5"),
        "the reason must name the parameter and the value: {message}"
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_query_and_a_legacy_parameter_both_apply() {
    let dir = test_support::scratch_dir("rt_mixed");
    let tdb = test_support::TestDb::connect_with_dir("forum-route-mixed", &dir).await;
    seed(tdb.db()).await;

    // They are conjunctive, not alternatives. A reader who types `winter` in the
    // box *and* the page adds `category=meta` from a dropdown means both, and
    // quietly honouring only one is a filter that lies.
    assert_eq!(
        titles(&tdb, &dir, "/api/v1/forum-search?q=winter&category=meta").await,
        vec!["A thread about winter".to_string()]
    );
    assert!(
        titles(&tdb, &dir, "/api/v1/forum-search?q=winter&category=nope")
            .await
            .is_empty(),
        "the dropdown can still rule the query out"
    );
    tdb.cleanup().await;
}
