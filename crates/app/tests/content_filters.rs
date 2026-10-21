//! M5-04 — content warnings are visible, filterable, and never hidden.
//!
//! # What this proves
//!
//! * **A warning is information, never a gate on the whole archive** (spec
//!   §10.2). A reader who filters nothing sees everything, with the warnings
//!   attached; a reader who filters "graphic violence" sees a *filtered list*.
//! * **The filter is applied INSIDE the pool query**, before scoring (spec
//!   §12.7). "Filtering a finished result list returns a subset of a
//!   *ranking*, which is not the same as a ranking over a smaller pool."
//! * **The count reflects the filter.** A reader told there are forty works
//!   and shown twelve, with nothing saying which twelve, has been told a lie.
//! * **The filter is visible and reversible.** A reader who cannot see why a
//!   work is missing cannot undo the thing that hid it.
//! * **The filters are per reader**, in `content_filters`, and not a global
//!   setting: one reader's filter does not change another's list.
//!
//! # Why the SQL is the thing under test
//!
//! The card says the filter must be inside the pool query, and that is not
//! easy to assert from the outside — a filtered list looks the same whether
//! the filter was applied in SQL or in the route afterwards. So the test
//! asserts the thing that *differs*: the count. A query that filters after
//! the fact reports the count it had before filtering, because the count and
//! the list came from different queries.

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
        "darkhold-filters-{tag}-{}-{:?}",
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

    /// The `Cookie` header, the way a browser sends it: **one** header with
    /// every cookie in it.
    ///
    /// Sending one header per cookie is a mistake that hides itself: the
    /// server reads the *first* one, so the session cookie arrives and the
    /// rest do not. A browser joins them, and so does this.
    fn cookie_header(&self) -> Option<String> {
        if self.cookies.is_empty() {
            return None;
        }
        Some(
            self.cookies
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<String>>()
                .join("; "),
        )
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
        if let Some(cookie) = self.cookie_header() {
            builder = builder.header(header::COOKIE, cookie);
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

    /// A GET that returns the **raw body**, because the point of these tests
    /// is what the body says.
    async fn get_html(&mut self, uri: &str) -> (StatusCode, String) {
        let mut builder = Request::builder().method("GET").uri(uri);
        if let Some(cookie) = self.cookie_header() {
            builder = builder.header(header::COOKIE, cookie);
        }
        let response = self
            .app
            .clone()
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        self.capture(&response);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&body).into_owned())
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

    /// The same app with **no cookies**: a signed-out reader.
    fn clone_without_cookies(&self) -> Client {
        Client {
            app: self.app.clone(),
            cookies: Vec::new(),
        }
    }

    async fn register(&mut self, email: &str, handle: &str) {
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

/// Create a taxonomy node and return its `(id, norm)`.
///
/// The filter matches on the node's **normalised** name and the work's tag
/// points at the node's **id**, and the two are different things: the id is
/// an opaque identifier and the norm is the lowercase canonical. A filter
/// that carried the id would exclude nothing, and one that carried the
/// canonical would miss a node whose norm differs.
async fn tag_node(client: &mut Client, canonical: &str) -> (String, String) {
    let (status, body) = client
        .post(
            "/api/v1/taxonomy",
            json!({ "kind": "tag", "canonical": canonical }),
        )
        .await;
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "create the node: {status} {body}"
    );
    let node = &body["node"];
    let id = node["id"].as_str().expect("the node's id").to_owned();
    let norm = node["norm"].as_str().expect("the node's norm").to_owned();
    (id, norm)
}

/// Tag an existing work with a second node.
async fn tag_work(client: &mut Client, work_id: &str, node_id: &str) {
    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/tags"),
            json!({ "node_id": node_id, "weight": 1 }),
        )
        .await;
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "tag the work: {status} {body}"
    );
}

