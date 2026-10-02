//! §16.1 — series-aware recommendation, green on SQLite and PostgreSQL.
//!
//! `crates/db/src/series_recs.rs` closes gap D on the ideas list (#21), which the
//! audit called "the cheapest real engine gap — the data is already there and the
//! recommendation is a query over it". It was true about the data and quietly wrong
//! about the rest: `media::collection_media` returns *media* records, not the works
//! in a collection, so nothing could walk a series in order.
//!
//! This file is about the queries where an obvious implementation returns a
//! plausible wrong answer. Each has a test that only the real rule can pass.
//!
//!   * "Next" must be `position + 1`, not "the next one you haven't read". A reader
//!     who skipped entry 2 is *not* to be offered entry 3: the author's numbering
//!     is the only ordering the platform knows.
//!   * The anchor must be the FURTHEST finished entry, not the first row that
//!     matched. Reading 1 then 3 must suggest 4, not 2.
//!   * A finished work must never be suggested, even when it is `position + 1`.
//!   * A series of one has no next entry, so a reader who finished it gets nothing —
//!     not the entry they just finished.
//!   * An *abandoned* entry (started, never finished) must still be suggested.

use lorehaven_db::series_recs::{next_entries, NextEntry, Origin};
use test_support::{scratch_dir, TestDb};

const T0: i64 = 1_767_225_600; // 2026-01-01
const DAY: i64 = 86_400;
const FROM: i64 = T0;
const TO: i64 = T0 + 30 * DAY;

/// A reader, plus a series of `entries` works owned by `owner`.
struct Fixture {
    tdb: TestDb,
    reader: String,
    owner: String,
    series: String,
    works: Vec<String>,
}

impl Fixture {
    /// `entries` works at positions 1..=n, owned by `owner`.
    async fn with_series(tag: &str, entries: usize, owner_is_reader: bool) -> Self {
        let dir = scratch_dir(tag);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        let reader = account(&tdb, &format!("{tag}-reader@example.com")).await;
        let owner = if owner_is_reader {
            reader.clone()
        } else {
            account(&tdb, &format!("{tag}-author@example.com")).await
        };

        let series = uuid::Uuid::new_v4().to_string();
        exec_with(
            &tdb,
            "INSERT INTO media_collections \
                 (id, collection_kind, owning_account_id, title, visibility, created_at, \
                  updated_at) \
             VALUES (?1#u, 'series', ?2#u, 'The Long Arc', 'public', '2026-01-01T00:00:00Z', \
                 '2026-01-01T00:00:00Z')",
            &[&series, &owner],
        )
        .await;

        let mut works = Vec::with_capacity(entries);
        for n in 1..=entries {
            let work = uuid::Uuid::new_v4().to_string();
            exec_with(
                &tdb,
                "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at, \
                     generated_content_posture) \
                 VALUES (?1#u, (SELECT id FROM pseuds WHERE account_id = ?2#u), ?3, \
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'forbid')",
                &[&work, &owner, &format!("Entry {n}")],
            )
            .await;
            exec_with(
                &tdb,
                "INSERT INTO media_collection_items (id, collection_id, work_id, position, \
                     added_at) \
                 VALUES (?1#u, ?2#u, ?3#u, ?4#i, '2026-01-01T00:00:00Z')",
                &[
                    &uuid::Uuid::new_v4().to_string(),
                    &series,
                    &work,
                    &n.to_string(),
                ],
            )
            .await;
            works.push(work);
        }
        Self {
            tdb,
            reader,
            owner,
            series,
            works,
        }
    }

    /// Record `finished` for the reader on the given entry indices.
    async fn finish(&self, positions: &[usize]) {
        for &p in positions {
            let work = self
                .works
                .get(p - 1)
                .unwrap_or_else(|| panic!("no entry at position {p}"))
                .clone();
            self.set_status(&work, "finished", Some("2026-01-05T00:00:00Z"))
                .await;
        }
    }

    /// Record `started` on an entry: engaged with, never finished.
    async fn start(&self, position: usize) {
        let work = self.works[position - 1].clone();
        self.set_status(&work, "reading", Some("2026-01-05T00:00:00Z"))
            .await;
    }

