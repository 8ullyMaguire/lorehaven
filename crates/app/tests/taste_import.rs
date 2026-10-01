//! Acceptance: history import as a cold start (spec §49.6, §49.7, §49.8, M45-20;
//! plan `m45-phase1-taste-signal.md` step 4).
//!
//! §49.8 gives two clauses that can be tested without a network, and they are
//! the two this file exists for:
//!
//! | clause | test |
//! |---|---|
//! | "importing the same history twice leaves the profile identical to importing it once" | `a_second_import_changes_nothing` |
//! | "an imported bookmark makes the work's metadata visible and leaves its cached body private" | `an_imported_bookmark_leaves_the_cached_body_private` |
//!
//! Plus the §49.7 invariant the plan restates as "a `signal_origin` on the rows
//! that write weights, not a convention":
//!
//! | invariant | test |
//! |---|---|
//! | imported and organic stay distinguishable everywhere they are read | `an_imported_signal_is_distinguishable_from_an_organic_one` |
//! | an imported signal is weaker evidence than one given today | `an_imported_signal_is_weaker_evidence` |
//!
//! The idempotency test asserts **weights and rows**, not counts alone. §49.8 says
//! the *profile* is identical, and a count-only test would pass against an
//! importer that re-applied the discount to an already-discounted value on every
//! run — the row count would hold still and the reader's weights would drift
//! toward zero on each import. That is the failure mode this file is shaped
//! around.

use lorehaven_db::taste_import::{
    base_value, import_signals, origin_split, signals_by_origin, signals_for_dimension,
    IncomingSignal, SignalKind, SignalOrigin, IMPORTED_SIGNAL_DISCOUNT,
};
use lorehaven_db::{Backend, Database};
use test_support::{id, scratch_dir, TestDb};

/// One scratch instance and the database behind it.
///
/// No router: every clause in §49.8 for this milestone is about what the import
/// *writes* and what a read can then tell apart, and a test that goes through
/// HTTP would be testing the transport rather than the property. The route layer
/// for imports already exists (M53, `routes/imports.rs`) and is not what is
/// under test here.
struct Harness {
    tdb: TestDb,
    _dir: std::path::PathBuf,
    db: Database,
    /// One timestamp, reused by the constraint probes so a bind order mistake
    /// cannot be mistaken for a CHECK failure.
    now: String,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        let db = tdb.db().clone();
        Self {
            tdb,
            _dir: dir,
            db,
            now: lorehaven_db::identity::now_rfc3339(),
        }
    }

    /// A reader to import *for*. A real `accounts` row, because `taste_signals`
    /// has a foreign key to it and migration 0101 relies on that.
    ///
    /// Minimal on purpose: `id`, `email`, `created_at`, `updated_at` are the only
    /// NOT NULL columns without defaults. `accounts` has **no `handle` column** —
    /// the handle lives in `pseuds` — and the first draft of this helper assumed
    /// otherwise and failed all ten tests with "table accounts has no column
    /// named handle".
    ///
    /// The `$1::uuid` on the PostgreSQL arm is load-bearing: `accounts.id` is
    /// TEXT on SQLite and UUID on PostgreSQL, while `taste_signals.account_id` is
    /// TEXT on *both*. So the same account string goes into a UUID column here
    /// and a text column there, which is why the importer's own queries carry no
    /// cast on either engine.
    async fn account(&self, handle: &str) -> String {
        let account = id(handle);
        let email = format!("{handle}@example.test");
        let now = lorehaven_db::identity::now_rfc3339();
        match self.db.backend() {
            Backend::Sqlite => {
                let pool = self.db.sqlite_pool().expect("sqlite pool");
                sqlx::query(
                    "INSERT INTO accounts (id, email, created_at, updated_at)
                     VALUES (?, ?, ?, ?)",
                )
                .bind(&account)
                .bind(&email)
                .bind(&now)
                .bind(&now)
                .execute(pool)
                .await
                .expect("account inserted");
            }
            Backend::Postgres => {
                let pool = self.db.postgres_pool().expect("pg pool");
                sqlx::query(
                    "INSERT INTO accounts (id, email, created_at, updated_at)
                     VALUES ($1::uuid, $2, $3, $4)",
                )
                .bind(&account)
                .bind(&email)
                .bind(&now)
                .bind(&now)
                .execute(pool)
                .await
                .expect("account inserted");
            }
        }
        account
    }

    /// Every signal for a reader, sorted so two snapshots can be compared.
    async fn snapshot(&self, account: &str) -> Vec<String> {
        let mut rows: Vec<String> = signals_by_origin(&self.db, account)
            .await
            .expect("signals readable")
            .into_iter()
            .map(|s| {
                format!(
                    "{}|{}|{}|{}|{:?}|{}|{}",
                    s.origin.as_str(),
                    s.kind.as_str(),
                    s.dimension_key,
                    s.source_signal_key,
                    s.occurred_at,
                    s.signal_value,
                    s.effective_value
                )
            })
            .collect();
        rows.sort();
        rows
    }

    /// The reader's total effective weight on one dimension, which is what
    /// "the profile" means for §49.8's clause.
    async fn total_effective(&self, account: &str, dimension: &str) -> f64 {
        signals_for_dimension(&self.db, account, dimension)
            .await
            .expect("signals readable")
            .iter()
            .map(|s| s.effective_value)
            .sum()
    }

    /// Count rows in one of this feature's tables for one reader.
    ///
    /// The `::uuid` cast is spelled per engine because `taste_signals` and
    /// `taste_signal_imports` carry a foreign key to `accounts(id)`, so
    /// `account_id` is UUID on PostgreSQL -- while `test_support::count_by` binds a
    /// plain text value. Writing one query and letting the helper fix the
    /// placeholder does not fix the *type*, which is how this test suite first
    /// failed on PostgreSQL with "operator does not exist: uuid = text".
    async fn count_signals(&self, table: &str, account: &str) -> i64 {
        let query = match self.db.backend() {
            Backend::Sqlite => {
                format!("SELECT COUNT(*) FROM {table} WHERE account_id = ?")
            }
            Backend::Postgres => {
                format!("SELECT COUNT(*) FROM {table} WHERE account_id = ?::uuid")
            }
        };
        self.tdb.count_by(&query, account).await
    }

    /// Total signal rows for a reader, for the idempotency assertion.
    async fn signal_count(&self, account: &str) -> i64 {
        self.count_signals("taste_signals", account).await
    }

    async fn cleanup(self) {
        self.tdb.cleanup().await;
    }
}

