//! Scribble Hub, driven through the API responses and pages recorded live.
//!
//! Every assertion here is against something recorded on 2026-09-11 and
//! committed under `tests/fixtures/scribblehub/` — see the `## scribblehub`
//! section of `tests/fixtures/README.md` for where each one came from.
//!
//! The suite is offline. The site is behind Cloudflare, but the documents it
//! takes to import a work were recorded once and committed, so CI needs no
//! solver and no network.

use lorehaven_scrapers::sites::scribblehub::ScribbleHub;
use lorehaven_scrapers::{SourceAdapter, SourceError, WorkStatus};
use time::macros::datetime;

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/scribblehub")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} is not readable: {error}", path.display()))
}

fn adapter() -> ScribbleHub {
    ScribbleHub::new()
}

const STORY: &str = "2357420";
const STORY_URL: &str = "https://www.scribblehub.com/series/2357420/worlds-cutest-alchemist/";

#[test]
fn the_story_object_is_read_for_everything_the_api_states() {
    let story = adapter()
        .parse_story(&fixture("story.json"), STORY)
        .expect("the recorded story object parses");

    assert_eq!(story.id, 2_357_420);
    assert_eq!(story.title, "World\u{2019}s Cutest Alchemist");
    assert_eq!(story.slug, "2357420-worlds-cutest-alchemist");
    assert_eq!(story.author.display_name, "drava");
    assert_eq!(story.author.username, "drava");
    assert_eq!(story.status, "ongoing");
    // The site's own count for the whole work, which is what the adapter uses to
    // know how many chapters to collect and to check the list against.
    assert_eq!(story.chapter_count, 113);
    assert_eq!(story.word_count, 241_715);
    assert!(!story.is_mature);
    assert!(story.is_accessible);
    assert!(story
        .description
        .starts_with("Awakening is the dream of many"));
}

#[test]
fn a_work_is_built_from_its_story_object_and_its_chapters() {
    let story = adapter()
        .parse_story(&fixture("story.json"), STORY)
        .unwrap();
    let pages = [
        adapter()
            .parse_chapter_page(&fixture("chapters.json"))
            .unwrap(),
        adapter()
            .parse_chapter_page(&fixture("chapters-page-2.json"))
            .unwrap(),
        adapter()
            .parse_chapter_page(&fixture("chapters-page-3.json"))
            .unwrap(),
    ];
    let chapters = adapter()
        .assemble_chapters(&pages, STORY, story.chapter_count)
        .expect("the recorded pages add up to the stated count");

    let work = story.into_work(
        lorehaven_scrapers::SourceKey::new("scribblehub"),
        STORY_URL.to_owned(),
        chapters,
    );

    assert_eq!(work.source_work_key, STORY);
    assert_eq!(work.source_url, STORY_URL);
    assert_eq!(work.title, "World\u{2019}s Cutest Alchemist");
    assert_eq!(work.author_text, "drava");
    assert_eq!(
        work.author_url.as_deref(),
        Some("https://www.scribblehub.com/profile/108709/drava/")
    );
    assert_eq!(work.status, WorkStatus::Ongoing);
    assert_eq!(work.word_count, Some(241_715));
    // The site publishes a boolean rather than a scale; `Mature` is its own word
    // for it, and this work is not flagged.
    assert_eq!(work.rating_text, None);
    assert_eq!(work.updated_at, Some(datetime!(2026-09-10 23:30:13 UTC)));
    // The story object carries no created date; the first chapter's is the
    // publication, and the adapter does not claim otherwise on the work itself.
    assert_eq!(work.published_at, None);

    // Genres and tags are one list in the domain, and the site publishes both.
    assert_eq!(work.tags.len(), 12);
    assert_eq!(work.tags[0], "Action");
    assert_eq!(work.tags[4], "Slice of Life");
    assert_eq!(work.tags[5], "Carefree Protagonist");

    // No language is reported: the API has no language field, and the prose
    // would be a guess.
    assert_eq!(work.language, None);
}

#[test]
fn the_pages_add_up_to_the_count_and_the_last_page_is_the_short_one() {
    // The API returns fifty to a request and accepts `page=N`. The recorded
    // work is 113 chapters, so the pages are 50 + 50 + 13 and a fourth is
    // empty — recorded to pin the end condition down.
    let first = adapter()
        .parse_chapter_page(&fixture("chapters.json"))
        .unwrap();
    let second = adapter()
        .parse_chapter_page(&fixture("chapters-page-2.json"))
        .unwrap();
    let third = adapter()
        .parse_chapter_page(&fixture("chapters-page-3.json"))
        .unwrap();
    let fourth = adapter()
        .parse_chapter_page(&fixture("chapters-page-4.json"))
        .unwrap();

    assert_eq!(first.len(), 50);
    assert_eq!(second.len(), 50);
    assert_eq!(third.len(), 13);
    assert!(fourth.is_empty(), "the page past the end is empty");

    let chapters = adapter()
        .assemble_chapters(&[first, second, third], STORY, 113)
        .expect("113 chapters");
    assert_eq!(chapters.len(), 113);
    assert_eq!(chapters[0].ordinal, 1);
    assert_eq!(chapters[112].ordinal, 113);
    assert_eq!(chapters[0].source_chapter_key, "2357479");
    assert_eq!(chapters[0].title, "about the novel :)");
}

