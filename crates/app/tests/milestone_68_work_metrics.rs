//! M68 — Work card aggregate metrics (spec §9.5, §9.4, §10)
//! (`crates/db/src/work_metrics.rs`).
//!
//! Fourteen public functions with no test touching them. Two things here are
//! worth a test rather than a read:
//!
//!   * **`increment_counter` is public, takes a caller-supplied column name, and
//!     interpolates it straight into SQL.** Its comment says "the column name is
//!     validated by the caller (always a literal in this module)" — but it has
//!     no callers at all outside this module, and being `pub` it is reachable
//!     from anywhere in the crate. It is an injection-shaped API waiting for a
//!     caller that passes user input. Pinned below rather than removed, because
//!     whether the counter set should be a closed enum or a wider escape hatch
//!     is a design decision.
//!   * **The counters are counts, not averages, and the materialized row is a
//!     cache that can disagree with the source tables.** `get_metrics` falls
//!     back to `compute_live` when the row is missing, so the two paths have to
//!     agree — a test that only exercises one of them misses the divergence that
//!     matters. Both are covered here, and `recompute_and_store` is the repair
//!     tool that reconciles them.
//!
//! This module has already been through the INT4/UUID traps -- its own comments
//! record the `CAST(... AS BIGINT)` on every aggregate, the `$1::uuid` on the
//! kudos insert, and the `to_char(now() AT TIME ZONE 'UTC', ...)` for the TEXT
//! `created_at`. Those fixes are worth keeping honest: this suite is the only
//! thing that would notice if one of them were reverted.

use std::path::PathBuf;

use lorehaven_db::work_metrics as wm;
use test_support::TestDb;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m68-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

struct Db {
    tdb: TestDb,
    _dir: PathBuf,
}

impl Db {
    async fn new(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        Self { tdb, _dir: dir }
    }
    fn db(&self) -> &lorehaven_db::Database {
        self.tdb.db()
    }
    /// SQLite stores a BOOLEAN-declared column as an INTEGER; PostgreSQL
    /// refuses an integer for a real boolean. `review.is_public` is a genuine
    /// BOOLEAN while `collections.is_public` is a BIGINT flag, so the literal has
    /// to follow the column, not the harness.
    fn bool_lit(&self, v: bool) -> &'static str {
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => {
                if v {
                    "1"
                } else {
                    "0"
                }
            }
            lorehaven_db::Backend::Postgres => {
                if v {
                    "true"
                } else {
                    "false"
                }
            }
        }
    }

    /// Like `exec`, but surfaces the error instead of panicking -- for the one
    /// test that asserts a statement is *refused*.
    async fn try_exec(&self, query: &str) -> Result<(), sqlx::Error> {
        let q = self.tdb.sql(query);
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => {
                sqlx::query(&q)
                    .execute(self.db().sqlite_pool().expect("sqlite"))
                    .await?;
            }
            lorehaven_db::Backend::Postgres => {
                sqlx::query(&q)
                    .execute(self.db().postgres_pool().expect("pg"))
                    .await?;
            }
        }
        Ok(())
    }

    async fn exec(&self, query: &str) {
        let q = self.tdb.sql(query);
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => {
                sqlx::query(&q)
                    .execute(self.db().sqlite_pool().expect("sqlite"))
                    .await
                    .expect("exec");
            }
            lorehaven_db::Backend::Postgres => {
                sqlx::query(&q)
                    .execute(self.db().postgres_pool().expect("pg"))
                    .await
                    .expect("exec");
            }
        }
    }
    /// The `accounts` -> `pseuds` -> `works` chain. `works.owner_pseud_id` and
    /// `work_kudos.account_id` are foreign keys, so a kudos test needs real
    /// rows at every level.
    async fn account(&self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.exec(&format!(
            "INSERT INTO accounts (id, email, created_at, updated_at) \
             VALUES ('{id}', 'a{id}@example.test', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')"
        ))
        .await;
        id
    }
    async fn pseud(&self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.exec(&format!(
            "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
             VALUES ('{id}', '{}', 'h{}', 'H', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
            self.account().await,
            id.replace('-', "")
        ))
        .await;
        id
    }
    async fn work(&self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.exec(&format!(
            "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at) \
             VALUES ('{id}', '{}', 'A Work', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
            self.pseud().await
        ))
        .await;
        id
    }
}

// ------------------------------------------------------------- live counts

