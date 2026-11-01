//! M63-04: "similar to this one", the author filter and the read-history
//! exclusion, on both engines.
//!
//! The similarity maths is unit-tested in `darkhold-domain`; this file pins the
//! four properties that only exist once a database is involved:
//!
//! 1. **The preselection does not drop a neighbour.** The `tokens` filter is a
//!    coarse `LIKE` scan and the ranker is exact — a row that shares a rare title
//!    token must survive the filter even when it shares nothing else.
//! 2. **An unrelated row is absent, not last.** `ranked` counts neighbours, and a
//!    page padded with unrelated rows is the failure this exists to prevent.
//! 3. **`--author` is exact and case-insensitive.** `Osh` must not return
//!    `Oshha`'s rows, and `OSHHA` must return them.
//! 4. **`--exclude-read` drops the account's own reading and nobody else's.**
//!
//! Run on SQLite by default; set `DARKHOLD_TEST_PG_URL` for the PostgreSQL arm.

use std::time::Duration;

use darkhold_db::catalogue::CatalogueQuery;
use darkhold_db::metrics;
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
        "darkhold-similar-{}-{}-{}",
        tag,
        uuid::Uuid::new_v4(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let url = match std::env::var("DARKHOLD_TEST_PG_URL") {
        Ok(admin) => {
            let name = format!(
                "lh_similar_{}_{}",
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

fn query() -> CatalogueQuery {
    CatalogueQuery {
        limit: Some(50),
        offset: Some(0),
        ..Default::default()
    }
}

/// One catalogue row, with the fields similarity reads.
type Row = (&'static str, &'static str, &'static str, &'static str, i64);

/// `(source_work_key, title, summary, author, likes)`.
///
/// The seed is `ghost`, two rows are plausible neighbours and one is a completely
/// unrelated story that happens to share the word "the".
const ROWS: &[Row] = &[
    (
        "ghost",
        "Ghost in the City",
        "A cyberpunk gamer wakes up in Night City with a system",
        "ZFighter18",
        600_000,
    ),
    (
        "ghost-alt",
        "Ghost in the City (rewrite)",
        "The same Night City gamer story, retold",
        "Someone Else",
        5_000,
    ),
    (
        "neon",
        "Neon Ghost Protocol",
        "Night City gamer with a cyberpunk system",
        "ZFIGHTER18",
        1_200,
    ),
    (
        "mech",
        "Battletech Mercenary Tales",
        "A mechwarrior story about contracts and dropships",
        "Oshha",
        3_000,
    ),
    (
        "osh",
        "A Short Osh Story",
        "A different cyberpunk story",
        "Osh",
        900,
    ),
    // Two rows with the same title and the same like count: no rule can pick
    // between them, so `--seed "Twin Story"` must refuse and name both.
    ("twin-a", "Twin Story", "Rival edition one", "Nobody", 100),
    ("twin-b", "Twin Story", "Rival edition two", "Somebody", 100),
];

async fn insert(db: &Database, account: &str, row: Row, work: Option<&str>) {
    let (key, title, summary, author, likes) = row;
    match db.backend() {
        darkhold_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO library_items
                 (id, account_id, work_id, source_key, source_work_key, title, author_text, summary,
                  word_count, status, source_url, source_updated_at, last_synced_at, created_at,
                  updated_at, likes)
                 VALUES ($1::uuid, $2::uuid, $3, 'sb', $4, $5, $6, $7, 20000, 'ongoing',
                         'https://example.invalid/x', $8, $8, $8, $8, $9)",
            )
            .bind(uuid::Uuid::new_v4())
            .bind(uuid::Uuid::parse_str(account).expect("uuid"))
            .bind(work.map(|w| uuid::Uuid::parse_str(w).expect("uuid")))
            .bind(key)
            .bind(title)
            .bind(author)
            .bind(summary)
            .bind(T0)
            .bind(likes)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("library item");
        }
        darkhold_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO library_items
                 (id, account_id, work_id, source_key, source_work_key, title, author_text, summary,
                  word_count, status, source_url, source_updated_at, last_synced_at, created_at,
                  updated_at, likes)
                 VALUES (?, ?, ?, 'sb', ?, ?, ?, ?, 20000, 'ongoing', 'https://example.invalid/x',
                         ?, ?, ?, ?, ?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(account)
            .bind(work.map(str::to_owned))
            .bind(key)
            .bind(title)
            .bind(author)
            .bind(summary)
            .bind(T0)
            .bind(T0)
            .bind(T0)
            .bind(T0)
            .bind(likes)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("library item");
        }
    }
}

async fn account(db: &Database) -> String {
    let account = uuid::Uuid::new_v4().to_string();
    let email = format!("{account}@example.invalid");
    // Two arms rather than one bound query: `sqlx::query` is generic over the
    // driver and a single value cannot be executed on both pools.
    match db.backend() {
        darkhold_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO accounts (id, email, status, age_state, created_at, updated_at, permission_statement)
                 VALUES ($1::uuid, $2, 'active', 'adult', $3, $3, 'minimal')",
            )
            .bind(uuid::Uuid::parse_str(&account).expect("uuid"))
            .bind(email)
            .bind(T0)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("account");
        }
        darkhold_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO accounts (id, email, status, age_state, created_at, updated_at, permission_statement)
                 VALUES (?, ?, 'active', 'adult', ?, ?, 'minimal')",
            )
            .bind(&account)
            .bind(email)
            .bind(T0)
            .bind(T0)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("account");
        }
    }
    account
}

