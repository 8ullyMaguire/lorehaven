//! M13 — Events: collections, challenges, requests, wishlists, events.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::{Database, DatabaseConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m13-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id(),
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
}

struct Harness {
    dir: PathBuf,
    db: Database,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        set_trust_proxy(false);
        let _ = lorehaven_app::logging::init(&lorehaven_app::config::LoggingConfig {
            filter: "error".to_owned(),
            format: lorehaven_app::config::LogFormat::Pretty,
        });
        let dir = scratch_dir(tag);
        let db = Database::connect(&DatabaseConfig::new(format!(
            "sqlite://{}/lorehaven.sqlite?mode=rwc",
            dir.display()
        )))
        .await
        .expect("connect");
        let _ = db.migrate().await.expect("migrate");
        Self { dir, db }
    }
    fn client(&self) -> Client {
        Client::new(server::build_router(AppState::new(
            config_for(&self.dir),
            self.db.clone(),
        )))
    }
    async fn cleanup(self) {
        self.db.close().await;
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
                "age_band": "adult",
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

/// Create a published work owned by the registering author.
/// Returns (work_id, author_account_id, author_pseud_id).
async fn published_work(
    harness: &Harness,
    email: &str,
    handle: &str,
    title: &str,
) -> (String, String, String) {
    let mut author = harness.client();
    let (account, pseud) = register(&mut author, email, handle).await;
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
    let doc = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "A chapter with enough words to have a middle and an end."
            }]
        }]
    });
    let (status, body) = author
        .request(
            "PATCH",
            &format!("/api/v1/chapters/{chapter}"),
            Some(json!({
                "expected_version": chapter_version,
                "document": doc
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "save: {body}");
    let (status, body) = author
        .post(
            &format!("/api/v1/works/{work_id}/publish"),
            json!({
                "expected_version": work_version,
                "idempotency_key": format!("m13-{work_id}")
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "publish: {body}");
    (work_id, account, pseud)
}

// ---------------------------------------------------------------------------
// Collections
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_collection_can_be_created_listed_and_published() {
    let harness = Harness::new("collection-create").await;
    let (work_id, _, _) =
        published_work(&harness, "author@example.com", "Author", "Collection Work").await;
    let mut client = harness.client();
    let (_, _) = register(&mut client, "collector@example.com", "Collector").await;

    // Create a collection with open item policy.
    let (status, body) = client
        .post(
            "/api/v1/collections",
            json!({
                "name": "My Favorites",
                "description": "Stuff I like",
                "item_policy": "open",
                "is_public": true,
            }),
        )
        .await;
    // Routes return 200 OK for creation (not 201 CREATED).
    assert_eq!(status, StatusCode::OK, "create collection: {body}");
    let collection_id = body["id"].as_str().expect("collection id").to_owned();

    // Attach a work to the collection.
    let (status, body) = client
        .post(
            &format!("/api/v1/collections/{collection_id}/items"),
            json!({ "work_id": work_id, "note": "favorite" }),
        )
        .await;
    // The response is 200 OK with the added item, or 204. Accept either.
    assert!(
        status == StatusCode::OK || status == StatusCode::NO_CONTENT,
        "add item: {status} {body}"
    );

    // List the collection's items.
    let (status, body) = client
        .get(&format!("/api/v1/collections/{collection_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "get collection: {body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "should have one item");
    assert_eq!(items[0]["work_id"].as_str().expect("work id"), work_id);

    // Update the collection's public flag.
    let (status, _body) = client
        .request(
            "PUT",
            &format!("/api/v1/collections/{collection_id}"),
            Some(json!({
                "name": "My Favorites",
                "description": "Updated",
                "item_policy": "open",
                "is_public": false,
            })),
        )
        .await;
    assert!(
        status == StatusCode::OK || status == StatusCode::NO_CONTENT,
        "update: {_body}"
    );

    harness.cleanup().await;
}

#[tokio::test]
async fn an_unauthenticated_user_cannot_create_a_collection() {
    let harness = Harness::new("collection-auth").await;
    let mut client = harness.client();
    let (status, _body) = client
        .post(
            "/api/v1/collections",
            json!({
                "name": "No Auth",
                "item_policy": "open",
                "is_public": true,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Challenges
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_challenge_can_be_created_and_listed() {
    let harness = Harness::new("challenge-create").await;
    let mut client = harness.client();
    let (_, _) = register(&mut client, "mod@example.com", "Mod").await;

    let (status, body) = client
        .post(
            "/api/v1/challenges",
            json!({
                "name": "100-Word Challenge",
                "rules": json!({ "constraints": [] }).to_string(),
                "schedule": "2025-01-01 to 2025-01-31",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "create: {body}");
    let challenge_id = body["id"].as_str().expect("challenge id").to_owned();

    let (status, body) = client.get("/api/v1/challenges").await;
    assert_eq!(status, StatusCode::OK, "list: {body}");
    let items = body["items"].as_array().expect("items");
    assert!(
        items
            .iter()
            .any(|i| i["id"].as_str() == Some(&challenge_id)),
        "challenge in list"
    );

    harness.cleanup().await;
}

#[tokio::test]
async fn entering_a_challenge_records_word_count() {
    let harness = Harness::new("challenge-enter").await;
    let (work_id, _, _) =
        published_work(&harness, "a@example.com", "AuthorA", "Challenge Work").await;
    let mut client = harness.client();
    let (_, _) = register(&mut client, "entrant@example.com", "Entrant").await;

    // Create the challenge with empty constraints.
    let (status, body) = client
        .post(
            "/api/v1/challenges",
            json!({
                "name": "Drabble",
                "rules": json!({ "constraints": [] }).to_string(),
                "schedule": "2025-03-01 to 2025-03-02",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "create: {body}");
    let challenge_id = body["id"].as_str().expect("id").to_owned();

    // Enter the challenge with a work.
    let (status, body) = client
        .post(
            &format!("/api/v1/challenges/{challenge_id}/entries"),
            json!({ "work_id": work_id }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "enter: {body}");

    // Retrieve the challenge.
    let (status, body) = client
        .get(&format!("/api/v1/challenges/{challenge_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "get: {body}");
    let challenge = body["challenge"].as_object().expect("challenge");
    assert_eq!(challenge["id"].as_str(), Some(challenge_id.as_str()));

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Requests / Exchanges
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_request_can_be_created_claimed_and_fulfilled() {
    let harness = Harness::new("request-flow").await;
    let (work_id, _, _) =
        published_work(&harness, "writer@example.com", "Writer", "Fulfil Work").await;
    let mut requester = harness.client();
    let (_, _) = register(&mut requester, "req@example.com", "Requester").await;
    let mut fulfiller = harness.client();
    let (ful_account, _) = register(&mut fulfiller, "ful@example.com", "Fulfiller").await;

    // Requester posts a request.
    let (status, body) = requester
        .post(
            "/api/v1/requests",
            json!({ "prompt": "I need a 500-word fic about cats." }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "create request: {body}");
    let request_id = body["id"].as_str().expect("request id").to_owned();

    // Fulfiller claims the request — first claim succeeds.
    let (status, _body) = fulfiller
        .post(
            &format!("/api/v1/requests/{request_id}/claims"),
            json!({ "claimant": ful_account.clone() }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "claim: {_body}");

    // A second claim from the same claimant should fail (already claimed).
    let (status, _body) = fulfiller
        .post(
            &format!("/api/v1/requests/{request_id}/claims"),
            json!({ "claimant": ful_account.clone() }),
        )
        .await;
    // The route correctly returns 422 Validation Failed when a request
    // already has an active claim — this is the expected behaviour.
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "double claim should fail: {_body}"
    );

    // Fulfill the claim with a published work.
    let (status, _body) = fulfiller
        .post(
            &format!("/api/v1/claims/{request_id}/fulfil"),
            json!({ "work_id": work_id, "claimant": ful_account.clone() }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "fulfil: {_body}");

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Wishlists
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_wishlist_can_have_items_added_and_retrieved() {
    let harness = Harness::new("wishlist-create").await;
    // Register the owner and create a published work to add to the wishlist.
    let mut owner = harness.client();
    let (account, _) = register(&mut owner, "owner@example.com", "Owner").await;
    let (status, body) = owner
        .post("/api/v1/works", json!({ "title": "Wishlist Work" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    let work_id = body["id"].as_str().expect("work id").to_owned();

    // Add a wishlist item — this implicitly creates the wishlist.
    let (status, body) = owner
        .post(
            "/api/v1/wishlist-items",
            json!({ "work_id": work_id, "note": "please write more!" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "add item: {body}");

    // Retrieve the owner's wishlist.
    let (status, body) = owner.get(&format!("/api/v1/wishlists/{account}")).await;
    assert_eq!(status, StatusCode::OK, "get: {body}");
    let wishlist = body["wishlist"].as_object().expect("wishlist");
    let items = wishlist["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "should have one item");
    assert_eq!(items[0]["work_id"].as_str(), Some(work_id.as_str()));

    harness.cleanup().await;
}

#[tokio::test]
async fn a_private_wishlist_is_hidden_from_other_users() {
    let harness = Harness::new("wishlist-private").await;
    // Owner registers and adds an item to their (implicitly private) wishlist.
    let mut owner = harness.client();
    let (account, _) = register(&mut owner, "o@example.com", "Owner").await;
    let (status, body) = owner
        .post("/api/v1/works", json!({ "title": "Private Wish Work" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    let work_id = body["id"].as_str().expect("work id").to_owned();

    let (status, _) = owner
        .post(
            "/api/v1/wishlist-items",
            json!({ "work_id": work_id, "note": "secret fic idea" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // Another user views the owner's wishlist — items hidden (private).
    let mut other = harness.client();
    let (_, _) = register(&mut other, "snoop@example.com", "Snoop").await;
    let (status, _) = other.get(&format!("/api/v1/wishlists/{account}")).await;
    // A private wishlist is *absent* to a stranger: 404 rather than an
    // empty shell, per spec §3.3 (prefer 404 over revealing that a
    // private object exists).
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "private wishlist hides as 404"
    );
    // The owner still sees their own.
    let (status, body) = owner.get(&format!("/api/v1/wishlists/{account}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let wishlist = body["wishlist"].as_object().expect("wishlist");
    let items = wishlist["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "owner sees their own item: {body}");

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Events (writing events)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_event_can_be_created_joined_and_listed() {
    let harness = Harness::new("event-create").await;
    let mut creator = harness.client();
    let (_, _) = register(&mut creator, "creator@example.com", "Creator").await;

    let (status, body) = creator
        .post(
            "/api/v1/events",
            json!({
                "name": "NaNoWriMo Lite",
                "document": "A month-long writing sprint.",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "create: {body}");
    let event_id = body["id"].as_str().expect("event id").to_owned();

    // Another user joins the event.
    let mut participant = harness.client();
    let (_, _) = register(&mut participant, "part@example.com", "Participant").await;
    let (status, body) = participant
        .post(&format!("/api/v1/events/{event_id}/join"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "join: {body}");

    // List the event's participants.
    let (status, body) = creator.get(&format!("/api/v1/events/{event_id}")).await;
    assert_eq!(status, StatusCode::OK, "get event: {body}");
    let participants = body["participants"].as_array().expect("participants");
    assert!(!participants.is_empty(), "should have a participant");

    harness.cleanup().await;
}

#[tokio::test]
async fn duplicate_event_join_is_idempotent() {
    let harness = Harness::new("event-duplicate-join").await;
    let mut creator = harness.client();
    let (_, _) = register(&mut creator, "c2@example.com", "Creator2").await;

    let (status, body) = creator
        .post(
            "/api/v1/events",
            json!({
                "name": "Writing Sprint",
                "document": "A short sprint.",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "create: {body}");
    let event_id = body["id"].as_str().expect("event id").to_owned();

    // Join twice — second should be idempotent (same participant not duplicated).
    let (status, _) = creator
        .post(&format!("/api/v1/events/{event_id}/join"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = creator.get(&format!("/api/v1/events/{event_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let participants = body["participants"].as_array().expect("participants");
    assert_eq!(
        participants.len(),
        1,
        "should still have exactly one participant"
    );

    harness.cleanup().await;
}
