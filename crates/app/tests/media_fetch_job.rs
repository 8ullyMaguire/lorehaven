//! M32-07d: the media fetch job, end to end (spec §32.7.2).
//!
//! M32-07c left a chain with no driver: `plan_fetch` refused a host,
//! `fingerprint_encoded` decoded bytes, `record_fingerprint` wrote both hashes,
//! and nothing ever called them in sequence. `record_fingerprint` had no
//! production caller at all.
//!
//! These tests stand up a real HTTP server on loopback, serve a real PNG from
//! it, and drive the worker handler against it. Loopback is the one address the
//! SSRF guard refuses unconditionally, so the tests need a way to say "this
//! connection is the one under test" without weakening the guard for production
//! traffic — see `handle_media_fetch_with_client` and the injected-resolver
//! comment on it.

use lorehaven_app::config::Config;
use lorehaven_app::media_fetch::MediaFingerprint;
use lorehaven_app::state::AppState;
use lorehaven_db::media_resilience;
use lorehaven_domain::media_resilience::MediaKind;
use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-mfetch-{tag}-{}-{:?}",
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
    config
}

/// A body the fetch job will accept: a real 4x4 PNG.
fn png_bytes() -> Vec<u8> {
    lorehaven_app::media_fetch::test_support_png_4x4()
}

/// A PNG whose brightness rises left to right, so the dHash is all ones.
fn ramp_png_bytes() -> Vec<u8> {
    lorehaven_app::media_fetch::test_support_ramp_png()
}

