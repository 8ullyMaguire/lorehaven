//! Acceptance: the bot link flow and the token foundation it stands on
//! (spec §23.2, §23.1; M54-01, M54-02).
//!
//! Every test here exists because of a **defect found by reading the code**,
//! not by a test failing — the token machinery had no test that would have
//! caught any of them. The defects are named in `docs/plans/m54-bot-core.md` and
//! referred to by their letters below, so a future reader can look up why a
//! particular assertion is as paranoid as it is.
//!
//! The load-bearing properties, in order of severity:
//!
//! 1. **Revocation is scoped (D1).** `POST /me/tokens/{id}` used to take a
//!    session, discard it, and run an UPDATE with no account predicate — an
//!    unauthenticated, cross-account denial of service. The first two tests are
//!    the ones that would have caught it.
//! 2. **A token can expire (D2, D4).** `expires_at` existed since migration
//!    0001, was read by `list_tokens`, and was written by nothing, so every
//!    token on every instance was permanent. And `resolve_token` did not check
//!    the column, so fixing the write alone would have been nothing.
//! 3. **Use is observable (D3).** `last_used_at` had the same shape.
//! 4. **A link challenge is single-use and short-lived**, and the token it mints
//!    carries only the scopes the reader granted.
//! 5. **A bot never receives a password** — enforced by a test that sends one.
//! 6. **An unknown API path is a 404, not the SPA shell (D9).** The fallback
//!    answered anything extensionless with 200 and an HTML body, so a client
//!    calling a door this build does not have got 200 and `null`.
//!
//! Everything runs on both engines. The store queries under test are
//! dialect-branched, and a suite pinned to SQLite proves the SQLite arm while
//! the PostgreSQL arm ships unverified — the migration parity test does not
//! help, because it compares *declarations*, not behaviour.

use axum::http::StatusCode;
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_db::DatabaseConfig;
use serde_json::json;
use test_support::{scratch_dir, sign_in_as, TestClient, TestDb, TEST_PASSWORD};

/// One test's app, database and cookie jar.
///
/// The app is built from the `TestDb`'s *own* database rather than a
/// hand-written config. That distinction is the whole reason this struct
/// exists: a config naming a SQLite URL while the pool is on PostgreSQL builds
/// an app that reads a database nobody migrated, so every authenticated
/// request comes back 401 — under PostgreSQL only, and passing locally.
struct Harness {
    _dir: std::path::PathBuf,
    tdb: TestDb,
    client: TestClient,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        let mut config = Config::development_defaults();
        config.storage.root = dir.clone();
        config.database = if tdb.is_postgres() {
            DatabaseConfig::new(std::env::var("LOREHAVEN_TEST_PG_URL").unwrap_or_default())
        } else {
            DatabaseConfig::new(format!(
                "sqlite://{}/lorehaven.sqlite?mode=rwc",
                dir.display()
            ))
        };
        let client = TestClient::new(server::build_router(AppState::new(
            config.clone(),
            tdb.db().clone(),
        )));
        Self {
            _dir: dir,
            tdb,
            client,
        }
    }

    async fn signed_in(&mut self, email: &str, handle: &str) -> String {
        test_support::register(&mut self.client, email, handle).await
    }

    /// The account the *current session* belongs to, read back through the
    /// session cookie the way the server does.
    ///
    /// Resolved through the cookie rather than read as "the first account row",
    /// which reports the first *registered* account no matter who is signed in.
    async fn current_account_id(&self) -> String {
        let raw = self
            .client
            .cookie("lorehaven_session")
            .expect("a session cookie, or the test never signed in");
        self.tdb
            .fetch_text_column(
                "sessions",
                "account_id",
                "token_hash",
                &lorehaven_app::crypto::hash_token(raw),
            )
            .await
            .expect("the session row behind the cookie")
    }

    async fn current_pseud_id(&self) -> String {
        let account = self.current_account_id().await;
        self.tdb
            .fetch_text_column("pseuds", "id", "account_id", &account)
            .await
            .expect("the account has a pseud")
    }

    /// Become a different account, for the cross-account tests.
    async fn switch_to(&mut self, email: &str, handle: &str) -> String {
        sign_in_as(&mut self.client, &self.tdb, email, handle).await
    }

    /// Issue a token directly and return `(id, raw_secret)`.
    async fn issue(
        &self,
        account: &str,
        scopes: &[&str],
        expires_at: Option<String>,
    ) -> (String, String) {
        let raw = uuid::Uuid::new_v4().to_string();
        let hash = lorehaven_app::crypto::hash_token(&raw);
        let parsed: Vec<lorehaven_domain::api_scopes::Scope> = scopes
            .iter()
            .map(|s| {
                use std::str::FromStr as _;
                lorehaven_domain::api_scopes::Scope::from_str(s).expect("a known scope")
            })
            .collect();
        let id = lorehaven_db::external::issue_token_expiring(
            self.tdb.db(),
            account,
            "personal",
            "test",
            &hash,
            &parsed,
            expires_at.as_deref(),
        )
        .await
        .expect("issue a token");
        (id, raw)
    }

    /// Register a bot through the API. Returns `(bot_id, raw_secret)`.
    async fn register_bot(&mut self, name: &str) -> (String, String) {
        let (status, body) = self
            .client
            .post(
                "/api/v1/me/bots",
                json!({
                    "name": name,
                    "contact": "ops@example.test",
                    "user_agent": "m54-test/1",
                    "scopes": ["content.read", "library.read"],
                }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "register bot: {body}");
        (
            body["bot_id"].as_str().expect("bot_id").to_owned(),
            body["token"].as_str().expect("token").to_owned(),
        )
    }

    /// The token id a bot registration was issued with, or None.
    async fn bot_token_id(&self, bot_id: &str) -> Option<String> {
        self.tdb
            .fetch_text_column("bot_registrations", "token_id", "id", bot_id)
            .await
    }
}

