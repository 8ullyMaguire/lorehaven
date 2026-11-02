//! M45-29 — §60 finisher signals, on both engines.
//!
//! What the domain tests cannot reach:
//!
//! 1. Only `complete` counts as finished; only `complete` + `abandoned` count as concluded.
//!    `in_progress` is neither — a work being written is not a verdict.
//! 2. An author with no works gets zeroes, not a failed query (`SUM` over no rows is NULL).
//! 3. Revision days come back distinct and ascending.
//! 4. A work whose revisions all landed on one day is **unmeasured**, with a reason — the import
//!    shape, which is the live instance's shape.
//! 5. A work with days on both sides of a gap has that gap as its cadence.
//! 6. An author's cadence pools their measurable works and is unmeasured when they have none.
//! 7. A schema without `revisions` answers unmeasured with a reason, never an error.
//! 8. The summary sentence carries no negative word.
//!
//! Run on SQLite by default; set `DARKHOLD_TEST_PG_URL` for the PostgreSQL arm.

use std::time::Duration;

use darkhold_db::finishers::{
    author_cadence, author_history, has_revisions, pseud_exists, work_cadence, work_days,
};
use darkhold_db::{Backend, Database};
use darkhold_domain::finishers::{summary, Cadence};

const T0: &str = "2026-03-01T00:00:00Z";

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
        "darkhold-finishers-{}-{}-{}",
        tag,
        uuid::Uuid::new_v4(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let url = match std::env::var("DARKHOLD_TEST_PG_URL") {
        Ok(admin) => {
            let name = format!(
                "lh_finishers_{}_{}",
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

async fn who(db: &Database, tag: &str) -> String {
    let account = uuid::Uuid::new_v4().to_string();
    let pseud = uuid::Uuid::new_v4().to_string();
    let handle = format!("{tag}-{}", &account[..8]);
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?, ?, ?, ?)")
                .bind(&account)
                .bind(format!("{tag}@example.invalid"))
                .bind(T0)
                .bind(T0)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("account");
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&pseud)
            .bind(&account)
            .bind(&handle)
            .bind(&handle)
            .bind(T0)
            .bind(T0)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("pseud");
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO accounts (id, email, created_at, updated_at)
                 VALUES ($1::uuid, $2, $3::timestamptz, $3::timestamptz)",
            )
            .bind(uuid::Uuid::parse_str(&account).expect("uuid"))
            .bind(format!("{tag}@example.invalid"))
            .bind(T0)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("account");
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES ($1::uuid, $2::uuid, $3, $3, $4::timestamptz, $4::timestamptz)",
            )
            .bind(uuid::Uuid::parse_str(&pseud).expect("uuid"))
            .bind(uuid::Uuid::parse_str(&account).expect("uuid"))
            .bind(&handle)
            .bind(T0)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("pseud");
        }
    }
    pseud
}

async fn work(db: &Database, pseud: &str, completion: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO works (id, owner_pseud_id, title, completion, created_at, updated_at,
                                    generated_content_posture)
                 VALUES (?, ?, 'A work', ?, ?, ?, 'forbid')",
            )
            .bind(&id)
            .bind(pseud)
            .bind(completion)
            .bind(T0)
            .bind(T0)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("work");
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO works (id, owner_pseud_id, title, completion, created_at, updated_at,
                                    generated_content_posture)
                 VALUES ($1::uuid, $2::uuid, 'A work', $3, $4::timestamptz, $4::timestamptz, 'forbid')",
            )
            .bind(uuid::Uuid::parse_str(&id).expect("uuid"))
            .bind(uuid::Uuid::parse_str(pseud).expect("uuid"))
            .bind(completion)
            .bind(T0)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("work");
        }
    }
    id
}

