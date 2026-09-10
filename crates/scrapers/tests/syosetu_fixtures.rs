//! Syosetu, driven through recorded pages.
//!
//! Every assertion is against a page recorded from the live site on 2026-09-10
//! and committed under `tests/fixtures/syosetu/`. Nothing here reaches the
//! network.
//!
//! Three works are recorded, and the reason for each is different:
//!
//! | Work | Episodes | Why |
//! |---|---|---|
//! | `n2267be` (Ｒｅ：ゼロから始める異世界生活) | 795 across 8 list pages | the paginated case, which is where a partial read is possible |
//! | `n9525ii` (外典) | 6 on one list page | a serialized work short enough to assemble completely from fixtures |
//! | `n2611bq` (風呂場でクトゥルフなう) | 1, a 短編 | the one-shot shape, which has no episode list at all |
//!
//! Titles are asserted as `collapse_whitespace` leaves them: the site separates a
//! title's halves with an ideographic space (`U+3000`), and the adapter collapses
//! runs of whitespace, so the expectation carries a plain space. That is a real
//! normalisation and not an accident of copying — recorded because the difference
//! is invisible in a diff.
//!
//! The 795-episode work is the important one. Its episode list serves 100 rows and
//! a pager, and the number 795 appears only on its info page — so an adapter that
//! reads the work page and stops imports a seventh of it as though it were the
//! whole. The test that matters most here is the one asserting that this adapter
//! *refuses* rather than returning 200 chapters.

use lorehaven_scrapers::sites::syosetu::Syosetu;
use lorehaven_scrapers::SourceAdapter;
use lorehaven_scrapers::SourceError;
use lorehaven_scrapers::WorkStatus;
use url::Url;

const LONG_NCODE: &str = "n2267be";
const SHORT_NCODE: &str = "n9525ii";
const TANPEN_NCODE: &str = "n2611bq";

const LONG_URL: &str = "https://ncode.syosetu.com/n2267be/";
const SHORT_URL: &str = "https://ncode.syosetu.com/n9525ii/";
const TANPEN_URL: &str = "https://ncode.syosetu.com/n2611bq/";

/// Read a recorded page.
fn fixture(name: &str) -> String {
    let path = format!("tests/fixtures/syosetu/{name}");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("the recorded fixture {path} must exist: {error}"))
}

fn url(raw: &str) -> Url {
    Url::parse(raw).expect("a fixture URL must parse")
}

fn adapter() -> Syosetu {
    Syosetu::new()
}

/// The 6-episode work, assembled from all the pages that describe it.
fn short_work() -> lorehaven_scrapers::SourceWork {
    adapter()
        .assemble(
            SHORT_NCODE,
            &fixture("short-work.html"),
            &fixture("short-info.html"),
            &[fixture("short-work.html")],
        )
        .expect("the 6-episode work must assemble")
}

/// The one-shot, assembled.
fn one_shot() -> lorehaven_scrapers::SourceWork {
    adapter()
        .assemble(
            TANPEN_NCODE,
            &fixture("tanpen-work.html"),
            &fixture("tanpen-info.html"),
            &[],
        )
        .expect("the one-shot must assemble")
}

// ---------------------------------------------------------------------------
// The info page
// ---------------------------------------------------------------------------

#[test]
fn the_info_page_is_read_as_the_work_the_site_says_it_is() {
    let work = adapter()
        .preview_from_html(
            &fixture("long-info.html"),
            &url("https://ncode.syosetu.com/novelview/infotop/ncode/n2267be/"),
        )
        .expect("the recorded info page must parse");

    // Japanese throughout, and asserted as such: a broken encoding would produce
    // mojibake rather than a failure, and that is the point of recording a
    // non-Latin source early.
    assert_eq!(work.title, "Ｒｅ：ゼロから始める異世界生活");
    assert_eq!(work.author_text, "鼠色猫/長月達平");
    assert_eq!(work.source_work_key, LONG_NCODE);
    assert_eq!(work.source_url, LONG_URL);
}

