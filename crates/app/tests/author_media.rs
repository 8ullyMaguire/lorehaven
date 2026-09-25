//! Author media tools (spec §32.7.8): preferences & targeted bounties.

use lorehaven_db::media_resilience;
use lorehaven_db::Backend;
use std::path::PathBuf;
use test_support::id;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-am-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[tokio::test]
async fn author_preferences_crud() {
    let dir = scratch_dir("prefs");
    let tdb = test_support::TestDb::connect_with_dir("am-prefs", &dir).await;
    let db = tdb.db();

    let account_id = id("author-001");

    // Upsert preferences
    media_resilience::upsert_author_preferences(db, &account_id, true, true, "immediate", true, 5)
        .await
        .expect("upsert prefs");

    let prefs = media_resilience::get_author_preferences(db, &account_id)
        .await
        .expect("get prefs");
    assert!(prefs.auto_submit_to_archive);
    assert!(prefs.prefer_curator_verified);
    assert_eq!(prefs.broken_link_notifications, "immediate");
    assert!(prefs.allow_curator_edits);
    assert_eq!(prefs.minimum_healthy_links, 5);

    // Update
    media_resilience::upsert_author_preferences(db, &account_id, false, false, "weekly", false, 2)
        .await
        .expect("update prefs");

    let prefs = media_resilience::get_author_preferences(db, &account_id)
        .await
        .expect("get prefs");
    assert!(!prefs.auto_submit_to_archive);
    assert_eq!(prefs.minimum_healthy_links, 2);
}

#[tokio::test]
async fn targeted_bounty_post_and_claim() {
    let dir = scratch_dir("tb");
    let tdb = test_support::TestDb::connect_with_dir("am-tb", &dir).await;
    let db = tdb.db();

    let bounty_id = "tb-001";
    let work_id = "work-001";
    let author_id = "author-001";

    // Post a targeted bounty
    media_resilience::post_targeted_bounty(
        db,
        bounty_id,
        work_id,
        None,
        None,
        author_id,
        50,
        Some("Find a working mirror for the chapter 7 moodboard"),
    )
    .await
    .expect("post bounty");

    let bounties = media_resilience::list_targeted_bounties_for_work(db, work_id)
        .await
        .expect("list bounties");
    assert_eq!(bounties.len(), 1);
    assert_eq!(bounties[0].reward, 50);
    assert_eq!(bounties[0].status, "open");

    // Claim it
    let claimed = media_resilience::claim_targeted_bounty(db, bounty_id, "curator-y")
        .await
        .expect("claim bounty");
    assert!(claimed);

    // Cannot claim again
    let claimed_again = media_resilience::claim_targeted_bounty(db, bounty_id, "curator-z")
        .await
        .expect("claim bounty");
    assert!(!claimed_again);

    let bounties = media_resilience::list_targeted_bounties_for_work(db, work_id)
        .await
        .expect("list bounties");
    assert_eq!(bounties[0].status, "claimed");
}

#[tokio::test]
async fn targeted_bounty_with_chapter() {
    let dir = scratch_dir("tb-ch");
    let tdb = test_support::TestDb::connect_with_dir("am-tb-ch", &dir).await;
    let db = tdb.db();

    let bounty_id = "tb-002";
    let work_id = "work-002";
    let chapter_id = "ch-007";

    media_resilience::post_targeted_bounty(
        db,
        bounty_id,
        work_id,
        Some(chapter_id),
        None,
        "author-002",
        75,
        Some("Mirror needed for chapter 7"),
    )
    .await
    .expect("post bounty with chapter");

    let bounties = media_resilience::list_targeted_bounties_for_work(db, work_id)
        .await
        .expect("list bounties");
    assert_eq!(bounties.len(), 1);
    assert_eq!(bounties[0].chapter_id, Some(chapter_id.to_string()));
    assert_eq!(bounties[0].reward, 75);
}

#[tokio::test]
async fn author_media_health_report_tiers() {
    let dir = scratch_dir("health");
    let tdb = test_support::TestDb::connect_with_dir("am-health", &dir).await;
    let db = tdb.db();

    let account_id = id("author-003");
    let pseud_id = id(&format!("{}-pseud", account_id));

    // Seed account → pseud → works in FK order.
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?, ?, datetime('now'), datetime('now'))")
                .bind(&account_id)
                .bind(format!("{}@test.dev", account_id))
                .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
            sqlx::query("INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) VALUES (?, ?, ?, ?, datetime('now'), datetime('now'))")
                .bind(&pseud_id).bind(&account_id).bind(&pseud_id).bind(&pseud_id)
                .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
            for (wid, title) in [("work-a", "Work A"), ("work-b", "Work B")] {
                sqlx::query("INSERT INTO works (id, title, owner_pseud_id, lifecycle, visibility, created_at, updated_at) VALUES (?, ?, ?, 'published', 'public', datetime('now'), datetime('now'))")
                    .bind(id(wid)).bind(title).bind(&pseud_id)
                    .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
            }
        }
        Backend::Postgres => {
            sqlx::query("INSERT INTO accounts (id, email, created_at, updated_at) VALUES ($1::uuid, $2, now(), now())")
                .bind(&account_id)
                .bind(format!("{}@test.dev", account_id))
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
            sqlx::query("INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) VALUES ($1::uuid, $2::uuid, $3, $4, now(), now())")
                .bind(&pseud_id).bind(&account_id).bind(&pseud_id).bind(&pseud_id)
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
            for (wid, title) in [("work-a", "Work A"), ("work-b", "Work B")] {
                sqlx::query("INSERT INTO works (id, title, owner_pseud_id, lifecycle, visibility, created_at, updated_at) VALUES ($1::uuid, $2, $3::uuid, 'published', 'public', now(), now())")
                    .bind(id(wid)).bind(title).bind(&pseud_id)
                    .execute(db.postgres_pool().expect("postgres")).await.unwrap();
            }
        }
    }

    // Work A: one reference (new, so zero healthy links → broken tier).
    let (created_a, _) = media_resilience::upsert_media_reference_for_import(
        db,
        &id("work-a"),
        None,
        "https://example.com/a.png",
    )
    .await
    .expect("upsert ref a");
    assert!(created_a);

    // Work B: one reference (new, so zero healthy links → broken tier).
    let (created_b, _) = media_resilience::upsert_media_reference_for_import(
        db,
        &id("work-b"),
        None,
        "https://example.com/b.png",
    )
    .await
    .expect("upsert ref b");
    assert!(created_b);

    let report = media_resilience::author_media_health_report(db, &account_id)
        .await
        .expect("health report");

    // Both works appear with 1 total reference each.
    assert_eq!(report.len(), 2);
    let a = report
        .iter()
        .find(|r| r.work_title == "Work A")
        .expect("work a");
    let b = report
        .iter()
        .find(|r| r.work_title == "Work B")
        .expect("work b");
    assert_eq!(a.total_references, 1);
    assert_eq!(a.healthy_references, 0);
    assert_eq!(b.total_references, 1);
    assert_eq!(b.healthy_references, 0);
}