    async fn set_status(&self, work: &str, status: &str, finished_at: Option<&str>) {
        exec_with(
            &self.tdb,
            "INSERT INTO reading_status (id, account_id, subject_type, subject_id, status, \
                 started_at, finished_at, updated_at) \
             VALUES (?1#u, ?2#u, 'work', ?3#u, ?4, '2026-01-02T00:00:00Z', ?5, \
                 '2026-01-05T00:00:00Z')",
            &[
                &uuid::Uuid::new_v4().to_string(),
                &self.reader,
                &work.to_string(),
                &status.to_string(),
                &finished_at.unwrap_or("").to_string(),
            ],
        )
        .await;
    }

    async fn suggestions(&self) -> Vec<NextEntry> {
        next_entries(self.tdb.db(), &self.reader, FROM, TO)
            .await
            .expect("series suggestions")
    }
}

// ── the tests ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_next_entry_after_the_one_you_finished_is_suggested() {
    // The baseline: five entries, you finished the first, you get the second.
    let f = Fixture::with_series("sr_basic", 5, false).await;
    f.finish(&[1]).await;
    let got = f.suggestions().await;
    assert_eq!(got.len(), 1, "one series, one suggestion");
    assert_eq!(got[0].work_id, f.works[1], "position 2, not 1 and not 3");
    assert_eq!(got[0].position, 2);
    assert_eq!(got[0].after_work_id.as_deref(), Some(f.works[0].as_str()));
    assert_eq!(got[0].origin, Origin::NextAfterFinished);
    assert_eq!(got[0].collection_id, f.series, "and it names its series");
    assert_eq!(got[0].series_title, "The Long Arc");
    assert_eq!(got[0].entry_title, "Entry 2");
}

#[tokio::test]
async fn the_anchor_is_the_first_unfinished_entry_not_the_first_you_finished() {
    // Reading 1 then 3 of a five-part series leaves 2 and 4 unread, and the
    // suggestion is 2 -- the *first* unread, not 4.
    //
    // The first version of this test asserted the opposite ("position 4, after the
    // furthest"), which is the rule the original query implemented. The test was
    // wrong and so was the query: anchoring on `MAX(finished)` abandons every gap,
    // and a reader who skipped entry 2 is precisely the reader who most needs to be
    // told it exists.
    let f = Fixture::with_series("sr_first_unfinished", 5, false).await;
    f.finish(&[1, 3]).await;
    let got = f.suggestions().await;
    assert_eq!(got[0].work_id, f.works[1], "position 2, the gap");
    assert_eq!(got[0].position, 2);
    assert_eq!(
        got[0].after_position,
        Some(1),
        "and the reason line names entry 1, the nearest one before it"
    );
}

#[tokio::test]
async fn a_skipped_entry_is_still_the_suggestion() {
    // You read 1 and 2, skipped 3, and read 4. The next entry is 3, not 5: the
    // author's numbering is the only ordering the platform knows, and a "smart"
    // gap-filling order would silently contradict it.
    let f = Fixture::with_series("sr_skip", 5, false).await;
    f.finish(&[1, 2, 4]).await;
    let got = f.suggestions().await;
    assert_eq!(
        got[0].work_id, f.works[2],
        "the gap is filled before the tail is suggested"
    );
    assert_eq!(got[0].position, 3);
}

#[tokio::test]
async fn a_work_you_already_finished_is_never_suggested() {
    // Belt and braces against the rule above: even if a row says `position + 1`,
    // a finished entry is not a recommendation. Without the NOT EXISTS guard this
    // returns an entry the reader has demonstrably finished.
    let f = Fixture::with_series("sr_norepeat", 4, false).await;
    f.finish(&[1, 2]).await;
    // Re-finish 3 as well, out of order, so `position + 1` from the anchor (2) is 3.
    f.set_status(
        &f.works[2].clone(),
        "finished",
        Some("2026-01-06T00:00:00Z"),
    )
    .await;
    let got = f.suggestions().await;
    for entry in &got {
        assert_ne!(
            entry.work_id, f.works[2],
            "entry 3 is finished and must not be suggested"
        );
    }
}

#[tokio::test]
async fn a_one_entry_series_suggests_nothing() {
    // You finished the only entry. The next entry does not exist, so the answer is
    // nothing -- not the entry you just finished, which is the one outcome a feed
    // must never produce.
    let f = Fixture::with_series("sr_single", 1, false).await;
    f.finish(&[1]).await;
    assert!(
        f.suggestions().await.is_empty(),
        "a completed series offers nothing further"
    );
}

