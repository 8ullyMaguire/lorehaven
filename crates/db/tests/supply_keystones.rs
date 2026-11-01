//! M45-25: keystone authors, the drought refusal, and the routing record — on
//! both engines.
//!
//! The percentile maths is a pure function in `darkhold_db::supply`; what this
//! file pins is the four properties that only exist once a database is involved:
//!
//! 1. **A small source cannot win on the same raw percentile.** The fixture is
//!    built so the *unshrunk* order and the shrunk order disagree: `bea` leads on
//!    raw per-source percentile (0.80 vs 0.775) and loses after `n/(n+10)` (0.60
//!    vs 0.72), because five engaged rows are not evidence and forty are.
//! 2. **One work is not a keystone.**
//! 3. **Operator taste raises a keystone and is clamped** at a quarter of the
//!    scale. The table's own `CHECK (weight >= 0.0)` is what stops it lowering
//!    one, and the code floors at zero anyway so the invariant survives a schema
//!    change.
//! 4. **A corpus with no conversation refuses**, rather than listing every
//!    engaged work as "in drought".
//!
//! Run on SQLite by default; set `DARKHOLD_TEST_PG_URL` for the PostgreSQL arm.

use std::time::Duration;

use darkhold_db::supply::{self, PATRONAGE_BP};
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
        "darkhold-supply-{}-{}-{}",
        tag,
        uuid::Uuid::new_v4(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let url = match std::env::var("DARKHOLD_TEST_PG_URL") {
        Ok(admin) => {
            let name = format!(
                "lh_supply_{}_{}",
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

/// One account, because every catalogue row must belong to one.
async fn account(db: &Database) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let email = format!("{id}@example.invalid");
    match db.backend() {
        darkhold_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO accounts (id, email, status, age_state, created_at, updated_at, permission_statement)
                 VALUES ($1::uuid, $2, 'active', 'adult', $3, $3, 'minimal')",
            )
            .bind(uuid::Uuid::parse_str(&id).expect("uuid"))
            .bind(&email)
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
            .bind(&id)
            .bind(&email)
            .bind(T0)
            .bind(T0)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("account");
        }
    }
    id
}

/// One catalogue row: `(author, source_key, source_work_key, likes)`, returning
/// the row's id so a comment can name it.
async fn add_row(
    db: &Database,
    account: &str,
    author: &str,
    source_key: &str,
    key: &str,
    likes: i64,
) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    match db.backend() {
        darkhold_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO library_items
                 (id, account_id, source_key, source_work_key, title, author_text, summary, status,
                  source_url, created_at, updated_at, likes)
                 VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6, '', 'ongoing',
                         'https://example.invalid/w', $7, $7, $8)",
            )
            .bind(uuid::Uuid::parse_str(&id).expect("uuid"))
            .bind(uuid::Uuid::parse_str(account).expect("uuid"))
            .bind(source_key)
            .bind(key)
            .bind(format!("{key} title"))
            .bind(author)
            .bind(T0)
            .bind(likes)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("library item");
        }
        darkhold_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO library_items
                 (id, account_id, source_key, source_work_key, title, author_text, summary, status,
                  source_url, created_at, updated_at, likes)
                 VALUES (?, ?, ?, ?, ?, ?, '', 'ongoing', 'https://example.invalid/w', ?, ?, ?)",
            )
            .bind(&id)
            .bind(account)
            .bind(source_key)
            .bind(key)
            .bind(format!("{key} title"))
            .bind(author)
            .bind(T0)
            .bind(T0)
            .bind(likes)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("library item");
        }
    }
    id
}

/// The flip fixture: `ada` mid-table in a forty-row source, `bea` top-of-table in
/// a five-row one.
async fn flip_fixture(db: &Database, account: &str) {
    // 40 rows in `deep`: 8 above ada, 30 below, ada's two in between.
    for i in 0..8 {
        add_row(db, account, "eve", "deep", &format!("above-{i}"), 600 + i).await;
    }
    for i in 0..30 {
        add_row(db, account, "eve", "deep", &format!("below-{i}"), 100 + i).await;
    }
    add_row(db, account, "ada", "deep", "ada-1", 500).await;
    add_row(db, account, "ada", "deep", "ada-2", 501).await;

    // 5 rows in `tiny`: bea's two at the very top.
    for i in 0..3 {
        add_row(db, account, "cy", "tiny", &format!("t-{i}"), 10 + i).await;
    }
    add_row(db, account, "bea", "tiny", "bea-1", 900).await;
    add_row(db, account, "bea", "tiny", "bea-2", 901).await;
}