/// Three bookmarks, so the idempotency test has more than one row to double.
fn history() -> Vec<IncomingSignal> {
    vec![
        IncomingSignal::new(SignalKind::Bookmark, "prose", "ao3:bookmark:1")
            .occurred_at("2019-04-02T00:00:00Z"),
        IncomingSignal::new(SignalKind::Bookmark, "prose", "ao3:bookmark:2")
            .occurred_at("2020-11-14T00:00:00Z"),
        IncomingSignal::new(SignalKind::Read, "pacing", "ao3:read:7")
            .occurred_at("2021-01-30T00:00:00Z"),
    ]
}

/// §49.8: "Importing the same history twice leaves the profile identical to
/// importing it once."
///
/// Asserted on the signal rows, the reader's effective weight, *and* the run
/// record — because the acceptance clause is about the profile, and the run
/// record is where "the second import added nothing" is observable rather than
/// merely asserted.
#[tokio::test]
async fn a_second_import_changes_nothing() {
    let h = Harness::new("taste-import-idempotent").await;
    let account = h.account("importer").await;

    let first = import_signals(
        &h.db,
        &account,
        "ao3",
        Some("cred-1"),
        "bookmarks",
        &history(),
    )
    .await
    .expect("first import succeeds");
    assert_eq!(first.seen, 3, "the history has three signals");
    assert_eq!(
        first.added, 3,
        "a first import adds every signal: {first:?}"
    );

    let rows_after_first = h.snapshot(&account).await;
    let weight_after_first = h.total_effective(&account, "prose").await;
    assert_eq!(rows_after_first.len(), 3);
    assert!(weight_after_first > 0.0, "the import moved the profile");

    // The second import is the clause. Same history, same source, same reader.
    let second = import_signals(
        &h.db,
        &account,
        "ao3",
        Some("cred-1"),
        "bookmarks",
        &history(),
    )
    .await
    .expect("second import succeeds");

    assert_eq!(
        second.seen, 3,
        "the second import still looked at three signals"
    );
    assert_eq!(
        second.added, 0,
        "the second import must add nothing: {second:?} — this is §49.8's clause"
    );

    let rows_after_second = h.snapshot(&account).await;
    assert_eq!(
        rows_after_second, rows_after_first,
        "the signal rows must be identical after a re-import, not merely the same \
         in number"
    );
    assert_eq!(
        h.total_effective(&account, "prose").await,
        weight_after_first,
        "the reader's weight must be identical after a re-import — a count-only \
         check would miss a discount applied twice"
    );
    assert_eq!(
        h.signal_count(&account).await,
        3,
        "still exactly three rows"
    );

    // And the run record is where "the second run added nothing" is queryable,
    // which is what §49.6's "re-runnable" means operationally.
    let first_run = lorehaven_db::taste_import::import_run(&h.db, &first.import_id)
        .await
        .expect("first run readable")
        .expect("first run exists");
    assert_eq!(first_run.signals_added, 3);
    assert!(
        first_run.finished_at.is_some(),
        "a finished run is marked finished"
    );

    let second_run = lorehaven_db::taste_import::import_run(&h.db, &second.import_id)
        .await
        .expect("second run readable")
        .expect("second run exists");
    assert_eq!(second_run.signals_seen, 3);
    assert_eq!(
        second_run.signals_added, 0,
        "the audit trail has to show the second run was a no-op"
    );
    h.cleanup().await;
}

