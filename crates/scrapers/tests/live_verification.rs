//! Live verification: does each adapter still read its source, today?
//!
//! # Why this is not part of the normal test run
//!
//! Every other test in this crate is offline, and that is deliberate: a test
//! that reaches the network fails on a plane, fails when a site is down, and
//! fails when a site changes its markup — three different problems reported as
//! one red test. The fixture tests are the ones that guard the parsers, and
//! they run everywhere.
//!
//! This file answers a different question, and spec §11.7 requires it: *has the
//! source changed under us?* A parser that matches a recorded page from last
//! month proves nothing about today. So these tests are `#[ignore]`d and run
//! deliberately:
//!
//! ```text
//! cargo test -p lorehaven-scrapers --test live_verification -- --ignored --nocapture
//! ```
//!
//! # What they assert
//!
//! Only facts that are stable and that a real parse must produce: a work exists,
//! has a title, and has the chapters the site says it has. They do not assert
//! exact chapter titles or summaries — those change whenever the author edits,
//! and a test that fails because somebody fixed a typo is a test that gets
//! deleted.
//!
//! Each one is polite: one request a second is the floor the fetcher enforces
//! from the source's own `robots.txt`, so a full Syosetu walk takes about ten
//! seconds and that is the correct cost.

use std::time::Duration;

use lorehaven_scrapers::safety::{FetchPolicy, SafeFetcher};
use lorehaven_scrapers::{sites, SourceAdapter, SourceError, SourceKey, WorkStatus};
use url::Url;

/// A fetcher for one source, built the way the importer builds it.
fn fetcher_for(key: &str) -> SafeFetcher {
    let registry = sites::default_registry();
    let adapter = registry
        .by_key(&SourceKey::new(key))
        .unwrap_or_else(|error| panic!("no adapter for {key}: {error}"));
    let mut policy = FetchPolicy::for_source(adapter.capabilities());
    // Generous, and deliberately so: this is a manual run against a live site,
    // and a source that is briefly slow should look like a slow source rather
    // than like a parser that no longer works. A failure here is a reason to
    // re-run before it is a reason to edit the parser.
    policy.timeout = Duration::from_secs(60);
    SafeFetcher::new(adapter.hosts(), policy)
}

/// The adapter as a value, so its methods can be called.
///
/// The registry hands out a borrow, and these tests want to call through to the
/// adapter more than once. Building one directly keeps the test honest: it
/// exercises the same type the registry would have routed to.
fn adapter_for(key: &str) -> Box<dyn SourceAdapter> {
    match key {
        "ao3" => Box::new(sites::ao3::ArchiveSoftware::new()),
        "efiction" => Box::new(sites::efiction::Efiction::new()),
        "royalroad" => Box::new(sites::royalroad::RoyalRoad::new()),
        "syosetu" => Box::new(sites::syosetu::Syosetu::new()),
        other => panic!("no owned constructor for {other}"),
    }
}

async fn preview(key: &str, raw: &str) -> lorehaven_scrapers::SourceWork {
    let adapter = adapter_for(key);
    let url = Url::parse(raw).expect("the fixture URL parses");
    assert!(
        adapter.can_handle(&url),
        "{key} claims {raw} but can_handle said no"
    );
    let fetch = fetcher_for(key);
    adapter
        .preview(&fetch, &url, None)
        .await
        .unwrap_or_else(|error| panic!("{key} could not read {raw}: {error}"))
}

#[tokio::test]
#[ignore = "reaches the network; run deliberately"]
async fn archive_of_our_own_still_reads() {
    let work = preview("ao3", "https://archiveofourown.org/works/92356871").await;
    println!(
        "ao3: {:?} by {:?}, {} chapters, {:?} words, {:?}",
        work.title,
        work.author_text,
        work.chapter_count(),
        work.word_count,
        work.status
    );
    assert!(!work.title.trim().is_empty(), "no title");
    assert!(!work.author_text.trim().is_empty(), "no author");
    // Not an exact number: a work gains chapters whenever its author posts, and
    // a test that fails for that is a test that gets deleted. What must hold is
    // that the chapter list was found and carries real identities.
    assert!(work.chapter_count() >= 1, "no chapters listed");
    assert!(
        work.chapters
            .iter()
            .all(|chapter| !chapter.source_chapter_key.is_empty()),
        "a chapter has no source key, so the list was not really parsed"
    );
    assert!(
        work.updated_at.is_some(),
        "the page carries a date and the parser did not find it"
    );
}