#[test]
fn the_sites_own_chapter_numbers_have_gaps_and_are_not_the_ordinal() {
    // The recorded work runs `1, 3, 4, 5 …` because chapter 2 was deleted, and
    // its last chapter is numbered 115 while being the 113th. A reader's
    // progress and notes are mapped onto the ordinal, so a gap would move a
    // chapter under somebody's bookmark.
    let first = adapter()
        .parse_chapter_page(&fixture("chapters.json"))
        .unwrap();
    assert_eq!(
        first.iter().take(4).map(|c| c.number).collect::<Vec<_>>(),
        vec![1, 3, 4, 5]
    );

    let chapters = adapter()
        .assemble_chapters(
            &[
                first,
                adapter()
                    .parse_chapter_page(&fixture("chapters-page-2.json"))
                    .unwrap(),
                adapter()
                    .parse_chapter_page(&fixture("chapters-page-3.json"))
                    .unwrap(),
            ],
            STORY,
            113,
        )
        .unwrap();

    // Ordinals are dense, 1..=113, whatever the site's numbering does.
    assert!(chapters
        .iter()
        .enumerate()
        .all(|(index, entry)| { entry.ordinal == u32::try_from(index).unwrap() + 1 }));
    // The site's number for the last chapter is 115, not 113.
    assert_eq!(chapters[112].ordinal, 113);
}

#[test]
fn the_chapters_word_counts_add_up_to_the_works_own_total() {
    // A cross-check that the recorded pages are the *whole* list rather than the
    // first page of it: the site states a total for the work, and the chapters
    // it lists sum to exactly that. If the adapter were reading a capped list,
    // these two numbers would disagree — which is the failure this adapter is
    // built to refuse rather than to paper over.
    let story = adapter()
        .parse_story(&fixture("story.json"), STORY)
        .unwrap();
    let mut listed = 0i64;
    for page in [
        "chapters.json",
        "chapters-page-2.json",
        "chapters-page-3.json",
    ] {
        for chapter in adapter().parse_chapter_page(&fixture(page)).unwrap() {
            listed += i64::from(chapter.word_count);
        }
    }

    assert_eq!(story.chapter_count as usize, 113);
    assert_eq!(listed, story.word_count);
    assert_eq!(listed, 241_715);
}

#[test]
fn a_missing_story_is_not_found_and_an_unknown_fault_is_loud() {
    // The API answers a missing story with `404` and a `fa_story_not_found`
    // envelope whose `data` is a *status object*, not a payload. Read as a
    // payload it is a deserialisation failure about an integer where a string
    // was expected — a parse error for a work that simply does not exist.
    let error = adapter()
        .parse_story(&fixture("missing-story.json"), "999999999")
        .expect_err("a missing story is not a work");
    assert!(matches!(error, SourceError::NotFound), "{error}");

    // A fault code this build has never seen is not evidence that a work is
    // missing, so it is reported as the parse failure it is.
    let error = adapter()
        .parse_story(
            r#"{"code":"fa_rate_limited","message":"Slow down.","data":{"status":429}}"#,
            STORY,
        )
        .expect_err("an unknown fault is not a missing work");
    assert!(matches!(error, SourceError::Parse(_)), "{error}");
    assert!(format!("{error}").contains("fa_rate_limited"), "{error}");
}

#[test]
fn a_chapter_page_is_read_for_its_prose_and_states_which_chapter_it_is() {
    let story = adapter()
        .parse_story(&fixture("story.json"), STORY)
        .unwrap();
    let chapters = adapter()
        .assemble_chapters(
            &[
                adapter()
                    .parse_chapter_page(&fixture("chapters.json"))
                    .unwrap(),
                adapter()
                    .parse_chapter_page(&fixture("chapters-page-2.json"))
                    .unwrap(),
                adapter()
                    .parse_chapter_page(&fixture("chapters-page-3.json"))
                    .unwrap(),
            ],
            STORY,
            story.chapter_count,
        )
        .unwrap();
    let work = story.into_work(
        lorehaven_scrapers::SourceKey::new("scribblehub"),
        STORY_URL.to_owned(),
        chapters,
    );

    let chapter = adapter()
        .parse_chapter(&fixture("chapter-1.html"), &work, "2357479")
        .expect("the recorded chapter page parses");

    assert_eq!(chapter.source_chapter_key, "2357479");
    assert_eq!(chapter.ordinal, 1);
    assert_eq!(chapter.title, "about the novel :)");
    assert!(
        chapter.content_html.starts_with("<p>Hello!"),
        "{}",
        &chapter.content_html[..80.min(chapter.content_html.len())]
    );
    // The site chrome around the prose is not carried.
    assert!(!chapter.content_html.contains("comments"));
    assert!(!chapter.content_html.contains("btn-next"));

    // The recorded series page is a *work* page: it has no `#chp_raw`, so
    // feeding it to the chapter parser is refused rather than producing an
    // empty chapter.
    let error = adapter()
        .parse_chapter(&fixture("work.html"), &work, "2357479")
        .expect_err("the series page is not a chapter page");
    assert!(format!("{error}").contains("#chp_raw"), "{error}");
}