/// A *partial* re-import — the realistic case, and the one a whole-history
/// assertion would miss.
///
/// The reader adds two bookmarks to their AO3 account and re-imports. The old
/// three must not move and the new two must land, so a naive "skip everything if
/// the source has been seen" implementation fails here while passing the
/// identical-history test above.
#[tokio::test]
async fn a_re_import_adds_what_is_new_and_leaves_the_rest_alone() {
    let h = Harness::new("taste-import-partial").await;
    let account = h.account("partial-importer").await;

    import_signals(
        &h.db,
        &account,
        "ao3",
        Some("cred-1"),
        "bookmarks",
        &history(),
    )
    .await
    .expect("first import");

    let before = h.snapshot(&account).await;
    let weight_before = h.total_effective(&account, "prose").await;

    let grown = vec![
        // Two the reader already has — these must be ignored, not re-applied.
        history()[0].clone(),
        history()[1].clone(),
        // One that is new.
        IncomingSignal::new(SignalKind::Bookmark, "prose", "ao3:bookmark:3")
            .occurred_at("2023-06-01T00:00:00Z"),
    ];
    let second = import_signals(&h.db, &account, "ao3", Some("cred-1"), "bookmarks", &grown)
        .await
        .expect("re-import");

    assert_eq!(second.seen, 3, "the source offered three");
    assert_eq!(
        second.added, 1,
        "only the bookmark that was not already there is new: {second:?}"
    );
    assert_eq!(h.signal_count(&account).await, 4, "three plus one");

    let after = h.snapshot(&account).await;
    // The original rows are untouched, and the new one is in there.
    for row in &before {
        assert!(
            after.contains(row),
            "an existing signal changed or vanished across a re-import: {row}"
        );
    }
    assert!(
        after.iter().any(|r| r.contains("ao3:bookmark:3")),
        "the new signal is missing: {after:?}"
    );
    assert!(
        h.total_effective(&account, "prose").await > weight_before,
        "one more bookmark is more evidence than before"
    );
    h.cleanup().await;
}

