//! Ficbook, driven through the pages recorded from the live site.
//!
//! Every assertion here is against something recorded on 2026-09-11 and
//! committed under `tests/fixtures/ficbook/` — see the `## ficbook` section of
//! `tests/fixtures/README.md` for where each page came from.
//!
//! The suite is offline. Nothing here reaches the network, so it runs in CI
//! without a solver and without the site's consent changing under it.

use lorehaven_scrapers::sites::ficbook::Ficbook;
use lorehaven_scrapers::{SourceAdapter, WorkStatus};
use time::macros::datetime;
use url::Url;

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ficbook")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} is not readable: {error}", path.display()))
}

fn adapter() -> Ficbook {
    Ficbook::new()
}

const WORK: &str = "01899919-f575-76ed-8476-cec2348b02bf";
const WORK_URL: &str = "https://ficbook.net/readfic/01899919-f575-76ed-8476-cec2348b02bf";
const FINISHED: &str = "01a05297-2f11-74e0-a894-8ed36d62a32a";

#[test]
fn a_work_page_is_read_for_everything_it_states() {
    let url = Url::parse(WORK_URL).unwrap();
    let work = adapter()
        .preview_from_html(&fixture("work.html"), &url)
        .expect("the recorded work page parses");

    assert_eq!(work.source_work_key, WORK);
    assert_eq!(work.source_url, WORK_URL);
    assert_eq!(work.title, "Проклятая река");
    assert_eq!(work.author_text, "FieryQueen");
    assert_eq!(
        work.author_url.as_deref(),
        Some("https://ficbook.net/authors/1878391")
    );
    assert!(
        work.summary.starts_with("Поддавшись желанию"),
        "{}",
        work.summary
    );
    assert_eq!(work.status, WorkStatus::Ongoing);
    assert_eq!(work.rating_text.as_deref(), Some("NC-17"));

    // The size line, read by unit because the two recorded shapes differ and
    // because its thousands separator is a non-breaking space.
    assert_eq!(work.word_count, Some(77_507));
    assert_eq!(work.chapters.len(), 24);
    assert_eq!(work.tags.len(), 35);
    assert_eq!(work.tags[0], "AU");

    // The work page carries no prose: `#content` is on a part page only.
    assert!(!fixture("work.html").contains("id=\"content\""));
}

#[test]
fn the_part_list_is_in_the_pages_order_and_carries_the_sites_own_ids() {
    let url = Url::parse(WORK_URL).unwrap();
    let work = adapter()
        .preview_from_html(&fixture("work.html"), &url)
        .unwrap();

    // Ordinals are one-based and follow the page, which is publication order.
    assert_eq!(work.chapters[0].ordinal, 1);
    assert_eq!(work.chapters[23].ordinal, 24);
    assert_eq!(work.chapters[0].source_chapter_key, "35183469");
    assert_eq!(work.chapters[0].title, "Глава 1. Часть I. Незваный гость");

    // The "next chapter" navigation links on the same page are the same shape
    // as the list's, so a selector that collected every `a.part-link` would put
    // a duplicate in the list. The keys are therefore unique.
    let mut keys: Vec<&str> = work
        .chapters
        .iter()
        .map(|chapter| chapter.source_chapter_key.as_str())
        .collect();
    let count = keys.len();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), count, "a part appears twice in the list");
}

#[test]
fn the_works_dates_are_its_first_and_last_parts_read_in_the_sites_zone() {
    let url = Url::parse(WORK_URL).unwrap();
    let work = adapter()
        .preview_from_html(&fixture("work.html"), &url)
        .unwrap();

    // The page writes `3 августа 2023 г., 12:37` and `11 сентября 2026 г.,
    // 12:36` — Moscow time, so 09:37 and 09:36 UTC. Read as UTC they would be
    // three hours out.
    assert_eq!(work.published_at, Some(datetime!(2023-08-03 09:37:00 UTC)));
    assert_eq!(work.updated_at, Some(datetime!(2026-09-11 09:36:00 UTC)));
}

#[test]
fn a_finished_work_says_so_and_its_size_line_has_the_other_shape() {
    let url = Url::parse(&format!("https://ficbook.net/readfic/{FINISHED}")).unwrap();
    let work = adapter()
        .preview_from_html(&fixture("work-finished.html"), &url)
        .unwrap();

    assert_eq!(work.title, "Шелест лилий");
    assert_eq!(work.status, WorkStatus::Complete);
    assert_eq!(work.rating_text.as_deref(), Some("R"));
    // `24 страницы, 8 122 слова, 4 части` — no plan prefix, and the word form is
    // `слова` rather than `слов`.
    assert_eq!(work.word_count, Some(8_122));
    assert_eq!(work.chapters.len(), 4);
    assert_eq!(work.tags.len(), 10);
    assert_eq!(work.summary, "А, может, стоило поговорить?");
}

#[test]
fn the_work_pages_language_field_is_not_read_as_the_works_language() {
    // `itemprop="inLanguage"` is `ru-Latn` on every work recorded — the site's
    // own interface locale, written into a machine-readable field. Reported as
    // the work's language it would claim every ficbook work is
    // Russian-in-Latin-script, including the many written in Cyrillic.
    assert!(fixture("work.html").contains("inLanguage\" content=\"ru-Latn\""));
    assert!(
        fixture("work-finished.html").contains("inLanguage\" content=\"ru-Latn\""),
        "the field is the same on both recorded works, which is why it is not the work's language"
    );

    let url = Url::parse(WORK_URL).unwrap();
    let work = adapter()
        .preview_from_html(&fixture("work.html"), &url)
        .unwrap();
    assert_eq!(work.language, None);
}

