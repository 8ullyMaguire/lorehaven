//! M3-02 — the earned side: the four axes, and what is *not* one of them.
//!
//! # The rule this file proves
//!
//! **Trust is earned through reviewed conduct** — not post count, not word
//! count, not kudos, not credits, not any activity-volume metric (spec §14.4).
//!
//! The counter-example is the whole test: an account with **ten thousand posts
//! and no reviewed contributions** must land on a **lower** level than an
//! account with **twenty reviewed contributions and ninety days of tenure**.
//! And a second test says the same thing from the other side — two accounts
//! differing only in activity volume score the same — so the rule is enforced
//! by the *shape of the computation* and not by a claim about it.
//!
//! # What the four axes are
//!
//! | axis | what it counts | why it is reviewed conduct |
//! |---|---|---|
//! | tenure days | days since the account was created | time, which nobody can rush |
//! | accepted contributions | proposals **passed** or **approved** | somebody else accepted them |
//! | vote score | the public ratings on the account's published work | readers rated it |
//! | report accuracy | reports **upheld** minus reports **dismissed** | a reviewer decided |

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use darkhold_app::config::Config;
use darkhold_app::server::{self, set_trust_proxy};
use darkhold_app::state::AppState;
use darkhold_db::DatabaseConfig;
use darkhold_domain::trust::{assess, TrustAxes, TrustEvidence, TrustLadder};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "darkhold-axes-{tag}-{}-{:?}",
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
}

#[test]
fn ten_thousand_posts_is_a_lower_level_than_twenty_reviewed_contributions() {
    // **The specification's own counter-example**, as a test.
    let axes = TrustAxes::default();
    let ladder = TrustLadder::default_ladder();

    let prolific = TrustEvidence {
        tenure_days: 0,
        accepted_contributions: 0,
        vote_score: 0,
        report_accuracy: 0,
        activity_volume: 10_000,
    };
    let reviewed = TrustEvidence {
        tenure_days: 90,
        accepted_contributions: 20,
        vote_score: 0,
        report_accuracy: 0,
        activity_volume: 0,
    };

    let prolific_level = assess(&prolific, &axes, &ladder).level;
    let reviewed_level = assess(&reviewed, &axes, &ladder).level;

    assert!(
        reviewed_level > prolific_level,
        "reviewed conduct beats volume: {reviewed_level} ({}) vs {prolific_level} ({})",
        ladder.name_of(reviewed_level).unwrap_or("?"),
        ladder.name_of(prolific_level).unwrap_or("?"),
    );
    assert_eq!(
        prolific_level, 1,
        "ten thousand posts earns nothing above 'new'"
    );
    assert!(
        reviewed_level >= 2,
        "twenty accepted contributions and ninety days earns a member"
    );
}

#[test]
fn two_accounts_differing_only_in_volume_score_the_same() {
    // **The rule enforced by the computation's shape.** The score function
    // reads four fields; `activity_volume` is not one of them, and this is
    // what "not an axis" means when it is said about code.
    let axes = TrustAxes::default();
    let ladder = TrustLadder::default_ladder();
    let quiet = TrustEvidence {
        tenure_days: 90,
        accepted_contributions: 20,
        vote_score: 3,
        report_accuracy: 1,
        activity_volume: 0,
    };
    let loud = TrustEvidence {
        activity_volume: 1_000_000,
        ..quiet
    };
    assert_eq!(
        axes.weighted(&quiet),
        axes.weighted(&loud),
        "the activity volume is not read by the score"
    );
    assert_eq!(
        assess(&quiet, &axes, &ladder).level,
        assess(&loud, &axes, &ladder).level
    );
}

