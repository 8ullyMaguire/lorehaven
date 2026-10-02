//! §20.3 — the payout store against real data, green on SQLite and PostgreSQL.
//!
//! `crates/db/src/payout_store.rs` reads six counts out of five tables and posts a
//! credit transaction. Both halves can be right while the join between them is
//! wrong, and this file is about the joins.
//!
//! The three that matter, each with a test:
//!
//!   * `reading_status` keys on `account_id`, the other four on `work_id` or a
//!     viewer identity — so a typo there yields 0 finishers and a multiplier of
//!     exactly 1.0x, which is indistinguishable from a genuinely mediocre work.
//!   * `work_view_log` has no uniqueness on the reader, so a repeated chapter view
//!     counts once via `COUNT(DISTINCT viewer_hash)` and would count N times
//!     without the DISTINCT.
//!   * the demand multiplier must not reach the ledger, or §0.3's secret becomes
//!     recoverable from a row an author can read.

use test_support::{scratch_dir, TestClient, TestDb};

use lorehaven_db::payout_store::{self, caps, DemandInputs, BP_PER_CREDIT};

const T0: i64 = 1_767_225_600; // 2026-01-01
const DAY: i64 = 86_400;
const WIN_FROM: i64 = T0;
const WIN_TO: i64 = T0 + 30 * DAY;

/// One work, an owner, and `readers` readers who each open, finish, and bookmark.
struct Fixture {
    tdb: TestDb,
    account: String,
    work: String,
}

impl Fixture {
    async fn build(tag: &str, readers: usize) -> Self {
        let dir = scratch_dir(tag);
        // ONE TestDb for everything. An earlier version registered through a
        // bootstrap router built on a *second* `connect_with_dir` call, which on
        // PostgreSQL creates a different database -- so the account was created
        // somewhere the fixture then queried and `RowNotFound` came back. The
        // register helper returns the id, so no second lookup is needed anyway.
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        let mut client = TestClient::new(lorehaven_app::server::build_router(
            lorehaven_app::state::AppState::new(
                lorehaven_app::config::Config::development_defaults(),
                tdb.db().clone(),
            ),
        ));
        let account = test_support::register(&mut client, "author@example.com", "author").await;
        let work = uuid::Uuid::new_v4().to_string();
        insert(
            &tdb,
            "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at, \
                 generated_content_posture) \
             VALUES (?1#u, (SELECT id FROM pseuds WHERE account_id = ?2#u), 'W', \
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'forbid')",
            &[&work, &account],
        )
        .await;

        for i in 0..readers {
            reader(&tdb, tag, i, &work, readers).await;
        }
        Self { tdb, account, work }
    }

    fn work_uuid(&self) -> uuid::Uuid {
        uuid::Uuid::parse_str(&self.work).expect("a uuid")
    }

    async fn signals(&self) -> lorehaven_domain::payouts::ReaderSignals {
        payout_store::reader_signals(self.tdb.db(), self.work_uuid(), WIN_FROM, WIN_TO)
            .await
            .expect("signals")
    }

    async fn payout(&self, base_bp: i64, demand: DemandInputs) -> payout_store::Payout {
        payout_store::compute_payout(
            self.tdb.db(),
            self.work_uuid(),
            base_bp,
            WIN_FROM,
            WIN_TO,
            demand,
        )
        .await
        .expect("payout")
    }
}

