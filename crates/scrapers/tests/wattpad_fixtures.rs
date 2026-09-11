//! Wattpad, driven through recorded JSON, prose and bytes.
//!
//! Every assertion here is against something recorded from the live site on
//! 2026-09-11 and committed under `tests/fixtures/wattpad/`. Nothing reaches the
//! network, and nothing here needs a solver or an operator's allowance: the
//! recordings were taken through a plain client, because both halves of this
//! source answer one.
//!
//! # The one that is not like the others
//!
//! `part-text.html.gz` and `part-text.html` are the **same response**, recorded
//! twice: once as the site sent it (`content-encoding: gzip`, 12,514 bytes) and
//! once decompressed (25,434 bytes). They are here because Wattpad's prose
//! endpoint compressed a response that had explicitly asked for `identity` — and
//! a fetcher that passed those bytes through would have stored a chapter as
//! mojibake, reported success, and left the failure to whoever opened the
//! chapter. The pair makes that testable against real bytes rather than against
//! a fixture this repository compressed itself.
//!
//! The endpoint's behaviour varies by edge: measured the same day, a later
//! request with no `Accept-Encoding` at all was answered in plain text. So it is
//! not a property this adapter may assume either way, which is why the handling
//! lives in the shared fetcher and is asserted here on the real bytes.

use lorehaven_scrapers::safety::decode_response_body;
use lorehaven_scrapers::sites::wattpad::{Wattpad, HOST, SOURCE_KEY};
use lorehaven_scrapers::{SourceAdapter, SourceError, WorkStatus};
use time::macros::datetime;
use url::Url;

const STORY_URL: &str = "https://www.wattpad.com/story/410445604-the-older-swan-paul-lahote";
const STORY_ID: &str = "410445604";
/// The recorded work's `Chapter One`, whose whole prose is one part.
const CHAPTER_ONE_PART: &str = "1623966332";
/// Its `Photo Gallery` part: the site's first part, and not prose.
const GALLERY_PART: &str = "1623782492";

/// Read a recorded file as text.
fn fixture(name: &str) -> String {
    let path = format!("tests/fixtures/wattpad/{name}");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("the recorded fixture {path} must exist: {error}"))
}

/// Read a recorded file as bytes.
fn fixture_bytes(name: &str) -> Vec<u8> {
    let path = format!("tests/fixtures/wattpad/{name}");
    std::fs::read(&path)
        .unwrap_or_else(|error| panic!("the recorded fixture {path} must exist: {error}"))
}

fn url(raw: &str) -> Url {
    Url::parse(raw).expect("a fixture URL must parse")
}

/// The recorded work.
fn recorded_work() -> lorehaven_scrapers::SourceWork {
    Wattpad::new()
        .preview_from_html(&fixture("story.json"), &url(STORY_URL))
        .expect("the recorded story document must parse")
}

#[test]
fn the_story_document_is_read_as_the_work_the_site_describes() {
    let work = recorded_work();

    assert_eq!(work.source_key.as_str(), SOURCE_KEY);
    assert_eq!(work.source_work_key, STORY_ID);
    assert_eq!(work.title, "The Older Swan | Paul Lahote");
    // Canonical without the slug, which the site serves and which the title can
    // invalidate.
    assert_eq!(work.source_url, "https://www.wattpad.com/story/410445604");
}

#[test]
fn the_author_is_named_by_the_document_and_linked_by_the_sites_own_shape() {
    let work = recorded_work();

    assert_eq!(work.author_text, "love-yourself-xoxo");
    // The document names the author and does not link them; `/user/{name}` is
    // the address the site's own markup uses for the same author.
    assert_eq!(
        work.author_url.as_deref(),
        Some("https://www.wattpad.com/user/love-yourself-xoxo")
    );
}

#[test]
fn the_summary_is_the_description_the_author_wrote() {
    let work = recorded_work();

    assert!(
        work.summary
            .starts_with("\"I'm sorry, you're a what now?\""),
        "the description is the summary: {:?}",
        work.summary
    );
    assert!(work.summary.contains("PaulxFemOC"));
}

#[test]
fn the_language_and_tags_are_the_ones_the_document_carries() {
    let work = recorded_work();

    assert_eq!(work.language.as_deref(), Some("English"));
    assert!(work.tags.contains(&"fanfiction".to_owned()));
    assert!(work.tags.contains(&"paullahote".to_owned()));
    assert_eq!(work.tags.len(), 13, "tags: {:?}", work.tags);
    assert_eq!(work.tags[12], "wolfpack");
}

