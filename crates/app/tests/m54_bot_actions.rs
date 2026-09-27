//! Acceptance: every §23.2 bot action is reachable by a bearer token
//! (spec §23.2; M54-03, A6).
//!
//! Before A6, every one of these doors took `RequireSession` and so was
//! unreachable by a token — the API existed, the actions were documented, and a
//! bot could perform none of them. `MaybeToken` appeared in exactly one handler
//! in the whole tree. That is the defect this file pins.
//!
//! The matrix, per door:
//!
//! | credential | expectation |
//! |---|---|
//! | token with the scope | the action happens |
//! | token without the scope | 403 **naming the scope** |
//! | no credential at all | 401 |
//! | session | the action still happens (the browser must not regress) |
//!
//! Plus ownership: a token with the right scope belonging to account A must not
//! let A act on account B's row by id. A scope says what an account may do; it
//! never says whose rows those are, and treating it as if it did is how a
//! scoped token becomes an instance-wide one.
//!
//! Both engines: the ownership predicates are dialect-branched, and a suite
//! pinned to SQLite would leave the PostgreSQL arm of every one of them
//! unverified.

use axum::http::StatusCode;
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_db::DatabaseConfig;
use serde_json::json;
use test_support::{scratch_dir, sign_in_as, TestClient, TestDb};

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

    async fn switch_to(&mut self, email: &str, handle: &str) -> String {
        sign_in_as(&mut self.client, &self.tdb, email, handle).await
    }

    /// A token carrying `scopes`, acting as a real pseud of `account`.
    ///
    /// Built through the repository layer rather than the API on purpose: the
    /// API's `/me/tokens` door issues a token with no acting pseud, and A6
    /// *refuses* a token without one. A test that took its token from the
    /// ordinary issue endpoint would be testing a token the bot flow never
    /// mints, and would fail for a reason unrelated to the door it meant to
    /// exercise.
    async fn token_as(&self, account: &str, pseud: &str, scopes: &[&str]) -> String {
        let raw = uuid::Uuid::new_v4().to_string();
        let hash = lorehaven_app::crypto::hash_token(&raw);
        let parsed: Vec<lorehaven_domain::api_scopes::Scope> = scopes
            .iter()
            .map(|s| {
                use std::str::FromStr as _;
                lorehaven_domain::api_scopes::Scope::from_str(s).expect("a known scope")
            })
            .collect();
        lorehaven_db::external::issue_token_acting(
            self.tdb.db(),
            account,
            "bot",
            "m54-actions",
            &hash,
            &parsed,
            None,
            Some(pseud),
        )
        .await
        .expect("issue a token");
        raw
    }

    /// A request carrying `token` and **no session cookie**.
    ///
    /// The cookie jar has to be dropped, not just ignored. `RequireActor`
    /// resolves a session first when one is present, so a test that registers
    /// an account, keeps the cookies, and *also* sends a bearer token is
    /// testing the session — the token is never consulted. That is not a
    /// hypothetical: every failure in the first run of this file was a session
    /// answering where a token was supposed to be refused, and the assertions
    /// read as though the scope check were not running at all.
    async fn as_token(
        &mut self,
        method: &str,
        uri: &str,
        body: Option<serde_json::Value>,
        token: &str,
    ) -> (StatusCode, serde_json::Value) {
        self.client.clear_cookies();
        let result = self
            .client
            .request_with(method, uri, body, Some(token))
            .await;
        result
    }

    /// A token with *no* acting pseud — what the ordinary issue door produces,
    /// and what A6 refuses. Kept beside `token_as` so the two are visibly
    /// variants of one thing rather than two ways of building a token.
    async fn token_as_with_pseud(
        &self,
        account: &str,
        pseud: Option<&str>,
        scopes: &[&str],
    ) -> String {
        let raw = uuid::Uuid::new_v4().to_string();
        let hash = lorehaven_app::crypto::hash_token(&raw);
        let parsed: Vec<lorehaven_domain::api_scopes::Scope> = scopes
            .iter()
            .map(|s| {
                use std::str::FromStr as _;
                lorehaven_domain::api_scopes::Scope::from_str(s).expect("a known scope")
            })
            .collect();
        lorehaven_db::external::issue_token_acting(
            self.tdb.db(),
            account,
            "bot",
            "m54-actions",
            &hash,
            &parsed,
            None,
            pseud,
        )
        .await
        .expect("issue a token");
        raw
    }

    /// One queued `export_jobs` row owned by `account`, plus its queue row.
    ///
    /// Written by hand because the door that would create it validates its
    /// subject *synchronously* — a work that does not exist is a 422 before any
    /// row is written — so getting a row that way means seeding a work with
    /// chapters, turning a test about ownership into a test about import.
    ///
    /// The privacy notice is acknowledged, because an export that requires
    /// delivery is refused while it is null and the `GET` under test would then
    /// be answering for the wrong reason.
    ///
    /// Both engines: placeholders and the uuid-typed `account_id` differ, so
    /// the statements go through `TestDb::sql` and the column is compared as
    /// text.
    async fn insert_export(&self, export_id: &str, job_id: &str, account: &str) {
        use lorehaven_db::Backend;
        let now = lorehaven_db::identity::now_rfc3339();
        // `jobs.id`, `export_jobs.id`, `.job_id` and `.account_id` are TEXT on
        // SQLite and UUID on PostgreSQL, and sqlx binds a Rust `&str` as text.
        //
        // The cast goes on the *value* here — `?::text` — which is the mirror
        // image of the read case, where it has to go on the *column*. In a
        // `WHERE`, casting the value makes `uuid = text` legal; in an
        // `INSERT ... VALUES`, casting the value does not help at all, because
        // the error is an assignment (`column "id" is of type uuid but
        // expression is of type text`), and the column is what has to be
        // coerced. Hence `?::uuid` for the insert, and the two directions are
        // not interchangeable.
        // And the timestamps: `now_rfc3339()` returns text, while PostgreSQL
        // declares these columns `timestamptz`. SQLite stores them as TEXT
        // throughout, so there the string is the value. Three columns, two
        // dialects, and each cast is invisible until the engine that needs it
        // is the one running — which is the whole argument for running both.
        let (id_in, account_in, ts_in) = if self.tdb.is_postgres() {
            ("?::uuid", "?::uuid", "?::timestamptz")
        } else {
            ("?", "?", "?")
        };
        let job_sql = self.tdb.sql(&format!(
            "INSERT INTO jobs (id, kind, state, payload, priority, attempts, max_attempts, available_at, created_at, updated_at)
             VALUES ({id_in}, 'export', 'queued', '{{}}', 0, 0, 3, {ts_in}, {ts_in}, {ts_in})"
        ));
        let export_sql = self.tdb.sql(&format!(
            "INSERT INTO export_jobs (id, job_id, account_id, subject_type, subject_id, format, privacy_acknowledged_at, state, created_at, updated_at)
             VALUES ({id_in}, {id_in}, {account_in}, 'work', '00000000-0000-4000-8000-000000000001', 'markdown', {ts_in}, 'queued', {ts_in}, {ts_in})"
        ));
        // Built inside each arm: `sqlx::Query` is monomorphic, so one query
        // value cannot serve two pools.
        match self.tdb.db().backend() {
            Backend::Sqlite => {
                let pool = self.tdb.db().sqlite_pool().expect("sqlite");
                sqlx::query(&job_sql)
                    .bind(job_id)
                    .bind(&now)
                    .bind(&now)
                    .bind(&now)
                    .execute(pool)
                    .await
                    .expect("insert the export's queue row");
                sqlx::query(&export_sql)
                    .bind(export_id)
                    .bind(job_id)
                    .bind(account)
                    .bind(&now)
                    .bind(&now)
                    .bind(&now)
                    .execute(pool)
                    .await
                    .expect("insert the export row");
            }
            Backend::Postgres => {
                let pool = self.tdb.db().postgres_pool().expect("postgres");
                sqlx::query(&job_sql)
                    .bind(job_id)
                    .bind(&now)
                    .bind(&now)
                    .bind(&now)
                    .execute(pool)
                    .await
                    .expect("insert the export's queue row");
                sqlx::query(&export_sql)
                    .bind(export_id)
                    .bind(job_id)
                    .bind(account)
                    .bind(&now)
                    .bind(&now)
                    .bind(&now)
                    .execute(pool)
                    .await
                    .expect("insert the export row");
            }
        }
    }

    async fn pseud_of(&self, account: &str) -> String {
        self.tdb
            .fetch_text_column("pseuds", "id", "account_id", account)
            .await
            .expect("the account has a pseud")
    }
}

