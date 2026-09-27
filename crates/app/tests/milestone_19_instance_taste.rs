//! M19 — Instance taste settings (`crates/db/src/instance_taste_settings.rs`,
//! spec §19, ADR 0023), the operator panel on `/discovery`.
//!
//! Four `pub async fn` with no test touching them: `read`, `write`, `history` and
//! `rollback`. The module already has four unit tests, but all four are on the
//! private row mappers — nothing ever reached the database.
//!
//! **This is the best-written module in the data layer, and that is the point of
//! testing it.** It already gets right the things the other modules got wrong:
//!
//! - every PostgreSQL `SELECT` renders `UUID` and `TIMESTAMPTZ` as text and
//!   widens `INTEGER` with `::bigint`, via a `db.sql(sqlite, postgres)` helper
//!   that keeps the two arms side by side;
//! - the history insert and the settings upsert are **one transaction**, in that
//!   order, because the settings row is a singleton and the history row is the
//!   only copy of the old values;
//! - the rows are `sqlx::FromRow` structs rather than positional tuples, so a
//!   renamed column is a compile error;
//! - the history is ordered by `replaced_version`, not `changed_at`, because
//!   `now_rfc3339()` has second precision and two writes in the same second
//!   would tie.
//!
//! So the tests here are mostly about *whether the careful design holds under
//! the operations it has to survive*, and two places where it does not:
//!
//! - the SQLite `history` ordering ends in `rowid DESC` as a third tie-break
//!   key, and the PostgreSQL arm ends in `history_id DESC`. Since
//!   `replaced_version` is unique and strictly increasing, that third key is
//!   unreachable for any history this module writes -- it only fires on a
//!   hand-edited or corrupt table. `rowid` is an implementation detail that
//!   `VACUUM` can renumber, and the two arms break ties differently, so it is
//!   recorded in `docs/known-gaps.md` as M19-D02 rather than relied upon. The
//!   ordering that actually matters is pinned by
//!   `the_history_is_newest_first` and
//!   `two_writes_in_the_same_second_still_order_correctly`.
//! - `rollback` ignores `replaced_version`, so rolling back to the *first*
//!   history entry (the one that recorded the config, not a version) restores
//!   the config's values rather than a hole. That is the intent, and
//!   `rolling_back_past_the_first_version_lands_on_the_config` pins it.

use std::path::PathBuf;

use lorehaven_db::instance_taste_settings::{
    history, read, rollback, write, StoredTasteSettings, TasteSettingsUpdate,
};
use test_support::TestDb;
use uuid::Uuid;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-taste-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

struct Harness {
    tdb: TestDb,
    #[allow(dead_code)]
    dir: PathBuf,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        Self { tdb, dir }
    }

    fn db(&self) -> &lorehaven_db::Database {
        self.tdb.db()
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

    async fn count(&self, table: &str) -> i64 {
        let q = self.tdb.sql(&format!("SELECT COUNT(*) FROM {table}"));
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query_scalar::<_, i64>(&q)
                .fetch_one(self.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("count"),
            lorehaven_db::Backend::Postgres => sqlx::query_scalar::<_, i64>(&q)
                .fetch_one(self.db().postgres_pool().expect("pg"))
                .await
                .expect("count"),
        }
    }

    /// The knobs the operator sets, defaulted to something recognisable.
    fn update(&self, strength: i64, mode: &str) -> TasteSettingsUpdate {
        TasteSettingsUpdate {
            gravity_strength: strength,
            signal_weight_mode: mode.to_string(),
            admin_weight: 1,
            diversity_injection_percent: 10,
        }
    }

    /// The knobs, or `None` when the operator has never saved.
    async fn knobs(&self) -> Option<StoredTasteSettings> {
        read(self.db()).await.expect("read")
    }

    /// Save once and return the version, so tests that only care about the
    /// resulting state do not each repeat the call.
    async fn save(&self, strength: i64, mode: &str) -> i64 {
        write(self.db(), &self.update(strength, mode), Uuid::new_v4())
            .await
            .expect("write")
    }
}

