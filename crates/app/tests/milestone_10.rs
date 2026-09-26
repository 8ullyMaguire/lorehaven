//! M10 — Search, taxonomy, body search, query language.
//!
//! Drives the real router against a real SQLite file.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_app::worker::{Worker, WorkerOptions};
use lorehaven_db::{Backend, DatabaseConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m10-{tag}-{}-{:?}",
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
    // Raise rate limits for parallel test execution.
    config.rate_limits.auth = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.write = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.search = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.default = lorehaven_app::limiter::Quota {
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
            let Ok(text) = value.to_str() else { continue };
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
        let report: Vec<String> = tdb.applied_migrations().to_vec();
        assert!(
            report.contains(&"0011_taxonomy".to_owned()),
            "taxonomy migration must apply: {report:?}"
        );
        Self { dir, tdb }
    }
    fn client(&self) -> Client {
        Client::new(server::build_router(AppState::new(
            config_for(&self.dir),
            self.tdb.db().clone(),
        )))
    }
    async fn cleanup(self) {
        self.tdb.cleanup().await;
        let _ = std::fs::remove_dir_all(self.dir);
    }
}

const PASSWORD: &str = "a-long-enough-passphrase";

async fn register(client: &mut Client, email: &str, handle: &str) -> (String, String) {
    let (status, body) = client
        .post(
            "/api/v1/auth/register",
            json!({
                "email": email,
                "password": PASSWORD,
                "handle": handle,
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

async fn published_work(harness: &Harness, email: &str, handle: &str, title: &str) -> String {
    let mut author = harness.client();
    let _ = register(&mut author, email, handle).await;
    let (status, body) = author
        .post("/api/v1/works", json!({ "title": title }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    let work_id = body["id"].as_str().expect("id").to_owned();
    let work_version = body["version"].as_i64().expect("version");
    let (status, body) = author
        .post(
            &format!("/api/v1/works/{work_id}/chapters"),
            json!({ "title": "One" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "chapter: {body}");
    let chapter = body["id"].as_str().expect("chapter").to_owned();
    let chapter_version = body["version"].as_i64().expect("version");
    let doc = json!({ "type": "doc", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "A chapter with enough words to have a middle." }] }] });
    let (status, body) = author
        .request(
            "PATCH",
            &format!("/api/v1/chapters/{chapter}"),
            Some(json!({ "expected_version": chapter_version, "document": doc })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "save: {body}");
    let (status, body) = author
        .post(
            &format!("/api/v1/works/{work_id}/publish"),
            json!({ "expected_version": work_version, "idempotency_key": format!("m10-{work_id}") }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "publish: {body}");
    work_id
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn search_empty_query_returns_all_public_works() {
    let harness = Harness::new("search-empty").await;
    let _ = published_work(&harness, "a@example.com", "AuthorA", "Empty Query Work").await;
    let mut client = harness.client();
    let (status, body) = client.get("/api/v1/search?q=").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{body}");
    harness.cleanup().await;
}

#[tokio::test]
async fn search_by_title_finds_matching_work() {
    let harness = Harness::new("search-title").await;
    let _ = published_work(&harness, "a@example.com", "AuthorA", "Winter Journey").await;
    let _ = published_work(&harness, "b@example.com", "AuthorB", "Summer Tale").await;
    let mut client = harness.client();
    let (status, body) = client.get("/api/v1/search?q=winter").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert!(!items.is_empty(), "expected match for 'winter': {body}");
    assert!(
        items
            .iter()
            .any(|i| i["title"].as_str().unwrap().contains("Winter")),
        "{body}"
    );
    harness.cleanup().await;
}

#[tokio::test]
async fn search_fielded_fandom_uses_exists() {
    let harness = Harness::new("search-fielded").await;
    let work_id = published_work(&harness, "a@example.com", "AuthorA", "Tagged Work").await;
    let mut client = harness.client();
    let _ = register(&mut client, "b@example.com", "AuthorB").await;
    // Create a fandom node.
    let (status, body) = client
        .post(
            "/api/v1/taxonomy",
            json!({ "kind": "fandom", "canonical": "HarryPotter" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "create node: {body}");
    let node_id = body["node"]["id"].as_str().expect("id");
    // Tag the work.
    let (status, _) = client
        .post(
            &format!("/api/v1/works/{work_id}/tags"),
            json!({ "node_id": node_id }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "tag work");
    // Search by fandom.
    let (status, body) = client.get("/api/v1/search?q=fandom:harrypotter").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert!(
        !items.is_empty(),
        "expected match for fandom:harrypotter: {body}"
    );
    harness.cleanup().await;
}

#[tokio::test]
async fn search_anonymous_cannot_see_drafts() {
    let harness = Harness::new("search-anon-draft").await;
    // Create a draft (unpublished) work.
    let mut author = harness.client();
    let _ = register(&mut author, "a@example.com", "AuthorA").await;
    let (status, body) = author
        .post("/api/v1/works", json!({ "title": "Draft Work" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create draft: {body}");
    let draft_id = body["id"].as_str().expect("id");

    // Anonymous search should not find the draft.
    let mut anon = harness.client();
    let (status, body) = anon.get("/api/v1/search?q=draft").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert!(
        !items.iter().any(|i| i["work_id"] == draft_id),
        "anonymous should not see drafts: {body}"
    );
    harness.cleanup().await;
}

#[tokio::test]
async fn search_in_work_returns_paragraph_positions() {
    let harness = Harness::new("search-in-work").await;
    let _ = published_work(&harness, "a@example.com", "AuthorA", "Searchable Work").await;
    let mut client = harness.client();
    // First find the work.
    let (status, body) = client.get("/api/v1/search?q=Searchable").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert!(!items.is_empty(), "expected match: {body}");
    let work_id = items[0]["work_id"].as_str().unwrap().to_owned();

    // Search within the work.
    let (status, body) = client
        .get(&format!("/api/v1/search/in-work/{work_id}?q=chapter"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // The body is an array of InWorkMatch objects.
    assert!(body.is_array(), "expected array: {body}");
    harness.cleanup().await;
}

#[tokio::test]
async fn taxonomy_autocomplete_returns_nodes() {
    let harness = Harness::new("taxonomy-autocomplete").await;
    let mut client = harness.client();
    let _ = register(&mut client, "a@example.com", "AuthorA").await;

    // Create some nodes.
    let (status, _) = client
        .post(
            "/api/v1/taxonomy",
            json!({ "kind": "fandom", "canonical": "Harry Potter" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "create harry potter");

    let (status, _) = client
        .post(
            "/api/v1/taxonomy",
            json!({ "kind": "fandom", "canonical": "Lord of the Rings" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "create lotr");

    // Autocomplete by prefix.
    let (status, body) = client
        .get("/api/v1/taxonomy?kind=fandom&prefix=harry")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert!(!items.is_empty(), "expected autocomplete matches: {body}");
    harness.cleanup().await;
}

#[tokio::test]
async fn taxonomy_autocomplete_fuzzy_matches_typo() {
    let harness = Harness::new("taxonomy-fuzzy-autocomplete").await;
    let mut client = harness.client();
    let _ = register(&mut client, "a@example.com", "AuthorA").await;

    // Create a node with a typo-prone name.
    let (status, _) = client
        .post(
            "/api/v1/taxonomy",
            json!({ "kind": "fandom", "canonical": "Harry Potter" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "create harry potter");

    // Query with a typo: "harry poter" (missing second 't').
    let (status, body) = client
        .get("/api/v1/taxonomy?kind=fandom&prefix=harry%20poter")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items array");
    let norms: Vec<&str> = items.iter().filter_map(|i| i["norm"].as_str()).collect();
    assert!(
        norms.iter().any(|n| n.contains("harry potter")),
        "expected fuzzy match for 'harry poter', got: {norms:?}"
    );
    harness.cleanup().await;
}

#[tokio::test]
async fn search_is_deterministic_for_same_input() {
    let harness = Harness::new("search-deterministic").await;
    let _ = published_work(&harness, "a@example.com", "AuthorA", "Deterministic Work").await;
    let mut client = harness.client();
    let (_, body1) = client.get("/api/v1/search?q=deterministic").await;
    let (_, body2) = client.get("/api/v1/search?q=deterministic").await;
    assert_eq!(body1, body2, "search should be deterministic");
    harness.cleanup().await;
}

#[tokio::test]
async fn search_with_no_query_returns_cursor_envelope() {
    let harness = Harness::new("search-envelope").await;
    let _ = published_work(&harness, "a@example.com", "AuthorA", "Envelope Work").await;
    let mut client = harness.client();
    let (status, body) = client.get("/api/v1/search").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.get("items").is_some(), "expected items field: {body}");
    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// P1-A regression: /api/v1/public/works/{id} must not leak draft content
// ---------------------------------------------------------------------------

#[tokio::test]
async fn public_work_draft_returns_404_to_anonymous() {
    let harness = Harness::new("public-draft-anon").await;
    let mut author = harness.client();
    let _ = register(&mut author, "a@example.com", "AuthorA").await;

    // Create a draft (unpublished) work.
    let (status, body) = author
        .post("/api/v1/works", json!({ "title": "Secret Draft" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create draft: {body}");
    let draft_id = body["id"].as_str().expect("id");

    // Anonymous must get 404, not 200 with the title.
    let mut anon = harness.client();
    let (status, body) = anon.get(&format!("/api/v1/public/works/{draft_id}")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "anonymous must not read draft: {body}"
    );
    assert!(
        body.get("title").is_none(),
        "draft title must not leak: {body}"
    );
    harness.cleanup().await;
}

#[tokio::test]
async fn public_work_draft_returns_404_to_other_account() {
    let harness = Harness::new("public-draft-other").await;
    let mut author = harness.client();
    let _ = register(&mut author, "a@example.com", "AuthorA").await;

    let (status, body) = author
        .post("/api/v1/works", json!({ "title": "Secret Draft" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create draft: {body}");
    let draft_id = body["id"].as_str().expect("id");

    // A different account must also get 404.
    let mut other = harness.client();
    let _ = register(&mut other, "b@example.com", "OtherB").await;
    let (status, body) = other.get(&format!("/api/v1/public/works/{draft_id}")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "other account must not read draft: {body}"
    );
    harness.cleanup().await;
}

#[tokio::test]
async fn public_work_draft_returns_200_to_author() {
    let harness = Harness::new("public-draft-author").await;
    let mut author = harness.client();
    let _ = register(&mut author, "a@example.com", "AuthorA").await;

    let (status, body) = author
        .post("/api/v1/works", json!({ "title": "My Draft" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create draft: {body}");
    let draft_id = body["id"].as_str().expect("id");

    // The author (contributor) must be able to read their own draft.
    let (status, body) = author
        .get(&format!("/api/v1/public/works/{draft_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "author must read own draft: {body}");
    assert_eq!(body["work"]["title"], "My Draft");
    harness.cleanup().await;
}

#[tokio::test]
async fn public_work_published_returns_200_to_anonymous() {
    let harness = Harness::new("public-published-anon").await;
    let work_id = published_work(&harness, "a@example.com", "AuthorA", "Public Work").await;

    let mut anon = harness.client();
    let (status, body) = anon.get(&format!("/api/v1/public/works/{work_id}")).await;
    assert_eq!(status, StatusCode::OK, "anon read published: {body}");
    assert_eq!(body["work"]["title"], "Public Work");
    harness.cleanup().await;
}

#[tokio::test]
async fn public_work_unknown_id_returns_404() {
    let harness = Harness::new("public-unknown").await;
    let mut anon = harness.client();
    let (status, body) = anon
        .get("/api/v1/public/works/00000000-0000-0000-0000-000000000000")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unknown id must 404: {body}");
    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Public search (the anonymous door)
// ---------------------------------------------------------------------------

/// Put a term in the index without running the worker.
///
/// The index is filled by a `Reindex` job, so a test that wants to ask what the
/// search door does with an indexed work has to write the row itself. Both
/// dialects, because the predicate under test is written twice.
async fn seed_index_term(harness: &Harness, work_id: &str, term: &str) {
    let sql = harness.tdb.db().sql(
        "INSERT INTO works_index_terms (work_id, term, pos) VALUES (?, ?, 0)",
        "INSERT INTO works_index_terms (work_id, term, pos) VALUES ($1::uuid, $2, 0)",
    );
    match harness.tdb.db().backend() {
        Backend::Sqlite => {
            sqlx::query(sql.as_ref())
                .bind(work_id)
                .bind(term)
                .execute(harness.tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("seed index term");
        }
        Backend::Postgres => {
            sqlx::query(sql.as_ref())
                .bind(work_id)
                .bind(term)
                .execute(harness.tdb.db().postgres_pool().expect("postgres"))
                .await
                .expect("seed index term");
        }
    }
}

/// Set a work's visibility without going through the route that would also
/// enqueue its deindex event: the point is the state where the index still
/// holds a work the public may no longer see.
async fn set_visibility(harness: &Harness, work_id: &str, visibility: &str) {
    let sql = harness.tdb.db().sql(
        "UPDATE works SET visibility = ? WHERE id = ?",
        "UPDATE works SET visibility = $1 WHERE id = $2::uuid",
    );
    match harness.tdb.db().backend() {
        Backend::Sqlite => {
            sqlx::query(sql.as_ref())
                .bind(visibility)
                .bind(work_id)
                .execute(harness.tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("set visibility");
        }
        Backend::Postgres => {
            sqlx::query(sql.as_ref())
                .bind(visibility)
                .bind(work_id)
                .execute(harness.tdb.db().postgres_pool().expect("postgres"))
                .await
                .expect("set visibility");
        }
    }
}

/// A work with a chapter, left unpublished, carrying a distinctive word in the
/// text so the index row for it is unambiguous.
async fn draft_with_marker(client: &mut Client, marker: &str) -> String {
    let (status, body) = client
        .post(
            "/api/v1/works",
            json!({ "title": format!("{marker} draft") }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create draft: {body}");
    let work_id = body["id"].as_str().expect("id").to_owned();
    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/chapters"),
            json!({ "title": "One" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "chapter: {body}");
    let chapter = body["id"].as_str().expect("chapter").to_owned();
    let version = body["version"].as_i64().expect("version");
    let doc = json!({ "type": "doc", "content": [{ "type": "paragraph", "content": [
        { "type": "text", "text": format!("A {marker} grazes in the margins.") }] }] });
    let (status, body) = client
        .request(
            "PATCH",
            &format!("/api/v1/chapters/{chapter}"),
            Some(json!({ "expected_version": version, "document": doc })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "save draft chapter: {body}");
    work_id
}

fn titles(body: &Value) -> Vec<String> {
    body["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|r| r["title"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// The index is not a visibility boundary: the worker fills it from chapter
/// text whatever the work's lifecycle, and a deindex can lag or be overtaken by
/// a `Reindex` that lands after a withdrawal. The anonymous door is what keeps
/// an unpublished work out of the results (spec §3.3).
#[tokio::test]
async fn public_search_does_not_serve_a_draft_the_index_holds() {
    let harness = Harness::new("public-search-draft").await;
    let mut author = harness.client();
    let _ = register(&mut author, "d@example.com", "DraftAuthor").await;
    let work_id = draft_with_marker(&mut author, "zebracorn").await;
    seed_index_term(&harness, &work_id, "zebracorn").await;

    let mut anon = harness.client();
    let (status, body) = anon.get("/api/v1/public/search?q=zebracorn").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        titles(&body).is_empty(),
        "an unpublished work must not be served by the public search: {body}"
    );
    harness.cleanup().await;
}

#[tokio::test]
async fn public_search_does_not_serve_a_restricted_work_the_index_holds() {
    let harness = Harness::new("public-search-restricted").await;
    let work_id = published_work(&harness, "e@example.com", "HiddenAuthor", "Quokkafish").await;
    seed_index_term(&harness, &work_id, "quokkafish").await;
    set_visibility(&harness, &work_id, "restricted").await;

    let mut anon = harness.client();
    let (status, body) = anon.get("/api/v1/public/search?q=quokkafish").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        titles(&body).is_empty(),
        "a work the public may not see must not be served: {body}"
    );
    harness.cleanup().await;
}

/// The positive control for the two above: a published, public work the index
/// holds is still found, and its word count is real rather than the unmaintained
/// `works.word_count` column's zero.
#[tokio::test]
async fn public_search_serves_a_published_public_work_from_the_index() {
    let harness = Harness::new("public-search-published").await;
    let work_id = published_work(
        &harness,
        "f@example.com",
        "FoundAuthor",
        "Lighthouse Letters",
    )
    .await;
    seed_index_term(&harness, &work_id, "lighthouse").await;

    let mut anon = harness.client();
    let (status, body) = anon.get("/api/v1/public/search?q=lighthouse").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        titles(&body),
        vec!["Lighthouse Letters".to_owned()],
        "{body}"
    );
    assert!(
        body["results"][0]["word_count"].as_i64().unwrap_or(0) > 0,
        "word_count must come from the live revisions, not works.word_count: {body}"
    );
    harness.cleanup().await;
}

/// A door bots call: no query is an empty result list, not a framework-shaped
/// plain-text 400.
#[tokio::test]
async fn public_search_without_a_query_returns_an_empty_list() {
    let harness = Harness::new("public-search-noquery").await;
    let mut anon = harness.client();
    let (status, body) = anon.get("/api/v1/public/search").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(titles(&body).len(), 0, "{body}");
    harness.cleanup().await;
}

/// The publish path, end to end: the `publish.index` topic handler enqueues a
/// reindex job whose payload is `{"work_id": "…"}`, the worker runs it, the term
/// index fills, and the anonymous search finds the work.
///
/// This is the join between two files, and it was broken: the worker parsed the
/// reindex payload as a bare string, so every publish-time reindex failed
/// ("invalid type: map, expected a string") and a fresh instance served an empty
/// index for every query — the search doors looked implemented and answered
/// nothing.
#[tokio::test]
async fn publishing_indexes_the_work_so_the_public_search_can_find_it() {
    let harness = Harness::new("public-search-publish").await;
    let work_id =
        published_work(&harness, "g@example.com", "IndexedAuthor", "Indexed Byline").await;

    lorehaven_db::jobs::enqueue(
        harness.tdb.db(),
        lorehaven_domain::jobs::JobKind::Reindex,
        &json!({ "work_id": work_id }).to_string(),
        None,
        None,
        5,
        &lorehaven_domain::jobs::RetryPolicy::default(),
    )
    .await
    .expect("enqueue reindex");

    let state = AppState::new(config_for(&harness.dir), harness.tdb.db().clone());
    let report = Worker::new(WorkerOptions::default())
        .run_once(&state)
        .await
        .expect("worker pass");
    let (_, outcome) = report.job.expect("the reindex job must have run");
    assert_eq!(
        outcome,
        lorehaven_domain::jobs::JobState::Succeeded,
        "a publish-time reindex must not fail"
    );

    let mut anon = harness.client();
    let (status, body) = anon.get("/api/v1/public/search?q=chapter").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        titles(&body),
        vec!["Indexed Byline".to_owned()],
        "the indexed work must be findable: {body}"
    );
    harness.cleanup().await;
}

/// M47-04: server-side content-filter enforcement on search — blocked tag never
/// reaches the client. Inserts a taxonomy node and work_tag so the filter can
/// match a real tag relationship.
#[tokio::test]
async fn search_respects_content_filters() {
    let harness = Harness::new("search-content-filter").await;

    // Create two works.
    let alpha_id = published_work(&harness, "a@example.com", "AuthorA", "Alpha Work").await;
    let _beta_id = published_work(&harness, "b@example.com", "AuthorB", "Beta Work").await;

    // Insert a tag node and tag the Alpha work directly.
    let now = chrono::Utc::now().to_rfc3339();
    let node_id = uuid::Uuid::new_v4().to_string();
    let db = harness.tdb.db();
    let sql_node = db.sql(
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at) VALUES (?, ?, ?, ?, ?)",
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at) VALUES ($1::uuid, $2, $3, $4, $5::timestamptz)",
    );
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(&sql_node)
                .bind(&node_id)
                .bind("tag")
                .bind("enemies to lovers")
                .bind("enemies to lovers")
                .bind(&now)
                .execute(db.sqlite_pool().unwrap())
                .await
                .unwrap();
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(&sql_node)
                .bind(&node_id)
                .bind("tag")
                .bind("enemies to lovers")
                .bind("enemies to lovers")
                .bind(&now)
                .execute(db.postgres_pool().unwrap())
                .await
                .unwrap();
        }
    }
    let sql_tag = db.sql(
        "INSERT INTO work_tags (work_id, node_id, weight, added_at) VALUES (?, ?, ?, ?)",
        "INSERT INTO work_tags (work_id, node_id, weight, added_at) VALUES ($1::uuid, $2, $3, $4::timestamptz)",
    );
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(&sql_tag)
                .bind(&alpha_id)
                .bind(&node_id)
                .bind(1i64)
                .bind(&now)
                .execute(db.sqlite_pool().unwrap())
                .await
                .unwrap();
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(&sql_tag)
                .bind(&alpha_id)
                .bind(&node_id)
                .bind(1i64)
                .bind(&now)
                .execute(db.postgres_pool().unwrap())
                .await
                .unwrap();
        }
    }

    // Register a reader.
    let mut client = harness.client();
    register(&mut client, "reader@example.com", "Reader").await;

    // Block "enemies to lovers" tag.
    let (status, body) = client
        .post(
            "/api/v1/settings/content-filters",
            json!({ "filter_type": "tag", "value": "enemies to lovers" }),
        )
        .await;
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "add filter: {status} {body}"
    );

    // Search: only Beta should appear (Alpha has the blocked tag).
    let (status, body) = client.get("/api/v1/search?q=Work").await;
    assert_eq!(status, StatusCode::OK, "search with filter: {body}");
    let titles: Vec<String> = body["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|i| i["title"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        titles,
        vec!["Beta Work".to_owned()],
        "blocked tag must not appear in results: {body}"
    );

    harness.cleanup().await;
}
/// Content filters apply *before* paging, not after (spec §46.4, §46.7.1).
///
/// The sibling test `search_respects_content_filters` seeds two works and asks
/// for a one-item page, so both a correct implementation and one that filters
/// after the `LIMIT` answer with the single unblocked work. It cannot tell them
/// apart. This one is built so they differ.
///
/// Ordering is `score DESC, updated_at DESC`, and the term index is empty for
/// these works, so every score is 0 and the effective order is newest-first.
/// The blocked works are therefore created *last*, so they occupy the front of
/// the unfiltered page. With `limit=5`:
///
///   * filtering before the limit returns the five unblocked works;
///   * filtering after it returns nothing, because the first five rows are all
///     blocked and the filter then removes every one of them.
///
/// The old implementation returned the empty list.
#[tokio::test]
async fn content_filters_narrow_the_page_instead_of_shortening_it() {
    const UNBLOCKED: usize = 5;
    const BLOCKED: usize = 7;
    const PAGE: i64 = 5;

    let harness = Harness::new("search-content-filter-paging").await;

    // Unblocked first, so they sort *after* the blocked works below.
    for i in 0..UNBLOCKED {
        published_work(
            &harness,
            &format!("keep{i}@example.com"),
            &format!("KeepAuthor{i}"),
            &format!("Kept Work {i}"),
        )
        .await;
    }
    let mut blocked_ids = Vec::new();
    for i in 0..BLOCKED {
        blocked_ids.push(
            published_work(
                &harness,
                &format!("drop{i}@example.com"),
                &format!("DropAuthor{i}"),
                &format!("Dropped Work {i}"),
            )
            .await,
        );
    }

    // One taxonomy node, linked to every blocked work.
    let now = chrono::Utc::now().to_rfc3339();
    let node_id = uuid::Uuid::new_v4().to_string();
    let db = harness.tdb.db();
    let sql_node = db.sql(
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at) VALUES (?, ?, ?, ?, ?)",
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at) VALUES ($1::uuid, $2, $3, $4, $5::timestamptz)",
    );
    let sql_tag = db.sql(
        "INSERT INTO work_tags (work_id, node_id, weight, added_at) VALUES (?, ?, ?, ?)",
        "INSERT INTO work_tags (work_id, node_id, weight, added_at) VALUES ($1::uuid, $2, $3, $4::timestamptz)",
    );

    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(&sql_node)
                .bind(&node_id)
                .bind("tag")
                .bind("slow burn")
                .bind("slow burn")
                .bind(&now)
                .execute(db.sqlite_pool().unwrap())
                .await
                .unwrap();
            for work_id in &blocked_ids {
                sqlx::query(&sql_tag)
                    .bind(work_id)
                    .bind(&node_id)
                    .bind(1i64)
                    .bind(&now)
                    .execute(db.sqlite_pool().unwrap())
                    .await
                    .unwrap();
            }
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(&sql_node)
                .bind(&node_id)
                .bind("tag")
                .bind("slow burn")
                .bind("slow burn")
                .bind(&now)
                .execute(db.postgres_pool().unwrap())
                .await
                .unwrap();
            for work_id in &blocked_ids {
                sqlx::query(&sql_tag)
                    .bind(work_id)
                    .bind(&node_id)
                    .bind(1i64)
                    .bind(&now)
                    .execute(db.postgres_pool().unwrap())
                    .await
                    .unwrap();
            }
        }
    }

    // Site search answers `items`; the `titles` helper above reads `results`,
    // which is the in-work search's envelope.
    let item_titles = |body: &Value| -> Vec<String> {
        body["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|i| i["title"].as_str().unwrap_or_default().to_owned())
            .collect()
    };

    // Baseline: an anonymous search sees a full page, and it is the blocked
    // works. Without this the filtered assertion could pass for the wrong
    // reason -- an empty index would also answer with the right count.
    let mut anon = harness.client();
    let (status, body) = anon.get("/api/v1/search?q=Work&limit=5").await;
    assert_eq!(status, StatusCode::OK, "baseline search: {body}");
    let baseline = item_titles(&body);
    assert_eq!(
        baseline.len(),
        PAGE as usize,
        "baseline page must be full: {body}"
    );
    assert!(
        baseline.iter().all(|t| t.starts_with("Dropped Work")),
        "the newest works must lead the unfiltered page, or this test proves \
         nothing about ordering: {baseline:?}"
    );

    // A reader who blocks the tag.
    let mut client = harness.client();
    register(&mut client, "reader@example.com", "Reader").await;
    let (status, body) = client
        .post(
            "/api/v1/settings/content-filters",
            json!({ "filter_type": "tag", "value": "slow burn" }),
        )
        .await;
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "add filter: {status} {body}"
    );

    // Filtered: a full page of the unblocked works, not the empty page that
    // post-limit filtering produces.
    let (status, body) = client.get("/api/v1/search?q=Work&limit=5").await;
    assert_eq!(status, StatusCode::OK, "filtered search: {body}");
    let returned = item_titles(&body);
    assert_eq!(
        returned.len(),
        PAGE as usize,
        "the page must be full after filtering, not short: {returned:?}"
    );
    assert!(
        returned.iter().all(|t| t.starts_with("Kept Work")),
        "no blocked work may reach the client: {returned:?}"
    );

    harness.cleanup().await;
}

/// Export/import round-trip (M47-08, §46.4).
#[tokio::test]
async fn export_import_round_trip() {
    let harness = Harness::new("settings-export-import").await;
    let mut client = harness.client();
    let (_account_id, _pseud_id) = register(&mut client, "sarah@example.com", "sarah").await;

    // Set a search preference.
    let (status, _) = client
        .request(
            "PATCH",
            "/api/v1/settings/search",
            Some(json!({ "changes": [{ "key": "search.default_sort", "value": "relevance" }] })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // Add a content filter.
    let (status, _) = client
        .post(
            "/api/v1/settings/content-filters",
            json!({ "filter_type": "tag", "value": "spoilers" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // Set a notification route.
    let (status, _) = client
        .request("PATCH", "/api/v1/settings/notifications", Some(json!({ "changes": [{ "event_type": "gift", "channel": "email", "enabled": true }] })))
        .await;
    assert_eq!(status, StatusCode::OK);

    // Export.
    let (status, body) = client.get("/api/v1/settings/export").await;
    assert_eq!(status, StatusCode::OK, "export: {body}");
    let export = body;
    assert_eq!(
        export["version"].as_u64().unwrap(),
        1u64,
        "export version must be 1"
    );
    assert!(
        export["namespaces"]["search"].as_array().unwrap().len() == 1,
        "one search setting"
    );
    assert!(
        export["namespaces"]["content_filters"]
            .as_array()
            .unwrap()
            .len()
            == 1,
        "one content filter"
    );
    assert!(
        export["namespaces"]["notifications"]
            .as_array()
            .unwrap()
            .len()
            == 1,
        "one notification route"
    );

    // Import into a fresh account.
    let mut tom_client = harness.client();
    let (_tom_id, _tom_pseud) = register(&mut tom_client, "tom@example.com", "tom").await;

    let (status, body) = tom_client
        .post("/api/v1/settings/import", json!({ "data": export }))
        .await;
    assert_eq!(status, StatusCode::OK, "import: {body}");
    assert_eq!(
        body["accepted"].as_array().unwrap().len(),
        3,
        "all three settings imported: {body}"
    );
    assert!(
        body["rejected"].as_array().unwrap().is_empty(),
        "no rejections: {body}"
    );

    // Verify settings landed.
    let (status, body) = tom_client.get("/api/v1/settings/search").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["settings"][0]["value"].as_str(),
        Some("relevance"),
        "imported search setting must be visible: {body}"
    );

    // Reject unsupported version.
    let (status, _) = tom_client
        .post(
            "/api/v1/settings/import",
            json!({
                "data": {
                    "version": 99,
                    "exported_at": "2026-01-01T00:00:00Z",
                    "namespaces": {}
                }
            }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "unsupported version must 400"
    );

    harness.cleanup().await;
}

/// A content filter has to bind every surface, not just search.
///
/// Spec §46.4 says filters are enforced "on every query, feed, recommendation,
/// and notification pipeline" and §46.7.1 restates it as an invariant. Until
/// this, `list_content_filters` was called from exactly one place -- the search
/// route -- so a reader who blocked a tag still saw it in `/api/v1/discovery`,
/// served by three recommendation engines.
///
/// This exercises the two that read the works table. The third
/// (`media_reference_collaborative_recommendations`) correlates on
/// `work_media_references.work_id` rather than on `works`, and is covered in
/// `media_ref_collab.rs`, because proving it needs a media reference and a
/// bookmark to hang one off.
///
/// The reader is signed in, otherwise they have no filters to apply and the
/// test would pass for the wrong reason.
#[tokio::test]
async fn content_filters_reach_the_recommendation_surface() {
    let harness = Harness::new("discovery-content-filter").await;

    // Three published works. `public_recommendations` orders by `updated_at
    // DESC`, so all of them would be returned unfiltered.
    let mut ids = Vec::new();
    for i in 0..3 {
        ids.push(
            published_work(
                &harness,
                &format!("disc{i}@example.com"),
                &format!("DiscAuthor{i}"),
                &format!("Discoverable {i}"),
            )
            .await,
        );
    }

    // Tag two of them; the third stays clean.
    let now = chrono::Utc::now().to_rfc3339();
    let node_id = uuid::Uuid::new_v4().to_string();
    let db = harness.tdb.db();
    let sql_node = db.sql(
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at) VALUES (?, ?, ?, ?, ?)",
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at) VALUES ($1::uuid, $2, $3, $4, $5::timestamptz)",
    );
    let sql_tag = db.sql(
        "INSERT INTO work_tags (work_id, node_id, weight, added_at) VALUES (?, ?, ?, ?)",
        "INSERT INTO work_tags (work_id, node_id, weight, added_at) VALUES ($1::uuid, $2, $3, $4::timestamptz)",
    );
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(&sql_node)
                .bind(&node_id)
                .bind("tag")
                .bind("spoilers")
                .bind("spoilers")
                .bind(&now)
                .execute(db.sqlite_pool().unwrap())
                .await
                .unwrap();
            for work_id in ids.iter().take(2) {
                sqlx::query(&sql_tag)
                    .bind(work_id)
                    .bind(&node_id)
                    .bind(1i64)
                    .bind(&now)
                    .execute(db.sqlite_pool().unwrap())
                    .await
                    .unwrap();
            }
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(&sql_node)
                .bind(&node_id)
                .bind("tag")
                .bind("spoilers")
                .bind("spoilers")
                .bind(&now)
                .execute(db.postgres_pool().unwrap())
                .await
                .unwrap();
            for work_id in ids.iter().take(2) {
                sqlx::query(&sql_tag)
                    .bind(work_id)
                    .bind(&node_id)
                    .bind(1i64)
                    .bind(&now)
                    .execute(db.postgres_pool().unwrap())
                    .await
                    .unwrap();
            }
        }
    }

    // The reader registers on one client and must use *that* client for the
    // filter call -- `harness.client()` makes a fresh unauthenticated one, which
    // is a 401 and a test that passes for the wrong reason.
    let mut reader = harness.client();
    let (_account, pseud) = register(&mut reader, "disc@reader.example", "DiscReader").await;
    let pseud_id: uuid::Uuid = pseud.parse().expect("pseud id is a uuid");
    // The engines take the *pseud*, not the account: a filter belongs to a
    // pseud, and which pseud a session acts as is `sessions.active_pseud_id`.
    // The route resolves it the same way, so the filter set these engines read is
    // the set the reader just wrote.
    // The engines return `WorkId`; the helper hands back the wire-form string, so
    // compare as strings rather than re-parsing what is already in hand.
    let as_strings = |ids: &[lorehaven_domain::ids::WorkId]| -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    };
    let all = as_strings(
        &lorehaven_db::discovery::public_recommendations(db, Some(pseud_id), 50)
            .await
            .expect("unfiltered recommendations"),
    );
    for work_id in &ids {
        assert!(
            all.contains(work_id),
            "work {work_id} should appear before the filter exists"
        );
    }

    // Now block the tag.
    let (status, body) = reader
        .post(
            "/api/v1/settings/content-filters",
            json!({ "filter_type": "tag", "value": "spoilers" }),
        )
        .await;
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "add content filter: {status} {body}"
    );

    let filtered = as_strings(
        &lorehaven_db::discovery::public_recommendations(db, Some(pseud_id), 50)
            .await
            .expect("filtered recommendations"),
    );
    for work_id in ids.iter().take(2) {
        assert!(
            !filtered.contains(work_id),
            "blocked work {work_id} reached the recommendation surface"
        );
    }
    assert!(
        filtered.contains(&ids[2]),
        "the untagged work must still be recommended"
    );

    harness.cleanup().await;
}
