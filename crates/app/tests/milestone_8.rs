//! Milestone 8 acceptance tests (spec §16 as the plan numbers it; §14 in the
//! spec text).
//!
//! The plan's six named tests, plus the acceptance criteria around them that
//! would otherwise be asserted nowhere: that a shelf is scoped by ownership
//! rather than by its own flag, that a batch reports per item, that removing an
//! item and deleting its copy are different operations, and that a public saved
//! view cannot carry a filter only its owner can see.
//!
//! These run against the real router, a real SQLite file and a real storage
//! directory. Library items are seeded through `imports::upsert_library_item`,
//! which is the same repository call an import makes — so the rows these tests
//! assert on are the rows the product creates, not a fixture shaped to agree
//! with the assertions.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::storage::BlobStore;
use lorehaven_db::{imports, DatabaseConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m8-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn config_for(dir: &Path) -> Config {
    let mut config = Config::development_defaults();
    config.storage.root = dir.join("storage");
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

    async fn request(
        &mut self,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if !self.cookies.is_empty() {
            let header = self
                .cookies
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("; ");
            builder = builder.header(header::COOKIE, header);
        }
        if !matches!(method, "GET" | "HEAD" | "OPTIONS") {
            if let Some(token) = self.cookie("lorehaven_csrf") {
                builder = builder.header("x-csrf-token", token.to_owned());
            }
        }

        let request = match body {
            Some(value) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&value).expect("serialise")))
                .expect("request"),
            None => builder.body(Body::empty()).expect("request"),
        };

        let response = self.app.clone().oneshot(request).await.expect("response");
        let status = response.status();
        self.capture_cookies(&response);

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

    async fn patch(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("PATCH", uri, Some(body)).await
    }

    async fn put(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("PUT", uri, Some(body)).await
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
        let report: Vec<String> = tdb.applied_migrations().to_vec();
        assert!(
            report.contains(&"0009_library".to_owned()),
            "the library migration must apply: {report:?}"
        );

        Self { dir, tdb }
    }

    fn state(&self) -> AppState {
        AppState::new(config_for(&self.dir), self.tdb.db().clone())
    }

    fn client(&self) -> Client {
        Client::new(server::build_router(self.state()))
    }

    fn store(&self) -> BlobStore {
        BlobStore::new(self.dir.join("storage"))
    }

    async fn cleanup(self) {
        self.tdb.cleanup().await;
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const PASSWORD: &str = "a-long-enough-passphrase";

async fn register(client: &mut Client, email: &str, handle: &str) -> String {
    let (status, body) = client
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
    body["account"]["id"]
        .as_str()
        .expect("account id")
        .to_owned()
}

/// Seed a library item the way an import would.
async fn seed_item(harness: &Harness, account: &str, slug: &str, title: &str) -> String {
    let item = imports::upsert_library_item(
        harness.tdb.db(),
        account,
        "royalroad",
        slug,
        &imports::LibraryItemInput {
            title: title.to_owned(),
            author_text: "An Author".to_owned(),
            author_url: None,
            summary: "A summary.".to_owned(),
            language: Some("en".to_owned()),
            word_count: Some(1_000),
            status: "ongoing".to_owned(),
            source_url: format!("https://www.royalroad.com/fiction/{slug}"),
            source_updated_at: Some("2026-01-01T00:00:00Z".to_owned()),
            provenance_json: "{}".to_owned(),
        },
    )
    .await
    .expect("seed library item");
    item.id
}

async fn create_shelf(client: &mut Client, name: &str) -> String {
    let (status, body) = client
        .post("/api/v1/shelves", json!({ "name": name }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create shelf: {body}");
    body["id"].as_str().expect("shelf id").to_owned()
}

// ---------------------------------------------------------------------------
// A shelf is private, and is scoped by ownership rather than by its flag
// ---------------------------------------------------------------------------

/// Spec §14.1's shelves, and the two guarantees behind them: a new shelf is not
/// shared, and a shelf is addressed by its owner rather than by its identifier.
///
/// The second is the one that matters. A test that only checked the `is_public`
/// flag would pass on an implementation where any logged-in reader could read,
/// rename or fill any shelf — so this asserts that another account's requests
/// against the same identifier change nothing and reveal nothing.
#[tokio::test]
async fn a_shelf_is_private_until_it_is_published() {
    let harness = Harness::new("shelf-private").await;
    let mut owner = harness.client();
    let owner_id = register(&mut owner, "owner@example.com", "owner").await;
    let shelf_id = create_shelf(&mut owner, "Favourites").await;
    let item_id = seed_item(&harness, &owner_id, "1001", "A Work").await;

    // A new shelf is not shared, and it starts published to nobody.
    let (status, body) = owner.get("/api/v1/shelves").await;
    assert_eq!(status, StatusCode::OK, "list shelves: {body}");
    assert_eq!(body["items"][0]["is_public"], json!(false));
    assert_eq!(body["items"][0]["name"], json!("Favourites"));

    // Somebody else, on the same instance, sees nothing of it.
    let mut stranger = harness.client();
    register(&mut stranger, "stranger@example.com", "stranger").await;
    let (status, body) = stranger.get("/api/v1/shelves").await;
    assert_eq!(status, StatusCode::OK, "stranger shelves: {body}");
    assert_eq!(
        body["items"].as_array().map(Vec::len),
        Some(0),
        "a second account must not see the first account's shelf: {body}"
    );

    // And cannot reach it by identifier either — read, rename or fill.
    let (status, body) = stranger.get(&format!("/api/v1/shelves/{shelf_id}")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "another account's shelf must not resolve: {body}"
    );
    let (status, _) = stranger
        .patch(
            &format!("/api/v1/shelves/{shelf_id}"),
            json!({ "name": "Mine now", "expected_version": 1 }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "rename must not resolve");

    // Nor put their own item on it, nor the owner's item on a shelf of theirs:
    // the insert joins both ends against the account, so a mismatched pair
    // writes nothing.
    let (status, _) = stranger
        .post(
            &format!("/api/v1/shelves/{shelf_id}/items/{item_id}"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "placing must not resolve");

    // The owner can, and publishing is what makes the flag true — the shelf is
    // still theirs alone; the flag is the owner's statement of intent.
    let (status, _) = owner
        .post(
            &format!("/api/v1/shelves/{shelf_id}/items/{item_id}"),
            json!({}),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "the owner may place an item"
    );
    let (status, body) = owner.get(&format!("/api/v1/shelves/{shelf_id}")).await;
    assert_eq!(status, StatusCode::OK, "owner reads their shelf: {body}");
    assert_eq!(body["library_item_ids"], json!([item_id]));

    let (status, _) = owner
        .patch(
            &format!("/api/v1/shelves/{shelf_id}"),
            json!({ "is_public": true, "expected_version": 1 }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "publishing a shelf");
    let (_, body) = owner.get("/api/v1/shelves").await;
    assert_eq!(
        body["items"][0]["is_public"],
        json!(true),
        "publishing must be recorded: {body}"
    );

    // Deleting the shelf takes the placement with it and leaves the work in the
    // library (spec §14.1: "Deleting a shelf does not delete its works").
    let (status, _) = owner.delete(&format!("/api/v1/shelves/{shelf_id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = owner.get("/api/v1/library/items").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "library after shelf deletion: {body}"
    );
    assert_eq!(
        body["items"].as_array().map(Vec::len),
        Some(1),
        "the work must survive its shelf: {body}"
    );

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// A private tag is not a public tag
// ---------------------------------------------------------------------------

/// Spec §16's first pitfall, and the reason the private tags are their own table:
/// a tag one reader writes is invisible to every other reader, including when
/// they write the same word.
///
/// Asserted from both sides — the stranger's tag list, and a filtered listing —
/// because a single-table implementation with an `is_private` flag would pass
/// one of these and fail the other.
#[tokio::test]
async fn a_private_tag_is_not_a_public_tag() {
    let harness = Harness::new("private-tag").await;
    let mut alice = harness.client();
    let alice_id = register(&mut alice, "alice@example.com", "alice").await;
    let mut bob = harness.client();
    let bob_id = register(&mut bob, "bob@example.com", "bob").await;

    // Each reader has their own copy of the same work, and both tag it "wip".
    let alice_item = seed_item(&harness, &alice_id, "2001", "Alice's Copy").await;
    let bob_item = seed_item(&harness, &bob_id, "2002", "Bob's Copy").await;

    let (status, _) = alice
        .put(
            &format!("/api/v1/library/items/{alice_item}/tags/wip"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "alice tags her item");
    let (status, _) = bob
        .put(
            &format!("/api/v1/library/items/{bob_item}/tags/wip"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "bob tags his item");

    // Alice's tags are hers. Bob's word is the same word and not her row.
    let (status, body) = alice
        .get(&format!("/api/v1/library/items/{alice_item}/tags"))
        .await;
    assert_eq!(status, StatusCode::OK, "alice's tags: {body}");
    assert_eq!(body["items"], json!(["wip"]));

    let (status, body) = bob
        .get(&format!("/api/v1/library/items/{alice_item}/tags"))
        .await;
    assert_eq!(status, StatusCode::OK, "bob reading alice's item: {body}");
    assert_eq!(
        body["items"],
        json!([]),
        "a tag is not attached to the identifier, it is attached to the reader: {body}"
    );

    // The filter is scoped too: filtering by "wip" returns Alice's own work and
    // not Bob's, even though both are tagged with that word.
    let (status, body) = alice.get("/api/v1/library/items?tags=wip").await;
    assert_eq!(status, StatusCode::OK, "alice filtered: {body}");
    let titles: Vec<&str> = body["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|item| item["title"].as_str())
        .collect();
    assert_eq!(
        titles,
        vec!["Alice's Copy"],
        "a tag filter must not reach another reader's items: {body}"
    );

    // Untagging is per reader as well.
    let (status, _) = bob
        .delete(&format!("/api/v1/library/items/{alice_item}/tags/wip"))
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "bob has no tag on alice's item to remove"
    );
    let (_, body) = alice
        .get(&format!("/api/v1/library/items/{alice_item}/tags"))
        .await;
    assert_eq!(
        body["items"],
        json!(["wip"]),
        "bob's attempt must not have touched alice's tag"
    );

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Batch operations report per item
// ---------------------------------------------------------------------------

/// Spec §14.3's per-item outcome, and the plan's third pitfall.
///
/// Three items, two removed: the answer has to be a list and a count rather than
/// one boolean, and an identifier that is not this reader's has to be reported
/// as gone rather than as forbidden — distinguishing the two would confirm which
/// identifiers exist.
#[tokio::test]
async fn batch_delete_removes_only_the_selection() {
    let harness = Harness::new("batch-delete").await;
    let mut client = harness.client();
    let account = register(&mut client, "batch@example.com", "batch").await;

    let first = seed_item(&harness, &account, "3001", "First").await;
    let second = seed_item(&harness, &account, "3002", "Second").await;
    let third = seed_item(&harness, &account, "3003", "Third").await;

    // Somebody else's item, which this account must not be able to remove.
    let mut other = harness.client();
    let other_id = register(&mut other, "other@example.com", "other").await;
    let not_mine = seed_item(&harness, &other_id, "3004", "Not Mine").await;

    let (status, body) = client
        .post(
            "/api/v1/library/items/batch",
            json!({
                "ids": [first, second, not_mine],
                "delete_copy": false,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "batch remove: {body}");

    let succeeded: Vec<&str> = body["succeeded"]
        .as_array()
        .expect("succeeded")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        succeeded.len(),
        2,
        "exactly the two items this account owns: {body}"
    );
    assert!(succeeded.contains(&first.as_str()));
    assert!(succeeded.contains(&second.as_str()));

    let failed = body["failed"].as_array().expect("failed");
    assert_eq!(failed.len(), 1, "the third must be reported: {body}");
    assert_eq!(failed[0]["id"], json!(not_mine));
    // One code for "already deleted" and for "never yours": a batch that
    // distinguished them would tell a caller which identifiers exist.
    assert_eq!(failed[0]["code"], json!("NOT_FOUND"));

    // The summary is the sentence an interface shows, and it counts both halves.
    // The sentence names both counts and the total: "2 removed" alone would
    // hide the failure, and "1 failed" alone would hide that the other two
    // worked.
    assert_eq!(
        body["summary"],
        json!("2 of 3 removed; 1 could not be removed")
    );

    // The selection went and the rest stayed — including the other account's.
    let (_, body) = client.get("/api/v1/library/items").await;
    let titles: Vec<&str> = body["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|item| item["title"].as_str())
        .collect();
    assert_eq!(
        body["items"][0]["id"],
        json!(third),
        "the unselected item is the one that remains, and it is the same row: {body}"
    );
    assert_eq!(titles, vec!["Third"], "only the unselected item remains");
    let (_, body) = other.get("/api/v1/library/items").await;
    assert_eq!(
        body["items"].as_array().map(Vec::len),
        Some(1),
        "the other account's item must be untouched: {body}"
    );

    harness.cleanup().await;
}

/// Removing an item is not the same as deleting the copy it holds (spec §14's
/// acceptance list, "Removing an item distinguishes deleting a reference from
/// deleting a private copy").
#[tokio::test]
async fn removing_an_item_and_deleting_its_copy_are_different_operations() {
    let harness = Harness::new("removal-modes").await;
    let mut client = harness.client();
    let account = register(&mut client, "modes@example.com", "modes").await;

    // Two items, each holding one stored chapter of known size.
    let kept = seed_item(&harness, &account, "4001", "Keep The Copy").await;
    let deleted = seed_item(&harness, &account, "4002", "Delete The Copy").await;
    let store = harness.store();
    let mut sizes = Vec::new();
    let mut checksums = Vec::new();
    for (item, body) in [
        (&kept, b"kept".as_slice()),
        (&deleted, b"deleted now".as_slice()),
    ] {
        let (checksum, _key) = store
            .put(harness.tdb.db(), body, "text/plain")
            .await
            .expect("put");
        store
            .reference(harness.tdb.db(), &checksum, "library_item", item)
            .await
            .expect("reference");
        sizes.push(i64::try_from(body.len()).expect("size"));
        checksums.push(checksum);
    }
    let blobs_before = sizes.iter().sum::<i64>();

    let usage = lorehaven_db::library::storage_usage(harness.state().db(), &account)
        .await
        .expect("usage");
    assert_eq!(usage.imported_bytes, blobs_before, "both copies are stored");

    // Remove one as a reference only. The item and the reference to its copy
    // go; the bytes stay on disk, unreferenced, for the collector.
    let (status, body) = client
        .post(
            "/api/v1/library/items/batch",
            json!({ "ids": [kept], "delete_copy": false }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "reference-only removal: {body}");
    assert_eq!(
        body["freed_bytes"],
        json!(0),
        "nothing is freed *now*, so nothing may be reported as freed: {body}"
    );
    // The copy itself is still there — it is the *reference* that went, not the
    // bytes — and it is now unreferenced, which is what the maintenance sweep
    // collects.
    assert_eq!(
        store
            .stat(harness.tdb.db(), &checksums[0])
            .await
            .expect("stat")
            .map(|stat| stat.byte_size),
        Some(sizes[0]),
        "a reference-only removal must leave the stored copy in place"
    );
    let collectable = store
        .unreferenced(harness.tdb.db(), 50)
        .await
        .expect("unreferenced");
    assert!(
        collectable.contains(&checksums[0]),
        "the kept copy must be left for the maintenance sweep rather than held \
         by a reference to a row that no longer exists: {collectable:?}"
    );
    let usage = lorehaven_db::library::storage_usage(harness.state().db(), &account)
        .await
        .expect("usage");
    assert_eq!(
        usage.imported_bytes, sizes[1],
        "the removed item's bytes are no longer this library's: {usage:?}"
    );

    // Remove the other as a copy: its bytes go now, and only its bytes.
    let (status, body) = client
        .post(
            "/api/v1/library/items/batch",
            json!({ "ids": [deleted], "delete_copy": true }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "copy removal: {body}");
    assert_eq!(
        body["freed_bytes"],
        json!(sizes[1]),
        "only the deleted item's own bytes may be reported freed: {body}"
    );
    let usage = lorehaven_db::library::storage_usage(harness.state().db(), &account)
        .await
        .expect("usage");
    assert_eq!(
        usage.imported_bytes, 0,
        "nothing of this library's storage remains: {usage:?}"
    );

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Saved views
// ---------------------------------------------------------------------------

/// A saved view is a query that survives the round trip, and a public one may
/// not carry a filter only its owner can see (spec §14.2).
#[tokio::test]
async fn a_saved_view_round_trips_its_query() {
    let harness = Harness::new("saved-view").await;
    let mut client = harness.client();
    let account = register(&mut client, "views@example.com", "views").await;
    seed_item(&harness, &account, "5001", "Something").await;

    let stored = json!({
        "name": "Unread royalroad",
        "query": {
            "shelves": ["Favourites"],
            "tags": ["wip"],
            "statuses": ["reading"],
            "source": "royalroad",
            "updated_since": "2026-01-01T00:00:00Z",
            "sort": "words"
        },
        "scope": "library",
        "pinned": true,
        "sort": "words"
    });
    let (status, created) = client.post("/api/v1/saved-views", stored.clone()).await;
    assert_eq!(status, StatusCode::CREATED, "create view: {created}");
    let id = created["id"].as_str().expect("view id").to_owned();
    assert_eq!(created["needs_repair"], json!(false));
    assert_eq!(created["query"]["sort"], json!("words"));

    // Read back: every field, including the one the reader typed a date into.
    let (status, body) = client.get(&format!("/api/v1/saved-views/{id}")).await;
    assert_eq!(status, StatusCode::OK, "read view: {body}");
    assert_eq!(body["query"]["shelves"], json!(["Favourites"]));
    assert_eq!(body["query"]["tags"], json!(["wip"]));
    assert_eq!(body["query"]["statuses"], json!(["reading"]));
    assert_eq!(body["query"]["source"], json!("royalroad"));
    assert_eq!(
        body["query"]["updated_since"],
        json!("2026-01-01T00:00:00Z")
    );
    assert_eq!(body["sort"], json!("words"));
    assert_eq!(body["pinned"], json!(true));
    assert_eq!(body["scope"], json!("library"));

    // And it is in the listing, pinned.
    let (status, body) = client.get("/api/v1/saved-views").await;
    assert_eq!(status, StatusCode::OK, "list views: {body}");
    assert_eq!(body["items"].as_array().map(Vec::len), Some(1));

    // A view over the public library may not name a shelf, a private tag or a
    // reading status: those describe the reader, and a shared view that filtered
    // by them would answer with somebody else's shelves — or reveal that the
    // shelf exists.
    let (status, body) = client
        .post(
            "/api/v1/saved-views",
            json!({
                "name": "Shared, but private filters",
                "query": { "shelves": ["Favourites"] },
                "scope": "public",
            }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a public view with a private filter must be refused: {body}"
    );
    let refusal = body["error"]["message"]
        .as_str()
        .unwrap_or("<the envelope carried no message>");
    assert!(
        refusal.contains("shelves"),
        "the refusal must name what leaked; it said {refusal:?}"
    );

    // The same filters are fine when the view stays the reader's own.
    let (status, body) = client
        .post(
            "/api/v1/saved-views",
            json!({
                "name": "Private, private filters",
                "query": { "shelves": ["Favourites"] },
                "scope": "library",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "private view: {body}");

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Bookmarks outlive the item they point into
// ---------------------------------------------------------------------------

/// Deleting a library item leaves the reader's bookmarks alone.
///
/// A bookmark is the reader's note about a place in a work; it is not a
/// property of the item. Cascading the deletion would take a reader's notes with
/// a row they removed to tidy up.
#[tokio::test]
async fn deleting_a_library_item_leaves_the_reader_s_bookmarks_alone() {
    let harness = Harness::new("bookmark-survives").await;
    let mut client = harness.client();
    let account = register(&mut client, "bookmarks@example.com", "bookmarks").await;
    let item = seed_item(&harness, &account, "6001", "Bookmarked").await;

    let (status, body) = client
        .post(
            "/api/v1/bookmarks",
            json!({
                "subject_type": "library_item",
                "subject_id": item,
                "position_permille": 250,
                "note": "the bit with the letter",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create bookmark: {body}");
    let bookmark_id = body["id"].as_str().expect("bookmark id").to_owned();
    // Private unless asked, which is what the body did not ask.
    assert_eq!(body["is_public"], json!(false));

    let (status, _) = client
        .post(
            "/api/v1/library/items/batch",
            json!({ "ids": [item], "delete_copy": false }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "remove the item");

    let (status, body) = client.get("/api/v1/bookmarks").await;
    assert_eq!(status, StatusCode::OK, "bookmarks after removal: {body}");
    assert_eq!(
        body["items"].as_array().map(Vec::len),
        Some(1),
        "the bookmark must survive the item it points into: {body}"
    );
    assert_eq!(body["items"][0]["id"], json!(bookmark_id));
    assert_eq!(body["items"][0]["note"], json!("the bit with the letter"));
    assert_eq!(body["items"][0]["position_permille"], json!(250));

    // And it can still be edited and deleted on its own.
    let (status, _) = client
        .patch(
            &format!("/api/v1/bookmarks/{bookmark_id}"),
            json!({ "note": "still here", "expected_version": 1 }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "editing a surviving bookmark"
    );
    let (status, _) = client
        .delete(&format!("/api/v1/bookmarks/{bookmark_id}"))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    harness.cleanup().await;
}

/// Spec §14.1's acceptance: "Public bookmark lists exclude private entries".
#[tokio::test]
async fn a_public_bookmark_list_excludes_private_entries() {
    let harness = Harness::new("bookmark-public").await;
    let mut client = harness.client();
    let account = register(&mut client, "public-bm@example.com", "publicbm").await;
    let item = seed_item(&harness, &account, "6002", "Shared Work").await;

    for (note, public) in [("mine alone", false), ("for anyone", true)] {
        let (status, body) = client
            .post(
                "/api/v1/bookmarks",
                json!({
                    "subject_type": "library_item",
                    "subject_id": item,
                    "note": note,
                    "is_public": public,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "create bookmark: {body}");
    }

    // The public list is the one query in the module that is not account-scoped,
    // so it is the one that has to filter on its own.
    let public =
        lorehaven_db::library::public_bookmarks_for(harness.state().db(), "library_item", &item)
            .await
            .expect("public bookmarks");
    assert_eq!(public.len(), 1, "only the shared bookmark may be listed");
    assert_eq!(public[0].note, "for anyone");

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

/// The storage figure is the sum of what is stored, counted once per blob.
///
/// Both halves are asserted: two distinct copies add up, and one copy shared by
/// two items counts once — because the number is physical, and a reader freeing
/// space is freeing bytes that exist rather than the sum of the sizes the items
/// appear to be.
#[tokio::test]
async fn storage_usage_matches_the_sum_of_the_items() {
    let harness = Harness::new("storage").await;
    let mut client = harness.client();
    let account = register(&mut client, "storage@example.com", "storage").await;

    let first = seed_item(&harness, &account, "7001", "First").await;
    let second = seed_item(&harness, &account, "7002", "Second").await;

    let store = harness.store();
    let bodies: [&[u8]; 3] = [b"chapter one", b"chapter two", b"a shared chapter"];
    let mut checksums = Vec::new();
    for body in bodies {
        let (checksum, _key) = store
            .put(harness.tdb.db(), body, "text/plain")
            .await
            .expect("put");
        checksums.push(checksum);
    }
    // The first two belong to one item each; the third is shared by both.
    store
        .reference(harness.tdb.db(), &checksums[0], "library_item", &first)
        .await
        .expect("reference");
    store
        .reference(harness.tdb.db(), &checksums[1], "library_item", &second)
        .await
        .expect("reference");
    store
        .reference(harness.tdb.db(), &checksums[2], "library_item", &first)
        .await
        .expect("reference");
    store
        .reference(harness.tdb.db(), &checksums[2], "library_item", &second)
        .await
        .expect("reference");

    let expected: i64 = bodies
        .iter()
        .map(|body| i64::try_from(body.len()).expect("size"))
        .sum();

    let (status, body) = client.get("/api/v1/library/storage").await;
    assert_eq!(status, StatusCode::OK, "storage: {body}");
    assert_eq!(body["item_count"], json!(2));
    assert_eq!(
        body["imported_bytes"],
        json!(expected),
        "the shared chapter must be counted once, not twice: {body}"
    );
    assert_eq!(
        body["blob_count"],
        json!(3),
        "three distinct blobs back the two items: {body}"
    );
    assert_eq!(body["total_bytes"], json!(expected));
    assert_eq!(body["export_bytes"], json!(0));

    // Removing an item that shares a blob must not free the shared blob: the
    // store checks the reference count, so the bytes are still accounted for.
    let (status, body) = client
        .post(
            "/api/v1/library/items/batch",
            json!({ "ids": [first], "delete_copy": true }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "remove with copy: {body}");
    assert_eq!(
        body["freed_bytes"],
        json!(bodies[0].len()),
        "only the blob nothing else referenced may be freed: {body}"
    );

    let (_, body) = client.get("/api/v1/library/storage").await;
    let remaining = i64::try_from(bodies[1].len() + bodies[2].len()).expect("size");
    assert_eq!(
        body["imported_bytes"],
        json!(remaining),
        "the shared blob survives its first owner: {body}"
    );

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Reading status
// ---------------------------------------------------------------------------

/// A reading status carries two stamps that mean different things, and the
/// difference is only visible over a sequence of changes.
#[tokio::test]
async fn a_reading_status_keeps_its_first_start_and_follows_its_finish() {
    let harness = Harness::new("status").await;
    let mut client = harness.client();
    let account = register(&mut client, "status@example.com", "status").await;
    let item = seed_item(&harness, &account, "8001", "Tracked").await;

    let (status, body) = client
        .put(
            &format!("/api/v1/library/items/{item}/status"),
            json!({ "status": "reading" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "set reading: {body}");
    let started = body["started_at"].as_str().expect("started_at").to_owned();
    assert_eq!(body["finished_at"], json!(null));

    // Finished sets the finish stamp.
    let (status, body) = client
        .put(
            &format!("/api/v1/library/items/{item}/status"),
            json!({ "status": "finished" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "set finished: {body}");
    assert!(body["finished_at"].is_string(), "finishing stamps the time");
    assert_eq!(
        body["started_at"].as_str(),
        Some(started.as_str()),
        "the first start is not rewritten"
    );

    // Reopening clears the finish and keeps the original start: a work that was
    // finished and then continued is not finished.
    let (status, body) = client
        .put(
            &format!("/api/v1/library/items/{item}/status"),
            json!({ "status": "reading" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "reopen: {body}");
    assert_eq!(body["finished_at"], json!(null), "the finish is cleared");
    assert_eq!(
        body["started_at"].as_str(),
        Some(started.as_str()),
        "and the first start still stands"
    );

    // An unknown status is refused rather than dropped, which would answer a
    // different question than the one asked.
    let (status, _) = client
        .put(
            &format!("/api/v1/library/items/{item}/status"),
            json!({ "status": "halfway" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // And the filter over statuses reaches the same rows.
    let (_, body) = client.get("/api/v1/library/items?statuses=reading").await;
    assert_eq!(body["total"], json!(1), "the item is being read: {body}");
    let (_, body) = client.get("/api/v1/library/items?statuses=finished").await;
    assert_eq!(body["total"], json!(0), "and is not finished: {body}");

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// The update check is a job
// ---------------------------------------------------------------------------

/// Spec §14.1's update check answers `202` with a job, and an empty library is
/// refused rather than queued: a job that reads no network and writes no record
/// is a job that looks like it did something.
#[tokio::test]
async fn the_update_check_is_queued_as_a_job() {
    let harness = Harness::new("update-check").await;
    let mut client = harness.client();
    let account = register(&mut client, "updates@example.com", "updates").await;

    let (status, body) = client
        .post("/api/v1/library/updates/check", json!({}))
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "an empty library has nothing to check: {body}"
    );

    seed_item(&harness, &account, "9001", "Checkable").await;
    let (status, body) = client
        .post("/api/v1/library/updates/check", json!({}))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "queue a check: {body}");
    assert!(
        body["job_id"].is_string(),
        "the answer names the job: {body}"
    );
    assert_eq!(body["items"], json!(1));

    // It is really in the queue, as an update check rather than as something
    // else that happens to be queued.
    let queued = lorehaven_db::jobs::all_jobs(harness.state().db(), None, 50, None)
        .await
        .expect("list jobs");
    assert!(
        queued.iter().any(|job| job.kind == "update_check"),
        "the job must be an update check: {queued:?}"
    );

    harness.cleanup().await;
}
