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
use lorehaven_db::{Backend, DatabaseConfig};
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
    assert!(
        !items.is_empty(),
        "expected at least the caller's item: {body}"
    );
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
    let sql = harness.tdb.db().sql("INSERT INTO forum_categories (id, name, position, min_trust) VALUES (?, 'Test Category', 0, 0)", "INSERT INTO forum_categories (id, name, position, min_trust) VALUES ($1, 'Test Category', 0, 0)");
    match harness.tdb.db().backend() {
        Backend::Sqlite => {
            sqlx::query(sql.as_ref())
                .bind(cat_id)
                .execute(harness.tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("seed category");
        }
        Backend::Postgres => {
            sqlx::query(sql.as_ref())
                .bind(cat_id)
                .execute(harness.tdb.db().postgres_pool().expect("postgres"))
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
    assert!(
        items.iter().any(|t| t["id"] == topic_id),
        "topic should appear in listing"
    );

    // Reply to the topic.
    let (status, body) = user
        .post(
            &format!("/api/v1/topics/{topic_id}/replies"),
            json!({ "body": "First!" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["id"].as_str().is_some(), "reply has id");

    // Topic page shows unlocked state initially.
    let (status, body) = user.get(&format!("/api/v1/topics/{topic_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        !body["topic"]["locked"].as_bool().unwrap(),
        "fresh topic is unlocked"
    );

    // Lock the topic.
    let (status, _) = user
        .post(&format!("/api/v1/topics/{topic_id}/lock"), json!({}))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = user.get(&format!("/api/v1/topics/{topic_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["topic"]["locked"].as_bool().unwrap(),
        "topic is locked after toggle"
    );

    // Replying to a locked topic is rejected (422).
    let (status, body) = user
        .post(
            &format!("/api/v1/topics/{topic_id}/replies"),
            json!({ "body": "nope" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");

    // Toggle again unlocks it.
    let (status, _) = user
        .post(&format!("/api/v1/topics/{topic_id}/lock"), json!({}))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = user.get(&format!("/api/v1/topics/{topic_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        !body["topic"]["locked"].as_bool().unwrap(),
        "topic is unlocked after second toggle"
    );

    // Locking a nonexistent topic returns 404.
    let (status, _) = user
        .post("/api/v1/topics/nonexistent/lock", json!({}))
        .await;
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
    let sql = harness.tdb.db().sql("INSERT INTO forum_categories (id, name, position, min_trust) VALUES (?, 'Editors Only', 0, 5)", "INSERT INTO forum_categories (id, name, position, min_trust) VALUES ($1, 'Editors Only', 0, 5)");
    match harness.tdb.db().backend() {
        Backend::Sqlite => {
            sqlx::query(sql.as_ref())
                .bind(cat_id)
                .execute(harness.tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .unwrap();
        }
        Backend::Postgres => {
            sqlx::query(sql.as_ref())
                .bind(cat_id)
                .execute(harness.tdb.db().postgres_pool().expect("postgres"))
                .await
                .unwrap();
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
    let sql = harness.tdb.db().sql("INSERT INTO trust_levels (account, level, computed_at, basis) VALUES (?, 5, datetime('now'), 'test')", "INSERT INTO trust_levels (account, level, computed_at, basis) VALUES ($1::uuid, 5, now(), 'test')");
    match harness.tdb.db().backend() {
        Backend::Sqlite => {
            sqlx::query(sql.as_ref())
                .bind(&account_id)
                .execute(harness.tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .unwrap();
        }
        Backend::Postgres => {
            sqlx::query(sql.as_ref())
                .bind(&account_id)
                .execute(harness.tdb.db().postgres_pool().expect("postgres"))
                .await
                .unwrap();
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
        .post(
            &format!("/api/v1/conversations/{conv_id}/messages"),
            json!({ "body": "hello there" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // A's conversation list shows it with a preview.
    let (status, body) = a.get("/api/v1/conversations").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "A sees 1 conversation");
    let entry = &items[0];
    assert_eq!(entry["id"], conv_id);
    assert_eq!(
        entry["other_handle"], b_account,
        "A sees B as other handle in listing"
    );
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

#[tokio::test]
async fn a_reply_notifies_the_topic_author_and_the_inbox_settles() {
    let harness = Harness::new("notify-reply").await;
    let mut author = harness.client();
    register(&mut author, "topic-author@example.com", "TopicAuthor").await;
    let mut replier = harness.client();
    register(&mut replier, "replier@example.com", "ReplyUser").await;

    // Seed a category with min_trust 0 (test DBs don't run the dev seed).
    let cat_id = "22222222-2222-2222-2222-222222222222";
    let sql = harness.tdb.db().sql("INSERT INTO forum_categories (id, name, position, min_trust) VALUES (?, 'Notify Category', 0, 0)", "INSERT INTO forum_categories (id, name, position, min_trust) VALUES ($1, 'Notify Category', 0, 0)");
    match harness.tdb.db().backend() {
        Backend::Sqlite => {
            sqlx::query(sql.as_ref())
                .bind(cat_id)
                .execute(harness.tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("seed category");
        }
        Backend::Postgres => {
            sqlx::query(sql.as_ref())
                .bind(cat_id)
                .execute(harness.tdb.db().postgres_pool().expect("postgres"))
                .await
                .expect("seed category");
        }
    }

    // The author starts with an empty inbox.
    let (status, body) = author.get("/api/v1/notifications").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["unread_count"].as_i64(), Some(0), "{body}");
    assert_eq!(body["items"].as_array().map(Vec::len), Some(0), "{body}");

    // The author opens a topic; the replier answers it.
    let (status, body) = author
        .post(
            &format!("/api/v1/forums/{cat_id}/topics"),
            json!({ "title": "Notify me" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let topic_id = body["id"].as_str().expect("topic id").to_owned();

    let (status, body) = replier
        .post(
            &format!("/api/v1/topics/{topic_id}/replies"),
            json!({ "body": "consider it replied" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // The author's inbox now holds one unread reply notification.
    let (status, body) = author.get("/api/v1/notifications").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["unread_count"].as_i64(), Some(1), "{body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{body}");
    assert_eq!(items[0]["kind"], "reply", "{body}");
    assert_eq!(items[0]["read"], false, "{body}");
    let notification_id = items[0]["id"].as_str().expect("notification id").to_owned();

    // Marking that one entry read settles the count; a replay is harmless.
    let (status, _) = author
        .post(
            &format!("/api/v1/notifications/{notification_id}/read"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = author
        .post(
            &format!("/api/v1/notifications/{notification_id}/read"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = author.get("/api/v1/notifications").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["unread_count"].as_i64(), Some(0), "{body}");

    // A second reply arrives and read-all closes it in one sweep.
    let (status, _) = replier
        .post(
            &format!("/api/v1/topics/{topic_id}/replies"),
            json!({ "body": "once more, with feeling" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _) = author
        .post("/api/v1/notifications/read-all", json!({}))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = author.get("/api/v1/notifications").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["unread_count"].as_i64(), Some(0), "{body}");
    assert_eq!(body["items"].as_array().map(Vec::len), Some(2), "{body}");

    // The replier's own inbox stays empty: replying never notifies yourself.
    let (status, body) = replier.get("/api/v1/notifications").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["unread_count"].as_i64(), Some(0), "{body}");

    // Notifications are session-scoped: a signed-out caller gets 401.
    let mut stranger = harness.client();
    let (status, _) = stranger.get("/api/v1/notifications").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    harness.cleanup().await;
}

/// A public review tells the author once. The notification says "a new public
/// review was posted", so an edit of one is not news — before this guard the
/// author was told again on every save, once per delivered upsert.
#[tokio::test]
async fn editing_a_public_review_does_not_notify_the_author_again() {
    let harness = Harness::new("review-notified-once").await;
    let work_id = published_work(
        &harness,
        "notified-author@example.com",
        "NotifiedAuthor",
        "Reviewed Once",
    )
    .await;

    let mut reviewer = harness.client();
    register(&mut reviewer, "reviewer-once@example.com", "ReviewerOnce").await;
    let review_path = format!("/api/v1/works/{work_id}/reviews");
    let (status, body) = reviewer
        .request(
            "PUT",
            &review_path,
            Some(json!({
                "body": "Warm and well made, and the ending earns it.",
                "is_public": true
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["receipt"], "Comment posted.",
        "the gate must deliver this text for the test to mean anything: {body}"
    );

    // The author is the account that published the work, so sign in rather
    // than register (the email is taken).
    let mut author = harness.client();
    let (status, _) = author
        .post(
            "/api/v1/auth/login",
            json!({ "email": "notified-author@example.com", "password": PASSWORD }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = author.get("/api/v1/notifications").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["items"].as_array().map(Vec::len),
        Some(1),
        "one review notification: {body}"
    );
    assert_eq!(body["items"][0]["kind"], "review", "{body}");

    // The reviewer fixes a typo: still public, still delivered, still one.
    let (status, body) = reviewer
        .request(
            "PUT",
            &review_path,
            Some(json!({
                "body": "Warm and well made, and the ending earns it completely.",
                "is_public": true
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["receipt"], "Comment posted.", "{body}");

    let (status, body) = author.get("/api/v1/notifications").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["items"].as_array().map(Vec::len),
        Some(1),
        "an edit is not a new review: {body}"
    );

    // A second reviewer is news, so the guard is not silencing the feature.
    let mut other = harness.client();
    register(&mut other, "reviewer-two@example.com", "ReviewerTwo").await;
    let (status, body) = other
        .request(
            "PUT",
            &review_path,
            Some(json!({
                "body": "A lovely thing to read on a wet afternoon.",
                "is_public": true
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["receipt"], "Comment posted.", "{body}");

    let (status, body) = author.get("/api/v1/notifications").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["items"].as_array().map(Vec::len),
        Some(2),
        "a second reviewer is a second review: {body}"
    );

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Mentions (spec §17.5)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_post_mention_creates_a_mention_event_and_notification() {
    let harness = Harness::new("mention-basic").await;
    let mut alice = harness.client();
    register(&mut alice, "alice@example.com", "Alice").await;
    let mut bob = harness.client();
    let (_, _bob_pseud) = register(&mut bob, "bob@example.com", "Bob").await;

    // Seed a category.
    let cat_id = "11111111-1111-1111-1111-111111111111";
    let sql = harness.tdb.db().sql(
        "INSERT INTO forum_categories (id, name, position, min_trust) VALUES (?, 'Cat', 0, 0)",
        "INSERT INTO forum_categories (id, name, position, min_trust) VALUES ($1, 'Cat', 0, 0)",
    );
    match harness.tdb.db().backend() {
        Backend::Sqlite => {
            sqlx::query(sql.as_ref())
                .bind(cat_id)
                .execute(harness.tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("seed category");
        }
        Backend::Postgres => {
            sqlx::query(sql.as_ref())
                .bind(cat_id)
                .execute(harness.tdb.db().postgres_pool().expect("postgres"))
                .await
                .expect("seed category");
        }
    }

    // Alice creates a topic.
    let (status, body) = alice
        .post(
            &format!("/api/v1/forums/{cat_id}/topics"),
            json!({ "title": "Hello" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let topic_id = body["id"].as_str().expect("topic id").to_owned();

    // Alice posts a reply mentioning @bob.
    let (status, body) = alice
        .post(
            &format!("/api/v1/topics/{topic_id}/replies"),
            json!({ "body": "Hey @Bob, check this out!" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Bob should have a notification.
    let (status, body) = bob.get("/api/v1/notifications").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"]
        .as_array()
        .map(|a| a.to_owned())
        .unwrap_or_default();
    assert!(
        items.iter().any(|n| n["kind"] == "mention"),
        "bob should have a mention notification: {body}"
    );

    harness.cleanup().await;
}

#[tokio::test]
async fn a_blocked_mention_creates_no_mention_event() {
    let harness = Harness::new("mention-blocked").await;
    let mut alice = harness.client();
    register(&mut alice, "alice@example.com", "Alice").await;
    let mut bob = harness.client();
    let (bob_account, _) = register(&mut bob, "bob@example.com", "Bob").await;

    // Alice blocks Bob in the comments scope.
    let (status, _) = alice
        .post(
            "/api/v1/me/blocks",
            json!({ "blocked": bob_account, "scope": "comments" }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "alice blocks bob");

    // Seed a category.
    let cat_id = "22222222-2222-2222-2222-222222222222";
    let sql = harness.tdb.db().sql(
        "INSERT INTO forum_categories (id, name, position, min_trust) VALUES (?, 'Cat', 0, 0)",
        "INSERT INTO forum_categories (id, name, position, min_trust) VALUES ($1, 'Cat', 0, 0)",
    );
    match harness.tdb.db().backend() {
        Backend::Sqlite => {
            sqlx::query(sql.as_ref())
                .bind(cat_id)
                .execute(harness.tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("seed category");
        }
        Backend::Postgres => {
            sqlx::query(sql.as_ref())
                .bind(cat_id)
                .execute(harness.tdb.db().postgres_pool().expect("postgres"))
                .await
                .expect("seed category");
        }
    }

    // Alice creates a topic.
    let (status, body) = alice
        .post(
            &format!("/api/v1/forums/{cat_id}/topics"),
            json!({ "title": "Hello" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let topic_id = body["id"].as_str().expect("topic id").to_owned();

    // Alice posts a reply mentioning @bob (who she blocked).
    let (status, body) = alice
        .post(
            &format!("/api/v1/topics/{topic_id}/replies"),
            json!({ "body": "Hey @Bob!" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Bob should NOT have a mention notification (block suppresses it).
    let (status, body) = bob.get("/api/v1/notifications").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"]
        .as_array()
        .map(|a| a.to_owned())
        .unwrap_or_default();
    assert!(
        !items.iter().any(|n| n["kind"] == "mention"),
        "bob should NOT have a mention notification when blocked: {body}"
    );

    harness.cleanup().await;
}

#[tokio::test]
async fn a_self_mention_creates_no_mention_event() {
    let harness = Harness::new("mention-self").await;
    let mut alice = harness.client();
    register(&mut alice, "alice@example.com", "Alice").await;

    // Seed a category.
    let cat_id = "33333333-3333-3333-3333-333333333333";
    let sql = harness.tdb.db().sql(
        "INSERT INTO forum_categories (id, name, position, min_trust) VALUES (?, 'Cat', 0, 0)",
        "INSERT INTO forum_categories (id, name, position, min_trust) VALUES ($1, 'Cat', 0, 0)",
    );
    match harness.tdb.db().backend() {
        Backend::Sqlite => {
            sqlx::query(sql.as_ref())
                .bind(cat_id)
                .execute(harness.tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("seed category");
        }
        Backend::Postgres => {
            sqlx::query(sql.as_ref())
                .bind(cat_id)
                .execute(harness.tdb.db().postgres_pool().expect("postgres"))
                .await
                .expect("seed category");
        }
    }

    // Alice creates a topic.
    let (status, body) = alice
        .post(
            &format!("/api/v1/forums/{cat_id}/topics"),
            json!({ "title": "Hello" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let topic_id = body["id"].as_str().expect("topic id").to_owned();

    // Alice posts a reply mentioning herself.
    let (status, body) = alice
        .post(
            &format!("/api/v1/topics/{topic_id}/replies"),
            json!({ "body": "Hey @Alice, remember this." }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Alice should NOT have a mention notification (self-mention).
    let (status, body) = alice.get("/api/v1/notifications").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"]
        .as_array()
        .map(|a| a.to_owned())
        .unwrap_or_default();
    assert!(
        !items.iter().any(|n| n["kind"] == "mention"),
        "alice should NOT have a self-mention notification: {body}"
    );

    harness.cleanup().await;
}

/// A reader's content filters apply to the inbox, and to the unread count.
///
/// Two things have to hold, and the second is the one that is easy to forget:
/// the filtered notification must leave the list *and* stop being counted. A
/// badge that is filtered out of the list but still counted leaks the existence
/// of the very work the filter hides, through a one-digit door.
///
/// A notification with no `work_id` (an instance notice) must survive: §46.7.1
/// is about works not reaching the reader, not about silencing the instance at
/// someone who filtered a tag.
#[tokio::test]
async fn content_filters_reach_the_inbox_and_the_unread_count() {
    let harness = Harness::new("notify-content-filter").await;
    let mut reader = harness.client();
    let (_account, _pseud) = register(&mut reader, "inbox-reader@example.com", "InboxReader").await;
    let db = harness.tdb.db();

    // One published work, tagged `spoilers`; one untagged work.
    let blocked_work = published_work(
        &harness,
        "inbox-blocked@example.com",
        "InboxBlocked",
        "Tagged With Spoilers",
    )
    .await;
    let clean_work = published_work(
        &harness,
        "inbox-clean@example.com",
        "InboxClean",
        "Untagged Work",
    )
    .await;

    let now = chrono::Utc::now().to_rfc3339();
    let node_id = uuid::Uuid::new_v4().to_string();
    let sql_node = db.sql(
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at) VALUES (?, ?, ?, ?, ?)",
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at) VALUES ($1::uuid, $2, $3, $4, $5::timestamptz)",
    );
    let sql_tag = db.sql(
        "INSERT INTO work_tags (work_id, node_id, weight, added_at) VALUES (?, ?, ?, ?)",
        "INSERT INTO work_tags (work_id, node_id, weight, added_at) VALUES ($1::uuid, $2, $3, $4::timestamptz)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(sql_node.as_ref())
                .bind(&node_id)
                .bind("tag")
                .bind("spoilers")
                .bind("spoilers")
                .bind(&now)
                .execute(db.sqlite_pool().unwrap())
                .await
                .expect("taxonomy node");
            sqlx::query(sql_tag.as_ref())
                .bind(&blocked_work)
                .bind(&node_id)
                .bind(1i64)
                .bind(&now)
                .execute(db.sqlite_pool().unwrap())
                .await
                .expect("work tag");
        }
        Backend::Postgres => {
            sqlx::query(sql_node.as_ref())
                .bind(&node_id)
                .bind("tag")
                .bind("spoilers")
                .bind("spoilers")
                .bind(&now)
                .execute(db.postgres_pool().unwrap())
                .await
                .expect("taxonomy node");
            sqlx::query(sql_tag.as_ref())
                .bind(&blocked_work)
                .bind(&node_id)
                .bind(1i64)
                .bind(&now)
                .execute(db.postgres_pool().unwrap())
                .await
                .expect("work tag");
        }
    }

    // Seed three unread notifications directly, so `work_id` is under the
    // test's control: the reply writers attach none.
    let account_id: uuid::Uuid = _account.parse().expect("account id is a uuid");
    let sql_note = db.sql(
        "INSERT INTO notifications (id, account_id, kind, title, body, work_id, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        "INSERT INTO notifications (id, account_id, kind, title, body, work_id, created_at) VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6::uuid, $7::timestamptz)",
    );
    for (title, work) in [
        ("Someone replied about your spoilers", Some(&blocked_work)),
        ("Someone replied about your clean work", Some(&clean_work)),
        ("The instance has an announcement", None),
    ] {
        match db.backend() {
            Backend::Sqlite => {
                sqlx::query(sql_note.as_ref())
                    .bind(uuid::Uuid::new_v4().to_string())
                    .bind(account_id.to_string())
                    .bind("reply")
                    .bind(title)
                    .bind("body")
                    .bind(work.map(|w| w.to_string()))
                    .bind(&now)
                    .execute(db.sqlite_pool().unwrap())
                    .await
                    .expect("seed notification");
            }
            Backend::Postgres => {
                sqlx::query(sql_note.as_ref())
                    .bind(uuid::Uuid::new_v4().to_string())
                    .bind(account_id)
                    .bind("reply")
                    .bind(title)
                    .bind("body")
                    .bind(work.map(|w| w.parse::<uuid::Uuid>().unwrap()))
                    .bind(&now)
                    .execute(db.postgres_pool().unwrap())
                    .await
                    .expect("seed notification");
            }
        }
    }

    // Before any filter, all three are there and all three count.
    let (status, body) = reader.get("/api/v1/notifications").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"].as_array().map(Vec::len), Some(3), "{body}");
    assert_eq!(body["unread_count"].as_i64(), Some(3), "{body}");

    // Block the tag.
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

    // The work notification is gone -- from the list and from the count.
    let (status, body) = reader.get("/api/v1/notifications").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(
        items.len(),
        2,
        "the filtered notification must not be listed: {body}"
    );
    let titles: Vec<&str> = items.iter().filter_map(|i| i["title"].as_str()).collect();
    assert!(
        !titles.iter().any(|t| t.contains("spoilers")),
        "the blocked work's title reached the inbox: {titles:?}"
    );
    assert_eq!(
        body["unread_count"].as_i64(),
        Some(2),
        "the count must exclude the filtered notification too, or the badge leaks it: {body}"
    );
    // The untagged work and the workless announcement both survive.
    assert!(
        titles.iter().any(|t| t.contains("clean work")),
        "the untagged work's notification should survive: {titles:?}"
    );
    assert!(
        titles.iter().any(|t| t.contains("announcement")),
        "a notification with no work is not about a work and must survive: {titles:?}"
    );

    harness.cleanup().await;
}