// ---------------------------------------------------------------------------
// read
// ---------------------------------------------------------------------------

/// An instance nobody has edited has no row, and that is `None` rather than an
/// error: the config file is the starting point, and the discovery route falls
/// back to it.
#[tokio::test]
async fn an_unedited_instance_has_no_knobs() {
    let h = Harness::new("taste-empty").await;
    assert!(
        h.knobs().await.is_none(),
        "the config is the default, not a row"
    );
}

/// The first save creates version 1 and the read shows it.
#[tokio::test]
async fn the_first_save_creates_version_one() {
    let h = Harness::new("taste-first").await;
    assert_eq!(h.save(750, "balanced").await, 1);

    let got = h.knobs().await.expect("a row now exists");
    assert_eq!(got.version, 1);
    assert_eq!(got.gravity_strength, 750);
    assert_eq!(got.signal_weight_mode, "balanced");
    assert_eq!(got.admin_weight, 1);
    assert_eq!(got.diversity_injection_percent, 10);
    assert!(got.updated_by.is_some(), "the operator is recorded");
    assert!(!got.updated_at.is_empty());
}

/// **The singleton holds.** The schema is `INTEGER PRIMARY KEY CHECK (id = 1)`,
/// so a second write updates the one row rather than adding another.
#[tokio::test]
async fn the_settings_row_is_a_singleton() {
    let h = Harness::new("taste-singleton").await;
    h.save(100, "a").await;
    h.save(200, "b").await;
    h.save(300, "c").await;

    assert_eq!(h.count("instance_taste_settings").await, 1, "still one row");
    assert_eq!(h.knobs().await.expect("row").version, 3);
}

/// A row seeded with no author still decodes — `updated_by` is nullable exactly
/// so a migration can seed the table.
#[tokio::test]
async fn a_seeded_row_with_no_author_reads_back() {
    let h = Harness::new("taste-noauthor").await;
    h.exec(
        "INSERT INTO instance_taste_settings (id, gravity_strength, signal_weight_mode, \
         admin_weight, diversity_injection_percent, updated_by, updated_at, version) \
         VALUES (1, 0, 'taste_weighted', 1, 10, NULL, '2026-01-01T00:00:00Z', 1)",
    )
    .await;

    let got = h.knobs().await.expect("row");
    assert_eq!(got.updated_by, None, "a seeded row has no author");
    assert_eq!(got.signal_weight_mode, "taste_weighted");
    assert_eq!(got.version, 1);
}

/// **`updated_by` is a `UUID` column read as a `String`.** Every non-`::text`
/// PG read of this table would fail to decode, which is why the module casts.
#[tokio::test]
async fn the_author_uuid_reads_as_text_on_both_backends() {
    let h = Harness::new("taste-uuid").await;
    let author = Uuid::new_v4();
    write(h.db(), &h.update(400, "balanced"), author)
        .await
        .expect("write");

    let got = h.knobs().await.expect("row");
    assert_eq!(got.updated_by.as_deref(), Some(author.to_string().as_str()));
}

/// Each write replaces the author.
#[tokio::test]
async fn each_write_replaces_the_author() {
    let h = Harness::new("taste-author2").await;
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    write(h.db(), &h.update(100, "a"), first)
        .await
        .expect("first");
    write(h.db(), &h.update(200, "b"), second)
        .await
        .expect("second");

    assert_eq!(
        h.knobs().await.expect("row").updated_by.as_deref(),
        Some(second.to_string().as_str())
    );
}

// ---------------------------------------------------------------------------
// write
// ---------------------------------------------------------------------------