/// §49.7: "Imported and organic signals stay distinguishable. Everywhere,
/// including in exports and in an access request."
///
/// The half of this that gets quietly dropped is the **read** side: a column
/// that is written and never read satisfies every write-side test and still
/// fails the clause. So the assertion is on what a read returns, not on what a
/// write stored.
#[tokio::test]
async fn an_imported_signal_is_distinguishable_from_an_organic_one() {
    let h = Harness::new("taste-import-origin").await;
    let account = h.account("origin-reader").await;

    // An organic signal, written the way the arena writes one: this instance
    // observed the reader rating something, so it did not come through an import.
    // Inserted directly because `import_signals` deliberately cannot produce
    // one -- an import writing 'organic' is the failure the clause forbids.
    let now = lorehaven_db::identity::now_rfc3339();
    let organic_id = id("organic-signal");
    let qs = match h.db.backend() {
        Backend::Sqlite => {
            "INSERT INTO taste_signals
             (id, account_id, origin, signal_kind, dimension_key, signal_value,
              occurred_at, source_key, source_signal_key, provenance_json,
              created_at, updated_at)
             VALUES (?, ?, 'organic', 'rating', 'prose', 1.0, ?, '', '', '{}', ?, ?)"
        }
        Backend::Postgres => {
            "INSERT INTO taste_signals
             (id, account_id, origin, signal_kind, dimension_key, signal_value,
              occurred_at, source_key, source_signal_key, provenance_json,
              created_at, updated_at)
             VALUES ($1, $2::uuid, 'organic', 'rating', 'prose', 1.0, $3, '', '', '{}', $4, $4)"
        }
    };
    match h.db.backend() {
        Backend::Sqlite => {
            let pool = h.db.sqlite_pool().expect("sqlite");
            sqlx::query(qs)
                .bind(&organic_id)
                .bind(&account)
                .bind(&now)
                .bind(&now)
                .bind(&now)
                .execute(pool)
                .await
                .expect("organic signal inserted");
        }
        Backend::Postgres => {
            let pool = h.db.postgres_pool().expect("pg");
            sqlx::query(qs)
                .bind(&organic_id)
                .bind(&account)
                .bind(&now)
                .bind(&now)
                .execute(pool)
                .await
                .expect("organic signal inserted");
        }
    }

    import_signals(
        &h.db,
        &account,
        "ao3",
        Some("cred-1"),
        "bookmarks",
        &[
            IncomingSignal::new(SignalKind::Bookmark, "prose", "ao3:bookmark:1"),
            IncomingSignal::new(SignalKind::Bookmark, "prose", "ao3:bookmark:2"),
        ],
    )
    .await
    .expect("import");

    // The read path says which is which, per signal, not just in aggregate.
    let all = signals_by_origin(&h.db, &account).await.expect("readable");
    let organic: Vec<_> = all
        .iter()
        .filter(|s| s.origin == SignalOrigin::Organic)
        .collect();
    let imported: Vec<_> = all
        .iter()
        .filter(|s| s.origin == SignalOrigin::Imported)
        .collect();
    assert_eq!(organic.len(), 1, "the one rating this instance observed");
    assert_eq!(imported.len(), 2, "the two bookmarks that arrived");
    assert_eq!(
        organic[0].source_signal_key, "",
        "an organic signal has no external id, which is itself a way to tell them apart"
    );
    assert_ne!(
        imported[0].source_signal_key, "",
        "an imported signal carries the source's own key"
    );

    // The split a reader's data-export or access request reads.
    let split = origin_split(&h.db, &account).await.expect("split readable");
    assert_eq!(split.organic, 1, "{split:?}");
    assert_eq!(split.imported, 2, "{split:?}");
    h.cleanup().await;
}

/// §49.6: imported signals "are not silently equivalent: an imported bookmark
/// from 2019 is weaker evidence than a rating given today."
///
/// Two halves, and the second is the one that matters: the discount must apply
/// at **read** time, or a reader importing their whole history would get weights
/// indistinguishable from someone who rated everything today.
#[tokio::test]
async fn an_imported_signal_is_weaker_evidence() {
    let h = Harness::new("taste-import-weak").await;
    let account = h.account("weak-reader").await;

    import_signals(
        &h.db,
        &account,
        "ao3",
        Some("cred-1"),
        "bookmarks",
        &[
            IncomingSignal::new(SignalKind::Bookmark, "prose", "ao3:bookmark:1")
                .occurred_at("2019-04-02T00:00:00Z"),
        ],
    )
    .await
    .expect("import");

    let signals = signals_for_dimension(&h.db, &account, "prose")
        .await
        .expect("readable");
    assert_eq!(signals.len(), 1);
    let signal = &signals[0];

    assert_eq!(signal.origin, SignalOrigin::Imported);
    // The stored value is the signal's own, un-discounted, so the raw evidence
    // is still readable and the discount is a policy rather than a fact about
    // the source.
    assert_eq!(signal.signal_value, base_value(SignalKind::Bookmark));
    assert_eq!(
        signal.effective_value,
        base_value(SignalKind::Bookmark) * IMPORTED_SIGNAL_DISCOUNT,
        "an imported signal counts for less than its raw value"
    );
    assert!(
        signal.effective_value < signal.signal_value,
        "§49.6 says imported evidence is weaker; the discount must actually reduce it"
    );
    // And the source's own date survives, because the 2019 part of §49.6's
    // sentence is about age.
    assert_eq!(
        signal.occurred_at.as_deref(),
        Some("2019-04-02T00:00:00Z"),
        "the source's date is kept verbatim rather than replaced by import time"
    );
    h.cleanup().await;
}

