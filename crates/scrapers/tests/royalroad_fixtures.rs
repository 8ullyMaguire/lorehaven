//! Royal Road, driven through recorded pages.
//!
//! Every assertion here is against a page recorded from the live site on
//! 2026-09-10 and committed under `tests/fixtures/royalroad/`. Nothing in this
//! file reaches the network: a test that reaches the network is a test that
//! fails on a plane, and a parser written from memory of a site's markup is a
//! parser written from a guess.
//!
//! The fixtures and the adapter were checked against each other rather than one
//! being written to fit the other. Where the ported code was wrong about the
//! site, the assertion here is the corrected reading, and the reason is in the
//! adapter's module documentation.

use lorehaven_scrapers::sites::royalroad::RoyalRoad;
use lorehaven_scrapers::SourceAdapter;
use lorehaven_scrapers::SourceError;
use url::Url;

const WORK_URL: &str = "https://www.royalroad.com/fiction/21220/mother-of-learning";
const CHAPTER_1_URL: &str = "https://www.royalroad.com/fiction/21220/mother-of-learning/chapter/301778/1-good-morning-brother";

/// Read a recorded page.
fn fixture(name: &str) -> String {
    let path = format!("tests/fixtures/royalroad/{name}");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("the recorded fixture {path} must exist: {error}"))
}

fn url(raw: &str) -> Url {
    Url::parse(raw).expect("a fixture URL must parse")
}

/// The work, as read from the recorded work page.
fn recorded_work() -> lorehaven_scrapers::SourceWork {
    RoyalRoad::new()
        .preview_from_html(&fixture("work.html"), &url(WORK_URL))
        .expect("the recorded work page must parse")
}

#[test]
fn the_work_page_is_read_as_the_work_the_site_says_it_is() {
    let work = recorded_work();

    assert_eq!(work.source_work_key, "21220");
    assert_eq!(work.title, "Mother of Learning");
    assert_eq!(work.source_url, WORK_URL);
}

#[test]
fn the_author_is_read_with_the_profile_the_structured_block_carries() {
    let work = recorded_work();

    // The ported adapter left `author_url` empty and read the name out of the
    // markup; the structured block supplies both, so both are asserted.
    assert_eq!(work.author_text, "nobody103");
    assert_eq!(
        work.author_url.as_deref(),
        Some("https://www.royalroad.com/profile/100374")
    );
}

#[test]
fn the_summary_is_read_as_prose_and_not_as_markup() {
    let work = recorded_work();

    // The description in the structured block is HTML. A summary stored with
    // its `<p>` intact would be shown to a reader as literal angle brackets.
    assert!(
        work.summary
            .starts_with("Zorian is a teenage mage of humble birth"),
        "the summary should begin with the work's own opening line, got {:?}",
        work.summary.chars().take(80).collect::<String>()
    );
    assert!(
        !work.summary.contains('<') && !work.summary.contains("&lt;"),
        "the summary still carries markup: {:?}",
        work.summary.chars().take(200).collect::<String>()
    );
    assert!(work.summary.contains("time loop"));
}

#[test]
fn the_dates_are_the_sites_dates_and_not_the_moment_of_the_import() {
    let work = recorded_work();

    // The ported adapter set both of these to the current time, which made every
    // imported work claim it was written today.
    let published = work.published_at.expect("the work publishes a date");
    assert_eq!(
        (published.year(), published.month() as u8, published.day()),
        (2018, 10, 28)
    );

    let updated = work.updated_at.expect("the work publishes a revision date");
    assert_eq!(
        (updated.year(), updated.month() as u8, updated.day()),
        (2023, 7, 6)
    );
    assert!(
        updated > published,
        "the revision date should be after the publication date"
    );
}

#[test]
fn the_status_is_the_one_the_site_declares() {
    let work = recorded_work();

    // The page labels this work COMPLETED beside its title. The ported adapter
    // hardcoded "ongoing" for every work it read.
    assert_eq!(work.status, lorehaven_scrapers::WorkStatus::Complete);
}

#[test]
fn the_word_count_is_the_word_count_and_not_the_page_estimate() {
    let work = recorded_work();

    // The tooltip says both "2,932 pages" and "806,306 words". Taking the first
    // number in the sentence would store the page count as a word count.
    assert_eq!(work.word_count, Some(806_306));
}

