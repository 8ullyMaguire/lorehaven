//! M45-27 — gift exchanges, through the real router.
//!
//! The matcher lives in `darkhold-domain` and the record in `darkhold-db`; both have their
//! own tests. What this file pins is what only the route can get wrong:
//!
//! - **the public summary is public, and the rest is not** — a missing event is `404`, an
//!   anonymous write is `401`;
//! - **joining twice is the same participation**, so a client may call it on every load;
//! - **terms before joining are refused on the field**, not stored and ignored;
//! - **only the organiser may match or reveal**, and a second match names the first;
//! - **the recipient is secret until reveal** — `mine` answers `recipient: null` with the
//!   reason, rather than refusing as though the caller did something wrong;
//! - **an impossible pairing is a `422` naming the participants**, with nothing written.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use darkhold_app::config::Config;
use darkhold_app::server;
use darkhold_app::state::AppState;
use serde_json::{json, Value};
use test_support::TestDb;
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery staple 7";

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "darkhold-m45-27-{tag}-{}-{:?}",
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
    config
}

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

    async fn request(&mut self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        if !self.cookies.is_empty() {
            let jar = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            builder = builder.header(header::COOKIE, jar);
            if let Some((_, token)) = self.cookies.iter().find(|(k, _)| k == "darkhold_csrf") {
                builder = builder.header("x-csrf-token", token);
            }
        }
        let body = match body {
            Some(v) => Body::from(v.to_string()),
            None => Body::empty(),
        };
        let response = self
            .app
            .clone()
            .oneshot(builder.body(body).expect("request"))
            .await
            .expect("response");
        let status = response.status();
        self.capture(&response);
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .expect("body");
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        };
        (status, value)
    }

    async fn get(&mut self, uri: &str) -> (StatusCode, Value) {
        self.request("GET", uri, None).await
    }

    async fn post(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("POST", uri, Some(body)).await
    }

    async fn put(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("PUT", uri, Some(body)).await
    }

    /// Register, and return `(account id, pseud id)`. The pseud is created by
    /// registration but is NOT this session's active one until it is activated, which
    /// is what the "no pseud" door below relies on.
    async fn register(&mut self, email: &str, handle: &str) -> (String, String) {
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
        let account = me["account"]["id"].as_str().expect("account id").to_owned();
        let pseud = me["pseuds"][0]["id"].as_str().expect("pseud id").to_owned();
        (account, pseud)
    }

    async fn activate(&mut self, pseud: &str) {
        let (status, body) = self
            .post(&format!("/api/v1/pseuds/{pseud}/activate"), json!({}))
            .await;
        assert!(
            status == StatusCode::NO_CONTENT || status == StatusCode::OK,
            "activate {pseud}: {status} {body}"
        );
    }
}

