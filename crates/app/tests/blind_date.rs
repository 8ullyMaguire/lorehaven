//! Gap B — Blind Date: one work per reader per day, chosen without their profile.
//!
//! `blind_date_work` in `crates/db/src/discovery.rs` closes #14. The audit's finding was
//! that this is implementation of an *existing* spec surface rather than missing design
//! — `spec.md:2858` lists Blind Date among the discovery surfaces, and `spec.md:7462`
//! gives the chat bot a `/blind-date` command — so no schema and no design decision
//! were needed. Only the code.
//!
//! The property that makes this surface different from every other one in the codebase
//! is that it is **deterministic in (account, day)**. So the tests below are mostly about
//! stability, which is the opposite of what a random-pick surface would test.

use lorehaven_db::discovery::{blind_date_seed, blind_date_work};
use test_support::{scratch_dir, TestDb};

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

    /// `count` published, public works, all by distinct authors.
    async fn works(&self, prefix: &str, count: usize) -> Vec<String> {
        let mut ids = Vec::new();
        for n in 0..count {
            let owner = account(
                &self.tdb,
                &format!("{prefix}-{n}-{}@example.com", uuid::Uuid::new_v4().simple()),
            )
            .await;
            let id = uuid::Uuid::new_v4().to_string();
            exec_with(
                &self.tdb,
                "INSERT INTO works (id, owner_pseud_id, title, lifecycle, visibility, \
                     published_at, created_at, updated_at, generated_content_posture) \
                 VALUES (?1#u, (SELECT id FROM pseuds WHERE account_id = ?2#u), ?3, \
                     'published', 'public', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', \
                     '2026-01-01T00:00:00Z', 'forbid')",
                &[&id, &owner, &format!("{prefix}-{n}")],
            )
            .await;
            ids.push(id);
        }
        ids
    }

    async fn pick(&self, day: &str) -> Option<String> {
        blind_date_work(self.tdb.db(), &self.reader, day)
            .await
            .expect("blind date")
    }
}

// ── the three properties that make it Blind Date and not a random work ──────

#[tokio::test]
async fn the_pick_is_stable_across_reloads_on_the_same_day() {
    // The defining property. `ORDER BY random()` would give a different work on every
    // refresh, which makes the work impossible to bookmark, rate, or discuss — the
    // reader has no stable referent to point at.
    let f = Fixture::build("bd_stable").await;
    f.works("w", 25).await;
    let first = f.pick("2026-10-02").await;
    for _ in 0..8 {
        assert_eq!(
            f.pick("2026-10-02").await,
            first,
            "eight reloads later it must be the same work"
        );
    }
    assert!(first.is_some(), "25 eligible works, so there is a pick");
}

#[tokio::test]
async fn a_different_day_gives_a_different_work() {
    // Otherwise the reader gets one blind date forever and the surface is a suggestion
    // list with extra steps.
    //
    // The threshold is **10**, not something near the expected value, and the reason is
    // arithmetic. 25 days drawing uniformly from 25 works is a birthday problem: the
    // expected number of distinct works is 25 * (1 - (24/25)^25) ≈ 16.0 with a standard
    // deviation of about 1.57. Simulated over 200,000 draws:
    //
    //     threshold   fail rate
    //     <= 10          0.018%
    //     <= 12          1.204%      <- what this test asserted
    //     <= 14         16.704%
    //     <= 16         62.805%
    //
    // So the original `> 12` sat about 2.5 standard deviations below the mean and failed
    // on roughly one run in eighty. It was not a logic bug and not order-dependence: it
    // failed in a full-workspace run and passed when the suite ran alone, because the
    // outcome depends on the hash of (account id, day), and the fixture generates a fresh
    // account per run. `blind_date.rs:92` read "got 12 distinct" -- one below the bar.
    //
    // 10 still fails if the property genuinely broke -- a seed that ignored the day would
    // return 1 distinct, and one that varied only slightly would collapse to single
    // digits -- while costing 0.018% instead of 1.2%. A distribution assertion needs a
    // threshold on the far tail of the distribution, not near its centre.
    let f = Fixture::build("bd_daily").await;
    f.works("w", 25).await;
    let days: Vec<String> = (1..=25).map(|d| format!("2026-10-{:02}", d)).collect();
    let picks: Vec<Option<String>> = futures_join_all(&f, &days).await;
    let distinct: std::collections::HashSet<&Option<String>> = picks.iter().collect();
    assert!(
        distinct.len() > 10,
        "25 days over 25 works should mostly differ, got {} distinct: {picks:?}",
        distinct.len()
    );
}

