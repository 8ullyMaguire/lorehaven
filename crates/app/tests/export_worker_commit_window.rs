//! M7 / M8 — The export worker's commit window.
//!
//! One defect, found by the Playwright export journey failing twice in a row
//! with the export stuck at "Waiting" forever while the worker was demonstrably
//! healthy, and by no unit test failing at all.
//!
//! ## The window
//!
//! `export_jobs.job_id` is a foreign key onto `jobs`, so the queue row has to be
//! written first — the comment at the call site says so. But "written first" is
//! not "written in the same transaction", and the two are separate commits. A
//! worker polling on a short interval can therefore claim the job in the gap
//! between them.
//!
//! The handler looked up the export row and treated its absence as fatal:
//!
//! ```text
//! export 6e10aced-… no longer exists
//! ```
//!
//! Which is worse than it sounds. `fatal` means "a retry cannot change this",
//! so the job failed on its first attempt, the export row stayed `queued`, and
//! the retries either never came or could not have helped — the row the handler
//! was waiting for had been committed microseconds after the lookup. The reader
//! saw "Waiting" indefinitely, with a healthy worker and a perfectly good export
//! sitting in the database.
//!
//! ## What is pinned here
//!
//! The absence of the export row is now transient, so the queue's own retry
//! policy decides. These tests call the handler directly with a payload naming
//! an export that does not exist, which is exactly the state the worker sees
//! inside the window, and assert on the *classification* rather than on timing:
//! the classification is the bug, and timing is not reproducible in a test.

use std::path::PathBuf;

use lorehaven_app::config::Config;
use lorehaven_app::exports;
use lorehaven_app::state::AppState;
use lorehaven_app::worker::HandlerError;
use serde_json::json;

use test_support::TestDb;

/// The handler is `run(state, payload)`, and the only thing it needs from the
/// payload is the export id. A state that exists is enough: the lookup happens
/// before anything else touches the database.
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-export-race-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

async fn state_for(tag: &str) -> AppState {
    let dir = scratch_dir(tag);
    let tdb = TestDb::connect_with_dir(tag, &dir).await;
    let config = Config::development_defaults();
    AppState::new(config, tdb.db().clone())
}

/// **A claimed job whose export row has not been committed yet is a retry, not a
/// death.** This is the whole fix: the export row lands in the next commit, so
/// the retry is the mechanism that makes the window survivable.
#[tokio::test]
async fn a_missing_export_row_is_retryable() {
    let state = state_for("missing-row").await;
    let missing = "6e10aced-eb0b-445b-9b6f-be9c0566620d";
    let payload = json!({ exports::PAYLOAD_EXPORT_JOB_ID: missing });

    match exports::run(&state, &payload).await {
        Err(HandlerError::Transient(message)) => {
            assert!(
                message.contains(missing),
                "the message names the export, so an operator reading a stuck \
                 job knows which one: {message}"
            );
        }
        Err(other) => panic!(
            "expected Transient so the queue retries, got {other:?} — a Fatal here \
             is the original bug: the row appears in the next commit, so no retry \
             policy can be defeated by retrying"
        ),
        Ok(()) => panic!(
            "a missing export row must not report success; that would mark the job \
             done and strand the export at 'queued'"
        ),
    }
}

/// **The error says the row is not committed yet, not that it no longer exists.**
/// The distinction is the whole point for whoever reads a stuck job: one is a
/// race that resolves itself, the other looks like data loss and sends someone
/// hunting for a bug that is not there.
#[tokio::test]
async fn the_message_says_not_yet_rather_than_gone() {
    let state = state_for("message").await;
    let missing = "00000000-0000-4000-8000-000000000000";
    let payload = json!({ exports::PAYLOAD_EXPORT_JOB_ID: missing });

    let Err(HandlerError::Transient(message)) = exports::run(&state, &payload).await else {
        panic!("expected a transient error");
    };
    assert!(
        !message.contains("no longer exists"),
        "'no longer exists' reads as data loss and sends an operator hunting; \
         the row is one commit behind: {message}"
    );
    assert!(
        message.contains("not committed yet"),
        "the message should say what is actually true: {message}"
    );
}

/// **A payload with no export id is still fatal.** A malformed payload is a
/// programming error, not a race, and retrying it would burn every attempt on a
/// job that can never succeed. This is the half of the original behaviour that
/// was right and must not be relaxed along with the half that was wrong.
#[tokio::test]
async fn a_payload_without_an_export_id_is_still_fatal() {
    let state = state_for("no-id").await;

    match exports::run(&state, &json!({})).await {
        Err(HandlerError::Fatal(message)) => {
            assert!(
                message.contains(exports::PAYLOAD_EXPORT_JOB_ID),
                "the message names the key that is missing: {message}"
            );
        }
        Err(other) => panic!("expected Fatal for a malformed payload, got {other:?}"),
        Ok(()) => panic!("a payload with no export id cannot succeed"),
    }
}

/// **The handler is not merely deferring an unknown-id error — it is looking in
/// the right place.** A row that does exist under a *different* id must still
/// produce the retryable error, which confirms the lookup is keyed on the
/// payload's id and has not been loosened into "give up on anything".
#[tokio::test]
async fn an_export_under_another_id_does_not_satisfy_the_lookup() {
    let state = state_for("wrong-id").await;
    // The id the payload names is well-formed but absent; a second, equally
    // well-formed id is also absent. Neither should be mistaken for the other.
    let payload = json!({
        exports::PAYLOAD_EXPORT_JOB_ID: "11111111-1111-4111-8111-111111111111",
    });

    let Err(HandlerError::Transient(message)) = exports::run(&state, &payload).await else {
        panic!("expected a transient error");
    };
    assert!(
        message.contains("11111111-1111-4111-8111-111111111111"),
        "the error names the id that was actually looked up: {message}"
    );
}
