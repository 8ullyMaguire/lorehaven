//! Gap F — the hidden-classics strategy: good works the feed has not surfaced.
//!
//! `hidden_classics_strategy` in `crates/db/src/rec_strategy.rs` closes #32 and #33
//! together, on the audit's argument that they are one gap: both are "the ranking
//! engines over-reward the already-popular", and both are answered by ranking quality
//! *relative to* reach rather than absolutely.
//!
//! The sibling strategies in that module rank by absolute quantities — bookmark
//! counts, graph degree, curation. That is the property this file exists to distinguish,
//! so the tests below are mostly about what the score does when two works have similar
//! absolute numbers and different distributions.
//!
//! The rules that need proving, each with the fixture that only it can fail:
//!
//!   * quality is completion over DISTINCT readers — a chapter opened four times is
//!     one reader, and counting rows would inflate the denominator;
//!   * reach is log-scaled, so the strategy returns *ranked* works rather than only
//!     works nobody has seen;
//!   * a work below §20.3's 10-reader floor is excluded rather than divided;
//!   * automated views are not reach, so a crawler cannot bury a hidden classic;
//!   * the score is per-unit-of-reach, so a much-more-seen work with the same absolute
//!     completion count loses.
//!
//! **A note on choosing these numbers, because it cost real time.** The first mutation
//! pass recorded three survivors: `COUNT(DISTINCT …)`, the `is_automated` filter, and
//! the log denominator. Every one survived because the fixture's *comparator* was picked
//! by eye and happened to hold under both the real query and the mutation. The fix is to
//! solve for the comparator rather than guess it: the correct behaviour and the mutated
//! behaviour must land on OPPOSITE sides of it, or the assertion cannot discriminate.
//!
//!     score(readers, completions) = (completions / readers) / (1 + log10(1 + readers))

use lorehaven_db::rec_strategy::{default_strategies, hidden_classics_strategy, RecContext};
use test_support::{scratch_dir, TestDb};

/// A reader account plus a scratch database.
struct Fixture {
    tdb: TestDb,
    reader: String,
}

impl Fixture {
    async fn build(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        let reader = account(&tdb, &format!("{tag}-reader@example.com")).await;
        Self { tdb, reader }
    }

    /// One published work with `readers` distinct human viewers and `completions`
    /// readers who finished it.
    async fn work(&self, title: &str, readers: usize, completions: usize) -> String {
        let owner = account(
            &self.tdb,
            &format!(
                "{title}-{}-author@example.com",
                uuid::Uuid::new_v4().simple()
            ),
        )
        .await;
        let work = uuid::Uuid::new_v4().to_string();
        exec_with(
            &self.tdb,
            "INSERT INTO works (id, owner_pseud_id, title, lifecycle, created_at, updated_at, \
                 generated_content_posture) \
             VALUES (?1#u, (SELECT id FROM pseuds WHERE account_id = ?2#u), ?3, 'published', \
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'forbid')",
            &[&work, &owner, &title.to_string()],
        )
        .await;
        for n in 0..readers {
            // `work_view_log`'s key is (work_id, viewer_hash, viewed_at), so distinct
            // readers get distinct hashes at a shared timestamp and no row collides.
            self.view(
                &work,
                &format!("{title}-v{n}"),
                "2026-01-03T00:00:00Z",
                false,
            )
            .await;
        }
        for n in 0..completions {
            let who = account(
                &self.tdb,
                &format!("{title}-c{n}-{}@example.com", uuid::Uuid::new_v4().simple()),
            )
            .await;
            exec_with(
                &self.tdb,
                "INSERT INTO reading_status (id, account_id, subject_type, subject_id, status, \
                     started_at, finished_at, updated_at) \
                 VALUES (?1#u, ?2#u, 'work', ?3#u, 'finished', '2026-01-02T00:00:00Z', \
                     '2026-01-05T00:00:00Z', '2026-01-05T00:00:00Z')",
                &[&uuid::Uuid::new_v4().to_string(), &who, &work.clone()],
            )
            .await;
        }
        work
    }

