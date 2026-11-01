//! M59: the extension host, through the real router.
//!
//! The resolution rules are unit-tested in `crates/app/src/ext/mod.rs`. These are
//! the properties that belong to the assembled server:
//!
//! - **a disabled surface is absent and says which one it is** — `404`, the code
//!   `MODULE_UNAVAILABLE`, and the module's own id in the message;
//! - **the same route is not a 404 when the surface is on**, which is what stops
//!   the refusal above from being unconditional;
//! - **nothing else moves** — disabling one extension leaves its neighbours
//!   answering exactly as before;
//! - **the catalogue is core**: `/api/v1/modules` answers on an instance that
//!   has switched everything else off, because a page must be able to ask why it
//!   is empty;
//! - **no surface can be mounted outside the mechanism** — the test walks
//!   `server.rs` itself and fails if a module is mounted without a registry row.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use darkhold_app::config::Config;
use darkhold_app::server;
use darkhold_app::state::AppState;
use serde_json::Value;
use test_support::TestDb;
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "darkhold-m65-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn config_with(dir: &Path, disabled: &[&str]) -> Config {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    config.modules.disabled = disabled.iter().map(|s| (*s).to_owned()).collect();
    config
}

fn config_enabling(dir: &Path, enabled: &[&str]) -> Config {
    let mut config = config_with(dir, &[]);
    config.modules.enabled = enabled.iter().map(|s| (*s).to_owned()).collect();
    config
}

async fn state(tag: &str, disabled: &[&str]) -> (AppState, TestDb) {
    let dir = scratch_dir(tag);
    let db = TestDb::connect_with_dir(tag, &dir).await;
    let config = config_with(&dir, disabled);
    let state = AppState::new(config, db.db().clone());
    (state, db)
}

async fn state_enabling(tag: &str, enabled: &[&str]) -> (AppState, TestDb) {
    let dir = scratch_dir(tag);
    let db = TestDb::connect_with_dir(tag, &dir).await;
    let config = config_enabling(&dir, enabled);
    let state = AppState::new(config, db.db().clone());
    (state, db)
}