#[tokio::test]
async fn finishing_the_last_entry_but_two_still_suggests_the_first_unfinished() {
    // Finishing entry 3 of a 3-part series leaves 1 and 2 unread, so the answer is
    // 2 -- not "nothing", which is what the original `MAX(finished) + 1` rule
    // produced.
    let f = Fixture::with_series("sr_last", 3, false).await;
    f.finish(&[3]).await;
    let got = f.suggestions().await;
    assert_eq!(got.len(), 1);
    // Position 1, not position 2. I wrote `works[1]` expecting "the one before what
    // I finished", but the rule is the FIRST UNFINISHED entry and entry 1 has not
    // been read. The assertion caught my reasoning, not the query -- and it is the
    // same rule `a_skipped_entry_is_still_the_suggestion` already pins.
    assert_eq!(
        got[0].work_id, f.works[0],
        "entry 3 being finished does not mean entry 1 is"
    );
    assert_eq!(got[0].position, 1);
}

#[tokio::test]
async fn a_fully_finished_series_suggests_nothing() {
    // The case the "last entry has no successor" test was *meant* to cover: every
    // entry finished, so there is no candidate and no cold start either.
    let f = Fixture::with_series("sr_complete", 3, false).await;
    f.finish(&[1, 2, 3]).await;
    assert!(
        f.suggestions().await.is_empty(),
        "nothing left to read in a completed series"
    );
}

#[tokio::test]
async fn an_abandoned_entry_is_still_suggested() {
    // Started entry 2 and never finished it. Entry 2 is still what to read: a
    // suggestion of entry 3 hides an unfinished book without telling anyone.
    let f = Fixture::with_series("sr_abandoned", 4, false).await;
    f.finish(&[1]).await;
    f.start(2).await;
    let got = f.suggestions().await;
    assert_eq!(got[0].work_id, f.works[1], "the unfinished entry 2");
}

#[tokio::test]
async fn a_series_you_own_but_have_not_read_offers_its_first_entry() {
    // Cold start: you are the author of a five-part series and have read none of
    // it. Entry one is the natural starting point.
    let f = Fixture::with_series("sr_cold", 5, true).await;
    let got = f.suggestions().await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].work_id, f.works[0], "entry 1");
    assert_eq!(got[0].origin, Origin::FirstInSeries);
    assert_eq!(got[0].after_work_id, None, "nothing was finished");
}

#[tokio::test]
async fn owning_a_series_you_have_read_stops_the_cold_start_suggestion() {
    // Otherwise an author who read their own series is offered entry 1 forever, as
    // a duplicate of the `next_after_finished` suggestion for the same work.
    let f = Fixture::with_series("sr_owned", 3, true).await;
    f.finish(&[1]).await;
    let got = f.suggestions().await;
    assert_eq!(got.len(), 1, "exactly one suggestion, not two for one work");
    assert_eq!(got[0].origin, Origin::NextAfterFinished);
    assert_eq!(got[0].work_id, f.works[1]);
}

#[tokio::test]
async fn a_completion_outside_the_window_does_not_start_a_suggestion() {
    // The window bounds *finishing*. Without the filter the query would suggest
    // the next entry for every series ever finished, forever.
    let f = Fixture::with_series("sr_window", 4, false).await;
    f.finish(&[1]).await;
    let old = next_entries(f.tdb.db(), &f.reader, T0 - 90 * DAY, T0 - 60 * DAY)
        .await
        .expect("an out-of-window query");
    assert!(
        old.is_empty(),
        "a finish 60-90 days ago is outside a 30-day window"
    );
    let recent = f.suggestions().await;
    assert_eq!(recent.len(), 1, "and inside the window it counts");
}

#[tokio::test]
async fn another_readers_finished_entries_do_not_start_your_suggestion() {
    // The query joins on `account_id`. Forgetting that join filter means every
    // series anybody on the instance has finished becomes your recommendation.
    let f = Fixture::with_series("sr_scoped", 4, false).await;
    let stranger = account(&f.tdb, "sr_stranger@example.com").await;
    exec_with(
        &f.tdb,
        "INSERT INTO reading_status (id, account_id, subject_type, subject_id, status, \
             started_at, finished_at, updated_at) \
         VALUES (?1#u, ?2#u, 'work', ?3#u, 'finished', '2026-01-02T00:00:00Z', \
             '2026-01-05T00:00:00Z', '2026-01-05T00:00:00Z')",
        &[
            &uuid::Uuid::new_v4().to_string(),
            &stranger,
            &f.works[0].clone(),
        ],
    )
    .await;
    assert!(
        f.suggestions().await.is_empty(),
        "someone else's reading is not your history"
    );
}

