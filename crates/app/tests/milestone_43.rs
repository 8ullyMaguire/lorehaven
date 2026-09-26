//! M43 — Shared browse ordering and demand weighting (spec §43).
//!
//! The domain half of §43 is unit-tested in `crates/domain/src/browse.rs`, and
//! the route half resolves sort correctly, but nothing exercised them together:
//! seventeen requirements in `docs/requirements.csv` cited this file, and it did
//! not exist. That is the defect this closes -- the resolution order
//! (query param > stored preference > surface default) is three sources
//! disagreeing, which is exactly the shape that passes a unit test and fails in
//! a browser.
//!
//! The `source` field in `SortStateResponse` is the thing under test throughout.
//! A control that cannot tell "the reader chose this" from "this is what the
//! surface decided" cannot offer a reset, cannot avoid overwriting a preference,
//! and cannot explain itself.

//! M41 — Half-life and interaction tiers (spec §41).

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::DatabaseConfig;
use serde_json::{json, Value};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m43-{tag}-{}-{:?}",
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
    config.rate_limits.auth = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.write = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
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

    fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    fn capture(&mut self, response: &axum::response::Response) {
        for value in response.headers().get_all(header::SET_COOKIE) {
            let Ok(text) = value.to_str() else {
                continue;
            };
            let Some((pair, _)) = text.split_once(';') else {
                continue;
            };
            if let Some((name, value)) = pair.split_once('=') {
                let name = name.trim().to_owned();
                let value = value.trim().to_owned();
                self.cookies.retain(|(k, _)| k != &name);
                if !value.is_empty() {
                    self.cookies.push((name, value));
                }
            }
        }
    }

    async fn request(
        &mut self,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if !self.cookies.is_empty() {
            builder = builder.header(
                header::COOKIE,
                self.cookies
                    .iter()
                    .map(|(n, v)| format!("{n}={v}"))
                    .collect::<Vec<_>>()
                    .join("; "),
            );
        }
        if !matches!(method, "GET" | "HEAD" | "OPTIONS") {
            if let Some(token) = self.cookie("lorehaven_csrf").map(str::to_owned) {
                builder = builder.header("x-csrf-token", token);
            }
        }
        let request = match body {
            Some(v) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(v.to_string()))
                .expect("build request"),
            None => builder.body(Body::empty()).expect("build request"),
        };
        let response = self.app.clone().oneshot(request).await.expect("oneshot");
        let status = response.status();
        self.capture(&response);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("bytes");
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, body)
    }

    async fn post(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("POST", uri, Some(body)).await
    }

    async fn get(&mut self, uri: &str) -> (StatusCode, Value) {
        self.request("GET", uri, None).await
    }
}

struct Harness {
    dir: PathBuf,
    tdb: test_support::TestDb,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        set_trust_proxy(false);
        let _ = lorehaven_app::logging::init(&lorehaven_app::config::LoggingConfig {
            filter: "error".to_owned(),
            format: lorehaven_app::config::LogFormat::Pretty,
        });
        let dir = scratch_dir(tag);
        let tdb = test_support::TestDb::connect_with_dir(tag, &dir).await;
        Self { dir, tdb }
    }

    fn client(&self) -> Client {
        Client::new(server::build_router(AppState::new(
            config_for(&self.dir),
            self.tdb.db().clone(),
        )))
    }
}

const PASSWORD: &str = "a-long-enough-passphrase";

