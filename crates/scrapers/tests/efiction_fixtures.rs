//! eFiction, driven through recorded pages.
//!
//! Every assertion here is against a page recorded from the live site on
//! 2026-09-10 and committed under `tests/fixtures/efiction/`. Nothing in this
//! file reaches the network: a test that reaches the network is a test that fails
//! on a plane, and a parser written from memory of a site's markup is a parser
//! written from a guess.
//!
//! Five members are represented, because one is not enough to describe a family.
//! `tgstorytime.com` and `giantessworld.net` disagree about where a work's
//! rating lives, what a date looks like, whether there is a chapter container at
//! all, and how a class value is rendered — and every one of those disagreements
//! is an assertion below rather than a comment.
//!
//! The work-page assertions go through the adapters' `*_from_html` entry points,
//! which need no fetcher. The chapter assertions go through
//! [`FixtureFetcher`], because how an adapter *addresses* a chapter — the URL it
//! builds for chapter N — is as much a part of reading it as the parse is, and a
//! fixture served at the wrong URL would pass a test that could not work against
//! the real site.

use lorehaven_scrapers::safety::{decode_body, FixtureFetcher};
use lorehaven_scrapers::sites::efiction::Efiction;
use lorehaven_scrapers::SourceAdapter;
use lorehaven_scrapers::SourceError;
use lorehaven_scrapers::SourceWork;
use lorehaven_scrapers::WorkStatus;
use url::Url;

const TG_WORK: &str = "https://www.tgstorytime.com/viewstory.php?sid=6369&index=1";
const TG_CHAPTER_1: &str =
    "https://www.tgstorytime.com/viewstory.php?sid=6369&textsize=0&chapter=1";
/// A chapter URL as a reader would copy it out of their address bar.
const TG_WORK_FROM_CHAPTER: &str =
    "https://www.tgstorytime.com/viewstory.php?sid=6369&textsize=0&chapter=1";
const GW_WORK: &str = "https://www.giantessworld.net/viewstory.php?sid=11369&index=1";
const GW_CHAPTER_1: &str =
    "https://www.giantessworld.net/viewstory.php?sid=11369&textsize=0&chapter=1";
const GW_CHAPTER_2: &str =
    "https://www.giantessworld.net/viewstory.php?sid=11369&textsize=0&chapter=2";

/// Read a recorded page.
///
/// Through the crate's own decoder, because these pages declare `ISO-8859-1`
/// and carry Windows-1252 bytes: read as UTF-8 they do not decode at all, and
/// read lossily they lose the apostrophes the assertions below depend on.
fn fixture(name: &str) -> String {
    let path = format!("tests/fixtures/efiction/{name}");
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("the recorded fixture {path} must exist: {error}"));
    decode_body(&bytes, None)
}

fn url(raw: &str) -> Url {
    Url::parse(raw).expect("a fixture URL must parse")
}

/// The work, as read from a recorded work page.
fn recorded_work(file: &str, raw: &str) -> SourceWork {
    Efiction::new()
        .preview_from_html(&fixture(file), &url(raw))
        .unwrap_or_else(|error| panic!("{file} must parse: {error}"))
}

// ---------------------------------------------------------------------------
// The work page
// ---------------------------------------------------------------------------

#[test]
fn a_work_page_is_read_as_the_work_the_member_says_it_is() {
    let work = recorded_work("tgstorytime-work.html", TG_WORK);

    assert_eq!(work.title, "Camming down the rabbit hole");
    assert_eq!(work.author_text, "Dani_does_Dallas");
    assert_eq!(work.source_key.as_str(), "efiction");
}

#[test]
fn the_work_key_carries_the_member_because_a_sid_is_only_unique_within_one() {
    // `sid=6369` exists on every archive in the family, so a key that was only
    // the sid would collide across members and two different works would share
    // one import.
    assert_eq!(
        recorded_work("tgstorytime-work.html", TG_WORK).source_work_key,
        "tgstorytime.com/6369"
    );
    assert_eq!(
        recorded_work("giantessworld-work.html", GW_WORK).source_work_key,
        "giantessworld.net/11369"
    );
}