#[tokio::test]
async fn a_work_with_no_activity_has_every_counter_at_zero() {
    let h = Db::new("zero").await;
    let work = h.work().await;
    let m = wm::compute_live(h.db(), &work).await.unwrap();
    assert_eq!(m.views, 0);
    assert_eq!(m.complete_reads, 0);
    assert_eq!(m.reactions, 0);
    assert_eq!(m.kudos, 0);
    assert_eq!(m.bookmarks, 0);
    assert_eq!(m.collection_adds, 0);
    assert_eq!(m.reviews, 0);
}

#[tokio::test]
async fn views_are_counted_per_viewer_and_timestamp() {
    let h = Db::new("views").await;
    let work = h.work().await;
    assert!(wm::record_view(
        h.db(),
        &work,
        "viewer-a",
        "2026-02-01T00:00:00+00:00",
        false
    )
    .await
    .unwrap());
    assert!(wm::record_view(
        h.db(),
        &work,
        "viewer-b",
        "2026-02-01T00:00:00+00:00",
        false
    )
    .await
    .unwrap());
    assert_eq!(wm::count_views(h.db(), &work).await.unwrap(), 2);
}

#[tokio::test]
async fn an_automated_view_is_recorded_but_not_counted() {
    let h = Db::new("views-automated").await;
    let work = h.work().await;
    // The event is kept -- it is evidence -- but it does not inflate the card.
    assert!(
        wm::record_view(h.db(), &work, "crawler", "2026-02-01T00:00:00+00:00", true)
            .await
            .unwrap()
    );
    assert_eq!(
        wm::count_views(h.db(), &work).await.unwrap(),
        0,
        "an automated view is excluded from the count"
    );
    wm::record_view(h.db(), &work, "reader", "2026-02-01T00:00:00+00:00", false)
        .await
        .unwrap();
    assert_eq!(wm::count_views(h.db(), &work).await.unwrap(), 1);
}

#[tokio::test]
async fn the_same_viewer_at_the_same_instant_is_deduplicated() {
    let h = Db::new("views-dup").await;
    let work = h.work().await;
    let at = "2026-02-01T00:00:00+00:00";
    assert!(wm::record_view(h.db(), &work, "viewer-a", at, false)
        .await
        .unwrap());
    // The unique key is (work_id, viewer_hash, viewed_at), so a replayed event
    // is refused and `record_view` reports false.
    assert!(
        !wm::record_view(h.db(), &work, "viewer-a", at, false)
            .await
            .unwrap(),
        "a duplicate event at the same timestamp is refused"
    );
    assert_eq!(wm::count_views(h.db(), &work).await.unwrap(), 1);
    // A different timestamp for the same viewer is a different row: the dedup
    // is per event, not per reader.
    assert!(wm::record_view(
        h.db(),
        &work,
        "viewer-a",
        "2026-02-01T00:05:00+00:00",
        false
    )
    .await
    .unwrap());
    assert_eq!(wm::count_views(h.db(), &work).await.unwrap(), 2);
}

#[tokio::test]
async fn a_complete_read_is_a_reader_who_finished() {
    let h = Db::new("reads").await;
    let work = h.work().await;
    let account = h.account().await;
    h.exec(&format!(
        "INSERT INTO reading_status (id, account_id, subject_type, subject_id, status, updated_at) \
         VALUES ('{}', '{account}', 'work', '{work}', 'finished', '2026-02-01T00:00:00+00:00')",
        uuid::Uuid::new_v4()
    ))
    .await;
    assert_eq!(wm::count_complete_reads(h.db(), &work).await.unwrap(), 1);
    // A reader's status is one row per (account, subject) -- it is updated, not
    // appended to. Moving back to `reading` therefore drops the complete read.
    h.exec(&format!(
        "UPDATE reading_status SET status = 'reading' WHERE account_id = '{account}' \
         AND subject_type = 'work' AND subject_id = '{work}'"
    ))
    .await;
    assert_eq!(wm::count_complete_reads(h.db(), &work).await.unwrap(), 0);
}

#[tokio::test]
async fn a_reaction_is_one_per_pseud_per_work() {
    let h = Db::new("reactions").await;
    let work = h.work().await;
    for pseud in ["alice", "bob"] {
        h.exec(&format!(
            "INSERT INTO work_reactions (work_id, pseud, vote_type, created_at, updated_at) \
             VALUES ('{work}', '{pseud}', 'like', '2026-02-01T00:00:00+00:00', '2026-02-01T00:00:00+00:00')"
        ))
        .await;
    }
    assert_eq!(wm::count_reactions(h.db(), &work).await.unwrap(), 2);
    // The primary key is (work_id, pseud), so a second reaction by the same
    // pseud is a constraint violation rather than a second count. Reacting
    // again therefore has to *change* the existing row, not add one -- which is
    // what the caller does with an UPSERT of its own.
    let changed = h
        .try_exec(&format!(
            "UPDATE work_reactions SET vote_type = 'love' \
             WHERE work_id = '{work}' AND pseud = 'alice'"
        ))
        .await;
    assert!(changed.is_ok());
    assert_eq!(
        wm::count_reactions(h.db(), &work).await.unwrap(),
        2,
        "changing a reaction does not add to the count"
    );
}