#[test]
fn the_timestamps_are_the_documents_own_and_are_read_as_utc() {
    // `2026-04-22T02:39:18Z` — an explicit `Z`, which is a zone rather than a
    // guess. Read as local time they would be hours out.
    let work = recorded_work();

    assert_eq!(work.published_at, Some(datetime!(2026-04-22 02:39:18 UTC)));
    assert_eq!(work.updated_at, Some(datetime!(2026-09-08 04:00:57 UTC)));
}

#[test]
fn a_word_count_is_not_reported_because_the_site_publishes_characters() {
    // `length: 291689` is what the document carries, and a page of the same
    // chapter states `wordCount: 3676` against `length: 18380` — a ratio of
    // five, which is what a character count looks like. Reporting the character
    // count as a word count would be wrong by a factor a reader would not check,
    // so the honest answer is none at all.
    let work = recorded_work();
    assert_eq!(work.word_count, None);
}

#[test]
fn the_part_list_is_complete_in_the_order_the_site_publishes_it() {
    let work = recorded_work();

    assert_eq!(work.chapter_count(), 31);
    assert_eq!(work.chapters[0].ordinal, 1);
    assert_eq!(work.chapters[0].title, "Photo Gallery");
    assert_eq!(work.chapters[3].title, "Chapter One");
    assert_eq!(work.chapters[30].ordinal, 31);
    // The author's own typo, kept because it is the title the site shows.
    assert_eq!(work.chapters[25].title, "Chapter Twnety-Three");

    let ordinals: Vec<u32> = work.chapters.iter().map(|entry| entry.ordinal).collect();
    assert_eq!(ordinals, (1..=31).collect::<Vec<u32>>());
}

#[test]
fn a_part_is_keyed_by_the_sites_own_id_because_that_is_what_its_reader_takes() {
    // The prose endpoint is addressed by the part's id, so the id is the
    // identity and the position is not: inserting a part would renumber every
    // chapter after it.
    let work = recorded_work();

    assert_eq!(work.chapters[3].source_chapter_key, CHAPTER_ONE_PART);
    assert_eq!(work.chapters[0].source_chapter_key, GALLERY_PART);
}

#[test]
fn an_unfinished_work_is_ongoing() {
    // `completed: false` is the document's own flag, and unlike FanFiction.net's
    // `Status:` field it is always present — which is what makes reading the
    // negative here the truth rather than an assumption.
    assert_eq!(recorded_work().status, WorkStatus::Ongoing);
}

#[test]
fn a_missing_story_is_not_found_rather_than_a_parse_failure() {
    // The site reports it as a JSON error body with HTTP 400, so the status code
    // is not where this is decided.
    let err = Wattpad::new()
        .preview_from_html(&fixture("missing-story.json"), &url(STORY_URL))
        .expect_err("a missing story must not parse as a work");
    assert!(
        matches!(err, SourceError::NotFound),
        "expected NotFound, got {err:?}"
    );
}

#[test]
fn a_page_that_is_not_the_sites_json_is_a_parse_failure() {
    // The distinction the recordings exist to make: a shape that changed must
    // not be reported as a work that was deleted — and the site's missing-work
    // page is HTML, so it is a different thing from its missing-work JSON.
    let err = Wattpad::new()
        .preview_from_html(&fixture("missing-story.html"), &url(STORY_URL))
        .expect_err("an HTML error page is not a story document");
    assert!(
        matches!(err, SourceError::Parse(_)),
        "expected Parse, got {err:?}"
    );
}

#[test]
fn a_document_that_answers_for_another_story_is_refused() {
    // A document served under the wrong id would attach a work to somebody
    // else's row, which is worse than a failed import.
    let err = Wattpad::new()
        .preview_from_html(
            &fixture("story.json"),
            &url("https://www.wattpad.com/story/1"),
        )
        .expect_err("a document for another story is not this story");
    assert!(
        matches!(err, SourceError::Parse(_)),
        "expected Parse, got {err:?}"
    );
}

#[test]
fn a_chapter_is_read_from_the_fragment_the_text_endpoint_returns() {
    let work = recorded_work();
    let chapter = Wattpad::new().chapter_from_text(
        &fixture("part-text.html"),
        &work,
        4,
        Some(&url(STORY_URL)),
    );

    // The caller supplies the ordinal because the fragment cannot: the prose
    // endpoint is addressed by the chapter's id, so the response carries no
    // marker saying which chapter it is.
    assert_eq!(chapter.ordinal, 4);
    assert_eq!(chapter.source_chapter_key, CHAPTER_ONE_PART);
    assert_eq!(chapter.title, "Chapter One");

    assert!(
        chapter
            .content_html
            .contains("The slamming of dresser drawers"),
        "the prose must survive sanitation: {}",
        &chapter.content_html[..chapter.content_html.len().min(300)]
    );
    assert!(chapter.content_html.contains("<p>"));
    // The paragraph anchors are the site's own, not content.
    assert!(!chapter.content_html.contains("data-p-id"));
    assert!(!chapter.content_html.contains("<script"));
}

