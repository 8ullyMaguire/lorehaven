//! M3-01 — the trust ladder: seven levels, their names, and their unlocks.
//!
//! # What this file proves
//!
//! 1. **The ladder is the specification's table.** Seven levels, the
//!    specification's names, in order.
//! 2. **The names and the unlock table are configuration.** The code holds no
//!    hard-coded level numbers, so a route asks for a *privilege* and the
//!    ladder says which level holds it. A test proves it by changing the
//!    ladder in the configuration and watching the gate move.
//! 3. **The specification's Darkhold unlocks are at the specification's
//!    levels** — a mood at level 2, a channel at level 3, the moderation queue
//!    at level 4.
//! 4. **A guest can never extend trust**, and a configuration that says
//!    otherwise is refused at startup rather than ignored.
//!
//! # The mood
//!
//! The specification puts *create a mood* at level 2 and *create a channel* at
//! level 3, and this file proves both. The **route** for a mood arrives with
//! M17 and the route for a channel with M6, so what is proved here is the
//! **unlock table** — which is what M3-01 ships, and what those routes will
//! consult when they arrive.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use darkhold_app::config::Config;
use darkhold_app::server::{self, set_trust_proxy};
use darkhold_app::state::AppState;
use darkhold_db::DatabaseConfig;
use darkhold_domain::trust::{Privilege, TrustLadder};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "darkhold-trust-{tag}-{}-{:?}",
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

    /// Register and return the account id.
    async fn register(&mut self, email: &str, handle: &str) -> String {
        let (status, body) = self
            .request(
                "POST",
                "/api/v1/auth/register",
                Some(json!({
                    "email": email,
                    "password": PASSWORD,
                    "handle": handle,
                    "display_name": handle,
                    "age_band": "adult",
                })),
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

    /// Set an account's trust level, the way an operator or a test would.
    async fn set_level(&self, account: &str, level: i64) {
        darkhold_db::governance::set_trust(self.tdb.db(), account, level, "test")
            .await
            .expect("set the trust level");
    }
}

#[test]
fn the_ladder_is_the_specifications_seven_levels_and_their_names() {
    let ladder = TrustLadder::default_ladder();
    let names: Vec<&str> = ladder
        .sorted()
        .iter()
        .map(|spec| spec.name.as_str())
        .collect();
    assert_eq!(
        names,
        vec![
            "anonymous",
            "new",
            "member",
            "trusted",
            "moderator",
            "senior reviewer",
            "trustee"
        ],
        "spec §14.4's table, in order"
    );
}

#[test]
fn a_mood_is_a_level_two_unlock_and_a_channel_a_level_three() {
    let ladder = TrustLadder::default_ladder();
    assert_eq!(
        ladder.level_holding(Privilege::CreateMood),
        Some(2),
        "spec §14.4: level 2 creates a mood"
    );
    assert_eq!(
        ladder.level_holding(Privilege::CreateChannel),
        Some(3),
        "spec §14.4: level 3 creates a channel"
    );
    assert!(
        !ladder.holds(1, Privilege::CreateMood),
        "a level-1 account cannot create a mood"
    );
    assert!(
        ladder.holds(2, Privilege::CreateMood),
        "a level-2 account can"
    );
}

#[test]
fn the_moderation_queue_is_level_four_and_the_full_engine_is_too() {
    let ladder = TrustLadder::default_ladder();
    assert_eq!(ladder.level_holding(Privilege::ModerationQueue), Some(4));
    assert_eq!(ladder.level_holding(Privilege::FastHide), Some(4));
    assert_eq!(ladder.level_holding(Privilege::LockPin), Some(4));
    assert_eq!(ladder.level_holding(Privilege::FullEngine), Some(4));
    assert_eq!(ladder.level_holding(Privilege::ResolveReports), Some(5));
    assert_eq!(ladder.level_holding(Privilege::ScopedBans), Some(5));
    assert_eq!(ladder.level_holding(Privilege::JuryService), Some(5));
}

#[test]
fn the_trustee_holds_everything_except_the_instance_settings() {
    // Spec §14.4's row for level 6: "everything except instance-wide
    // settings". The instance-wide settings are the **operator role's**, and
    // the operator role is not the trust ladder.
    let ladder = TrustLadder::default_ladder();
    assert!(!ladder.holds(6, Privilege::InstanceSettings));
    for privilege in Privilege::all() {
        if privilege == Privilege::InstanceSettings {
            continue;
        }
        assert!(
            ladder.holds(6, privilege),
            "the trustee holds {privilege:?}"
        );
    }
}

#[tokio::test]
async fn a_reader_below_the_ladder_cannot_hold_the_moderation_queue() {
    // **The gate asks for a privilege, not for a number.** The route does not
    // know that the moderation queue is level 4 — the ladder does.
    let harness = Harness::new("queue").await;
    let mut client = harness.client();
    let account = client.register("reader@example.test", "Reader").await;

    // Below the bar.
    harness.set_level(&account, 1).await;
    let (status, body) = client.get("/api/v1/moderation/queue").await;
    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::UNAUTHORIZED,
        "a level-1 reader cannot hold the moderation queue: {status} {body}"
    );

    // At the bar.
    harness.set_level(&account, 4).await;
    let (status, body) = client.get("/api/v1/moderation/queue").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a level-4 reader holds the moderation queue: {body}"
    );
}

