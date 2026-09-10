//! The AO3 adapter against recorded pages (spec §11.7: "Add representative
//! fixtures").
//!
//! Every page here was fetched from the live site and is committed verbatim.
//! Nothing in this file touches the network — a test that reaches the network
//! fails on a plane, and worse, passes or fails depending on what the site
//! served that afternoon.
//!
//! Provenance of each fixture:
//!
//! | File | Source |
//! |---|---|
//! | `ao3/work.html` | `archiveofourown.org/works/92356871` (redirects to chapter 1) |
//! | `ao3/work-full.html` | `archiveofourown.org/works/92356871?view_full_work=true` |
//! | `ao3/work-ongoing.html` | `archiveofourown.org/works/91806026` |
//! | `ao3/not-found.html` | `archiveofourown.org/works/99999999999999` (404) |
//!
//! The fixture is the contract. When the site changes its markup these tests
//! fail, which is the entire point: the failure is a signal that the parser
//! needs a person, not a silently empty chapter list.

use lorehaven_scrapers::sites::ao3::ArchiveSoftware;
use lorehaven_scrapers::{SourceAdapter, SourceError, WorkStatus};
use url::Url;

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"))
}

fn adapter() -> ArchiveSoftware {
    ArchiveSoftware::new()
}

fn url(raw: &str) -> Url {
    Url::parse(raw).unwrap()
}

#[test]
fn a_work_page_yields_its_metadata() {
    let work = adapter()
        .preview_from_html(
            &fixture("ao3/work.html"),
            &url("https://archiveofourown.org/works/92356871"),
        )
        .expect("the recorded work page parses");

    assert_eq!(work.source_key.as_str(), "ao3");
    assert_eq!(work.source_work_key, "92356871");
    assert_eq!(work.title, "Vampiric Concerto");
    assert_eq!(work.author_text, "ashenthea");
    assert_eq!(
        work.author_url.as_deref(),
        Some("https://archiveofourown.org/users/ashenthea/pseuds/ashenthea")
    );
    assert_eq!(work.status, WorkStatus::Complete);
    assert_eq!(work.word_count, Some(2116));
    assert_eq!(work.language.as_deref(), Some("English"));
    // The summary is prose, not markup: the blockquote's inner HTML is stripped.
    assert!(
        work.summary.starts_with("Dracula, pining after the love"),
        "summary was {:?}",
        work.summary
    );
    assert!(
        !work.summary.contains('<'),
        "summary kept markup: {:?}",
        work.summary
    );

    // The canonical URL is rebuilt from the host and the work id, not taken from
    // whatever the request happened to be.
    assert_eq!(
        work.source_url,
        "https://archiveofourown.org/works/92356871"
    );
}

#[test]
fn a_work_page_yields_its_chapter_list_with_the_sources_own_keys() {
    let work = adapter()
        .preview_from_html(
            &fixture("ao3/work.html"),
            &url("https://archiveofourown.org/works/92356871"),
        )
        .expect("parses");

    assert_eq!(work.chapter_count(), 3);
    let ordinals: Vec<u32> = work.chapters.iter().map(|c| c.ordinal).collect();
    assert_eq!(ordinals, vec![1, 2, 3]);
    // The ids come from the chapter index's option values, so a re-import keys
    // the same chapter the same way.
    let keys: Vec<&str> = work
        .chapters
        .iter()
        .map(|c| c.source_chapter_key.as_str())
        .collect();
    assert_eq!(keys, vec!["246151286", "246151581", "246152346"]);
    // Titles are reduced from `1. Title`, not left as the index renders them.
    assert_eq!(work.chapters[0].title, "2 Minutes to Midnight");
    assert_eq!(work.chapters[1].title, "Stratego");
    assert_eq!(work.chapters[2].title, "Nocturne in the Moonlight");
}

#[test]
fn a_work_page_yields_its_rating_warnings_and_tags() {
    let work = adapter()
        .preview_from_html(
            &fixture("ao3/work.html"),
            &url("https://archiveofourown.org/works/92356871"),
        )
        .expect("parses");

    assert_eq!(work.rating_text.as_deref(), Some("Teen And Up Audiences"));
    assert_eq!(
        work.warning_texts,
        vec!["Graphic Depictions Of Violence", "Major Character Death"]
    );
    // Fandoms, relationships, characters and freeforms all land in one tag list
    // until Milestone 9 gives them types.
    for expected in [
        "X-Men (Comicverse)",
        "Emma Frost/Sage | Tessa Karisik",
        "Emma Frost",
        "Lesbian",
    ] {
        assert!(
            work.tags.iter().any(|tag| tag == expected),
            "missing tag {expected:?} in {:?}",
            work.tags
        );
    }
}