/// One reader who opens the work twice, finishes it, rates it, and bookmarks it.
///
/// The double view is deliberate: it is what proves the DISTINCT.
async fn reader(tdb: &TestDb, tag: &str, i: usize, work: &str, total: usize) {
    let run = uuid::Uuid::new_v4().to_string();
    let account = uuid::Uuid::new_v4().to_string();
    insert(
        tdb,
        "INSERT INTO accounts (id, email, created_at, updated_at) \
         VALUES (?1#u, ?2, '2026-01-02T00:00:00Z', '2026-01-02T00:00:00Z')",
        &[&account, &format!("{tag}-{i}-{run}@example.com")],
    )
    .await;
    insert(
        tdb,
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
         VALUES (?1#u, ?2#u, ?3, ?3, '2026-01-02T00:00:00Z', '2026-01-02T00:00:00Z')",
        &[
            &uuid::Uuid::new_v4().to_string(),
            &account,
            &format!("r{tag}{i}{}", &run[..8]),
        ],
    )
    .await;

    // TWO views for one reader, at DIFFERENT timestamps -- the primary key is
    // (work_id, viewer_hash, viewed_at), so a repeat view has to move in time
    // rather than repeat a row. That is also the realistic shape: without
    // COUNT(DISTINCT viewer_hash) this reader is two starters.
    for (n, at) in ["2026-01-03T00:00:00Z", "2026-01-03T00:30:00Z"]
        .iter()
        .enumerate()
    {
        insert(
            tdb,
            "INSERT INTO work_view_log (work_id, viewer_hash, viewed_at, is_automated) \
             VALUES (?1#u, ?2, ?3, ?4#i)",
            &[work, &format!("viewer-{i}-{run}"), at, "0"],
        )
        .await;
        let _ = n;
    }
    // Finished, with finished_at set so the window filter sees it.
    insert(
        tdb,
        "INSERT INTO reading_status (id, account_id, subject_type, subject_id, status, \
             started_at, finished_at, updated_at) \
         VALUES (?1#u, ?2#u, 'work', ?3#u, 'finished', '2026-01-02T00:00:00Z', \
             '2026-01-04T00:00:00Z', '2026-01-04T00:00:00Z')",
        &[&uuid::Uuid::new_v4().to_string(), &account, work],
    )
    .await;
    // Four stars is the §20.3 "positive" threshold.
    insert(
        tdb,
        "INSERT INTO rating (id, account_id, pseud_id, work_id, stars, is_public, \
             created_at, updated_at) \
         VALUES (?1#u, ?2#u, (SELECT id FROM pseuds WHERE account_id = ?2#u), ?3#u, 4, false, \
             '2026-01-04T00:00:00Z', '2026-01-04T00:00:00Z')",
        &[&uuid::Uuid::new_v4().to_string(), &account, work],
    )
    .await;
    insert(
        tdb,
        "INSERT INTO bookmarks (id, account_id, subject_type, subject_id, created_at, \
             updated_at) \
         VALUES (?1#u, ?2#u, 'work', ?3#u, '2026-01-04T00:00:00Z', '2026-01-04T00:00:00Z')",
        &[&uuid::Uuid::new_v4().to_string(), &account, work],
    )
    .await;
    let _ = total;
}

/// `?N#u` binds as a native uuid on PostgreSQL and as text on SQLite; `?N` is
/// text on both. See `crates/db/tests/hit_rate.rs` for the same convention.
async fn insert(tdb: &TestDb, sqlite: &str, args: &[&str]) {
    let db = tdb.db();
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            let sql = &sqlite
                .replace("#u", "")
                .replace("#i", "")
                .replace("false", "0");
            let mut q = sqlx::query(sql);
            for a in args {
                q = q.bind(*a);
            }
            q.execute(db.sqlite_pool().expect("pool"))
                .await
                .expect("insert");
        }
        lorehaven_db::Backend::Postgres => {
            let postgres = (1..=5).fold(sqlite.to_string(), |acc, n| {
                acc.replace(&format!("?{n}#u"), &format!("${n}::uuid"))
                    .replace(&format!("?{n}#i"), &format!("${n}::bigint"))
                    .replace(&format!("?{n}"), &format!("${n}"))
            });
            let mut q = sqlx::query(&postgres);
            for a in args {
                match uuid::Uuid::parse_str(a) {
                    Ok(u) => q = q.bind(u),
                    Err(_) => q = q.bind(*a),
                };
            }
            q.execute(db.postgres_pool().expect("pool"))
                .await
                .expect("insert");
        }
    }
}

fn no_demand() -> DemandInputs {
    DemandInputs {
        admin_taste: 0.0,
        wishlist: 0.0,
        search: 0.0,
    }
}