// ---------------------------------------------------------------------------
// D1 — revocation is scoped to the caller's own account
//
// These two tests are the ones that would have caught the defect. The endpoint
// accepted an unauthenticated request and revoked anyone's token by id.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_anonymous_request_cannot_revoke_a_token() {
    let mut h = Harness::new("revoke-anon").await;
    let owner = h.signed_in("owner@example.test", "owner").await;
    let (token_id, raw) = h.issue(&owner, &["content.read"], None).await;

    // A separate harness's app, so no cookie is sent at all — and its own
    // database, so the token genuinely does not exist there.
    let mut anon = Harness::new("revoke-anon-peer").await;
    let (status, body) = anon
        .client
        .request_with(
            "POST",
            &format!("/api/v1/me/tokens/{token_id}"),
            None,
            Some(&raw),
        )
        .await;
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::UNAUTHORIZED,
        "an unauthenticated revoke must not succeed: {status} {body}"
    );

    // The owner's token is untouched in the owner's database.
    assert!(
        lorehaven_db::external::resolve_token(h.tdb.db(), &lorehaven_app::crypto::hash_token(&raw))
            .await
            .expect("resolve")
            .is_some(),
        "a token must survive a revoke attempt from somewhere else"
    );
}

#[tokio::test]
async fn one_account_cannot_revoke_another_accounts_token() {
    let mut h = Harness::new("revoke-cross").await;
    let owner = h.signed_in("owner@example.test", "owner").await;
    let (token_id, raw) = h.issue(&owner, &["content.read"], None).await;

    // Genuinely become the other account. Registering a second account without
    // logging in leaves the session as the owner, and then this is the owner
    // revoking their own token — 200, for a reason that has nothing to do with
    // the account predicate. That is how the test was first written.
    let intruder = h.switch_to("intruder@example.test", "intruder").await;
    assert_ne!(
        intruder, owner,
        "the caller really is a different account; if this fails, the rest of \
         this test is measuring nothing"
    );

    let (status, body) = h
        .client
        .request("POST", &format!("/api/v1/me/tokens/{token_id}"), None)
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "another account's token is not found for the caller, and 404 rather than \
         403 so the response does not confirm it exists: {body}"
    );

    assert!(
        lorehaven_db::external::resolve_token(h.tdb.db(), &lorehaven_app::crypto::hash_token(&raw))
            .await
            .expect("resolve")
            .is_some(),
        "the owner's token is still live: a cross-account revoke must not take"
    );

    // And the owner can revoke their own — so switch *back* first. The session
    // is the intruder's at this point; asking again without a switch and
    // expecting 200 asserts the wrong account is the owner.
    let back = h.switch_to("owner@example.test", "owner").await;
    assert_eq!(back, owner, "back to the owner");
    let (status, body) = h
        .client
        .request("POST", &format!("/api/v1/me/tokens/{token_id}"), None)
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the owner revokes their own: {body}"
    );
}