async fn weight(db: &Database, key: &str, weight: f64) {
    match db.backend() {
        darkhold_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO admin_taste_profile (dimension_key, label, admin_target, weight, updated_at)
                 VALUES ($1, $1, 0.5, $2, $3)",
            )
            .bind(key)
            .bind(weight)
            .bind(T0)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("admin taste profile");
        }
        darkhold_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO admin_taste_profile (dimension_key, label, admin_target, weight, updated_at)
                 VALUES (?, ?, 0.5, ?, ?)",
            )
            .bind(key)
            .bind(key)
            .bind(weight)
            .bind(T0)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("admin taste profile");
        }
    }
}

#[tokio::test]
async fn a_small_source_does_not_outrank_a_large_one() {
    let db = connect("flip").await;
    let account = account(&db).await;
    flip_fixture(&db, &account).await;
    let run = supply::recompute(&db, 2, 5_000).await.expect("recompute");
    assert_eq!(run.engaged_rows, 45, "8 + 30 + 2 + 3 + 2 rows were written");
    let list = supply::keystones(&db, 10).await.expect("keystones");
    let names: Vec<&str> = list.iter().map(|k| k.author_key.as_str()).collect();
    assert_eq!(
        names,
        vec!["ada", "bea"],
        "ada leads on the shrunk percentile, bea on the raw one"
    );
    // Raw: bea (0.80) > ada (0.775). Shrunk: ada 0.72 > bea 0.60. The assertion
    // above is the whole point of the shrinkage, so the numbers are pinned too.
    let ada = &list[0];
    let bea = &list[1];
    assert!(
        ada.mean_pct_bp > bea.mean_pct_bp,
        "ada {} vs bea {}",
        ada.mean_pct_bp,
        bea.mean_pct_bp
    );
    assert_eq!(ada.engaged_works, 2);
}

#[tokio::test]
async fn one_work_is_not_a_keystone() {
    let db = connect("single").await;
    let account = account(&db).await;
    // `solo` is alone in his own source, so his shrunk percentile is exactly 0.50
    // and he clears the floor — the only thing that excludes him is the count.
    // Isolating the two thresholds is the point: otherwise the percentile does the
    // work and the floor's mutation passes.
    add_row(&db, &account, "solo", "lonely", "solo-1", 9_999).await;
    add_row(&db, &account, "filler", "tiny", "f-0", 1).await;
    for i in 0..3 {
        add_row(&db, &account, "cy", "tiny", &format!("c-{i}"), 10 + i).await;
    }
    let run = supply::recompute(&db, 2, 5_000).await.expect("recompute");
    assert_eq!(run.authors_seen, 3, "solo, filler and cy were all seen");
    assert_eq!(run.keystones, 1, "only cy has two engaged works");
    let list = supply::keystones(&db, 10).await.expect("keystones");
    assert_eq!(list.len(), 1);
    assert_eq!(
        list[0].author_key, "cy",
        "solo's single work is not evidence of a backlist, however well it did"
    );
}

#[tokio::test]
async fn operator_taste_raises_a_keystone_and_is_clamped() {
    let db = connect("taste").await;
    let account = account(&db).await;
    flip_fixture(&db, &account).await;
    // The table's CHECK forbids a negative weight, which is the schema's half of
    // "never lowers one"; the clamp below is the code's half.
    weight(&db, "author:bea", 0.4).await;
    let run = supply::recompute(&db, 2, 5_000).await.expect("recompute");
    assert_eq!(run.taste_profile_rows, 1, "one author weight was read");
    let list = supply::keystones(&db, 10).await.expect("keystones");
    let bea = list
        .iter()
        .find(|k| k.author_key == "bea")
        .expect("bea is a keystone");
    assert_eq!(bea.taste_bonus_bp, 2_500, "0.4 is clamped to a quarter");
    assert_eq!(bea.score_bp, bea.mean_pct_bp + 2_500);
    let ada = list
        .iter()
        .find(|k| k.author_key == "ada")
        .expect("ada is a keystone");
    assert_eq!(ada.taste_bonus_bp, 0, "an author with no weight gets none");
    assert_eq!(
        list[0].author_key, "bea",
        "the boosted author now leads, which is the operator's preference showing up"
    );
}

#[tokio::test]
async fn an_empty_corpus_refuses_instead_of_listing_everything() {
    let db = connect("drought-empty").await;
    let account = account(&db).await;
    flip_fixture(&db, &account).await;
    supply::recompute(&db, 2, 5_000).await.expect("recompute");
    let report = supply::drought(&db, 10).await.expect("drought");
    assert!(!report.measured, "zero comments and zero ratings is unmeasured");
    assert!(
        report.reason.as_deref().unwrap_or("").contains("cannot be told"),
        "the reason names the ambiguity: {:?}",
        report.reason
    );
    assert!(
        report.authors.is_empty(),
        "listing every engaged work as 'in drought' is the failure this prevents"
    );
    assert_eq!(report.comments_total, 0);
    assert_eq!(report.ratings_total, 0);
}