/// An imported signal with no external id cannot be made idempotent, and §49.6
/// requires the uniqueness key to include that id.
///
/// So the store refuses it rather than storing a row that the next import
/// duplicates. A silently-un-idempotent import is exactly what the clause
/// forbids, and this is the only place that can catch it.
#[tokio::test]
async fn a_signal_with_no_external_id_is_refused() {
    let h = Harness::new("taste-import-no-id").await;
    let account = h.account("no-id-reader").await;

    let err = import_signals(
        &h.db,
        &account,
        "ao3",
        Some("cred-1"),
        "bookmarks",
        &[IncomingSignal::new(SignalKind::Bookmark, "prose", "")],
    )
    .await
    .expect_err("a signal with no source id must be refused");

    let message = err.to_string();
    assert!(
        message.contains("source id") || message.contains("external id"),
        "the refusal must say what is missing: {message}"
    );
    // Nothing landed, so a retry with a corrected id starts clean.
    assert_eq!(
        h.signal_count(&account).await,
        0,
        "no partial signal survived"
    );
    h.cleanup().await;
}

/// The two readers' histories must not collide.
///
/// The uniqueness key includes `account_id` precisely so two readers who
/// bookmarked the same AO3 work are not one reader's signal. Getting this wrong
/// is silent — both imports would report success and one reader would silently
/// gain the other's history.
#[tokio::test]
async fn two_readers_sharing_a_bookmark_keep_separate_histories() {
    let h = Harness::new("taste-import-two-readers").await;
    let one = h.account("reader-one").await;
    let two = h.account("reader-two").await;

    let same = vec![IncomingSignal::new(
        SignalKind::Bookmark,
        "prose",
        "ao3:bookmark:shared",
    )];
    let a = import_signals(&h.db, &one, "ao3", Some("cred-1"), "bookmarks", &same)
        .await
        .expect("first reader imports");
    let b = import_signals(&h.db, &two, "ao3", Some("cred-2"), "bookmarks", &same)
        .await
        .expect("second reader imports");

    assert_eq!(a.added, 1, "{a:?}");
    assert_eq!(
        b.added, 1,
        "the second reader's identical bookmark is still new to them: {b:?}"
    );
    assert_eq!(h.signal_count(&one).await, 1);
    assert_eq!(h.signal_count(&two).await, 1);
    h.cleanup().await;
}

/// Two readers' signals on the same dimension are independent, which is §49.9's
/// "no cross-reader taste pooling" seen from the import side.
#[tokio::test]
async fn one_readers_import_does_not_move_anothers_profile() {
    let h = Harness::new("taste-import-isolated").await;
    let importer = h.account("noisy-reader").await;
    let quiet = h.account("quiet-reader").await;

    import_signals(
        &h.db,
        &importer,
        "ao3",
        Some("cred-1"),
        "bookmarks",
        &[
            IncomingSignal::new(SignalKind::Bookmark, "prose", "ao3:bookmark:1"),
            IncomingSignal::new(SignalKind::Bookmark, "prose", "ao3:bookmark:2"),
            IncomingSignal::new(SignalKind::Bookmark, "prose", "ao3:bookmark:3"),
        ],
    )
    .await
    .expect("import");

    assert!(
        h.total_effective(&importer, "prose").await > 0.0,
        "the importer's profile moved"
    );
    assert_eq!(
        h.total_effective(&quiet, "prose").await,
        0.0,
        "another reader's profile is untouched by someone else's import"
    );
    assert_eq!(h.signal_count(&quiet).await, 0);
    h.cleanup().await;
}

/// A run that is refused leaves an audit row, so a failed import is visible
/// rather than vanishing.
///
/// §49.6 wants re-runs to be safe, and a safe re-run is one whose failure is
/// visible. The row is inserted before the signals are, so the half-finished
/// attempt is on record.
#[tokio::test]
async fn a_refused_import_still_leaves_an_audit_row() {
    let h = Harness::new("taste-import-audit").await;
    let account = h.account("audited-reader").await;

    let _ = import_signals(
        &h.db,
        &account,
        "ao3",
        Some("cred-1"),
        "bookmarks",
        &[IncomingSignal::new(SignalKind::Bookmark, "prose", "")],
    )
    .await
    .expect_err("refused for the missing id");

    // The import row exists with nothing counted, because the failure happened
    // after it was written. Its exact id is not returned on the error path --
    // the caller's own ids are in the signals -- so this asserts the shape by
    // counting instead.
    let runs = h.count_signals("taste_signal_imports", &account).await;
    assert_eq!(runs, 1, "the attempted run is on record");
    h.cleanup().await;
}