// ---------------------------------------------------------------------------
// D2 / D4 — a token can expire, and expiry is enforced at resolution
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_token_with_an_expiry_in_the_past_does_not_resolve() {
    let mut h = Harness::new("expiry-past").await;
    let account = h.signed_in("owner@example.test", "owner").await;
    let expired = lorehaven_db::identity::in_seconds(-60);
    let (_id, raw) = h.issue(&account, &["content.read"], Some(expired)).await;

    assert!(
        lorehaven_db::external::resolve_token(h.tdb.db(), &lorehaven_app::crypto::hash_token(&raw))
            .await
            .expect("resolve")
            .is_none(),
        "an expired token must not authenticate: expires_at existed since 0001, \
         was read by list_tokens, and was written by nothing"
    );
}

#[tokio::test]
async fn a_token_with_a_future_expiry_does_resolve() {
    let mut h = Harness::new("expiry-future").await;
    let account = h.signed_in("owner@example.test", "owner").await;
    let future = lorehaven_db::identity::in_seconds(600);
    let (_id, raw) = h.issue(&account, &["content.read"], Some(future)).await;

    assert!(
        lorehaven_db::external::resolve_token(h.tdb.db(), &lorehaven_app::crypto::hash_token(&raw))
            .await
            .expect("resolve")
            .is_some(),
        "the expiry arm must not refuse a token that has not expired"
    );
}