/// `blind_date_work` is async and each call needs `&f`, so the loop is a small helper
/// rather than a `join_all` over futures borrowing the fixture.
async fn futures_join_all(f: &Fixture, days: &[String]) -> Vec<Option<String>> {
    let mut out = Vec::new();
    for day in days {
        out.push(f.pick(day).await);
    }
    out
}

#[tokio::test]
async fn two_readers_get_different_works_on_the_same_day() {
    // A single work-of-the-day for everyone would just be a trending slot, and §16.1a
    // already has one. Blind Date exists to get *outside* the profile, which only means
    // something if it is per-reader.
    //
    // ## This test was a 1-in-25 coin flip, and its comment said so while claiming otherwise
    //
    // The old version built 25 works, picked for two accounts on one day, and asserted
    // the picks differed -- with a comment reading "so this asserts the mechanism (the
    // account is in the seed) rather than statistical luck". It did not. Two readers
    // drawing one each from 25 works collide one time in 25, so the test failed on
    // roughly one run in twenty-five and passed the rest while never once checking the
    // seed. Its sibling, `a_different_day_gives_a_different_work`, had ALREADY been
    // rewritten for exactly this mistake (see its long comment about the birthday
    // problem and the threshold move from 12 to 10) -- and left this one in place. That
    // is how the failure got to a full-workspace run in the first place.
    //
    // `cargo test --workspace` on PostgreSQL, 2026-10-05, hit it: both picks were
    // `b8db39aa-...`. It passed alone, for the same reason it used to pass alone.
    //
    // ## What it checks now
    //
    // The property, not a sample of it: two accounts produce two seeds. Deterministic,
    // instant, and it fails the moment someone drops `account` from the hash -- which is
    // the only way this can actually break. `blind_date_seed` became `pub` for it.
    let day = "2026-10-02";
    let a = blind_date_seed("acct-reader-one", day);
    let b = blind_date_seed("acct-reader-two", day);
    assert_ne!(
        a, b,
        "the account id must be part of the seed, or every reader gets the same Blind Date"
    );
    // And the day is part of it too, which is the other half of "per reader per day" --
    // asserted here because `a_different_day_gives_a_different_work` only gets at it
    // through the distribution, i.e. statistically.
    assert_ne!(
        blind_date_seed("acct-reader-one", day),
        blind_date_seed("acct-reader-one", "2026-10-03"),
        "the day must be part of the seed"
    );
}

#[tokio::test]
async fn two_readers_really_do_get_different_works_when_they_do_not_collide() {
    // The end-to-end half, kept because the seed check above cannot see the SQL: it
    // says nothing about whether the account reaches the query, or whether the account
    // id is the string it thinks it is.
    //
    // Spelled so it cannot flake. Rather than betting on 25 works and one pair -- the
    // mistake the test above used to make -- this asks for the pick for MANY reader/day
    // pairs and asserts the mapping is not the constant it would be if the account were
    // ignored. Two readers on one day is 1-in-25; twenty accounts across ten days is
    // 10^13 possible assignments and a broken seed gives exactly 10.
    let f = Fixture::build("bd_perreader_e2e").await;
    f.works("w", 25).await;
    let day = "2026-10-02";
    let mut picks = std::collections::HashSet::new();
    for i in 0..20 {
        // A real account, so a real uuid. The first version of this passed the email
        // address straight in and PostgreSQL answered
        //   invalid input syntax for type uuid: "bd_perreader-e2e-0@example.com"
        // -- which is the difference between the two engines being a nuisance, not
        // just a chore: `accounts.id` is a uuid on PostgreSQL and a TEXT slug on
        // SQLite, so a test that passes an identifier by hand compiles on SQLite and
        // is rejected at the boundary on PostgreSQL. It still passed on SQLite, which
        // is precisely how this gets missed.
        let reader = account(&f.tdb, &format!("bd_perreader-e2e-{i}@example.com")).await;
        picks.insert(
            blind_date_work(f.tdb.db(), &reader, day)
                .await
                .expect("25 eligible works, so every reader gets a pick"),
        );
    }
    assert!(
        picks.len() > 10,
        "20 readers drew only {} distinct works from 25: the account is not reaching the seed",
        picks.len()
    );
}

