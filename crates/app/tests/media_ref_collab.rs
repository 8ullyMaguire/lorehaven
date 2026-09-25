//! Media-reference collaborative recommendations (spec §32.7.3, §9.10).
//!
//! Works sharing media references with the user's bookmarked works.

use lorehaven_db::media_resilience;
use lorehaven_db::Backend;
use lorehaven_domain::ids::WorkId;

async fn seed_work(db: &lorehaven_db::Database, work_id: &WorkId, title: &str, owner_pseud: &str) {
    let id = work_id.to_string();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO works (id, title, owner_pseud_id, lifecycle, visibility, created_at, updated_at)
                 VALUES (?, ?, ?, 'published', 'public', datetime('now'), datetime('now'))",
            )
            .bind(&id)
            .bind(title)
            .bind(owner_pseud)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("seed work");
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO works (id, title, owner_pseud_id, lifecycle, visibility, created_at, updated_at)
                 VALUES ($1::uuid, $2, $3::uuid, 'published', 'public', NOW(), NOW())",
            )
            .bind(&id)
            .bind(title)
            .bind(owner_pseud)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("seed work");
        }
    }
}

async fn seed_pseud(db: &lorehaven_db::Database, id: &str, account_id: &str, handle: &str) {
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES (?, ?, ?, ?, datetime('now'), datetime('now'))",
            )
            .bind(id)
            .bind(account_id)
            .bind(handle)
            .bind(handle)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("seed pseud");
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES ($1::uuid, $2::uuid, $3, $4, NOW(), NOW())",
            )
            .bind(id)
            .bind(account_id)
            .bind(handle)
            .bind(handle)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("seed pseud");
        }
    }
}

async fn seed_bookmark(db: &lorehaven_db::Database, id: &str, account_id: &str, work_id: &str) {
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO bookmarks (id, account_id, subject_type, subject_id, note, is_public, created_at, updated_at, version)
                 VALUES (?, ?, 'work', ?, '', 0, datetime('now'), datetime('now'), 1)",
            )
            .bind(id)
            .bind(account_id)
            .bind(work_id)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("seed bookmark");
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO bookmarks (id, account_id, subject_type, subject_id, note, is_public, created_at, updated_at, version)
                 VALUES ($1::uuid, $2::uuid, 'work', $3::uuid, '', 0, NOW(), NOW(), 1)",
            )
            .bind(id)
            .bind(account_id)
            .bind(work_id)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("seed bookmark");
        }
    }
}

async fn seed_media_ref(db: &lorehaven_db::Database, ref_id: &str) {
    media_resilience::insert_media_reference(
        db,
        ref_id,
        &format!("sha256:{ref_id}"),
        lorehaven_domain::media_resilience::MediaKind::Image,
    )
    .await
    .expect("seed media ref");
}

async fn seed_work_media_link(
    db: &lorehaven_db::Database,
    id: &str,
    work_id: &WorkId,
    ref_id: &str,
) {
    media_resilience::insert_work_media_reference(
        db,
        id,
        &work_id.to_string(),
        None,
        ref_id,
        lorehaven_domain::media_resilience::MediaContextKind::Moodboard,
        "https://example.com/display",
        None,
    )
    .await
    .expect("seed work-media link");
}

#[tokio::test]
async fn media_ref_collab_empty_account() {
    let dir = test_support::scratch_dir("mrc_empty");
    let tdb = test_support::TestDb::connect_with_dir("mrc-empty", &dir).await;
    let db = tdb.db();

    let recs = lorehaven_db::discovery::media_reference_collaborative_recommendations(
        db,
        &test_support::id("nobody"),
        10,
    )
    .await
    .expect("recommendations");
    assert!(recs.is_empty());
}

