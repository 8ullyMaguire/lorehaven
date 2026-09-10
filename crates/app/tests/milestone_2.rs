//! Milestone 2 acceptance tests (spec §7).
//!
//! Spec §7 acceptance:
//!
//! * an account cannot edit another account's pseud;
//! * pseud linkage is absent from public API responses;
//! * session revocation takes effect;
//! * restricted content cannot be retrieved by bypassing the frontend;
//! * logs contain no passwords, reset tokens, or hidden linkage;
//! * minor-protective messaging defaults are stored from onboarding.
//!
//! These run against the real router over real HTTP request/response objects
//! and a real SQLite file. The only thing simulated is the browser: this file
//! contains a cookie jar and a CSRF-aware client, because those *are* the
//! contract being tested.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::{sessions, Database, DatabaseConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m2-{tag}-{}-{:?}",
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
        "sqlite://{}/lorehaven.sqlite?mode=rwc",
        dir.display()
    ));
    config
}

async fn scratch_database(dir: &Path) -> Database {
    Database::connect(&DatabaseConfig::new(format!(
        "sqlite://{}/lorehaven.sqlite?mode=rwc",
        dir.display()
    )))
    .await
    .expect("connect")
}

/// A client that keeps cookies and echoes the CSRF token, like a browser would.
struct Client {
    app: axum::Router,
    cookies: Vec<(String, String)>,
    /// Set when a request should deliberately omit the CSRF header.
    omit_csrf: bool,
}

impl Client {
    fn new(app: axum::Router) -> Self {
        Self {
            app,
            cookies: Vec::new(),
            omit_csrf: false,
        }
    }

    fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn capture_cookies(&mut self, response: &axum::response::Response) {
        for value in response.headers().get_all(header::SET_COOKIE) {
            let Ok(text) = value.to_str() else { continue };
            let Some((pair, _attributes)) = text.split_once(';') else {
                continue;
            };
            if let Some((name, value)) = pair.split_once('=') {
                let name = name.trim().to_owned();
                let value = value.trim().to_owned();
                self.cookies.retain(|(key, _)| key != &name);
                if !value.is_empty() {
                    self.cookies.push((name, value));
                }
            }
        }
    }

    fn cookie_header(&self) -> String {
        self.cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    async fn request(
        &mut self,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        let cookies = self.cookie_header();
        if !cookies.is_empty() {
            builder = builder.header(header::COOKIE, cookies);
        }

        // A real client echoes the CSRF token on state-changing requests,
        // reading it from the readable cookie.
        let state_changing = !matches!(method, "GET" | "HEAD" | "OPTIONS");
        if state_changing && !self.omit_csrf {
            if let Some(token) = self.cookie("lorehaven_csrf") {
                builder = builder.header("x-csrf-token", token.to_owned());
            }
        }

        let request = match body {
            Some(ref value) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(value).expect("serialise")))
                .expect("request"),
            None => builder.body(Body::empty()).expect("request"),
        };

        let response = self.app.clone().oneshot(request).await.expect("response");

        let status = response.status();
        self.capture_cookies(&response);

        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
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

    async fn patch(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("PATCH", uri, Some(body)).await
    }

    async fn delete(&mut self, uri: &str) -> (StatusCode, Value) {
        self.request("DELETE", uri, None).await
    }
}

struct Harness {
    dir: PathBuf,
    db: Database,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        // The trust-proxy flag is process-wide and set at startup; the default
        // is what the tests want, but reset it so a previous test cannot leak.
        set_trust_proxy(false);

        // Route faults through the same logging the server uses, so a failing
        // test prints the cause rather than only the masked 500.
        let _ = lorehaven_app::logging::init(&lorehaven_app::config::LoggingConfig {
            filter: "error".to_owned(),
            format: lorehaven_app::config::LogFormat::Pretty,
        });

        let dir = scratch_dir(tag);
        let db = scratch_database(&dir).await;
        db.migrate().await.expect("migrate");

        Self { dir, db }
    }

    fn client(&self) -> Client {
        let config = config_for(&self.dir);
        let app = server::build_router(AppState::new(config, self.db.clone()));
        Client::new(app)
    }

    async fn cleanup(self) {
        self.db.close().await;
        let _ = std::fs::remove_dir_all(self.dir);
    }
}

/// A valid registration body.
fn registration(email: &str, handle: &str, password: &str) -> Value {
    json!({
        "email": email,
        "password": password,
        "handle": handle,
        "display_name": handle,
        "age_band": "adult",
    })
}

const GOOD_PASSWORD: &str = "a-long-enough-passphrase";