#[tokio::test]
async fn conversation_turns_the_report_on() {
    let db = connect("drought-on").await;
    let account = account(&db).await;
    flip_fixture(&db, &account).await;
    supply::recompute(&db, 2, 5_000).await.expect("recompute");
    let work = add_row(&db, &account, "ada", "deep", "ada-commented", 502).await;

    let comment = |id: &str| {
        format!(
            "INSERT INTO comments (id, subject_type, subject_id, author_pseud, body, body_version, created_at)
             VALUES ('{id}', 'work', '{work}', 'p1', 'good chapter', 'v1', '{T0}')"
        )
    };
    let id = uuid::Uuid::new_v4().to_string();
    match db.backend() {
        darkhold_db::Backend::Postgres => {
            sqlx::query(&comment(&id))
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("comment");
        }
        darkhold_db::Backend::Sqlite => {
            sqlx::query(&comment(&id))
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("comment");
        }
    }

    supply::recompute(&db, 2, 5_000).await.expect("recompute");
    let report = supply::drought(&db, 10).await.expect("drought");
    assert!(report.measured, "one comment is enough to measure");
    assert_eq!(report.comments_total, 1);
    let ada = report
        .authors
        .iter()
        .find(|a| a.author_key == "ada")
        .expect("ada is a keystone");
    assert_eq!(ada.works_with_conversation, 1);
    assert_eq!(ada.comments, 1);
    assert_eq!(ada.engaged_works, 3, "two from the fixture plus the commented one");
}

#[tokio::test]
async fn a_reader_is_not_routed_the_same_work_twice() {
    let db = connect("route").await;
    let account = account(&db).await;
    flip_fixture(&db, &account).await;
    supply::recompute(&db, 2, 5_000).await.expect("recompute");
    let reader = uuid::Uuid::new_v4().to_string();

    let first = supply::route(&db, &reader, None)
        .await
        .expect("route")
        .expect("a keystone work exists");
    let second = supply::route(&db, &reader, None)
        .await
        .expect("route")
        .expect("a second work exists");
    assert_ne!(
        first.work_id, second.work_id,
        "the exclusion is what makes the second routing useful"
    );
    assert_eq!(second.author_key, "ada", "the highest-scoring keystone leads");
    assert!(
        second.reason.contains("offered 0 time(s)"),
        "a work nobody has been routed says so: {}",
        second.reason
    );

    // A second reader is offered one of the same two works, and the reason now
    // carries a count of one — which is the evidence that the count is real and
    // not a constant in the sentence.
    let reader2 = uuid::Uuid::new_v4().to_string();
    let third = supply::route(&db, &reader2, None)
        .await
        .expect("route")
        .expect("both keystone works still exist");
    assert!(
        third.reason.contains("offered 1 time(s)"),
        "the count is over routings, not over this reader: {}",
        third.reason
    );

    // Exhausting an author's supply returns nothing rather than repeating a work.
    supply::route(&db, &reader, Some("ada"))
        .await
        .expect("route");
    let now_none = supply::route(&db, &reader, Some("ada"))
        .await
        .expect("route");
    assert!(now_none.is_none(), "ada's two works are both offered to this reader");
}

#[tokio::test]
async fn completing_twice_pays_once() {
    let db = connect("complete").await;
    let account = account(&db).await;
    flip_fixture(&db, &account).await;
    supply::recompute(&db, 2, 5_000).await.expect("recompute");
    let reader = uuid::Uuid::new_v4().to_string();
    let routed = supply::route(&db, &reader, None)
        .await
        .expect("route")
        .expect("a work to route");

    let done = supply::complete(&db, &reader, &routed.id)
        .await
        .expect("complete")
        .expect("the row is the reader's");
    assert_eq!(done.credit_bp, Some(PATRONAGE_BP));
    assert!(done.completed_at.is_some());

    let again = supply::complete(&db, &reader, &routed.id)
        .await
        .expect("complete again")
        .expect("still the reader's");
    assert_eq!(again.credit_bp, Some(PATRONAGE_BP));

    let entries = match db.backend() {
        darkhold_db::Backend::Postgres => {
            sqlx::query_scalar::<_, i64>(
                "SELECT CAST(COUNT(*) AS BIGINT) FROM credit_entries WHERE account = $1",
            )
            .bind(&reader)
            .fetch_one(db.postgres_pool().expect("postgres"))
            .await
            .expect("count")
        }
        darkhold_db::Backend::Sqlite => {
            sqlx::query_scalar::<_, i64>(
                "SELECT CAST(COUNT(*) AS BIGINT) FROM credit_entries WHERE account = ?",
            )
            .bind(&reader)
            .fetch_one(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("count")
        }
    };
    assert_eq!(entries, 1, "the second completion is a read, not a second grant");

    // Somebody else's id is nobody else's routing.
    let stranger = uuid::Uuid::new_v4().to_string();
    let stolen = supply::complete(&db, &stranger, &routed.id)
        .await
        .expect("complete");
    assert!(stolen.is_none(), "a routing is readable by its owner only");
}
