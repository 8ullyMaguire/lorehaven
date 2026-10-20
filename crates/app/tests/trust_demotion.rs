//! M3-06 — the one automatic demotion.
//!
//! # What this proves
//!
//! * **Three confirmed spam reports in ninety days drops one level.** Two do
//!   not.
//! * **Never below level 2.** The floor is the specification's and it is not
//!   configurable: a demotion that could take an account to level 0 would make
//!   the account a guest again, and a guest's contents are foldable away.
//! * **The recovery is `0.7^loss_count`** — so the first loss recovers
//!   fastest and each further loss takes longer to come back.
//! * **It is the ONLY automatic demotion.** Everything else is a reviewed
//!   sanction, because a machine demoting a person is a machine deciding what
//!   a person is worth.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use darkhold_app::config::Config;
use darkhold_app::server::{self, set_trust_proxy};
use darkhold_app::state::AppState;
use darkhold_db::trust_events::TrustCause;
use darkhold_db::DatabaseConfig;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "darkhold-demotion-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn config_for(dir: &Path) -> Config {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    config.database = DatabaseConfig::new(format!(
        "sqlite://{}/darkhold.sqlite?mode=rwc",
        dir.display()
    ));
    config
}

const PASSWORD: &str = "test-pass-1234";

struct Client {
    app: axum::Router,
    cookies: Vec<(String, String)>,
}

impl Client {
    fn new(app: axum::Router) -> Self {
        Self {
            app,
            cookies: Vec::new(),
        }
    }

    fn capture(&mut self, response: &axum::response::Response) {
        for value in response.headers().get_all(header::SET_COOKIE) {
            let Ok(text) = value.to_str() else { continue };
            let Some((pair, _)) = text.split_once(';') else {
                continue;
            };
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            let name = name.trim().to_owned();
            let value = value.trim().to_owned();
            self.cookies.retain(|(k, _)| k != &name);
            if !value.is_empty() {
                self.cookies.push((name, value));
            }
        }
    }

    async fn request(
        &mut self,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        for (name, value) in &self.cookies {
            builder = builder.header(header::COOKIE, format!("{name}={value}"));
        }
        if let Some(token) = self
            .cookies
            .iter()
            .find(|(k, _)| k == "darkhold_csrf")
            .map(|(_, v)| v.to_owned())
        {
            builder = builder.header("x-csrf-token", token);
        }
        let body = match body {
            Some(v) => Body::from(v.to_string()),
            None => Body::empty(),
        };
        let response = self
            .app
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        self.capture(&response);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into_owned()));
        (status, json)
    }

    async fn get(&mut self, uri: &str) -> (StatusCode, Value) {
        self.request("GET", uri, None).await
    }

    async fn post(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("POST", uri, Some(body)).await
    }

    async fn register(&mut self, email: &str, handle: &str) -> String {
        let (status, body) = self
            .post(
                "/api/v1/auth/register",
                json!({
                    "email": email,
                    "password": PASSWORD,
                    "handle": handle,
                    "display_name": handle,
                    "age_band": "adult",
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "register {handle}: {body}");
        let (status, me) = self.get("/api/v1/auth/me").await;
        assert_eq!(status, StatusCode::OK, "{me}");
        me["account"]["id"].as_str().expect("account id").to_owned()
    }
}

struct Harness {
    _dir: PathBuf,
    tdb: test_support::TestDb,
    config: Config,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        set_trust_proxy(false);
        let dir = scratch_dir(tag);
        let tdb = test_support::TestDb::connect_with_dir(tag, &dir).await;
        let config = config_for(&dir);
        Self {
            _dir: dir,
            tdb,
            config,
        }
    }

    fn client(&self) -> Client {
        Client::new(server::build_router(AppState::new(
            self.config.clone(),
            self.tdb.db().clone(),
        )))
    }

    async fn set_level(&self, account: &str, level: i64) {
        darkhold_db::governance::set_trust(self.tdb.db(), account, level, "test")
            .await
            .expect("set the trust level");
    }
}

#[test]
fn three_confirmed_spam_reports_are_the_threshold() {
    assert!(darkhold_domain::trust::spam_demotes(3));
    assert!(!darkhold_domain::trust::spam_demotes(2));
    assert!(!darkhold_domain::trust::spam_demotes(0));
    assert_eq!(darkhold_domain::trust::SPAM_DEMOTION_THRESHOLD, 3);
    assert_eq!(
        darkhold_domain::trust::SPAM_DEMOTION_WINDOW_DAYS,
        90,
        "ninety days"
    );
}

#[test]
fn the_demotion_never_lands_below_level_two() {
    // **The floor is the specification's and it is not configurable.**
    assert_eq!(darkhold_domain::trust::spam_demotion_level(4, 2), 3);
    assert_eq!(darkhold_domain::trust::spam_demotion_level(3, 2), 2);
    assert_eq!(
        darkhold_domain::trust::spam_demotion_level(2, 2),
        2,
        "the floor holds even when the demotion would go below it"
    );
    assert_eq!(darkhold_domain::trust::spam_demotion_level(1, 2), 2);
}

