//! FanFiction.net and FictionPress, driven through recorded pages.
//!
//! Every assertion here is against a page recorded from the live site on
//! 2026-09-11 and committed under `tests/fixtures/ffnet/`. Nothing here reaches
//! the network: the recordings themselves needed the unblock path, so a test
//! that re-fetched them would need a solver service running to check a parser.
//!
//! The two hosts are the point of the set. They run one script and disagree
//! about attribute quoting, date format, whether a work carries a `Status:`
//! field and whether it lists its characters — so most tests below run twice,
//! once per host, and the pair is what holds the reading to what the site
//! actually emits.

use lorehaven_scrapers::sites::ffnet::{FanFiction, Site};
use lorehaven_scrapers::sites::ffnet::{FANFICTION_NET_KEY, FICTIONPRESS_KEY};
use lorehaven_scrapers::{SourceAdapter, SourceError, Wall, WorkStatus};
use time::macros::datetime;
use url::Url;

const FANFICTION_WORK: &str = "https://www.fanfiction.net/s/12345678/1/";
const FANFICTION_COMPLETE: &str = "https://www.fanfiction.net/s/5782108/1/";
const FICTIONPRESS_WORK: &str = "https://www.fictionpress.com/s/3280165/1/Unrelenting";
const FICTIONPRESS_SECOND: &str = "https://www.fictionpress.com/s/2171761/1/Vampiric-Desires";

/// Read a recorded page.
fn fixture(name: &str) -> String {
    let path = format!("tests/fixtures/ffnet/{name}");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("the recorded fixture {path} must exist: {error}"))
}

fn url(raw: &str) -> Url {
    Url::parse(raw).expect("a fixture URL must parse")
}

/// The two-chapter FanFiction.net work, as read from its recorded page.
fn two_chapter_work() -> lorehaven_scrapers::SourceWork {
    FanFiction::fanfiction_net()
        .preview_from_html(&fixture("ffnet-work.html"), &url(FANFICTION_WORK))
        .expect("the recorded work page must parse")
}

/// The 122-chapter completed FanFiction.net work.
fn long_work() -> lorehaven_scrapers::SourceWork {
    FanFiction::fanfiction_net()
        .preview_from_html(
            &fixture("ffnet-work-complete.html"),
            &url(FANFICTION_COMPLETE),
        )
        .expect("the recorded work page must parse")
}

/// The seventeen-chapter FictionPress work.
fn fictionpress_work() -> lorehaven_scrapers::SourceWork {
    FanFiction::fiction_press()
        .preview_from_html(&fixture("fictionpress-work.html"), &url(FICTIONPRESS_WORK))
        .expect("the recorded work page must parse")
}

#[test]
fn the_work_page_is_read_as_the_work_the_site_says_it_is() {
    let work = two_chapter_work();

    assert_eq!(work.source_key.as_str(), FANFICTION_NET_KEY);
    assert_eq!(work.source_work_key, "12345678");
    assert_eq!(work.title, "Jillian Holtzmann: Ace Attorney");
    // Canonical without the slug: the site serves `/s/{id}/` and redirects its
    // slugged form to it, so a stored address that carried a slug would be an
    // address the title can invalidate.
    assert_eq!(work.source_url, "https://www.fanfiction.net/s/12345678/");
}

#[test]
fn the_author_is_read_with_the_profile_the_byline_links() {
    let work = two_chapter_work();

    assert_eq!(work.author_text, "Pieland24");
    assert_eq!(
        work.author_url.as_deref(),
        Some("https://www.fanfiction.net/u/3631163/Pieland24")
    );

    // The same reading on the sibling host, whose byline is a different author.
    let work = fictionpress_work();
    assert_eq!(work.author_text, "OdderThings");
    assert_eq!(
        work.author_url.as_deref(),
        Some("https://www.fictionpress.com/u/1057028/OdderThings")
    );
}

#[test]
fn the_summary_is_the_block_the_page_sets_beside_the_byline() {
    let work = two_chapter_work();

    assert_eq!(
        work.summary,
        "Literally a Phoenix Wright AU that no one asked for. Slow-burn Holtzbert."
    );
}