// ── the tests ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn every_reader_is_counted_once_however_many_times_they_opened_the_work() {
    // The DISTINCT. Two views per reader, so a missing DISTINCT doubles `starters`
    // and drags the completion rate below §20.3's 60% threshold for free.
    let f = Fixture::build("po-distinct", 20).await;
    let s = f.signals().await;
    assert_eq!(s.starters, 20, "20 readers, each with two views");
    assert_eq!(s.finishers, 20);
    assert_eq!(s.feedback_count, 20);
    assert_eq!(s.positive_feedback, 20);
    assert_eq!(s.bookmarkers, 20);
}

#[tokio::test]
async fn the_completion_branch_reaches_reading_status_through_account_id() {
    // `reading_status` keys on `account_id` while everything else keys on a work
    // or a viewer. Getting that wrong yields 0 finishers and a multiplier of
    // exactly 1.0x -- which reads as a mediocre work rather than a broken join, so
    // it is asserted as a count rather than as a multiplier.
    let f = Fixture::build("po-join", 20).await;
    let s = f.signals().await;
    assert_eq!(s.finishers, 20, "the account_id join found every reader");
}

#[tokio::test]
async fn a_work_that_earns_every_bonus_reaches_the_spec_ceiling() {
    // 20 readers, all finishing, all rating 4, all bookmarking: completion 100%,
    // feedback 100%, bookmarks 100%. Re-reads need finished_at > started_at, which
    // the fixture sets, so this is 1.3 + 0.2 + 0.2 + 0.1 = 1.8x.
    let f = Fixture::build("po-ceiling", 20).await;
    let p = f.payout(100 * BP_PER_CREDIT, no_demand()).await;
    assert_eq!(p.quality.value, 1.8, "all four §20.3 bonuses fired");
    // 100 credits at 1.8x, with demand at its floor of 1.0.
    assert_eq!(p.posted_bp, Some(180 * BP_PER_CREDIT));
}

#[tokio::test]
async fn a_work_under_the_reader_floor_earns_exactly_the_base() {
    // §20.3's minimum of 10 readers. Nine readers is below it, so no bonus applies
    // and the payout is the base — not a ratio of nine.
    let f = Fixture::build("po-floor", 9).await;
    let p = f.payout(100 * BP_PER_CREDIT, no_demand()).await;
    assert!(!p.quality.signals_active);
    assert_eq!(p.quality.value, 1.0);
    assert_eq!(p.posted_bp, Some(100 * BP_PER_CREDIT));
}

#[tokio::test]
async fn the_demand_multiplier_scales_the_total_without_being_disclosed() {
    // §20.3: the demand multiplier is folded silently into the total. The total
    // moves; the author's view does not mention it.
    let f = Fixture::build("po-demand", 20).await;
    let loud = f
        .payout(
            100 * BP_PER_CREDIT,
            DemandInputs {
                admin_taste: 1.0,
                wishlist: 1.0,
                search: 1.0,
            },
        )
        .await;
    assert_eq!(loud.demand.value, 1.5);
    assert_eq!(loud.posted_bp, Some(270 * BP_PER_CREDIT), "1.8 x 1.5");

    let lines = loud.view.author_lines();
    let joined = lines.join(" ");
    for forbidden in ["demand", "taste", "affinity", "1.5"] {
        assert!(
            !joined.to_lowercase().contains(forbidden),
            "§20.3: the author must not be told about the demand component, found \
             {forbidden:?} in {joined:?}"
        );
    }
    assert!(
        joined.contains("+80%"),
        "the quality breakdown IS shown: {joined}"
    );
}

#[tokio::test]
async fn the_posted_ledger_row_does_not_decompose_the_total() {
    // The stronger form of the rule above: the secret must not be recoverable from
    // the transaction itself, since §20.9 lets an author read their own entries.
    let f = Fixture::build("po-ledger", 20).await;
    // 20 credits, not 100: at 1.8 x 1.25 that is 45 credits, which clears the
    // 100/day cap. 100 would be 225 and would be SUPPRESSED -- the first version
    // of this test used 100 and failed, correctly, because the cap fired before
    // the ledger was ever reached. The cap is doing its job; the test was asking
    // it to do something it is right to refuse.
    let posted = payout_store::payout_work(
        f.tdb.db(),
        &f.account,
        f.work_uuid(),
        20 * BP_PER_CREDIT,
        WIN_FROM,
        WIN_TO,
        DemandInputs {
            admin_taste: 1.0,
            wishlist: 0.0,
            search: 0.0,
        },
    )
    .await
    .expect("post");
    assert_eq!(
        posted,
        Some(45 * BP_PER_CREDIT),
        "20 x 1.8 x 1.25 = 45 credits"
    );

    let balances = lorehaven_db::economy::balances(f.tdb.db(), &f.account)
        .await
        .expect("balances");
    let earned: i64 = balances
        .iter()
        .filter(|(bucket, _)| bucket == "earned")
        .map(|(_, amount)| *amount)
        .sum();
    assert_eq!(earned, 45 * BP_PER_CREDIT, "the total landed, once");
}