#[tokio::test]
async fn a_non_series_collection_is_never_suggested() {
    // `collection_kind` is the discriminator. A reading list or anthology has
    // `position` too, and treating it as a series would invent reading orders for
    // collections that do not have one.
    let dir = scratch_dir("sr_kind");
    let tdb = TestDb::connect_with_dir("sr_kind", &dir).await;
    let reader = account(&tdb, "sr_kind@example.com").await;
    let list = uuid::Uuid::new_v4().to_string();
    exec_with(
        &tdb,
        "INSERT INTO media_collections \
             (id, collection_kind, owning_account_id, title, visibility, created_at, \
              updated_at) \
         VALUES (?1#u, 'reading_list', ?2#u, 'Rainy Days', 'public', '2026-01-01T00:00:00Z', \
             '2026-01-01T00:00:00Z')",
        &[&list, &reader],
    )
    .await;
    let work = uuid::Uuid::new_v4().to_string();
    exec_with(
        &tdb,
        "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at, \
             generated_content_posture) \
         VALUES (?1#u, (SELECT id FROM pseuds WHERE account_id = ?2#u), 'Short Story', \
             '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'forbid')",
        &[&work, &reader],
    )
    .await;
    exec_with(
        &tdb,
        "INSERT INTO media_collection_items (id, collection_id, work_id, position, added_at) \
         VALUES (?1#u, ?2#u, ?3#u, 1, '2026-01-01T00:00:00Z')",
        &[&uuid::Uuid::new_v4().to_string(), &list, &work],
    )
    .await;
    exec_with(
        &tdb,
        "INSERT INTO reading_status (id, account_id, subject_type, subject_id, status, \
             started_at, finished_at, updated_at) \
         VALUES (?1#u, ?2#u, 'work', ?3#u, 'finished', '2026-01-02T00:00:00Z', \
             '2026-01-05T00:00:00Z', '2026-01-05T00:00:00Z')",
        &[&uuid::Uuid::new_v4().to_string(), &reader.clone(), &work],
    )
    .await;
    let got = next_entries(tdb.db(), &reader, FROM, TO)
        .await
        .expect("query");
    assert!(
        got.is_empty(),
        "a reading list has no reading order, so it suggests nothing"
    );
}

#[tokio::test]
async fn a_work_in_no_series_is_not_suggested() {
    // The join goes through `media_collection_items`, so a standalone work cannot
    // appear. Trivially true of the query and worth pinning: it is what stops a
    // "next entry" feed from becoming a general unread feed.
    let f = Fixture::with_series("sr_noseries", 3, false).await;
    let orphan = uuid::Uuid::new_v4().to_string();
    exec_with(
        &f.tdb,
        "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at, \
             generated_content_posture) \
         VALUES (?1#u, (SELECT id FROM pseuds WHERE account_id = ?2#u), 'Standalone', \
             '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'forbid')",
        &[&orphan, &f.owner],
    )
    .await;
    f.finish(&[1]).await;
    let got = f.suggestions().await;
    assert_eq!(got.len(), 1);
    assert_ne!(got[0].work_id, orphan);
    assert_eq!(got[0].work_id, f.works[1]);
}