#[test]
fn the_author_url_is_carried_from_the_author_entry() {
    let work = adapter()
        .preview_from_html(
            &fixture("long-info.html"),
            &url("https://ncode.syosetu.com/novelview/infotop/ncode/n2267be/"),
        )
        .expect("the recorded info page must parse");

    assert_eq!(
        work.author_url.as_deref(),
        Some("https://mypage.syosetu.com/235132/")
    );
}

#[test]
fn the_language_is_the_one_the_page_declares() {
    let work = adapter()
        .preview_from_html(
            &fixture("long-info.html"),
            &url("https://ncode.syosetu.com/novelview/infotop/ncode/n2267be/"),
        )
        .expect("the recorded info page must parse");

    // From `html lang="ja"` — the page publishing a language rather than this
    // adapter assuming one because the text looks Japanese.
    assert_eq!(work.language.as_deref(), Some("ja"));
}

#[test]
fn the_dates_are_japanese_local_time_and_not_the_moment_of_the_import() {
    let work = adapter()
        .preview_from_html(
            &fixture("long-info.html"),
            &url("https://ncode.syosetu.com/novelview/infotop/ncode/n2267be/"),
        )
        .expect("the recorded info page must parse");

    let published = work.published_at.expect("掲載日 is on the page");
    assert_eq!(
        (published.year(), published.month() as u8, published.day()),
        (2012, 4, 20)
    );
    assert_eq!(published.hour(), 21);
    // The offset is the assertion that matters: read as UTC, every date on this
    // site would be nine hours out.
    assert_eq!(published.offset().whole_hours(), 9);

    let updated = work.updated_at.expect("最新掲載日 is on the page");
    assert_eq!(
        (updated.year(), updated.month() as u8, updated.day()),
        (2026, 8, 24)
    );
    assert!(updated > published);
}

#[test]
fn the_word_count_is_the_sites_number_in_characters() {
    let work = adapter()
        .preview_from_html(
            &fixture("long-info.html"),
            &url("https://ncode.syosetu.com/novelview/infotop/ncode/n2267be/"),
        )
        .expect("the recorded info page must parse");

    // 9,666,529文字. The commas are the site's, and the number is not the page
    // count the way Royal Road's tooltip would tempt a parser to read.
    assert_eq!(work.word_count, Some(9_666_529));
}

#[test]
fn the_status_is_the_label_the_site_shows() {
    let work = adapter()
        .preview_from_html(
            &fixture("long-info.html"),
            &url("https://ncode.syosetu.com/novelview/infotop/ncode/n2267be/"),
        )
        .expect("the recorded info page must parse");

    // 連載中 beside the title.
    assert_eq!(work.status, WorkStatus::Ongoing);
}

#[test]
fn the_tags_are_carried_and_split_the_way_the_page_separates_them() {
    let work = adapter()
        .preview_from_html(
            &fixture("long-info.html"),
            &url("https://ncode.syosetu.com/novelview/infotop/ncode/n2267be/"),
        )
        .expect("the recorded info page must parse");

    // The page mixes `&nbsp;` and plain spaces between keywords, so this asserts
    // that both were resolved: a split that missed `&nbsp;` would yield one long
    // tag containing the lot.
    assert_eq!(
        work.tags,
        vec![
            "R15".to_owned(),
            "残酷な描写あり".to_owned(),
            "異世界転移".to_owned(),
            "シリアス".to_owned(),
            "ほのぼの".to_owned(),
            "異世界".to_owned(),
            "ファンタジー".to_owned(),
            "銀髪ヒロイン".to_owned(),
            "感想乞食".to_owned(),
            "バトル".to_owned(),
            "時間遡行".to_owned(),
            "死に戻り".to_owned(),
        ]
    );
}

#[test]
fn the_summary_is_read_as_prose_and_not_as_markup() {
    let work = adapter()
        .preview_from_html(
            &fixture("long-info.html"),
            &url("https://ncode.syosetu.com/novelview/infotop/ncode/n2267be/"),
        )
        .expect("the recorded info page must parse");

    assert!(work
        .summary
        .starts_with("突如、コンビニ帰りに異世界へ召喚された"));
    assert!(
        !work.summary.contains('<') && !work.summary.contains("&lt;"),
        "the summary still carries markup: {:?}",
        work.summary.chars().take(200).collect::<String>()
    );
}