    /// One `work_view_log` row. A repeat view by the same reader needs a different
    /// `viewed_at`, which is what the phantom-reader test below exploits.
    async fn view(&self, work: &str, viewer: &str, at: &str, automated: bool) {
        exec_with(
            &self.tdb,
            "INSERT INTO work_view_log (work_id, viewer_hash, viewed_at, is_automated) \
             VALUES (?1#u, ?2, ?3, ?4#i)",
            &[
                &work.to_string(),
                &viewer.to_string(),
                &at.to_string(),
                &if automated { "1" } else { "0" }.to_owned(),
            ],
        )
        .await;
    }

    /// The strategy's ranked output for this fixture's reader.
    async fn ranked(&self) -> Vec<String> {
        hidden_classics_strategy()(
            self.tdb.db(),
            RecContext {
                account_id: self.reader.clone(),
                seen: vec![],
                cap: 50,
            },
        )
        .await
        .expect("the hidden-classics strategy")
    }
}

// ── the registration ────────────────────────────────────────────────────────

#[test]
fn the_strategy_is_registered_under_its_documented_name() {
    // A strategy that works and is not in `default_strategies` is a strategy no recipe
    // can name, which is the "implemented and invisible" failure again — the same one
    // that hid `quality_multiplier` and, before it, gap G's store.
    let strategies = default_strategies();
    assert!(
        strategies.contains_key("hidden_classics"),
        "a recipe must be able to name it: {:?}",
        strategies.keys().collect::<Vec<_>>()
    );
}

// ── the ranking rules ───────────────────────────────────────────────────────

#[tokio::test]
async fn a_good_work_beats_a_popular_work_of_the_same_completion_count() {
    // The gap's central claim. Both works have 20 completions; the first has 20 readers
    // (100% completion) and the second 400 (5%). An absolute-count ranking cannot
    // separate them — the absolute numbers are identical — which is why every sibling
    // strategy ranks blind to this.
    let f = Fixture::build("hc_basic").await;
    let hidden = f.work("hidden", 20, 20).await;
    let popular = f.work("popular", 400, 20).await;

    let ranked = f.ranked().await;
    assert_eq!(ranked.len(), 2, "both qualify: {ranked:?}");
    assert_eq!(
        ranked[0], hidden,
        "the well-read work outranks the much-more-seen one"
    );
    assert!(
        ranked.contains(&popular),
        "and the popular one is still offered"
    );
}

#[tokio::test]
async fn a_work_below_the_reader_floor_is_excluded_rather_than_divided() {
    // §20.3's minimum is 10 readers. A work with 2 readers who both finished has a
    // perfect 1.0 completion rate, and would otherwise be a candidate for the top slot.
    let f = Fixture::build("hc_floor").await;
    let tiny = f.work("tiny", 2, 2).await;
    let real = f.work("real", 40, 20).await;

    let ranked = f.ranked().await;
    // Asserting on the ORDER here was my first mistake: a 2-of-2 work scores 0.677 and a
    // 40-of-20 work scores 0.191, so the tiny work loses either way and the test passed
    // with the floor removed. What the floor decides is whether the tiny work appears AT
    // ALL — which is the thing the minimum exists to prevent, since a two-of-two work
    // has a perfect 1.0 rate and would dominate any completion-rate ranking.
    assert!(
        !ranked.contains(&tiny),
        "2 readers is not a completion rate, so it must not be ranked at all"
    );
    assert_eq!(ranked.first(), Some(&real), "and the real work is offered");
}

#[tokio::test]
async fn a_work_nobody_finished_is_not_a_hidden_classic() {
    // Reach with no completions is a work nobody finished, which is not "hidden and
    // good" — it is just visible and unwanted. `completions > 0` is what separates them.
    let f = Fixture::build("hc_nofinish").await;
    let unfinished = f.work("unfinished", 100, 0).await;
    let good = f.work("good", 30, 25).await;
    let ranked = f.ranked().await;
    assert!(!ranked.contains(&unfinished));
    assert_eq!(ranked.first(), Some(&good));
}

