//! Phase 5 (§32.7.3 & §32.7.5): reverse search & curator bounty queue.

use lorehaven_db::media_resilience;
use lorehaven_domain::media_resilience::MediaKind;

#[tokio::test]
async fn reverse_search_by_perceptual_hash() {
    let dir = test_support::scratch_dir("ma_rs");
    let tdb = test_support::TestDb::connect_with_dir("ma-rs", &dir).await;
    let db = tdb.db();

    // Insert two references, then set perceptual hash on both (simulates curator setting it)
    media_resilience::insert_media_reference(db, "ref-rs-1", "sha256:abc", MediaKind::Image)
        .await.expect("insert ref 1");
    media_resilience::insert_media_reference(db, "ref-rs-2", "sha256:def", MediaKind::Image)
        .await.expect("insert ref 2");

    // Update perceptual_hash directly (in production this is done by curator/background job)
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query("UPDATE media_references SET perceptual_hash = ? WHERE id = ?")
                .bind("hash123").bind("ref-rs-1")
                .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
            sqlx::query("UPDATE media_references SET perceptual_hash = ? WHERE id = ?")
                .bind("hash123").bind("ref-rs-2")
                .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query("UPDATE media_references SET perceptual_hash = $1 WHERE id = $2")
                .bind("hash123").bind("ref-rs-1")
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
            sqlx::query("UPDATE media_references SET perceptual_hash = $1 WHERE id = $2")
                .bind("hash123").bind("ref-rs-2")
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
        }
    }

    let found = media_resilience::find_by_perceptual_hash(db, "hash123", 0)
        .await.expect("reverse search");
    assert_eq!(found.len(), 2);
    assert!(found.iter().any(|r| r.id == "ref-rs-1"));
    assert!(found.iter().any(|r| r.id == "ref-rs-2"));
}

#[tokio::test]
async fn curator_bounty_queue_finds_low_health_references() {
    let dir = test_support::scratch_dir("ma_cbq");
    let tdb = test_support::TestDb::connect_with_dir("ma-cbq", &dir).await;
    let db = tdb.db();

    // Insert reference with no links (should appear in queue with threshold 3)
    media_resilience::insert_media_reference(db, "ref-cbq-1", "sha256:c1", MediaKind::Image)
        .await.expect("insert ref");

    let queue = media_resilience::find_curator_bounty_queue(db, 3, 50)
        .await.expect("bounty queue");
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].id, "ref-cbq-1");
}
