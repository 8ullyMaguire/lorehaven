//! The source revision cache (spec §10.4, §11.7).
//!
//! # What these tests are actually defending
//!
//! A cache is easy to write and hard to write *safely*, because its failure mode
//! is not an error — it is a plausible wrong answer. Four properties matter, and
//! each has a test here because each is a way to get content wrong rather than
//! merely slow:
//!
//! 1. **A `304` serves the bytes we already hold.** If it did not, the source
//!    would be telling us "unchanged" and we would store an empty chapter.
//! 2. **A credentialed read is not a public read.** Same URL, different reader,
//!    different page. Sharing an entry would hand gated content to somebody who
//!    cannot see it.
//! 3. **An adapter's entries die with the adapter.** A parser change means the
//!    bytes on disk are no longer what this build would have stored, so they must
//!    not be served as though they were.
//! 4. **The cache is never the reason a read fails.** Every internal failure
//!    degrades to an ordinary fetch.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use lorehaven_app::revisions::{CachingFetcher, PUBLIC_SCOPE};
use lorehaven_db::revisions;
use lorehaven_db::storage::BlobStore;
use lorehaven_scrapers::{
    ConditionalFetch, FetchClass, Fetched, Fetcher, Provenance, RevisionValidators, SourceError,
    SourceResult,
};

const SOURCE: &str = "ao3";
const ADAPTER: &str = "0.1.0";
const URL: &str = "https://archiveofourown.org/works/92356871";

// ---------------------------------------------------------------------------
// A fetcher that counts, and plays back a script
// ---------------------------------------------------------------------------

/// What the inner fetcher should do when asked.
#[derive(Debug, Clone)]
enum Answer {
    /// A page with these bytes and this `ETag`.
    Page { body: String, etag: Option<String> },
    /// The source confirms the revision is unchanged.
    NotModified,
    /// A page the posture requires be thrown away: the bytes are already gone by
    /// the time it reaches here, exactly as a real discarded read arrives.
    ///
    /// `body` is supplied rather than fixed at `""` on purpose. A real discarded
    /// read arrives with the bytes already cleared, so scripting it that way makes
    /// "skip it because it was discarded" and "skip it because it is empty"
    /// indistinguishable — and a mutation swapping one check for the other would
    /// pass. Here the discarded page still carries bytes, so only the flag can
    /// keep it out of the cache.
    Discarded { body: String, etag: Option<String> },
    /// The source fails.
    Fail,
}

/// The validators a request carried: an `ETag` and a `Last-Modified`.
///
/// Its own type rather than a tuple, because `Option<(Option<String>,
/// Option<String>)>` is a shape nobody reads correctly the first time — and the
/// distinction it encodes (was there a condition at all, and which half) is
/// exactly what these tests assert on.
type SentValidators = Option<(Option<String>, Option<String>)>;

/// Records every request so a test can assert on what was *asked*, not only on
/// what came back. A cache bug that still returns the right page is invisible
/// from the answer alone.
#[derive(Default)]
struct Log {
    /// Each call, in order: the validators the cache offered, if any.
    calls: Mutex<Vec<SentValidators>>,
    /// How many times a form was posted.
    posts: AtomicUsize,
}

struct Scripted {
    /// Answers in order. The last one repeats, so a test only writes the
    /// transitions it cares about.
    answers: Vec<Answer>,
    log: Arc<Log>,
}

impl Scripted {
    fn new(answers: Vec<Answer>) -> Self {
        Self {
            answers,
            log: Arc::new(Log::default()),
        }
    }

    fn log(&self) -> Arc<Log> {
        Arc::clone(&self.log)
    }

    fn answer(&self, index: usize) -> Answer {
        self.answers
            .get(index)
            .or_else(|| self.answers.last())
            .cloned()
            .unwrap_or(Answer::Fail)
    }
}