#[tokio::test]
async fn a_bookmark_is_counted_for_a_work_subject_only() {
    let h = Db::new("bookmarks").await;
    let work = h.work().await;
    let account = h.account().await;
    h.exec(&format!(
        "INSERT INTO bookmarks (id, account_id, subject_type, subject_id, created_at, updated_at) \
         VALUES ('{}', '{account}', 'work', '{work}', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
        uuid::Uuid::new_v4()
    ))
    .await;
    assert_eq!(wm::count_bookmarks(h.db(), &work).await.unwrap(), 1);
    // A bookmark of something else is not a bookmark of this work.
    h.exec(&format!(
        "INSERT INTO bookmarks (id, account_id, subject_type, subject_id, created_at, updated_at) \
         VALUES ('{}', '{account}', 'series', '{work}', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
        uuid::Uuid::new_v4()
    ))
    .await;
    assert_eq!(wm::count_bookmarks(h.db(), &work).await.unwrap(), 1);
}

#[tokio::test]
async fn only_public_collections_count_towards_collection_adds() {
    let h = Db::new("collection-adds").await;
    let work = h.work().await;
    for (name, is_public) in [("public shelf", 1), ("private shelf", 0)] {
        let id = uuid::Uuid::new_v4().to_string();
        // `collections` here is the events table (0014): TEXT ids, an `owner`
        // pseud, and a required `item_policy`. `is_public` is a BIGINT flag, not
        // a boolean -- the reason the module's count query can compare it to 1
        // on both dialects.
        h.exec(&format!(
            "INSERT INTO collections (id, name, owner, item_policy, is_public, created_at) \
             VALUES ('{id}', '{name}', 'owner-pseud', 'anyone', {is_public}, '2026-01-01T00:00:00+00:00')"
        ))
        .await;
        h.exec(&format!(
            "INSERT INTO collection_items (collection_id, work_id, added_by, added_at) \
             VALUES ('{id}', '{work}', 'owner-pseud', '2026-01-01T00:00:00+00:00')"
        ))
        .await;
    }
    assert_eq!(
        wm::count_collection_adds(h.db(), &work).await.unwrap(),
        1,
        "a private collection is not public evidence of interest"
    );
}

#[tokio::test]
async fn only_undeleted_public_reviews_are_counted() {
    let h = Db::new("reviews").await;
    let work = h.work().await;
    let pseud = h.pseud().await;
    let account = h.account().await;
    h.exec(&format!(
        "INSERT INTO review (id, account_id, pseud_id, work_id, body, is_public, created_at, updated_at) \
         VALUES ('{}', '{account}', '{pseud}', '{work}', 'good', {}, '2026-02-01T00:00:00+00:00', '2026-02-01T00:00:00+00:00')",
        uuid::Uuid::new_v4(),
        h.bool_lit(true)
    ))
    .await;
    assert_eq!(wm::count_reviews(h.db(), &work).await.unwrap(), 1);
    // A deleted review is not shown on the card.
    h.exec(&format!(
        "UPDATE review SET deleted_at = '2026-02-02T00:00:00+00:00' WHERE work_id = '{work}'"
    ))
    .await;
    assert_eq!(wm::count_reviews(h.db(), &work).await.unwrap(), 0);
}

// ------------------------------------------------------------------ kudos

#[tokio::test]
async fn kudos_toggle_on_and_off_and_tracks_the_aggregate() {
    let h = Db::new("kudos").await;
    let work = h.work().await;
    let account = h.account().await;
    assert!(
        wm::toggle_kudos(h.db(), &work, &account).await.unwrap(),
        "first toggle is on"
    );
    assert_eq!(wm::count_kudos(h.db(), &work).await.unwrap(), 1);
    assert!(
        !wm::toggle_kudos(h.db(), &work, &account).await.unwrap(),
        "second toggle is off"
    );
    assert_eq!(wm::count_kudos(h.db(), &work).await.unwrap(), 0);
}