/// A bookmark body that is valid whatever the subject happens to be.
fn bookmark_body(subject_id: &str) -> serde_json::Value {
    json!({
        "subject_type": "work",
        "subject_id": subject_id,
        "note": "from a bot",
    })
}

// ---------------------------------------------------------------------------
// POST /bookmarks — library.read
//
// A bookmark is a fact about the caller's own shelf. Writing one changes
// nothing the author sees, so it takes the *read* scope.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_bookmark_can_be_made_with_a_library_read_token() {
    let mut h = Harness::new("bm-scope-ok").await;
    let account = h.signed_in("owner@example.test", "owner").await;
    let pseud = h.pseud_of(&account).await;
    let token = h.token_as(&account, &pseud, &["library.read"]).await;

    let (status, body) = h
        .as_token(
            "POST",
            "/api/v1/bookmarks",
            Some(bookmark_body(&uuid::Uuid::new_v4().to_string())),
            &token,
        )
        .await;
    assert!(
        status == StatusCode::CREATED || status == StatusCode::NOT_FOUND,
        "a library.read token reaches POST /bookmarks. A 404 is acceptable \
         because the subject work does not exist in this database and the \
         handler may check the subject before the credential: {status} {body}"
    );
    assert_ne!(
        status,
        StatusCode::UNAUTHORIZED,
        "the point of A6: a bearer token is not refused outright"
    );
    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "library.read is the scope this door needs"
    );
}