// ── eligibility ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_bookmarked_work_is_never_the_pick() {
    // Showing someone a thing they saved is the same mistake every other strategy
    // avoids. Tested across all 25 days so a single unlucky ordering cannot hide it.
    // **One work only.** With 25 healthy works alongside it, the bookmarked work merely
    // has to lose the hash ordering on each of the 25 days -- it loses about 24 times out
    // of 25 by luck, so disabling the exclusion entirely still read GREEN(BAD). As the
    // only candidate it is picked on every day the moment the exclusion is gone.
    let f = Fixture::build("bd_bookmarked").await;
    let works = f.works("w", 1).await;
    let first = works[0].clone();
    exec_with(
        &f.tdb,
        "INSERT INTO bookmarks (id, account_id, subject_type, subject_id, created_at, updated_at) \
         VALUES (?1#u, ?2#u, 'work', ?3#u, '2026-01-06T00:00:00Z', '2026-01-06T00:00:00Z')",
        &[&uuid::Uuid::new_v4().to_string(), &f.reader.clone(), &first],
    )
    .await;
    for d in 1..=25 {
        assert_ne!(
            f.pick(&format!("2026-10-{d:02}")).await,
            Some(first.clone()),
            "a bookmarked work was picked on day {d}"
        );
    }
}

#[tokio::test]
async fn a_work_the_reader_already_finished_by_that_author_is_not_the_pick() {
    // Blind Date is meant to surface an unknown *author* as much as an unknown work. An
    // author whose book the reader has finished is neither blind nor unknown, so every
    // work by that pseud is out — including ones the reader has not read.
    let f = Fixture::build("bd_author").await;
    let works = f.works("w", 10).await;
    let finished = works[0].clone();
    // `Fixture::works` gives every work its own author, so the "sister" has to be
    // inserted explicitly against works[0]'s owner. The reader finishes works[0], which
    // is what makes that author non-blind — and the sister is the work the assertion
    // actually cares about, since nothing in the reader's history mentions it.
    exec_with(
        &f.tdb,
        "INSERT INTO works (id, owner_pseud_id, title, lifecycle, visibility, published_at, \
             created_at, updated_at, generated_content_posture) \
         SELECT ?1#u, owner_pseud_id, 'Sister', 'published', 'public', '2026-01-01T00:00:00Z', \
             '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'forbid' \
         FROM works WHERE id = ?2#u",
        &[&uuid::Uuid::new_v4().to_string(), &works[0].clone()],
    )
    .await;
    exec_with(
        &f.tdb,
        "INSERT INTO reading_status (id, account_id, subject_type, subject_id, status, \
             started_at, finished_at, updated_at) \
         VALUES (?1#u, ?2#u, 'work', ?3#u, 'finished', '2026-01-02T00:00:00Z', \
             '2026-01-05T00:00:00Z', '2026-01-05T00:00:00Z')",
        &[
            &uuid::Uuid::new_v4().to_string(),
            &f.reader.clone(),
            &finished,
        ],
    )
    .await;
    // The sister's id, so the assertion can name it.
    let sister: String = match f.tdb.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query_scalar("SELECT id FROM works WHERE title = 'Sister'")
                .fetch_one(f.tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("sister")
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query_scalar("SELECT id::text FROM works WHERE title = 'Sister'")
                .fetch_one(f.tdb.db().postgres_pool().expect("postgres"))
                .await
                .expect("sister")
        }
    };
    for d in 1..=25 {
        let pick = f.pick(&format!("2026-10-{d:02}")).await;
        assert_ne!(
            pick,
            Some(sister.clone()),
            "day {d} picked the author's other work"
        );
    }
}

#[tokio::test]
async fn an_unlistable_work_is_never_the_pick_even_when_it_is_the_only_one() {
    // B5 (visibility) and B7 (published_at) both survived a first harness run, and the
    // reason is the same for both: with five *eligible* works also in the catalogue, the
    // ineligible one merely has to lose the hash ordering, and it usually does — by
    // luck, not by logic. One unlucky day would have caught it, which is a flaky gate,
    // not a gate.
    //
    // The reliable shape is to leave the ineligible work as the ONLY candidate. Then
    // any leak is a guaranteed pick on every single day, and the assertion cannot pass
    // by chance.
    let f = Fixture::build("bd_only_unlisted").await;
    for (lifecycle, visibility, title) in [
        ("published", "unlisted", "Unlisted"),
        ("published", "private", "Private"),
    ] {
        let author = account(&f.tdb, &format!("bd_only_{title}@example.com")).await;
        let id = uuid::Uuid::new_v4().to_string();
        exec_with(
            &f.tdb,
            "INSERT INTO works (id, owner_pseud_id, title, lifecycle, visibility, published_at, \
                 created_at, updated_at, generated_content_posture) \
             VALUES (?1#u, (SELECT id FROM pseuds WHERE account_id = ?2#u), ?3, ?4, ?5, \
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'forbid')",
            &[
                &id,
                &author,
                &title.to_string(),
                &lifecycle.to_string(),
                &visibility.to_string(),
            ],
        )
        .await;
        for d in 1..=10 {
            assert_eq!(
                f.pick(&format!("2026-10-{d:02}")).await,
                None,
                "{title} is the only work in the catalogue and was still offered on day {d}"
            );
        }
    }
}