#[test]
fn a_whole_work_page_yields_every_chapter_body() {
    let adapter = adapter();
    let work = adapter
        .preview_from_html(
            &fixture("ao3/work.html"),
            &url("https://archiveofourown.org/works/92356871"),
        )
        .expect("parses");

    let chapters = adapter
        .chapters_from_html(&fixture("ao3/work-full.html"), &work)
        .expect("the recorded whole-work page parses");

    assert_eq!(chapters.len(), 3);
    let ordinals: Vec<u32> = chapters.iter().map(|c| c.ordinal).collect();
    assert_eq!(ordinals, vec![1, 2, 3]);

    // The chapter keys in the whole-work page come from each chapter's own link,
    // and they must be the same keys the work page's index reported — otherwise
    // a re-import would treat every chapter as new.
    let keys: Vec<&str> = chapters
        .iter()
        .map(|c| c.source_chapter_key.as_str())
        .collect();
    assert_eq!(keys, vec!["246151286", "246151581", "246152346"]);

    for chapter in &chapters {
        assert!(
            !chapter.content_html.is_empty(),
            "chapter {} has no body",
            chapter.ordinal
        );
        assert!(
            chapter.content_html.contains('<'),
            "chapter {} lost all its markup",
            chapter.ordinal
        );
    }
    // The first chapter's prose, as the site renders it.
    assert!(
        chapters[0].content_html.contains("shadowed annals"),
        "chapter 1 body was {:?}",
        &chapters[0].content_html[..200.min(chapters[0].content_html.len())]
    );
    assert_eq!(chapters[2].title, "Nocturne in the Moonlight");
}

#[test]
fn the_sanitised_body_carries_no_scripts_or_handlers() {
    let adapter = adapter();
    let work = adapter
        .preview_from_html(
            &fixture("ao3/work.html"),
            &url("https://archiveofourown.org/works/92356871"),
        )
        .expect("parses");
    let chapters = adapter
        .chapters_from_html(&fixture("ao3/work-full.html"), &work)
        .expect("parses");

    for chapter in &chapters {
        let body = &chapter.content_html;
        for forbidden in [
            "<script",
            "<style",
            "onclick=",
            "onerror=",
            "<iframe",
            "javascript:",
        ] {
            assert!(
                !body.contains(forbidden),
                "chapter {} kept {forbidden}",
                chapter.ordinal
            );
        }
        // The site's own landmark heading is chrome, not content.
        assert!(
            !body.contains("Chapter Text"),
            "chapter {} kept the landmark heading",
            chapter.ordinal
        );
    }
}

#[test]
fn an_ongoing_work_is_reported_as_ongoing() {
    let work = adapter()
        .preview_from_html(
            &fixture("ao3/work-ongoing.html"),
            &url("https://archiveofourown.org/works/91806026"),
        )
        .expect("parses");

    assert_eq!(work.title, "Specific Impulses");
    assert_eq!(work.author_text, "withMoxie");
    // `Updated:` rather than `Completed:`, and `10/30` chapters.
    assert_eq!(work.status, WorkStatus::Ongoing);
    assert_eq!(work.chapter_count(), 10);
    assert_eq!(work.source_work_key, "91806026");
}

#[test]
fn the_sites_own_not_found_page_is_not_a_work() {
    let error = adapter()
        .preview_from_html(
            &fixture("ao3/not-found.html"),
            &url("https://archiveofourown.org/works/99999999999999"),
        )
        .expect_err("a 404 page is not a work");
    assert_eq!(error, SourceError::NotFound);
}

#[test]
fn a_page_that_is_not_a_work_page_fails_loudly() {
    // The failure mode this guards against: the site changes, the selector stops
    // matching, and the import stores a work with no chapters — which looks
    // exactly like success.
    let error = adapter()
        .preview_from_html(
            "<html><body><h1>We have moved</h1></body></html>",
            &url("https://archiveofourown.org/works/1"),
        )
        .expect_err("a page with no work on it is a parse failure");
    assert!(
        matches!(error, SourceError::Parse(_)),
        "expected a parse error, got {error:?}"
    );

    let work = adapter()
        .preview_from_html(
            &fixture("ao3/work.html"),
            &url("https://archiveofourown.org/works/92356871"),
        )
        .expect("parses");
    let error = adapter()
        .chapters_from_html("<html><body><div>nothing here</div></body></html>", &work)
        .expect_err("a whole-work page with no chapters is a parse failure");
    assert!(
        matches!(error, SourceError::Parse(_)),
        "expected a parse error, got {error:?}"
    );
}

#[test]
fn a_url_that_is_not_a_work_is_refused() {
    let error = adapter()
        .preview_from_html(
            &fixture("ao3/work.html"),
            &url("https://archiveofourown.org/works/search"),
        )
        .expect_err("the search page is not a work");
    assert!(
        matches!(error, SourceError::Unsupported(_)),
        "expected unsupported, got {error:?}"
    );
}