impl Scripted {
    /// Record this call and return which scripted answer it is.
    ///
    /// The count of calls *is* the script's cursor, so `get` and
    /// `get_conditional` share one counter: a test that reads a page and then
    /// conditions on it sees one continuous sequence rather than two.
    fn advance(&self, known: Option<&RevisionValidators>) -> usize {
        let mut calls = self.log.calls.lock().expect("the log is not poisoned");
        calls.push(known.map(|v| (v.etag.clone(), v.last_modified.clone())));
        calls.len() - 1
    }
}

#[async_trait]
impl Fetcher for Scripted {
    async fn get(&self, url: &str) -> SourceResult<Fetched> {
        let index = self.advance(None);
        match self.answer(index) {
            Answer::Page { body, etag } => Ok(Fetched {
                final_url: url.to_owned(),
                body,
                content_type: Some("text/html".to_owned()),
                etag,
                last_modified: None,
                provenance: Provenance::Source,
                fetch_class: FetchClass::Content,
                discarded: false,
            }),
            // A bare `GET` cannot receive a `304`; the trait's own default is to
            // read the page, so that is what this does.
            Answer::NotModified => Ok(Fetched {
                final_url: url.to_owned(),
                body: String::new(),
                content_type: None,
                etag: None,
                last_modified: None,
                provenance: Provenance::Source,
                fetch_class: FetchClass::Content,
                discarded: false,
            }),
            // A discarded read arrives with the bytes already gone, exactly as a
            // real one does — the fetcher clears them before anything downstream
            // sees the page. Carrying a validator is the point: `remember` skips
            // pages with none, so without one this would pass for the wrong
            // reason.
            Answer::Discarded { body, etag } => Ok(Fetched::from_source(url, body, None)
                .discarded()
                .with_etag(etag)),
            Answer::Fail => Err(SourceError::Network("scripted failure".to_owned())),
        }
    }

    async fn post_form(&self, _url: &str, _fields: &[(&str, &str)]) -> SourceResult<Fetched> {
        self.log.posts.fetch_add(1, Ordering::SeqCst);
        Ok(Fetched {
            final_url: URL.to_owned(),
            body: "<html>posted</html>".to_owned(),
            content_type: None,
            etag: None,
            last_modified: None,
            provenance: Provenance::Source,
            fetch_class: FetchClass::Content,
            discarded: false,
        })
    }

    async fn get_conditional(
        &self,
        url: &str,
        known: Option<&RevisionValidators>,
    ) -> SourceResult<ConditionalFetch> {
        let index = self.advance(known);
        match self.answer(index) {
            Answer::NotModified => {
                // A `304` to a request carrying no validator is a protocol fault,
                // so the script refuses to produce one: reaching here without a
                // condition would be the test asserting something impossible.
                assert!(
                    known.is_some_and(|v| v.etag.is_some() || v.last_modified.is_some()),
                    "the script answered 304 to an unconditional request"
                );
                Ok(ConditionalFetch::NotModified)
            }
            // This is the arm the cache actually reaches — `remember` is fed from
            // here, not from `get` — so without it the discarded-cache test would
            // exercise a path the cache never takes.
            Answer::Discarded { body, etag } => Ok(ConditionalFetch::Fetched(
                Fetched::from_source(url, body, None)
                    .discarded()
                    .with_etag(etag),
            )),
            Answer::Page { body, etag } => Ok(ConditionalFetch::Fetched(Fetched {
                final_url: url.to_owned(),
                body,
                content_type: Some("text/html".to_owned()),
                etag,
                last_modified: None,
                provenance: Provenance::Source,
                fetch_class: FetchClass::Content,
                discarded: false,
            })),
            Answer::Fail => Err(SourceError::Network("scripted failure".to_owned())),
        }
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Harness {
    dir: PathBuf,
    tdb: test_support::TestDb,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("lorehaven-revisions-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");

        let tdb = test_support::TestDb::connect_with_dir(tag, &dir).await;
        let report: Vec<String> = tdb.applied_migrations().to_vec();
        assert!(
            report.contains(&"0007_revision_cache".to_owned()),
            "the revision cache migration must apply: {report:?}"
        );
        Self { dir, tdb }
    }

    /// The cache under test, wrapping `inner` under the given scope.
    fn caching<'a>(
        &'a self,
        inner: Scripted,
        scope: &str,
        adapter: &str,
    ) -> CachingFetcher<'a, Scripted> {
        CachingFetcher::new(
            inner,
            self.tdb.db(),
            self.dir.join("storage"),
            SOURCE,
            adapter,
            scope,
            604800, // 7 days, same as the default
        )
    }
}