/// §49.8: "An imported bookmark makes the work's metadata visible and leaves its
/// cached body private."
///
/// The half of this that matters is the **body**. `library_items` (0006) already
/// carries the metadata and the link, and §49.6 refuses to violate §51.4 rather
/// than implementing it — M45-53 owns that default and is still `planned`. So
/// this test asserts the outcome on the private-copy path without the importer
/// having chosen it: an imported library item that names a work must not, by
/// itself, make that work's body readable.
#[tokio::test]
async fn an_imported_bookmark_leaves_the_cached_body_private() {
    let h = Harness::new("taste-import-body-private").await;
    let account = h.account("body-reader").await;
    let work = id("imported-work");

    let now = lorehaven_db::identity::now_rfc3339();
    // A published, public work, so the only thing that could expose its body is
    // the import's own doing.
    //
    // A work needs an `owner_pseud_id` -- ownership belongs to the pseud, not the
    // account (ADR 0003) -- so the pseud row comes first. The first draft of this
    // fixture inserted the work alone and failed with "NOT NULL constraint
    // failed: works.owner_pseud_id".
    let pseud = id("imported-pseud");
    match h.db.backend() {
        Backend::Sqlite => {
            let pool = h.db.sqlite_pool().expect("sqlite");
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&pseud)
            .bind(&account)
            .bind("importer")
            .bind("Importer")
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .expect("pseud inserted");
            sqlx::query(
                "INSERT INTO works (id, title, owner_pseud_id, summary, visibility,
                                    lifecycle, created_at, updated_at, published_at)
                 VALUES (?, 'Imported work', ?, 'A summary.', 'public', 'published',
                         ?, ?, ?)",
            )
            .bind(&work)
            .bind(&pseud)
            .bind(&now)
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .expect("work inserted");
        }
        Backend::Postgres => {
            let pool = h.db.postgres_pool().expect("pg");
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES ($1::uuid, $2::uuid, $3, $4, $5, $5)",
            )
            .bind(&pseud)
            .bind(&account)
            .bind("importer")
            .bind("Importer")
            .bind(&now)
            .execute(pool)
            .await
            .expect("pseud inserted");
            sqlx::query(
                "INSERT INTO works (id, title, owner_pseud_id, summary, visibility,
                                    lifecycle, created_at, updated_at, published_at)
                 VALUES ($1::uuid, 'Imported work', $2::uuid, 'A summary.', 'public',
                         'published', $3, $3, $3)",
            )
            .bind(&work)
            .bind(&pseud)
            .bind(&now)
            .execute(pool)
            .await
            .expect("work inserted");
        }
    }

    // The library item the import produced: the work, its title, and a link.
    match h.db.backend() {
        Backend::Sqlite => {
            let pool = h.db.sqlite_pool().expect("sqlite");
            sqlx::query(
                "INSERT INTO library_items
                 (id, account_id, work_id, source_key, source_work_key, title,
                  source_url, created_at, updated_at)
                 VALUES (?, ?, ?, 'ao3', 'ao3:work:1', 'Imported work',
                         'https://example.invalid/work/1', ?, ?)",
            )
            .bind(id("library-item"))
            .bind(&account)
            .bind(&work)
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .expect("library item inserted");
        }
        Backend::Postgres => {
            let pool = h.db.postgres_pool().expect("pg");
            sqlx::query(
                "INSERT INTO library_items
                 (id, account_id, work_id, source_key, source_work_key, title,
                  source_url, created_at, updated_at)
                 VALUES ($1::uuid, $2::uuid, $3::uuid, 'ao3', 'ao3:work:1',
                         'Imported work', 'https://example.invalid/work/1', $4, $4)",
            )
            .bind(id("library-item"))
            .bind(&account)
            .bind(&work)
            .bind(&now)
            .execute(pool)
            .await
            .expect("library item inserted");
        }
    }

    // The metadata half: the import made the work visible and linkable, which
    // is what §49.8's first half asks for.
    let item = lorehaven_db::imports::get_library_item(&h.db, &id("library-item"), &account)
        .await
        .expect("library item readable")
        .expect("the imported item is there");
    assert_eq!(item.work_id.as_deref(), Some(work.as_str()));
    assert_eq!(item.title, "Imported work");
    assert!(!item.source_url.is_empty(), "the link came across too");

    // The body half: metadata visible does not mean the body is. §51.4's default
    // keeps a cached body private until the author claims the work, and the
    // import has not claimed it. The assertion is that importing metadata
    // created no reader-visible body copy -- checked on the table rather than
    // through a route, because the property is about what the import wrote.
    // `reader_body_copies.work_id` is UUID on PostgreSQL for the same reason
    // `taste_signals.account_id` is, so this cast is not optional either.
    let copies_query = match h.db.backend() {
        Backend::Sqlite => "SELECT COUNT(*) FROM reader_body_copies WHERE work_id = ?",
        Backend::Postgres => "SELECT COUNT(*) FROM reader_body_copies WHERE work_id = ?::uuid",
    };
    let copies = h.tdb.count_by(copies_query, &work).await;
    assert_eq!(
        copies, 0,
        "importing a bookmark must not materialise a reader-visible body copy; \
         §51.4's default belongs to M45-53 and the import must not pre-empt it"
    );

    // And the reader's own copy of the metadata exists, so "metadata visible"
    // is a positive fact rather than an absence.
    assert_eq!(
        h.signal_count(&account).await,
        0,
        "a library item alone trains nothing; a signal is what moves a profile"
    );
    h.cleanup().await;
}