#[test]
fn the_metadata_line_is_read_through_its_labels_on_both_hosts() {
    let work = two_chapter_work();
    assert_eq!(work.rating_text.as_deref(), Some("Fiction T"));
    assert_eq!(work.language.as_deref(), Some("English"));
    assert_eq!(work.word_count, Some(3_781));

    let work = fictionpress_work();
    assert_eq!(work.rating_text.as_deref(), Some("Fiction M"));
    assert_eq!(work.language.as_deref(), Some("English"));
    assert_eq!(work.word_count, Some(31_021));
}

#[test]
fn genres_and_characters_are_read_from_the_unlabeled_run() {
    let work = two_chapter_work();
    assert_eq!(
        work.tags,
        vec![
            "Humor",
            "Romance",
            "J. Holtzmann",
            "Patty T.",
            "Erin G.",
            "Abby Y."
        ]
    );

    // FictionPress omits the characters field on both recordings, so the run is
    // two fields where FanFiction.net's is three. Reading it by position from
    // the start of the line files a character list as genres here.
    let work = fictionpress_work();
    assert_eq!(work.tags, vec!["Romance", "Angst"]);
}

#[test]
fn a_work_without_a_status_field_is_unknown_and_not_ongoing() {
    // The field is present or absent, and that is the whole of it. The ported
    // adapter answered `ongoing` for every work, which on this site is wrong
    // half the time with nothing on the page to indicate it.
    assert_eq!(two_chapter_work().status, WorkStatus::Unknown);
    assert_eq!(long_work().status, WorkStatus::Complete);
}

#[test]
fn the_timestamps_are_the_epochs_the_page_carries_not_the_dates_it_shows() {
    // FanFiction.net writes `3/14/2015` and FictionPress writes `Jun 20, 2016`
    // for the same field. Neither is parsed: both hosts carry the real
    // timestamp in `data-xutime`, and these are those.
    let work = two_chapter_work();
    assert_eq!(work.updated_at, Some(datetime!(2017-02-07 17:23:17 UTC)));
    assert_eq!(work.published_at, Some(datetime!(2017-01-31 16:16:14 UTC)));

    let work = long_work();
    assert_eq!(work.updated_at, Some(datetime!(2015-03-14 15:59:42 UTC)));
    assert_eq!(work.published_at, Some(datetime!(2010-02-28 08:12:39 UTC)));

    // The sibling's visible dates are the abbreviated-month shape.
    let work = fictionpress_work();
    assert_eq!(work.updated_at, Some(datetime!(2016-06-20 15:38:19 UTC)));
    assert_eq!(work.published_at, Some(datetime!(2016-03-13 15:18:39 UTC)));
}

#[test]
fn the_chapter_list_is_complete_and_is_not_doubled() {
    // The list is rendered twice — top and bottom navigation — and the two are
    // identical. A parser that collected every `#chap_select option` would
    // report twice the work's length and import every chapter twice.
    let work = two_chapter_work();
    assert_eq!(work.chapter_count(), 2);

    let work = fictionpress_work();
    assert_eq!(work.chapter_count(), 17);

    // 122 chapters, and the site's own count beside them agrees: 244 options in
    // the recording, two renders of the same 122.
    let work = long_work();
    assert_eq!(work.chapter_count(), 122);
}

#[test]
fn a_chapter_is_keyed_by_its_ordinal_because_that_is_what_the_source_has() {
    let work = long_work();

    assert_eq!(work.chapters[0].ordinal, 1);
    assert_eq!(work.chapters[0].source_chapter_key, "1");
    assert_eq!(work.chapters[0].title, "A Day of Very Low Probability");
    assert_eq!(work.chapters[121].ordinal, 122);
    assert_eq!(work.chapters[121].source_chapter_key, "122");
    assert_eq!(
        work.chapters[121].title,
        "Something to Protect: Hermione Granger"
    );

    // Every ordinal is present exactly once, in order.
    let ordinals: Vec<u32> = work.chapters.iter().map(|entry| entry.ordinal).collect();
    assert_eq!(ordinals, (1..=122).collect::<Vec<u32>>());
}

