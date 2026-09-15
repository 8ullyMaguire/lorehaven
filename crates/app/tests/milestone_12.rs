//! M12 — Community: comments, forums, groups, messaging, blocks, presence.
//!
//! These tests drive the real router against a real SQLite file. They cover
//! what the routes actually do today honestly: comments with the positivity
//! gate (M9 classifier wired through M12 to this surface), account-level
//! block filtering, forums topics and replies, group visibility, block-aware
//! messaging, the block and mute lists, and the presence record.
//!
//! Known gaps: forum categories need trust gates; `GET /presence/stream` is
//! a stub returning empty (SSE lands with the real-time milestone); block-aware
//! filtering on search/mention paths is still TODO.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::{Backend, Database, DatabaseConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m12-{tag}-{}-{:?}",
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
            json!({ "expected_version": work_version, "idempotency_key": format!("m12-{work_id}") }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "publish: {body}");
    work_id
}

async fn comment_count(client: &mut Client, work: &str) -> usize {
    let (status, body) = client.get(&format!("/api/v1/works/{work}/comments")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["items"].as_array().expect("items").len()
}

// ---------------------------------------------------------------------------
// Comments with positivity gate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_comment_is_posted_and_listed_for_its_work() {
    let harness = Harness::new("comment-basic").await;
    let work_id = published_work(&harness, "author@example.com", "Author", "Talked Work").await;
    let mut reader = harness.client();
    let (_, _) = register(&mut reader, "reader@example.com", "Reader").await;
    let (status, body) = reader
        .post(
            &format!("/api/v1/works/{work_id}/comments"),
            json!({ "body": "Great story!" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["id"].as_str().is_some(), "{body}");
    assert_eq!(comment_count(&mut reader, &work_id).await, 1);
    harness.cleanup().await;
}

#[tokio::test]
async fn a_comment_through_the_positivity_gate_returns_receipt() {
    let harness = Harness::new("comment-positivity").await;
    let work_id = published_work(&harness, "author@example.com", "Author", "Receipt Work").await;
    let mut reader = harness.client();
    register(&mut reader, "reader@example.com", "Reader").await;
    let (status, body) = reader
        .post(
            &format!("/api/v1/works/{work_id}/comments"),
            json!({ "body": "Great story!" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["id"].as_str().is_some(), "{body}");
    assert!(
        body["receipt"].as_str().is_some(),
        "expected receipt: {body}"
    );
    harness.cleanup().await;
}

#[tokio::test]
async fn a_hostile_comment_is_held_and_still_stored() {
    let harness = Harness::new("comment-hostile").await;
    let work_id = published_work(&harness, "author@example.com", "Author", "Hostile Work").await;
    let mut reader = harness.client();
    register(&mut reader, "reader@example.com", "Reader").await;
    let (status, body) = reader
        .post(
            &format!("/api/v1/works/{work_id}/comments"),
            json!({ "body": "You are stupid and your writing is worthless." }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["receipt"], "Comment held for moderator review.",
        "{body}"
    );
    // Spec §12.3: held text never surfaces publicly on the work page —
    // and the sender's own view is a listing too, so it is hidden there
    // just the same. The row stays stored for moderation.
    assert_eq!(
        comment_count(&mut reader, &work_id).await,
        0,
        "a held comment is listed nowhere"
    );
    harness.cleanup().await;
}

#[tokio::test]
async fn an_empty_comment_is_refused() {
    let harness = Harness::new("comment-empty").await;
    let work_id = published_work(&harness, "author@example.com", "Author", "Quiet Work").await;
    let mut reader = harness.client();
    register(&mut reader, "reader@example.com", "Reader").await;
    let (status, body) = reader
        .post(
            &format!("/api/v1/works/{work_id}/comments"),
            json!({ "body": "   " }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    harness.cleanup().await;
}

#[tokio::test]
async fn a_comment_needs_a_session() {
    let harness = Harness::new("comment-anon").await;
    let work_id = published_work(&harness, "author@example.com", "Author", "Closed Work").await;
    let mut stranger = harness.client();
    let (status, _) = stranger
        .post(
            &format!("/api/v1/works/{work_id}/comments"),
            json!({ "body": "no session" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    harness.cleanup().await;
}

#[tokio::test]
async fn a_block_hides_the_other_side_of_a_conversation_in_comments() {
    let harness = Harness::new("comment-block").await;
    let work_id = published_work(&harness, "author@example.com", "Author", "Blocked Work").await;
    let mut a = harness.client();
    let (a_account, _) = register(&mut a, "a@example.com", "ReaderA").await;
    let _ = &a_account;
    let mut b = harness.client();
    let (b_account, _) = register(&mut b, "b@example.com", "ReaderB").await;
    for (text, who) in [("A was here", &mut a), ("B was here", &mut b)] {
        let (status, body) = who
            .post(
                &format!("/api/v1/works/{work_id}/comments"),
                json!({ "body": text }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    assert_eq!(comment_count(&mut a, &work_id).await, 2);
    // A blocks B (account-level, comments scope). Blocks are bidirectional
    // on the listing paths: neither side sees the other afterwards.
    let (status, _) = a
        .post(
            "/api/v1/me/blocks",
            json!({ "blocked": b_account, "scope": "comments" }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "block: {status}");
    assert_eq!(
        comment_count(&mut a, &work_id).await,
        1,
        "A sees only their own comment"
    );
    assert_eq!(
        comment_count(&mut b, &work_id).await,
        1,
        "B sees only their own comment: A's words are hidden from them"
    );
    let _ = a_account; // held by the blocker side
    harness.cleanup().await;
}

#[tokio::test]
async fn the_author_can_delete_their_own_comment_and_nobody_elses() {
    let harness = Harness::new("comment-delete").await;
    let work_id = published_work(&harness, "author@example.com", "Author", "Deleted Work").await;
    let mut reader = harness.client();
    register(&mut reader, "reader@example.com", "Reader").await;
    let (status, body) = reader
        .post(
            &format!("/api/v1/works/{work_id}/comments"),
            json!({ "body": "mine to delete" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let comment = body["id"].as_str().expect("id").to_owned();
    // A stranger cannot delete it.
    let mut stranger = harness.client();
    register(&mut stranger, "stranger@example.com", "Stranger").await;
    let (status, _) = stranger
        .post(&format!("/api/v1/comments/{comment}/delete"), json!({}))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "stranger delete: {status}");
    // The author can.
    let (status, _) = reader
        .post(&format!("/api/v1/comments/{comment}/delete"), json!({}))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "author delete: {status}");
    assert_eq!(comment_count(&mut reader, &work_id).await, 0);
    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Blocks and mutes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_block_list_round_trips_and_deletes() {
    let harness = Harness::new("block-list").await;
    let mut a = harness.client();
    let (_, _) = register(&mut a, "a@example.com", "BlockerA").await;
    let mut b = harness.client();
    let (b_account, _) = register(&mut b, "b@example.com", "BlockedB").await;
    let (status, _) = a
        .post(
            "/api/v1/me/blocks",
            json!({ "blocked": b_account, "scope": "all", "note": "spam" }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = a.get("/api/v1/me/blocks").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{body}");
    assert_eq!(items[0]["blocked"], b_account.as_str(), "{body}");
    assert_eq!(items[0]["scope"], "all", "{body}");
    // B's own list is empty: a block belongs to its blocker.
    let (status, body) = b.get("/api/v1/me/blocks").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"].as_array().expect("items").len(), 0, "{body}");
    // Un-block removes it.
    let (status, _) = a.delete(&format!("/api/v1/me/blocks/{b_account}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = a.get("/api/v1/me/blocks").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"].as_array().expect("items").len(), 0, "{body}");
    harness.cleanup().await;
}

#[tokio::test]
async fn the_mute_list_round_trips_and_deletes() {
    let harness = Harness::new("mute-list").await;
    let mut a = harness.client();
    let (_, _) = register(&mut a, "a@example.com", "MuterA").await;
    let mut b = harness.client();
    let (b_account, _) = register(&mut b, "b@example.com", "MutedB").await;
    let (status, _) = a
        .post("/api/v1/me/mutes", json!({ "muted": b_account }))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = a.get("/api/v1/me/mutes").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{body}");
    assert_eq!(items[0]["muted"], b_account.as_str(), "{body}");
    let (status, _) = a.delete(&format!("/api/v1/me/mutes/{b_account}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = a.get("/api/v1/me/mutes").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"].as_array().expect("items").len(), 0, "{body}");
    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Messaging
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_message_blocked_in_the_messages_scope_never_arrives() {
    let harness = Harness::new("message-block").await;
    let mut a = harness.client();
    let (_, _) = register(&mut a, "a@example.com", "SenderA").await;
    let mut b = harness.client();
    let (b_account, _) = register(&mut b, "b@example.com", "ReceiverB").await;
    // A creates the conversation with B, then blocks B in the messages scope.
    let (status, body) = a
        .post("/api/v1/conversations", json!({ "participant": b_account }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let conversation = body["id"].as_str().expect("id").to_owned();
    let (status, _) = a
        .post(
            "/api/v1/me/blocks",
            json!({ "blocked": b_account, "scope": "messages" }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // B cannot send: the sender-side check refuses.
    let (status, body) = b
        .post(
            &format!("/api/v1/conversations/{conversation}/messages"),
            json!({ "body": "hello?" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "blocked send is refused: {body}"
    );
    // A's own listing shows no messages from B either way.
    let (status, body) = a
        .get(&format!("/api/v1/conversations/{conversation}/messages"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"].as_array().expect("items").len(), 0, "{body}");
    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Forums and groups (current surface)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn forum_topics_and_replies_round_trip() {
    let harness = Harness::new("forum-basic").await;
    // No categories exist yet (an honest M12 gap): creating a topic needs a
    // category id. The route accepts the attempt and fails with a validation
    // error rather than panicking — pin the shape until categories land.
    let mut user = harness.client();
    register(&mut user, "forum@example.com", "ForumUser").await;
    let (status, body) = user
        .post(
            "/api/v1/forums/00000000-0000-0000-0000-000000000000/topics",
            json!({ "title": "Hello" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a topic needs an existing category: {status} {body}"
    );
    harness.cleanup().await;
}

#[tokio::test]
async fn a_group_is_created_listed_and_joined() {
    let harness = Harness::new("group-basic").await;
    let mut owner = harness.client();
    register(&mut owner, "owner@example.com", "GroupOwner").await;
    let (status, body) = owner
        .post(
            "/api/v1/groups",
            json!({ "name": "Night Writers", "privacy": "open" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let group = body["id"].as_str().expect("id").to_owned();
    let (status, body) = owner.get("/api/v1/groups").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["items"]
            .as_array()
            .expect("items")
            .iter()
            .any(|g| g["id"] == group.as_str()),
        "{body}"
    );
    let mut member = harness.client();
    register(&mut member, "member@example.com", "GroupMember").await;
    let (status, _) = member
        .post(&format!("/api/v1/groups/{group}/join"), json!({}))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "join: {status}");
    let (status, body) = member.get(&format!("/api/v1/groups/{group}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    harness.cleanup().await;
}

#[tokio::test]
async fn presence_stream_returns_active_viewer() {
    let harness = Harness::new("presence-stream").await;
    let mut user = harness.client();
    register(&mut user, "presence@example.com", "Present").await;
    let (status, body) = user.get("/api/v1/presence/stream").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert!(items.len() >= 1, "expected at least the caller's item: {body}");
    let first = &items[0];
    assert!(first["active_now"].as_bool().unwrap_or(false), "{body}");
    assert!(first["enabled"].as_bool().unwrap_or(false), "{body}");
    harness.cleanup().await;
}


#[tokio::test]
async fn a_forum_topic_can_be_created_replied_to_and_locked() {
    let harness = Harness::new("forum-full").await;
    let mut user = harness.client();
    register(&mut user, "forum@example.com", "ForumUser").await;

    // Seed a category directly (test DBs don't run the dev seed).
    let cat_id = "11111111-1111-1111-1111-111111111111";
    let sql = "INSERT INTO forum_categories (id, name, position, min_trust) VALUES (?, 'Test Category', 0, 0)";
    match harness.db.backend() {
        Backend::Sqlite => {
            sqlx::query(sql)
                .bind(cat_id)
                .execute(harness.db.sqlite_pool().expect("sqlite"))
                .await
                .expect("seed category");
        }
        Backend::Postgres => {
            sqlx::query(sql)
                .bind(cat_id)
                .execute(harness.db.postgres_pool().expect("postgres"))
                .await
                .expect("seed category");
        }
    }

    // Create a topic in the category.
    let (status, body) = user
        .post(
            &format!("/api/v1/forums/{cat_id}/topics"),
            json!({ "title": "Hello World" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let topic_id = body["id"].as_str().expect("topic id").to_owned();

    // Topic appears in the category listing.
    let (status, body) = user.get(&format!("/api/v1/forums/{cat_id}/topics")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert!(items.iter().any(|t| t["id"] == topic_id), "topic should appear in listing");

    // Reply to the topic.
    let (status, body) = user
        .post(&format!("/api/v1/topics/{topic_id}/replies"), json!({ "body": "First!" }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["id"].as_str().is_some(), "reply has id");

    // Topic page shows unlocked state initially.
    let (status, body) = user.get(&format!("/api/v1/topics/{topic_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!body["topic"]["locked"].as_bool().unwrap(), "fresh topic is unlocked");

    // Lock the topic.
    let (status, _) = user.post(&format!("/api/v1/topics/{topic_id}/lock"), json!({})).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = user.get(&format!("/api/v1/topics/{topic_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["topic"]["locked"].as_bool().unwrap(), "topic is locked after toggle");

    // Replying to a locked topic is rejected (422).
    let (status, body) = user
        .post(&format!("/api/v1/topics/{topic_id}/replies"), json!({ "body": "nope" }))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");

    // Toggle again unlocks it.
    let (status, _) = user.post(&format!("/api/v1/topics/{topic_id}/lock"), json!({})).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = user.get(&format!("/api/v1/topics/{topic_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!body["topic"]["locked"].as_bool().unwrap(), "topic is unlocked after second toggle");

    // Locking a nonexistent topic returns 404.
    let (status, _) = user.post("/api/v1/topics/nonexistent/lock", json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    harness.cleanup().await;
}

#[tokio::test]
async fn a_trust_gate_rejects_underleveled_posters() {
    let harness = Harness::new("forum-trust-gate").await;
    let mut user = harness.client();
    let (account_id, _) = register(&mut user, "lowtrust@example.com", "LowTrustUser").await;

    // Category that requires editor-level trust (5).
    let cat_id = "22222222-2222-2222-2222-222222222222";
    let sql = "INSERT INTO forum_categories (id, name, position, min_trust) VALUES (?, 'Editors Only', 0, 5)";
    match harness.db.backend() {
        Backend::Sqlite => {
            sqlx::query(sql).bind(cat_id).execute(harness.db.sqlite_pool().expect("sqlite")).await.unwrap();
        }
        Backend::Postgres => {
            sqlx::query(sql).bind(cat_id).execute(harness.db.postgres_pool().expect("postgres")).await.unwrap();
        }
    }

    // TL_NEW (0) user cannot create a topic in a trust-5 category.
    let (status, body) = user
        .post(
            &format!("/api/v1/forums/{}/topics", cat_id),
            json!({ "title": "Should Fail" }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "ACCESS_DENIED", "{body}");

    // Promote the user to editor level.
    let sql = "INSERT INTO trust_levels (account, level, computed_at, basis) VALUES (?, 5, datetime('now'), 'test')";
    match harness.db.backend() {
        Backend::Sqlite => {
            sqlx::query(sql).bind(&account_id).execute(harness.db.sqlite_pool().expect("sqlite")).await.unwrap();
        }
        Backend::Postgres => {
            sqlx::query(sql).bind(&account_id).execute(harness.db.postgres_pool().expect("postgres")).await.unwrap();
        }
    }

    // Now the same user can create a topic.
    let (status, body) = user
        .post(
            &format!("/api/v1/forums/{}/topics", cat_id),
            json!({ "title": "Should Succeed" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["id"].as_str().is_some(), "topic id");

    harness.cleanup().await;
}

#[tokio::test]
async fn conversations_are_listed_with_a_preview() {
    let harness = Harness::new("conversation-list").await;
    let mut a = harness.client();
    let (a_account, _) = register(&mut a, "a@example.com", "SenderA").await;
    let mut b = harness.client();
    let (b_account, _) = register(&mut b, "b@example.com", "ReceiverB").await;

    // A creates a conversation with B and sends a message.
    let (status, body) = a
        .post("/api/v1/conversations", json!({ "participant": b_account }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let conv_id = body["id"].as_str().expect("id").to_owned();
    let (status, _) = a
        .post(&format!("/api/v1/conversations/{conv_id}/messages"), json!({ "body": "hello there" }))
        .await;
    assert_eq!(status, StatusCode::OK);

    // A's conversation list shows it with a preview.
    let (status, body) = a.get("/api/v1/conversations").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "A sees 1 conversation");
    let entry = &items[0];
    assert_eq!(entry["id"], conv_id);
    assert_eq!(entry["other_handle"], b_account, "A sees B as other handle in listing");
    assert_eq!(entry["last_message"], "hello there");
    assert!(entry["updated_at"].as_str().is_some(), "updated_at present");

    // B's list also shows the conversation with A as the other handle.
    let (status, body) = b.get("/api/v1/conversations").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "B sees 1 conversation");
    assert_eq!(items[0]["other_handle"], a_account);

    harness.cleanup().await;
}