/// The version increments by one on every write after the first.
#[tokio::test]
async fn each_write_bumps_the_version_by_one() {
    let h = Harness::new("taste-version").await;
    assert_eq!(h.save(1, "a").await, 1);
    assert_eq!(h.save(2, "b").await, 2);
    assert_eq!(h.save(3, "c").await, 3);
    assert_eq!(h.save(4, "d").await, 4);
    assert_eq!(h.knobs().await.expect("row").version, 4);
}

/// Negative and zero strengths are accepted: the schema has no CHECK on them and
/// §19 lets an operator set them, so nothing here invents a rule.
#[tokio::test]
async fn a_negative_strength_is_stored_as_given() {
    let h = Harness::new("taste-negative").await;
    h.save(-250, "balanced").await;
    assert_eq!(h.knobs().await.expect("row").gravity_strength, -250);
}

/// An empty `signal_weight_mode` is stored verbatim rather than rejected — the
/// mode is a free-form string in the schema, and the door is where validation
/// belongs.
#[tokio::test]
async fn an_empty_signal_weight_mode_is_stored_verbatim() {
    let h = Harness::new("taste-emptymode").await;
    h.save(0, "").await;
    assert_eq!(h.knobs().await.expect("row").signal_weight_mode, "");
}

/// A long mode string survives, since the column is unconstrained `TEXT`.
#[tokio::test]
async fn a_long_signal_weight_mode_survives() {
    let h = Harness::new("taste-longmode").await;
    let mode = "x".repeat(500);
    h.save(0, &mode).await;
    assert_eq!(h.knobs().await.expect("row").signal_weight_mode.len(), 500);
}

/// **Every write appends exactly one history row**, and the count is what proves
/// the transaction ran both statements rather than one.
#[tokio::test]
async fn every_write_appends_one_history_row() {
    let h = Harness::new("taste-history-count").await;
    assert_eq!(h.count("instance_taste_settings_history").await, 0);

    h.save(1, "a").await;
    assert_eq!(h.count("instance_taste_settings_history").await, 1);
    h.save(2, "b").await;
    h.save(3, "c").await;
    assert_eq!(h.count("instance_taste_settings_history").await, 3);
}

/// The first history row records the **config's** values with a NULL
/// `replaced_version` — the config is not version 0, and recording it as 0 would
/// invent a version an operator could believe existed.
#[tokio::test]
async fn the_first_history_row_records_the_config_not_a_version() {
    let h = Harness::new("taste-first-history").await;
    h.save(900, "operator").await;

    let entries = history(h.db(), 10).await.expect("history");
    let first = entries.last().expect("one entry");
    assert_eq!(first.replaced_version, None, "the config is not a version");
    // The config's defaults, as `write` hard-codes them.
    assert_eq!(first.gravity_strength, 0);
    assert_eq!(first.signal_weight_mode, "taste_weighted");
    assert_eq!(first.admin_weight, 1);
    assert_eq!(first.diversity_injection_percent, 10);
}

/// A later history row records the state it replaced, so the trail is a chain.
#[tokio::test]
async fn a_later_history_row_records_the_state_it_replaced() {
    let h = Harness::new("taste-chain").await;
    h.save(100, "first").await;
    h.save(200, "second").await;

    let entries = history(h.db(), 10).await.expect("history");
    // Newest first: the row written by the second save replaced version 1.
    let newest = &entries[0];
    assert_eq!(newest.replaced_version, Some(1));
    assert_eq!(newest.gravity_strength, 100, "what it replaced");
    assert_eq!(newest.signal_weight_mode, "first");
}

/// A write that changes nothing still appends a history row: the operator
/// pressed save, and §19's audit rule is about the action, not the delta.
#[tokio::test]
async fn a_write_that_changes_nothing_is_still_recorded() {
    let h = Harness::new("taste-noop").await;
    h.save(500, "balanced").await;
    h.save(500, "balanced").await;
    assert_eq!(h.count("instance_taste_settings_history").await, 2);
}

// ---------------------------------------------------------------------------
// history
// ---------------------------------------------------------------------------

