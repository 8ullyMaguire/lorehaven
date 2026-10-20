//! M2-06: the age state, the guardian authorisation, and the age gate's
//! separation from the consent gate.
//!
//! # The gap this file exists for
//!
//! **A minor could never be authorised.** The `guardian_authorizations` table
//! existed, and nothing ever wrote to it: an account that declared itself below
//! the age threshold landed in `authorization_required` and **stayed there
//! forever**, so the guardian workflow the instance advertised could not
//! happen. A minor could not participate, whatever the operator set up.
//!
//! # The second gap
//!
//! **The age gate was not separate from the consent gate**, because
//! `consent.age_gate` was read by nothing. It is configuration that looked
//! applied and was not — the same defect the configuration contract's own rule
//! names: *"a setting that reads as configurable and is not is worse than one
//! that is absent."*
//!
//! Now: the consent gate refuses a *mutating request* without consent, and the
//! age gate decides what an account may *do* once it exists. They are different
//! questions, and `age_gate = "optional"` is the instance saying the age
//! declaration is optional — with `"required"` the default, because D-9 makes the
//! default instance an adult-content host.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use darkhold_app::config::Config;
use darkhold_app::server::{self, set_trust_proxy};
use darkhold_app::state::AppState;
use darkhold_db::DatabaseConfig;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "darkhold-age-{tag}-{}-{:?}",
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

    /// Register and return the account id.
    async fn register(&mut self, email: &str, handle: &str, age_band: &str) -> String {
        let (status, body) = self
            .post(
                "/api/v1/auth/register",
                json!({
                    "email": email,
                    "password": PASSWORD,
                    "handle": handle,
                    "display_name": handle,
                    "age_band": age_band,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "register {handle}: {body}");
        let (status, me) = self.get("/api/v1/auth/me").await;
        assert_eq!(status, StatusCode::OK, "{me}");
        me["account"]["id"].as_str().expect("account id").to_owned()
    }

    /// Whether this client may write, as the capabilities say.
    async fn can_write(&mut self) -> bool {
        let (status, me) = self.get("/api/v1/auth/me").await;
        assert_eq!(status, StatusCode::OK, "{me}");
        me["capabilities"]["can_write"].as_bool().unwrap_or(false)
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

    /// A harness whose instance has the guardian workflow turned on.
    async fn with_guardian_workflow(tag: &str) -> Self {
        let mut harness = Self::new(tag).await;
        harness.config.age.guardian_workflow_enabled = true;
        harness
    }

    fn client(&self) -> Client {
        Client::new(server::build_router(AppState::new(
            self.config.clone(),
            self.tdb.db().clone(),
        )))
    }
}

#[tokio::test]
async fn a_minor_waits_for_a_guardian_and_may_not_write_until_one_arrives() {
    let harness = Harness::with_guardian_workflow("minor").await;
    let mut client = harness.client();
    let account_id = client
        .register("minor@example.test", "Minor", "minor")
        .await;

    // The account is waiting for an authorisation.
    assert!(
        !client.can_write().await,
        "an account awaiting a guardian authorisation may not write"
    );
    let account: darkhold_domain::AccountId = account_id.parse().expect("account id");
    let stored = darkhold_db::identity::find_account(harness.tdb.db(), account)
        .await
        .expect("read")
        .expect("the account exists");
    assert_eq!(
        stored.age_state,
        darkhold_domain::policy::AgeState::AuthorizationRequired,
        "the account is waiting for a guardian, not merely flagged"
    );

    // The guardian arrives.
    darkhold_db::identity::grant_guardian_authorization(
        harness.tdb.db(),
        account,
        "a guardian's reference",
    )
    .await
    .expect("grant the authorisation");

    // And now it may write.
    assert!(
        client.can_write().await,
        "an account with a recorded guardian authorisation may write"
    );
    let stored = darkhold_db::identity::find_account(harness.tdb.db(), account)
        .await
        .expect("read")
        .expect("the account exists");
    assert_eq!(
        stored.age_state,
        darkhold_domain::policy::AgeState::AuthorizedUnderPolicy,
        "the grant moved the age state -- a recorded authorisation that does not \
         change the state is a row nobody reads"
    );

    // And the authorisation is readable, because a takedown asks "who was
    // allowed to see this".
    let authorisation = darkhold_db::identity::guardian_authorization(harness.tdb.db(), account)
        .await
        .expect("read the authorisation")
        .expect("there is one");
    assert_eq!(authorisation.status, "granted");
    assert_eq!(
        authorisation.verification_reference.as_deref(),
        Some("a guardian's reference"),
        "the authorisation carries what the guardian's verification produced"
    );

    harness.tdb.db().close().await;
}

#[tokio::test]
async fn an_account_with_no_guardian_workflow_cannot_be_authorised_into_writing() {
    // **The default.** With no workflow configured, an under-threshold account
    // is created `restricted`: it may read and may not write, message or be
    // listed. Reading remains available -- and is available anonymously in any
    // case, so refusing registration would withhold nothing while pretending
    // to protect someone.
    let harness = Harness::new("no-workflow").await;
    let mut client = harness.client();
    client
        .register("minor2@example.test", "Minor2", "minor")
        .await;

    assert!(
        !client.can_write().await,
        "an under-threshold account on an instance with no guardian workflow may \
         not write"
    );
}

#[tokio::test]
async fn the_age_gate_is_separate_from_the_consent_gate() {
    // The consent gate refuses a *mutating request* without consent. The age
    // gate decides what an account may *do* once it exists. They are different
    // questions, and the configuration says which one the instance has turned
    // on.
    //
    // `age_gate = "required"` is the default, because D-9 makes the default
    // instance an adult-content host. `"optional"` is the instance saying the
    // declaration is optional -- and it is a **widening**, so it has to be
    // chosen deliberately.
    let harness = Harness::new("gate").await;
    let mut client = harness.client();
    client
        .register("unknown@example.test", "Unknown", "unknown")
        .await;

    assert!(
        !client.can_write().await,
        "with the age gate required, an account of unknown age may not write"
    );

    // The same account on an instance with the gate optional. A second
    // harness, because the session belongs to the client that made it and a
    // client with a different router is a different client.
    let optional = Harness::new("gate-optional").await;
    let mut optional_config = optional.config.clone();
    optional_config.consent.age_gate = "optional".to_owned();
    let mut client = Client::new(server::build_router(AppState::new(
        optional_config,
        optional.tdb.db().clone(),
    )));
    client
        .register("unknown2@example.test", "Unknown2", "unknown")
        .await;
    let (status, me) = client.get("/api/v1/auth/me").await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert!(
        me["capabilities"]["can_write"].as_bool().unwrap_or(false),
        "with the age gate optional, an account of unknown age may write: {me}"
    );

    harness.tdb.db().close().await;
    optional.tdb.db().close().await;
}
