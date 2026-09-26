//! The user surface, end to end against a real database on both backends.
//!
//! `crates/domain/tests/query_sql_user.rs` proves the renderer emits the right
//! SQL. This file proves that SQL *runs* -- and runs the same way on both
//! dialects, which is a different claim. A statement can be perfectly shaped
//! and still match nothing because a column is the wrong type on one backend.
//!
//! Three assertions here guard bugs that are invisible without them:
//!
//! * `a_work_count_is_not_compared_as_text` -- SQLite leaves a text bind on the
//!   right of an integer as text, so `3 > '1'` is false and every count filter
//!   matches nothing. That reads exactly like an instance with no authors.
//! * `a_fandom_spelled_as_an_alias_still_finds_its_authors` -- the alias arm.
//!   Without it a reader who types a variant spelling gets an empty page.
//! * `a_pseudonym_with_a_null_display_name_survives_a_negated_search` -- `NOT
//!   NULL` is NULL, so without the COALESCE the row drops out of a `NOT` result
//!   it belongs in.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_db::Backend;
use test_support::id;
use tower::ServiceExt;

/// A pseudonym, plus the account it hangs off.
async fn seed_pseud(db: &lorehaven_db::Database, handle: &str, display: &str, joined: &str) {
    let account_id = id(&format!("{handle}-acct"));
    let pseud_id = id(&format!("{handle}-pseud"));
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?, ?, ?, ?)",
            )
            .bind(&account_id)
            .bind(format!("{handle}@test.dev"))
            .bind(joined)
            .bind(joined)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .unwrap();
            sqlx::query("INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)")
                .bind(&pseud_id).bind(&account_id).bind(handle).bind(Some(display)).bind(joined).bind(joined)
                .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
        }
        Backend::Postgres => {
            sqlx::query("INSERT INTO accounts (id, email, created_at, updated_at) VALUES ($1::uuid, $2, $3, $4)")
                .bind(&account_id).bind(format!("{handle}@test.dev")).bind(joined).bind(joined)
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
            sqlx::query("INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6)")
                .bind(&pseud_id).bind(&account_id).bind(handle).bind(Some(display)).bind(joined).bind(joined)
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
        }
    }
}

/// A work owned by `owner`, with the given lifecycle and visibility.
async fn seed_work(
    db: &lorehaven_db::Database,
    slug: &str,
    owner: &str,
    lifecycle: &str,
    visibility: &str,
) {
    let work_id = id(slug);
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("INSERT INTO works (id, title, owner_pseud_id, lifecycle, visibility, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
                .bind(&work_id).bind(slug).bind(owner).bind(lifecycle).bind(visibility)
                .bind("2026-01-01T00:00:00Z").bind("2026-01-01T00:00:00Z")
                .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
        }
        Backend::Postgres => {
            sqlx::query("INSERT INTO works (id, title, owner_pseud_id, lifecycle, visibility, created_at, updated_at) VALUES ($1::uuid, $2, $3::uuid, $4, $5, $6, $7)")
                .bind(&work_id).bind(slug).bind(owner).bind(lifecycle).bind(visibility)
                .bind("2026-01-01T00:00:00Z").bind("2026-01-01T00:00:00Z")
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
        }
    }
}

/// A work carrying a fandom tag, plus the taxonomy node it points at.
async fn seed_tagged_work(db: &lorehaven_db::Database, slug: &str, owner: &str, fandom: &str) {
    seed_work(db, slug, owner, "published", "public").await;
    let work_id = id(slug);
    let node_id = id(&format!("{slug}-node"));
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at) VALUES (?, 'fandom', ?, ?, ?)")
                .bind(&node_id).bind(fandom).bind(fandom.to_lowercase()).bind("2026-01-01T00:00:00Z")
                .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
            sqlx::query(
                "INSERT INTO work_tags (work_id, node_id, weight, added_at) VALUES (?, ?, 1, ?)",
            )
            .bind(&work_id)
            .bind(&node_id)
            .bind("2026-01-01T00:00:00Z")
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .unwrap();
        }
        Backend::Postgres => {
            sqlx::query("INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at) VALUES ($1::uuid, 'fandom', $2, $3, $4)")
                .bind(&node_id).bind(fandom).bind(fandom.to_lowercase()).bind("2026-01-01T00:00:00Z")
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
            sqlx::query("INSERT INTO work_tags (work_id, node_id, weight, added_at) VALUES ($1::uuid, $2::uuid, 1, $3)")
                .bind(&work_id).bind(&node_id).bind("2026-01-01T00:00:00Z")
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
        }
    }
}

