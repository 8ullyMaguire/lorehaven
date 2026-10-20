//! M2-05: scoped API tokens, and the two ways a token must not exceed its account.
//!
//! # The hole this file exists for
//!
//! **Any account could mint a token with `admin.write`.** The route parsed the
//! requested scopes, stored them, and the resolver trusted the stored set — so
//! the token opened every admin door, and the check that would have caught it
//! was at the *door* ("does this token have the scope"), where the token did.
//!
//! The rule is now: **a token can never exceed its account** (spec §20.2). The
//! check is at the *mint*, and it asks the account's own privileges.
//!
//! # And the second hole
//!
//! **A suspended account's token still worked.** The resolver asked only about
//! the token, never about the account, so a credential outlived the identity
//! that held it. The same is true of a **folded** account (spec §14.2): the fold
//! leaves a tombstone, and a tombstone must not act.
//!
//! **The default has to be the closed one**, so the resolver filters on
//! `status = 'active'` rather than `status <> 'folded'` — a status nobody has
//! thought about is not treated as active.

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
        "darkhold-api-token-{tag}-{}-{:?}",
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
    /// The bearer token, when one has been issued.
    token: Option<String>,
}

impl Client {
    fn new(app: axum::Router) -> Self {
        Self {
            app,
            cookies: Vec::new(),
            token: None,
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
        if let Some(token) = self.token.clone() {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        } else {
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
        let account_id = me["account"]["id"].as_str().expect("account id").to_owned();
        let pseud_id = me["pseuds"][0]["id"].as_str().expect("pseud id").to_owned();
        (account_id, pseud_id)
    }

    /// Issue a token and return the secret — **the only time it is ever
    /// returned**.
    ///
    /// The acting pseud is named, because `RequireActor` refuses a token that
    /// has none: a token belongs to an account but acts as a pseud, and those
    /// are different identities the schema keeps strictly apart.
    async fn issue_token(&mut self, pseud_id: &str, scopes: &[&str]) -> (StatusCode, Value) {
        self.post(
            "/api/v1/me/tokens",
            json!({
                "name": "a token",
                "kind": "api",
                "scopes": scopes,
                "acting_pseud_id": pseud_id,
            }),
        )
        .await
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

#[tokio::test]
async fn an_ordinary_account_may_not_mint_an_admin_token() {
    let harness = Harness::new("admin-scope").await;
    let mut client = harness.client();
    let (_account, pseud) = client.register("ordinary@example.test", "Ordinary").await;

    // **The hole.** Before the check, this returned a token and the token
    // opened every admin door.
    let (status, body) = client
        .issue_token(&pseud, &["content.read", "admin.write"])
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "an account that is not an operator may not mint an admin token: {body}"
    );
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("admin.write"),
        "the refusal names the scope it refused: {message}"
    );

    // And the scope the account MAY hold is still mintable.
    let (status, body) = client
        .issue_token(&pseud, &["content.read", "content.write"])
        .await;
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "an ordinary account may still mint a token it is entitled to: {status} {body}"
    );

    harness.tdb.db().close().await;
}

#[tokio::test]
async fn a_read_scoped_token_cannot_write() {
    let harness = Harness::new("read-only").await;
    let mut client = harness.client();
    let (_account, pseud) = client.register("writer@example.test", "Writer").await;

    let (status, body) = client.issue_token(&pseud, &["content.read"]).await;
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "issue a read-only token: {status} {body}"
    );
    let _ = body;
    let secret = body["token"]
        .as_str()
        .expect("the token is returned once, at issue")
        .to_owned();

    // A second client, carrying only the token.
    let mut bearer = Client::new(server::build_router(AppState::new(
        harness.config.clone(),
        harness.tdb.db().clone(),
    )));
    bearer.token = Some(secret);

    // The read works. `/api/v1/me/credential` is the token's own door: it
    // answers "who am I, and what may this token do" and nothing else.
    let (status, body) = bearer.get("/api/v1/me/credential").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a read-scoped token may read its own credential: {body}"
    );
    assert_eq!(body["via"], "token", "the credential is the token: {body}");

    // The write does not. `POST /api/v1/imports` is behind `content.write`,
    // and an import writes the work into the library.
    let (status, body) = bearer
        .post(
            "/api/v1/imports",
            json!({ "source": "https://example.invalid/", "url": "https://example.invalid/x" }),
        )
        .await;
    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::UNAUTHORIZED,
        "a read-scoped token must not write: {status} {body}"
    );
    assert!(
        body.to_string().contains("scope"),
        "the refusal names the scope: {body}"
    );

    harness.tdb.db().close().await;
}

#[tokio::test]
async fn a_token_whose_account_was_folded_cannot_act() {
    // Spec §14.2: the fold leaves a tombstone, and a tombstone must not act.
    //
    // **The fold itself is M20's** — the re-parenting of every row a guest
    // authored, in one transaction, with the schema-driven completeness check.
    // What M2-05 owns is the **marker** and the **refusal**: the state that
    // says "this identity is a tombstone", and the rule that anything acting as
    // a folded account does not act.
    let harness = Harness::new("folded").await;
    let mut client = harness.client();
    let (account_id, pseud) = client.register("guest@example.test", "Guest").await;

    let (status, body) = client.issue_token(&pseud, &["content.read"]).await;
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "issue a token: {status} {body}"
    );
    let secret = body["token"].as_str().expect("token").to_owned();

    // It works while the account is active.
    let mut bearer = Client::new(server::build_router(AppState::new(
        harness.config.clone(),
        harness.tdb.db().clone(),
    )));
    bearer.token = Some(secret.clone());
    let (status, body) = bearer.get("/api/v1/me/credential").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the token works while the account is active: {body}"
    );

    // Fold the account.
    let account: darkhold_domain::AccountId = account_id.parse().expect("account id");
    let other = "00000000-0000-4000-8000-000000000001";
    darkhold_db::identity::fold_account(harness.tdb.db(), account, other)
        .await
        .expect("fold the account");

    // And the token stops working.
    let (status, body) = bearer.get("/api/v1/me/credential").await;
    assert!(
        status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
        "a folded account's token must not act: {status} {body}"
    );

    harness.tdb.db().close().await;
}

#[tokio::test]
async fn a_token_is_shown_once_and_never_again() {
    let harness = Harness::new("shown-once").await;
    let mut client = harness.client();
    let (_account, pseud) = client.register("once@example.test", "Once").await;

    let (status, body) = client.issue_token(&pseud, &["content.read"]).await;
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "issue: {status} {body}"
    );
    assert!(
        body["token"].is_string(),
        "the secret is returned at issue: {body}"
    );

    // And the list never carries it again.
    let (status, list) = client.get("/api/v1/me/tokens").await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let items = list["tokens"].as_array().expect("tokens");
    assert_eq!(items.len(), 1, "one token: {list}");
    let serialised = serde_json::to_string(&items[0]).expect("serialise");
    assert!(
        !serialised.contains(body["token"].as_str().unwrap_or_default()),
        "the token list must not carry the secret: {serialised}"
    );
    assert!(
        items[0].get("id").is_some(),
        "the list carries the token's own id -- which identifies it without being \
         the secret: {serialised}"
    );
    assert!(
        items[0].get("token_hash").is_none() && items[0].get("token").is_none(),
        "and the list carries neither the secret nor its hash: {serialised}"
    );

    harness.tdb.db().close().await;
}
