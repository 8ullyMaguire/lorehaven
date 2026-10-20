//! M3-04 — the elastic allocation: a site with three users still has a
//! moderator.
//!
//! # What this proves
//!
//! Spec §14.4: "When a privilege's eligible pool is **empty**, the requirement
//! falls until at least `trust.elastic_min_holders` holders exist — but never
//! below `trust.floor_level`."
//!
//! * **With no level-4 accounts, a level-2 account holds the moderation
//!   queue.** The site is small, and a site with three users still has a
//!   moderator.
//! * **When a level-4 account appears, the requirement returns to 4 on the
//!   next evaluation** — and the level-2 account loses the privilege
//!   *without anybody deciding to take it*. That is the **no ratchet**: the
//!   requirement is a pure function of the pool, so it has no memory of what
//!   it decided last time.
//! * **The elastic holder's grant is recorded as `cause = elastic`**, because
//!   a privilege that nobody holds and that somebody therefore holds by the
//!   allocation is a fact about the instance.

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
        "darkhold-elastic-{tag}-{}-{:?}",
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

    /// **The next evaluation** (spec §14.4).
    async fn evaluate(&self) {
        darkhold_db::governance::evaluate_requirements(
            self.tdb.db(),
            self.config.trust.ladder(),
            &self.config.trust.require,
            &self.config.trust.elastic_policy(),
        )
        .await
        .expect("evaluate the requirements");
    }
}

#[tokio::test]
async fn a_site_with_no_moderators_still_has_one() {
    let harness = Harness::new("small-site").await;
    let mut client = harness.client();

    // Three users, none of them a moderator. **Nobody** holds the moderation
    // queue, so the pool is empty and the requirement falls.
    let mut reader = harness.client();
    let reader_account = reader.register("reader@example.test", "Reader").await;
    harness.set_level(&reader_account, 2).await;
    client.register("second@example.test", "Second").await;
    client.register("third@example.test", "Third").await;

    harness.evaluate().await;

    let requirement = darkhold_db::governance::requirement_for(
        harness.tdb.db(),
        darkhold_domain::trust::Privilege::ModerationQueue,
    )
    .await
    .expect("read the requirement")
    .expect("the requirement is stored");
    assert_eq!(
        requirement.basis,
        darkhold_domain::trust::RequirementBasis::Elastic,
        "the pool was empty, so the requirement fell"
    );
    assert_eq!(
        requirement.level, 2,
        "and it fell to the floor -- the level that actually has somebody in it"
    );

    // **And the level-2 reader holds the moderation queue.**
    let (status, body) = reader.get("/api/v1/moderation/queue").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a site with three users still has a moderator: {body}"
    );
}

#[tokio::test]
async fn a_moderator_appearing_takes_the_privilege_back_on_the_next_evaluation() {
    // **No ratchet.** The requirement returns to nominal as soon as enough
    // people clear the real bar — and the level-2 holder loses the privilege
    // *without anybody deciding to take it*.
    let harness = Harness::new("no-ratchet").await;
    let mut reader = harness.client();
    let reader_account = reader.register("reader@example.test", "Reader").await;
    harness.set_level(&reader_account, 2).await;
    harness.evaluate().await;

    // The elastic holder has the queue.
    let (status, _) = reader.get("/api/v1/moderation/queue").await;
    assert_eq!(status, StatusCode::OK, "the elastic holder has the queue");

    // **And the grant is recorded**, because a privilege that nobody holds and
    // that somebody therefore holds by the allocation is a fact about the
    // instance.
    let events = darkhold_db::trust_events::list_for_account(harness.tdb.db(), &reader_account, 50)
        .await
        .expect("read the history");
    let elastic = events
        .iter()
        .find(|event| {
            event.cause == TrustCause::Elastic
                && event.evidence_ref.as_deref() == Some("moderation_queue")
        })
        .expect("the elastic grant is recorded");
    assert_eq!(
        elastic.from_level, elastic.to_level,
        "the grant did not move the level"
    );
    assert_eq!(
        elastic.evidence_ref.as_deref(),
        Some("moderation_queue"),
        "the event says which privilege was granted: {elastic:?}"
    );

    // Now a real moderator appears.
    let mut moderator = harness.client();
    let moderator_account = moderator.register("mod@example.test", "Mod").await;
    harness.set_level(&moderator_account, 4).await;

    // **The next evaluation.**
    harness.evaluate().await;

    let requirement = darkhold_db::governance::requirement_for(
        harness.tdb.db(),
        darkhold_domain::trust::Privilege::ModerationQueue,
    )
    .await
    .expect("read the requirement")
    .expect("the requirement is stored");
    assert_eq!(
        requirement.basis,
        darkhold_domain::trust::RequirementBasis::Nominal,
        "the requirement returns to nominal as soon as enough people clear the bar"
    );
    assert_eq!(requirement.level, 4);

    // And the level-2 reader has lost it — without anybody deciding to take
    // it.
    let (status, body) = reader.get("/api/v1/moderation/queue").await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the elastic holder lost the privilege when the real bar was cleared: {body}"
    );
    // And the moderator has it.
    let (status, body) = moderator.get("/api/v1/moderation/queue").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the moderator has the queue: {body}"
    );
}

#[tokio::test]
async fn the_elastic_requirement_never_falls_below_the_floor() {
    // **The floor is the floor.** A site with three brand-new accounts has
    // nobody above the floor, so the requirement stays at the floor and the
    // privilege is held by nobody — which is a better outcome than a
    // privilege held by whoever registered first.
    let harness = Harness::new("floor").await;
    let mut client = harness.client();
    let account = client.register("new@example.test", "New").await;
    harness.set_level(&account, 1).await;
    harness.evaluate().await;

    let requirement = darkhold_db::governance::requirement_for(
        harness.tdb.db(),
        darkhold_domain::trust::Privilege::ModerationQueue,
    )
    .await
    .expect("read the requirement")
    .expect("the requirement is stored");
    assert_eq!(
        requirement.level, harness.config.trust.floor_level,
        "the requirement never falls below the floor"
    );
    assert!(
        requirement.holders < harness.config.trust.elastic_min_holders,
        "and nobody holds it even at the floor -- which is the honest outcome"
    );

    let (status, _) = client.get("/api/v1/moderation/queue").await;
    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::UNAUTHORIZED,
        "a level-1 reader does not hold the queue: {status}"
    );
}