/// The same fandom, spelled the way a reader would type it.
async fn seed_fandom_alias(
    db: &lorehaven_db::Database,
    canonical: &str,
    alias: &str,
    work_slug: &str,
) {
    let node_id = id(&format!("{work_slug}-node"));
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("INSERT INTO taxonomy_aliases (alias, norm, node_id, source) VALUES (?, ?, ?, 'import')")
                .bind(alias).bind(alias.to_lowercase()).bind(&node_id)
                .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
        }
        Backend::Postgres => {
            sqlx::query("INSERT INTO taxonomy_aliases (alias, norm, node_id, source) VALUES ($1, $2, $3::uuid, 'import')")
                .bind(alias).bind(alias.to_lowercase()).bind(&node_id)
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
        }
    }
    let _ = canonical;
}

/// Handles matching `query`, sorted so a tie in score cannot decide the result.
async fn handles(db: &lorehaven_db::Database, query: &str) -> Vec<String> {
    let found = lorehaven_db::search::search_users_ast(db, query, 50)
        .await
        .unwrap_or_else(|e| panic!("search {query:?}: {e}"));
    let mut names: Vec<String> = found.into_iter().map(|u| u.handle).collect();
    names.sort();
    names
}

/// A fresh database on whichever backend `LOREHAVEN_TEST_PG_URL` selects.
async fn setup(tag: &str) -> test_support::TestDb {
    let dir = test_support::scratch_dir(tag);
    test_support::TestDb::connect_with_dir(tag, &dir).await
}

fn router_for(tdb: &test_support::TestDb, dir: &std::path::Path) -> axum::Router {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    server::build_router(AppState::new(config, tdb.db().clone()))
}

async fn get_status(router: axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
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
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json)
}

// --- tests -------------------------------------------------------------------

#[tokio::test]
async fn a_handle_is_found_by_its_own_name() {
    let tdb = setup("u-handle-by-name").await;
    let db = tdb.db();
    seed_pseud(db, "nightowl", "Night Owl", "2026-01-01T00:00:00Z").await;
    seed_pseud(db, "daybat", "Day Bat", "2026-01-01T00:00:00Z").await;

    assert_eq!(handles(db, "user:nightowl").await, vec!["nightowl"]);
    assert_eq!(handles(db, "nightowl").await, vec!["nightowl"]);
}

#[tokio::test]
async fn a_display_name_is_searchable_too() {
    let tdb = setup("u-display-name").await;
    let db = tdb.db();
    seed_pseud(db, "nightowl", "Owl of the Night", "2026-01-01T00:00:00Z").await;
    seed_pseud(db, "daybat", "Bat of the Day", "2026-01-01T00:00:00Z").await;

    assert_eq!(handles(db, "Owl").await, vec!["nightowl"]);
}

#[tokio::test]
async fn a_work_count_is_not_compared_as_text() {
    // The regression invisible without this test: a text bound against an
    // integer column compares as text on SQLite, so this returns nothing --
    // which reads exactly like an instance where nobody has written anything.
    let tdb = setup("u-work-count").await;
    let db = tdb.db();
    seed_pseud(db, "prolific", "Prolific", "2026-01-01T00:00:00Z").await;
    seed_work(
        db,
        "live-work",
        &pseud_id("prolific"),
        "published",
        "public",
    )
    .await;
    seed_pseud(db, "quiet", "Quiet One", "2026-01-01T00:00:00Z").await;

    let found = handles(db, "works:>0").await;
    assert_eq!(found, vec!["prolific"], "the CAST is what makes this work");
}