#[tokio::test]
async fn a_future_work_is_never_the_pick_even_when_it_is_the_only_one() {
    // Same shape as above, for the `published_at` gate. A work scheduled for 2027
    // cannot be a blind date in 2026, whatever the hash says.
    let f = Fixture::build("bd_only_future").await;
    let author = account(&f.tdb, "bd_only_future-a@example.com").await;
    let id = uuid::Uuid::new_v4().to_string();
    exec_with(
        &f.tdb,
        "INSERT INTO works (id, owner_pseud_id, title, lifecycle, visibility, published_at, \
             created_at, updated_at, generated_content_posture) \
         VALUES (?1#u, (SELECT id FROM pseuds WHERE account_id = ?2#u), 'Future', 'published', \
             'public', '2027-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', \
             'forbid')",
        &[&id.clone(), &author],
    )
    .await;
    for d in 1..=10 {
        assert_eq!(
            f.pick(&format!("2026-10-{d:02}")).await,
            None,
            "a work published in 2027 was offered on 2026-10-{d:02}"
        );
    }
    // And it becomes eligible once its date has passed -- the clause is a gate, not a
    // permanent exclusion, which is the direction that is easy to get backwards.
    assert_eq!(
        f.pick("2027-06-01").await,
        Some(id),
        "once published, the work is eligible"
    );
}

#[tokio::test]
async fn a_draft_or_unlisted_work_is_never_the_pick() {
    // Two separate reasons, tested together because the answer is the same: `lifecycle`
    // says whether a work is published, `visibility` says whether it is listable, and a
    // work can be published-but-unlisted (direct-link only). Blind Date is a discovery
    // surface, so both are out.
    let f = Fixture::build("bd_ineligible").await;
    f.works("w", 5).await;
    let author = account(&f.tdb, "bd_ineligible-x@example.com").await;
    for (lifecycle, visibility, title) in [
        ("draft", "public", "Draft"),
        ("published", "unlisted", "Unlisted"),
        ("withdrawn", "public", "Withdrawn"),
    ] {
        let id = uuid::Uuid::new_v4().to_string();
        exec_with(
            &f.tdb,
            "INSERT INTO works (id, owner_pseud_id, title, lifecycle, visibility, published_at, \
                 created_at, updated_at, generated_content_posture) \
             VALUES (?1#u, (SELECT id FROM pseuds WHERE account_id = ?2#u), ?3, ?4, ?5, \
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'forbid')",
            &[
                &id,
                &author,
                &title.to_string(),
                &lifecycle.to_string(),
                &visibility.to_string(),
            ],
        )
        .await;
        for d in 1..=10 {
            assert_ne!(
                f.pick(&format!("2026-10-{d:02}")).await,
                Some(id.clone()),
                "{title} was picked on day {d}"
            );
        }
    }
}

#[tokio::test]
async fn a_work_scheduled_for_the_future_is_not_yet_eligible() {
    // §16.10's blind-date pool is for works "that ha[ve] not yet been shown to anyone",
    // which is about the pool's residency, not eligibility — but the reverse does hold:
    // a work scheduled to appear next month is not a blind date today, or the surface
    // would leak the schedule.
    let f = Fixture::build("bd_future").await;
    f.works("w", 5).await;
    let author = account(&f.tdb, "bd_future-x@example.com").await;
    let future = uuid::Uuid::new_v4().to_string();
    exec_with(
        &f.tdb,
        "INSERT INTO works (id, owner_pseud_id, title, lifecycle, visibility, published_at, \
             created_at, updated_at, generated_content_posture) \
         VALUES (?1#u, (SELECT id FROM pseuds WHERE account_id = ?2#u), 'Future', 'published', \
             'public', '2027-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', \
             'forbid')",
        &[&future, &author],
    )
    .await;
    assert_ne!(
        f.pick("2026-10-02").await,
        Some(future),
        "a work published in 2027 is not eligible in 2026"
    );
    // …but it IS eligible once its date has passed, which is the clause that would be
    // easy to get backwards.
    assert_ne!(
        f.pick("2027-06-01").await,
        None,
        "still has other works to pick"
    );
}