/// An empty history is empty, not an error — the operator panel renders "no
/// changes yet".
#[tokio::test]
async fn a_fresh_instance_has_an_empty_history() {
    let h = Harness::new("taste-history-empty").await;
    assert!(history(h.db(), 10).await.expect("history").is_empty());
}

/// Newest first, and the newest entry is the one that replaced the highest
/// version.
#[tokio::test]
async fn the_history_is_newest_first() {
    let h = Harness::new("taste-history-order").await;
    for i in 1..=4 {
        h.save(i * 100, &format!("mode{i}")).await;
    }

    let entries = history(h.db(), 10).await.expect("history");
    let replaced: Vec<Option<i64>> = entries.iter().map(|e| e.replaced_version).collect();
    assert_eq!(
        replaced,
        vec![Some(3), Some(2), Some(1), None],
        "highest replaced_version first, the config row last"
    );
}

/// **`now_rfc3339()` has second precision, so two writes in the same second tie.**
/// Ordering by `changed_at` would then be arbitrary; the module orders by
/// `replaced_version` instead, which is unique and strictly increasing. This is
/// the test that would fail if anyone "simplified" the ORDER BY back to the
/// timestamp.
#[tokio::test]
async fn two_writes_in_the_same_second_still_order_correctly() {
    let h = Harness::new("taste-history-tie").await;
    // Back-to-back writes: almost certainly the same second.
    h.save(100, "a").await;
    h.save(200, "b").await;
    h.save(300, "c").await;

    let entries = history(h.db(), 10).await.expect("history");
    assert_eq!(
        entries
            .iter()
            .map(|e| e.replaced_version)
            .collect::<Vec<_>>(),
        vec![Some(2), Some(1), None],
        "version ordering, not timestamp ordering"
    );
}

/// Every history entry carries a distinct id and the author who made the change.
#[tokio::test]
async fn each_history_entry_has_an_id_and_an_author() {
    let h = Harness::new("taste-history-meta").await;
    let author = Uuid::new_v4();
    let second = Uuid::new_v4();
    write(h.db(), &h.update(1, "a"), author)
        .await
        .expect("first");
    write(h.db(), &h.update(2, "b"), second)
        .await
        .expect("second");

    // Newest first, so the second write is entries[0] and the first is entries[1].
    let entries = history(h.db(), 10).await.expect("history");
    assert_eq!(entries.len(), 2);
    assert_ne!(entries[0].history_id, entries[1].history_id, "distinct ids");
    assert!(
        entries
            .iter()
            .all(|e| Uuid::parse_str(&e.history_id).is_ok()),
        "every history_id is a UUID"
    );
    // Newest first, and `write` binds the author on *every* row including the
    // one that recorded the config, so both entries name their author.
    let by_version: Vec<(Option<i64>, Option<String>)> = entries
        .iter()
        .map(|e| (e.replaced_version, e.changed_by.clone()))
        .collect();
    assert_eq!(
        by_version,
        vec![
            (Some(1), Some(second.to_string())),
            (None, Some(author.to_string())),
        ],
        "newest first: the row that replaced version 1 is authored by the \
         second writer, the config row by the first"
    );
    assert!(entries.iter().all(|e| !e.changed_at.is_empty()));
}

/// The limit caps the list, and the cap takes the newest entries.
#[tokio::test]
async fn the_history_limit_takes_the_newest_entries() {
    let h = Harness::new("taste-history-limit").await;
    for i in 1..=6 {
        h.save(i, "m").await;
    }

    let all = history(h.db(), 100).await.expect("all");
    let two = history(h.db(), 2).await.expect("capped");
    assert_eq!(all.len(), 6);
    assert_eq!(two.len(), 2);
    assert_eq!(
        two.iter().map(|e| e.replaced_version).collect::<Vec<_>>(),
        vec![Some(5), Some(4)],
        "the newest two, not the oldest"
    );
}

