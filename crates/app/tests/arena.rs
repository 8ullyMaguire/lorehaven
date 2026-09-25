//! Taste Calibration Arena integration tests (spec §0.4.2a).
//!
//! Covers the DB layer: pool query, ballot recording, weight updates, and
//! the migration that creates the arena tables on both backends.

use lorehaven_db::taste_vectors::{
    get_arena_pool, get_arena_weights, record_arena_ballot, update_arena_weights,
};
use test_support::id;

/// The five works the arena fixtures draw from. Named once so the seed and the
/// tests that look them up cannot drift apart.
const WORK_IDS: [&str; 5] = ["work-a1", "work-a2", "work-a3", "work-a4", "work-a5"];

async fn seed_account_and_works(db: &lorehaven_db::Database) {
    let account_id = id("acc-arena");
    let pseud_id = id("acc-arena-pseud");
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query("INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?, ?, datetime('now'), datetime('now'))")
                .bind(&account_id).bind("arena@test.dev")
                .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
            sqlx::query("INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) VALUES (?, ?, ?, ?, datetime('now'), datetime('now'))")
                .bind(&pseud_id).bind(&account_id).bind(&pseud_id).bind(&pseud_id)
                .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
            for (i, slug) in WORK_IDS.iter().enumerate() {
                let wid = id(slug);
                sqlx::query("INSERT INTO works (id, title, owner_pseud_id, lifecycle, visibility, created_at, updated_at) VALUES (?, ?, ?, 'published', 'public', datetime('now'), datetime('now'))")
                    .bind(&wid).bind(format!("Arena Fic {}", i + 1)).bind(&pseud_id)
                    .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
                seed_chapter(db, &wid, &pseud_id, i).await;
            }
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query("INSERT INTO accounts (id, email, created_at, updated_at) VALUES ($1::uuid, $2, now(), now())")
                .bind(&account_id).bind("arena@test.dev")
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
            sqlx::query("INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) VALUES ($1::uuid, $2::uuid, $3, $4, now(), now())")
                .bind(&pseud_id).bind(&account_id).bind(&pseud_id).bind(&pseud_id)
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
            for (i, slug) in WORK_IDS.iter().enumerate() {
                let wid = id(slug);
                sqlx::query("INSERT INTO works (id, title, owner_pseud_id, lifecycle, visibility, created_at, updated_at) VALUES ($1::uuid, $2, $3::uuid, 'published', 'public', now(), now())")
                    .bind(&wid).bind(format!("Arena Fic {}", i + 1)).bind(&pseud_id)
                    .execute(db.postgres_pool().expect("postgres")).await.unwrap();
                seed_chapter(db, &wid, &pseud_id, i).await;
            }
        }
    }
}

/// Seed one chapter + current revision per work so word_count is populated
/// (word_count lives on chapter_revisions, not works).
async fn seed_chapter(db: &lorehaven_db::Database, work_id: &str, pseud_id: &str, i: usize) {
    let chapter_id = id(&format!("{work_id}-ch1"));
    let revision_id = id(&format!("{work_id}-rev1"));
    let words = 5000 + i * 100;
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query("INSERT INTO chapters (id, work_id, order_key, title, created_at, updated_at) VALUES (?, ?, 10, 'Chapter 1', datetime('now'), datetime('now'))")
                .bind(&chapter_id).bind(work_id)
                .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
            sqlx::query("INSERT INTO chapter_revisions (id, chapter_id, revision_number, document_json, sanitized_html, plain_text, word_count, created_by_pseud_id, created_at) VALUES (?, ?, 1, '{}', '', '', ?, ?, datetime('now'))")
                .bind(&revision_id).bind(&chapter_id).bind(words as i64).bind(pseud_id)
                .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
            sqlx::query("UPDATE chapters SET current_revision_id = ? WHERE id = ?")
                .bind(&revision_id)
                .bind(&chapter_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .unwrap();
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query("INSERT INTO chapters (id, work_id, order_key, title, created_at, updated_at) VALUES ($1::uuid, $2::uuid, 10, 'Chapter 1', now(), now())")
                .bind(&chapter_id).bind(work_id)
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
            sqlx::query("INSERT INTO chapter_revisions (id, chapter_id, revision_number, document_json, sanitized_html, plain_text, word_count, created_by_pseud_id, created_at) VALUES ($1::uuid, $2::uuid, 1, '{}', '', '', $3, $4::uuid, now())")
                .bind(&revision_id).bind(&chapter_id).bind(words as i64).bind(pseud_id)
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
            sqlx::query("UPDATE chapters SET current_revision_id = $1::uuid WHERE id = $2::uuid")
                .bind(&revision_id)
                .bind(&chapter_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .unwrap();
        }
    }
}