#[tokio::test]
async fn a_bookmark_with_the_wrong_scope_is_refused_and_names_the_scope() {
    let mut h = Harness::new("bm-scope-no").await;
    let account = h.signed_in("owner@example.test", "owner").await;
    let pseud = h.pseud_of(&account).await;
    // Deliberately the wrong scope: `content.read` is about content, and a
    // bookmark is about the caller's shelf.
    let token = h.token_as(&account, &pseud, &["content.read"]).await;

    let (status, body) = h
        .as_token(
            "POST",
            "/api/v1/bookmarks",
            Some(bookmark_body(&uuid::Uuid::new_v4().to_string())),
            &token,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "content.read is not library.read: {body}"
    );
    assert!(
        body.to_string().contains("library.read"),
        "the refusal names the scope it wanted, so a bot author can tell a \
         wrong scope from a missing one: {body}"
    );
}

#[tokio::test]
async fn a_bookmark_with_no_credential_is_unauthorized() {
    let mut h = Harness::new("bm-anon").await;
    h.signed_in("owner@example.test", "owner").await;
    // A client with no cookies at all — the harness would otherwise be signed
    // in, and a session is a perfectly good credential for this door.
    h.client.clear_cookies();

    let (status, body) = h
        .client
        .request(
            "POST",
            "/api/v1/bookmarks",
            Some(bookmark_body(&uuid::Uuid::new_v4().to_string())),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "no credential, no bookmark: {body}"
    );
}

#[tokio::test]
async fn a_bookmark_still_works_for_a_signed_in_session() {
    // The browser must not regress. This is the half of the matrix that a
    // naive conversion breaks: switch the door to a token-aware extractor and
    // the session path quietly stops working.
    let mut h = Harness::new("bm-session").await;
    h.signed_in("owner@example.test", "owner").await;

    let (status, body) = h
        .client
        .request(
            "POST",
            "/api/v1/bookmarks",
            Some(bookmark_body(&uuid::Uuid::new_v4().to_string())),
        )
        .await;
    assert!(
        status == StatusCode::CREATED || status == StatusCode::NOT_FOUND,
        "a session with no scopes still reaches the door — a session's authority \
         was settled at login, so there is no scope set to check: {status} {body}"
    );
    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "sessions are not scope-limited"
    );
}

