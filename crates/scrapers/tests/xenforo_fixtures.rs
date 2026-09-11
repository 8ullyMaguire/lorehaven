//! XenForo, driven through the pages recorded from three live forums.
//!
//! Every assertion here is against something recorded on 2026-09-11 and
//! committed under `tests/fixtures/xenforo/` — see the `## xenforo` section of
//! `tests/fixtures/README.md` for where each page came from.
//!
//! Two of the three recorded forums serve a plain request, so most of this suite
//! could have been recorded without a solver at all; SpaceBattles could not, and
//! its pages went through one. The suite is offline either way.

use lorehaven_scrapers::robots::RobotsRules;
use lorehaven_scrapers::sites::xenforo::{ThreadSummary, XenForo};
use lorehaven_scrapers::{SourceAdapter, Wall, WorkStatus};
use time::macros::datetime;
use url::Url;

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/xenforo")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} is not readable: {error}", path.display()))
}

const THREAD: &str = "262832";
const THREAD_URL: &str = "https://forums.spacebattles.com/threads/262832/";

fn spacebattles() -> XenForo {
    XenForo::spacebattles()
}

/// The work the two recorded SpaceBattles pages describe.
fn recorded_work() -> lorehaven_scrapers::SourceWork {
    let adapter = spacebattles();
    let thread = adapter
        .parse_thread(&fixture("thread.html"), THREAD)
        .expect("the recorded thread page parses");

    let mut marks = adapter
        .parse_threadmark_page(&fixture("threadmarks.html"), THREAD)
        .expect("the recorded first page of the list parses");
    let second = adapter
        .parse_threadmark_page(&fixture("threadmarks-page-2.html"), THREAD)
        .expect("the recorded second page of the list parses");
    marks.append(second);

    adapter.assemble_work(THREAD, thread, marks)
}

#[test]
fn the_threadmark_list_is_read_for_its_chapters_and_its_stated_total() {
    let adapter = spacebattles();
    let marks = adapter
        .parse_threadmark_page(&fixture("threadmarks.html"), THREAD)
        .expect("the recorded list parses");

    // The header. `Threadmarks: 42` is the total, and it is the check that makes
    // a partial import impossible.
    assert_eq!(marks.stated, Some(42));
    assert_eq!(marks.status, WorkStatus::Ongoing);
    // The header writes `2013-06-24T23:27:50-0400` — an offset of `-0400`, which
    // RFC 3339 does not allow. Read with its offset that is 03:27:50 UTC the
    // next day; read as UTC it would be four hours early.
    assert_eq!(marks.created, Some(datetime!(2013-06-25 03:27:50 UTC)));
    assert_eq!(marks.author.as_deref(), Some("master arminas"));

    // One page of a paginated list: 25, the site's page size.
    assert_eq!(marks.chapters.len(), 25);
    assert_eq!(marks.chapters[0].ordinal, 1);
    assert_eq!(marks.chapters[0].source_chapter_key, "11149727");
    assert_eq!(marks.chapters[0].title, "Prologue; September 27, 2596");
    assert_eq!(marks.chapters[24].ordinal, 25);

    // The dates are carried beside the chapters, which is what a work's last
    // change is read from.
    assert_eq!(marks.dates[0], Some(datetime!(2013-06-25 03:28:23 UTC)));

    // The work's last change is the **newest** chapter, and on this work that is
    // not the last chapter in reading order: the recorded list's newest date is
    // at ordinal 6 — a chapter added in 2024 to a thread running since 2013 —
    // while the list ends on a chapter posted in 2021. Taking the last element
    // would report a two-year-old date as the work's last change.
    assert_eq!(marks.updated(), Some(datetime!(2024-04-09 02:33:07 UTC)));
}