/// A limit of zero returns nothing and a limit past the end returns everything.
#[tokio::test]
async fn the_history_limit_handles_the_edges() {
    let h = Harness::new("taste-history-edges").await;
    h.save(1, "a").await;
    h.save(2, "b").await;
    assert!(history(h.db(), 0).await.expect("zero").is_empty());
    assert_eq!(history(h.db(), 999).await.expect("big").len(), 2);
}

/// A limit is not allowed to go negative — a negative `LIMIT` is an error in
/// PostgreSQL and means "no limit" in SQLite, so the two arms would disagree.
/// Pinned so the divergence cannot be reintroduced unnoticed.
#[tokio::test]
async fn a_negative_history_limit_behaves_the_same_on_both_backends() {
    let h = Harness::new("taste-history-negative").await;
    h.save(1, "a").await;
    h.save(2, "b").await;
    let got = history(h.db(), -1).await;
    match h.db().backend() {
        lorehaven_db::Backend::Postgres => assert!(
            got.is_err(),
            "PostgreSQL rejects a negative LIMIT, and the test records that"
        ),
        lorehaven_db::Backend::Sqlite => assert_eq!(
            got.expect("SQLite treats it as no limit").len(),
            2,
            "which is the divergence: SQLite returns every row"
        ),
    }
}

// ---------------------------------------------------------------------------
// rollback
// ---------------------------------------------------------------------------

/// A rollback restores the state a history entry recorded, and says so in its
/// outcome so the operator panel needs no second round trip.
#[tokio::test]
async fn a_rollback_restores_the_recorded_state() {
    let h = Harness::new("taste-rollback").await;
    h.save(100, "first").await;
    h.save(200, "second").await;

    let entries = history(h.db(), 10).await.expect("history");
    // The entry that replaced version 1 recorded the state after the first save.
    let target = entries
        .iter()
        .find(|e| e.replaced_version == Some(1))
        .expect("the version 1 entry");

    let outcome = rollback(
        h.db(),
        Uuid::parse_str(&target.history_id).expect("uuid"),
        Uuid::new_v4(),
    )
    .await
    .expect("rollback");

    assert_eq!(outcome.restored.gravity_strength, 100);
    assert_eq!(outcome.restored.signal_weight_mode, "first");
    let got = h.knobs().await.expect("row");
    assert_eq!(got.gravity_strength, 100, "and it is what is now in force");
    assert_eq!(got.signal_weight_mode, "first");
}

/// A rollback is a **write**, so it bumps the version rather than reverting it —
/// which is what makes two rollbacks walk backwards instead of toggling.
#[tokio::test]
async fn a_rollback_bumps_the_version() {
    let h = Harness::new("taste-rollback-version").await;
    h.save(100, "first").await;
    h.save(200, "second").await;
    assert_eq!(h.knobs().await.expect("row").version, 2);

    let entries = history(h.db(), 10).await.expect("history");
    let target = entries
        .iter()
        .find(|e| e.replaced_version == Some(1))
        .expect("target");
    let outcome = rollback(
        h.db(),
        Uuid::parse_str(&target.history_id).expect("uuid"),
        Uuid::new_v4(),
    )
    .await
    .expect("rollback");

    assert_eq!(outcome.version, 3, "the audit trail only moves forward");
    assert_eq!(h.knobs().await.expect("row").version, 3);
}