#[tokio::test]
async fn arena_pool_excludes_voted_works() {
    let dir = test_support::scratch_dir("arena_pool");
    let tdb = test_support::TestDb::connect_with_dir("arena-pool", &dir).await;
    let db = tdb.db();
    seed_account_and_works(db).await;

    // Record one ballot; the two voted works must drop out of the pool.
    let (acct, w1, w2) = (id("acc-arena"), id("work-a1"), id("work-a2"));
    record_arena_ballot(db, &acct, &w1, &w2, &[]).await.unwrap();

    let pool = get_arena_pool(db, &id("acc-arena"), 10).await.unwrap();
    assert!(pool.iter().all(|w| w.0 != w1 && w.0 != w2));
    assert_eq!(pool.len(), 3);
}

#[tokio::test]
async fn arena_weights_roundtrip() {
    let dir = test_support::scratch_dir("arena_wts");
    let tdb = test_support::TestDb::connect_with_dir("arena-wts", &dir).await;
    let db = tdb.db();
    seed_account_and_works(db).await;

    // No weights yet.
    assert!(get_arena_weights(db, &id("acc-arena"))
        .await
        .unwrap()
        .is_empty());

    // Upsert a weight, then read it back.
    update_arena_weights(db, &id("acc-arena"), "prose", 0.35, 1200.0, 5)
        .await
        .unwrap();
    let weights = get_arena_weights(db, &id("acc-arena")).await.unwrap();
    assert_eq!(weights.len(), 1);
    assert_eq!(weights[0].0, "prose");
    assert!((weights[0].1 - 0.35).abs() < 1e-9);
    assert!((weights[0].2 - 1200.0).abs() < 1e-9);
    assert_eq!(weights[0].3, 5);

    // Upsert again — one row per (account, dimension).
    update_arena_weights(db, &id("acc-arena"), "prose", 0.42, 1250.0, 6)
        .await
        .unwrap();
    let weights = get_arena_weights(db, &id("acc-arena")).await.unwrap();
    assert_eq!(weights.len(), 1);
    assert!((weights[0].1 - 0.42).abs() < 1e-9);
}

#[tokio::test]
async fn arena_ballot_updates_elo() {
    let dir = test_support::scratch_dir("arena_elo");
    let tdb = test_support::TestDb::connect_with_dir("arena-elo", &dir).await;
    let db = tdb.db();
    seed_account_and_works(db).await;

    // A ballot with a reason tag should update the tagged dimension's Elo.
    // This mirrors the route flow: record ballot -> apply Elo -> persist weights.
    record_arena_ballot(
        db,
        &id("acc-arena"),
        &id("work-a1"),
        &id("work-a2"),
        &["prose".to_string()],
    )
    .await
    .unwrap();

    let elos = vec![lorehaven_domain::taste_vector::DimensionElo {
        dimension_key: "prose".to_string(),
        elo_rating: 1000.0,
        matches_played: 0,
    }];
    let ballot = lorehaven_domain::taste_vector::ArenaBallot {
        best_work_id: id("work-a1"),
        worst_work_id: id("work-a2"),
        reason_tags: vec!["prose".to_string()],
    };
    let round = lorehaven_domain::taste_vector::ArenaRound {
        cards: vec![
            lorehaven_domain::taste_vector::ArenaCard {
                work_id: id("work-a1"),
                title: "Arena Fic 1".to_string(),
                fandom: "Test Fandom".to_string(),
                tags: vec![],
                word_count: 5000,
                excerpt: String::new(),
                target_dimension: "prose".to_string(),
            },
            lorehaven_domain::taste_vector::ArenaCard {
                work_id: id("work-a2"),
                title: "Arena Fic 2".to_string(),
                fandom: "Test Fandom".to_string(),
                tags: vec![],
                word_count: 5100,
                excerpt: String::new(),
                target_dimension: "prose".to_string(),
            },
        ],
    };
    let updated = lorehaven_domain::taste_vector::apply_arena_ballot(&elos, &round, &ballot);
    assert_eq!(updated.len(), 1);
    assert_ne!(
        updated[0].elo_rating, 1000.0,
        "Elo should move off the default after a ballot"
    );
    assert_eq!(updated[0].matches_played, 1);

    for elo in &updated {
        lorehaven_db::taste_vectors::update_arena_weights(
            db,
            &id("acc-arena"),
            &elo.dimension_key,
            0.35,
            elo.elo_rating,
            elo.matches_played as i64,
        )
        .await
        .unwrap();
    }

    let weights = get_arena_weights(db, &id("acc-arena")).await.unwrap();
    let prose = weights.iter().find(|w| w.0 == "prose");
    assert!(
        prose.is_some(),
        "reason-tagged dimension should gain a weight row"
    );
    let (_, _, elo, matches) = prose.unwrap();
    assert_ne!(
        *elo, 1000.0,
        "Elo should move off the 1000.0 default after a ballot"
    );
    assert_eq!(*matches, 1);
}