// ---------------------------------------------------------------------------
// The episode list, and the refusal that matters
// ---------------------------------------------------------------------------

#[test]
fn the_first_list_page_yields_the_hundred_rows_it_shows() {
    let work = adapter()
        .preview_from_html(&fixture("long-work-p1.html"), &url(LONG_URL))
        .expect("the recorded work page must parse");

    // The page states エピソード 1 ～ 100 を表示中, and the adapter refuses a page
    // whose rows disagree with that statement, so this is the number the site
    // itself claims.
    assert_eq!(work.chapter_count(), 100);

    let first = &work.chapters[0];
    assert_eq!(first.ordinal, 1);
    assert_eq!(first.source_chapter_key, "1");
    assert_eq!(first.title, "プロローグ 『始まりの余熱』");

    let second = &work.chapters[1];
    assert_eq!(second.ordinal, 2);
    assert_eq!(second.source_chapter_key, "2");
    assert_eq!(second.title, "第一章１ 『ギザ十は使えない』");

    let last = &work.chapters[99];
    assert_eq!(last.ordinal, 100);
    assert_eq!(last.source_chapter_key, "100");
}

#[test]
fn the_chapter_keys_are_the_sites_own_episode_numbers() {
    let work = adapter()
        .preview_from_html(&fixture("long-work-p1.html"), &url(LONG_URL))
        .expect("the recorded work page must parse");

    // Not loop positions: the key is the number in the episode's URL, which is
    // what the site displays as `2/795`. A key that were the row index would
    // renumber every later episode the moment one was inserted.
    for (index, chapter) in work.chapters.iter().enumerate() {
        assert_eq!(chapter.ordinal as usize, index + 1);
        assert_eq!(chapter.source_chapter_key, chapter.ordinal.to_string());
    }
}

#[test]
fn a_later_list_page_continues_from_where_the_previous_one_stopped() {
    let page2 = adapter()
        .preview_from_html(&fixture("long-work-p2.html"), &url(LONG_URL))
        .expect("the recorded second page must parse");

    // The recorded page 2 states エピソード 101 ～ 200 を表示中.
    assert_eq!(page2.chapter_count(), 100);
    assert_eq!(page2.chapters[0].ordinal, 101);
    assert_eq!(page2.chapters[0].source_chapter_key, "101");
    assert_eq!(page2.chapters[99].ordinal, 200);
}

#[test]
fn a_partial_walk_is_refused_rather_than_imported_as_a_short_work() {
    // This is the test the three-page arrangement exists for.
    //
    // The work has 795 episodes. Its episode list serves 100 per page and says so
    // only on the info page. Given pages 1 and 2, a careless adapter reports 200
    // chapters and the import stores a seventh of the work, reporting success.
    let error = adapter()
        .assemble(
            LONG_NCODE,
            &fixture("long-work-p1.html"),
            &fixture("long-info.html"),
            &[fixture("long-work-p1.html"), fixture("long-work-p2.html")],
        )
        .expect_err("a two-page walk of an eight-page work must not assemble");

    match error {
        SourceError::Parse(message) => {
            // The message has to name both numbers, because an operator reading a
            // failed import needs to know which of the two moved.
            assert!(
                message.contains("795"),
                "the refusal should name the stated total: {message}"
            );
            assert!(
                message.contains("200"),
                "the refusal should name what was read: {message}"
            );
        }
        other => panic!("expected a parse failure, got {other:?}"),
    }
}

#[test]
fn a_one_page_walk_of_a_paginated_work_is_refused_too() {
    let error = adapter()
        .assemble(
            LONG_NCODE,
            &fixture("long-work-p1.html"),
            &fixture("long-info.html"),
            &[fixture("long-work-p1.html")],
        )
        .expect_err("one page of an eight-page work must not assemble");

    // One page is the case a real bug produces, and it must be as loud as two.
    assert!(matches!(error, SourceError::Parse(_)), "got {error:?}");
}

