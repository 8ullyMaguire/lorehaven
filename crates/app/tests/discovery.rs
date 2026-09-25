//! Phase 5 (§32.7.3 & §32.7.5): reverse search & curator bounty queue.

use lorehaven_db::media_resilience;
use lorehaven_domain::media_resilience::MediaKind;

#[tokio::test]
async fn reverse_search_by_perceptual_hash() {
    let dir = test_support::scratch_dir("ma_rs");
    let tdb = test_support::TestDb::connect_with_dir("ma-rs", &dir).await;
    let db = tdb.db();

    // Insert two references, then set perceptual hash on both (simulates curator setting it)
    media_resilience::insert_media_reference(
        db,
        &test_support::id("ref-rs-1"),
        "sha256:abc",
        MediaKind::Image,
    )
    .await
    .expect("insert ref 1");
    media_resilience::insert_media_reference(
        db,
        &test_support::id("ref-rs-2"),
        "sha256:def",
        MediaKind::Image,
    )
    .await
    .expect("insert ref 2");

    // Update perceptual_hash directly (in production this is done by curator/background job)
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query("UPDATE media_references SET perceptual_hash = ? WHERE id = ?")
                .bind("00ff00ff")
                .bind(test_support::id("ref-rs-1"))
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .unwrap();
            sqlx::query("UPDATE media_references SET perceptual_hash = ? WHERE id = ?")
                .bind("00ff00ff")
                .bind(test_support::id("ref-rs-2"))
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .unwrap();
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query("UPDATE media_references SET perceptual_hash = $1 WHERE id = $2::uuid")
                .bind("00ff00ff")
                .bind(test_support::id("ref-rs-1"))
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .unwrap();
            sqlx::query("UPDATE media_references SET perceptual_hash = $1 WHERE id = $2::uuid")
                .bind("00ff00ff")
                .bind(test_support::id("ref-rs-2"))
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .unwrap();
        }
    }

    // The stored value is a hex perceptual hash, not an arbitrary string: the
    // search compares fingerprints, so a non-hex value is not a hash to compare
    // and is refused rather than scored. The threshold is the spec's default 6,
    // and an identical hash is distance 0, so both references match.
    let found = media_resilience::find_by_perceptual_hash(db, "00ff00ff", 6)
        .await
        .expect("reverse search");
    assert_eq!(found.len(), 2);
    assert!(found.iter().any(|r| r.id == test_support::id("ref-rs-1")));
    assert!(found.iter().any(|r| r.id == test_support::id("ref-rs-2")));
}

#[tokio::test]
async fn curator_bounty_queue_finds_low_health_references() {
    let dir = test_support::scratch_dir("ma_cbq");
    let tdb = test_support::TestDb::connect_with_dir("ma-cbq", &dir).await;
    let db = tdb.db();

    // Insert reference with no links (should appear in queue with threshold 3)
    media_resilience::insert_media_reference(db, "ref-cbq-1", "sha256:c1", MediaKind::Image)
        .await
        .expect("insert ref");

    let queue = media_resilience::find_curator_bounty_queue(db, 3, 50)
        .await
        .expect("bounty queue");
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].id, "ref-cbq-1");
}

#[tokio::test]
async fn find_works_by_media_reference() {
    let dir = test_support::scratch_dir("find-wmr");
    let tdb = test_support::TestDb::connect_with_dir("find-wmr", &dir).await;
    let db = tdb.db();

    // Each of these is its own UUID. Suffixing one onto another produced
    // "<uuid>-pseud", which is not a UUID -- and it is a `&str` that SQLite
    // stores in a TEXT column without complaint, so only PostgreSQL objected.
    let account_id = test_support::id("acc-finder");
    let pseud_id = test_support::id("pseud-finder");

    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query("INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?, ?, datetime('now'), datetime('now'))")
                .bind(&account_id).bind(format!("{}@test.dev", account_id))
                .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
            sqlx::query("INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) VALUES (?, ?, ?, ?, datetime('now'), datetime('now'))")
                .bind(&pseud_id).bind(account_id).bind(&pseud_id).bind(&pseud_id)
                .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
            for wid in [test_support::id("work-x"), test_support::id("work-y")] {
                sqlx::query("INSERT INTO works (id, title, owner_pseud_id, lifecycle, visibility, created_at, updated_at) VALUES (?, ?, ?, 'published', 'public', datetime('now'), datetime('now'))")
                    .bind(&wid).bind(format!("Title {wid}")).bind(&pseud_id)
                    .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
            }
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query("INSERT INTO accounts (id, email, created_at, updated_at) VALUES ($1::uuid, $2, now(), now())")
                .bind(&account_id).bind(format!("{}@test.dev", account_id))
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
            sqlx::query("INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) VALUES ($1::uuid, $2::uuid, $3, $4, now(), now())")
                .bind(&pseud_id).bind(account_id).bind(&pseud_id).bind(&pseud_id)
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
            for wid in [test_support::id("work-x"), test_support::id("work-y")] {
                sqlx::query("INSERT INTO works (id, title, owner_pseud_id, lifecycle, visibility, created_at, updated_at) VALUES ($1::uuid, $2, $3::uuid, 'published', 'public', now(), now())")
                    .bind(&wid).bind(format!("Title {wid}")).bind(&pseud_id)
                    .execute(db.postgres_pool().expect("postgres")).await.unwrap();
            }
        }
    }

    let (created, reference_id) = media_resilience::upsert_media_reference_for_import(
        db,
        "work-x",
        None,
        "https://img.example.com/xyz.png",
    )
    .await
    .expect("upsert import");
    assert!(created);

    // Same URL for work-y — dedup returns false but still inserts the work association.
    let (created2, _) = media_resilience::upsert_media_reference_for_import(
        db,
        "work-y",
        None,
        "https://img.example.com/xyz.png",
    )
    .await
    .expect("upsert import 2");
    assert!(!created2);

    let result = media_resilience::find_works_by_media_reference(db, &reference_id)
        .await
        .expect("find works");

    assert_eq!(result.len(), 2);
    assert_eq!(result[0].0, "work-x");
    assert_eq!(result[1].0, "work-y");
}