// ---------------------------------------------------------------------------
// 1. A 304 serves the bytes we already hold
// ---------------------------------------------------------------------------

/// A discarded read is not filed in the cache (spec §11.5 amendment §1.2: a
/// metadata fetch writes "no cache fill").
///
/// The failure this prevents is specific. `remember` files a page whenever it
/// carries a validator, and the body of a discarded read is empty by the time it
/// gets there. Caching it would store a zero-byte entry with a live `ETag`, and
/// every later read of that URL would be answered `304` and served *nothing*,
/// forever. A cache that remembers having seen a page it does not have is worse
/// than one that forgot.
#[tokio::test]
async fn a_discarded_read_is_not_filed_in_the_cache() {
    let harness = Harness::new("discarded-cache").await;
    let inner = Scripted::new(vec![
        // Discarded, but it DOES carry a validator — so the discard check is the
        // only thing that can keep it out of the cache.
        Answer::Discarded {
            body: "<html>the forbidden chapter, in full</html>".to_owned(),
            etag: Some("\"v1\"".to_owned()),
        },
        // A real page, which the cache would answer from what it held if the
        // discarded read had been filed.
        Answer::Page {
            body: "<html>the real first revision</html>".to_owned(),
            etag: Some("\"v2\"".to_owned()),
        },
    ]);
    let log = inner.log();
    let cache = harness.caching(inner, PUBLIC_SCOPE, ADAPTER);

    let first = cache.get(URL).await.expect("the discarded read");
    assert!(
        first.discarded,
        "the page says it was discarded, and it is the flag — not the body's length — that keeps \
         it out of the cache"
    );

    let second = cache.get(URL).await.expect("the second read");
    assert_eq!(
        second.body, "<html>the real first revision</html>",
        "the second read must come from the source, not from a cache entry filed by the \
         discarded read — a zero-byte cache entry would be served here and parsed as an empty \
         document"
    );

    let calls = log.calls.lock().expect("the log is not poisoned").clone();
    assert_eq!(
        calls[1], None,
        "and no validator was offered on the second read, because the discarded one was never \
         filed: a stale \"v1\" would have been sent against the wrong revision"
    );

    harness.cleanup();
}

#[tokio::test]
async fn a_304_serves_the_body_the_source_says_is_unchanged() {
    let harness = Harness::new("304").await;
    let inner = Scripted::new(vec![
        Answer::Page {
            body: "<html>first revision</html>".to_owned(),
            etag: Some("\"v1\"".to_owned()),
        },
        Answer::NotModified,
    ]);
    let log = inner.log();
    let cache = harness.caching(inner, PUBLIC_SCOPE, ADAPTER);

    let first = cache.get(URL).await.expect("the first read");
    assert_eq!(first.body, "<html>first revision</html>");
    assert_eq!(first.etag.as_deref(), Some("\"v1\""));

    // The second read is offered the validator, and the source says unchanged.
    let second = cache.get(URL).await.expect("the second read");
    assert_eq!(
        second.body, "<html>first revision</html>",
        "a 304 must serve the bytes already held, not an empty body"
    );

    let calls = log.calls.lock().expect("the log is not poisoned").clone();
    assert_eq!(calls.len(), 2);
    assert!(
        calls[0].is_none(),
        "the first read had nothing to offer conditionally"
    );
    assert_eq!(
        calls[1],
        Some((Some("\"v1\"".to_owned()), None)),
        "the second read must offer the stored validator"
    );

    harness.cleanup();
}