#[tokio::test]
async fn two_series_contribute_one_suggestion_each() {
    // Not "the best series", not "all entries" -- each series contributes its own
    // next entry, so the feed scales with the reader's series, not with catalogue
    // size.
    let f = Fixture::with_series("sr_two", 3, false).await;
    let second = uuid::Uuid::new_v4().to_string();
    exec_with(
        &f.tdb,
        "INSERT INTO media_collections \
             (id, collection_kind, owning_account_id, title, visibility, created_at, \
              updated_at) \
         VALUES (?1#u, 'series', ?2#u, 'Second Series', 'public', '2026-01-01T00:00:00Z', \
             '2026-01-01T00:00:00Z')",
        &[&second, &f.owner],
    )
    .await;
    let mut second_works = Vec::new();
    for n in 1..=2 {
        let work = uuid::Uuid::new_v4().to_string();
        exec_with(
            &f.tdb,
            "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at, \
                 generated_content_posture) \
             VALUES (?1#u, (SELECT id FROM pseuds WHERE account_id = ?2#u), ?3, \
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'forbid')",
            &[&work, &f.owner, &format!("Second {n}")],
        )
        .await;
        exec_with(
            &f.tdb,
            "INSERT INTO media_collection_items (id, collection_id, work_id, position, \
                 added_at) \
             VALUES (?1#u, ?2#u, ?3#u, ?4#i, '2026-01-01T00:00:00Z')",
            &[
                &uuid::Uuid::new_v4().to_string(),
                &second.clone(),
                &work,
                &n.to_string(),
            ],
        )
        .await;
        second_works.push(work);
    }
    f.finish(&[1]).await;
    exec_with(
        &f.tdb,
        "INSERT INTO reading_status (id, account_id, subject_type, subject_id, status, \
             started_at, finished_at, updated_at) \
         VALUES (?1#u, ?2#u, 'work', ?3#u, 'finished', '2026-01-02T00:00:00Z', \
             '2026-01-05T00:00:00Z', '2026-01-05T00:00:00Z')",
        &[
            &uuid::Uuid::new_v4().to_string(),
            &f.reader.clone(),
            &second_works[0].clone(),
        ],
    )
    .await;

    let got = f.suggestions().await;
    assert_eq!(got.len(), 2, "one per series");
    let mut titles: Vec<&str> = got.iter().map(|e| e.series_title.as_str()).collect();
    titles.sort_unstable();
    assert_eq!(titles, ["Second Series", "The Long Arc"]);
}

// ── fixtures ────────────────────────────────────────────────────────────────

/// An account with a pseud, by email. `register` in `test_support` needs a router,
/// so the rows are written directly — see `crates/db/tests/hit_rate.rs` for the same
/// reason.
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

/// `?N#u` binds a native uuid on PostgreSQL and text on SQLite.
///
/// `?N` is text on both, and `?N#i` an integer — `media_collection_items.position` is
/// INTEGER on SQLite and PostgreSQL alike, so plain `?N` would work, but the cast is
/// kept so an integer argument cannot silently bind as text.
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
                };
            }
            q.execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("fixture insert");
        }
    }
}