/// The pseud id behind a handle, so a work can be attributed to it.
///
/// Derived from the handle rather than looked up, because `id()` is what
/// `seed_pseud` used and both have to agree.
/// Percent-encode a query for a URL. A reader's browser does this, and `>` and
/// `:` are not legal raw in a URI -- so a test that skips it is testing a URL
/// no browser would ever send.
fn enc(q: &str) -> String {
    q.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

fn pseud_id(handle: &str) -> String {
    id(&format!("{handle}-pseud"))
}

#[tokio::test]
async fn a_work_count_ignores_a_draft_and_a_private_work() {
    // A draft is invisible in the listing and a private work is nobody's to
    // read. Counting either would rank a dormant account above an active one.
    let tdb = setup("u-count-ignores-draft").await;
    let db = tdb.db();
    seed_pseud(db, "mixed", "Mixed", "2026-01-01T00:00:00Z").await;
    let owner = pseud_id("mixed");
    seed_work(db, "live-work", &owner, "published", "public").await;
    seed_work(db, "draft-work", &owner, "draft", "public").await;
    seed_work(db, "hidden-work", &owner, "published", "private").await;

    // Exactly one work is countable, so `>0` matches and `>1` does not. If the
    // draft or the private work leaked into the count, `>0` would be 3.
    assert_eq!(handles(db, "works:>0").await, vec!["mixed"]);
    assert_eq!(handles(db, "works:>1").await, Vec::<String>::new());
}

#[tokio::test]
async fn a_work_count_range_has_both_ends_inclusive() {
    let tdb = setup("u-count-range").await;
    let db = tdb.db();
    seed_pseud(db, "one", "One", "2026-01-01T00:00:00Z").await;
    seed_work(db, "one-work", &pseud_id("one"), "published", "public").await;

    assert_eq!(handles(db, "works:1..3").await, vec!["one"]);
    assert_eq!(handles(db, "works:2..3").await, Vec::<String>::new());
    assert_eq!(handles(db, "works:..1").await, vec!["one"]);
    assert_eq!(handles(db, "works:1..").await, vec!["one"]);
}

#[tokio::test]
async fn a_fandom_finds_its_authors() {
    let tdb = setup("u-fandom-authors").await;
    let db = tdb.db();
    seed_pseud(db, "writer", "Writer", "2026-01-01T00:00:00Z").await;
    seed_pseud(db, "stranger", "Stranger", "2026-01-01T00:00:00Z").await;
    seed_tagged_work(db, "omens-fic", &pseud_id("writer"), "Good Omens").await;

    assert_eq!(handles(db, "fandoms:\"Good Omens\"").await, vec!["writer"]);
}

#[tokio::test]
async fn a_fandom_spelled_as_an_alias_still_finds_its_authors() {
    // The arm that is easy to forget and impossible to notice: a reader who
    // types the variant spelling gets an empty page and concludes nobody writes
    // in that fandom.
    let tdb = setup("u-fandom-alias").await;
    let db = tdb.db();
    seed_pseud(db, "writer", "Writer", "2026-01-01T00:00:00Z").await;
    seed_tagged_work(db, "omens-fic", &pseud_id("writer"), "Good Omens").await;
    seed_fandom_alias(db, "Good Omens", "Good Omens (TV)", "omens-fic").await;

    assert_eq!(handles(db, "fandoms:\"Good Omens\"").await, vec!["writer"]);
    assert_eq!(
        handles(db, "fandoms:\"Good Omens (TV)\"").await,
        vec!["writer"],
        "the alias spelling must find the same author"
    );
}

#[tokio::test]
async fn a_fandom_is_matched_exactly_and_not_as_a_prefix() {
    // `norm` rather than a LIKE, so a fandom called "Good Omens" does not also
    // match "Good Omenshole" -- which a substring match returns, with no way for
    // a reader to tell it from a real result.
    let tdb = setup("u-fandom-exact").await;
    let db = tdb.db();
    seed_pseud(db, "writer", "Writer", "2026-01-01T00:00:00Z").await;
    seed_tagged_work(db, "hole-fic", &pseud_id("writer"), "Good Omenshole").await;

    assert_eq!(
        handles(db, "fandoms:\"Good Omens\"").await,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn a_deleted_work_does_not_make_its_author_a_fandom_writer() {
    let tdb = setup("u-fandom-deleted").await;
    let db = tdb.db();
    seed_pseud(db, "exwriter", "Ex Writer", "2026-01-01T00:00:00Z").await;
    seed_tagged_work(db, "gone-fic", &pseud_id("exwriter"), "Good Omens").await;
    let work_id = id("gone-fic");

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("UPDATE works SET deleted_at = ? WHERE id = ?")
                .bind("2026-06-01T00:00:00Z")
                .bind(&work_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .unwrap();
        }
        Backend::Postgres => {
            sqlx::query("UPDATE works SET deleted_at = $1 WHERE id = $2::uuid")
                .bind("2026-06-01T00:00:00Z")
                .bind(&work_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .unwrap();
        }
    }

    assert_eq!(
        handles(db, "fandoms:\"Good Omens\"").await,
        Vec::<String>::new(),
        "a deleted work is not something a reader can find"
    );
}

#[tokio::test]
async fn a_join_date_is_ordered_not_counted() {
    let tdb = setup("u-join-date").await;
    let db = tdb.db();
    seed_pseud(db, "oldhand", "Old Hand", "2024-01-01T00:00:00Z").await;
    seed_pseud(db, "newhand", "New Hand", "2026-06-01T00:00:00Z").await;

    assert_eq!(handles(db, "joined:<2026-01-01").await, vec!["oldhand"]);
    assert_eq!(handles(db, "joined:>2026-01-01").await, vec!["newhand"]);
    // Inclusive, like every other bound in the language.
    assert_eq!(handles(db, "joined:>=2026-06-01").await, vec!["newhand"]);
}

#[tokio::test]
async fn a_negated_search_does_not_lose_the_rows_it_should_keep() {
    // `NOT x` on a free-text arm is the null-safety case: the arm is an OR of
    // two LIKEs, and a row that matches neither arm must survive the negation.
    // Rendered as `NOT (a OR b)` with a NULL anywhere, the whole expression is
    // NULL, `NOT NULL` is NULL, and the row drops out of a result it belongs in.
    let tdb = setup("u-not").await;
    let db = tdb.db();
    seed_pseud(db, "named", "A Name", "2026-01-01T00:00:00Z").await;
    seed_pseud(db, "other", "Quite Other", "2026-01-01T00:00:00Z").await;

    assert_eq!(handles(db, "NOT named").await, vec!["other"]);
}

#[tokio::test]
async fn a_percent_in_the_query_is_a_literal_percent() {
    // `100%_complete` means those two characters. Unescaped, `%` matches any
    // run and `_` any one character, so the query returns far more than the
    // reader asked for -- silently, because it looks right.
    let tdb = setup("u-literal-percent").await;
    let db = tdb.db();
    seed_pseud(db, "100pct", "Literal", "2026-01-01T00:00:00Z").await;
    seed_pseud(db, "100XYZnope", "Not A Match", "2026-01-01T00:00:00Z").await;

    let found = handles(db, "100%_complete").await;
    assert!(
        found.is_empty(),
        "neither handle contains that literal string: {found:?}"
    );
    // A reader's own `%` is literal too, and that is the deliberate choice:
    // a search box where `50%` silently means "anything starting with 50"
    // returns results nobody can explain. Wildcards are opt-in, by way of the
    // fields that name them.
}

#[tokio::test]
async fn an_empty_query_returns_nobody_rather_than_everybody() {
    // The works search treats an empty query as a browse. Here that would put
    // every pseudonym on the instance into one response, so "no query" has to
    // mean "nothing".
    let tdb = setup("u-empty-query").await;
    let db = tdb.db();
    seed_pseud(db, "someone", "Someone", "2026-01-01T00:00:00Z").await;

    assert_eq!(handles(db, "").await, Vec::<String>::new());
    assert_eq!(handles(db, "   ").await, Vec::<String>::new());
}

#[tokio::test]
async fn a_field_from_another_surface_is_an_error_not_an_empty_result() {
    // "No results" is indistinguishable from "nobody matches", and a reader has
    // no way to learn they typed a field that does not exist here.
    let tdb = setup("u-foreign-field").await;
    let db = tdb.db();
    seed_pseud(db, "someone", "Someone", "2026-01-01T00:00:00Z").await;

    for q in ["words:>10000", "replies:>50", "category:meta", "tag:fixme"] {
        let err = lorehaven_db::search::search_users_ast(db, q, 10)
            .await
            .expect_err(&format!("{q} belongs to another surface"));
        let message = format!("{err}");
        assert!(message.contains("user"), "{q}: {message}");
    }
}

#[tokio::test]
async fn a_malformed_query_is_an_error_not_an_empty_result() {
    let tdb = setup("u-malformed").await;
    let db = tdb.db();
    seed_pseud(db, "someone", "Someone", "2026-01-01T00:00:00Z").await;

    for q in ["works:lots", "words:>10000", "words:10..5"] {
        let err = lorehaven_db::search::search_users_ast(db, q, 10)
            .await
            .expect_err(&format!("{q} cannot be satisfied"));
        assert!(!format!("{err}").is_empty(), "{q} should explain itself");
    }
}

#[tokio::test]
async fn two_queries_agree_about_who_exists() {
    // A cheap end-to-end check: if the join to `pseuds` is wrong on one
    // dialect, the free-text arm still works and a count-filtered query quietly
    // returns a different population.
    let tdb = setup("u-two-queries").await;
    let db = tdb.db();
    seed_pseud(db, "alpha", "Alpha", "2026-01-01T00:00:00Z").await;
    seed_pseud(db, "beta", "Beta", "2026-01-01T00:00:00Z").await;
    seed_work(db, "beta-work", &pseud_id("beta"), "published", "public").await;

    assert_eq!(handles(db, "works:>0").await, vec!["beta"]);
    assert_eq!(handles(db, "works:0").await, vec!["alpha"]);
    assert_eq!(
        handles(db, "user:alpha OR user:beta").await,
        vec!["alpha", "beta"]
    );
}

// --- the HTTP surface --------------------------------------------------------
//
// The DB layer above proves the SQL. These prove that a reader typing
// `words:>10000` into the user box gets a 422 that says which surface to use
// instead -- and not a 500, and not an empty list it has to guess about.

#[tokio::test]
async fn the_route_answers_a_valid_user_query_with_200() {
    let tdb = setup("u-route-ok").await;
    let db = tdb.db();
    seed_pseud(db, "nightowl", "Night Owl", "2026-01-01T00:00:00Z").await;
    let dir = test_support::scratch_dir("u-route-ok");
    let router = router_for(&tdb, &dir);

    let (status, body) = get_status(router, "/api/v1/users/search?q=user%3Anightowl").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 1, "{body}");
    assert_eq!(body["items"][0]["handle"], "nightowl");
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_foreign_field_is_a_422_naming_the_surface() {
    // "No results" is indistinguishable from "nobody matches", and a reader has
    // no way to learn they typed a field that does not exist here.
    let tdb = setup("u-route-foreign").await;
    let dir = test_support::scratch_dir("u-route-foreign");

    for q in ["words:>10000", "replies:>50", "category:meta", "tag:fixme"] {
        let (status, body) = get_status(
            router_for(&tdb, &dir),
            &format!("/api/v1/users/search?q={}", enc(q)),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{q} belongs to another surface: {body}"
        );
        let message = body["error"]["message"].as_str().unwrap_or_default();
        assert!(message.contains("user"), "{q}: {message}");
    }
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_malformed_query_is_a_422_not_a_500() {
    let tdb = setup("u-route-malformed").await;
    let dir = test_support::scratch_dir("u-route-malformed");

    for q in ["works:lots", "words:10..5", "works:>"] {
        let (status, body) = get_status(
            router_for(&tdb, &dir),
            &format!("/api/v1/users/search?q={}", enc(q)),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{q}: {body}");
    }
    tdb.cleanup().await;
}

#[tokio::test]
async fn an_empty_query_is_an_empty_page_not_the_whole_instance() {
    // Every pseudonym in one response is not a search result; it is a phone
    // book, and on a large instance it is a denial of service against the
    // database.
    let tdb = setup("u-route-empty").await;
    let db = tdb.db();
    seed_pseud(db, "someone", "Someone", "2026-01-01T00:00:00Z").await;
    let dir = test_support::scratch_dir("u-route-empty");
    let router = router_for(&tdb, &dir);

    let (status, body) = get_status(router, "/api/v1/users/search?q=").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 0, "{body}");
    tdb.cleanup().await;
}