#[test]
fn the_two_pages_add_up_to_the_stated_total_and_the_ordinals_stay_dense() {
    let adapter = spacebattles();
    let mut marks = adapter
        .parse_threadmark_page(&fixture("threadmarks.html"), THREAD)
        .unwrap();
    let second = adapter
        .parse_threadmark_page(&fixture("threadmarks-page-2.html"), THREAD)
        .unwrap();
    assert_eq!(second.chapters.len(), 17);

    marks.append(second);

    // 25 + 17 is exactly the 42 the header states. This is the check the adapter
    // performs on a live import, asserted here against the recorded pages.
    assert_eq!(marks.chapters.len(), 42);
    assert_eq!(marks.stated, Some(42));

    // Ordinals are dense and in reading order across the page boundary: a reader's
    // progress and bookmarks are keyed to the ordinal, so two chapters sharing
    // one would put a chapter under somebody else's bookmark.
    assert!(marks
        .chapters
        .iter()
        .enumerate()
        .all(|(index, entry)| entry.ordinal == u32::try_from(index).unwrap() + 1));
    assert_eq!(marks.chapters[41].ordinal, 42);

    // No post appears twice, which is what a pagination overlap would look like.
    let mut keys: Vec<&str> = marks
        .chapters
        .iter()
        .map(|entry| entry.source_chapter_key.as_str())
        .collect();
    let count = keys.len();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), count);
    assert_eq!(marks.dates.len(), count);
}

#[test]
fn the_last_chapter_in_reading_order_is_not_the_newest_thing_in_the_thread() {
    let work = recorded_work();

    assert_eq!(work.chapters.len(), 42);
    // The final chapter of the list, which is the work's 42nd.
    assert_eq!(work.chapters[41].source_chapter_key, "80490627");
    assert_eq!(work.chapters[41].title, "April 17, 3026");

    // And the work's last change is a different chapter: ordinal 6 was posted in
    // 2024, while the list ends on one posted in 2021. `updated_at` is the newest
    // date, not the last one in reading order.
    assert_eq!(work.updated_at, Some(datetime!(2024-04-09 02:33:07 UTC)));
    assert_ne!(
        work.updated_at,
        Some(datetime!(2021-11-30 22:06:09 UTC)),
        "the last chapter's date is not the work's last change on this work"
    );
    // Publication is the header's own date.
    assert_eq!(work.published_at, Some(datetime!(2013-06-25 03:27:50 UTC)));
}

#[test]
fn the_thread_page_is_read_for_what_the_work_is() {
    let adapter = spacebattles();
    let thread = adapter
        .parse_thread(&fixture("thread.html"), THREAD)
        .unwrap();

    assert_eq!(thread.title, "By The Horns (Story only Thread)");
    assert_eq!(thread.author_text, "master arminas");
    assert_eq!(
        thread.author_url.as_deref(),
        Some("https://forums.spacebattles.com/members/master-arminas.28195/")
    );
    // The site's own one-line summary is used rather than the first post, which
    // on this thread is the author's index post but on many threads is the first
    // chapter. See the module documentation.
    assert!(
        thread.summary.starts_with("Okay, by popular request"),
        "{}",
        thread.summary
    );
    // One tag, and its category is stripped: the theme renders `Setting` from the
    // icon's <title> before the tag's own name.
    assert_eq!(thread.tags, vec!["battletech"]);
}

#[test]
fn a_chapter_is_read_from_the_article_for_that_post_and_not_from_the_first_one() {
    // The page the add-on serves for one post carries the **whole thread** — 26
    // articles, each with its own `.bbWrapper`. An unscoped selector returns the
    // first post's prose for every chapter, which is the trap this test exists
    // for: the first post is a different post, with different words in it.
    let page = fixture("post-11149727.html");
    assert!(
        page.matches("bbWrapper").count() > 1,
        "the recorded page carries more than one post"
    );

    let work = recorded_work();
    let chapter = spacebattles()
        .parse_post(&page, &work, "11149727")
        .expect("the recorded post page carries the post asked for");

    assert_eq!(chapter.source_chapter_key, "11149727");
    assert_eq!(chapter.ordinal, 1);
    assert_eq!(chapter.title, "Prologue; September 27, 2596");

    // The post asked for, not the first post on the page.
    // The prose is the requested post's. Its own first lines are the work's
    // title block, and the sentence that identifies it uniquely.
    assert!(
        chapter.content_html.contains("Stephen T Bynum"),
        "the prose belongs to the post asked for"
    );
    assert!(
        chapter.content_html.contains("Helena Vickers"),
        "the prose belongs to the post asked for"
    );
    assert!(
        !chapter.content_html.contains("by popular request"),
        "the first post's prose was returned for a later post"
    );
    assert!(chapter.content_html.len() > 5_000);
}