#[test]
fn a_part_that_is_a_gallery_is_still_read_as_the_text_it_contains() {
    // The recorded work's first part is a `Photo Gallery`: 54 characters of the
    // same markup, holding an image. It is a part the reader is shown, so it is
    // a chapter here — the site lists it, and dropping it silently would report
    // the work as shorter than its author published it.
    let work = recorded_work();
    let chapter = Wattpad::new().chapter_from_text(
        &fixture("part-empty.html"),
        &work,
        1,
        Some(&url(STORY_URL)),
    );

    assert_eq!(chapter.title, "Photo Gallery");
    assert!(chapter.content_html.contains("Hazel"));
    // And the image is gone rather than pointed at (spec §11.5): a stored
    // chapter must not make the reader's browser fetch from a third party.
    assert!(!chapter.content_html.contains("<img"));
    assert!(!chapter.content_html.contains("img.wattpad.com"));
}

#[test]
fn the_bytes_the_site_sent_gzip_inflate_to_the_text_it_meant() {
    // The pair of recordings, and the reason both are here. The compressed
    // fixture is what the site actually sent; the plain one is what a reader
    // should get. This asserts the fetcher's own decoder against real bytes
    // rather than against a fixture this repository compressed itself.
    let compressed = fixture_bytes("part-text.html.gz");
    let plain = fixture("part-text.html");

    assert!(compressed.len() < plain.len());
    assert_eq!(
        decode_response_body(&compressed, Some("text/plain; charset=UTF-8"), Some("gzip")),
        plain,
        "the recorded gzip must inflate to the recorded text"
    );

    // And the failure this guards against, shown rather than described: read as
    // text without undoing the compression, the same bytes are not the page.
    let read_as_text = decode_response_body(&compressed, Some("text/plain; charset=UTF-8"), None);
    assert_ne!(read_as_text, plain);
    assert!(
        !read_as_text.contains("The slamming of dresser drawers"),
        "the prose is not recoverable without inflating"
    );
}

#[test]
fn the_sites_own_file_forbids_the_path_the_prose_comes_from() {
    // Why this adapter's chapter half is an operator's decision rather than a
    // default. The rules are parsed by the same parser the fetcher uses, against
    // the same product token, so this is the site's instruction and not a
    // summary of it.
    use lorehaven_scrapers::robots::RobotsRules;

    let rules = RobotsRules::parse(&fixture("robots.txt"), "Lorehaven");

    assert!(
        !rules.allows("/apiv2/?m=storytext&id=1623966332&page="),
        "the prose endpoint is disallowed by the site's own rules"
    );
    // And the metadata path this adapter previews through is permitted, which is
    // what makes a preview possible with no allowance from anybody.
    assert!(rules.allows("/api/v3/stories/410445604"));
    assert!(rules.allows("/story/410445604-the-older-swan-paul-lahote"));
    // The site publishes no crawl delay, so the fetcher's one-second floor is
    // the pace — and "no information" is not permission to go faster.
    assert_eq!(rules.crawl_delay(), None);
}

#[test]
fn the_adapter_declares_what_the_source_has_and_not_what_an_instance_may_do() {
    let capabilities = Wattpad::new().capabilities();

    // `chapters: true` is a statement about the *source*: Wattpad serves every
    // part's prose and this adapter reads it. Whether a given instance may fetch
    // it is that instance's `imports.honour_robots`, answered per instance and
    // not compiled into an adapter — and reporting `false` here would tell a
    // compliant instance that chapters are impossible, which is a different
    // claim and a wrong one.
    assert!(capabilities.chapters);
    assert!(capabilities.metadata);
    assert!(capabilities.per_chapter_fetch);
    assert!(capabilities.incremental);
    // Author pages exist and their markup has not been recorded.
    assert!(!capabilities.bibliography);
    assert_eq!(capabilities.min_interval_millis, Some(1_000));
    assert_eq!(HOST, "wattpad.com");
    assert_eq!(Wattpad::new().hosts(), vec!["wattpad.com"]);
    assert_eq!(Wattpad::new().display_name(), "Wattpad");
}
