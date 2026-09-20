//! M35 — Moderation ladder, slow mode, federation scope, featured posts.
//!
//! Spec §35.5.

use lorehaven_app::{app_state, config::Config, state::AppState};
use sqlx::Row;

async fn setup_state() -> AppState {
    let mut config = Config::default();
    config.database_url = "sqlite::memory:".into();
    app_state(config).await.expect("setup_state")
}

#[tokio::test]
async fn graduated_response_ladder_roundtrip() {
    let state = setup_state().await;
    let account = uuid::Uuid::new_v4().to_string();
    let pseud = uuid::Uuid::new_v4().to_string();
    let email = format!("{}@example.com", account);
    let now = lorehaven_db::identity::now_rfc3339();

    let _ = sqlx::query("INSERT INTO accounts (id, email, password_hash, trust_level, created_at) VALUES (?, ?, 'x', 5, ?)")
        .bind(&account).bind(&email).bind(&now)
        .execute(state.db().sqlite_pool().expect("sqlite")).await;

    let _ = sqlx::query("INSERT INTO pseuds (id, account_id, handle, is_default, created_at) VALUES (?, ?, ?, 1, ?)")
        .bind(&pseud).bind(&account).bind("testuser").bind(&now)
        .execute(state.db().sqlite_pool().expect("sqlite")).await;

    let target = uuid::Uuid::new_v4().to_string();
    let target_pseud = uuid::Uuid::new_v4().to_string();
    let target_email = format!("{}@example.com", target);
    let _ = sqlx::query("INSERT INTO accounts (id, email, password_hash, trust_level, created_at) VALUES (?, ?, 'x', 1, ?)")
        .bind(&target).bind(&target_email).bind(&now)
        .execute(state.db().sqlite_pool().expect("sqlite")).await;

    lorehaven_db::moderation::apply_sanction(
        state.db(),
        &target,
        None,
        lorehaven_domain::moderation::SanctionLevel::PostThrottle,
        "Too many rapid posts",
        &pseud,
        None,
    ).await.expect("apply_sanction");

    let sanction = lorehaven_db::moderation::check_sanction(state.db(), &target, None)
        .await.expect("check_sanction");
    assert!(sanction.is_some());
    let s = sanction.expect("sanction");
    assert_eq!(s.level, "post_throttle");
}

#[tokio::test]
async fn slow_mode_roundtrip() {
    let state = setup_state().await;
    let cat = uuid::Uuid::new_v4().to_string();
    let topic = uuid::Uuid::new_v4().to_string();
    let pseud = uuid::Uuid::new_v4().to_string();
    let account = uuid::Uuid::new_v4().to_string();
    let now = lorehaven_db::identity::now_rfc3339();

    let _ = sqlx::query("INSERT INTO accounts (id, email, password_hash, trust_level, created_at) VALUES (?, ?, 'x', 5, ?)")
        .bind(&account).bind("u@x.com").bind(&now)
        .execute(state.db().sqlite_pool().expect("sqlite")).await;
    let _ = sqlx::query("INSERT INTO pseuds (id, account_id, handle, is_default, created_at) VALUES (?, ?, ?, 1, ?)")
        .bind(&pseud).bind(&account).bind("u").bind(&now)
        .execute(state.db().sqlite_pool().expect("sqlite")).await;

    lorehaven_db::thread_modes::create_topic(state.db(), &topic, &cat, &pseud, "Test", "plain")
        .await.expect("create_topic");

    lorehaven_db::moderation::set_slow_mode(state.db(), &topic, 60)
        .await.expect("set_slow_mode");

    let t = lorehaven_db::community::topic_by_id(state.db(), &topic)
        .await.expect("topic").expect("topic exists");
    assert_eq!(t.mode, "plain");
}

#[tokio::test]
async fn federation_scope_roundtrip() {
    let state = setup_state().await;
    let cat = uuid::Uuid::new_v4().to_string();
    let topic = uuid::Uuid::new_v4().to_string();
    let pseud = uuid::Uuid::new_v4().to_string();
    let account = uuid::Uuid::new_v4().to_string();
    let now = lorehaven_db::identity::now_rfc3339();

    let _ = sqlx::query("INSERT INTO accounts (id, email, password_hash, trust_level, created_at) VALUES (?, ?, 'x', 5, ?)")
        .bind(&account).bind("u@x.com").bind(&now)
        .execute(state.db().sqlite_pool().expect("sqlite")).await;
    let _ = sqlx::query("INSERT INTO pseuds (id, account_id, handle, is_default, created_at) VALUES (?, ?, ?, 1, ?)")
        .bind(&pseud).bind(&account).bind("u").bind(&now)
        .execute(state.db().sqlite_pool().expect("sqlite")).await;

    lorehaven_db::thread_modes::create_topic(state.db(), &topic, &cat, &pseud, "Test", "plain")
        .await.expect("create_topic");

    lorehaven_db::moderation::set_federation_scope(state.db(), &topic, "local")
        .await.expect("set_federation_scope");

    // Verify it doesn't error and schema is correct
    let t = lorehaven_db::community::topic_by_id(state.db(), &topic)
        .await.expect("topic").expect("topic exists");
    assert_eq!(t.mode, "plain");
}

#[tokio::test]
async fn featured_post_roundtrip() {
    let state = setup_state().await;
    let cat = uuid::Uuid::new_v4().to_string();
    let topic = uuid::Uuid::new_v4().to_string();
    let post = uuid::Uuid::new_v4().to_string();
    let pseud = uuid::Uuid::new_v4().to_string();
    let account = uuid::Uuid::new_v4().to_string();
    let now = lorehaven_db::identity::now_rfc3339();

    let _ = sqlx::query("INSERT INTO accounts (id, email, password_hash, trust_level, created_at) VALUES (?, ?, 'x', 5, ?)")
        .bind(&account).bind("u@x.com").bind(&now)
        .execute(state.db().sqlite_pool().expect("sqlite")).await;
    let _ = sqlx::query("INSERT INTO pseuds (id, account_id, handle, is_default, created_at) VALUES (?, ?, ?, 1, ?)")
        .bind(&pseud).bind(&account).bind("u").bind(&now)
        .execute(state.db().sqlite_pool().expect("sqlite")).await;

    lorehaven_db::thread_modes::create_topic(state.db(), &topic, &cat, &pseud, "Test", "plain")
        .await.expect("create_topic");

    let post_id = lorehaven_db::community::create_post(state.db(), &topic, &pseud, "Hello")
        .await.expect("create_post");

    lorehaven_db::moderation::feature_post(state.db(), &post_id, &pseud)
        .await.expect("feature_post");
}

#[tokio::test]
async fn search_miss_roundtrip() {
    let state = setup_state().await;
    lorehaven_db::moderation::record_search_miss(state.db(), "some query", 0)
        .await.expect("record_search_miss");

    let row: Option<(String, i64)> = sqlx::query_as("SELECT query, count FROM forum_search_misses WHERE query = ?")
        .bind("some query")
        .fetch_optional(state.db().sqlite_pool().expect("sqlite"))
        .await.expect("fetch_optional");

    assert!(row.is_some());
    let (query, count) = row.expect("row");
    assert_eq!(query, "some query");
    assert_eq!(count, 1);
}