#[test]
fn the_four_axes_are_the_ones_the_specification_names() {
    // **Four axes, and not one more.** Spec §14.4: "tenure days, accepted
    // contributions, vote score and report accuracy".
    let axes = TrustAxes::default();
    let evidence = TrustEvidence {
        tenure_days: 10,
        accepted_contributions: 2,
        vote_score: 3,
        report_accuracy: 1,
        activity_volume: 99,
    };
    let weighted = axes.weighted(&evidence);
    let without_activity = axes.weighted(&TrustEvidence {
        activity_volume: 0,
        ..evidence
    });
    assert_eq!(weighted, without_activity);

    // And the weights are configuration: changing one changes the score.
    let heavier = TrustAxes {
        tenure_days: 10.0,
        ..TrustAxes::default()
    };
    assert!(
        heavier.weighted(&evidence) > weighted,
        "the weights are the operator's to set"
    );
}

#[tokio::test]
async fn the_evidence_reader_counts_reviewed_conduct_and_nothing_else() {
    let harness = Harness::new("evidence").await;
    let mut client = harness.client();
    let account = client.register("axes@example.test", "Axes").await;

    // A brand-new account has no reviewed conduct at all.
    let evidence = darkhold_db::governance::evidence_for(harness.tdb.db(), &account)
        .await
        .expect("read the evidence");
    assert_eq!(evidence.accepted_contributions, 0);
    assert_eq!(evidence.vote_score, 0);
    assert_eq!(evidence.report_accuracy, 0);
    assert!(evidence.tenure_days >= 0);

    // A comment is activity, and activity is not an axis — but it *is*
    // measured, because the counter-example needs a number.
    let (status, body) = client
        .post("/api/v1/works", json!({ "title": "A Work" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    let work_id = body["id"].as_str().expect("work id").to_owned();
    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/comments"),
            json!({ "body": "A comment with enough text to be a comment." }),
        )
        .await;
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "comment: {status} {body}"
    );

    let evidence = darkhold_db::governance::evidence_for(harness.tdb.db(), &account)
        .await
        .expect("read the evidence");
    assert!(
        evidence.activity_volume > 0,
        "the activity volume is measured -- so the counter-example can be stated"
    );
    assert_eq!(
        evidence.accepted_contributions, 0,
        "a comment is not a contribution: nobody accepted it"
    );
    assert_eq!(
        evidence.vote_score, 0,
        "the work is not published, so it has no ratings to score"
    );
}

#[tokio::test]
async fn a_computed_level_is_written_with_the_basis_that_explains_it() {
    // **A level with no basis is a level somebody has to take on trust.**
    let harness = Harness::new("basis").await;
    let mut client = harness.client();
    let account = client.register("basis@example.test", "Basis").await;

    let assessment = darkhold_db::governance::recompute_trust(
        harness.tdb.db(),
        &account,
        &harness.config.trust.trust_settings(),
    )
    .await
    .expect("recompute the trust level");

    // The stored basis carries the four axis values and the weights, so the
    // level can be explained.
    //
    // **The basis is a JSON document on PostgreSQL and TEXT on SQLite**, so the
    // read casts on the one side that needs it.
    let sql = harness.tdb.db().sql(
        "SELECT basis FROM trust_levels WHERE account = ?",
        "SELECT basis::text FROM trust_levels WHERE account = $1",
    );
    let row: Option<(String,)> = match harness.tdb.db().backend() {
        darkhold_db::Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(&account)
                .fetch_optional(harness.tdb.db().sqlite_pool().expect("sqlite"))
                .await
        }
        darkhold_db::Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(&account)
                .fetch_optional(harness.tdb.db().postgres_pool().expect("postgres"))
                .await
        }
    }
    .expect("read the basis");
    let (basis,) = row.expect("a stored level");
    let parsed: Value = serde_json::from_str(&basis).expect("the basis is JSON");
    assert_eq!(
        parsed["evidence"]["tenure_days"],
        assessment.evidence.tenure_days
    );
    assert_eq!(
        parsed["evidence"]["accepted_contributions"],
        assessment.evidence.accepted_contributions
    );
    assert!(
        parsed["evidence"].get("activity_volume").is_none(),
        "the activity volume is not in the basis: it is not an axis, and a basis \\
         that carried it would be a basis for a number nobody should see"
    );
    assert_eq!(parsed["axes"]["tenure_days"], 1.0);
}
