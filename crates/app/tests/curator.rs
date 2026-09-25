//! Media curator role & bounty system (spec §32.7.5).

use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::media_resilience;
use lorehaven_domain::media_resilience::VerificationType;
use std::path::PathBuf;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-cur-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[tokio::test]
async fn curator_role_management() {
    let dir = scratch_dir("role");
    let tdb = test_support::TestDb::connect_with_dir("cur-role", &dir).await;
    let db = tdb.db();

    let account_id = "user-curator-001";

    // Not a curator initially
    let is_curator = media_resilience::is_active_curator(db, account_id)
        .await
        .expect("check curator");
    assert!(!is_curator);

    // Opt in
    media_resilience::opt_in_curator(db, account_id)
        .await
        .expect("opt in");
    let is_curator = media_resilience::is_active_curator(db, account_id)
        .await
        .expect("check curator");
    assert!(is_curator);

    // Opt out
    media_resilience::opt_out_curator(db, account_id)
        .await
        .expect("opt out");
    let is_curator = media_resilience::is_active_curator(db, account_id)
        .await
        .expect("check curator");
    assert!(!is_curator);
}

#[tokio::test]
async fn curator_list_active() {
    let dir = scratch_dir("list");
    let tdb = test_support::TestDb::connect_with_dir("cur-list", &dir).await;
    let db = tdb.db();

    media_resilience::opt_in_curator(db, "user-001")
        .await
        .expect("opt in 1");
    media_resilience::opt_in_curator(db, "user-002")
        .await
        .expect("opt in 2");

    let curators = media_resilience::list_active_curators(db)
        .await
        .expect("list curators");
    assert_eq!(curators.len(), 2);
    assert!(curators.contains(&"user-001".to_string()));
    assert!(curators.contains(&"user-002".to_string()));

    // Opt out one
    media_resilience::opt_out_curator(db, "user-001")
        .await
        .expect("opt out");
    let curators = media_resilience::list_active_curators(db)
        .await
        .expect("list curators");
    assert_eq!(curators.len(), 1);
    assert!(!curators.contains(&"user-001".to_string()));
}

#[tokio::test]
async fn link_verification_quorum() {
    let dir = scratch_dir("quorum");
    let tdb = test_support::TestDb::connect_with_dir("cur-quorum", &dir).await;
    let db = tdb.db();

    let link_id = "link-001";
    let media_ref_id = "ref-001";

    // No verifications yet
    let count = media_resilience::count_link_verifiers(db, link_id)
        .await
        .expect("count verifiers");
    assert_eq!(count, 0);
    let quorum = media_resilience::has_quorum(db, link_id)
        .await
        .expect("quorum check");
    assert!(!quorum);

    // First verification
    let id1 = "verify-001";
    media_resilience::record_link_verification(
        db,
        id1,
        link_id,
        media_ref_id,
        "curator-a",
        VerificationType::ExactMatch,
        1.0,
    )
    .await
    .expect("record verification");

    let count = media_resilience::count_link_verifiers(db, link_id)
        .await
        .expect("count verifiers");
    assert_eq!(count, 1);
    let quorum = media_resilience::has_quorum(db, link_id)
        .await
        .expect("quorum check");
    assert!(!quorum);

    // Second verification — quorum reached
    let id2 = "verify-002";
    media_resilience::record_link_verification(
        db,
        id2,
        link_id,
        media_ref_id,
        "curator-b",
        VerificationType::PerceptualMatch,
        0.85,
    )
    .await
    .expect("record verification");

    let count = media_resilience::count_link_verifiers(db, link_id)
        .await
        .expect("count verifiers");
    assert_eq!(count, 2);
    let quorum = media_resilience::has_quorum(db, link_id)
        .await
        .expect("quorum check");
    assert!(quorum);
}

#[tokio::test]
async fn curator_cannot_double_verify() {
    let dir = scratch_dir("double");
    let tdb = test_support::TestDb::connect_with_dir("cur-double", &dir).await;
    let db = tdb.db();

    let link_id = "link-002";
    let media_ref_id = "ref-002";
    let curator_id = "curator-x";

    let id1 = "verify-010";
    media_resilience::record_link_verification(
        db,
        id1,
        link_id,
        media_ref_id,
        curator_id,
        VerificationType::ExactMatch,
        1.0,
    )
    .await
    .expect("first verification");

    let already = media_resilience::curator_verified_link(db, curator_id, link_id)
        .await
        .expect("check verified");
    assert!(already);
}

#[tokio::test]
async fn standing_bounty_matching() {
    let dir = scratch_dir("bounty");
    let tdb = test_support::TestDb::connect_with_dir("cur-bounty", &dir).await;
    let db = tdb.db();

    // Insert a standing bounty via the repository
    let pool = db.sqlite_pool().expect("sqlite");
    sqlx::query(
        "INSERT INTO curator_standing_bounties
            (id, name, provider, healthy_links_below, has_archive_link, reward, enabled)
         VALUES (?, 'Mirror all Tumblr-hosted images', 'tumblr', 3, 0, 30, 1)",
    )
    .bind("bounty-001")
    .execute(pool)
    .await
    .expect("insert bounty");

    // Reference with 1 healthy link should match (below threshold of 3)
    let media_ref_id = "ref-005";
    media_resilience::insert_media_reference(
        db,
        media_ref_id,
        "hash-bounty",
        lorehaven_domain::media_resilience::MediaKind::Image,
    )
    .await
    .expect("insert ref");

    media_resilience::insert_availability_link(
        db,
        "link-bounty",
        media_ref_id,
        "https://example.com/b.png",
        lorehaven_domain::media_resilience::LinkProvider::Tumblr,
        None,
        50,
    )
    .await
    .expect("insert link");

    let bounties = media_resilience::find_matching_standing_bounties(db, media_ref_id, 1, 0, false)
        .await
        .expect("find bounties");
    assert_eq!(bounties.len(), 1);
    assert_eq!(bounties[0].bounty_id, "bounty-001");
    assert_eq!(bounties[0].reward, 30);
}