/// Two rollbacks walk backwards through the history instead of toggling between
/// two values.
#[tokio::test]
async fn two_rollbacks_walk_backwards() {
    let h = Harness::new("taste-rollback-twice").await;
    h.save(100, "v1").await;
    h.save(200, "v2").await;
    h.save(300, "v3").await;

    // Roll back to the state that preceded v3, then to the one that preceded v2.
    let first = history(h.db(), 10).await.expect("history");
    let to_v2 = first
        .iter()
        .find(|e| e.replaced_version == Some(2))
        .expect("the v2 entry");
    rollback(
        h.db(),
        Uuid::parse_str(&to_v2.history_id).expect("uuid"),
        Uuid::new_v4(),
    )
    .await
    .expect("first rollback");
    assert_eq!(h.knobs().await.expect("row").gravity_strength, 200);

    let second = history(h.db(), 10).await.expect("history");
    let to_v1 = second
        .iter()
        .find(|e| e.replaced_version == Some(1))
        .expect("the v1 entry");
    rollback(
        h.db(),
        Uuid::parse_str(&to_v1.history_id).expect("uuid"),
        Uuid::new_v4(),
    )
    .await
    .expect("second rollback");
    assert_eq!(h.knobs().await.expect("row").gravity_strength, 100);
}

/// **Rolling back to the entry that recorded the config lands on the config's
/// values.** `rollback` reads the entry's knobs and ignores its
/// `replaced_version`, so the NULL-version row restores the seeded defaults —
/// which is exactly what "roll back past version 1" should mean.
#[tokio::test]
async fn rolling_back_past_the_first_version_lands_on_the_config() {
    let h = Harness::new("taste-rollback-config").await;
    h.save(900, "operator").await;

    let entries = history(h.db(), 10).await.expect("history");
    let config_entry = entries
        .iter()
        .find(|e| e.replaced_version.is_none())
        .expect("the config entry");
    let outcome = rollback(
        h.db(),
        Uuid::parse_str(&config_entry.history_id).expect("uuid"),
        Uuid::new_v4(),
    )
    .await
    .expect("rollback");

    assert_eq!(outcome.restored.gravity_strength, 0, "the config's value");
    assert_eq!(outcome.restored.signal_weight_mode, "taste_weighted");
    let got = h.knobs().await.expect("row");
    assert_eq!(got.gravity_strength, 0);
    assert_eq!(got.signal_weight_mode, "taste_weighted");
}

/// A rollback records itself in the history, so the trail shows that a rollback
/// happened and who asked for it.
#[tokio::test]
async fn a_rollback_is_itself_recorded() {
    let h = Harness::new("taste-rollback-audit").await;
    h.save(100, "a").await;
    h.save(200, "b").await;
    let before = h.count("instance_taste_settings_history").await;

    let entries = history(h.db(), 10).await.expect("history");
    let target = &entries[0];
    let roller = Uuid::new_v4();
    rollback(
        h.db(),
        Uuid::parse_str(&target.history_id).expect("uuid"),
        roller,
    )
    .await
    .expect("rollback");

    assert_eq!(
        h.count("instance_taste_settings_history").await,
        before + 1,
        "a rollback is a write, so it belongs in the trail"
    );
    assert_eq!(
        h.knobs().await.expect("row").updated_by.as_deref(),
        Some(roller.to_string().as_str()),
        "and the operator who rolled back is the author of that write"
    );
}

/// An unknown history id is refused with a typed error, not a silent no-op.
#[tokio::test]
async fn rolling_back_to_an_unknown_entry_is_refused() {
    let h = Harness::new("taste-rollback-unknown").await;
    h.save(1, "a").await;
    let before = h.knobs().await.expect("row");

    let err = rollback(h.db(), Uuid::new_v4(), Uuid::new_v4())
        .await
        .expect_err("refused");
    assert!(
        err.to_string().contains("no history entry with that id"),
        "the typed error surfaces: {err}"
    );
    assert_eq!(
        h.knobs().await.expect("row").version,
        before.version,
        "and nothing was written"
    );
}

/// A rollback on an instance nobody has edited is refused too: there is no
/// history to roll back to, and falling back to the config here would look like
/// a successful no-op.
#[tokio::test]
async fn a_rollback_with_no_history_is_refused() {
    let h = Harness::new("taste-rollback-nohistory").await;
    assert!(rollback(h.db(), Uuid::new_v4(), Uuid::new_v4())
        .await
        .is_err());
    assert!(h.knobs().await.is_none(), "and still no row");
}