struct Harness {
    _dir: PathBuf,
    tdb: TestDb,
    config: Config,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
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


/// An event created by `client`, which must hold an activated pseud.
async fn make_event(client: &mut Client, name: &str) -> String {
    let (status, body) = client
        .post(
            "/api/v1/events",
            json!({ "name": name, "document": "a gift exchange" }),
        )
        .await;
    // `post_event` answers 200 with the id, not 201 — measured, not assumed.
    assert_eq!(status, StatusCode::OK, "create event: {body}");
    body["id"].as_str().expect("event id").to_owned()
}

/// A registered, activated participant.
async fn participant(h: &Harness, email: &str, handle: &str) -> Client {
    let mut client = h.client();
    let (_account, pseud) = client.register(email, handle).await;
    client.activate(&pseud).await;
    client
}

#[tokio::test]
async fn the_public_summary_is_public_and_a_missing_event_is_404() {
    let h = Harness::new("public").await;
    let mut anon = h.client();
    let (status, body) = anon.get("/api/v1/exchanges/no-such-event").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"]["code"], json!("NOT_FOUND"));

    let mut organiser = participant(&h, "organiser@example.invalid", "organiser").await;
    let event = make_event(&mut organiser, "Yuletide").await;
    let (status, body) = anon.get(&format!("/api/v1/exchanges/{event}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["participants"], json!(0), "{body}");
    assert_eq!(body["matched"], json!(0));
    assert_eq!(body["revealed"], json!(false));
}

#[tokio::test]
async fn every_other_door_needs_a_session() {
    let h = Harness::new("anon").await;
    let mut anon = h.client();
    for (method, uri) in [
        ("POST", "/api/v1/exchanges/x/join"),
        ("PUT", "/api/v1/exchanges/x/terms"),
        ("POST", "/api/v1/exchanges/x/match"),
        ("POST", "/api/v1/exchanges/x/reveal"),
        ("GET", "/api/v1/exchanges/x/mine"),
    ] {
        let (status, _) = anon.request(method, uri, Some(json!({}))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri}");
    }
}

#[tokio::test]
async fn joining_twice_is_the_same_participation() {
    let h = Harness::new("join").await;
    let mut organiser = participant(&h, "org@example.invalid", "org").await;
    let event = make_event(&mut organiser, "Yuletide").await;

    let (status, first) = organiser.post(&format!("/api/v1/exchanges/{event}/join"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["new"], json!(true));
    let (status, second) = organiser.post(&format!("/api/v1/exchanges/{event}/join"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["new"], json!(false), "the second join is not new");

    let (status, body) = organiser.get(&format!("/api/v1/exchanges/{event}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["participants"], json!(1), "{body}");
}

#[tokio::test]
async fn terms_before_joining_are_refused_on_the_field() {
    let h = Harness::new("terms").await;
    let mut organiser = participant(&h, "org2@example.invalid", "org2").await;
    let event = make_event(&mut organiser, "Yuletide").await;

    let (status, body) = organiser
        .put(
            &format!("/api/v1/exchanges/{event}/terms"),
            json!({ "offers": ["dragon"], "wants": ["space"] }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(
        body["error"]["field_errors"]["event_id"].is_string(),
        "the refusal names the reason: {body}"
    );
}

#[tokio::test]
async fn a_self_exclusion_is_refused_on_the_field() {
    let h = Harness::new("selfex").await;
    let mut organiser = participant(&h, "org3@example.invalid", "org3").await;
    let event = make_event(&mut organiser, "Yuletide").await;
    organiser.post(&format!("/api/v1/exchanges/{event}/join"), json!({})).await;
    let (_status, me) = organiser.get("/api/v1/auth/me").await;
    let pseud = me["active_pseud_id"]
        .as_str()
        .or_else(|| me["pseuds"][0]["id"].as_str())
        .expect("pseud id")
        .to_owned();

    let (status, body) = organiser
        .put(
            &format!("/api/v1/exchanges/{event}/terms"),
            json!({ "excludes": [pseud] }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(
        body["error"]["field_errors"]["excludes"].is_string(),
        "{body}"
    );
}

#[tokio::test]
async fn only_the_organiser_matches_and_a_second_match_names_the_first() {
    let h = Harness::new("match").await;
    let mut organiser = participant(&h, "host@example.invalid", "host").await;
    let event = make_event(&mut organiser, "Yuletide").await;
    organiser.post(&format!("/api/v1/exchanges/{event}/join"), json!({})).await;

    let mut other = participant(&h, "guest@example.invalid", "guest").await;
    other.post(&format!("/api/v1/exchanges/{event}/join"), json!({})).await;

    let (status, body) = other.post(&format!("/api/v1/exchanges/{event}/match"), json!({})).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a guest cannot match: {body}");

    let (status, body) = organiser.post(&format!("/api/v1/exchanges/{event}/match"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["matched"], json!(2), "{body}");
    let assigned_at = body["assignments"][0]["assigned_at"]
        .as_str()
        .expect("assigned_at")
        .to_owned();

    let (status, body) = organiser.post(&format!("/api/v1/exchanges/{event}/match"), json!({})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let reason = body["error"]["field_errors"]["event_id"]
        .as_str()
        .expect("field error");
    assert!(reason.contains(&assigned_at), "the refusal names the first: {reason}");
}

#[tokio::test]
async fn the_recipient_is_secret_until_the_organiser_reveals() {
    let h = Harness::new("reveal").await;
    let mut organiser = participant(&h, "host2@example.invalid", "host2").await;
    let event = make_event(&mut organiser, "Yuletide").await;
    organiser.post(&format!("/api/v1/exchanges/{event}/join"), json!({})).await;
    let mut guest = participant(&h, "guest2@example.invalid", "guest2").await;
    guest.post(&format!("/api/v1/exchanges/{event}/join"), json!({})).await;
    organiser.post(&format!("/api/v1/exchanges/{event}/match"), json!({})).await;

    let (status, body) = guest.get(&format!("/api/v1/exchanges/{event}/mine")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["recipient"], Value::Null, "secret before reveal: {body}");
    assert_eq!(body["revealed"], json!(false));
    assert!(body["reason"].as_str().is_some_and(|r| r.contains("secret")));

    let (status, body) = guest.post(&format!("/api/v1/exchanges/{event}/reveal"), json!({})).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "only the organiser reveals: {body}");

    let (status, body) = organiser.post(&format!("/api/v1/exchanges/{event}/reveal"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["revealed"], json!(true));

    let (status, body) = guest.get(&format!("/api/v1/exchanges/{event}/mine")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["recipient"].is_string(),
        "after reveal the giver is told: {body}"
    );
    assert_eq!(body["revealed"], json!(true));
}

#[tokio::test]
async fn an_impossible_pairing_answers_422_naming_the_participants() {
    let h = Harness::new("impossible").await;
    let mut organiser = participant(&h, "host3@example.invalid", "host3").await;
    let event = make_event(&mut organiser, "Yuletide").await;
    organiser.post(&format!("/api/v1/exchanges/{event}/join"), json!({})).await;
    let mut guest = participant(&h, "guest3@example.invalid", "guest3").await;
    guest.post(&format!("/api/v1/exchanges/{event}/join"), json!({})).await;

    // The guest refuses the organiser, and with two participants that leaves no pairing.
    let (_status, me) = organiser.get("/api/v1/auth/me").await;
    let host_pseud = me["active_pseud_id"]
        .as_str()
        .or_else(|| me["pseuds"][0]["id"].as_str())
        .expect("pseud")
        .to_owned();
    let (status, body) = guest
        .put(
            &format!("/api/v1/exchanges/{event}/terms"),
            json!({ "excludes": [host_pseud] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = organiser.post(&format!("/api/v1/exchanges/{event}/match"), json!({})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let reason = body["error"]["field_errors"]["participants"]
        .as_str()
        .expect("field error");
    assert!(reason.contains("unmatched"), "{reason}");

    let (status, body) = organiser.get(&format!("/api/v1/exchanges/{event}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["matched"], json!(0), "nothing was written: {body}");
}

#[tokio::test]
async fn the_module_is_registered_without_a_nav_entry() {
    let h = Harness::new("module").await;
    let mut anon = h.client();
    let (status, body) = anon.get("/api/v1/modules").await;
    assert_eq!(status, StatusCode::OK);
    let row = body["modules"]
        .as_array()
        .expect("modules")
        .iter()
        .find(|m| m["id"] == json!("exchanges"))
        .unwrap_or_else(|| panic!("exchanges is not in the registry payload"));
    assert_eq!(row["enabled"], json!(true));
    assert_eq!(row["toggle"], json!("operator"));
    assert_eq!(row["nav"], json!([]), "a seasonal surface has no sidebar entry");
}