// ---------------------------------------------------------------------------
// Registration and sessions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn registration_creates_an_account_a_pseud_and_a_session() {
    let harness = Harness::new("register").await;
    let mut client = harness.client();

    let (status, body) = client
        .post(
            "/api/v1/auth/register",
            registration("writer@example.com", "Quill", GOOD_PASSWORD),
        )
        .await;

    assert_eq!(status, StatusCode::CREATED, "body: {body}");
    assert_eq!(body["account"]["email"], "writer@example.com");
    assert_eq!(body["account"]["age_state"], "declared_adult");
    assert_eq!(body["account"]["email_verified"], false);
    assert_eq!(body["capabilities"]["can_read"], true);
    assert_eq!(body["capabilities"]["can_write"], true);

    // Both cookies were issued.
    assert!(
        client.cookie("lorehaven_session").is_some(),
        "session cookie"
    );
    assert!(client.cookie("lorehaven_csrf").is_some(), "csrf cookie");

    // The password is not echoed anywhere.
    assert!(!body.to_string().contains(GOOD_PASSWORD));

    // And the session works.
    let (status, me) = client.get("/api/v1/auth/me").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["pseuds"].as_array().expect("pseuds").len(), 1);
    assert_eq!(me["pseuds"][0]["handle"], "Quill");
    assert!(me["active_pseud_id"].is_string());

    harness.cleanup().await;
}

#[tokio::test]
async fn registration_rejects_a_duplicate_address_and_handle() {
    let harness = Harness::new("dupes").await;
    let mut client = harness.client();

    client
        .post(
            "/api/v1/auth/register",
            registration("a@example.com", "First", GOOD_PASSWORD),
        )
        .await;

    let (status, body) = client
        .post(
            "/api/v1/auth/register",
            registration("A@Example.com", "Second", GOOD_PASSWORD),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "VALIDATION_FAILED");
    assert!(body["error"]["field_errors"]["email"].is_string());

    let (status, body) = client
        .post(
            "/api/v1/auth/register",
            registration("b@example.com", "first", GOOD_PASSWORD),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"]["field_errors"]["handle"].is_string());

    harness.cleanup().await;
}

#[tokio::test]
async fn registration_rejects_a_weak_password_with_a_field_error() {
    let harness = Harness::new("weak").await;
    let mut client = harness.client();

    let (status, body) = client
        .post(
            "/api/v1/auth/register",
            registration("weak@example.com", "Weak", "short"),
        )
        .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"]["field_errors"]["password"]
        .as_str()
        .is_some_and(|m| m.contains("12")));

    harness.cleanup().await;
}