#[tokio::test]
async fn a_reader_who_opened_a_work_twice_is_one_reader_not_two() {
    // `COUNT(DISTINCT v.viewer_hash)` → `COUNT(v.viewer_hash)` survived the first
    // mutation pass, because every other fixture gives each reader exactly one view row.
    //
    // The failure it hides is the whole denominator: a reader who reopens a chapter
    // inflates the reach count, which lowers the score, which pushes a genuinely
    // well-read work down the ranking. Repeat views are legal and realistic — the key is
    // (work_id, viewer_hash, viewed_at), so they only need a different timestamp.
    //
    // **The comparator was solved for, not chosen.** The requirement is that the correct
    // count and the wrong one land on OPPOSITE sides of it:
    //
    //   DISTINCT:  (20/20) / (1 + log10(21))     = 0.4306   <- outranks
    //   comparator 300 readers / 273 completions = 0.2616
    //   COUNT:     (20/75) / (1 + log10(76))     = 0.0926   <- does not
    //
    // so `phantom` needs 20 DISTINCT readers but 75 rows — the 20 original viewers, then
    // the same twenty again 55 more times at later timestamps. Margins of 0.169 either
    // side, so no rounding decides it.
    let f = Fixture::build("hc_dedupe").await;
    let phantom = f.work("phantom", 20, 20).await;
    // 55 more rows from the SAME twenty readers, each at its own timestamp — the
    // primary key is (work_id, viewer_hash, viewed_at), so a repeat view is legal as
    // long as the time differs. This is the whole shape of the bug: 75 rows, 20 readers.
    for n in 0..55 {
        f.view(
            &phantom,
            &format!("phantom-v{}", n % 20),
            &format!("2026-01-04T00:{:02}:{:02}Z", n / 60, n % 60),
            false,
        )
        .await;
    }
    let popular_good = f.work("populargood", 300, 273).await;

    let ranked = f.ranked().await;
    let position = |id: &str| ranked.iter().position(|r| r == id).unwrap_or(usize::MAX);
    assert!(
        position(&phantom) < position(&popular_good),
        "55 distinct readers is not 20: phantom at {} vs popular-good at {} (ranked {ranked:?})",
        position(&phantom),
        position(&popular_good)
    );
}

#[tokio::test]
async fn a_crawler_cannot_dilute_a_hidden_classics_reach() {
    // `v.is_automated = 0` → `TRUE` also survived the first pass: no fixture had an
    // automated view.
    //
    // The failure is asymmetric and therefore sneaky. Counting crawler views inflates
    // ONLY the denominator of works a crawler walked but no person read, so the works
    // most likely to be genuinely undiscovered get pushed furthest down — the strategy
    // stops working and looks misconfigured rather than wrong.
    let f = Fixture::build("hc_crawler").await;
    let crawled = f.work("crawled", 20, 20).await;
    for n in 0..500 {
        f.view(&crawled, &format!("bot-{n}"), "2026-01-03T00:00:00Z", true)
            .await;
    }
    let quiet = f.work("quiet", 30, 24).await;
    let ranked = f.ranked().await;
    assert_eq!(
        ranked.first(),
        Some(&crawled),
        "500 crawler views are not reach; the work a person read perfectly still \
         ranks first"
    );
    assert!(ranked.contains(&quiet));
}