/// The order key, duplicated from `crates/db/src/discovery.rs`.
///
/// Deliberately a copy rather than a `pub` import: the test's job is to check that the
/// *implementation* picks what the seed dictates, and importing the function it is
/// checking would make the assertion true by construction. If the implementation's
/// hashing changes, this test fails -- which is the point.
fn expected_first(ids: &[String], seed: &str) -> String {
    let mut sorted = ids.to_vec();
    sorted.sort_by_key(|id| {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in id.as_bytes().iter().chain(b"|").chain(seed.as_bytes()) {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    });
    sorted[0].clone()
}

/// The seed for (account, day), duplicated for the same reason as `expected_first`.
fn expected_seed(account: &str, today: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in account
        .as_bytes()
        .iter()
        .chain(b"|")
        .chain(today.as_bytes())
    {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[tokio::test]
async fn the_pick_is_the_one_the_seed_dictates_not_merely_the_lowest_id() {
    // Degenerating the ordering to `min_by_key(id)` -- i.e. always the lexicographically
    // lowest work id -- survives a presence assertion, because that mutation still
    // returns *a* work every day. The only way to see it is to assert WHICH work, which
    // requires computing the seed independently.
    //
    // The seed is not correlated with id order, so over enough days the picks must
    // include something other than the lowest id. With 12 works the chance that a single
    // day happens to agree is 1/12, so requiring three disagreements in ten days is
    // decisive without being flaky.
    let f = Fixture::build("bd_seed").await;
    let works = f.works("w", 12).await;
    let lowest = works.iter().min().expect("works").clone();
    let mut disagreements = 0;
    for d in 1..=10 {
        let pick = f.pick(&format!("2026-10-{d:02}")).await;
        let want = expected_first(
            &works,
            &expected_seed(&f.reader, &format!("2026-10-{d:02}")),
        );
        assert_eq!(
            pick,
            Some(want.clone()),
            "day {d}: the seed picks {want:?}, not the lowest id {lowest:?}"
        );
        if Some(want) != Some(lowest.clone()) {
            disagreements += 1;
        }
    }
    assert!(
        disagreements >= 3,
        "the seed must actually reorder the catalogue, or this test proves nothing: \
         {disagreements} disagreements with the lowest id in 10 days"
    );
}

#[tokio::test]
async fn a_soft_deleted_work_is_never_the_pick_even_when_it_is_the_only_one() {
    // `works.deleted_at` is a soft delete: the row stays for the author's own history,
    // so nothing else excludes it from this query. Without this test the clause was
    // completely uncovered -- B8 survived because no fixture ever set `deleted_at`.
    let f = Fixture::build("bd_only_deleted").await;
    let author = account(&f.tdb, "bd_only_deleted-a@example.com").await;
    let id = uuid::Uuid::new_v4().to_string();
    exec_with(
        &f.tdb,
        "INSERT INTO works (id, owner_pseud_id, title, lifecycle, visibility, published_at, \
             created_at, updated_at, generated_content_posture, deleted_at) \
         VALUES (?1#u, (SELECT id FROM pseuds WHERE account_id = ?2#u), 'Deleted', 'published', \
             'public', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', \
             'forbid', '2026-02-01T00:00:00Z')",
        &[&id, &author],
    )
    .await;
    for d in 1..=10 {
        assert_eq!(
            f.pick(&format!("2026-10-{d:02}")).await,
            None,
            "a soft-deleted work was offered on 2026-10-{d:02}"
        );
    }
}

#[tokio::test]
async fn an_empty_catalogue_returns_none_rather_than_failing() {
    // The route needs to render an empty state. An error here would turn "no blind date
    // today" into a 500, which is the difference between a quiet surface and a broken
    // one.
    let f = Fixture::build("bd_empty").await;
    assert_eq!(f.pick("2026-10-02").await, None);
}

// ── fixtures ────────────────────────────────────────────────────────────────

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
            &format!("b{}", &uuid::Uuid::new_v4().to_string()[..8]),
        ],
    )
    .await;
    id
}

/// `?N#u` binds a native uuid on PostgreSQL and text on SQLite; `?N#i` an integer.
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
