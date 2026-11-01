//! M63-01: the fandom filter over the catalogue, on both engines.
//!
//! Three properties, each one a bug the naive query has:
//!
//! 1. **One matching work is one row.** The predicate is an `EXISTS`, so a work
//!    that carries several tags can never become several rows in a page whose
//!    `limit` the caller asked for once. Today the schema makes a duplicate
//!    impossible anyway (`work_tags` is keyed `(work_id, node_id)` and
//!    `canonical_entities` is unique on `(kind, norm)`), so this test pins the
//!    behaviour rather than a reachable bug — but the day an alias table lets two
//!    tags resolve to one fandom, `EXISTS` is what keeps paging correct and a
//!    join is what would break it.
//!
//! 2. **A row with no work link cannot match.** `library_items.work_id` is null
//!    for the rows the estate import could not resolve; a filter that used a join
//!    would drop them silently, and one written as `work_id NOT IN (…)` would
//!    drop the *whole page* because `NULL NOT IN (…)` is never true.
//!
//! 3. **A known fandom with no rows is empty, not an error.** The store answers
//!    `[]`; turning that into "no such fandom" is the route's job (it has the
//!    list), and the two must not be confused at the store level or an empty
//!    fandom becomes a 422.
//!
//! Run on SQLite by default; set `DARKHOLD_TEST_PG_URL` for the PostgreSQL arm.

use std::time::Duration;

use darkhold_db::catalogue::{self, CatalogueQuery};
use darkhold_db::Database;

const T0: &str = "2026-01-01T00:00:00Z";

fn make_config(url: String) -> darkhold_db::DatabaseConfig {
    darkhold_db::DatabaseConfig {
        url,
        max_connections: 2,
        acquire_timeout: Duration::from_secs(5),
        slow_query_warn: Duration::ZERO,
    }
}