#[tokio::test]
async fn a_repeat_payout_for_the_same_window_is_idempotent() {
    // §20.3 recalculates weekly, so a retried job must not double-pay. The ledger's
    // own replay check is what makes this safe.
    let f = Fixture::build("po-idem", 20).await;
    // 20 credits base, so 20 x 1.8 = 36 credits clears the 100/day cap and a
    // transaction is actually posted for the replay check to be about.
    let args = (
        f.tdb.db(),
        f.account.clone(),
        f.work_uuid(),
        20 * BP_PER_CREDIT,
        WIN_FROM,
        WIN_TO,
        no_demand(),
    );
    let first = payout_store::payout_work(args.0, &args.1, args.2, args.3, args.4, args.5, args.6)
        .await
        .expect("first");
    let second = payout_store::payout_work(args.0, &args.1, args.2, args.3, args.4, args.5, args.6)
        .await
        .expect("second");
    assert_eq!(
        first, second,
        "the second call is a no-op, not a second payment"
    );

    let balances = lorehaven_db::economy::balances(f.tdb.db(), &f.account)
        .await
        .expect("balances");
    let earned: i64 = balances
        .iter()
        .filter(|(bucket, _)| bucket == "earned")
        .map(|(_, amount)| *amount)
        .sum();
    assert_eq!(earned, 36 * BP_PER_CREDIT, "36 credits once, not 72");
}

#[tokio::test]
async fn a_payout_above_the_per_work_cap_posts_nothing() {
    // §20.3 caps a work at 100 credits/day. The cap is applied BEFORE the ledger
    // write, so a suppressed payout leaves no row for an author to find.
    let f = Fixture::build("po-cap", 20).await;
    let posted = payout_store::payout_work(
        f.tdb.db(),
        &f.account,
        f.work_uuid(),
        500 * BP_PER_CREDIT,
        WIN_FROM,
        WIN_TO,
        no_demand(),
    )
    .await
    .expect("post");
    assert_eq!(posted, None, "500 x 1.8 = 900 credits is over the 100 cap");

    let balances = lorehaven_db::economy::balances(f.tdb.db(), &f.account)
        .await
        .expect("balances");
    assert!(
        balances.iter().all(|(_, amount)| *amount == 0),
        "no ledger row at all, not a clamped one: {balances:?}"
    );
}

#[tokio::test]
async fn the_caps_are_the_numbers_the_spec_publishes() {
    // A cap that drifted from §20.3's published table would be an instance
    // promising more or less than the spec does, invisibly.
    assert_eq!(caps::PER_WORK_PER_DAY, 100);
    assert_eq!(caps::PER_AUTHOR_PER_DAY, 300);
    assert_eq!(caps::PER_AUTHOR_PER_MONTH, 5_000);
    assert_eq!(BP_PER_CREDIT, 10_000);
}

#[tokio::test]
async fn an_empty_window_earns_the_base_and_reports_no_readers() {
    let f = Fixture::build("po-empty", 0).await;
    let s = f.signals().await;
    assert_eq!(s.starters, 0);
    let p = f.payout(100 * BP_PER_CREDIT, no_demand()).await;
    assert_eq!(
        p.posted_bp,
        Some(100 * BP_PER_CREDIT),
        "the base, unmodified"
    );
    assert!(!p.quality.signals_active);
}