/// A unit of `work`, plus one revision per date given.
async fn revisions_on(db: &Database, work_id: &str, pseud: &str, dates: &[&str]) {
    let unit = uuid::Uuid::new_v4().to_string();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO units (id, work_id, order_key, title, created_at, updated_at)
                 VALUES (?, ?, 10, 'one', ?, ?)",
            )
            .bind(&unit)
            .bind(work_id)
            .bind(T0)
            .bind(T0)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("unit");
            for (n, date) in dates.iter().enumerate() {
                sqlx::query(
                    "INSERT INTO revisions (id, unit_id, revision_number, document_json,
                        sanitized_html, plain_text, word_count, note, created_by_pseud_id,
                        restored_from_id, created_at)
                     VALUES (?, ?, ?, '{}', '', 'text', 10, NULL, ?, NULL, ?)",
                )
                .bind(uuid::Uuid::new_v4().to_string())
                .bind(&unit)
                .bind(n as i64 + 1)
                .bind(pseud)
                .bind(*date)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("revision");
            }
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO units (id, work_id, order_key, title, created_at, updated_at)
                 VALUES ($1::uuid, $2::uuid, 10, 'one', $3::timestamptz, $3::timestamptz)",
            )
            .bind(uuid::Uuid::parse_str(&unit).expect("uuid"))
            .bind(uuid::Uuid::parse_str(work_id).expect("uuid"))
            .bind(T0)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("unit");
            for (n, date) in dates.iter().enumerate() {
                sqlx::query(
                    "INSERT INTO revisions (id, unit_id, revision_number, document_json,
                        sanitized_html, plain_text, word_count, note, created_by_pseud_id,
                        restored_from_id, created_at)
                     VALUES ($1::uuid, $2::uuid, $3, '{}', '', 'text', 10, NULL, $4::uuid, NULL, $5::timestamptz)",
                )
                .bind(uuid::Uuid::new_v4())
                .bind(uuid::Uuid::parse_str(&unit).expect("uuid"))
                .bind(n as i64 + 1)
                .bind(uuid::Uuid::parse_str(pseud).expect("uuid"))
                .bind(*date)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("revision");
            }
        }
    }
}

#[tokio::test]
async fn only_complete_is_finished_and_only_conclusions_are_counted() {
    let db = connect("history").await;
    let author = who(&db, "author").await;
    work(&db, &author, "complete").await;
    work(&db, &author, "complete").await;
    work(&db, &author, "abandoned").await;
    work(&db, &author, "in_progress").await;
    work(&db, &author, "hiatus").await;

    let h = author_history(&db, &author).await.expect("history");
    assert_eq!(h.finished, 2, "in_progress and hiatus are not finished");
    assert_eq!(h.concluded, 3, "in_progress and hiatus are not concluded either");
    assert_eq!(summary(&h), "finished 2 of 3 concluded works");
}

#[tokio::test]
async fn an_author_with_no_works_gets_zeroes_not_an_error() {
    let db = connect("empty").await;
    let author = who(&db, "empty").await;
    let h = author_history(&db, &author).await.expect("history");
    assert_eq!((h.finished, h.concluded), (0, 0));
    assert_eq!(summary(&h), "no concluded works yet");
}

#[tokio::test]
async fn an_unknown_pseud_is_distinguishable_from_an_author_with_nothing() {
    let db = connect("unknown").await;
    let author = who(&db, "known").await;
    assert!(pseud_exists(&db, &author).await.expect("exists"));
    assert!(!pseud_exists(&db, &uuid::Uuid::new_v4().to_string())
        .await
        .expect("exists"));
}

#[tokio::test]
async fn work_days_are_distinct_and_ascending() {
    let db = connect("days").await;
    let author = who(&db, "author").await;
    let work_id = work(&db, &author, "in_progress").await;
    revisions_on(
        &db,
        &work_id,
        &author,
        &[
            "2026-02-01T10:00:00Z",
            "2026-01-01T10:00:00Z",
            "2026-02-01T18:00:00Z",
            "2026-01-15T10:00:00Z",
        ],
    )
    .await;

    let days = work_days(&db, &work_id).await.expect("days");
    assert_eq!(days.len(), 3, "same-day revisions are one day: {days:?}");
    let mut sorted = days.clone();
    sorted.sort_unstable();
    assert_eq!(days, sorted, "days must come back ascending");
}