#[tokio::test]
async fn a_changed_page_replaces_the_cached_one() {
    let harness = Harness::new("changed").await;
    let inner = Scripted::new(vec![
        Answer::Page {
            body: "<html>old</html>".to_owned(),
            etag: Some("\"v1\"".to_owned()),
        },
        Answer::Page {
            body: "<html>new</html>".to_owned(),
            etag: Some("\"v2\"".to_owned()),
        },
        Answer::NotModified,
    ]);
    let cache = harness.caching(inner, PUBLIC_SCOPE, ADAPTER);

    cache.get(URL).await.expect("first");
    let second = cache.get(URL).await.expect("second");
    assert_eq!(second.body, "<html>new</html>");
    assert_eq!(second.etag.as_deref(), Some("\"v2\""));

    // The third read must offer the *new* validator, proving the entry was
    // replaced rather than accumulated.
    let third = cache.get(URL).await.expect("third");
    assert_eq!(third.body, "<html>new</html>");
    assert_eq!(revisions::count(harness.tdb.db()).await.expect("count"), 1);

    harness.cleanup();
}

// ---------------------------------------------------------------------------
// 2. A credentialed read is not a public read
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_credentialed_read_is_never_served_to_a_public_one() {
    let harness = Harness::new("scope").await;

    let private = Scripted::new(vec![Answer::Page {
        body: "<html>gated</html>".to_owned(),
        etag: Some("\"gated\"".to_owned()),
    }]);
    harness
        .caching(private, "pseud-1", ADAPTER)
        .get(URL)
        .await
        .expect("the credentialed read");

    // The same URL, read anonymously. There is no entry under `public`, so the
    // source is read in full — and the answer is whatever the public sees.
    let public = Scripted::new(vec![Answer::Page {
        body: "<html>public</html>".to_owned(),
        etag: Some("\"public\"".to_owned()),
    }]);
    let log = public.log();
    let served = harness
        .caching(public, PUBLIC_SCOPE, ADAPTER)
        .get(URL)
        .await
        .expect("the public read");

    assert_eq!(
        served.body, "<html>public</html>",
        "a public read must not be answered from another reader's credentialed entry"
    );
    let calls = log.calls.lock().expect("the log is not poisoned").clone();
    assert!(
        calls.first().is_some_and(Option::is_none),
        "the public read must have been made unconditionally, not offered the gated entry"
    );

    harness.cleanup();
}

#[tokio::test]
async fn two_readers_do_not_share_a_credentialed_entry() {
    let harness = Harness::new("readers").await;

    let first = Scripted::new(vec![Answer::Page {
        body: "<html>reader one</html>".to_owned(),
        etag: Some("\"one\"".to_owned()),
    }]);
    harness
        .caching(first, "pseud-1", ADAPTER)
        .get(URL)
        .await
        .expect("reader one's read");

    let second = Scripted::new(vec![Answer::Page {
        body: "<html>reader two</html>".to_owned(),
        etag: Some("\"two\"".to_owned()),
    }]);
    let log = second.log();
    let served = harness
        .caching(second, "pseud-2", ADAPTER)
        .get(URL)
        .await
        .expect("reader two's read");

    assert_eq!(served.body, "<html>reader two</html>");
    assert!(
        log.calls
            .lock()
            .expect("the log is not poisoned")
            .first()
            .is_some_and(Option::is_none),
        "a second reader must be read fresh, not offered the first reader's revision"
    );

    harness.cleanup();
}