#[tokio::test]
async fn media_ref_collab_finds_shared_media() {
    let dir = test_support::scratch_dir("mrc_shared");
    let tdb = test_support::TestDb::connect_with_dir("mrc-shared", &dir).await;
    let db = tdb.db();

    use lorehaven_db::identity::{create_account, AccountStatus};
    use lorehaven_domain::policy::AgeState;
    let account_a = create_account(
        db,
        "a@mrc.test",
        AgeState::DeclaredAdult,
        AccountStatus::Active,
    )
    .await
    .expect("create A")
    .to_string();
    let account_b = create_account(
        db,
        "b@mrc.test",
        AgeState::DeclaredAdult,
        AccountStatus::Active,
    )
    .await
    .expect("create B")
    .to_string();

    seed_pseud(db, &test_support::id("pseud-a"), &account_a, "handle-a").await;
    seed_pseud(db, &test_support::id("pseud-b"), &account_b, "handle-b").await;

    let work1 = WorkId::new();
    let work2 = WorkId::new();
    let work3 = WorkId::new();

    seed_work(db, &work1, "Work One", &test_support::id("pseud-a")).await;
    seed_work(db, &work2, "Work Two", &test_support::id("pseud-b")).await;
    seed_work(db, &work3, "Work Three", &test_support::id("pseud-b")).await;

    seed_media_ref(db, &test_support::id("ref-1")).await;
    seed_media_ref(db, &test_support::id("ref-2")).await;

    // W1 uses ref-1; W2 uses ref-1 (shared) and ref-2; W3 uses ref-2 only.
    seed_work_media_link(db, &test_support::id("wmr-1"), &work1, "ref-1").await;
    seed_work_media_link(db, &test_support::id("wmr-2"), &work2, "ref-1").await;
    seed_work_media_link(db, &test_support::id("wmr-3"), &work2, "ref-2").await;
    seed_work_media_link(db, &test_support::id("wmr-4"), &work3, "ref-2").await;

    // A bookmarks W1.
    seed_bookmark(db, "bm-1", &account_a, &work1.to_string()).await;

    let recs =
        lorehaven_db::discovery::media_reference_collaborative_recommendations(db, &account_a, 10)
            .await
            .expect("recommendations");

    let ids: Vec<String> = recs.iter().map(|w| w.to_string()).collect();
    assert!(
        ids.contains(&work2.to_string()),
        "W2 should be recommended, got: {ids:?}"
    );
    assert!(
        !ids.contains(&work3.to_string()),
        "W3 should not be recommended, got: {ids:?}"
    );
    assert!(
        !ids.contains(&work1.to_string()),
        "bookmarked work should not be recommended"
    );
}

#[tokio::test]
async fn media_ref_collab_ranks_by_shared_count() {
    let dir = test_support::scratch_dir("mrc_rank");
    let tdb = test_support::TestDb::connect_with_dir("mrc-rank", &dir).await;
    let db = tdb.db();

    use lorehaven_db::identity::{create_account, AccountStatus};
    use lorehaven_domain::policy::AgeState;
    let account_a = create_account(
        db,
        "a@rank.test",
        AgeState::DeclaredAdult,
        AccountStatus::Active,
    )
    .await
    .expect("create A")
    .to_string();
    let account_b = create_account(
        db,
        "b@rank.test",
        AgeState::DeclaredAdult,
        AccountStatus::Active,
    )
    .await
    .expect("create B")
    .to_string();

    seed_pseud(db, &test_support::id("pseud-a"), &account_a, "handle-a").await;
    seed_pseud(db, &test_support::id("pseud-b"), &account_b, "handle-b").await;

    let work1 = WorkId::new();
    let work2 = WorkId::new();
    let work3 = WorkId::new();

    seed_work(db, &work1, "Work One", &test_support::id("pseud-a")).await;
    seed_work(db, &work2, "Work Two", &test_support::id("pseud-b")).await;
    seed_work(db, &work3, "Work Three", &test_support::id("pseud-b")).await;

    seed_media_ref(db, &test_support::id("ref-1")).await;
    seed_media_ref(db, &test_support::id("ref-2")).await;
    seed_media_ref(db, &test_support::id("ref-3")).await;

    // W1 uses ref-1, ref-2. W2 shares both (2 shared). W3 shares only ref-1 (1 shared).
    seed_work_media_link(db, &test_support::id("wmr-1"), &work1, "ref-1").await;
    seed_work_media_link(db, &test_support::id("wmr-2"), &work2, "ref-1").await;
    seed_work_media_link(db, &test_support::id("wmr-3"), &work1, "ref-2").await;
    seed_work_media_link(db, &test_support::id("wmr-4"), &work2, "ref-2").await;
    seed_work_media_link(db, &test_support::id("wmr-5"), &work3, "ref-1").await;

    seed_bookmark(db, "bm-1", &account_a, &work1.to_string()).await;

    let recs =
        lorehaven_db::discovery::media_reference_collaborative_recommendations(db, &account_a, 10)
            .await
            .expect("recommendations");

    // W2 (2 shared refs) should rank above W3 (1 shared ref).
    assert_eq!(recs.len(), 2, "both W2 and W3 should be recommended");
    assert_eq!(recs[0], work2, "W2 (2 shared) should be first");
    assert_eq!(recs[1], work3, "W3 (1 shared) should be second");
}