#[test]
fn the_info_page_states_the_total_that_the_list_pages_must_match() {
    // Guards the two tests above from becoming vacuous. If the recorded info page
    // ever stopped stating 全795エピソード, `assemble` would have nothing to check
    // the list against and a partial walk would pass.
    assert!(
        fixture("long-info.html").contains("全795エピソード"),
        "the recorded info page no longer states its episode total, so the \
         partial-walk refusal is checking nothing"
    );
    // And the pages really do cover 200 episodes, so the mismatch is 200 vs 795
    // and not 0 vs 795.
    let page1 = fixture("long-work-p1.html");
    let page2 = fixture("long-work-p2.html");
    assert!(page1.contains("エピソード&nbsp;1&nbsp;～&nbsp;100"));
    assert!(page2.contains("エピソード&nbsp;101&nbsp;～&nbsp;200"));
    // The pager promises eight pages, which is where 795 / 100 comes from.
    assert!(page1.contains("?p=8"));
}

#[test]
fn the_short_work_assembles_completely() {
    let work = short_work();

    assert_eq!(work.title, "Ｒｅ：ゼロから始める異世界生活 外典");
    assert_eq!(work.source_work_key, SHORT_NCODE);
    assert_eq!(work.status, WorkStatus::Ongoing);
    // Six episodes on one page, and the info page agrees, so the count check
    // passes — the other direction of the test above.
    assert_eq!(work.chapter_count(), 6);
    assert_eq!(work.chapters[0].source_chapter_key, "1");
    assert_eq!(work.chapters[0].title, "リゼロＥＸ 『とある殉教者の訃報』");
    assert_eq!(work.chapters[5].source_chapter_key, "6");
}

#[test]
fn the_short_works_infos_page_agrees_with_its_list() {
    // The counterpart to the long work's guard: a fixture where the two numbers
    // do agree, so the check is not simply refusing everything.
    assert!(fixture("short-info.html").contains("全6エピソード"));
}

#[test]
fn the_source_url_is_the_work_page_even_for_an_info_url() {
    // `fetch_chapters` reads `source_url`, so an info or episode URL stored there
    // would make a chapter import fetch the info page and find no prose.
    let work = adapter()
        .preview_from_html(
            &fixture("short-info.html"),
            &url("https://ncode.syosetu.com/novelview/infotop/ncode/n9525ii/"),
        )
        .expect("the recorded info page must parse");

    assert_eq!(work.source_url, SHORT_URL);
}

// ---------------------------------------------------------------------------
// The one-shot shape
// ---------------------------------------------------------------------------

#[test]
fn a_one_shot_is_one_chapter_keyed_by_its_own_ncode() {
    let work = one_shot();

    assert_eq!(work.title, "風呂場でクトゥルフなう");
    assert_eq!(work.author_text, "ＭＣＣ");
    assert_eq!(work.source_url, TANPEN_URL);
    assert_eq!(work.status, WorkStatus::Complete);
    assert_eq!(work.chapter_count(), 1);

    let chapter = &work.chapters[0];
    assert_eq!(chapter.ordinal, 1);
    // Not "1": there is no episode 1 to name, and this key is how
    // `fetch_chapter` knows to re-read the work page instead of building an
    // episode URL that does not exist.
    assert_eq!(chapter.source_chapter_key, TANPEN_NCODE);
    assert_eq!(chapter.title, "風呂場でクトゥルフなう");
}

#[test]
fn a_one_shots_dates_are_what_the_page_states_and_nothing_is_invented() {
    let work = one_shot();

    assert_eq!(work.published_at.map(|d| d.year()), Some(2013));
    assert_eq!(work.word_count, Some(7_420));
    // A 短編's info page has no 最新掲載日 and there is no episode list to take a
    // date from, so the revision date is absent. The page does not state one.
    assert_eq!(work.updated_at, None);
}