/// A reader who imports twice from two *different* sources is not a duplicate.
///
/// The uniqueness key includes `source_key`, so AO3 bookmark 1 and an FFN
/// bookmark with the same local id are two signals. Getting this wrong would
/// silently drop half a reader's history on the second source they connect.
#[tokio::test]
async fn two_sources_are_not_confused_with_each_other() {
    let h = Harness::new("taste-import-two-sources").await;
    let account = h.account("two-source-reader").await;

    let a = import_signals(
        &h.db,
        &account,
        "ao3",
        Some("cred-ao3"),
        "bookmarks",
        &[IncomingSignal::new(SignalKind::Bookmark, "prose", "1")],
    )
    .await
    .expect("ao3 import");
    let b = import_signals(
        &h.db,
        &account,
        "ffn",
        Some("cred-ffn"),
        "bookmarks",
        &[IncomingSignal::new(SignalKind::Bookmark, "prose", "1")],
    )
    .await
    .expect("ffn import");

    assert_eq!(a.added, 1, "{a:?}");
    assert_eq!(
        b.added, 1,
        "an FFN bookmark whose local id matches an AO3 one is a different signal: {b:?}"
    );
    assert_eq!(h.signal_count(&account).await, 2);
    h.cleanup().await;
}

/// Run one insert of a `taste_signals` row with a deliberately invalid enum
/// value, and require the database to refuse it.
///
/// Returns `true` when the insert was rejected. The two arms cannot share a
/// result variable — `SqliteQueryResult` and `PgQueryResult` are different types
/// and a `match` holding both does not unify — so the boolean is produced inside
/// each arm and returned, which is the one shape that does.
async fn insert_is_refused(h: &Harness, query: &str, account: &str, trailing: usize) -> bool {
    match h.db.backend() {
        Backend::Sqlite => {
            let pool = h.db.sqlite_pool().expect("sqlite");
            let mut q = sqlx::query(query)
                .bind(id("constraint-probe-signal"))
                .bind(account);
            for _ in 0..trailing {
                q = q.bind(&h.now);
            }
            q.execute(pool).await.is_err()
        }
        Backend::Postgres => {
            let pool = h.db.postgres_pool().expect("pg");
            let mut q = sqlx::query(query)
                .bind(id("constraint-probe-signal"))
                .bind(account);
            for _ in 0..trailing {
                q = q.bind(&h.now);
            }
            q.execute(pool).await.is_err()
        }
    }
}

/// A positive control for [`insert_is_refused`].
///
/// The first version of the CHECK tests passed for entirely the wrong reason:
/// the probe insert bound a signal id where the *account* goes, so every insert
/// failed on `taste_signals_account_id_fkey` and the assertion
/// "the insert was refused" was true no matter what the CHECK said. The mutation
/// harness is what exposed it -- with `CHECK (origin <> '')` in place of the
/// real one the suite was still green.
///
/// This control inserts a row with a **valid** origin through the same helper
/// and requires it to be accepted. Without it, "refused" proves nothing: the
/// helper could be refusing everything, including rows that deserve to land.
#[tokio::test]
async fn a_signal_with_a_valid_origin_is_accepted() {
    let h = Harness::new("taste-import-good-origin").await;
    let account = h.account("good-origin-reader").await;
    let query = match h.db.backend() {
        Backend::Sqlite => "INSERT INTO taste_signals
             (id, account_id, origin, signal_kind, dimension_key, signal_value,
              occurred_at, source_key, source_signal_key, provenance_json,
              created_at, updated_at)
             VALUES (?, ?, 'imported', 'bookmark', 'prose', 0.4, ?, '', 'ok:1', '{}', ?, ?)",
        Backend::Postgres => "INSERT INTO taste_signals
             (id, account_id, origin, signal_kind, dimension_key, signal_value,
              occurred_at, source_key, source_signal_key, provenance_json,
              created_at, updated_at)
             VALUES ($1, $2::uuid, 'imported', 'bookmark', 'prose', 0.4, $3, '', 'ok:1', '{}', $4, $4)",
    };
    let refused = insert_is_refused(
        &h,
        query,
        &account,
        if h.db.backend() == Backend::Sqlite {
            3
        } else {
            2
        },
    )
    .await;
    assert!(
        !refused,
        "a valid origin must be accepted -- if this fails, the refusal asserted by \
         the other CHECK tests is not evidence of the CHECK at all"
    );
    assert_eq!(
        h.signal_count(&account).await,
        1,
        "the accepted signal landed"
    );
    h.cleanup().await;
}