#[tokio::test]
#[ignore = "reaches the network; run deliberately"]
async fn royal_road_still_reads() {
    let work = preview(
        "royalroad",
        "https://www.royalroad.com/fiction/21220/mother-of-learning",
    )
    .await;
    println!(
        "royalroad: {:?} by {:?}, {} chapters, {:?} words, {:?}",
        work.title,
        work.author_text,
        work.chapter_count(),
        work.word_count,
        work.status
    );
    assert!(!work.title.trim().is_empty(), "no title");
    assert!(!work.author_text.trim().is_empty(), "no author");
    assert!(
        work.author_url.is_some(),
        "the author URL is on the page; a blank one means the selector moved"
    );
    assert!(
        work.chapter_count() > 100,
        "a long work lost its chapter list"
    );
    assert!(work.word_count.unwrap_or(0) > 0, "no word count");
    // The recorded fixture is a completed work. If this fails the site changed
    // and the fixture, not the parser, is what needs re-reading.
    assert_eq!(
        work.status,
        WorkStatus::Complete,
        "status changed at the source"
    );
    assert!(
        work.published_at.is_some() && work.updated_at.is_some(),
        "the JSON-LD dates are gone"
    );
}

#[tokio::test]
#[ignore = "reaches the network; run deliberately"]
async fn syosetu_still_reads() {
    let work = preview("syosetu", "https://ncode.syosetu.com/n2267be/").await;
    println!(
        "syosetu: {:?} by {:?}, {} episodes, {:?}",
        work.title,
        work.author_text,
        work.chapter_count(),
        work.word_count
    );
    assert!(!work.title.trim().is_empty(), "no title");
    assert!(!work.author_text.trim().is_empty(), "no author");
    // The whole reason this adapter reads the info page and every list page:
    // the work page alone lists 100 and says nothing about the rest.
    assert!(
        work.chapter_count() > 100,
        "only {} episodes listed: the paginated walk stopped early",
        work.chapter_count()
    );
}

#[tokio::test]
#[ignore = "reaches the network; run deliberately"]
async fn a_chapter_body_still_comes_back_with_text() {
    // The failure this guards against is the one the ported code had: a failed
    // chapter fetch that returned an empty string and reported success.
    let adapter = adapter_for("royalroad");
    let url = Url::parse("https://www.royalroad.com/fiction/21220/mother-of-learning")
        .expect("the fixture URL parses");
    let fetch = fetcher_for("royalroad");
    let work = adapter
        .preview(&fetch, &url, None)
        .await
        .expect("the work reads");
    // One chapter, not all 109: this is the per-chapter path, and it is the one
    // a retry uses. Reading the whole work would take two minutes at the
    // source's own pace to prove nothing extra.
    assert!(
        adapter.capabilities().per_chapter_fetch,
        "the adapter advertises per-chapter fetch, so this path must exist"
    );
    let chapter = adapter
        .fetch_chapter(&fetch, &work, 1, None)
        .await
        .expect("chapter 1 reads on its own");
    println!(
        "royalroad chapter {}: {:?}, {} bytes of html",
        chapter.ordinal,
        chapter.title,
        chapter.content_html.len()
    );
    assert_eq!(chapter.ordinal, 1, "the ordinal is not the one asked for");
    assert!(
        !chapter.source_chapter_key.is_empty(),
        "the chapter has no source key"
    );
    assert!(
        chapter.content_html.len() > 500,
        "chapter 1 came back with {} bytes: an empty body reported as success",
        chapter.content_html.len()
    );
    assert!(
        chapter.content_html.contains('<'),
        "the body is not html, so nothing was sanitised"
    );
    // And a chapter the work does not have is refused rather than invented.
    let missing = adapter.fetch_chapter(&fetch, &work, 5000, None).await;
    assert!(
        missing.is_err(),
        "a chapter past the end of the work was reported as read"
    );
}