#[tokio::test]
async fn an_anthology_in_a_series_position_is_not_a_reading_order() {
    // The mutation that widened `collection_kind = 'series'` to any kind left all
    // fifteen green, because every other fixture collection *was* a series. The
    // distinction matters for a reason the data makes plain: an anthology has a
    // `position` too, and reading it as a reading order would tell a reader that
    // the fifth story in an anthology is the one to read after the fourth -- which
    // is a statement about an author's curation, not about a sequence.
    let dir = scratch_dir("sr_anthology");
    let tdb = TestDb::connect_with_dir("sr_anthology", &dir).await;
    let reader = account(&tdb, "sr_anth@example.com").await;
    let anth = uuid::Uuid::new_v4().to_string();
    exec_with(
        &tdb,
        "INSERT INTO media_collections \
             (id, collection_kind, owning_account_id, title, visibility, created_at, \
              updated_at) \
         VALUES (?1#u, 'anthology', ?2#u, 'Shades', 'public', '2026-01-01T00:00:00Z', \
             '2026-01-01T00:00:00Z')",
        &[&anth, &reader.clone()],
    )
    .await;
    let mut works = Vec::new();
    for n in 1..=3 {
        let work = uuid::Uuid::new_v4().to_string();
        exec_with(
            &tdb,
            "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at, \
                 generated_content_posture) \
             VALUES (?1#u, (SELECT id FROM pseuds WHERE account_id = ?2#u), ?3, \
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'forbid')",
            &[&work, &reader.clone(), &format!("Story {n}")],
        )
        .await;
        exec_with(
            &tdb,
            "INSERT INTO media_collection_items (id, collection_id, work_id, position, \
                 added_at) \
             VALUES (?1#u, ?2#u, ?3#u, ?4#i, '2026-01-01T00:00:00Z')",
            &[
                &uuid::Uuid::new_v4().to_string(),
                &anth.clone(),
                &work.clone(),
                &n.to_string(),
            ],
        )
        .await;
        works.push(work);
    }
    // The reader finishes the FIRST story, which under a widened filter would make
    // story 2 the "next entry".
    exec_with(
        &tdb,
        "INSERT INTO reading_status (id, account_id, subject_type, subject_id, status, \
             started_at, finished_at, updated_at) \
         VALUES (?1#u, ?2#u, 'work', ?3#u, 'finished', '2026-01-02T00:00:00Z', \
             '2026-01-05T00:00:00Z', '2026-01-05T00:00:00Z')",
        &[
            &uuid::Uuid::new_v4().to_string(),
            &reader.clone(),
            &works[0].clone(),
        ],
    )
    .await;
    let got = next_entries(tdb.db(), &reader, FROM, TO)
        .await
        .expect("query");
    assert!(
        got.is_empty(),
        "an anthology is a curation, not a sequence: {}",
        got.iter()
            .map(|e| e.entry_title.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
}

#[tokio::test]
async fn a_series_finished_only_outside_the_window_offers_nothing_next() {
    // The mutation that removed the window filter from the `finished` CTE left all
    // fifteen green, because every other fixture finished inside the window. Without
    // the filter the query would suggest the next entry for every series a reader has
    // *ever* finished, and the window would be decorative.
    //
    // Note this is a different filter from the one the first draft guarded. The cold
    // start branch checks "finished anything, ever" on purpose -- having read part of
    // a series is not erased by a window. The *forward* branch is window-scoped, and
    // this test is what keeps that difference honest.
    let dir = scratch_dir("sr_win2");
    let tdb = TestDb::connect_with_dir("sr_win2", &dir).await;
    let reader = account(&tdb, "sr_win2@example.com").await;
    let owner = account(&tdb, "sr_win2-author@example.com").await;
    let series = uuid::Uuid::new_v4().to_string();
    exec_with(
        &tdb,
        "INSERT INTO media_collections \
             (id, collection_kind, owning_account_id, title, visibility, created_at, \
              updated_at) \
         VALUES (?1#u, 'series', ?2#u, 'Old Faithful', 'public', '2026-01-01T00:00:00Z', \
             '2026-01-01T00:00:00Z')",
        &[&series, &owner.clone()],
    )
    .await;
    let mut works = Vec::new();
    for n in 1..=3 {
        let work = uuid::Uuid::new_v4().to_string();
        exec_with(
            &tdb,
            "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at, \
                 generated_content_posture) \
             VALUES (?1#u, (SELECT id FROM pseuds WHERE account_id = ?2#u), ?3, \
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'forbid')",
            &[&work, &owner.clone(), &format!("Vol {n}")],
        )
        .await;
        exec_with(
            &tdb,
            "INSERT INTO media_collection_items (id, collection_id, work_id, position, \
                 added_at) \
             VALUES (?1#u, ?2#u, ?3#u, ?4#i, '2026-01-01T00:00:00Z')",
            &[
                &uuid::Uuid::new_v4().to_string(),
                &series.clone(),
                &work.clone(),
                &n.to_string(),
            ],
        )
        .await;
        works.push(work);
    }
    // Finished long ago: 2025-06-01, well outside the January 2026 window.
    exec_with(
        &tdb,
        "INSERT INTO reading_status (id, account_id, subject_type, subject_id, status, \
             started_at, finished_at, updated_at) \
         VALUES (?1#u, ?2#u, 'work', ?3#u, 'finished', '2025-05-30T00:00:00Z', \
             '2025-06-01T00:00:00Z', '2025-06-01T00:00:00Z')",
        &[
            &uuid::Uuid::new_v4().to_string(),
            &reader.clone(),
            &works[0].clone(),
        ],
    )
    .await;
    let got = next_entries(tdb.db(), &reader, FROM, TO)
        .await
        .expect("query");
    assert!(
        got.is_empty(),
        "a finish seven months ago is not a fresh reading event: {:?}",
        got.iter().map(|e| e.position).collect::<Vec<_>>()
    );

    // The same series, the same reader, a finish INSIDE the window: now it counts.
    exec_with(
        &tdb,
        "INSERT INTO reading_status (id, account_id, subject_type, subject_id, status, \
             started_at, finished_at, updated_at) \
         VALUES (?1#u, ?2#u, 'work', ?3#u, 'finished', '2026-01-02T00:00:00Z', \
             '2026-01-05T00:00:00Z', '2026-01-05T00:00:00Z')",
        &[
            &uuid::Uuid::new_v4().to_string(),
            &reader.clone(),
            &works[1].clone(),
        ],
    )
    .await;
    let recent = next_entries(tdb.db(), &reader, FROM, TO)
        .await
        .expect("query");
    assert_eq!(
        recent.len(),
        1,
        "with a recent finish the first unfinished entry is offered again"
    );
    assert_eq!(recent[0].work_id, works[0], "volume 1, still unread");
}