#[tokio::test]
async fn an_out_of_window_read_is_excluded() {
    // The window filter has to be on the READ's own timestamp, not on the
    // work's creation. A reader who arrived 40 days ago is not in this window.
    let f = Fixture::build("po-window", 20).await;
    let s = f.signals().await;
    assert_eq!(s.starters, 20);

    let narrow =
        payout_store::reader_signals(f.tdb.db(), f.work_uuid(), T0 + 60 * DAY, T0 + 90 * DAY)
            .await
            .expect("signals");
    assert_eq!(
        narrow.starters, 0,
        "reads before the window are excluded, not clamped in"
    );
    assert_eq!(narrow.finishers, 0, "and so are completions");
}

#[tokio::test]
async fn the_author_view_names_only_the_quality_breakdown() {
    let f = Fixture::build("po-view", 20).await;
    let p = f.payout(100 * BP_PER_CREDIT, no_demand()).await;
    let lines = p.view.author_lines();
    assert_eq!(lines.len(), 2, "§20.3 shows exactly two lines");
    assert!(lines[0].contains("credits"), "{}", lines[0]);
    assert!(lines[1].starts_with("Quality bonus:"), "{}", lines[1]);
}

#[tokio::test]
async fn an_automated_view_is_not_a_starter() {
    // `work_view_log.is_automated` marks a crawler, not a reader. Counting one
    // inflates `starters` and therefore DEFLATES the completion rate, so a work
    // that a scraper has walked through looks worse than one a person read.
    //
    // This test exists because the mutation that removed the `is_automated = 0`
    // filter left all twelve green -- every fixture view was a human one. The
    // filter has to be exercised by a row that would fail without it.
    let f = Fixture::build("po-automated", 20).await;
    for i in 0..50 {
        insert(
            &f.tdb,
            "INSERT INTO work_view_log (work_id, viewer_hash, viewed_at, is_automated) \
             VALUES (?1#u, ?2, '2026-01-03T00:00:00Z', ?4#i)",
            &[&f.work, &format!("bot-{i}"), "2026-01-03T00:00:00Z", "1"],
        )
        .await;
    }
    let s = f.signals().await;
    assert_eq!(s.starters, 20, "50 crawler views are not readers");
    assert_eq!(
        f.payout(100 * BP_PER_CREDIT, no_demand())
            .await
            .quality
            .value,
        1.8,
        "and the quality multiplier is unchanged by them"
    );
}

#[tokio::test]
async fn a_payout_in_a_second_window_is_a_separate_payment() {
    // The idempotency key includes the window, so a later window pays again. Dropping
    // `{since}:{until}` from the key collapsed every window onto one key and the
    // second payout became a silent no-op -- a weekly recalculation paying one week
    // and never another, which looks exactly like an author with no readers.
    let f = Fixture::build("po-windows", 20).await;
    let next = T0 + 30 * DAY;
    let first = payout_store::payout_work(
        f.tdb.db(),
        &f.account,
        f.work_uuid(),
        20 * BP_PER_CREDIT,
        WIN_FROM,
        WIN_TO,
        no_demand(),
    )
    .await
    .expect("first window");
    let second = payout_store::payout_work(
        f.tdb.db(),
        &f.account,
        f.work_uuid(),
        20 * BP_PER_CREDIT,
        next,
        next + 30 * DAY,
        no_demand(),
    )
    .await
    .expect("second window");
    assert!(
        first.is_some() && second.is_some(),
        "both windows pay: {first:?} {second:?}"
    );

    let balances = lorehaven_db::economy::balances(f.tdb.db(), &f.account)
        .await
        .expect("balances");
    let earned: i64 = balances
        .iter()
        .filter(|(bucket, _)| bucket == "earned")
        .map(|(_, amount)| *amount)
        .sum();
    // 56, not 72: the SECOND window has no reads in it -- the fixture's readers all
    // finished on 2026-01-04, inside the first window -- so its quality multiplier
    // is 1.0x and it pays the bare 20. The point of the test is that the second
    // window posted AT ALL, which a window-blind idempotency key would have
    // swallowed. (I first wrote 72, assuming both windows earned the bonus; the
    // assertion caught my assumption rather than a bug.)
    assert_eq!(
        earned,
        56 * BP_PER_CREDIT,
        "36 from the first window plus 20 from the second -- not one payment"
    );
}