#[tokio::test]
#[ignore = "reaches the network; run deliberately"]
async fn the_efiction_family_still_reads() {
    // giantessworld.net publishes no robots.txt at all, so the fallback pacing of
    // one request a second applies and the work page and its chapters are open.
    // It is one of nine reachable members of the family; see
    // `tests/fixtures/README.md` for what the other thirteen answer with.
    let work = preview(
        "efiction",
        "https://www.giantessworld.net/viewstory.php?sid=11369&index=1",
    )
    .await;
    println!(
        "giantessworld: {:?} by {:?}, {} chapters, {:?} words, {:?}, {:?}",
        work.title,
        work.author_text,
        work.chapter_count(),
        work.word_count,
        work.status,
        work.rating_text
    );
    assert!(!work.title.trim().is_empty(), "no title");
    assert!(!work.author_text.trim().is_empty(), "no author");
    assert!(
        work.chapter_count() >= 1,
        "no chapters listed, so the chapter walk found nothing"
    );
    assert!(
        work.chapters
            .iter()
            .all(|chapter| !chapter.source_chapter_key.is_empty()),
        "a chapter has no source key"
    );
    assert!(
        work.chapters
            .windows(2)
            .all(|pair| pair[0].ordinal < pair[1].ordinal),
        "the chapter list is not in order"
    );
    assert!(
        !work.summary.trim().is_empty(),
        "no summary, so neither the label nor the summary element was read"
    );
    // Every key must be the member's own `chapid` rather than the ordinal. A
    // fallback key is indistinguishable from a real one until an author inserts a
    // chapter, which is exactly when a reader's place in the work is lost.
    assert!(
        work.chapters.iter().all(|chapter| chapter
            .source_chapter_key
            .chars()
            .all(|c| c.is_ascii_digit())),
        "a chapter key is not a chapid: {:?}",
        work.chapters
            .iter()
            .map(|chapter| chapter.source_chapter_key.as_str())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
#[ignore = "reaches the network; run deliberately"]
async fn an_efiction_member_that_disallows_the_archive_is_refused_by_its_own_rules() {
    // `tgstorytime.com` serves `User-agent: *` / `Disallow: /` and nothing else.
    // It is one of the two members whose markup is recorded under
    // `tests/fixtures/efiction/`, so the parser is known to read it — and the
    // import must still refuse it, because the archive's own instructions are the
    // one thing an importer does not get to override. This asserts the refusal
    // comes from the rules and not from the parser: a 403 or a parse failure here
    // would mean the fetcher never consulted robots.txt at all.
    let adapter = adapter_for("efiction");
    let fetch = fetcher_for("efiction");
    let url = Url::parse("https://www.tgstorytime.com/viewstory.php?sid=6369&index=1")
        .expect("the URL parses");

    let error = adapter
        .preview(&fetch, &url, None)
        .await
        .expect_err("tgstorytime disallows the whole site, so nothing may be read");
    println!("tgstorytime: {error}");
    assert!(
        matches!(error, SourceError::Refused(_)),
        "a robots refusal is a refusal, not {error:?}"
    );
    assert!(
        format!("{error}").contains("robots.txt"),
        "the refusal must name the rule it came from: {error}"
    );
}

#[tokio::test]
#[ignore = "reaches the network; run deliberately"]
async fn an_efiction_chapter_body_still_comes_back_with_text() {
    // The failure this guards against is the one the ported code had: a failed
    // chapter fetch that returned an empty string and reported success.
    let adapter = adapter_for("efiction");
    let url = Url::parse("https://www.giantessworld.net/viewstory.php?sid=11369&index=1")
        .expect("the URL parses");
    let fetch = fetcher_for("efiction");
    let work = adapter
        .preview(&fetch, &url, None)
        .await
        .expect("the work reads");

    assert!(
        adapter.capabilities().per_chapter_fetch,
        "the adapter advertises per-chapter fetch, so this path must exist"
    );
    let chapter = adapter
        .fetch_chapter(&fetch, &work, 1, None)
        .await
        .expect("chapter 1 reads on its own");
    println!(
        "efiction chapter {}: {:?}, {} bytes of html",
        chapter.ordinal,
        chapter.title,
        chapter.content_html.len()
    );
    assert_eq!(chapter.ordinal, 1, "the ordinal is not the one asked for");
    assert!(
        !chapter.source_chapter_key.is_empty(),
        "the chapter has no source key"
    );
    assert!(
        chapter.content_html.len() > 500,
        "chapter 1 came back with {} bytes: an empty body reported as success",
        chapter.content_html.len()
    );
    // ^ A live run says the chapter is still readable; *which* URL was asked for
    // is asserted against a recording fetcher in `efiction_fixtures.rs`, because
    // `SafeFetcher` does not keep a log of what it fetched and a test that read
    // its own request would be testing the wrong object.
    //
    // A chapter the work does not have is refused rather than invented.
    assert!(
        adapter
            .fetch_chapter(&fetch, &work, 5000, None)
            .await
            .is_err(),
        "a chapter past the end of the work was reported as read"
    );
}