#[test]
fn a_chapter_page_that_states_another_chapter_is_refused() {
    // The page states its own address. Stored under the wrong ordinal it would
    // land under somebody's bookmark for a different chapter.
    let story = adapter()
        .parse_story(&fixture("story.json"), STORY)
        .unwrap();
    let chapters = adapter()
        .assemble_chapters(
            &[
                adapter()
                    .parse_chapter_page(&fixture("chapters.json"))
                    .unwrap(),
                adapter()
                    .parse_chapter_page(&fixture("chapters-page-2.json"))
                    .unwrap(),
                adapter()
                    .parse_chapter_page(&fixture("chapters-page-3.json"))
                    .unwrap(),
            ],
            STORY,
            story.chapter_count,
        )
        .unwrap();
    let work = story.into_work(
        lorehaven_scrapers::SourceKey::new("scribblehub"),
        STORY_URL.to_owned(),
        chapters,
    );

    let error = adapter()
        .parse_chapter(&fixture("chapter-1.html"), &work, "2357468")
        .expect_err("the page is chapter 2357479, not 2357468");
    let message = format!("{error}");
    assert!(message.contains("2357479"), "{message}");
    assert!(message.contains("2357468"), "{message}");
}

#[test]
fn the_series_page_shows_fifteen_of_a_hundred_and_thirteen() {
    // Recorded as the evidence for why this adapter reads the API instead of the
    // page: the reader's page lists fifteen chapters in descending order beside
    // a header stating the work's real size. An adapter built on it would import
    // fifteen of a hundred and thirteen and report a hundred and thirteen.
    let page = fixture("work.html");
    assert!(
        page.contains("class=\"cnt_toc\">113<"),
        "the header states 113"
    );
    let in_page = page.matches("class=\"toc_w\"").count();
    assert_eq!(in_page, 15, "the page lists fifteen chapters");

    // And the order is descending: the first listed is the newest.
    let first = page
        .find("class=\"toc_a\"")
        .map(|at| &page[at..at + 200.min(page.len() - at)])
        .expect("a chapter anchor");
    assert!(first.contains("Chapter 112"), "{first}");
}

#[test]
fn the_addresses_this_adapter_asks_for_are_ones_the_robots_file_allows() {
    // Scribble Hub's `robots.txt` is itself behind the challenge — the fetcher
    // reads it plainly, records that the rules are unknown, and uses the default
    // pace rather than treating the site as unrestricted. The recorded text is
    // what the solver returned: it disallows only `/wp-admin/`, and explicitly
    // allows the one `admin-ajax` path inside it.
    let rules = fixture("robots.txt");
    assert!(rules.contains("Disallow: /wp-admin/"));
    assert!(rules.contains("Allow: /wp-admin/admin-ajax.php"));

    let adapter = adapter();

    // Every address the adapter builds is outside that prefix.
    for url in [
        adapter.api_story(STORY),
        adapter.api_chapters(STORY, 1),
        adapter.chapter_url(STORY, "2357420-worlds-cutest-alchemist", "2357479"),
        adapter.story_url(STORY),
    ] {
        let parsed = url::Url::parse(&url).expect("a built address is a URL");
        assert_eq!(parsed.host_str(), Some("www.scribblehub.com"), "{url}");
        assert!(
            !parsed.path().starts_with("/wp-admin/"),
            "{url} is inside the prefix the site disallows"
        );
    }

    // The addresses a *reader* hands the importer are the series and reading
    // ones, and those are what the adapter claims. The API addresses are its own
    // way of reading what those pages carry, and are not reader-facing
    // addresses, so `can_handle` declining them is correct rather than a gap.
    for raw in [
        STORY_URL,
        "https://www.scribblehub.com/read/2357420-worlds-cutest-alchemist/chapter/2357479/",
    ] {
        assert!(
            adapter.can_handle(&url::Url::parse(raw).unwrap()),
            "{raw} is an address a reader can hand the importer"
        );
    }
    assert!(!adapter.can_handle(&url::Url::parse(&adapter.api_story(STORY)).unwrap()));
}

#[test]
fn a_completed_work_says_so() {
    let story = adapter()
        .parse_story(&fixture("story-completed.json"), "2102556")
        .expect("the recorded completed work parses");
    assert_eq!(story.title, "Bloodkin");
    assert_eq!(story.status, "completed");
    assert_eq!(story.chapter_count, 119);
    assert_eq!(story.word_count, 235_489);

    let work = story.into_work(
        lorehaven_scrapers::SourceKey::new("scribblehub"),
        adapter().story_url("2102556"),
        Vec::new(),
    );
    assert_eq!(work.status, WorkStatus::Complete);
    assert_eq!(work.updated_at, Some(datetime!(2026-09-08 12:21:35 UTC)));
}