#[tokio::test]
async fn media_ref_collab_excludes_own_works() {
    let dir = test_support::scratch_dir("mrc_own");
    let tdb = test_support::TestDb::connect_with_dir("mrc-own", &dir).await;
    let db = tdb.db();

    use lorehaven_db::identity::{create_account, AccountStatus};
    use lorehaven_domain::policy::AgeState;
    let account_a = create_account(
        db,
        "a@own.test",
        AgeState::DeclaredAdult,
        AccountStatus::Active,
    )
    .await
    .expect("create A")
    .to_string();

    seed_pseud(db, &test_support::id("pseud-a"), &account_a, "handle-a").await;

    let work1 = WorkId::new();
    let work2 = WorkId::new();

    seed_work(db, &work1, "Work One", &test_support::id("pseud-a")).await;
    seed_work(db, &work2, "Work Two", &test_support::id("pseud-a")).await;

    seed_media_ref(db, &test_support::id("ref-1")).await;

    // Both works owned by A share ref-1, but W2 is A's own work.
    seed_work_media_link(db, &test_support::id("wmr-1"), &work1, "ref-1").await;
    seed_work_media_link(db, &test_support::id("wmr-2"), &work2, "ref-1").await;

    // A bookmarks W1.
    seed_bookmark(db, "bm-1", &account_a, &work1.to_string()).await;

    let recs =
        lorehaven_db::discovery::media_reference_collaborative_recommendations(db, &account_a, 10)
            .await
            .expect("recommendations");

    // W2 is owned by A → excluded even though it shares ref-1.
    let ids: Vec<String> = recs.iter().map(|w| w.to_string()).collect();
    assert!(
        !ids.contains(&work2.to_string()),
        "own work should not be recommended: {ids:?}"
    );
}

#[tokio::test]
async fn media_ref_collab_no_bookmarks() {
    let dir = test_support::scratch_dir("mrc_nb");
    let tdb = test_support::TestDb::connect_with_dir("mrc-nb", &dir).await;
    let db = tdb.db();

    use lorehaven_db::identity::{create_account, AccountStatus};
    use lorehaven_domain::policy::AgeState;
    let account_a = create_account(
        db,
        "a@nb.test",
        AgeState::DeclaredAdult,
        AccountStatus::Active,
    )
    .await
    .expect("create A")
    .to_string();

    seed_pseud(db, &test_support::id("pseud-a"), &account_a, "handle-a").await;
    let work1 = WorkId::new();
    seed_work(db, &work1, "Work One", &test_support::id("pseud-a")).await;
    seed_media_ref(db, &test_support::id("ref-1")).await;
    seed_work_media_link(db, &test_support::id("wmr-1"), &work1, "ref-1").await;

    // No bookmarks for A → no recommendations.
    let recs =
        lorehaven_db::discovery::media_reference_collaborative_recommendations(db, &account_a, 10)
            .await
            .expect("recommendations");
    assert!(recs.is_empty());
}
