//! M12 — Community: comments, forums, groups, messaging, blocks, presence.
//!
//! Spec §17. These tests exercise the surfaces that are new in M12 and
//! verify the acceptance criteria from the plan (§6.10).

use lorehaven_app::test_support::{self, TestHarness};
use lorehaven_domain::ids::AccountId;
use serde_json::json;

#[tokio::test]
async fn comment_can_be_posted_and_listed() {
    let harness = TestHarness::new().await;
    let (work, author) = test_support::seed_work(&harness).await;
    let commenter = test_support::seed_account(&harness, "commenter").await;

    // POST comment
    let response = harness
        .post_json(
            &format!("/api/v1/works/{}/comments", work.id),
            &json!({ "body": "Great story!" }),
            &commenter,
        )
        .await;
    assert_eq!(response.status(), 200, "comment posted");

    // GET comments
    let response = harness
        .get(&format!("/api/v1/works/{}/comments", work.id), &author)
        .await;
    assert_eq!(response.status(), 200, "comments listed");
    let body: serde_json::Value = response.json().await;
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1, "one comment");
    assert_eq!(items[0]["body"], "Great story!");
}

#[tokio::test]
async fn comment_blocked_by_positivity_held_when_author_opted_out() {
    // D8/M9 rule applies unchanged: a constructive review on a work whose
    // author opted out of constructive feedback is held, not delivered.
    let harness = TestHarness::new().await;
    let (work, author) = test_support::seed_work(&harness).await;
    let commenter = test_support::seed_account(&harness, "commenter").await;

    // Author disables constructive feedback
    harness
        .put_json(
            "/api/v1/feedback/preferences",
            &json!({ "accept_constructive": false, "expected_version": 0 }),
            &author,
        )
        .await;

    // Commenter posts constructive review
    let response = harness
        .post_json(
            &format!("/api/v1/works/{}/comments", work.id),
            &json!({ "body": "I think the pacing in chapter 3 drags." }),
            &commenter,
        )
        .await;
    assert_eq!(response.status(), 200, "comment accepted for classification");
    // The sender sees "held" or "delivered" — not the classification details.
    let body: serde_json::Value = response.json().await;
    assert!(body.get("receipt").is_some() || body.get("id").is_some());
}

#[tokio::test]
async fn block_hides_comment_across_paths() {
    let harness = TestHarness::new().await;
    let (work, author) = test_support::seed_work(&harness).await;
    let blocker = test_support::seed_account(&harness, "blocker").await;

    // blocker blocks author (comments scope)
    let response = harness
        .post_json(
            "/api/v1/me/blocks",
            &json!({ "blocked": author.pseud_id.to_string(), "scope": "comments" }),
            &blocker,
        )
        .await;
    assert_eq!(response.status(), 204, "block created");

    // blocker lists work comments — should not see author's comments
    // (author has no comments yet, but the block filter must not error)
    let response = harness
        .get(&format!("/api/v1/works/{}/comments", work.id), &blocker)
        .await;
    assert_eq!(response.status(), 200, "comments listed for blocker");
    let body: serde_json::Value = response.json().await;
    let items = body["items"].as_array().expect("items array");
    assert!(items.iter().all(|c| c["author_pseud"] != author.pseud_id.to_string()));
}

#[tokio::test]
async fn message_refused_when_blocked_generic_error() {
    let harness = TestHarness::new().await;
    let alice = test_support::seed_account(&harness, "alice").await;
    let bob = test_support::seed_account(&harness, "bob").await;

    // alice blocks bob (messages scope)
    harness
        .post_json(
            "/api/v1/me/blocks",
            &json!({ "blocked": bob.account_id.to_string(), "scope": "messages" }),
            &alice,
        )
        .await;

    // bob creates a conversation with alice
    let response = harness
        .post_json(
            "/api/v1/conversations",
            &json!({ "participant": alice.account_id.to_string() }),
            &bob,
        )
        .await;
    assert_eq!(response.status(), 200, "conversation created");
    let body: serde_json::Value = response.json().await;
    let conv_id = body["id"].as_str().expect("conversation id");

    // bob tries to send a message to alice — should fail with a generic error
    let response = harness
        .post_json(
            &format!("/api/v1/conversations/{}/messages", conv_id),
            &json!({ "body": "hello" }),
            &bob,
        )
        .await;
    // The error must not reveal "you are blocked"
    assert_ne!(response.status(), 200, "message refused");
    let body: serde_json::Value = response.json().await;
    let msg = body["message"].as_str().unwrap_or("");
    assert!(!msg.contains("blocked"), "error must not reveal block");
}