#[tokio::test]
async fn kudos_from_two_accounts_are_independent() {
    let h = Db::new("kudos-two").await;
    let work = h.work().await;
    let a = h.account().await;
    let b = h.account().await;
    assert!(wm::toggle_kudos(h.db(), &work, &a).await.unwrap());
    assert!(wm::toggle_kudos(h.db(), &work, &b).await.unwrap());
    assert_eq!(wm::count_kudos(h.db(), &work).await.unwrap(), 2);
    // One account opting out leaves the other's kudos alone.
    assert!(!wm::toggle_kudos(h.db(), &work, &a).await.unwrap());
    assert_eq!(wm::count_kudos(h.db(), &work).await.unwrap(), 1);
}

#[tokio::test]
async fn toggling_kudos_keeps_the_materialized_aggregate_in_step() {
    let h = Db::new("kudos-aggregate").await;
    let work = h.work().await;
    let account = h.account().await;
    wm::toggle_kudos(h.db(), &work, &account).await.unwrap();
    wm::increment_views(h.db(), &work).await.unwrap();
    let m = wm::get_metrics(h.db(), &work.parse().expect("uuid"))
        .await
        .unwrap();
    assert_eq!(m.kudos, 1);
    assert_eq!(m.views, 1);
    wm::toggle_kudos(h.db(), &work, &account).await.unwrap();
    let m = wm::get_metrics(h.db(), &work.parse().expect("uuid"))
        .await
        .unwrap();
    assert_eq!(m.kudos, 0, "the aggregate decrements with the toggle");
    assert_eq!(m.views, 1, "and the other counters are untouched");
}

// ------------------------------------------------------- materialized row

#[tokio::test]
async fn get_metrics_falls_back_to_live_counts_when_no_row_exists() {
    let h = Db::new("fallback").await;
    let work = h.work().await;
    wm::record_view(h.db(), &work, "reader", "2026-02-01T00:00:00+00:00", false)
        .await
        .unwrap();
    // No aggregate row has been written, so the read path has to compute it.
    let m = wm::get_metrics(h.db(), &work.parse().expect("uuid"))
        .await
        .unwrap();
    assert_eq!(m.work_id, work);
    assert_eq!(m.views, 1, "the fallback sees the same view the log does");
}

#[tokio::test]
async fn a_materialized_row_is_preferred_over_a_live_recompute() {
    let h = Db::new("materialized").await;
    let work = h.work().await;
    wm::increment_views(h.db(), &work).await.unwrap();
    wm::increment_views(h.db(), &work).await.unwrap();
    // A view in the log would make the live count disagree with the cache. The
    // cache is what the card reads, so the disagreement is deliberate and the
    // repair tool is what reconciles it.
    wm::record_view(h.db(), &work, "reader", "2026-02-01T00:00:00+00:00", false)
        .await
        .unwrap();
    let m = wm::get_metrics(h.db(), &work.parse().expect("uuid"))
        .await
        .unwrap();
    assert_eq!(m.views, 2, "the materialized row is what a read returns");
    assert_eq!(
        wm::count_views(h.db(), &work).await.unwrap(),
        1,
        "the log says one"
    );
}

#[tokio::test]
async fn recompute_and_store_reconciles_the_cache_with_the_sources() {
    let h = Db::new("recompute").await;
    let work = h.work().await;
    wm::increment_views(h.db(), &work).await.unwrap();
    wm::increment_views(h.db(), &work).await.unwrap();
    wm::record_view(h.db(), &work, "reader", "2026-02-01T00:00:00+00:00", false)
        .await
        .unwrap();
    let repaired = wm::recompute_and_store(h.db(), &work).await.unwrap();
    assert_eq!(
        repaired.views, 1,
        "the repair overwrites the cache with the truth"
    );
    let m = wm::get_metrics(h.db(), &work.parse().expect("uuid"))
        .await
        .unwrap();
    assert_eq!(m.views, 1, "and the read now agrees with the log");
}

#[tokio::test]
async fn recomputing_twice_is_idempotent() {
    let h = Db::new("recompute-twice").await;
    let work = h.work().await;
    wm::record_view(h.db(), &work, "reader", "2026-02-01T00:00:00+00:00", false)
        .await
        .unwrap();
    let first = wm::recompute_and_store(h.db(), &work).await.unwrap();
    let second = wm::recompute_and_store(h.db(), &work).await.unwrap();
    assert_eq!(first.views, second.views, "recompute does not accumulate");
    assert_eq!(wm::count_views(h.db(), &work).await.unwrap(), 1);
}

#[tokio::test]
async fn recompute_for_a_work_that_does_not_exist_is_refused() {
    // `work_metric_aggregates.work_id` references `works(id)`, so the repair
    // tool cannot invent metrics for a work that was never created. This is the
    // right shape -- a cache row for a nonexistent work would outlive the work.
    let h = Db::new("recompute-unknown").await;
    let work = uuid::Uuid::new_v4().to_string();
    assert!(
        wm::recompute_and_store(h.db(), &work).await.is_err(),
        "the foreign key stops a metrics row for a work that is not there"
    );
}