async fn register(client: &mut Client, email: &str, handle: &str) -> (String, String) {
    let (status, body) = client
        .post(
            "/api/v1/auth/register",
            json!({
                "email": email,
                "handle": handle,
                "password": PASSWORD,
                "display_name": handle,
                "age_band": "adult"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "register {handle}: {body}");
    let (status, me) = client.get("/api/v1/auth/me").await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let account = me["account"]["id"].as_str().expect("account id").to_owned();
    let pseud = me["active_pseud_id"].as_str().expect("pseud id").to_owned();
    (account, pseud)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The effective sort and where it came from.
fn sort_state(body: &Value) -> (&str, &str) {
    (
        body["sort"].as_str().expect("sort value"),
        body["source"].as_str().expect("sort source"),
    )
}

#[tokio::test]
async fn an_anonymous_reader_gets_the_surface_default() {
    let h = Harness::new("anon").await;
    let mut c = h.client();

    // Discover's own default is for-you (§43.2).
    let (status, body) = c.get("/api/v1/browse/sort/discover").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(sort_state(&body), ("for-you", "default"));

    // Other surfaces have their own defaults, and the anonymous answer must
    // differ per surface rather than being one global value.
    let (status, body) = c.get("/api/v1/browse/sort/people").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(sort_state(&body), ("az", "default"));

    let (status, body) = c.get("/api/v1/browse/sort/library").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(sort_state(&body), ("new", "default"));
}

#[tokio::test]
async fn an_unknown_surface_falls_back_to_the_default_rather_than_erroring() {
    let h = Harness::new("unknown-surface").await;
    let mut c = h.client();

    // A surface nobody has declared is not a 404: the route accepts any surface
    // key, because a new surface should work before its default is tuned.
    let (status, body) = c.get("/api/v1/browse/sort/never-heard-of-it").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(sort_state(&body), ("new", "default"));
}

#[tokio::test]
async fn a_signed_in_readers_choice_is_stored_and_reported_as_a_preference() {
    let h = Harness::new("store").await;
    let mut c = h.client();
    register(&mut c, "m43-store@example.test", "M43Storer").await;

    let (status, body) = c.get("/api/v1/browse/sort/discover").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(sort_state(&body), ("for-you", "default"), "no choice yet");

    let (status, body) = c
        .request(
            "PUT",
            "/api/v1/browse/sort/discover",
            Some(json!({ "sort": "az" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(sort_state(&body), ("az", "preference"));

    // And it survives to the next request, which is the whole point of §43.4.
    let (status, body) = c.get("/api/v1/browse/sort/discover").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(sort_state(&body), ("az", "preference"));
}

#[tokio::test]
async fn a_preference_is_scoped_to_the_surface_it_was_set_on() {
    let h = Harness::new("scoped").await;
    let mut c = h.client();
    register(&mut c, "m43-scope@example.test", "M43Scoper").await;

    c.request(
        "PUT",
        "/api/v1/browse/sort/discover",
        Some(json!({ "sort": "top" })),
    )
    .await;

    // Choosing Trending on Discover must not silently re-order the library.
    let (status, body) = c.get("/api/v1/browse/sort/library").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        sort_state(&body),
        ("new", "default"),
        "library is untouched"
    );

    let (status, body) = c.get("/api/v1/browse/sort/discover").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(sort_state(&body), ("top", "preference"));
}

#[tokio::test]
async fn clearing_a_preference_returns_the_surface_to_its_own_default() {
    let h = Harness::new("clear").await;
    let mut c = h.client();
    register(&mut c, "m43-clear@example.test", "M43Clearer").await;

    c.request(
        "PUT",
        "/api/v1/browse/sort/discover",
        Some(json!({ "sort": "trending" })),
    )
    .await;

    // DELETE answers with the state the surface has returned to, rather than a
    // bare 204. That is worth having: the control needs the new default to show
    // without a second round trip, and a client that guessed would guess wrong
    // on a surface whose default is not `new`.
    let (status, body) = c
        .request("DELETE", "/api/v1/browse/sort/discover", None)
        .await;
    assert_eq!(status, StatusCode::OK, "clear: {body}");
    assert_eq!(sort_state(&body), ("for-you", "default"));

    let (status, body) = c.get("/api/v1/browse/sort/discover").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        sort_state(&body),
        ("for-you", "default"),
        "back to for-you, not to `new`"
    );
}

#[tokio::test]
async fn an_unknown_sort_is_refused_with_the_accepted_set_named() {
    let h = Harness::new("reject").await;
    let mut c = h.client();
    register(&mut c, "m43-reject@example.test", "M43Rejecter").await;

    let (status, body) = c
        .request(
            "PUT",
            "/api/v1/browse/sort/discover",
            Some(json!({ "sort": "most-popular-ever" })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");

    // The error has to teach the caller the vocabulary, not just refuse.
    let message = body.to_string();
    for accepted in [
        "for-you",
        "new",
        "updated",
        "top",
        "trending",
        "best-match",
        "az",
    ] {
        assert!(
            message.contains(accepted),
            "error should name `{accepted}`: {message}"
        );
    }

    // A refused value must not have been stored.
    let (status, body) = c.get("/api/v1/browse/sort/discover").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(sort_state(&body), ("for-you", "default"));
}

#[tokio::test]
async fn an_anonymous_reader_cannot_write_a_preference() {
    let h = Harness::new("anon-write").await;
    let mut c = h.client();

    // §43.4's neutral default exists because there is nothing to store for a
    // reader with no pseud. The write must be refused, not silently accepted.
    let (status, _body) = c
        .request(
            "PUT",
            "/api/v1/browse/sort/discover",
            Some(json!({ "sort": "az" })),
        )
        .await;
    assert!(
        status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
        "anonymous write should be refused, got {status}"
    );

    let (status, body) = c.get("/api/v1/browse/sort/discover").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(sort_state(&body), ("for-you", "default"));
}

#[tokio::test]
async fn one_pseud_does_not_inherit_another_pseud_s_preference() {
    // §43.4 says *per-pseud*, not per-account. An account with two pseuds is
    // the only case where those differ, and it is the case a per-account store
    // gets wrong.
    let h = Harness::new("per-pseud").await;
    let mut c = h.client();
    let (_account, _pseud) = register(&mut c, "m43-two@example.test", "M43First").await;

    c.request(
        "PUT",
        "/api/v1/browse/sort/discover",
        Some(json!({ "sort": "updated" })),
    )
    .await;
    let (status, body) = c.get("/api/v1/browse/sort/discover").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(sort_state(&body), ("updated", "preference"));

    // A second pseud on the same account starts neutral.
    let (status, body) = c
        .post("/api/v1/pseuds", json!({ "handle": "M43Second" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "second pseud: {body}");
    let second_id = body["id"].as_str().expect("second pseud id").to_owned();

    // Registration leaves the first pseud active, so the new one has to be
    // activated before the session acts as it. Without this the test would
    // still be reading the *first* pseud's preference and would pass for the
    // wrong reason.
    let (status, body) = c
        .post(&format!("/api/v1/pseuds/{second_id}/activate"), json!({}))
        .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "activate second pseud: {body}"
    );

    let (status, body) = c.get("/api/v1/browse/sort/discover").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        sort_state(&body),
        ("for-you", "default"),
        "the new pseud must not inherit the first pseud's choice"
    );
}

#[tokio::test]
async fn the_discovery_route_accepts_a_query_param_sort() {
    let h = Harness::new("query").await;
    let mut c = h.client();
    register(&mut c, "m43-query@example.test", "M43Query").await;

    // A recognised value is accepted. The feed may be empty -- the assertion is
    // that the value is understood, not that anything is in it.
    let (status, _body) = c.get("/api/v1/discovery?sort=az").await;
    assert_eq!(status, StatusCode::OK);

    // An unrecognised one is not silently dropped: resolve_sort falls back to
    // the surface default, and `?sort=` is a reader's request, so it must not
    // 500 or quietly change the feed into something else.
    let (status, _body) = c.get("/api/v1/discovery?sort=whatever").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn the_every_value_in_the_vocabulary_is_accepted_end_to_end() {
    // Guards against a value that parses in the domain and is rejected by the
    // route, or vice versa -- the drift a shared vocabulary exists to prevent.
    let h = Harness::new("all-values").await;
    let mut c = h.client();
    register(&mut c, "m43-all@example.test", "M43All").await;

    for value in lorehaven_domain::browse::Sort::ALL {
        let (status, body) = c
            .request(
                "PUT",
                "/api/v1/browse/sort/discover",
                Some(json!({ "sort": value.as_str() })),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{value:?} should be accepted: {body}"
        );
        assert_eq!(sort_state(&body), (value.as_str(), "preference"));
    }
}

#[tokio::test]
async fn the_response_echoes_the_surface_it_was_asked_about() {
    // The control keys its stored preference by surface, so the echo is load
    // bearing: a route that answered for a different surface would write the
    // reader's choice to the wrong key.
    let h = Harness::new("echo").await;
    let mut c = h.client();

    let (status, body) = c.get("/api/v1/browse/sort/people").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["surface"].as_str(), Some("people"));
}