#[test]
fn a_chapter_url_previews_the_whole_work() {
    // A reader pastes whichever URL they were reading. Refusing the chapter URL
    // would refuse a URL the site itself links to.
    let work = recorded_work("tgstorytime-work.html", TG_WORK_FROM_CHAPTER);
    assert_eq!(work.source_url, TG_WORK);
    assert_eq!(work.chapters.len(), 19);
}

#[test]
fn the_author_is_read_with_the_profile_the_page_links_to() {
    let work = recorded_work("tgstorytime-work.html", TG_WORK);
    assert_eq!(
        work.author_url.as_deref(),
        Some("https://www.tgstorytime.com/viewuser.php?uid=14631")
    );
}

#[test]
fn a_member_that_emits_its_action_box_early_is_still_read_correctly() {
    // tgstorytime's work page carries **two** `#pagetitle` elements: the header
    // with the title and author, and a second holding only a `Report` link. An
    // adapter that took the first `#pagetitle` would be reading whichever the
    // skin emitted first; this one finds the header by the author link it
    // contains. The assertion is that the title is the work's, not `Report`.
    let work = recorded_work("tgstorytime-work.html", TG_WORK);
    assert_eq!(work.title, "Camming down the rabbit hole");
    assert_ne!(work.title, "Report");
}

// ---------------------------------------------------------------------------
// The chapter list, and the key that is not the ordinal
// ---------------------------------------------------------------------------

#[test]
fn tgstorytime_chapters_are_read_from_its_own_container() {
    let work = recorded_work("tgstorytime-work.html", TG_WORK);

    assert_eq!(work.chapters.len(), 19);
    assert_eq!(work.chapters[0].ordinal, 1);
    assert_eq!(work.chapters[0].title, "In the beginning - Chapter 1");
    assert_eq!(work.chapters[18].ordinal, 19);
    assert_eq!(work.chapters[18].title, "Getting my mojo back - Chapter 19");
}

#[test]
fn giantessworld_chapters_are_read_without_any_chapter_container() {
    // This member emits no `div#chapterlist` — the container tgstorytime uses
    // for every chapter. An adapter written against that container imports
    // nothing here and reports success, which is the failure mode this whole
    // directory exists to catch.
    let work = recorded_work("giantessworld-work.html", GW_WORK);

    assert_eq!(work.chapters.len(), 13);
    assert_eq!(work.chapters[0].title, "Chapter 1");
    assert_eq!(work.chapters[12].ordinal, 13);
}