/// Serve `body` at `/` on loopback and return its address.
///
/// Loopback is refused by the SSRF guard, which is correct in production and
/// useless in a test. Rather than weaken `is_forbidden_ip` for loopback — which
/// would reopen the exact hole the guard exists to close — the test drives the
/// handler through the same code path with a resolver that maps this one
/// loopback address to "public". A production build never installs it.
struct TestServer {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl TestServer {
    fn start(body: Vec<u8>, content_type: &'static str, status: u16) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("local addr");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stop_flag.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(mut stream) = stream else { continue };
                // Read the request head, then answer once. The fetch job makes
                // one request per test, so a single response is enough.
                let mut buf = [0u8; 2048];
                let _ = std::io::Read::read(&mut stream, &mut buf);
                let head = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                use std::io::Write;
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });
        Self {
            addr,
            stop,
            handle: Some(handle),
        }
    }

    fn url(&self) -> String {
        format!("http://{}/image.png", self.addr)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Unblock `incoming()` so the thread can observe the stop flag.
        let _ = std::net::TcpStream::connect(self.addr);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Set up a reference plus its availability link, as `add_media_reference` does.
async fn seed_reference(state: &AppState, url: &str) -> String {
    let reference_id = uuid::Uuid::new_v4().to_string();
    media_resilience::insert_media_reference(
        state.db(),
        &reference_id,
        "pending",
        MediaKind::Image,
    )
    .await
    .expect("insert media reference");
    media_resilience::insert_availability_link(
        state.db(),
        &uuid::Uuid::new_v4().to_string(),
        &reference_id,
        url,
        lorehaven_domain::media_resilience::LinkProvider::Other,
        None,
        100,
    )
    .await
    .expect("insert availability link");
    reference_id
}

#[tokio::test]
async fn a_fetched_image_gets_both_hashes_stored() {
    let dir = scratch_dir("full");
    let tdb = test_support::TestDb::connect_with_dir("mfetch-full", &dir).await;
    let state = AppState::new(config_for(&dir), tdb.db().clone());
    let server = TestServer::start(png_bytes(), "image/png", 200);
    let reference_id = seed_reference(&state, &server.url()).await;

    lorehaven_app::media_job::handle_media_fetch_for_test(&state, &reference_id, server.addr.ip())
        .await
        .expect("the fetch job succeeds");

    let stored = media_resilience::find_media_reference_by_id(state.db(), &reference_id)
        .await
        .expect("find")
        .expect("exists");
    assert!(
        stored.content_hash.starts_with("sha256:"),
        "the exact hash must be stored, got {:?}",
        stored.content_hash
    );
    let perceptual = stored
        .perceptual_hash
        .as_deref()
        .expect("a decoded PNG must produce a perceptual hash");
    assert_eq!(
        perceptual.len(),
        16,
        "a dHash is 16 hex digits, got {perceptual}"
    );
}

#[tokio::test]
async fn the_stored_perceptual_hash_is_the_one_the_ramp_would_produce() {
    // Not just "some hash" — the right one. A ramp image differences to all
    // ones, which is independently checkable.
    let dir = scratch_dir("ramp");
    let tdb = test_support::TestDb::connect_with_dir("mfetch-ramp", &dir).await;
    let state = AppState::new(config_for(&dir), tdb.db().clone());
    let server = TestServer::start(ramp_png_bytes(), "image/png", 200);
    let reference_id = seed_reference(&state, &server.url()).await;

    lorehaven_app::media_job::handle_media_fetch_for_test(&state, &reference_id, server.addr.ip())
        .await
        .expect("the fetch job succeeds");

    let stored = media_resilience::find_media_reference_by_id(state.db(), &reference_id)
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(
        stored.perceptual_hash.as_deref(),
        Some("ffffffffffffffff"),
        "an all-brightness-rising ramp differences to all ones"
    );
}

#[tokio::test]
async fn the_pending_placeholder_is_replaced_by_the_real_hashes() {
    // `add_media_reference` writes the literal string "pending" and its comment
    // promises "content hash computed async by the pipeline". This is the test
    // that makes that promise true.
    let dir = scratch_dir("placeholder");
    let tdb = test_support::TestDb::connect_with_dir("mfetch-ph", &dir).await;
    let state = AppState::new(config_for(&dir), tdb.db().clone());
    let server = TestServer::start(png_bytes(), "image/png", 200);
    let reference_id = seed_reference(&state, &server.url()).await;

    let before = media_resilience::find_media_reference_by_id(state.db(), &reference_id)
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(before.content_hash, "pending");

    lorehaven_app::media_job::handle_media_fetch_for_test(&state, &reference_id, server.addr.ip())
        .await
        .expect("the fetch job succeeds");

    let after = media_resilience::find_media_reference_by_id(state.db(), &reference_id)
        .await
        .expect("find")
        .expect("exists");
    assert_ne!(
        after.content_hash, "pending",
        "the placeholder must be replaced"
    );
    assert_ne!(after.content_hash, "sha256:0000", "not a null hash");
}

#[tokio::test]
async fn a_fetched_image_is_found_by_the_curators_dedup_search() {
    // The point of the whole chain, asserted end to end: a hash written by the
    // worker is found by the search the curator runs.
    let dir = scratch_dir("search");
    let tdb = test_support::TestDb::connect_with_dir("mfetch-search", &dir).await;
    let state = AppState::new(config_for(&dir), tdb.db().clone());
    let server = TestServer::start(ramp_png_bytes(), "image/png", 200);
    let reference_id = seed_reference(&state, &server.url()).await;

    lorehaven_app::media_job::handle_media_fetch_for_test(&state, &reference_id, server.addr.ip())
        .await
        .expect("the fetch job succeeds");

    let found = media_resilience::find_by_perceptual_hash(state.db(), "ffffffffffffffff", 0)
        .await
        .expect("search");
    assert_eq!(
        found.len(),
        1,
        "the curator's search must find the worker's hash, got {found:?}"
    );
    assert_eq!(found[0].id, reference_id);
}

#[tokio::test]
async fn a_non_image_response_is_refused_and_the_hash_is_not_faked() {
    let dir = scratch_dir("notmedia");
    let tdb = test_support::TestDb::connect_with_dir("mfetch-nm", &dir).await;
    let state = AppState::new(config_for(&dir), tdb.db().clone());
    let server = TestServer::start(b"<html>nope</html>".to_vec(), "text/html", 200);
    let reference_id = seed_reference(&state, &server.url()).await;

    let outcome = lorehaven_app::media_job::handle_media_fetch_for_test(
        &state,
        &reference_id,
        server.addr.ip(),
    )
    .await;

    assert!(
        outcome.is_err(),
        "an HTML body served as an author's faceclaim must not succeed"
    );
    // And the row must not have been given a hash that means nothing.
    let stored = media_resilience::find_media_reference_by_id(state.db(), &reference_id)
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(
        stored.content_hash, "pending",
        "a refused fetch must not write a hash"
    );
}

#[tokio::test]
async fn a_404_is_permanent_and_a_503_is_transient() {
    // Getting this backwards either retries a deleted image forever or abandons
    // a mirror that was briefly down, so the classification is the assertion.
    for (status, want_transient) in [(404u16, false), (410, false), (503, true), (429, true)] {
        let dir = scratch_dir(&format!("status{status}"));
        let tdb = test_support::TestDb::connect_with_dir(&format!("mfetch-s{status}"), &dir).await;
        let state = AppState::new(config_for(&dir), tdb.db().clone());
        let server = TestServer::start(b"".to_vec(), "image/png", status);
        let reference_id = seed_reference(&state, &server.url()).await;

        let error = lorehaven_app::media_job::handle_media_fetch_for_test(
            &state,
            &reference_id,
            server.addr.ip(),
        )
        .await
        .expect_err("a non-2xx must not succeed");

        // Assert the error's *type*, not a word in its message. A substring
        // check on prose is a test of the wording, and rewording the message
        // would break it; the type is the contract the worker acts on.
        let is_transient = matches!(error, lorehaven_app::worker::HandlerError::Transient(_));
        let is_fatal = matches!(error, lorehaven_app::worker::HandlerError::Fatal(_));
        assert!(
            is_transient || is_fatal,
            "a classification failure must be one or the other, got {error:?}"
        );
        assert_eq!(
            is_transient,
            want_transient,
            "status {status} should{} be transient; error was {error:?}",
            if want_transient { "" } else { " not" }
        );
    }
}

#[tokio::test]
async fn a_reference_with_no_availability_link_is_a_fatal_job_not_a_retry_forever() {
    // Nothing to fetch. Retrying this every minute for the life of the instance
    // is the failure mode to avoid.
    let dir = scratch_dir("nolink");
    let tdb = test_support::TestDb::connect_with_dir("mfetch-nolink", &dir).await;
    let state = AppState::new(config_for(&dir), tdb.db().clone());
    let reference_id = uuid::Uuid::new_v4().to_string();
    media_resilience::insert_media_reference(
        state.db(),
        &reference_id,
        "pending",
        MediaKind::Image,
    )
    .await
    .expect("insert media reference");

    let error = lorehaven_app::media_job::handle_media_fetch_for_test(
        &state,
        &reference_id,
        "127.0.0.1".parse().expect("ip"),
    )
    .await
    .expect_err("no link means nothing to fetch");
    assert!(
        error.message().contains("no availability link"),
        "the error must name the cause, got: {}",
        error.message()
    );
}

#[tokio::test]
async fn a_second_fetch_of_the_same_image_does_not_duplicate_or_drift() {
    // A retry after a partial failure re-runs the job. It must land on the same
    // hashes, not append a row or change the answer.
    let dir = scratch_dir("idempotent");
    let tdb = test_support::TestDb::connect_with_dir("mfetch-idem", &dir).await;
    let state = AppState::new(config_for(&dir), tdb.db().clone());
    let server = TestServer::start(ramp_png_bytes(), "image/png", 200);
    let reference_id = seed_reference(&state, &server.url()).await;

    for _ in 0..2 {
        lorehaven_app::media_job::handle_media_fetch_for_test(
            &state,
            &reference_id,
            server.addr.ip(),
        )
        .await
        .expect("the fetch job succeeds");
    }

    let found = media_resilience::find_by_perceptual_hash(state.db(), "ffffffffffffffff", 0)
        .await
        .expect("search");
    assert_eq!(found.len(), 1, "a re-fetch must not create a second row");
    assert_eq!(found[0].id, reference_id);
}

#[tokio::test]
async fn a_fetch_that_cannot_decode_still_stores_the_exact_hash_and_no_perceptual_one() {
    // A server that lies about the content type: it says image/png and serves
    // bytes that are not a decodable image. The exact hash of those bytes is
    // still a true fact worth keeping, and the perceptual hash must be NULL
    // rather than empty — an empty string compares as distance 0 against every
    // other undecodable image and would merge them all into one reference.
    let dir = scratch_dir("undecodable");
    let tdb = test_support::TestDb::connect_with_dir("mfetch-undec", &dir).await;
    let state = AppState::new(config_for(&dir), tdb.db().clone());
    let server = TestServer::start(b"\x89PNG\r\n\x1a\ngarbage".to_vec(), "image/png", 200);
    let reference_id = seed_reference(&state, &server.url()).await;

    lorehaven_app::media_job::handle_media_fetch_for_test(&state, &reference_id, server.addr.ip())
        .await
        .expect("an undecodable body is still a successful fetch");

    let stored = media_resilience::find_media_reference_by_id(state.db(), &reference_id)
        .await
        .expect("find")
        .expect("exists");
    assert!(
        stored.content_hash.starts_with("sha256:"),
        "the exact hash of the bytes is still true, got {:?}",
        stored.content_hash
    );
    assert_eq!(
        stored.perceptual_hash, None,
        "an undecodable body must leave the perceptual hash NULL, not empty"
    );
}

#[tokio::test]
async fn the_job_kind_exists_and_is_interactive() {
    use lorehaven_domain::jobs::{JobKind, ResourceClass, ALL_KINDS};
    // A job kind that is not in the vocabulary cannot be enqueued, and one that
    // is not classified cannot be claimed.
    assert!(
        ALL_KINDS.contains(&JobKind::MediaFetch),
        "MediaFetch must be in ALL_KINDS or enqueue cannot store it"
    );
    assert_eq!(JobKind::MediaFetch.as_str(), "media_fetch");
    assert_eq!(JobKind::parse("media_fetch"), Some(JobKind::MediaFetch));
    assert_eq!(
        JobKind::MediaFetch.resource_class(),
        ResourceClass::Interactive
    );
    // And it must be the only kind with this index, which the exhaustive match
    // in `kind_index` is there to guarantee.
    let indices: Vec<usize> = ALL_KINDS.iter().map(|k| k.kind_index()).collect();
    let mut unique = indices.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        indices.len(),
        "kind_index must be unique per kind"
    );
}