#[test]
fn a_one_shots_info_page_carries_no_episode_count_element() {
    // Why the two shapes are distinguished by the total rather than by the label:
    // the element is genuinely absent, not zero.
    let info = fixture("tanpen-info.html");
    assert!(info.contains("短編"));
    assert!(!info.contains("p-infotop-type__allep"));
}

// ---------------------------------------------------------------------------
// Chapter bodies
// ---------------------------------------------------------------------------

#[test]
fn an_episodes_prose_is_read_with_its_position_from_the_site() {
    let work = short_work();
    let chapters = adapter()
        .chapters_from_html(&fixture("short-ep2.html"), &work)
        .expect("the recorded episode must parse");

    assert_eq!(chapters.len(), 1);
    let chapter = &chapters[0];
    // From the page's own `N/total` marker, which reads `2/6` here. Without it the
    // page would have to be matched to the work by title.
    assert_eq!(chapter.ordinal, 2);
    assert_eq!(chapter.source_chapter_key, "2");
    assert_eq!(chapter.title, "リゼロＥＸ 『鬼も幸福も』");
    assert!(
        chapter.content_html.contains("スバル"),
        "the episode's prose is missing from the body"
    );
}

#[test]
fn the_first_episode_is_read_as_the_first() {
    let work = short_work();
    let chapters = adapter()
        .chapters_from_html(&fixture("short-ep1.html"), &work)
        .expect("the recorded episode must parse");

    assert_eq!(chapters[0].ordinal, 1);
    assert_eq!(chapters[0].source_chapter_key, "1");
    assert_eq!(chapters[0].title, "リゼロＥＸ 『とある殉教者の訃報』");
}

#[test]
fn the_authors_note_is_kept_and_set_apart_from_the_story() {
    let work = short_work();
    let chapters = adapter()
        .chapters_from_html(&fixture("short-ep1.html"), &work)
        .expect("the recorded episode must parse");
    let body = &chapters[0].content_html;

    // This episode has three text sections: a preface, the story, and an
    // afterword. The notes are the author speaking outside the narrative, and
    // they are wrapped in `blockquote` — the sanitiser's allow-list has no `div`
    // and no `class`, so a classed wrapper would be stripped and the note would
    // become indistinguishable from the prose.
    assert!(
        body.contains("<blockquote>"),
        "the author's note is not set apart from the story"
    );
    // The note's own words, and the story's, both present — so neither was
    // dropped in the process.
    assert!(body.contains("執筆の息抜きに書いた"));
    assert!(body.contains("その場所は暗く、陰気な雰囲気に満たされた空間だった"));
}

#[test]
fn a_chapter_body_carries_no_script_and_no_site_chrome() {
    let work = short_work();
    for name in ["short-ep1.html", "short-ep2.html"] {
        let chapters = adapter()
            .chapters_from_html(&fixture(name), &work)
            .expect("the recorded episode must parse");
        let body = &chapters[0].content_html;

        assert!(
            !body.contains("<script"),
            "{name} produced a script element"
        );
        assert!(
            !body.contains("javascript:"),
            "{name} produced a javascript: URL"
        );
        assert!(
            !body.contains("onclick"),
            "{name} produced an inline handler"
        );
        // The site's own furniture, none of which is the author's prose.
        assert!(
            !body.contains("p-novel__"),
            "{name} kept the site's wrapper classes"
        );
        assert!(
            !body.contains("js-novel-text"),
            "{name} kept the site's text marker"
        );
        assert!(!body.contains("c-pager"), "{name} kept the site's pager");
    }
}

#[test]
fn a_one_shots_prose_is_read_from_the_work_page_itself() {
    let work = one_shot();
    let chapters = adapter()
        .chapters_from_html(&fixture("tanpen-work.html"), &work)
        .expect("the recorded one-shot work page must parse");

    assert_eq!(chapters.len(), 1);
    assert_eq!(chapters[0].ordinal, 1);
    assert_eq!(chapters[0].source_chapter_key, TANPEN_NCODE);
    assert!(
        chapters[0].content_html.len() > 1_000,
        "the one-shot's prose looks empty"
    );
}