#[test]
fn the_tags_are_carried_as_display_text() {
    let work = recorded_work();

    assert_eq!(
        work.tags,
        vec![
            "Time Loop".to_owned(),
            "Adventure".to_owned(),
            "Fantasy".to_owned(),
            "Mystery".to_owned(),
            "Magic".to_owned(),
        ]
    );
}

#[test]
fn the_language_is_the_one_the_structured_block_declares() {
    let work = recorded_work();

    assert_eq!(work.language.as_deref(), Some("en-US"));
}

#[test]
fn every_chapter_the_table_lists_is_read() {
    let work = recorded_work();

    // The recorded page's `#chapters` table states `data-chapters="109"`, and the
    // adapter refuses to return a list that disagrees with it — so this asserts
    // the fixture still carries that attribute, without which the guard in the
    // adapter would be checking nothing.
    assert!(
        fixture("work.html").contains(r#"data-chapters="109""#),
        "the fixture no longer states its chapter count, so the adapter's agreement check is vacuous"
    );

    assert_eq!(work.chapter_count(), 109);
}

#[test]
fn the_chapters_are_numbered_from_one_without_gaps() {
    let work = recorded_work();

    let ordinals: Vec<u32> = work.chapters.iter().map(|c| c.ordinal).collect();
    let expected: Vec<u32> = (1..=109).collect();
    assert_eq!(ordinals, expected);
}

#[test]
fn each_chapter_carries_the_sites_own_id() {
    let work = recorded_work();

    let first = &work.chapters[0];
    assert_eq!(first.source_chapter_key, "301778");
    assert_eq!(first.title, "1. Good Morning Brother");

    // The site's own numbering is punctuation-free on screen but is not in the
    // markup: this title carries a typographic apostrophe, and a chapter keyed
    // by an ASCII-normalised title would be a different chapter.
    let second = &work.chapters[1];
    assert_eq!(second.source_chapter_key, "301781");
    assert_eq!(second.title, "2. Life\u{2019}s Little Problems");

    let last = &work.chapters[108];
    assert_eq!(last.title, "New story is out - Zenith of Sorcery");
}

#[test]
fn no_two_chapters_share_a_key() {
    let work = recorded_work();

    let mut keys: Vec<&str> = work
        .chapters
        .iter()
        .map(|chapter| chapter.source_chapter_key.as_str())
        .collect();
    let total = keys.len();
    keys.sort_unstable();
    keys.dedup();
    // A duplicated key would make one chapter overwrite another on re-import.
    assert_eq!(keys.len(), total, "two chapters share a source key");
}

#[test]
fn every_chapter_has_a_key_and_a_title() {
    let work = recorded_work();

    for chapter in &work.chapters {
        assert!(
            !chapter.source_chapter_key.is_empty(),
            "chapter {} has no key",
            chapter.ordinal
        );
        assert!(
            chapter
                .source_chapter_key
                .chars()
                .all(|c| c.is_ascii_digit()),
            "chapter {} has a key that is not the site's numeric id: {:?}",
            chapter.ordinal,
            chapter.source_chapter_key
        );
        assert!(
            !chapter.title.is_empty(),
            "chapter {} has no title",
            chapter.ordinal
        );
    }
}

#[test]
fn a_chapter_page_is_read_as_its_own_chapter() {
    let work = recorded_work();
    let chapters = RoyalRoad::new()
        .chapters_from_html(&fixture("chapter-1.html"), &work)
        .expect("the recorded chapter page must parse");

    // The ordinal is not on a chapter page, so it is recovered by matching the
    // page's title against the work's list. Getting this wrong would file every
    // chapter of a work under one position.
    assert_eq!(chapters.len(), 1);
    let chapter = &chapters[0];
    assert_eq!(chapter.ordinal, 1);
    assert_eq!(chapter.source_chapter_key, "301778");
    assert_eq!(chapter.title, "1. Good Morning Brother");
}

#[test]
fn the_prose_is_read_and_the_sites_furniture_is_not() {
    let work = recorded_work();
    let chapters = RoyalRoad::new()
        .chapters_from_html(&fixture("chapter-1.html"), &work)
        .expect("the recorded chapter page must parse");
    let body = &chapters[0].content_html;

    // The opening line of the chapter, from the recorded page.
    assert!(
        body.contains("Zorian\u{2019}s eyes abruptly shot open"),
        "the chapter's prose is missing from the body"
    );

    // Three things the recorded page contains that are not the author's prose.
    // The author's note is a sibling of the body element rather than a child,
    // which is the distinction that keeps it out.
    assert!(
        !body.contains("author-note"),
        "the author's note leaked into the prose"
    );
    assert!(
        !body.contains("Kindle"),
        "the note's Amazon link leaked into the prose"
    );
    assert!(
        !body.contains("Patreon") && !body.contains("fa-paypal"),
        "the support buttons leaked into the prose"
    );
    assert_eq!(
        body.matches("chapter-content").count(),
        0,
        "the body still carries the site's own wrapper class"
    );
}

#[test]
fn a_chapter_body_carries_no_script() {
    let work = recorded_work();
    for name in ["chapter-1.html", "chapter-2.html"] {
        let chapters = RoyalRoad::new()
            .chapters_from_html(&fixture(name), &work)
            .expect("the recorded chapter page must parse");
        let body = &chapters[0].content_html;
        assert!(
            !body.contains("<script"),
            "{name} produced a body containing a script element"
        );
        assert!(
            !body.contains("javascript:"),
            "{name} produced a body containing a javascript: URL"
        );
        assert!(
            !body.contains("onclick"),
            "{name} produced a body containing an inline event handler"
        );
    }
}

#[test]
fn a_later_chapter_is_read_as_its_own_position() {
    let work = recorded_work();
    let chapters = RoyalRoad::new()
        .chapters_from_html(&fixture("chapter-2.html"), &work)
        .expect("the recorded chapter page must parse");

    let chapter = &chapters[0];
    assert_eq!(chapter.ordinal, 2);
    assert_eq!(chapter.source_chapter_key, "301781");
    assert_eq!(chapter.title, "2. Life\u{2019}s Little Problems");
    assert!(chapter.content_html.contains("academ"));
}

#[test]
fn the_sites_own_not_found_page_is_a_missing_work_and_not_an_empty_one() {
    let missing = url("https://www.royalroad.com/fiction/999999999/definitely-not-real");

    // The distinction matters: reported as an empty work, a dead URL imports as
    // a success with no chapters, and the reader is told the site had nothing.
    let error = RoyalRoad::new()
        .preview_from_html(&fixture("not-found.html"), &missing)
        .expect_err("the recorded error page must not parse as a work");
    assert!(
        matches!(error, SourceError::NotFound),
        "expected NotFound, got {error:?}"
    );
}

#[test]
fn the_not_found_page_carries_none_of_the_work_landmarks() {
    // Guards the check above from being satisfied by accident: if the recorded
    // error page ever gained a chapter table, `is_not_found` would stop firing
    // and the test above would be asserting a coincidence.
    let page = fixture("not-found.html");
    assert!(!page.contains(r#"id="chapters""#));
    assert!(!page.contains("chapter-row"));
    assert!(!page.contains("font-white"));
}

#[test]
fn a_chapter_url_previews_its_work() {
    // A reader who pastes a chapter link means the work it belongs to, not a
    // work whose title happens to be that chapter's.
    let work = RoyalRoad::new()
        .preview_from_html(&fixture("work.html"), &url(CHAPTER_1_URL))
        .expect("the recorded work page must parse");

    assert_eq!(work.source_work_key, "21220");
    assert_eq!(work.source_url, WORK_URL);
    assert_eq!(work.chapter_count(), 109);
}

#[test]
fn the_adapter_is_in_the_default_registry() {
    let registry = lorehaven_scrapers::sites::default_registry();

    let routed = registry
        .route(&url(WORK_URL))
        .expect("a Royal Road work URL should route to an adapter");
    assert_eq!(routed.key().as_str(), "royalroad");

    // A URL on the same host that is not a work must not route, or the importer
    // would try to import a search page.
    assert!(registry
        .route(&url("https://www.royalroad.com/fictions/search?title=x"))
        .is_err());

    assert!(registry
        .route(&url("https://www.example.com/fiction/1/x"))
        .is_err());
}