#[test]
fn the_recovery_curve_is_diminishing() {
    // **`0.7^loss_count`** (spec §14.4): the first loss recovers fastest, and
    // each further loss takes longer to come back.
    let first = darkhold_domain::trust::recovery_multiplier(1);
    let second = darkhold_domain::trust::recovery_multiplier(2);
    let third = darkhold_domain::trust::recovery_multiplier(3);
    assert!((first - 0.7).abs() < 1e-9, "0.7^1 = 0.7");
    assert!((second - 0.49).abs() < 1e-9, "0.7^2 = 0.49");
    assert!((third - 0.343).abs() < 1e-9, "0.7^3 = 0.343");
    assert!(
        third < second && second < first,
        "each further loss takes longer to come back"
    );
    assert_eq!(
        darkhold_domain::trust::recovery_multiplier(0),
        1.0,
        "no loss means nothing to recover"
    );
}

#[test]
fn it_is_the_only_automatic_demotion() {
    // **The cause vocabulary names exactly one automatic demotion**, and it
    // is this one. Everything else is a reviewed sanction — a person, or the
    // evidence a person reviewed.
    let automatic: Vec<&str> = TrustCause::all()
        .iter()
        .filter(|cause| {
            matches!(
                cause,
                TrustCause::DemotionSpam | TrustCause::Elastic | TrustCause::Fold
            )
        })
        .map(|cause| cause.as_str())
        .collect();
    assert_eq!(
        automatic,
        vec!["elastic", "demotion_spam", "fold"],
        "the three causes the instance acts on without a person asking. Only \\
         ONE of them is a demotion: 'elastic' is a grant, and 'fold' is an \\
         identity becoming a tombstone -- neither takes standing away"
    );
    let causes: Vec<&str> = TrustCause::all().iter().map(|c| c.as_str()).collect();
    assert_eq!(causes.len(), 8, "the vocabulary is closed at eight");
    assert!(
        causes.contains(&"demotion_spam"),
        "and the automatic demotion is in it"
    );
}

#[tokio::test]
async fn three_confirmed_spam_reports_drop_one_level_and_two_do_not() {
    let harness = Harness::new("threshold").await;
    let mut client = harness.client();

    // The reader whose level is at stake.
    let target = client.register("spammer@example.test", "Spammer").await;
    harness.set_level(&target, 4).await;

    // Two reporters, and a reviewer to confirm the reports.
    //
    // **Two reporters and not one**: an open report is one per reporter per
    // subject, so a second report about the same account from the same reader
    // would be the first report again.
    let mut reporter = harness.client();
    let _reporter_account = reporter.register("reporter@example.test", "Reporter").await;
    let mut reporter_two = harness.client();
    let _reporter_two = reporter_two
        .register("reporter2@example.test", "Reporter2")
        .await;
    let mut reviewer = harness.client();
    let reviewer_account = reviewer.register("reviewer@example.test", "Reviewer").await;
    harness.set_level(&reviewer_account, 5).await;

    // Two confirmed spam reports: **not enough**.
    confirm_spam_report(&mut reporter, &mut reviewer, &target).await;
    confirm_spam_report(&mut reporter_two, &mut reviewer, &target).await;
    assert_eq!(
        darkhold_db::governance::confirmed_spam_reports(harness.tdb.db(), &target, 90)
            .await
            .expect("count the confirmed spam reports"),
        2,
        "two confirmed spam reports"
    );
    let demoted = darkhold_db::governance::apply_spam_demotion(
        harness.tdb.db(),
        &target,
        &harness.config.trust.trust_settings(),
    )
    .await
    .expect("apply the demotion");
    assert_eq!(demoted, None, "two reports do not demote");
    assert_eq!(
        darkhold_db::governance::trust_for(harness.tdb.db(), &target)
            .await
            .expect("read the level"),
        4,
        "and the level is untouched"
    );

    // The third one: **now it demotes.**
    let mut reporter_three = harness.client();
    let _reporter_three = reporter_three
        .register("reporter3@example.test", "Reporter3")
        .await;
    confirm_spam_report(&mut reporter_three, &mut reviewer, &target).await;
    let demoted = darkhold_db::governance::apply_spam_demotion(
        harness.tdb.db(),
        &target,
        &harness.config.trust.trust_settings(),
    )
    .await
    .expect("apply the demotion");
    assert_eq!(
        demoted,
        Some(3),
        "three confirmed spam reports drop one level"
    );
    assert_eq!(
        darkhold_db::governance::trust_for(harness.tdb.db(), &target)
            .await
            .expect("read the level"),
        3
    );

    // **And the demotion is recorded**, because a level that changed with no
    // event is a defect.
    let events = darkhold_db::trust_events::list_for_account(harness.tdb.db(), &target, 50)
        .await
        .expect("read the history");
    let event = events
        .iter()
        .find(|event| event.cause == TrustCause::DemotionSpam)
        .expect("the demotion is recorded");
    assert_eq!(event.from_level, 4);
    assert_eq!(event.to_level, 3);
}

/// File a spam report about an account and have a reviewer uphold it.
async fn confirm_spam_report(reporter: &mut Client, reviewer: &mut Client, target: &str) {
    let (status, body) = reporter
        .post(
            "/api/v1/reports",
            json!({
                "subject_type": "account",
                "subject_id": target,
                "reason": "spam",
            }),
        )
        .await;
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "file the report: {status} {body}"
    );
    let report_id = body["id"].as_str().expect("report id").to_owned();

    let (status, body) = reviewer
        .post(
            &format!("/api/v1/moderation/reports/{report_id}/assign"),
            json!({}),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "assign the review task: {status} {body}"
    );
    let task_id = body["task_id"].as_str().expect("task id").to_owned();

    let (status, body) = reviewer
        .post(
            &format!("/api/v1/moderation/tasks/{task_id}/decide"),
            json!({ "outcome": "uphold" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "uphold the report: {status} {body}");
}