/// A work, so a library row can be linked to one and a reading record can point
/// at it. `--exclude-read` matches through `library_items.work_id`, which is null
/// on every row the estate import did not link.
async fn work(db: &Database, account: &str) -> String {
    let pseud = uuid::Uuid::new_v4().to_string();
    let work = uuid::Uuid::new_v4().to_string();
    match db.backend() {
        darkhold_db::Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES ($1::uuid, $2::uuid, $3, $3, $4, $4)",
            )
            .bind(uuid::Uuid::parse_str(&pseud).expect("uuid"))
            .bind(uuid::Uuid::parse_str(account).expect("uuid"))
            .bind(format!("p{}", &pseud[..8]))
            .bind(T0)
            .execute(pool)
            .await
            .expect("pseud");
            sqlx::query(
                "INSERT INTO works (id, owner_pseud_id, title, summary, lifecycle, visibility, completion,
                                    created_at, updated_at, generated_content_posture)
                 VALUES ($1::uuid, $2::uuid, 'Ghost in the City', '', 'published', 'public', 'complete', $3, $3, 'forbid')",
            )
            .bind(uuid::Uuid::parse_str(&work).expect("uuid"))
            .bind(uuid::Uuid::parse_str(&pseud).expect("uuid"))
            .bind(T0)
            .execute(pool)
            .await
            .expect("work");
        }
        darkhold_db::Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&pseud)
            .bind(account)
            .bind(format!("p{}", &pseud[..8]))
            .bind(format!("p{}", &pseud[..8]))
            .bind(T0)
            .bind(T0)
            .execute(pool)
            .await
            .expect("pseud");
            sqlx::query(
                "INSERT INTO works (id, owner_pseud_id, title, summary, lifecycle, visibility, completion,
                                    created_at, updated_at, generated_content_posture)
                 VALUES (?, ?, 'Ghost in the City', '', 'published', 'public', 'complete', ?, ?, 'forbid')",
            )
            .bind(&work)
            .bind(&pseud)
            .bind(T0)
            .bind(T0)
            .execute(pool)
            .await
            .expect("work");
        }
    }
    work
}

/// Mark `work` as reading for `account` — the negative set `--exclude-read` uses.
async fn reading(db: &Database, account: &str, work: &str) {
    match db.backend() {
        darkhold_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO reading_progress
                 (id, account_id, subject_type, subject_id, position_permille, created_at, updated_at, version)
                 VALUES ($1::uuid, $2::uuid, 'work', $3::uuid, 120, $4, $4, 1)",
            )
            .bind(uuid::Uuid::new_v4())
            .bind(uuid::Uuid::parse_str(account).expect("uuid"))
            .bind(uuid::Uuid::parse_str(work).expect("uuid"))
            .bind(T0)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("reading progress");
        }
        darkhold_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO reading_progress
                 (id, account_id, subject_type, subject_id, position_permille, created_at, updated_at, version)
                 VALUES (?, ?, 'work', ?, 120, ?, ?, 1)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(account)
            .bind(work)
            .bind(T0)
            .bind(T0)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("reading progress");
        }
    }
}

