//! M39 — Resource Directory (spec §39).
//!
//! These tests prove, at the HTTP layer:
//!
//! - Lists: create, slug lookup, instance lists sort first.
//! - Visibility (§39.3): anonymous sees approved only; the submitter sees
//!   their own pending; the operator sees everything.
//! - Submission validation: URL rules (absolute http, no localhost/private
//!   hosts, no userinfo), title/description lengths, tag normalization.
//! - Voting (§39.4): up/down/toggle/flip; weighted score; my_vote is the
//!   direction only, never the weight; a TL4 vote outranks a TL0 vote.
//! - Moderation: approve, remove (hard delete, votes stay for audit).
//! - Config: extra categories and the weighting mode are TOML-tunable.

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
        "lorehaven-m39-{tag}-{}-{:?}",
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
                .body(Body::from(serde_json::to_vec(&v).expect("serialise")))
                .expect("request"),
            None => builder.body(Body::empty()).expect("request"),
        };
        let response = self.app.clone().oneshot(request).await.expect("response");
        let status = response.status();
        self.capture(&response);
        let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024)
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
    async fn delete(&mut self, uri: &str) -> (StatusCode, Value) {
        self.request("DELETE", uri, None).await
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

/// Make an account the instance operator (TL6) straight in the DB.
async fn make_operator(tdb: &test_support::TestDb, account_id: &str) {
    lorehaven_db::directory::set_account_trust_for_tests(tdb.db(), account_id, 6).await;
}

/// Submit a valid external entry and return its id.
async fn submit_entry(client: &mut Client, title: &str, url: &str) -> String {
    let (status, body) = client
        .post(
            "/api/v1/directory/entries",
            json!({
                "list": "external-sites",
                "kind": "external",
                "category": "fanfiction_archive",
                "title": title,
                "url": url,
                "description": "A test entry.",
                "tags": ["archive", "multi-fandom"]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "submit {title}: {body}");
    body["entry"]["id"].as_str().expect("entry id").to_owned()
}

#[tokio::test]
async fn lists_crud_and_instance_lists_sort_first() {
    let harness = Harness::new("lists").await;
    let mut operator = harness.client();
    let (_op_account, _op_pseud) = register(&mut operator, "op@example.com", "operator").await;
    make_operator(&harness.tdb, &_op_account).await;

    // Seed the two instance lists (operator-only).
    let (status, body) = operator
        .post(
            "/api/v1/directory/lists",
            json!({
                "slug": "external-sites",
                "title": "External sites",
                "description": "Fanfiction archives and communities elsewhere.",
                "kind": "external"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let (status, body) = operator
        .post(
            "/api/v1/directory/lists",
            json!({
                "slug": "reading-lists",
                "title": "Reading lists",
                "description": "Curated lists of works on this instance.",
                "kind": "internal"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // A regular account cannot create lists.
    let mut member = harness.client();
    let (_m_account, _m_pseud) = register(&mut member, "member@example.com", "memberer").await;
    let (status, body) = member
        .post(
            "/api/v1/directory/lists",
            json!({
                "slug": "member-list",
                "title": "Member list",
                "description": "",
                "kind": "external"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // Anonymous can browse the lists.
    let mut anon = harness.client();
    let (status, body) = anon.get("/api/v1/directory/lists").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let lists = body["items"].as_array().expect("items");
    assert_eq!(lists.len(), 2, "{body}");
    assert_eq!(lists[0]["slug"], "external-sites");

    // Slug lookup.
    let (status, body) = anon.get("/api/v1/directory/lists/external-sites").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["list"]["slug"], "external-sites");

    let (status, _body) = anon.get("/api/v1/directory/lists/nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn submission_validates_url_title_description_tags() {
    let harness = Harness::new("validation").await;
    let mut operator = harness.client();
    let (_op_account, _op_pseud) = register(&mut operator, "op@example.com", "operator").await;
    make_operator(&harness.tdb, &_op_account).await;
    seed_list(&mut operator).await;

    let mut member = harness.client();
    let (_m_account, _m_pseud) = register(&mut member, "member@example.com", "memberer").await;

    // Bad URLs, each naming its refusal reason.
    for (url, reason) in [
        ("ftp://example.com", "url_scheme_not_http: ftp"),
        ("http://localhost/x", "url_host_is_localhost"),
        ("http://127.0.0.1/x", "url_host_is_private_address"),
        ("http://user@127.0.0.1/", "url_userinfo_refused"),
        ("not a url", "url_not_absolute"),
    ] {
        let (status, body) = member
            .post(
                "/api/v1/directory/entries",
                json!({
                    "list": "external-sites",
                    "kind": "external",
                    "category": "fanfiction_archive",
                    "title": "X",
                    "url": url,
                    "description": ""
                }),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{url}: {body}");
        assert_eq!(body["error"]["field_errors"]["reason"], reason, "{url}: {body}");
    }

    // Title rules.
    let (status, body) = member
        .post(
            "/api/v1/directory/entries",
            json!({
                "list": "external-sites",
                "kind": "external",
                "category": "fanfiction_archive",
                "title": "   ",
                "url": "https://example.com",
                "description": ""
            }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["field_errors"]["reason"], "title_empty");

    let (status, body) = member
        .post(
            "/api/v1/directory/entries",
            json!({
                "list": "external-sites",
                "kind": "external",
                "category": "fanfiction_archive",
                "title": "x".repeat(121),
                "url": "https://example.com",
                "description": ""
            }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["field_errors"]["reason"], "title_too_long");

    // Description rule.
    let (status, body) = member
        .post(
            "/api/v1/directory/entries",
            json!({
                "list": "external-sites",
                "kind": "external",
                "category": "fanfiction_archive",
                "title": "Fine",
                "url": "https://example.com",
                "description": "y".repeat(501)
            }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["field_errors"]["reason"], "description_too_long");

    // Good submission passes and normalizes tags.
    let (status, body) = member
        .post(
            "/api/v1/directory/entries",
            json!({
                "list": "external-sites",
                "kind": "external",
                "category": "fanfiction_archive",
                "title": "A good archive",
                "url": "https://archive.example.org",
                "description": "Fine.",
                "tags": ["Archive", " archive ", "", "too-long-tag-aaaaaaaaaaaaaaaaaaaaaaaaa"]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["entry"]["tags"], json!(["archive"]));
}

#[tokio::test]
async fn visibility_approved_own_pending_operator() {
    let harness = Harness::new("visibility").await;
    let mut operator = harness.client();
    let (op_account, _op_pseud) = register(&mut operator, "op@example.com", "operator").await;
    make_operator(&harness.tdb, &op_account).await;
    seed_list(&mut operator).await;

    // Member submits; entry is pending.
    let mut member = harness.client();
    let (_m_account, _m_pseud) = register(&mut member, "member@example.com", "memberer").await;
    let entry_id = submit_entry(&mut member, "Pending one", "https://one.example.org").await;

    // Anonymous sees nothing.
    let mut anon = harness.client();
    let (status, body) = anon.get("/api/v1/directory/entries?list=external-sites").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"].as_array().expect("items").len(), 0, "{body}");

    // The submitter sees their own pending entry.
    let (status, body) = member
        .get("/api/v1/directory/entries?list=external-sites")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"].as_array().expect("items").len(), 1, "{body}");

    // Another member sees nothing.
    let mut other = harness.client();
    let (_o_account, _o_pseud) = register(&mut other, "other@example.com", "other").await;
    let (status, body) = other
        .get("/api/v1/directory/entries?list=external-sites")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"].as_array().expect("items").len(), 0, "{body}");

    // The operator sees pending entries in the moderation queue.
    let (status, body) = operator.get("/api/v1/directory/moderation").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let pending = body["items"].as_array().expect("items");
    assert_eq!(pending.len(), 1, "{body}");
    assert_eq!(pending[0]["id"], entry_id);

    // Approve it.
    let (status, body) = operator
        .post(&format!("/api/v1/directory/entries/{entry_id}/approve"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Now anonymous sees it; the queue is empty.
    let (status, body) = anon.get("/api/v1/directory/entries?list=external-sites").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"].as_array().expect("items").len(), 1, "{body}");
    let (_status, body) = operator.get("/api/v1/directory/moderation").await;
    assert_eq!(body["items"].as_array().expect("items").len(), 0, "{body}");
}

#[tokio::test]
async fn voting_up_down_toggle_flip_and_weighted_score() {
    let harness = Harness::new("voting").await;
    let mut operator = harness.client();
    let (op_account, _op_pseud) = register(&mut operator, "op@example.com", "operator").await;
    make_operator(&harness.tdb, &op_account).await;
    seed_list(&mut operator).await;

    let mut member = harness.client();
    let (_m_account, _m_pseud) = register(&mut member, "member@example.com", "memberer").await;
    let entry_id = submit_entry(&mut member, "Votable", "https://votable.example.org").await;

    // Approve so others can vote.
    let (status, _b) = operator
        .post(&format!("/api/v1/directory/entries/{entry_id}/approve"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);

    // Another member upvotes.
    let mut voter = harness.client();
    let (_v_account, _v_pseud) = register(&mut voter, "voter@example.com", "voter").await;
    // A fresh member votes at TL0 (0.5) with no taste profile (floor 0.75):
    // weight 0.375 — the default trust_and_taste mode.
    let (status, body) = voter
        .post(&format!("/api/v1/directory/entries/{entry_id}/vote"), json!({"value": 1}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["score"], json!(0.375), "{body}");
    assert_eq!(body["my_vote"], json!(1), "{body}");

    // Toggle off by repeating.
    let (status, body) = voter
        .post(&format!("/api/v1/directory/entries/{entry_id}/vote"), json!({"value": 1}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["score"], json!(0.0), "{body}");
    assert_eq!(body["my_vote"], Value::Null, "{body}");

    // Downvote instead.
    let (status, body) = voter
        .post(&format!("/api/v1/directory/entries/{entry_id}/vote"), json!({"value": -1}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["score"], json!(-0.375), "{body}");
    assert_eq!(body["my_vote"], json!(-1), "{body}");

    // Anonymous cannot vote.
    let mut anon = harness.client();
    let (status, _b) = anon
        .post(&format!("/api/v1/directory/entries/{entry_id}/vote"), json!({"value": 1}))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // A TL4 vote outweighs a TL0 vote (spec §39.4): the score moves more
    // than the TL0's 0.5 default weight.
    let mut tl4 = harness.client();
    let (tl4_account, _p) = register(&mut tl4, "tl4@example.com", "tl4").await;
    lorehaven_db::directory::set_account_trust_for_tests(harness.tdb.db(), &tl4_account, 4).await;
    let (status, body) = tl4
        .post(&format!("/api/v1/directory/entries/{entry_id}/vote"), json!({"value": 1}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let score = body["score"].as_f64().expect("score");
    assert!(score > 0.0, "TL4 upvote must lift the score above zero: {body}");
    // The response never carries weights.
    let raw = serde_json::to_string(&body).expect("serialise");
    assert!(!raw.contains("weight"), "weights are never disclosed: {raw}");
}

#[tokio::test]
async fn removal_is_hard_delete_and_votes_stay_for_audit() {
    let harness = Harness::new("removal").await;
    let mut operator = harness.client();
    let (op_account, _op_pseud) = register(&mut operator, "op@example.com", "operator").await;
    make_operator(&harness.tdb, &op_account).await;
    seed_list(&mut operator).await;

    let mut member = harness.client();
    let (_m_account, _m_pseud) = register(&mut member, "member@example.com", "memberer").await;
    let entry_id = submit_entry(&mut member, "Doomed", "https://doomed.example.org").await;
    let (status, _b) = operator
        .post(&format!("/api/v1/directory/entries/{entry_id}/approve"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);

    let mut voter = harness.client();
    let (_v_account, _v_pseud) = register(&mut voter, "voter@example.com", "voter").await;
    let (status, _b) = voter
        .post(&format!("/api/v1/directory/entries/{entry_id}/vote"), json!({"value": 1}))
        .await;
    assert_eq!(status, StatusCode::OK);

    // Operator removes the entry.
    let (status, _b) = operator
        .delete(&format!("/api/v1/directory/entries/{entry_id}"))
        .await;
    assert_eq!(status, StatusCode::OK);

    // Gone from every view.
    let (status, body) = operator
        .get("/api/v1/directory/entries?list=external-sites")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"].as_array().expect("items").len(), 0, "{body}");
    let (status, _b) = operator
        .get(&format!("/api/v1/directory/entries/{entry_id}"))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // But the vote row is still there for audit.
    let db = harness.tdb.db();
    let votes: Vec<(String, i64)> =
        lorehaven_db::directory::votes_for_tests(db, &entry_id).await;
    assert_eq!(votes.len(), 1, "votes survive entry removal for audit");
}

#[tokio::test]
async fn config_extra_categories_and_weighting_mode() {
    let harness = Harness::new("config").await;
    let mut operator = harness.client();
    let (op_account, _op_pseud) = register(&mut operator, "op@example.com", "operator").await;
    make_operator(&harness.tdb, &op_account).await;
    seed_list(&mut operator).await;

    // The seed categories are browsable via tabs.
    let (status, body) = operator.get("/api/v1/directory/categories").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let cats = body["items"].as_array().expect("items");
    assert!(cats.iter().any(|c| c["category"] == "fanfiction_archive"), "{body}");

    // A category outside the seed set is refused unless configured.
    let mut member = harness.client();
    let (_m_account, _m_pseud) = register(&mut member, "member@example.com", "memberer").await;
    let (status, body) = member
        .post(
            "/api/v1/directory/entries",
            json!({
                "list": "external-sites",
                "kind": "external",
                "category": "custom_category",
                "title": "Custom site",
                "url": "https://custom.example.org",
                "description": ""
            }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["field_errors"]["reason"], "category_not_allowed");
}

async fn seed_list(client: &mut Client) {
    let (status, body) = client
        .post(
            "/api/v1/directory/lists",
            json!({
                "slug": "external-sites",
                "title": "External sites",
                "kind": "external",
                "description": "Test list."
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "seed list: {body}");
}
