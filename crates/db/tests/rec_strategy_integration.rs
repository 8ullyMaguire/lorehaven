//! M52-05 integration: recommendation strategies produce ranked results.

use lorehaven_db::rec_strategy::{
    author_graph_strategy, bandit_strategy, completion_weight_strategy, cooccurrence_strategy,
    curator_prior_strategy, tag_graph_strategy, time_decay_strategy, RecContext, RecRegistry,
};
use lorehaven_db::Database;
use std::time::Duration;

fn make_config(url: String) -> lorehaven_db::DatabaseConfig {
    lorehaven_db::DatabaseConfig {
        url,
        max_connections: 5,
        acquire_timeout: Duration::from_secs(5),
        slow_query_warn: Duration::ZERO,
    }
}

/// Create a unique temp directory for an isolated test database.
fn temp_db_dir() -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("lorehaven-test-{}", uuid::Uuid::new_v4()));
    dir
}

async fn setup_db() -> (Database, std::path::PathBuf) {
    let dir = temp_db_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let url = format!("sqlite://{}/lorehaven.sqlite?mode=rwc", dir.display());
    let config = make_config(url);
    let db = Database::connect(&config).await.unwrap();
    db.migrate().await.unwrap();
    (db, dir)
}

fn cleanup(dir: &std::path::Path) {
    let _ = std::fs::remove_dir_all(dir);
}

fn ctx(account_id: &str) -> RecContext {
    RecContext {
        account_id: account_id.to_string(),
        seen: vec![],
        cap: 10,
    }
}

#[tokio::test]
async fn registry_with_cooccurrence_empty_for_no_bookmarks() {
    let (db, dir) = setup_db().await;
    let mut reg = RecRegistry::new(60.0);
    reg.register("cooccurrence", cooccurrence_strategy());
    let result = reg.generate(&db, ctx("user-1")).await.unwrap();
    cleanup(&dir);
    assert!(result.is_empty());
}

#[tokio::test]
async fn registry_with_tag_graph_empty_for_no_bookmarks() {
    let (db, dir) = setup_db().await;
    let mut reg = RecRegistry::new(60.0);
    reg.register("tag_graph", tag_graph_strategy());
    let result = reg.generate(&db, ctx("user-1")).await.unwrap();
    cleanup(&dir);
    assert!(result.is_empty());
}

#[tokio::test]
async fn registry_with_author_graph_empty_for_no_bookmarks() {
    let (db, dir) = setup_db().await;
    let mut reg = RecRegistry::new(60.0);
    reg.register("author_graph", author_graph_strategy());
    let result = reg.generate(&db, ctx("user-1")).await.unwrap();
    cleanup(&dir);
    assert!(result.is_empty());
}

#[tokio::test]
async fn registry_with_time_decay_empty_for_no_bookmarks() {
    let (db, dir) = setup_db().await;
    let mut reg = RecRegistry::new(60.0);
    reg.register("time_decay", time_decay_strategy());
    let result = reg.generate(&db, ctx("user-1")).await.unwrap();
    cleanup(&dir);
    assert!(result.is_empty());
}

#[tokio::test]
async fn registry_with_completion_weight_empty_for_no_works() {
    let (db, dir) = setup_db().await;
    let mut reg = RecRegistry::new(60.0);
    reg.register("completion_weight", completion_weight_strategy());
    let result = reg.generate(&db, ctx("user-1")).await.unwrap();
    cleanup(&dir);
    assert!(result.is_empty());
}

#[tokio::test]
async fn registry_with_curator_prior_empty_for_no_bookmarks() {
    let (db, dir) = setup_db().await;
    let mut reg = RecRegistry::new(60.0);
    reg.register("curator_prior", curator_prior_strategy());
    let result = reg.generate(&db, ctx("user-1")).await.unwrap();
    cleanup(&dir);
    assert!(result.is_empty());
}

#[tokio::test]
async fn registry_with_bandit_empty_for_no_works() {
    let (db, dir) = setup_db().await;
    let mut reg = RecRegistry::new(60.0);
    reg.register("bandit", bandit_strategy());
    let result = reg.generate(&db, ctx("user-1")).await.unwrap();
    cleanup(&dir);
    assert!(result.is_empty());
}

#[tokio::test]
async fn registry_blends_all_strategies_without_error() {
    let (db, dir) = setup_db().await;
    let mut reg = RecRegistry::new(60.0);
    reg.register("cooccurrence", cooccurrence_strategy());
    reg.register("tag_graph", tag_graph_strategy());
    reg.register("author_graph", author_graph_strategy());
    reg.register("time_decay", time_decay_strategy());
    reg.register("completion_weight", completion_weight_strategy());
    reg.register("curator_prior", curator_prior_strategy());
    reg.register("bandit", bandit_strategy());
    let result = reg.generate(&db, ctx("user-1")).await.unwrap();
    cleanup(&dir);
    assert!(result.is_empty());
}