#[test]
fn an_untitled_chapter_keeps_the_name_the_site_generated_for_it() {
    let work = fictionpress_work();
    assert_eq!(work.chapters[0].title, "Chapter 1");
    assert_eq!(work.chapters[16].title, "Chapter 17");

    // And where the author did name them, the site prefixes the work's own
    // title to each. The prefix is part of the title the site shows, so it is
    // part of the title that is stored.
    let work = FanFiction::fiction_press()
        .preview_from_html(
            &fixture("fictionpress-work-second.html"),
            &url(FICTIONPRESS_SECOND),
        )
        .expect("the recorded work page must parse");
    assert_eq!(work.source_work_key, "2171761");
    assert_eq!(work.title, "Vampiric Desires");
    assert_eq!(work.chapters[0].title, "Vampiric Desires Chapter 1");
    assert_eq!(work.chapters[3].title, "Vampiric Desires Chapter 4");
    // This recording carries no `Follows:` field at all, and no characters.
    assert_eq!(work.word_count, Some(2_171));
    assert_eq!(work.tags, vec!["Supernatural", "Fantasy"]);
    assert_eq!(work.updated_at, Some(datetime!(2007-11-03 23:07:05 UTC)));
    assert_eq!(work.published_at, Some(datetime!(2006-05-11 22:33:23 UTC)));
}

#[test]
fn a_chapter_page_states_its_own_position_and_the_body_is_the_prose() {
    let work = two_chapter_work();
    let chapters = FanFiction::fanfiction_net()
        .chapters_from_html(&fixture("ffnet-chapter-2.html"), &work)
        .expect("the recorded chapter page must parse");

    assert_eq!(chapters.len(), 1, "a chapter document holds one chapter");
    let chapter = &chapters[0];

    // The page does not otherwise say which chapter it is. The option the site
    // marks `selected` does, and it is why this page reads as chapter 2 rather
    // than as chapter 1 — the same page shape as the work page it came from.
    assert_eq!(chapter.ordinal, 2);
    assert_eq!(chapter.source_chapter_key, "2");
    assert_eq!(chapter.title, "Chapter 2");

    assert!(
        chapter
            .content_html
            .contains("Abby was the first to greet them."),
        "the prose must survive sanitation: {}",
        chapter.content_html
    );
    assert!(chapter.content_html.contains("<p>"));
    // `div#storytextp` is the wrapper and `div#storytext` is the prose, one
    // character apart. A prefix match would return the prose wrapped in its own
    // container; the exact id does not.
    assert!(!chapter.content_html.contains("storytext"));
    // And the site's chrome is gone.
    assert!(!chapter.content_html.contains("<script"));
    assert!(!chapter.content_html.contains("onclick"));
}

#[test]
fn the_work_page_is_read_as_chapter_one_by_the_same_rule() {
    // `/s/{id}/` is chapter 1's page, and its own `selected` option says so.
    let work = two_chapter_work();
    let chapters = FanFiction::fanfiction_net()
        .chapters_from_html(&fixture("ffnet-work.html"), &work)
        .expect("the recorded work page must parse as a chapter");

    assert_eq!(chapters[0].ordinal, 1);
    assert_eq!(chapters[0].title, "Chapter 1");
    assert!(!chapters[0].content_html.is_empty());
}

#[test]
fn a_missing_work_is_not_found_on_both_hosts() {
    // Both hosts answer HTTP 200 for a work that does not exist, so nothing may
    // be read from the status code. These are the recorded pages.
    let err = FanFiction::fanfiction_net()
        .preview_from_html(
            &fixture("ffnet-not-found.html"),
            &url("https://www.fanfiction.net/s/99999999999999/1/"),
        )
        .expect_err("a missing work must not parse as a work");
    assert!(
        matches!(err, SourceError::NotFound),
        "expected NotFound, got {err:?}"
    );

    let err = FanFiction::fiction_press()
        .preview_from_html(
            &fixture("fictionpress-not-found.html"),
            &url("https://www.fictionpress.com/s/99999999999999/1/"),
        )
        .expect_err("a missing work must not parse as a work");
    assert!(
        matches!(err, SourceError::NotFound),
        "expected NotFound, got {err:?}"
    );
}