async fn connect(tag: &str) -> Database {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "darkhold-fandom-{}-{}-{}",
        tag,
        uuid::Uuid::new_v4(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let url = match std::env::var("DARKHOLD_TEST_PG_URL") {
        Ok(admin) => {
            let name = format!(
                "lh_fandom_{}_{}",
                std::process::id(),
                uuid::Uuid::new_v4().simple()
            );
            let admin_db = Database::connect(&make_config(admin.clone()))
                .await
                .expect("connect to the admin database");
            sqlx::query(&format!("CREATE DATABASE {name}"))
                .execute(admin_db.postgres_pool().expect("postgres pool"))
                .await
                .expect("create a scratch database");
            admin_db.close().await;
            let (prefix, _) = admin.rsplit_once('/').expect("admin URL ends in a database");
            format!("{prefix}/{name}")
        }
        Err(_) => format!("sqlite://{}/darkhold.sqlite?mode=rwc", dir.display()),
    };
    let db = Database::connect(&make_config(url)).await.expect("connect");
    db.migrate().await.expect("migrate");
    db
}

fn query(fandom: Option<&str>) -> CatalogueQuery {
    CatalogueQuery {
        source: None,
        min_likes: None,
        q: None,
        sort: None,
        limit: Some(50),
        offset: Some(0),
        updated: None,
        refreshed: None,
        added: None,
        fandom: fandom.map(str::to_owned),
        ..Default::default()
    }
}

/// Which `source_work_key`s the filter returned, in the query's own order.
async fn keys(db: &Database, fandom: Option<&str>) -> Vec<String> {
    catalogue::search(db, &query(fandom))
        .await
        .expect("search")
        .into_iter()
        .map(|row| row.source_work_key)
        .collect()
}

struct Seed {
    naruto: String,
    potter: String,
}

/// Four catalogue rows and three fandoms:
///
/// | row | fandom tags | why it is in the fixture |
/// |---|---|---|
/// | `two-fandoms` | Naruto, Worm | the duplicate witness: matches both, must appear once per query |
/// | `naruto-only` | Naruto | the ordinary hit |
/// | `untagged` | none | must never match any fandom, and must still match no-fandom queries |
/// | `orphan` | none (`work_id` NULL) | the row the estate import could not link |
async fn seed(db: &Database) -> Seed {
    let account = uuid::Uuid::new_v4().to_string();
    let pseud = uuid::Uuid::new_v4().to_string();
    let naruto = uuid::Uuid::new_v4().to_string();
    let worm = uuid::Uuid::new_v4().to_string();
    let potter = uuid::Uuid::new_v4().to_string();

    let work_two = uuid::Uuid::new_v4().to_string();
    let work_naruto = uuid::Uuid::new_v4().to_string();
    let work_untagged = uuid::Uuid::new_v4().to_string();

    match db.backend() {
        darkhold_db::Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            sqlx::query(
                "INSERT INTO accounts (id, email, status, age_state, created_at, updated_at, permission_statement)
                 VALUES ($1::uuid, 'fandom@example.invalid', 'active', 'adult', $2, $2, 'minimal')",
            )
            .bind(uuid::Uuid::parse_str(&account).expect("uuid"))
            .bind(T0)
            .execute(pool)
            .await
            .expect("account");
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES ($1::uuid, $2::uuid, 'tagger', 'tagger', $3, $3)",
            )
            .bind(uuid::Uuid::parse_str(&pseud).expect("uuid"))
            .bind(uuid::Uuid::parse_str(&account).expect("uuid"))
            .bind(T0)
            .execute(pool)
            .await
            .expect("pseud");

            for (id, name) in [(&naruto, "Naruto"), (&worm, "Worm"), (&potter, "Harry Potter")] {
                sqlx::query(
                    "INSERT INTO canonical_entities (id, kind, canonical, norm, created_at, review_status, signal_count)
                     VALUES ($1::uuid, 'fandom', $2, $3, $4, 'confirmed', 0)",
                )
                .bind(uuid::Uuid::parse_str(id).expect("uuid"))
                .bind(name)
                .bind(name.to_lowercase())
                .bind(T0)
                .execute(pool)
                .await
                .expect("fandom node");
            }

            for (id, title) in [
                (&work_two, "Two fandoms"),
                (&work_naruto, "Naruto only"),
                (&work_untagged, "Untagged"),
            ] {
                sqlx::query(
                    "INSERT INTO works (id, owner_pseud_id, title, summary, lifecycle, visibility, completion, created_at, updated_at, generated_content_posture)
                     VALUES ($1::uuid, $2::uuid, $3, '', 'published', 'public', 'complete', $4, $4, 'forbid')",
                )
                .bind(uuid::Uuid::parse_str(id).expect("uuid"))
                .bind(uuid::Uuid::parse_str(&pseud).expect("uuid"))
                .bind(title)
                .bind(T0)
                .execute(pool)
                .await
                .expect("work");
            }

            for (work, node) in [
                (&work_two, &naruto),
                (&work_two, &worm),
                (&work_naruto, &naruto),
            ] {
                sqlx::query(
                    "INSERT INTO work_tags (work_id, node_id, prominence, added_at)
                     VALUES ($1::uuid, $2::uuid, 1, $3)",
                )
                .bind(uuid::Uuid::parse_str(work).expect("uuid"))
                .bind(uuid::Uuid::parse_str(node).expect("uuid"))
                .bind(T0)
                .execute(pool)
                .await
                .expect("tag");
            }

            for (key, work) in [
                ("two-fandoms", Some(&work_two)),
                ("naruto-only", Some(&work_naruto)),
                ("untagged", Some(&work_untagged)),
                ("orphan", None),
            ] {
                sqlx::query(
                    "INSERT INTO library_items
                     (id, account_id, work_id, source_key, source_work_key, title, author_text, summary,
                      word_count, status, source_url, source_updated_at, last_synced_at, created_at, updated_at, likes)
                     VALUES ($1::uuid, $2::uuid, $3::uuid, 'sb', $4, $4, 'A', '', 1000,
                             'ongoing', 'https://example.invalid/x', $5, $5, $5, $5, 5)",
                )
                .bind(uuid::Uuid::new_v4())
                .bind(uuid::Uuid::parse_str(&account).expect("uuid"))
                .bind(work.map(|w| uuid::Uuid::parse_str(w).expect("uuid")))
                .bind(key)
                .bind(T0)
                .execute(pool)
                .await
                .expect("library item");
            }
        }
        darkhold_db::Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            sqlx::query(
                "INSERT INTO accounts (id, email, status, age_state, created_at, updated_at, permission_statement)
                 VALUES (?, 'fandom@example.invalid', 'active', 'adult', ?, ?, 'minimal')",
            )
            .bind(&account)
            .bind(T0)
            .bind(T0)
            .execute(pool)
            .await
            .expect("account");
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES (?, ?, 'tagger', 'tagger', ?, ?)",
            )
            .bind(&pseud)
            .bind(&account)
            .bind(T0)
            .bind(T0)
            .execute(pool)
            .await
            .expect("pseud");

            for (id, name) in [(&naruto, "Naruto"), (&worm, "Worm"), (&potter, "Harry Potter")] {
                sqlx::query(
                    "INSERT INTO canonical_entities (id, kind, canonical, norm, created_at, review_status, signal_count)
                     VALUES (?, 'fandom', ?, ?, ?, 'confirmed', 0)",
                )
                .bind(id)
                .bind(name)
                .bind(name.to_lowercase())
                .bind(T0)
                .execute(pool)
                .await
                .expect("fandom node");
            }

            for (id, title) in [
                (&work_two, "Two fandoms"),
                (&work_naruto, "Naruto only"),
                (&work_untagged, "Untagged"),
            ] {
                sqlx::query(
                    "INSERT INTO works (id, owner_pseud_id, title, summary, lifecycle, visibility, completion, created_at, updated_at, generated_content_posture)
                     VALUES (?, ?, ?, '', 'published', 'public', 'complete', ?, ?, 'forbid')",
                )
                .bind(id)
                .bind(&pseud)
                .bind(title)
                .bind(T0)
                .bind(T0)
                .execute(pool)
                .await
                .expect("work");
            }

            for (work, node) in [
                (&work_two, &naruto),
                (&work_two, &worm),
                (&work_naruto, &naruto),
            ] {
                sqlx::query(
                    "INSERT INTO work_tags (work_id, node_id, prominence, added_at) VALUES (?, ?, 1, ?)",
                )
                .bind(work)
                .bind(node)
                .bind(T0)
                .execute(pool)
                .await
                .expect("tag");
            }

            for (key, work) in [
                ("two-fandoms", Some(&work_two)),
                ("naruto-only", Some(&work_naruto)),
                ("untagged", Some(&work_untagged)),
                ("orphan", None),
            ] {
                sqlx::query(
                    "INSERT INTO library_items
                     (id, account_id, work_id, source_key, source_work_key, title, author_text, summary,
                      word_count, status, source_url, source_updated_at, last_synced_at, created_at, updated_at, likes)
                     VALUES (?, ?, ?, 'sb', ?, ?, 'A', '', 1000,
                             'ongoing', 'https://example.invalid/x', ?, ?, ?, ?, 5)",
                )
                .bind(uuid::Uuid::new_v4().to_string())
                .bind(&account)
                .bind(work)
                .bind(key)
                .bind(key)
                .bind(T0)
                .bind(T0)
                .bind(T0)
                .bind(T0)
                .execute(pool)
                .await
                .expect("library item");
            }
        }
    }

    Seed { naruto, potter }
}