#[test]
fn a_post_the_served_page_does_not_carry_is_refused() {
    let work = recorded_work();
    let error = spacebattles()
        .parse_post(&fixture("post-11149727.html"), &work, "999999999")
        .expect_err("a post that is not on the page is not a chapter");
    assert!(format!("{error}").contains("999999999"), "{error}");
}

#[test]
fn a_thread_page_carries_a_widget_list_and_the_count_check_is_what_refuses_it() {
    // A trap worth writing down: a thread page is **not** free of threadmarks.
    // It carries a sidebar of the most recent few, which parses as a perfectly
    // valid short chapter list — five items against a stated forty-two.
    //
    // Nothing in this adapter reads a thread page as a chapter list, and this
    // test is the reason that has to stay true: the parser will happily return
    // the widget, and only the count check downstream notices that it is a
    // fraction of the work.
    let marks = spacebattles()
        .parse_threadmark_page(&fixture("thread.html"), THREAD)
        .expect("the widget parses as a list");

    assert!(
        marks.chapters.len() < 10,
        "the widget is a handful of items"
    );
    assert_eq!(
        marks.stated,
        Some(42),
        "and the page still states the work's real size, which is what catches it"
    );
    assert_ne!(
        marks.chapters.len(),
        marks.stated.unwrap() as usize,
        "a widget that matched the total would defeat the check"
    );

    // The adapter's own addresses never do this: the list it asks for is the
    // threadmarks route, and a work is refused when the two numbers disagree.
    assert!(spacebattles()
        .threadmarks_url(THREAD, 1)
        .contains("/threadmarks"));
    assert!(!spacebattles().thread_url(THREAD).contains("/threadmarks"));
}

#[test]
fn a_completed_work_is_read_as_complete() {
    let adapter = spacebattles();
    let marks = adapter
        .parse_threadmark_page(&fixture("threadmarks-complete.html"), "1333280")
        .expect("the recorded completed work parses");

    assert_eq!(marks.status, WorkStatus::Complete);
    assert_eq!(marks.stated, Some(1));
    assert_eq!(marks.chapters.len(), 1);
    assert_eq!(marks.chapters[0].source_chapter_key, "125333051");
    assert_eq!(
        marks.chapters[0].title,
        "The poem, may my cat rest in peace"
    );
    // Opened 2026-09-09, written in one post: the two dates are days apart and
    // read from different fields.
    assert_eq!(marks.created, Some(datetime!(2026-09-09 22:54:33 UTC)));

    let work = adapter.assemble_work("1333280", ThreadSummary::default(), marks);
    assert_eq!(work.status, WorkStatus::Complete);
    assert_eq!(work.published_at, Some(datetime!(2026-09-09 22:54:33 UTC)));
}