#[test]
fn the_missing_work_page_is_not_read_as_a_moderation_hold() {
    // It carries the sentence "Story is unavailable for reading.", which is
    // boilerplate for a work that simply never existed. Reading it as a hold
    // would send an operator looking for a takedown that never happened.
    let page = fixture("ffnet-not-found.html");
    assert!(page.contains("Story is unavailable for reading."));

    let err = FanFiction::fanfiction_net()
        .preview_from_html(
            &page,
            &url("https://www.fanfiction.net/s/99999999999999/1/"),
        )
        .expect_err("not a work");
    assert!(matches!(err, SourceError::NotFound));
}

#[test]
fn a_page_that_is_neither_a_work_nor_the_sites_not_found_page_is_a_parse_failure() {
    // The distinction the recordings exist to make: a markup change must not be
    // reported as a deleted work, and a deleted work must not be reported as a
    // markup change.
    let err = FanFiction::fanfiction_net()
        .preview_from_html(
            "<html><body><h1>Service unavailable</h1></body></html>",
            &url(FANFICTION_WORK),
        )
        .expect_err("an unrecognised page is not a work");
    assert!(
        matches!(err, SourceError::Parse(_)),
        "expected Parse, got {err:?}"
    );
}

#[test]
fn each_host_declares_the_wall_that_was_measured_against_it() {
    // The fact this module exists to keep. FanFiction.net accepted a browser
    // fingerprint on 2026-09-11; FictionPress refused three different browsers
    // and needed the solver. An adapter that inherited one from the other would
    // be wrong about its own host in whichever direction they differ.
    assert_eq!(FanFiction::fanfiction_net().wall(), Wall::Fingerprint);
    assert_eq!(FanFiction::fiction_press().wall(), Wall::Solver);
}

#[test]
fn each_host_registers_under_its_own_key_and_claims_only_its_own_addresses() {
    let ffnet = FanFiction::fanfiction_net();
    let fictionpress = FanFiction::fiction_press();

    assert_eq!(ffnet.key().as_str(), FANFICTION_NET_KEY);
    assert_eq!(fictionpress.key().as_str(), FICTIONPRESS_KEY);
    assert_eq!(ffnet.display_name(), "FanFiction.net");
    assert_eq!(fictionpress.display_name(), "FictionPress");

    assert_eq!(ffnet.hosts(), vec!["fanfiction.net"]);
    assert_eq!(fictionpress.hosts(), vec!["fictionpress.com"]);

    // The allow-list holds the bare host; a URL that arrives with `www.` is the
    // same site, and both spellings must be claimed.
    assert!(ffnet.can_handle(&url("https://fanfiction.net/s/1/1/")));
    assert!(ffnet.can_handle(&url("https://www.fanfiction.net/s/1/1/")));
    assert!(!ffnet.can_handle(&url("https://www.fanfiction.net/u/1/Someone")));

    assert_eq!(Site::FanFictionNet.key(), FANFICTION_NET_KEY);
}

#[test]
fn the_source_publishes_its_pace_and_the_adapter_repeats_it() {
    // Both hosts publish `crawl-delay: 5` in their own robots.txt, where
    // `User-agent: *` is `Allow: /`. This is the adapter telling an operator
    // what to expect; the fetcher enforces it per host either way.
    assert_eq!(
        FanFiction::fanfiction_net()
            .capabilities()
            .min_interval_millis,
        Some(5_000)
    );
    assert_eq!(
        FanFiction::fiction_press()
            .capabilities()
            .min_interval_millis,
        Some(5_000)
    );
}

#[test]
fn a_single_chapter_can_be_retried_without_the_rest_of_the_work() {
    // Every chapter has its own address and the address follows from the
    // ordinal, so the retry criterion is genuinely available on this source.
    let capabilities = FanFiction::fanfiction_net().capabilities();
    assert!(capabilities.per_chapter_fetch);
    assert!(capabilities.metadata);
    assert!(capabilities.chapters);
    // Author pages exist but their markup has not been recorded, so this claims
    // nothing about them.
    assert!(!capabilities.bibliography);
}