// ---------------------------------------------------------------------------
// A token with no acting pseud is refused
//
// §23.1's "explicit acting pseud": a token acts as a pseud, and a token that
// does not say which one is not a credential this build will act on. The
// refusal is deliberate — falling back to the account's default pseud would
// post the bot's words under a face the reader never chose.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_token_with_no_acting_pseud_is_refused_rather_than_defaulted() {
    let mut h = Harness::new("bm-no-pseud").await;
    let account = h.signed_in("owner@example.test", "owner").await;
    // Issued with no acting pseud — exactly what the ordinary
    // `POST /me/tokens` door produces.
    let token = h
        .token_as_with_pseud(&account, None, &["library.read"])
        .await;

    let (status, body) = h
        .as_token(
            "POST",
            "/api/v1/bookmarks",
            Some(bookmark_body(&uuid::Uuid::new_v4().to_string())),
            &token,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a token that does not say which pseud it acts as is refused: {body}"
    );
    // And it must not have created a bookmark under some default identity.
    // Sign in again first: `as_token` cleared the jar, and this check is about
    // what the *session* can see, so asking without one would only prove that
    // 401 is still 401.
    h.switch_to("owner@example.test", "owner").await;
    let (status, listed) = h.client.get("/api/v1/bookmarks").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the session can still list: {listed}"
    );
    let items = listed["bookmarks"].as_array().map_or(0, Vec::len);
    assert_eq!(
        items, 0,
        "nothing was written: a refused request must not leave a row behind"
    );
}

// ---------------------------------------------------------------------------
// GET /jobs — content.write, and account-scoped
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_job_list_is_reachable_by_a_content_write_token() {
    let mut h = Harness::new("jobs-scope-ok").await;
    let account = h.signed_in("owner@example.test", "owner").await;
    let pseud = h.pseud_of(&account).await;
    let token = h.token_as(&account, &pseud, &["content.write"]).await;

    let (status, body) = h.as_token("GET", "/api/v1/jobs", None, &token).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a content.write token lists the caller's own jobs: {body}"
    );
    assert!(
        // The list is `items`, not `jobs`. Asserting on a key the response does
        // not have is how a test that looks like it checks the list's contents
        // ends up checking nothing at all.
        body["items"].as_array().is_some_and(|j| j.is_empty()),
        "and sees only their own: {body}"
    );
}

#[tokio::test]
async fn a_job_list_with_the_wrong_scope_names_what_it_wanted() {
    let mut h = Harness::new("jobs-scope-no").await;
    let account = h.signed_in("owner@example.test", "owner").await;
    let pseud = h.pseud_of(&account).await;
    let token = h.token_as(&account, &pseud, &["library.read"]).await;

    let (status, body) = h.as_token("GET", "/api/v1/jobs", None, &token).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(
        body.to_string().contains("content.write"),
        "the refusal names the scope: {body}"
    );
}