// ------------------------------------------------------------- counters

#[tokio::test]
async fn incrementing_a_counter_creates_the_row_lazily_and_accumulates() {
    let h = Db::new("counter").await;
    let work = h.work().await;
    wm::increment_counter(h.db(), &work, "bookmarks", 1)
        .await
        .unwrap();
    wm::increment_counter(h.db(), &work, "bookmarks", 1)
        .await
        .unwrap();
    wm::increment_counter(h.db(), &work, "bookmarks", 3)
        .await
        .unwrap();
    let m = wm::get_metrics(h.db(), &work.parse().expect("uuid"))
        .await
        .unwrap();
    assert_eq!(
        m.bookmarks, 5,
        "the first insert is the seed and later ones add"
    );
    assert_eq!(m.views, 0, "the other counters are left at their default");
}

#[tokio::test]
async fn a_counter_can_be_moved_by_a_negative_delta() {
    let h = Db::new("counter-negative").await;
    let work = h.work().await;
    wm::increment_counter(h.db(), &work, "reviews", 5)
        .await
        .unwrap();
    wm::increment_counter(h.db(), &work, "reviews", -2)
        .await
        .unwrap();
    let m = wm::get_metrics(h.db(), &work.parse().expect("uuid"))
        .await
        .unwrap();
    assert_eq!(m.reviews, 3);
}

#[tokio::test]
async fn increment_views_moves_the_views_counter_by_one() {
    let h = Db::new("increment-views").await;
    let work = h.work().await;
    wm::increment_views(h.db(), &work).await.unwrap();
    wm::increment_views(h.db(), &work).await.unwrap();
    let m = wm::get_metrics(h.db(), &work.parse().expect("uuid"))
        .await
        .unwrap();
    assert_eq!(m.views, 2);
}

#[tokio::test]
async fn increment_counter_will_interpolate_any_column_name_it_is_given() {
    // KNOWN DEFECT. `increment_counter` is `pub`, takes `column: &str` and
    // interpolates it into the INSERT column list, the VALUES clause and the
    // `ON CONFLICT DO UPDATE SET` clause. Its comment says "the column name is
    // validated by the caller (always a literal in this module)" -- but it has
    // no callers outside this module, and `pub` makes it reachable from anywhere
    // in the crate, so the validation the comment relies on does not exist. A
    // caller that passed a request-supplied name would be a SQL injection.
    //
    // Not changed here, because closing it is a design decision: either make the
    // function private and add seven thin wrappers, or validate the name against
    // the known counter set and return an error for anything else. The second
    // keeps the escape hatch for the seed script; the first is safer. What is
    // not a decision is leaving a `pub` interpolating entry point undocumented
    // as dangerous, which is what this comment is for.
    let h = Db::new("counter-inject").await;
    let work = h.work().await;
    // A name that is not a counter column is a SQL error, not a silent success.
    let err = wm::increment_counter(h.db(), &work, "not_a_column", 1).await;
    assert!(
        err.is_err(),
        "an unknown column name fails loudly rather than corrupting the row"
    );
    // A name that *is* a column works -- which is exactly the problem: there is
    // no allowlist between the caller and the statement.
    wm::increment_counter(h.db(), &work, "views", 1)
        .await
        .unwrap();
    assert_eq!(
        wm::get_metrics(h.db(), &work.parse().expect("uuid"))
            .await
            .unwrap()
            .views,
        1
    );
}

#[tokio::test]
async fn the_counter_columns_are_the_seven_the_card_shows() {
    let h = Db::new("counter-set").await;
    let work = h.work().await;
    // The set the module itself uses. Written out here so that a rename or a
    // new counter has to be a deliberate change to this test as well.
    for column in [
        "views",
        "complete_reads",
        "reactions",
        "kudos",
        "bookmarks",
        "collection_adds",
        "reviews",
    ] {
        wm::increment_counter(h.db(), &work, column, 1)
            .await
            .unwrap_or_else(|e| panic!("{column} should be a counter column: {e}"));
    }
    let m = wm::get_metrics(h.db(), &work.parse().expect("uuid"))
        .await
        .unwrap();
    assert_eq!(
        (
            m.views,
            m.complete_reads,
            m.reactions,
            m.kudos,
            m.bookmarks,
            m.collection_adds,
            m.reviews
        ),
        (1, 1, 1, 1, 1, 1, 1),
        "every counter column moved independently"
    );
}