/// Publish a work carrying a tag, and return its `(work id, unit id)`.
async fn work_with_tag(client: &mut Client, title: &str, tag_node_id: &str) -> (String, String) {
    let (status, body) = client
        .post("/api/v1/works", json!({ "title": title }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create the work: {body}");
    let work_id = body["id"].as_str().expect("the work's id").to_owned();

    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/chapters"),
            json!({ "title": "A Unit" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create a unit: {body}");
    let unit_id = body["id"].as_str().expect("the unit's id").to_owned();

    let (status, body) = client
        .patch(
            &format!("/api/v1/chapters/{unit_id}"),
            json!({
                "expected_version": 1,
                "document": {
                    "type": "doc",
                    "content": [{
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "Some text." }],
                    }],
                },
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "write the unit: {body}");

    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/publish"),
            json!({ "expected_version": 1 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "publish the work: {body}");

    // The tag. The node is a real taxonomy node, so the id is a real id.
    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/tags"),
            json!({ "node_id": tag_node_id, "weight": 1 }),
        )
        .await;
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "tag the work: {status} {body}"
    );

    (work_id, unit_id)
}

#[tokio::test]
async fn a_filtered_listing_has_the_filtered_works_absent_and_the_count_agrees() {
    let harness = Harness::new("filtered").await;
    let mut client = harness.client();
    client.register("filer@example.test", "filer").await;

    // Two tags: the one the reader is **browsing by**, and the one they
    // **filter on**. Both works carry the browsing tag, and only one carries
    // the filtered one -- so the filter has something to remove without
    // emptying the listing.
    let (listing_node, listing_norm) = tag_node(&mut client, "listing-tag").await;
    let (filtered_node, filtered_norm) = tag_node(&mut client, "filtered-tag").await;

    let (kept, _kept_unit) = work_with_tag(&mut client, "A Work Kept", &listing_node).await;
    let (hidden, _hidden_unit) = work_with_tag(&mut client, "A Work Hidden", &listing_node).await;
    tag_work(&mut client, &hidden, &filtered_node).await;

    // **Before the filter**, both works are in the listing.
    let (status, body) = client.get(&format!("/api/v1/tags/{listing_norm}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["total"].as_i64(),
        Some(2),
        "two works to start: {body}"
    );

    // **The filter is added.**
    let (status, body) = client
        .post(
            "/api/v1/settings/content-filters",
            json!({ "filter_type": "tag", "value": filtered_norm }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "add the filter: {body}");

    // **The listing is filtered, and the COUNT agrees with it.**
    let (status, body) = client.get(&format!("/api/v1/tags/{listing_norm}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(
        items.len(),
        1,
        "the filtered work is absent from the list: {body}"
    );
    assert_eq!(
        items[0]["work_id"].as_str(),
        Some(kept.as_str()),
        "and the one that is left is the one that was not filtered: {body}"
    );
    assert_eq!(
        body["total"].as_i64(),
        Some(1),
        "**the count reflects the filter**, not the count before it: {body}"
    );

    // **The filter is visible**, so the reader can see why the work is gone.
    let filters = body["filters"].as_array().expect("the filters");
    assert_eq!(
        filters.len(),
        1,
        "the listing says which filters it applied: {body}"
    );
    assert_eq!(filters[0]["kind"].as_str(), Some("tag"));
    assert_eq!(filters[0]["value"].as_str(), Some(filtered_norm.as_str()));

    // **And it is reversible.** Clearing the filter restores the work.
    let (status, body) = client
        .delete(&format!(
            "/api/v1/settings/content-filters/tag/{filtered_norm}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "clear the filter: {body}");

    let (status, body) = client.get(&format!("/api/v1/tags/{listing_norm}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["total"].as_i64(),
        Some(2),
        "clearing the filter restores the works: {body}"
    );
    assert_eq!(
        body["items"].as_array().expect("items").len(),
        2,
        "and the list: {body}"
    );
}

#[tokio::test]
async fn a_warning_is_declared_on_the_work_and_shown_to_a_reader() {
    let harness = Harness::new("warning").await;
    let mut client = harness.client();
    client.register("warmer@example.test", "warmer").await;

    let (node_id, _norm) = tag_node(&mut client, "listing-tag").await;
    let (work_id, unit_id) = work_with_tag(&mut client, "A Work With A Warning", &node_id).await;

    // **The warning node.** A warning is taxonomy-typed, so the node is an
    // ordinary taxonomy node of kind `warning`.
    let (status, body) = client
        .post(
            "/api/v1/taxonomy",
            json!({ "kind": "warning", "canonical": "graphic-violence" }),
        )
        .await;
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "create the warning node: {status} {body}"
    );
    let warning_node = body["node"]["id"].as_str().expect("the node's id");

    // **The author declares it.**
    let (status, body) = client
        .post(
            &format!("/api/v1/works/{work_id}/warnings"),
            json!({
                "warning_node_id": warning_node,
                "severity": 2,
                "depiction": "on_page",
                "declaration": "declared",
                "spoiler_safe_note": "A fight, described.",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "declare the warning: {body}");
    assert_eq!(
        body["severity"].as_i64(),
        Some(2),
        "the severity is recorded as it was declared: {body}"
    );

    // **A reader sees the warning attached**, without asking for it and
    // without filtering anything (spec §10.2). The reader's PAGE carries it,
    // above the text -- a warning a reader has to go looking for is a warning
    // that protects nobody.
    let (status, page) = client
        .get_html(&format!("/works/{work_id}/units/{unit_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(
        page.contains("reader-surface__warnings"),
        "the reader's page carries the warnings: {page}"
    );
    assert!(
        page.contains("graphic-violence"),
        "and names the warning: {page}"
    );
    assert!(
        page.contains("(heavy)"),
        "with the severity in words, because a number is a scale nobody can \
         read: {page}"
    );
    assert!(
        page.contains("A fight, described."),
        "and the author's note for a reader who wants to avoid the detail: {page}"
    );
}

#[tokio::test]
async fn a_filter_is_one_readers_and_not_another_readers() {
    let harness = Harness::new("perreader").await;
    let mut client = harness.client();
    client
        .register("reader-one@example.test", "readerone")
        .await;

    let (node_id, norm) = tag_node(&mut client, "per-reader-test-tag").await;
    let (_work_id, _unit_id) = work_with_tag(&mut client, "A Work", &node_id).await;

    let (status, body) = client
        .post(
            "/api/v1/settings/content-filters",
            json!({ "filter_type": "tag", "value": norm }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // **A signed-out reader sees everything.** The filters are per reader, and
    // a reader who filters nothing sees everything (spec §10.2).
    let mut anonymous = client.clone_without_cookies();
    let (status, body) = anonymous.get(&format!("/api/v1/tags/{norm}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["total"].as_i64(),
        Some(1),
        "another reader's filter does not change this reader's list: {body}"
    );
    assert!(
        body["filters"].as_array().expect("filters").is_empty(),
        "and the filters are not shown either: {body}"
    );
}