#[test]
fn the_chapter_key_is_the_members_own_chapid_on_both_members() {
    // Not the ordinal: a chapter's position changes when the author inserts one
    // before it, and a reader's progress and notes are keyed on this.
    let tg = recorded_work("tgstorytime-work.html", TG_WORK);
    assert_eq!(tg.chapters[0].source_chapter_key, "32368");
    assert_eq!(tg.chapters[18].source_chapter_key, "50151");

    let gw = recorded_work("giantessworld-work.html", GW_WORK);
    assert_eq!(gw.chapters[0].source_chapter_key, "51874");
    assert_eq!(gw.chapters[12].source_chapter_key, "51886");
    assert!(
        gw.chapters.iter().all(|chapter| chapter
            .source_chapter_key
            .chars()
            .all(|c| c.is_ascii_digit())),
        "every chapter key must be a chapid, never an ordinal fallback: {:?}",
        gw.chapters
            .iter()
            .map(|chapter| chapter.source_chapter_key.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn the_printable_whole_work_link_is_not_a_chapter() {
    // `chapter=all` is the printable view of the entire work. Read as a chapter
    // it would add a fourteenth entry to a thirteen-chapter work, and the count
    // check would then refuse a page that is entirely correct.
    let work = recorded_work("giantessworld-work.html", GW_WORK);
    assert_eq!(work.chapters.len(), 13);
}

#[test]
fn a_page_whose_stated_count_disagrees_with_its_links_is_refused() {
    // The rule this crate exists for: a chapter list that silently drops entries
    // produces a truncated work that looks complete. Here the page still states
    // the truth and links have gone missing, which is what a changed skin looks
    // like from the outside.
    let damaged = fixture("giantessworld-work.html").replace(
        "<span class=\"label\">Chapters: </span> 13",
        "<span class=\"label\">Chapters: </span> 99",
    );
    let error = Efiction::new()
        .preview_from_html(&damaged, &url(GW_WORK))
        .expect_err("a count that disagrees with the links must not be imported");

    match error {
        SourceError::Parse(message) => {
            assert!(
                message.contains("99"),
                "the stated count is named: {message}"
            );
            assert!(message.contains("13"), "the count read is named: {message}");
        }
        other => panic!("a count mismatch is a parse failure, not {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Metadata the two members disagree about
// ---------------------------------------------------------------------------

#[test]
fn the_rating_is_found_wherever_the_member_keeps_it() {
    // giantessworld writes `Rated:` with the rest of the metadata; tgstorytime
    // writes it in a `div.storyinfo` of its own, above the block. Reading only
    // the block reports no rating for one member and a rating for the other.
    assert_eq!(
        recorded_work("tgstorytime-work.html", TG_WORK)
            .rating_text
            .as_deref(),
        Some("Adult")
    );
    assert_eq!(
        recorded_work("giantessworld-work.html", GW_WORK)
            .rating_text
            .as_deref(),
        Some("X")
    );
}

#[test]
fn the_chrome_around_a_rating_is_not_part_of_it() {
    // tgstorytime's markup is `Rated: Adult <a href="modules/epubversion/…">
    // Download ePub</a>`. Every anchor in a block is not a value, and a walk that
    // took them all reported this work's rating as `Adult Download ePub`.
    let rating = recorded_work("tgstorytime-work.html", TG_WORK).rating_text;
    assert_eq!(rating.as_deref(), Some("Adult"));
    assert!(
        !rating.unwrap_or_default().contains("Download"),
        "a download link is furniture, not a rating"
    );
}

#[test]
fn both_members_completed_vocabularies_are_read_as_a_status() {
    assert_eq!(
        recorded_work("giantessworld-work.html", GW_WORK).status,
        WorkStatus::Complete,
        "giantessworld writes `Completed: Yes`"
    );
    assert_eq!(
        recorded_work("tgstorytime-work.html", TG_WORK).status,
        WorkStatus::Ongoing,
        "tgstorytime writes `Completed: Story Incomplete`"
    );
}

#[test]
fn the_two_date_shapes_the_family_writes_are_both_read() {
    let tg = recorded_work("tgstorytime-work.html", TG_WORK);
    let published = tg.published_at.expect("`Published: 08/06/21` is a date");
    assert_eq!(
        (published.year(), published.month() as u8, published.day()),
        (2021, 6, 8),
        "read day-first: eFiction's own default, and these are British archives"
    );

    let gw = recorded_work("giantessworld-work.html", GW_WORK);
    let published = gw
        .published_at
        .expect("`Published: January 18 2022` is a date");
    assert_eq!(
        (published.year(), published.month() as u8, published.day()),
        (2022, 1, 18),
        "a month-name date with no comma, which a `%B %d, %Y` table misses"
    );
}

#[test]
fn a_word_count_is_read_as_a_number_and_is_the_works_own() {
    assert_eq!(
        recorded_work("tgstorytime-work.html", TG_WORK).word_count,
        Some(166_149)
    );
    assert_eq!(
        recorded_work("giantessworld-work.html", GW_WORK).word_count,
        Some(27_507)
    );
}

#[test]
fn a_summary_that_contains_commas_is_not_truncated_at_the_first_one() {
    // giantessworld's summary is three paragraphs of prose with commas in it. A
    // reader that treated every label as a list returned the work's description
    // as its own opening clause.
    let summary = recorded_work("giantessworld-work.html", GW_WORK).summary;

    assert!(
        summary.len() > 400,
        "the whole summary: {} chars",
        summary.len()
    );
    assert!(
        summary.contains("feel free to ask questions about it"),
        "the last paragraph survives: {summary:?}"
    );
}

#[test]
fn a_member_with_no_summary_label_falls_back_to_its_summary_element() {
    // tgstorytime has no `Summary:` label in the metadata block at all; its
    // description is in a `div.summarytext` beside it.
    let summary = recorded_work("tgstorytime-work.html", TG_WORK).summary;
    assert!(
        summary.starts_with("The story of how during the pandemic of 2020"),
        "{summary:?}"
    );
}

#[test]
fn the_familys_vocabulary_is_not_split_on_its_own_slashes() {
    // `Slow/Gradual Change` and `FF/m` are single values on the members that
    // write them. Splitting a value on `/` broke both in half and produced tags
    // called `Slow` and `m`.
    let tg = recorded_work("tgstorytime-work.html", TG_WORK).tags;
    assert!(tg.contains(&"Slow/Gradual Change".to_owned()), "{tg:?}");
    assert!(!tg.contains(&"Slow".to_owned()), "{tg:?}");

    let gw = recorded_work("giantessworld-work.html", GW_WORK).tags;
    assert!(gw.contains(&"FF/m".to_owned()), "{gw:?}");
    assert!(!gw.contains(&"FF".to_owned()), "{gw:?}");
}

#[test]
fn a_category_that_holds_a_comma_is_one_value_because_the_member_says_so() {
    // tgstorytime renders `Characters` as a single link whose text happens to
    // contain a comma; giantessworld renders `Categories` as one link per value.
    // The markup says which is which and the text does not.
    let tg = recorded_work("tgstorytime-work.html", TG_WORK).tags;
    assert!(
        tg.contains(&"Male to Female, Young Adult (20-26 yrs)".to_owned()),
        "{tg:?}"
    );

    let gw = recorded_work("giantessworld-work.html", GW_WORK).tags;
    for expected in ["Young Adult 20-29", "Breast Enlargement", "New World Order"] {
        assert!(
            gw.contains(&expected.to_owned()),
            "{expected} missing from {gw:?}"
        );
    }
}

#[test]
fn a_label_that_says_none_is_absence_rather_than_a_tag_called_none() {
    // `Characters: None` on giantessworld's work, `Series: None` on both.
    for work in [
        recorded_work("giantessworld-work.html", GW_WORK),
        recorded_work("tgstorytime-work.html", TG_WORK),
    ] {
        assert!(
            !work.tags.iter().any(|tag| tag.eq_ignore_ascii_case("none")),
            "`None` is the member writing that the label has no value: {:?}",
            work.tags
        );
    }
}

#[test]
fn a_warning_whose_text_is_a_sentence_survives_whole() {
    let warnings = recorded_work("giantessworld-work.html", GW_WORK).warning_texts;
    assert_eq!(
        warnings,
        vec!["Following story may contain inappropriate material for certain audiences".to_owned()]
    );
}

// ---------------------------------------------------------------------------
// The chapter page, and the prose
// ---------------------------------------------------------------------------
//
// Two facts about an eFiction chapter have to be tested separately, because they
// fail separately: the **address** the adapter asks for — the reading view at
// `&textsize=0&chapter=N` rather than the print view at `&chapter=N` — and the
// **parse** of what comes back. The address is asserted through the fetcher's
// record of what was requested, which holds whether or not a body came back; the
// parse is asserted through both the fetcher and the `*_from_html` seam.

/// A fetcher serving a recorded work page and the chapters of it that were
/// recorded, at the reading-view URL the adapter is expected to build.
///
/// The URLs are written out rather than derived, because they are the claim.
fn tg_reading() -> FixtureFetcher {
    FixtureFetcher::new()
        .with_page(TG_WORK, fixture("tgstorytime-work.html"))
        .with_page(TG_CHAPTER_1, fixture("tgstorytime-story-1.html"))
}

fn gw_reading() -> FixtureFetcher {
    FixtureFetcher::new()
        .with_page(GW_WORK, fixture("giantessworld-work.html"))
        .with_page(GW_CHAPTER_1, fixture("giantessworld-story-1.html"))
}

#[tokio::test]
async fn a_preview_asks_for_the_work_page_and_for_no_chapter_at_all() {
    // A preview that had already read a chapter body would be doing work it was
    // not asked to, and on a long work that is one request per chapter for a page
    // a reader is still deciding about.
    let fetch = tg_reading();
    Efiction::new()
        .preview(&fetch, &url(TG_WORK_FROM_CHAPTER), None)
        .await
        .expect("the recorded work page reads");

    assert_eq!(
        fetch.requested(),
        vec![TG_WORK.to_owned()],
        "exactly one request, to the normalised work page"
    );
}

#[tokio::test]
async fn a_chapter_is_addressed_at_the_reading_view_and_not_the_print_view() {
    // The print URL is the one a table of contents links to, so building it is
    // the obvious thing to do and the wrong thing to do: it is a request to
    // print, and one member of this family disallows
    // `viewstory.php?action=printable&*` in its own robots.txt by name.
    //
    // Only chapter 1 is recorded, and that is enough: what the second request
    // *was* is recorded by the fetcher whatever comes back, and the error names
    // the URL it could not find a body for.
    let fetch = gw_reading();
    let adapter = Efiction::new();
    let work = adapter
        .preview(&fetch, &url(GW_WORK), None)
        .await
        .expect("the recorded work page reads");

    let chapter = adapter
        .fetch_chapter(&fetch, &work, 1, None)
        .await
        .expect("chapter 1 reads on its own");
    assert_eq!(chapter.ordinal, 1);
    assert_eq!(chapter.source_chapter_key, "51874");
    assert!(
        chapter.content_html.contains("Sunspeak"),
        "the prose is there"
    );

    let second = adapter
        .fetch_chapter(&fetch, &work, 2, None)
        .await
        .expect_err("chapter 2 has no recorded body");
    assert!(
        format!("{second}").contains(GW_CHAPTER_2),
        "chapter 2 must be asked for at the reading view, and it was asked for \
         somewhere else: {second}"
    );

    for requested in fetch.requested() {
        assert!(
            !requested.contains("chapter=") || requested.contains("textsize=0"),
            "a chapter was requested at the print view: {requested}"
        );
    }
}

#[tokio::test]
async fn a_chapter_the_work_does_not_have_is_refused_rather_than_invented() {
    let fetch = gw_reading();
    let adapter = Efiction::new();
    let work = adapter
        .preview(&fetch, &url(GW_WORK), None)
        .await
        .expect("the recorded work page reads");

    let missing = adapter.fetch_chapter(&fetch, &work, 5000, None).await;
    assert!(matches!(missing, Err(SourceError::NotFound)), "{missing:?}");
}

#[tokio::test]
async fn a_whole_work_read_that_cannot_finish_does_not_report_success_on_part() {
    // Only chapter 1 of this work's thirteen is recorded, so the run stops at
    // chapter 2. What is asserted is that it stops with an *error*: a caller
    // taking the chapters it did return would be handed the first chapter of a
    // thirteen-chapter work and told the import worked.
    let fetch = gw_reading();
    let adapter = Efiction::new();
    let work = adapter
        .preview(&fetch, &url(GW_WORK), None)
        .await
        .expect("the recorded work page reads");

    let result = adapter.fetch_chapters(&fetch, &work, None).await;
    assert!(
        result.is_err(),
        "a partial read reported as success: {:?} chapters",
        result.map(|chapters| chapters.len())
    );
    assert_eq!(
        fetch.times_requested("chapter=1"),
        1,
        "and it read in order rather than re-reading: {:?}",
        fetch.requested()
    );
}

#[tokio::test]
async fn a_member_answering_with_its_print_template_still_yields_the_prose() {
    // The fallback that makes the choice of view safe. A member that answered a
    // reading-view URL with its print template would put the prose in
    // `div.chapter` instead of `div#story`, and every chapter would be lost to a
    // parse failure if only one container were read.
    let adapter = Efiction::new();

    let reading_fetch = tg_reading();
    let reading_work = adapter
        .preview(&reading_fetch, &url(TG_WORK), None)
        .await
        .expect("the recorded work page reads");
    let from_reading = adapter
        .fetch_chapter(&reading_fetch, &reading_work, 1, None)
        .await
        .expect("chapter 1 reads from the reading view");

    // The same URL as `tg_reading` serves, with the print view's body.
    let print_fetch = FixtureFetcher::new()
        .with_page(TG_WORK, fixture("tgstorytime-work.html"))
        .with_page(TG_CHAPTER_1, fixture("tgstorytime-chapter-1.html"));
    let print_work = adapter
        .preview(&print_fetch, &url(TG_WORK), None)
        .await
        .expect("the recorded work page reads");
    let from_print = adapter
        .fetch_chapter(&print_fetch, &print_work, 1, None)
        .await
        .expect("chapter 1 reads from the print template");

    assert_eq!(from_reading.ordinal, from_print.ordinal);
    assert_eq!(
        from_reading.source_chapter_key,
        from_print.source_chapter_key
    );
    assert!(
        from_print.content_html.contains("Back in January 2020"),
        "the print view's own container is read"
    );
    assert_eq!(
        from_reading.content_html, from_print.content_html,
        "the same chapter is the same prose whichever view served it"
    );
}

#[test]
fn a_chapter_page_yields_one_chapter_with_its_work_page_identity() {
    // The print view carries `div.chaptertitle`, `TITLE by AUTHOR`, which is how
    // the `*_from_html` seam recovers a chapter's place with no fetcher and so no
    // URL to take the ordinal from.
    let work = recorded_work("tgstorytime-work.html", TG_WORK);
    let chapters = Efiction::new()
        .chapters_from_html(&fixture("tgstorytime-chapter-1.html"), &work)
        .expect("the recorded chapter page parses");

    assert_eq!(chapters.len(), 1);
    assert_eq!(chapters[0].ordinal, 1);
    assert_eq!(chapters[0].source_chapter_key, "32368");
    assert_eq!(chapters[0].title, "In the beginning - Chapter 1");
}

#[test]
fn a_chapters_position_is_recovered_from_the_pages_own_heading() {
    let work = recorded_work("giantessworld-work.html", GW_WORK);
    let chapters = Efiction::new()
        .chapters_from_html(&fixture("giantessworld-chapter-2.html"), &work)
        .expect("the recorded chapter page parses");

    assert_eq!(chapters[0].ordinal, 2);
    assert_eq!(chapters[0].source_chapter_key, "51875");
    assert_eq!(chapters[0].title, "Chapter 2");
}

#[test]
fn a_chapter_page_that_does_not_name_itself_is_refused_with_that_reason() {
    // The reading view carries no `div.chaptertitle`, so a page of it cannot say
    // which chapter it is — which is fine for `fetch_chapter`, whose caller knows
    // the ordinal, and impossible for the `*_from_html` seam, which does not. The
    // failure has to say *that* rather than claim the work does not list it: one
    // is a page that needs its ordinal supplied and the other is a work that has
    // changed underneath a reader.
    let work = recorded_work("tgstorytime-work.html", TG_WORK);
    let error = Efiction::new()
        .chapters_from_html(&fixture("tgstorytime-story-1.html"), &work)
        .expect_err("the reading view cannot be placed without an ordinal");

    match error {
        SourceError::Parse(message) => assert!(
            message.contains("does not name which chapter"),
            "the reason must be the missing heading: {message}"
        ),
        other => panic!("a page with no heading is a parse failure, not {other:?}"),
    }
}

#[test]
fn the_prose_is_the_chapter_and_not_the_pages_furniture() {
    let work = recorded_work("giantessworld-work.html", GW_WORK);
    let chapters = Efiction::new()
        .chapters_from_html(&fixture("giantessworld-chapter-1.html"), &work)
        .expect("the recorded chapter page parses");
    let body = &chapters[0].content_html;

    assert!(body.contains("Sunspeak"), "the prose is there");
    assert!(
        !body.contains("archived at"),
        "the member's own footer line is not prose: {:.200}",
        body
    );
}

#[test]
fn a_chapters_author_notes_are_not_folded_into_its_prose() {
    // `div.notes` / `div.noteinfo` is the chapter's own editorial block, and it
    // is deliberately not part of the body: folding it in would mean inventing
    // markup to delimit it inside prose the author wrote.
    let work = recorded_work("tgstorytime-work.html", TG_WORK);
    let chapters = Efiction::new()
        .chapters_from_html(&fixture("tgstorytime-chapter-1.html"), &work)
        .expect("the recorded chapter page parses");

    assert!(
        !chapters[0]
            .content_html
            .contains("please forgive my spelling"),
        "the author's note stays out of the body"
    );
}

#[test]
fn the_prose_is_sanitised_on_the_way_out() {
    let work = recorded_work("giantessworld-work.html", GW_WORK);
    let chapters = Efiction::new()
        .chapters_from_html(&fixture("giantessworld-chapter-1.html"), &work)
        .expect("the recorded chapter page parses");
    let body = chapters[0].content_html.to_ascii_lowercase();

    for forbidden in ["<script", "javascript:", "onclick", "<iframe"] {
        assert!(
            !body.contains(forbidden),
            "{forbidden} must not survive into a stored chapter"
        );
    }
}

// ---------------------------------------------------------------------------
// The three ways a page is not a story
// ---------------------------------------------------------------------------

#[test]
fn a_moderation_hold_is_not_reported_as_a_work_that_does_not_exist() {
    // eFiction answers this with HTTP 200 and a page of ordinary furniture, so
    // there is no status code to read. The work exists and the member will not
    // serve it, and an import that said "no work at that URL" would send an
    // operator looking for a mistake they did not make.
    for (file, raw) in [
        (
            "tgstorytime-access-denied.html",
            "https://www.tgstorytime.com/viewstory.php?sid=6300",
        ),
        (
            "giantessworld-access-denied.html",
            "https://www.giantessworld.net/viewstory.php?sid=11300",
        ),
    ] {
        let error = Efiction::new()
            .preview_from_html(&fixture(file), &url(raw))
            .expect_err("an unvalidated story is not an importable work");
        assert!(
            matches!(error, SourceError::Withheld(_)),
            "{file} must be withheld, not {error:?}"
        );
        assert_eq!(error.category(), "withheld");
        assert!(!error.is_transient(), "and no retry will change it");
        assert!(
            !error.needs_the_reader(),
            "and there is nothing for a reader to fix"
        );
    }
}

#[test]
fn a_content_gate_asks_the_reader_rather_than_deciding_for_them() {
    for (file, raw) in [
        (
            "gluttony-content-warning.html",
            "https://www.gluttonyfiction.com/viewstory.php?sid=313",
        ),
        (
            "narutofic-content-warning.html",
            "https://www.narutofic.org/viewstory.php?sid=11545",
        ),
        (
            "ninelives-content-warning.html",
            "https://www.ninelivesarchive.com/viewstory.php?sid=3205",
        ),
    ] {
        let error = Efiction::new()
            .preview_from_html(&fixture(file), &url(raw))
            .expect_err("an adult gate is not an importable work until it is acknowledged");
        assert!(
            matches!(error, SourceError::AuthRequired(_)),
            "{file} must ask for a credential, not {error:?}"
        );
        assert!(error.needs_the_reader(), "the job pauses for the reader");
        assert!(!error.is_transient(), "and a retry alone does not help");
    }
}

#[test]
fn an_archives_own_statistics_are_never_read_as_the_works_metadata() {
    // The trap. Two of these gates carry the archive's totals — `Members:`,
    // `Series:`, `Stories:`, `Chapters:`, `Word count:`, `Reviewers:` — and
    // `Chapters:` and `Word count:` are also story fields. A parser that read
    // label spans generically would report this archive's 25,318 chapters and
    // 47,323,633 words as one work's, and nothing about the result would look
    // impossible.
    let page = fixture("narutofic-content-warning.html");
    assert!(
        page.contains("47323633"),
        "the archive's word count is on the page"
    );
    assert!(page.contains("25318"), "and so is its chapter count");

    let error = Efiction::new()
        .preview_from_html(
            &page,
            &url("https://www.narutofic.org/viewstory.php?sid=11545"),
        )
        .expect_err("a gate is not a work");
    assert!(
        matches!(error, SourceError::AuthRequired(_)),
        "the archive's totals must not turn a gate into a work: {error:?}"
    );
}

#[test]
fn a_challenge_wall_is_reported_as_a_block_rather_than_as_a_missing_work() {
    // Identification by the interstitial's own markers, not by the absence of
    // content: a page recognised only by what it lacks turns the next new gate
    // into "no work at that URL".
    let error = Efiction::new()
        .preview_from_html(
            &fixture("cloudflare-challenge.html"),
            &url("https://www.mugglenetfanfiction.com/viewstory.php?sid=1"),
        )
        .expect_err("a challenge wall is not a work");

    assert!(matches!(error, SourceError::Blocked), "{error:?}");
    assert!(error.is_transient(), "a challenge wall can lift");
}

#[test]
fn every_member_in_the_family_is_tried_not_only_the_two_that_were_recorded() {
    // The three gates are on three more members, and they are recognised by the
    // same walk as the two worked examples — which is the claim that this is one
    // adapter for a family rather than two adapters sharing a file name.
    for raw in [
        "https://www.gluttonyfiction.com/viewstory.php?sid=313",
        "https://www.narutofic.org/viewstory.php?sid=11545",
        "https://www.ninelivesarchive.com/viewstory.php?sid=3205",
    ] {
        assert!(
            Efiction::new().can_handle(&url(raw)),
            "{raw} belongs to a member of the family"
        );
    }
}