// ---------------------------------------------------------------------------
// 3. An adapter's entries die with the adapter
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_new_adapter_version_stops_using_the_old_entries() {
    let harness = Harness::new("adapter").await;

    let old = Scripted::new(vec![Answer::Page {
        body: "<html>parsed by 0.1.0</html>".to_owned(),
        etag: Some("\"v1\"".to_owned()),
    }]);
    harness
        .caching(old, PUBLIC_SCOPE, "0.1.0")
        .get(URL)
        .await
        .expect("the old adapter's read");

    let new = Scripted::new(vec![Answer::Page {
        body: "<html>parsed by 0.2.0</html>".to_owned(),
        etag: Some("\"v1\"".to_owned()),
    }]);
    let log = new.log();
    let served = harness
        .caching(new, PUBLIC_SCOPE, "0.2.0")
        .get(URL)
        .await
        .expect("the new adapter's read");

    assert_eq!(served.body, "<html>parsed by 0.2.0</html>");
    assert!(
        log.calls
            .lock()
            .expect("the log is not poisoned")
            .first()
            .is_some_and(Option::is_none),
        "an adapter version the entry was not filed under must not be offered its validator"
    );

    harness.cleanup();
}

// ---------------------------------------------------------------------------
// 4. The cache is never the reason a read fails
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_form_submission_is_never_answered_from_the_cache() {
    let harness = Harness::new("post").await;
    let inner = Scripted::new(vec![Answer::Page {
        body: "<html>page</html>".to_owned(),
        etag: Some("\"v1\"".to_owned()),
    }]);
    let log = inner.log();
    let cache = harness.caching(inner, PUBLIC_SCOPE, ADAPTER);

    // Prime the cache for the URL...
    cache.get(URL).await.expect("the page read");

    // ...then post to it. The source must be asked, and the answer must be the
    // source's, not the cached page's.
    let posted = cache
        .post_form(URL, &[("field", "value")])
        .await
        .expect("the post");
    assert_eq!(posted.body, "<html>posted</html>");
    assert_eq!(log.posts.load(Ordering::SeqCst), 1);

    harness.cleanup();
}

#[tokio::test]
async fn a_page_with_no_validators_is_not_stored() {
    let harness = Harness::new("novalidators").await;
    let inner = Scripted::new(vec![Answer::Page {
        body: "<html>no etag</html>".to_owned(),
        etag: None,
    }]);
    let log = inner.log();
    let cache = harness.caching(inner, PUBLIC_SCOPE, ADAPTER);

    cache.get(URL).await.expect("first");
    cache.get(URL).await.expect("second");

    assert_eq!(
        revisions::count(harness.tdb.db()).await.expect("count"),
        0,
        "a page with nothing to ask about should not fill the cache"
    );
    let calls = log.calls.lock().expect("the log is not poisoned").clone();
    assert_eq!(calls.len(), 2);
    assert!(
        calls[1].is_none(),
        "the second read had no validator to offer, correctly"
    );

    harness.cleanup();
}

#[tokio::test]
async fn an_expired_entry_is_not_offered() {
    let harness = Harness::new("expired").await;
    let inner = Scripted::new(vec![Answer::Page {
        body: "<html>fresh</html>".to_owned(),
        etag: Some("\"v1\"".to_owned()),
    }]);
    let cache = harness.caching(inner, PUBLIC_SCOPE, ADAPTER);
    cache.get(URL).await.expect("prime");

    // Age the entry past its expiry. The bytes stay; only the validator stops
    // being offered.
    let key = revisions::RevisionKey {
        source_key: SOURCE,
        revision_key: URL,
        adapter_version: ADAPTER,
        security_scope: PUBLIC_SCOPE,
    };
    let mut entry = revisions::find_fresh(harness.tdb.db(), &key)
        .await
        .expect("read")
        .expect("the entry exists");
    entry.expires_at = "2000-01-01T00:00:00Z".to_owned();
    revisions::upsert(harness.tdb.db(), &key, &entry)
        .await
        .expect("age");

    assert!(
        revisions::find_fresh(harness.tdb.db(), &key)
            .await
            .expect("read")
            .is_none(),
        "an expired entry must not be returned as fresh"
    );

    harness.cleanup();
}