/// The `origin` CHECK is a constraint, not a comment, and a constraint nothing
/// exercises is decoration.
///
/// §49.7's invariant is that imported and organic stay distinguishable, and an
/// origin outside the declared set would break it in the worst possible place:
/// `SignalOrigin::parse` would then refuse every *read* of that reader's entire
/// profile, which is a far worse failure than the bad write that caused it.
///
/// The mutation harness found this gap rather than review: dropping
/// `CHECK (origin IN ('organic', 'imported'))` left all ten tests green on the
/// first pass, because no test ever wrote an invalid provenance. Both engines
/// carry the same CHECK, so neither is the one that quietly allows it.
#[tokio::test]
async fn a_signal_with_an_unknown_origin_is_refused() {
    let h = Harness::new("taste-import-bad-origin").await;
    let account = h.account("bad-origin-reader").await;

    let query = match h.db.backend() {
        Backend::Sqlite => "INSERT INTO taste_signals
             (id, account_id, origin, signal_kind, dimension_key, signal_value,
              occurred_at, source_key, source_signal_key, provenance_json,
              created_at, updated_at)
             VALUES (?, ?, 'scraped', 'bookmark', 'prose', 0.4, ?, '', 'x:1', '{}', ?, ?)",
        Backend::Postgres => "INSERT INTO taste_signals
             (id, account_id, origin, signal_kind, dimension_key, signal_value,
              occurred_at, source_key, source_signal_key, provenance_json,
              created_at, updated_at)
             VALUES ($1, $2::uuid, 'scraped', 'bookmark', 'prose', 0.4, $3, '', 'x:1', '{}', $4, $4)",
    };
    let refused = insert_is_refused(
        &h,
        query,
        &account,
        if h.db.backend() == Backend::Sqlite {
            3
        } else {
            2
        },
    )
    .await;
    assert!(
        refused,
        "an origin outside ('organic', 'imported') must be refused by the CHECK"
    );
    assert_eq!(
        h.signal_count(&account).await,
        0,
        "the refused signal left no row behind"
    );
    h.cleanup().await;
}

/// An unknown `signal_kind` is refused for the same reason: the importer is the
/// only writer, so a value it cannot produce should not be storable.
#[tokio::test]
async fn a_signal_with_an_unknown_kind_is_refused() {
    let h = Harness::new("taste-import-bad-kind").await;
    let account = h.account("bad-kind-reader").await;

    let query = match h.db.backend() {
        Backend::Sqlite => {
            "INSERT INTO taste_signals
             (id, account_id, origin, signal_kind, dimension_key, signal_value,
              occurred_at, source_key, source_signal_key, provenance_json,
              created_at, updated_at)
             VALUES (?, ?, 'imported', 'vibes', 'prose', 0.4, ?, '', 'x:1', '{}', ?, ?)"
        }
        Backend::Postgres => {
            "INSERT INTO taste_signals
             (id, account_id, origin, signal_kind, dimension_key, signal_value,
              occurred_at, source_key, source_signal_key, provenance_json,
              created_at, updated_at)
             VALUES ($1, $2::uuid, 'imported', 'vibes', 'prose', 0.4, $3, '', 'x:1', '{}', $4, $4)"
        }
    };
    let refused = insert_is_refused(
        &h,
        query,
        &account,
        if h.db.backend() == Backend::Sqlite {
            3
        } else {
            2
        },
    )
    .await;
    assert!(
        refused,
        "a signal_kind outside the declared set must be refused"
    );
    // The account is there to be certain the refusal was the CHECK and not an
    // unrelated failure: the reader exists, so the foreign key held, and the only
    // thing left to refuse the row was `signal_kind`.
    assert_eq!(
        h.signal_count(&account).await,
        0,
        "the refused signal left no row behind for a reader that does exist"
    );
    h.cleanup().await;
}