#[tokio::test]
async fn reach_costs_less_and_less_so_a_widely_read_good_work_still_ranks() {
    // Replacing the log denominator with a linear one also survived the first pass,
    // because the existing fixtures had reach within one order of magnitude.
    //
    // With linear division the denominator grows without bound, so a work with 4000
    // readers at 90% completion scores below one with 12 readers at 75% by two orders
    // of magnitude — and the strategy degenerates into "only things nobody has read",
    // which is the original problem restated rather than solved. Under the log it is
    // 0.1956 against 0.3548, so both are in the same conversation.
    let f = Fixture::build("hc_wide").await;
    let very_wide = f.work("verywide", 4000, 3600).await;
    let narrow = f.work("narrow", 12, 9).await;
    let ranked = f.ranked().await;
    assert!(
        ranked.contains(&very_wide),
        "a work 300x more widely read still belongs in the ranking: {ranked:?}"
    );
    assert!(ranked.contains(&narrow));

    // And the log still discriminates at equal order of magnitude: quality decides.
    // **The order is the assertion; presence is not.** A linear denominator still
    // returns every eligible work, so a presence-only test passes with the mutation
    // applied -- which is exactly why the first pass recorded a survivor here. The pair
    // below is one where the two denominators actively disagree:
    //
    //   log:    (100/100)/(1+log10(101)) = 0.3329  >  (6/10)/(1+log10(11)) = 0.2939
    //   linear: (100/100)/100          = 0.0100  <  (6/10)/10            = 0.0600
    //
    // The widely-read perfect work wins under the log and loses under a linear divisor,
    // because linear division punishes reach twice over instead of once.
    let f2 = Fixture::build("hc_wide2").await;
    let wide_perfect = f2.work("wideperfect", 100, 100).await;
    let narrow_ok = f2.work("narrowok", 10, 6).await;
    let ranked = f2.ranked().await;
    let position = |id: &str| ranked.iter().position(|r| r == id).unwrap_or(usize::MAX);
    assert_eq!(
        ranked.len(),
        2,
        "both clear the 10-reader floor: {ranked:?}"
    );
    assert!(
        position(&wide_perfect) < position(&narrow_ok),
        "a log denominator charges reach once, not twice: {} vs {}",
        position(&wide_perfect),
        position(&narrow_ok)
    );
}

#[tokio::test]
async fn two_works_that_tie_are_ordered_by_id_not_by_database_order() {
    // Reversing the tie-break survives unless two works score *exactly* equal, which
    // means identical (readers, completions): 20/20 and 20/20 both score 0.4306.
    //
    // The order is then decided by the work id, ascending. Asserting the ordering rather
    // than membership is the whole point -- a "both are present" assertion passes either
    // way. Seeded ids are random, so the test compares against the sorted pair rather
    // than hardcoding one, which keeps it honest without depending on insertion order.
    let f = Fixture::build("hc_tie").await;
    let a = f.work("tiea", 20, 20).await;
    let b = f.work("tieb", 20, 20).await;
    let ranked = f.ranked().await;
    assert_eq!(ranked.len(), 2, "both qualify: {ranked:?}");
    let mut by_id = vec![a.clone(), b.clone()];
    by_id.sort();
    assert_eq!(
        ranked, by_id,
        "ties break on ascending work id so the same catalogue feeds the same reader \
         twice"
    );
}

#[tokio::test]
async fn a_work_the_reader_already_bookmarked_is_not_offered_again() {
    // The one exclusion every sibling strategy shares, so this one must have it too.
    // Offering something the reader has bookmarked is telling them to do a thing they
    // have already done.
    let f = Fixture::build("hc_bookmarked").await;
    let work = f.work("bookmarked", 30, 25).await;
    exec_with(
        &f.tdb,
        "INSERT INTO bookmarks (id, account_id, subject_type, subject_id, created_at, updated_at) \
         VALUES (?1#u, ?2#u, 'work', ?3#u, '2026-01-06T00:00:00Z', '2026-01-06T00:00:00Z')",
        &[
            &uuid::Uuid::new_v4().to_string(),
            &f.reader.clone(),
            &work.clone(),
        ],
    )
    .await;
    assert!(
        !f.ranked().await.contains(&work),
        "a work the reader bookmarked is not news to them"
    );
}

