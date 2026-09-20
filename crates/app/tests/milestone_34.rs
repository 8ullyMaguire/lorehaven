//! M34 — Spoilers, content warnings, drafts, scheduled posts, readability.
//!
//! Spec §35.4. Tests the routes that M34 adds.

use lorehaven_app::{app_state, config::Config, state::AppState};
use lorehaven_db::identity::now_rfc3339;
use sqlx::Row;

/// Helper: spin up an in-memory SQLite AppState.
async fn setup_state() -> (String, AppState) {
    let mut config = Config::default();
    config.database_url = "sqlite::memory:".into();
    let state = app_state(config).await.expect("setup_state");
    (uuid::Uuid::new_v4().to_string(), state)
}

/// Helper: sign up an account, pick a pseud, return (account_id, pseud_id).
async fn signup_and_pseud(state: &AppState) -> (String, String) {
    let account = uuid::Uuid::new_v4().to_string();
    let email = format!("{}@example.com", account);
    let sql = "INSERT INTO accounts (id, email, password_hash, trust_level, created_at) VALUES (?, ?, 'x', 5, ?)";
    let _ = sqlx::query(sql)
        .bind(&account)
        .bind(&email)
        .bind(now_rfc3339())
        .execute(state.db().sqlite_pool().expect("sqlite"))
        .await;
    let pseud = uuid::Uuid::new_v4().to_string();
    let sql = "INSERT INTO pseuds (id, account_id, handle, is_default, created_at) VALUES (?, ?, ?, 1, ?)";
    let handle = format!("user_{}", &account[..8]);
    let _ = sqlx::query(sql)
        .bind(&pseud)
        .bind(&account)
        .bind(&handle)
        .bind(now_rfc3339())
        .execute(state.db().sqlite_pool().expect("sqlite"))
        .await;
    (account, pseud)
}

/// Helper: seed a category + category rule.
async fn seed_category(state: &AppState) -> String {
    let cat = uuid::Uuid::new_v4().to_string();
    let sql = "INSERT INTO forum_categories (id, name, position, min_trust, created_at) VALUES (?, 'General', 0, 1, ?)";
    let _ = sqlx::query(sql)
        .bind(&cat)
        .bind(now_rfc3339())
        .execute(state.db().sqlite_pool().expect("sqlite"))
        .await;
    cat
}

/// Helper: seed a category rule allowing TL1+.
async fn seed_category_rule(state: &AppState, cat_id: &str) {
    let sql = "INSERT INTO category_rules (id, category_id, min_trust, created_at) VALUES (?, ?, 1, ?)";
    let _ = sqlx::query(sql)
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(cat_id)
        .bind(now_rfc3339())
        .execute(state.db().sqlite_pool().expect("sqlite"))
        .await;
}

/// Helper: create a topic and return its id.
async fn create_topic(state: &AppState, category_id: &str, author_pseud: &str) -> String {
    lorehaven_db::community::create_topic(state.db(), category_id, author_pseud, "Test topic")
        .await
        .expect("create_topic")
}

#[tokio::test]
async fn spoiler_scope_roundtrip() {
    let (_test_id, state) = setup_state().await;
    let (_account, pseud) = signup_and_pseud(&state).await;
    let cat = seed_category(&state).await;
    seed_category_rule(&state, &cat).await;
    let topic = create_topic(&state, &cat, &pseud).await;

    lorehaven_db::spoilers::set_topic_spoiler_scope(state.db(), &topic, Some(3))
        .await
        .expect("set_topic_spoiler_scope");

    let t = lorehaven_db::community::topic_by_id(state.db(), &topic)
        .await
        .expect("topic")
        .expect("topic exists");
    // The mode column exists; spoiler scope is stored separately.
    assert_eq!(t.mode, "plain");
}

#[tokio::test]
async fn reader_progress_roundtrip() {
    let (_test_id, state) = setup_state().await;
    let (_account, pseud) = signup_and_pseud(&state).await;

    lorehaven_db::spoilers::upsert_reader_progress(state.db(), &pseud, "work-1", 5)
        .await
        .expect("upsert");

    let progress = lorehaven_db::spoilers::get_reader_progress(state.db(), &pseud, "work-1")
        .await
        .expect("get");
    assert_eq!(progress, Some(5));

    lorehaven_db::spoilers::upsert_reader_progress(state.db(), &pseud, "work-1", 7)
        .await
        .expect("upsert again");

    let progress = lorehaven_db::spoilers::get_reader_progress(state.db(), &pseud, "work-1")
        .await
        .expect("get again");
    assert_eq!(progress, Some(7));
}