#[test]
fn the_three_forums_record_the_same_add_on_and_their_own_answers() {
    // SufficientVelocity, recorded plainly. The same template as SpaceBattles —
    // `data-template="svThreadmarks_threadmark_list"` — and a completely
    // different wall.
    let sv = XenForo::sufficient_velocity();
    let sv_marks = sv
        .parse_threadmark_page(&fixture("threadmarks-sufficientvelocity.html"), "148769")
        .expect("the recorded SufficientVelocity list parses");
    assert_eq!(sv_marks.status, WorkStatus::Ongoing);
    assert_eq!(sv_marks.stated, Some(77));
    assert_eq!(sv_marks.chapters.len(), 77);
    assert_eq!(sv_marks.chapters[0].ordinal, 1);
    assert_eq!(sv_marks.chapters[76].ordinal, 77);

    // QuestionableQuesting writes `Incomplete` for the state the other two call
    // `Ongoing`. Same state, different word — which is why the mapping is by
    // meaning and not by string comparison against one host's label.
    let qq = XenForo::questionable_questing();
    let qq_page = fixture("threadmarks-questionablequesting.html");
    assert!(
        qq_page.contains("<dd>Incomplete</dd>"),
        "the recorded QQ page no longer carries the label this test is about"
    );
    let qq_marks = qq
        .parse_threadmark_page(&qq_page, "39359")
        .expect("the recorded QuestionableQuesting list parses");
    assert_eq!(qq_marks.status, WorkStatus::Ongoing);
    assert_eq!(qq_marks.stated, Some(17));
    assert_eq!(qq_marks.chapters.len(), 17);

    // Each forum is its own source, and none of them declares an expensive wall.
    //
    // SpaceBattles was first recorded as `Wall::Solver` because a browser
    // `User-Agent` sent over non-browser TLS is challenged — the probe caused the
    // wall it reported. Declaring it would make every instance without a solver
    // refuse to import from SpaceBattles before queueing, for a host that serves
    // a plain request. See the module documentation for the re-measurement.
    for forum in [sv.forum(), qq.forum(), spacebattles().forum()] {
        assert_eq!(forum.wall(), Wall::None, "{forum:?}");
    }
    // The robots files are where the hosts genuinely differ.
    assert!(
        !fixture("robots-spacebattles.txt").is_empty()
            && fixture("robots-spacebattles.txt") == fixture("robots-sufficientvelocity.txt"),
        "two hosts publish the same file"
    );
    assert_eq!(sv.key().as_str(), "sufficientvelocity");
    assert_eq!(qq.key().as_str(), "questionablequesting");
    assert_eq!(spacebattles().key().as_str(), "spacebattles");
    assert_eq!(sv.display_name(), "Sufficient Velocity");

    // An address built by one forum is not claimed by another, even though the
    // paths are identical.
    let sv_thread = Url::parse(&sv.thread_url("148769")).unwrap();
    assert!(sv.can_handle(&sv_thread));
    assert!(!spacebattles().can_handle(&sv_thread));
    assert!(!qq.can_handle(&sv_thread));
}

#[test]
fn the_robots_files_allow_us_and_the_named_blocklist_is_what_would_stop_us() {
    // This is the one source in the repository where our own product token
    // decides whether we may read at all: both files are `Allow: /` for `*`,
    // followed by about ninety lines that disallow individual crawlers by name.
    // `RobotsRules::parse` is called with our token, so the claim this adapter
    // rests on is that our token is not one of those names.
    let rules = RobotsRules::parse(&fixture("robots-spacebattles.txt"), "Lorehaven");
    assert!(
        rules.was_found(),
        "the recorded file should have been found"
    );
    for path in [
        "/threads/262832/",
        "/threads/262832/threadmarks",
        "/threads/262832/post-11149727",
        "/forums/creative-writing.18/",
    ] {
        assert!(
            rules.allows(path),
            "Lorehaven should be allowed to read {path}"
        );
    }
    // No pace is published, so the fetcher's one-second floor is the pace.
    assert_eq!(rules.crawl_delay(), None);

    // And the blocklist really does block by name: the same file refuses a
    // crawler that is on it. This is what makes the assertion above mean
    // something — without it, a file that allowed everything to everyone would
    // pass the loop above just as well.
    for blocked in ["GPTBot", "ClaudeBot", "anthropic-ai", "CCBot", "Bytespider"] {
        let rules = RobotsRules::parse(&fixture("robots-spacebattles.txt"), blocked);
        assert!(
            !rules.allows("/threads/262832/threadmarks"),
            "the recorded file should disallow {blocked}"
        );
    }

    // SufficientVelocity publishes a byte-identical file, which is exactly why
    // the two are separate sources rather than one: the file agreeing does not
    // mean the wall agrees.
    assert_eq!(
        fixture("robots-spacebattles.txt"),
        fixture("robots-sufficientvelocity.txt")
    );
}

