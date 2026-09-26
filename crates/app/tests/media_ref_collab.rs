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
                 VALUES (?, ?, 'work', ?, '', FALSE, datetime('now'), datetime('now'), 1)",
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
                 VALUES ($1::uuid, $2::uuid, 'work', $3::uuid, '', FALSE, NOW(), NOW(), 1)",
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
        None,
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
    seed_work_media_link(
        db,
        &test_support::id("wmr-1"),
        &work1,
        &test_support::id("ref-1"),
    )
    .await;
    seed_work_media_link(
        db,
        &test_support::id("wmr-2"),
        &work2,
        &test_support::id("ref-1"),
    )
    .await;
    seed_work_media_link(
        db,
        &test_support::id("wmr-3"),
        &work2,
        &test_support::id("ref-2"),
    )
    .await;
    seed_work_media_link(
        db,
        &test_support::id("wmr-4"),
        &work3,
        &test_support::id("ref-2"),
    )
    .await;

    // A bookmarks W1.
    seed_bookmark(
        db,
        &test_support::id("bm-1"),
        &account_a,
        &work1.to_string(),
    )
    .await;

    let recs = lorehaven_db::discovery::media_reference_collaborative_recommendations(
        db, &account_a, None, 10,
    )
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
    seed_work_media_link(
        db,
        &test_support::id("wmr-1"),
        &work1,
        &test_support::id("ref-1"),
    )
    .await;
    seed_work_media_link(
        db,
        &test_support::id("wmr-2"),
        &work2,
        &test_support::id("ref-1"),
    )
    .await;
    seed_work_media_link(
        db,
        &test_support::id("wmr-3"),
        &work1,
        &test_support::id("ref-2"),
    )
    .await;
    seed_work_media_link(
        db,
        &test_support::id("wmr-4"),
        &work2,
        &test_support::id("ref-2"),
    )
    .await;
    seed_work_media_link(
        db,
        &test_support::id("wmr-5"),
        &work3,
        &test_support::id("ref-1"),
    )
    .await;

    seed_bookmark(
        db,
        &test_support::id("bm-1"),
        &account_a,
        &work1.to_string(),
    )
    .await;

    let recs = lorehaven_db::discovery::media_reference_collaborative_recommendations(
        db, &account_a, None, 10,
    )
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
    seed_work_media_link(
        db,
        &test_support::id("wmr-1"),
        &work1,
        &test_support::id("ref-1"),
    )
    .await;
    seed_work_media_link(
        db,
        &test_support::id("wmr-2"),
        &work2,
        &test_support::id("ref-1"),
    )
    .await;

    // A bookmarks W1.
    seed_bookmark(
        db,
        &test_support::id("bm-1"),
        &account_a,
        &work1.to_string(),
    )
    .await;

    let recs = lorehaven_db::discovery::media_reference_collaborative_recommendations(
        db, &account_a, None, 10,
    )
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
    seed_work_media_link(
        db,
        &test_support::id("wmr-1"),
        &work1,
        &test_support::id("ref-1"),
    )
    .await;

    // No bookmarks for A → no recommendations.
    let recs = lorehaven_db::discovery::media_reference_collaborative_recommendations(
        db, &account_a, None, 10,
    )
    .await
    .expect("recommendations");
    assert!(recs.is_empty());
}