async fn get(state: &AppState, path: &str) -> (StatusCode, Value) {
    let app = server::build_router(state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .uri(path)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 512 * 1024)
        .await
        .expect("body");
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

#[tokio::test]
async fn a_disabled_surface_is_absent_and_names_itself() {
    let (state, _db) = state("disabled", &["dnf"]).await;

    let (status, body) = get(&state, "/api/v1/me/dnf").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    let error = &body["error"];
    assert_eq!(error["code"], "MODULE_UNAVAILABLE", "body: {body}");
    let message = error["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("dnf"),
        "the refusal must name the module it is about: {message}"
    );
}

#[tokio::test]
async fn the_same_route_is_not_a_404_when_the_surface_is_on() {
    // Two controls in one: the 404 above is not "a route that never existed", and
    // `quiz` — off by default since §59.10 — comes back when the instance's
    // `enabled` list names it. That is the whole instance layer, in one request.
    let (state, _db) = state_enabling("enabled", &["quiz"]).await;

    let (status, body) = get(&state, "/api/v1/quiz/works").await;
    assert_ne!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body: {body}");
}

#[tokio::test]
async fn disabling_one_surface_leaves_its_neighbours_answering() {
    let (state, _db) = state("neighbours", &["dnf"]).await;

    let (draw, _) = get(&state, "/api/v1/discovery/draw").await;
    let (catalogue, _) = get(&state, "/api/v1/catalogue?limit=1").await;
    assert_ne!(draw, StatusCode::NOT_FOUND, "draw must still answer");
    assert_ne!(catalogue, StatusCode::NOT_FOUND, "catalogue must still answer");
}

#[tokio::test]
async fn the_catalogue_is_core_and_lists_every_registered_surface() {
    let (state, _db) = state("catalogue", &["dnf"]).await;

    let (status, body) = get(&state, "/api/v1/modules").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let extensions = body["modules"].as_array().expect("array");
    assert!(
        extensions.len() >= 90,
        "the registry should hold every surface; got {}",
        extensions.len()
    );

    let dnf = extensions
        .iter()
        .find(|row| row["id"] == "dnf")
        .expect("dnf is registered");
    assert_eq!(dnf["enabled"], false, "disabled in this instance's config");
    assert_eq!(dnf["stability"], "stable");

    let quiz = extensions
        .iter()
        .find(|row| row["id"] == "quiz")
        .expect("quiz is registered");
    assert_eq!(quiz["enabled"], false, "off by default since §59.10");
    assert_eq!(quiz["toggle"], "per_user");

    let works = extensions
        .iter()
        .find(|row| row["id"] == "works")
        .expect("works is registered");
    assert_eq!(works["enabled"], true, "untouched surfaces keep their default");

    // The core is named so a client can tell "no row" from "not on".
    let core = body["core"].as_array().expect("core list");
    assert!(core.iter().any(|id| id == "auth"));

    // §59.7: the idea catalogue rides the same payload, because an operator asking
    // "what else is there" should not have to know a second endpoint exists.
    let ideas = body["ideas"].as_array().expect("ideas array");
    assert!(
        ideas.len() >= 150,
        "the idea catalogue should carry the roadmap; got {}",
        ideas.len()
    );
    let first = &ideas[0];
    assert!(first["source"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(first["relevance"].as_u64().is_some_and(|r| r <= 10));
    assert_eq!(body["counts"]["ideas"], ideas.len());
}

#[tokio::test]
async fn an_extension_the_instance_switched_off_is_still_listed_as_off() {
    // The catalogue is the operator's index into their own config: a row that
    // vanished would leave them hunting the id in a 6,000-line file.
    let (state, _db) = state("listed", &["arena"]).await;
    let (_, body) = get(&state, "/api/v1/modules").await;
    let arena = body["modules"]
        .as_array()
        .expect("array")
        .iter()
        .find(|row| row["id"] == "arena")
        .expect("arena listed");
    assert_eq!(arena["enabled"], false);
}

#[tokio::test]
async fn every_mounted_module_is_core_or_registered() {
    // The gate against the next surface: a module mounted without a registry row
    // would be a surface nothing can switch off, and its absence would never be
    // tested. Reads `server.rs` because that is where mounting happens.
    let source = include_str!("../src/server.rs");
    let mut mounted: Vec<String> = Vec::new();
    for (idx, _) in source.match_indices("module_mount(&state, \"") {
        let rest = &source[idx + "module_mount(&state, \"".len()..];
        let Some(end) = rest.find('"') else { continue };
        mounted.push(rest[..end].to_owned());
    }
    assert!(
        mounted.len() > 80,
        "the sweep mounts every module; found {} module_mount call sites",
        mounted.len()
    );

    let mut unknown: Vec<String> = mounted
        .into_iter()
        .filter(|id| darkhold_domain::modules::find(id).is_none() && !darkhold_domain::modules::is_core(id))
        .collect();
    unknown.sort();
    unknown.dedup();
    assert!(
        unknown.is_empty(),
        "these modules are mounted but not registered, so nothing can switch them off \
         and nothing tests their absence: {unknown:?}"
    );
}

#[tokio::test]
async fn a_config_that_names_a_fixed_surface_does_not_load() {
    let dir = scratch_dir("fixed");
    let slug_dir = dir.join("slug");
    std::fs::create_dir_all(&slug_dir).expect("dir");
    let mut config = config_with(&dir, &["moderation"]);
    config.modules.disabled = vec!["moderation".to_owned()];
    let result = config.validate();
    let error = result.expect_err("a fixed surface must not be switchable");
    let text = format!("{error:#}");
    assert!(text.contains("moderation"), "{text}");
    assert!(text.contains("§59.4"), "the refusal quotes the rule: {text}");
}

#[tokio::test]
async fn a_typo_in_the_extension_table_does_not_load() {
    let dir = scratch_dir("typo");
    let config = config_with(&dir, &["quizz"]);
    let error = config.validate().expect_err("a typo must not start an instance");
    let text = format!("{error:#}");
    assert!(text.contains("quizz"), "{text}");
    assert!(text.contains("`quiz`"), "the nearest id is offered: {text}");
}