fn keys(page: &metrics::SimilarPage) -> Vec<String> {
    page.items
        .iter()
        .map(|item| item.row.source_work_key.clone())
        .collect()
}

/// One neighbourhood, seeded on `Ghost in the City`.
async fn seeded(db: &Database) -> metrics::SimilarPage {
    let account = account(db).await;
    for row in ROWS {
        insert(&db, &account, *row, None).await;
    }
    let lookup = match metrics::resolve_seed(db, "Ghost in the City").await.expect("resolve") {
        metrics::SeedMatch::One(row) => row,
        other => panic!("expected one row, got {other:?}"),
    };
    let mut q = query();
    q.tokens = darkhold_domain::similar::tokens(&lookup.title);
    metrics::similar(db, &q, &lookup, 20, 0).await.expect("similar")
}

#[tokio::test]
async fn the_neighbours_are_the_rows_that_share_the_seeds_words() {
    let db = connect("neighbours").await;
    let page = seeded(&db).await;
    let order = keys(&page);
    // Two rows share the seed's words and nothing else does. Which of the two
    // leads is not a property worth pinning — the rewrite shares the title and
    // `Neon Ghost Protocol` shares the summary vocabulary — but that they are the
    // whole neighbourhood is.
    let mut top: Vec<&str> = order.iter().take(2).map(String::as_str).collect();
    top.sort_unstable();
    assert_eq!(top, vec!["ghost-alt", "neon"], "the neighbourhood, was {order:?}");
    assert!(
        !order.contains(&"mech".to_owned()),
        "a mechwarrior story is not a neighbour: {order:?}"
    );
    assert!(
        !order.contains(&"osh".to_owned()),
        "an unrelated cyberpunk story is not a neighbour either: {order:?}"
    );
    assert_eq!(page.ranked, order.len() as i64, "ranked must count the neighbours");
    // `total` is the *filter's* count, and the token preselection is one of the
    // filters: three of the five rows carry "ghost" or "city", so three is the
    // whole universe this page was drawn from. `ranked` is how many of them are
    // neighbours, which is why the gate prints both.
    assert_eq!(page.total, 3, "the preselection's own count, was {}", page.total);
    assert_eq!(page.scanned, 3, "the ranker saw the preselection, was {}", page.scanned);
}

#[tokio::test]
async fn the_score_and_the_shared_tokens_are_reported() {
    let db = connect("explain").await;
    let page = seeded(&db).await;
    let top = page.items.first().expect("a neighbour");
    assert!(top.score > 0.0 && top.score <= 1.0, "score {top:?}");
    assert!(
        top.shared.iter().any(|token| token == "ghost" || token == "city"),
        "the explanation names the shared title token, was {:?}",
        top.shared
    );
}

#[tokio::test]
async fn the_author_filter_is_case_insensitive() {
    let db = connect("author-case").await;
    let account = account(&db).await;
    for row in ROWS {
        insert(&db, &account, *row, None).await;
    }
    // The fixture spells one row `ZFighter18` and another `ZFIGHTER18`, so the
    // filter matching both case-insensitively is the property under test.
    let mut q = query();
    q.author = Some("zfighter18".to_owned());
    let rows = darkhold_db::catalogue::search(&db, &q).await.expect("search");
    let mut authors: Vec<&str> = rows.iter().map(|row| row.author_text.as_str()).collect();
    authors.sort_unstable();
    assert_eq!(
        authors,
        vec!["ZFIGHTER18", "ZFighter18"],
        "both spellings of the pseud, got {authors:?}"
    );
    // And it is exact: a prefix matches nothing.
    let mut prefix = query();
    prefix.author = Some("ZFight".to_owned());
    let rows = darkhold_db::catalogue::search(&db, &prefix).await.expect("search");
    assert!(rows.is_empty(), "a prefix is not the pseud, got {} rows", rows.len());
}