/// A content filter has to reach the media-reference engine too.
///
/// This is the one recommendation surface whose work id does *not* live on
/// `works`: it selects `work_media_references` and groups on `wmr.work_id`, so
/// the exclusion has to correlate there instead of on `w.id`. A predicate written
/// for the `works` shape correlates against a column that does not exist, which
/// is a 500 on PostgreSQL and a silently-unfiltered result on SQLite -- and this
/// file's other tests all pass `None`, so nothing else would have caught it.
///
/// Reader A bookmarks W1; W2 and W3 both share a media reference with it, so both
/// are recommended. Tagging W3 must remove exactly W3.
#[tokio::test]
async fn media_ref_collab_applies_content_filters() {
    let dir = test_support::scratch_dir("mrc_filter");
    let tdb = test_support::TestDb::connect_with_dir("mrc-filter", &dir).await;
    let db = tdb.db();

    use lorehaven_db::identity::{create_account, AccountStatus};
    use lorehaven_domain::policy::AgeState;
    let account_a = create_account(
        db,
        "fa@mrc.test",
        AgeState::DeclaredAdult,
        AccountStatus::Active,
    )
    .await
    .expect("create A")
    .to_string();
    let account_b = create_account(
        db,
        "fb@mrc.test",
        AgeState::DeclaredAdult,
        AccountStatus::Active,
    )
    .await
    .expect("create B")
    .to_string();
    let pseud_a = test_support::id("pseud-fa");
    let pseud_b = test_support::id("pseud-fb");
    seed_pseud(db, &pseud_a, &account_a, "handle-fa").await;
    seed_pseud(db, &pseud_b, &account_b, "handle-fb").await;

    let bookmarked = WorkId::new();
    let allowed = WorkId::new();
    let blocked = WorkId::new();
    seed_work(db, &bookmarked, "Bookmarked", &pseud_a).await;
    seed_work(db, &allowed, "Allowed", &pseud_b).await;
    seed_work(db, &blocked, "Blocked", &pseud_b).await;

    seed_media_ref(db, &test_support::id("fref-1")).await;
    seed_work_media_link(
        db,
        &test_support::id("fwmr-1"),
        &bookmarked,
        &test_support::id("fref-1"),
    )
    .await;
    seed_work_media_link(
        db,
        &test_support::id("fwmr-2"),
        &allowed,
        &test_support::id("fref-1"),
    )
    .await;
    seed_work_media_link(
        db,
        &test_support::id("fwmr-3"),
        &blocked,
        &test_support::id("fref-1"),
    )
    .await;
    seed_bookmark(
        db,
        &test_support::id("fbm-1"),
        &account_a,
        &bookmarked.to_string(),
    )
    .await;

    // Tag only the blocked work.
    let now = chrono::Utc::now().to_rfc3339();
    let node_id = uuid::Uuid::new_v4().to_string();
    let sql_node = db.sql(
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at) VALUES (?, ?, ?, ?, ?)",
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at) VALUES ($1::uuid, $2, $3, $4, $5::timestamptz)",
    );
    let sql_tag = db.sql(
        "INSERT INTO work_tags (work_id, node_id, weight, added_at) VALUES (?, ?, ?, ?)",
        "INSERT INTO work_tags (work_id, node_id, weight, added_at) VALUES ($1::uuid, $2, $3, $4::timestamptz)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql_node)
                .bind(&node_id)
                .bind("tag")
                .bind("gore")
                .bind("gore")
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("node");
            sqlx::query(&sql_tag)
                .bind(blocked.to_string())
                .bind(&node_id)
                .bind(1i64)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("tag");
        }
        Backend::Postgres => {
            sqlx::query(&sql_node)
                .bind(&node_id)
                .bind("tag")
                .bind("gore")
                .bind("gore")
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("node");
            sqlx::query(&sql_tag)
                .bind(blocked.to_string())
                .bind(&node_id)
                .bind(1i64)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("tag");
        }
    }

    // The reader's pseud owns the filter, exactly as the settings route stores it.
    let pseud_id: uuid::Uuid = pseud_a.parse().expect("pseud id is a uuid");
    lorehaven_db::settings::add_content_filter(db, pseud_id, "tag", "gore", &now)
        .await
        .expect("store the filter under the reader's pseud");

    let recs = lorehaven_db::discovery::media_reference_collaborative_recommendations(
        db,
        &account_a,
        Some(pseud_id),
        10,
    )
    .await
    .expect("recommendations");

    assert!(
        !recs.contains(&blocked),
        "the blocked work must not be recommended"
    );
    assert!(
        recs.contains(&allowed),
        "the untagged work sharing the reference must still be recommended: {recs:?}"
    );
}