#[tokio::test]
async fn adding_a_media_reference_queues_the_fetch_job() {
    // The route's own promise: "content hash computed async by the pipeline".
    // This asserts a job row exists, which is what makes the pipeline a
    // pipeline rather than a comment.
    use lorehaven_db::jobs::all_jobs;
    use lorehaven_domain::jobs::JobKind;

    let dir = scratch_dir("route-enqueue");
    let tdb = test_support::TestDb::connect_with_dir("mfetch-enq", &dir).await;
    let state = AppState::new(config_for(&dir), tdb.db().clone());

    let reference_id = seed_reference(&state, "http://example.com/never-fetched.png").await;
    // The enqueue half, called directly: the route's next step.
    lorehaven_app::media_job::enqueue_media_fetch(state.db(), &reference_id)
        .await
        .expect("enqueue");

    let jobs = all_jobs(state.db(), None, 50, None)
        .await
        .expect("list jobs");
    let ours: Vec<_> = jobs
        .into_iter()
        .filter(|j| j.kind == JobKind::MediaFetch.as_str())
        .collect();
    assert_eq!(ours.len(), 1, "exactly one fetch job, got {}", ours.len());
    assert!(
        ours[0].payload.contains(&reference_id),
        "the payload must name the reference it fetches, got {}",
        ours[0].payload
    );
}

#[tokio::test]
async fn a_fingerprint_from_the_fetch_matches_the_one_stored() {
    // Guards the From bridge the job relies on, so a change to either struct
    // cannot silently zero the dimensions.
    let fp = MediaFingerprint::without_perceptual_hash(b"bytes");
    let bridged: media_resilience::Fingerprint = (&fp).into();
    assert_eq!(bridged.content_hash, fp.content_hash);
    assert_eq!(bridged.perceptual_hash, None);
    assert_eq!(bridged.width, None, "an unknown width is None, not 0");
    assert_eq!(bridged.height, None);
}