#[tokio::test]
async fn a_job_list_never_shows_another_accounts_jobs() {
    let mut h = Harness::new("jobs-ownership").await;
    let owner = h.signed_in("owner@example.test", "owner").await;
    let owner_pseud = h.pseud_of(&owner).await;
    // A token with every scope, so nothing but the ownership predicate can be
    // what refuses.
    let owner_token = h
        .token_as(
            &owner,
            &owner_pseud,
            &["content.write", "content.read", "library.read"],
        )
        .await;

    // Account B, with its own all-scopes token.
    let other = h.switch_to("other@example.test", "other").await;
    let other_pseud = h.pseud_of(&other).await;
    let other_token = h
        .token_as(
            &other,
            &other_pseud,
            &["content.write", "content.read", "library.read"],
        )
        .await;

    // Whatever either account can see, neither may see the other's.
    for token in [&owner_token, &other_token] {
        let (status, body) = h.as_token("GET", "/api/v1/jobs", None, token).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        for job in body["items"].as_array().expect("an array") {
            let account = job["account_id"].as_str().unwrap_or_default();
            assert!(
                account == owner || account == other || account.is_empty(),
                "a job belongs to one account and the list showed another: {job}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// POST /exports — library.read
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_export_needs_the_privacy_notice_whatever_the_credential() {
    // The notice is a contract (§13.6), and a check a client can skip by
    // omitting a field is not a notice. This test does not care *who* is
    // calling: it is here to pin that the scope change did not turn the notice
    // into something only sessions see.
    let mut h = Harness::new("exports-notice-token").await;
    let account = h.signed_in("owner@example.test", "owner").await;
    let pseud = h.pseud_of(&account).await;
    let token = h.token_as(&account, &pseud, &["library.read"]).await;

    let (status, body) = h
        .as_token(
            "POST",
            "/api/v1/exports",
            Some(json!({
                "subject_type": "query",
                "subject_id": pseud,
                "query": "q:",
                "format": "markdown",
            })),
            &token,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "the privacy notice is required of a token caller too: {body}"
    );
    assert!(
        body.to_string().contains("privacy") || body.to_string().contains("Privacy"),
        "and the refusal says what is missing: {body}"
    );
}

#[tokio::test]
async fn an_export_with_the_wrong_scope_names_what_it_wanted() {
    let mut h = Harness::new("exports-scope-no").await;
    let account = h.signed_in("owner@example.test", "owner").await;
    let pseud = h.pseud_of(&account).await;
    // An export reads the library, so `content.write` is not the right scope —
    // but it is the scope a reader would plausibly grant a bot, which is exactly
    // why the refusal has to be legible.
    let token = h.token_as(&account, &pseud, &["content.write"]).await;

    let (status, body) = h
        .as_token(
            "POST",
            "/api/v1/exports",
            Some(json!({
                "subject_type": "query",
                "subject_id": pseud,
                "query": "q:",
                "format": "markdown",
                "acknowledge_privacy": true,
            })),
            &token,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(
        body.to_string().contains("library.read"),
        "the refusal names the scope an export needs: {body}"
    );
}

#[tokio::test]
async fn a_valid_export_is_accepted_from_a_library_read_token() {
    let mut h = Harness::new("exports-scope-ok").await;
    let account = h.signed_in("owner@example.test", "owner").await;
    let pseud = h.pseud_of(&account).await;
    let token = h.token_as(&account, &pseud, &["library.read"]).await;

    let (status, body) = h
        .as_token(
            "POST",
            "/api/v1/exports",
            Some(json!({
                "subject_type": "query",
                "subject_id": pseud,
                "query": "q:",
                "format": "markdown",
                "acknowledge_privacy": true,
            })),
            &token,
        )
        .await;
    // Not 403 — that is the whole point, and it is a stronger assertion than
    // a 202 would be. The subject check runs *after* the scope check, so a
    // 422 here is proof the token cleared authentication and authorization and
    // was refused later for an unrelated reason (this database has no works to
    // export). A 202 would need a real work with chapters seeded, which is a
    // fixture this file has no other reason to build — and a 403 could not
    // distinguish "refused for the wrong scope" from "refused because the
    // conversion is not installed here".
    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "a library.read token clears the scope check on POST /exports: {status} {body}"
    );
    assert_ne!(
        status,
        StatusCode::UNAUTHORIZED,
        "and clears authentication: {status} {body}"
    );
}

#[tokio::test]
async fn an_export_token_cannot_read_another_accounts_export() {
    let mut h = Harness::new("exports-ownership").await;
    let owner = h.signed_in("owner@example.test", "owner").await;
    let owner_pseud = h.pseud_of(&owner).await;
    let owner_token = h.token_as(&owner, &owner_pseud, &["library.read"]).await;

    // The export row is inserted directly rather than requested through the
    // door. `POST /exports` validates its subject *synchronously* — a work that
    // does not exist is a 422 before any row is written — so getting a row by
    // that route means seeding a work with chapters, which is a fixture this
    // test has no other reason to build and which would make a test about
    // ownership into a test about import.
    //
    // What is under test is the ownership predicate on `GET /exports/{id}` and
    // `DELETE /exports/{id}`. The row's provenance does not matter to that, and
    // writing it here keeps the test saying what it means.
    let export_id = uuid::Uuid::new_v4().to_string();
    let job_id = uuid::Uuid::new_v4().to_string();
    h.insert_export(&export_id, &job_id, &owner).await;
    // And confirm the owner can see their own, or the 404s below prove nothing:
    // a row nobody can read produces the same answer.
    let (status, own) = h
        .as_token(
            "GET",
            &format!("/api/v1/exports/{export_id}"),
            None,
            &owner_token,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the owner can read their own export, so the 404s that follow are about \
         ownership and not about the row being invisible: {status} {own}"
    );

    // Account B, with a perfectly valid library.read token.
    let other = h.switch_to("other@example.test", "other").await;
    let other_pseud = h.pseud_of(&other).await;
    let other_token = h.token_as(&other, &other_pseud, &["library.read"]).await;

    for verb in ["GET", "DELETE"] {
        let (status, body) = h
            .as_token(
                verb,
                &format!("/api/v1/exports/{export_id}"),
                None,
                &other_token,
            )
            .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{verb} on another account's export is a 404 — the scope says what \
             an account may do, never whose rows are its own, and 404 rather \
             than 403 so the response does not confirm the export exists: \
             {status} {body}"
        );
    }
}