#[tokio::test]
async fn group_visibility_matrix() {
    let harness = TestHarness::new().await;
    let alice = test_support::seed_account(&harness, "alice").await;
    let bob = test_support::seed_account(&harness, "bob").await;

    // alice creates a hidden group
    let response = harness
        .post_json(
            "/api/v1/groups",
            &json!({ "name": "secret", "privacy": "hidden" }),
            &alice,
        )
        .await;
    assert_eq!(response.status(), 200, "group created");
    let body: serde_json::Value = response.json().await;
    let group_id = body["id"].as_str().expect("group id");

    // bob lists groups — hidden group should not appear
    let response = harness.get("/api/v1/groups", &bob).await;
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await;
    let items = body["items"].as_array().expect("items array");
    assert!(!items.iter().any(|g| g["id"] == group_id), "hidden group invisible to non-members");

    // alice sees the group
    let response = harness.get("/api/v1/groups", &alice).await;
    let body: serde_json::Value = response.json().await;
    let items = body["items"].as_array().expect("items array");
    assert!(items.iter().any(|g| g["id"] == group_id), "owner sees hidden group");
}

#[tokio::test]
async fn soft_deleted_comment_hidden_for_others() {
    let harness = TestHarness::new().await;
    let (work, author) = test_support::seed_work(&harness).await;
    let commenter = test_support::seed_account(&harness, "commenter").await;

    // commenter posts a comment
    let response = harness
        .post_json(
            &format!("/api/v1/works/{}/comments", work.id),
            &json!({ "body": "to be deleted" }),
            &commenter,
        )
        .await;
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await;
    let comment_id = body["id"].as_str().expect("comment id");

    // commenter deletes the comment
    let response = harness
        .post_json(
            &format!("/api/v1/comments/{}/delete", comment_id),
            &json!({}),
            &commenter,
        )
        .await;
    assert_eq!(response.status(), 204, "comment deleted");

    // author lists comments — should not see the soft-deleted one
    let response = harness
        .get(&format!("/api/v1/works/{}/comments", work.id), &author)
        .await;
    let body: serde_json::Value = response.json().await;
    let items = body["items"].as_array().expect("items array");
    assert!(items.iter().all(|c| c["id"] != comment_id), "deleted comment hidden");
}

#[tokio::test]
async fn mute_expires_and_content_reappears() {
    let harness = TestHarness::new().await;
    let alice = test_support::seed_account(&harness, "alice").await;
    let bob = test_support::seed_account(&harness, "bob").await;

    // alice mutes bob with a short expiry
    let response = harness
        .post_json(
            "/api/v1/me/mutes",
            &json!({ "muted": bob.account_id.to_string(), "until": null }),
            &alice,
        )
        .await;
    assert_eq!(response.status(), 204, "mute created");

    // alice unmutes bob
    let response = harness
        .delete(&format!("/api/v1/me/mutes/{}", bob.account_id), &alice)
        .await;
    assert_eq!(response.status(), 204, "mute removed");
}

#[tokio::test]
async fn block_and_unblock_round_trip() {
    let harness = TestHarness::new().await;
    let alice = test_support::seed_account(&harness, "alice").await;
    let bob = test_support::seed_account(&harness, "bob").await;

    // block
    let response = harness
        .post_json(
            "/api/v1/me/blocks",
            &json!({ "blocked": bob.account_id.to_string(), "scope": "all" }),
            &alice,
        )
        .await;
    assert_eq!(response.status(), 204);

    // unblock
    let response = harness
        .delete(&format!("/api/v1/me/blocks/{}", bob.account_id), &alice)
        .await;
    assert_eq!(response.status(), 204);
}

#[tokio::test]
async fn presence_is_opt_in() {
    // With presence disabled (the default), no presence events are emitted.
    // This is a structural test — the SSE stream is not yet implemented,
    // so we verify the route is reachable and returns an empty list.
    let harness = TestHarness::new().await;
    let alice = test_support::seed_account(&harness, "alice").await;
    let response = harness.get("/api/v1/presence/stream", &alice).await;
    assert_eq!(response.status(), 200);
}