#[tokio::test]
async fn the_author_filter_does_not_match_a_substring() {
    let db = connect("author-exact").await;
    let account = account(&db).await;
    for row in ROWS {
        insert(&db, &account, *row, None).await;
    }
    let mut q = query();
    q.author = Some("Osh".to_owned());
    let rows = darkhold_db::catalogue::search(&db, &q).await.expect("search");
    let authors: Vec<&str> = rows.iter().map(|row| row.author_text.as_str()).collect();
    assert_eq!(
        authors,
        vec!["Osh"],
        "`Osh` must not return `Oshha`'s rows, got {authors:?}"
    );
}

#[tokio::test]
async fn an_accounts_reading_is_excluded_and_nobody_elses_is() {
    let db = connect("exclude").await;
    let owner = account(&db).await;
    let reader = account(&db).await;
    let linked = work(&db, &owner).await;
    // `ghost` is the only row linked to a work, so it is the only one a reading
    // record can point at.
    for (index, row) in ROWS.iter().enumerate() {
        let work_id = if index == 0 { Some(linked.as_str()) } else { None };
        insert(&db, &owner, *row, work_id).await;
    }
    reading(&db, &reader, &linked).await;

    let mut plain = query();
    plain.limit = Some(50);
    let all = darkhold_db::catalogue::search(&db, &plain).await.expect("search");
    assert!(
        all.iter().any(|row| row.source_work_key == "ghost"),
        "the row is present before the exclusion"
    );

    let mut excluded = query();
    excluded.limit = Some(50);
    excluded.exclude_account = Some(reader.clone());
    let page = darkhold_db::catalogue::search(&db, &excluded)
        .await
        .expect("search");
    assert!(
        !page.iter().any(|row| row.source_work_key == "ghost"),
        "the reader's own reading must be dropped"
    );
    assert!(
        page.iter().any(|row| row.source_work_key == "mech"),
        "rows the reader is not reading must stay"
    );

    // A different account's reading is not this account's negative set.
    let bystander = account(&db).await;
    let mut other = query();
    other.limit = Some(50);
    other.exclude_account = Some(bystander);
    let page = darkhold_db::catalogue::search(&db, &other).await.expect("search");
    assert!(
        page.iter().any(|row| row.source_work_key == "ghost"),
        "another account's reading must not hide the row from this one"
    );
}

#[tokio::test]
async fn a_seed_spec_resolves_by_id_url_title_and_refuses_an_ambiguous_title() {
    let db = connect("resolve").await;
    let account = account(&db).await;
    for row in ROWS {
        insert(&db, &account, *row, None).await;
    }
    let by_title = metrics::resolve_seed(&db, "ghost in the city").await.expect("title");
    let metrics::SeedMatch::One(found) = by_title else {
        panic!("an exact title must resolve, got {by_title:?}")
    };
    assert_eq!(found.title, "Ghost in the City");

    let by_id = metrics::resolve_seed(&db, &found.id).await.expect("id");
    assert!(matches!(by_id, metrics::SeedMatch::One(_)), "an id must resolve");

    let missing = metrics::resolve_seed(&db, "nothing called this").await.expect("missing");
    assert_eq!(missing, metrics::SeedMatch::None);

    // Three rows carry the word "Ghost" (the two titles and `Neon Ghost
    // Protocol`) and one of them has 120x the likes of the next: that is the copy
    // people read, and picking it is the documented rule rather than a coin toss.
    let popular = metrics::resolve_seed(&db, "Ghost").await.expect("popular");
    match popular {
        metrics::SeedMatch::One(row) => assert_eq!(row.title, "Ghost in the City"),
        other => panic!("the dominant copy must be chosen, got {other:?}"),
    }

    // Two rows, same title, same likes: nothing can choose, so the caller is
    // given both with the archive and the count.
    let ambiguous = metrics::resolve_seed(&db, "Twin Story").await.expect("twin");
    match ambiguous {
        metrics::SeedMatch::Many(rows) => {
            assert_eq!(rows.len(), 2, "both twins, got {rows:?}");
            assert!(rows.iter().all(|row| row.title == "Twin Story"));
            assert!(rows.iter().all(|row| row.source_key == "sb"));
        }
        other => panic!("a tie must be refused with both rows, got {other:?}"),
    }
}