#[tokio::test]
async fn the_ladder_is_configuration_and_moving_it_moves_the_gate() {
    // **The proof that the code holds no hard-coded level numbers.** An
    // instance that wants its moderators at level 2 writes a different ladder,
    // and the gate follows — with no change to the route.
    let harness = Harness::new("moved-ladder").await;
    let mut client = harness.client();
    let account = client.register("moved@example.test", "Moved").await;
    harness.set_level(&account, 3).await;

    // **A second account at level 4**, so the pool is *not* empty and the
    // elastic allocation does not move the requirement. Without it the
    // requirement would fall to whatever level has somebody in it, and the
    // test would be proving the elastic rule rather than the ladder.
    let mut moderator = harness.client();
    let moderator_account = moderator.register("mod@example.test", "Mod").await;
    harness.set_level(&moderator_account, 4).await;

    // With the default ladder, a level-3 reader cannot hold the queue.
    let (status, body) = client.get("/api/v1/moderation/queue").await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the default ladder puts the queue at level 4: {status} {body}"
    );
    assert_eq!(
        body["error"]["code"], "TRUST_LEVEL_INSUFFICIENT",
        "the refusal names the trust level it wanted: {body}"
    );

    // With a ladder that hands it out at level 2, the same reader can — and
    // **the route did not change**.
    let mut config = harness.config.clone();
    let mut moved = TrustLadder::default_ladder();
    for spec in moved.levels.iter_mut() {
        if spec.level == 2 {
            spec.unlocks.push(Privilege::ModerationQueue);
        }
        if spec.level == 4 {
            spec.unlocks.retain(|p| *p != Privilege::ModerationQueue);
        }
    }
    assert_eq!(
        moved.level_holding(Privilege::ModerationQueue),
        Some(2),
        "the ladder now hands the queue out at level 2"
    );
    config.trust.levels = moved;

    let mut moved_client = Client::new(server::build_router(AppState::new(
        config,
        harness.tdb.db().clone(),
    )));
    // Carry the session across to the new router.
    moved_client.cookies = client.cookies.clone();
    let (status, body) = moved_client.get("/api/v1/moderation/queue").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the moved ladder moved the gate, with no change to the route: {body}"
    );
}

#[test]
fn a_guest_never_earns_trust_and_a_configuration_that_says_otherwise_is_refused() {
    // **False, always** (spec §14.1). A configuration that says otherwise is
    // refused at startup rather than ignored, because a guest who can extend
    // its trust makes the guest door a trust farm.
    let config = Config::development_defaults();
    assert!(
        !config.trust.guest_earns,
        "the default is false, and it is not configurable"
    );

    // The refusal is the configuration's own validation, so the check is a
    // *startup error* rather than a silent clamp.
    let error = Config::parse_from_str("[trust]\nguest_earns = true\n")
        .expect_err("a guest that earns trust must be refused");
    assert!(
        error.to_string().contains("guest_earns"),
        "the refusal names the setting: {error}"
    );
}

#[test]
fn the_operator_may_tighten_a_requirement_and_may_not_loosen_it() {
    let ladder = TrustLadder::default_ladder();
    let mut requirements = darkhold_domain::trust::TrustRequirements::default();

    requirements
        .require
        .insert(Privilege::TagProposals.as_str().to_owned(), 4);
    assert_eq!(requirements.level_for(&ladder, Privilege::TagProposals), 4);
    assert!(!requirements.holds(&ladder, 3, Privilege::TagProposals));
    assert!(requirements.holds(&ladder, 4, Privilege::TagProposals));

    // A loosening is **named**, so the operator hears about the mistake they
    // believe they did not make.
    requirements
        .require
        .insert(Privilege::TagProposals.as_str().to_owned(), 1);
    let loosening = requirements.loosening(&ladder);
    assert_eq!(loosening.len(), 1);
    assert_eq!(loosening[0].0, Privilege::TagProposals);
}