#[test]
fn a_one_shots_work_page_has_no_episode_list() {
    // Which is why the two shapes are told apart by the presence of a list rather
    // than by the number of rows it has.
    let page = fixture("tanpen-work.html");
    assert!(!page.contains("p-eplist__sublist"));
    assert!(page.contains("p-novel__text"));
}

// ---------------------------------------------------------------------------
// Failure, and routing
// ---------------------------------------------------------------------------

#[test]
fn the_sites_error_page_is_a_missing_work_and_not_an_empty_one() {
    let missing = url("https://ncode.syosetu.com/n9999zz/");

    let error = adapter()
        .preview_from_html(&fixture("not-found.html"), &missing)
        .expect_err("the recorded error page must not parse as a work");
    assert!(
        matches!(error, SourceError::NotFound),
        "expected NotFound, got {error:?}"
    );

    // And on the info-page path, which reads a different parser.
    let missing_info = url("https://ncode.syosetu.com/novelview/infotop/ncode/n9999zz/");
    let error = adapter()
        .preview_from_html(&fixture("not-found.html"), &missing_info)
        .expect_err("the recorded error page must not parse as an info page");
    assert!(
        matches!(error, SourceError::NotFound),
        "expected NotFound, got {error:?}"
    );
}

#[test]
fn the_not_found_page_carries_none_of_the_work_landmarks() {
    // Guards the test above from being satisfied by accident.
    let page = fixture("not-found.html");
    assert!(!page.contains("p-eplist__sublist"));
    assert!(!page.contains("p-novel__text"));
    assert!(!page.contains("p-infotop-data__title"));
    assert!(page.contains("見つかりません"));
}

#[test]
fn an_episode_page_with_no_prose_is_a_parse_failure() {
    let work = short_work();
    let error = adapter()
        .chapters_from_html("<html><body><p>nothing here</p></body></html>", &work)
        .expect_err("a page with no prose must not become a chapter");

    assert!(matches!(error, SourceError::Parse(_)), "got {error:?}");
}

#[test]
fn the_adapter_is_in_the_default_registry() {
    let registry = lorehaven_scrapers::sites::default_registry();

    let routed = registry
        .route(&url(LONG_URL))
        .expect("a Syosetu work URL should route to an adapter");
    assert_eq!(routed.key().as_str(), "syosetu");

    // An episode URL routes to the same adapter, so a reader pasting one gets the
    // work it belongs to.
    assert!(registry
        .route(&url("https://ncode.syosetu.com/n2267be/57/"))
        .is_ok());

    // The site's non-work pages must not route, or the importer would try to
    // import a help page.
    assert!(registry.route(&url("https://ncode.syosetu.com/")).is_err());
    assert!(registry
        .route(&url("https://ncode.syosetu.com/novelview/"))
        .is_err());
    assert!(registry
        .route(&url("https://www.example.com/n2267be/"))
        .is_err());
}

#[test]
fn the_adult_host_routes_to_nothing() {
    // Out of scope by decision: the host is behind an age gate, so no page could
    // be recorded to write a parser against. A source that appears in the
    // catalogue and always refuses is worse than one that is absent.
    let registry = lorehaven_scrapers::sites::default_registry();
    assert!(registry
        .route(&url("https://novel18.syosetu.com/n2267be/"))
        .is_err());
}

// ---------------------------------------------------------------------------
// Implementation notes, asserted
// ---------------------------------------------------------------------------

// Two properties of this adapter are deliberately not tested, and said so here
// rather than left as silence:
//
// * **The pager's page count against the pages read.** `assemble` checks that the
//   number of pages it was given matches the last page the site's pager
//   advertises. No recorded fixture can reach that check: for the 795-episode
//   work the episode-count check fires first and gives the more informative
//   message, and a fixture where the rows agreed with the total while the pages
//   did not would have to come from a site contradicting itself. It is a
//   defensive check, and it is recorded as unreachable rather than covered.
//
// * **The robots cache expiring and re-reading.** Covered in
//   `crates/scrapers/src/safety.rs`, including why the re-read itself is not
//   unit-tested: it would need a real request.
