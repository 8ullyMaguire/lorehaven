//! M45-21 acceptance: structured DNF (did-not-finish) reasons for abandoned works.
//!
//! Tests the DNF DB layer against an in-memory SQLite database and verifies
//! that unauthenticated HTTP requests are rejected with 401.

use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_db::{identity, Database, DatabaseConfig};
use serde_json::Value;
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m45-21-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn config_for(dir: &PathBuf) -> Config {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    config.database = DatabaseConfig::new(format!(
        "sqlite://{}/lorehaven.sqlite?mode=rwc",
        dir.display()
    ));
    config
}

struct Harness {
    _dir: PathBuf,
    config: Config,
    db: Database,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let config = config_for(&dir);
        let db = Database::connect(&config.database)
            .await
            .expect("db connect");
        db.migrate().await.expect("migrations");
        Self {
            _dir: dir,
            config,
            db,
        }
    }

    fn router(&self) -> axum::Router {
        server::build_router(AppState::new(self.config.clone(), self.db.clone()))
    }

    async fn call(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let builder = Request::builder().uri(path).method(method);
        let req = if let Some(b) = body {
            builder
                .header("content-type", "application/json")
                .body(Body::from(b.to_string()))
                .unwrap()
        } else {
            builder.body(Body::empty()).unwrap()
        };
        let resp = self.router().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 10_000_000)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json)
    }
}

#[tokio::test]
async fn test_dnf_endpoints_unauthenticated() {
    let h = Harness::new("dnf-unauth").await;

    let body = serde_json::json!({
        "reason": "slow_pacing",
        "note": "Got too slow at chapter 10"
    });

    // All endpoints require authentication
    let (status, _) = h
        .call(
            Method::POST,
            "/api/v1/works/11111111-1111-1111-1111-111111111111/dnf",
            Some(body.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = h
        .call(
            Method::PUT,
            "/api/v1/works/11111111-1111-1111-1111-111111111111/dnf",
            Some(body),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = h
        .call(
            Method::GET,
            "/api/v1/works/11111111-1111-1111-1111-111111111111/dnf",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = h
        .call(
            Method::DELETE,
            "/api/v1/works/11111111-1111-1111-1111-111111111111/dnf",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = h.call(Method::GET, "/api/v1/me/dnf", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_dnf_db_layer_roundtrip() {
    use lorehaven_db::dnf;

    let h = Harness::new("dnf-db").await;
    let db = &h.db;

    // Create real account + pseud + work so FK constraints are satisfied
    let account = identity::create_account(
        db,
        "dnf-test@example.com",
        lorehaven_domain::policy::AgeState::Unknown,
        identity::AccountStatus::Active,
    )
    .await
    .expect("create account");
    let pseud = identity::create_pseud(db, account, "dnf-tester", "DNF Tester")
        .await
        .expect("create pseud");
    let work = lorehaven_db::content::create_work(db, pseud, "DNF Test Work", Some("public"))
        .await
        .expect("create work");

    let now = "2026-09-24T12:00:00Z";

    // Upsert a DNF record
    dnf::upsert_dnf(
        db,
        account,
        pseud,
        work.id,
        "slow_pacing",
        Some("Bogged down"),
        true,
        now,
    )
    .await
    .expect("upsert DNF");

    // Read it back
    let row = dnf::read_dnf(db, pseud, work.id)
        .await
        .expect("read DNF")
        .expect("record exists");
    assert_eq!(row.reason, "slow_pacing");
    assert_eq!(row.note.as_deref(), Some("Bogged down"));
    assert!(row.is_public);

    // Upsert again (update)
    dnf::upsert_dnf(db, account, pseud, work.id, "triggering", None, false, now)
        .await
        .expect("update DNF");

    let row = dnf::read_dnf(db, pseud, work.id)
        .await
        .expect("read after update")
        .expect("record still exists");
    assert_eq!(row.reason, "triggering");
    assert!(!row.is_public);

    // Aggregate counts (only public records show up)
    let counts = dnf::aggregate_dnf_counts(db, work.id)
        .await
        .expect("aggregate DNF");
    assert!(
        counts.is_empty(),
        "no public records after update to private"
    );

    // Make it public again for aggregate test
    dnf::upsert_dnf(
        db,
        account,
        pseud,
        work.id,
        "abandoned_by_author",
        None,
        true,
        now,
    )
    .await
    .expect("make public");

    let counts = dnf::aggregate_dnf_counts(db, work.id)
        .await
        .expect("aggregate DNF");
    assert_eq!(counts.len(), 1);
    assert_eq!(counts[0].0, "abandoned_by_author");
    assert_eq!(counts[0].1, 1);

    // List by pseud
    let list = dnf::list_dnf_by_pseud(db, pseud).await.expect("list DNF");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].reason, "abandoned_by_author");

    // Delete
    let deleted = dnf::delete_dnf(db, pseud, work.id, now)
        .await
        .expect("delete DNF");
    assert!(deleted);

    // After delete, read returns None
    let row = dnf::read_dnf(db, pseud, work.id)
        .await
        .expect("read after delete");
    assert!(row.is_none());

    // Second delete returns false
    let deleted = dnf::delete_dnf(db, pseud, work.id, now)
        .await
        .expect("delete again");
    assert!(!deleted);
}

#[tokio::test]
async fn test_dnf_unique_constraint_per_pseud_work() {
    use lorehaven_db::dnf;

    let h = Harness::new("dnf-unique").await;
    let db = &h.db;

    let account = identity::create_account(
        db,
        "dnf-unique@example.com",
        lorehaven_domain::policy::AgeState::Unknown,
        identity::AccountStatus::Active,
    )
    .await
    .expect("create account");
    let pseud = identity::create_pseud(db, account, "dnf-unique", "DNF Unique")
        .await
        .expect("create pseud");
    let work = lorehaven_db::content::create_work(db, pseud, "DNF Unique Work", Some("public"))
        .await
        .expect("create work");

    let now = "2026-09-24T12:00:00Z";

    // First insert
    dnf::upsert_dnf(db, account, pseud, work.id, "slow_pacing", None, false, now)
        .await
        .expect("first upsert");

    // Second insert (same pseud+work) should update, not error
    dnf::upsert_dnf(db, account, pseud, work.id, "triggering", None, true, now)
        .await
        .expect("second upsert should update");

    // Only one record
    let list = dnf::list_dnf_by_pseud(db, pseud).await.expect("list DNF");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].reason, "triggering");

    // A different pseud can also DNF the same work
    let other_account = identity::create_account(
        db,
        "dnf-other@example.com",
        lorehaven_domain::policy::AgeState::Unknown,
        identity::AccountStatus::Active,
    )
    .await
    .expect("create other account");
    let other_pseud = identity::create_pseud(db, other_account, "dnf-other", "DNF Other")
        .await
        .expect("create other pseud");
    dnf::upsert_dnf(
        db,
        other_account,
        other_pseud,
        work.id,
        "not_my_taste",
        None,
        true,
        now,
    )
    .await
    .expect("other pseud upsert");

    // Aggregate should now show two records
    let counts = dnf::aggregate_dnf_counts(db, work.id).await.unwrap();
    let total: i64 = counts.iter().map(|(_, c)| c).sum();
    assert_eq!(total, 2);
}