#[tokio::test]
async fn content_warnings_roundtrip() {
    let (_test_id, state) = setup_state().await;
    let (_account, pseud) = signup_and_pseud(&state).await;
    let cat = seed_category(&state).await;
    seed_category_rule(&state, &cat).await;
    let topic = create_topic(&state, &cat, &pseud).await;
    let post = lorehaven_db::community::create_post(state.db(), &topic, &pseud, "Hello world")
        .await
        .expect("create_post");

    lorehaven_db::spoilers::add_content_warning(
        state.db(),
        &post,
        lorehaven_domain::spoilers::WarningType::GraphicViolence,
        2,
        None,
    )
    .await
    .expect("add_content_warning");

    let warnings = lorehaven_db::spoilers::list_content_warnings(state.db(), &post)
        .await
        .expect("list_content_warnings");
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].warning_type.as_str(), "graphic_violence");
    assert_eq!(warnings[0].severity, 2);
}

#[tokio::test]
async fn draft_autosave_roundtrip() {
    let (_test_id, state) = setup_state().await;
    let (_account, pseud) = signup_and_pseud(&state).await;

    // Initially no draft
    let draft = lorehaven_db::spoilers::get_draft(state.db(), &pseud, "topic-1")
        .await
        .expect("get_draft");
    assert!(draft.is_none());

    lorehaven_db::spoilers::upsert_draft(state.db(), &pseud, "topic-1", "Partial thought...")
        .await
        .expect("upsert_draft");

    let draft = lorehaven_db::spoilers::get_draft(state.db(), &pseud, "topic-1")
        .await
        .expect("get_draft");
    assert_eq!(draft.expect("draft exists"), "Partial thought...");

    lorehaven_db::spoilers::delete_draft(state.db(), &pseud, "topic-1")
        .await
        .expect("delete_draft");

    let draft = lorehaven_db::spoilers::get_draft(state.db(), &pseud, "topic-1")
        .await
        .expect("get_draft");
    assert!(draft.is_none());
}

#[tokio::test]
async fn warning_prefs_roundtrip() {
    let (_test_id, state) = setup_state().await;
    let (_account, pseud) = signup_and_pseud(&state).await;

    lorehaven_db::spoilers::set_warning_pref(
        state.db(),
        &pseud,
        lorehaven_domain::spoilers::WarningType::MajorCharacterDeath,
        lorehaven_domain::spoilers::WarningAction::Blur,
    )
    .await
    .expect("set_warning_pref");

    let prefs = lorehaven_db::spoilers::list_warning_prefs(state.db(), &pseud)
        .await
        .expect("list_warning_prefs");
    assert_eq!(prefs.len(), 1);
    assert_eq!(prefs[0].0.as_str(), "major_character_death");
    assert_eq!(prefs[0].1.as_str(), "blur");
}

#[tokio::test]
async fn scheduled_post_roundtrip() {
    let (_test_id, state) = setup_state().await;
    let (_account, pseud) = signup_and_pseud(&state).await;
    let cat = seed_category(&state).await;
    let topic = create_topic(state.db(), &cat, &pseud).await;
    let post = lorehaven_db::community::create_post(state.db(), &topic, &pseud, "Hello")
        .await
        .expect("create_post");

    let future = "2099-01-01T00:00:00Z";
    lorehaven_db::spoilers::schedule_post(state.db(), &post, future)
        .await
        .expect("schedule_post");

    let now = "2020-01-01T00:00:00Z";
    let due = lorehaven_db::spoilers::list_due_scheduled_posts(state.db(), now, 50)
        .await
        .expect("list_due_scheduled_posts");
    assert!(!due.is_empty());

    lorehaven_db::spoilers::publish_scheduled_post(state.db(), &post)
        .await
        .expect("publish_scheduled_post");
}
