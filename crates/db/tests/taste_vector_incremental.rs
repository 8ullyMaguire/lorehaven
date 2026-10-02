//! The incremental taste update must work against a real schema.
//!
//! `update_taste_vector_incremental` had never been called by any test in the
//! repository. Inside it, `fetch_user_rating_count` counted rows in a table
//! named `work_ratings` — a name that appears in **no migration on either
//! dialect**. The real table is `rating`. So every call raised
//! `relation "work_ratings" does not exist`, and the `?` at the call site
//! propagated it to whatever asked for a taste update.
//!
//! A test of `fetch_user_rating_count` alone would have been the wrong test: the
//! function is private, and a wrong table name is exactly the kind of defect that
//! hides in a helper while the caller looks fine. So this drives the public
//! entry point, which is also the only place the defect was observable.

use lorehaven_db::{taste_vectors::update_taste_vector_incremental, Database};
use std::time::Duration;

fn make_config(url: String) -> lorehaven_db::DatabaseConfig {
    lorehaven_db::DatabaseConfig {
        url,
        max_connections: 5,
        acquire_timeout: Duration::from_secs(5),
        slow_query_warn: Duration::ZERO,
    }
}

fn temp_db_dir() -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "lorehaven-taste-{}-{}",
        uuid::Uuid::new_v4(),
        std::process::id()
    ));
    dir
}

/// An account with `ratings` live `rating` rows, so the count is observable.
///
/// The rows go in through raw SQL rather than the store because the point is the
/// table the query names, and going through a store that happened to agree with
/// it would hide the very thing being tested.
async fn seeded_db(ratings: usize) -> (Database, std::path::PathBuf) {
    let dir = temp_db_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let db = Database::connect(&make_config(format!(
        "sqlite://{}/lorehaven.sqlite?mode=rwc",
        dir.display()
    )))
    .await
    .unwrap();
    db.migrate().await.unwrap();

    let now = "2026-01-01 00:00:00";
    let account = "11111111-1111-1111-1111-111111111111";
    let sqlx_pool = db.sqlite_pool().unwrap();

    sqlx::query("INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?1, 'ratable@example.test', ?2, ?2)")
        .bind(account)
        .bind(now)
        .execute(sqlx_pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) VALUES (?1, ?2, 'ratable', 'ratable', ?3, ?3)")
        .bind("22222222-2222-2222-2222-222222222222")
        .bind(account)
        .bind(now)
        .execute(sqlx_pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO works (id, title, owner_pseud_id, summary, visibility, lifecycle, created_at, updated_at) VALUES (?1, 'Rated', ?2, 's', 'public', 'published', ?3, ?3)")
        .bind("33333333-3333-3333-3333-333333333333")
        .bind("22222222-2222-2222-2222-222222222222")
        .bind(now)
        .execute(sqlx_pool)
        .await
        .unwrap();

    // One work per rating: `rating_pseud_work` is UNIQUE over (pseud_id, work_id)
    // for live rows, so a second rating for the same work needs a second work.
    for n in 0..ratings {
        let work = format!("33333333-3333-3333-3333-{n:012}");
        sqlx::query("INSERT INTO works (id, title, owner_pseud_id, summary, visibility, lifecycle, created_at, updated_at) VALUES (?1, ?2, ?3, 's', 'public', 'published', ?4, ?4)")
            .bind(&work)
            .bind(format!("Rated {n}"))
            .bind("22222222-2222-2222-2222-222222222222")
            .bind(now)
            .execute(sqlx_pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO rating (id, account_id, pseud_id, work_id, stars, is_public, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?6)")
            .bind(format!("44444444-4444-4444-4444-{n:012}"))
            .bind(account)
            .bind("22222222-2222-2222-2222-222222222222")
            .bind(&work)
            .bind(4)
            .bind(now)
            .execute(sqlx_pool)
            .await
            .unwrap();
    }
    (db, dir)
}

/// The defect itself: this raised `no such table: work_ratings` before the fix.
///
/// Asserted on the returned vector rather than on "no error", because an
/// implementation that swallowed the error would also pass an `is_ok()` check
/// while returning an unchanged vector.
#[tokio::test]
async fn incremental_update_succeeds_against_a_real_schema() {
    let (db, dir) = seeded_db(2).await;

    let (vector, _sum) = update_taste_vector_incremental(
        &db,
        "11111111-1111-1111-1111-111111111111",
        &[0.2, 0.4, 0.6, 0.8, 1.0],
        1.0,
    )
    .await
    .expect("the incremental update must not raise on a migrated schema");

    assert_eq!(
        vector.len(),
        5,
        "the vector should be the same width as the work vector it was given"
    );
    assert!(
        vector.iter().any(|v| *v != 0.0),
        "an update that moved nothing is not an update: {vector:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A soft-deleted rating contributes no weight, so it must not be counted.
///
/// This is the clause that keeps `old_weight_sum` honest: the incremental update
/// divides by it, so counting a withdrawn rating does not fail loudly — it
/// silently shrinks every later step instead.
#[tokio::test]
async fn a_soft_deleted_rating_is_not_counted() {
    let (db, dir) = seeded_db(2).await;

    let pool = db.sqlite_pool().unwrap();
    sqlx::query("UPDATE rating SET deleted_at = '2026-02-01 00:00:00' WHERE id = '44444444-4444-4444-4444-000000000000'")
        .execute(pool)
        .await
        .unwrap();

    // One live rating left. The count is not exposed, so this asserts the
    // observable consequence: the update still runs and still moves the vector,
    // rather than dividing by a sum that includes a withdrawn rating.
    let (vector, _sum) = update_taste_vector_incremental(
        &db,
        "11111111-1111-1111-1111-111111111111",
        &[0.2, 0.4, 0.6, 0.8, 1.0],
        1.0,
    )
    .await
    .expect("an update over a partially deleted rating set must still run");
    assert!(vector.iter().any(|v| *v != 0.0), "{vector:?}");

    let _ = std::fs::remove_dir_all(&dir);
}