#[tokio::test]
async fn issuing_a_token_through_the_api_stores_the_expiry_it_was_asked_for() {
    let mut h = Harness::new("issue-expiry").await;
    h.signed_in("owner@example.test", "owner").await;

    let (status, body) = h
        .client
        .post(
            "/api/v1/me/tokens",
            json!({
                "name": "short-lived",
                "kind": "personal",
                "scopes": ["content.read"],
                "expires_in_seconds": 600,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "issue: {body}");
    assert!(
        body["expires_at"].is_string(),
        "the expiry is echoed back, so a caller can see the lifetime it got: {body}"
    );

    let (status, listed) = h.client.get("/api/v1/me/tokens").await;
    assert_eq!(status, StatusCode::OK, "list: {listed}");
    let tokens = listed["tokens"].as_array().expect("an array");
    let stored = tokens
        .iter()
        .find(|t| t["name"] == "short-lived")
        .expect("the token is listed");
    assert!(
        stored["expires_at"].is_string(),
        "the stored row carries the expiry: {stored}"
    );
}

#[tokio::test]
async fn an_expiry_outside_the_accepted_range_is_refused_with_the_range_named() {
    let mut h = Harness::new("issue-expiry-range").await;
    h.signed_in("owner@example.test", "owner").await;

    // Zero: refused rather than clamped, because a clamped expiry is a lifetime
    // nobody chose and nobody can see.
    let (status, body) = h
        .client
        .post(
            "/api/v1/me/tokens",
            json!({
                "name": "nope",
                "kind": "personal",
                "scopes": ["content.read"],
                "expires_in_seconds": 0,
            }),
        )
        .await;
    assert!(
        status == StatusCode::UNPROCESSABLE_ENTITY || status == StatusCode::BAD_REQUEST,
        "a zero expiry is refused: {status} {body}"
    );

    // Beyond the ceiling: the refusal names the ceiling.
    let (status, body) = h
        .client
        .post(
            "/api/v1/me/tokens",
            json!({
                "name": "forever",
                "kind": "personal",
                "scopes": ["content.read"],
                "expires_in_seconds": 100_000_000_000_i64,
            }),
        )
        .await;
    assert!(
        status == StatusCode::UNPROCESSABLE_ENTITY || status == StatusCode::BAD_REQUEST,
        "an expiry beyond the ceiling is refused: {status} {body}"
    );
    let rendered = body.to_string();
    assert!(
        rendered.contains("31536000") || rendered.contains("one year"),
        "the refusal names the accepted range, so a caller can fix the call: {body}"
    );
}

// ---------------------------------------------------------------------------
// D3 — a token's use is observable
// ---------------------------------------------------------------------------

#[tokio::test]
async fn last_used_at_is_null_before_use_and_set_after_it() {
    let mut h = Harness::new("last-used").await;
    let account = h.signed_in("owner@example.test", "owner").await;
    let (token_id, raw) = h.issue(&account, &["content.read"], None).await;

    assert!(
        h.tdb
            .fetch_text_column("api_tokens", "last_used_at", "id", &token_id)
            .await
            .is_none(),
        "a token nobody has used has no last use — the column existed and was \
         written by nothing, so this was null forever"
    );

    // A request that actually resolves the token. `/media` is the route that
    // reads `MaybeToken` today (D8). My first draft used `/media/search`, which
    // does not exist, and the SPA fallback answered it 200 with an HTML shell —
    // so the "authenticated call" this test depends on never reached a handler.
    // It passed anyway, because the assertion accepted 200.
    let (status, body) = h
        .client
        .request_with("GET", "/api/v1/media", None, Some(&raw))
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the bearer-authenticated request reached the handler: {status} {body}"
    );

    assert!(
        h.tdb
            .fetch_text_column("api_tokens", "last_used_at", "id", &token_id)
            .await
            .is_some(),
        "after an authenticated call the token's last use is recorded"
    );
}

// ---------------------------------------------------------------------------
// §23.1 — the acting pseud is explicit, and checked
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_token_records_the_acting_pseud_it_was_issued_for() {
    let mut h = Harness::new("acting-pseud").await;
    h.signed_in("owner@example.test", "owner").await;
    let pseud = h.current_pseud_id().await;

    let (status, body) = h
        .client
        .post(
            "/api/v1/me/tokens",
            json!({
                "name": "as-myself",
                "kind": "personal",
                "scopes": ["content.read"],
                "acting_pseud_id": pseud,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "issue with acting pseud: {body}");
    assert_eq!(
        body["acting_pseud_id"].as_str(),
        Some(pseud.as_str()),
        "the acting pseud is echoed, so it is explicit to the caller too: {body}"
    );

    let raw = body["token"].as_str().expect("token").to_owned();
    let identity =
        lorehaven_db::external::resolve_token(h.tdb.db(), &lorehaven_app::crypto::hash_token(&raw))
            .await
            .expect("resolve")
            .expect("the token resolves");
    assert_eq!(
        identity.acting_pseud_id.as_deref(),
        Some(pseud.as_str()),
        "a token acts as the pseud it was issued for, not as the account's \
         default — §23.1 requires the acting pseud to be explicit"
    );
}

#[tokio::test]
async fn a_token_cannot_act_as_another_accounts_pseud() {
    let mut h = Harness::new("acting-pseud-foreign").await;
    let owner = h.signed_in("owner@example.test", "owner").await;
    let other = h.switch_to("other@example.test", "other").await;
    assert_ne!(other, owner, "two distinct accounts");
    let foreign_pseud = h.current_pseud_id().await;

    // Become the owner again and try to borrow the other account's pseud.
    h.switch_to("owner@example.test", "owner").await;
    let (status, body) = h
        .client
        .post(
            "/api/v1/me/tokens",
            json!({
                "name": "borrowed-face",
                "kind": "personal",
                "scopes": ["content.read"],
                "acting_pseud_id": foreign_pseud,
            }),
        )
        .await;
    assert!(
        status == StatusCode::UNPROCESSABLE_ENTITY || status == StatusCode::BAD_REQUEST,
        "naming another account's pseud is refused: {status} {body}"
    );
}

// ---------------------------------------------------------------------------
// §23.2 — the link flow
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_challenge_is_short_lived_and_records_what_the_bot_asked_for() {
    let mut h = Harness::new("challenge-ttl").await;
    h.signed_in("owner@example.test", "owner").await;
    let (bot_id, _raw) = h.register_bot("Archivist").await;

    let code = lorehaven_db::external::create_link_challenge(
        h.tdb.db(),
        &bot_id,
        &["library.read".into()],
        600,
    )
    .await
    .expect("create a challenge");
    let challenge = lorehaven_db::external::find_link_challenge(h.tdb.db(), &code)
        .await
        .expect("find")
        .expect("the challenge exists");
    assert_eq!(challenge.state, "pending");
    assert!(
        challenge.expires_at > challenge.created_at,
        "a challenge's expiry is in the future when it is issued: {} vs {}",
        challenge.expires_at,
        challenge.created_at
    );
    assert_eq!(
        challenge.requested_scopes,
        vec!["library.read".to_owned()],
        "the recorded request is what the confirmation page shows the reader"
    );
}

#[tokio::test]
async fn a_challenge_is_confirmed_at_most_once() {
    let mut h = Harness::new("challenge-single").await;
    let account = h.signed_in("owner@example.test", "owner").await;
    let pseud = h.current_pseud_id().await;
    let (bot_id, _raw) = h.register_bot("Archivist").await;
    let code = lorehaven_db::external::create_link_challenge(h.tdb.db(), &bot_id, &[], 600)
        .await
        .expect("create a challenge");

    let (first_token, _) = h.issue(&account, &["library.read"], None).await;
    assert!(
        lorehaven_db::external::confirm_link_challenge(h.tdb.db(), &code, &pseud, &first_token)
            .await
            .expect("confirm"),
        "the first confirmation binds the challenge"
    );

    let (second_token, _) = h.issue(&account, &["library.read"], None).await;
    assert!(
        !lorehaven_db::external::confirm_link_challenge(h.tdb.db(), &code, &pseud, &second_token)
            .await
            .expect("second confirm"),
        "a second confirmation of the same challenge must mint nothing: the \
         predicate `pseud_id IS NULL` is what makes this true under concurrency"
    );
}

#[tokio::test]
async fn an_expired_challenge_is_refused_and_swept() {
    let mut h = Harness::new("challenge-expired").await;
    let account = h.signed_in("owner@example.test", "owner").await;
    let pseud = h.current_pseud_id().await;
    let (bot_id, _raw) = h.register_bot("Archivist").await;
    let code = lorehaven_db::external::create_link_challenge(h.tdb.db(), &bot_id, &[], -1)
        .await
        .expect("create a challenge already past its expiry");

    let (token_id, _) = h.issue(&account, &["library.read"], None).await;
    assert!(
        !lorehaven_db::external::confirm_link_challenge(h.tdb.db(), &code, &pseud, &token_id)
            .await
            .expect("confirm"),
        "a challenge past its expiry cannot be confirmed — the expiry arm is in \
         the same predicate, so a stale code in a chat log is worthless"
    );

    // And the sweeper is the only thing that may call it 'expired', so it has to
    // actually run for the state to become true.
    assert_eq!(
        lorehaven_db::external::expire_stale_challenges(h.tdb.db())
            .await
            .expect("sweep"),
        1,
        "the sweeper expires the stale row"
    );
    let after = lorehaven_db::external::find_link_challenge(h.tdb.db(), &code)
        .await
        .expect("find")
        .expect("still there");
    assert_eq!(
        after.state, "expired",
        "and the state says so, rather than a pending row whose expiry has passed"
    );
}

#[tokio::test]
async fn a_challenge_code_is_high_entropy_and_not_a_uuid() {
    // Not a test of behaviour so much as of a decision: this nonce is the one
    // value between a leaked chat message and a token. A uuid v4 is 122 bits in
    // an enumerable format; 32 hex-encoded random bytes is the floor that is
    // comfortable.
    let mut h = Harness::new("challenge-entropy").await;
    h.signed_in("owner@example.test", "owner").await;
    let (bot_id, _raw) = h.register_bot("Archivist").await;
    let code = lorehaven_db::external::create_link_challenge(h.tdb.db(), &bot_id, &[], 600)
        .await
        .expect("create");
    assert_eq!(
        code.len(),
        64,
        "32 bytes hex-encoded, not a 36-character uuid"
    );
    assert!(uuid::Uuid::parse_str(&code).is_err());
}

#[tokio::test]
async fn a_bot_registration_names_a_token_that_actually_resolves() {
    let mut h = Harness::new("bot-token-link").await;
    h.signed_in("owner@example.test", "owner").await;
    let (bot_id, raw) = h.register_bot("Archivist").await;

    // The registration row names the token it was issued with.
    h.bot_token_id(&bot_id)
        .await
        .expect("the registration names its token");
    // And the secret it returned resolves, so a bot can call the API
    // immediately — the link flow exists to hand a bot a token *without* a
    // password, not to hand it one and make it wait.
    assert!(
        lorehaven_db::external::resolve_token(h.tdb.db(), &lorehaven_app::crypto::hash_token(&raw))
            .await
            .expect("resolve")
            .is_some(),
        "the token a bot is given at registration resolves"
    );
}

// ---------------------------------------------------------------------------
// §23.2's hard constraint: a bot never receives a password
// ---------------------------------------------------------------------------

#[tokio::test]
async fn no_link_door_accepts_a_password() {
    let mut h = Harness::new("no-password").await;
    h.signed_in("owner@example.test", "owner").await;
    let (bot_id, raw) = h.register_bot("Archivist").await;
    let code = lorehaven_db::external::create_link_challenge(h.tdb.db(), &bot_id, &[], 600)
        .await
        .expect("create");

    // Every door the link flow has, offered a password. Each must refuse. This
    // is the test that makes §23.2's "bots never receive the user's password"
    // a checked property rather than a sentence in a document.
    for uri in [
        "/api/v1/link/challenge".to_owned(),
        "/api/v1/link/confirm".to_owned(),
        format!("/api/v1/link/challenge/{code}/redeem"),
    ] {
        let (status, body) = h
            .client
            .request_with(
                "POST",
                &uri,
                Some(json!({
                    "code": code,
                    "scopes": ["library.read"],
                    "password": TEST_PASSWORD,
                })),
                Some(&raw),
            )
            .await;
        // 404 (the door does not exist in this build) or 422 (it exists and
        // refuses the unknown field). What is *not* acceptable is success, and
        // equally not a 2xx that quietly ignores the password.
        assert!(
            status == StatusCode::NOT_FOUND
                || status == StatusCode::UNPROCESSABLE_ENTITY
                || status == StatusCode::BAD_REQUEST,
            "POST {uri} must refuse a payload carrying a password: {status} {body}"
        );
    }
}

// ---------------------------------------------------------------------------
// D9 — an unknown API path is a 404, not the SPA shell
//
// Found while writing this suite: `POST /api/v1/link/challenge` returned 200
// with a `null` body while the door did not exist, because the static-asset
// fallback answers anything extensionless with the HTML shell. A client cannot
// tell "this instance does not have that feature" from "that worked".
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_unknown_api_path_is_a_404_not_the_spa_shell() {
    let mut h = Harness::new("api-404").await;
    h.signed_in("owner@example.test", "owner").await;

    // Only genuinely absent paths. `/me/tokens/{id}` *does* exist, so a GET on
    // it is 405 METHOD_NOT_ALLOWED — a correct answer from the router, and not
    // the shell. Asserting 404 there would be asserting a falsehood about the
    // route table to prove a point about the fallback.
    for uri in ["/api/v1/does-not-exist", "/api/v1/link/challenge"] {
        let (status, body) = h.client.get(uri).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "GET {uri} must be a 404, not the SPA shell: {status} {body}"
        );
    }

    // And a path that exists under a different method still says so, rather
    // than falling through to HTML.
    let (status, body) = h.client.get("/api/v1/me/tokens/some-id").await;
    assert_eq!(
        status,
        StatusCode::METHOD_NOT_ALLOWED,
        "a real route with the wrong method reports the method, not the shell: \
         {status} {body}"
    );
}

/// A browser route still gets the shell — the fallback is for the SPA's own
/// routes, and the fix must not break client-side routing.
#[tokio::test]
async fn a_browser_route_still_gets_the_shell() {
    let mut h = Harness::new("spa-route").await;
    let (status, _) = h.client.get("/library").await;
    // 200 when assets are present, 404 when this build has no bundle on disk.
    // Either is fine; what is *not* fine is a 400 or a 500.
    assert!(
        status == StatusCode::OK || status == StatusCode::NOT_FOUND,
        "a client-side route is served by the fallback, not rejected: {status}"
    );
}