#[tokio::test]
async fn login_accepts_the_right_password_and_refuses_the_wrong_one() {
    let harness = Harness::new("login").await;
    let mut client = harness.client();
    client
        .post(
            "/api/v1/auth/register",
            registration("login@example.com", "Login", GOOD_PASSWORD),
        )
        .await;

    // Wrong password.
    let fresh = harness.client();
    let mut fresh = fresh;
    let (status, body) = fresh
        .post(
            "/api/v1/auth/login",
            json!({ "email": "login@example.com", "password": "not-the-password" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "AUTH_REQUIRED");
    // The message must not reveal which half was wrong.
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(!message.contains("password is wrong"), "{message}");
    assert!(!message.contains("no such"), "{message}");

    // Unknown address: same status and code.
    let (status_unknown, body_unknown) = fresh
        .post(
            "/api/v1/auth/login",
            json!({ "email": "nobody@example.com", "password": GOOD_PASSWORD }),
        )
        .await;
    assert_eq!(
        status_unknown, status,
        "an unknown address must look identical"
    );
    assert_eq!(body_unknown["error"]["code"], body["error"]["code"]);

    // Right password.
    let (status, body) = fresh
        .post(
            "/api/v1/auth/login",
            json!({ "email": "LOGIN@example.com", "password": GOOD_PASSWORD }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(fresh.cookie("lorehaven_session").is_some());

    harness.cleanup().await;
}

#[tokio::test]
async fn email_is_matched_case_insensitively_at_login() {
    let harness = Harness::new("case").await;
    let mut client = harness.client();
    client
        .post(
            "/api/v1/auth/register",
            registration("Mixed@Example.COM", "Mixed", GOOD_PASSWORD),
        )
        .await;

    let mut fresh = harness.client();
    let (status, _) = fresh
        .post(
            "/api/v1/auth/login",
            json!({ "email": "mixed@example.com", "password": GOOD_PASSWORD }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    harness.cleanup().await;
}

#[tokio::test]
async fn logout_revokes_the_session_and_clears_the_cookies() {
    let harness = Harness::new("logout").await;
    let mut client = harness.client();
    let (_, registered) = client
        .post(
            "/api/v1/auth/register",
            registration("out@example.com", "Out", GOOD_PASSWORD),
        )
        .await;
    let account_id: lorehaven_domain::AccountId = registered["account"]["id"]
        .as_str()
        .expect("account id")
        .parse()
        .expect("uuid");

    let (status, _) = client.post("/api/v1/auth/logout", Value::Null).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // The cookie jar now holds nothing, so the next call is anonymous.
    assert!(client.cookie("lorehaven_session").is_none());

    let (status, _) = client.get("/api/v1/auth/me").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // The session row itself is revoked, not merely forgotten by the client.
    let live = sessions::live_sessions_for_account(&harness.db, account_id, &sessions::now())
        .await
        .expect("list sessions");
    assert!(live.is_empty(), "logout must revoke the row");

    harness.cleanup().await;
}

#[tokio::test]
async fn sessions_are_listed_and_individually_revocable() {
    let harness = Harness::new("sessions").await;
    let mut client = harness.client();
    client
        .post(
            "/api/v1/auth/register",
            registration("sess@example.com", "Sess", GOOD_PASSWORD),
        )
        .await;

    // A second sign-in from "another device".
    let mut other = harness.client();
    other
        .post(
            "/api/v1/auth/login",
            json!({ "email": "sess@example.com", "password": GOOD_PASSWORD }),
        )
        .await;

    let (status, list) = client.get("/api/v1/auth/sessions").await;
    assert_eq!(status, StatusCode::OK);
    let items = list.as_array().expect("array");
    assert_eq!(items.len(), 2, "both devices should be listed: {list}");
    assert_eq!(
        items.iter().filter(|s| s["current"] == true).count(),
        1,
        "exactly one session is the current one"
    );

    // Revoke the other one, from this one.
    let other_id = items
        .iter()
        .find(|s| s["current"] == false)
        .and_then(|s| s["id"].as_str())
        .expect("other session id")
        .to_owned();

    let (status, _) = client
        .delete(&format!("/api/v1/auth/sessions/{other_id}"))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // **And it takes effect**: the revoked device is now anonymous.
    let (status, _) = other.get("/api/v1/auth/me").await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "revocation must end the session, not merely hide it"
    );

    // The revoking session is unaffected.
    let (status, _) = client.get("/api/v1/auth/me").await;
    assert_eq!(status, StatusCode::OK);

    harness.cleanup().await;
}

#[tokio::test]
async fn a_session_belonging_to_someone_else_is_not_found() {
    let harness = Harness::new("cross-session").await;

    let mut alice = harness.client();
    alice
        .post(
            "/api/v1/auth/register",
            registration("alice@example.com", "Alice", GOOD_PASSWORD),
        )
        .await;

    let mut mallory = harness.client();
    mallory
        .post(
            "/api/v1/auth/register",
            registration("mallory@example.com", "Mallory", GOOD_PASSWORD),
        )
        .await;

    let (_, alice_list) = alice.get("/api/v1/auth/sessions").await;
    let alice_session = alice_list[0]["id"].as_str().expect("id").to_owned();

    // Mallory tries to log Alice out.
    let (status, body) = mallory
        .delete(&format!("/api/v1/auth/sessions/{alice_session}"))
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a session that is not yours must not be distinguishable from one that does not exist"
    );
    assert_eq!(body["error"]["code"], "NOT_FOUND");

    // Alice is still signed in.
    let (status, _) = alice.get("/api/v1/auth/me").await;
    assert_eq!(status, StatusCode::OK);

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// CSRF
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_state_changing_request_without_a_csrf_token_is_refused() {
    let harness = Harness::new("csrf").await;
    let mut client = harness.client();
    client
        .post(
            "/api/v1/auth/register",
            registration("csrf@example.com", "Csrf", GOOD_PASSWORD),
        )
        .await;

    // Now behave like a cross-site attacker: cookies are sent (the browser
    // does that automatically) but we cannot read them, so no header.
    client.omit_csrf = true;
    let (status, body) = client
        .post("/api/v1/pseuds", json!({ "handle": "Attack" }))
        .await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "ACCESS_DENIED");

    // And the write did not happen.
    let (_, list) = client.get("/api/v1/pseuds").await;
    assert_eq!(list.as_array().expect("array").len(), 1);

    // With the header, the same request succeeds.
    client.omit_csrf = false;
    let (status, _) = client
        .post("/api/v1/pseuds", json!({ "handle": "Legitimate" }))
        .await;
    assert_eq!(status, StatusCode::CREATED);

    harness.cleanup().await;
}

#[tokio::test]
async fn a_wrong_csrf_token_is_refused() {
    let harness = Harness::new("csrf-wrong").await;
    let mut client = harness.client();
    client
        .post(
            "/api/v1/auth/register",
            registration("csrf2@example.com", "Csrf2", GOOD_PASSWORD),
        )
        .await;

    let builder = Request::builder()
        .method("POST")
        .uri("/api/v1/pseuds")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, client.cookie_header())
        .header("x-csrf-token", "not-the-real-token");

    let request = builder
        .body(Body::from(br#"{"handle":"Sneaky"}"#.to_vec()))
        .expect("request");

    let response = client.app.clone().oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    harness.cleanup().await;
}

#[tokio::test]
async fn reads_need_no_csrf_token() {
    let harness = Harness::new("csrf-get").await;
    let mut client = harness.client();
    client
        .post(
            "/api/v1/auth/register",
            registration("get@example.com", "Get", GOOD_PASSWORD),
        )
        .await;

    client.omit_csrf = true;
    let (status, _) = client.get("/api/v1/auth/me").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a GET must not require a CSRF token"
    );

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Pseud isolation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_account_cannot_edit_another_accounts_pseud() {
    let harness = Harness::new("isolation").await;

    let mut alice = harness.client();
    let (_, alice_me) = alice
        .post(
            "/api/v1/auth/register",
            registration("alice2@example.com", "AliceWrites", GOOD_PASSWORD),
        )
        .await;

    // Alice creates a second pseud for herself.
    let (status, created) = alice
        .post(
            "/api/v1/pseuds",
            json!({ "handle": "AliceSecret", "display_name": "Anonymous" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "body: {created}");
    let alice_pseud = created["id"].as_str().expect("id").to_owned();

    let mut mallory = harness.client();
    mallory
        .post(
            "/api/v1/auth/register",
            registration("mallory2@example.com", "Mallory", GOOD_PASSWORD),
        )
        .await;

    // Mallory tries to edit Alice's pseud.
    let (status, body) = mallory
        .patch(
            &format!("/api/v1/pseuds/{alice_pseud}"),
            json!({ "expected_version": 1, "display_name": "Hijacked" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "another account's pseud must be indistinguishable from a missing one"
    );
    assert_eq!(body["error"]["code"], "NOT_FOUND");

    // And it is unchanged.
    let (_, list) = alice.get("/api/v1/pseuds").await;
    let pseud = list
        .as_array()
        .expect("array")
        .iter()
        .find(|p| p["id"] == alice_pseud.as_str())
        .expect("pseud present");
    assert_eq!(pseud["display_name"], "Anonymous");

    // Mallory also cannot activate it.
    let (status, _) = mallory
        .post(
            &format!("/api/v1/pseuds/{alice_pseud}/activate"),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let _ = alice_me;
    harness.cleanup().await;
}

#[tokio::test]
async fn pseud_linkage_is_absent_from_every_response() {
    let harness = Harness::new("linkage").await;
    let mut client = harness.client();
    let (_, registered) = client
        .post(
            "/api/v1/auth/register",
            registration("link@example.com", "Linkage", GOOD_PASSWORD),
        )
        .await;

    let account_id = registered["account"]["id"].as_str().expect("id").to_owned();

    // Every response whose subject is a *pseud* must not mention an account.
    for uri in ["/api/v1/pseuds", "/api/v1/pseuds/Linkage/profile"] {
        let (status, body) = client.get(uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        let text = body.to_string();
        assert!(
            !text.contains(&account_id),
            "{uri} leaked the account id behind a pseud: {text}"
        );
        assert!(
            !text.contains("account_id"),
            "{uri} exposed an account_id field: {text}"
        );
    }

    // `/auth/me` describes *your own* account, so it does carry your account
    // id — but the pseud objects inside it must still not link back.
    let (status, me) = client.get("/api/v1/auth/me").await;
    assert_eq!(status, StatusCode::OK);
    for pseud in me["pseuds"].as_array().expect("pseuds") {
        let object = pseud.as_object().expect("object");
        assert!(
            !object.contains_key("account_id"),
            "a pseud object carried account_id: {pseud}"
        );
        assert_eq!(
            object.len(),
            4,
            "a pseud view should expose exactly id, handle, display_name and bio: {pseud}"
        );
    }

    harness.cleanup().await;
}

#[tokio::test]
async fn activating_a_pseud_is_per_session() {
    let harness = Harness::new("activate").await;

    let mut laptop = harness.client();
    laptop
        .post(
            "/api/v1/auth/register",
            registration("two@example.com", "First", GOOD_PASSWORD),
        )
        .await;
    let (_, second) = laptop
        .post("/api/v1/pseuds", json!({ "handle": "Second" }))
        .await;
    let second_id = second["id"].as_str().expect("id").to_owned();

    // The phone signs in separately.
    let mut phone = harness.client();
    phone
        .post(
            "/api/v1/auth/login",
            json!({ "email": "two@example.com", "password": GOOD_PASSWORD }),
        )
        .await;

    // The laptop switches to the second pseud.
    let (status, _) = laptop
        .post(&format!("/api/v1/pseuds/{second_id}/activate"), Value::Null)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, laptop_me) = laptop.get("/api/v1/auth/me").await;
    assert_eq!(laptop_me["active_pseud_id"], second_id.as_str());

    // The phone is unaffected: the active pseud belongs to the session, not the
    // account, so one device must not switch another.
    let (_, phone_me) = phone.get("/api/v1/auth/me").await;
    assert_ne!(
        phone_me["active_pseud_id"],
        second_id.as_str(),
        "switching pseud on one device must not change another device"
    );

    harness.cleanup().await;
}

#[tokio::test]
async fn editing_a_pseud_with_a_stale_version_is_a_conflict() {
    let harness = Harness::new("revision").await;
    let mut client = harness.client();
    let (_, registered) = client
        .post(
            "/api/v1/auth/register",
            registration("rev@example.com", "Rev", GOOD_PASSWORD),
        )
        .await;
    let (_, pseuds) = client.get("/api/v1/pseuds").await;
    let pseud_id = pseuds[0]["id"].as_str().expect("a pseud id").to_owned();
    let _ = &registered;

    let (status, _) = client
        .patch(
            &format!("/api/v1/pseuds/{pseud_id}"),
            json!({ "expected_version": 1, "display_name": "Renamed" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // A second device still believes the version is 1.
    let (status, body) = client
        .patch(
            &format!("/api/v1/pseuds/{pseud_id}"),
            json!({ "expected_version": 1, "display_name": "Also renamed" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "REVISION_CONFLICT");

    harness.cleanup().await;
}

#[tokio::test]
async fn a_hidden_pseud_is_not_publicly_visible() {
    let harness = Harness::new("hidden").await;
    let mut client = harness.client();
    client
        .post(
            "/api/v1/auth/register",
            registration("hidden@example.com", "Visible", GOOD_PASSWORD),
        )
        .await;

    // Listed by default.
    let (status, profile) = client.get("/api/v1/pseuds/Visible/profile").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(profile["handle"], "Visible");

    // Hide it.
    let (_, pseuds) = client.get("/api/v1/pseuds").await;
    let id = pseuds[0]["id"].as_str().expect("id").to_owned();
    let version = pseuds[0]["version"].as_i64().expect("version");
    let (status, _) = client
        .patch(
            &format!("/api/v1/pseuds/{id}"),
            json!({ "expected_version": version, "discoverability": "hidden" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // Now it is a 404, not a 403: the site does not confirm it exists.
    let (status, _) = client.get("/api/v1/pseuds/Visible/profile").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Privacy and content settings
// ---------------------------------------------------------------------------

#[tokio::test]
async fn onboarding_stores_the_protective_defaults() {
    let harness = Harness::new("privacy-defaults").await;
    let mut client = harness.client();
    client
        .post(
            "/api/v1/auth/register",
            registration("privacy@example.com", "Privacy", GOOD_PASSWORD),
        )
        .await;

    let (status, view) = client.get("/api/v1/settings/privacy").await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(view["account"]["messaging_policy"], "contacts_only");
    assert_eq!(view["account"]["directory_listing"], "listed");
    assert_eq!(view["account"]["instance_affinity"], "true");
    assert_eq!(view["account"]["taste_learning"], "true");

    // Bookmarks are pseud-scoped: they describe a public face, so each pseud
    // carries its own value rather than sharing the account's.
    let pseuds = view["pseuds"].as_object().expect("pseuds");
    assert_eq!(pseuds.len(), 1, "one pseud after registration");
    for (_, values) in pseuds {
        assert_eq!(values["public_bookmarks"], "private");
        assert_eq!(values["public_follows"], "private");
    }

    // The schema is returned so the interface cannot drift from the server.
    let schema = view["schema"].as_array().expect("schema");
    assert!(schema.iter().any(|k| k["key"] == "messaging_policy"));

    harness.cleanup().await;
}

#[tokio::test]
async fn a_minor_gets_stored_protective_defaults_and_no_public_identity() {
    let harness = Harness::new("minor").await;
    let mut client = harness.client();

    let (status, body) = client
        .post(
            "/api/v1/auth/register",
            json!({
                "email": "young@example.com",
                "password": GOOD_PASSWORD,
                "handle": "Young",
                "age_band": "minor",
            }),
        )
        .await;

    assert_eq!(status, StatusCode::CREATED, "body: {body}");

    // No guardian workflow is configured, so the account is created restricted
    // rather than pretending a workflow exists.
    assert_eq!(body["account"]["age_state"], "restricted");
    assert_eq!(body["capabilities"]["can_read"], true);
    assert_eq!(body["capabilities"]["can_write"], false);
    assert_eq!(body["capabilities"]["can_message"], false);
    assert_eq!(body["capabilities"]["can_be_listed"], false);
    assert_eq!(body["capabilities"]["max_rating"], "general");
    assert!(
        body["capabilities"]["restriction_note"].is_string(),
        "the restriction must be explained, not merely enforced"
    );

    // The defaults are *stored*, not computed at render time.
    let (_, privacy) = client.get("/api/v1/settings/privacy").await;
    assert_eq!(privacy["account"]["messaging_policy"], "nobody");
    assert_eq!(privacy["account"]["directory_listing"], "hidden");
    assert_eq!(privacy["account"]["taste_learning"], "false");

    // And the restriction is enforced server-side, not by hiding a button.
    let (status, _) = client
        .post("/api/v1/pseuds", json!({ "handle": "Another" }))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    harness.cleanup().await;
}

#[tokio::test]
async fn privacy_changes_are_validated_before_anything_is_written() {
    let harness = Harness::new("privacy-patch").await;
    let mut client = harness.client();
    client
        .post(
            "/api/v1/auth/register",
            registration("patch@example.com", "Patch", GOOD_PASSWORD),
        )
        .await;

    // One good key and one bad: nothing may change.
    let (status, body) = client
        .patch(
            "/api/v1/settings/privacy",
            json!({ "changes": { "messaging_policy": "anyone", "not_a_key": "x" } }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"]["field_errors"]["not_a_key"].is_string());

    let (_, view) = client.get("/api/v1/settings/privacy").await;
    assert_ne!(
        view["account"]["messaging_policy"], "anyone",
        "a rejected request must not have half-applied"
    );

    // A value outside the permitted set is refused too.
    let (status, _) = client
        .patch(
            "/api/v1/settings/privacy",
            json!({ "changes": { "messaging_policy": "everyone-i-like" } }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // A pseud-scoped key at account scope is refused, and vice versa: a key
    // whose meaning depends on which endpoint wrote it is not a setting.
    let (_, pseuds) = client.get("/api/v1/pseuds").await;
    let pseud_id = pseuds[0]["id"].as_str().expect("id");

    let (status, body) = client
        .patch(
            "/api/v1/settings/privacy",
            json!({ "changes": { "public_bookmarks": "public" } }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"]["field_errors"]["public_bookmarks"].is_string());

    let (status, body) = client
        .patch(
            "/api/v1/settings/privacy",
            json!({ "pseud_id": pseud_id, "changes": { "messaging_policy": "anyone" } }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"]["field_errors"]["messaging_policy"].is_string());

    // A valid change at each scope applies and is reported back.
    let (status, view) = client
        .patch(
            "/api/v1/settings/privacy",
            json!({ "changes": { "messaging_policy": "anyone" } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "body: {view}");
    assert_eq!(view["account"]["messaging_policy"], "anyone");

    let (status, view) = client
        .patch(
            "/api/v1/settings/privacy",
            json!({ "pseud_id": pseud_id, "changes": { "public_bookmarks": "public" } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "body: {view}");
    assert_eq!(view["pseuds"][pseud_id]["public_bookmarks"], "public");

    harness.cleanup().await;
}

#[tokio::test]
async fn content_settings_never_exceed_the_policy_ceiling() {
    let harness = Harness::new("content").await;
    let mut client = harness.client();
    client
        .post(
            "/api/v1/auth/register",
            registration("content@example.com", "Content", GOOD_PASSWORD),
        )
        .await;

    let (status, view) = client.get("/api/v1/settings/content").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view["max_rating"], "general", "starts conservative");
    assert_eq!(view["policy_ceiling"], "explicit");

    // An adult may widen to the ceiling.
    let (status, view) = client
        .patch(
            "/api/v1/settings/content",
            json!({ "expected_version": 1, "max_rating": "explicit" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "body: {view}");
    assert_eq!(view["effective_max_rating"], "explicit");

    // A stale version conflicts.
    let (status, body) = client
        .patch(
            "/api/v1/settings/content",
            json!({ "expected_version": 1, "max_rating": "general" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "REVISION_CONFLICT");

    // An unknown rating is refused.
    let (_, current) = client.get("/api/v1/settings/content").await;
    let version = current["version"].as_i64().expect("version");
    let (status, _) = client
        .patch(
            "/api/v1/settings/content",
            json!({ "expected_version": version, "max_rating": "whatever" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    harness.cleanup().await;
}

#[tokio::test]
async fn a_minor_cannot_widen_past_the_policy_ceiling() {
    let harness = Harness::new("minor-content").await;
    let mut client = harness.client();
    client
        .post(
            "/api/v1/auth/register",
            json!({
                "email": "minorfilter@example.com",
                "password": GOOD_PASSWORD,
                "handle": "MinorFilter",
                "age_band": "minor",
            }),
        )
        .await;

    let (_, view) = client.get("/api/v1/settings/content").await;
    assert_eq!(view["policy_ceiling"], "general");

    let version = view["version"].as_i64().expect("version");
    let (status, view) = client
        .patch(
            "/api/v1/settings/content",
            json!({ "expected_version": version, "max_rating": "explicit" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "the preference is stored");
    assert_eq!(view["max_rating"], "explicit");
    assert_eq!(
        view["effective_max_rating"], "general",
        "but the effective ceiling is the policy's, not the preference's"
    );

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Password reset
// ---------------------------------------------------------------------------

#[tokio::test]
async fn password_reset_is_indistinguishable_for_unknown_addresses() {
    let harness = Harness::new("reset-enum").await;
    let mut client = harness.client();
    client
        .post(
            "/api/v1/auth/register",
            registration("known@example.com", "Known", GOOD_PASSWORD),
        )
        .await;

    let (status_known, body_known) = client
        .post(
            "/api/v1/auth/password-reset",
            json!({ "email": "known@example.com" }),
        )
        .await;
    let (status_unknown, body_unknown) = client
        .post(
            "/api/v1/auth/password-reset",
            json!({ "email": "nobody@example.com" }),
        )
        .await;

    assert_eq!(status_known, StatusCode::OK);
    assert_eq!(status_unknown, StatusCode::OK);
    assert_eq!(body_known["message"], body_unknown["message"]);
    // The development token is the one difference, and only outside production.
    assert!(body_known["development_token"].is_string());
    assert!(body_unknown["development_token"].is_null());

    harness.cleanup().await;
}

#[tokio::test]
async fn a_reset_token_is_single_use_and_ends_every_session() {
    let harness = Harness::new("reset").await;
    let mut client = harness.client();
    client
        .post(
            "/api/v1/auth/register",
            registration("reset@example.com", "Reset", GOOD_PASSWORD),
        )
        .await;

    // A second device is signed in.
    let mut other = harness.client();
    other
        .post(
            "/api/v1/auth/login",
            json!({ "email": "reset@example.com", "password": GOOD_PASSWORD }),
        )
        .await;

    let (_, started) = client
        .post(
            "/api/v1/auth/password-reset",
            json!({ "email": "reset@example.com" }),
        )
        .await;
    let token = started["development_token"]
        .as_str()
        .expect("a development token")
        .to_owned();

    let new_password = "a-brand-new-long-passphrase";
    let (status, body) = client
        .post(
            "/api/v1/auth/password-reset/complete",
            json!({ "token": token, "new_password": new_password }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "body: {body}");

    // Every session is ended: "I reset my password" also means "whoever was in
    // my account is out".
    let (status, _) = client.get("/api/v1/auth/me").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = other.get("/api/v1/auth/me").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // The token cannot be used twice.
    let (status, body) = client
        .post(
            "/api/v1/auth/password-reset/complete",
            json!({ "token": token, "new_password": "yet-another-passphrase-x" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"]["field_errors"]["token"].is_string());

    // The old password no longer works; the new one does.
    let mut fresh = harness.client();
    let (status, _) = fresh
        .post(
            "/api/v1/auth/login",
            json!({ "email": "reset@example.com", "password": GOOD_PASSWORD }),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = fresh
        .post(
            "/api/v1/auth/login",
            json!({ "email": "reset@example.com", "password": new_password }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    harness.cleanup().await;
}

#[tokio::test]
async fn requesting_a_second_reset_invalidates_the_first_link() {
    let harness = Harness::new("reset-supersede").await;
    let mut client = harness.client();
    client
        .post(
            "/api/v1/auth/register",
            registration("supersede@example.com", "Supersede", GOOD_PASSWORD),
        )
        .await;

    let (_, first) = client
        .post(
            "/api/v1/auth/password-reset",
            json!({ "email": "supersede@example.com" }),
        )
        .await;
    let first_token = first["development_token"]
        .as_str()
        .expect("token")
        .to_owned();

    let (_, second) = client
        .post(
            "/api/v1/auth/password-reset",
            json!({ "email": "supersede@example.com" }),
        )
        .await;
    let second_token = second["development_token"]
        .as_str()
        .expect("token")
        .to_owned();

    assert_ne!(first_token, second_token);

    // The first link is dead.
    let (status, _) = client
        .post(
            "/api/v1/auth/password-reset/complete",
            json!({ "token": first_token, "new_password": "first-attempt-passphrase" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // The second works.
    let (status, _) = client
        .post(
            "/api/v1/auth/password-reset/complete",
            json!({ "token": second_token, "new_password": "second-attempt-passphrase" }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Rate limiting
// ---------------------------------------------------------------------------

#[tokio::test]
async fn repeated_login_attempts_are_rate_limited() {
    let harness = Harness::new("ratelimit").await;
    let mut client = harness.client();

    // The auth burst is 10 per account, multiplied by the address multiplier
    // (4 by default) for an anonymous caller, so the address bucket holds 40
    // tokens. Failures cost one each.
    let mut saw_limit = false;
    let mut last_body = Value::Null;
    for _ in 0..200 {
        let (status, body) = client
            .post(
                "/api/v1/auth/login",
                json!({ "email": "nobody@example.com", "password": "guessing-away-1" }),
            )
            .await;
        last_body = body;
        if status == StatusCode::TOO_MANY_REQUESTS {
            saw_limit = true;
            assert_eq!(last_body["error"]["code"], "RATE_LIMITED");
            break;
        }
    }

    assert!(
        saw_limit,
        "a credential-stuffing loop must be rate limited; last response: {last_body}"
    );

    harness.cleanup().await;
}

#[tokio::test]
async fn the_limiter_refuses_a_route_that_declares_no_class() {
    // Two properties, checked separately because they are enforced in
    // different ways.
    //
    // 1. The enforcer itself fails closed when there is no class. Checked by
    //    calling the middleware directly, which is the only way to reach it
    //    without a class: `build_router` never mounts an unclassified route
    //    under /api/v1, and axum does not expose the route table for a
    //    boot-time audit.
    use axum::middleware;
    use lorehaven_app::limiter::{Classified, RouteClass};

    let harness = Harness::new("unclassified").await;
    let config = config_for(&harness.dir);
    let state = AppState::new(config, harness.db.clone());

    let unclassified: axum::Router<AppState> =
        axum::Router::new().route("/bare", axum::routing::get(|| async { "ok" }));
    let app = unclassified
        .layer(middleware::from_fn_with_state(
            state.clone(),
            lorehaven_app::limiter::enforce,
        ))
        .with_state(state.clone());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/bare")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(
        response.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "a request the limiter cannot classify must be refused, not waved through"
    );

    // 2. A route mounted through the class helper *is* limited. This is the
    //    property that actually protects the application, and it is checked
    //    end to end rather than by inspection.
    let limited: axum::Router<AppState> =
        axum::Router::new().route("/ok", axum::routing::get(|| async { "ok" }));
    let app = limited
        .layer(middleware::from_fn_with_state(
            state.clone(),
            lorehaven_app::limiter::enforce,
        ))
        .layer(axum::Extension(Classified(RouteClass::Default)))
        .with_state(state);

    let mut limited_count = 0;
    for _ in 0..600 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/ok")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            limited_count += 1;
            break;
        }
    }
    assert_eq!(limited_count, 1, "a classified route must be rate limited");

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Logging hygiene
// ---------------------------------------------------------------------------

#[tokio::test]
async fn no_response_contains_a_credential() {
    let harness = Harness::new("hygiene").await;
    let mut client = harness.client();

    let (_, registered) = client
        .post(
            "/api/v1/auth/register",
            registration("hygiene@example.com", "Hygiene", GOOD_PASSWORD),
        )
        .await;
    let session_token = client
        .cookie("lorehaven_session")
        .expect("session cookie")
        .to_owned();

    let (_, started) = client
        .post(
            "/api/v1/auth/password-reset",
            json!({ "email": "hygiene@example.com" }),
        )
        .await;
    let reset_token = started["development_token"]
        .as_str()
        .expect("token")
        .to_owned();

    // The session token must never appear in a body: it is the credential.
    assert!(!registered.to_string().contains(&session_token));
    // The reset token is returned exactly once, in its own response, and never
    // alongside the account identifier.
    let account_id = registered["account"]["id"].as_str().expect("account id");
    assert!(
        !started.to_string().contains(account_id),
        "the reset response must not identify the account it was issued for"
    );
    assert!(!registered.to_string().contains(&reset_token));

    // And no response carries a password hash.
    let (_, me) = client.get("/api/v1/auth/me").await;
    assert!(!me.to_string().contains("argon2"));
    assert!(!me.to_string().contains("password"));

    harness.cleanup().await;
}