#[tokio::test]
async fn an_unpublished_work_is_never_offered() {
    // Lifecycle filter, same as every sibling. A draft with 50 completions is not a
    // hidden classic; it is not published.
    let f = Fixture::build("hc_lifecycle").await;
    let owner = account(&f.tdb, "hc_lifecycle-draft@example.com").await;
    let draft = uuid::Uuid::new_v4().to_string();
    exec_with(
        &f.tdb,
        "INSERT INTO works (id, owner_pseud_id, title, lifecycle, created_at, updated_at, \
             generated_content_posture) \
         VALUES (?1#u, (SELECT id FROM pseuds WHERE account_id = ?2#u), 'Draft', 'draft', \
             '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'forbid')",
        &[&draft, &owner],
    )
    .await;
    // **The draft needs views AND completions, or this test passes for the wrong
    // reason.** My first version seeded neither, so `completions > 0` excluded it and the
    // lifecycle clause was never exercised — dropping that clause left the suite green.
    // A draft that readers finished is only excluded because it is not published, which
    // is the thing under test.
    for n in 0..30 {
        f.view(
            &draft,
            &format!("draft-v{n}"),
            "2026-01-03T00:00:00Z",
            false,
        )
        .await;
        let who = account(
            &f.tdb,
            &format!("draft-c{n}-{}@example.com", uuid::Uuid::new_v4().simple()),
        )
        .await;
        exec_with(
            &f.tdb,
            "INSERT INTO reading_status (id, account_id, subject_type, subject_id, status, \
                 started_at, finished_at, updated_at) \
             VALUES (?1#u, ?2#u, 'work', ?3#u, 'finished', '2026-01-02T00:00:00Z', \
                 '2026-01-05T00:00:00Z', '2026-01-05T00:00:00Z')",
            &[&uuid::Uuid::new_v4().to_string(), &who, &draft.clone()],
        )
        .await;
    }
    let ranked = f.ranked().await;
    assert!(
        !ranked.contains(&draft),
        "a draft with 30 readers and 30 completions scores 0.4306 -- the best in this \
         fixture -- and is still not published"
    );
}

#[tokio::test]
async fn an_empty_catalogue_returns_nothing_rather_than_failing() {
    // §16.1a: "A failing or empty strategy is skipped, never fatal." An empty result is
    // a legitimate answer that the blend must handle, so it has to be an empty list and
    // not an error.
    let f = Fixture::build("hc_empty").await;
    assert_eq!(f.ranked().await, Vec::<String>::new());
}

// ── fixtures ────────────────────────────────────────────────────────────────

/// An account with a pseud, by email. Written out rather than going through
/// `test_support::register` so each test can name the accounts it creates.
async fn account(tdb: &TestDb, email: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    exec_with(
        tdb,
        "INSERT INTO accounts (id, email, created_at, updated_at) \
         VALUES (?1#u, ?2, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        &[&id, &email.to_string()],
    )
    .await;
    exec_with(
        tdb,
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
         VALUES (?1#u, ?2#u, ?3, ?3, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        &[
            &uuid::Uuid::new_v4().to_string(),
            &id.clone(),
            &format!("h{}", &uuid::Uuid::new_v4().to_string()[..8]),
        ],
    )
    .await;
    id
}

/// `?N#u` binds a native uuid on PostgreSQL and text on SQLite; `?N#i` an integer;
/// a bare `?N` a string. All three dialects live in one helper because every fixture in
/// this file needs all three, and a per-backend fixture helper would triple the file.
async fn exec_with(tdb: &TestDb, sqlite: &str, args: &[&String]) {
    let db = tdb.db();
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            let sql = sqlite.replace("#u", "").replace("#i", "");
            let mut q = sqlx::query(&sql);
            for a in args {
                q = q.bind(a.as_str());
            }
            q.execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("fixture insert");
        }
        lorehaven_db::Backend::Postgres => {
            let pg = (1..=8).fold(sqlite.to_string(), |acc, n| {
                acc.replace(&format!("?{n}#u"), &format!("${n}::uuid"))
                    .replace(&format!("?{n}#i"), &format!("${n}::bigint"))
                    .replace(&format!("?{n}"), &format!("${n}"))
            });
            let mut q = sqlx::query(&pg);
            for a in args {
                match uuid::Uuid::parse_str(a) {
                    Ok(u) => q = q.bind(u),
                    Err(_) => q = q.bind(a.as_str()),
                }
            }
            q.execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("fixture insert");
        }
    }
}