#[tokio::test]
async fn purging_removes_only_the_expired_entries() {
    let harness = Harness::new("purge").await;

    // One entry that stays, one that goes.
    let live = harness.caching(
        Scripted::new(vec![Answer::Page {
            body: "<html>live</html>".to_owned(),
            etag: Some("\"live\"".to_owned()),
        }]),
        PUBLIC_SCOPE,
        ADAPTER,
    );
    live.get(URL).await.expect("the live entry");
    drop(live);

    let doomed = harness.caching(
        Scripted::new(vec![Answer::Page {
            body: "<html>doomed</html>".to_owned(),
            etag: Some("\"doomed\"".to_owned()),
        }]),
        PUBLIC_SCOPE,
        ADAPTER,
    );
    let doomed_url = "https://archiveofourown.org/works/91806026";
    doomed.get(doomed_url).await.expect("the doomed entry");
    drop(doomed);

    let key = revisions::RevisionKey {
        source_key: SOURCE,
        revision_key: doomed_url,
        adapter_version: ADAPTER,
        security_scope: PUBLIC_SCOPE,
    };
    let mut entry = revisions::find_fresh(harness.tdb.db(), &key)
        .await
        .expect("read")
        .expect("the entry exists");
    entry.expires_at = "2000-01-01T00:00:00Z".to_owned();
    revisions::upsert(harness.tdb.db(), &key, &entry)
        .await
        .expect("age");

    let purged = revisions::purge_expired(harness.tdb.db())
        .await
        .expect("purge");
    assert_eq!(purged, 1, "only the expired entry should go");
    assert_eq!(revisions::count(harness.tdb.db()).await.expect("count"), 1);
    assert!(
        revisions::find_fresh(
            harness.tdb.db(),
            &revisions::RevisionKey {
                source_key: SOURCE,
                revision_key: URL,
                adapter_version: ADAPTER,
                security_scope: PUBLIC_SCOPE,
            },
        )
        .await
        .expect("read")
        .is_some(),
        "the live entry must survive the purge"
    );

    harness.cleanup();
}

#[tokio::test]
async fn a_cache_whose_bytes_have_gone_falls_back_to_reading_the_page() {
    let harness = Harness::new("missingbytes").await;
    let inner = Scripted::new(vec![
        // 1: the first read, stored.
        Answer::Page {
            body: "<html>v1</html>".to_owned(),
            etag: Some("\"v1\"".to_owned()),
        },
        // 2: the conditional read, which the source says is unchanged — but the
        //    bytes it refers to are about to be gone.
        Answer::NotModified,
        // 3: the unconditional re-read the cache falls back to.
        Answer::Page {
            body: "<html>v1 again</html>".to_owned(),
            etag: Some("\"v1\"".to_owned()),
        },
    ]);
    let cache = harness.caching(inner, PUBLIC_SCOPE, ADAPTER);
    cache.get(URL).await.expect("prime");

    // Delete the blob, leaving the entry pointing at nothing — the state a
    // storage-pruning bug would leave behind.
    let key = revisions::RevisionKey {
        source_key: SOURCE,
        revision_key: URL,
        adapter_version: ADAPTER,
        security_scope: PUBLIC_SCOPE,
    };
    let entry = revisions::find_fresh(harness.tdb.db(), &key)
        .await
        .expect("read")
        .expect("the entry exists");
    // Through the store's own path helper, because blobs are sharded by
    // checksum: guessing the layout would make this test pass or fail on a
    // decision that is not its subject.
    std::fs::remove_file(BlobStore::new(harness.dir.join("storage")).path_for(&entry.checksum))
        .expect("remove the blob");

    // The reader gets a page rather than an error, and it is the page the source
    // was asked for — not an empty body standing in for "unchanged".
    let served = cache
        .get(URL)
        .await
        .expect("a missing blob must not fail the read");
    assert_eq!(
        served.body, "<html>v1 again</html>",
        "a 304 whose bytes are gone must fall back to reading the page"
    );

    harness.cleanup();
}

impl Harness {
    fn cleanup(&self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