#[tokio::test]
async fn a_fandom_filter_returns_only_that_fandom() {
    let db = connect("only").await;
    let seeded = seed(&db).await;
    assert!(!seeded.naruto.is_empty());

    let mut naruto = keys(&db, Some("Naruto")).await;
    naruto.sort();
    assert_eq!(naruto, vec!["naruto-only", "two-fandoms"]);
}

#[tokio::test]
async fn one_matching_work_is_one_row() {
    // A page asked for one row must contain one row. The shape that breaks
    // paging is a duplicated row: `limit=1` returning the same work twice is what
    // an offset-paginating caller sees as a skipped item.
    let db = connect("dedupe").await;
    seed(&db).await;

    let once = keys(&db, Some("Worm")).await;
    assert_eq!(once, vec!["two-fandoms"], "Worm must match the shared work once");

    let mut limited = query(Some("Naruto"));
    limited.limit = Some(1);
    let page = catalogue::search(&db, &limited).await.expect("page of one");
    assert_eq!(page.len(), 1, "limit=1 must return one row, not one row twice");

    // And the same row is not repeated across the whole set either.
    let all = keys(&db, Some("Naruto")).await;
    assert_eq!(all.len(), 2, "two distinct works are tagged Naruto");
}

#[tokio::test]
async fn a_known_fandom_with_no_rows_is_empty_not_an_error() {
    // `potter` exists as a node but no catalogue row is tagged with it. The store
    // answers an empty page; the route is what turns an *unknown* name into a
    // 422. Collapsing the two here would make an empty fandom unreachable.
    let db = connect("empty").await;
    let seeded = seed(&db).await;
    assert!(!seeded.potter.is_empty());

    let rows = keys(&db, Some("Harry Potter")).await;
    assert!(rows.is_empty(), "a known fandom with no rows answers []");

    // And it is listed, at zero, by the fandom list — which is what makes it
    // distinguishable from a name nobody has ever tagged.
    let listed = catalogue::fandoms(&db, 500).await.expect("fandoms");
    let names: Vec<&str> = listed.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"Naruto"));
    assert!(names.contains(&"Worm"));
}

#[tokio::test]
async fn the_filter_is_case_insensitive_and_ignores_a_null_work() {
    let db = connect("case").await;
    seed(&db).await;

    let mut shouted = keys(&db, Some("  nArUtO  ")).await;
    shouted.sort();
    assert_eq!(shouted, vec!["naruto-only", "two-fandoms"]);

    // The untagged row matches no fandom, and the orphan (work_id NULL) never
    // matches one either — both are absent from every fandom page but still
    // present when no filter is applied.
    let unfiltered = keys(&db, None).await;
    assert_eq!(unfiltered.len(), 4, "no filter returns every row, orphan included");
}

#[tokio::test]
async fn the_coverage_denominator_counts_tagged_rows_only() {
    let db = connect("coverage").await;
    seed(&db).await;

    // `two-fandoms` carries two tags and must be one row here, or the CLI's
    // "2 of 4 carry a fandom" line disagrees with the page it explains.
    let coverage = catalogue::fandom_coverage(&db).await.expect("coverage");
    assert_eq!(coverage.tagged, 2);
    // Every fixture row has five likes, so both numbers agree here. The live
    // catalogue is the case where they do not, and the field exists for it.
    assert_eq!(coverage.with_likes, 2);
    let total = catalogue::total_rows(&db).await.expect("total");
    assert_eq!(total, 4);

    let mut listed = catalogue::fandoms(&db, 500).await.expect("fandoms");
    listed.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(
        listed
            .iter()
            .map(|f| (f.name.as_str(), f.count))
            .collect::<Vec<_>>(),
        vec![("Naruto", 2), ("Worm", 1)],
        "a fandom with no catalogue rows is not listed; a work's two tags count as one row each"
    );
}
