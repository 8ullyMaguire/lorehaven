//! CHYOA, driven through recorded pages.
//!
//! Every assertion here is against something recorded on 2026-09-20 and
//! committed under `tests/fixtures/chyoa/` — see the `## chyoa` section of
//! `tests/fixtures/README.md` for where each page came from.

use lorehaven_scrapers::sites::chyoa::Chyoa;
use lorehaven_scrapers::SourceAdapter;
use url::Url;

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/chyoa")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} is not readable: {error}", path.display()))
}

const STORY_URL: &str = "https://chyoa.com/story/The-Waifu-Catalog--Beta-Testers.47833";
const CHAPTER_URL: &str = "https://chyoa.com/chapter/Introduction.1215015";

#[test]
fn the_story_page_parses_metadata_and_chapter_list() {
    let adapter = Chyoa::new();
    let url = Url::parse(STORY_URL).unwrap();
    let work = adapter
        .preview_from_html(&fixture("story.html"), &url)
        .expect("the recorded story page parses");

    assert_eq!(work.title, "The Waifu Catalog- Beta Testers");
    assert_eq!(work.author_text, "Jerynboe");
    assert!(
        work.summary.contains("beta testing"),
        "summary should be from the meta description"
    );
    assert!(
        work.author_url
            .as_ref()
            .is_some_and(|u| u.contains("/user/Jerynboe")),
        "author url should point to the contributor's profile"
    );

    // Branch links on the page give 10 unique chapter links.
    assert!(
        work.chapters.len() >= 5,
        "expected at least 5 chapters from the branch list, got {}",
        work.chapters.len()
    );

    // Tags are extracted from anchors matching /tag/{name}.
    assert!(
        work.tags.iter().any(|t| t.to_lowercase().contains("waifu")),
        "tags should include 'waifu catalog'"
    );
}

#[test]
fn the_chapter_page_parses_its_body() {
    let adapter = Chyoa::new();
    let url = Url::parse(CHAPTER_URL).unwrap();
    let work = adapter
        .preview_from_html(&fixture("chapter.html"), &url)
        .expect("the recorded chapter page parses");

    assert_eq!(work.title, "The Waifu Catalog- Beta Testers");
    // A chapter page still carries the full branch list (same markup as the
    // story page), so the preview reflects all listed branches.
    assert!(
        work.chapters.len() >= 5,
        "expected at least 5 chapters from the chapter page's branch list, got {}",
        work.chapters.len()
    );

    // chapters_from_html on the chapter fixture returns the body.
    let chapters = adapter
        .chapters_from_html(&fixture("chapter.html"), &work)
        .expect("chapter fixture parses to a chapter");
    assert_eq!(chapters.len(), 1);
    assert!(
        chapters[0].content_html.contains("endless sky")
            || chapters[0].content_html.contains("Company"),
        "chapter body should contain prose from the story"
    );
}

#[test]
fn story_url_is_recognised() {
    let adapter = Chyoa::new();
    let url = Url::parse(STORY_URL).unwrap();
    assert!(adapter.can_handle(&url));
}

#[test]
fn chapter_url_is_recognised() {
    let adapter = Chyoa::new();
    let url = Url::parse(CHAPTER_URL).unwrap();
    assert!(adapter.can_handle(&url));
}

#[test]
fn unrelated_urls_are_not_claimed() {
    let adapter = Chyoa::new();
    let url = Url::parse("https://example.com/story/123").unwrap();
    assert!(!adapter.can_handle(&url));
}