#[test]
fn a_part_page_states_its_own_address_and_is_read_for_its_prose() {
    let url = Url::parse(WORK_URL).unwrap();
    let work = adapter()
        .preview_from_html(&fixture("work.html"), &url)
        .unwrap();

    let chapter = adapter()
        .parse_chapter(&fixture("chapter-1.html"), &work, "35183469")
        .expect("the recorded part page parses");

    assert_eq!(chapter.source_chapter_key, "35183469");
    assert_eq!(chapter.ordinal, 1);
    assert_eq!(chapter.title, "Глава 1. Часть I. Незваный гость");
    assert!(!chapter.content_html.is_empty());
    // Prose, not navigation: the page carries the whole site chrome around it.
    assert!(
        !chapter.content_html.contains("list-of-fanfic-parts"),
        "the chapter body must not carry the work's part list"
    );
}

#[test]
fn a_part_the_page_did_not_answer_with_is_refused() {
    // The page states its own address. A page that answers with another chapter
    // would have its prose stored under the wrong ordinal, and a reader's
    // progress and notes are mapped onto the ordinal across a re-import.
    let url = Url::parse(WORK_URL).unwrap();
    let work = adapter()
        .preview_from_html(&fixture("work.html"), &url)
        .unwrap();

    let error = adapter()
        .parse_chapter(&fixture("chapter-1.html"), &work, "35203286")
        .expect_err("a page for another part is not part 35203286");
    let message = format!("{error}");
    assert!(message.contains("35203286"), "{message}");
    assert!(message.contains("35183469"), "{message}");
}

#[test]
fn a_work_page_is_refused_by_the_chapter_parser_rather_than_imported_blank() {
    // A work page has no `#content`. Returning an empty chapter for it would
    // store a work whose every chapter is blank and report success.
    let url = Url::parse(WORK_URL).unwrap();
    let work = adapter()
        .preview_from_html(&fixture("work.html"), &url)
        .unwrap();

    let error = adapter()
        .parse_chapter(&fixture("work.html"), &work, "35183469")
        .expect_err("a work page is not a part page");
    assert!(format!("{error}").contains("#content"), "{error}");
}

#[test]
fn the_footnotes_are_recovered_from_outside_the_prose() {
    // Nine placeholders in the prose, nine notes in a script at the bottom of
    // the page. An adapter that read `#content` alone would drop every one of
    // them, silently, with a chapter that looks complete.
    let body = fixture("chapter-1.html");
    let placeholders = body.matches("<span class=\"footnote\"").count();
    assert!(
        placeholders >= 9,
        "the recorded part has {placeholders} references"
    );

    let url = Url::parse(WORK_URL).unwrap();
    let work = adapter()
        .preview_from_html(&fixture("work.html"), &url)
        .unwrap();
    let chapter = adapter().parse_chapter(&body, &work, "35183469").unwrap();

    assert!(
        chapter.content_html.contains("<sup>[1]</sup>"),
        "the first reference is numbered where the reader meets it"
    );
    assert!(
        chapter.content_html.contains("polnalyubvi"),
        "the note's text is carried, not dropped"
    );
    assert!(
        !chapter.content_html.contains("class=\"footnote\""),
        "the empty placeholders are replaced by their notes"
    );
    // The escapes in the script are decoded: `\u0441` is `с`, not five
    // characters of text.
    assert!(
        !chapter.content_html.contains("u0441"),
        "{}",
        chapter.content_html
    );
}

#[test]
fn the_site_refuses_a_work_that_does_not_exist_with_a_real_404() {
    // Unlike FanFiction.net, which answers a missing story with `200` and a
    // notice, this site answers `404`. The recorded page is kept as evidence
    // that the adapter does not need a structural check for it.
    let body = fixture("not-found.html");
    assert!(body.contains("404"));
    assert!(!body.contains("id=\"content\""));
    assert!(!body.contains("ds-label"));
}

#[test]
fn every_address_this_adapter_asks_for_is_one_its_robots_txt_allows() {
    // The source's own rule is `Disallow: /*?*` — every query string — and it
    // writes decorative ones on its own links. The adapter strips the query and
    // the fragment rather than asking for an address its own front door would
    // be refused. What it would actually ask for is checked here by
    // construction: nothing in `asked` carries either.
    assert!(
        fixture("robots.txt").contains("Disallow: /*?*"),
        "the rule this test exists for is no longer in the recorded file"
    );

    let url = Url::parse(WORK_URL).unwrap();
    let work = adapter()
        .preview_from_html(&fixture("work.html"), &url)
        .unwrap();

    let mut asked = vec![work.source_url.clone()];
    for chapter in &work.chapters {
        asked.push(adapter().part_url(&work.source_work_key, &chapter.source_chapter_key));
    }

    // A decorated address — which is what the site's own work links are — is
    // claimed, and normalises to one of the addresses above.
    let decorated = Url::parse(&format!("{WORK_URL}?from_promo=1#part_content")).unwrap();
    assert!(adapter().can_handle(&decorated));
    assert_eq!(decorated.query(), Some("from_promo=1"));
    assert_eq!(decorated.fragment(), Some("part_content"));

    for raw in asked {
        assert!(
            !raw.contains('?'),
            "{raw} carries a query string, which this source disallows"
        );
        assert!(
            !raw.contains('#'),
            "{raw} carries a fragment, which the server never sees"
        );
        let url = Url::parse(&raw).unwrap();
        assert!(adapter().can_handle(&url), "{raw} is not claimed");
    }
}