#[test]
fn a_forum_that_publishes_no_robots_file_has_no_rules_to_follow() {
    // QuestionableQuesting answers `404` for `/robots.txt` — no file at all,
    // which by `robots.txt` semantics means unrestricted, and by this fetcher's
    // behaviour means the default pace rather than no pace.
    //
    // That mapping lives in the fetcher, not here: `robots_for` turns a `404`
    // into `RobotsRules::unrestricted()` (asserted beside it in `safety.rs`), and
    // a page that failed to load at all is recorded as *unknown* rather than as
    // no rules. What this test pins down is the shape a missing file produces,
    // which is the one this forum is read under.
    let missing = RobotsRules::unrestricted();
    assert!(!missing.was_found());
    assert!(missing.allows("/threads/39359/threadmarks"));
    assert!(missing.allows("/"));
    // No pace is published, so the fetcher's one-second floor is the pace.
    assert_eq!(missing.crawl_delay(), None);

    // And a body that arrived but stated nothing is not the same thing as a
    // missing file — it was found, it just restricts nothing.
    let empty = RobotsRules::parse("", "Lorehaven");
    assert!(empty.was_found());
    assert!(empty.allows("/threads/39359/threadmarks"));
}

#[test]
fn a_work_is_assembled_from_both_documents_the_same_way_the_live_path_does() {
    // `preview` reads two documents and puts them together; the fixture seam is
    // handed one string, so it returns what that one document says the work is.
    // This is the seam's contract, asserted so it cannot drift silently.
    let adapter = spacebattles();
    let url = Url::parse(THREAD_URL).unwrap();

    let from_thread_page = adapter
        .preview_from_html(&fixture("thread.html"), &url)
        .expect("the thread page is enough to say what the work is");
    assert_eq!(from_thread_page.title, "By The Horns (Story only Thread)");
    assert_eq!(from_thread_page.source_work_key, THREAD);
    assert_eq!(from_thread_page.source_url, THREAD_URL);
    assert_eq!(from_thread_page.author_text, "master arminas");
    assert_eq!(from_thread_page.tags, vec!["battletech"]);
    // No chapters and no status: those are the other document's to state.
    assert!(from_thread_page.chapters.is_empty());
    assert_eq!(from_thread_page.status, WorkStatus::Unknown);

    // And assembled with both, the same call the live path makes.
    let work = recorded_work();
    assert_eq!(work.chapters.len(), 42);
    assert_eq!(work.status, WorkStatus::Ongoing);
    assert_eq!(work.source_url, THREAD_URL);
    assert_eq!(work.author_text, "master arminas");
    // Not stated anywhere readable: the list abbreviates its per-chapter counts
    // (`1.7k`) and the forum states no total. See the module documentation.
    assert_eq!(work.word_count, None);
    assert_eq!(work.language, None);
    assert_eq!(work.rating_text, None);
}

#[test]
fn the_chapters_a_page_carries_are_returned_in_the_works_order() {
    // A post address serves the containing page, and nothing on that page says
    // which post was asked for. What it indisputably carries is a set of this
    // work's chapters, in the page's order.
    let work = recorded_work();
    let chapters = spacebattles()
        .chapters_from_html(&fixture("post-11149727.html"), &work)
        .expect("the recorded page carries chapters of this work");

    assert!(!chapters.is_empty());
    // Only posts this work lists, and each at the ordinal the work gives it.
    for chapter in &chapters {
        let entry = work
            .chapters
            .iter()
            .find(|entry| entry.source_chapter_key == chapter.source_chapter_key)
            .expect("a returned chapter is one the work lists");
        assert_eq!(chapter.ordinal, entry.ordinal);
    }
    // Ascending by ordinal, which is the work's reading order.
    assert!(chapters
        .windows(2)
        .all(|pair| pair[0].ordinal < pair[1].ordinal));
    // The post this fixture is named for is among them.
    assert!(chapters
        .iter()
        .any(|chapter| chapter.source_chapter_key == "11149727"));

    // A page with none of this work's chapters is refused rather than read as
    // an empty chapter.
    let error = spacebattles()
        .chapters_from_html(&fixture("threadmarks.html"), &work)
        .expect_err("a threadmark list carries no prose");
    assert!(
        format!("{error}").contains("none of its chapters"),
        "{error}"
    );
}