#[tokio::test]
async fn one_day_of_revisions_is_unmeasured_and_says_so() {
    let db = connect("oneday").await;
    let author = who(&db, "author").await;
    let work_id = work(&db, &author, "in_progress").await;
    // The import shape: every revision written on the day the import ran.
    revisions_on(&db, &work_id, &author, &["2026-10-27T15:05:58Z", "2026-10-27T15:06:01Z"]).await;

    match work_cadence(&db, &work_id).await.expect("cadence") {
        Cadence::Unmeasured { reason } => {
            assert!(reason.contains("one day"), "{reason:?}");
            assert!(
                darkhold_domain::finishers::is_positive(&reason),
                "even a refusal is phrased positively: {reason:?}"
            );
        }
        other => panic!("one day is not a cadence: {other:?}"),
    }
}

#[tokio::test]
async fn a_work_with_two_days_has_that_gap_as_its_cadence() {
    let db = connect("twoday").await;
    let author = who(&db, "author").await;
    let work_id = work(&db, &author, "in_progress").await;
    revisions_on(&db, &work_id, &author, &["2026-01-01T10:00:00Z", "2026-01-15T10:00:00Z"]).await;

    assert_eq!(
        work_cadence(&db, &work_id).await.expect("cadence"),
        Cadence::Measurable {
            median_days: 14,
            samples: 1
        }
    );
}

#[tokio::test]
async fn an_author_cadence_pools_their_works_and_is_unmeasured_without_any() {
    let db = connect("authorcadence").await;
    let author = who(&db, "author").await;

    // No works at all.
    match author_cadence(&db, &author).await.expect("cadence") {
        Cadence::Unmeasured { reason } => assert!(reason.contains("no works"), "{reason:?}"),
        other => panic!("an author with no works has no cadence: {other:?}"),
    }

    // A work whose revisions span three days: gaps 10 and 4 -> that work's median is 7.
    let first = work(&db, &author, "complete").await;
    revisions_on(
        &db,
        &first,
        &author,
        &["2026-01-01T10:00:00Z", "2026-01-11T10:00:00Z", "2026-01-15T10:00:00Z"],
    )
    .await;
    // A second work on one day adds nothing measurable.
    let second = work(&db, &author, "in_progress").await;
    revisions_on(&db, &second, &author, &["2026-02-01T10:00:00Z"]).await;

    // One work has a cadence, the other does not: the author's sample is 1, not 2, and the answer
    // is that work's median rather than a median over pooled days.
    assert_eq!(
        author_cadence(&db, &author).await.expect("cadence"),
        Cadence::Measurable {
            median_days: 7,
            samples: 1
        }
    );
}

#[tokio::test]
async fn a_schema_without_revisions_answers_unmeasured_not_an_error() {
    let db = connect("norevisions").await;
    let author = who(&db, "author").await;
    let work_id = work(&db, &author, "in_progress").await;
    assert!(has_revisions(&db).await.expect("catalog"));

    let drop = "DROP TABLE revisions";
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(drop)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("drop");
        }
        Backend::Postgres => {
            sqlx::query(&format!("{drop} CASCADE"))
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("drop");
        }
    }

    assert!(!has_revisions(&db).await.expect("catalog"));
    match work_cadence(&db, &work_id).await.expect("no database error") {
        Cadence::Unmeasured { reason } => assert!(reason.contains("revisions"), "{reason:?}"),
        other => panic!("a schema without revisions cannot measure: {other:?}"),
    }
    assert!(!author_cadence(&db, &author).await.expect("cadence").is_measurable());
}
